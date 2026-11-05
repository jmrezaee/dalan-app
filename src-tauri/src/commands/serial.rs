//! Serial port commands (S03). The line settings belong to a host whose protocol is `serial`.

use tauri::State;

use crate::error::AppResult;
use crate::serial::{self, SerialConfig};
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn serial_get(state: State<'_, AppState>, host_id: String) -> AppResult<SerialConfig> {
    state
        .with_db_session(move |c, s| serial::get(c, &s.vault_id, &host_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn serial_set(
    state: State<'_, AppState>,
    host_id: String,
    config: SerialConfig,
) -> AppResult<()> {
    state
        .with_db_session(move |c, s| serial::set(c, &s.vault_id, &host_id, &config))
        .await
}

/// Serial devices on this computer, for the picker.
#[tauri::command]
#[specta::specta]
pub async fn serial_ports(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    state.require_unlocked()?;
    Ok(tokio::task::spawn_blocking(serial::list_ports)
        .await
        .unwrap_or_default())
}
