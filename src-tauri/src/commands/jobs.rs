//! Saved jobs (D14, E1). Thin commands over `dbs::jobs`. Saving takes what the window set up (a copy request, restore
//! options) plus what only Rust holds (the dump's path, a backup folder, the masking key). Running a job is the
//! window opening the database's tab and going through the usual review with the saved plan: the confirmation is
//! built fresh from the current state, and the run is recorded when the copy, restore or backup ends.

use tauri::ipc::Channel;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::dbs::dumpfile::DumpInfo;
use crate::dbs::jobs::{self, JobKind, JobRow, JobRun, NewJob, RunRecord};
use crate::dbs::schedule::{Every, Retention, Schedule};
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
    state: &AppState,
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
                    destination: None,
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
    // A dump fetched from a host is saved by where it is there, so the job fetches it again (D15).
    let (plan, destination, summary) = match &dump.origin {
        Some((host_id, path)) => (
            serde_json::json!({ "host_id": host_id, "remote_path": path, "options": options }),
            Some(serde_json::json!({ "host_id": host_id })),
            format!("{summary} (the dump is on a host)"),
        ),
        None => (
            serde_json::json!({ "path": dump.path, "options": options }),
            None,
            summary,
        ),
    };
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
                    destination,
                },
            )
        })
        .await
}

/// A folder on a saved host, picked in the SFTP picker (D15, E8).
#[derive(Debug, Clone, Deserialize, Type)]
pub struct HostFolder {
    pub host_id: String,
    pub dir: String,
}

/// Saves a backup of this tab's database into a folder: on a host when `host` is given, otherwise one chosen now in
/// the system dialog (`None` when that was cancelled).
#[tauri::command]
#[specta::specta]
pub async fn job_save_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    conn_id: String,
    name: String,
    host: Option<HostFolder>,
) -> AppResult<Option<JobRow>> {
    state.require_unlocked()?;
    let db = state.sql.db_id(&conn_id)?;
    if let Some(h) = host {
        let dir = h.dir.trim().to_string();
        if dir.is_empty() {
            return Err(AppError::validation("Pick a folder on the host."));
        }
        let label: String = {
            let id = h.host_id.clone();
            state
                .with_db_session(move |c, s| {
                    use rusqlite::OptionalExtension;
                    c.query_row(
                        "SELECT label FROM hosts WHERE id = ?1 AND vault_id = ?2",
                        rusqlite::params![id, s.vault_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| AppError::not_found("That host is no longer saved."))
                })
                .await?
        };
        let target = crate::sql::gate_target(&state.store(), &db).await?;
        let summary = format!("Back up {} to {dir} on {label}", target.name);
        let plan = serde_json::json!({ "host_id": h.host_id, "dir": dir });
        return state
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
                        destination: Some(serde_json::json!({ "host_id": h.host_id })),
                    },
                )
            })
            .await
            .map(Some);
    }
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
                    destination: None,
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
        file_name: plan
            .get("path")
            .or_else(|| plan.get("remote_path"))
            .and_then(|p| p.as_str())
            .and_then(|p| {
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
    if let (Some(host_id), Some(remote)) = (
        plan.get("host_id").and_then(|p| p.as_str()),
        plan.get("remote_path").and_then(|p| p.as_str()),
    ) {
        let fs = crate::sftp::Fs::Remote(std::sync::Arc::new(
            crate::dbs::runner::open_host(&state.store(), host_id).await?,
        ));
        let (tmp, local) = crate::dbs::runner::fetch(&fs, remote).await?;
        let id = uuid::Uuid::now_v7().to_string();
        let mut dump = crate::dbs::dumpfile::pick(&local, id.clone()).await?;
        dump._fetched = Some(tmp);
        dump.origin = Some((host_id.to_string(), remote.to_string()));
        return super::restore::keep_dump(&state, &conn_id, id, dump).await;
    }
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
    super::restore::keep_dump(&state, &conn_id, id, dump).await
}

/// Runs a saved backup into its folder (here or on a host): `<database>-<job>-<yyyy-mm-dd-hhmm>.<ext>`, then
/// applies the job's retention. Recorded as a run.
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
    let started = crate::db::now_ms();
    let result =
        crate::dbs::runner::backup_into(&state, &conn_id, &job_id, &transfer_id, &|bytes| {
            let _ = progress.send(TransferProgress { rows: 0.0, bytes });
        })
        .await;
    match result {
        Ok(done) => {
            record(
                &state,
                &job_id,
                started,
                "ok",
                None,
                Some(done.bytes as i64),
                0,
                done.log.join("\n"),
            )
            .await;
            Ok(super::sql::TransferDone {
                path: done.path,
                rows: 0.0,
                bytes: done.bytes,
                check: Some(done.check),
                file_id: None,
            })
        }
        Err(e) => {
            let status = if e.message == "Stopped." {
                "stopped"
            } else {
                "failed"
            };
            record(
                &state,
                &job_id,
                started,
                status,
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

/// D15: sets or clears a job's schedule (backups, and copies whose target isn't production) and its retention.
#[tauri::command]
#[specta::specta]
pub async fn job_set_schedule(
    state: State<'_, AppState>,
    job_id: String,
    schedule: Option<Schedule>,
    retention: Option<Retention>,
) -> AppResult<JobRow> {
    state.require_unlocked()?;
    let id = job_id.clone();
    state
        .with_db_session(move |c, s| {
            jobs::set_schedule(c, s, &id, schedule, retention)?;
            jobs::list(c, s)?
                .into_iter()
                .find(|j| j.id == id)
                .ok_or_else(|| AppError::not_found("That job is no longer saved."))
        })
        .await
}

/// What a schedule means: in words, and its next three runs (epoch ms). An invalid cron says why.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SchedulePreview {
    pub text: String,
    pub next: Vec<f64>,
}

#[tauri::command]
#[specta::specta]
pub fn job_schedule_preview(every: Every) -> AppResult<SchedulePreview> {
    let cron = every.cron()?;
    let mut next = Vec::new();
    let mut t = crate::db::now_ms();
    for _ in 0..3 {
        match crate::dbs::schedule::next(&cron, t) {
            Some(x) => {
                next.push(x as f64);
                t = x;
            }
            None => break,
        }
    }
    if next.is_empty() {
        return Err(AppError::validation(
            "That schedule never runs (no such date within a year).",
        ));
    }
    Ok(SchedulePreview {
        text: every.describe(),
        next,
    })
}
