//! SSH sessions (F01, F11, F12; S18). One tokio task per session owns the connection.
//! The UI talks to it through `Cmd`s; it talks back through `SessionIo` (typed events plus raw bytes).
//!
//! Terminal data is bytes end to end: it is never turned into a `String` here (hard rule 3).

mod client;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::{mpsc, oneshot};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::state::Store;

pub use client::run;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Connecting,
    Authenticating,
    Ready,
    Disconnected,
    Closed,
}

/// A question the session needs answered before it can continue.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptKind {
    HostKeyNew {
        host: String,
        key_type: String,
        fingerprint: String,
    },
    /// Blocking. The default answer is "no".
    HostKeyChanged {
        host: String,
        key_type: String,
        old_fingerprint: String,
        new_fingerprint: String,
    },
    Password {
        host: String,
        user: String,
    },
    /// An encrypted private key needs its passphrase.
    Passphrase {
        key_label: String,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    State { state: SessionState },
    Prompt { prompt_id: String, kind: PromptKind },
    Error { code: ErrorCode, message: String },
    Exit { status: Option<u32> },
}

/// The UI's answer to a prompt. `accept: false` is Cancel. `text` carries a password.
#[derive(Debug, Deserialize, Type)]
pub struct PromptAnswer {
    pub accept: bool,
    pub text: Option<String>,
    pub remember: bool,
}

/// Raw terminal bytes. Sent over the Tauri channel as a binary body (an `ArrayBuffer` in JS),
/// not as a JSON array of numbers. The `Type` derive only exists for the generated bindings.
#[derive(Type)]
#[specta(transparent)]
pub struct RawBytes(pub Vec<u8>);

impl tauri::ipc::IpcResponse for RawBytes {
    fn body(self) -> tauri::Result<tauri::ipc::InvokeResponseBody> {
        Ok(tauri::ipc::InvokeResponseBody::Raw(self.0))
    }
}

/// Where a session sends things. Closures keep the SSH code free of Tauri types, so tests can
/// collect events in a channel instead.
pub struct SessionIo {
    pub event: Box<dyn Fn(SessionEvent) + Send + Sync>,
    pub data: Box<dyn Fn(Vec<u8>) + Send + Sync>,
}

pub enum Cmd {
    Write(Vec<u8>),
    Resize(u32, u32),
    /// The UI finished drawing this many bytes (backpressure).
    Ack(usize),
    Close,
}

const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Prompts waiting for an answer, by id.
#[derive(Default)]
pub struct Prompts {
    pending: Mutex<HashMap<String, oneshot::Sender<PromptAnswer>>>,
}

impl Prompts {
    /// Emits the prompt and waits. `None` = cancelled, timed out, or the session ended.
    pub async fn ask(&self, io: &SessionIo, kind: PromptKind) -> Option<PromptAnswer> {
        let id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().ok()?.insert(id.clone(), tx);
        (io.event)(SessionEvent::Prompt {
            prompt_id: id.clone(),
            kind,
        });
        let answer = tokio::time::timeout(PROMPT_TIMEOUT, rx)
            .await
            .ok()
            .and_then(Result::ok);
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
        answer.filter(|a| a.accept)
    }

    pub fn answer(&self, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        let tx = self
            .pending
            .lock()
            .map_err(|_| AppError::internal("prompt lock poisoned"))?
            .remove(prompt_id)
            .ok_or_else(|| AppError::not_found("That question is no longer open."))?;
        let _ = tx.send(answer); // the session may have ended meanwhile; nothing to do then
        Ok(())
    }

    /// Dropping the senders makes every waiting `ask` return `None`.
    fn cancel_all(&self) {
        if let Ok(mut p) = self.pending.lock() {
            p.clear();
        }
    }
}

/// What a task does once connected.
#[derive(Clone)]
pub enum Job {
    Shell,
    /// S08: install a public key in `authorized_keys`.
    Export {
        key_id: String,
        dir: String,
        file: String,
    },
}

