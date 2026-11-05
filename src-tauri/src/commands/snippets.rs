//! Snippet commands (S11, S12). Running a snippet needs no command: the UI writes it to a session.

use tauri::State;

use crate::error::AppResult;
use crate::snippets::{self, SnippetInput, SnippetsAll};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn snippets_list(state: State<'_, AppState>) -> AppResult<SnippetsAll> {
    state
        .with_db_session(|c, s| snippets::list(c, &s.vault_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn snippet_save(state: State<'_, AppState>, input: SnippetInput) -> AppResult<()> {
    state
        .with_db_session(move |c, s| snippets::save(c, &s.vault_id, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn snippet_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| snippets::delete(c, &s.vault_id, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn package_save(
    state: State<'_, AppState>,
    id: Option<String>,
    label: String,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| snippets::package_save(c, &s.vault_id, id.as_deref(), &label))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn package_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| snippets::package_delete(c, &s.vault_id, &id))
        .await
}
