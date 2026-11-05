//! Hosts (S02, S03). A host row always has its own `connection_profiles` row; NULL fields in the
//! profile mean "inherit" from its group (see `groups.rs`). Passwords live in `secrets`, never in these structs.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::groups::{self, GroupMap, GroupSummary, Inherited};
use crate::vault::{
    secrets::{self, SecretKind},
    VaultSession,
};

pub const DEFAULT_PORT: u16 = 22;
const MAX_TAG_LEN: usize = 32;

/// What the list needs to draw a card. No secret, only `has_password` flags.
#[derive(Debug, Clone, Serialize, Type)]
pub struct HostSummary {
    pub id: String,
    pub label: String,
    pub address: String,
    /// The host's own port. `None` = inherit (from its group, else 22).
    pub port: Option<u16>,
    /// The host's own username. `None` = inherit.
    pub username: Option<String>,
    /// The host has its own saved password.
    pub has_password: bool,
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    /// What actually gets used after inheritance. Cards show these.
    pub effective_username: Option<String>,
    pub effective_port: u16,
    pub effective_has_password: bool,
    /// What the host would get from its groups if it set nothing itself ("from Production").
    pub inherited: Inherited,
    /// Epoch ms. Fits a JS number safely, so it is exported as `number` rather than bigint.
    #[specta(type = Option<f64>)]
    pub last_connected_at: Option<i64>,
}

#[derive(Debug, Serialize, Type)]
pub struct HostsTree {
    pub groups: Vec<GroupSummary>,
    pub hosts: Vec<HostSummary>,
}

/// Create and update share one shape. `id` is `None` to create.
#[derive(Debug, Deserialize, Type)]
pub struct HostInput {
    pub id: Option<String>,
    pub label: String,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// `Some(non-empty)` sets or replaces the saved password. `None` leaves it alone.
    pub password: Option<String>,
    /// Removes the saved password (only used on update).
    pub clear_password: bool,
    pub group_id: Option<String>,
    /// Tag names. Missing tags are created, and tags no host uses any more are removed.
    pub tags: Vec<String>,
}

struct HostRow {
    id: String,
    label: String,
    address: String,
    port: Option<u16>,
    username: Option<String>,
    has_password: bool,
    group_id: Option<String>,
    last_connected_at: Option<i64>,
}

const SELECT_HOST: &str = "SELECT h.id, h.label, h.address, p.ssh_port, p.username, p.password_secret_id IS NOT NULL, h.group_id, h.last_connected_at
     FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id";

fn row_to_host(r: &rusqlite::Row) -> rusqlite::Result<HostRow> {
    Ok(HostRow {
        id: r.get(0)?,
        label: r.get(1)?,
        address: r.get(2)?,
        port: r.get::<_, Option<i64>>(3)?.map(|p| p as u16),
        username: r.get(4)?,
        has_password: r.get(5)?,
        group_id: r.get(6)?,
        last_connected_at: r.get(7)?,
    })
}

