//! Groups (S02, S04, F04) and inheritance. A group owns a `connection_profiles` row just like a
//! host. A NULL field in a profile means "inherit": we walk host -> group -> parent group -> ...
//! one field at a time and remember which group each value came from (for "from Production" hints).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::vault::{secrets, VaultSession};

/// A group plus the inheritable fields of its profile. Loaded once per request, then walked in memory.
#[derive(Debug, Clone)]
pub struct GroupRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub password_secret_id: Option<String>,
    pub key_id: Option<String>,
    pub identity_id: Option<String>,
}

pub type GroupMap = HashMap<String, GroupRow>;

pub fn load(conn: &Connection, vault_id: &str) -> AppResult<GroupMap> {
    let mut st = conn.prepare(
        "SELECT g.id, g.parent_id, g.label, p.username, p.ssh_port, p.password_secret_id, p.key_id, p.identity_id
         FROM groups g JOIN connection_profiles p ON p.id = g.profile_id WHERE g.vault_id = ?1",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok(GroupRow {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                label: r.get(2)?,
                username: r.get(3)?,
                port: r.get::<_, Option<i64>>(4)?.map(|p| p as u16),
                password_secret_id: r.get(5)?,
                key_id: r.get(6)?,
                identity_id: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().map(|g| (g.id.clone(), g)).collect())
}

/// What an identity (S07) contributes at the level where it is chosen.
#[derive(Debug, Clone)]
pub struct IdentityRow {
    pub username: String,
    pub password_secret_id: Option<String>,
    pub key_id: Option<String>,
}

pub type IdentityMap = HashMap<String, IdentityRow>;

pub fn load_identities(conn: &Connection, vault_id: &str) -> AppResult<IdentityMap> {
    let mut st = conn.prepare(
        "SELECT id, username, password_secret_id, key_id FROM identities WHERE vault_id = ?1",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                IdentityRow {
                    username: r.get(1)?,
                    password_secret_id: r.get(2)?,
                    key_id: r.get(3)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// One link in the inheritance chain: a host's own profile, or one group's.
#[derive(Default, Clone, Copy)]
pub struct Level<'a> {
    /// Group label, or `None` for the host itself.
    pub label: Option<&'a str>,
    pub username: Option<&'a str>,
    pub port: Option<u16>,
    pub password_secret_id: Option<&'a str>,
    pub key_id: Option<&'a str>,
    pub identity_id: Option<&'a str>,
}

impl<'a> From<&'a GroupRow> for Level<'a> {
    fn from(g: &'a GroupRow) -> Self {
        Level {
            label: Some(&g.label),
            username: g.username.as_deref(),
            port: g.port,
            password_secret_id: g.password_secret_id.as_deref(),
            key_id: g.key_id.as_deref(),
            identity_id: g.identity_id.as_deref(),
        }
    }
}

/// A resolved value and where it came from (`None` = the item itself, else a group label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub from: Option<String>,
}

#[derive(Debug, Default)]
pub struct Resolved {
    pub username: Option<Sourced<String>>,
    pub port: Option<Sourced<u16>>,
    pub password_secret_id: Option<Sourced<String>>,
    pub key_id: Option<Sourced<String>>,
}

/// The groups above `start`, nearest first. The step limit stops a corrupt cycle from looping forever.
pub fn chain<'a>(map: &'a GroupMap, start: Option<&str>) -> Vec<Level<'a>> {
    let mut out = Vec::new();
    let mut cur = start.and_then(|id| map.get(id));
    for _ in 0..=map.len() {
        let Some(g) = cur else { break };
        out.push(Level::from(g));
        cur = g.parent_id.as_deref().and_then(|id| map.get(id));
    }
    out
}

