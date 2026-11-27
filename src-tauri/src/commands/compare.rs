//! Schema compare and sync (D16, E3). Each side is a schema of an open database tab or a saved snapshot. The
//! comparison is read fresh every time, also for "Sync": the held script is built from what the two sides are at
//! that moment, then runs through the target's gate (schema class; a drop or a type change makes it dangerous, so
//! the typed name is asked). "Copy as migration…" writes the same script to a file chosen in the system dialog.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::dbs::gate;
use crate::dbs::schemadiff::{self, Catalog, DiffItem, Family, Statement, TableDdl};
use crate::dbs::snapshots::{self, SnapshotRow};
use crate::error::{AppError, AppResult};
use crate::redis::store::RedisWriteMode;
use crate::sql::schemacat::{self, SyncReport};
use crate::sql::{self, classify::Class};
use crate::state::AppState;

/// One side of a compare.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Side {
    Database {
        conn_id: String,
        schema: String,
    },
    Snapshot {
        id: String,
    },
    /// A dump picked in a tab (`restore_pick`), read through a scratch database on that tab's server.
    Dump {
        conn_id: String,
        dump_id: String,
        schema: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SchemaCompare {
    pub family: Family,
    pub source_label: String,
    pub target_label: String,
    pub items: Vec<DiffItem>,
    pub tables: Vec<TableDdl>,
    /// The target is a live database (a snapshot can't be changed).
    pub can_sync: bool,
}

/// What is held for "Sync": the target schema and its statements.
pub struct SyncJob {
    pub schema: String,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SyncPlan {
    pub pending_id: String,
    /// `confirm` or `typed`.
    pub level: String,
    pub typed_name: String,
    pub environment: String,
    pub script: String,
    pub statements: f64,
    pub dangerous: f64,
}

fn when(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

async fn side(state: &AppState, s: &Side) -> AppResult<(Catalog, String)> {
    match s {
        Side::Database { conn_id, schema } => {
            let db = sql::gate_target(&state.store(), &state.sql.db_id(conn_id)?).await?;
            let raw = state.sql.connect_to(conn_id, None).await?;
            let cat = schemacat::read(&raw, schema).await?;
            Ok((cat, format!("{} · {schema}", db.name)))
        }
        Side::Dump {
            conn_id,
            dump_id,
            schema,
        } => {
            let db_id = state.sql.db_id(conn_id)?;
            let db = sql::gate_target(&state.store(), &db_id).await?;
            let dump = state.sql.dump(conn_id, dump_id)?;
            let cfg = sql::load(&state.store(), &db_id).await?;
            let ssh = state.sql.ssh(conn_id).await?;
            let owned = super::restore::Owned::get(&state.sql, conn_id, cfg.engine).await?;
            let name = dump.info.file_name.clone();
            let cat = schemacat::from_dump(
                owned.conn(),
                cfg,
                ssh,
                dump,
                dump_id,
                schemacat::Scratch {
                    name: &db.name,
                    environment: &db.environment,
                    mode: db.mode,
                },
                schema.as_deref(),
                async |scratch| state.sql.connect_to(conn_id, Some(scratch)).await,
            )
            .await?;
            let label = format!("{name} · {}", cat.schema);
            Ok((cat, label))
        }
        Side::Snapshot { id } => {
            let id = id.clone();
            let (cat, label, at) = state
                .with_db_session(move |c, s| {
                    let (cat, label) = snapshots::load(c, s, &id)?;
                    let at = snapshots::list(c, s)?
                        .into_iter()
                        .find(|x| x.id == id)
                        .map(|x| x.captured_at as i64)
                        .unwrap_or(0);
                    Ok((cat, label, at))
                })
                .await?;
            let name = label.split(" · ").next().unwrap_or("").to_string();
            let label = format!("{name} · {} · snapshot of {}", cat_schema(&cat), when(at));
            Ok((cat, label))
        }
    }
}

fn cat_schema(c: &Catalog) -> &str {
    &c.schema
}

async fn both(
    state: &AppState,
    source: &Side,
    target: &Side,
) -> AppResult<(Catalog, String, Catalog, String)> {
    let (s, sl) = side(state, source).await?;
    let (t, tl) = side(state, target).await?;
    if s.family != t.family {
        return Err(AppError::validation(
            "Schemas compare within one engine: PostgreSQL with PostgreSQL, MySQL or MariaDB with MySQL or MariaDB.",
        ));
    }
    Ok((s, sl, t, tl))
}

#[tauri::command]
#[specta::specta]
pub async fn schema_compare(
    state: State<'_, AppState>,
    source: Side,
    target: Side,
) -> AppResult<SchemaCompare> {
    state.require_unlocked()?;
    let (s, sl, t, tl) = both(&state, &source, &target).await?;
    let (items, tables) = schemadiff::diff(&s, &t);
    Ok(SchemaCompare {
        family: s.family,
        source_label: sl,
        target_label: tl,
        items,
        tables,
        can_sync: matches!(target, Side::Database { .. }),
    })
}

async fn build(
    state: &AppState,
    source: &Side,
    target: &Side,
    chosen: &[String],
) -> AppResult<(Vec<Statement>, String, Family, String)> {
    let (s, sl, t, tl) = both(state, source, target).await?;
    let (items, _) = schemadiff::diff(&s, &t);
    let chosen: HashSet<String> = chosen.iter().cloned().collect();
    let stmts = schemadiff::script(&s, &t, &items, &chosen);
    if stmts.is_empty() {
        return Err(AppError::validation(
            "Nothing to do: the chosen differences are gone (the schemas changed since the compare). Compare again.",
        ));
    }
    let text = schemadiff::script_text(
        s.family,
        &t.schema,
        &format!("Make {tl} match {sl}"),
        &stmts,
    );
    Ok((stmts, text, s.family, t.schema))
}

/// The script for the chosen differences, for the editor ("Open in a query tab").
#[tauri::command]
#[specta::specta]
pub async fn schema_sync_script(
    state: State<'_, AppState>,
    source: Side,
    target: Side,
    chosen: Vec<String>,
) -> AppResult<String> {
    state.require_unlocked()?;
    Ok(build(&state, &source, &target, &chosen).await?.1)
}

/// Builds the script fresh and holds it for the target's confirmation.
#[tauri::command]
#[specta::specta]
pub async fn schema_sync_prepare(
    state: State<'_, AppState>,
    source: Side,
    target: Side,
    chosen: Vec<String>,
) -> AppResult<SyncPlan> {
    state.require_unlocked()?;
    let Side::Database { conn_id, .. } = &target else {
        return Err(AppError::validation(
            "A snapshot can't be changed: pick a database as the target.",
        ));
    };
    let (stmts, script, _, schema) = build(&state, &source, &target, &chosen).await?;
    let db = sql::gate_target(&state.store(), &state.sql.db_id(conn_id)?).await?;
    if db.environment == "production" && db.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&db.name));
    }
    let dangerous = stmts.iter().filter(|s| s.dangerous).count();
    let class = if dangerous > 0 {
        Class::Dangerous
    } else {
        Class::Schema
    };
    let decision = gate::decide(db.mode, class);
    if decision == gate::Decision::Refuse {
        return Err(gate::read_only_error(&db.name));
    }
    let count = stmts
        .iter()
        .filter(|s| !s.sql.trim_start().starts_with("--"))
        .count();
    let id = state.sql.compare_pending.put(
        conn_id,
        class,
        SyncJob {
            schema,
            statements: stmts,
        },
    )?;
    Ok(SyncPlan {
        pending_id: id,
        level: if decision == gate::Decision::Typed {
            "typed"
        } else {
            "confirm"
        }
        .into(),
        typed_name: db.name,
        environment: db.environment,
        script,
        statements: count as f64,
        dangerous: dangerous as f64,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn schema_sync_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
) -> AppResult<SyncReport> {
    state.require_unlocked()?;
    let class = state
        .sql
        .compare_pending
        .peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let db = sql::gate_target(&state.store(), &db_id).await?;
    if db.environment == "production" && db.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&db.name));
    }
    gate::approve(db.mode, class, &db.name, typed_name.as_deref())?;
    let (job, _) = state.sql.compare_pending.take(&conn_id, &pending_id)?;
    let raw = state.sql.connect_to(&conn_id, None).await?;
    let rep = schemacat::apply(&raw, &job.schema, &job.statements).await?;
    let text = format!(
        "schema sync of {}: {} of {} statements{}",
        job.schema,
        rep.ran,
        rep.total,
        if rep.failed.is_some() { ", failed" } else { "" }
    );
    let _ = state
        .store()
        .with_db_session(move |c, s| crate::dbs::history::remember(c, s, &db_id, "sync", &text))
        .await;
    Ok(rep)
}

