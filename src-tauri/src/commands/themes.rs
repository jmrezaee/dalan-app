//! Terminal theme commands (S03, S04).

use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;
use crate::themes::{self, Themes};

#[tauri::command]
#[specta::specta]
pub async fn terminal_themes_list(state: State<'_, AppState>) -> AppResult<Themes> {
    state
        .with_db_session(|c, s| themes::list(c, &s.vault_id))
        .await
}

/// `kind` is `host` or `group`; `theme: None` goes back to inheriting.
#[tauri::command]
#[specta::specta]
pub async fn terminal_theme_set(
    state: State<'_, AppState>,
    kind: String,
    id: String,
    theme: Option<String>,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| themes::set(c, &s.vault_id, &kind, &id, theme.as_deref()))
        .await
}
