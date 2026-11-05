//! Workspace template commands (S19, F09).

use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;
use crate::workspaces::{self, TemplateInput, TemplateSummary};

#[tauri::command]
#[specta::specta]
pub async fn workspaces_list(state: State<'_, AppState>) -> AppResult<Vec<TemplateSummary>> {
    state
        .with_db_session(|c, s| workspaces::list(c, &s.vault_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_save(state: State<'_, AppState>, input: TemplateInput) -> AppResult<String> {
    state
        .with_db_session(move |c, s| workspaces::save(c, &s.vault_id, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| workspaces::delete(c, &s.vault_id, &id))
        .await
}
