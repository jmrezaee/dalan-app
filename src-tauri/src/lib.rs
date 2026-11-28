pub mod agent;
pub mod ai;
pub mod commands;
pub mod db;
pub mod dbs;
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
pub mod mongo;
pub mod mosh;
pub mod open_url;
pub mod osdetect;
mod perms;
pub mod proxy;
pub mod pty;
pub mod quit;
pub mod redis;
pub mod serial;
pub mod settings;
pub mod sftp;
pub mod snippets;
pub mod sql;
pub mod ssh;
pub mod state;
pub mod system_integration;
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
            commands::app::quit_app,
            commands::app::quit_cancel,
            commands::app::links_take,
            commands::app::system_integration_get,
            commands::app::system_integration_set,
            commands::app::open_default_apps,
            commands::databases::db_list,
            commands::databases::db_save,
            commands::databases::db_delete,
            commands::databases::db_test,
            commands::databases::db_parse_url,
            commands::databases::db_engines,
            commands::databases::saved_list,
            commands::databases::saved_save,
            commands::databases::saved_delete,
            commands::mongo::mg_tree,
            commands::mongo::mg_find,
            commands::mongo::mg_count,
            commands::mongo::mg_doc_get,
            commands::mongo::mg_export,
            commands::mongo::mg_shell,
            commands::mongo::mg_shell_commit,
            commands::mongo::mg_aggregate,
            commands::mongo::mg_agg_prepare,
            commands::mongo::mg_view_prepare,
            commands::mongo::mg_pipeline_code,
            commands::mongo::mg_schema,
            commands::mongo::mg_indexes,
            commands::mongo::mg_index_prepare,
            commands::mongo::mg_validation,
            commands::mongo::mg_validation_check,
            commands::mongo::mg_validation_prepare,
            commands::mongo::mg_doc_prepare,
            commands::mongo::mg_doc_commit,
            commands::mongo::mg_doc_discard,
            commands::sql::db_open,
            commands::sql::db_respond,
            commands::sql::db_close,
            commands::sql::sql_objects,
            commands::sql::sql_table_info,
            commands::sql::sql_table_page,
            commands::sql::sql_count,
            commands::sql::sql_cell,
            commands::sql::sql_run,
            commands::sql::sql_run_commit,
            commands::sql::sql_run_discard,
            commands::sql::sql_txn,
            commands::sql::sql_grid_prepare,
            commands::sql::sql_grid_commit,
            commands::sql::sql_grid_discard,
            commands::sql::sql_structure,
            commands::sql::sql_structure_prepare,
            commands::sql::sql_structure_commit,
            commands::sql::sql_structure_discard,
            commands::sql::sql_explain,
            commands::sql::sql_diagram,
            commands::sql::sql_save_export,
            commands::sql::sql_activity,
            commands::sql::sql_session_prepare,
            commands::sql::sql_session_commit,
            commands::sql::sql_session_discard,
            commands::sql::sql_export_table,
            commands::sql::sql_transfer_stop,
            commands::sql::sql_import_pick,
            commands::sql::sql_import_preview,
            commands::sql::sql_import_prepare,
            commands::sql::sql_import_commit,
            commands::sql::sql_import_discard,
            commands::sql::sql_backup_check,
            commands::sql::sql_backup_start,
            commands::restore::restore_pick,
            commands::restore::restore_prepare,
            commands::restore::restore_commit,
            commands::restore::restore_forget,
            commands::restore::restore_save_database,
            commands::copy::copy_prepare,
            commands::copy::copy_commit,
            commands::copy::copy_related,
            commands::jobs::jobs_list,
            commands::jobs::job_runs,
            commands::jobs::job_rename,
            commands::jobs::job_delete,
            commands::jobs::job_save_copy,
            commands::jobs::job_save_restore,
            commands::jobs::job_save_backup,
            commands::jobs::job_setup,
            commands::jobs::restore_pick_job,
            commands::jobs::job_backup_run,
            commands::bulk::bulk_export,
            commands::bulk::bulk_import_pick,
            commands::bulk::bulk_infer,
            commands::bulk::bulk_import_prepare,
            commands::bulk::bulk_import_commit,
            commands::bulk::bulk_import_discard,
            commands::bulk::mg_import_pick,
            commands::bulk::mg_import_preview,
            commands::bulk::mg_import_prepare,
            commands::bulk::mg_import_commit,
            commands::bulk::mg_import_discard,
            commands::checks::backup_check_pick,
            commands::checks::backup_verify,
            commands::compare::schema_compare,
            commands::compare::schema_sync_script,
            commands::compare::schema_sync_prepare,
            commands::compare::schema_sync_commit,
            commands::compare::schema_sync_discard,
            commands::compare::schema_script_save,
            commands::compare::schema_snapshot_save,
            commands::compare::schema_snapshots,
            commands::compare::schema_snapshot_delete,
            commands::compare::data_columns,
            commands::compare::data_compare,
            commands::compare::data_match_prepare,
            commands::compare::data_match_commit,
            commands::compare::data_match_discard,
            commands::compare::data_match_script,
            commands::jobs::job_set_schedule,
            commands::jobs::job_schedule_preview,
            commands::restore::restore_pick_remote,
            commands::restore::restore_pick_dropped,
            commands::sql::sql_cancel,
            commands::sql::sql_editor_close,
            commands::sql::sql_catalog,
            commands::sql::sql_format,
            commands::sql::sql_history,
            commands::sql::sql_history_clear,
            commands::redis::redis_db_list,
            commands::redis::redis_db_save,
            commands::redis::redis_db_delete,
            commands::redis::redis_db_test,
            commands::redis::redis_open,
            commands::redis::redis_respond,
            commands::redis::redis_select,
            commands::redis::redis_close,
            commands::redis::redis_scan,
            commands::redis::redis_key_info,
            commands::redis::redis_value_page,
            commands::redis::redis_write_prepare,
            commands::redis::redis_write_commit,
            commands::redis::redis_set_mode,
            commands::redis::redis_console_exec,
            commands::redis::redis_console_commit,
            commands::redis::redis_console_history,
            commands::redis::redis_console_history_clear,
            commands::redis::redis_commands,
            commands::redis::redis_info,
            commands::redis::redis_slowlog,
            commands::redis::redis_sub_start,
            commands::redis::redis_sub_stop,
            commands::redis::redis_bulk_preview,
            commands::redis::redis_bulk_run,
            commands::redis::redis_bulk_stop,
            commands::redis::redis_nodes,
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
            commands::sftp::edit_apps,
            commands::sftp::edit_app_forget,
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
            events::TunnelChanged,
            events::QuitRequested,
            events::LinkReceived,
            events::JobsChanged,
            events::FileDropped
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
#[cfg(any(debug_assertions, test))]
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
    let app = app.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        // A second launch with `--open-url ssh://...` (a link clicked in another app) is queued for the UI.
        open_url::handle_args(app, &args);
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }));
    app.plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
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
            app.manage(open_url::LinkQueue::default());
            // `--minimized` (start with Windows) starts in the taskbar; a link on the command line brings the window up.
            let args: Vec<String> = std::env::args().collect();
            if open_url::wants_minimized(&args) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.minimize();
                }
            }
            open_url::handle_args(app.handle(), &args);
            // The updater only exists in release builds, whose config carries `plugins.updater`.
            if app.config().plugins.0.contains_key("updater") {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }
            vault::spawn_auto_lock(app.handle().clone());
            #[cfg(target_os = "macos")]
            ask_before_quit_from_the_menu(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Dalan")
        .run(|app, event| {
            use tauri::{RunEvent, WindowEvent};
            use tauri_specta::Event;
            // The red close button ends here. (⌘Q does not: the macOS menu's own Quit goes around this, see
            // `ask_before_quit_from_the_menu`.) `code: None` means the user asked; the `app.exit(0)` from
            // `quit_app` carries a code and passes straight through.
            let ask = |app: &tauri::AppHandle| quit_question(app);
            match event {
                RunEvent::WindowEvent {
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    if let Some(n) = ask(app) {
                        api.prevent_close();
                        let _ = events::QuitRequested { live_sessions: n }.emit(app);
                    }
                }
                // A file dragged in from Finder: kept here, the window is told its name and id.
                RunEvent::WindowEvent {
                    event: WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }),
                    ..
                } => {
                    let state = app.state::<AppState>();
                    if let (true, Some(p)) = (state.is_unlocked(), paths.first()) {
                        let id = uuid::Uuid::now_v7().to_string();
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if let Ok(mut slot) = state.dropped.lock() {
                            *slot = Some((id.clone(), p.clone()));
                        }
                        let _ = events::FileDropped { id, name }.emit(app);
                    }
                }
                RunEvent::ExitRequested {
                    api, code: None, ..
                } => {
                    if let Some(n) = ask(app) {
                        api.prevent_exit();
                        let _ = events::QuitRequested { live_sessions: n }.emit(app);
                    }
                }
                _ => {}
            }
        });
}

