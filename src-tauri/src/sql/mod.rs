//! SQL databases (D2 onward; spec section 3). `pg` is PostgreSQL; MySQL and MariaDB join in D7. This file is the shared
//! part: opening a database's connection through its route (with the host's questions and the steps shown in S33's
//! "Connecting" card), and keeping the open connections by id so a tab's commands find theirs.

pub mod activity;
pub mod backup;
pub mod classify;
pub mod explain;
pub mod diagram;
pub mod grid;
pub mod mydb;
pub mod myextra;
pub mod mytransfer;
pub mod mysql;
pub mod pg;
pub mod structure;
pub mod transfer;
pub mod run;
#[cfg(test)]
mod tests;
pub mod values;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use specta::Type;
use tokio::sync::watch;
use zeroize::Zeroizing;

use crate::dbs::store as dbstore;
use crate::dbs::transport::{self, Target, TlsMode};
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::hosts;
use crate::redis::conn::{
    sign_in_to_host, Emit as RedisEmit, RedisEvent, RedisPlanStep, RedisStepState,
};
use crate::redis::store::RedisRoute;
use crate::ssh::{Connected, PromptAnswer, Prompts, SessionEvent};
use crate::state::Store;

use pg::{PgConn, Reopen, SqlServerInfo};

/// What a connecting or open SQL tab hears. The first four are the same machinery as a Redis tab's.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DbEvent {
    Plan {
        steps: Vec<RedisPlanStep>,
    },
    Step {
        index: u32,
        state: RedisStepState,
        detail: Option<String>,
    },
    /// A question or a note from the SSH sign-in of a hop. Answer prompts with `db_respond`.
    Host {
        event: SessionEvent,
    },
    Ready {
        info: SqlServerInfo,
    },
    Failed {
        code: ErrorCode,
        message: String,
    },
    /// The server closed the connection (or the host's SSH did).
    Closed,
}

pub type Emit = Arc<dyn Fn(DbEvent) + Send + Sync>;

/// Everything needed to connect, from a saved database or from the editor's form (Test connection).
pub struct SqlConfig {
    pub name: String,
    pub engine: Engine,
    pub route: RedisRoute,
    pub host_id: Option<String>,
    pub address: String,
    pub port: u16,
    pub database: Option<String>,
    pub user: String,
    pub password: Option<Zeroizing<String>>,
    pub tls: TlsMode,
    pub server_name: Option<String>,
    /// MongoDB: the database that holds the user (`authSource`).
    pub auth_source: Option<String>,
}

/// Reads a saved database, password included, for opening it.
pub async fn load(store: &Store, db_id: &str) -> AppResult<SqlConfig> {
    let id = db_id.to_string();
    store
        .with_db_session(move |c, s| {
            let d = dbstore::get(c, &s.vault_id, &id)?;
            let password = dbstore::password(c, s, &id)?;
            dbstore::touch_opened(c, &s.vault_id, &id)?;
            Ok(SqlConfig {
                name: d.name,
                engine: d.engine,
                route: d.route,
                host_id: d.host_id,
                address: d.address,
                port: d.port.unwrap_or_else(|| d.engine.default_port()),
                database: d.default_database,
                user: d.username.unwrap_or_default(),
                password,
                tls: TlsMode::parse(&d.tls).ok_or_else(|| {
                    AppError::validation(
                        "This database's TLS setting isn't one of off, require or verify.",
                    )
                })?,
                server_name: d.tls_server_name,
                auth_source: d.options.auth_source,
            })
        })
        .await
}

/// The steps shown before anything is tried: one per host in the chain, then the database.
async fn plan(store: &Store, cfg: &SqlConfig) -> AppResult<(Vec<RedisPlanStep>, usize)> {
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
    let target = Target {
        address: cfg.address.clone(),
        port: cfg.port,
        tls: false,
        tls_server_name: None,
    };
    steps.push(RedisPlanStep {
        label: format!("Reach {} at {}", cfg.engine.label(), target.display()),
        detail: (cfg.tls != TlsMode::Off).then(|| "with TLS".to_string()),
    });
    steps.push(RedisPlanStep {
        label: if cfg.user.is_empty() {
            format!("Sign in to {}", cfg.engine.label())
        } else {
            format!("Sign in as {}", cfg.user)
        },
        detail: None,
    });
    Ok((steps, hops))
}

