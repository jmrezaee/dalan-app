//! Pub/Sub (F17; S32). Listening uses its own connection, opened through the same route as the tab's (directly, or
//! through the host), so the Browser and the Console keep working. Messages arrive as pushes on that connection;
//! one task batches them for the UI: about every 8 ms or 64 KB, like terminal output.
//!
//! Backpressure: a batch never holds more than `MAX_BATCH` messages. In a flood the rest are counted as dropped,
//! and the UI says so, instead of the tab freezing or memory growing without end.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use redis::aio::MultiplexedConnection;
use redis::{PushInfo, PushKind, Value};
use serde::Serialize;
use specta::Type;
use tokio::sync::mpsc::UnboundedReceiver;

use super::browse::RedisBytes;
use super::conn::redis_error;
use crate::db::now_ms;
use crate::error::{AppError, AppResult};

const FLUSH_EVERY: Duration = Duration::from_millis(8);
const FLUSH_AT_BYTES: usize = 64 * 1024;
/// The most messages one batch carries (so at most 62,500 a second reach the UI, which keeps the last 5,000).
pub const MAX_BATCH: usize = 500;
/// A tab listens to at most this many channels and patterns.
const MAX_NAMES: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct RedisMessage {
    /// When Dalan received it (this computer's clock), in milliseconds since 1970.
    #[specta(type = f64)]
    pub at_ms: i64,
    pub channel: RedisBytes,
    /// The pattern that matched, when it came through a pattern.
    pub pattern: Option<String>,
    pub payload: RedisBytes,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedisSubEvent {
    Messages {
        messages: Vec<RedisMessage>,
        /// Messages left out of this batch because too many arrived at once.
        dropped: u32,
    },
    /// The listening connection ended (the server closed it, or the route dropped).
    Ended,
}

pub type SubEmit = Arc<dyn Fn(RedisSubEvent) + Send + Sync>;

fn bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::BulkString(b) => Some(b.clone()),
        Value::SimpleString(s) => Some(s.clone().into_bytes()),
        _ => None,
    }
}

/// A push from the server as a message for the UI. Subscribe confirmations and other pushes are `None`.
pub fn message(push: &PushInfo, at_ms: i64) -> Option<RedisMessage> {
    let (pattern, channel, payload) = match (&push.kind, push.data.as_slice()) {
        (PushKind::Message | PushKind::SMessage, [channel, payload]) => (None, channel, payload),
        (PushKind::PMessage, [pattern, channel, payload]) => (
            Some(String::from_utf8_lossy(&bytes(pattern)?).into_owned()),
            channel,
            payload,
        ),
        _ => return None,
    };
    Some(RedisMessage {
        at_ms,
        channel: RedisBytes::from_bytes(bytes(channel)?),
        pattern,
        payload: RedisBytes::from_bytes(bytes(payload)?),
    })
}

/// What is waiting to go to the UI.
#[derive(Default)]
struct Batch {
    messages: Vec<RedisMessage>,
    bytes: usize,
    dropped: u32,
}

impl Batch {
    fn add(&mut self, m: RedisMessage) {
        if self.messages.len() >= MAX_BATCH {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.bytes += m.payload.as_bytes().len() + m.channel.as_bytes().len();
        self.messages.push(m);
    }
    fn full(&self) -> bool {
        self.bytes >= FLUSH_AT_BYTES
    }
    fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.dropped == 0
    }
    fn take(&mut self) -> RedisSubEvent {
        let b = std::mem::take(self);
        RedisSubEvent::Messages {
            messages: b.messages,
            dropped: b.dropped,
        }
    }
}

