//! Thin `#[tauri::command]` functions. Logic lives in the area modules (db, vault, ssh, ...).

pub mod app;
pub mod hosts;
pub mod session;
pub mod vault;
