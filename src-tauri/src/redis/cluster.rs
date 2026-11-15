//! Redis Cluster (F19; replica/redis/spec.md "Cluster and Sentinel").
//!
//! The library's own cluster client opens its own sockets, which could not go through our SSH route. So the
//! routing is here: a slot map read from the cluster, one connection per node opened through the same route as the
//! tab, keys hashed to slots (CRC16, honouring `{hash tags}`), and `MOVED` / `ASK` followed at most five times.
//!
//! `Conn` is what the rest of the Redis code talks to. It is either one server or a cluster, and it implements the
//! `redis` crate's `ConnectionLike`, so `redis::cmd(...).query_async(&mut conn)` works for both.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::{
    Arg, Cmd, ErrorKind, Pipeline, RedisError, RedisFuture, RedisResult, ServerErrorKind, Value,
};
use serde::Serialize;
use specta::Type;

use super::conn::Opener;
use super::gate::CommandTable;
use crate::error::{AppError, AppResult, ErrorCode};

pub const SLOTS: u16 = 16384;
/// How many `MOVED` / `ASK` answers one command follows before giving up.
pub const MAX_REDIRECTS: usize = 5;

/// CRC16-CCITT (XModem), the checksum Redis Cluster hashes keys with. Bit by bit: short, and fast enough here.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The slot a key lives in. If the key has `{...}` with something inside, only that part is hashed, so
/// `{user1000}.following` and `{user1000}.followers` share a slot.
pub fn slot(key: &[u8]) -> u16 {
    let hashed = match key.iter().position(|&b| b == b'{') {
        Some(open) => match key[open + 1..].iter().position(|&b| b == b'}') {
            // `{}` (nothing inside) does not count: the whole key is hashed.
            Some(len) if len > 0 => &key[open + 1..open + 1 + len],
            _ => key,
        },
        None => key,
    };
    crc16(hashed) % SLOTS
}

/// One node as the Server view lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct RedisNode {
    /// `primary`, `replica` or `sentinel`.
    pub role: String,
    /// `10.0.2.11:7000`
    pub address: String,
    /// The slots it serves (`0–5460`), or the primary it follows (`of 10.0.2.11:7000`).
    pub detail: String,
    /// `ok`, `catching up`, `failed`, or the server's own word.
    pub state: String,
}

