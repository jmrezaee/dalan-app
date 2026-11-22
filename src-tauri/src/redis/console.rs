//! The console (F16; S30; replica/redis/spec.md). A line is split into arguments the way a shell would, shown to
//! the write gate, and then run, refused, or held for a confirmation. Replies are printed here, in Rust, in a
//! reading-friendly form, so the bytes of a value never have to become a JavaScript string by guesswork.
//!
//! The tab has one connection that the Browser, the keep-alive and the Console share. Commands that would take
//! that connection over (`MULTI` on the wire, blocking pops, `HELLO`, `CLIENT REPLY`) are handled or refused here:
//! `MULTI` queues in this module and `EXEC` sends the queue as one transaction.

use std::sync::Mutex;
use std::time::Instant;

use redis::Value;
use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use super::cluster::Conn;
use super::gate::{self, Class, CommandTable, Decision};
use super::write::{self, Args, PendingStore, RedisConfirmLevel, RedisPending, Target};
use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::history::{open_command, seal_command};
use crate::vault::VaultSession;

/// A reply is cut here; the UI shows the first 1,000 lines and offers the rest.
pub const MAX_LINES: usize = 10_000;
const KEEP_HISTORY: i64 = 500;

/// Splits a console line into arguments. Spaces separate; `"..."` keeps spaces and understands `\n`, `\r`, `\t`,
/// `\\`, `\"` and `\xNN` (so binary can be typed); `'...'` is literal except `\'`.
pub fn parse_line(line: &str) -> AppResult<Args> {
    let bad = |what: &str| AppError::validation(format!("{what} Nothing was sent."));
    let bytes = line.as_bytes();
    let mut args: Args = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let mut arg = Vec::new();
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                loop {
                    let Some(&b) = bytes.get(i) else {
                        return Err(bad("A quote is never closed."));
                    };
                    i += 1;
                    if b == quote {
                        break;
                    }
                    if b != b'\\' {
                        arg.push(b);
                        continue;
                    }
                    let Some(&next) = bytes.get(i) else {
                        return Err(bad("A quote is never closed."));
                    };
                    i += 1;
                    if quote == b'\'' {
                        // Single quotes: only \' means something.
                        if next != b'\'' {
                            arg.push(b'\\');
                        }
                        arg.push(next);
                        continue;
                    }
                    match next {
                        b'n' => arg.push(b'\n'),
                        b'r' => arg.push(b'\r'),
                        b't' => arg.push(b'\t'),
                        b'a' => arg.push(7),
                        b'b' => arg.push(8),
                        b'x' => {
                            let hex = bytes
                                .get(i..i + 2)
                                .and_then(|h| std::str::from_utf8(h).ok())
                                .and_then(|h| u8::from_str_radix(h, 16).ok())
                                .ok_or_else(|| bad("\\x needs two hex digits, like \\x0a."))?;
                            arg.push(hex);
                            i += 2;
                        }
                        other => arg.push(other),
                    }
                }
                // redis-cli's rule: a closing quote is followed by a space or the end.
                if bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace()) {
                    return Err(bad("Put a space after a closing quote."));
                }
            }
            _ => {
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    arg.push(bytes[i]);
                    i += 1;
                }
            }
        }
        args.push(arg);
    }
    Ok(args)
}

fn upper(arg: Option<&Vec<u8>>) -> String {
    arg.map(|a| String::from_utf8_lossy(a).to_ascii_uppercase())
        .unwrap_or_default()
}

/// Commands that carry a password are never written to history.
pub fn keep_in_history(args: &[Vec<u8>]) -> bool {
    let has = |word: &str| {
        args.iter()
            .skip(1)
            .any(|a| a.eq_ignore_ascii_case(word.as_bytes()))
    };
    match (upper(args.first()).as_str(), upper(args.get(1)).as_str()) {
        ("AUTH", _) | ("ACL", "SETUSER") => false,
        ("HELLO", _) => !has("AUTH"),
        ("MIGRATE", _) => !(has("AUTH") || has("AUTH2")),
        ("CONFIG", "SET") => !args.iter().skip(2).any(|a| {
            ["requirepass", "masterauth", "masteruser"]
                .iter()
                .any(|secret| a.eq_ignore_ascii_case(secret.as_bytes()))
        }),
        ("", _) => false,
        _ => true,
    }
}

