//! Changing data (F15; replica/redis/spec.md section 4). Every change is a typed `RedisWriteOp` from an editor.
//!
//! `prepare` turns the op into the exact command bytes, asks the gate, and then either runs it (Open mode), refuses
//! it (Read-only), or keeps the bytes for two minutes and answers with a `RedisPending` to show the person.
//! `commit` runs those kept bytes, once. The UI never sends a command of its own, so it cannot claim a
//! confirmation it did not show.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use redis::Value;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::browse::RedisBytes;
use super::cluster::Conn;
use super::conn::redis_error;
use super::gate::{self, Class, CommandTable, Decision};
use super::store::RedisWriteMode;
use crate::error::{AppError, AppResult, ErrorCode};

/// How long a prepared write waits for its confirmation.
pub const PENDING_FOR: Duration = Duration::from_secs(120);
/// "Now" and "After" in a confirmation show values up to this size; bigger ones are left out.
const SHOWN_BYTES: usize = 64 * 1024;

pub(super) type Args = Vec<Vec<u8>>;

#[derive(Debug, Deserialize, Type)]
pub struct RedisFieldInput {
    pub field: RedisBytes,
    pub value: RedisBytes,
}

/// One change, as an editor asks for it.
#[derive(Debug, Deserialize, Type)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RedisWriteOp {
    SetString {
        key: RedisBytes,
        value: RedisBytes,
    },
    /// A key of the JSON module (`JSON.SET key $ text`). A plain string that holds JSON uses `SetString`.
    SetJson {
        key: RedisBytes,
        text: String,
    },
    HashSet {
        key: RedisBytes,
        field: RedisBytes,
        value: RedisBytes,
    },
    HashRemove {
        key: RedisBytes,
        field: RedisBytes,
    },
    /// `start` adds at the head (`LPUSH`), otherwise at the tail (`RPUSH`).
    ListPush {
        key: RedisBytes,
        value: RedisBytes,
        start: bool,
    },
    /// Lists are addressed by position, so the item the person saw (`expect`) is checked right before the change.
    ListSet {
        key: RedisBytes,
        index: u32,
        expect: RedisBytes,
        value: RedisBytes,
    },
    ListRemove {
        key: RedisBytes,
        index: u32,
        expect: RedisBytes,
    },
    SetAdd {
        key: RedisBytes,
        member: RedisBytes,
    },
    SetRemove {
        key: RedisBytes,
        member: RedisBytes,
    },
    /// Adds the member, or gives it a new score.
    ZsetAdd {
        key: RedisBytes,
        member: RedisBytes,
        score: String,
    },
    ZsetRemove {
        key: RedisBytes,
        member: RedisBytes,
    },
    /// `id: None` lets the server choose (`*`).
    StreamAdd {
        key: RedisBytes,
        id: Option<String>,
        fields: Vec<RedisFieldInput>,
    },
    StreamDelete {
        key: RedisBytes,
        id: String,
    },
    /// `ttl_ms: None` removes the expiry.
    Expire {
        key: RedisBytes,
        #[specta(type = Option<f64>)]
        ttl_ms: Option<u64>,
    },
    /// Never overwrites: refused when `to` exists.
    Rename {
        key: RedisBytes,
        to: RedisBytes,
    },
    Delete {
        key: RedisBytes,
    },
    /// A new key with its first item. Never overwrites. `field` is for a hash and a stream, `score` for a
    /// sorted set.
    Create {
        key: RedisBytes,
        key_type: String,
        field: Option<RedisBytes>,
        value: RedisBytes,
        score: Option<String>,
        #[specta(type = Option<f64>)]
        ttl_ms: Option<u64>,
    },
    Publish {
        channel: RedisBytes,
        message: RedisBytes,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisConfirmLevel {
    Confirm,
    /// The person must type the database's name.
    Typed,
}

/// A write that waits for its confirmation: what to show the person.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisPending {
    pub id: String,
    pub level: RedisConfirmLevel,
    /// The exact command, as text, with long arguments shortened.
    pub command_preview: String,
    /// What it changes: the key, and the field or position inside it.
    pub target: String,
    pub before: Option<RedisBytes>,
    pub after: Option<RedisBytes>,
    /// Deleting a key: how many items go with it.
    #[specta(type = Option<f64>)]
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RedisWriteOutcome {
    /// It ran. `reply` is the server's answer when it is worth showing (a new stream entry's id).
    Done {
        reply: Option<String>,
    },
    Pending {
        pending: RedisPending,
    },
}

/// The command bytes of one op, with what must hold for them to run.
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    /// One command, or several that run together (`MULTI`).
    commands: Vec<Args>,
    /// Asked right before running: the reply must be `expect` (as bytes), or the data changed under the person.
    check: Option<(Args, Vec<u8>)>,
    /// A reply that means "a key with that name already exists".
    exists_reply: Option<Value>,
    target: String,
    after: Option<RedisBytes>,
}

fn args(parts: &[&[u8]]) -> Args {
    parts.iter().map(|p| p.to_vec()).collect()
}

fn shown(b: &RedisBytes) -> Option<RedisBytes> {
    (b.as_bytes().len() <= SHOWN_BYTES).then(|| b.clone())
}

