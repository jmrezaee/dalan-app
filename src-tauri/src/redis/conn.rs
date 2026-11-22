//! Live Redis connections (F13; S33 "Connecting" and "When it fails"). One per Redis tab.
//!
//! Opening returns at once with an id; progress arrives as `RedisEvent`s: the plan (one step per SSH hop, then
//! "reach Redis", then "sign in"), each step's state, the host's own questions (the S18 prompts: host key,
//! password, passphrase, 2FA), and finally `Ready` or `Failed`.
//!
//! While connected, a task pings every 30 s. When the connection drops it tries again after 1, 2, 4, 8 and 15 s
//! (five tries), telling the tab each time. Nothing is ever replayed: a command that was in flight just fails.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, ProtocolVersion, RedisConnectionInfo, RedisError};
use serde::Serialize;
use specta::Type;
use tokio::sync::watch;
use zeroize::Zeroizing;

use super::browse::RedisBytes;
use super::bulk::BulkState;
use super::cluster::{self, Cluster, Conn, RedisNode};
use super::console::ConsoleState;
use super::gate::CommandTable;
use super::pubsub::{Listener, SubEmit};
use super::sentinel;
use super::store::{self, RedisKind, RedisRoute};
use super::transport::{self, BoxStream, Target};
use super::write::PendingStore;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::hosts;
use crate::ssh::{
    connect_authenticated, Connected, PromptAnswer, Prompts, SessionEvent, SessionIo,
};
use crate::state::Store;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PING_EVERY: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
/// Waits before each reconnect attempt, in seconds. The length is the number of tries.
const BACKOFF: [u32; 5] = [1, 2, 4, 8, 15];

#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisPlanStep {
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RedisStepState {
    Active,
    Done,
    Failed,
}

/// Keys in one logical database (`db0`, `db1`, ...), for the picker in the tab's toolbar.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisDbKeys {
    pub index: u32,
    // A count far below 2^53, so a JS number holds it exactly.
    #[specta(type = f64)]
    pub keys: u64,
}

/// What the server said about itself right after sign-in.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisServerInfo {
    pub version: String,
    /// `standalone`, `cluster` or `sentinel`, in the server's own word.
    pub mode: String,
    /// A replica only accepts reads: the tab opens read-only.
    pub replica: bool,
    pub resp3: bool,
    /// How many logical databases the server has (16 unless configured otherwise).
    pub db_count: u32,
    /// The databases that hold keys.
    pub databases: Vec<RedisDbKeys>,
    pub selected: u32,
    pub latency_ms: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedisEvent {
    /// The steps of this connection attempt, in order. Sent first, and again before each reconnect.
    Plan {
        steps: Vec<RedisPlanStep>,
    },
    Step {
        index: u32,
        state: RedisStepState,
        detail: Option<String>,
    },
    /// A question or a note from the SSH sign-in of a hop (the S18 machinery). Answer prompts with `redis_respond`.
    Host {
        event: SessionEvent,
    },
    Ready {
        info: RedisServerInfo,
    },
    Latency {
        ms: u32,
    },
    Failed {
        code: ErrorCode,
        message: String,
    },
    /// The connection dropped; trying again in `retry_in_secs` (attempt `attempt` of `of`).
    Dropped {
        attempt: u32,
        of: u32,
        retry_in_secs: u32,
    },
    Closed,
}

pub type Emit = Arc<dyn Fn(RedisEvent) + Send + Sync>;

/// Everything needed to connect, from a saved database or from the editor's form (Test connection).
pub struct Config {
    pub name: String,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    pub kind: RedisKind,
    /// Sentinel only: the name the sentinels know the primary by.
    pub sentinel_master: Option<String>,
    pub target: Target,
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub db: u32,
}

/// A signed-in connection, with what keeps it alive.
pub struct Connection {
    /// One server, or a cluster (which routes each command to the node that holds its key).
    pub conn: Conn,
    pub info: RedisServerInfo,
    /// The server's command table, read once after sign-in, for the write gate. Empty when the server would not
    /// show it: the gate then treats every command it does not know by name as a write.
    pub table: Arc<CommandTable>,
    /// How to open more connections through the same route (cluster nodes, the Pub/Sub listener, sentinels).
    /// It holds the SSH connection the streams run through; dropping the last copy closes them.
    opener: Opener,
    /// The server this connection signed in to (for Sentinel: the primary the sentinels named).
    target: Target,
    /// Sentinel only: the sentinels and the primary's name, for the Nodes panel.
    sentinels: Option<(Vec<(String, u16)>, String)>,
}

