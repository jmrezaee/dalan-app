//! Bulk actions on a pattern (F18; S33): preview, confirm, run in batches, stop.
//!
//! The preview walks the whole keyspace with `SCAN` (never `KEYS`) and keeps the matching keys here, in Rust. The
//! run acts on exactly those keys, never on more than the person was shown. Every bulk action is `dangerous` for
//! the gate: refused in Read-only, and the database's name must be typed in the other modes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use redis::Value;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::browse::RedisBytes;
use super::cluster::Conn;
use super::conn::redis_error;
use super::gate::{self, Class, CommandTable, Decision};
use super::write::{self, Target};
use crate::error::{AppError, AppResult, ErrorCode};

/// Keys per batch, and the pause between batches so the server stays responsive.
pub const BATCH: usize = 500;
const PAUSE: Duration = Duration::from_millis(20);
const SCAN_COUNT: usize = 1000;
/// A preview is kept this long.
const KEEP_FOR: Duration = Duration::from_secs(600);
/// More matches than this is refused: narrow the pattern (each kept key costs memory).
pub const MAX_KEYS: usize = 5_000_000;
const SAMPLE: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RedisBulkAction {
    Delete,
    Expire {
        #[specta(type = f64)]
        ttl_ms: u64,
    },
    Persist,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisBulkPreview {
    /// Pass it to `redis_bulk_run`. Empty when the scan was stopped: a half-scanned preview can't be run.
    pub id: String,
    #[specta(type = f64)]
    pub count: u64,
    pub sample: Vec<RedisBytes>,
    /// The whole keyspace was walked.
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedisBulkEvent {
    Scanning {
        #[specta(type = f64)]
        found: u64,
        #[specta(type = f64)]
        scanned: u64,
    },
    Running {
        #[specta(type = f64)]
        done: u64,
        #[specta(type = f64)]
        total: u64,
    },
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisBulkResult {
    /// Keys the action was sent for.
    #[specta(type = f64)]
    pub processed: u64,
    /// Keys the server says it changed (a key that expired meanwhile is not one).
    #[specta(type = f64)]
    pub changed: u64,
    pub stopped: bool,
}

pub type BulkEmit = std::sync::Arc<dyn Fn(RedisBulkEvent) + Send + Sync>;

struct Held {
    id: String,
    keys: Vec<Vec<u8>>,
    at: Instant,
}

/// One tab's bulk state: the previewed keys and the Stop flag.
#[derive(Default)]
pub struct BulkState {
    held: Mutex<Option<Held>>,
    stop: AtomicBool,
}

impl BulkState {
    /// "Stop": the scan or the run ends after its current batch. What is done stays done.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    fn keep(&self, keys: Vec<Vec<u8>>) -> AppResult<String> {
        let id = uuid::Uuid::now_v7().to_string();
        *self
            .held
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))? = Some(Held {
            id: id.clone(),
            keys,
            at: Instant::now(),
        });
        Ok(id)
    }
    /// The previewed keys, once. A second run, another preview's id or an old preview is refused.
    fn take(&self, id: &str, now: Instant) -> AppResult<Vec<Vec<u8>>> {
        let held = self
            .held
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))?
            .take()
            .filter(|h| h.id == id && !id.is_empty());
        match held {
            Some(h) if now.duration_since(h.at) < KEEP_FOR => Ok(h.keys),
            _ => Err(AppError::new(
                ErrorCode::RedisConfirmNeeded,
                "That preview is gone or too old. Nothing was sent. Preview again.",
            )),
        }
    }
}

/// Walks the keyspace and keeps what matches. `pattern` empty means every key.
pub async fn preview(
    conn: &mut Conn,
    state: &BulkState,
    pattern: &str,
    key_type: Option<&str>,
    emit: &BulkEmit,
) -> AppResult<RedisBulkPreview> {
    state.stop.store(false, Ordering::Relaxed);
    let mut keys: Vec<Vec<u8>> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut scanned = 0u64;
    // One server, or every primary of a cluster, one after another.
    let targets = conn.scan_targets().await?;
    let mut complete = true;
    'nodes: for mut target in targets {
        let mut cursor = "0".to_string();
        loop {
            if state.stopped() {
                complete = false;
                break 'nodes;
            }
            let mut cmd = redis::cmd("SCAN");
            cmd.arg(&cursor);
            if !pattern.is_empty() && pattern != "*" {
                cmd.arg("MATCH").arg(pattern.as_bytes());
            }
            cmd.arg("COUNT").arg(SCAN_COUNT);
            if let Some(t) = key_type.filter(|t| !t.is_empty()) {
                cmd.arg("TYPE")
                    .arg(if t == "json" { "ReJSON-RL" } else { t });
            }
            let (next, page): (String, Vec<Vec<u8>>) = cmd
                .query_async(&mut target)
                .await
                .map_err(|e| redis_error(&e))?;
            scanned += SCAN_COUNT as u64;
            for k in page {
                // SCAN can return a key twice while the keyspace changes.
                if seen.insert(k.clone()) {
                    keys.push(k);
                }
            }
            if keys.len() > MAX_KEYS {
                return Err(AppError::validation(
                    "More than 5,000,000 keys match. Narrow the pattern and preview again.",
                ));
            }
            emit(RedisBulkEvent::Scanning {
                found: keys.len() as u64,
                scanned,
            });
            cursor = next;
            if cursor == "0" {
                break;
            }
        }
    }
    let count = keys.len() as u64;
    let sample = keys
        .iter()
        .take(SAMPLE)
        .cloned()
        .map(RedisBytes::from_bytes)
        .collect();
    let id = if complete && count > 0 {
        state.keep(keys)?
    } else {
        String::new()
    };
    Ok(RedisBulkPreview {
        id,
        count,
        sample,
        complete,
    })
}

