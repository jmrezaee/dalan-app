//! Session logs (S15): what a terminal printed, kept encrypted on disk. One "recent" log per host
//! (the previous one is deleted when a newer session ends) plus any log the user bookmarks.
//!
//! File format, in `<app data>/logs/<id>.dlog`: repeated `[u32 big-endian length][24-byte nonce]
//! [ciphertext]`, each chunk sealed with the vault key (aad = log id + chunk number, so chunks
//! can't be moved or reordered). Only terminal output is kept, as raw bytes: replaying them in a
//! read-only terminal reproduces what the screen showed.
//!
//! Logs are off until the user turns them on in Settings. After a master-password change, older
//! logs can't be opened (the key changed); they can still be deleted.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::state::Store;
use crate::vault::kdf::Key;
use crate::vault::{crypto, VaultSession};

/// A session that prints more than this stops being recorded (the log says so by ending there).
const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;
const CHUNK_AT: usize = 32 * 1024;
const MAX_COMMENT: usize = 500;

#[derive(Debug, Clone, Serialize, Type)]
pub struct LogSummary {
    pub id: String,
    pub host_id: Option<String>,
    /// `None` for the local terminal.
    pub host_label: Option<String>,
    #[specta(type = f64)]
    pub started_at: i64,
    #[specta(type = Option<f64>)]
    pub ended_at: Option<i64>,
    #[specta(type = f64)]
    pub size_bytes: u64,
    pub bookmarked: bool,
    pub comment: Option<String>,
}

pub fn file_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.dlog"))
}

fn aad(id: &str, seq: u64) -> Vec<u8> {
    format!("{id}:{seq}").into_bytes()
}

// ------------------------------------------------------------------ file writer and reader

/// Appends sealed chunks. Not thread-safe by itself: `LogTap` wraps it in a mutex.
pub struct Recorder {
    id: String,
    file: File,
    key: Key,
    seq: u64,
    pending: Vec<u8>,
    /// Plaintext bytes accepted so far.
    written: u64,
}

impl Recorder {
    pub fn create(dir: &Path, id: &str, key: Key) -> std::io::Result<Self> {
        crate::perms::private_dir(dir)?;
        let path = file_path(dir, id);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        crate::perms::private_file(&path)?;
        Ok(Recorder {
            id: id.into(),
            file,
            key,
            seq: 0,
            pending: Vec::new(),
            written: 0,
        })
    }

    pub fn push(&mut self, bytes: &[u8]) {
        let room = MAX_LOG_BYTES.saturating_sub(self.written) as usize;
        let take = &bytes[..bytes.len().min(room)];
        self.pending.extend_from_slice(take);
        self.written += take.len() as u64;
        if self.pending.len() >= CHUNK_AT {
            self.flush();
        }
    }

    /// Seals and writes what is buffered. A disk error drops the chunk: a log is best effort and
    /// must never break the terminal.
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let plain = std::mem::take(&mut self.pending);
        let Ok((nonce, ct)) = crypto::seal(&self.key, &aad(&self.id, self.seq), &plain) else {
            return;
        };
        self.seq += 1;
        let mut out = Vec::with_capacity(4 + nonce.len() + ct.len());
        out.extend_from_slice(&((nonce.len() + ct.len()) as u32).to_be_bytes());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        let _ = self.file.write_all(&out);
    }

    pub fn size(&self) -> u64 {
        self.written
    }
}

/// Decrypts a log chunk by chunk into `sink`.
pub fn read_chunks(
    dir: &Path,
    id: &str,
    key: &Key,
    mut sink: impl FnMut(Vec<u8>),
) -> AppResult<()> {
    let mut f = File::open(file_path(dir, id))
        .map_err(|_| AppError::not_found("That log's file is gone."))?;
    let cant =
        || AppError::internal("This log can't be opened. It may predate a master password change.");
    let mut seq = 0u64;
    loop {
        let mut len = [0u8; 4];
        match f.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(_) => return Err(cant()),
        }
        let len = u32::from_be_bytes(len) as usize;
        if !(crypto::NONCE_LEN..=CHUNK_AT * 2 + 64).contains(&len) {
            return Err(cant());
        }
        let mut buf = vec![0u8; len];
        // A half-written last chunk (crash) just ends the log.
        if f.read_exact(&mut buf).is_err() {
            return Ok(());
        }
        let plain = crypto::open(
            key,
            &aad(id, seq),
            &buf[..crypto::NONCE_LEN],
            &buf[crypto::NONCE_LEN..],
        )
        .ok_or_else(cant)?;
        sink(plain.to_vec());
        seq += 1;
    }
}

// ------------------------------------------------------------------ database

