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
    state.with_db(move |c| settings::set(c, &key, &value)).await
}
