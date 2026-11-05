//! Command history (S20 History tab): the commands you typed in a terminal, per server, so you can
//! run one again from the side panel. Off until switched on in Settings.
//!
//! A terminal gives us keystrokes, not commands, so this is a best effort:
//! - A line edited with arrows, Tab or history recall isn't recorded (we can't know what the shell
//!   ended up with), nor one cancelled with Ctrl-C or Ctrl-U.
//! - A line typed right after a prompt that asks for a "password" or "passphrase" is dropped, so
//!   `sudo` and `ssh` passwords don't land in the history. It's a text match on the prompt, not a
//!   guarantee.
//! - A line that carries an obvious secret (`--password=…`, `TOKEN=…`, `mysql -pSECRET`,
//!   `sshpass -p …`, a bearer header) is dropped too (see `looks_secret`).
//!
//! Commands are encrypted with the vault's data key (`command_enc`). Rows written before that
//! (plain `command`) are encrypted the next time history is read or written.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use crate::db::now_ms;
use crate::error::AppResult;
use crate::state::Store;
use crate::vault::{crypto, VaultSession};

const MAX_COMMAND: usize = 2000;
/// Newest commands kept per server.
const KEEP_PER_HOST: i64 = 500;
const TAIL: usize = 96;

/// Turns the bytes of one session into finished command lines.
#[derive(Default)]
pub struct Tracker {
    line: Vec<u8>,
    /// The line was edited in a way we can't follow (escape sequence, Tab).
    unknown: bool,
    /// The last output looked like a password prompt.
    secret: bool,
    /// The end of what the terminal printed.
    tail: Vec<u8>,
}

impl Tracker {
    /// What the terminal printed. Marks the next typed line as secret after a password prompt.
    pub fn output(&mut self, bytes: &[u8]) {
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > TAIL * 2 {
            self.tail.drain(..self.tail.len() - TAIL);
        }
        let shown = String::from_utf8_lossy(&self.tail).to_lowercase();
        let shown = shown.trim_end();
        if shown.ends_with(':') && (shown.contains("password") || shown.contains("passphrase")) {
            self.secret = true;
        }
    }

    /// What the user typed. Returns the lines completed by Enter that are worth keeping.
    pub fn input(&mut self, bytes: &[u8]) -> Vec<String> {
        // Pasted text arrives wrapped in these markers: they are not edits.
        let cleaned: Vec<u8> = {
            let s = bytes.to_vec();
            let mut out = s.clone();
            for marker in [&b"\x1b[200~"[..], &b"\x1b[201~"[..]] {
                while let Some(i) = out.windows(marker.len()).position(|w| w == marker) {
                    out.drain(i..i + marker.len());
                }
            }
            out
        };
        let mut done = Vec::new();
        for &b in &cleaned {
            match b {
                b'\r' | b'\n' => {
                    let keep = !self.unknown && !self.secret;
                    let text = String::from_utf8_lossy(&self.line).trim().to_string();
                    if keep && !text.is_empty() && text.len() <= MAX_COMMAND {
                        done.push(text);
                    }
                    self.line.clear();
                    self.unknown = false;
                    self.secret = false;
                }
                0x7f | 0x08 => {
                    // Backspace removes a whole UTF-8 character, not one byte.
                    while let Some(c) = self.line.pop() {
                        if c & 0xC0 != 0x80 {
                            break;
                        }
                    }
                }
                0x03 | 0x15 => {
                    self.line.clear();
                    self.unknown = false;
                }
                0x1b | 0x09 => self.unknown = true,
                b if b < 0x20 => {}
                b => self.line.push(b),
            }
        }
        done
    }
}

/// A session's tracker plus where to save what it finds.
pub struct HistoryTap {
    store: Store,
    host_id: Option<String>,
    pub tracker: Mutex<Tracker>,
}

impl HistoryTap {
    /// `None` when history is off.
    pub async fn start(store: &Store, host_id: Option<String>) -> Option<Arc<HistoryTap>> {
        let on = store.with_db(|c| Ok(enabled(c))).await.unwrap_or(false);
        on.then(|| {
            Arc::new(HistoryTap {
                store: store.clone(),
                host_id,
                tracker: Mutex::default(),
            })
        })
    }

