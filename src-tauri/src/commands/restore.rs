//! Restore any dump (D12, S48, F31). Thin commands over `dbs::dumpfile` and `sql::restore`: pick a file or folder
//! (the path stays in Rust; the window gets an id), plan and hold the restore through the gate, run it with progress
//! on a `Channel`, and let the dump go. Stop is `sql_transfer_stop` with the run's transfer id.

use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::dbs::dumpfile::{self, DumpInfo};
use crate::dbs::gate;
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::redis::store::RedisWriteMode;
use crate::sql::restore::{
    self, Conn, PlanInput, RestoreOptions, RestorePlan, RestoreProgress, RestoreResult, RunInput,
};
use crate::sql::{self, SqlManager};
use crate::state::AppState;

/// The tab's connection, owned for the length of a command.
enum Owned {
    Pg(Arc<sql::pg::PgConn>),
    My(Arc<sql::mydb::MyDb>),
    Mongo(Arc<crate::mongo::conn::MongoConn>),
}

impl Owned {
    async fn get(m: &SqlManager, conn_id: &str, engine: Engine) -> AppResult<Owned> {
        Ok(match engine {
            Engine::Mongodb => Owned::Mongo(m.mongo(conn_id).await?),
            Engine::Mysql | Engine::Mariadb => {
                Owned::My(m.my(conn_id).await?.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::DbUnreachable,
                        "This database isn't connected right now.",
                    )
                })?)
            }
            _ => Owned::Pg(m.pg(conn_id).await?),
        })
    }
    fn conn(&self) -> Conn<'_> {
        match self {
            Owned::Pg(c) => Conn::Pg(c),
            Owned::My(c) => Conn::My(c),
            Owned::Mongo(c) => Conn::Mongo(c),
        }
    }
}

/// The system dialog for a dump file (or a dump folder), then the dump is read: what it is and what is inside.
/// `None` when the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn restore_pick(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    folder: bool,
) -> AppResult<Option<DumpInfo>> {
    state.require_unlocked()?;
    state.sql.db_id(&conn_id)?;
    let dialog_app = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        let d = dialog_app.dialog().file().set_title(if folder {
            "Restore from a dump folder"
        } else {
            "Restore from a dump"
        });
        if folder {
            d.blocking_pick_folder()
        } else {
            d.blocking_pick_file()
        }
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(p) = picked else { return Ok(None) };
    let path = p
        .into_path()
        .map_err(|_| AppError::validation("That location can't be used."))?;
    let id = uuid::Uuid::now_v7().to_string();
    let dump = dumpfile::pick(&path, id.clone()).await?;
    let info = dump.info.clone();
    state.sql.put_dump(&conn_id, id, dump);
    Ok(Some(info))
}

/// Plans the restore and, unless something blocks it, holds it for the confirmation.
#[tauri::command]
#[specta::specta]
pub async fn restore_prepare(
    state: State<'_, AppState>,
    conn_id: String,
    dump_id: String,
    options: RestoreOptions,
) -> AppResult<RestorePlan> {
    state.require_unlocked()?;
    let db_id = state.sql.db_id(&conn_id)?;
    let dump = state.sql.dump(&conn_id, &dump_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    let cfg = sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let owned = Owned::get(&state.sql, &conn_id, cfg.engine).await?;
    let environment = target.environment.clone();
    let (mut plan, job) = restore::plan(PlanInput {
        conn: owned.conn(),
        cfg: &cfg,
        ssh: ssh.as_deref(),
        dump: &dump,
        dump_id,
        opts: options,
        saved_name: target.name,
        environment: target.environment,
        mode: target.mode,
    })
    .await?;
    if plan.blocked.is_none() {
        let class = restore::class_of(&job, &environment);
        plan.pending_id = Some(state.sql.restore_jobs.put(&conn_id, class, job)?);
    }
    Ok(plan)
}

/// Runs the held restore once the confirmation (typed when the review asked for it) arrives and the mode is asked again.
#[tauri::command]
#[specta::specta]
pub async fn restore_commit(
    state: State<'_, AppState>,
    conn_id: String,
    pending_id: String,
    typed_name: Option<String>,
    transfer_id: String,
    progress: Channel<RestoreProgress>,
) -> AppResult<RestoreResult> {
    state.require_unlocked()?;
    let class = state.sql.restore_jobs.peek_class(&conn_id, &pending_id)?;
    let db_id = state.sql.db_id(&conn_id)?;
    let target = sql::gate_target(&state.store(), &db_id).await?;
    let (job, _) = state.sql.restore_jobs.take(&conn_id, &pending_id)?;
    if !job.new_database
        && target.environment == "production"
        && target.mode != RedisWriteMode::Open
    {
        return Err(gate::read_only_error(&target.name));
    }
    gate::approve(target.mode, class, &job.typed_name, typed_name.as_deref())?;
    let dump = state.sql.dump(&conn_id, &job.dump_id)?;
    let mut cfg = sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let owned = Owned::get(&state.sql, &conn_id, cfg.engine).await?;
    if job.new_database {
        restore::create_database(&owned.conn(), &job.target_db, job_encoding(&job)).await?;
    }
    cfg.database = Some(job.target_db.clone());
    let stop = state.sql.begin_transfer(&transfer_id);
    let ran = restore::run(RunInput {
        job: &job,
        cfg: &cfg,
        ssh,
        dump: Arc::clone(&dump),
        stop,
        progress: &|p| {
            let _ = progress.send(p);
        },
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    let (outcome, message, rep) = match ran {
        Ok(x) => x,
        Err(e) => {
            // Nothing ran (the upload or the tool's start failed): a database made a moment ago goes again.
            restore::after(&owned.conn(), &job, "failed").await;
            return Err(e);
        }
    };
    let removed = restore::after(&owned.conn(), &job, &outcome).await;
    let result = restore::result(&job, outcome, message, rep, removed);
    let text = restore::summary(&dump.info.file_name, &result);
    let _ = state
        .store()
        .with_db_session(move |c, s| crate::dbs::history::remember(c, s, &db_id, "restore", &text))
        .await;
    Ok(result)
}

fn job_encoding(job: &restore::Job) -> Option<&str> {
    restore::encoding(job)
}

/// The dump isn't needed any more (the dialog closed): its unpacked folder, if any, goes with it.
#[tauri::command]
#[specta::specta]
pub fn restore_forget(
    state: State<'_, AppState>,
    conn_id: String,
    dump_id: String,
) -> AppResult<()> {
    state.sql.forget_dump(&conn_id, &dump_id);
    Ok(())
}
