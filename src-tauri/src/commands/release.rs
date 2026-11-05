//! About, updates and diagnostics (S23 About, P13).

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

use crate::diagnostics;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The release build's config has `plugins.updater`; dev builds don't, and say so.
fn updater_configured(app: &AppHandle) -> bool {
    app.config().plugins.0.contains_key("updater")
}

#[derive(Serialize, Type)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
}

/// `None` = you are up to date.
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> AppResult<Option<UpdateInfo>> {
    if !updater_configured(&app) {
        return Err(AppError::validation("Updates aren't set up in this build."));
    }
    let updater = app
        .updater()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let update = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("Couldn't check for updates: {e}")))?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version,
        notes: u.body,
    }))
}

/// Downloads the newest version, installs it and restarts the app.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle) -> AppResult<()> {
    if !updater_configured(&app) {
        return Err(AppError::validation("Updates aren't set up in this build."));
    }
    let updater = app
        .updater()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("Couldn't check for updates: {e}")))?
    else {
        return Err(AppError::validation(
            "You're already on the newest version.",
        ));
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| AppError::internal(format!("The update failed: {e}")))?;
    app.restart()
}

/// Text to paste into an issue: version, system, and the last log lines with servers, addresses
/// and usernames removed. Works while locked (then only the generic removal applies).
#[tauri::command]
#[specta::specta]
pub async fn diagnostics_text(app: AppHandle, state: State<'_, AppState>) -> AppResult<String> {
    let terms = state
        .with_db_session(|c, _| diagnostics::private_terms(c))
        .await
        .unwrap_or_default();
    let log_dir = app.path().app_log_dir().ok();
    let tail = log_dir.as_deref().map_or_else(
        || "(no log file yet)".into(),
        |d| diagnostics::tail_log(d, diagnostics::LOG_LINES),
    );
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    let report = format!(
        "Dalan {}\nSystem: {} {}\nUpdates: {}\n\n--- last {} log lines (servers, addresses and usernames removed) ---\n{}\n",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        if updater_configured(&app) { "on" } else { "not set up in this build" },
        diagnostics::LOG_LINES,
        tail,
    );
    Ok(diagnostics::redact(&report, home.as_deref(), &terms))
}