fn tags_by_host(conn: &Connection, vault_id: &str) -> AppResult<HashMap<String, Vec<String>>> {
    let mut st = conn.prepare(
        "SELECT ht.host_id, t.name FROM host_tags ht JOIN tags t ON t.id = ht.tag_id WHERE t.vault_id = ?1 ORDER BY t.name COLLATE NOCASE",
    )?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for row in st.query_map([vault_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (host, tag) = row?;
        map.entry(host).or_default().push(tag);
    }
    Ok(map)
}

fn summarize(h: HostRow, tags: Vec<String>, groups: &GroupMap) -> HostSummary {
    let w = groups::walk(groups, h.group_id.as_deref());
    HostSummary {
        effective_username: h
            .username
            .clone()
            .or_else(|| w.username.map(|(u, _)| u.to_string())),
        effective_port: h.port.or(w.port.map(|(p, _)| p)).unwrap_or(DEFAULT_PORT),
        effective_has_password: h.has_password || w.password.is_some(),
        inherited: Inherited::from(&w),
        id: h.id,
        label: h.label,
        address: h.address,
        port: h.port,
        username: h.username,
        has_password: h.has_password,
        group_id: h.group_id,
        tags,
        last_connected_at: h.last_connected_at,
    }
}

pub fn tree(conn: &Connection, vault_id: &str) -> AppResult<HostsTree> {
    let groups = groups::load(conn, vault_id)?;
    let mut tags = tags_by_host(conn, vault_id)?;
    let mut h = conn.prepare(&format!(
        "{SELECT_HOST} WHERE h.vault_id = ?1 ORDER BY h.sort, h.label COLLATE NOCASE"
    ))?;
    let hosts = h
        .query_map([vault_id], row_to_host)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|row| {
            let t = tags.remove(&row.id).unwrap_or_default();
            summarize(row, t, &groups)
        })
        .collect();
    Ok(HostsTree {
        groups: groups::list(conn, vault_id)?,
        hosts,
    })
}

pub fn get(conn: &Connection, vault_id: &str, id: &str) -> AppResult<HostSummary> {
    let row = conn
        .query_row(
            &format!("{SELECT_HOST} WHERE h.id = ?1 AND h.vault_id = ?2"),
            params![id, vault_id],
            row_to_host,
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    let tags = tags_by_host(conn, vault_id)?
        .remove(&row.id)
        .unwrap_or_default();
    Ok(summarize(row, tags, &groups::load(conn, vault_id)?))
}

/// Trims and checks the fields. Returns (label, address, username, tags).
fn validate(i: &HostInput) -> AppResult<(String, String, Option<String>, Vec<String>)> {
    let label = i.label.trim().to_string();
    let address = i.address.trim().to_string();
    if label.is_empty() {
        return Err(AppError::validation("Give the host a label."));
    }
    if address.is_empty() || address.chars().any(char::is_whitespace) {
        return Err(AppError::validation("Enter an address without spaces."));
    }
    if i.port == Some(0) {
        return Err(AppError::validation("Port must be between 1 and 65535."));
    }
    let username = i
        .username
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(String::from);
    // Tags: trimmed, no blanks, unique ignoring case (the table's unique index is NOCASE too).
    let mut tags: Vec<String> = Vec::new();
    for t in i.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if t.chars().count() > MAX_TAG_LEN {
            return Err(AppError::validation(format!(
                "Tags can be up to {MAX_TAG_LEN} characters."
            )));
        }
        if !tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            tags.push(t.to_string());
        }
    }
    Ok((label, address, username, tags))
}

fn set_tags(tx: &Transaction, vault_id: &str, host_id: &str, tags: &[String]) -> AppResult<()> {
    tx.execute("DELETE FROM host_tags WHERE host_id = ?1", [host_id])?;
    for name in tags {
        tx.execute(
            "INSERT OR IGNORE INTO tags (id, vault_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![uuid::Uuid::now_v7().to_string(), vault_id, name, now_ms()],
        )?;
        let tag_id: String = tx.query_row(
            "SELECT id FROM tags WHERE vault_id = ?1 AND name = ?2",
            params![vault_id, name],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO host_tags (host_id, tag_id) VALUES (?1, ?2)",
            params![host_id, tag_id],
        )?;
    }
    // Tags nothing uses any more are dropped, so the filter list stays honest.
    tx.execute(
        "DELETE FROM tags WHERE vault_id = ?1 AND id NOT IN (SELECT tag_id FROM host_tags)",
        [vault_id],
    )?;
    Ok(())
}

