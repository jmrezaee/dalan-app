//! The Server view (S31): what the server says about itself (`INFO`) and its own slow log. Read-only.
//! Dalan never turns on live command tracing (`MONITOR`).

use redis::Value;
use serde::Serialize;
use specta::Type;

use super::cluster::Conn;
use super::conn::redis_error;
use super::gate;
use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct RedisDbStats {
    pub index: u32,
    #[specta(type = f64)]
    pub keys: u64,
    #[specta(type = f64)]
    pub expires: u64,
    /// Average time left of the keys that expire, in milliseconds; 0 when none do.
    #[specta(type = f64)]
    pub avg_ttl_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct RedisServerStats {
    pub version: String,
    pub mode: String,
    pub role: String,
    #[specta(type = f64)]
    pub uptime_secs: u64,
    #[specta(type = f64)]
    pub memory_used: u64,
    /// 0: no limit is set.
    #[specta(type = f64)]
    pub memory_max: u64,
    #[specta(type = f64)]
    pub clients: u64,
    #[specta(type = f64)]
    pub blocked_clients: u64,
    #[specta(type = f64)]
    pub ops_per_sec: u64,
    #[specta(type = f64)]
    pub hits: u64,
    #[specta(type = f64)]
    pub misses: u64,
    pub databases: Vec<RedisDbStats>,
}

fn field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
}

fn number(info: &str, name: &str) -> u64 {
    field(info, name).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// `INFO` as the tiles need it. A field the server leaves out (an old version, a limited user) is zero or "unknown".
pub fn parse_info(info: &str) -> RedisServerStats {
    let databases = info
        .lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let index = name.strip_prefix("db")?.parse().ok()?;
            let part = |key: &str| {
                rest.split(',')
                    .find_map(|kv| kv.trim().strip_prefix(key)?.strip_prefix('='))
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(0)
            };
            Some(RedisDbStats {
                index,
                keys: part("keys"),
                expires: part("expires"),
                avg_ttl_ms: part("avg_ttl"),
            })
        })
        .collect();
    RedisServerStats {
        version: field(info, "redis_version")
            .unwrap_or("unknown")
            .to_string(),
        mode: field(info, "redis_mode")
            .unwrap_or("standalone")
            .to_string(),
        role: match field(info, "role") {
            Some("slave") => "replica".to_string(),
            Some("master") | None => "primary".to_string(),
            Some(other) => other.to_string(),
        },
        uptime_secs: number(info, "uptime_in_seconds"),
        memory_used: number(info, "used_memory"),
        memory_max: number(info, "maxmemory"),
        clients: number(info, "connected_clients"),
        blocked_clients: number(info, "blocked_clients"),
        ops_per_sec: number(info, "instantaneous_ops_per_sec"),
        hits: number(info, "keyspace_hits"),
        misses: number(info, "keyspace_misses"),
        databases,
    }
}

