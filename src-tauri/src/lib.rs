pub mod commands;
pub mod db;
pub mod error;
pub mod import;
pub mod keys;
pub mod logs;
pub mod pty;
pub mod sftp;
pub mod ssh;
pub mod state;
pub mod tunnels;
pub mod vault;

use tauri::Manager;
use tauri_specta::{collect_commands, Builder};

use crate::state::AppState;

/// Builds the command list once. `run()` mounts it, and the test below exports TS bindings from it.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::app::app_info,
        commands::vault::vault_status,
    ])
}

const BINDINGS_PATH: &str = "../app/types/bindings.ts";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // In debug builds, refresh app/types/bindings.ts every time the app starts.
    #[cfg(debug_assertions)]
    builder
        .export(specta_typescript::Typescript::default(), BINDINGS_PATH)
        .expect("failed to export TypeScript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("vault.db"))?;
            app.manage(AppState::new(conn));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Dalan");
}

#[cfg(test)]
mod tests {
    /// `cargo test` regenerates the bindings, so CI can diff them without launching the app.
    #[test]
    fn export_bindings() {
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default(),
                super::BINDINGS_PATH,
            )
            .expect("failed to export TypeScript bindings");
    }
}
