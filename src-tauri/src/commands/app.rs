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