fn key_text(key: &RedisBytes) -> String {
    gate::preview(&[key.as_bytes().to_vec()])
}

fn not_empty(key: &RedisBytes) -> AppResult<()> {
    if key.as_bytes().is_empty() {
        return Err(AppError::validation("A key needs a name."));
    }
    Ok(())
}

/// A sorted-set score as Redis reads it: a number, `inf`, `+inf` or `-inf`.
fn score_arg(score: &str) -> AppResult<Vec<u8>> {
    let s = score.trim();
    let ok = matches!(s.to_ascii_lowercase().as_str(), "inf" | "+inf" | "-inf")
        || s.parse::<f64>().is_ok_and(f64::is_finite);
    if ok {
        Ok(s.as_bytes().to_vec())
    } else {
        Err(AppError::validation("A score is a number, inf or -inf."))
    }
}

fn stream_id_arg(id: &str) -> AppResult<Vec<u8>> {
    let ok = !id.is_empty()
        && id.len() <= 41
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'-' || b == b'*');
    if ok {
        Ok(id.as_bytes().to_vec())
    } else {
        Err(AppError::validation(
            "A stream id looks like 1731000000000-0, or * to let the server choose.",
        ))
    }
}

fn valid_json(text: &str) -> AppResult<()> {
    serde_json::from_str::<serde::de::IgnoredAny>(text)
        .map(|_| ())
        .map_err(|e| AppError::validation(format!("That isn't valid JSON: {e}")))
}

