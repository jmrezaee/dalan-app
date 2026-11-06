//! Hosts (S02, S03). A host row always has its own `connection_profiles` row; NULL fields in the
//! profile mean "inherit" from its group (see `groups.rs`). Passwords live in `secrets`, never in these structs.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::groups::{self, GroupMap, GroupSummary, IdentityMap, Inherited, Level};
use crate::vault::{
    secrets::{self, SecretKind},
    VaultSession,
};

pub const DEFAULT_PORT: u16 = 22;
pub const TELNET_PORT: u16 = 23;
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
    /// The host's own key and identity (S07). `None` = inherit.
    pub key_id: Option<String>,
    pub identity_id: Option<String>,
    pub effective_key_id: Option<String>,
    pub startup_snippet_id: Option<String>,
    /// `inherit`, `none` or `custom`.
    pub jump_mode: String,
    pub jump_host_ids: Vec<String>,
    /// The chain actually used, after inheritance (empty = direct).
    pub effective_jump_host_ids: Vec<String>,
    /// What the host would get from its groups if it set nothing itself ("from Production").
    pub inherited: Inherited,
    /// Epoch ms. Fits a JS number safely, so it is exported as `number` rather than bigint.
    #[specta(type = Option<f64>)]
    pub last_connected_at: Option<i64>,
    /// Detected after the first connect: `ubuntu`, `debian`, `macos`, `linux`, ... or `unknown`.
    pub os_hint: Option<String>,
    /// `ssh` or `telnet`.
    pub protocol: String,
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
    pub key_id: Option<String>,
    pub identity_id: Option<String>,
    pub startup_snippet_id: Option<String>,
    /// `inherit`, `none` or `custom`.
    pub jump_mode: String,
    /// Hops in order, for `custom`.
    pub jump_host_ids: Vec<String>,
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
    key_id: Option<String>,
    identity_id: Option<String>,
    password_secret_id: Option<String>,
    profile_id: String,
    chain_mode: Option<String>,
    startup_snippet_id: Option<String>,
    os_hint: Option<String>,
    protocol: String,
    /// Filled in by `with_jumps` (a separate query).
    jump_host_ids: Vec<String>,
}

const SELECT_HOST: &str = "SELECT h.id, h.label, h.address, p.ssh_port, p.username, p.password_secret_id IS NOT NULL, h.group_id, h.last_connected_at, p.key_id, p.identity_id, p.password_secret_id, h.profile_id, p.chain_mode, p.startup_snippet_id, h.os_hint, h.protocol
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
        key_id: r.get(8)?,
        identity_id: r.get(9)?,
        password_secret_id: r.get(10)?,
        profile_id: r.get(11)?,
        chain_mode: r.get(12)?,
        startup_snippet_id: r.get(13)?,
        os_hint: r.get(14)?,
        protocol: r.get(15)?,
        jump_host_ids: Vec::new(),
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

impl HostRow {
    fn level(&self) -> Level<'_> {
        Level {
            label: None,
            username: self.username.as_deref(),
            port: self.port,
            password_secret_id: self.password_secret_id.as_deref(),
            key_id: self.key_id.as_deref(),
            identity_id: self.identity_id.as_deref(),
            chain_mode: self.chain_mode.as_deref(),
            jump: &self.jump_host_ids,
            startup_snippet_id: self.startup_snippet_id.as_deref(),
        }
    }
}

