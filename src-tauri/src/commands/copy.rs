//! Copy between databases (D13, S49, F32). Thin commands over `sql::copy`: plan and hold the copy through the
//! target's gate, then run it with progress on a `Channel`. Stop is `sql_transfer_stop` with the run's transfer id.
//! The source is the tab the copy starts from; the target is another open tab (or a new database on its server).

use tauri::ipc::Channel;
use tauri::State;

use super::restore::Owned;
use crate::dbs::gate;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::store::RedisWriteMode;
use crate::sql::copy::{
    self, CopyJob, CopyMode, CopyPlan, CopyProgress, CopyReport, CopyRequest, CopyTablePlan,
};
use crate::sql::restore;
use crate::sql::{self, classify::Class};
use crate::state::AppState;

/// Plans the copy and, unless something blocks it, holds it for the confirmation.
#[tauri::command]
#[specta::specta]
pub async fn copy_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    request: CopyRequest,
    job_id: Option<String>,
) -> AppResult<CopyPlan> {
    state.require_unlocked()?;
    let src_id = state.sql.db_id(&conn_id)?;
    let dst_id = state.sql.db_id(&request.target_conn_id)?;
    let src_cfg = sql::load(&state.store(), &src_id).await?;
    let dst_cfg = sql::load(&state.store(), &dst_id).await?;
    let source = sql::gate_target(&state.store(), &src_id).await?;
    let target = sql::gate_target(&state.store(), &dst_id).await?;
    if !copy::compatible(src_cfg.engine, dst_cfg.engine) {
        return Err(AppError::validation(format!(
            "{} and {} don't copy to each other: MongoDB copies go to MongoDB, and SQL copies between PostgreSQL, MySQL and MariaDB.",
            src_cfg.engine.label(),
            dst_cfg.engine.label()
        )));
    }
    let cross = copy::family(src_cfg.engine) != copy::family(dst_cfg.engine);
    if request.routines && cross {
        return Err(AppError::validation("Functions, procedures and triggers copy between databases of the same engine only. Between engines they are listed with their DDL in the report."));
    }
    if request.objects.is_empty() {
        return Err(AppError::validation("Choose at least one table to copy."));
    }
    if target.mode == RedisWriteMode::ReadOnly {
        return Err(gate::read_only_error(&target.name));
    }
    if target.environment == "production" && target.mode != RedisWriteMode::Open {
        return Err(AppError::new(ErrorCode::DbReadOnlyMode, format!("{} is a production database. To copy into it, switch it to Open first (that needs the master password).", target.name)));
    }
    let dst = Owned::get(&state.sql, &request.target_conn_id, dst_cfg.engine).await?;
    let new_db = request
        .new_database
        .clone()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let target_db = match &new_db {
        Some(n) => {
            restore::valid_new_name(dst_cfg.engine, n)?;
            if restore::database_exists(&dst.conn(), n).await? {
                return Err(AppError::validation(format!("A database named {n} already exists on that server. Pick another name, or copy into it from its own tab.")));
            }
            n.clone()
        }
        None => restore::current_db(&dst.conn(), &dst_cfg)
            .await?
            .ok_or_else(|| AppError::validation("The target connection isn't to one database. Copy into a new database, or set a default database for it."))?,
    };
    let have = if new_db.is_some() {
        Vec::new()
    } else {
        restore::existing_names(&dst.conn(), &target_db).await?
    };
    let have: Vec<String> = have.iter().map(|h| h.to_ascii_lowercase()).collect();
    let same_place = src_id == dst_id && new_db.is_none();
    let src_raw = state.sql.connect_to(&conn_id, None).await?;
    let est = copy::estimates(&src_raw, &request.objects).await;
    let mut tables = Vec::new();
    let (mut missing, mut there) = (Vec::new(), Vec::new());
    for (o, rows) in request.objects.iter().zip(est) {
        let to = copy::target_name(o);
        let key = if dst_cfg.engine == crate::dbs::Engine::Postgres {
            // Between engines a table lands in `public`.
            format!("{}.{to}", if cross { "public" } else { o.schema.as_str() })
        } else {
            to.clone()
        };
        let exists = have.contains(&key.to_ascii_lowercase());
        let names = if o.kind == "view" || o.kind == "matview" {
            Vec::new()
        } else {
            copy::column_names(&src_raw, o).await
        };
        // Between engines: each column's mapping (with the person's choices), and the DDL that will run.
        let (columns, ddl) = if cross && o.kind == "table" {
            let t = match &src_raw {
                crate::sql::RawConn::Pg(c) => {
                    crate::sql::copycross::describe_pg(c, &o.schema, &o.name).await?
                }
                crate::sql::RawConn::My(c) => {
                    crate::sql::copycross::describe_my(c, &o.schema, &o.name).await?
                }
                crate::sql::RawConn::Mongo(_) => unreachable!("checked above"),
            };
            let to_my = copy::family(dst_cfg.engine) == "mysql";
            let cols = t
                .cols
                .iter()
                .map(|c| {
                    let chosen = o
                        .types
                        .as_ref()
                        .and_then(|v| v.iter().find(|x| x.name == c.name))
                        .map(|x| x.target_type.clone())
                        .filter(|x| !x.trim().is_empty());
                    copy::CopyColumn {
                        name: c.name.clone(),
                        source_type: c.source_type.clone(),
                        lossy: if chosen.is_some() {
                            None
                        } else {
                            c.mapped.lossy.clone()
                        },
                        target_type: chosen.unwrap_or_else(|| c.mapped.target.clone()),
                    }
                })
                .collect();
            let ddl = match o.ddl.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                Some(d) => {
                    crate::sql::copycross::check_ddl(d, to_my)?;
                    d.to_string()
                }
                None => crate::sql::copycross::create_sql(&t, o, &to, "public", to_my).1,
            };
            (cols, Some(ddl))
        } else {
            (Vec::new(), None)
        };
        if same_place
            && to == o.name
            && (dst_cfg.engine == crate::dbs::Engine::Postgres || o.schema == target_db)
        {
            return Err(AppError::validation(format!("{} would be copied onto itself. Give it another name, or copy into another database.", o.name)));
        }
        if exists && request.mode == CopyMode::Create && request.content != copy::CopyContent::Data
        {
            there.push(key.clone());
        }
        if !exists
            && matches!(
                request.mode,
                CopyMode::Empty | CopyMode::Append | CopyMode::Upsert
            )
        {
            missing.push(key.clone());
        }
        tables.push(CopyTablePlan {
            source: if o.schema.is_empty() || dst_cfg.engine == crate::dbs::Engine::Mongodb {
                o.name.clone()
            } else {
                format!("{}.{}", o.schema, o.name)
            },
            target: key,
            kind: o.kind.clone(),
            exists,
            action: copy::action(request.mode, exists).into(),
            rows_estimate: rows,
            columns,
            ddl,
            suggested_masks: names
                .iter()
                .filter(|c| !o.masks.iter().flatten().any(|m| m.column == **c))
                .filter_map(|c| {
                    crate::dbs::masking::suggest(c).map(|kind| {
                        crate::dbs::masking::MaskSuggestion {
                            table: o.name.clone(),
                            column: c.clone(),
                            kind,
                        }
                    })
                })
                .collect(),
            column_names: names,
        });
    }
    let blocked = if !there.is_empty() {
        Some(format!("{} already in {target_db}: {}. Choose “Drop and recreate”, “Empty then load”, “Add rows” or “Insert or replace”, or copy into a new database.", there.len(), there.iter().take(6).cloned().collect::<Vec<_>>().join(", ")))
    } else if !missing.is_empty() {
        Some(format!(
            "{} not in {target_db} yet: {}. Choose “Create” for them.",
            missing.len(),
            missing
                .iter()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ))
    } else {
        None
    };
    let changes_existing = tables.iter().any(|t| t.exists)
        && matches!(
            request.mode,
            CopyMode::Recreate | CopyMode::Empty | CopyMode::Upsert
        );
    let class = if changes_existing || target.environment == "production" {
        Class::Dangerous
    } else {
        Class::Schema
    };
    let level = if gate::decide(target.mode, class) == gate::Decision::Typed {
        "typed"
    } else {
        "confirm"
    };
    let mut warnings = Vec::new();
    if src_cfg.engine == crate::dbs::Engine::Mongodb {
        warnings.push("MongoDB copies aren't one point in time: documents written during the copy may or may not be in it.".to_string());
    }
    if cross {
        warnings.push(format!("{} to {}: views, functions and triggers aren't translated; the report lists them with their DDL. Columns marked lossy change on the way.", src_cfg.engine.label(), dst_cfg.engine.label()));
    } else if src_cfg.engine != dst_cfg.engine {
        warnings.push(format!("{} to {}: most tables copy as they are; features only one of them has will fail and be reported.", src_cfg.engine.label(), dst_cfg.engine.label()));
    }
    let masked = request.objects.iter().any(|o| {
        o.masks
            .iter()
            .flatten()
            .any(|m| m.kind != crate::dbs::masking::MaskKind::Keep)
    });
    if source.environment == "production" && target.environment != "production" && !masked {
        warnings.push(format!(
            "Copying production data to {} with no masking. Add rules?",
            target.name
        ));
    }
    let mut plan = CopyPlan {
        pending_id: None,
        blocked,
        level: level.into(),
        typed_name: target.name.clone(),
        source_name: source.name.clone(),
        target_name: target.name.clone(),
        environment: target.environment.clone(),
        target_db: target_db.clone(),
        new_database: new_db.is_some(),
        tables: tables.clone(),
        warnings,
        snapshot: src_cfg.engine != crate::dbs::Engine::Mongodb,
    };
    if plan.blocked.is_none() {
        // A saved job keeps its masking key, so its hashes and fakes match run after run.
        let mask_key = match &job_id {
            Some(id) => {
                let id = id.clone();
                let plan = state
                    .store()
                    .with_db_session(move |c, s| crate::dbs::jobs::plan(c, s, &id))
                    .await?
                    .1;
                super::jobs::key_of(&plan)
            }
            None => crate::dbs::masking::new_key(),
        };
        let job = CopyJob {
            mask_key,
            job_id,
            req: request,
            source_engine: src_cfg.engine,
            target_db,
            new_database: new_db.is_some(),
            typed_name: target.name,
            plans: tables,
        };
        plan.pending_id = Some(state.sql.copy_jobs.put(&conn_id, class, job)?);
    }
    Ok(plan)
}