#[tauri::command]
#[specta::specta]
pub fn schema_sync_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) {
    let _ = state.sql.compare_pending.take(&conn_id, &pending_id);
}

/// "Copy as migration…": the script into a file chosen in the system dialog. Returns the file's name, `None` when
/// the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn schema_script_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source: Side,
    target: Side,
    chosen: Vec<String>,
) -> AppResult<Option<String>> {
    state.require_unlocked()?;
    let (_, text, _, schema) = build(&state, &source, &target, &chosen).await?;
    let name = format!(
        "migrate-{}-{}.sql",
        schemadiff_safe(&schema),
        chrono::Local::now().format("%Y-%m-%d-%H%M")
    );
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Save the migration")
            .set_file_name(&name)
            .add_filter("SQL", &["sql"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(p) = picked else { return Ok(None) };
    let path = p
        .into_path()
        .map_err(|_| AppError::validation("That location can't be used."))?;
    std::fs::write(&path, text)
        .map_err(|e| AppError::validation(format!("Couldn't write the file: {e}")))?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}

fn schemadiff_safe(s: &str) -> String {
    crate::dbs::schedule::safe(s)
}

/// Saves the schema of an open database as a snapshot to compare against later.
#[tauri::command]
#[specta::specta]
pub async fn schema_snapshot_save(
    state: State<'_, AppState>,
    conn_id: String,
    schema: String,
) -> AppResult<SnapshotRow> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let raw = state.sql.connect_to(&conn_id, None).await?;
    let cat = schemacat::read(&raw, &schema).await?;
    state
        .with_db_session(move |c, s| snapshots::save(c, s, &db_id, &cat))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn schema_snapshots(state: State<'_, AppState>) -> AppResult<Vec<SnapshotRow>> {
    state.require_unlocked()?;
    state.with_db_session(|c, s| snapshots::list(c, s)).await
}

#[tauri::command]
#[specta::specta]
pub async fn schema_snapshot_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .with_db_session(move |c, s| snapshots::delete(c, s, &id))
        .await
}

// ---- Data compare (E4)

use crate::sql::datacmp::{self, DataCompareResult, DataProgress, DataSide, MatchReport};
use tauri::ipc::Channel;

#[derive(Debug, Clone, Serialize, Type)]
pub struct DataColumn {
    pub name: String,
    pub kind: crate::dbs::datadiff::ValueKind,
    pub in_target: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct DataColumns {
    pub columns: Vec<DataColumn>,
    pub primary_key: Vec<String>,
}

/// The source table's columns (and which the target has too), and its primary key: the default key.
#[tauri::command]
#[specta::specta]
pub async fn data_columns(
    state: State<'_, AppState>,
    source: DataSide,
    target: DataSide,
) -> AppResult<DataColumns> {
    state.require_unlocked()?;
    let src = state.sql.connect_to(&source.conn_id, None).await?;
    let dst = state.sql.connect_to(&target.conn_id, None).await?;
    let (sc, pk) = datacmp::columns(&src, &source.schema, &source.table).await?;
    let (tc, _) = datacmp::columns(&dst, &target.schema, &target.table).await?;
    Ok(DataColumns {
        columns: sc
            .into_iter()
            .map(|c| DataColumn {
                in_target: tc.iter().any(|t| t.name == c.name),
                kind: c.kind,
                name: c.name,
            })
            .collect(),
        primary_key: pk,
    })
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn data_compare(
    state: State<'_, AppState>,
    source: DataSide,
    target: DataSide,
    key: Vec<String>,
    quick: bool,
    transfer_id: String,
    progress: Channel<DataProgress>,
) -> AppResult<DataCompareResult> {
    state.require_unlocked()?;
    let src = state.sql.connect_to(&source.conn_id, None).await?;
    let dst = state.sql.connect_to(&target.conn_id, None).await?;
    let (sr, tr, skipped) = datacmp::readers(&src, &source, &dst, &target, &key).await?;
    let stop = state.sql.begin_transfer(&transfer_id);
    let res = datacmp::compare(&src, &sr, &dst, &tr, quick, &stop, &|p| {
        let _ = progress.send(p);
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    let mut res = res?;
    res.skipped = skipped;
    if quick && sr.family != tr.family {
        res.warnings
            .push("The quick pass works within one engine; every row was compared.".into());
    }
    Ok(res)
}

/// What is held for "Make target match": the compare's request (the differences are read again when it runs).
pub struct DataJob {
    pub source: DataSide,
    pub target: DataSide,
    pub key: Vec<String>,
    pub quick: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MatchPlan {
    pub pending_id: String,
    pub level: String,
    pub typed_name: String,
    pub environment: String,
}

/// Holds "Make target match" for the target's confirmation: a write; on production always typed (it can delete).
#[tauri::command]
#[specta::specta]
pub async fn data_match_prepare(
    state: State<'_, AppState>,
    source: DataSide,
    target: DataSide,
    key: Vec<String>,
    quick: bool,
) -> AppResult<MatchPlan> {
    state.require_unlocked()?;
    let db = sql::gate_target(&state.store(), &state.sql.db_id(&target.conn_id)?).await?;
    if db.environment == "production" && db.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&db.name));
    }
    let class = if db.environment == "production" {
        Class::Dangerous
    } else {
        Class::Write
    };
    let decision = gate::decide(db.mode, class);
    if decision == gate::Decision::Refuse {
        return Err(gate::read_only_error(&db.name));
    }
    let conn = target.conn_id.clone();
    let id = state.sql.data_pending.put(
        &conn,
        class,
        DataJob {
            source,
            target,
            key,
            quick,
        },
    )?;
    Ok(MatchPlan {
        pending_id: id,
        level: if decision == gate::Decision::Typed {
            "typed"
        } else {
            "confirm"
        }
        .into(),
        typed_name: db.name,
        environment: db.environment,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn data_match_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    transfer_id: String,
    progress: Channel<DataProgress>,
) -> AppResult<MatchReport> {
    state.require_unlocked()?;
    let class = state.sql.data_pending.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let db = sql::gate_target(&state.store(), &db_id).await?;
    if db.environment == "production" && db.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&db.name));
    }
    gate::approve(db.mode, class, &db.name, typed_name.as_deref())?;
    let (job, _) = state.sql.data_pending.take(&conn_id, &pending_id)?;
    let src = state.sql.connect_to(&job.source.conn_id, None).await?;
    let dst = state.sql.connect_to(&job.target.conn_id, None).await?;
    let (sr, tr, _) = datacmp::readers(&src, &job.source, &dst, &job.target, &job.key).await?;
    let stop = state.sql.begin_transfer(&transfer_id);
    let rep = datacmp::make_match(&src, &sr, &dst, &tr, job.quick, &stop, &|p| {
        let _ = progress.send(p);
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    let rep = rep?;
    let text = format!(
        "make {}.{} match: {} inserted, {} updated, {} deleted",
        job.target.schema, job.target.table, rep.inserted, rep.updated, rep.deleted
    );
    let _ = state
        .store()
        .with_db_session(move |c, s| {
            crate::dbs::history::remember(c, s, &db_id, "data sync", &text)
        })
        .await;
    Ok(rep)
}

#[tauri::command]
#[specta::specta]
pub fn data_match_discard(state: State<'_, AppState>, conn_id: String, pending_id: String) {
    let _ = state.sql.data_pending.take(&conn_id, &pending_id);
}

/// "Save as a script…": the inserts, updates and deletes into a file chosen in the system dialog.
#[tauri::command]
#[specta::specta]
pub async fn data_match_script(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source: DataSide,
    target: DataSide,
    key: Vec<String>,
    quick: bool,
) -> AppResult<Option<String>> {
    state.require_unlocked()?;
    let name = format!(
        "match-{}-{}.sql",
        schemadiff_safe(&target.table),
        chrono::Local::now().format("%Y-%m-%d-%H%M")
    );
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Save the statements")
            .set_file_name(&name)
            .add_filter("SQL", &["sql"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(p) = picked else { return Ok(None) };
    let path = p
        .into_path()
        .map_err(|_| AppError::validation("That location can't be used."))?;
    let src = state.sql.connect_to(&source.conn_id, None).await?;
    let dst = state.sql.connect_to(&target.conn_id, None).await?;
    let (sr, tr, _) = datacmp::readers(&src, &source, &dst, &target, &key).await?;
    let file = std::fs::File::create(&path)
        .map_err(|e| AppError::validation(format!("Couldn't create the file: {e}")))?;
    let mut out = std::io::BufWriter::new(file);
    let title = format!(
        "Make {}.{} match {}.{}",
        target.schema, target.table, source.schema, source.table
    );
    datacmp::write_script(&src, &sr, &dst, &tr, quick, &mut out, &title).await?;
    use std::io::Write;
    out.flush().map_err(|e| AppError::internal(e.to_string()))?;
    Ok(path.file_name().map(|n| n.to_string_lossy().into_owned()))
}
