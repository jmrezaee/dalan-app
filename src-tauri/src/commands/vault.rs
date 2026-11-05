//! Vault commands (F10, S24). Passwords arrive as `String` once and are zeroized after use.

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};
use tauri_specta::Event;
use zeroize::Zeroizing;

use crate::error::AppResult;
use crate::events::{LockReason, VaultLocked};
use crate::state::AppState;
use crate::vault::{self, kdf::KdfParams};

#[derive(Serialize, Type)]
pub struct VaultStatus {
    pub exists: bool,
    pub unlocked: bool,
    /// 0 = never auto-lock.
    pub auto_lock_minutes: u32,
    /// Set while wrong-password attempts are cooling down.
    pub cooldown_secs: Option<u32>,
}

/// First run? Locked? Allowed while locked.
#[tauri::command]
#[specta::specta]
pub async fn vault_status(state: State<'_, AppState>) -> AppResult<VaultStatus> {
    let (exists, auto_lock_minutes) = state
        .with_db(|c| Ok((vault::exists(c)?, vault::auto_lock_minutes(c))))
        .await?;
    Ok(VaultStatus {
        exists,
        unlocked: state.is_unlocked(),
        auto_lock_minutes,
        cooldown_secs: state.cooldown_secs().map(|s| s as u32),
    })
}

/// F10: derive the key, write the vault row, and leave the vault unlocked.
#[tauri::command]
#[specta::specta]
pub async fn vault_create(state: State<'_, AppState>, master_password: String) -> AppResult<()> {
    let pw = Zeroizing::new(master_password);
    let session = state
        .with_db(move |c| vault::create(c, &pw, KdfParams::default()))
        .await?;
    state.set_session(Some(session));
    Ok(())
}

/// Verify the password, hold the key in memory. Five wrong tries start a 30 second cooldown.
#[tauri::command]
#[specta::specta]
pub async fn vault_unlock(state: State<'_, AppState>, master_password: String) -> AppResult<()> {
    state.check_cooldown()?;
    let pw = Zeroizing::new(master_password);
    match state.with_db(move |c| vault::unlock(c, &pw)).await {
        Ok(session) => {
            state.reset_failures();
            state.set_session(Some(session));
            Ok(())
        }
        Err(e) => {
            if e.code == crate::error::ErrorCode::WrongPassword {
                state.record_failure();
            }
            Err(e)
        }
    }
}

/// Drops the key. The UI confirms first if sessions are open.
#[tauri::command]
#[specta::specta]
pub fn vault_lock(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    state.set_session(None);
    VaultLocked {
        reason: LockReason::Manual,
    }
    .emit(&app)
    .ok();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn vault_change_password(
    state: State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> AppResult<()> {
    state.require_unlocked()?;
    state.check_cooldown()?;
    let (old, new) = (Zeroizing::new(old_password), Zeroizing::new(new_password));
    let session = state
        .with_db(move |c| vault::change_password(c, &old, &new, KdfParams::default()))
        .await?;
    state.set_session(Some(session));
    Ok(())
}

/// The UI calls this on user activity (throttled) so auto-lock counts idle time, not uptime.
#[tauri::command]
#[specta::specta]
pub fn vault_touch(state: State<'_, AppState>) {
    state.touch();
}