/// Why the console won't send this, when it is about the shared connection and not about safety (that is the gate's).
fn unsupported(args: &[Vec<u8>]) -> Option<&'static str> {
    let has = |word: &str| {
        args.iter()
            .skip(1)
            .any(|a| a.eq_ignore_ascii_case(word.as_bytes()))
    };
    match (upper(args.first()).as_str(), upper(args.get(1)).as_str()) {
        (
            "BLPOP" | "BRPOP" | "BLMOVE" | "BRPOPLPUSH" | "BLMPOP" | "BZPOPMIN" | "BZPOPMAX"
            | "BZMPOP" | "WAIT" | "WAITAOF",
            _,
        ) => Some("It waits on the server, and that would hold this tab's connection."),
        ("XREAD" | "XREADGROUP", _) if has("BLOCK") => {
            Some("With BLOCK it waits on the server, and that would hold this tab's connection.")
        }
        ("HELLO" | "RESET" | "QUIT", _) | ("CLIENT", "REPLY") => {
            Some("It would change how this tab's connection talks to the server.")
        }
        ("AUTH", _) => {
            Some("The sign-in belongs to the database's settings. Edit the database to change it.")
        }
        ("WATCH" | "UNWATCH", _) => {
            Some("This tab's connection is shared with the Browser, so WATCH can't be kept.")
        }
        _ => None,
    }
}

/// The safe form the console offers instead of a heavy command.
fn safer(args: &[Vec<u8>]) -> Option<String> {
    let arg = |i: usize| args.get(i).map(|a| gate::preview(std::slice::from_ref(a)));
    Some(match upper(args.first()).as_str() {
        "KEYS" => format!("SCAN 0 MATCH {} COUNT 500", arg(1)?),
        "SMEMBERS" => format!("SSCAN {} 0 COUNT 500", arg(1)?),
        "HGETALL" | "HKEYS" | "HVALS" => format!("HSCAN {} 0 COUNT 500", arg(1)?),
        "LRANGE" => format!("LRANGE {} 0 499", arg(1)?),
        "ZRANGE" => format!("ZRANGE {} 0 499", arg(1)?),
        "ZREVRANGE" => format!("ZREVRANGE {} 0 499", arg(1)?),
        _ => return None,
    })
}

// ---- Printing a reply.

fn quoted(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('"');
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            for c in text.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
                    c => out.push(c),
                }
            }
        }
        // Not text: every byte that isn't plain ASCII is shown as \xNN.
        Err(_) => {
            for &b in bytes {
                match b {
                    b'"' => out.push_str("\\\""),
                    b'\\' => out.push_str("\\\\"),
                    0x20..=0x7e => out.push(b as char),
                    _ => out.push_str(&format!("\\x{b:02x}")),
                }
            }
        }
    }
    out.push('"');
    out
}

/// One reply as lines, in the style people know from `redis-cli`: `(integer) 3`, `"text"`, `1) ...` with nested
/// lists indented under their number. Stops adding lines at `MAX_LINES`.
fn print(v: &Value, first: &str, rest: &str, out: &mut Vec<String>) {
    if out.len() >= MAX_LINES {
        return;
    }
    let mut line = |text: String| out.push(format!("{first}{text}"));
    let list = |items: Vec<(String, &Value)>, empty: &str, out: &mut Vec<String>| {
        if items.is_empty() {
            out.push(format!("{first}{empty}"));
            return;
        }
        let width = items.len().to_string().len();
        for (i, (mark, item)) in items.iter().enumerate() {
            let label = format!("{:>width$}{mark} ", i + 1);
            let lead = if i == 0 {
                first.to_string()
            } else {
                rest.to_string()
            };
            let pad = format!("{rest}{}", " ".repeat(label.len()));
            print(item, &format!("{lead}{label}"), &pad, out);
        }
    };
    match v {
        Value::Nil => line("(nil)".into()),
        Value::Int(i) => line(format!("(integer) {i}")),
        Value::Double(d) => line(format!("(double) {d}")),
        Value::Boolean(b) => line(format!("({b})")),
        Value::Okay => line("OK".into()),
        Value::SimpleString(s) => line(s.clone()),
        Value::BulkString(b) => line(quoted(b)),
        Value::VerbatimString { text, .. } => {
            // INFO-like text: one console line per line of the reply.
            let mut lines = text.lines();
            line(lines.next().unwrap_or_default().to_string());
            for l in lines {
                if out.len() >= MAX_LINES {
                    break;
                }
                out.push(format!("{rest}{l}"));
            }
        }
        Value::BigNumber(n) => line(format!("(big number) {n:?}")),
        Value::ServerError(e) => line(format!("(error) {}", server_error_text(e))),
        Value::Array(items) => list(
            items.iter().map(|i| (")".to_string(), i)).collect(),
            "(empty array)",
            out,
        ),
        Value::Set(items) => list(
            items.iter().map(|i| ("~".to_string(), i)).collect(),
            "(empty set)",
            out,
        ),
        Value::Map(pairs) => {
            if pairs.is_empty() {
                return line("(empty hash)".into());
            }
            let width = pairs.len().to_string().len();
            for (i, (k, val)) in pairs.iter().enumerate() {
                let label = format!("{:>width$}# ", i + 1);
                let lead = if i == 0 {
                    first.to_string()
                } else {
                    rest.to_string()
                };
                let mut key = Vec::new();
                print(k, "", "", &mut key);
                let key = key.join(" ");
                let pad = format!(
                    "{rest}{}",
                    " ".repeat(label.len() + key.chars().count() + 4)
                );
                print(val, &format!("{lead}{label}{key} => "), &pad, out);
            }
        }
        Value::Attribute { data, .. } => print(data, first, rest, out),
        Value::Push { data, .. } => list(
            data.iter().map(|i| (")".to_string(), i)).collect(),
            "(empty push)",
            out,
        ),
        // `Value` can grow new kinds; show them as the crate prints them rather than fail.
        other => line(format!("{other:?}")),
    }
}