fn step(emit: &Emit, index: u32, state: RedisStepState, detail: Option<String>) {
    emit(DbEvent::Step {
        index,
        state,
        detail,
    });
}

/// The way to the server: the steps shown, the host sign-in (if any), and the first stream to the database. Shared
/// by every engine; the engine's own sign-in comes after it.
async fn route(
    store: &Store,
    cfg: &Arc<SqlConfig>,
    prompts: &Arc<Prompts>,
    emit: &Emit,
) -> AppResult<(Option<Arc<Connected>>, Reopen, transport::BoxStream, u32)> {
    let store = store.clone();
    let (prompts, emit) = (Arc::clone(prompts), Arc::clone(emit));
    let (steps, hops) = plan(&store, cfg).await?;
    emit(DbEvent::Plan { steps });
    // The host sign-in speaks Redis' event type; only its steps and questions matter here.
    let redis_emit: RedisEmit = {
        let emit = Arc::clone(&emit);
        Arc::new(move |e| match e {
            RedisEvent::Step {
                index,
                state,
                detail,
            } => emit(DbEvent::Step {
                index,
                state,
                detail,
            }),
            RedisEvent::Host { event } => emit(DbEvent::Host { event }),
            _ => {}
        })
    };
    let ssh = match (&cfg.host_id, cfg.route) {
        (Some(host_id), RedisRoute::Host) => Some(Arc::new(
            sign_in_to_host(&store, host_id, hops, &prompts, &redis_emit).await?,
        )),
        _ => None,
    };
    let reach = hops as u32;
    step(&emit, reach, RedisStepState::Active, None);
    let target = Target {
        address: cfg.address.clone(),
        port: cfg.port,
        tls: false,
        tls_server_name: None,
    };
    // The way to the server, kept: a cancel request and each editor's own connection take it again.
    let reopen: Reopen = {
        let (ssh, target, engine) = (ssh.clone(), target.clone(), cfg.engine);
        Arc::new(move || {
            let (ssh, target) = (ssh.clone(), target.clone());
            Box::pin(async move {
                match &ssh {
                    Some(ssh) => transport::through_host_for(engine, ssh, &target).await,
                    None => transport::direct_for(engine, &target).await,
                }
            })
        })
    };
    let stream = (reopen)().await;
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            step(
                &emit,
                reach,
                RedisStepState::Failed,
                Some(e.message.clone()),
            );
            return Err(e);
        }
    };
    step(&emit, reach, RedisStepState::Done, None);
    Ok((ssh, reopen, stream, reach))
}

/// One whole attempt: host sign-in (if any), the stream, the database sign-in. The SSH connection is returned and
/// must stay alive as long as the database connection does.
pub async fn connect(
    store: Store,
    cfg: Arc<SqlConfig>,
    prompts: Arc<Prompts>,
    emit: Emit,
    on_close: impl FnOnce(Option<String>) + Send + 'static,
) -> AppResult<Opened> {
    let (ssh, reopen, stream, reach) = route(&store, &cfg, &prompts, &emit).await?;
    step(&emit, reach + 1, RedisStepState::Active, None);
    if matches!(cfg.engine, Engine::Mysql | Engine::Mariadb) {
        return connect_my(&cfg, stream, reopen, ssh, &emit, reach).await;
    }
    if cfg.engine == Engine::Mongodb {
        return connect_mongo(&cfg, stream, reopen, ssh, &emit, reach).await;
    }
    let server_name = cfg
        .server_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| cfg.address.clone());
    let login = pg::Login {
        user: cfg.user.clone(),
        password: cfg.password.clone(),
        database: cfg.database.clone(),
        tls: cfg.tls,
        server_name,
    };
    let (conn, info) = match PgConn::connect(stream, login.clone(), Arc::clone(&reopen), on_close).await {
        Ok(v) => v,
        Err(e) => {
            step(
                &emit,
                reach + 1,
                RedisStepState::Failed,
                Some(e.message.clone()),
            );
            return Err(e);
        }
    };
    step(&emit, reach + 1, RedisStepState::Done, None);
    let opener: Opener = Arc::new(move || {
        let (reopen, login) = (Arc::clone(&reopen), login.clone());
        Box::pin(async move {
            let stream = (reopen)().await?;
            let (conn, _) = PgConn::connect(stream, login, Arc::clone(&reopen), |_| {}).await?;
            Ok(EditorConn::Pg(Box::new(conn)))
        })
    });
    Ok(Opened { db: Db::Pg(Arc::new(conn)), info, ssh, opener })
}