/// Opens signed-in connections to this database's servers through the tab's route: directly, or as another
/// `direct-tcpip` channel on the same SSH connection. Never a local listener. The password stays zeroized.
#[derive(Clone)]
pub struct Opener {
    /// `Arc`: the cluster's node connections and the listener share the one SSH connection.
    ssh: Option<Arc<Connected>>,
    username: Option<String>,
    password: Option<Zeroizing<String>>,
    db: u32,
    tls: bool,
    /// What the first sign-in found the server speaks.
    resp3: bool,
}

impl Opener {
    pub fn tls(&self) -> bool {
        self.tls
    }

    fn target(&self, host: &str, port: u16) -> Target {
        Target {
            address: host.to_string(),
            port,
            tls: self.tls,
            // A certificate name set for the database's address does not fit another node: check each by its own.
            tls_server_name: None,
        }
    }

    /// The sign-in on a fresh stream. `auth`: with the database's user, password and logical database.
    /// `pushes`: a listening connection, which gets no response timeout (it is quiet between messages by design).
    async fn handshake(
        &self,
        stream: BoxStream,
        protocol: ProtocolVersion,
        auth: bool,
        pushes: Option<tokio::sync::mpsc::UnboundedSender<redis::PushInfo>>,
    ) -> AppResult<MultiplexedConnection> {
        let mut info = RedisConnectionInfo::default().set_protocol(protocol);
        if auth {
            info = info.set_db(i64::from(self.db));
            if let Some(u) = &self.username {
                info = info.set_username(u);
            }
            if let Some(p) = &self.password {
                info = info.set_password(p.as_str());
            }
        }
        let config = match pushes {
            Some(tx) => AsyncConnectionConfig::new().set_push_sender(tx),
            None => AsyncConnectionConfig::new().set_response_timeout(Some(COMMAND_TIMEOUT)),
        };
        let (conn, driver) = tokio::time::timeout(
            transport::CONNECT_TIMEOUT,
            MultiplexedConnection::new_with_config(&info, stream, config),
        )
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::RedisUnreachable,
                "Redis didn't answer the sign-in in time.",
            )
        })?
        .map_err(|e| redis_error(&e))?;
        // The driver moves bytes between the stream and the connection handle until the stream ends.
        tokio::spawn(driver);
        Ok(conn)
    }

    fn protocol(&self) -> ProtocolVersion {
        if self.resp3 {
            ProtocolVersion::RESP3
        } else {
            ProtocolVersion::RESP2
        }
    }

    /// A signed-in connection to another node of the same cluster.
    pub async fn open(&self, host: &str, port: u16) -> AppResult<MultiplexedConnection> {
        let stream = open_stream(self.ssh.as_deref(), &self.target(host, port)).await?;
        self.handshake(stream, self.protocol(), true, None).await
    }

    /// A connection to a sentinel: the older protocol and no sign-in (sentinels have their own, which Dalan
    /// does not store yet).
    pub async fn open_plain(&self, host: &str, port: u16) -> AppResult<MultiplexedConnection> {
        let stream = open_stream(self.ssh.as_deref(), &self.target(host, port)).await?;
        self.handshake(stream, ProtocolVersion::RESP2, false, None)
            .await
    }
}

impl Connection {
    /// A second connection for listening: its own stream through the same route (never a local listener), RESP3,
    /// with the server's pushes sent to the returned receiver.
    async fn listener(
        &self,
    ) -> AppResult<(
        MultiplexedConnection,
        tokio::sync::mpsc::UnboundedReceiver<redis::PushInfo>,
    )> {
        if !self.info.resp3 {
            return Err(AppError::validation(
                "Listening needs Redis 6 or newer (this server speaks the older protocol).",
            ));
        }
        let stream = open_stream(self.opener.ssh.as_deref(), &self.target).await?;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let conn = self
            .opener
            .handshake(stream, ProtocolVersion::RESP3, true, Some(tx))
            .await?;
        Ok((conn, rx))
    }

