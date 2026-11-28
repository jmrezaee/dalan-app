//! Bulk export (D17, E6; replica/databases/restore-copy.md §9): many tables or collections at once, one file each,
//! into a new folder or a `.zip`, with a `manifest.json` (the source, the time, and per table its file, columns
//! and types, and rows) and, if asked, `schema.sql` with the tables' DDL. Masking rules apply to every table that
//! has the column. Rust picks the place and writes every file; an export that doesn't finish leaves nothing.

use std::io::Write;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::dbs::masking::MaskRule;
use crate::dbs::schedule::safe;
use crate::error::{AppError, AppResult};
use crate::sql::transfer::ExportFormat;
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, Type)]
pub struct BulkObject {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct BulkExport {
    pub objects: Vec<BulkObject>,
    /// SQL tables only (collections are always JSON lines).
    pub format: ExportFormat,
    pub header: bool,
    pub masks: Vec<MaskRule>,
    /// `schema.sql` with the tables' CREATE statements (SQL engines).
    pub ddl: bool,
    pub zip: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkProgress {
    pub index: f64,
    pub total: f64,
    pub table: String,
    pub rows: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkDone {
    /// The folder or the zip (its name only is shown).
    pub name: String,
    pub files: f64,
    pub rows: f64,
    pub warnings: Vec<String>,
}

/// Files go into a folder or into one zip.
pub(crate) enum Sink {
    Dir(std::path::PathBuf),
    Zip(Box<zip::ZipWriter<std::fs::File>>),
}

impl Sink {
    /// Runs `write` with a writer for the file `name`.
    fn file(
        &mut self,
        name: &str,
        write: &mut dyn FnMut(&mut (dyn Write + Send)) -> AppResult<()>,
    ) -> AppResult<()> {
        let io = |e: std::io::Error| AppError::internal(format!("Couldn't write {name}: {e}"));
        match self {
            Sink::Dir(d) => {
                let mut f =
                    std::io::BufWriter::new(std::fs::File::create(d.join(name)).map_err(io)?);
                write(&mut f)?;
                f.flush().map_err(io)
            }
            Sink::Zip(z) => {
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                z.start_file(name, opts)
                    .map_err(|e| AppError::internal(format!("Couldn't write {name}: {e}")))?;
                write(z.as_mut())
            }
        }
    }
}

/// The table's columns and types, for the manifest.
async fn columns(raw: &crate::sql::RawConn, schema: &str, table: &str) -> serde_json::Value {
    match crate::sql::datacmp::columns(raw, schema, table).await {
        Ok((cols, pk)) => serde_json::json!({
            "columns": cols.iter().map(|c| serde_json::json!({ "name": c.name, "type": c.full_type })).collect::<Vec<_>>(),
            "primary_key": pk,
        }),
        Err(_) => serde_json::json!({}),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn bulk_export(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    transfer_id: String,
    request: BulkExport,
    progress: Channel<BulkProgress>,
) -> AppResult<Option<BulkDone>> {
    state.require_unlocked()?;
    if request.objects.is_empty() {
        return Err(AppError::validation("Choose at least one table."));
    }
    let db_id = state.sql.db_id(&conn_id)?;
    let cfg = crate::sql::load(&state.store(), &db_id).await?;
    let saved = crate::sql::gate_target(&state.store(), &db_id).await?;
    let base = format!(
        "{}-export-{}",
        safe(&cfg.database.clone().unwrap_or_else(|| saved.name.clone())),
        chrono::Local::now().format("%Y-%m-%d-%H%M")
    );
    // Where: a new folder inside the one chosen, or a zip.
    let (target, sink) = if request.zip {
        let Some(path) = super::sql::pick_save(&app, format!("{base}.zip")).await? else {
            return Ok(None);
        };
        let f = std::fs::File::create(&path)
            .map_err(|e| AppError::validation(format!("Couldn't create that file: {e}")))?;
        (path, Sink::Zip(Box::new(zip::ZipWriter::new(f))))
    } else {
        let a = app.clone();
        let picked = tokio::task::spawn_blocking(move || {
            crate::commands::sql::pick_or("folder", || {
                a.dialog()
                    .file()
                    .set_title("Folder for the export")
                    .blocking_pick_folder()
            })
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
        let Some(dir) = picked else { return Ok(None) };
        let dir = dir
            .into_path()
            .map_err(|_| AppError::validation("That folder can't be used."))?
            .join(&base);
        std::fs::create_dir(&dir)
            .map_err(|e| AppError::validation(format!("Couldn't make the folder {base}: {e}")))?;
        (dir.clone(), Sink::Dir(dir))
    };
    let stop = state.sql.begin_transfer(&transfer_id);
    let result = export_into(&state, &conn_id, &request, sink, &stop, &|p| {
        let _ = progress.send(p);
    })
    .await
    .map(|(files, rows, warnings)| BulkDone {
        name: target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files,
        rows,
        warnings,
    });
    state.sql.end_transfer(&transfer_id);
    match result {
        Ok(done) => Ok(Some(done)),
        Err(e) => {
            // An export that didn't finish leaves nothing behind.
            if request.zip {
                let _ = std::fs::remove_file(&target);
            } else {
                let _ = std::fs::remove_dir_all(&target);
            }
            Err(e)
        }
    }
}

/// Writes the export into `sink` (a new folder, or a zip it finishes). Returns the files, rows and warnings.
pub(crate) async fn export_into(
    state: &AppState,
    conn_id: &str,
    request: &BulkExport,
    mut sink: Sink,
    stop: &std::sync::atomic::AtomicBool,
    progress: &(dyn Fn(BulkProgress) + Sync),
) -> AppResult<(f64, f64, Vec<String>)> {
    let db_id = state.sql.db_id(conn_id)?;
    let cfg = crate::sql::load(&state.store(), &db_id).await?;
    let saved = crate::sql::gate_target(&state.store(), &db_id).await?;
    let mongo = crate::sql::backup::is_mongo(cfg.engine);
    let raw = if mongo {
        None
    } else {
        Some(state.sql.connect_to(conn_id, None).await?)
    };
    let mut warnings = Vec::new();
    if mongo && !request.masks.is_empty() {
        warnings.push(
            "Masking isn't applied to collections yet: their documents are exported as they are."
                .to_string(),
        );
    }
    let total = request.objects.len() as f64;
    let (mut tables, mut all_rows) = (Vec::new(), 0f64);
    for (i, o) in request.objects.iter().enumerate() {
        let ext = if mongo {
            "jsonl"
        } else {
            request.format.extension()
        };
        let file = format!("{}.{}.{ext}", safe(&o.schema), safe(&o.name));
        let report = |rows: f64, _bytes: f64| {
            progress(BulkProgress {
                index: i as f64,
                total,
                table: format!("{}.{}", o.schema, o.name),
                rows,
            });
        };
        report(0.0, 0.0);
        let mut rows = 0f64;
        // The writers are blocking `Write`s (a zip entry is one): the export is awaited from inside the write,
        // on this worker (`block_in_place` lets tokio move other tasks off it meanwhile).
        let rt = tokio::runtime::Handle::current();
        sink.file(&file, &mut |out| {
            let r = tokio::task::block_in_place(|| {
                rt.block_on(async {
                    if mongo {
                        let c = state.sql.mongo(conn_id).await?;
                        crate::mongo::docs::export(
                            &c,
                            &o.schema,
                            &o.name,
                            "{}",
                            &request.masks,
                            out,
                            stop,
                            &report,
                        )
                        .await
                    } else if let Some(my) = state.sql.my(conn_id).await? {
                        my.export_table(
                            &o.schema,
                            &o.name,
                            "",
                            request.format,
                            request.header,
                            &request.masks,
                            out,
                            &report,
                            stop,
                        )
                        .await
                        .map(|x| x.0)
                    } else {
                        state
                            .sql
                            .pg(conn_id)
                            .await?
                            .export_table(
                                &o.schema,
                                &o.name,
                                "",
                                request.format,
                                request.header,
                                &request.masks,
                                out,
                                &report,
                                stop,
                            )
                            .await
                            .map(|x| x.0)
                    }
                })
            });
            rows = r?;
            Ok(())
        })?;
        all_rows += rows;
        let mut entry =
            serde_json::json!({ "schema": o.schema, "name": o.name, "file": file, "rows": rows });
        if let Some(raw) = &raw {
            if let (Some(m), serde_json::Value::Object(cols)) = (
                entry.as_object_mut(),
                columns(raw, &o.schema, &o.name).await,
            ) {
                m.extend(cols);
            }
        }
        tables.push(entry);
    }
    if request.ddl {
        if let Some(raw) = &raw {
            let mut text = format!(
                "-- The tables of {} as Dalan exported them.\n\n",
                saved.name
            );
            let mut schemas: Vec<&str> =
                request.objects.iter().map(|o| o.schema.as_str()).collect();
            schemas.dedup();
            for s in schemas {
                let cat = crate::sql::schemacat::read(raw, s).await?;
                let chosen: Vec<&crate::dbs::schemadiff::Table> = cat
                    .tables
                    .iter()
                    .filter(|t| {
                        request
                            .objects
                            .iter()
                            .any(|o| o.schema == s && o.name == t.name)
                    })
                    .collect();
                for t in &chosen {
                    text.push_str(&format!("{};\n\n", t.create));
                    if cat.family == crate::dbs::schemadiff::Family::Pg {
                        for ix in &t.indexes {
                            text.push_str(&format!("{};\n", ix.definition));
                        }
                    }
                }
                for t in &chosen {
                    for c in t.constraints.iter().filter(|c| c.kind == "f") {
                        text.push_str(&match cat.family {
                            crate::dbs::schemadiff::Family::Pg => format!(
                                "ALTER TABLE {} ADD CONSTRAINT {} {};\n",
                                crate::dbs::schemadiff::quote_pg(&t.name),
                                crate::dbs::schemadiff::quote_pg(&c.name),
                                c.definition
                            ),
                            crate::dbs::schemadiff::Family::My => format!(
                                "ALTER TABLE {} ADD {};\n",
                                crate::dbs::schemadiff::quote_my(&t.name),
                                c.definition
                            ),
                        });
                    }
                }
            }
            sink.file("schema.sql", &mut |out| {
                out.write_all(text.as_bytes())
                    .map_err(|e| AppError::internal(e.to_string()))
            })?;
        }
    }
    let manifest = serde_json::json!({
        "made_by": "Dalan",
        "source": { "name": saved.name, "engine": cfg.engine.label(), "database": cfg.database },
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "format": if mongo { "jsonl" } else { request.format.extension() },
        "header": request.header,
        "masked_columns": request.masks.iter().filter(|m| m.kind != crate::dbs::masking::MaskKind::Keep).map(|m| m.column.clone()).collect::<Vec<_>>(),
        "tables": tables,
        "schema_sql": request.ddl && !mongo,
    });
    let text =
        serde_json::to_string_pretty(&manifest).map_err(|e| AppError::internal(e.to_string()))?;
    sink.file("manifest.json", &mut |out| {
        out.write_all(text.as_bytes())
            .map_err(|e| AppError::internal(e.to_string()))
    })?;
    if let Sink::Zip(z) = sink {
        z.finish()
            .map_err(|e| AppError::internal(format!("Couldn't finish the zip: {e}")))?;
    }
    Ok((total, all_rows, warnings))
}

#[cfg(test)]
pub(crate) mod docker_tests {
    use super::*;
    use crate::dbs::store::{DatabaseInput, DbOptions};
    use crate::redis::store::{RedisEnvironment, RedisRoute, RedisWriteMode};

    /// A saved database for the test vault, opened with no window.
    pub(crate) async fn open_db(
        state: &AppState,
        engine: crate::dbs::Engine,
        port: u16,
        db: &str,
        user: &str,
        pw: &str,
    ) -> String {
        let (db, user, pw) = (db.to_string(), user.to_string(), pw.to_string());
        let row = state
            .store()
            .with_db_session(move |c, s| {
                crate::dbs::store::save(
                    c,
                    s,
                    DatabaseInput {
                        id: None,
                        engine,
                        name: format!("t-{db}-{port}"),
                        environment: RedisEnvironment::Staging,
                        route: RedisRoute::Direct,
                        host_id: None,
                        address: "127.0.0.1".into(),
                        port: Some(port),
                        default_database: Some(db),
                        username: Some(user),
                        password: Some(pw),
                        clear_password: false,
                        tls: "off".into(),
                        tls_server_name: None,
                        write_mode: RedisWriteMode::Open,
                        options: DbOptions {
                            auth_source: (engine == crate::dbs::Engine::Mongodb)
                                .then(|| "admin".into()),
                            ..Default::default()
                        },
                        from_url: None,
                    },
                    false,
                )
            })
            .await
            .unwrap();
        state
            .sql
            .open_headless(state.store(), row.id)
            .await
            .expect("opening the test database")
    }

    pub(crate) async fn state() -> AppState {
        let conn = crate::db::open_in_memory().unwrap();
        let session = crate::vault::create(
            &conn,
            "correct-horse-staple",
            crate::vault::kdf::KdfParams::cheap(),
        )
        .unwrap();
        let state = AppState::new(conn);
        state.set_session(Some(session));
        state
    }

    /// D18 Docker: an export read back by the bulk import into another schema: every table created from the
    /// manifest's types, a JSON lines file beside them with types read from its values, and the same rows.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs the Docker test servers"]
    async fn l3_an_export_imports_into_new_tables_with_the_same_rows() {
        let psql = crate::sql::copy::docker_tests::psql;
        psql("shop", "drop schema if exists d18a cascade; drop schema if exists d18b cascade; create schema d18a; create schema d18b;
            create table d18a.people (id int primary key, name text, born date, score numeric(6,2));
            insert into d18a.people select g, 'p' || g, date '2000-01-01' + g, g / 4.0 from generate_series(1, 300) g;
            create table d18a.tags (id bigint primary key, label varchar(40));
            insert into d18a.tags values (1, 'a, quoted'), (2, 'line
            break');");
        let state = state().await;
        let conn = open_db(
            &state,
            crate::dbs::Engine::Postgres,
            15432,
            "shop",
            "app",
            "test-pg",
        )
        .await;
        let dir = std::env::temp_dir().join(format!("dalan-bulk-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let req = BulkExport {
            objects: vec![
                BulkObject {
                    schema: "d18a".into(),
                    name: "people".into(),
                },
                BulkObject {
                    schema: "d18a".into(),
                    name: "tags".into(),
                },
            ],
            format: ExportFormat::Csv,
            header: true,
            masks: vec![],
            ddl: false,
            zip: false,
        };
        let stop = std::sync::atomic::AtomicBool::new(false);
        export_into(&state, &conn, &req, Sink::Dir(dir.clone()), &stop, &|_| {})
            .await
            .unwrap();
        std::fs::write(dir.join("events.jsonl"), "{\"id\": 1, \"at\": \"2026-01-02T10:00:00\", \"ok\": true}\n{\"id\": 2, \"at\": \"2026-01-03T11:00:00\", \"ok\": false}\n").unwrap();
        let (tables, default_schema, my) = tables_here(&state, &conn).await.unwrap();
        let listed = bulk_import_list(
            &state,
            &conn,
            dir.clone(),
            false,
            tables,
            default_schema,
            my,
        )
        .await
        .unwrap();
        assert!(listed.from.is_some(), "the manifest names the source");
        let mut items = Vec::new();
        for f in &listed.files {
            let table = f.table.clone();
            let inferred = bulk_infer_in(
                &state,
                conn.clone(),
                f.id.clone(),
                f.delimiter.clone(),
                true,
                "d18b".into(),
                table.clone(),
            )
            .await
            .unwrap();
            if table == "people" {
                assert!(inferred.from_manifest, "the export's own types");
                assert!(
                    inferred.ddl.contains("score numeric(6,2)")
                        && inferred.ddl.contains("PRIMARY KEY (id)"),
                    "{}",
                    inferred.ddl
                );
            }
            if table == "events" {
                assert!(!inferred.from_manifest);
                assert!(
                    inferred.ddl.contains("at timestamp") && inferred.ddl.contains("ok boolean"),
                    "{}",
                    inferred.ddl
                );
            }
            items.push(BulkItem {
                file_id: f.id.clone(),
                name: f.name.clone(),
                schema: "d18b".into(),
                table,
                delimiter_name: f.delimiter.clone(),
                header: true,
                mapping: inferred
                    .columns
                    .iter()
                    .map(|c| Some(c.name.clone()))
                    .collect(),
                create: Some(inferred.ddl),
            });
        }
        assert_eq!(
            items.len(),
            3,
            "{:?}",
            listed.files.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let pending = bulk_import_prepare_in(&state, conn.clone(), items)
            .await
            .unwrap();
        assert_eq!(pending.creates, 3.0);
        let report = bulk_import_commit_in(&state, conn.clone(), pending.id, &|_| {})
            .await
            .unwrap();
        assert!(report.iter().all(|r| r.error.is_none()), "{report:?}");
        assert_eq!(psql("shop", "select count(*) from d18b.people"), "300");
        assert_eq!(psql("shop", "select count(*) from d18a.people a join d18b.people b using (id, name, born, score)"), "300", "the values came through");
        assert_eq!(
            psql("shop", "select label from d18b.tags where id = 2"),
            "line\n            break"
        );
        assert_eq!(
            psql("shop", "select count(*) from d18b.events where ok"),
            "1"
        );
        let _ = std::fs::remove_dir_all(&dir);
        psql(
            "shop",
            "drop schema d18a cascade; drop schema d18b cascade;",
        );
    }

    /// D17 Docker: two tables of the test PostgreSQL into a folder (masked emails, DDL, manifest), and into a zip.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs the Docker test servers"]
    async fn l3_many_tables_export_to_a_folder_and_a_zip_with_a_manifest() {
        crate::sql::copy::docker_tests::psql("shop", "drop schema if exists d17e cascade; create schema d17e;
            create table d17e.people (id int primary key, name text, email text);
            insert into d17e.people select g, 'p' || g, 'p' || g || '@real.example' from generate_series(1, 120) g;
            create table d17e.notes (id int primary key, person int references d17e.people(id), body text);
            insert into d17e.notes values (1, 1, 'hello'), (2, 2, 'line\nbreak');");
        let state = state().await;
        let conn = open_db(
            &state,
            crate::dbs::Engine::Postgres,
            15432,
            "shop",
            "app",
            "test-pg",
        )
        .await;
        let dir = std::env::temp_dir().join(format!("dalan-bulk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let req = BulkExport {
            objects: vec![
                BulkObject {
                    schema: "d17e".into(),
                    name: "people".into(),
                },
                BulkObject {
                    schema: "d17e".into(),
                    name: "notes".into(),
                },
            ],
            format: ExportFormat::Csv,
            header: true,
            masks: vec![MaskRule {
                column: "email".into(),
                kind: crate::dbs::masking::MaskKind::FakeEmail,
                value: None,
            }],
            ddl: true,
            zip: false,
        };
        let stop = std::sync::atomic::AtomicBool::new(false);
        let (files, rows, _) =
            export_into(&state, &conn, &req, Sink::Dir(dir.clone()), &stop, &|_| {})
                .await
                .unwrap();
        let m0: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!((files, rows), (2.0, 122.0), "{m0}");
        let people = std::fs::read_to_string(dir.join("d17e.people.csv")).unwrap();
        assert!(people.starts_with("id,name,email"), "{people}");
        assert!(
            !people.contains("@real.example"),
            "the emails weren't masked"
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["tables"][0]["rows"], 120.0);
        assert_eq!(manifest["tables"][0]["columns"][2]["name"], "email");
        assert_eq!(manifest["tables"][0]["primary_key"][0], "id");
        assert_eq!(manifest["masked_columns"][0], "email");
        let ddl = std::fs::read_to_string(dir.join("schema.sql")).unwrap();
        assert!(
            ddl.contains("CREATE TABLE people")
                && ddl.contains("FOREIGN KEY (person) REFERENCES people(id)"),
            "{ddl}"
        );

        let zip_path = dir.join("out.zip");
        let f = std::fs::File::create(&zip_path).unwrap();
        let req = BulkExport {
            format: ExportFormat::Json,
            zip: true,
            ddl: false,
            masks: vec![],
            ..req
        };
        export_into(
            &state,
            &conn,
            &req,
            Sink::Zip(Box::new(zip::ZipWriter::new(f))),
            &stop,
            &|_| {},
        )
        .await
        .unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["d17e.notes.json", "d17e.people.json", "manifest.json"]
        );
        let mut notes = String::new();
        std::io::Read::read_to_string(&mut z.by_name("d17e.notes.json").unwrap(), &mut notes)
            .unwrap();
        assert!(notes.contains("line\\nbreak"), "{notes}");
        let _ = std::fs::remove_dir_all(&dir);
        crate::sql::copy::docker_tests::psql("shop", "drop schema d17e cascade;");
    }
}

// ---- Bulk import (E6)

use crate::dbs::bulkfiles::{self, InferredColumn};
use crate::dbs::gate::{self, Decision};
use crate::dbs::schemadiff::Family;
use crate::sql::classify::Class;
use crate::sql::transfer::{read_limited, ImportPlan};

/// What a pick leaves for the import: unzipped and converted files, and the export's manifest.
pub struct BulkPicked {
    pub _tmp: crate::dbs::dumpfile::TempDir,
    pub manifest: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkFile {
    /// For `sql_import_preview` and the import (the path stays in Rust).
    pub id: String,
    pub name: String,
    /// `,` or `tab`.
    pub delimiter: String,
    /// JSON turned into CSV with a header.
    pub json: bool,
    /// The table it goes into, matched by name (the window can change it).
    pub schema: String,
    pub table: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkFiles {
    pub source: String,
    pub files: Vec<BulkFile>,
    /// Files that aren't CSV, TSV or JSON.
    pub skipped: Vec<String>,
    /// The folder or zip came from a Dalan export (its manifest names the source).
    pub from: Option<String>,
}

fn family_of(my: bool) -> Family {
    if my {
        Family::My
    } else {
        Family::Pg
    }
}

/// Every table of this connection, as `(schema, table)`, and the schema new tables go into by default.
async fn tables_here(
    state: &AppState,
    conn_id: &str,
) -> AppResult<(Vec<(String, String)>, String, bool)> {
    let raw = state.sql.connect_to(conn_id, None).await?;
    let (sql, my) = match &raw {
        crate::sql::RawConn::My(_) => ("select table_schema, table_name from information_schema.tables where table_type = 'BASE TABLE' and table_schema not in ('mysql','sys','performance_schema','information_schema')", true),
        _ => ("select table_schema::text, table_name::text from information_schema.tables where table_type = 'BASE TABLE' and table_schema not in ('pg_catalog','information_schema')", false),
    };
    let rows: Vec<(String, String)> = match &raw {
        crate::sql::RawConn::My(m) => m
            .query(sql, 0)
            .await?
            .rows
            .into_iter()
            .filter_map(|r| Some((r.first().cloned().flatten()?, r.get(1).cloned().flatten()?)))
            .collect(),
        crate::sql::RawConn::Pg(_) => crate::sql::datacmp::text_rows_pub(&raw, sql)
            .await?
            .into_iter()
            .filter_map(|r| Some((r.first().cloned().flatten()?, r.get(1).cloned().flatten()?)))
            .collect(),
        crate::sql::RawConn::Mongo(_) => {
            return Err(AppError::validation(
                "Bulk import is for SQL databases; MongoDB has its own import.",
            ))
        }
    };
    let cfg = crate::sql::load(&state.store(), &state.sql.db_id(conn_id)?).await?;
    let default = if my {
        cfg.database.unwrap_or_default()
    } else {
        "public".to_string()
    };
    Ok((rows, default, my))
}

/// The folder (or zip) dialog, then every CSV, TSV and JSON file in it, each matched to a table by its name.
#[tauri::command]
#[specta::specta]
pub async fn bulk_import_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    zip: bool,
) -> AppResult<Option<BulkFiles>> {
    state.require_unlocked()?;
    let (tables, default_schema, my) = tables_here(&state, &conn_id).await?;
    let a = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        let d = a.dialog().file().set_title(if zip {
            "Import a zip of CSV and JSON files"
        } else {
            "Import a folder of CSV and JSON files"
        });
        if zip {
            crate::commands::sql::pick_or("open", || {
                d.add_filter("Zip", &["zip"]).blocking_pick_file()
            })
        } else {
            crate::commands::sql::pick_or("folder", || d.blocking_pick_folder())
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(p) = picked else { return Ok(None) };
    let chosen = p
        .into_path()
        .map_err(|_| AppError::validation("That location can't be used."))?;
    bulk_import_list(&state, &conn_id, chosen, zip, tables, default_schema, my)
        .await
        .map(Some)
}

/// Lists a folder (or a zip, unpacked into a private folder) for the import: every CSV, TSV and JSON file, JSON
/// turned into CSV, each matched to a table by its name.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn bulk_import_list(
    state: &AppState,
    conn_id: &str,
    chosen: std::path::PathBuf,
    zip: bool,
    tables: Vec<(String, String)>,
    default_schema: String,
    my: bool,
) -> AppResult<BulkFiles> {
    let conn_id = conn_id.to_string();
    let tmp = crate::dbs::dumpfile::TempDir::new("bulk")?;
    // The files: those of the folder, or those unpacked from the zip (top level only, safe names).
    let dir = if zip {
        let t = tmp.0.clone();
        let src = chosen.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            let mut z = zip::ZipArchive::new(
                std::fs::File::open(&src).map_err(|e| AppError::validation(e.to_string()))?,
            )
            .map_err(|e| AppError::validation(format!("That isn't a zip: {e}")))?;
            for i in 0..z.len() {
                let mut f = z
                    .by_index(i)
                    .map_err(|e| AppError::validation(e.to_string()))?;
                let Some(name) = f
                    .enclosed_name()
                    .and_then(|p| p.file_name().map(|n| n.to_owned()))
                else {
                    continue;
                };
                if f.is_dir() {
                    continue;
                }
                let mut out = std::fs::File::create(t.join(name))
                    .map_err(|e| AppError::internal(e.to_string()))?;
                std::io::copy(&mut f, &mut out)
                    .map_err(|e| AppError::validation(format!("Couldn't unpack the zip: {e}")))?;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        tmp.0.clone()
    } else {
        chosen.clone()
    };
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| AppError::validation(format!("Couldn't read that folder: {e}")))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    let manifest: Option<serde_json::Value> = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let (mut files, mut skipped) = (Vec::new(), Vec::new());
    for name in names {
        if name == "manifest.json" || name == "schema.sql" {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        let path = dir.join(&name);
        let (path, delimiter, json) = match ext {
            "csv" | "txt" => (path, ",", false),
            "tsv" => (path, "tab", false),
            "json" | "jsonl" | "ndjson" => {
                let p = path.clone();
                let text = tokio::task::spawn_blocking(move || read_limited(&p))
                    .await
                    .map_err(|e| AppError::internal(e.to_string()))??;
                match bulkfiles::json_to_csv(&text) {
                    Ok(csv) => {
                        let out = tmp.0.join(format!("{name}.csv"));
                        std::fs::write(&out, csv).map_err(|e| AppError::internal(e.to_string()))?;
                        (out, ",", true)
                    }
                    Err(e) => {
                        skipped.push(format!("{name} ({})", e.message));
                        continue;
                    }
                }
            }
            _ => {
                skipped.push(name);
                continue;
            }
        };
        let (schema, table, exists) = bulkfiles::match_table(&name, &tables, &default_schema);
        files.push(BulkFile {
            id: state.sql.put_file(&conn_id, path),
            name,
            delimiter: delimiter.into(),
            json,
            schema,
            table,
            exists,
        });
    }
    let from = manifest.as_ref().and_then(|m| {
        m["source"]["name"]
            .as_str()
            .map(|n| format!("{n} ({})", m["exported_at"].as_str().unwrap_or("")))
    });
    if let Ok(mut m) = state.sql.bulk_picked.lock() {
        m.insert(
            conn_id.clone(),
            BulkPicked {
                _tmp: tmp,
                manifest,
            },
        );
    }
    let _ = my;
    Ok(BulkFiles {
        source: chosen
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files,
        skipped,
        from,
    })
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct InferredTable {
    pub ddl: String,
    pub columns: Vec<InferredColumn>,
    /// The types came from the export's manifest (same engine), not from the values.
    pub from_manifest: bool,
}

fn delimiter_char(d: &str) -> char {
    if d == "tab" {
        '\t'
    } else {
        ','
    }
}

/// A CREATE TABLE for a file whose table isn't there: the export's own types when its manifest has them (same
/// engine), else types read from the values.
#[tauri::command]
#[specta::specta]
pub async fn bulk_infer(
    state: State<'_, AppState>,
    conn_id: String,
    file_id: String,
    delimiter_name: String,
    header: bool,
    schema: String,
    table: String,
) -> AppResult<InferredTable> {
    bulk_infer_in(
        &state,
        conn_id,
        file_id,
        delimiter_name,
        header,
        schema,
        table,
    )
    .await
}

pub(crate) async fn bulk_infer_in(
    state: &AppState,
    conn_id: String,
    file_id: String,
    delimiter_name: String,
    header: bool,
    schema: String,
    table: String,
) -> AppResult<InferredTable> {
    state.require_unlocked()?;
    let my = state.sql.my(&conn_id).await?.is_some();
    let f = family_of(my);
    let path = state.sql.file(&conn_id, &file_id)?;
    let text = tokio::task::spawn_blocking(move || read_limited(&path))
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
    let records = crate::sql::transfer::parse_csv(&text, delimiter_char(&delimiter_name));
    let first = records.first().cloned().unwrap_or_default();
    let names: Vec<String> = if header {
        first.clone()
    } else {
        (1..=first.len()).map(|i| format!("column_{i}")).collect()
    };
    let body = if header {
        &records[1.min(records.len())..]
    } else {
        &records[..]
    };
    // The manifest's columns, for a table of the same name, exported from the same engine.
    let manifest = state
        .sql
        .bulk_picked
        .lock()
        .ok()
        .and_then(|m| m.get(&conn_id).and_then(|p| p.manifest.clone()));
    let from = manifest.as_ref().and_then(|m| {
        let engine = m["source"]["engine"].as_str().unwrap_or("");
        let same = if my {
            engine == "MySQL" || engine == "MariaDB"
        } else {
            engine == "PostgreSQL"
        };
        if !same {
            return None;
        }
        m["tables"]
            .as_array()?
            .iter()
            .find(|t| t["name"].as_str() == Some(table.as_str()))
            .cloned()
    });
    let (columns, pk, from_manifest) =
        match from.as_ref().and_then(|t| t["columns"].as_array().cloned()) {
            Some(cols) if !cols.is_empty() => (
                cols.iter()
                    .map(|c| InferredColumn {
                        name: c["name"].as_str().unwrap_or("").into(),
                        type_name: c["type"].as_str().unwrap_or("text").into(),
                    })
                    .collect::<Vec<_>>(),
                from.as_ref()
                    .and_then(|t| {
                        t["primary_key"].as_array().map(|a| {
                            a.iter()
                                .filter_map(|k| k.as_str().map(String::from))
                                .collect()
                        })
                    })
                    .unwrap_or_default(),
                true,
            ),
            _ => (bulkfiles::infer(f, &names, body), Vec::new(), false),
        };
    Ok(InferredTable {
        ddl: bulkfiles::create_ddl(f, &schema, &table, &columns, &pk),
        columns,
        from_manifest,
    })
}

#[derive(Debug, Clone, Deserialize, Type)]
pub struct BulkItem {
    pub file_id: String,
    pub name: String,
    pub schema: String,
    pub table: String,
    pub delimiter_name: String,
    pub header: bool,
    /// For each file column, the table column it goes into (`None`: skipped).
    pub mapping: Vec<Option<String>>,
    /// The reviewed CREATE TABLE, for a table that isn't there yet.
    pub create: Option<String>,
}

/// One file of a held bulk import: its plan when the table is there, or its CREATE TABLE to run first.
pub struct BulkHeld {
    pub item: BulkItem,
    pub plan: Option<ImportPlan>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkImportPending {
    pub id: String,
    pub db_name: String,
    pub environment: String,
    pub files: f64,
    pub creates: f64,
    pub rows: f64,
}

async fn plan_for(state: &AppState, conn_id: &str, it: &BulkItem) -> AppResult<ImportPlan> {
    let path = state.sql.file(conn_id, &it.file_id)?;
    let text = tokio::task::spawn_blocking(move || read_limited(&path))
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
    let d = delimiter_char(&it.delimiter_name);
    if let Some(my) = state.sql.my(conn_id).await? {
        my.import_plan(&it.schema, &it.table, &text, d, it.header, &it.mapping)
            .await
    } else {
        state
            .sql
            .pg(conn_id)
            .await?
            .import_plan(&it.schema, &it.table, &text, d, it.header, &it.mapping)
            .await
    }
}

/// Checks every file (mappings against their tables, the CREATE TABLEs) and holds the run for the confirmation.
#[tauri::command]
#[specta::specta]
pub async fn bulk_import_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    items: Vec<BulkItem>,
) -> AppResult<BulkImportPending> {
    bulk_import_prepare_in(&state, conn_id, items).await
}

pub(crate) async fn bulk_import_prepare_in(
    state: &AppState,
    conn_id: String,
    items: Vec<BulkItem>,
) -> AppResult<BulkImportPending> {
    state.require_unlocked()?;
    if items.is_empty() {
        return Err(AppError::validation("Choose at least one file."));
    }
    let db = crate::sql::gate_target(&state.store(), &state.sql.db_id(&conn_id)?).await?;
    let my = state.sql.my(&conn_id).await?.is_some();
    let creates = items.iter().filter(|i| i.create.is_some()).count();
    let class = if creates > 0 {
        Class::Schema
    } else {
        Class::Write
    };
    if gate::decide(db.mode, class) == Decision::Refuse {
        return Err(gate::read_only_error(&db.name));
    }
    let mut held = Vec::new();
    let mut rows = 0u64;
    for it in items {
        let plan = match &it.create {
            Some(ddl) => {
                bulkfiles::check_create(family_of(my), ddl, &it.table)?;
                let path = state.sql.file(&conn_id, &it.file_id)?;
                let text = tokio::task::spawn_blocking(move || read_limited(&path))
                    .await
                    .map_err(|e| AppError::internal(e.to_string()))??;
                rows += crate::sql::transfer::parse_csv(&text, delimiter_char(&it.delimiter_name))
                    .len()
                    .saturating_sub(usize::from(it.header)) as u64;
                None
            }
            None => {
                let p = plan_for(state, &conn_id, &it)
                    .await
                    .map_err(|e| AppError::new(e.code, format!("{}: {}", it.name, e.message)))?;
                rows += p.rows;
                Some(p)
            }
        };
        held.push(BulkHeld { item: it, plan });
    }
    let files = held.len() as f64;
    let id = state.sql.bulk_pending.put(&conn_id, class, held)?;
    Ok(BulkImportPending {
        id,
        db_name: db.name,
        environment: db.environment,
        files,
        creates: creates as f64,
        rows: rows as f64,
    })
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct BulkFileReport {
    pub name: String,
    pub table: String,
    pub rows: f64,
    pub created: bool,
    pub error: Option<String>,
}

/// Runs the held import: each CREATE TABLE, then each file in its own transaction (one failing doesn't stop the
/// others; the report says which). Progress per file on `progress`.
#[tauri::command]
#[specta::specta]
pub async fn bulk_import_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    progress: Channel<BulkProgress>,
) -> AppResult<Vec<BulkFileReport>> {
    bulk_import_commit_in(&state, conn_id, pending_id, &|p| {
        let _ = progress.send(p);
    })
    .await
}

pub(crate) async fn bulk_import_commit_in(
    state: &AppState,
    conn_id: String,
    pending_id: String,
    progress: &(dyn Fn(BulkProgress) + Sync),
) -> AppResult<Vec<BulkFileReport>> {
    state.require_unlocked()?;
    let class = state.sql.bulk_pending.peek_class(&conn_id, &pending_id)?;
    let db = crate::sql::gate_target(&state.store(), &state.sql.db_id(&conn_id)?).await?;
    gate::approve(db.mode, class, &db.name, None)?;
    let (held, _) = state.sql.bulk_pending.take(&conn_id, &pending_id)?;
    let total = held.len() as f64;
    let mut out = Vec::new();
    for (i, h) in held.into_iter().enumerate() {
        let table = format!("{}.{}", h.item.schema, h.item.table);
        progress(BulkProgress {
            index: i as f64,
            total,
            table: table.clone(),
            rows: 0.0,
        });
        let created = h.item.create.is_some();
        let run = async {
            let plan = match (&h.item.create, h.plan) {
                (Some(ddl), _) => {
                    let stmts = [ddl.clone()];
                    if let Some(my) = state.sql.my(&conn_id).await? {
                        my.structure_commit(&stmts).await?;
                    } else {
                        state
                            .sql
                            .pg(&conn_id)
                            .await?
                            .structure_commit(&stmts)
                            .await?;
                    }
                    plan_for(state, &conn_id, &h.item).await?
                }
                (None, Some(p)) => p,
                (None, None) => return Err(AppError::internal("no plan")),
            };
            if let Some(my) = state.sql.my(&conn_id).await? {
                my.import_commit(&plan).await
            } else {
                state.sql.pg(&conn_id).await?.import_commit(&plan).await
            }
        }
        .await;
        out.push(match run {
            Ok(n) => BulkFileReport {
                name: h.item.name.clone(),
                table,
                rows: n as f64,
                created,
                error: None,
            },
            Err(e) => BulkFileReport {
                name: h.item.name.clone(),
                table,
                rows: 0.0,
                created,
                error: Some(e.message),
            },
        });
    }
    Ok(out)
}

#[tauri::command]
#[specta::specta]
pub fn bulk_import_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) {
    let _ = state.sql.bulk_pending.take(&conn_id, &pending_id);
}

// ---- MongoDB import (E6)

use crate::mongo::import::{self as mgimport, MgFormat, MgImportMode, MgImportReport};

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgImportPreview {
    pub format: MgFormat,
    pub docs: f64,
    /// The first five documents as relaxed extended JSON.
    pub sample: Vec<String>,
    /// Top-level fields of those documents (for "upsert by").
    pub fields: Vec<String>,
}

/// What is held for the confirmation: the place, the file and the mode (the file is read again when it runs).
pub struct MgImportJob {
    pub db: String,
    pub coll: String,
    pub file_id: String,
    pub mode: MgImportMode,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MgImportPending {
    pub id: String,
    /// `confirm`, or `typed` for a replace.
    pub level: String,
    pub typed_name: String,
    pub environment: String,
    pub docs: f64,
}

async fn mg_docs(
    state: &AppState,
    conn_id: &str,
    file_id: &str,
) -> AppResult<(MgFormat, Vec<bson::Document>)> {
    let path = state.sql.file(conn_id, file_id)?;
    tokio::task::spawn_blocking(move || mgimport::parse(&read_limited(&path)?))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

/// The open-file dialog for a MongoDB import (JSON lines, a JSON array, or CSV).
#[tauri::command]
#[specta::specta]
pub async fn mg_import_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
) -> AppResult<Option<super::sql::CsvFile>> {
    state.require_unlocked()?;
    state.sql.mongo(&conn_id).await?;
    Ok(super::sql::pick_open(&app, "Import into a collection")
        .await?
        .map(|p| super::sql::CsvFile {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            id: state.sql.put_file(&conn_id, p),
        }))
}

#[tauri::command]
#[specta::specta]
pub async fn mg_import_preview(
    state: State<'_, AppState>,
    conn_id: String,
    file_id: String,
) -> AppResult<MgImportPreview> {
    state.require_unlocked()?;
    let (format, docs) = mg_docs(&state, &conn_id, &file_id).await?;
    let mut fields: Vec<String> = Vec::new();
    for d in docs.iter().take(50) {
        for k in d.keys() {
            if !fields.contains(k) {
                fields.push(k.clone());
            }
        }
    }
    Ok(MgImportPreview {
        format,
        docs: docs.len() as f64,
        sample: docs
            .iter()
            .take(5)
            .map(crate::mongo::docs::relaxed)
            .collect(),
        fields,
    })
}

/// Reads the file (any error is said now) and holds the import: insert and upsert are writes, replace removes every
/// document first, so it asks for the typed name.
#[tauri::command]
#[specta::specta]
pub async fn mg_import_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    db: String,
    coll: String,
    file_id: String,
    mode: MgImportMode,
) -> AppResult<MgImportPending> {
    state.require_unlocked()?;
    let saved = crate::sql::gate_target(&state.store(), &state.sql.db_id(&conn_id)?).await?;
    if coll.trim().is_empty() {
        return Err(AppError::validation("Name the collection."));
    }
    if let MgImportMode::Upsert { field } = &mode {
        if field.trim().is_empty() {
            return Err(AppError::validation("Name the field to upsert by."));
        }
    }
    let (_, docs) = mg_docs(&state, &conn_id, &file_id).await?;
    let class = if matches!(mode, MgImportMode::Replace) {
        Class::Dangerous
    } else {
        Class::Write
    };
    let decision = gate::decide(saved.mode, class);
    if decision == Decision::Refuse {
        return Err(gate::read_only_error(&saved.name));
    }
    let id = state.sql.mg_import_pending.put(
        &conn_id,
        class,
        MgImportJob {
            db,
            coll,
            file_id,
            mode,
        },
    )?;
    Ok(MgImportPending {
        id,
        level: if decision == Decision::Typed {
            "typed"
        } else {
            "confirm"
        }
        .into(),
        typed_name: saved.name,
        environment: saved.environment,
        docs: docs.len() as f64,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn mg_import_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    progress: Channel<BulkProgress>,
) -> AppResult<MgImportReport> {
    state.require_unlocked()?;
    let class = state
        .sql
        .mg_import_pending
        .peek_class(&conn_id, &pending_id)?;
    let saved = crate::sql::gate_target(&state.store(), &state.sql.db_id(&conn_id)?).await?;
    gate::approve(saved.mode, class, &saved.name, typed_name.as_deref())?;
    let (job, _) = state.sql.mg_import_pending.take(&conn_id, &pending_id)?;
    let (_, docs) = mg_docs(&state, &conn_id, &job.file_id).await?;
    let total = docs.len() as f64;
    let conn = state.sql.mongo(&conn_id).await?;
    let table = format!("{}.{}", job.db, job.coll);
    mgimport::write(&conn, &job.db, &job.coll, docs, &job.mode, &|done| {
        let _ = progress.send(BulkProgress {
            index: 0.0,
            total,
            table: table.clone(),
            rows: done,
        });
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn mg_import_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) {
    let _ = state.sql.mg_import_pending.take(&conn_id, &pending_id);
}
