//! Proxy commands (S03, S04).

use tauri::State;

use crate::error::AppResult;
use crate::proxy::{self, ProxyChoice, ProxyConfig};
use crate::state::AppState;

/// `kind` is `host` or `group`.
#[tauri::command]
#[specta::specta]
pub async fn proxy_get(
    state: State<'_, AppState>,
    kind: String,
    id: String,
) -> AppResult<ProxyConfig> {
    state
        .with_db_session(move |c, s| proxy::get(c, &s.vault_id, &kind, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn proxy_set(
    state: State<'_, AppState>,
    kind: String,
    id: String,
    choice: ProxyChoice,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| proxy::set(c, &s.vault_id, &kind, &id, &choice))
        .await
}