    /// The Server view's Nodes panel: a cluster's primaries and replicas, or a Sentinel set. Empty for one server.
    async fn nodes(&self) -> AppResult<Vec<RedisNode>> {
        match (&self.conn, &self.sentinels) {
            (Conn::Cluster(c), _) => c.nodes().await,
            (_, Some((sentinels, master))) => {
                sentinel::nodes(&self.opener, sentinels, master).await
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn shutdown(self) {
        // The connection handles first (a cluster holds copies of the opener), then the SSH connection under them.
        let ssh = self.opener.ssh.clone();
        drop(self.conn);
        if let Some(ssh) = ssh {
            let _ = ssh
                .handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

pub(super) fn redis_error(e: &RedisError) -> AppError {
    let text = e.to_string();
    let code = e.code().unwrap_or("");
    if e.kind() == redis::ErrorKind::AuthenticationFailed
        || matches!(code, "WRONGPASS" | "NOAUTH" | "NOPERM")
        || text.contains("WRONGPASS")
        || text.contains("invalid password")
        || text.contains("invalid username-password")
    {
        return AppError::new(
            ErrorCode::RedisAuthFailed,
            "Wrong username or password. Redis refused the sign-in.",
        );
    }
    if e.is_io_error() || e.is_connection_dropped() || e.is_timeout() {
        return AppError::new(
            ErrorCode::RedisUnreachable,
            "The connection to Redis closed before it answered. Is this really a Redis port?",
        );
    }
    // The server's own sentence, as it wrote it: `WRONGTYPE Operation against a key ...`. (The crate's own
    // formatting puts the code in quotes, or uses its name for the error, which is not what the server said.)
    let said = match (e.code(), e.detail()) {
        (Some(code), Some(detail)) => format!("{code} {detail}"),
        (Some(code), None) => code.to_string(),
        _ => text,
    };
    AppError::new(ErrorCode::RedisRefused, said)
}

/// The steps the tab shows for this database, before anything is tried.
async fn plan(store: &Store, cfg: &Config) -> AppResult<(Vec<RedisPlanStep>, usize)> {
    let mut steps = Vec::new();
    let mut hops = 0;
    if let (RedisRoute::Host, Some(host_id)) = (cfg.route, cfg.host_id.clone()) {
        let chain = store
            .with_db_session(move |c, s| hosts::connect_plan(c, s, &host_id))
            .await?;
        hops = chain.len();
        for (i, hop) in chain.iter().enumerate() {
            steps.push(RedisPlanStep {
                label: format!("Sign in to {}", hop.label),
                detail: (i > 0).then(|| format!("through {}", chain[i - 1].label)),
            });
        }
    }
    if cfg.kind == RedisKind::Sentinel {
        steps.push(RedisPlanStep {
            label: format!(
                "Ask the sentinels for {}",
                cfg.sentinel_master.as_deref().unwrap_or("the primary")
            ),
            detail: None,
        });
    }
    steps.push(RedisPlanStep {
        label: if cfg.kind == RedisKind::Sentinel {
            "Reach the primary".to_string()
        } else {
            format!("Reach Redis at {}", cfg.target.display())
        },
        detail: cfg.target.tls.then(|| "with TLS".to_string()),
    });
    steps.push(RedisPlanStep {
        label: match &cfg.username {
            Some(u) => format!("Sign in as {u}"),
            None => "Sign in to Redis".to_string(),
        },
        detail: None,
    });
    Ok((steps, hops))
}

/// Signs in to the host (every hop asks its own questions through `emit`), or says why it couldn't.
pub(crate) async fn sign_in_to_host(
    store: &Store,
    host_id: &str,
    hops: usize,
    prompts: &Arc<Prompts>,
    emit: &Emit,
) -> AppResult<Connected> {
    let last_error: Arc<Mutex<Option<(ErrorCode, String)>>> = Arc::default();
    // Which hop is signing in now. Each hop ends with a "Signed in" progress line from the SSH client.
    let current = Arc::new(AtomicUsize::new(0));
    let io = {
        let (emit, last, current) = (
            Arc::clone(emit),
            Arc::clone(&last_error),
            Arc::clone(&current),
        );
        Arc::new(SessionIo {
            event: Box::new(move |e| match e {
                SessionEvent::Error { code, message } => {
                    if let Ok(mut l) = last.lock() {
                        *l = Some((code, message));
                    }
                }
                SessionEvent::Step { text } => {
                    let i = current.load(Ordering::Relaxed);
                    if text.starts_with("Signed in") {
                        emit(RedisEvent::Step {
                            index: i as u32,
                            state: RedisStepState::Done,
                            detail: None,
                        });
                        if i + 1 < hops {
                            current.store(i + 1, Ordering::Relaxed);
                            emit(RedisEvent::Step {
                                index: (i + 1) as u32,
                                state: RedisStepState::Active,
                                detail: None,
                            });
                        }
                    } else {
                        emit(RedisEvent::Step {
                            index: i as u32,
                            state: RedisStepState::Active,
                            detail: Some(text),
                        });
                    }
                }
                prompt @ SessionEvent::Prompt { .. } => emit(RedisEvent::Host { event: prompt }),
                _ => {}
            }),
            data: Box::new(|_| {}),
        })
    };
    emit(RedisEvent::Step {
        index: 0,
        state: RedisStepState::Active,
        detail: None,
    });
    match connect_authenticated(store, &io, prompts, host_id, None, false).await {
        Some(ssh) => Ok(ssh),
        None => {
            let failed = current.load(Ordering::Relaxed) as u32;
            let err = last_error
                .lock()
                .ok()
                .and_then(|mut l| l.take())
                .map(|(code, message)| AppError::new(code, message))
                .unwrap_or_else(|| {
                    AppError::new(
                        ErrorCode::AuthFailed,
                        "The sign-in to the host was cancelled.",
                    )
                });
            emit(RedisEvent::Step {
                index: failed,
                state: RedisStepState::Failed,
                detail: Some(err.message.clone()),
            });
            Err(err)
        }
    }
}

async fn open_stream(ssh: Option<&Connected>, target: &Target) -> AppResult<BoxStream> {
    match ssh {
        Some(ssh) => transport::through_host(ssh, target).await,
        None => transport::direct(target).await,
    }
}

/// Signs in on a fresh stream. RESP3 first (`HELLO 3`); a server too old for it gets a second stream and RESP2.
async fn sign_in(
    opener: &Opener,
    target: &Target,
    first: BoxStream,
) -> AppResult<(MultiplexedConnection, bool)> {
    match opener
        .handshake(first, ProtocolVersion::RESP3, true, None)
        .await
    {
        Ok(conn) => Ok((conn, true)),
        // Wrong password, nothing there, bad certificate: a second try would say the same.
        Err(e) if e.code != ErrorCode::RedisRefused => Err(e),
        Err(_) => {
            let second = open_stream(opener.ssh.as_deref(), target).await?;
            let conn = opener
                .handshake(second, ProtocolVersion::RESP2, true, None)
                .await?;
            Ok((conn, false))
        }
    }
}

/// Keys in a whole cluster: `DBSIZE` of every primary, added up.
async fn total_keys(conn: &Conn) -> u64 {
    let mut total = 0u64;
    for mut node in conn.scan_targets().await.unwrap_or_default() {
        total += redis::cmd("DBSIZE")
            .query_async::<u64>(&mut node)
            .await
            .unwrap_or(0);
    }
    total
}

fn info_field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
}

/// `db0:keys=12408,expires=3,avg_ttl=0` lines from `INFO keyspace`.
fn keyspace(info: &str) -> Vec<RedisDbKeys> {
    info.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let index = name.strip_prefix("db")?.parse().ok()?;
            let keys = rest
                .split(',')
                .find_map(|kv| kv.strip_prefix("keys="))?
                .trim()
                .parse()
                .ok()?;
            Some(RedisDbKeys { index, keys })
        })
        .collect()
}

async fn server_info(
    conn: &mut MultiplexedConnection,
    resp3: bool,
    selected: u32,
) -> AppResult<RedisServerInfo> {
    let started = Instant::now();
    let _: String = redis::cmd("PING")
        .query_async(conn)
        .await
        .map_err(|e| redis_error(&e))?;
    let latency_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    // A limited ACL user may not be allowed INFO or CONFIG: the tab still opens, with what is known.
    let info: String = redis::cmd("INFO")
        .query_async(conn)
        .await
        .unwrap_or_default();
    let db_count = redis::cmd("CONFIG")
        .arg("GET")
        .arg("databases")
        .query_async::<Vec<String>>(conn)
        .await
        .ok()
        .and_then(|v| v.get(1)?.parse().ok())
        .unwrap_or(16);
    let _: Result<(), _> = redis::cmd("CLIENT")
        .arg("SETNAME")
        .arg("dalan")
        .query_async(conn)
        .await;
    Ok(RedisServerInfo {
        version: info_field(&info, "redis_version")
            .unwrap_or("unknown")
            .to_string(),
        mode: info_field(&info, "redis_mode")
            .unwrap_or("standalone")
            .to_string(),
        replica: matches!(info_field(&info, "role"), Some("slave" | "replica")),
        resp3,
        db_count,
        databases: keyspace(&info),
        selected,
        latency_ms,
    })
}

/// One whole attempt: host sign-in (if any), the sentinels (if any), the stream, the Redis sign-in, the server's
/// details, and for a cluster its slot map.
pub async fn connect(
    store: &Store,
    cfg: &Config,
    prompts: &Arc<Prompts>,
    emit: &Emit,
) -> AppResult<Connection> {
    let (steps, hops) = plan(store, cfg).await?;
    emit(RedisEvent::Plan { steps });
    let ssh = match (&cfg.host_id, cfg.route) {
        (Some(host_id), RedisRoute::Host) => Some(Arc::new(
            sign_in_to_host(store, host_id, hops, prompts, emit).await?,
        )),
        _ => None,
    };
    let mut opener = Opener {
        ssh,
        username: cfg.username.clone(),
        password: cfg.password.clone(),
        // A cluster has one logical database.
        db: if cfg.kind == RedisKind::Cluster {
            0
        } else {
            cfg.db
        },
        tls: cfg.target.tls,
        resp3: true,
    };
    let step = |index: u32, state: RedisStepState, detail: Option<String>| {
        emit(RedisEvent::Step {
            index,
            state,
            detail,
        });
    };
    let fail = |index: u32, e: AppError| {
        step(index, RedisStepState::Failed, Some(e.message.clone()));
        e
    };

    // Sentinel: the address is the sentinels'; ask them where the primary is.
    let mut next = hops as u32;
    let mut target = cfg.target.clone();
    let mut sentinels = None;
    if cfg.kind == RedisKind::Sentinel {
        let find = next;
        next += 1;
        step(find, RedisStepState::Active, None);
        let list = sentinel::parse_list(&cfg.target.address, cfg.target.port);
        let master = cfg.sentinel_master.clone().unwrap_or_default();
        let (host, port) = sentinel::find_primary(&opener, &list, &master)
            .await
            .map_err(|e| fail(find, e))?;
        step(find, RedisStepState::Done, Some(cluster::join(&host, port)));
        target = Target {
            address: host,
            port,
            ..cfg.target.clone()
        };
        sentinels = Some((list, master));
    }
    let (reach, auth) = (next, next + 1);

    step(reach, RedisStepState::Active, None);
    let stream = open_stream(opener.ssh.as_deref(), &target)
        .await
        .map_err(|e| fail(reach, e))?;
    step(reach, RedisStepState::Done, None);

    step(auth, RedisStepState::Active, None);
    let (mut single, resp3) = sign_in(&opener, &target, stream)
        .await
        .map_err(|e| fail(auth, e))?;
    opener.resp3 = resp3;
    let mut info = server_info(&mut single, resp3, opener.db)
        .await
        .map_err(|e| fail(auth, e))?;
    if sentinels.is_some() && info.replica {
        return Err(fail(
            auth,
            AppError::new(
                ErrorCode::RedisUnreachable,
                format!(
                    "The sentinels pointed at {}, but it says it is a replica. A failover may be under way; try again in a moment.",
                    target.display()
                ),
            ),
        ));
    }
    let table = Arc::new(
        redis::cmd("COMMAND")
            .query_async::<redis::Value>(&mut single)
            .await
            .map(|reply| CommandTable::from_reply(&reply))
            .unwrap_or_default(),
    );
    let conn = if info.mode == "cluster" {
        // The slot map, and from here on one connection per node, each through the same route.
        let seed = cluster::join(&target.address, target.port);
        let c = Cluster::new(opener.clone(), Arc::clone(&table), seed, single)
            .await
            .map_err(|e| fail(auth, e))?;
        let conn = Conn::Cluster(c);
        info.databases = vec![RedisDbKeys {
            index: 0,
            keys: total_keys(&conn).await,
        }];
        info.db_count = 1;
        info.selected = 0;
        conn
    } else if cfg.kind == RedisKind::Cluster {
        return Err(fail(
            auth,
            AppError::validation(
                "This server isn't running as a cluster. Edit the database and choose Single server.",
            ),
        ));
    } else {
        Conn::Single(single)
    };
    step(auth, RedisStepState::Done, None);
    Ok(Connection {
        conn,
        info,
        table,
        opener,
        target,
        sentinels,
    })
}

/// What `redis_db_test` answers.
#[derive(Debug, Clone, Serialize, Type)]
pub struct RedisTestResult {
    pub version: String,
    pub mode: String,
    pub replica: bool,
    pub latency_ms: u32,
}

/// Connects with the form's values, reads the server's details and closes. Nobody can answer a host's question
/// from the editor, so a host that needs one (a new key, a password) fails with a message that says so.
pub async fn test(store: &Store, cfg: Config) -> AppResult<RedisTestResult> {
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let host_label = cfg.name.clone();
    let c = connect(store, &cfg, &prompts, &quiet).await.map_err(|e| {
        if e.code == ErrorCode::AuthFailed && cfg.route == RedisRoute::Host {
            AppError::new(
                ErrorCode::AuthFailed,
                format!("The host for {host_label} needs an answer from you (a new host key or a password). Open a terminal on it once, then test again."),
            )
        } else {
            e
        }
    })?;
    let result = RedisTestResult {
        version: c.info.version.clone(),
        mode: c.info.mode.clone(),
        replica: c.info.replica,
        latency_ms: c.info.latency_ms,
    };
    c.shutdown().await;
    Ok(result)
}

/// One Redis tab's connection, as the manager keeps it.
struct Live {
    prompts: Arc<Prompts>,
    /// Set to true by `close`: every wait in the task ends.
    cancel: watch::Sender<bool>,
    /// `Some` while connected. A tokio mutex, because commands hold it across an `.await`.
    conn: tokio::sync::Mutex<Option<Connection>>,
    selected: AtomicU32,
    db_id: String,
    /// Writes waiting for their confirmation. They belong to this connection and end with it.
    pending: Arc<PendingStore>,
    /// The console's open `MULTI`, if any.
    console: Arc<ConsoleState>,
    /// The Pub/Sub listener: its own connection, opened when the tab first listens.
    sub: tokio::sync::Mutex<Option<Listener>>,
    bulk: Arc<BulkState>,
}

/// What a write needs from a live connection.
pub struct Writer {
    pub conn: Conn,
    pub table: Arc<CommandTable>,
    pub replica: bool,
    pub db_id: String,
    pub pending: Arc<PendingStore>,
    pub console: Arc<ConsoleState>,
    pub bulk: Arc<BulkState>,
}

#[derive(Default)]
pub struct RedisManager {
    conns: Mutex<HashMap<String, Arc<Live>>>,
}

async fn cancelled(rx: &mut watch::Receiver<bool>) {
    // `changed` errors when the sender is gone, which also means "stop".
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

impl RedisManager {
    fn get(&self, conn_id: &str) -> AppResult<Arc<Live>> {
        self.conns
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))?
            .get(conn_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("That Redis connection is closed."))
    }

    /// Starts connecting and returns the connection's id at once. Everything else arrives through `emit`.
    pub fn open(&self, store: Store, db_id: String, emit: Emit) -> AppResult<String> {
        let conn_id = uuid::Uuid::now_v7().to_string();
        let (cancel, cancel_rx) = watch::channel(false);
        let live = Arc::new(Live {
            prompts: Arc::new(Prompts::default()),
            cancel,
            conn: tokio::sync::Mutex::new(None),
            selected: AtomicU32::new(u32::MAX),
            db_id: db_id.clone(),
            pending: Arc::default(),
            console: Arc::default(),
            sub: tokio::sync::Mutex::new(None),
            bulk: Arc::default(),
        });
        self.conns
            .lock()
            .map_err(|_| AppError::internal("redis lock poisoned"))?
            .insert(conn_id.clone(), Arc::clone(&live));
        tokio::spawn(run(store, db_id, live, emit, cancel_rx));
        Ok(conn_id)
    }

    /// The answer to a host's question (host key, password, passphrase, 2FA) asked while opening.
    pub fn respond(&self, conn_id: &str, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        self.get(conn_id)?.prompts.answer(prompt_id, answer)
    }

    /// A clone of the connection handle for one command. Errors while the tab is connecting or reconnecting.
    pub async fn handle(&self, conn_id: &str) -> AppResult<Conn> {
        let live = self.get(conn_id)?;
        let guard = live.conn.lock().await;
        guard.as_ref().map(|c| c.conn.clone()).ok_or_else(|| {
            AppError::new(
                ErrorCode::RedisUnreachable,
                "This database isn't connected right now.",
            )
        })
    }

    /// The connection with what the write gate needs. Errors while the tab is connecting or reconnecting.
    pub async fn writer(&self, conn_id: &str) -> AppResult<Writer> {
        let live = self.get(conn_id)?;
        let guard = live.conn.lock().await;
        let c = guard.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::RedisUnreachable,
                "This database isn't connected right now.",
            )
        })?;
        Ok(Writer {
            conn: c.conn.clone(),
            table: Arc::clone(&c.table),
            replica: c.info.replica,
            db_id: live.db_id.clone(),
            pending: Arc::clone(&live.pending),
            console: Arc::clone(&live.console),
            bulk: Arc::clone(&live.bulk),
        })
    }

    /// F17: starts listening to a channel or a pattern. The first call opens the listening connection.
    pub async fn sub_start(
        &self,
        conn_id: &str,
        name: &RedisBytes,
        pattern: bool,
        emit: SubEmit,
    ) -> AppResult<()> {
        let live = self.get(conn_id)?;
        let mut sub = live.sub.lock().await;
        if sub.is_none() {
            let guard = live.conn.lock().await;
            let c = guard.as_ref().ok_or_else(|| {
                AppError::new(
                    ErrorCode::RedisUnreachable,
                    "This database isn't connected right now.",
                )
            })?;
            let (conn, pushes) = c.listener().await?;
            *sub = Some(Listener::new(conn, pushes, Arc::clone(&emit)));
        }
        // `sub` is `Some` here: it was just filled if it was empty.
        let Some(listener) = sub.as_mut() else {
            return Err(AppError::internal("no listener"));
        };
        listener.set_emit(emit);
        listener.start(name, pattern).await
    }

    /// Stops listening to one name. The listening connection closes with its last name.
    pub async fn sub_stop(&self, conn_id: &str, name: &RedisBytes, pattern: bool) -> AppResult<()> {
        let live = self.get(conn_id)?;
        let mut sub = live.sub.lock().await;
        if let Some(listener) = sub.as_mut() {
            listener.stop(name, pattern).await?;
            if listener.is_idle() {
                *sub = None;
            }
        }
        Ok(())
    }

    /// "Stop" in the bulk dialog. Works while the scan or the run holds the connection.
    pub fn bulk_stop(&self, conn_id: &str) {
        if let Ok(live) = self.get(conn_id) {
            live.bulk.stop();
        }
    }

    /// The console ran `SELECT`: remember it, so a reconnect comes back to the same logical database.
    pub fn note_selected(&self, conn_id: &str, index: u32) {
        if let Ok(live) = self.get(conn_id) {
            live.selected.store(index, Ordering::Relaxed);
        }
    }

    /// Switches the logical database (`SELECT`) and says how many keys it holds. A cluster has only database 0;
    /// asking for it answers with the keys of every primary together.
    pub async fn select(&self, conn_id: &str, index: u32) -> AppResult<RedisDbKeys> {
        let live = self.get(conn_id)?;
        let mut conn = self.handle(conn_id).await?;
        if conn.is_cluster() {
            if index != 0 {
                return Err(AppError::validation("A cluster has one logical database."));
            }
            return Ok(RedisDbKeys {
                index: 0,
                keys: total_keys(&conn).await,
            });
        }
        redis::cmd("SELECT")
            .arg(index)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| redis_error(&e))?;
        live.selected.store(index, Ordering::Relaxed);
        let keys: u64 = redis::cmd("DBSIZE")
            .query_async(&mut conn)
            .await
            .map_err(|e| redis_error(&e))?;
        Ok(RedisDbKeys { index, keys })
    }

    /// S31 "Nodes": the cluster's nodes or the Sentinel set, read fresh. Empty for a single server.
    pub async fn nodes(&self, conn_id: &str) -> AppResult<Vec<RedisNode>> {
        let live = self.get(conn_id)?;
        let guard = live.conn.lock().await;
        match guard.as_ref() {
            Some(c) => c.nodes().await,
            None => Err(AppError::new(
                ErrorCode::RedisUnreachable,
                "This database isn't connected right now.",
            )),
        }
    }

    pub fn close(&self, conn_id: &str) {
        let live = self.conns.lock().ok().and_then(|mut m| m.remove(conn_id));
        if let Some(live) = live {
            live.prompts.cancel_all_pub();
            let _ = live.cancel.send(true);
        }
    }

    /// Lock (F10): every Redis connection ends.
    pub fn close_all(&self) {
        let all: Vec<Arc<Live>> = self
            .conns
            .lock()
            .map(|mut m| m.drain().map(|(_, v)| v).collect())
            .unwrap_or_default();
        for live in all {
            live.prompts.cancel_all_pub();
            let _ = live.cancel.send(true);
        }
    }

    pub fn live_count(&self) -> usize {
        self.conns.lock().map(|m| m.len()).unwrap_or(0)
    }
}