/// Moves pushes to the UI until the connection ends.
async fn pump(mut pushes: UnboundedReceiver<PushInfo>, emit: Arc<Mutex<SubEmit>>) {
    let send = |e: RedisSubEvent| {
        // Clone the callback out of the lock, so a slow UI never holds it.
        let f = emit.lock().map(|g| Arc::clone(&g)).ok();
        if let Some(f) = f {
            f(e);
        }
    };
    let mut batch = Batch::default();
    loop {
        // Wait for the first push without a timer; once something is waiting, flush within 8 ms.
        let next = if batch.is_empty() {
            pushes.recv().await
        } else {
            match tokio::time::timeout(FLUSH_EVERY, pushes.recv()).await {
                Ok(p) => p,
                Err(_) => {
                    send(batch.take());
                    continue;
                }
            }
        };
        match next {
            Some(p) if p.kind == PushKind::Disconnection => break,
            Some(p) => {
                if let Some(m) = message(&p, now_ms()) {
                    batch.add(m);
                    if batch.full() {
                        send(batch.take());
                    }
                }
            }
            None => break,
        }
    }
    if !batch.is_empty() {
        send(batch.take());
    }
    send(RedisSubEvent::Ended);
}

/// The listening side of one tab.
pub struct Listener {
    conn: MultiplexedConnection,
    emit: Arc<Mutex<SubEmit>>,
    task: tokio::task::JoinHandle<()>,
    /// (is a pattern, the name's bytes)
    names: HashSet<(bool, Vec<u8>)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Dropping the connection handle closes the stream; the task would end by itself, this just makes it prompt.
        self.task.abort();
    }
}

impl Listener {
    /// `conn` is a fresh RESP3 connection whose pushes arrive on `pushes`.
    pub fn new(
        conn: MultiplexedConnection,
        pushes: UnboundedReceiver<PushInfo>,
        emit: SubEmit,
    ) -> Self {
        let emit = Arc::new(Mutex::new(emit));
        let task = tokio::spawn(pump(pushes, Arc::clone(&emit)));
        Self {
            conn,
            emit,
            task,
            names: HashSet::new(),
        }
    }

    /// Messages go to this callback from now on (the tab's newest channel).
    pub fn set_emit(&self, emit: SubEmit) {
        if let Ok(mut e) = self.emit.lock() {
            *e = emit;
        }
    }

    pub async fn start(&mut self, name: &RedisBytes, pattern: bool) -> AppResult<()> {
        let n = name.as_bytes();
        if n.is_empty() {
            return Err(AppError::validation(
                "Type a channel name or a pattern such as orders.*.",
            ));
        }
        let entry = (pattern, n.to_vec());
        if self.names.contains(&entry) {
            return Ok(());
        }
        if self.names.len() >= MAX_NAMES {
            return Err(AppError::validation(
                "A tab listens to at most 50 channels and patterns.",
            ));
        }
        let done = if pattern {
            self.conn.psubscribe(n).await
        } else {
            self.conn.subscribe(n).await
        };
        done.map_err(|e| redis_error(&e))?;
        self.names.insert(entry);
        Ok(())
    }

    pub async fn stop(&mut self, name: &RedisBytes, pattern: bool) -> AppResult<()> {
        let n = name.as_bytes();
        if !self.names.remove(&(pattern, n.to_vec())) {
            return Ok(());
        }
        let done = if pattern {
            self.conn.punsubscribe(n).await
        } else {
            self.conn.unsubscribe(n).await
        };
        done.map_err(|e| redis_error(&e))
    }

    pub fn is_idle(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }
    fn msg(i: usize, payload: &str) -> RedisMessage {
        RedisMessage {
            at_ms: i as i64,
            channel: RedisBytes::from_bytes(b"orders.paid".to_vec()),
            pattern: None,
            payload: RedisBytes::from_bytes(payload.as_bytes().to_vec()),
        }
    }

