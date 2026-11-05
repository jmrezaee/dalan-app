//! Environment variable commands (S03, S04).

use tauri::State;

use crate::env::{self, EnvConfig, EnvVar};
use crate::error::AppResult;
use crate::state::AppState;

/// `kind` is `host` or `group`.
#[tauri::command]
#[specta::specta]
pub async fn env_get(state: State<'_, AppState>, kind: String, id: String) -> AppResult<EnvConfig> {
    state
        .with_db_session(move |c, s| env::get(c, &s.vault_id, &kind, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn env_set(
    state: State<'_, AppState>,
    kind: String,
    id: String,
    mode: String,
    vars: Vec<EnvVar>,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| env::set(c, &s.vault_id, &kind, &id, &mode, &vars))
        .await
}