/// One batch as a pipeline, and how many keys the server says it changed.
fn batch_command(
    action: RedisBulkAction,
    keys: &[Vec<u8>],
    unlink: bool,
    per_key: bool,
) -> redis::Pipeline {
    let mut pipe = redis::pipe();
    match action {
        // In a cluster the keys of a batch live in different slots, and one command may only name keys of one
        // slot. So each key gets its own command; `Conn` sends each to its node, grouped per node.
        RedisBulkAction::Delete if per_key => {
            for k in keys {
                pipe.cmd(if unlink { "UNLINK" } else { "DEL" })
                    .arg(k.as_slice());
            }
        }
        RedisBulkAction::Delete => {
            let c = pipe.cmd(if unlink { "UNLINK" } else { "DEL" });
            for k in keys {
                c.arg(k.as_slice());
            }
        }
        RedisBulkAction::Expire { ttl_ms } => {
            for k in keys {
                pipe.cmd("PEXPIRE").arg(k.as_slice()).arg(ttl_ms);
            }
        }
        RedisBulkAction::Persist => {
            for k in keys {
                pipe.cmd("PERSIST").arg(k.as_slice());
            }
        }
    }
    pipe
}

fn changed(reply: &Value) -> u64 {
    match reply {
        Value::Int(i) => (*i).max(0) as u64,
        Value::Array(items) => items.iter().map(changed).sum(),
        _ => 0,
    }
}

fn action_name(action: RedisBulkAction) -> &'static str {
    match action {
        RedisBulkAction::Delete => "bulk delete",
        RedisBulkAction::Expire { .. } => "bulk set expiry",
        RedisBulkAction::Persist => "bulk remove expiry",
    }
}

/// The gate's part, before anything is sent: the mode must allow a dangerous command, and the name must match.
pub fn approve(
    target: &Target,
    action: RedisBulkAction,
    typed_name: Option<&str>,
) -> AppResult<()> {
    if let RedisBulkAction::Expire { ttl_ms: 0 } = action {
        return Err(AppError::validation(
            "An expiry is at least a millisecond away.",
        ));
    }
    match gate::decide(target.mode(), Class::Dangerous) {
        Decision::Refuse(code) => Err(write::refuse(
            target,
            Class::Dangerous,
            action_name(action),
            code,
        )),
        _ if typed_name == Some(target.name.as_str()) => Ok(()),
        _ => Err(AppError::new(
            ErrorCode::RedisConfirmNeeded,
            "The name doesn't match the database's name. Nothing was sent.",
        )),
    }
}