    #[test]
    fn pushes_become_messages_and_everything_else_is_ignored() {
        let m = message(
            &PushInfo {
                kind: PushKind::Message,
                data: vec![b("deploys"), b("api v2.41.0 started")],
            },
            7,
        )
        .unwrap();
        assert_eq!(m.at_ms, 7);
        assert_eq!(
            m.channel,
            RedisBytes::Text {
                text: "deploys".into()
            }
        );
        assert_eq!(
            m.payload,
            RedisBytes::Text {
                text: "api v2.41.0 started".into()
            }
        );
        assert_eq!(m.pattern, None);
        let p = message(
            &PushInfo {
                kind: PushKind::PMessage,
                data: vec![
                    b("orders.*"),
                    b("orders.paid"),
                    Value::BulkString(vec![0xff, 0x00]),
                ],
            },
            0,
        )
        .unwrap();
        assert_eq!(p.pattern.as_deref(), Some("orders.*"));
        assert_eq!(
            p.channel,
            RedisBytes::Text {
                text: "orders.paid".into()
            }
        );
        assert_eq!(
            p.payload,
            RedisBytes::Binary {
                bytes: vec![0xff, 0x00]
            },
            "a payload is bytes"
        );
        for other in [
            PushInfo {
                kind: PushKind::Subscribe,
                data: vec![b("deploys"), Value::Int(1)],
            },
            PushInfo {
                kind: PushKind::Unsubscribe,
                data: vec![b("deploys"), Value::Int(0)],
            },
            PushInfo {
                kind: PushKind::Message,
                data: vec![b("only-a-channel")],
            },
            PushInfo {
                kind: PushKind::Invalidate,
                data: vec![],
            },
        ] {
            assert_eq!(message(&other, 0), None, "{:?}", other.kind);
        }
    }

    #[test]
    fn a_batch_is_capped_and_counts_what_it_dropped() {
        let mut batch = Batch::default();
        assert!(batch.is_empty());
        for i in 0..(MAX_BATCH + 120) {
            batch.add(msg(i, "x"));
        }
        assert!(!batch.is_empty());
        let RedisSubEvent::Messages { messages, dropped } = batch.take() else {
            panic!()
        };
        assert_eq!(messages.len(), MAX_BATCH);
        assert_eq!(dropped, 120);
        assert_eq!(messages[0].at_ms, 0, "the oldest are kept, in order");
        assert!(batch.is_empty(), "taking a batch empties it");
    }

    #[test]
    fn a_batch_is_full_at_64_kb() {
        let mut batch = Batch::default();
        let big = "x".repeat(20 * 1024);
        for i in 0..3 {
            assert!(!batch.full());
            batch.add(msg(i, &big));
        }
        batch.add(msg(3, &big));
        assert!(batch.full());
    }

    #[tokio::test]
    async fn the_pump_batches_pushes_and_says_when_the_connection_ends() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let seen: Arc<Mutex<Vec<RedisSubEvent>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let emit: SubEmit = Arc::new(move |e| sink.lock().unwrap().push(e));
        // A flood: more than one batch can carry, all waiting before the pump runs.
        for i in 0..(MAX_BATCH * 3) {
            tx.send(PushInfo {
                kind: PushKind::Message,
                data: vec![b("flood"), b(&i.to_string())],
            })
            .unwrap();
        }
        tx.send(PushInfo {
            kind: PushKind::Subscribe,
            data: vec![b("flood"), Value::Int(1)],
        })
        .unwrap();
        drop(tx);
        pump(rx, Arc::new(Mutex::new(emit))).await;
        let events = seen.lock().unwrap();
        assert!(matches!(events.last(), Some(RedisSubEvent::Ended)));
        let (mut kept, mut lost) = (0, 0);
        for e in events.iter() {
            if let RedisSubEvent::Messages { messages, dropped } = e {
                assert!(messages.len() <= MAX_BATCH);
                kept += messages.len();
                lost += *dropped as usize;
            }
        }
        assert_eq!(
            kept + lost,
            MAX_BATCH * 3,
            "every message is either delivered or counted"
        );
        assert!(kept >= MAX_BATCH);
    }
}