/// Turns an op into its command bytes. Pure: no server, so the whole mapping is unit-tested.
fn plan(op: &RedisWriteOp, table: &CommandTable) -> AppResult<Plan> {
    use RedisWriteOp::*;
    let one = |commands: Args, target: String, after: Option<RedisBytes>| Plan {
        commands: vec![commands],
        check: None,
        exists_reply: None,
        target,
        after,
    };
    let inside =
        |key: &RedisBytes, part: &RedisBytes| format!("{} › {}", key_text(key), key_text(part));
    let at = |key: &RedisBytes, index: u32| format!("{} › #{index}", key_text(key));
    let list_check = |key: &RedisBytes, index: u32, expect: &RedisBytes| {
        Some((
            args(&[b"LINDEX", key.as_bytes(), index.to_string().as_bytes()]),
            expect.as_bytes().to_vec(),
        ))
    };
    let pexpire = |key: &RedisBytes, ms: u64| -> AppResult<Args> {
        if ms == 0 {
            return Err(AppError::validation(
                "An expiry is at least a millisecond away.",
            ));
        }
        Ok(args(&[
            b"PEXPIRE",
            key.as_bytes(),
            ms.to_string().as_bytes(),
        ]))
    };
    match op {
        SetString { key, .. }
        | SetJson { key, .. }
        | HashSet { key, .. }
        | HashRemove { key, .. }
        | ListPush { key, .. }
        | ListSet { key, .. }
        | ListRemove { key, .. }
        | SetAdd { key, .. }
        | SetRemove { key, .. }
        | ZsetAdd { key, .. }
        | ZsetRemove { key, .. }
        | StreamAdd { key, .. }
        | StreamDelete { key, .. }
        | Expire { key, .. }
        | Rename { key, .. }
        | Delete { key }
        | Create { key, .. } => not_empty(key)?,
        Publish { channel, .. } => not_empty(channel)?,
    }
    Ok(match op {
        // KEEPTTL: changing a value must not quietly remove its expiry.
        // ponytail: needs Redis 6.0 or newer; an older server answers with a syntax error and nothing changes.
        SetString { key, value } => one(
            args(&[b"SET", key.as_bytes(), value.as_bytes(), b"KEEPTTL"]),
            key_text(key),
            shown(value),
        ),
        SetJson { key, text } => {
            valid_json(text)?;
            one(
                args(&[b"JSON.SET", key.as_bytes(), b"$", text.as_bytes()]),
                key_text(key),
                shown(&RedisBytes::Text { text: text.clone() }),
            )
        }
        HashSet { key, field, value } => one(
            args(&[b"HSET", key.as_bytes(), field.as_bytes(), value.as_bytes()]),
            inside(key, field),
            shown(value),
        ),
        HashRemove { key, field } => one(
            args(&[b"HDEL", key.as_bytes(), field.as_bytes()]),
            inside(key, field),
            None,
        ),
        ListPush { key, value, start } => one(
            args(&[
                if *start { b"LPUSH" } else { b"RPUSH" },
                key.as_bytes(),
                value.as_bytes(),
            ]),
            format!(
                "{} › {}",
                key_text(key),
                if *start { "start" } else { "end" }
            ),
            shown(value),
        ),
        ListSet {
            key,
            index,
            expect,
            value,
        } => Plan {
            check: list_check(key, *index, expect),
            ..one(
                args(&[
                    b"LSET",
                    key.as_bytes(),
                    index.to_string().as_bytes(),
                    value.as_bytes(),
                ]),
                at(key, *index),
                shown(value),
            )
        },
        ListRemove { key, index, expect } => {
            // Redis removes list items by value, not by position. So: mark the position with a value nothing
            // else has, then remove that value. Both in one MULTI, so nobody sees the mark.
            let mark = format!("__dalan_removed_{}__", uuid::Uuid::now_v7());
            Plan {
                commands: vec![
                    args(&[
                        b"LSET",
                        key.as_bytes(),
                        index.to_string().as_bytes(),
                        mark.as_bytes(),
                    ]),
                    args(&[b"LREM", key.as_bytes(), b"1", mark.as_bytes()]),
                ],
                check: list_check(key, *index, expect),
                exists_reply: None,
                target: at(key, *index),
                after: None,
            }
        }
        SetAdd { key, member } => one(
            args(&[b"SADD", key.as_bytes(), member.as_bytes()]),
            inside(key, member),
            None,
        ),
        SetRemove { key, member } => one(
            args(&[b"SREM", key.as_bytes(), member.as_bytes()]),
            inside(key, member),
            None,
        ),
        ZsetAdd { key, member, score } => {
            let s = score_arg(score)?;
            let after = RedisBytes::from_bytes(s.clone());
            one(
                args(&[b"ZADD", key.as_bytes(), &s, member.as_bytes()]),
                inside(key, member),
                Some(after),
            )
        }
        ZsetRemove { key, member } => one(
            args(&[b"ZREM", key.as_bytes(), member.as_bytes()]),
            inside(key, member),
            None,
        ),
        StreamAdd { key, id, fields } => {
            if fields.is_empty() || fields.iter().any(|f| f.field.as_bytes().is_empty()) {
                return Err(AppError::validation(
                    "A stream entry needs at least one named field.",
                ));
            }
            let id = stream_id_arg(
                id.as_deref()
                    .map(str::trim)
                    .filter(|i| !i.is_empty())
                    .unwrap_or("*"),
            )?;
            let mut c = args(&[b"XADD", key.as_bytes(), &id]);
            for f in fields {
                c.push(f.field.as_bytes().to_vec());
                c.push(f.value.as_bytes().to_vec());
            }
            one(c, key_text(key), None)
        }
        StreamDelete { key, id } => {
            let id = stream_id_arg(id)?;
            one(
                args(&[b"XDEL", key.as_bytes(), &id]),
                format!("{} › {}", key_text(key), String::from_utf8_lossy(&id)),
                None,
            )
        }
        Expire {
            key,
            ttl_ms: Some(ms),
        } => one(pexpire(key, *ms)?, key_text(key), None),
        Expire { key, ttl_ms: None } => {
            one(args(&[b"PERSIST", key.as_bytes()]), key_text(key), None)
        }
        Rename { key, to } => {
            not_empty(to)?;
            if key.as_bytes() == to.as_bytes() {
                return Err(AppError::validation("That is already its name."));
            }
            Plan {
                exists_reply: Some(Value::Int(0)),
                ..one(
                    args(&[b"RENAMENX", key.as_bytes(), to.as_bytes()]),
                    key_text(key),
                    Some(to.clone()),
                )
            }
        }
        Delete { key } => {
            // UNLINK frees the memory in the background; servers before 4.0 only have DEL.
            let name: &[u8] = if table.is_empty() || table.has("UNLINK") {
                b"UNLINK"
            } else {
                b"DEL"
            };
            one(args(&[name, key.as_bytes()]), key_text(key), None)
        }
        Create {
            key,
            key_type,
            field,
            value,
            score,
            ttl_ms,
        } => {
            let k = key.as_bytes();
            let v = value.as_bytes();
            let need_field = || {
                field
                    .as_ref()
                    .filter(|f| !f.as_bytes().is_empty())
                    .ok_or_else(|| AppError::validation("Give the first field a name."))
            };
            let mut first = match key_type.as_str() {
                "string" => args(&[b"SET", k, v, b"NX"]),
                "json" => {
                    let text = std::str::from_utf8(v)
                        .map_err(|_| AppError::validation("That isn't valid JSON."))?;
                    valid_json(text)?;
                    args(&[b"JSON.SET", k, b"$", v, b"NX"])
                }
                "hash" => args(&[b"HSET", k, need_field()?.as_bytes(), v]),
                "list" => args(&[b"RPUSH", k, v]),
                "set" => args(&[b"SADD", k, v]),
                "zset" => args(&[b"ZADD", k, &score_arg(score.as_deref().unwrap_or("0"))?, v]),
                "stream" => args(&[b"XADD", k, b"*", need_field()?.as_bytes(), v]),
                _ => return Err(AppError::validation("Pick a type for the new key.")),
            };
            // `SET ... NX PX` is one command, so a string is created and timed at once. The other types cannot
            // refuse to touch an existing key by themselves: `EXISTS` is asked right before they run.
            let atomic = matches!(key_type.as_str(), "string" | "json");
            let mut commands = Vec::new();
            match (ttl_ms, key_type.as_str()) {
                (Some(ms), "string") => {
                    pexpire(key, *ms)?;
                    first.push(b"PX".to_vec());
                    first.push(ms.to_string().into_bytes());
                    commands.push(first);
                }
                (Some(ms), _) => {
                    commands.push(first);
                    commands.push(pexpire(key, *ms)?);
                }
                (None, _) => commands.push(first),
            }
            Plan {
                commands,
                check: Some((args(&[b"EXISTS", k]), b"0".to_vec())),
                exists_reply: atomic.then_some(Value::Nil),
                target: key_text(key),
                after: shown(value),
            }
        }
        Publish { channel, message } => one(
            args(&[b"PUBLISH", channel.as_bytes(), message.as_bytes()]),
            key_text(channel),
            shown(message),
        ),
    })
}