fn server_error_text(e: &redis::ServerError) -> String {
    match e.details() {
        Some(d) => format!("{} {d}", e.code()),
        None => e.code().to_string(),
    }
}

/// What the console prints for one command.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisConsoleReply {
    pub lines: Vec<String>,
    /// The reply had more than 10,000 lines; the rest was left out.
    pub truncated: bool,
    /// The server answered with an error of its own (the lines hold it).
    pub error: bool,
    pub elapsed_ms: f64,
    /// `SELECT n` ran: the tab is on this logical database now.
    pub selected: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RedisConsoleOutcome {
    Reply {
        reply: RedisConsoleReply,
    },
    /// Not sent yet. `heavy`: it only reads, but a lot; `safer` is the form to offer instead.
    Pending {
        pending: RedisPending,
        heavy: bool,
        safer: Option<String>,
    },
}

fn reply_of(value: &Value, started: Instant, selected: Option<u32>) -> RedisConsoleReply {
    let mut lines = Vec::new();
    print(value, "", "", &mut lines);
    let truncated = lines.len() >= MAX_LINES;
    RedisConsoleReply {
        lines,
        truncated,
        error: matches!(value, Value::ServerError(_)),
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        selected,
    }
}

fn text_reply(text: &str, error: bool) -> RedisConsoleOutcome {
    RedisConsoleOutcome::Reply {
        reply: RedisConsoleReply {
            lines: vec![text.to_string()],
            truncated: false,
            error,
            elapsed_ms: 0.0,
            selected: None,
        },
    }
}

/// Sends commands and prints the answer. A server's own error is printed, not thrown: it is the reply.
async fn run(
    conn: &mut Conn,
    commands: &[Args],
    selected: Option<u32>,
) -> AppResult<RedisConsoleReply> {
    let started = Instant::now();
    match write::send(conn, commands).await {
        Ok(value) => Ok(reply_of(&value, started, selected)),
        Err(e) if e.code == ErrorCode::RedisRefused => Ok(RedisConsoleReply {
            lines: vec![format!("(error) {}", e.message)],
            truncated: false,
            error: true,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            selected: None,
        }),
        Err(e) => Err(e),
    }
}

/// The commands queued since `MULTI`, with the class of each. `None`: no transaction is open.
#[derive(Default)]
pub struct ConsoleState(Mutex<Queue>);

/// `None`: no `MULTI` is open.
type Queue = Option<Vec<(Args, Class)>>;

impl ConsoleState {
    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Queue>> {
        self.0
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))
    }
}

/// How big the collection is that an unbounded read would return (for the gate's "heavy").
async fn length_for(conn: &mut Conn, args: &[Vec<u8>]) -> Option<u64> {
    let size = gate::unbounded_read(args)?;
    let key = args.get(1)?;
    redis::cmd(size)
        .arg(key.as_slice())
        .query_async::<u64>(conn)
        .await
        .ok()
}

