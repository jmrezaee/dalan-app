//! Saved jobs (D14, E1). Thin commands over `dbs::jobs`. Saving takes what the window set up (a copy request, restore
//! options) plus what only Rust holds (the dump's path, a backup folder, the masking key). Running a job is the
//! window opening the database's tab and going through the usual review with the saved plan: the confirmation is
//! built fresh from the current state, and the run is recorded when the copy, restore or backup ends.

use tauri::ipc::Channel;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use serde::Serialize;
use specta::Type;

use crate::dbs::dumpfile::DumpInfo;
use crate::dbs::jobs::{self, JobKind, JobRow, JobRun, NewJob, RunRecord};
use crate::error::{AppError, AppResult};
use crate::sql::copy::CopyRequest;
use crate::sql::restore::RestoreOptions;
use crate::sql::transfer::TransferProgress;
use crate::state::AppState;

/// The masking key in a saved plan (hex), or a new one for a plan saved without it.
pub fn key_of(plan: &serde_json::Value) -> [u8; 32] {
    let hex = plan.get("mask_key").and_then(|v| v.as_str()).unwrap_or("");
    let mut k = [0u8; 32];
    if hex.len() == 64 {
        for (i, b) in k.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0);
        }
        return k;
    }
    crate::dbs::masking::new_key()
}