    pub fn output(&self, bytes: &[u8]) {
        if let Ok(mut t) = self.tracker.lock() {
            t.output(bytes);
        }
    }

    pub fn input(&self, bytes: &[u8]) {
        let lines = self
            .tracker
            .lock()
            .map(|mut t| t.input(bytes))
            .unwrap_or_default();
        if lines.is_empty() {
            return;
        }
        let (store, host) = (self.store.clone(), self.host_id.clone());
        tauri::async_runtime::spawn(async move {
            let _ = store
                .with_db_session(move |c, s| {
                    for l in &lines {
                        record(c, s, host.as_deref(), l)?;
                    }
                    Ok(())
                })
                .await;
        });
    }
}

/// Open sessions' history taps, by session id. Cleared when the vault locks.
#[derive(Default)]
pub struct HistoryManager {
    taps: Mutex<HashMap<String, Arc<HistoryTap>>>,
}

impl HistoryManager {
    pub fn add(&self, session_id: &str, tap: Arc<HistoryTap>) {
        if let Ok(mut m) = self.taps.lock() {
            m.insert(session_id.to_string(), tap);
        }
    }
    pub fn remove(&self, session_id: &str) {
        if let Ok(mut m) = self.taps.lock() {
            m.remove(session_id);
        }
    }
    pub fn clear(&self) {
        if let Ok(mut m) = self.taps.lock() {
            m.clear();
        }
    }
    pub fn input(&self, session_id: &str, bytes: &[u8]) {
        let tap = self
            .taps
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).cloned());
        if let Some(t) = tap {
            t.input(bytes);
        }
    }
}