/// Runs one console line through the gate.
pub async fn exec(
    conn: &mut Conn,
    table: &CommandTable,
    pending: &PendingStore,
    state: &ConsoleState,
    target: &Target,
    line: &str,
) -> AppResult<RedisConsoleOutcome> {
    let args = parse_line(line)?;
    if args.is_empty() {
        return Err(AppError::validation("Type a command."));
    }
    let name = upper(args.first());
    if let Some(why) = unsupported(&args) {
        log::warn!("redis: refused on {} (console): {name}", target.name);
        return Err(AppError::new(
            ErrorCode::RedisRefused,
            format!("Not sent. {why}"),
        ));
    }

    // ---- Transactions are queued here and sent whole by EXEC.
    match name.as_str() {
        "MULTI" => {
            let mut q = state.lock()?;
            if q.is_some() {
                return Ok(text_reply(
                    "(error) ERR MULTI calls can not be nested",
                    true,
                ));
            }
            *q = Some(Vec::new());
            return Ok(text_reply("OK", false));
        }
        "DISCARD" => {
            return Ok(match state.lock()?.take() {
                Some(_) => text_reply("OK", false),
                None => text_reply("(error) ERR DISCARD without MULTI", true),
            });
        }
        "EXEC" => {
            let Some(queue) = state.lock()?.take() else {
                return Ok(text_reply("(error) ERR EXEC without MULTI", true));
            };
            if queue.is_empty() {
                return Ok(text_reply("(empty array)", false));
            }
            let class = gate::strongest(&queue.iter().map(|(_, c)| *c).collect::<Vec<_>>());
            let commands: Vec<Args> = queue.into_iter().map(|(a, _)| a).collect();
            return finish(conn, pending, target, commands, class, "EXEC", None, None).await;
        }
        _ => {}
    }

    let length = length_for(conn, &args).await;
    let class = gate::classify(table, &args, length);
    if let Some(queue) = state.lock()?.as_mut() {
        // Inside MULTI: classify as it is queued. A command the mode refuses is not queued.
        if let Decision::Refuse(code) = gate::decide(target.mode(), class) {
            return Err(write::refuse(target, class, &name, code));
        }
        queue.push((args, class));
        return Ok(text_reply("QUEUED", false));
    }
    let selected = (name == "SELECT")
        .then(|| std::str::from_utf8(args.get(1)?).ok()?.parse::<u32>().ok())
        .flatten();
    let offer = if class == Class::Heavy {
        safer(&args)
    } else {
        None
    };
    finish(
        conn,
        pending,
        target,
        vec![args],
        class,
        &name,
        selected,
        offer,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finish(
    conn: &mut Conn,
    pending: &PendingStore,
    target: &Target,
    commands: Vec<Args>,
    class: Class,
    name: &str,
    selected: Option<u32>,
    safer: Option<String>,
) -> AppResult<RedisConsoleOutcome> {
    let level = match gate::decide(target.mode(), class) {
        Decision::Run => {
            if class != Class::Read {
                log::info!("redis: write on {} ({class:?}): {name}", target.name);
            }
            let reply = run(conn, &commands, selected).await?;
            return Ok(RedisConsoleOutcome::Reply { reply });
        }
        Decision::Refuse(code) => return Err(write::refuse(target, class, name, code)),
        Decision::Confirm => RedisConfirmLevel::Confirm,
        Decision::Typed => RedisConfirmLevel::Typed,
    };
    let preview = commands
        .iter()
        .map(|c| gate::preview(c))
        .collect::<Vec<_>>()
        .join("\n");
    let id = write::hold(pending, commands, class, name.to_string())?;
    Ok(RedisConsoleOutcome::Pending {
        pending: RedisPending {
            id,
            level,
            command_preview: preview,
            target: name.to_string(),
            before: None,
            after: None,
            count: None,
        },
        heavy: class == Class::Heavy,
        safer,
    })
}

/// "Send", "Run anyway" or the typed-name dialog's button: runs what `exec` held, once.
pub async fn commit(
    conn: &mut Conn,
    pending: &PendingStore,
    target: &Target,
    pending_id: &str,
    typed_name: Option<&str>,
) -> AppResult<RedisConsoleReply> {
    let (commands, class) = write::release(pending, target, pending_id, typed_name)?;
    let name = upper(commands.first().and_then(|c| c.first()));
    if class != Class::Heavy {
        log::info!("redis: write on {} ({class:?}): {name}", target.name);
    }
    let selected = (commands.len() == 1 && name == "SELECT")
        .then(|| {
            std::str::from_utf8(commands[0].get(1)?)
                .ok()?
                .parse::<u32>()
                .ok()
        })
        .flatten();
    run(conn, &commands, selected).await
}

// ---- History: the lines typed for one database, encrypted like terminal history.

/// Settings > Databases > "Keep console history". On unless it was turned off.
pub fn history_enabled(conn: &Connection) -> bool {
    crate::settings::get_all(conn)
        .ok()
        .and_then(|m| {
            m.get("redis")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        })
        .and_then(|v| v.get("keep_history").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// Saves a console line, unless it carries a password. The same line twice in a row is kept once; 500 per database.
pub fn remember(conn: &Connection, s: &VaultSession, db_id: &str, line: &str) -> AppResult<()> {
    let line = line.trim();
    match parse_line(line) {
        Ok(args) if keep_in_history(&args) => {}
        _ => return Ok(()),
    }
    let last: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT id, body_enc FROM database_history WHERE database_id = ?1 AND kind = 'command' ORDER BY ran_at DESC, rowid DESC LIMIT 1",
            params![db_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    if let Some((id, enc)) = last {
        if open_command(s, &id, &enc).as_deref() == Some(line) {
            return Ok(());
        }
    }
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO database_history (id, database_id, kind, body_enc, ran_at) VALUES (?1, ?2, 'command', ?3, ?4)",
        params![id, db_id, seal_command(s, &id, line)?, now_ms()],
    )?;
    conn.execute(
        "DELETE FROM database_history WHERE database_id = ?1 AND kind = 'command' AND rowid NOT IN (SELECT rowid FROM database_history WHERE database_id = ?1 AND kind = 'command' ORDER BY ran_at DESC, rowid DESC LIMIT ?2)",
        params![db_id, KEEP_HISTORY],
    )?;
    Ok(())
}

/// The database's history, newest first.
pub fn history(conn: &Connection, s: &VaultSession, db_id: &str) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.body_enc FROM database_history h JOIN databases d ON d.id = h.database_id
         WHERE h.database_id = ?1 AND h.kind = 'command' AND d.vault_id = ?2 ORDER BY h.ran_at DESC, h.rowid DESC",
    )?;
    let rows = stmt.query_map(params![db_id, s.vault_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, enc) = row?;
        if let Some(line) = open_command(s, &id, &enc) {
            out.push(line);
        }
    }
    Ok(out)
}

pub fn clear_history(conn: &Connection, s: &VaultSession, db_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM database_history WHERE kind = 'command' AND database_id IN (SELECT id FROM databases WHERE id = ?1 AND vault_id = ?2)",
        params![db_id, s.vault_id],
    )?;
    Ok(())
}

// ---- Completion: the server's commands with their syntax.

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisCommandInfo {
    pub name: String,
    /// `key field value [field value ...]`, from the server's own documentation when it has it (Redis 7).
    pub syntax: String,
    pub summary: Option<String>,
    pub write: bool,
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(b) => String::from_utf8(b.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        Value::VerbatimString { text, .. } => Some(text.clone()),
        _ => None,
    }
}

/// A RESP2 flat list or a RESP3 map, as (name, value) pairs.
fn fields(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Map(pairs) => pairs
            .iter()
            .filter_map(|(k, val)| Some((text_of(k)?, val)))
            .collect(),
        Value::Array(items) => items
            .chunks(2)
            .filter_map(|c| Some((text_of(c.first()?)?, c.get(1)?)))
            .collect(),
        _ => Vec::new(),
    }
}

