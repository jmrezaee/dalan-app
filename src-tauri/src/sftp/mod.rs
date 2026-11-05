//! SFTP and the file panes (F07; S21, S22). Two kinds of place share one interface: the local
//! disk and a remote host over SFTP. Every file operation runs here in Rust; the webview only sees
//! names, sizes and progress.

mod edit;
mod fs;
mod transfer;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};
use crate::ssh::{PromptAnswer, Prompts};

pub use edit::{EditEmit, EditEvent, EditManager, EditUpload};
pub use fs::{Fs, RemoteFs};
pub use transfer::{Choice, Emit as TransferEmit, TransferEvent, TransferItem, TransferManager};

/// One copy from `src_path` on `src` into the folder `dst_dir` on `dst`.
pub fn transfer_job(src: Fs, src_path: String, dst: Fs, dst_dir: String) -> transfer::Job {
    transfer::Job {
        src,
        src_path,
        dst,
        dst_dir,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Other,
}

/// One row in a file pane.
#[derive(Debug, Clone, Serialize, Type)]
pub struct FsEntry {
    pub name: String,
    pub kind: EntryKind,
    /// A symbolic link (`kind` describes what it points at).
    pub symlink: bool,
    /// Bytes. Exported as `number`: file sizes stay far below 2^53.
    #[specta(type = f64)]
    pub size: u64,
    /// Modified time, epoch ms.
    #[specta(type = Option<f64>)]
    pub mtime: Option<i64>,
    /// Unix permission bits (e.g. 0o644), if the place reports them.
    pub mode: Option<u32>,
}

/// Open SFTP sessions by id, and the connections still waiting for their questions to be answered.
#[derive(Default)]
pub struct SftpManager {
    sessions: Mutex<HashMap<String, Arc<RemoteFs>>>,
    opening: Mutex<HashMap<String, Arc<Prompts>>>,
    pub transfers: TransferManager,
    pub edits: EditManager,
}

impl SftpManager {
    pub fn add(&self, id: String, fs: Arc<RemoteFs>) {
        if let Ok(mut m) = self.sessions.lock() {
            m.insert(id, fs);
        }
    }

    pub fn remove(&self, id: &str) {
        let removed = self.sessions.lock().ok().and_then(|mut m| m.remove(id));
        drop(removed); // dropping the session closes its SSH connection
    }

    /// Vault locked: every SFTP session and transfer ends.
    pub fn close_all(&self) {
        if let Ok(mut m) = self.sessions.lock() {
            m.clear();
        }
        // Connections still asking questions (host key, password) must stop asking.
        if let Ok(m) = self.opening.lock() {
            for p in m.values() {
                p.cancel_all_pub();
            }
        }
        self.transfers.cancel_all();
        self.edits.close_all();
    }

    /// Turns `"local"` or an sftp id into something to run operations on.
    pub fn fs(&self, endpoint: &str) -> AppResult<Fs> {
        if endpoint == "local" {
            return Ok(Fs::Local);
        }
        let m = self
            .sessions
            .lock()
            .map_err(|_| AppError::internal("sftp lock poisoned"))?;
        m.get(endpoint)
            .map(|r| Fs::Remote(Arc::clone(r)))
            .ok_or_else(|| AppError::not_found("That connection is closed. Open the server again."))
    }

    pub fn begin_open(&self, open_id: &str, prompts: Arc<Prompts>) {
        if let Ok(mut m) = self.opening.lock() {
            m.insert(open_id.to_string(), prompts);
        }
    }

    pub fn end_open(&self, open_id: &str) {
        if let Ok(mut m) = self.opening.lock() {
            m.remove(open_id);
        }
    }

    pub fn respond(&self, open_id: &str, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        let prompts = {
            let m = self
                .opening
                .lock()
                .map_err(|_| AppError::internal("sftp lock poisoned"))?;
            Arc::clone(
                m.get(open_id)
                    .ok_or_else(|| AppError::not_found("That connection isn't opening."))?,
            )
        };
        prompts.answer(prompt_id, answer)
    }
}

#[cfg(test)]
mod tests;

/// Connects to a host (through its jump hosts, asking the usual questions on `io`) and starts the
/// SFTP subsystem on a new channel.
pub async fn open_remote(
    store: &crate::state::Store,
    io: &Arc<crate::ssh::SessionIo>,
    prompts: &Arc<Prompts>,
    host_id: &str,
) -> AppResult<RemoteFs> {
    let conn = crate::ssh::connect_authenticated(store, io, prompts, host_id, None, false)
        .await
        .ok_or_else(|| {
            AppError::new(
                crate::error::ErrorCode::HostUnreachable,
                "Couldn't connect to that server.",
            )
        })?;
    let label = conn.label.clone();
    let channel = conn
        .handle
        .channel_open_session()
        .await
        .map_err(|e| AppError::internal(format!("Couldn't open a channel on {label}: {e}")))?;
    channel.request_subsystem(true, "sftp").await.map_err(|_| {
        AppError::new(
            crate::error::ErrorCode::SftpDisabled,
            format!("SFTP is turned off on {label}."),
        )
    })?;
    let sftp = russh_sftp::client::SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError::internal(format!("{label} didn't start SFTP: {e}")))?;
    Ok(RemoteFs::new(sftp, label, conn))
}
