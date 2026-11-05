//! Rust -> UI events that are not tied to one command call. Typed through tauri-specta.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    /// The user pressed Lock.
    Manual,
    /// Idle for longer than the auto-lock setting.
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct VaultLocked {
    pub reason: LockReason,
}

/// Sent shortly before an idle auto-lock so the UI can warn.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct AutoLockWarning {
    pub seconds_left: u32,
}