/// The MySQL and MariaDB sign-in on the stream the route gave, and the way to open an editor tab's own connection.
async fn connect_my(
    cfg: &Arc<SqlConfig>,
    stream: transport::BoxStream,
    reopen: Reopen,
    ssh: Option<Arc<Connected>>,
    emit: &Emit,
    reach: u32,
) -> AppResult<Opened> {
    let server_name = cfg.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| cfg.address.clone());
    let login = mysql::MyLogin { user: cfg.user.clone(), password: cfg.password.clone(), database: cfg.database.clone(), tls: cfg.tls, server_name };
    let (conn, info) = match mysql::MyConn::connect(stream, login.clone(), Arc::clone(&reopen)).await {
        Ok(v) => v,
        Err(e) => {
            step(emit, reach + 1, RedisStepState::Failed, Some(e.message.clone()));
            return Err(e);
        }
    };
    step(emit, reach + 1, RedisStepState::Done, None);
    let opener: Opener = Arc::new(move || {
        let (reopen, login) = (Arc::clone(&reopen), login.clone());
        Box::pin(async move {
            let stream = (reopen)().await?;
            let (conn, _) = mysql::MyConn::connect(stream, login, Arc::clone(&reopen)).await?;
            Ok(EditorConn::My(Box::new(mydb::MyEditor::new(conn))))
        })
    });
    Ok(Opened { db: Db::My(Arc::new(mydb::MyDb::new(conn))), info, ssh, opener })
}

/// The MongoDB sign-in on the stream the route gave. Documents need no editor connections, so the opener refuses.
async fn connect_mongo(
    cfg: &Arc<SqlConfig>,
    stream: transport::BoxStream,
    reopen: Reopen,
    ssh: Option<Arc<Connected>>,
    emit: &Emit,
    reach: u32,
) -> AppResult<Opened> {
    let server_name = cfg.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| cfg.address.clone());
    let login = crate::mongo::conn::MongoLogin { user: cfg.user.clone(), password: cfg.password.clone(), auth_db: cfg.auth_source.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| "admin".into()), tls: cfg.tls, server_name };
    let (conn, info) = match crate::mongo::conn::MongoConn::connect(stream, login, reopen).await {
        Ok(v) => v,
        Err(e) => {
            step(emit, reach + 1, RedisStepState::Failed, Some(e.message.clone()));
            return Err(e);
        }
    };
    step(emit, reach + 1, RedisStepState::Done, None);
    let opener: Opener = Arc::new(|| Box::pin(async { Err(AppError::validation("MongoDB tabs don't open editor connections.")) }));
    let version = match &info.replica_set {
        Some(set) => format!("{} (replica set {set}{})", info.version, if info.is_primary { ", primary" } else { "" }),
        None => info.version.clone(),
    };
    let info = SqlServerInfo { engine: "mongodb".into(), version, database: cfg.database.clone().unwrap_or_else(|| "admin".into()), user: info.user, latency_ms: info.latency_ms };
    Ok(Opened { db: Db::Mongo(Arc::new(conn)), info, ssh, opener })
}

