//! The write gate (replica/redis/spec.md section 4). **One** function decides for every command, wherever it comes
//! from: an editor, the console, a publish, a bulk action. The UI never decides and cannot skip it.
//!
//! Two steps:
//!   1. `classify`: what kind of command is this? read, write, dangerous, heavy or refused.
//!   2. `decide`: what does this database's mode say about that kind? run, confirm, typed confirm, or refuse.
//!
//! "Read-only" here is Dalan's own promise: the command is refused before anything leaves this computer. Dalan
//! does not change a server setting to enforce it.

use std::collections::HashMap;

use serde::Serialize;
use specta::Type;

use super::store::RedisWriteMode;
use crate::error::ErrorCode;

/// Reading a collection whole is "heavy" once it holds more than this many items; deleting such a key is dangerous.
pub const BIG: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    // The order is the strength: `EXEC` takes the strongest class of what was queued.
    Read,
    Write,
    Heavy,
    Dangerous,
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Run,
    /// Show what it will do and ask.
    Confirm,
    /// Ask, and make the person type the database's name.
    Typed,
    Refuse(ErrorCode),
}

/// What the server said about one command in `COMMAND`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub write: bool,
    pub readonly: bool,
    /// How many arguments it takes, counting its own name; negative means "at least".
    pub arity: i64,
}

/// The server's command table, read once after sign-in. Keys are upper-case: `SET`, and `CONFIG|GET` for a
/// subcommand (newer servers list those separately).
#[derive(Debug, Clone, Default)]
pub struct CommandTable(HashMap<String, Flags>);

impl CommandTable {
    /// From the reply of `COMMAND`: one array per command, `[name, arity, [flags...], first, last, step, ...]`,
    /// where newer servers add the subcommands (same shape) as the tenth element.
    pub fn from_reply(reply: &redis::Value) -> Self {
        let mut table = HashMap::new();
        if let redis::Value::Array(commands) = reply {
            for c in commands {
                Self::add(&mut table, c, true);
            }
        }
        Self(table)
    }

    fn add(table: &mut HashMap<String, Flags>, entry: &redis::Value, top: bool) {
        let redis::Value::Array(parts) = entry else {
            return;
        };
        let text = |v: &redis::Value| match v {
            redis::Value::BulkString(b) => String::from_utf8(b.clone()).ok(),
            redis::Value::SimpleString(s) => Some(s.clone()),
            _ => None,
        };
        let Some(name) = parts.first().and_then(text) else {
            return;
        };
        let mut flags = Flags::default();
        if let Some(redis::Value::Int(n)) = parts.get(1) {
            flags.arity = *n;
        }
        if let Some(redis::Value::Array(list) | redis::Value::Set(list)) = parts.get(2) {
            for f in list.iter().filter_map(text) {
                match f.to_ascii_lowercase().as_str() {
                    "write" => flags.write = true,
                    "readonly" => flags.readonly = true,
                    _ => {}
                }
            }
        }
        table.insert(name.to_ascii_uppercase(), flags);
        if top {
            if let Some(redis::Value::Array(subs)) = parts.get(9) {
                for s in subs {
                    Self::add(table, s, false);
                }
            }
        }
    }

    #[cfg(test)]
    pub fn of(entries: &[(&str, bool, bool)]) -> Self {
        Self(
            entries
                .iter()
                .map(|(n, write, readonly)| {
                    (
                        n.to_string(),
                        Flags {
                            write: *write,
                            readonly: *readonly,
                            arity: 0,
                        },
                    )
                })
                .collect(),
        )
    }