/// Writes host + profile (+ password secret, tags) in one transaction, so a failure leaves nothing behind.
pub fn save(conn: &mut Connection, s: &VaultSession, input: &HostInput) -> AppResult<HostSummary> {
    let (label, address, username, tags) = validate(input)?;
    let password = input.password.as_deref().filter(|p| !p.is_empty());
    if let Some(g) = &input.group_id {
        if !groups::load(conn, &s.vault_id)?.contains_key(g) {
            return Err(AppError::validation("That group no longer exists."));
        }
    }
    let now = now_ms();
    let tx = conn.transaction()?;

    let id = match &input.id {
        None => {
            let (host_id, profile_id) = (
                uuid::Uuid::now_v7().to_string(),
                uuid::Uuid::now_v7().to_string(),
            );
            let (secret_id, _) = groups::next_secret(&tx, s, None, password, false)?;
            tx.execute(
                "INSERT INTO connection_profiles (id, vault_id, username, password_secret_id, ssh_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![profile_id, s.vault_id, username, secret_id, input.port, now],
            )?;
            tx.execute(
                "INSERT INTO hosts (id, vault_id, group_id, profile_id, label, address, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![host_id, s.vault_id, input.group_id, profile_id, label, address, now],
            )?;
            host_id
        }
        Some(id) => {
            let (profile_id, old_secret): (String, Option<String>) = tx
                .query_row(
                    "SELECT h.profile_id, p.password_secret_id FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.id = ?1 AND h.vault_id = ?2",
                    params![id, s.vault_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
            // New password replaces in place; clear removes the secret; otherwise untouched.
            let (secret_id, orphan) =
                groups::next_secret(&tx, s, old_secret, password, input.clear_password)?;
            tx.execute(
                "UPDATE connection_profiles SET username = ?1, password_secret_id = ?2, ssh_port = ?3, updated_at = ?4 WHERE id = ?5",
                params![username, secret_id, input.port, now, profile_id],
            )?;
            if let Some(o) = orphan {
                secrets::delete(&tx, s, &o)?;
            }
            tx.execute(
                "UPDATE hosts SET label = ?1, address = ?2, group_id = ?3, updated_at = ?4 WHERE id = ?5",
                params![label, address, input.group_id, now, id],
            )?;
            id.clone()
        }
    };
    set_tags(&tx, &s.vault_id, &id, &tags)?;
    tx.commit()?;
    get(conn, &s.vault_id, &id)
}

/// Deletes the host, its profile and its password secret. (`in_use` for jump hops arrives with P8.)
pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let tx = conn.transaction()?;
    let (profile_id, secret_id): (String, Option<String>) = tx
        .query_row(
            "SELECT h.profile_id, p.password_secret_id FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.id = ?1 AND h.vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    tx.execute("DELETE FROM hosts WHERE id = ?1", [id])?;
    tx.execute(
        "DELETE FROM connection_profiles WHERE id = ?1",
        [profile_id],
    )?;
    if let Some(sid) = secret_id {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

/// Everything a connection needs. The password is decrypted here, in Rust, and wiped on drop.
pub struct ConnectInfo {
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<zeroize::Zeroizing<String>>,
}

/// Resolves host -> group -> parent group per field, so a host with no username of its own
/// signs in with its group's.
pub fn connect_info(conn: &Connection, s: &VaultSession, host_id: &str) -> AppResult<ConnectInfo> {
    let h = conn
        .query_row(
            &format!("{SELECT_HOST} WHERE h.id = ?1 AND h.vault_id = ?2"),
            params![host_id, s.vault_id],
            row_to_host,
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    let own_secret: Option<String> = conn.query_row(
        "SELECT p.password_secret_id FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.id = ?1",
        [host_id],
        |r| r.get(0),
    )?;
    let map = groups::load(conn, &s.vault_id)?;
    let w = groups::walk(&map, h.group_id.as_deref());
    let secret_id = own_secret.or_else(|| w.password.map(|(sid, _)| sid.to_string()));
    let password = match secret_id {
        Some(id) => {
            let bytes = secrets::get(conn, s, &id)?;
            Some(zeroize::Zeroizing::new(
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        }
        None => None,
    };
    Ok(ConnectInfo {
        label: h.label,
        address: h.address,
        port: h.port.or(w.port.map(|(p, _)| p)).unwrap_or(DEFAULT_PORT),
        username: h
            .username
            .or_else(|| w.username.map(|(u, _)| u.to_string())),
        password,
    })
}

/// "Remember" on the password prompt: stores the password for this host's profile.
pub fn save_password(
    conn: &mut Connection,
    s: &VaultSession,
    host_id: &str,
    password: &str,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    let (profile_id, old): (String, Option<String>) = tx
        .query_row(
            "SELECT h.profile_id, p.password_secret_id FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.id = ?1 AND h.vault_id = ?2",
            params![host_id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    match old {
        Some(id) => secrets::replace(&tx, s, &id, password.as_bytes())?,
        None => {
            let id = secrets::put(&tx, s, SecretKind::Password, password.as_bytes())?;
            tx.execute(
                "UPDATE connection_profiles SET password_secret_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![id, now_ms(), profile_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn mark_connected(conn: &Connection, vault_id: &str, host_id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE hosts SET last_connected_at = ?1 WHERE id = ?2 AND vault_id = ?3",
        params![now_ms(), host_id, vault_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn fresh() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    fn input(label: &str) -> HostInput {
        HostInput {
            id: None,
            label: label.into(),
            address: "10.0.0.5".into(),
            port: None,
            username: Some("deploy".into()),
            password: Some("hunter2hunter2".into()),
            clear_password: false,
            group_id: None,
            tags: vec![],
        }
    }

    fn secret_count(c: &Connection) -> i64 {
        c.query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn create_stores_password_encrypted_and_reports_a_flag() {
        let (mut c, s) = fresh();
        let h = save(&mut c, &s, &input("web-1")).unwrap();
        assert!(h.has_password);
        assert_eq!(h.username.as_deref(), Some("deploy"));
        let ct: Vec<u8> = c
            .query_row("SELECT ciphertext FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&ct).contains("hunter2"));
        assert_eq!(tree(&c, &s.vault_id).unwrap().hosts.len(), 1);
    }

    #[test]
    fn update_keeps_replaces_and_clears_the_password() {
        let (mut c, s) = fresh();
        let h = save(&mut c, &s, &input("web-1")).unwrap();
        let mut u = input("web-1b");
        u.id = Some(h.id.clone());
        u.password = None;
        assert!(save(&mut c, &s, &u).unwrap().has_password); // kept
        u.password = Some("a-new-password".into());
        save(&mut c, &s, &u).unwrap();
        assert_eq!(secret_count(&c), 1); // replaced in place, no orphan
        u.password = None;
        u.clear_password = true;
        let cleared = save(&mut c, &s, &u).unwrap();
        assert!(!cleared.has_password);
        assert_eq!(cleared.label, "web-1b");
        assert_eq!(secret_count(&c), 0);
    }

    #[test]
    fn delete_removes_host_profile_and_secret() {
        let (mut c, s) = fresh();
        let h = save(&mut c, &s, &input("web-1")).unwrap();
        delete(&mut c, &s, &h.id).unwrap();
        assert!(tree(&c, &s.vault_id).unwrap().hosts.is_empty());
        assert_eq!(secret_count(&c), 0);
        let profiles: i64 = c
            .query_row("SELECT count(*) FROM connection_profiles", [], |r| r.get(0))
            .unwrap();
        assert_eq!(profiles, 0);
        assert!(delete(&mut c, &s, &h.id).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        let (mut c, s) = fresh();
        let mut i = input(" ");
        assert!(save(&mut c, &s, &i).is_err());
        i = input("x");
        i.address = "a b".into();
        assert!(save(&mut c, &s, &i).is_err());
        i = input("x");
        i.port = Some(0);
        assert!(save(&mut c, &s, &i).is_err());
    }
}
