//! Port forwarding (F06; S09, S10). Rules live in `port_forwards`. A running rule is one tokio task
//! that owns an SSH connection (through any jump hosts), serves the forward, and reconnects with
//! backoff when the connection drops. Local (`-L`), remote (`-R`) and SOCKS5 (`-D`) are supported.

mod run;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;

use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::{PromptAnswer, Prompts, SessionEvent};
use crate::state::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    /// `-L`: a port on this computer reaches a destination on the server's network.
    Local,
    /// `-R`: a port on the server reaches a destination on this computer.
    Remote,
    /// `-D`: a SOCKS5 proxy on this computer, exiting on the server.
    Dynamic,
}

impl ForwardKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Dynamic => "dynamic",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "local" => Some(Self::Local),
            "remote" => Some(Self::Remote),
            "dynamic" => Some(Self::Dynamic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    Stopped,
    Starting,
    Active,
    Reconnecting,
    Error,
}

/// What the UI shows for a rule's live state. Also the payload of the `TunnelChanged` event.
#[derive(Debug, Clone, Serialize, Type)]
pub struct TunnelStatus {
    pub id: String,
    pub state: TunnelState,
    pub connections: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ForwardRule {
    pub id: String,
    pub host_id: String,
    pub label: String,
    pub kind: ForwardKind,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_address: Option<String>,
    pub dest_port: Option<u16>,
    pub autostart: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ForwardSummary {
    pub id: String,
    pub host_id: String,
    pub host_label: String,
    pub label: String,
    pub kind: ForwardKind,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_address: Option<String>,
    pub dest_port: Option<u16>,
    pub autostart: bool,
    pub state: TunnelState,
    pub connections: u32,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Type)]
pub struct ForwardInput {
    pub id: Option<String>,
    pub host_id: String,
    pub label: String,
    pub kind: ForwardKind,
    pub bind_address: String,
    pub bind_port: u16,
    pub dest_address: Option<String>,
    pub dest_port: Option<u16>,
    pub autostart: bool,
}

// ------------------------------------------------------------------ rules (database)

fn row_to_rule(r: &rusqlite::Row) -> rusqlite::Result<ForwardRule> {
    let kind: String = r.get(3)?;
    Ok(ForwardRule {
        id: r.get(0)?,
        host_id: r.get(1)?,
        label: r.get(2)?,
        kind: ForwardKind::parse(&kind).unwrap_or(ForwardKind::Local),
        bind_address: r.get(4)?,
        bind_port: r.get::<_, i64>(5)? as u16,
        dest_address: r.get(6)?,
        dest_port: r.get::<_, Option<i64>>(7)?.map(|p| p as u16),
        autostart: r.get(8)?,
    })
}

const SELECT_RULE: &str = "SELECT id, host_id, label, kind, bind_address, bind_port, dest_address, dest_port, autostart FROM port_forwards";

pub fn list_rules(conn: &Connection, vault_id: &str) -> AppResult<Vec<(ForwardRule, String)>> {
    let mut st = conn.prepare("SELECT f.id, f.host_id, f.label, f.kind, f.bind_address, f.bind_port, f.dest_address, f.dest_port, f.autostart, h.label
         FROM port_forwards f JOIN hosts h ON h.id = f.host_id WHERE f.vault_id = ?1 ORDER BY f.label COLLATE NOCASE")?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok((row_to_rule(r)?, r.get::<_, String>(9)?))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn get_rule(conn: &Connection, vault_id: &str, id: &str) -> AppResult<ForwardRule> {
    conn.query_row(
        &format!("{SELECT_RULE} WHERE id = ?1 AND vault_id = ?2"),
        params![id, vault_id],
        row_to_rule,
    )
    .optional()?
    .ok_or_else(|| AppError::not_found("That rule no longer exists."))
}

pub fn autostart_rules(conn: &Connection, vault_id: &str) -> AppResult<Vec<ForwardRule>> {
    let mut st = conn.prepare(&format!(
        "{SELECT_RULE} WHERE vault_id = ?1 AND autostart = 1"
    ))?;
    let rows = st
        .query_map([vault_id], row_to_rule)?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn valid_bind_address(a: &str) -> bool {
    a == "localhost" || a.parse::<IpAddr>().is_ok()
}

/// Validates and writes a rule. Two local or dynamic rules can't listen on the same address and port.
pub fn save_rule(conn: &Connection, vault_id: &str, i: &ForwardInput) -> AppResult<String> {
    let label = i.label.trim();
    if label.is_empty() {
        return Err(AppError::validation("Give the rule a name."));
    }
    let bind = i.bind_address.trim();
    if !valid_bind_address(bind) {
        return Err(AppError::validation(
            "Use an IP address like 127.0.0.1 for where to listen.",
        ));
    }
    if i.bind_port == 0 {
        return Err(AppError::validation("Port must be between 1 and 65535."));
    }
    let (dest_address, dest_port) = match i.kind {
        ForwardKind::Dynamic => (None, None),
        _ => {
            let a = i
                .dest_address
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty());
            match (a, i.dest_port.filter(|p| *p != 0)) {
                (Some(a), Some(p)) if !a.chars().any(char::is_whitespace) => {
                    (Some(a.to_string()), Some(p))
                }
                _ => {
                    return Err(AppError::validation(
                        "Enter where the connection should go, with a port.",
                    ))
                }
            }
        }
    };
    let host_ok: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM hosts WHERE id = ?1 AND vault_id = ?2)",
        params![i.host_id, vault_id],
        |r| r.get(0),
    )?;
    if !host_ok {
        return Err(AppError::validation("Choose a server for this rule."));
    }
    if i.kind != ForwardKind::Remote {
        let clash: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM port_forwards WHERE vault_id = ?1 AND bind_address = ?2 AND bind_port = ?3 AND kind IN ('local','dynamic') AND id IS NOT ?4)",
            params![vault_id, bind, i.bind_port, i.id],
            |r| r.get(0),
        )?;
        if clash {
            return Err(AppError::validation(format!(
                "Another rule already listens on {bind}:{}. Pick a different port.",
                i.bind_port
            )));
        }
    }
    let now = now_ms();
    match &i.id {
        None => {
            let id = uuid::Uuid::now_v7().to_string();
            conn.execute(
                "INSERT INTO port_forwards (id, vault_id, host_id, label, kind, bind_address, bind_port, dest_address, dest_port, autostart, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                params![id, vault_id, i.host_id, label, i.kind.as_str(), bind, i.bind_port, dest_address, dest_port, i.autostart, now],
            )?;
            Ok(id)
        }
        Some(id) => {
            let n = conn.execute(
                "UPDATE port_forwards SET host_id = ?1, label = ?2, kind = ?3, bind_address = ?4, bind_port = ?5, dest_address = ?6, dest_port = ?7, autostart = ?8, updated_at = ?9
                 WHERE id = ?10 AND vault_id = ?11",
                params![i.host_id, label, i.kind.as_str(), bind, i.bind_port, dest_address, dest_port, i.autostart, now, id, vault_id],
            )?;
            if n == 0 {
                return Err(AppError::not_found("That rule no longer exists."));
            }
            Ok(id.clone())
        }
    }
}