    fn get(&self, name: &str, sub: Option<&str>) -> Option<Flags> {
        sub.and_then(|s| self.0.get(&format!("{name}|{s}")))
            .or_else(|| self.0.get(name))
            .copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every top-level command with its flags, by name (subcommands, `CONFIG|GET`, are left out).
    pub fn commands(&self) -> Vec<(&str, Flags)> {
        let mut all: Vec<(&str, Flags)> = self
            .0
            .iter()
            .filter(|(n, _)| !n.contains('|'))
            .map(|(n, f)| (n.as_str(), *f))
            .collect();
        all.sort_by(|a, b| a.0.cmp(b.0));
        all
    }

    /// Whether the server knows this command at all (the JSON module's `JSON.GET`, for example).
    pub fn has(&self, name: &str) -> bool {
        self.0.contains_key(&name.to_ascii_uppercase())
    }
}

fn upper(arg: Option<&Vec<u8>>) -> String {
    arg.map(|a| String::from_utf8_lossy(a).trim().to_ascii_uppercase())
        .unwrap_or_default()
}

/// `CLUSTER` subcommands that change the cluster. The rest (`INFO`, `NODES`, `SLOTS`, `SHARDS`, ...) only read.
const CLUSTER_CHANGES: &[&str] = &[
    "ADDSLOTS",
    "ADDSLOTSRANGE",
    "DELSLOTS",
    "DELSLOTSRANGE",
    "FLUSHSLOTS",
    "FORGET",
    "MEET",
    "RESET",
    "FAILOVER",
    "REPLICATE",
    "SETSLOT",
    "SET-CONFIG-EPOCH",
    "BUMPEPOCH",
    "SAVECONFIG",
];

/// A read of a whole collection: the command that tells its size, so the gate can call it "heavy" when it is big.
/// `LRANGE k 0 -1` and `ZRANGE k 0 -1` count; a bounded range does not.
pub fn unbounded_read(args: &[Vec<u8>]) -> Option<&'static str> {
    let whole =
        || args.get(2).is_some_and(|a| a == b"0") && args.get(3).is_some_and(|a| a == b"-1");
    match upper(args.first()).as_str() {
        "SMEMBERS" => Some("SCARD"),
        "HGETALL" | "HKEYS" | "HVALS" => Some("HLEN"),
        "LRANGE" if whole() => Some("LLEN"),
        "ZRANGE" | "ZREVRANGE" if whole() => Some("ZCARD"),
        _ => None,
    }
}

/// What kind of command this is. `length` is the size of the collection an unbounded read would return (see
/// `unbounded_read`), when the caller looked it up.
pub fn classify(table: &CommandTable, args: &[Vec<u8>], length: Option<u64>) -> Class {
    let name = upper(args.first());
    let sub = upper(args.get(1));
    let sub = (!sub.is_empty()).then_some(sub.as_str());
    if name.is_empty() {
        return Class::Refused;
    }
    match (name.as_str(), sub) {
        // Always refused: they can hurt a busy server, or they take over the connection.
        ("MONITOR" | "SYNC" | "PSYNC", _)
        | (
            "SUBSCRIBE" | "PSUBSCRIBE" | "SSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE"
            | "SUNSUBSCRIBE",
            _,
        )
        | ("CLIENT", Some("PAUSE")) => return Class::Refused,

        (
            "FLUSHALL" | "FLUSHDB" | "SHUTDOWN" | "DEBUG" | "SWAPDB" | "REPLICAOF" | "SLAVEOF"
            | "FAILOVER",
            _,
        )
        | ("MIGRATE" | "SAVE" | "BGSAVE" | "BGREWRITEAOF", _)
        | ("CONFIG", Some("SET" | "REWRITE" | "RESETSTAT"))
        | ("SCRIPT", Some("FLUSH" | "KILL"))
        | ("FUNCTION", Some("FLUSH" | "DELETE" | "RESTORE"))
        | ("ACL", Some("SETUSER" | "DELUSER" | "LOAD" | "SAVE"))
        | ("MODULE", Some("LOAD" | "LOADEX" | "UNLOAD"))
        | ("CLIENT", Some("KILL"))
        | ("SLOWLOG" | "LATENCY", Some("RESET")) => return Class::Dangerous,
        ("CLUSTER", Some(s)) if CLUSTER_CHANGES.contains(&s) => return Class::Dangerous,
        ("RESTORE" | "RESTORE-ASKING", _)
            if args
                .iter()
                .skip(4)
                .any(|a| a.eq_ignore_ascii_case(b"REPLACE")) =>
        {
            return Class::Dangerous
        }

        ("KEYS", _) => return Class::Heavy,

        // Scripts can do anything; their read-only forms promise not to write.
        ("EVAL" | "EVALSHA" | "FCALL", _) => return Class::Write,
        ("EVAL_RO" | "EVALSHA_RO" | "FCALL_RO", _) => return Class::Read,
        ("PUBLISH" | "SPUBLISH", _) => return Class::Write,
        _ => {}
    }
    if unbounded_read(args).is_some() && length.is_some_and(|n| n > BIG) {
        return Class::Heavy;
    }
    match table.get(&name, sub) {
        Some(f) if f.write => Class::Write,
        Some(_) => Class::Read,
        // Unknown to the server's table (a module command, or the table could not be read): assume it writes.
        None => Class::Write,
    }
}