/// The command that reads what a change will replace, for "Now" in the confirmation.
fn before_command(op: &RedisWriteOp) -> Option<Args> {
    use RedisWriteOp::*;
    Some(match op {
        SetString { key, .. } => args(&[
            b"GETRANGE",
            key.as_bytes(),
            b"0",
            (SHOWN_BYTES).to_string().as_bytes(),
        ]),
        SetJson { key, .. } => args(&[b"JSON.GET", key.as_bytes()]),
        HashSet { key, field, .. } | HashRemove { key, field } => {
            args(&[b"HGET", key.as_bytes(), field.as_bytes()])
        }
        ZsetAdd { key, member, .. } | ZsetRemove { key, member } => {
            args(&[b"ZSCORE", key.as_bytes(), member.as_bytes()])
        }
        _ => return None,
    })
}

fn reply_bytes(v: Value) -> Option<Vec<u8>> {
    match v {
        Value::BulkString(b) => Some(b),
        Value::SimpleString(s) => Some(s.into_bytes()),
        Value::VerbatimString { text, .. } => Some(text.into_bytes()),
        Value::Int(i) => Some(i.to_string().into_bytes()),
        Value::Double(d) => Some(d.to_string().into_bytes()),
        _ => None,
    }
}

pub(super) async fn send(conn: &mut Conn, commands: &[Args]) -> AppResult<Value> {
    let build = |a: &Args| {
        let mut c = redis::Cmd::new();
        for part in a {
            c.arg(part.as_slice());
        }
        c
    };
    let reply = match commands {
        [single] => build(single).query_async::<Value>(conn).await,
        many => {
            // `atomic()` wraps the commands in MULTI ... EXEC: all of them run, or none.
            let mut pipe = redis::pipe();
            pipe.atomic();
            for c in many {
                pipe.add_command(build(c));
            }
            pipe.query_async::<Value>(conn).await
        }
    };
    reply.map_err(|e| redis_error(&e))
}

/// What is kept between prepare and commit.
#[derive(Debug)]
struct Stored {
    plan: Plan,
    class: Class,
    created: Instant,
}

/// The prepared writes of one connection. Each connection has its own, so an id from another tab is unknown here.
#[derive(Default)]
pub struct PendingStore(Mutex<HashMap<String, Stored>>);

fn confirm_needed(message: &str) -> AppError {
    AppError::new(ErrorCode::RedisConfirmNeeded, message)
}

impl PendingStore {
    fn put(&self, plan: Plan, class: Class, now: Instant) -> AppResult<String> {
        let id = uuid::Uuid::now_v7().to_string();
        let mut map = self
            .0
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))?;
        // Confirmations nobody answered are dropped here, so the map cannot grow without end.
        map.retain(|_, s| now.duration_since(s.created) < PENDING_FOR);
        map.insert(
            id.clone(),
            Stored {
                plan,
                class,
                created: now,
            },
        );
        Ok(id)
    }

    /// Removes and returns the prepared write. Removing first is what makes a second commit fail.
    fn take(&self, id: &str, now: Instant) -> AppResult<Stored> {
        let stored = self
            .0
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))?
            .remove(id)
            .ok_or_else(|| {
                confirm_needed(
                    "That confirmation was already used or isn't for this tab. Nothing was sent.",
                )
            })?;
        if now.duration_since(stored.created) >= PENDING_FOR {
            return Err(confirm_needed("That confirmation is more than two minutes old. Nothing was sent. Try the change again."));
        }
        Ok(stored)
    }
}

/// The database as the gate needs it, read fresh for every write.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub mode: RedisWriteMode,
    /// A replica only accepts reads, whatever the saved mode says.
    pub replica: bool,
}

impl Target {
    pub(super) fn mode(&self) -> RedisWriteMode {
        if self.replica {
            RedisWriteMode::ReadOnly
        } else {
            self.mode
        }
    }
}

fn command_name(plan: &Plan) -> String {
    plan.commands
        .first()
        .and_then(|c| c.first())
        .map(|n| String::from_utf8_lossy(n).to_ascii_uppercase())
        .unwrap_or_default()
}

pub(super) fn refuse(target: &Target, class: Class, name: &str, code: ErrorCode) -> AppError {
    log::warn!("redis: refused on {} ({class:?}): {name}", target.name);
    match code {
        ErrorCode::RedisReadOnlyMode if target.replica => AppError::new(
            code,
            format!(
                "{} is a replica: it only accepts reads. Nothing was sent.",
                target.name
            ),
        ),
        ErrorCode::RedisReadOnlyMode => AppError::new(
            code,
            format!("{} is read-only in Dalan. Nothing was sent.", target.name),
        ),
        _ => AppError::new(
            code,
            format!("Dalan doesn't send {name}. Nothing was sent."),
        ),
    }
}