/// A primary with its slots and replicas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shard {
    pub slots: Vec<(u16, u16)>,
    pub primary: String,
    /// (address, health)
    pub replicas: Vec<(String, String)>,
    pub health: String,
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(b) => String::from_utf8(b.clone()).ok(),
        Value::SimpleString(s) => Some(s.clone()),
        Value::VerbatimString { text, .. } => Some(text.clone()),
        _ => None,
    }
}
fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        other => text(other)?.parse().ok(),
    }
}
/// A RESP2 flat list or a RESP3 map, as (name, value) pairs.
fn fields(v: &Value) -> Vec<(String, &Value)> {
    match v {
        Value::Map(pairs) => pairs
            .iter()
            .filter_map(|(k, val)| Some((text(k)?, val)))
            .collect(),
        Value::Array(items) => items
            .chunks(2)
            .filter_map(|c| Some((text(c.first()?)?, c.get(1)?)))
            .collect(),
        _ => Vec::new(),
    }
}
/// A node that reports no address of its own means "the one you asked".
fn address(host: &str, port: i64, seed_host: &str) -> String {
    let host = if host.is_empty() || host == "?" {
        seed_host
    } else {
        host
    };
    join(host, port as u16)
}
pub fn join(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
/// `10.0.2.11:7000` or `[::1]:7000` as (host, port).
pub fn split(addr: &str) -> Option<(String, u16)> {
    let (host, port) = addr.rsplit_once(':')?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some((host.to_string(), port.parse().ok()?))
}

/// `CLUSTER SHARDS` (Redis 7): one map per shard with `slots` (start, end, start, end, ...) and `nodes`.
pub fn parse_shards(v: &Value, seed_host: &str, tls: bool) -> Vec<Shard> {
    let Value::Array(shards) = v else {
        return Vec::new();
    };
    shards
        .iter()
        .filter_map(|shard| {
            let mut slots = Vec::new();
            let mut primary = None;
            let mut replicas = Vec::new();
            let mut health = String::new();
            for (name, val) in fields(shard) {
                match (name.as_str(), val) {
                    ("slots", Value::Array(s)) => {
                        slots = s
                            .chunks(2)
                            .filter_map(|p| Some((int(p.first()?)? as u16, int(p.get(1)?)? as u16)))
                            .collect();
                    }
                    ("nodes", Value::Array(nodes)) => {
                        for node in nodes {
                            let (mut host, mut port, mut tls_port, mut role, mut state) =
                                (String::new(), 0, 0, String::new(), String::new());
                            for (k, x) in fields(node) {
                                match k.as_str() {
                                    // `endpoint` is what clients should use; `ip` is the fallback.
                                    "endpoint" => {
                                        host = text(x)
                                            .filter(|e| !e.is_empty() && e != "?")
                                            .unwrap_or(host)
                                    }
                                    "ip" if host.is_empty() => host = text(x).unwrap_or_default(),
                                    "port" => port = int(x).unwrap_or(0),
                                    "tls-port" => tls_port = int(x).unwrap_or(0),
                                    "role" => role = text(x).unwrap_or_default(),
                                    "health" => state = text(x).unwrap_or_default(),
                                    _ => {}
                                }
                            }
                            let addr = address(
                                &host,
                                if tls && tls_port > 0 { tls_port } else { port },
                                seed_host,
                            );
                            if role == "master" {
                                primary = Some(addr);
                                health = state;
                            } else {
                                replicas.push((addr, state));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(Shard {
                slots,
                primary: primary?,
                replicas,
                health,
            })
        })
        .collect()
}

/// `CLUSTER SLOTS` (every version): `[start, end, [ip, port, id, ...], [replica ip, port, ...]...]` per range.
/// Ranges of the same primary are joined into one shard.
pub fn parse_slots(v: &Value, seed_host: &str) -> Vec<Shard> {
    let Value::Array(ranges) = v else {
        return Vec::new();
    };
    let node = |n: &Value| -> Option<String> {
        let Value::Array(parts) = n else {
            return None;
        };
        Some(address(
            &text(parts.first()?).unwrap_or_default(),
            int(parts.get(1)?)?,
            seed_host,
        ))
    };
    let mut shards: Vec<Shard> = Vec::new();
    for range in ranges {
        let Value::Array(parts) = range else {
            continue;
        };
        let (Some(start), Some(end), Some(primary)) = (
            parts.first().and_then(int),
            parts.get(1).and_then(int),
            parts.get(2).and_then(node),
        ) else {
            continue;
        };
        let replicas: Vec<(String, String)> = parts
            .iter()
            .skip(3)
            .filter_map(node)
            .map(|a| (a, String::new()))
            .collect();
        match shards.iter_mut().find(|s| s.primary == primary) {
            Some(s) => s.slots.push((start as u16, end as u16)),
            None => shards.push(Shard {
                slots: vec![(start as u16, end as u16)],
                primary,
                replicas,
                health: String::new(),
            }),
        }
    }
    shards
}

/// The Server view's rows: each primary, then its replicas.
pub fn nodes_of(shards: &[Shard]) -> Vec<RedisNode> {
    let state = |health: &str| match health {
        "" | "online" => "ok".to_string(),
        "loading" => "catching up".to_string(),
        other => other.to_string(),
    };
    let mut sorted: Vec<&Shard> = shards.iter().collect();
    sorted.sort_by_key(|s| s.slots.first().copied().unwrap_or((u16::MAX, 0)));
    let mut out = Vec::new();
    for s in sorted {
        let slots = s
            .slots
            .iter()
            .map(|(a, b)| {
                if a == b {
                    a.to_string()
                } else {
                    format!("{a}–{b}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push(RedisNode {
            role: "primary".into(),
            address: s.primary.clone(),
            detail: if slots.is_empty() {
                "no slots".into()
            } else {
                slots
            },
            state: state(&s.health),
        });
        for (addr, health) in &s.replicas {
            out.push(RedisNode {
                role: "replica".into(),
                address: addr.clone(),
                detail: format!("of {}", s.primary),
                state: state(health),
            });
        }
    }
    out
}

/// Which primary serves which slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotMap {
    /// (first slot, last slot, primary's address), sorted by first slot.
    ranges: Vec<(u16, u16, String)>,
    /// Every primary, in a fixed order (so "node 2" of a scan means the same node on the next call).
    primaries: Vec<String>,
}

impl SlotMap {
    pub fn from_shards(shards: &[Shard]) -> Self {
        let mut ranges: Vec<(u16, u16, String)> = shards
            .iter()
            .flat_map(|s| s.slots.iter().map(|(a, b)| (*a, *b, s.primary.clone())))
            .collect();
        ranges.sort();
        let mut primaries: Vec<String> = shards.iter().map(|s| s.primary.clone()).collect();
        primaries.sort();
        primaries.dedup();
        Self { ranges, primaries }
    }
    pub fn node(&self, slot: u16) -> Option<&str> {
        // Binary search for the last range that starts at or before the slot.
        let i = self.ranges.partition_point(|r| r.0 <= slot);
        let r = self.ranges.get(i.checked_sub(1)?)?;
        (slot <= r.1).then_some(r.2.as_str())
    }
    pub fn primaries(&self) -> &[String] {
        &self.primaries
    }
}

/// The key a command is about, for routing. `None`: no key (or we can't tell), so any node may answer; a wrong
/// guess is corrected by the server's `MOVED`.
pub fn key_of<'a>(table: &CommandTable, args: &[&'a [u8]]) -> Option<&'a [u8]> {
    let name = String::from_utf8_lossy(args.first()?).to_ascii_uppercase();
    let has_keys = |i: usize| {
        args.get(i)
            .and_then(|n| std::str::from_utf8(n).ok()?.parse::<usize>().ok())
            .is_some_and(|n| n > 0)
    };
    match name.as_str() {
        // EVAL script numkeys key ...
        "EVAL" | "EVALSHA" | "EVAL_RO" | "EVALSHA_RO" | "FCALL" | "FCALL_RO" => {
            return has_keys(2).then(|| args.get(3).copied()).flatten()
        }
        "XREAD" | "XREADGROUP" => {
            let at = args
                .iter()
                .position(|a| a.eq_ignore_ascii_case(b"STREAMS"))?;
            return args.get(at + 1).copied();
        }
        _ => {}
    }
    match table.first_key(&name) {
        Some(i) if i > 0 => args.get(i as usize).copied(),
        Some(_) => None,
        // Unknown to the table (a module command): the usual place for a key is right after the name.
        None => args.get(1).copied(),
    }
}

fn simple_args(cmd: &Cmd) -> Vec<&[u8]> {
    cmd.args_iter()
        .filter_map(|a| match a {
            Arg::Simple(bytes) => Some(bytes),
            // A cursor placeholder (or a kind of argument added later) is never a key.
            _ => None,
        })
        .collect()
}

fn is_moved(e: &RedisError) -> bool {
    e.kind() == ErrorKind::Server(ServerErrorKind::Moved)
}

/// What the redirect loop needs from a cluster. The real one is `Cluster`; the tests use a scripted fake.
pub trait Nodes: Send + Sync {
    /// The node to try first for this slot (`None`: no key, any node).
    fn node_for(&self, slot: Option<u16>) -> String;
    /// Sends one command to one node. `asking` puts `ASKING` in front (the answer to an `ASK`).
    fn send<'a>(&'a self, addr: &'a str, cmd: &'a Cmd, asking: bool) -> RedisFuture<'a, Value>;
    /// Reads the slot map again (after a `MOVED`: the cluster changed).
    fn refresh<'a>(&'a self) -> RedisFuture<'a, ()>;
}

/// Sends a command where its slot lives, following `MOVED` (and refreshing the map) and `ASK` (once, with
/// `ASKING`), at most `MAX_REDIRECTS` times. Any other answer, error or not, is returned untouched.
pub async fn route<N: Nodes + ?Sized>(
    nodes: &N,
    cmd: &Cmd,
    slot: Option<u16>,
) -> RedisResult<Value> {
    let mut addr = nodes.node_for(slot);
    let mut asking = false;
    for _ in 0..=MAX_REDIRECTS {
        let err = match nodes.send(&addr, cmd, asking).await {
            Err(e) if e.redirect_node().is_some() => e,
            other => return other,
        };
        // `redirect_node` was just checked, so this always has a value.
        let Some((to, _)) = err.redirect_node() else {
            return Err(err);
        };
        // ":7001" means "the same host, another port".
        let next = match to.strip_prefix(':') {
            Some(port) => format!(
                "{}:{port}",
                split(&addr).map(|(h, _)| h).unwrap_or_default()
            ),
            None => to.to_string(),
        };
        asking = !is_moved(&err);
        if is_moved(&err) {
            // The map is stale. If reading it again fails, the address in the error is still the right next try.
            let _ = nodes.refresh().await;
        }
        addr = next;
    }
    Err(RedisError::from((
        ErrorKind::Client,
        "too many redirects",
        "The cluster kept sending this command elsewhere. It may be resharding; try again in a moment.".to_string(),
    )))
}

/// A cluster as the tab uses it: the slot map and one connection per node.
pub struct Cluster {
    opener: Opener,
    table: Arc<CommandTable>,
    seed: String,
    map: RwLock<SlotMap>,
    shards: RwLock<Vec<Shard>>,
    conns: tokio::sync::Mutex<HashMap<String, MultiplexedConnection>>,
}

fn unreachable(e: AppError) -> RedisError {
    RedisError::from(std::io::Error::other(e.message))
}

impl Cluster {
    /// `seed` is the node the database's address points at, already signed in.
    pub async fn new(
        opener: Opener,
        table: Arc<CommandTable>,
        seed_addr: String,
        seed: MultiplexedConnection,
    ) -> AppResult<Arc<Self>> {
        let cluster = Arc::new(Self {
            opener,
            table,
            seed: seed_addr.clone(),
            map: RwLock::default(),
            shards: RwLock::default(),
            conns: tokio::sync::Mutex::new(HashMap::from([(seed_addr, seed)])),
        });
        cluster
            .read_map()
            .await
            .map_err(|e| super::conn::redis_error(&e))?;
        if cluster.primaries().is_empty() {
            return Err(AppError::new(
                ErrorCode::RedisRefused,
                "This cluster has no slots assigned yet. Finish setting it up, then open it again.",
            ));
        }
        Ok(cluster)
    }

    /// The connection to a node, opened through the tab's route the first time it is needed.
    async fn conn(&self, addr: &str) -> RedisResult<MultiplexedConnection> {
        let mut conns = self.conns.lock().await;
        if let Some(c) = conns.get(addr) {
            return Ok(c.clone());
        }
        let (host, port) = split(addr).ok_or_else(|| {
            RedisError::from(std::io::Error::other(format!("{addr} isn't an address")))
        })?;
        let c = self.opener.open(&host, port).await.map_err(unreachable)?;
        conns.insert(addr.to_string(), c.clone());
        Ok(c)
    }

    async fn read_map(&self) -> RedisResult<()> {
        let mut seed = self.conn(&self.seed).await?;
        let seed_host = split(&self.seed).map(|(h, _)| h).unwrap_or_default();
        // CLUSTER SHARDS (Redis 7) says who is healthy; older servers only have CLUSTER SLOTS.
        let shards = match redis::cmd("CLUSTER")
            .arg("SHARDS")
            .query_async::<Value>(&mut seed)
            .await
        {
            Ok(v) => parse_shards(&v, &seed_host, self.opener.tls()),
            Err(_) => Vec::new(),
        };
        let shards = if shards.is_empty() {
            let v: Value = redis::cmd("CLUSTER")
                .arg("SLOTS")
                .query_async(&mut seed)
                .await?;
            parse_slots(&v, &seed_host)
        } else {
            shards
        };
        let map = SlotMap::from_shards(&shards);
        // A poisoned lock only means another thread panicked while writing; the data is replaced whole here.
        *self.map.write().unwrap_or_else(|p| p.into_inner()) = map;
        *self.shards.write().unwrap_or_else(|p| p.into_inner()) = shards;
        Ok(())
    }

    pub fn primaries(&self) -> Vec<String> {
        self.map
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .primaries()
            .to_vec()
    }

    /// The Server view's Nodes panel, read fresh.
    pub async fn nodes(&self) -> AppResult<Vec<RedisNode>> {
        self.read_map()
            .await
            .map_err(|e| super::conn::redis_error(&e))?;
        Ok(nodes_of(
            &self.shards.read().unwrap_or_else(|p| p.into_inner()),
        ))
    }

    /// One connection per primary, in the map's fixed order (for `SCAN` across the cluster).
    pub async fn primary_conns(&self) -> AppResult<Vec<MultiplexedConnection>> {
        let mut out = Vec::new();
        for addr in self.primaries() {
            out.push(
                self.conn(&addr)
                    .await
                    .map_err(|e| super::conn::redis_error(&e))?,
            );
        }
        Ok(out)
    }

    fn slot_of(&self, cmd: &Cmd) -> Option<u16> {
        key_of(&self.table, &simple_args(cmd)).map(slot)
    }

    /// A pipeline. A transaction goes whole to the node of its first key (a second slot is the server's own
    /// `CROSSSLOT` error, returned untouched). Otherwise each command goes to its own node and the replies come
    /// back in the order the commands were given.
    async fn pipeline(
        &self,
        pipeline: &Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisResult<Vec<Value>> {
        let cmds: Vec<&Cmd> = pipeline.cmd_iter().collect();
        if pipeline.is_transaction() {
            let slot = cmds.iter().find_map(|c| self.slot_of(c));
            let mut addr = self.node_for(slot);
            let mut last = None;
            for _ in 0..=MAX_REDIRECTS {
                let mut node = self.conn(&addr).await?;
                match node.req_packed_commands(pipeline, offset, count).await {
                    Err(e) if e.redirect_node().is_some() => {
                        if is_moved(&e) {
                            let _ = self.read_map().await;
                        }
                        if let Some((to, _)) = e.redirect_node() {
                            addr = to.to_string();
                        }
                        last = Some(e);
                    }
                    other => return other,
                }
            }
            return Err(
                last.unwrap_or_else(|| RedisError::from((ErrorKind::Client, "too many redirects")))
            );
        }
        // Group the commands by node, remembering where each reply belongs.
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, c) in cmds.iter().enumerate() {
            let addr = self.node_for(self.slot_of(c));
            match groups.iter_mut().find(|g| g.0 == addr) {
                Some(g) => g.1.push(i),
                None => groups.push((addr, vec![i])),
            }
        }
        let mut replies: Vec<Option<Value>> = vec![None; cmds.len()];
        for (addr, indexes) in groups {
            let mut sub = Pipeline::with_capacity(indexes.len());
            for &i in &indexes {
                sub.add_command(cmds[i].clone());
            }
            let mut node = self.conn(&addr).await?;
            match node.req_packed_commands(&sub, 0, indexes.len()).await {
                Ok(values) => {
                    for (i, v) in indexes.iter().zip(values) {
                        replies[*i] = Some(v);
                    }
                }
                // Something in this group has moved: send its commands one at a time, each following its redirect.
                Err(e) if e.redirect_node().is_some() => {
                    for &i in &indexes {
                        replies[i] = Some(route(self, cmds[i], self.slot_of(cmds[i])).await?);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(replies
            .into_iter()
            .map(|v| v.unwrap_or(Value::Nil))
            .skip(offset)
            .take(count)
            .collect())
    }
}

impl Nodes for Cluster {
    fn node_for(&self, slot: Option<u16>) -> String {
        let map = self.map.read().unwrap_or_else(|p| p.into_inner());
        slot.and_then(|s| map.node(s))
            .or_else(|| map.primaries().first().map(String::as_str))
            .unwrap_or(&self.seed)
            .to_string()
    }

    fn send<'a>(&'a self, addr: &'a str, cmd: &'a Cmd, asking: bool) -> RedisFuture<'a, Value> {
        Box::pin(async move {
            let mut node = self.conn(addr).await?;
            if !asking {
                return node.req_packed_command(cmd).await;
            }
            // ASKING applies to the very next command on this connection, so both go in one pipeline.
            let mut pipe = redis::pipe();
            pipe.cmd("ASKING").ignore().add_command(cmd.clone());
            let (reply,): (Value,) = pipe.query_async(&mut node).await?;
            Ok(reply)
        })
    }

    fn refresh<'a>(&'a self) -> RedisFuture<'a, ()> {
        Box::pin(self.read_map())
    }
}

/// The tab's connection: one server, or a cluster. Cloning is cheap (a handle), like the library's own type.
#[derive(Clone)]
pub enum Conn {
    Single(MultiplexedConnection),
    Cluster(Arc<Cluster>),
}

impl Conn {
    pub fn is_cluster(&self) -> bool {
        matches!(self, Self::Cluster(_))
    }

    /// Where a keyspace walk goes: the server itself, or every primary of the cluster in a fixed order.
    pub async fn scan_targets(&self) -> AppResult<Vec<MultiplexedConnection>> {
        match self {
            Self::Single(c) => Ok(vec![c.clone()]),
            Self::Cluster(c) => c.primary_conns().await,
        }
    }
}

impl ConnectionLike for Conn {
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, Value> {
        match self {
            Self::Single(c) => c.req_packed_command(cmd),
            Self::Cluster(c) => {
                Box::pin(async move { route(c.as_ref(), cmd, c.slot_of(cmd)).await })
            }
        }
    }

    fn req_packed_commands<'a>(
        &'a mut self,
        pipeline: &'a Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<Value>> {
        match self {
            Self::Single(c) => c.req_packed_commands(pipeline, offset, count),
            Self::Cluster(c) => Box::pin(async move { c.pipeline(pipeline, offset, count).await }),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            Self::Single(c) => c.get_db(),
            Self::Cluster(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn crc16_matches_the_known_vectors() {
        // The check value of CRC-16/XMODEM, and the example in the Redis Cluster specification.
        assert_eq!(crc16(b"123456789"), 0x31C3);
        assert_eq!(crc16(b""), 0);
        assert_eq!(crc16(b"A"), 0x58E5);
    }

    #[test]
    fn keys_hash_to_the_slots_redis_gives_them() {
        // `CLUSTER KEYSLOT` answers these on any Redis.
        assert_eq!(slot(b"foo"), 12182);
        assert_eq!(slot(b"bar"), 5061);
        assert_eq!(slot(b"hello"), 866);
        assert_eq!(slot(b"somekey"), 11058);
        assert_eq!(slot(b""), 0);
        assert!(slot(b"anything at all") < SLOTS);
    }

    #[test]
    fn hash_tags_put_keys_in_the_same_slot() {
        assert_eq!(slot(b"{user1000}.following"), slot(b"user1000"));
        assert_eq!(slot(b"{user1000}.following"), slot(b"{user1000}.followers"));
        assert_eq!(slot(b"cart:{42}:items"), slot(b"42"));
        // The first `{` and the first `}` after it decide.
        assert_eq!(slot(b"foo{bar}{zap}"), slot(b"bar"));
        assert_eq!(slot(b"foo{{bar}}zap"), slot(b"{bar"));
        // An empty tag, or one that never closes, does not count.
        assert_eq!(slot(b"foo{}{bar}"), crc16(b"foo{}{bar}") % SLOTS);
        assert_eq!(slot(b"{}user"), crc16(b"{}user") % SLOTS);
        assert_eq!(slot(b"foo{bar"), crc16(b"foo{bar") % SLOTS);
        assert_eq!(slot(b"foo}bar{"), crc16(b"foo}bar{") % SLOTS);
    }

    fn b(s: &str) -> Value {
        Value::BulkString(s.as_bytes().to_vec())
    }
    fn map(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(pairs.into_iter().map(|(k, v)| (b(k), v)).collect())
    }
    fn shard_node(ip: &str, port: i64, role: &str, health: &str) -> Value {
        map(vec![
            ("id", b("abc")),
            ("port", Value::Int(port)),
            ("ip", b(ip)),
            ("endpoint", b(ip)),
            ("role", b(role)),
            ("replication-offset", Value::Int(0)),
            ("health", b(health)),
        ])
    }

    #[test]
    fn cluster_shards_becomes_shards_a_slot_map_and_the_nodes_panel() {
        let reply = Value::Array(vec![
            map(vec![
                (
                    "slots",
                    Value::Array(vec![Value::Int(5461), Value::Int(10922)]),
                ),
                (
                    "nodes",
                    Value::Array(vec![
                        shard_node("10.0.2.12", 7000, "master", "online"),
                        shard_node("10.0.2.15", 7000, "replica", "loading"),
                    ]),
                ),
            ]),
            map(vec![
                ("slots", Value::Array(vec![Value::Int(0), Value::Int(5460)])),
                (
                    "nodes",
                    Value::Array(vec![
                        shard_node("10.0.2.14", 7000, "replica", "online"),
                        shard_node("10.0.2.11", 7000, "master", "online"),
                    ]),
                ),
            ]),
            // RESP2: flat lists instead of maps, and two ranges for one primary.
            Value::Array(vec![
                b("slots"),
                Value::Array(vec![
                    Value::Int(10923),
                    Value::Int(12000),
                    Value::Int(12002),
                    Value::Int(16383),
                ]),
                b("nodes"),
                Value::Array(vec![Value::Array(vec![
                    b("ip"),
                    b(""),
                    b("port"),
                    Value::Int(7002),
                    b("role"),
                    b("master"),
                    b("health"),
                    b("online"),
                ])]),
            ]),
            // A shard with no primary (it failed) is left out of the map.
            map(vec![
                ("slots", Value::Array(vec![])),
                (
                    "nodes",
                    Value::Array(vec![shard_node("10.0.2.19", 7000, "replica", "fail")]),
                ),
            ]),
        ]);
        let shards = parse_shards(&reply, "10.0.2.13", false);
        assert_eq!(shards.len(), 3);
        assert_eq!(
            shards[2].primary, "10.0.2.13:7002",
            "a node without an address is the one we asked"
        );
        let m = SlotMap::from_shards(&shards);
        assert_eq!(m.node(0), Some("10.0.2.11:7000"));
        assert_eq!(m.node(5460), Some("10.0.2.11:7000"));
        assert_eq!(m.node(5461), Some("10.0.2.12:7000"));
        assert_eq!(m.node(10922), Some("10.0.2.12:7000"));
        assert_eq!(m.node(12000), Some("10.0.2.13:7002"));
        assert_eq!(m.node(12001), None, "a slot nobody serves");
        assert_eq!(m.node(16383), Some("10.0.2.13:7002"));
        assert_eq!(
            m.primaries(),
            ["10.0.2.11:7000", "10.0.2.12:7000", "10.0.2.13:7002"]
        );

        let nodes = nodes_of(&shards);
        let row = |i: usize| {
            (
                nodes[i].role.as_str(),
                nodes[i].address.as_str(),
                nodes[i].detail.as_str(),
                nodes[i].state.as_str(),
            )
        };
        assert_eq!(row(0), ("primary", "10.0.2.11:7000", "0–5460", "ok"));
        assert_eq!(
            row(1),
            ("replica", "10.0.2.14:7000", "of 10.0.2.11:7000", "ok")
        );
        assert_eq!(row(2), ("primary", "10.0.2.12:7000", "5461–10922", "ok"));
        assert_eq!(
            row(3),
            (
                "replica",
                "10.0.2.15:7000",
                "of 10.0.2.12:7000",
                "catching up"
            )
        );
        assert_eq!(
            row(4),
            (
                "primary",
                "10.0.2.13:7002",
                "10923–12000, 12002–16383",
                "ok"
            )
        );
    }

    #[test]
    fn tls_uses_the_tls_port_and_cluster_slots_is_the_fallback() {
        let node = map(vec![
            ("ip", b("10.0.2.11")),
            ("port", Value::Int(7000)),
            ("tls-port", Value::Int(7100)),
            ("role", b("master")),
            ("health", b("online")),
        ]);
        let reply = Value::Array(vec![map(vec![
            (
                "slots",
                Value::Array(vec![Value::Int(0), Value::Int(16383)]),
            ),
            ("nodes", Value::Array(vec![node])),
        ])]);
        assert_eq!(parse_shards(&reply, "x", true)[0].primary, "10.0.2.11:7100");
        assert_eq!(
            parse_shards(&reply, "x", false)[0].primary,
            "10.0.2.11:7000"
        );

        let n = |ip: &str, port: i64| Value::Array(vec![b(ip), Value::Int(port), b("id")]);
        let slots = Value::Array(vec![
            Value::Array(vec![
                Value::Int(0),
                Value::Int(8191),
                n("10.0.2.11", 7000),
                n("10.0.2.14", 7000),
            ]),
            Value::Array(vec![
                Value::Int(8192),
                Value::Int(16000),
                n("10.0.2.12", 7000),
            ]),
            Value::Array(vec![
                Value::Int(16001),
                Value::Int(16383),
                n("10.0.2.11", 7000),
                n("10.0.2.14", 7000),
            ]),
        ]);
        let shards = parse_slots(&slots, "seed");
        assert_eq!(shards.len(), 2);
        assert_eq!(shards[0].slots, [(0, 8191), (16001, 16383)]);
        assert_eq!(
            shards[0].replicas,
            [("10.0.2.14:7000".to_string(), String::new())]
        );
        let m = SlotMap::from_shards(&shards);
        assert_eq!(m.node(16383), Some("10.0.2.11:7000"));
        assert_eq!(m.node(9000), Some("10.0.2.12:7000"));
        assert!(parse_slots(&Value::Nil, "seed").is_empty());
        assert_eq!(SlotMap::default().node(5), None);
        assert_eq!(split("[::1]:7000"), Some(("::1".to_string(), 7000)));
        assert_eq!(join("::1", 7000), "[::1]:7000");
    }

    #[test]
    fn the_key_of_a_command_is_found_for_routing() {
        let table = CommandTable::with_keys(&[
            ("GET", 1),
            ("SET", 1),
            ("PING", 0),
            ("INFO", 0),
            ("OBJECT", 2),
            ("EVAL", 0),
            ("XREAD", 0),
        ]);
        let key = |line: &str| {
            let owned: Vec<Vec<u8>> = line
                .split_whitespace()
                .map(|a| a.as_bytes().to_vec())
                .collect();
            let args: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
            key_of(&table, &args).map(|k| String::from_utf8_lossy(k).into_owned())
        };
        assert_eq!(key("GET user:1").as_deref(), Some("user:1"));
        assert_eq!(key("set user:1 v").as_deref(), Some("user:1"));
        assert_eq!(key("OBJECT ENCODING user:1").as_deref(), Some("user:1"));
        assert_eq!(key("PING"), None);
        assert_eq!(key("INFO keyspace"), None);
        assert_eq!(key("EVAL return 1 user:1 arg").as_deref(), Some("user:1"));
        assert_eq!(key("EVAL return 0"), None);
        assert_eq!(
            key("XREAD COUNT 2 STREAMS events 0").as_deref(),
            Some("events")
        );
        // A module command the table doesn't know: the argument after the name.
        assert_eq!(key("JSON.GET doc $").as_deref(), Some("doc"));
        assert_eq!(key(""), None);
    }

    /// A scripted cluster: each node answers a list of replies in order, and every send is recorded.
    #[derive(Default)]
    struct Fake {
        first: String,
        script: Mutex<HashMap<String, Vec<RedisResult<Value>>>>,
        sent: Mutex<Vec<(String, bool)>>,
        refreshed: Mutex<usize>,
    }
    impl Fake {
        fn new(first: &str, script: Vec<(&str, Vec<RedisResult<Value>>)>) -> Self {
            Self {
                first: first.into(),
                script: Mutex::new(
                    script
                        .into_iter()
                        .map(|(a, r)| (a.to_string(), r))
                        .collect(),
                ),
                ..Self::default()
            }
        }
        fn sent(&self) -> Vec<(String, bool)> {
            self.sent.lock().unwrap().clone()
        }
    }
    impl Nodes for Fake {
        fn node_for(&self, _slot: Option<u16>) -> String {
            self.first.clone()
        }
        fn send<'a>(
            &'a self,
            addr: &'a str,
            _cmd: &'a Cmd,
            asking: bool,
        ) -> RedisFuture<'a, Value> {
            self.sent.lock().unwrap().push((addr.to_string(), asking));
            let next = self
                .script
                .lock()
                .unwrap()
                .get_mut(addr)
                .filter(|r| !r.is_empty())
                .map(|r| r.remove(0));
            Box::pin(async move {
                next.unwrap_or_else(|| Err(RedisError::from(std::io::Error::other("no such node"))))
            })
        }
        fn refresh<'a>(&'a self) -> RedisFuture<'a, ()> {
            *self.refreshed.lock().unwrap() += 1;
            Box::pin(async { Ok(()) })
        }
    }
    fn moved(to: &str) -> RedisResult<Value> {
        Err(RedisError::from((
            ErrorKind::Server(ServerErrorKind::Moved),
            "MOVED",
            format!("3999 {to}"),
        )))
    }
    fn ask(to: &str) -> RedisResult<Value> {
        Err(RedisError::from((
            ErrorKind::Server(ServerErrorKind::Ask),
            "ASK",
            format!("3999 {to}"),
        )))
    }
    fn get() -> Cmd {
        let mut c = redis::cmd("GET");
        c.arg("user:1");
        c
    }

    #[tokio::test]
    async fn a_command_that_lands_on_the_right_node_is_sent_once() {
        let f = Fake::new("a:7000", vec![("a:7000", vec![Ok(b("v"))])]);
        assert_eq!(route(&f, &get(), Some(1)).await.unwrap(), b("v"));
        assert_eq!(f.sent(), [("a:7000".to_string(), false)]);
        assert_eq!(*f.refreshed.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn moved_is_followed_and_the_map_is_read_again() {
        let f = Fake::new(
            "a:7000",
            vec![
                ("a:7000", vec![moved("b:7000")]),
                ("b:7000", vec![Ok(b("v"))]),
            ],
        );
        assert_eq!(route(&f, &get(), Some(1)).await.unwrap(), b("v"));
        assert_eq!(
            f.sent(),
            [("a:7000".to_string(), false), ("b:7000".to_string(), false)]
        );
        assert_eq!(*f.refreshed.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn ask_is_followed_once_with_asking_and_the_map_is_left_alone() {
        let f = Fake::new(
            "a:7000",
            vec![
                ("a:7000", vec![ask("b:7000")]),
                ("b:7000", vec![Ok(b("v"))]),
            ],
        );
        assert_eq!(route(&f, &get(), Some(1)).await.unwrap(), b("v"));
        assert_eq!(
            f.sent(),
            [("a:7000".to_string(), false), ("b:7000".to_string(), true)]
        );
        assert_eq!(
            *f.refreshed.lock().unwrap(),
            0,
            "ASK is for one command; the slot has not moved yet"
        );
        // ASK then MOVED (the migration finished in between): the second hop is a plain send again.
        let f = Fake::new(
            "a:7000",
            vec![
                ("a:7000", vec![ask("b:7000")]),
                ("b:7000", vec![moved("c:7000")]),
                ("c:7000", vec![Ok(b("v"))]),
            ],
        );
        assert_eq!(route(&f, &get(), Some(1)).await.unwrap(), b("v"));
        assert_eq!(f.sent()[2], ("c:7000".to_string(), false));
    }

    #[tokio::test]
    async fn a_redirect_without_a_host_means_the_same_host() {
        let f = Fake::new(
            "10.0.2.11:7000",
            vec![
                ("10.0.2.11:7000", vec![moved(":7001")]),
                ("10.0.2.11:7001", vec![Ok(b("v"))]),
            ],
        );
        assert_eq!(route(&f, &get(), None).await.unwrap(), b("v"));
        assert_eq!(f.sent()[1].0, "10.0.2.11:7001");
    }

    #[tokio::test]
    async fn redirects_stop_after_five_and_other_errors_come_back_untouched() {
        // Two nodes that keep pointing at each other.
        let ping_pong = |n: usize| (0..n).map(|_| moved("b:7000")).collect::<Vec<_>>();
        let f = Fake::new(
            "a:7000",
            vec![
                ("a:7000", ping_pong(4)),
                ("b:7000", (0..4).map(|_| moved("a:7000")).collect()),
            ],
        );
        let err = route(&f, &get(), Some(1)).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Client);
        assert_eq!(
            f.sent().len(),
            MAX_REDIRECTS + 1,
            "the first try and five redirects"
        );

        let crossslot = || {
            Err(RedisError::from((
                ErrorKind::Server(ServerErrorKind::CrossSlot),
                "CROSSSLOT",
                "Keys in request don't hash to the same slot".to_string(),
            )))
        };
        let f = Fake::new("a:7000", vec![("a:7000", vec![crossslot()])]);
        let err = route(&f, &get(), Some(1)).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Server(ServerErrorKind::CrossSlot));
        assert_eq!(f.sent().len(), 1);
    }
}
