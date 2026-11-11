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

/// A port-forward rule changed state or connection count (S09).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct TunnelChanged {
    pub id: String,
    pub state: crate::tunnels::TunnelState,
    pub connections: u32,
    pub error: Option<String>,
}

/// The window was closed or ⌘Q pressed while sessions are live: the UI asks before quitting.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct QuitRequested {
    pub live_sessions: u32,
}

/// A link (`dalan-app.exe --open-url ssh://...`) arrived, on a cold start or from a second launch. The UI asks
/// `links_take` for what is waiting (it may not have been listening when a cold start queued the first one).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct LinkReceived {}