/// "Test connection" for a SQL database: the whole sign-in, then closed. A host that needs an answer from the user
/// is not asked here.
pub async fn test(store: &Store, cfg: SqlConfig) -> AppResult<SqlServerInfo> {
    if matches!(cfg.engine, Engine::Mysql | Engine::Mariadb) {
        return test_my(store, cfg).await;
    }
    if cfg.engine == Engine::Mongodb {
        return test_mongo(store, cfg).await;
    }
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let name = cfg.name.clone();
    let route = cfg.route;
    let Opened { info, .. } = connect(store.clone(), Arc::new(cfg), prompts, quiet, |_| {})
        .await
        .map_err(|e| {
            if e.code == ErrorCode::AuthFailed && route == RedisRoute::Host {
                AppError::new(ErrorCode::AuthFailed, format!("The host for {name} needs an answer from you (a new host key or a password). Open a terminal on it once, then test again."))
            } else {
                e
            }
        })?;
    Ok(info)
}

/// One tab's connection from start to close. A named function (not a closure in `spawn`) so the compiler can see it is Send.
async fn run(
    store: Store,
    db_id: String,
    live: Arc<Live>,
    emit: Emit,
    mut cancel: watch::Receiver<bool>,
) {
    let closed = Arc::new(AtomicBool::new(false));
    let result = tokio::select! {
        r = attempt(&store, &db_id, &live, &emit, &closed) => r,
        _ = cancel.changed() => return,
    };
    match result {
        Ok(Opened { db, info, ssh, opener }) => {
            *live.opener.lock().await = Some(opener);
            match db {
                Db::Pg(c) => *live.conn.lock().await = Some(c),
                Db::My(c) => *live.my.lock().await = Some(c),
                Db::Mongo(c) => *live.mongo.lock().await = Some(c),
            }
            emit(DbEvent::Ready { info });
            // Hold the SSH connection until the tab closes (backups use it too).
            *live.ssh.lock().await = ssh.clone();
            let _ssh = ssh;
            let _ = cancel.changed().await;
            closed.store(true, Ordering::SeqCst);
        }
        Err(e) => emit(DbEvent::Failed {
            code: e.code,
            message: e.message,
        }),
    }
}

/// Opens one more connection to the same server by the same route (an editor tab's own).
type Opener = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = AppResult<EditorConn>> + Send>,
        > + Send
        + Sync,
>;

/// What a successful sign-in gives the tab: the engine's connection for browsing, the server's answer, the host's
/// SSH connection (if the route has one) and the way to open an editor connection.
pub struct Opened {
    db: Db,
    info: SqlServerInfo,
    ssh: Option<Arc<Connected>>,
    opener: Opener,
}

enum Db {
    Pg(Arc<PgConn>),
    My(Arc<mydb::MyDb>),
    Mongo(Arc<crate::mongo::conn::MongoConn>),
}

async fn attempt(
    store: &Store,
    db_id: &str,
    live: &Arc<Live>,
    emit: &Emit,
    closed: &Arc<AtomicBool>,
) -> AppResult<Opened> {
    let cfg = Arc::new(load(store, db_id).await?);
    let (e, f) = (Arc::clone(emit), Arc::clone(closed));
    connect(
        store.clone(),
        cfg,
        Arc::clone(&live.prompts),
        Arc::clone(emit),
        move |_| {
            // The connection ended on its own (the server hung up, or the SSH connection under it did).
            if !f.swap(true, Ordering::SeqCst) {
                e(DbEvent::Closed);
            }
        },
    )
    .await
}

