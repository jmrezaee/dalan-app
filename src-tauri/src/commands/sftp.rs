//! SFTP and file-pane commands (F07; S21, S22). An `endpoint` is `"local"` or an `sftp_id`.

use std::sync::{Arc, Mutex};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
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

/// Which app opens the downloaded copy.
#[derive(Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EditApp {
    /// The system's default app for the file type.
    Default,
    /// The app chosen last time for this file type (the default app if none or it was removed).
    Saved,
    /// Ask with the OS file dialog, then remember the choice for this file type.
    Choose,
}

/// The file type used to remember an app: the lowercase extension, or "" for none.
fn ext_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// File types the system would run instead of showing. A server file of these types is never handed
/// to the system's default app, and the binary or bundle ones are never opened at all.
const SCRIPT_TYPES: [&str; 8] = [
    "command", "desktop", "terminal", "sh", "bat", "cmd", "ps1", "scpt",
];
const PROGRAM_TYPES: [&str; 11] = [
    "app", "exe", "jar", "pkg", "dmg", "msi", "com", "scr", "vbs", "workflow", "action",
];

/// `true` when opening this file would run it. `chosen_app`: the user picked an editor themselves,
/// which makes plain scripts safe to open as text.
fn runs_when_opened(name: &str, chosen_app: bool) -> bool {
    let ext = ext_of(name);
    PROGRAM_TYPES.contains(&ext.as_str()) || (!chosen_app && SCRIPT_TYPES.contains(&ext.as_str()))
}

/// "Visual Studio Code.app" -> "Visual Studio Code".
fn app_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// The apps remembered per file type: extension -> display name, for the menu labels.
#[tauri::command]
#[specta::specta]
pub async fn edit_apps(state: State<'_, AppState>) -> AppResult<HashMap<String, String>> {
    state.require_unlocked()?;
    let all = state
        .with_db(|c| Ok(crate::settings::open_with_all(c)))
        .await?;
    Ok(all.into_iter().map(|(k, v)| (k, app_name(&v))).collect())
}

/// Forgets the app remembered for a file type.
#[tauri::command]
#[specta::specta]
pub async fn edit_app_forget(state: State<'_, AppState>, ext: String) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .with_db(move |c| crate::settings::open_with_remove(c, &ext))
        .await
}

/// Downloads a server file, opens it in an app and watches it. A `changed` event means the local
/// copy was saved; the UI then offers `edit_upload`. Returns `None` when the app dialog is cancelled.
/// The app path never comes from the webview: it is the OS dialog's answer or one Rust stored.
#[tauri::command]
#[specta::specta]
pub async fn edit_open(
    app: AppHandle,
    state: State<'_, AppState>,
    endpoint: String,
    path: String,
    open_with: EditApp,
    events: Channel<EditEvent>,
) -> AppResult<Option<EditOpened>> {
    state.require_unlocked()?;
    let fs = state.sftp.fs(&endpoint)?;
    let ext = ext_of(&path);
    let program: Option<String> = match open_with {
        EditApp::Default => None,
        EditApp::Saved => {
            let saved = state
                .with_db(|c| Ok(crate::settings::open_with_all(c)))
                .await?;
            saved
                .get(&ext)
                .filter(|p| std::path::Path::new(p).exists())
                .cloned()
        }
        EditApp::Choose => {
            // The dialog blocks until closed, so it runs off the async threads.
            let dialog_app = app.clone();
            let picked = tokio::task::spawn_blocking(move || {
                let mut d = dialog_app.dialog().file().set_title("Open with");
                if cfg!(target_os = "macos") {
                    d = d.set_directory("/Applications");
                }
                d.blocking_pick_file()
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
            let Some(picked) = picked else {
                return Ok(None);
            };
            let p = picked
                .into_path()
                .map_err(|e| AppError::internal(e.to_string()))?
                .to_string_lossy()
                .into_owned();
            Some(p)
        }
    };
    if runs_when_opened(&path, program.is_some()) {
        return Err(AppError::validation(
            "That kind of file would run on your computer if opened. Choose Open with… and pick an editor.",
        ));
    }
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
        .open_path(local.to_string_lossy(), program.as_deref())
    {
        state.sftp.edits.close(&edit_id);
        return Err(AppError::internal(format!("Couldn't open the file: {e}")));
    }
    // Files without an extension would share one entry for every such file, so they are not remembered.
    if let (EditApp::Choose, Some(p), false) = (open_with, program, ext.is_empty()) {
        // Remember it only once it really opened the file.
        state
            .with_db(move |c| crate::settings::open_with_set(c, &ext, &p))
            .await?;
    }
    Ok(Some(EditOpened { edit_id }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_and_scripts_are_refused_for_the_default_app() {
        for n in [
            "a.command",
            "x.APP",
            "setup.exe",
            "run.sh",
            "t.Terminal",
            "a.jar",
            "m.desktop",
            "s.ps1",
        ] {
            assert!(runs_when_opened(n, false), "{n}");
        }
        assert!(!runs_when_opened("notes.txt", false));
        assert!(!runs_when_opened(".bashrc", false));
    }

    #[test]
    fn a_chosen_editor_may_open_scripts_but_never_programs() {
        assert!(!runs_when_opened("deploy.sh", true));
        assert!(!runs_when_opened("a.command", true));
        assert!(runs_when_opened("Tool.app", true));
        assert!(runs_when_opened("setup.EXE", true));
    }
}
