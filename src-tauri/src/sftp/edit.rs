//! Edit a server file with a local app (F07): download to a temp folder, open it, notice saves,
//! upload on request. `Fs::Local` works as the "server" too, which is how the tests run.
//!
//! The watcher polls the temp file's modified time and size instead of using OS file events:
//! one file per edit, so a 0.7 s poll is cheap and behaves the same on every platform.
//! ponytail: polling, switch to `notify` if many simultaneous edits ever matter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use specta::Type;
use tokio::task::JoinHandle;

use super::fs::{basename, Fs};
use super::EntryKind;
use crate::error::{AppError, AppResult};

const POLL: Duration = Duration::from_millis(700);
/// Bigger files aren't something to open in an editor.
const MAX_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditEvent {
    /// The local copy was saved. The UI asks whether to upload it.
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum EditUpload {
    Uploaded,
    /// The file on the server changed since it was downloaded. Upload again with `force` to overwrite.
    RemoteChanged,
}

pub type EditEmit = Arc<dyn Fn(EditEvent) + Send + Sync>;

struct Edit {
    fs: Fs,
    remote_path: String,
    local: PathBuf,
    /// The server file's modified time when we last downloaded or uploaded it.
    remote_mtime: Arc<Mutex<Option<i64>>>,
    watcher: JoinHandle<()>,
}

#[derive(Default)]
pub struct EditManager {
    edits: Mutex<HashMap<String, Edit>>,
}

/// Modified time + size: a save changes at least one of them.
fn signature(p: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl EditManager {
    /// Downloads `remote_path` under `tmp_root` and starts watching it. Returns (edit id, local path).
    pub async fn open(
        &self,
        fs: Fs,
        remote_path: &str,
        tmp_root: &Path,
        emit: EditEmit,
    ) -> AppResult<(String, PathBuf)> {
        let stat = fs.stat(remote_path).await?;
        if stat.kind != EntryKind::File {
            return Err(AppError::validation("Only files can be edited."));
        }
        if stat.size > MAX_BYTES {
            return Err(AppError::validation(
                "That file is too big to edit (over 50 MB).",
            ));
        }
        let name = basename(&fs, remote_path);
        if name.is_empty() || name == ".." || name == "." || name.contains(['/', '\\']) {
            return Err(AppError::validation("That file name can't be edited."));
        }
        let id = uuid::Uuid::now_v7().to_string();
        let dir = tmp_root.join(&id);
        let io =
            |e: std::io::Error| AppError::internal(format!("Couldn't save a temporary copy: {e}"));
        tokio::fs::create_dir_all(&dir).await.map_err(io)?;
        let local = dir.join(&name);
        let download = async {
            let mut r = fs.open_read(remote_path).await?;
            let mut f = tokio::fs::File::create(&local).await.map_err(io)?;
            tokio::io::copy(&mut r, &mut f).await.map_err(io)?;
            AppResult::Ok(())
        };
        if let Err(e) = download.await {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }

        let remote_mtime = Arc::new(Mutex::new(stat.mtime));
        let watched = local.clone();
        // Baseline taken now, not inside the task, so a save right after the download still counts.
        let mut seen = signature(&watched);
        // The task owns its own copies; the manager only keeps the handle to stop it.
        let watcher = tokio::spawn(async move {
            let mut pending = None;
            loop {
                tokio::time::sleep(POLL).await;
                let now = signature(&watched);
                if now != seen {
                    // Changed since last poll: wait for it to hold still (editors write in steps).
                    seen = now;
                    pending = now;
                } else if pending.is_some() && pending == now {
                    pending = None;
                    emit(EditEvent::Changed);
                }
            }
        });
        self.edits
            .lock()
            .map_err(|_| AppError::internal("edit lock poisoned"))?
            .insert(
                id.clone(),
                Edit {
                    fs,
                    remote_path: remote_path.to_string(),
                    local: local.clone(),
                    remote_mtime,
                    watcher,
                },
            );
        Ok((id, local))
    }

    /// Sends the local copy back. Unless `force`, refuses when the server file changed meanwhile.
    pub async fn upload(&self, id: &str, force: bool) -> AppResult<EditUpload> {
        let (fs, remote_path, local, mtime) = {
            let m = self
                .edits
                .lock()
                .map_err(|_| AppError::internal("edit lock poisoned"))?;
            let e = m
                .get(id)
                .ok_or_else(|| AppError::not_found("That edit is closed."))?;
            (
                e.fs.clone(),
                e.remote_path.clone(),
                e.local.clone(),
                Arc::clone(&e.remote_mtime),
            )
        };
        let known = *mtime
            .lock()
            .map_err(|_| AppError::internal("edit lock poisoned"))?;
        if !force {
            let now = fs.stat(&remote_path).await?.mtime;
            if known.is_some() && now != known {
                return Ok(EditUpload::RemoteChanged);
            }
        }
        let io =
            |e: std::io::Error| AppError::internal(format!("Couldn't read the local copy: {e}"));
        let mut src = tokio::fs::File::open(&local).await.map_err(io)?;
        let mut dst = fs.open_write(&remote_path).await?;
        tokio::io::copy(&mut src, &mut dst).await.map_err(io)?;
        tokio::io::AsyncWriteExt::shutdown(&mut dst)
            .await
            .map_err(io)?;
        drop(dst);
        *mtime
            .lock()
            .map_err(|_| AppError::internal("edit lock poisoned"))? =
            fs.stat(&remote_path).await?.mtime;
        Ok(EditUpload::Uploaded)
    }

    /// Stops watching and deletes the temporary copy.
    pub fn close(&self, id: &str) {
        let removed = self.edits.lock().ok().and_then(|mut m| m.remove(id));
        if let Some(e) = removed {
            e.watcher.abort();
            if let Some(dir) = e.local.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    pub fn close_all(&self) {
        let ids: Vec<String> = self
            .edits
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            self.close(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn edit_roundtrip_with_remote_change_guard() {
        let root = std::env::temp_dir().join(format!("dalan-edit-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("notes.txt");
        std::fs::write(&remote, "one").unwrap();
        let (m, changes) = (EditManager::default(), Arc::new(AtomicUsize::new(0)));
        let c = Arc::clone(&changes);
        let emit: EditEmit = Arc::new(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let (id, local) = m
            .open(
                Fs::Local,
                remote.to_str().unwrap(),
                &root.join("edit-tmp"),
                emit,
            )
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "one");

        // Saving the local copy raises exactly one `Changed`, then upload copies it over.
        std::fs::write(&local, "two two").unwrap();
        tokio::time::sleep(POLL * 4).await;
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        assert_eq!(m.upload(&id, false).await.unwrap(), EditUpload::Uploaded);
        assert_eq!(std::fs::read_to_string(&remote).unwrap(), "two two");

        // Someone else changes the server file: refuse, unless forced.
        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::write(&remote, "theirs!").unwrap();
        std::fs::write(&local, "mine mine").unwrap();
        assert_eq!(
            m.upload(&id, false).await.unwrap(),
            EditUpload::RemoteChanged
        );
        assert_eq!(std::fs::read_to_string(&remote).unwrap(), "theirs!");
        assert_eq!(m.upload(&id, true).await.unwrap(), EditUpload::Uploaded);
        assert_eq!(std::fs::read_to_string(&remote).unwrap(), "mine mine");

        m.close(&id);
        assert!(!local.exists());
        assert!(m.upload(&id, false).await.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn folders_and_missing_files_are_refused() {
        let m = EditManager::default();
        let emit: EditEmit = Arc::new(|_| {});
        let dir = std::env::temp_dir();
        assert!(m
            .open(Fs::Local, dir.to_str().unwrap(), &dir, Arc::clone(&emit))
            .await
            .is_err());
        assert!(m
            .open(Fs::Local, "/definitely/not/here", &dir, emit)
            .await
            .is_err());
    }
}