pub fn delete_rule(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM port_forwards WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That rule no longer exists."));
    }
    Ok(())
}

// ------------------------------------------------------------------ plumbing shared with ssh

/// For a remote forward: where the connections the server sends us are delivered.
pub struct RemoteHook {
    pub dest_address: String,
    pub dest_port: u16,
    pub open: Arc<AtomicU32>,
}

/// Copies bytes both ways until either side closes, counting the connection while it is open.
pub async fn pipe<A, B>(mut a: A, mut b: B, open: Arc<AtomicU32>)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    open.fetch_add(1, Ordering::Relaxed);
    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
    open.fetch_sub(1, Ordering::Relaxed);
}

// ------------------------------------------------------------------ running tunnels

pub type Emit = Arc<dyn Fn(TunnelStatus) + Send + Sync>;
pub type UserEvents = Box<dyn Fn(SessionEvent) + Send + Sync>;

struct Running {
    host_id: String,
    cancel: watch::Sender<bool>,
    status: Arc<Mutex<TunnelStatus>>,
    prompts: Arc<Prompts>,
    emit: Emit,
}

#[derive(Default)]
pub struct TunnelManager {
    running: Mutex<HashMap<String, Running>>,
}

fn is_live(s: TunnelState) -> bool {
    matches!(
        s,
        TunnelState::Starting | TunnelState::Active | TunnelState::Reconnecting
    )
}

