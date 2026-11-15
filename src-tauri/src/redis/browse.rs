//! The browser's read side (F14; S28, S29; replica/redis/spec.md "Values are bytes").
//!
//! Keys and values are byte strings. They are never pushed through `String` on the way from the server: a value is
//! sent to the UI as text only when it *is* text (valid UTF-8, no control characters except tab and newline), and as
//! its raw bytes otherwise. A key is addressed by its bytes, so a binary key works like any other.
//!
//! Nothing here writes. Listing uses `SCAN` (never `KEYS`); values load in pages of 500; a string over 1 MB loads its
//! first 64 KB until "Load all" is asked for.

use redis::aio::MultiplexedConnection;
use redis::Value;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::conn::redis_error;
use crate::error::{AppError, AppResult};

pub const PAGE: usize = 500;
/// Strings larger than this open as a preview first.
pub const BIG_STRING: u64 = 1024 * 1024;
pub const PREVIEW_BYTES: u64 = 64 * 1024;

/// A key, a field or a value. `Text` when the bytes are text a person can read and edit; `Binary` otherwise.
/// A `Text` goes back to Rust as the same bytes (UTF-8 round-trips exactly), so either form addresses a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedisBytes {
    Text { text: String },
    Binary { bytes: Vec<u8> },
}

impl RedisBytes {
    pub fn from_bytes(b: Vec<u8>) -> Self {
        match String::from_utf8(b) {
            // Tab and newline are ordinary in text values; any other control character means "not text".
            Ok(text)
                if !text
                    .chars()
                    .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r') =>
            {
                Self::Text { text }
            }
            Ok(text) => Self::Binary {
                bytes: text.into_bytes(),
            },
            Err(e) => Self::Binary {
                bytes: e.into_bytes(),
            },
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text { text } => text.as_bytes(),
            Self::Binary { bytes } => bytes,
        }
    }
}

