//! SSH agent forwarding per host or group (S03, S04). When on, the server can ask this computer's
//! agent (`$SSH_AUTH_SOCK`) to sign for it, so you can hop on from there without copying keys.
//! Dalan's own vault keys are never offered: only a running system agent is forwarded.
//!
//! Off by default and per host on purpose: anyone with root on that server can use your agent
//! while you are connected. Stored in the profile (`agent_forwarding` NULL = inherit, 1 on, 0 off).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::groups;

/// `kind`: `inherit`, `on` or `off`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct AgentChoice {
    pub kind: String,
}

#[derive(Debug, Serialize, Type)]
pub struct AgentConfig {
    pub own: AgentChoice,
    /// What the groups above give (`None` = nothing set), and the group.
    pub inherited: Option<bool>,
    pub inherited_from: Option<String>,
    /// A system agent is running on this computer (`SSH_AUTH_SOCK` is set).
    pub available: bool,
}

pub fn available() -> bool {
    cfg!(unix) && std::env::var_os("SSH_AUTH_SOCK").is_some_and(|s| !s.is_empty())
}

/// Per level: the stored flag (`None` = inherit).
fn load(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
) -> AppResult<Vec<(Option<String>, Option<bool>)>> {
    groups::profile_chain(conn, vault_id, kind, id)?
        .into_iter()
        .map(|(label, pid)| {
            let v: Option<i64> = conn.query_row(
                "SELECT agent_forwarding FROM connection_profiles WHERE id = ?1",
                [pid],
                |r| r.get(0),
            )?;
            Ok((label, v.map(|v| v != 0)))
        })
        .collect()
}

pub fn get(conn: &Connection, vault_id: &str, kind: &str, id: &str) -> AppResult<AgentConfig> {
    let lv = load(conn, vault_id, kind, id)?;
    let own = match lv[0].1 {
        None => "inherit",
        Some(true) => "on",
        Some(false) => "off",
    };
    let above = lv[1..].iter().find_map(|(l, v)| v.map(|v| (v, l.clone())));
    Ok(AgentConfig {
        own: AgentChoice { kind: own.into() },
        inherited: above.as_ref().map(|(v, _)| *v),
        inherited_from: above.and_then(|(_, l)| l),
        available: available(),
    })
}

pub fn set(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
    c: &AgentChoice,
) -> AppResult<()> {
    let v: Option<i64> = match c.kind.as_str() {
        "inherit" => None,
        "on" => Some(1),
        "off" => Some(0),
        _ => return Err(AppError::validation("Unknown setting.")),
    };
    let table = if kind == "group" { "groups" } else { "hosts" };
    let n = conn.execute(
        &format!("UPDATE connection_profiles SET agent_forwarding = ?1, updated_at = ?2 WHERE id = (SELECT profile_id FROM {table} WHERE id = ?3 AND vault_id = ?4)"),
        params![v, now_ms(), id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That item no longer exists."));
    }
    Ok(())
}

/// Whether a host's shell should ask for agent forwarding, after inheritance (default off).
pub fn resolve_for_host(conn: &Connection, vault_id: &str, host_id: &str) -> AppResult<bool> {
    Ok(load(conn, vault_id, "host", host_id)?
        .into_iter()
        .find_map(|(_, v)| v)
        .unwrap_or(false))
}

/// Connects the server's agent channel to the local agent and copies bytes both ways until either
/// side closes. Returns `false` when there is no agent to talk to.
pub async fn serve<S>(channel: S) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    #[cfg(unix)]
    {
        let Some(path) = std::env::var_os("SSH_AUTH_SOCK").filter(|p| !p.is_empty()) else {
            return false;
        };
        let Ok(mut agent) = tokio::net::UnixStream::connect(path).await else {
            return false;
        };
        tokio::spawn(async move {
            let mut channel = channel;
            let _ = tokio::io::copy_bidirectional(&mut channel, &mut agent).await;
        });
        true
    }
    #[cfg(not(unix))]
    {
        let _ = channel;
        false // Windows' agent uses a named pipe, which isn't supported yet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::groups::GroupInput;
    use crate::hosts::{self, HostInput};
    use crate::vault::{self, kdf::KdfParams};

    #[test]
    fn flag_inherits_and_defaults_off() {
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let g = groups::save(
            &mut c,
            &s,
            &GroupInput {
                id: None,
                parent_id: None,
                label: "Corp".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                key_id: None,
                identity_id: None,
                startup_snippet_id: None,
                jump_mode: "inherit".into(),
                jump_host_ids: vec![],
            },
        )
        .unwrap();
        let h = hosts::save(
            &mut c,
            &s,
            &HostInput {
                id: None,
                label: "h".into(),
                address: "10.0.0.1".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                group_id: Some(g.id.clone()),
                key_id: None,
                identity_id: None,
                startup_snippet_id: None,
                jump_mode: "inherit".into(),
                jump_host_ids: vec![],
                tags: vec![],
            },
        )
        .unwrap();
        let v = &s.vault_id;
        let ch = |k: &str| AgentChoice { kind: k.into() };
        assert!(!resolve_for_host(&c, v, &h.id).unwrap());
        set(&c, v, "group", &g.id, &ch("on")).unwrap();
        assert!(resolve_for_host(&c, v, &h.id).unwrap());
        let cfg = get(&c, v, "host", &h.id).unwrap();
        assert_eq!(
            (
                cfg.inherited,
                cfg.inherited_from.as_deref(),
                cfg.own.kind.as_str()
            ),
            (Some(true), Some("Corp"), "inherit")
        );
        set(&c, v, "host", &h.id, &ch("off")).unwrap(); // own value beats the group
        assert!(!resolve_for_host(&c, v, &h.id).unwrap());
        set(&c, v, "host", &h.id, &ch("inherit")).unwrap();
        assert!(resolve_for_host(&c, v, &h.id).unwrap());
        assert!(set(&c, v, "host", &h.id, &ch("maybe")).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn serve_pipes_to_the_local_agent_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Unix socket paths are limited to ~100 bytes, and macOS temp dirs are long: use a short one.
        let dir = std::path::PathBuf::from(format!(
            "/tmp/dl-{}",
            &uuid::Uuid::now_v7().simple().to_string()[20..]
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut b = [0u8; 5];
            s.read_exact(&mut b).await.unwrap();
            s.write_all(&b.map(|c| c.to_ascii_uppercase()))
                .await
                .unwrap();
        });
        // Environment is process-wide; this is the only test that touches it.
        std::env::set_var("SSH_AUTH_SOCK", &sock);
        let (mut ours, theirs) = tokio::io::duplex(64);
        assert!(serve(theirs).await);
        ours.write_all(b"hello").await.unwrap();
        let mut out = [0u8; 5];
        ours.read_exact(&mut out).await.unwrap();
        assert_eq!(&out, b"HELLO");
        std::env::set_var("SSH_AUTH_SOCK", dir.join("missing.sock"));
        let (_a, b) = tokio::io::duplex(64);
        assert!(!serve(b).await);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
