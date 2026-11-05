//! Agent forwarding commands (S03, S04).

use tauri::State;

use crate::agent::{self, AgentChoice, AgentConfig};
use crate::error::AppResult;
use crate::state::AppState;

/// `kind` is `host` or `group`.
#[tauri::command]
#[specta::specta]
pub async fn agent_get(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> AppResult<AgentConfig> {
    state
        .with_db_session(move |c, s| agent::get(c, &s.vault_id, &kind, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_set(
    state: State<'_, AppState>,
    kind: String,
    id: String,
    choice: AgentChoice,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| agent::set(c, &s.vault_id, &kind, &id, &choice))
        .await
}
