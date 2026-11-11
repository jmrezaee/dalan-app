use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

#[derive(Serialize, Type)]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub platform: String,
}

/// S23 About: version, data dir, platform.
#[tauri::command]
#[specta::specta]
pub fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        data_dir: dir.display().to_string(),
        platform: std::env::consts::OS.to_string(),
    })
}

/// The UI's answer "Quit anyway": ends the app (and with it every live session).
#[tauri::command]
#[specta::specta]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// The UI's answer "Keep working": the next quit asks again.
#[tauri::command]
#[specta::specta]
pub fn quit_cancel(state: tauri::State<'_, crate::state::AppState>) {
    state.quit.cancel();
}

/// The `ssh://` links that arrived and wait for the UI (see `open_url`). `None` is a link that didn't parse.
#[tauri::command]
#[specta::specta]
pub fn links_take(
    queue: tauri::State<'_, crate::open_url::LinkQueue>,
) -> Vec<Option<crate::open_url::SshTarget>> {
    queue.take()
}

/// Settings > System (Windows): what the registry says now. Other systems answer `supported: false`.
#[tauri::command]
#[specta::specta]
pub fn system_integration_get() -> AppResult<crate::system_integration::SystemIntegration> {
    crate::system_integration::get()
}

/// Turns "Open ssh:// links with Dalan" and "Start Dalan when I sign in" on or off. A switch left `None` is kept.
#[tauri::command]
#[specta::specta]
pub fn system_integration_set(
    ssh_links: Option<bool>,
    autostart: Option<bool>,
) -> AppResult<crate::system_integration::SystemIntegration> {
    if let Some(on) = ssh_links {
        crate::system_integration::set_ssh_links(on)?;
    }
    if let Some(on) = autostart {
        crate::system_integration::set_autostart(on)?;
    }
    crate::system_integration::get()
}

/// Opens Windows Settings > Default apps (opened here, not by the webview).
#[tauri::command]
#[specta::specta]
pub fn open_default_apps(app: AppHandle) -> AppResult<()> {
    #[cfg(windows)]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url("ms-settings:defaultapps", None::<&str>)
            .map_err(|e| AppError::internal(e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err(AppError::validation("This is a Windows setting."))
    }
}