struct Live {
    db_id: String,
    prompts: Arc<Prompts>,
    cancel: watch::Sender<bool>,
    conn: tokio::sync::Mutex<Option<Arc<PgConn>>>,
    /// MySQL and MariaDB tabs hold this instead.
    my: tokio::sync::Mutex<Option<Arc<mydb::MyDb>>>,
    /// MongoDB tabs hold this.
    mongo: tokio::sync::Mutex<Option<Arc<crate::mongo::conn::MongoConn>>>,
    opener: tokio::sync::Mutex<Option<Opener>>,
    /// The SSH connection to the route's last host, for tools that run there (backup, restore).
    ssh: tokio::sync::Mutex<Option<Arc<Connected>>>,
    /// One connection per editor tab, so a transaction in one never touches another (spec section 3).
    editors: tokio::sync::Mutex<HashMap<String, Arc<Editor>>>,
}

/// An editor tab's own connection, whichever engine: the same calls, answered by PostgreSQL or by MySQL.
pub enum EditorConn {
    Pg(Box<PgConn>),
    My(Box<mydb::MyEditor>),
}

const MY_LATER: &str = "This isn't available for MySQL and MariaDB yet; it arrives in a later update.";

impl EditorConn {
    pub async fn run_script(&self, pieces: &[classify::Piece], opts: &run::RunOpts, cancel: &AtomicBool, allow_write: bool, emit: &(dyn Fn(run::SqlRunEvent) + Sync)) -> AppResult<()> {
        match self {
            EditorConn::Pg(c) => c.run_script(pieces, opts, cancel, allow_write, emit).await,
            EditorConn::My(c) => c.run_script(pieces, opts, cancel, allow_write, emit).await,
        }
    }
    pub async fn preview(&self, pieces: &[classify::Piece]) -> AppResult<Vec<Option<String>>> {
        match self {
            EditorConn::Pg(c) => c.preview(pieces).await,
            EditorConn::My(c) => c.preview(pieces).await,
        }
    }
    pub async fn cancel_running(&self) -> AppResult<()> {
        match self {
            EditorConn::Pg(c) => c.cancel_running().await,
            EditorConn::My(c) => c.cancel_running().await,
        }
    }
    pub fn has_txn(&self) -> bool {
        match self {
            EditorConn::Pg(c) => c.has_txn(),
            EditorConn::My(c) => c.has_txn(),
        }
    }
    pub async fn txn_begin(&self) -> AppResult<()> {
        match self {
            EditorConn::Pg(c) => c.txn_begin().await,
            EditorConn::My(c) => c.txn_begin().await,
        }
    }
    pub async fn txn_end(&self, commit: bool) -> AppResult<()> {
        match self {
            EditorConn::Pg(c) => c.txn_end(commit).await,
            EditorConn::My(c) => c.txn_end(commit).await,
        }
    }
    pub async fn txn_state(&self) -> AppResult<run::TxnState> {
        match self {
            EditorConn::Pg(c) => c.txn_state().await,
            EditorConn::My(c) => c.txn_state().await,
        }
    }
    pub async fn explain(&self, text: &str, analyze: bool, timeout_ms: u32) -> AppResult<explain::ExplainPlan> {
        match self {
            EditorConn::Pg(c) => c.explain(text, analyze, timeout_ms).await,
            EditorConn::My(c) => c.explain(text, analyze, timeout_ms).await,
        }
    }
}

/// An editor tab's connection and its stop flag.
pub struct Editor {
    pub conn: EditorConn,
    /// Set by Stop; a run clears it when it starts.
    pub stop: AtomicBool,
    /// A run is going: only then does Stop send a cancel to the server.
    pub running: AtomicBool,
}