/// Runs the action on the previewed keys, 500 at a time with a short pause. Stop keeps what is done.
/// The caller has asked `approve` first; this takes the preview, so it runs once.
pub async fn run(
    conn: &mut Conn,
    table: &CommandTable,
    state: &BulkState,
    target: &Target,
    preview_id: &str,
    action: RedisBulkAction,
    emit: &BulkEmit,
) -> AppResult<RedisBulkResult> {
    // Asked again here, without the name: a mode that became Read-only since the dialog opened still refuses.
    if let Decision::Refuse(code) = gate::decide(target.mode(), Class::Dangerous) {
        return Err(write::refuse(
            target,
            Class::Dangerous,
            action_name(action),
            code,
        ));
    }
    let keys = state.take(preview_id, Instant::now())?;
    state.stop.store(false, Ordering::Relaxed);
    let total = keys.len() as u64;
    let unlink = table.is_empty() || table.has("UNLINK");
    log::info!(
        "redis: write on {} (Dangerous): {} of {total} keys",
        target.name,
        action_name(action)
    );
    let (mut processed, mut hit) = (0u64, 0u64);
    for chunk in keys.chunks(BATCH) {
        if state.stopped() {
            break;
        }
        let reply: Value = batch_command(action, chunk, unlink, conn.is_cluster())
            .query_async(conn)
            .await
            .map_err(|e| redis_error(&e))?;
        processed += chunk.len() as u64;
        hit += changed(&reply);
        emit(RedisBulkEvent::Running {
            done: processed,
            total,
        });
        if processed < total {
            tokio::time::sleep(PAUSE).await;
        }
    }
    Ok(RedisBulkResult {
        processed,
        changed: hit,
        stopped: processed < total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redis::store::RedisWriteMode;

    fn db(mode: RedisWriteMode) -> Target {
        Target {
            name: "cache-prod".into(),
            mode,
            replica: false,
        }
    }
    fn keys(n: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| format!("session:{i}").into_bytes())
            .collect()
    }
    fn packed(p: &redis::Pipeline) -> String {
        String::from_utf8_lossy(&p.get_packed_pipeline()).replace("\r\n", " ")
    }

    #[test]
    fn a_bulk_action_is_dangerous_in_every_mode() {
        let del = RedisBulkAction::Delete;
        // Read-only: refused, whatever is typed.
        assert_eq!(
            approve(&db(RedisWriteMode::ReadOnly), del, Some("cache-prod"))
                .unwrap_err()
                .code,
            ErrorCode::RedisReadOnlyMode
        );
        // Confirm and Open: the exact name.
        for mode in [RedisWriteMode::Confirm, RedisWriteMode::Open] {
            for wrong in [None, Some(""), Some("cache-pro"), Some("Cache-prod")] {
                assert_eq!(
                    approve(&db(mode), del, wrong).unwrap_err().code,
                    ErrorCode::RedisConfirmNeeded,
                    "{mode:?} {wrong:?}"
                );
            }
            assert!(approve(&db(mode), del, Some("cache-prod")).is_ok());
        }
        let replica = Target {
            replica: true,
            ..db(RedisWriteMode::Open)
        };
        assert_eq!(
            approve(&replica, del, Some("cache-prod")).unwrap_err().code,
            ErrorCode::RedisReadOnlyMode
        );
        assert_eq!(
            approve(
                &db(RedisWriteMode::Open),
                RedisBulkAction::Expire { ttl_ms: 0 },
                Some("cache-prod")
            )
            .unwrap_err()
            .code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn a_preview_runs_once_and_only_by_its_own_id() {
        let state = BulkState::default();
        let now = Instant::now();
        let id = state.keep(keys(3)).unwrap();
        assert_eq!(
            state.take("someone-elses", now).unwrap_err().code,
            ErrorCode::RedisConfirmNeeded
        );
        // Asking with the wrong id used the preview up: better to preview again than to guess.
        assert!(state.take(&id, now).is_err());
        let id = state.keep(keys(3)).unwrap();
        assert_eq!(state.take(&id, now).unwrap().len(), 3);
        assert!(state.take(&id, now).is_err(), "a second run is refused");
        let id = state.keep(keys(3)).unwrap();
        assert!(
            state.take(&id, Instant::now() + KEEP_FOR).is_err(),
            "an old preview is refused"
        );
        assert!(state.take("", now).is_err());
        // A new preview replaces the old one.
        let old = state.keep(keys(1)).unwrap();
        let new = state.keep(keys(2)).unwrap();
        assert_ne!(old, new);
        assert_eq!(state.take(&new, now).unwrap().len(), 2);
    }

    #[test]
    fn batches_are_one_unlink_or_one_command_per_key() {
        let k = keys(3);
        assert!(
            packed(&batch_command(RedisBulkAction::Delete, &k, true, false))
                .contains("UNLINK $9 session:0 $9 session:1 $9 session:2")
        );
        assert!(
            packed(&batch_command(RedisBulkAction::Delete, &k, false, false))
                .contains("DEL $9 session:0")
        );
        let expire = packed(&batch_command(
            RedisBulkAction::Expire { ttl_ms: 60_000 },
            &k,
            true,
            false,
        ));
        assert_eq!(expire.matches("PEXPIRE").count(), 3);
        assert!(expire.contains("60000"));
        assert_eq!(
            packed(&batch_command(RedisBulkAction::Persist, &k, true, false))
                .matches("PERSIST")
                .count(),
            3
        );
        // A cluster: one UNLINK per key, so no command names keys of two slots.
        let per_key = packed(&batch_command(RedisBulkAction::Delete, &k, true, true));
        assert_eq!(per_key.matches("UNLINK").count(), 3);
        assert_eq!(
            keys(1201).chunks(BATCH).map(<[_]>::len).collect::<Vec<_>>(),
            [500, 500, 201]
        );
    }

    #[test]
    fn changed_counts_what_the_server_says_it_touched() {
        assert_eq!(changed(&Value::Array(vec![Value::Int(498)])), 498);
        assert_eq!(
            changed(&Value::Array(vec![
                Value::Int(1),
                Value::Int(0),
                Value::Int(1)
            ])),
            2
        );
        assert_eq!(changed(&Value::Nil), 0);
    }

    #[test]
    fn stop_is_a_flag_the_loops_read() {
        let state = BulkState::default();
        assert!(!state.stopped());
        state.stop();
        assert!(state.stopped());
    }
}
