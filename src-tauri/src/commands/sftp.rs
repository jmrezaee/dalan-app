//! SFTP and file-pane commands (F07; S21, S22). An `endpoint` is `"local"` or an `sftp_id`.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::sftp::{self, Choice, EditEvent, EditUpload, FsEntry, TransferEvent, TransferItem};
use crate::ssh::{PromptAnswer, Prompts, SessionEvent, SessionIo};
use crate::state::AppState;

#[derive(Serialize, Type)]
pub struct SftpOpened {
    pub sftp_id: String,
    /// The folder a new pane starts in.
    pub home: String,
    pub label: String,
}

/// Opens an SFTP session on a host. The caller picks `open_id` so it can answer host-key and
/// password questions (events on `events`, answers to `sftp_respond`) before this call returns.
#[tauri::command]
#[specta::specta]
pub async fn sftp_open(
    state: State<'_, AppState>,
    open_id: String,
    host_id: String,
    events: Channel<SessionEvent>,
) -> AppResult<SftpOpened> {
    state.require_unlocked()?;
    let prompts = Arc::new(Prompts::default());
    state.sftp.begin_open(&open_id, Arc::clone(&prompts));
    let last_error: Arc<Mutex<Option<AppError>>> = Arc::default();
    let io = {
        let last = Arc::clone(&last_error);
        Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::Error { code, message } = &e {
                    if let Ok(mut l) = last.lock() {
                        *l = Some(AppError::new(*code, message.clone()));
                    }
                }
                let _ = events.send(e);
            }),
            data: Box::new(|_| {}),
        })
    };
    let result = sftp::open_remote(&state.store(), &io, &prompts, &host_id).await;
    state.sftp.end_open(&open_id);
    let remote = match result {
        Ok(r) => r,
        // Prefer the specific reason the connection code reported (wrong password, unreachable, ...).
        Err(e) => {
            return Err(last_error
                .lock()
                .ok()
                .and_then(|mut l| l.take())
                .unwrap_or(e))
        }
    };
    let label = remote.label.clone();
    // The vault may have locked while we were connecting: don't register a session after that.
    state.require_unlocked()?;
    let remote = Arc::new(remote);
    let home = sftp::Fs::Remote(Arc::clone(&remote))
        .home()
        .await
        .unwrap_or_else(|_| "/".into());
    let sftp_id = uuid::Uuid::now_v7().to_string();
    state.sftp.add(sftp_id.clone(), remote);
    Ok(SftpOpened {
        sftp_id,
        home,
        label,
    })
}

#[tauri::command]
#[specta::specta]
pub fn sftp_respond(
    state: State<'_, AppState>,
    open_id: String,
    prompt_id: String,
    answer: PromptAnswer,
) -> AppResult<()> {
    state.sftp.respond(&open_id, &prompt_id, answer)
}

#[tauri::command]
#[specta::specta]
pub fn sftp_close(state: State<'_, AppState>, sftp_id: String) {
    state.sftp.remove(&sftp_id);
}

#[tauri::command]
#[specta::specta]
pub async fn fs_realpath(
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
) -> AppResult<String> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.realpath(&path).await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_home(state: State<'_, AppState>, endpoint: String) -> AppResult<String> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.home().await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_list(
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
    show_hidden: bool,
) -> AppResult<Vec<FsEntry>> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.list(&path, show_hidden).await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_mkdir(state: State<'_, AppState>, endpoint: String, path: String) -> AppResult<()> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.mkdir(&path).await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_rename(
    state: State<'_, AppState>,
    endpoint: String,
    from: String,
    to: String,
) -> AppResult<()> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.rename(&from, &to).await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_remove(
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
) -> AppResult<()> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.remove(&path).await
}

#[tauri::command]
#[specta::specta]
pub async fn fs_chmod(
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
    mode: u32,
) -> AppResult<()> {
    state.require_unlocked()?;
    state.sftp.fs(&endpoint)?.chmod(&path, mode).await
}

#[derive(Serialize, Type)]
pub struct TransferStarted {
    pub transfer_id: String,
}

/// Queues copies between any two places. Progress, conflicts and the end arrive on `events`.
#[tauri::command]
#[specta::specta]
pub async fn transfer_start(
    state: State<'_, AppState>,
    items: Vec<TransferItem>,
    events: Channel<TransferEvent>,
) -> AppResult<TransferStarted> {
    state.require_unlocked()?;
    let mut jobs = Vec::new();
    for i in items {
        jobs.push(sftp::transfer_job(
            state.sftp.fs(&i.src_endpoint)?,
            i.src_path,
            state.sftp.fs(&i.dst_endpoint)?,
            i.dst_dir,
        ));
    }
    let emit: sftp::TransferEmit = Arc::new(move |e| {
        let _ = events.send(e);
    });
    Ok(TransferStarted {
        transfer_id: state.sftp.transfers.start(jobs, emit)?,
    })
}

#[tauri::command]
#[specta::specta]
pub fn transfer_resolve(
    state: State<'_, AppState>,
    transfer_id: String,
    choice: Choice,
    apply_to_all: bool,
) -> AppResult<()> {
    state
        .sftp
        .transfers
        .resolve(&transfer_id, choice, apply_to_all)
}

#[tauri::command]
#[specta::specta]
pub fn transfer_cancel(state: State<'_, AppState>, transfer_id: String) -> AppResult<()> {
    state.sftp.transfers.cancel(&transfer_id)
}

#[derive(Serialize, Type)]
pub struct EditOpened {
    pub edit_id: String,
}

/// Downloads a server file, opens it in the system's default app and watches it. A `changed`
/// event means the local copy was saved; the UI then offers `edit_upload`.
#[tauri::command]
#[specta::specta]
pub async fn edit_open(
    app: AppHandle,
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
    events: Channel<EditEvent>,
) -> AppResult<EditOpened> {
    state.require_unlocked()?;
    let fs = state.sftp.fs(&endpoint)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::internal(e.to_string()))?
        .join("edit-tmp");
    let emit: sftp::EditEmit = Arc::new(move |e| {
        let _ = events.send(e);
    });
    let (edit_id, local) = state.sftp.edits.open(fs, &path, &root, emit).await?;
    if let Err(e) = app
        .opener()
        .open_path(local.to_string_lossy(), None::<&str>)
    {
        state.sftp.edits.close(&edit_id);
        return Err(AppError::internal(format!("Couldn't open the file: {e}")));
    }
    Ok(EditOpened { edit_id })
}

/// Uploads the edited copy. `RemoteChanged` means the server file changed meanwhile: ask, then
/// call again with `force`.
#[tauri::command]
#[specta::specta]
pub async fn edit_upload(
    state: State<'_, AppState>,
    edit_id: String,
    force: bool,
) -> AppResult<EditUpload> {
    state.require_unlocked()?;
    state.sftp.edits.upload(&edit_id, force).await
}

#[tauri::command]
#[specta::specta]
pub fn edit_close(state: State<'_, AppState>, edit_id: String) {
    state.sftp.edits.close(&edit_id);
}