/// The open SQL connections, by id. One per tab; lock closes them all.
#[derive(Default)]
pub struct SqlManager {
    /// Scripts held for the gate's answer, by id (bound to their connection).
    pub pending: crate::dbs::gate::PendingStore<run::HeldRun>,
    /// Grid edits held for the commit review.
    /// Structure changes held for the review.
    pub structure_pending: crate::dbs::gate::PendingStore<Vec<String>>,
    /// Session cancels and ends held for the gate's answer: (pid, end).
    pub session_pending: crate::dbs::gate::PendingStore<(i32, bool, String)>,
    /// Files chosen for import, by id: the window only ever holds the id and the name, never the path.
    pub import_files: Mutex<HashMap<String, (String, std::path::PathBuf)>>,
    pub import_pending: crate::dbs::gate::PendingStore<transfer::ImportPlan>,
    pub restore_pending: crate::dbs::gate::PendingStore<std::path::PathBuf>,
    /// Running exports, backups and restores by id, so Stop can reach them.
    pub transfers: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub grid_pending: crate::dbs::gate::PendingStore<Vec<grid::GridStatement>>,
    /// MongoDB document writes held for their confirmation.
    pub mongo_pending: crate::dbs::gate::PendingStore<crate::mongo::docs::MgWrite>,
    conns: Mutex<HashMap<String, Arc<Live>>>,
}

impl SqlManager {
    fn get(&self, conn_id: &str) -> AppResult<Arc<Live>> {
        self.conns
            .lock()
            .map_err(|_| AppError::internal("sql lock poisoned"))?
            .get(conn_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("That database connection is closed."))
    }

    /// Starts connecting and returns the connection's id at once. Everything else arrives through `emit`.
    pub fn open(&self, store: Store, db_id: String, emit: Emit) -> AppResult<String> {
        let conn_id = uuid::Uuid::now_v7().to_string();
        let (cancel, cancel_rx) = watch::channel(false);
        let live = Arc::new(Live {
            db_id: db_id.clone(),
            opener: tokio::sync::Mutex::new(None),
            ssh: tokio::sync::Mutex::new(None),
            editors: tokio::sync::Mutex::new(HashMap::new()),
            prompts: Arc::new(Prompts::default()),
            cancel,
            conn: tokio::sync::Mutex::new(None),
            my: tokio::sync::Mutex::new(None),
            mongo: tokio::sync::Mutex::new(None),
        });
        self.conns
            .lock()
            .map_err(|_| AppError::internal("sql lock poisoned"))?
            .insert(conn_id.clone(), Arc::clone(&live));
        tokio::spawn(run(store, db_id, live, emit, cancel_rx));
        Ok(conn_id)
    }

    pub fn respond(&self, conn_id: &str, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        self.get(conn_id)?.prompts.answer(prompt_id, answer)
    }