pub async fn info(conn: &mut Conn) -> AppResult<RedisServerStats> {
    let text: String = redis::cmd("INFO")
        .query_async(conn)
        .await
        .map_err(|e| redis_error(&e))?;
    let mut stats = parse_info(&text);
    if conn.is_cluster() {
        // The tiles are one node's; the keys are the whole cluster's: every primary's db0, added up.
        let (mut keys, mut expires) = (0, 0);
        for mut node in conn.scan_targets().await? {
            let keyspace: String = redis::cmd("INFO")
                .arg("keyspace")
                .query_async(&mut node)
                .await
                .unwrap_or_default();
            if let Some(db0) = parse_info(&keyspace).databases.first() {
                keys += db0.keys;
                expires += db0.expires;
            }
        }
        let avg_ttl_ms = stats.databases.first().map_or(0, |d| d.avg_ttl_ms);
        stats.databases = vec![RedisDbStats {
            index: 0,
            keys,
            expires,
            avg_ttl_ms,
        }];
    }
    Ok(stats)
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct RedisSlowEntry {
    #[specta(type = f64)]
    pub id: u64,
    /// When it ran, in seconds since 1970 (the server's clock).
    #[specta(type = f64)]
    pub at_secs: u64,
    #[specta(type = f64)]
    pub micros: u64,
    /// The command as text, long arguments shortened.
    pub command: String,
    /// The client's address, when the server records it (Redis 4 and newer).
    pub client: String,
}

/// `SLOWLOG GET`: `[[id, time, micros, [args...], client-addr, client-name], ...]`.
pub fn parse_slowlog(v: Value) -> Vec<RedisSlowEntry> {
    let Value::Array(entries) = v else {
        return Vec::new();
    };
    let int = |v: Option<&Value>| match v {
        Some(Value::Int(i)) => (*i).max(0) as u64,
        _ => 0,
    };
    entries
        .iter()
        .filter_map(|e| {
            let Value::Array(parts) = e else {
                return None;
            };
            let args: Vec<Vec<u8>> = match parts.get(3) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|x| match x {
                        Value::BulkString(b) => Some(b.clone()),
                        Value::SimpleString(s) => Some(s.clone().into_bytes()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let client = match parts.get(4) {
                Some(Value::BulkString(b)) => String::from_utf8_lossy(b).into_owned(),
                Some(Value::SimpleString(s)) => s.clone(),
                _ => String::new(),
            };
            Some(RedisSlowEntry {
                id: int(parts.first()),
                at_secs: int(parts.get(1)),
                micros: int(parts.get(2)),
                command: gate::preview(&args),
                // "10.0.1.14:51234": the port says nothing useful.
                client: client
                    .rsplit_once(':')
                    .map(|(ip, _)| ip.to_string())
                    .unwrap_or(client),
            })
        })
        .collect()
}

/// The newest 50 slow commands. A user who may not read the log gets an empty list.
pub async fn slowlog(conn: &mut Conn) -> AppResult<Vec<RedisSlowEntry>> {
    let reply = redis::cmd("SLOWLOG")
        .arg("GET")
        .arg(50)
        .query_async::<Value>(conn)
        .await;
    match reply {
        Ok(v) => Ok(parse_slowlog(v)),
        Err(e) if e.is_io_error() || e.is_connection_dropped() || e.is_timeout() => {
            Err(redis_error(&e))
        }
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_becomes_the_tiles_numbers() {
        let info = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\nuptime_in_seconds:3542400\r\n# Clients\r\nconnected_clients:212\r\nblocked_clients:0\r\n# Memory\r\nused_memory:1932735283\r\nmaxmemory:4294967296\r\n# Stats\r\ninstantaneous_ops_per_sec:8420\r\nkeyspace_hits:962\r\nkeyspace_misses:38\r\n# Replication\r\nrole:slave\r\n# Keyspace\r\ndb0:keys=12408,expires=9960,avg_ttl=18720000\r\ndb1:keys=318,expires=0,avg_ttl=0\r\ndb3:keys=42,expires=42,avg_ttl=840000,subexpiry=0\r\n";
        let s = parse_info(info);
        assert_eq!(
            (s.version.as_str(), s.mode.as_str(), s.role.as_str()),
            ("7.2.4", "standalone", "replica")
        );
        assert_eq!(
            (s.uptime_secs, s.clients, s.blocked_clients, s.ops_per_sec),
            (3_542_400, 212, 0, 8420)
        );
        assert_eq!(
            (s.memory_used, s.memory_max, s.hits, s.misses),
            (1_932_735_283, 4_294_967_296, 962, 38)
        );
        assert_eq!(s.databases.len(), 3);
        assert_eq!(
            s.databases[0],
            RedisDbStats {
                index: 0,
                keys: 12408,
                expires: 9960,
                avg_ttl_ms: 18_720_000
            }
        );
        assert_eq!(s.databases[2].index, 3);
        // Nothing readable: zeros, not a failure.
        let empty = parse_info("");
        assert_eq!(
            (empty.version.as_str(), empty.role.as_str(), empty.clients),
            ("unknown", "primary", 0)
        );
        assert!(empty.databases.is_empty());
    }

    #[test]
    fn the_slow_log_is_read_newest_first_with_the_command_as_text() {
        let b = |s: &str| Value::BulkString(s.as_bytes().to_vec());
        let v = Value::Array(vec![
            Value::Array(vec![
                Value::Int(14),
                Value::Int(1_794_770_531),
                Value::Int(184_000),
                Value::Array(vec![
                    b("ZRANGEBYSCORE"),
                    b("leaderboard:weekly"),
                    b("-inf"),
                    b("+inf"),
                ]),
                b("10.0.1.14:51234"),
                b("api"),
            ]),
            // Redis before 4.0: four elements.
            Value::Array(vec![
                Value::Int(13),
                Value::Int(1_794_770_320),
                Value::Int(96_000),
                Value::Array(vec![b("SMEMBERS"), b("audit ids")]),
            ]),
            Value::Nil,
        ]);
        let log = parse_slowlog(v);
        assert_eq!(log.len(), 2);
        assert_eq!(
            log[0],
            RedisSlowEntry {
                id: 14,
                at_secs: 1_794_770_531,
                micros: 184_000,
                command: "ZRANGEBYSCORE leaderboard:weekly -inf +inf".into(),
                client: "10.0.1.14".into()
            }
        );
        assert_eq!(log[1].command, "SMEMBERS \"audit ids\"");
        assert_eq!(log[1].client, "");
        assert!(parse_slowlog(Value::Nil).is_empty());
    }
}
