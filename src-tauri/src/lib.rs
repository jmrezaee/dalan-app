pub mod agent;
pub mod ai;
pub mod commands;
pub mod db;
pub mod diagnostics;
pub mod env;
pub mod error;
pub mod events;
pub mod groups;
pub mod history;
pub mod hosts;
pub mod import;
pub mod keys;
pub mod known_hosts;
pub mod logs;
pub mod mosh;
pub mod osdetect;
mod perms;
pub mod proxy;
pub mod pty;
pub mod serial;
pub mod settings;
pub mod sftp;
pub mod snippets;
pub mod ssh;
pub mod state;
pub mod telnet;
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
            commands::hosts::host_duplicate,
            commands::hosts::hosts_shared_logins,
            commands::hosts::host_set_protocol,
            commands::serial::serial_get,
            commands::serial::serial_set,
            commands::serial::serial_ports,
            commands::hosts::group_save,
            commands::hosts::group_delete,
            commands::hosts::hosts_move,
            commands::hosts::tags_list,
            commands::hosts::tag_delete,
            commands::hosts::recent_connections,
            commands::snippets::snippets_move,
            commands::sftp::fs_realpath,
            commands::session::session_open,
            commands::session::local_session_open,
            commands::session::session_write,
            commands::session::session_resize,
            commands::session::session_ack,
            commands::session::session_respond,
            commands::session::session_close,
            commands::session::session_reconnect,
            commands::session::session_reconnect_with,
            commands::session::known_hosts_list,
            commands::session::known_host_delete,
            commands::session::key_export_to_host,
            commands::workspaces::workspaces_list,
            commands::workspaces::workspace_save,
            commands::workspaces::workspace_delete,
            commands::sftp::edit_open,
            commands::sftp::edit_upload,
            commands::sftp::edit_close,
            commands::release::update_check,
            commands::release::update_install,
            commands::release::diagnostics_text,
            commands::history::history_list,
            commands::history::history_clear,
            commands::logs::logs_list,
            commands::logs::log_bookmark,
            commands::logs::log_delete,
            commands::logs::log_replay,
            commands::ai::ai_get,
            commands::ai::ai_save,
            commands::ai::ai_suggest,
            commands::agent::agent_get,
            commands::agent::agent_set,
            commands::proxy::proxy_get,
            commands::proxy::proxy_set,
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
            commands::keychain::key_set_certificate,
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

/// An acknowledgement from the UI: that many bytes of terminal output are no longer in flight.
/// (`fetch_update` is renamed `try_update` in newer Rust; the old name works on both, so it stays.)
#[allow(deprecated)]
pub(crate) fn release_unacked(unacked: &std::sync::atomic::AtomicUsize, n: usize) {
    let _ = unacked.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |u| Some(u.saturating_sub(n)),
    );
}

/// Anchored to this crate, not to the working directory: a debug app started from Finder has `/` as its cwd.
const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/types/bindings.ts");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    // In debug builds, refresh app/types/bindings.ts every time the app starts.
    // A failure here must not stop the app (the source folder may be read-only or gone): just log it.
    #[cfg(debug_assertions)]
    if let Err(e) = builder.export(specta_typescript::Typescript::default(), BINDINGS_PATH) {
        eprintln!("Couldn't refresh app/types/bindings.ts: {e}");
    }

    let app = tauri::Builder::default();
    // Must be the first plugin. A second launch (a double-click on the icon) shows the running window instead.
    #[cfg(desktop)]
    let app = app.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));
    app.plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let dir = app.path().app_data_dir()?;
            perms::private_dir(&dir)?;
            // Edited-file copies from a previous run are stale: nothing is watching them any more.
            let _ = std::fs::remove_dir_all(dir.join("edit-tmp"));
            let conn = db::open(&dir.join("vault.db"))?;
            for f in ["vault.db", "vault.db-wal", "vault.db-shm"] {
                let _ = perms::private_file(&dir.join(f));
            }
            // Logs a previous run never got to close (quit mid-session) count as ended when they started.
            let _ = conn.execute(
                "UPDATE session_logs SET ended_at = started_at WHERE ended_at IS NULL",
                [],
            );
            app.manage(AppState::new(conn));
            // The updater only exists in release builds, whose config carries `plugins.updater`.
            if app.config().plugins.0.contains_key("updater") {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }
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