    /// The connection for one command. Errors while the tab is still connecting.
    pub async fn pg(&self, conn_id: &str) -> AppResult<Arc<PgConn>> {
        let live = self.get(conn_id)?;
        if live.my.lock().await.is_some() {
            return Err(AppError::validation(MY_LATER));
        }
        let guard = live.conn.lock().await;
        guard.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::DbUnreachable,
                "This database isn't connected right now.",
            )
        })
    }

    /// The MongoDB connection of a tab. Errors for another engine's tab or while still connecting.
    pub async fn mongo(&self, conn_id: &str) -> AppResult<Arc<crate::mongo::conn::MongoConn>> {
        self.get(conn_id)?.mongo.lock().await.clone().ok_or_else(|| AppError::new(ErrorCode::DbUnreachable, "This database isn't connected right now."))
    }

    /// The MySQL or MariaDB connection of a tab, `None` for a PostgreSQL tab.
    pub async fn my(&self, conn_id: &str) -> AppResult<Option<Arc<mydb::MyDb>>> {
        Ok(self.get(conn_id)?.my.lock().await.clone())
    }

    /// The SQL the tab speaks (for splitting and classifying what is typed).
    pub async fn dialect(&self, conn_id: &str) -> AppResult<classify::Dialect> {
        Ok(if self.get(conn_id)?.my.lock().await.is_some() { classify::Dialect::My } else { classify::Dialect::Pg })
    }

    /// Manual transactions open right now, for the quit question (quitting rolls them back).
    pub fn open_transactions(&self) -> usize {
        let Ok(conns) = self.conns.lock() else { return 0 };
        conns
            .values()
            .map(|l| {
                l.editors
                    .try_lock()
                    .map(|e| e.values().filter(|x| x.conn.has_txn()).count())
                    .unwrap_or(0)
            })
            .sum()
    }

    /// Remembers a picked file for this connection and returns its id (at most eight are kept).
    pub fn put_file(&self, conn_id: &str, path: std::path::PathBuf) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        if let Ok(mut f) = self.import_files.lock() {
            if f.len() >= 8 {
                f.clear();
            }
            f.insert(id.clone(), (conn_id.to_string(), path));
        }
        id
    }

    /// The path behind a file id, only for the connection that picked it.
    pub fn file(&self, conn_id: &str, id: &str) -> AppResult<std::path::PathBuf> {
        self.import_files
            .lock()
            .ok()
            .and_then(|f| f.get(id).filter(|(c, _)| c == conn_id).map(|(_, p)| p.clone()))
            .ok_or_else(|| AppError::validation("Choose the file again."))
    }

    /// The SSH connection of a host route (`None` on a direct route).
    pub async fn ssh(&self, conn_id: &str) -> AppResult<Option<Arc<Connected>>> {
        Ok(self.get(conn_id)?.ssh.lock().await.clone())
    }

    /// Starts a transfer's stop flag; `stop_transfer` sets it.
    pub fn begin_transfer(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut m) = self.transfers.lock() {
            m.insert(id.to_string(), Arc::clone(&flag));
        }
        flag
    }
    pub fn end_transfer(&self, id: &str) {
        if let Ok(mut m) = self.transfers.lock() {
            m.remove(id);
        }
    }
    pub fn stop_transfer(&self, id: &str) {
        if let Ok(m) = self.transfers.lock() {
            if let Some(f) = m.get(id) {
                f.store(true, Ordering::SeqCst);
            }
        }
    }

    /// The saved database a connection was opened for.
    pub fn db_id(&self, conn_id: &str) -> AppResult<String> {
        Ok(self.get(conn_id)?.db_id.clone())
    }

    /// The editor tab's own connection, opened on first use.
    pub async fn editor(&self, conn_id: &str, editor_id: &str) -> AppResult<Arc<Editor>> {
        let live = self.get(conn_id)?;
        // Held while connecting, so two first runs of one editor don't open two connections.
        let mut editors = live.editors.lock().await;
        if let Some(e) = editors.get(editor_id) {
            return Ok(Arc::clone(e));
        }
        let opener = live.opener.lock().await.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::DbUnreachable,
                "This database isn't connected right now.",
            )
        })?;
        let conn = opener().await?;
        // Activity shows Dalan's own connections as such and never offers to kill them.
        if let (EditorConn::My(e), Some(my)) = (&conn, live.my.lock().await.as_ref()) {
            if let Ok(mut own) = my.own.lock() {
                own.push(e.conn.connection_id);
            }
        }
        let editor = Arc::new(Editor {
            conn,
            stop: AtomicBool::new(false),
            running: AtomicBool::new(false),
        });
        editors.insert(editor_id.to_string(), Arc::clone(&editor));
        Ok(editor)
    }

    /// The editor tab closed: its connection ends with it.
    pub async fn close_editor(&self, conn_id: &str, editor_id: &str) {
        if let Ok(live) = self.get(conn_id) {
            let gone = live.editors.lock().await.remove(editor_id);
            // The server may hand the connection's id to somebody else: it is no longer Dalan's.
            if let (Some(e), Some(my)) = (gone, live.my.lock().await.as_ref()) {
                if let EditorConn::My(c) = &e.conn {
                    if let Ok(mut own) = my.own.lock() {
                        own.retain(|id| *id != c.conn.connection_id);
                    }
                }
            }
        }
    }

    pub fn close(&self, conn_id: &str) {
        self.pending.drop_conn(conn_id);
        self.grid_pending.drop_conn(conn_id);
        self.mongo_pending.drop_conn(conn_id);
        self.session_pending.drop_conn(conn_id);
        self.import_pending.drop_conn(conn_id);
        if let Ok(mut f) = self.import_files.lock() {
            f.retain(|_, (c, _)| c != conn_id);
        }
        self.restore_pending.drop_conn(conn_id);
        self.structure_pending.drop_conn(conn_id);
        let live = self.conns.lock().ok().and_then(|mut m| m.remove(conn_id));
        if let Some(live) = live {
            live.prompts.cancel_all_pub();
            let _ = live.cancel.send(true);
        }
    }

    /// Lock (F10): every SQL connection ends.
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
}