/// One argument of `COMMAND DOCS` as people write syntax: `[COUNT count]`, `field value [field value ...]`, `NX|XX`.
fn argument_syntax(arg: &Value) -> String {
    let (mut name, mut kind, mut token, mut optional, mut multiple, mut children) =
        (String::new(), String::new(), None, false, false, Vec::new());
    for (k, v) in fields(arg) {
        match k.as_str() {
            "name" => name = text_of(v).unwrap_or_default(),
            "type" => kind = text_of(v).unwrap_or_default(),
            "token" => token = text_of(v),
            "flags" => {
                if let Value::Array(f) | Value::Set(f) = v {
                    for flag in f.iter().filter_map(text_of) {
                        optional |= flag == "optional";
                        multiple |= flag.starts_with("multiple");
                    }
                }
            }
            "arguments" => {
                if let Value::Array(a) = v {
                    children = a.iter().map(argument_syntax).collect();
                }
            }
            _ => {}
        }
    }
    let body = match kind.as_str() {
        "pure-token" => String::new(),
        "block" => children.join(" "),
        "oneof" => children.join("|"),
        _ => name,
    };
    let mut s = match (token, body.is_empty()) {
        (Some(t), true) => t,
        (Some(t), false) => format!("{t} {body}"),
        (None, _) => body,
    };
    if multiple {
        s = format!("{s} [{s} ...]");
    }
    if optional {
        s = format!("[{s}]");
    }
    s
}

/// Without the server's documentation: what the arity says.
fn arity_syntax(arity: i64) -> String {
    match arity {
        1 => String::new(),
        n if n > 1 => (1..n).map(|_| "arg").collect::<Vec<_>>().join(" "),
        n => {
            let least = (-n - 1).max(0) as usize;
            let mut parts = vec!["arg"; least];
            parts.push("[arg ...]");
            parts.join(" ")
        }
    }
}