pub fn enabled(conn: &Connection) -> bool {
    crate::settings::get_all(conn)
        .ok()
        .and_then(|m| {
            m.get("history")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        })
        .and_then(|v| v.get("enabled").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

// ------------------------------------------------------------------ storage

#[derive(Debug, Serialize, Type)]
pub struct HistoryEntry {
    pub id: String,
    pub command: String,
    #[specta(type = f64)]
    pub ran_at: i64,
}

/// True for a line that obviously carries a credential. A text match, not a guarantee.
pub fn looks_secret(line: &str) -> bool {
    let l = line.to_lowercase();
    const MARKERS: [&str; 14] = [
        "--password",
        "--passwd",
        "--token",
        "--secret",
        "--api-key",
        "password=",
        "passwd=",
        "token=",
        "secret=",
        "apikey=",
        "api_key=",
        "authorization:",
        "bearer ",
        "sshpass",
    ];
    if MARKERS.iter().any(|m| l.contains(m)) {
        return true;
    }
    // `mysql -pSECRET` (the value is glued to the flag). `-p` alone, or `-p 22`, is harmless.
    let mut words = l.split_whitespace();
    let first = words.next().unwrap_or("");
    matches!(first, "mysql" | "mysqldump" | "mysqladmin" | "mariadb")
        && words.any(|w| w.starts_with("-p") && w.len() > 2 && !w.starts_with("--"))
}

fn seal_command(s: &VaultSession, id: &str, command: &str) -> AppResult<Vec<u8>> {
    let (nonce, ct) = crypto::seal(&s.data_key, id.as_bytes(), command.as_bytes())?;
    Ok([nonce, ct].concat())
}

fn open_command(s: &VaultSession, id: &str, blob: &[u8]) -> Option<String> {
    if blob.len() <= crypto::NONCE_LEN {
        return None;
    }
    let (nonce, ct) = blob.split_at(crypto::NONCE_LEN);
    let plain = crypto::open(&s.data_key, id.as_bytes(), nonce, ct)?;
    String::from_utf8(plain.to_vec()).ok()
}

/// Encrypts rows saved as plain text before the data key existed.
fn encrypt_legacy(conn: &Connection, s: &VaultSession) -> AppResult<()> {
    let rows: Vec<(String, String)> = {
        let mut st = conn.prepare(
            "SELECT id, command FROM command_history WHERE command_enc IS NULL AND command != ''",
        )?;
        let mapped = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        mapped.collect::<Result<_, _>>()?
    };
    for (id, command) in rows {
        conn.execute(
            "UPDATE command_history SET command = '', command_enc = ?1 WHERE id = ?2",
            params![seal_command(s, &id, &command)?, id],
        )?;
    }
    Ok(())
}

fn row_command(s: &VaultSession, id: &str, plain: String, enc: Option<Vec<u8>>) -> Option<String> {
    match enc {
        Some(blob) => open_command(s, id, &blob),
        None => Some(plain),
    }
}

/// Saves a command for a server (`None` = local terminal). The same command twice in a row is
/// kept once, and only the newest `KEEP_PER_HOST` stay. Lines that look like secrets are skipped.
pub fn record(
    conn: &Connection,
    s: &VaultSession,
    host_id: Option<&str>,
    command: &str,
) -> AppResult<()> {
    if looks_secret(command) {
        return Ok(());
    }
    encrypt_legacy(conn, s)?;
    let last: Option<(String, Option<Vec<u8>>, String)> = conn
        .query_row(
            "SELECT command, command_enc, id FROM command_history WHERE host_id IS ?1 ORDER BY ran_at DESC, rowid DESC LIMIT 1",
            params![host_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    if let Some((plain, enc, id)) = last {
        if row_command(s, &id, plain, enc).as_deref() == Some(command) {
            return Ok(());
        }
    }
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO command_history (id, host_id, command, command_enc, ran_at) VALUES (?1, ?2, '', ?3, ?4)",
        params![id, host_id, seal_command(s, &id, command)?, now_ms()],
    )?;
    conn.execute(
        "DELETE FROM command_history WHERE host_id IS ?1 AND rowid NOT IN (SELECT rowid FROM command_history WHERE host_id IS ?1 ORDER BY ran_at DESC, rowid DESC LIMIT ?2)",
        params![host_id, KEEP_PER_HOST],
    )?;
    Ok(())
}

/// Newest first. `query` filters by substring (case-insensitive; done here, after decrypting).
pub fn list(
    conn: &Connection,
    s: &VaultSession,
    host_id: Option<&str>,
    query: Option<&str>,
    limit: u32,
) -> AppResult<Vec<HistoryEntry>> {
    encrypt_legacy(conn, s)?;
    let needle = query.unwrap_or("").to_lowercase();
    let mut st = conn.prepare(
        "SELECT id, command, command_enc, ran_at FROM command_history WHERE host_id IS ?1 ORDER BY ran_at DESC, rowid DESC",
    )?;
    let mut rows = st.query_map(params![host_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<Vec<u8>>>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    let max = limit.min(500) as usize;
    while out.len() < max {
        let Some(row) = rows.next() else { break };
        let (id, plain, enc, ran_at) = row?;
        let Some(command) = row_command(s, &id, plain, enc) else {
            continue; // can't be opened (should not happen): skip rather than fail the whole list
        };
        if needle.is_empty() || command.to_lowercase().contains(&needle) {
            out.push(HistoryEntry {
                id,
                command,
                ran_at,
            });
        }
    }
    Ok(out)
}

/// One server's history, or everything.
pub fn clear(conn: &Connection, host_id: Option<&str>, all: bool) -> AppResult<()> {
    if all {
        conn.execute("DELETE FROM command_history", [])?;
    } else {
        conn.execute(
            "DELETE FROM command_history WHERE host_id IS ?1",
            params![host_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn typed(t: &mut Tracker, s: &str) -> Vec<String> {
        t.input(s.as_bytes())
    }

    #[test]
    fn tracker_keeps_plain_lines_and_drops_edited_and_secret_ones() {
        let mut t = Tracker::default();
        assert_eq!(typed(&mut t, "ls -la\r"), vec!["ls -la"]);
        assert_eq!(typed(&mut t, "gti\x7f\x7fit status\r"), vec!["git status"]); // backspace
        assert_eq!(typed(&mut t, "rm -rf x\x03"), Vec::<String>::new()); // Ctrl-C cancels...
        assert_eq!(typed(&mut t, "pwd\r"), vec!["pwd"]); // ...and the next line is clean
        assert!(typed(&mut t, "\x1b[A\r").is_empty()); // history recall: unknown
        assert!(typed(&mut t, "cd /et\tc\r").is_empty()); // Tab completion: unknown
        assert!(typed(&mut t, "   \r").is_empty());
        assert_eq!(typed(&mut t, "echo hé\x7f!\r"), vec!["echo h!"]); // backspace removes a whole character
                                                                      // Pasted text keeps its lines; the paste markers are ignored.
        assert_eq!(
            typed(&mut t, "\x1b[200~uname\ndate\x1b[201~\r"),
            vec!["uname", "date"]
        );
        // After a password prompt the next line is secret. A normal prompt resets nothing else.
        t.output(b"[sudo] password for me: ");
        assert!(typed(&mut t, "hunter2\r").is_empty());
        t.output(b"me@host:~$ ");
        assert_eq!(typed(&mut t, "whoami\r"), vec!["whoami"]);
        t.output(b"Enter passphrase for key: ");
        assert!(typed(&mut t, "secret\r").is_empty());
    }

    #[test]
    fn lines_with_obvious_secrets_are_not_kept() {
        for bad in [
            "mysql -u root -pSecret db",
            "mysql --password=x",
            "export API_TOKEN=abc",
            "curl -H 'Authorization: Bearer abc' x",
            "sshpass -p hunter2 ssh a@b",
            "deploy --token abc",
        ] {
            assert!(looks_secret(bad), "{bad}");
        }
        for ok in [
            "ssh -p 22 host",
            "mkdir -p a/b",
            "find . -print",
            "mysql -p",
            "git log token-list",
        ] {
            assert!(!looks_secret(ok), "{ok}");
        }
    }

    #[test]
    fn history_is_encrypted_at_rest_and_old_plain_rows_are_upgraded() {
        let c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        record(&c, &s, None, "echo hello").unwrap();
        record(&c, &s, None, "mysql -pTopSecret").unwrap(); // dropped
        let raw: (String, Option<Vec<u8>>) = c
            .query_row(
                "SELECT command, command_enc FROM command_history",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(raw.0, "");
        assert!(raw
            .1
            .is_some_and(|b| !String::from_utf8_lossy(&b).contains("hello")));
        assert_eq!(list(&c, &s, None, None, 10).unwrap().len(), 1);
        // A row from before the data key: plain text, no ciphertext.
        c.execute(
            "INSERT INTO command_history (id, host_id, command, ran_at) VALUES ('old', NULL, 'uptime', 1)",
            [],
        )
        .unwrap();
        let got = list(&c, &s, None, Some("UPT"), 10).unwrap();
        assert_eq!(got[0].command, "uptime");
        let plain: String = c
            .query_row(
                "SELECT command FROM command_history WHERE id = 'old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(plain, "", "upgraded to ciphertext");
        // Another vault key can't read it.
        let other = vault::VaultSession {
            vault_id: s.vault_id.clone(),
            key: s.key.clone(),
            data_key: zeroize::Zeroizing::new([9u8; 32]),
        };
        assert!(list(&c, &other, None, None, 10).unwrap().is_empty());
    }

    #[test]
    fn storage_dedupes_limits_and_filters_per_host() {
        let c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        // Local terminal (no host), so no foreign key is needed.
        record(&c, &s, None, "ls").unwrap();
        record(&c, &s, None, "ls").unwrap(); // repeat: kept once
        record(&c, &s, None, "docker ps").unwrap();
        record(&c, &s, None, "ls").unwrap(); // not a consecutive repeat
        let all = list(&c, &s, None, None, 50).unwrap();
        assert_eq!(
            all.iter().map(|e| e.command.as_str()).collect::<Vec<_>>(),
            vec!["ls", "docker ps", "ls"]
        );
        assert_eq!(list(&c, &s, None, Some("dock"), 50).unwrap().len(), 1);
        assert_eq!(list(&c, &s, None, Some("100%"), 50).unwrap().len(), 0); // % is literal
        for i in 0..520 {
            record(&c, &s, None, &format!("cmd {i}")).unwrap();
        }
        assert_eq!(list(&c, &s, None, None, 500).unwrap().len(), 500);
        assert_eq!(list(&c, &s, None, None, 1).unwrap()[0].command, "cmd 519");
        clear(&c, None, false).unwrap();
        assert!(list(&c, &s, None, None, 50).unwrap().is_empty());
        assert!(!enabled(&c)); // off until Settings turns it on
    }
}