/// Reads the saved database into a `Config`, password included.
pub async fn load(store: &Store, db_id: &str) -> AppResult<Config> {
    let id = db_id.to_string();
    store
        .with_db_session(move |c, s| {
            let d = store::get(c, &s.vault_id, &id)?;
            let password = store::password(c, s, &id)?;
            store::touch_opened(c, &s.vault_id, &id)?;
            Ok(Config {
                name: d.name,
                route: d.route,
                host_id: d.host_id,
                kind: d.kind,
                sentinel_master: d.sentinel_master,
                target: Target {
                    address: d.address,
                    port: d.port,
                    tls: d.tls,
                    tls_server_name: d.tls_server_name,
                },
                username: d.username,
                password,
                db: d.default_db,
            })
        })
        .await
}

/// The task behind one tab: connect, keep alive, reconnect, until closed or out of tries.
async fn run(
    store: Store,
    db_id: String,
    live: Arc<Live>,
    emit: Emit,
    mut cancel: watch::Receiver<bool>,
) {
    let mut attempt = 0usize; // reconnect attempts since the last good connection
    let mut ever_connected = false;
    loop {
        let result = tokio::select! {
            r = async {
                let mut cfg = load(&store, &db_id).await?;
                // After a drop, come back to the logical database the tab was on.
                let selected = live.selected.load(Ordering::Relaxed);
                if selected != u32::MAX {
                    cfg.db = selected;
                }
                connect(&store, &cfg, &live.prompts, &emit).await
            } => r,
            _ = cancelled(&mut cancel) => break,
        };
        match result {
            Ok(c) => {
                attempt = 0;
                ever_connected = true;
                live.selected.store(c.info.selected, Ordering::Relaxed);
                let info = c.info.clone();
                let mut pinger = c.conn.clone();
                *live.conn.lock().await = Some(c);
                emit(RedisEvent::Ready { info });
                // Keep-alive: a PING every 30 s. The first failure means the connection is gone.
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(PING_EVERY) => {}
                        _ = cancelled(&mut cancel) => break,
                    }
                    let started = Instant::now();
                    let pong = tokio::time::timeout(
                        PING_TIMEOUT,
                        redis::cmd("PING").query_async::<String>(&mut pinger),
                    )
                    .await;
                    match pong {
                        Ok(Ok(_)) => emit(RedisEvent::Latency {
                            ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
                        }),
                        _ => break,
                    }
                }
                // The listener ran through the same route: it ends with the connection. The tab listens again once it is back.
                live.sub.lock().await.take();
                if let Some(c) = live.conn.lock().await.take() {
                    c.shutdown().await;
                }
                if *cancel.borrow() {
                    break;
                }
            }
            // The first connection failed: say why and stop. The tab offers "Try again" or "Edit database".
            Err(e) if !ever_connected => {
                emit(RedisEvent::Failed {
                    code: e.code,
                    message: e.message,
                });
                return;
            }
            Err(_) => {}
        }
        // Dropped, or a reconnect attempt failed: wait and try again, up to five times.
        if attempt >= BACKOFF.len() {
            emit(RedisEvent::Failed {
                code: ErrorCode::RedisUnreachable,
                message: "The connection dropped and didn't come back after five tries.".into(),
            });
            return;
        }
        let wait = BACKOFF[attempt];
        attempt += 1;
        emit(RedisEvent::Dropped {
            attempt: attempt as u32,
            of: BACKOFF.len() as u32,
            retry_in_secs: wait,
        });
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(u64::from(wait))) => {}
            _ = cancelled(&mut cancel) => break,
        }
    }
    live.sub.lock().await.take();
    live.bulk.stop();
    if let Some(c) = live.conn.lock().await.take() {
        c.shutdown().await;
    }
    emit(RedisEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_fields_and_the_keyspace_are_read() {
        let info = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n# Replication\r\nrole:master\r\n# Keyspace\r\ndb0:keys=12408,expires=3,avg_ttl=0\r\ndb3:keys=2,expires=0,avg_ttl=0\r\n";
        assert_eq!(info_field(info, "redis_version"), Some("7.2.4"));
        assert_eq!(info_field(info, "redis_mode"), Some("standalone"));
        assert_eq!(info_field(info, "role"), Some("master"));
        assert_eq!(info_field(info, "missing"), None);
        // "redis_version" must not match a longer field that starts the same way
        assert_eq!(
            info_field(
                "redis_version_extra:1\r\nredis_version:7.0.0",
                "redis_version"
            ),
            Some("7.0.0")
        );
        let dbs = keyspace(info);
        assert_eq!(dbs.len(), 2);
        assert_eq!((dbs[0].index, dbs[0].keys), (0, 12408));
        assert_eq!((dbs[1].index, dbs[1].keys), (3, 2));
        assert!(keyspace("").is_empty());
    }

    #[test]
    fn a_servers_error_is_shown_as_the_server_wrote_it() {
        use redis::{ErrorKind, ServerErrorKind};
        let known = RedisError::from((
            ErrorKind::Server(ServerErrorKind::CrossSlot),
            "CROSSSLOT",
            "Keys in request don't hash to the same slot".to_string(),
        ));
        let e = redis_error(&known);
        assert_eq!(e.code, ErrorCode::RedisRefused);
        assert_eq!(
            e.message,
            "CROSSSLOT Keys in request don't hash to the same slot"
        );
        let other = redis::make_extension_error(
            "WRONGTYPE".into(),
            Some("Operation against a key holding the wrong kind of value".into()),
        );
        assert_eq!(
            redis_error(&other).message,
            "WRONGTYPE Operation against a key holding the wrong kind of value"
        );
        let bare = redis::make_extension_error("LOADING".into(), None);
        assert_eq!(redis_error(&bare).message, "LOADING");
    }

    #[test]
    fn the_backoff_is_one_two_four_eight_fifteen() {
        assert_eq!(BACKOFF, [1, 2, 4, 8, 15]);
    }

    #[tokio::test]
    async fn closing_an_unknown_connection_is_harmless_and_commands_on_it_are_refused() {
        let m = RedisManager::default();
        m.close("nope");
        m.close_all();
        assert_eq!(m.live_count(), 0);
        assert_eq!(
            m.handle("nope").await.err().map(|e| e.code),
            Some(ErrorCode::NotFound)
        );
    }
}