fn docs_map(docs: &Value) -> std::collections::HashMap<String, (String, Option<String>)> {
    fields(docs)
        .into_iter()
        .map(|(name, doc)| {
            let mut syntax = String::new();
            let mut summary = None;
            for (k, v) in fields(doc) {
                match k.as_str() {
                    "summary" => summary = text_of(v),
                    "arguments" => {
                        if let Value::Array(a) = v {
                            syntax = a.iter().map(argument_syntax).collect::<Vec<_>>().join(" ");
                        }
                    }
                    _ => {}
                }
            }
            (name.to_ascii_uppercase(), (syntax, summary))
        })
        .collect()
}

/// The commands this server has, for Tab completion and the syntax line.
pub async fn commands(conn: &mut Conn, table: &CommandTable) -> Vec<RedisCommandInfo> {
    // `COMMAND DOCS` exists from Redis 7; older servers get the arity instead.
    let docs = redis::cmd("COMMAND")
        .arg("DOCS")
        .query_async::<Value>(conn)
        .await
        .map(|v| docs_map(&v))
        .unwrap_or_default();
    table
        .commands()
        .into_iter()
        .map(|(name, flags)| {
            let doc = docs.get(name);
            RedisCommandInfo {
                name: name.to_string(),
                syntax: doc
                    .map(|d| d.0.clone())
                    .unwrap_or_else(|| arity_syntax(flags.arity)),
                summary: doc.and_then(|d| d.1.clone()),
                write: flags.write,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn words(line: &str) -> Vec<String> {
        parse_line(line)
            .unwrap()
            .iter()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    #[test]
    fn a_line_is_split_like_a_shell_would() {
        assert_eq!(words("GET k"), ["GET", "k"]);
        assert_eq!(words("  SET   k    v  "), ["SET", "k", "v"]);
        assert_eq!(words(r#"SET k "hello world""#), ["SET", "k", "hello world"]);
        assert_eq!(words(r#"SET k 'hello world'"#), ["SET", "k", "hello world"]);
        assert_eq!(words(r#"SET k """#), ["SET", "k", ""]);
        assert_eq!(words(r#"SET k "say \"hi\"""#), ["SET", "k", "say \"hi\""]);
        assert_eq!(words(r#"SET k "a\nb\tc\\d""#), ["SET", "k", "a\nb\tc\\d"]);
        assert_eq!(words(r#"SET k 'it\'s' "#), ["SET", "k", "it's"]);
        assert_eq!(
            words(r#"SET k 'a\nb'"#),
            ["SET", "k", "a\\nb"],
            "single quotes are literal"
        );
        assert_eq!(
            words(r#"HSET h f '{"a": 1}'"#),
            ["HSET", "h", "f", "{\"a\": 1}"]
        );
        assert_eq!(words("SET k سلام"), ["SET", "k", "سلام"]);
        assert!(parse_line("").unwrap().is_empty());
        assert!(parse_line("   ").unwrap().is_empty());
    }

    #[test]
    fn binary_can_be_typed_and_mistakes_are_refused() {
        assert_eq!(
            parse_line(r#"SET k "\xff\x00A""#).unwrap()[2],
            vec![0xff, 0x00, b'A']
        );
        for bad in [
            r#"SET k "open"#,
            "SET k 'open",
            r#"SET k "a\"#,
            r#"SET k "\xZZ""#,
            r#"SET k "\x1""#,
            r#"SET k "a"b"#,
        ] {
            assert_eq!(
                parse_line(bad).err().map(|e| e.code),
                Some(ErrorCode::Validation),
                "{bad}"
            );
        }
    }

    #[test]
    fn commands_that_carry_a_password_are_not_kept() {
        let keep = |line: &str| keep_in_history(&parse_line(line).unwrap());
        for line in [
            "AUTH hunter2",
            "auth default hunter2",
            "HELLO 3 AUTH default hunter2",
            "hello 3 auth u p",
            "ACL SETUSER app on >hunter2 ~* +@all",
            "acl setuser app",
            "CONFIG SET requirepass hunter2",
            "config set masterauth hunter2",
            "CONFIG SET masteruser repl",
            "CONFIG SET maxmemory 1gb requirepass hunter2",
            "MIGRATE h 6379 k 0 1000 AUTH hunter2",
            "MIGRATE h 6379 k 0 1000 AUTH2 u hunter2",
            "",
        ] {
            assert!(!keep(line), "{line}");
        }
        for line in [
            "GET k",
            "HELLO 3",
            "CONFIG SET maxmemory 1gb",
            "CONFIG GET requirepass",
            "ACL LIST",
            "MIGRATE h 6379 k 0 1000",
            "SET auth 1",
        ] {
            assert!(keep(line), "{line}");
        }
    }

    #[test]
    fn commands_that_would_take_the_shared_connection_are_not_sent() {
        let no = |line: &str| unsupported(&parse_line(line).unwrap()).is_some();
        for line in [
            "BLPOP q 0",
            "brpop q 5",
            "BZPOPMIN z 0",
            "XREAD BLOCK 0 STREAMS s $",
            "xreadgroup GROUP g c block 100 STREAMS s >",
            "WAIT 1 0",
            "HELLO 2",
            "CLIENT REPLY OFF",
            "RESET",
            "QUIT",
            "AUTH x",
            "WATCH k",
        ] {
            assert!(no(line), "{line}");
        }
        for line in [
            "LPOP q",
            "XREAD COUNT 10 STREAMS s 0",
            "CLIENT LIST",
            "GET block",
        ] {
            assert!(!no(line), "{line}");
        }
    }

    #[test]
    fn a_heavy_command_gets_a_safe_form() {
        let offer = |line: &str| safer(&parse_line(line).unwrap());
        assert_eq!(
            offer("KEYS user:*").as_deref(),
            Some("SCAN 0 MATCH user:* COUNT 500")
        );
        assert_eq!(
            offer("SMEMBERS audit:ids").as_deref(),
            Some("SSCAN audit:ids 0 COUNT 500")
        );
        assert_eq!(
            offer("HGETALL cache:catalog").as_deref(),
            Some("HSCAN cache:catalog 0 COUNT 500")
        );
        assert_eq!(offer("LRANGE q 0 -1").as_deref(), Some("LRANGE q 0 499"));
        assert_eq!(offer("ZRANGE z 0 -1").as_deref(), Some("ZRANGE z 0 499"));
        assert_eq!(
            offer(r#"KEYS "a b*""#).as_deref(),
            Some("SCAN 0 MATCH a b* COUNT 500")
        );
        assert_eq!(offer("GET k"), None);
    }

    fn printed(v: Value) -> Vec<String> {
        let mut out = Vec::new();
        print(&v, "", "", &mut out);
        out
    }
    fn b(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }

    #[test]
    fn replies_are_printed_with_their_types() {
        assert_eq!(printed(Value::Nil), ["(nil)"]);
        assert_eq!(printed(Value::Int(83640)), ["(integer) 83640"]);
        assert_eq!(printed(Value::Okay), ["OK"]);
        assert_eq!(printed(Value::SimpleString("PONG".into())), ["PONG"]);
        assert_eq!(printed(b("Sara Moradi")), ["\"Sara Moradi\""]);
        assert_eq!(printed(b("a\"b\\c\nd")), ["\"a\\\"b\\\\c\\nd\""]);
        assert_eq!(
            printed(Value::BulkString(vec![0xff, 0x00, b'A'])),
            ["\"\\xff\\x00A\""]
        );
        assert_eq!(printed(Value::Double(1.5)), ["(double) 1.5"]);
        assert_eq!(printed(Value::Boolean(true)), ["(true)"]);
        assert_eq!(printed(Value::Array(vec![])), ["(empty array)"]);
        assert_eq!(
            printed(Value::Array(vec![b("name"), b("Sara"), Value::Int(3)])),
            ["1) \"name\"", "2) \"Sara\"", "3) (integer) 3"]
        );
    }

    #[test]
    fn nested_lists_are_indented_under_their_number_and_long_lists_line_up() {
        // SCAN's shape: a cursor, then a list.
        let scan = Value::Array(vec![
            b("1792"),
            Value::Array(vec![b("user:1"), b("user:2")]),
            Value::Nil,
        ]);
        assert_eq!(
            printed(scan),
            [
                "1) \"1792\"",
                "2) 1) \"user:1\"",
                "   2) \"user:2\"",
                "3) (nil)"
            ]
        );
        let ten = printed(Value::Array((0..10).map(Value::Int).collect()));
        assert_eq!(ten[0], " 1) (integer) 0");
        assert_eq!(ten[9], "10) (integer) 9");
        let map = printed(Value::Map(vec![
            (b("plan"), b("team")),
            (b("tags"), Value::Array(vec![b("a"), b("b")])),
        ]));
        assert_eq!(
            map,
            [
                "1# \"plan\" => \"team\"",
                "2# \"tags\" => 1) \"a\"",
                "             2) \"b\""
            ]
        );
        let info = printed(Value::VerbatimString {
            format: redis::VerbatimFormat::Text,
            text: "# Server\r\nredis_version:7.2.4".into(),
        });
        assert_eq!(info, ["# Server", "redis_version:7.2.4"]);
    }

    #[test]
    fn a_huge_reply_stops_at_the_line_limit() {
        let out = printed(Value::Array(
            (0..(MAX_LINES as i64 + 500)).map(Value::Int).collect(),
        ));
        assert_eq!(out.len(), MAX_LINES);
    }

    #[test]
    fn syntax_comes_from_the_servers_documentation_or_from_the_arity() {
        let s = |x: &str| Value::SimpleString(x.to_string());
        let arg = |pairs: Vec<(&str, Value)>| {
            Value::Map(pairs.into_iter().map(|(k, v)| (b(k), v)).collect())
        };
        let key = arg(vec![("name", b("key")), ("type", b("key"))]);
        let pair = arg(vec![
            ("name", b("data")),
            ("type", b("block")),
            ("flags", Value::Array(vec![s("multiple")])),
            (
                "arguments",
                Value::Array(vec![
                    arg(vec![("name", b("field")), ("type", b("string"))]),
                    arg(vec![("name", b("value")), ("type", b("string"))]),
                ]),
            ),
        ]);
        assert_eq!(
            [&key, &pair].map(argument_syntax).join(" "),
            "key field value [field value ...]"
        );
        let count = arg(vec![
            ("name", b("count")),
            ("type", b("integer")),
            ("token", b("COUNT")),
            ("flags", Value::Array(vec![s("optional")])),
        ]);
        assert_eq!(argument_syntax(&count), "[COUNT count]");
        let cond = arg(vec![
            ("name", b("condition")),
            ("type", b("oneof")),
            ("flags", Value::Array(vec![s("optional")])),
            (
                "arguments",
                Value::Array(vec![
                    arg(vec![
                        ("name", b("nx")),
                        ("type", b("pure-token")),
                        ("token", b("NX")),
                    ]),
                    arg(vec![
                        ("name", b("xx")),
                        ("type", b("pure-token")),
                        ("token", b("XX")),
                    ]),
                ]),
            ),
        ]);
        assert_eq!(argument_syntax(&cond), "[NX|XX]");
        // The same in RESP2's flat lists.
        let flat = Value::Array(vec![b("name"), b("key"), b("type"), b("key")]);
        assert_eq!(argument_syntax(&flat), "key");
        let docs = Value::Map(vec![(
            b("hset"),
            Value::Map(vec![
                (b("summary"), b("Sets fields on a hash.")),
                (b("arguments"), Value::Array(vec![key, pair])),
            ]),
        )]);
        let map = docs_map(&docs);
        assert_eq!(
            map["HSET"],
            (
                "key field value [field value ...]".to_string(),
                Some("Sets fields on a hash.".to_string())
            )
        );

        assert_eq!(arity_syntax(1), "");
        assert_eq!(arity_syntax(3), "arg arg");
        assert_eq!(arity_syntax(-2), "arg [arg ...]");
        assert_eq!(arity_syntax(-1), "[arg ...]");
    }

    #[test]
    fn history_is_encrypted_deduped_filtered_and_cleared() {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        conn.execute(
            "INSERT INTO databases (id, vault_id, engine, name, route, address, created_at, updated_at) VALUES ('d1', ?1, 'redis', 'cache', 'direct', 'localhost', 0, 0)",
            params![s.vault_id],
        )
        .unwrap();
        for line in [
            "GET k",
            "GET k",
            "  TTL k  ",
            "AUTH hunter2",
            "CONFIG SET requirepass hunter2",
            "SET k \"unclosed",
            "INFO memory",
        ] {
            remember(&conn, &s, "d1", line).unwrap();
        }
        assert_eq!(
            history(&conn, &s, "d1").unwrap(),
            ["INFO memory", "TTL k", "GET k"]
        );
        // At rest nothing is readable.
        let raw: Vec<Vec<u8>> = conn
            .prepare("SELECT body_enc FROM database_history")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(raw.len(), 3);
        assert!(raw
            .iter()
            .all(|blob| !String::from_utf8_lossy(blob).contains("INFO")
                && !String::from_utf8_lossy(blob).contains("GET")));
        // Only the newest 500 stay.
        for i in 0..520 {
            remember(&conn, &s, "d1", &format!("GET k{i}")).unwrap();
        }
        let all = history(&conn, &s, "d1").unwrap();
        assert_eq!(all.len(), 500);
        assert_eq!(all[0], "GET k519");
        assert!(history(&conn, &s, "other").unwrap().is_empty());
        clear_history(&conn, &s, "d1").unwrap();
        assert!(history(&conn, &s, "d1").unwrap().is_empty());
    }

    #[test]
    fn multi_state_starts_closed() {
        assert!(ConsoleState::default().lock().unwrap().is_none());
    }
}
