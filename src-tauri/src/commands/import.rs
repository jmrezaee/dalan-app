//! Import commands (S25, F10). Preview first, then apply what the user ticked.

use tauri::State;

use crate::error::AppResult;
use crate::import::{self, ImportPreview, ImportResult, ImportRow};
use crate::state::AppState;

/// `kind` is `ssh_config` or `csv`. `path` is a file chosen in the dialog; for `ssh_config`,
/// `None` reads `~/.ssh/config`. Rust reads the file, the webview only knows the path.
#[tauri::command]
#[specta::specta]
pub async fn import_preview(
    state: State<'_, AppState>,
    kind: String,
    path: Option<String>,
) -> AppResult<ImportPreview> {
    state
        .with_db_session(move |c, s| {
            let text = import::read_file(&kind, path.as_deref())?;
            import::preview(c, &s.vault_id, &kind, &text)
        })
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn import_apply(
    state: State<'_, AppState>,
    rows: Vec<ImportRow>,
    group_id: Option<String>,
) -> AppResult<ImportResult> {
    state
        .with_db_session(move |c, s| import::apply(c, s, &rows, group_id.as_deref()))
        .await
}