impl TunnelManager {
    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, Running>>> {
        self.running
            .lock()
            .map_err(|_| AppError::internal("tunnel lock poisoned"))
    }

    pub fn status(&self, id: &str) -> Option<TunnelStatus> {
        let map = self.lock().ok()?;
        let r = map.get(id)?;
        r.status.lock().ok().map(|s| s.clone())
    }

    /// True while a rule on this host is starting, active or reconnecting (blocks deleting the host).
    pub fn running_for_host(&self, host_id: &str) -> bool {
        self.lock().is_ok_and(|m| {
            m.values()
                .any(|r| r.host_id == host_id && r.status.lock().is_ok_and(|s| is_live(s.state)))
        })
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.status(id).is_some_and(|s| is_live(s.state))
    }

    /// Starts a rule. `user_events` carries host-key and password questions to the UI; with `None`
    /// (autostart) nobody is there to answer, so such questions are refused and the rule reports an error.
    pub fn start(
        &self,
        store: Store,
        rule: ForwardRule,
        user_events: Option<UserEvents>,
        emit: Emit,
    ) -> AppResult<()> {
        let mut map = self.lock()?;
        if map
            .get(&rule.id)
            .is_some_and(|r| r.status.lock().is_ok_and(|s| is_live(s.state)))
        {
            return Ok(()); // starting twice is harmless
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let status = Arc::new(Mutex::new(TunnelStatus {
            id: rule.id.clone(),
            state: TunnelState::Starting,
            connections: 0,
            error: None,
        }));
        let prompts = Arc::new(if user_events.is_some() {
            Prompts::default()
        } else {
            Prompts::non_interactive()
        });
        map.insert(
            rule.id.clone(),
            Running {
                host_id: rule.host_id.clone(),
                cancel: cancel_tx,
                status: Arc::clone(&status),
                prompts: Arc::clone(&prompts),
                emit: Arc::clone(&emit),
            },
        );
        tauri::async_runtime::spawn(run::run_tunnel(
            store,
            rule,
            user_events,
            prompts,
            status,
            cancel_rx,
            emit,
        ));
        Ok(())
    }

    pub fn stop(&self, id: &str) {
        let removed = self.lock().ok().and_then(|mut m| m.remove(id));
        if let Some(r) = removed {
            let _ = r.cancel.send(true);
            r.prompts.cancel_all_pub();
            (r.emit)(TunnelStatus {
                id: id.to_string(),
                state: TunnelState::Stopped,
                connections: 0,
                error: None,
            });
        }
    }

    /// Vault locked: every tunnel ends.
    pub fn stop_all(&self) {
        let ids: Vec<String> = self
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            self.stop(&id);
        }
    }

    pub fn respond(&self, id: &str, prompt_id: &str, answer: PromptAnswer) -> AppResult<()> {
        let prompts = {
            let map = self.lock()?;
            Arc::clone(
                &map.get(id)
                    .ok_or_else(|| AppError::not_found("That rule isn't starting."))?
                    .prompts,
            )
        };
        prompts.answer(prompt_id, answer)
    }
}

/// A short description of a failure that retrying can't fix.
pub fn permanent(code: ErrorCode) -> bool {
    !matches!(code, ErrorCode::HostUnreachable | ErrorCode::KexFailed)
}

#[cfg(test)]
mod tests;

impl ForwardSummary {
    pub fn new(rule: ForwardRule, host_label: String, live: Option<TunnelStatus>) -> Self {
        let (state, connections, error) = live.map_or((TunnelState::Stopped, 0, None), |s| {
            (s.state, s.connections, s.error)
        });
        ForwardSummary {
            id: rule.id,
            host_id: rule.host_id,
            host_label,
            label: rule.label,
            kind: rule.kind,
            bind_address: rule.bind_address,
            bind_port: rule.bind_port,
            dest_address: rule.dest_address,
            dest_port: rule.dest_port,
            autostart: rule.autostart,
            state,
            connections,
            error,
        }
    }
}