fn summarize(h: HostRow, tags: Vec<String>, groups: &GroupMap, ids: &IdentityMap) -> HostSummary {
    let above = groups::chain(groups, h.group_id.as_deref());
    let inherited = Inherited::from(&groups::resolve(&above, ids));
    let mut levels = vec![h.level()];
    levels.extend(above);
    let r = groups::resolve(&levels, ids);
    HostSummary {
        effective_username: r.username.map(|s| s.value),
        effective_port: r.port.map_or(DEFAULT_PORT, |p| p.value),
        effective_has_password: r.password_secret_id.is_some(),
        effective_key_id: r.key_id.map(|k| k.value),
        effective_jump_host_ids: r.jump.map(|j| j.value).unwrap_or_default(),
        jump_mode: h.chain_mode.clone().unwrap_or_else(|| "inherit".into()),
        jump_host_ids: h.jump_host_ids.clone(),
        startup_snippet_id: h.startup_snippet_id.clone(),
        inherited,
        id: h.id,
        label: h.label,
        address: h.address,
        port: h.port,
        username: h.username,
        has_password: h.has_password,
        key_id: h.key_id,
        identity_id: h.identity_id,
        group_id: h.group_id,
        tags,
        last_connected_at: h.last_connected_at,
        os_hint: h.os_hint,
        protocol: h.protocol,
    }
}

pub fn tree(conn: &Connection, vault_id: &str) -> AppResult<HostsTree> {
    let groups = groups::load(conn, vault_id)?;
    let ids = groups::load_identities(conn, vault_id)?;
    let mut tags = tags_by_host(conn, vault_id)?;
    let mut jumps = groups::load_jumps(conn, vault_id)?;
    let mut h = conn.prepare(&format!(
        "{SELECT_HOST} WHERE h.vault_id = ?1 ORDER BY h.sort, h.label COLLATE NOCASE"
    ))?;
    let hosts = h
        .query_map([vault_id], row_to_host)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|mut row| {
            row.jump_host_ids = jumps.remove(&row.profile_id).unwrap_or_default();
            let t = tags.remove(&row.id).unwrap_or_default();
            summarize(row, t, &groups, &ids)
        })
        .collect();
    Ok(HostsTree {
        groups: groups::list(conn, vault_id)?,
        hosts,
    })
}

pub fn get(conn: &Connection, vault_id: &str, id: &str) -> AppResult<HostSummary> {
    let mut row = conn
        .query_row(
            &format!("{SELECT_HOST} WHERE h.id = ?1 AND h.vault_id = ?2"),
            params![id, vault_id],
            row_to_host,
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    row.jump_host_ids = groups::load_jumps(conn, vault_id)?
        .remove(&row.profile_id)
        .unwrap_or_default();
    let tags = tags_by_host(conn, vault_id)?
        .remove(&row.id)
        .unwrap_or_default();
    Ok(summarize(
        row,
        tags,
        &groups::load(conn, vault_id)?,
        &groups::load_identities(conn, vault_id)?,
    ))
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
    let (chain_mode, jumps) = groups::check_jump(
        conn,
        &s.vault_id,
        &input.jump_mode,
        &input.jump_host_ids,
        input.id.as_deref(),
    )?;
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
                "INSERT INTO connection_profiles (id, vault_id, username, password_secret_id, ssh_port, key_id, identity_id, chain_mode, startup_snippet_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![profile_id, s.vault_id, username, secret_id, input.port, input.key_id, input.identity_id, chain_mode, input.startup_snippet_id, now],
            )?;
            tx.execute(
                "INSERT INTO hosts (id, vault_id, group_id, profile_id, label, address, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![host_id, s.vault_id, input.group_id, profile_id, label, address, now],
            )?;
            groups::write_jumps(&tx, &profile_id, &jumps)?;
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
                "UPDATE connection_profiles SET username = ?1, password_secret_id = ?2, ssh_port = ?3, key_id = ?4, identity_id = ?5, chain_mode = ?6, startup_snippet_id = ?7, updated_at = ?8 WHERE id = ?9",
                params![username, secret_id, input.port, input.key_id, input.identity_id, chain_mode, input.startup_snippet_id, now, profile_id],
            )?;
            if let Some(o) = orphan {
                secrets::delete(&tx, s, &o)?;
            }
            groups::write_jumps(&tx, &profile_id, &jumps)?;
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

