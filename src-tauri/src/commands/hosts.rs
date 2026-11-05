//! Host commands (S02, S03). Every one needs an unlocked vault: `with_db_session` returns `locked` otherwise.

use tauri::State;

use crate::error::AppResult;
use crate::groups::{self, GroupInput, GroupSummary};
use crate::hosts::{self, HostInput, HostSummary, HostsTree};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn hosts_tree(state: State<'_, AppState>) -> AppResult<HostsTree> {
    state
        .with_db_session(|c, s| hosts::tree(c, &s.vault_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn host_get(state: State<'_, AppState>, id: String) -> AppResult<HostSummary> {
    state
        .with_db_session(move |c, s| hosts::get(c, &s.vault_id, &id))
        .await
}

/// Creates when `input.id` is `None`, otherwise updates. The password goes straight into `secrets`.
#[tauri::command]
#[specta::specta]
pub async fn host_save(state: State<'_, AppState>, input: HostInput) -> AppResult<HostSummary> {
    state
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn host_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    if state.sessions.has_live_session_for(&id) || state.tunnels.running_for_host(&id) {
        return Err(crate::error::AppError::new(
            crate::error::ErrorCode::InUse,
            "Close this host's tab and stop its port forwards before deleting it.",
        ));
    }
    state
        .with_db_session(move |c, s| hosts::delete(c, s, &id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn group_save(state: State<'_, AppState>, input: GroupInput) -> AppResult<GroupSummary> {
    state
        .with_db_session(move |c, s| groups::save(c, s, &input))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn group_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| groups::delete(c, s, &id))
        .await
}

/// F04: drag hosts into a group (`group_id: null` = the top level).
#[tauri::command]
#[specta::specta]
pub async fn hosts_move(
    state: State<'_, AppState>,
    ids: Vec<String>,
    group_id: Option<String>,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| groups::move_hosts(c, &s.vault_id, &ids, group_id.as_deref()))
        .await
}

/// Switches a host between `ssh` and `telnet`.
#[tauri::command]
#[specta::specta]
pub async fn host_set_protocol(
    state: State<'_, AppState>,
    id: String,
    protocol: String,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| hosts::set_protocol(c, s, &id, &protocol))
        .await
}