/// Per field, the first level (nearest first) that sets it wins. A level can set a field directly,
/// or through its identity (which supplies a username, a password and/or a key).
pub fn resolve(levels: &[Level], identities: &IdentityMap) -> Resolved {
    let mut r = Resolved::default();
    for l in levels {
        let ident = l.identity_id.and_then(|id| identities.get(id));
        let from = l.label.map(String::from);
        let src = |v: String| Sourced {
            value: v,
            from: from.clone(),
        };
        if r.username.is_none() {
            r.username = l
                .username
                .map(String::from)
                .or(ident.map(|i| i.username.clone()))
                .map(src);
        }
        if r.port.is_none() {
            r.port = l.port.map(|p| Sourced {
                value: p,
                from: from.clone(),
            });
        }
        if r.password_secret_id.is_none() {
            r.password_secret_id = l
                .password_secret_id
                .map(String::from)
                .or(ident.and_then(|i| i.password_secret_id.clone()))
                .map(src);
        }
        if r.key_id.is_none() {
            r.key_id = l
                .key_id
                .map(String::from)
                .or(ident.and_then(|i| i.key_id.clone()))
                .map(src);
        }
    }
    r
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct InheritedText {
    pub value: String,
    /// Label of the group the value comes from.
    pub group: String,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct InheritedPort {
    pub value: u16,
    pub group: String,
}

/// What this item would get if it set nothing itself. Drives the "from Production" hints.
#[derive(Debug, Clone, Default, Serialize, Type)]
pub struct Inherited {
    pub username: Option<InheritedText>,
    pub port: Option<InheritedPort>,
    /// Group label, when a password is inherited. The password itself never leaves Rust.
    pub password: Option<String>,
    /// The inherited key: `value` is the key's id (the UI looks up its label).
    pub key: Option<InheritedText>,
}

impl From<&Resolved> for Inherited {
    fn from(r: &Resolved) -> Self {
        let text = |s: &Option<Sourced<String>>| {
            s.as_ref().map(|s| InheritedText {
                value: s.value.clone(),
                group: s.from.clone().unwrap_or_default(),
            })
        };
        Inherited {
            username: text(&r.username),
            port: r.port.as_ref().map(|p| InheritedPort {
                value: p.value,
                group: p.from.clone().unwrap_or_default(),
            }),
            password: r.password_secret_id.as_ref().and_then(|p| p.from.clone()),
            key: text(&r.key_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct GroupSummary {
    pub id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub has_password: bool,
    pub key_id: Option<String>,
    pub identity_id: Option<String>,
    /// Hosts in this group and everything below it.
    pub host_count: u32,
    pub group_count: u32,
    /// What the group gets from its own ancestors.
    pub inherited: Inherited,
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Vec<GroupSummary>> {
    let map = load(conn, vault_id)?;
    let identities = load_identities(conn, vault_id)?;
    // Direct host counts, then roll them up the tree.
    let mut direct: HashMap<String, u32> = HashMap::new();
    let mut st = conn.prepare("SELECT group_id, count(*) FROM hosts WHERE vault_id = ?1 AND group_id IS NOT NULL GROUP BY group_id")?;
    for row in st.query_map([vault_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
    })? {
        let (id, n) = row?;
        direct.insert(id, n);
    }
    let mut total: HashMap<&str, u32> = HashMap::new();
    let mut children: HashMap<&str, u32> = HashMap::new();
    for g in map.values() {
        let n = direct.get(&g.id).copied().unwrap_or(0);
        // Add this group's hosts to itself and to every ancestor.
        let mut cur = Some(g);
        for _ in 0..=map.len() {
            let Some(c) = cur else { break };
            *total.entry(c.id.as_str()).or_default() += n;
            cur = c.parent_id.as_deref().and_then(|id| map.get(id));
        }
        if let Some(p) = &g.parent_id {
            *children.entry(p.as_str()).or_default() += 1;
        }
    }
    let mut out: Vec<GroupSummary> = map
        .values()
        .map(|g| GroupSummary {
            id: g.id.clone(),
            parent_id: g.parent_id.clone(),
            label: g.label.clone(),
            username: g.username.clone(),
            port: g.port,
            has_password: g.password_secret_id.is_some(),
            host_count: total.get(g.id.as_str()).copied().unwrap_or(0),
            group_count: children.get(g.id.as_str()).copied().unwrap_or(0),
            key_id: g.key_id.clone(),
            identity_id: g.identity_id.clone(),
            inherited: Inherited::from(&resolve(&chain(&map, g.parent_id.as_deref()), &identities)),
        })
        .collect();
    out.sort_by_key(|g| g.label.to_lowercase());
    Ok(out)
}

#[derive(Debug, Deserialize, Type)]
pub struct GroupInput {
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub label: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub clear_password: bool,
    pub key_id: Option<String>,
    pub identity_id: Option<String>,
}

/// True if `candidate` is `id` itself or sits somewhere below it (a move there would make a cycle).
fn is_self_or_descendant(map: &GroupMap, id: &str, candidate: &str) -> bool {
    let mut cur = map.get(candidate);
    for _ in 0..=map.len() {
        let Some(g) = cur else { return false };
        if g.id == id {
            return true;
        }
        cur = g.parent_id.as_deref().and_then(|p| map.get(p));
    }
    true // a cycle in the data: refuse rather than loop
}

/// Decides what a profile's password secret should be after an edit.
/// Returns (secret id to store, secret to delete once the profile no longer points at it).
pub fn next_secret(
    tx: &Transaction,
    s: &VaultSession,
    old: Option<String>,
    password: Option<&str>,
    clear: bool,
) -> AppResult<(Option<String>, Option<String>)> {
    Ok(match (password, old) {
        (Some(p), Some(sid)) => {
            secrets::replace(tx, s, &sid, p.as_bytes())?;
            (Some(sid), None)
        }
        (Some(p), None) => (
            Some(secrets::put(
                tx,
                s,
                secrets::SecretKind::Password,
                p.as_bytes(),
            )?),
            None,
        ),
        (None, Some(sid)) if clear => (None, Some(sid)),
        (None, old) => (old, None),
    })
}

pub fn save(
    conn: &mut Connection,
    s: &VaultSession,
    input: &GroupInput,
) -> AppResult<GroupSummary> {
    let label = input.label.trim().to_string();
    if label.is_empty() {
        return Err(AppError::validation("Give the group a name."));
    }
    if input.port == Some(0) {
        return Err(AppError::validation("Port must be between 1 and 65535."));
    }
    let username = input
        .username
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(String::from);
    let password = input.password.as_deref().filter(|p| !p.is_empty());
    let map = load(conn, &s.vault_id)?;
    if let Some(p) = &input.parent_id {
        if !map.contains_key(p) {
            return Err(AppError::validation("That parent group no longer exists."));
        }
        if let Some(id) = &input.id {
            if is_self_or_descendant(&map, id, p) {
                return Err(AppError::validation(
                    "A group can't go inside itself or one of its own groups.",
                ));
            }
        }
    }
    let now = now_ms();
    let tx = conn.transaction()?;
    let id = match &input.id {
        None => {
            let (gid, pid) = (
                uuid::Uuid::now_v7().to_string(),
                uuid::Uuid::now_v7().to_string(),
            );
            let (secret, _) = next_secret(&tx, s, None, password, false)?;
            tx.execute(
                "INSERT INTO connection_profiles (id, vault_id, username, password_secret_id, ssh_port, key_id, identity_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![pid, s.vault_id, username, secret, input.port, input.key_id, input.identity_id, now],
            )?;
            tx.execute(
                "INSERT INTO groups (id, vault_id, parent_id, profile_id, label, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![gid, s.vault_id, input.parent_id, pid, label, now],
            )?;
            gid
        }
        Some(id) => {
            let (pid, old): (String, Option<String>) = tx
                .query_row(
                    "SELECT g.profile_id, p.password_secret_id FROM groups g JOIN connection_profiles p ON p.id = g.profile_id WHERE g.id = ?1 AND g.vault_id = ?2",
                    params![id, s.vault_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| AppError::not_found("That group no longer exists."))?;
            let (secret, orphan) = next_secret(&tx, s, old, password, input.clear_password)?;
            tx.execute(
                "UPDATE connection_profiles SET username = ?1, password_secret_id = ?2, ssh_port = ?3, key_id = ?4, identity_id = ?5, updated_at = ?6 WHERE id = ?7",
                params![username, secret, input.port, input.key_id, input.identity_id, now, pid],
            )?;
            if let Some(o) = orphan {
                secrets::delete(&tx, s, &o)?;
            }
            tx.execute(
                "UPDATE groups SET label = ?1, parent_id = ?2, updated_at = ?3 WHERE id = ?4",
                params![label, input.parent_id, now, id],
            )?;
            id.clone()
        }
    };
    tx.commit()?;
    list(conn, &s.vault_id)?
        .into_iter()
        .find(|g| g.id == id)
        .ok_or_else(|| AppError::internal("saved group is missing"))
}

/// Children (groups and hosts) move up to this group's parent, then the group goes. One transaction.
pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let tx = conn.transaction()?;
    let (parent, pid, secret): (Option<String>, String, Option<String>) = tx
        .query_row(
            "SELECT g.parent_id, g.profile_id, p.password_secret_id FROM groups g JOIN connection_profiles p ON p.id = g.profile_id WHERE g.id = ?1 AND g.vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That group no longer exists."))?;
    tx.execute(
        "UPDATE groups SET parent_id = ?1 WHERE parent_id = ?2",
        params![parent, id],
    )?;
    tx.execute(
        "UPDATE hosts SET group_id = ?1 WHERE group_id = ?2",
        params![parent, id],
    )?;
    tx.execute("DELETE FROM groups WHERE id = ?1", [id])?;
    tx.execute("DELETE FROM connection_profiles WHERE id = ?1", [pid])?;
    if let Some(sid) = secret {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

/// F04: drag hosts into a group (`None` = the top level).
pub fn move_hosts(
    conn: &mut Connection,
    vault_id: &str,
    ids: &[String],
    group_id: Option<&str>,
) -> AppResult<()> {
    if let Some(g) = group_id {
        if !load(conn, vault_id)?.contains_key(g) {
            return Err(AppError::not_found("That group no longer exists."));
        }
    }
    let tx = conn.transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE hosts SET group_id = ?1, updated_at = ?2 WHERE id = ?3 AND vault_id = ?4",
            params![group_id, now_ms(), id, vault_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::hosts::{self, HostInput};
    use crate::vault::{self, kdf::KdfParams};

    fn fresh() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    fn group(
        c: &mut Connection,
        s: &VaultSession,
        label: &str,
        parent: Option<&str>,
        user: Option<&str>,
        pw: Option<&str>,
    ) -> GroupSummary {
        save(
            c,
            s,
            &GroupInput {
                id: None,
                parent_id: parent.map(String::from),
                label: label.into(),
                port: None,
                username: user.map(String::from),
                password: pw.map(String::from),
                clear_password: false,
                key_id: None,
                identity_id: None,
            },
        )
        .unwrap()
    }

    fn host(
        c: &mut Connection,
        s: &VaultSession,
        label: &str,
        group: Option<&str>,
    ) -> hosts::HostSummary {
        hosts::save(
            c,
            s,
            &HostInput {
                id: None,
                label: label.into(),
                address: "10.0.0.5".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                group_id: group.map(String::from),
                key_id: None,
                identity_id: None,
                tags: vec![],
            },
        )
        .unwrap()
    }

    #[test]
    fn host_inherits_per_field_from_the_nearest_group_and_reports_the_source() {
        let (mut c, s) = fresh();
        let prod = group(
            &mut c,
            &s,
            "Production",
            None,
            Some("deploy"),
            Some("group-pw-1234"),
        );
        let web = save(
            &mut c,
            &s,
            &GroupInput {
                id: None,
                parent_id: Some(prod.id.clone()),
                label: "Web".into(),
                port: Some(2200),
                username: None,
                password: None,
                clear_password: false,
                key_id: None,
                identity_id: None,
            },
        )
        .unwrap();
        let h = host(&mut c, &s, "web-1", Some(&web.id));
        // Port comes from Web (nearest), username and password from Production (one level further).
        assert_eq!(h.effective_port, 2200);
        assert_eq!(h.inherited.port.as_ref().unwrap().group, "Web");
        assert_eq!(h.effective_username.as_deref(), Some("deploy"));
        assert_eq!(h.inherited.username.as_ref().unwrap().group, "Production");
        assert!(h.effective_has_password && !h.has_password);
        assert_eq!(h.inherited.password.as_deref(), Some("Production"));
        // The connection uses the inherited values, including the decrypted password.
        let info = hosts::connect_info(&c, &s, &h.id).unwrap();
        assert_eq!(
            (info.port, info.username.as_deref()),
            (2200, Some("deploy"))
        );
        assert_eq!(
            info.password.as_deref().map(String::as_str),
            Some("group-pw-1234")
        );
        // A value set on the host itself wins.
        let mut own = HostInput {
            id: Some(h.id.clone()),
            label: "web-1".into(),
            address: "10.0.0.5".into(),
            port: Some(22),
            username: Some("root".into()),
            password: None,
            clear_password: false,
            key_id: None,
            identity_id: None,
            group_id: Some(web.id.clone()),
            tags: vec![],
        };
        let h2 = hosts::save(&mut c, &s, &own).unwrap();
        assert_eq!(
            (h2.effective_port, h2.effective_username.as_deref()),
            (22, Some("root"))
        );
        // And a host outside any group inherits nothing.
        own.id = None;
        own.group_id = None;
        own.username = None;
        own.port = None;
        let loose = hosts::save(&mut c, &s, &own).unwrap();
        assert_eq!((loose.effective_port, loose.effective_username), (22, None));
    }

    #[test]
    fn group_counts_roll_up_and_cycles_are_refused() {
        let (mut c, s) = fresh();
        let a = group(&mut c, &s, "A", None, None, None);
        let b = group(&mut c, &s, "B", Some(&a.id), None, None);
        host(&mut c, &s, "h1", Some(&b.id));
        host(&mut c, &s, "h2", Some(&a.id));
        let list = list(&c, &s.vault_id).unwrap();
        let a_row = list.iter().find(|g| g.id == a.id).unwrap();
        assert_eq!((a_row.host_count, a_row.group_count), (2, 1));
        // Moving A under its own child B would make a loop.
        let err = save(
            &mut c,
            &s,
            &GroupInput {
                id: Some(a.id.clone()),
                parent_id: Some(b.id.clone()),
                label: "A".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                key_id: None,
                identity_id: None,
            },
        );
        assert!(err.is_err());
        let err = save(
            &mut c,
            &s,
            &GroupInput {
                id: Some(a.id.clone()),
                parent_id: Some(a.id.clone()),
                label: "A".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                key_id: None,
                identity_id: None,
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn deleting_a_group_moves_children_up_and_removes_its_password() {
        let (mut c, s) = fresh();
        let top = group(&mut c, &s, "Top", None, None, None);
        let mid = group(
            &mut c,
            &s,
            "Mid",
            Some(&top.id),
            Some("u"),
            Some("secret-pw-123"),
        );
        let low = group(&mut c, &s, "Low", Some(&mid.id), None, None);
        let h = host(&mut c, &s, "h", Some(&mid.id));
        delete(&mut c, &s, &mid.id).unwrap();
        let t = hosts::tree(&c, &s.vault_id).unwrap();
        assert_eq!(
            t.groups
                .iter()
                .find(|g| g.id == low.id)
                .unwrap()
                .parent_id
                .as_deref(),
            Some(top.id.as_str())
        );
        assert_eq!(
            t.hosts
                .iter()
                .find(|x| x.id == h.id)
                .unwrap()
                .group_id
                .as_deref(),
            Some(top.id.as_str())
        );
        let secrets: i64 = c
            .query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(secrets, 0);
    }

    #[test]
    fn hosts_move_between_groups_and_to_the_top() {
        let (mut c, s) = fresh();
        let g = group(&mut c, &s, "G", None, None, None);
        let h = host(&mut c, &s, "h", None);
        move_hosts(
            &mut c,
            &s.vault_id,
            std::slice::from_ref(&h.id),
            Some(&g.id),
        )
        .unwrap();
        assert_eq!(
            hosts::get(&c, &s.vault_id, &h.id)
                .unwrap()
                .group_id
                .as_deref(),
            Some(g.id.as_str())
        );
        move_hosts(&mut c, &s.vault_id, std::slice::from_ref(&h.id), None).unwrap();
        assert_eq!(hosts::get(&c, &s.vault_id, &h.id).unwrap().group_id, None);
        assert!(move_hosts(&mut c, &s.vault_id, &[h.id], Some("nope")).is_err());
    }

    #[test]
    fn tags_are_created_deduplicated_and_cleaned_up() {
        let (mut c, s) = fresh();
        let mut i = HostInput {
            id: None,
            label: "h".into(),
            address: "a".into(),
            port: None,
            username: None,
            password: None,
            clear_password: false,
            key_id: None,
            identity_id: None,
            group_id: None,
            tags: vec!["Prod".into(), " prod ".into(), "web".into(), "".into()],
        };
        let h = hosts::save(&mut c, &s, &i).unwrap();
        assert_eq!(h.tags, vec!["Prod", "web"]); // duplicate ignoring case, blank dropped
        i.id = Some(h.id);
        i.tags = vec!["web".into()];
        let h = hosts::save(&mut c, &s, &i).unwrap();
        assert_eq!(h.tags, vec!["web"]);
        let n: i64 = c
            .query_row("SELECT count(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "an unused tag is removed");
    }
}