/// Copies a host with everything it owns: its profile, jump chain, tags, environment, serial settings
/// and every saved secret. Secrets are re-encrypted as new rows inside Rust and never leave it.
pub fn duplicate(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<HostSummary> {
    let tx = conn.transaction()?;
    let (profile_id, label): (String, String) = tx
        .query_row(
            "SELECT profile_id, label FROM hosts WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    let (new_host, new_profile, now) = (
        uuid::Uuid::now_v7().to_string(),
        uuid::Uuid::now_v7().to_string(),
        now_ms(),
    );
    // Same row, new ids; the secret columns are filled in below with copies.
    tx.execute(
        "INSERT INTO connection_profiles (id, vault_id, username, identity_id, password_secret_id, key_id, ssh_port, proxy_type, proxy_host, proxy_port, proxy_username, proxy_secret_id, chain_mode, agent_forwarding, startup_snippet_id, env_mode, terminal_theme, font_size, charset, keepalive_secs, sftp_start_path, created_at, updated_at)
         SELECT ?1, vault_id, username, identity_id, NULL, key_id, ssh_port, proxy_type, proxy_host, proxy_port, proxy_username, NULL, chain_mode, agent_forwarding, startup_snippet_id, env_mode, terminal_theme, font_size, charset, keepalive_secs, sftp_start_path, ?2, ?2
         FROM connection_profiles WHERE id = ?3",
        params![new_profile, now, profile_id],
    )?;
    for col in ["password_secret_id", "proxy_secret_id"] {
        let old: Option<String> = tx.query_row(
            &format!("SELECT {col} FROM connection_profiles WHERE id = ?1"),
            [&profile_id],
            |r| r.get(0),
        )?;
        if let Some(copy) = copy_secret(&tx, s, old.as_deref())? {
            tx.execute(
                &format!("UPDATE connection_profiles SET {col} = ?1 WHERE id = ?2"),
                params![copy, new_profile],
            )?;
        }
    }
    tx.execute(
        "INSERT INTO profile_jump_hosts (profile_id, position, jump_host_id) SELECT ?1, position, jump_host_id FROM profile_jump_hosts WHERE profile_id = ?2",
        params![new_profile, profile_id],
    )?;
    // Environment variables: plain values are copied as they are, sensitive ones as new secrets.
    let vars: Vec<(String, Option<String>, Option<String>, i64)> = {
        let mut st = tx.prepare(
            "SELECT name, value, value_secret_id, sort FROM profile_env_vars WHERE profile_id = ?1",
        )?;
        let rows = st.query_map([&profile_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    for (name, value, secret, sort) in vars {
        let copy = copy_secret(&tx, s, secret.as_deref())?;
        tx.execute(
            "INSERT INTO profile_env_vars (profile_id, name, value, value_secret_id, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_profile, name, value, copy, sort],
        )?;
    }
    tx.execute(
        "INSERT INTO hosts (id, vault_id, group_id, profile_id, label, address, os_hint, sftp_enabled, protocol, sort, created_at, updated_at)
         SELECT ?1, vault_id, group_id, ?2, ?3, address, os_hint, sftp_enabled, protocol, sort, ?4, ?4 FROM hosts WHERE id = ?5",
        params![new_host, new_profile, format!("{label} copy"), now, id],
    )?;
    tx.execute(
        "INSERT INTO serial_settings (host_id, baud, data_bits, stop_bits, parity, flow) SELECT ?1, baud, data_bits, stop_bits, parity, flow FROM serial_settings WHERE host_id = ?2",
        params![new_host, id],
    )?;
    tx.execute(
        "INSERT INTO host_tags (host_id, tag_id) SELECT ?1, tag_id FROM host_tags WHERE host_id = ?2",
        params![new_host, id],
    )?;
    tx.commit()?;
    get(conn, &s.vault_id, &new_host)
}

/// Hosts that save the same username and the same password, as groups of host ids (two or more each).
/// Passwords are decrypted and compared here, in Rust, and dropped (zeroized); only ids go to the UI.
pub fn shared_logins(conn: &Connection, s: &VaultSession) -> AppResult<Vec<Vec<String>>> {
    let mut st = conn.prepare(
        "SELECT h.id, p.username, p.password_secret_id FROM hosts h
         JOIN connection_profiles p ON p.id = h.profile_id
         WHERE h.vault_id = ?1 AND p.username IS NOT NULL AND p.password_secret_id IS NOT NULL",
    )?;
    let rows: Vec<(String, String, String)> = st
        .query_map([&s.vault_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut logins = Vec::new();
    for (id, user, secret) in rows {
        logins.push((id, user, secrets::get(conn, s, &secret)?));
    }
    // Few hosts have a saved password, so comparing each pair is fine.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    'next: for i in 0..logins.len() {
        for g in groups.iter_mut() {
            let first = &logins[g[0]];
            if first.1 == logins[i].1 && first.2.as_slice() == logins[i].2.as_slice() {
                g.push(i);
                continue 'next;
            }
        }
        groups.push(vec![i]);
    }
    Ok(groups
        .into_iter()
        .filter(|g| g.len() > 1)
        .map(|g| g.into_iter().map(|i| logins[i].0.clone()).collect())
        .collect())
}

/// Re-encrypts a secret as a new row, so two owners never share one. `None` stays `None`.
fn copy_secret(
    tx: &rusqlite::Transaction,
    s: &VaultSession,
    id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(id) = id else { return Ok(None) };
    let kind: String =
        tx.query_row("SELECT kind FROM secrets WHERE id = ?1", [id], |r| r.get(0))?;
    let kind = match kind.as_str() {
        "passphrase" => secrets::SecretKind::Passphrase,
        "private_key" => secrets::SecretKind::PrivateKey,
        "env_value" => secrets::SecretKind::EnvValue,
        _ => secrets::SecretKind::Password,
    };
    let plain = secrets::get(tx, s, id)?; // zeroized when dropped
    Ok(Some(secrets::put(tx, s, kind, &plain)?))
}

/// Deletes the host, its profile and its password secret. (`in_use` for jump hops arrives with P8.)
pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    // A host used as a jump host can't go until it is removed from those chains (RESTRICT in the schema).
    let mut users: Vec<String> = Vec::new();
    let mut st = conn.prepare(
        "SELECT COALESCE(h.label, g.label) FROM profile_jump_hosts j
         LEFT JOIN hosts h ON h.profile_id = j.profile_id
         LEFT JOIN groups g ON g.profile_id = j.profile_id
         WHERE j.jump_host_id = ?1",
    )?;
    for row in st.query_map([id], |r| r.get::<_, String>(0))? {
        users.push(row?);
    }
    drop(st);
    if !users.is_empty() {
        return Err(AppError::new(
            crate::error::ErrorCode::InUse,
            format!(
                "This host is a jump host for {}. Remove it from those chains first.",
                users.join(", ")
            ),
        ));
    }
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

/// Switches a host between `ssh`, `telnet` and `serial`. Only SSH keeps a saved password: Telnet
/// sends it unencrypted and a serial line has no sign-in of ours, so you type it in the terminal.
pub fn set_protocol(
    conn: &mut Connection,
    s: &VaultSession,
    host_id: &str,
    protocol: &str,
) -> AppResult<()> {
    if !["ssh", "telnet", "serial", "mosh"].contains(&protocol) {
        return Err(AppError::validation("Unknown protocol."));
    }
    let tx = conn.transaction()?;
    let (profile_id, secret): (String, Option<String>) = tx
        .query_row(
            "SELECT h.profile_id, p.password_secret_id FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.id = ?1 AND h.vault_id = ?2",
            params![host_id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    tx.execute(
        "UPDATE hosts SET protocol = ?1, updated_at = ?2 WHERE id = ?3",
        params![protocol, now_ms(), host_id],
    )?;
    if protocol == "telnet" || protocol == "serial" {
        if let Some(sid) = secret {
            tx.execute(
                "UPDATE connection_profiles SET password_secret_id = NULL WHERE id = ?1",
                [&profile_id],
            )?;
            secrets::delete(&tx, s, &sid)?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Everything a connection needs. The password is decrypted here, in Rust, and wiped on drop.
pub struct ConnectInfo {
    pub id: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<zeroize::Zeroizing<String>>,
    /// Key to try first (the private key is loaded separately, in `keys`).
    pub key_id: Option<String>,
    /// Script to type into the shell once it is ready (the host's startup snippet).
    pub startup_script: Option<String>,
    /// Hop host ids to go through first, in order (empty = direct).
    pub jump_host_ids: Vec<String>,
    /// SOCKS5 or HTTP proxy for the TCP connection to this host (used when it is the first hop).
    pub proxy: Option<crate::proxy::Proxy>,
    /// Ask the server to forward this computer's SSH agent (shells only).
    pub agent_forwarding: bool,
    /// `ssh` or `telnet`.
    pub protocol: String,
}

/// Resolves host -> group -> parent group per field (identities included), so a host with no
/// username of its own signs in with its group's.
pub fn connect_info(conn: &Connection, s: &VaultSession, host_id: &str) -> AppResult<ConnectInfo> {
    let mut h = conn
        .query_row(
            &format!("{SELECT_HOST} WHERE h.id = ?1 AND h.vault_id = ?2"),
            params![host_id, s.vault_id],
            row_to_host,
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That host no longer exists."))?;
    h.jump_host_ids = groups::load_jumps(conn, &s.vault_id)?
        .remove(&h.profile_id)
        .unwrap_or_default();
    let map = groups::load(conn, &s.vault_id)?;
    let ids = groups::load_identities(conn, &s.vault_id)?;
    let mut levels = vec![h.level()];
    levels.extend(groups::chain(&map, h.group_id.as_deref()));
    let r = groups::resolve(&levels, &ids);
    let password = match &r.password_secret_id {
        Some(sec) => {
            let bytes = secrets::get(conn, s, &sec.value)?;
            Some(zeroize::Zeroizing::new(
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        }
        None => None,
    };
    Ok(ConnectInfo {
        id: h.id.clone(),
        label: h.label.clone(),
        address: h.address.clone(),
        port: r.port.map_or(
            if h.protocol == "telnet" {
                TELNET_PORT
            } else {
                DEFAULT_PORT
            },
            |p| p.value,
        ),
        username: r.username.map(|u| u.value),
        password,
        key_id: r.key_id.map(|k| k.value),
        startup_script: match &r.startup_snippet_id {
            Some(sn) => crate::snippets::script(conn, &s.vault_id, &sn.value)?,
            None => None,
        },
        jump_host_ids: r.jump.map(|j| j.value).unwrap_or_default(),
        proxy: crate::proxy::resolve_for_host(conn, &s.vault_id, host_id)?,
        agent_forwarding: crate::agent::resolve_for_host(conn, &s.vault_id, host_id)?,
        protocol: h.protocol.clone(),
    })
}

/// Everything to connect to `host_id`: the jump hosts in order, then the target last.
/// A hop's own chain is not followed (like OpenSSH's `ProxyJump a,b`).
pub fn connect_plan(
    conn: &Connection,
    s: &VaultSession,
    host_id: &str,
) -> AppResult<Vec<ConnectInfo>> {
    let target = connect_info(conn, s, host_id)?;
    let mut plan = Vec::new();
    for hop in &target.jump_host_ids {
        plan.push(connect_info(conn, s, hop)?);
    }
    plan.push(target);
    Ok(plan)
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

/// Makes `key_id` the host's own key (after it was installed on the server).
pub fn link_key(conn: &Connection, vault_id: &str, host_id: &str, key_id: &str) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE connection_profiles SET key_id = ?1, updated_at = ?2 WHERE id = (SELECT profile_id FROM hosts WHERE id = ?3 AND vault_id = ?4)",
        params![key_id, now_ms(), host_id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That host no longer exists."));
    }
    Ok(())
}

/// Newest connections kept in `connection_history`.
const KEEP_CONNECTIONS: i64 = 200;

pub fn mark_connected(conn: &Connection, vault_id: &str, host_id: &str) -> AppResult<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE hosts SET last_connected_at = ?1 WHERE id = ?2 AND vault_id = ?3",
        params![now, host_id, vault_id],
    )?;
    conn.execute(
        "INSERT INTO connection_history (id, vault_id, host_id, connected_at) VALUES (?1, ?2, ?3, ?4)",
        params![uuid::Uuid::now_v7().to_string(), vault_id, host_id, now],
    )?;
    conn.execute(
        "DELETE FROM connection_history WHERE vault_id = ?1 AND rowid NOT IN (SELECT rowid FROM connection_history WHERE vault_id = ?1 ORDER BY connected_at DESC, rowid DESC LIMIT ?2)",
        params![vault_id, KEEP_CONNECTIONS],
    )?;
    Ok(())
}

/// One entry of the New Tab "recent" list: the server and when you last connected to it.
#[derive(Debug, Serialize, Type)]
pub struct RecentItem {
    pub host_id: String,
    pub label: String,
    pub address: String,
    #[specta(type = f64)]
    pub connected_at: i64,
}

/// Servers you connected to lately, newest first, each once.
pub fn recent_connections(
    conn: &Connection,
    vault_id: &str,
    limit: u32,
) -> AppResult<Vec<RecentItem>> {
    let mut st = conn.prepare(
        "SELECT h.id, h.label, h.address, MAX(c.connected_at) AS last FROM connection_history c JOIN hosts h ON h.id = c.host_id WHERE c.vault_id = ?1 GROUP BY h.id ORDER BY last DESC LIMIT ?2",
    )?;
    let rows = st
        .query_map(params![vault_id, limit.min(50)], |r| {
            Ok(RecentItem {
                host_id: r.get(0)?,
                label: r.get(1)?,
                address: r.get(2)?,
                connected_at: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// A tag and how many servers use it (the Tags menu).
#[derive(Debug, Serialize, Type)]
pub struct TagInfo {
    pub id: String,
    pub name: String,
    pub hosts: u32,
}

pub fn tags_list(conn: &Connection, vault_id: &str) -> AppResult<Vec<TagInfo>> {
    let mut st = conn.prepare(
        "SELECT t.id, t.name, COUNT(ht.host_id) FROM tags t LEFT JOIN host_tags ht ON ht.tag_id = t.id WHERE t.vault_id = ?1 GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok(TagInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                hosts: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Removes a tag from every server that has it.
pub fn tag_delete(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM tags WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?; // host_tags rows go with it (ON DELETE CASCADE)
    if n == 0 {
        return Err(AppError::not_found("That tag no longer exists."));
    }
    Ok(())
}

#[cfg(test)]
impl HostInput {
    /// Tests move inputs into closures; this avoids deriving `Clone` just for that.
    pub fn clone_for_test(&self) -> HostInput {
        HostInput {
            id: self.id.clone(),
            label: self.label.clone(),
            address: self.address.clone(),
            port: self.port,
            username: self.username.clone(),
            password: self.password.clone(),
            clear_password: self.clear_password,
            group_id: self.group_id.clone(),
            key_id: self.key_id.clone(),
            identity_id: self.identity_id.clone(),
            startup_snippet_id: self.startup_snippet_id.clone(),
            jump_mode: self.jump_mode.clone(),
            jump_host_ids: self.jump_host_ids.clone(),
            tags: self.tags.clone(),
        }
    }
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
            key_id: None,
            identity_id: None,
            startup_snippet_id: None,
            jump_mode: "inherit".into(),
            jump_host_ids: vec![],
            group_id: None,
            tags: vec![],
        }
    }

    fn secret_count(c: &Connection) -> i64 {
        c.query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn tags_list_counts_and_delete_removes_from_hosts_and_recents_are_distinct() {
        let (mut c, s) = fresh();
        let mut a = input("a");
        a.tags = vec!["prod".into(), "web".into()];
        let a = save(&mut c, &s, &a).unwrap();
        let mut b = input("b");
        b.tags = vec!["prod".into()];
        let b = save(&mut c, &s, &b).unwrap();
        let tags = tags_list(&c, &s.vault_id).unwrap();
        assert_eq!(
            tags.iter()
                .map(|t| (t.name.as_str(), t.hosts))
                .collect::<Vec<_>>(),
            vec![("prod", 2), ("web", 1)]
        );
        tag_delete(&c, &s.vault_id, &tags[0].id).unwrap();
        assert!(get(&c, &s.vault_id, &a.id).unwrap().tags == vec!["web".to_string()]);
        assert!(get(&c, &s.vault_id, &b.id).unwrap().tags.is_empty());
        assert!(tag_delete(&c, &s.vault_id, &tags[0].id).is_err());

        // Recent connections: each server once, newest first, written by mark_connected.
        let tick = || std::thread::sleep(std::time::Duration::from_millis(3)); // distinct millisecond stamps
        mark_connected(&c, &s.vault_id, &a.id).unwrap();
        tick();
        mark_connected(&c, &s.vault_id, &b.id).unwrap();
        tick();
        mark_connected(&c, &s.vault_id, &a.id).unwrap();
        let recent = recent_connections(&c, &s.vault_id, 10).unwrap();
        assert_eq!(
            recent
                .iter()
                .map(|r| r.label.as_str())
                .collect::<Vec<_>>()
                .len(),
            2
        );
        assert_eq!(recent[0].host_id, a.id);
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
    fn duplicate_copies_the_host_tags_and_password_as_its_own_secret() {
        let (mut c, s) = fresh();
        let mut a = input("web-1");
        a.tags = vec!["prod".into()];
        let a = save(&mut c, &s, &a).unwrap();
        let copy = duplicate(&mut c, &s, &a.id).unwrap();
        assert_ne!(copy.id, a.id);
        assert_eq!(copy.label, "web-1 copy");
        assert_eq!(copy.address, a.address);
        assert_eq!(copy.tags, vec!["prod".to_string()]);
        assert!(copy.has_password);
        // Two owners, two secrets: deleting one host must not take the other's password.
        assert_eq!(secret_count(&c), 2);
        let (a_pw, copy_pw) = (
            connect_info(&c, &s, &a.id).unwrap().password,
            connect_info(&c, &s, &copy.id).unwrap().password,
        );
        assert_eq!(a_pw.as_deref().map(|p| p.as_str()), Some("hunter2hunter2"));
        assert_eq!(
            copy_pw.as_deref().map(|p| p.as_str()),
            Some("hunter2hunter2")
        );
        delete(&mut c, &s, &a.id).unwrap();
        assert!(connect_info(&c, &s, &copy.id).unwrap().password.is_some());
    }

    #[test]
    fn shared_logins_lists_only_ids_of_hosts_with_the_same_user_and_password() {
        let (mut c, s) = fresh();
        let a = save(&mut c, &s, &input("a")).unwrap();
        let b = save(&mut c, &s, &input("b")).unwrap();
        let mut other = input("c");
        other.password = Some("a-different-one".into());
        save(&mut c, &s, &other).unwrap();
        let mut groups = shared_logins(&c, &s).unwrap();
        assert_eq!(groups.len(), 1);
        groups[0].sort();
        let mut want = vec![a.id, b.id];
        want.sort();
        assert_eq!(groups[0], want);
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