/// Whether a confirmed write may run now: the mode is asked again (it may have changed since the dialog opened),
/// and a typed confirmation must carry the database's exact name.
fn approve(
    stored_class: Class,
    target: &Target,
    name: &str,
    typed_name: Option<&str>,
) -> AppResult<()> {
    match gate::decide(target.mode(), stored_class) {
        Decision::Run | Decision::Confirm => Ok(()),
        Decision::Typed if typed_name == Some(target.name.as_str()) => Ok(()),
        Decision::Typed => Err(confirm_needed(
            "The name doesn't match the database's name. Nothing was sent.",
        )),
        Decision::Refuse(code) => Err(refuse(target, stored_class, name, code)),
    }
}

/// Runs a plan: its check first, then its commands.
async fn execute(
    conn: &mut Conn,
    plan: &Plan,
    target: &Target,
    class: Class,
) -> AppResult<RedisWriteOutcome> {
    if let Some((question, expect)) = &plan.check {
        let now = send(conn, std::slice::from_ref(question)).await?;
        if reply_bytes(now).as_deref() != Some(expect.as_slice()) {
            return Err(AppError::validation(
                if plan.exists_reply.is_some() || question[0] == b"EXISTS" {
                    "A key with that name already exists. Nothing was changed."
                } else {
                    "That item changed since you looked. Nothing was changed. Reload and try again."
                },
            ));
        }
    }
    let reply = send(conn, &plan.commands).await?;
    if plan.exists_reply.as_ref() == Some(&reply) {
        return Err(AppError::validation(
            "A key with that name already exists. Nothing was changed.",
        ));
    }
    log::info!(
        "redis: write on {} ({class:?}): {}",
        target.name,
        command_name(plan)
    );
    // Only a text reply is worth showing: the id of a new stream entry.
    let reply = match reply {
        Value::BulkString(b) => String::from_utf8(b).ok(),
        _ => None,
    };
    Ok(RedisWriteOutcome::Done { reply })
}

/// For the console and bulk actions: keeps raw commands until they are confirmed, like `prepare` does for an op.
pub(super) fn hold(
    pending: &PendingStore,
    commands: Vec<Args>,
    class: Class,
    target: String,
) -> AppResult<String> {
    let plan = Plan {
        commands,
        check: None,
        exists_reply: None,
        target,
        after: None,
    };
    pending.put(plan, class, Instant::now())
}

/// The other half of `hold`: the kept commands, once, when the mode (asked again) and the typed name allow it.
pub(super) fn release(
    pending: &PendingStore,
    target: &Target,
    pending_id: &str,
    typed_name: Option<&str>,
) -> AppResult<(Vec<Args>, Class)> {
    let stored = pending.take(pending_id, Instant::now())?;
    approve(
        stored.class,
        target,
        &command_name(&stored.plan),
        typed_name,
    )?;
    Ok((stored.plan.commands, stored.class))
}

/// Step one. Runs the write at once when the mode allows it, or answers with what to confirm.
pub async fn prepare(
    conn: &mut Conn,
    table: &CommandTable,
    pending: &PendingStore,
    target: &Target,
    op: RedisWriteOp,
) -> AppResult<RedisWriteOutcome> {
    let plan = plan(&op, table)?;
    let name = command_name(&plan);
    // The gate looks at the real command bytes; several commands count as their strongest.
    let mut class = gate::strongest(
        &plan
            .commands
            .iter()
            .map(|c| gate::classify(table, c, None))
            .collect::<Vec<_>>(),
    );
    let mut count = None;
    if let RedisWriteOp::Delete { key } = &op {
        // Deleting a key with more than 10,000 items is dangerous (it can stall an old server, and it is a lot to lose).
        let info = super::browse::key_info(conn, key).await?;
        if info.key_type == "none" {
            return Err(AppError::not_found("That key is gone."));
        }
        if info.key_type != "string" {
            count = Some(info.length);
            if info.length > gate::BIG {
                class = class.max(Class::Dangerous);
            }
        }
    }
    let level = match gate::decide(target.mode(), class) {
        Decision::Run => return execute(conn, &plan, target, class).await,
        Decision::Refuse(code) => return Err(refuse(target, class, &name, code)),
        Decision::Confirm => RedisConfirmLevel::Confirm,
        Decision::Typed => RedisConfirmLevel::Typed,
    };
    // "Now": what the change replaces. A server that refuses the question just leaves it out.
    let before = match (&op, before_command(&op)) {
        (RedisWriteOp::ListSet { expect, .. } | RedisWriteOp::ListRemove { expect, .. }, _) => {
            shown(expect)
        }
        (_, Some(q)) => send(conn, &[q])
            .await
            .ok()
            .and_then(reply_bytes)
            .map(RedisBytes::from_bytes),
        _ => None,
    };
    let preview = plan
        .commands
        .iter()
        .map(|c| gate::preview(c))
        .collect::<Vec<_>>()
        .join("\n");
    let shown_plan = (plan.target.clone(), plan.after.clone());
    let id = pending.put(plan, class, Instant::now())?;
    Ok(RedisWriteOutcome::Pending {
        pending: RedisPending {
            id,
            level,
            command_preview: preview,
            target: shown_plan.0,
            before,
            after: shown_plan.1,
            count,
        },
    })
}

