//! SQL databases (D2 onward; spec section 3). `pg` is PostgreSQL; MySQL and MariaDB join in D7. This file is the shared
//! part: opening a database's connection through its route (with the host's questions and the steps shown in S33's
//! "Connecting" card), and keeping the open connections by id so a tab's commands find theirs.

pub mod pg;
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

use pg::{PgConn, SqlServerInfo};

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

/// One whole attempt: host sign-in (if any), the stream, the database sign-in. The SSH connection is returned and
/// must stay alive as long as the database connection does.
pub async fn connect(
    store: Store,
    cfg: Arc<SqlConfig>,
    prompts: Arc<Prompts>,
    emit: Emit,
    on_close: impl FnOnce(Option<String>) + Send + 'static,
) -> AppResult<(Arc<PgConn>, SqlServerInfo, Option<Arc<Connected>>)> {
    if cfg.engine != Engine::Postgres {
        return Err(AppError::validation(format!(
            "Opening {} databases arrives in a later update.",
            cfg.engine.label()
        )));
    }
    let (steps, hops) = plan(&store, &cfg).await?;
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
    let stream = match &ssh {
        Some(ssh) => transport::through_host_for(cfg.engine, ssh, &target).await,
        None => transport::direct_for(cfg.engine, &target).await,
    };
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
    step(&emit, reach + 1, RedisStepState::Active, None);
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
    let (conn, info) = match PgConn::connect(stream, login, on_close).await {
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
    Ok((Arc::new(conn), info, ssh))
}

/// "Test connection" for a SQL database: the whole sign-in, then closed. A host that needs an answer from the user
/// is not asked here.
pub async fn test(store: &Store, cfg: SqlConfig) -> AppResult<SqlServerInfo> {
    let prompts = Arc::new(Prompts::non_interactive());
    let quiet: Emit = Arc::new(|_| {});
    let name = cfg.name.clone();
    let route = cfg.route;
    let (_conn, info, _ssh) = connect(store.clone(), Arc::new(cfg), prompts, quiet, |_| {})
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
        Ok((conn, info, ssh)) => {
            *live.conn.lock().await = Some(conn);
            emit(DbEvent::Ready { info });
            // Hold the SSH connection until the tab closes.
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

type Opened = (Arc<PgConn>, SqlServerInfo, Option<Arc<Connected>>);

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
    prompts: Arc<Prompts>,
    cancel: watch::Sender<bool>,
    conn: tokio::sync::Mutex<Option<Arc<PgConn>>>,
}

/// The open SQL connections, by id. One per tab; lock closes them all.
#[derive(Default)]
pub struct SqlManager {
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
            prompts: Arc::new(Prompts::default()),
            cancel,
            conn: tokio::sync::Mutex::new(None),
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
        let guard = live.conn.lock().await;
        guard.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::DbUnreachable,
                "This database isn't connected right now.",
            )
        })
    }

    pub fn close(&self, conn_id: &str) {
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