fn hex(k: &[u8; 32]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

/// Records a run of a saved job (a failure to record never fails the run itself).
#[allow(clippy::too_many_arguments)]
pub async fn record(
    state: &State<'_, AppState>,
    job_id: &str,
    started: i64,
    status: &'static str,
    rows: Option<i64>,
    bytes: Option<i64>,
    warnings: i64,
    log: String,
) {
    let id = job_id.to_string();
    let _ = state
        .store()
        .with_db_session(move |c, s| {
            jobs::record_run(
                c,
                s,
                &id,
                RunRecord {
                    started_at: started,
                    status,
                    rows,
                    bytes,
                    warnings,
                    log,
                },
            )
        })
        .await;
}

#[tauri::command]
#[specta::specta]
pub async fn jobs_list(state: State<'_, AppState>) -> AppResult<Vec<JobRow>> {
    state.require_unlocked()?;
    state.with_db_session(|c, s| jobs::list(c, s)).await
}

#[tauri::command]
#[specta::specta]
pub async fn job_runs(state: State<'_, AppState>, job_id: String) -> AppResult<Vec<JobRun>> {
    state.require_unlocked()?;
    state
        .with_db_session(move |c, s| jobs::runs(c, s, &job_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn job_rename(state: State<'_, AppState>, job_id: String, name: String) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .with_db_session(move |c, s| jobs::rename(c, s, &job_id, &name))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn job_delete(state: State<'_, AppState>, job_id: String) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .with_db_session(move |c, s| jobs::delete(c, s, &job_id))
        .await
}

/// Saves the copy set up in this tab's Copy view (its target is the other tab's saved database).
#[tauri::command]
#[specta::specta]
pub async fn job_save_copy(
    state: State<'_, AppState>,
    conn_id: String,
    name: String,
    request: CopyRequest,
) -> AppResult<JobRow> {
    state.require_unlocked()?;
    let source = state.sql.db_id(&conn_id)?;
    let target = state.sql.db_id(&request.target_conn_id)?;
    let src = crate::sql::gate_target(&state.store(), &source).await?;
    let dst = crate::sql::gate_target(&state.store(), &target).await?;
    let masked = request.objects.iter().any(|o| {
        o.masks
            .iter()
            .flatten()
            .any(|m| m.kind != crate::dbs::masking::MaskKind::Keep)
    });
    let summary = format!(
        "{} {} from {} to {}{}{}",
        request.objects.len(),
        if request.objects.len() == 1 {
            "object"
        } else {
            "objects"
        },
        src.name,
        request
            .new_database
            .clone()
            .filter(|n| !n.trim().is_empty())
            .map_or(dst.name.clone(), |n| format!("{n} on {}", dst.name)),
        if masked { ", masked" } else { "" },
        match request.mode {
            crate::sql::copy::CopyMode::Create => "",
            crate::sql::copy::CopyMode::Recreate => ", dropping and recreating",
            crate::sql::copy::CopyMode::Empty => ", emptying first",
            crate::sql::copy::CopyMode::Append => ", adding rows",
            crate::sql::copy::CopyMode::Upsert => ", inserting or replacing",
        }
    );
    let mut req = request;
    req.target_conn_id = String::new();
    let plan =
        serde_json::json!({ "request": req, "mask_key": hex(&crate::dbs::masking::new_key()) });
    state
        .with_db_session(move |c, s| {
            jobs::save(
                c,
                s,
                NewJob {
                    name,
                    kind: JobKind::Copy,
                    source: Some(source.clone()),
                    target: Some(target),
                    summary,
                    plan,
                },
            )
        })
        .await
}

/// Saves the restore set up in this tab's restore dialog: the dump's path (it stays in Rust) and the options.
#[tauri::command]
#[specta::specta]
pub async fn job_save_restore(
    state: State<'_, AppState>,
    conn_id: String,
    dump_id: String,
    name: String,
    options: RestoreOptions,
) -> AppResult<JobRow> {
    state.require_unlocked()?;
    let db = state.sql.db_id(&conn_id)?;
    let dump = state.sql.dump(&conn_id, &dump_id)?;
    let target = crate::sql::gate_target(&state.store(), &db).await?;
    let summary = format!(
        "{} into {}{}",
        dump.info.file_name,
        options
            .new_database
            .clone()
            .filter(|n| !n.trim().is_empty())
            .map_or(target.name.clone(), |n| format!(
                "a new database {n} on {}",
                target.name
            )),
        if options.existing == crate::dbs::dumpscan::Existing::Replace {
            ", replacing what is there"
        } else {
            ""
        }
    );
    let plan = serde_json::json!({ "path": dump.path, "options": options });
    state
        .with_db_session(move |c, s| {
            jobs::save(
                c,
                s,
                NewJob {
                    name,
                    kind: JobKind::Restore,
                    source: None,
                    target: Some(db),
                    summary,
                    plan,
                },
            )
        })
        .await
}

/// Saves a backup of this tab's database into a folder chosen now (`None` when the dialog was cancelled).
#[tauri::command]
#[specta::specta]
pub async fn job_save_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    name: String,
) -> AppResult<Option<JobRow>> {
    state.require_unlocked()?;
    let db = state.sql.db_id(&conn_id)?;
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Folder for the backups")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(folder) = picked else {
        return Ok(None);
    };
    let folder = folder
        .into_path()
        .map_err(|_| AppError::validation("That folder can't be used."))?;
    let target = crate::sql::gate_target(&state.store(), &db).await?;
    let summary = format!(
        "Back up {} to the folder {}",
        target.name,
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    let plan = serde_json::json!({ "folder": folder });
    state
        .with_db_session(move |c, s| {
            jobs::save(
                c,
                s,
                NewJob {
                    name,
                    kind: JobKind::Backup,
                    source: Some(db),
                    target: None,
                    summary,
                    plan,
                },
            )
        })
        .await
        .map(Some)
}

/// What the window needs to set a saved job up for its review (never a path or the masking key).
#[derive(Debug, Clone, Serialize, Type)]
pub struct JobSetup {
    pub kind: JobKind,
    pub source_database_id: Option<String>,
    pub target_database_id: Option<String>,
    pub copy: Option<CopyRequest>,
    pub restore: Option<RestoreOptions>,
    pub file_name: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn job_setup(state: State<'_, AppState>, job_id: String) -> AppResult<JobSetup> {
    state.require_unlocked()?;
    let id = job_id.clone();
    let (kind, plan) = state
        .with_db_session(move |c, s| jobs::plan(c, s, &id))
        .await?;
    let row = state
        .with_db_session(|c, s| jobs::list(c, s))
        .await?
        .into_iter()
        .find(|j| j.id == job_id)
        .ok_or_else(|| AppError::not_found("That job is no longer saved."))?;
    Ok(JobSetup {
        kind,
        source_database_id: row.source_database_id,
        target_database_id: row.target_database_id,
        copy: plan
            .get("request")
            .and_then(|r| serde_json::from_value(r.clone()).ok()),
        restore: plan
            .get("options")
            .and_then(|r| serde_json::from_value(r.clone()).ok()),
        file_name: plan.get("path").and_then(|p| p.as_str()).and_then(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        }),
    })
}

/// A saved restore's dump, read again from where it was (it may have changed since: the safety scan runs again).
#[tauri::command]
#[specta::specta]
pub async fn restore_pick_job(
    state: State<'_, AppState>,
    conn_id: String,
    job_id: String,
) -> AppResult<DumpInfo> {
    state.require_unlocked()?;
    let (_, plan) = state
        .with_db_session(move |c, s| jobs::plan(c, s, &job_id))
        .await?;
    let path = plan
        .get("path")
        .and_then(|p| p.as_str())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| AppError::validation("This job has no dump file."))?;
    if !path.exists() {
        return Err(AppError::validation(format!(
            "The dump {} isn't there any more. Save the job again with the new file.",
            path.display()
        )));
    }
    let id = uuid::Uuid::now_v7().to_string();
    let dump = crate::dbs::dumpfile::pick(&path, id.clone()).await?;
    let info = dump.info.clone();
    state.sql.put_dump(&conn_id, id, dump);
    Ok(info)
}

/// Runs a saved backup into its folder: `<database>-<job>-<yyyy-mm-dd-hhmm>.<ext>`. Recorded as a run.
#[tauri::command]
#[specta::specta]
pub async fn job_backup_run(
    state: State<'_, AppState>,
    conn_id: String,
    job_id: String,
    transfer_id: String,
    progress: Channel<TransferProgress>,
) -> AppResult<super::sql::TransferDone> {
    state.require_unlocked()?;
    let id = job_id.clone();
    let (_, plan) = state
        .with_db_session(move |c, s| jobs::plan(c, s, &id))
        .await?;
    let folder = plan
        .get("folder")
        .and_then(|p| p.as_str())
        .map(std::path::PathBuf::from)
        .ok_or_else(|| AppError::validation("This job has no folder."))?;
    if !folder.is_dir() {
        return Err(AppError::validation(format!(
            "The folder {} isn't there any more.",
            folder.display()
        )));
    }
    let db_id = state.sql.db_id(&conn_id)?;
    let cfg = crate::sql::load(&state.store(), &db_id).await?;
    let ssh = state.sql.ssh(&conn_id).await?;
    let name = state
        .with_db_session({
            let j = job_id.clone();
            move |c, s| {
                Ok(jobs::list(c, s)?
                    .into_iter()
                    .find(|x| x.id == j)
                    .map(|x| x.name)
                    .unwrap_or_default())
            }
        })
        .await?;
    let ext = if crate::sql::backup::is_my(cfg.engine) {
        "sql"
    } else if crate::sql::backup::is_mongo(cfg.engine) {
        "archive.gz"
    } else {
        "dump"
    };
    let secs = (crate::db::now_ms() / 1000) % 86_400;
    let stamp = format!(
        "{}-{:02}{:02}",
        super::sql::chrono_free_date(),
        secs / 3600,
        (secs % 3600) / 60
    );
    let safe = |x: &str| {
        x.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
    };
    let path = folder.join(format!(
        "{}-{}-{stamp}.{ext}",
        safe(&cfg.database.clone().unwrap_or_else(|| cfg.name.clone())),
        safe(&name)
    ));
    let started = crate::db::now_ms();
    let stop = state.sql.begin_transfer(&transfer_id);
    let file = std::fs::File::create(&path)
        .map_err(|e| AppError::validation(format!("Couldn't create the backup file: {e}")))?;
    let mut out = std::io::BufWriter::new(file);
    let result = crate::sql::backup::run(crate::sql::backup::Run {
        cfg: &cfg,
        ssh: ssh.as_deref(),
        tool: crate::sql::backup::Tool::Dump,
        stdout: Some(&mut out),
        stdin_file: None,
        progress: &|bytes| {
            let _ = progress.send(TransferProgress { rows: 0.0, bytes });
        },
        log: &|l| log::debug!("dump tool: {l}"),
        stop: &stop,
    })
    .await;
    state.sql.end_transfer(&transfer_id);
    drop(out);
    match result {
        Ok(()) => {
            let bytes = std::fs::metadata(&path)
                .map(|m| m.len() as f64)
                .unwrap_or(0.0);
            record(
                &state,
                &job_id,
                started,
                "ok",
                None,
                Some(bytes as i64),
                0,
                format!("Saved {}", path.display()),
            )
            .await;
            Ok(super::sql::TransferDone {
                path: path.display().to_string(),
                rows: 0.0,
                bytes,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            record(
                &state,
                &job_id,
                started,
                if e.message == "Stopped." {
                    "stopped"
                } else {
                    "failed"
                },
                None,
                None,
                0,
                e.message.clone(),
            )
            .await;
            Err(e)
        }
    }
}
