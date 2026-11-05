//! Session log commands (S15).

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::logs::{self, LogSummary};
use crate::ssh::RawBytes;
use crate::state::AppState;

pub fn logs_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::internal(e.to_string()))?
        .join("logs"))
}

#[tauri::command]
#[specta::specta]
pub async fn logs_list(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<LogSummary>> {
    let dir = logs_dir(&app)?;
    state
        .with_db_session(move |c, s| logs::list(c, &s.vault_id, &dir))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn log_bookmark(
    state: State<'_, AppState>,
    id: String,
    bookmarked: bool,
    comment: Option<String>,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| {
            logs::set_bookmark(c, &s.vault_id, &id, bookmarked, comment.as_deref())
        })
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn log_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    let dir = logs_dir(&app)?;
    state
        .with_db_session(move |c, s| logs::delete(c, &s.vault_id, &dir, &id))
        .await
}

/// Sends a log's bytes, decrypted, in order, over `data` (like live terminal output).
#[tauri::command]
#[specta::specta]
pub async fn log_replay(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    data: Channel<RawBytes>,
) -> AppResult<()> {
    let dir = logs_dir(&app)?;
    state
        .with_db_session(move |c, s| {
            // Only logs of this vault.
            logs::list(c, &s.vault_id, &dir)?
                .iter()
                .find(|l| l.id == id)
                .ok_or_else(|| AppError::not_found("That log no longer exists."))?;
            logs::read_chunks(&dir, &id, &s.key, |chunk| {
                let _ = data.send(RawBytes(chunk));
            })
        })
        .await
}
