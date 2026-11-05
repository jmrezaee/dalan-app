use std::sync::atomic::Ordering;

use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Serialize, Type)]
pub struct VaultStatus {
    pub exists: bool,
    pub unlocked: bool,
}

/// First run? Locked? (F10, S24). Allowed while locked.
#[tauri::command]
#[specta::specta]
pub async fn vault_status(state: State<'_, AppState>) -> AppResult<VaultStatus> {
    let exists = state
        .with_db(|c| {
            Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM vaults)", [], |r| {
                r.get::<_, bool>(0)
            })?)
        })
        .await?;
    Ok(VaultStatus {
        exists,
        unlocked: state.unlocked.load(Ordering::SeqCst),
    })
}