/// `EXEC` is as strong as the strongest command that was queued.
pub fn strongest(queued: &[Class]) -> Class {
    queued.iter().copied().max().unwrap_or(Class::Read)
}

/// The table of spec section 4.
pub fn decide(mode: RedisWriteMode, class: Class) -> Decision {
    use Class::*;
    use RedisWriteMode::*;
    match (class, mode) {
        (Read, _) => Decision::Run,
        (Write, ReadOnly) => Decision::Refuse(ErrorCode::RedisReadOnlyMode),
        (Write, Confirm) => Decision::Confirm,
        (Write, Open) => Decision::Run,
        (Heavy, _) => Decision::Confirm,
        (Dangerous, ReadOnly) => Decision::Refuse(ErrorCode::RedisReadOnlyMode),
        (Dangerous, Confirm | Open) => Decision::Typed,
        (Refused, _) => Decision::Refuse(ErrorCode::RedisRefused),
    }
}

/// From most to least protective. Loosening a production database's mode needs the master password.
pub fn protection(mode: RedisWriteMode) -> u8 {
    match mode {
        RedisWriteMode::ReadOnly => 2,
        RedisWriteMode::Confirm => 1,
        RedisWriteMode::Open => 0,
    }
}

/// The command as text for a confirmation, with long arguments shortened. Arguments are bytes: readable ones are
/// shown as they are (quoted when they hold a space or are empty), the rest with `\xNN` escapes.
pub fn preview(args: &[Vec<u8>]) -> String {
    const MAX_ARG: usize = 80;
    const MAX_ARGS: usize = 12;
    let mut parts: Vec<String> = args
        .iter()
        .take(MAX_ARGS)
        .enumerate()
        .map(|(i, a)| {
            let readable = std::str::from_utf8(a)
                .ok()
                .filter(|s| !s.chars().any(char::is_control));
            let mut shown = match readable {
                Some(s) => s.to_string(),
                None => a
                    .iter()
                    .map(|b| {
                        if b.is_ascii_graphic() && *b != b'\\' {
                            (*b as char).to_string()
                        } else {
                            format!("\\x{b:02x}")
                        }
                    })
                    .collect(),
            };
            let long = shown.chars().count() > MAX_ARG;
            if long {
                shown = shown.chars().take(MAX_ARG).collect::<String>() + "…";
            }
            // The command's own name is never quoted.
            if i > 0 && (shown.is_empty() || shown.contains(' ') || shown.contains('"')) {
                format!("\"{}\"", shown.replace('"', "\\\""))
            } else {
                shown
            }
        })
        .collect();
    if args.len() > MAX_ARGS {
        parts.push(format!("… {} more", args.len() - MAX_ARGS));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table like a real server's: (name, write, readonly). Anything not listed is "unknown".
    fn table() -> CommandTable {
        CommandTable::of(&[
            ("GET", false, true),
            ("HGET", false, true),
            ("HGETALL", false, true),
            ("SMEMBERS", false, true),
            ("LRANGE", false, true),
            ("ZRANGE", false, true),
            ("SCAN", false, true),
            ("TTL", false, true),
            ("DBSIZE", false, true),
            ("SET", true, false),
            ("DEL", true, false),
            ("UNLINK", true, false),
            ("HSET", true, false),
            ("EXPIRE", true, false),
            ("RENAMENX", true, false),
            ("XADD", true, false),
            ("RESTORE", true, false),
            // Known, with neither flag: connection and admin commands that change no data.
            ("PING", false, false),
            ("INFO", false, false),
            ("SELECT", false, false),
            ("CONFIG", false, false),
            ("CONFIG|GET", false, false),
            ("CLIENT", false, false),
            ("CLUSTER", false, false),
            ("SLOWLOG", false, false),
            ("SCRIPT", false, false),
            ("ACL", false, false),
            ("MULTI", false, false),
            ("EXEC", false, false),
        ])
    }

    fn args(line: &str) -> Vec<Vec<u8>> {
        line.split_whitespace()
            .map(|a| a.as_bytes().to_vec())
            .collect()
    }
    fn class(line: &str) -> Class {
        classify(&table(), &args(line), None)
    }

    #[test]
    fn reads_are_what_the_server_flags_read_only_or_does_not_flag_as_write() {
        for line in [
            "GET k",
            "HGET k f",
            "SCAN 0 MATCH user:* COUNT 500",
            "TTL k",
            "DBSIZE",
            "PING",
            "INFO server",
            "SELECT 3",
            "CONFIG GET maxmemory",
            "CLIENT LIST",
            "CLUSTER NODES",
            "CLUSTER INFO",
            "SLOWLOG GET 10",
            "ACL WHOAMI",
            "SCRIPT EXISTS abc",
        ] {
            assert_eq!(class(line), Class::Read, "{line}");
        }
    }

    #[test]
    fn writes_are_flagged_write_or_unknown_or_scripts_or_publish() {
        for line in [
            "SET k v",
            "DEL k",
            "UNLINK k",
            "HSET k f v",
            "EXPIRE k 60",
            "RENAMENX a b",
            "XADD s * f v",
            "RESTORE k 0 payload",
        ] {
            assert_eq!(class(line), Class::Write, "{line}");
        }
        // Unknown to the table means write: a module command, a typo, a command newer than the server.
        for line in [
            "JSON.SET k $ {}",
            "FT.CREATE idx",
            "NOSUCHCOMMAND",
            "TS.ADD k * 1",
        ] {
            assert_eq!(class(line), Class::Write, "{line}");
        }
        for line in [
            "EVAL return 0",
            "EVALSHA abc 0",
            "FCALL f 0",
            "PUBLISH news hi",
            "SPUBLISH news hi",
        ] {
            assert_eq!(class(line), Class::Write, "{line}");
        }
        for line in ["EVAL_RO return 0", "EVALSHA_RO abc 0", "FCALL_RO f 0"] {
            assert_eq!(class(line), Class::Read, "{line}");
        }
    }

    #[test]
    fn dangerous_commands_are_dangerous_whatever_the_table_says() {
        for line in [
            "FLUSHALL",
            "FLUSHALL ASYNC",
            "FLUSHDB",
            "FLUSHDB ASYNC",
            "SHUTDOWN",
            "SHUTDOWN NOSAVE",
            "CONFIG SET maxmemory 1",
            "CONFIG REWRITE",
            "CONFIG RESETSTAT",
            "DEBUG SLEEP 10",
            "DEBUG SEGFAULT",
            "DEBUG OBJECT k",
            "SWAPDB 0 1",
            "REPLICAOF NO ONE",
            "SLAVEOF host 6379",
            "FAILOVER",
            "CLUSTER ADDSLOTS 1",
            "CLUSTER DELSLOTS 1",
            "CLUSTER FORGET id",
            "CLUSTER MEET h 1",
            "CLUSTER RESET",
            "CLUSTER FAILOVER",
            "CLUSTER REPLICATE id",
            "CLUSTER SETSLOT 1 NODE id",
            "SCRIPT FLUSH",
            "SCRIPT KILL",
            "FUNCTION FLUSH",
            "FUNCTION DELETE lib",
            "FUNCTION RESTORE payload",
            "ACL SETUSER u on",
            "ACL DELUSER u",
            "ACL LOAD",
            "ACL SAVE",
            "MODULE LOAD /x.so",
            "MODULE UNLOAD m",
            "CLIENT KILL ID 3",
            "MIGRATE h 6379 k 0 1000",
            "RESTORE k 0 payload REPLACE",
            "SAVE",
            "BGSAVE",
            "BGREWRITEAOF",
            "SLOWLOG RESET",
            "LATENCY RESET",
        ] {
            assert_eq!(class(line), Class::Dangerous, "{line}");
        }
        // An empty table (the server would not show it) does not weaken this.
        assert_eq!(
            classify(&CommandTable::default(), &args("FLUSHALL"), None),
            Class::Dangerous
        );
    }

    #[test]
    fn refused_commands_and_an_empty_line() {
        for line in [
            "MONITOR",
            "SUBSCRIBE news",
            "PSUBSCRIBE n*",
            "SSUBSCRIBE s",
            "UNSUBSCRIBE",
            "SYNC",
            "PSYNC ? -1",
            "CLIENT PAUSE 1000",
        ] {
            assert_eq!(class(line), Class::Refused, "{line}");
        }
        assert_eq!(classify(&table(), &[], None), Class::Refused);
        assert_eq!(classify(&table(), &[b"  ".to_vec()], None), Class::Refused);
    }

    #[test]
    fn case_and_surrounding_spaces_do_not_matter() {
        for line in [
            "flushall",
            "FlushAll",
            "config set x 1",
            "Config SET x 1",
            "client pause 5",
            "monitor",
        ] {
            assert_ne!(class(line), Class::Read, "{line}");
            assert_ne!(class(line), Class::Write, "{line}");
        }
        assert_eq!(
            classify(&table(), &[b" get ".to_vec(), b"k".to_vec()], None),
            Class::Read
        );
        assert_eq!(
            classify(
                &table(),
                &[b"Set".to_vec(), b"k".to_vec(), b"v".to_vec()],
                None
            ),
            Class::Write
        );
        assert_eq!(
            classify(
                &table(),
                &[
                    b"restore".to_vec(),
                    b"k".to_vec(),
                    b"0".to_vec(),
                    b"p".to_vec(),
                    b"replace".to_vec()
                ],
                None
            ),
            Class::Dangerous
        );
    }

    #[test]
    fn heavy_is_keys_and_whole_reads_of_big_collections() {
        assert_eq!(class("KEYS *"), Class::Heavy);
        assert_eq!(class("keys user:*"), Class::Heavy);
        let big = Some(BIG + 1);
        let small = Some(BIG);
        for line in ["SMEMBERS s", "HGETALL h", "LRANGE l 0 -1", "ZRANGE z 0 -1"] {
            assert!(unbounded_read(&args(line)).is_some(), "{line}");
            assert_eq!(classify(&table(), &args(line), big), Class::Heavy, "{line}");
            assert_eq!(
                classify(&table(), &args(line), small),
                Class::Read,
                "{line}"
            );
            assert_eq!(classify(&table(), &args(line), None), Class::Read, "{line}");
        }
        // A bounded range is an ordinary read, however big the list is.
        for line in ["LRANGE l 0 99", "ZRANGE z 0 499", "LRANGE l 10 -1"] {
            assert_eq!(unbounded_read(&args(line)), None, "{line}");
            assert_eq!(classify(&table(), &args(line), big), Class::Read, "{line}");
        }
        assert_eq!(unbounded_read(&args("SMEMBERS s")), Some("SCARD"));
        assert_eq!(unbounded_read(&args("hgetall h")), Some("HLEN"));
    }

    #[test]
    fn the_decision_table_of_the_spec() {
        use Class::*;
        use Decision::*;
        use RedisWriteMode::*;
        let ro = Refuse(ErrorCode::RedisReadOnlyMode);
        let no = Refuse(ErrorCode::RedisRefused);
        let rows: [(Class, [Decision; 3]); 5] = [
            //            Read-only  Confirm   Open
            (Read, [Run, Run, Run]),
            (Write, [ro, Decision::Confirm, Run]),
            (
                Heavy,
                [Decision::Confirm, Decision::Confirm, Decision::Confirm],
            ),
            (Dangerous, [ro, Typed, Typed]),
            (Refused, [no, no, no]),
        ];
        for (class, want) in rows {
            for (mode, expected) in [ReadOnly, RedisWriteMode::Confirm, Open]
                .into_iter()
                .zip(want)
            {
                assert_eq!(decide(mode, class), expected, "{class:?} in {mode:?}");
            }
        }
    }

    #[test]
    fn exec_takes_the_strongest_class_of_its_queue() {
        use Class::*;
        assert_eq!(strongest(&[]), Read);
        assert_eq!(strongest(&[Read, Read]), Read);
        assert_eq!(strongest(&[Read, Write, Read]), Write);
        assert_eq!(strongest(&[Write, Dangerous, Read]), Dangerous);
        assert_eq!(strongest(&[Heavy, Write]), Heavy);
        assert_eq!(strongest(&[Dangerous, Refused]), Refused);
    }

    #[test]
    fn protection_orders_the_modes() {
        assert!(protection(RedisWriteMode::ReadOnly) > protection(RedisWriteMode::Confirm));
        assert!(protection(RedisWriteMode::Confirm) > protection(RedisWriteMode::Open));
    }

    #[test]
    fn the_command_table_reads_flags_and_subcommands_from_the_servers_reply() {
        use redis::Value::*;
        let b = |s: &str| BulkString(s.as_bytes().to_vec());
        let entry = |name: &str, flags: &[&str], subs: Vec<redis::Value>| {
            Array(vec![
                b(name),
                Int(-2),
                Array(flags.iter().map(|f| SimpleString(f.to_string())).collect()),
                Int(1),
                Int(1),
                Int(1),
                Array(vec![]),
                Array(vec![]),
                Array(vec![]),
                Array(subs),
            ])
        };
        let reply = Array(vec![
            entry("get", &["readonly", "fast"], vec![]),
            entry("set", &["write", "denyoom"], vec![]),
            entry(
                "config",
                &[],
                vec![
                    entry("config|get", &["admin", "noscript"], vec![]),
                    entry("config|set", &["admin"], vec![]),
                ],
            ),
            // An older server: six elements, no subcommands.
            Array(vec![
                b("ping"),
                Int(-1),
                Array(vec![b("fast")]),
                Int(0),
                Int(0),
                Int(0),
            ]),
        ]);
        let t = CommandTable::from_reply(&reply);
        assert_eq!(
            t.get("GET", None),
            Some(Flags {
                write: false,
                readonly: true,
                arity: -2
            })
        );
        assert_eq!(
            t.get("SET", None),
            Some(Flags {
                write: true,
                readonly: false,
                arity: -2
            })
        );
        assert_eq!(t.get("CONFIG", Some("GET")).map(|f| f.arity), Some(-2));
        assert_eq!(
            t.get("PING", None).map(|f| (f.write, f.arity)),
            Some((false, -1))
        );
        assert_eq!(
            t.commands().iter().map(|c| c.0).collect::<Vec<_>>(),
            ["CONFIG", "GET", "PING", "SET"]
        );
        assert_eq!(t.get("NOPE", None), None);
        assert!(t.has("get") && !t.has("json.get"));
        assert!(CommandTable::from_reply(&Nil).is_empty());
    }

    #[test]
    fn the_preview_shows_the_exact_command_with_long_and_binary_arguments_made_readable() {
        let a = |parts: &[&[u8]]| parts.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
        assert_eq!(
            preview(&a(&[b"HSET", b"user:1042:profile", b"plan", b"business"])),
            "HSET user:1042:profile plan business"
        );
        assert_eq!(
            preview(&a(&[b"SET", b"maintenance", b"Back at 14:30 UTC."])),
            "SET maintenance \"Back at 14:30 UTC.\""
        );
        assert_eq!(preview(&a(&[b"SET", b"k", b""])), "SET k \"\"");
        assert_eq!(
            preview(&a(&[b"SET", b"k", b"say \"hi\""])),
            "SET k \"say \\\"hi\\\"\""
        );
        assert_eq!(
            preview(&a(&[b"SET", &[0xff, 0x00, b'A'], b"v"])),
            "SET \\xff\\x00A v"
        );
        let long = "x".repeat(200);
        let shown = preview(&a(&[b"SET", b"k", long.as_bytes()]));
        assert_eq!(shown, format!("SET k {}…", "x".repeat(80)));
        let many: Vec<Vec<u8>> = std::iter::once(b"SADD".to_vec())
            .chain((0..20).map(|i| i.to_string().into_bytes()))
            .collect();
        assert!(preview(&many).ends_with("… 9 more"), "{}", preview(&many));
        assert_eq!(preview(&a(&[b"FLUSHDB", b"ASYNC"])), "FLUSHDB ASYNC");
    }
}