/// The database as the gate needs it, read fresh for every run (the mode may have changed since the tab opened).
pub struct GateTarget {
    pub name: String,
    pub mode: crate::redis::store::RedisWriteMode,
    pub environment: String,
}

pub async fn gate_target(store: &Store, db_id: &str) -> AppResult<GateTarget> {
    let id = db_id.to_string();
    store
        .with_db_session(move |c, s| {
            let d = dbstore::get(c, &s.vault_id, &id)?;
            Ok(GateTarget {
                name: d.name,
                mode: d.write_mode,
                environment: format!("{:?}", d.environment).to_ascii_lowercase(),
            })
        })
        .await
}

/// "Test connection" for MySQL and MariaDB: the route, then our own client's whole sign-in, then closed.
async fn test_my(store: &Store, cfg: SqlConfig) -> AppResult<SqlServerInfo> {
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let (name, route_kind) = (cfg.name.clone(), cfg.route);
    let cfg = Arc::new(cfg);
    let (_ssh, reopen, stream, _) = route(store, &cfg, &prompts, &quiet).await.map_err(|e| {
        if e.code == ErrorCode::AuthFailed && route_kind == RedisRoute::Host {
            AppError::new(ErrorCode::AuthFailed, format!("The host for {name} needs an answer from you (a new host key or a password). Open a terminal on it once, then test again."))
        } else {
            e
        }
    })?;
    let server_name = cfg.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| cfg.address.clone());
    let login = mysql::MyLogin { user: cfg.user.clone(), password: cfg.password.clone(), database: cfg.database.clone(), tls: cfg.tls, server_name };
    let (conn, info) = mysql::MyConn::connect(stream, login, reopen).await?;
    conn.close().await;
    Ok(info)
}

/// "Test connection" for MongoDB: the route, then our own client's `hello` and sign-in, then closed. The first member of
/// a seed list is the one tried.
async fn test_mongo(store: &Store, cfg: SqlConfig) -> AppResult<SqlServerInfo> {
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let (name, route_kind) = (cfg.name.clone(), cfg.route);
    let cfg = Arc::new(cfg);
    let (_ssh, reopen, stream, _) = route(store, &cfg, &prompts, &quiet).await.map_err(|e| {
        if e.code == ErrorCode::AuthFailed && route_kind == RedisRoute::Host {
            AppError::new(ErrorCode::AuthFailed, format!("The host for {name} needs an answer from you (a new host key or a password). Open a terminal on it once, then test again."))
        } else {
            e
        }
    })?;
    let server_name = cfg.server_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| cfg.address.clone());
    let login = crate::mongo::conn::MongoLogin { user: cfg.user.clone(), password: cfg.password.clone(), auth_db: cfg.auth_source.clone().filter(|a| !a.is_empty()).unwrap_or_else(|| "admin".into()), tls: cfg.tls, server_name };
    let (conn, info) = crate::mongo::conn::MongoConn::connect(stream, login, reopen).await?;
    conn.close().await;
    Ok(SqlServerInfo {
        engine: "mongodb".into(),
        version: match &info.replica_set {
            Some(set) => format!("{} (replica set {set}{})", info.version, if info.is_primary { ", primary" } else { "" }),
            None => info.version,
        },
        database: cfg.database.clone().unwrap_or_else(|| "admin".into()),
        user: info.user,
        latency_ms: info.latency_ms,
    })
}