struct Entry {
    job: Job,
    host_id: String,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    io: Arc<SessionIo>,
    prompts: Arc<Prompts>,
    size: (u32, u32),
    /// True while the task is alive. Reconnect is refused until it ends.
    running: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Entry>>,
}

impl SessionManager {
    fn spawn(store: Store, e: &mut Entry) {
        let (tx, rx) = mpsc::unbounded_channel();
        e.cmd_tx = tx;
        e.running.store(true, std::sync::atomic::Ordering::SeqCst);
        let (running, io, prompts) = (
            Arc::clone(&e.running),
            Arc::clone(&e.io),
            Arc::clone(&e.prompts),
        );
        let params = client::Params {
            host_id: e.host_id.clone(),
            cols: e.size.0,
            rows: e.size.1,
        };
        let job = e.job.clone();
        tauri::async_runtime::spawn(async move {
            match job {
                Job::Shell => {
                    client::run(store, Arc::clone(&io), Arc::clone(&prompts), params, rx).await
                }
                Job::Export { key_id, dir, file } => {
                    drop(rx); // an export takes no keystrokes
                    client::run_export(
                        store,
                        Arc::clone(&io),
                        Arc::clone(&prompts),
                        params.host_id,
                        key_id,
                        dir,
                        file,
                    )
                    .await;
                }
            }
            prompts.cancel_all();
            running.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// Starts connecting in the background and returns the session id at once.
    pub fn open(
        &self,
        store: Store,
        host_id: String,
        job: Job,
        cols: u32,
        rows: u32,
        io: SessionIo,
    ) -> AppResult<String> {
        let id = uuid::Uuid::now_v7().to_string();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut entry = Entry {
            job,
            host_id,
            cmd_tx: tx,
            io: Arc::new(io),
            prompts: Arc::new(Prompts::default()),
            size: (cols, rows),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        Self::spawn(store, &mut entry);
        self.lock()?.insert(id.clone(), entry);
        Ok(id)
    }

    /// F11: a new connection for the same tab. Only when the old one has ended.
    pub fn reconnect(&self, store: Store, id: &str) -> AppResult<()> {
        let mut map = self.lock()?;
        let e = map
            .get_mut(id)
            .ok_or_else(|| AppError::not_found("That session is closed."))?;
        if e.running.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::validation("That session is still connected."));
        }
        Self::spawn(store, e);
        Ok(())
    }

    pub fn send(&self, id: &str, cmd: Cmd) -> AppResult<()> {
        let mut map = self.lock()?;
        let e = map
            .get_mut(id)
            .ok_or_else(|| AppError::not_found("That session is closed."))?;
        if let Cmd::Resize(c, r) = cmd {
            e.size = (c, r); // remembered for a later reconnect
        }
        // A send error only means the task already ended; the UI has been told.
        let _ = e.cmd_tx.send(cmd);
        Ok(())
    }

    pub fn respond(&self, id: &str, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        let prompts = {
            let map = self.lock()?;
            Arc::clone(
                &map.get(id)
                    .ok_or_else(|| AppError::not_found("That session is closed."))?
                    .prompts,
            )
        };
        prompts.answer(prompt_id, answer)
    }

    pub fn close(&self, id: &str) {
        if let Ok(mut map) = self.sessions.lock() {
            if let Some(e) = map.remove(id) {
                let _ = e.cmd_tx.send(Cmd::Close);
                e.prompts.cancel_all();
            }
        }
    }

    /// Vault locked: every session ends.
    pub fn close_all(&self) {
        if let Ok(mut map) = self.sessions.lock() {
            for (_, e) in map.drain() {
                let _ = e.cmd_tx.send(Cmd::Close);
                e.prompts.cancel_all();
            }
        }
    }

    /// Hosts with a live session (to block deleting them).
    pub fn has_live_session_for(&self, host_id: &str) -> bool {
        self.sessions.lock().is_ok_and(|m| {
            m.values().any(|e| {
                e.host_id == host_id && e.running.load(std::sync::atomic::Ordering::SeqCst)
            })
        })
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, Entry>>> {
        self.sessions
            .lock()
            .map_err(|_| AppError::internal("session lock poisoned"))
    }
}
