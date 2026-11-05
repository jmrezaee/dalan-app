pub mod commands;
pub mod db;
pub mod env;
pub mod error;
pub mod events;
pub mod groups;
pub mod hosts;
pub mod import;
pub mod keys;
pub mod known_hosts;
pub mod logs;
pub mod pty;
pub mod settings;
pub mod sftp;
pub mod snippets;
pub mod ssh;
pub mod state;
pub mod themes;
pub mod tunnels;
pub mod vault;
pub mod workspaces;

use tauri::Manager;
use tauri_specta::{collect_commands, collect_events, Builder};

use crate::state::AppState;

/// Builds the command list once. `run()` mounts it, and the test below exports TS bindings from it.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::app::app_info,
            commands::vault::vault_status,
            commands::vault::vault_create,
            commands::vault::vault_unlock,
            commands::vault::vault_lock,
            commands::vault::vault_change_password,
            commands::vault::vault_touch,
            commands::hosts::hosts_tree,
            commands::hosts::host_get,
            commands::hosts::host_save,
            commands::hosts::host_delete,
            commands::hosts::group_save,
            commands::hosts::group_delete,
            commands::hosts::hosts_move,
            commands::session::session_open,
            commands::session::local_session_open,
            commands::session::session_write,
            commands::session::session_resize,
            commands::session::session_ack,
            commands::session::session_respond,
            commands::session::session_close,
            commands::session::session_reconnect,
            commands::session::known_hosts_list,
            commands::session::known_host_delete,
            commands::session::key_export_to_host,
            commands::workspaces::workspaces_list,
            commands::workspaces::workspace_save,
            commands::workspaces::workspace_delete,
            commands::sftp::edit_open,
            commands::sftp::edit_upload,
            commands::sftp::edit_close,
            commands::env::env_get,
            commands::env::env_set,
            commands::themes::terminal_themes_list,
            commands::themes::terminal_theme_set,
            commands::import::import_preview,
            commands::import::import_apply,
            commands::sftp::sftp_open,
            commands::sftp::sftp_respond,
            commands::sftp::sftp_close,
            commands::sftp::fs_home,
            commands::sftp::fs_list,
            commands::sftp::fs_mkdir,
            commands::sftp::fs_rename,
            commands::sftp::fs_remove,
            commands::sftp::fs_chmod,
            commands::sftp::transfer_start,
            commands::sftp::transfer_resolve,
            commands::sftp::transfer_cancel,
            commands::tunnels::forwards_list,
            commands::tunnels::forward_save,
            commands::tunnels::forward_delete,
            commands::tunnels::forward_start,
            commands::tunnels::forward_stop,
            commands::tunnels::forward_respond,
            commands::snippets::snippets_list,
            commands::snippets::snippet_save,
            commands::snippets::snippet_delete,
            commands::snippets::package_save,
            commands::snippets::package_delete,
            commands::settings::settings_get_all,
            commands::settings::settings_set,
            commands::keychain::keys_list,
            commands::keychain::key_generate,
            commands::keychain::key_import,
            commands::keychain::key_update,
            commands::keychain::key_delete,
            commands::keychain::identities_list,
            commands::keychain::identity_save,
            commands::keychain::identity_delete,
        ])
        .events(collect_events![
            events::VaultLocked,
            events::AutoLockWarning,
            events::TunnelChanged
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
            // Edited-file copies from a previous run are stale: nothing is watching them any more.
            let _ = std::fs::remove_dir_all(dir.join("edit-tmp"));
            let conn = db::open(&dir.join("vault.db"))?;
            app.manage(AppState::new(conn));
            vault::spawn_auto_lock(app.handle().clone());
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