/// Runs the held copy once confirmed (typed when the review asked), with the target's mode asked again.
#[tauri::command]
#[specta::specta]
pub async fn copy_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    transfer_id: String,
    progress: Channel<CopyProgress>,
) -> AppResult<CopyReport> {
    state.require_unlocked()?;
    let class = state.sql.copy_jobs.peek_class(&conn_id, &pending_id)?;
    let (job, _) = state.sql.copy_jobs.take(&conn_id, &pending_id)?;
    let dst_id = state.sql.db_id(&job.req.target_conn_id)?;
    let target = sql::gate_target(&state.store(), &dst_id).await?;
    if target.environment == "production" && target.mode != RedisWriteMode::Open {
        return Err(gate::read_only_error(&target.name));
    }
    gate::approve(target.mode, class, &job.typed_name, typed_name.as_deref())?;
    let dst_cfg = sql::load(&state.store(), &dst_id).await?;
    if job.new_database {
        let owned = Owned::get(&state.sql, &job.req.target_conn_id, dst_cfg.engine).await?;
        restore::create_database(&owned.conn(), &job.target_db, None).await?;
    }
    let src = state.sql.connect_to(&conn_id, None).await?;
    let dst = state
        .sql
        .connect_to(&job.req.target_conn_id, Some(job.target_db.clone()))
        .await?;
    let started = crate::db::now_ms();
    let stop = state.sql.begin_transfer(&transfer_id);
    let res = copy::run(copy::Run {
        job: &job,
        src,
        dst,
        stop: &stop,
        progress: &|p| {
            let _ = progress.send(p);
        },
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    let rep = res?;
    if let Some(id) = &job.job_id {
        let status = match rep.outcome.as_str() {
            "done" => "ok",
            "stopped" => "stopped",
            _ => "failed",
        };
        let mut log: Vec<String> = rep
            .tables
            .iter()
            .map(|t| {
                format!(
                    "{} → {}: {}, {} rows{}",
                    t.source,
                    t.target,
                    t.status,
                    t.rows,
                    t.error
                        .as_ref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                )
            })
            .collect();
        log.extend(rep.warnings.iter().cloned());
        super::jobs::record(
            &state,
            id,
            started,
            status,
            Some(rep.tables.iter().map(|t| t.rows).sum::<f64>() as i64),
            None,
            rep.warnings.len() as i64,
            log.join("\n"),
        )
        .await;
    }
    let source = sql::gate_target(&state.store(), &state.sql.db_id(&conn_id)?).await?;
    let text = format!(
        "copy from {} into {}: {}, {} tables, {} rows, {:.0} s",
        source.name,
        rep.target_db,
        rep.outcome,
        rep.tables.iter().filter(|t| t.status == "done").count(),
        rep.tables.iter().map(|t| t.rows).sum::<f64>(),
        rep.seconds
    );
    let _ = state
        .store()
        .with_db_session(move |c, s| crate::dbs::history::remember(c, s, &dst_id, "copy", &text))
        .await;
    Ok(rep)
}

/// "Select related": the tables the chosen ones reference by foreign key, all the way down, so a copy doesn't break
/// its keys. Returns them (the chosen ones not repeated).
#[tauri::command]
#[specta::specta]
pub async fn copy_related(
    state: State<'_, AppState>,
    conn_id: String,
    objects: Vec<copy::CopyObject>,
) -> AppResult<Vec<copy::CopyObject>> {
    state.require_unlocked()?;
    let raw = state.sql.connect_to(&conn_id, None).await?;
    copy::related(&raw, &objects).await
}
