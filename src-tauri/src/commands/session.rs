//! Session commands (F01, F11, F12; S18, S14). Hosts must exist; everything needs an unlocked vault.

use serde::Serialize;
use specta::Type;
use std::sync::Arc;
use tauri::ipc::Channel;

use tauri::{AppHandle, State};

use crate::commands::logs::logs_dir;
use crate::error::AppResult;
use crate::history::HistoryTap;
use crate::known_hosts::{self, KnownHost};
use crate::logs;
use crate::ssh::{Cmd, Job, PromptAnswer, RawBytes, SessionEvent, SessionIo, SessionState};
use crate::state::AppState;

/// Wires a session to the UI channels, copying output into a log when one is recording.
fn tapped_io(
    tap: Option<Arc<logs::LogTap>>,
    hist: Option<Arc<HistoryTap>>,
    events: Channel<SessionEvent>,
    data: Channel<RawBytes>,
) -> SessionIo {
    let tap_events = tap.clone();
    SessionIo {
        event: Box::new(move |e| {
            if let (
                Some(t),
                SessionEvent::State {
                    state: SessionState::Disconnected | SessionState::Closed,
                },
            ) = (&tap_events, &e)
            {
                t.ended();
            }
            let _ = events.send(e);
        }),
        data: Box::new(move |b| {
            if let Some(t) = &tap {
                t.data(&b);
            }
            if let Some(h) = &hist {
                h.output(&b);
            }
            let _ = data.send(RawBytes(b));
        }),
    }
}

#[derive(Serialize, Type)]
pub struct SessionOpened {
    pub session_id: String,
}

/// Opens a tab's connection. Returns at once; progress arrives on `events`, terminal output on `data`.
#[tauri::command]
#[specta::specta]
pub async fn session_open(
    app: AppHandle,
    state: State<'_, AppState>,
    host_id: String,
    cols: u32,
    rows: u32,
    events: Channel<SessionEvent>,
    data: Channel<RawBytes>,
) -> AppResult<SessionOpened> {
    state.require_unlocked()?;
    let tap = logs::LogTap::start(&state.store(), logs_dir(&app)?, Some(host_id.clone())).await;
    let hist = HistoryTap::start(&state.store(), Some(host_id.clone())).await;
    let io = tapped_io(tap, hist.clone(), events, data);
    // Telnet and serial hosts get their own session type; everything else is SSH.
    let hid = host_id.clone();
    let telnet = state
        .with_db_session(move |c, s| {
            Ok(crate::hosts::get(c, &s.vault_id, &hid)?.protocol == "telnet")
        })
        .await
        .unwrap_or(false);
    let job = if telnet { Job::Telnet } else { Job::Shell };
    let session_id = state
        .sessions
        .open(state.store(), host_id, job, cols, rows, io)?;
    if let Some(h) = hist {
        state.history.add(&session_id, h);
    }
    Ok(SessionOpened { session_id })
}

/// A shell on this computer, shown as a tab. Same channels as `session_open`.
#[tauri::command]
#[specta::specta]
pub async fn local_session_open(
    app: AppHandle,
    state: State<'_, AppState>,
    cols: u32,
    rows: u32,
    events: Channel<SessionEvent>,
    data: Channel<RawBytes>,
) -> AppResult<SessionOpened> {
    state.require_unlocked()?;
    let tap = logs::LogTap::start(&state.store(), logs_dir(&app)?, None).await;
    let hist = HistoryTap::start(&state.store(), None).await;
    let io = tapped_io(tap, hist.clone(), events, data);
    let session_id =
        state
            .sessions
            .open(state.store(), String::new(), Job::Local, cols, rows, io)?;
    if let Some(h) = hist {
        state.history.add(&session_id, h);
    }
    Ok(SessionOpened { session_id })
}

#[tauri::command]
#[specta::specta]
pub fn session_write(
    state: State<'_, AppState>,
    session_id: String,
    data: Vec<u8>,
) -> AppResult<()> {
    state.history.input(&session_id, &data);
    state.sessions.send(&session_id, Cmd::Write(data))
}

#[tauri::command]
#[specta::specta]
pub fn session_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    state.sessions.send(&session_id, Cmd::Resize(cols, rows))
}

/// Backpressure: the terminal reports how many bytes it has finished drawing.
#[tauri::command]
#[specta::specta]
pub fn session_ack(state: State<'_, AppState>, session_id: String, bytes: u32) -> AppResult<()> {
    state.sessions.send(&session_id, Cmd::Ack(bytes as usize))
}

#[tauri::command]
#[specta::specta]
pub fn session_respond(
    state: State<'_, AppState>,
    session_id: String,
    prompt_id: String,
    answer: PromptAnswer,
) -> AppResult<()> {
    state.sessions.respond(&session_id, &prompt_id, answer)
}

#[tauri::command]
#[specta::specta]
pub fn session_close(state: State<'_, AppState>, session_id: String) {
    state.history.remove(&session_id);
    state.sessions.close(&session_id);
}

#[tauri::command]
#[specta::specta]
pub fn session_reconnect(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    state.require_unlocked()?;
    state.sessions.reconnect(state.store(), &session_id)
}

/// F11 with a choice: reconnect and sign in with only the chosen method.
#[tauri::command]
#[specta::specta]
pub fn session_reconnect_with(
    state: State<'_, AppState>,
    session_id: String,
    method: crate::ssh::AuthMethod,
) -> AppResult<()> {
    state.require_unlocked()?;
    state
        .sessions
        .reconnect_with(state.store(), &session_id, method)
}

#[tauri::command]
#[specta::specta]
pub async fn known_hosts_list(state: State<'_, AppState>) -> AppResult<Vec<KnownHost>> {
    state
        .with_db_session(|c, s| known_hosts::list(c, &s.vault_id))
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn known_host_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .with_db_session(move |c, s| known_hosts::delete(c, &s.vault_id, &id))
        .await
}

/// S08: installs a key's public half on a host. Same plumbing as a session: progress and prompts
/// (host key, password) arrive on `events`, and answers go to `session_respond`.
#[tauri::command]
#[specta::specta]
pub async fn key_export_to_host(
    state: State<'_, AppState>,
    key_id: String,
    host_id: String,
    dir: String,
    file_name: String,
    events: Channel<SessionEvent>,
) -> AppResult<SessionOpened> {
    state.require_unlocked()?;
    let io = SessionIo {
        event: Box::new(move |e| {
            let _ = events.send(e);
        }),
        data: Box::new(|_| {}),
    };
    let job = Job::Export {
        key_id,
        dir,
        file: file_name,
    };
    let session_id = state
        .sessions
        .open(state.store(), host_id, job, 80, 24, io)?;
    Ok(SessionOpened { session_id })
}