/// `Some(n)` when quitting now would end `n` live sessions and the UI must be asked first.
fn quit_question(app: &tauri::AppHandle) -> Option<u32> {
    let st = app.try_state::<AppState>()?;
    match st
        .quit
        .decide(st.sessions.live_count() + st.sql.open_transactions())
    {
        quit::Decision::Allow => None,
        quit::Decision::Ask(n) => Some(n),
    }
}

/// The menu bar's Quit (⌘Q) calls the system's `terminate:`, which skips Tauri's exit events, so it is replaced by an
/// ordinary menu item that asks first and then exits through Tauri.
#[cfg(target_os = "macos")]
fn ask_before_quit_from_the_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItemBuilder, MenuItemKind};
    use tauri_specta::Event;
    let menu = Menu::default(app.handle())?;
    for item in menu.items()? {
        let MenuItemKind::Submenu(sub) = item else {
            continue;
        };
        for entry in sub.items()? {
            let MenuItemKind::Predefined(p) = &entry else {
                continue;
            };
            if p.text()?.starts_with("Quit") {
                let title = p.text()?;
                sub.remove(&entry)?;
                sub.append(
                    &MenuItemBuilder::with_id("quit-dalan", title)
                        .accelerator("CmdOrCtrl+Q")
                        .build(app)?,
                )?;
            }
        }
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id() == "quit-dalan" {
            match quit_question(app) {
                Some(n) => {
                    let _ = events::QuitRequested { live_sessions: n }.emit(app);
                }
                None => app.exit(0),
            }
        }
    });
    Ok(())
}

#[cfg(test)]
pub(crate) mod testservers;

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