pub fn enabled(conn: &Connection) -> bool {
    crate::settings::get_all(conn)
        .ok()
        .and_then(|m| {
            m.get("logs")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        })
        .and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

pub fn begin(conn: &Connection, s: &VaultSession, host_id: Option<&str>) -> AppResult<String> {
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO session_logs (id, vault_id, host_id, started_at, file_name) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, s.vault_id, host_id, now_ms(), format!("{id}.dlog")],
    )?;
    Ok(id)
}

fn remove(conn: &Connection, dir: &Path, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM session_logs WHERE id = ?1", [id])?;
    let _ = std::fs::remove_file(file_path(dir, id));
    Ok(())
}

/// A session ended (or paused): record the size, drop an empty log, and delete the host's older
/// non-bookmarked logs.
pub fn finish(conn: &Connection, vault_id: &str, dir: &Path, id: &str, size: u64) -> AppResult<()> {
    if size == 0 {
        return remove(conn, dir, id);
    }
    let host: Option<Option<String>> = conn
        .query_row(
            "SELECT host_id FROM session_logs WHERE id = ?1 AND vault_id = ?2",
            params![id, vault_id],
            |r| r.get(0),
        )
        .ok();
    let Some(host) = host else { return Ok(()) }; // deleted by the user meanwhile
    conn.execute(
        "UPDATE session_logs SET ended_at = ?1, size_bytes = ?2 WHERE id = ?3",
        params![now_ms(), size as i64, id],
    )?;
    let old: Vec<String> = {
        let mut st = conn.prepare(
            "SELECT id FROM session_logs WHERE vault_id = ?1 AND host_id IS ?2 AND bookmarked = 0 AND ended_at IS NOT NULL AND id <> ?3",
        )?;
        let rows = st.query_map(params![vault_id, host, id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for o in old {
        remove(conn, dir, &o)?;
    }
    Ok(())
}

pub fn list(conn: &Connection, vault_id: &str, dir: &Path) -> AppResult<Vec<LogSummary>> {
    let mut st = conn.prepare(
        "SELECT l.id, l.host_id, h.label, l.started_at, l.ended_at, l.size_bytes, l.bookmarked, l.comment
         FROM session_logs l LEFT JOIN hosts h ON h.id = l.host_id WHERE l.vault_id = ?1 ORDER BY l.started_at DESC",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            let id: String = r.get(0)?;
            let size: i64 = r.get(5)?;
            Ok(LogSummary {
                // A log the app never got to close (quit mid-session): size comes from the file.
                size_bytes: if size > 0 {
                    size as u64
                } else {
                    std::fs::metadata(file_path(dir, &id)).map_or(0, |m| m.len())
                },
                id,
                host_id: r.get(1)?,
                host_label: r.get(2)?,
                started_at: r.get(3)?,
                ended_at: r.get(4)?,
                bookmarked: r.get::<_, i64>(6)? != 0,
                comment: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn set_bookmark(
    conn: &Connection,
    vault_id: &str,
    id: &str,
    bookmarked: bool,
    comment: Option<&str>,
) -> AppResult<()> {
    let comment = comment.map(str::trim).filter(|c| !c.is_empty());
    if comment.is_some_and(|c| c.chars().count() > MAX_COMMENT) {
        return Err(AppError::validation(format!(
            "Comments can be up to {MAX_COMMENT} characters."
        )));
    }
    let n = conn.execute(
        "UPDATE session_logs SET bookmarked = ?1, comment = ?2 WHERE id = ?3 AND vault_id = ?4",
        params![i64::from(bookmarked), comment, id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That log no longer exists."));
    }
    Ok(())
}

pub fn delete(conn: &Connection, vault_id: &str, dir: &Path, id: &str) -> AppResult<()> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM session_logs WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(AppError::not_found("That log no longer exists."));
    }
    remove(conn, dir, id)
}

// ------------------------------------------------------------------ tap on a live session

/// Sits between a session and the UI: copies output into a log while the session runs.
pub struct LogTap {
    store: Store,
    dir: PathBuf,
    id: String,
    rec: Mutex<Option<Recorder>>,
    key: Mutex<Option<Key>>,
}

impl LogTap {
    /// `None` when logging is off. Otherwise inserts the log row (the file appears with the first output).
    pub async fn start(
        store: &Store,
        dir: PathBuf,
        host_id: Option<String>,
    ) -> Option<Arc<LogTap>> {
        let (id, key) = store
            .with_db_session(move |c, s| {
                if !enabled(c) {
                    return Ok(None);
                }
                Ok(Some((begin(c, s, host_id.as_deref())?, s.key.clone())))
            })
            .await
            .ok()
            .flatten()?;
        Some(Arc::new(LogTap {
            store: store.clone(),
            dir,
            id,
            rec: Mutex::new(None),
            key: Mutex::new(Some(key)),
        }))
    }

    pub fn data(&self, bytes: &[u8]) {
        let Ok(mut rec) = self.rec.lock() else { return };
        if rec.is_none() {
            let Some(key) = self.key.lock().ok().and_then(|k| k.clone()) else {
                return;
            };
            *rec = Recorder::create(&self.dir, &self.id, key).ok();
        }
        if let Some(r) = rec.as_mut() {
            r.push(bytes);
        }
    }

    /// The session ended or dropped: flush, then update the row in the background.
    pub fn ended(self: &Arc<Self>) {
        let size = match self.rec.lock() {
            Ok(mut rec) => rec.as_mut().map_or(0, |r| {
                r.flush();
                r.size()
            }),
            Err(_) => return,
        };
        let (store, dir, id) = (self.store.clone(), self.dir.clone(), self.id.clone());
        tauri::async_runtime::spawn(async move {
            // Fails when the vault was locked meanwhile; the row is then closed at the next start.
            let _ = store
                .with_db_session(move |c, s| finish(c, &s.vault_id, &dir, &id, size))
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!("dalan-logs-{}", uuid::Uuid::now_v7()))
    }

    #[test]
    fn recorder_roundtrip_is_encrypted_and_tamper_proof() {
        let dir = temp();
        let key: Key = zeroize::Zeroizing::new([7u8; 32]);
        let mut r = Recorder::create(&dir, "log1", key.clone()).unwrap();
        let big = vec![b'x'; CHUNK_AT + 10]; // forces two chunks
        r.push(b"hello ");
        r.push(&big);
        r.flush();
        assert_eq!(r.size(), 6 + big.len() as u64);
        let raw = std::fs::read(file_path(&dir, "log1")).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"hello")); // not plaintext
        let mut got = Vec::new();
        read_chunks(&dir, "log1", &key, |c| got.extend(c)).unwrap();
        assert_eq!(&got[..6], b"hello ");
        assert_eq!(got.len(), 6 + big.len());
        // Wrong key, or a renamed file (aad includes the id), can't be read.
        assert!(read_chunks(&dir, "log1", &zeroize::Zeroizing::new([8u8; 32]), |_| {}).is_err());
        std::fs::rename(file_path(&dir, "log1"), file_path(&dir, "log2")).unwrap();
        assert!(read_chunks(&dir, "log2", &key, |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retention_keeps_one_recent_per_host_plus_bookmarks() {
        let c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |size| {
            let id = begin(&c, &s, None).unwrap();
            std::fs::write(file_path(&dir, &id), b"x").unwrap();
            finish(&c, &s.vault_id, &dir, &id, size).unwrap();
            id
        };
        let a = mk(10);
        set_bookmark(&c, &s.vault_id, &a, true, Some("  keep me ")).unwrap();
        let b = mk(10);
        let b2 = mk(10); // replaces b
        let empty = mk(0); // never recorded anything: removed at once
        let ids: Vec<String> = list(&c, &s.vault_id, &dir)
            .unwrap()
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert!(ids.contains(&a) && ids.contains(&b2));
        assert!(!ids.contains(&b) && !ids.contains(&empty));
        assert!(!file_path(&dir, &b).exists());
        let kept = list(&c, &s.vault_id, &dir)
            .unwrap()
            .into_iter()
            .find(|l| l.id == a)
            .unwrap();
        assert_eq!(kept.comment.as_deref(), Some("keep me"));
        delete(&c, &s.vault_id, &dir, &a).unwrap();
        assert!(delete(&c, &s.vault_id, &dir, &a).is_err());
        assert!(!enabled(&c)); // off until Settings turns it on
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tap_records_output_only_when_enabled_and_finishes_the_row() {
        use crate::state::AppState;
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let key = s.key.clone();
        let state = AppState::new(conn);
        state.set_session(Some(s));
        let dir = temp();
        // Off by default: no tap, no row.
        assert!(LogTap::start(&state.store(), dir.clone(), None)
            .await
            .is_none());
        state
            .with_db(|c| crate::settings::set(c, "logs", r#"{"enabled":true}"#))
            .await
            .unwrap();
        let tap = LogTap::start(&state.store(), dir.clone(), None)
            .await
            .unwrap();
        tap.data(b"hello ");
        tap.data(b"world");
        tap.ended();
        // `ended` finishes the row in the background.
        let list = loop {
            let l = state
                .with_db_session({
                    let d = dir.clone();
                    move |c, s| list(c, &s.vault_id, &d)
                })
                .await
                .unwrap();
            if l.first().is_some_and(|l| l.ended_at.is_some()) {
                break l;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert_eq!(list[0].size_bytes, 11);
        let mut got = Vec::new();
        read_chunks(&dir, &list[0].id, &key, |c| got.extend(c)).unwrap();
        assert_eq!(got, b"hello world");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
