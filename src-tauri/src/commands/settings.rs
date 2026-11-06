//! Settings commands (S23). Allowed while locked is not needed: the UI reads them after unlock.

use std::collections::HashMap;

use tauri::State;

use crate::error::AppResult;
use crate::settings;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn settings_get_all(state: State<'_, AppState>) -> AppResult<HashMap<String, String>> {
    state.require_unlocked()?;
    state.with_db(|c| settings::get_all(c)).await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_set(state: State<'_, AppState>, key: String, value: String) -> AppResult<()> {
    state.require_unlocked()?;
    // `ai` holds where the API key is sent, so only `ai_save` (which checks it) may write it. `sftp` holds
    // the apps Dalan launches, so only the SFTP commands may write that.
    if key == "ai" {
        return Err(crate::error::AppError::validation(
            "Use the AI settings page for that.",
        ));
    }
    if key == "sftp" {
        return Err(crate::error::AppError::validation(
            "That setting can't be changed from here.",
        ));
    }
    state.with_db(move |c| settings::set(c, &key, &value)).await
}
