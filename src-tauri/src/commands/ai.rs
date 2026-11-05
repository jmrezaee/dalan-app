//! "Ask AI" commands. The API key stays in Rust; the webview only sees `has_key`.

use tauri::State;

use crate::ai::{self, AiConfig, AiSettings, Suggestion};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn ai_get(state: State<'_, AppState>) -> AppResult<AiConfig> {
    state.with_db_session(|c, _| Ok(ai::get(c))).await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_save(state: State<'_, AppState>, input: AiSettings) -> AppResult<AiConfig> {
    state
        .with_db_session(move |c, s| ai::save(c, s, &input))
        .await
}

/// One command for a plain-language request. `host_id` (or none for a local terminal) only picks
/// the OS name that goes along; nothing else about the host is sent.
#[tauri::command]
#[specta::specta]
pub async fn ai_suggest(
    state: State<'_, AppState>,
    request: String,
    host_id: Option<String>,
) -> AppResult<Suggestion> {
    let (ready, os) = state
        .with_db_session(move |c, s| {
            let os = match &host_id {
                Some(h) => crate::hosts::get(c, &s.vault_id, h)?.os_hint,
                None => Some(std::env::consts::OS.to_string()),
            };
            Ok((ai::ready(c, s)?, os))
        })
        .await?;
    ai::suggest(&ready, &request, os.as_deref()).await
}