/// One key in a scanned page.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisKeyRow {
    pub key: RedisBytes,
    /// `string`, `hash`, `list`, `set`, `zset`, `stream`, `json`, or the server's own word for a module type.
    pub key_type: String,
    /// Milliseconds until it expires; `None` = no expiry.
    #[specta(type = Option<f64>)]
    pub ttl_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisScanPage {
    /// Pass it back to continue. `"0"` means the whole keyspace has been walked.
    pub cursor: String,
    pub keys: Vec<RedisKeyRow>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisStreamGroup {
    pub name: String,
    /// Entries delivered to the group and not yet acknowledged.
    #[specta(type = f64)]
    pub pending: u64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisKeyInfo {
    pub key: RedisBytes,
    /// `none` when the key is gone.
    pub key_type: String,
    #[specta(type = Option<f64>)]
    pub ttl_ms: Option<i64>,
    /// Bytes for a string; fields, items, members or entries for the others.
    #[specta(type = f64)]
    pub length: u64,
    /// `MEMORY USAGE`, when the server allows it.
    #[specta(type = Option<f64>)]
    pub memory_bytes: Option<u64>,
    pub encoding: Option<String>,
    /// Streams only: the consumer groups (listing only in this version).
    pub stream_groups: Vec<RedisStreamGroup>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisField {
    pub field: RedisBytes,
    pub value: RedisBytes,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisScored {
    pub member: RedisBytes,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisStreamEntry {
    pub id: String,
    pub fields: Vec<RedisField>,
}

/// One page of a value. `next` is the cursor for the following page; `None` means that was the last one.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedisValuePage {
    String {
        value: RedisBytes,
        #[specta(type = f64)]
        total_bytes: u64,
        /// Only the first 64 KB were loaded (the string is over 1 MB). "Load all" asks again with `full`.
        truncated: bool,
    },
    /// A key of the JSON module, read with `JSON.GET`.
    Json { text: String },
    Hash {
        fields: Vec<RedisField>,
        next: Option<String>,
    },
    List {
        /// The position of the first item in this page.
        #[specta(type = f64)]
        start: u64,
        items: Vec<RedisBytes>,
        next: Option<String>,
    },
    Set {
        members: Vec<RedisBytes>,
        next: Option<String>,
    },
    /// Highest score first.
    Zset {
        members: Vec<RedisScored>,
        next: Option<String>,
    },
    /// Newest first.
    Stream {
        entries: Vec<RedisStreamEntry>,
        next: Option<String>,
    },
    /// A module type this version has no viewer for.
    Unsupported { key_type: String },
}

/// The server says `ReJSON-RL` for the JSON module's type; everything else is already a plain word.
fn type_name(server: &str) -> String {
    if server.eq_ignore_ascii_case("ReJSON-RL") {
        "json".to_string()
    } else {
        server.to_string()
    }
}

/// What to send as `SCAN ... TYPE` for one of our type words.
fn scan_type(ours: &str) -> &str {
    if ours == "json" {
        "ReJSON-RL"
    } else {
        ours
    }
}

/// `PTTL`: -1 is "no expiry", -2 is "no such key".
fn ttl(pttl: i64) -> Option<i64> {
    (pttl >= 0).then_some(pttl)
}

/// A `SCAN`-family cursor from the UI: digits only. `""` starts from the beginning.
fn cursor_arg(cursor: &str) -> AppResult<&str> {
    if cursor.is_empty() {
        return Ok("0");
    }
    if cursor.len() <= 20 && cursor.bytes().all(|b| b.is_ascii_digit()) {
        Ok(cursor)
    } else {
        Err(AppError::validation("That isn't a cursor."))
    }
}

/// The next cursor of a `SCAN`-family reply: `"0"` means done.
fn next_cursor(c: String) -> Option<String> {
    (c != "0").then_some(c)
}

/// The window of a list page that starts at `start`: (stop index for `LRANGE`, next start or `None`).
fn list_window(start: u64, len: u64) -> (u64, Option<u64>) {
    let stop = start.saturating_add(PAGE as u64 - 1);
    let next = (stop.saturating_add(1) < len).then(|| stop + 1);
    (stop, next)
}

fn bytes_of(v: Value) -> Option<Vec<u8>> {
    match v {
        Value::BulkString(b) => Some(b),
        Value::SimpleString(s) => Some(s.into_bytes()),
        Value::VerbatimString { text, .. } => Some(text.into_bytes()),
        Value::Int(i) => Some(i.to_string().into_bytes()),
        Value::Double(d) => Some(d.to_string().into_bytes()),
        _ => None,
    }
}

fn score_of(v: &Value) -> Option<f64> {
    match v {
        Value::Double(d) => Some(*d),
        Value::Int(i) => Some(*i as f64),
        Value::BulkString(b) => std::str::from_utf8(b).ok()?.parse().ok(),
        Value::SimpleString(s) => s.parse().ok(),
        _ => None,
    }
}

/// `field value field value ...` (RESP2) or a map (RESP3) as pairs.
fn pairs(v: Value) -> Vec<(Value, Value)> {
    match v {
        Value::Map(m) => m,
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len() / 2);
            let mut it = items.into_iter();
            while let (Some(a), Some(b)) = (it.next(), it.next()) {
                out.push((a, b));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn fields_of(v: Value) -> Vec<RedisField> {
    pairs(v)
        .into_iter()
        .filter_map(|(f, val)| {
            Some(RedisField {
                field: RedisBytes::from_bytes(bytes_of(f)?),
                value: RedisBytes::from_bytes(bytes_of(val)?),
            })
        })
        .collect()
}

/// Sorted-set members with scores: `member score member score` (RESP2) or `[member, score]` pairs (RESP3).
fn scored(v: Value) -> Vec<RedisScored> {
    let Value::Array(items) = v else {
        return Vec::new();
    };
    let nested = matches!(items.first(), Some(Value::Array(_)));
    let pair_list: Vec<(Value, Value)> = if nested {
        items
            .into_iter()
            .filter_map(|p| match p {
                Value::Array(mut two) if two.len() == 2 => {
                    let score = two.pop()?;
                    Some((two.pop()?, score))
                }
                _ => None,
            })
            .collect()
    } else {
        pairs(Value::Array(items))
    };
    pair_list
        .into_iter()
        .filter_map(|(m, s)| {
            Some(RedisScored {
                score: score_of(&s)?,
                member: RedisBytes::from_bytes(bytes_of(m)?),
            })
        })
        .collect()
}

/// `XREVRANGE`: `[[id, [field, value, ...]], ...]`.
fn stream_entries(v: Value) -> Vec<RedisStreamEntry> {
    let Value::Array(items) = v else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|e| {
            let Value::Array(mut two) = e else {
                return None;
            };
            if two.len() != 2 {
                return None;
            }
            let fields = fields_of(two.pop()?);
            let id = String::from_utf8(bytes_of(two.pop()?)?).ok()?;
            Some(RedisStreamEntry { id, fields })
        })
        .collect()
}

/// A stream id as the server prints it: `<ms>-<seq>`.
fn is_stream_id(s: &str) -> bool {
    let Some((ms, seq)) = s.split_once('-') else {
        return false;
    };
    let digits = |p: &str| !p.is_empty() && p.len() <= 20 && p.bytes().all(|b| b.is_ascii_digit());
    digits(ms) && digits(seq)
}

async fn run<T: redis::FromRedisValue>(
    conn: &mut MultiplexedConnection,
    cmd: &redis::Cmd,
) -> AppResult<T> {
    cmd.query_async(conn).await.map_err(|e| redis_error(&e))
}

/// One `SCAN` page. `pattern` is a glob (`user:*`); empty means everything. Each key comes with its type and expiry.
pub async fn scan(
    conn: &mut MultiplexedConnection,
    cursor: &str,
    pattern: &str,
    type_filter: Option<&str>,
    count: u32,
) -> AppResult<RedisScanPage> {
    let mut cmd = redis::cmd("SCAN");
    cmd.arg(cursor_arg(cursor)?);
    if !pattern.is_empty() && pattern != "*" {
        cmd.arg("MATCH").arg(pattern.as_bytes());
    }
    cmd.arg("COUNT").arg(count.clamp(10, 5000));
    if let Some(t) = type_filter.filter(|t| !t.is_empty()) {
        cmd.arg("TYPE").arg(scan_type(t));
    }
    let (next, keys): (String, Vec<Vec<u8>>) = run(conn, &cmd).await?;
    if keys.is_empty() {
        return Ok(RedisScanPage {
            cursor: next,
            keys: Vec::new(),
        });
    }
    // TYPE and PTTL for every key in one round trip.
    let mut pipe = redis::pipe();
    for k in &keys {
        pipe.cmd("TYPE").arg(k.as_slice());
        pipe.cmd("PTTL").arg(k.as_slice());
    }
    let details: Vec<Value> = pipe.query_async(conn).await.map_err(|e| redis_error(&e))?;
    let mut it = details.into_iter();
    let mut rows = Vec::with_capacity(keys.len());
    for k in keys {
        let t = it
            .next()
            .and_then(bytes_of)
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let pttl = match it.next() {
            Some(Value::Int(i)) => i,
            _ => -1,
        };
        // A key deleted between SCAN and TYPE is simply not listed.
        if t == "none" {
            continue;
        }
        rows.push(RedisKeyRow {
            key: RedisBytes::from_bytes(k),
            key_type: type_name(&t),
            ttl_ms: ttl(pttl),
        });
    }
    Ok(RedisScanPage {
        cursor: next,
        keys: rows,
    })
}

async fn key_type(conn: &mut MultiplexedConnection, key: &[u8]) -> AppResult<String> {
    let t: String = run(conn, redis::cmd("TYPE").arg(key)).await?;
    Ok(type_name(&t))
}

/// The command that says how big a value of this type is.
fn length_command(key_type: &str) -> Option<&'static str> {
    Some(match key_type {
        "string" => "STRLEN",
        "hash" => "HLEN",
        "list" => "LLEN",
        "set" => "SCARD",
        "zset" => "ZCARD",
        "stream" => "XLEN",
        _ => return None,
    })
}

pub async fn key_info(
    conn: &mut MultiplexedConnection,
    key: &RedisBytes,
) -> AppResult<RedisKeyInfo> {
    let k = key.as_bytes();
    let t = key_type(conn, k).await?;
    let pttl: i64 = run(conn, redis::cmd("PTTL").arg(k)).await?;
    let length: u64 = match length_command(&t) {
        Some(cmd) => run(conn, redis::cmd(cmd).arg(k)).await?,
        None => 0,
    };
    // Both can be refused for a limited ACL user: the header then just shows less.
    let memory_bytes = redis::cmd("MEMORY")
        .arg("USAGE")
        .arg(k)
        .query_async::<Option<u64>>(conn)
        .await
        .ok()
        .flatten();
    let encoding = redis::cmd("OBJECT")
        .arg("ENCODING")
        .arg(k)
        .query_async::<Option<String>>(conn)
        .await
        .ok()
        .flatten();
    let stream_groups = if t == "stream" {
        redis::cmd("XINFO")
            .arg("GROUPS")
            .arg(k)
            .query_async::<Value>(conn)
            .await
            .map(stream_groups)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(RedisKeyInfo {
        key: key.clone(),
        key_type: t,
        ttl_ms: ttl(pttl),
        length,
        memory_bytes,
        encoding,
        stream_groups,
    })
}

/// `XINFO GROUPS`: one map (or flat field/value list) per group; we keep the name and how many entries wait.
fn stream_groups(v: Value) -> Vec<RedisStreamGroup> {
    let Value::Array(groups) = v else {
        return Vec::new();
    };
    groups
        .into_iter()
        .filter_map(|g| {
            let mut name = None;
            let mut pending = 0;
            for (k, val) in pairs(g) {
                match bytes_of(k)?.as_slice() {
                    b"name" => name = String::from_utf8(bytes_of(val)?).ok(),
                    b"pending" => {
                        if let Value::Int(i) = val {
                            pending = i.max(0) as u64;
                        }
                    }
                    _ => {}
                }
            }
            Some(RedisStreamGroup {
                name: name?,
                pending,
            })
        })
        .collect()
}

/// One page of the key's value. `cursor` is `""` for the first page, then whatever `next` said.
/// `full` loads a big string whole ("Load all").
pub async fn value_page(
    conn: &mut MultiplexedConnection,
    key: &RedisBytes,
    cursor: &str,
    full: bool,
) -> AppResult<RedisValuePage> {
    let k = key.as_bytes();
    let t = key_type(conn, k).await?;
    Ok(match t.as_str() {
        "none" => return Err(AppError::not_found("That key is gone.")),
        "string" => {
            let total_bytes: u64 = run(conn, redis::cmd("STRLEN").arg(k)).await?;
            let truncated = total_bytes > BIG_STRING && !full;
            let bytes: Vec<u8> = if truncated {
                run(
                    conn,
                    redis::cmd("GETRANGE").arg(k).arg(0).arg(PREVIEW_BYTES - 1),
                )
                .await?
            } else {
                run::<Option<Vec<u8>>>(conn, redis::cmd("GET").arg(k))
                    .await?
                    .unwrap_or_default()
            };
            RedisValuePage::String {
                value: RedisBytes::from_bytes(bytes),
                total_bytes,
                truncated,
            }
        }
        "json" => {
            let bytes: Vec<u8> = run(conn, redis::cmd("JSON.GET").arg(k)).await?;
            RedisValuePage::Json {
                text: String::from_utf8_lossy(&bytes).into_owned(),
            }
        }
        "hash" => {
            let (next, flat): (String, Value) = run(
                conn,
                redis::cmd("HSCAN")
                    .arg(k)
                    .arg(cursor_arg(cursor)?)
                    .arg("COUNT")
                    .arg(PAGE),
            )
            .await?;
            RedisValuePage::Hash {
                fields: fields_of(flat),
                next: next_cursor(next),
            }
        }
        "list" => {
            let start: u64 = cursor_arg(cursor)?
                .parse()
                .map_err(|_| AppError::validation("That isn't a cursor."))?;
            let len: u64 = run(conn, redis::cmd("LLEN").arg(k)).await?;
            let (stop, next) = list_window(start, len);
            let items: Vec<Vec<u8>> =
                run(conn, redis::cmd("LRANGE").arg(k).arg(start).arg(stop)).await?;
            RedisValuePage::List {
                start,
                items: items.into_iter().map(RedisBytes::from_bytes).collect(),
                next: next.map(|n| n.to_string()),
            }
        }
        "set" => {
            let (next, members): (String, Vec<Vec<u8>>) = run(
                conn,
                redis::cmd("SSCAN")
                    .arg(k)
                    .arg(cursor_arg(cursor)?)
                    .arg("COUNT")
                    .arg(PAGE),
            )
            .await?;
            RedisValuePage::Set {
                members: members.into_iter().map(RedisBytes::from_bytes).collect(),
                next: next_cursor(next),
            }
        }
        "zset" => {
            let start: u64 = cursor_arg(cursor)?
                .parse()
                .map_err(|_| AppError::validation("That isn't a cursor."))?;
            let len: u64 = run(conn, redis::cmd("ZCARD").arg(k)).await?;
            let (stop, next) = list_window(start, len);
            // ZREVRANGE works on every server version; the newer `ZRANGE ... REV` says the same thing.
            let reply: Value = run(
                conn,
                redis::cmd("ZREVRANGE")
                    .arg(k)
                    .arg(start)
                    .arg(stop)
                    .arg("WITHSCORES"),
            )
            .await?;
            RedisValuePage::Zset {
                members: scored(reply),
                next: next.map(|n| n.to_string()),
            }
        }
        "stream" => {
            // The cursor is the last id of the page before; `(` makes it exclusive.
            let end = if cursor.is_empty() {
                "+".to_string()
            } else if is_stream_id(cursor) {
                format!("({cursor}")
            } else {
                return Err(AppError::validation("That isn't a cursor."));
            };
            let reply: Value = run(
                conn,
                redis::cmd("XREVRANGE")
                    .arg(k)
                    .arg(end)
                    .arg("-")
                    .arg("COUNT")
                    .arg(PAGE),
            )
            .await?;
            let entries = stream_entries(reply);
            let next = (entries.len() == PAGE)
                .then(|| entries.last().map(|e| e.id.clone()))
                .flatten();
            RedisValuePage::Stream { entries, next }
        }
        other => RedisValuePage::Unsupported {
            key_type: other.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> RedisBytes {
        RedisBytes::Text { text: s.into() }
    }
    fn bulk(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }

    #[test]
    fn text_is_text_and_everything_else_stays_bytes() {
        assert_eq!(
            RedisBytes::from_bytes(b"user:1042".to_vec()),
            text("user:1042")
        );
        assert_eq!(
            RedisBytes::from_bytes("سلام\tدنیا\nخط دوم".as_bytes().to_vec()),
            text("سلام\tدنیا\nخط دوم")
        );
        assert_eq!(RedisBytes::from_bytes(Vec::new()), text(""));
        // Not UTF-8: the bytes come through untouched.
        let raw = vec![0xff, 0xfe, 0x00, 0x41];
        assert_eq!(
            RedisBytes::from_bytes(raw.clone()),
            RedisBytes::Binary { bytes: raw.clone() }
        );
        // Valid UTF-8 with a control character (NUL, ESC) is not something to show as text.
        assert_eq!(
            RedisBytes::from_bytes(b"a\0b".to_vec()),
            RedisBytes::Binary {
                bytes: b"a\0b".to_vec()
            }
        );
        assert_eq!(
            RedisBytes::from_bytes(b"\x1b[31m".to_vec()),
            RedisBytes::Binary {
                bytes: b"\x1b[31m".to_vec()
            }
        );
        // Either form gives back exactly the bytes it was made from.
        assert_eq!(
            RedisBytes::from_bytes(raw.clone()).as_bytes(),
            raw.as_slice()
        );
        assert_eq!(text("کلید").as_bytes(), "کلید".as_bytes());
    }

    #[test]
    fn a_value_from_the_ui_addresses_the_same_bytes() {
        let from_ui: RedisBytes =
            serde_json::from_str(r#"{"kind":"binary","bytes":[255,0,65]}"#).unwrap();
        assert_eq!(from_ui.as_bytes(), [255, 0, 65]);
        let from_ui: RedisBytes =
            serde_json::from_str(r#"{"kind":"text","text":"user:1"}"#).unwrap();
        assert_eq!(from_ui.as_bytes(), b"user:1");
        assert_eq!(
            serde_json::to_string(&text("a")).unwrap(),
            r#"{"kind":"text","text":"a"}"#
        );
    }

    #[test]
    fn type_words_expiry_and_cursors() {
        assert_eq!(type_name("ReJSON-RL"), "json");
        assert_eq!(type_name("hash"), "hash");
        assert_eq!(scan_type("json"), "ReJSON-RL");
        assert_eq!(scan_type("zset"), "zset");
        assert_eq!(ttl(-1), None);
        assert_eq!(ttl(-2), None);
        assert_eq!(ttl(0), Some(0));
        assert_eq!(ttl(83_640_000), Some(83_640_000));
        assert_eq!(cursor_arg("").unwrap(), "0");
        assert_eq!(
            cursor_arg("17293822569102704640").unwrap(),
            "17293822569102704640"
        );
        for bad in [
            "-1",
            "1 MATCH *",
            "abc",
            "1;FLUSHALL",
            "123456789012345678901",
        ] {
            assert!(cursor_arg(bad).is_err(), "{bad}");
        }
        assert_eq!(next_cursor("0".into()), None);
        assert_eq!(next_cursor("42".into()), Some("42".into()));
        assert!(is_stream_id("1794770112-0"));
        for bad in [
            "",
            "1794770112",
            "-",
            "1-",
            "-1",
            "a-b",
            "1-2-3",
            "+",
            "(1-0",
        ] {
            assert!(!is_stream_id(bad), "{bad}");
        }
    }

    #[test]
    fn list_pages_are_five_hundred_long_and_know_when_they_end() {
        assert_eq!(list_window(0, 3), (499, None));
        assert_eq!(list_window(0, 500), (499, None));
        assert_eq!(list_window(0, 501), (499, Some(500)));
        assert_eq!(list_window(500, 48_210), (999, Some(1000)));
        assert_eq!(list_window(48_000, 48_210), (48_499, None));
        assert_eq!(list_window(0, 0), (499, None));
    }

    #[test]
    fn hash_fields_come_from_a_flat_list_or_a_map() {
        let flat = Value::Array(vec![bulk("name"), bulk("Sara"), bulk("seats"), bulk("12")]);
        let map = Value::Map(vec![
            (bulk("name"), bulk("Sara")),
            (bulk("seats"), bulk("12")),
        ]);
        for v in [flat, map] {
            let f = fields_of(v);
            assert_eq!(f.len(), 2);
            assert_eq!(
                (f[0].field.clone(), f[0].value.clone()),
                (text("name"), text("Sara"))
            );
            assert_eq!(f[1].value, text("12"));
        }
        // An odd trailing item is dropped, not paired with nothing.
        assert_eq!(
            fields_of(Value::Array(vec![bulk("a"), bulk("1"), bulk("b")])).len(),
            1
        );
        assert!(fields_of(Value::Nil).is_empty());
    }

    #[test]
    fn sorted_set_members_come_from_either_protocol() {
        let resp2 = Value::Array(vec![
            bulk("team:aurora"),
            bulk("9812"),
            bulk("team:kestrel"),
            bulk("9640.5"),
        ]);
        let resp3 = Value::Array(vec![
            Value::Array(vec![bulk("team:aurora"), Value::Double(9812.0)]),
            Value::Array(vec![bulk("team:kestrel"), Value::Double(9640.5)]),
        ]);
        for v in [resp2, resp3] {
            let m = scored(v);
            assert_eq!(m.len(), 2);
            assert_eq!(m[0].member, text("team:aurora"));
            assert_eq!(m[0].score, 9812.0);
            assert_eq!(m[1].score, 9640.5);
        }
        assert!(scored(Value::Array(vec![])).is_empty());
        // Infinite scores are valid in Redis.
        let inf = scored(Value::Array(vec![
            bulk("a"),
            bulk("inf"),
            bulk("b"),
            bulk("-inf"),
        ]));
        assert!(inf[0].score.is_infinite() && inf[1].score.is_infinite());
    }

    #[test]
    fn stream_entries_keep_their_ids_and_fields() {
        let reply = Value::Array(vec![
            Value::Array(vec![
                bulk("1794770112-0"),
                Value::Array(vec![
                    bulk("order"),
                    bulk("5512"),
                    bulk("status"),
                    bulk("paid"),
                ]),
            ]),
            Value::Array(vec![bulk("1794770098-0"), Value::Array(vec![])]),
        ]);
        let e = stream_entries(reply);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].id, "1794770112-0");
        assert_eq!(e[0].fields.len(), 2);
        assert_eq!(e[0].fields[1].value, text("paid"));
        assert!(e[1].fields.is_empty());
    }

    #[test]
    fn stream_groups_read_their_name_and_what_waits() {
        let g = |name: &str, pending: i64| {
            Value::Array(vec![
                bulk("name"),
                bulk(name),
                bulk("consumers"),
                Value::Int(2),
                bulk("pending"),
                Value::Int(pending),
            ])
        };
        let groups = stream_groups(Value::Array(vec![g("billing", 0), g("emails", 3)]));
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[1].name.as_str(), groups[1].pending), ("emails", 3));
    }

    #[test]
    fn every_basic_type_has_a_length_command_and_module_types_have_none() {
        for (t, cmd) in [
            ("string", "STRLEN"),
            ("hash", "HLEN"),
            ("list", "LLEN"),
            ("set", "SCARD"),
            ("zset", "ZCARD"),
            ("stream", "XLEN"),
        ] {
            assert_eq!(length_command(t), Some(cmd));
        }
        assert_eq!(length_command("json"), None);
        assert_eq!(length_command("TSDB-TYPE"), None);
    }
}
