//! Thin `#[tauri::command]` functions. Logic lives in the area modules (db, vault, ssh, ...).

pub mod app;
pub mod hosts;
pub mod keychain;
pub mod session;
pub mod settings;
pub mod snippets;
pub mod tunnels;
pub mod vault;