/// Step two. Runs the kept bytes, once.
pub async fn commit(
    conn: &mut Conn,
    pending: &PendingStore,
    target: &Target,
    pending_id: &str,
    typed_name: Option<&str>,
) -> AppResult<RedisWriteOutcome> {
    let stored = pending.take(pending_id, Instant::now())?;
    approve(
        stored.class,
        target,
        &command_name(&stored.plan),
        typed_name,
    )?;
    execute(conn, &stored.plan, target, stored.class).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> RedisBytes {
        RedisBytes::Text {
            text: s.to_string(),
        }
    }
    fn table() -> CommandTable {
        CommandTable::of(&[
            ("UNLINK", true, false),
            ("DEL", true, false),
            ("SET", true, false),
        ])
    }
    fn lines(op: RedisWriteOp) -> Vec<String> {
        plan(&op, &table())
            .unwrap()
            .commands
            .iter()
            .map(|c| gate::preview(c))
            .collect()
    }
    fn target(mode: RedisWriteMode) -> Target {
        Target {
            name: "cache-prod".into(),
            mode,
            replica: false,
        }
    }
    fn some_plan() -> Plan {
        plan(
            &RedisWriteOp::SetString {
                key: t("k"),
                value: t("v"),
            },
            &table(),
        )
        .unwrap()
    }

    #[test]
    fn every_op_becomes_the_command_it_should() {
        use RedisWriteOp::*;
        let cases: Vec<(RedisWriteOp, &[&str])> = vec![
            (
                SetString {
                    key: t("k"),
                    value: t("v"),
                },
                &["SET k v KEEPTTL"],
            ),
            (
                SetJson {
                    key: t("doc"),
                    text: "{\"a\":1}".into(),
                },
                &["JSON.SET doc $ \"{\\\"a\\\":1}\""],
            ),
            (
                HashSet {
                    key: t("h"),
                    field: t("plan"),
                    value: t("business"),
                },
                &["HSET h plan business"],
            ),
            (
                HashRemove {
                    key: t("h"),
                    field: t("plan"),
                },
                &["HDEL h plan"],
            ),
            (
                ListPush {
                    key: t("l"),
                    value: t("a"),
                    start: true,
                },
                &["LPUSH l a"],
            ),
            (
                ListPush {
                    key: t("l"),
                    value: t("a"),
                    start: false,
                },
                &["RPUSH l a"],
            ),
            (
                ListSet {
                    key: t("l"),
                    index: 3,
                    expect: t("old"),
                    value: t("new"),
                },
                &["LSET l 3 new"],
            ),
            (
                SetAdd {
                    key: t("s"),
                    member: t("m"),
                },
                &["SADD s m"],
            ),
            (
                SetRemove {
                    key: t("s"),
                    member: t("m"),
                },
                &["SREM s m"],
            ),
            (
                ZsetAdd {
                    key: t("z"),
                    member: t("m"),
                    score: " 9.5 ".into(),
                },
                &["ZADD z 9.5 m"],
            ),
            (
                ZsetAdd {
                    key: t("z"),
                    member: t("m"),
                    score: "-inf".into(),
                },
                &["ZADD z -inf m"],
            ),
            (
                ZsetRemove {
                    key: t("z"),
                    member: t("m"),
                },
                &["ZREM z m"],
            ),
            (
                StreamAdd {
                    key: t("x"),
                    id: None,
                    fields: vec![RedisFieldInput {
                        field: t("f"),
                        value: t("v"),
                    }],
                },
                &["XADD x * f v"],
            ),
            (
                StreamAdd {
                    key: t("x"),
                    id: Some("5-1".into()),
                    fields: vec![RedisFieldInput {
                        field: t("f"),
                        value: t("v"),
                    }],
                },
                &["XADD x 5-1 f v"],
            ),
            (
                StreamDelete {
                    key: t("x"),
                    id: "5-1".into(),
                },
                &["XDEL x 5-1"],
            ),
            (
                Expire {
                    key: t("k"),
                    ttl_ms: Some(90_000),
                },
                &["PEXPIRE k 90000"],
            ),
            (
                Expire {
                    key: t("k"),
                    ttl_ms: None,
                },
                &["PERSIST k"],
            ),
            (
                Rename {
                    key: t("a"),
                    to: t("b"),
                },
                &["RENAMENX a b"],
            ),
            (Delete { key: t("k") }, &["UNLINK k"]),
            (
                Create {
                    key: t("k"),
                    key_type: "string".into(),
                    field: None,
                    value: t("v"),
                    score: None,
                    ttl_ms: None,
                },
                &["SET k v NX"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "string".into(),
                    field: None,
                    value: t("v"),
                    score: None,
                    ttl_ms: Some(1000),
                },
                &["SET k v NX PX 1000"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "hash".into(),
                    field: Some(t("f")),
                    value: t("v"),
                    score: None,
                    ttl_ms: Some(1000),
                },
                &["HSET k f v", "PEXPIRE k 1000"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "list".into(),
                    field: None,
                    value: t("v"),
                    score: None,
                    ttl_ms: None,
                },
                &["RPUSH k v"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "set".into(),
                    field: None,
                    value: t("v"),
                    score: None,
                    ttl_ms: None,
                },
                &["SADD k v"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "zset".into(),
                    field: None,
                    value: t("v"),
                    score: Some("2".into()),
                    ttl_ms: None,
                },
                &["ZADD k 2 v"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "stream".into(),
                    field: Some(t("f")),
                    value: t("v"),
                    score: None,
                    ttl_ms: None,
                },
                &["XADD k * f v"],
            ),
            (
                Create {
                    key: t("k"),
                    key_type: "json".into(),
                    field: None,
                    value: t("[1]"),
                    score: None,
                    ttl_ms: None,
                },
                &["JSON.SET k $ [1] NX"],
            ),
            (
                Publish {
                    channel: t("news"),
                    message: t("hi"),
                },
                &["PUBLISH news hi"],
            ),
        ];
        for (op, want) in cases {
            let shown = format!("{op:?}");
            assert_eq!(lines(op), want, "{shown}");
        }
    }

    #[test]
    fn a_binary_key_is_addressed_by_its_bytes() {
        let key = RedisBytes::Binary {
            bytes: vec![0xff, 0x00, b'k'],
        };
        let p = plan(&RedisWriteOp::Delete { key }, &table()).unwrap();
        assert_eq!(p.commands[0][1], vec![0xff, 0x00, b'k']);
        assert_eq!(p.target, "\\xff\\x00k");
    }

    #[test]
    fn removing_a_list_item_marks_its_position_and_removes_the_mark_and_checks_what_was_there() {
        let p = plan(
            &RedisWriteOp::ListRemove {
                key: t("l"),
                index: 2,
                expect: t("old"),
            },
            &table(),
        )
        .unwrap();
        assert_eq!(p.commands.len(), 2);
        assert_eq!(&p.commands[0][..3], &args(&[b"LSET", b"l", b"2"])[..]);
        assert_eq!(&p.commands[1][..3], &args(&[b"LREM", b"l", b"1"])[..]);
        assert_eq!(p.commands[0][3], p.commands[1][3], "the same mark");
        assert!(p.commands[0][3].starts_with(b"__dalan_removed_"));
        assert_eq!(
            p.check,
            Some((args(&[b"LINDEX", b"l", b"2"]), b"old".to_vec()))
        );
        let set = plan(
            &RedisWriteOp::ListSet {
                key: t("l"),
                index: 2,
                expect: t("old"),
                value: t("new"),
            },
            &table(),
        )
        .unwrap();
        assert_eq!(set.check, p.check);
    }

    #[test]
    fn creating_and_renaming_never_overwrite() {
        let create = |ty: &str| {
            plan(
                &RedisWriteOp::Create {
                    key: t("k"),
                    key_type: ty.into(),
                    field: Some(t("f")),
                    value: t("1"),
                    score: None,
                    ttl_ms: None,
                },
                &table(),
            )
            .unwrap()
        };
        for ty in ["string", "json", "hash", "list", "set", "zset", "stream"] {
            assert_eq!(
                create(ty).check,
                Some((args(&[b"EXISTS", b"k"]), b"0".to_vec())),
                "{ty}"
            );
        }
        assert_eq!(create("string").exists_reply, Some(Value::Nil));
        assert_eq!(create("hash").exists_reply, None);
        let rename = plan(
            &RedisWriteOp::Rename {
                key: t("a"),
                to: t("b"),
            },
            &table(),
        )
        .unwrap();
        assert_eq!(rename.exists_reply, Some(Value::Int(0)));
    }

    #[test]
    fn delete_falls_back_to_del_on_a_server_without_unlink() {
        let old = CommandTable::of(&[("DEL", true, false)]);
        let p = plan(&RedisWriteOp::Delete { key: t("k") }, &old).unwrap();
        assert_eq!(p.commands[0][0], b"DEL");
        // A table that could not be read: UNLINK (every supported server has it).
        let p = plan(
            &RedisWriteOp::Delete { key: t("k") },
            &CommandTable::default(),
        )
        .unwrap();
        assert_eq!(p.commands[0][0], b"UNLINK");
    }

    #[test]
    fn bad_input_is_refused_before_anything_is_sent() {
        use RedisWriteOp::*;
        let bad = vec![
            SetString {
                key: t(""),
                value: t("v"),
            },
            SetJson {
                key: t("k"),
                text: "{nope".into(),
            },
            ZsetAdd {
                key: t("z"),
                member: t("m"),
                score: "high".into(),
            },
            ZsetAdd {
                key: t("z"),
                member: t("m"),
                score: "nan".into(),
            },
            StreamAdd {
                key: t("x"),
                id: None,
                fields: vec![],
            },
            StreamAdd {
                key: t("x"),
                id: Some("1; FLUSHALL".into()),
                fields: vec![RedisFieldInput {
                    field: t("f"),
                    value: t("v"),
                }],
            },
            StreamDelete {
                key: t("x"),
                id: "".into(),
            },
            Expire {
                key: t("k"),
                ttl_ms: Some(0),
            },
            Rename {
                key: t("a"),
                to: t("a"),
            },
            Rename {
                key: t("a"),
                to: t(""),
            },
            Create {
                key: t("k"),
                key_type: "hash".into(),
                field: None,
                value: t("v"),
                score: None,
                ttl_ms: None,
            },
            Create {
                key: t("k"),
                key_type: "nope".into(),
                field: None,
                value: t("v"),
                score: None,
                ttl_ms: None,
            },
            Create {
                key: t("k"),
                key_type: "json".into(),
                field: None,
                value: t("{"),
                score: None,
                ttl_ms: None,
            },
            Publish {
                channel: t(""),
                message: t("m"),
            },
        ];
        for op in bad {
            let shown = format!("{op:?}");
            assert_eq!(
                plan(&op, &table()).err().map(|e| e.code),
                Some(ErrorCode::Validation),
                "{shown}"
            );
        }
    }

    #[test]
    fn every_editor_op_is_a_write_for_the_gate() {
        // A table that knows nothing: unknown means write, so no editor op can slip through as a read.
        let ops = vec![
            RedisWriteOp::SetString {
                key: t("k"),
                value: t("v"),
            },
            RedisWriteOp::SetJson {
                key: t("k"),
                text: "1".into(),
            },
            RedisWriteOp::ListRemove {
                key: t("l"),
                index: 0,
                expect: t("a"),
            },
            RedisWriteOp::Expire {
                key: t("k"),
                ttl_ms: None,
            },
            RedisWriteOp::Delete { key: t("k") },
            RedisWriteOp::Publish {
                channel: t("c"),
                message: t("m"),
            },
        ];
        for op in ops {
            let p = plan(&op, &CommandTable::default()).unwrap();
            for c in &p.commands {
                assert_eq!(
                    gate::classify(&CommandTable::default(), c, None),
                    Class::Write,
                    "{}",
                    gate::preview(c)
                );
            }
        }
    }

    #[test]
    fn a_prepared_write_commits_once() {
        let store = PendingStore::default();
        let now = Instant::now();
        let id = store.put(some_plan(), Class::Write, now).unwrap();
        assert!(store.take(&id, now).is_ok());
        assert_eq!(
            store.take(&id, now).err().map(|e| e.code),
            Some(ErrorCode::RedisConfirmNeeded)
        );
    }

    #[test]
    fn a_prepared_write_expires_after_two_minutes() {
        let store = PendingStore::default();
        let now = Instant::now();
        let id = store.put(some_plan(), Class::Write, now).unwrap();
        let late = now + PENDING_FOR;
        assert_eq!(
            store.take(&id, late).err().map(|e| e.code),
            Some(ErrorCode::RedisConfirmNeeded)
        );
        // and it is gone, not just refused once
        assert!(store.take(&id, now).is_err());
        // just inside the two minutes is fine
        let id = store.put(some_plan(), Class::Write, now).unwrap();
        assert!(store
            .take(&id, now + PENDING_FOR - Duration::from_secs(1))
            .is_ok());
        // old confirmations are swept when a new one is stored
        store.put(some_plan(), Class::Write, now).unwrap();
        store
            .put(some_plan(), Class::Write, now + PENDING_FOR)
            .unwrap();
        assert_eq!(store.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_prepared_write_belongs_to_its_connection() {
        let (mine, other) = (PendingStore::default(), PendingStore::default());
        let now = Instant::now();
        let id = mine.put(some_plan(), Class::Write, now).unwrap();
        assert_eq!(
            other.take(&id, now).err().map(|e| e.code),
            Some(ErrorCode::RedisConfirmNeeded)
        );
        assert!(
            mine.take(&id, now).is_ok(),
            "asking the wrong connection does not use it up"
        );
        assert_eq!(
            mine.take("made-up", now).err().map(|e| e.code),
            Some(ErrorCode::RedisConfirmNeeded)
        );
    }

    #[test]
    fn a_typed_confirmation_needs_the_exact_name() {
        let db = target(RedisWriteMode::Confirm);
        for wrong in [
            None,
            Some(""),
            Some("cache-pro"),
            Some("Cache-Prod"),
            Some("cache-prod "),
            Some(" cache-prod"),
        ] {
            assert_eq!(
                approve(Class::Dangerous, &db, "UNLINK", wrong)
                    .err()
                    .map(|e| e.code),
                Some(ErrorCode::RedisConfirmNeeded),
                "{wrong:?}"
            );
        }
        assert!(approve(Class::Dangerous, &db, "UNLINK", Some("cache-prod")).is_ok());
        assert!(approve(
            Class::Dangerous,
            &target(RedisWriteMode::Open),
            "UNLINK",
            Some("cache-prod")
        )
        .is_ok());
        // An ordinary confirmation needs no name.
        assert!(approve(Class::Write, &db, "SET", None).is_ok());
    }

    #[test]
    fn the_mode_is_asked_again_at_commit() {
        // Prepared in Confirm mode; the database became Read-only (or turned out to be a replica) before the commit.
        let ro = target(RedisWriteMode::ReadOnly);
        assert_eq!(
            approve(Class::Write, &ro, "SET", None)
                .err()
                .map(|e| e.code),
            Some(ErrorCode::RedisReadOnlyMode)
        );
        assert_eq!(
            approve(Class::Dangerous, &ro, "UNLINK", Some("cache-prod"))
                .err()
                .map(|e| e.code),
            Some(ErrorCode::RedisReadOnlyMode)
        );
        let replica = Target {
            replica: true,
            ..target(RedisWriteMode::Open)
        };
        let e = approve(Class::Write, &replica, "SET", None).unwrap_err();
        assert_eq!(e.code, ErrorCode::RedisReadOnlyMode);
        assert!(e.message.contains("replica"));
    }
}
