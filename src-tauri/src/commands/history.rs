//! Command history commands (S20).

use tauri::State;

use crate::error::AppResult;
use crate::history::{self, HistoryEntry};
use crate::state::AppState;

/// `host_id: None` = the local terminal. Newest first.
#[tauri::command]
#[specta::specta]
pub async fn history_list(
    state: State<'_, AppState>,
    host_id: Option<String>,
    query: Option<String>,
    limit: u32,
) -> AppResult<Vec<HistoryEntry>> {
    state
        .with_db_session(move |c, s| {
            history::list(c, s, host_id.as_deref(), query.as_deref(), limit)
        })
        .await
}

/// Clears one server's history, or everything when `all` is true.
#[tauri::command]
#[specta::specta]
pub async fn history_clear(
    state: State<'_, AppState>,
    host_id: Option<String>,
    all: bool,
) -> AppResult<()> {
    state
        .with_db_session(move |c, _| history::clear(c, host_id.as_deref(), all))
        .await
}
