//! Environment variables per host or group (S03, S04), sent to the server when the shell starts
//! (SSH `env` requests, like `SendEnv`; servers only accept names their `AcceptEnv` allows).
//!
//! Each level has a mode: `inherit` (use what the groups above give, ignore own rows), `merge`
//! (inherited plus own, own wins) or `replace` (own only). Resolved host -> group -> parent group.
//! Values are plain text here; sensitive values would go through `secrets` (not built yet).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::groups;

const MAX_VARS: usize = 50;
const MAX_NAME: usize = 64;
const MAX_VALUE: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// A variable the groups above supply, and which group it came from.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
pub struct InheritedVar {
    pub name: String,
    pub value: String,
    pub from: Option<String>,
}

#[derive(Debug, Serialize, Type)]
pub struct EnvConfig {
    /// `inherit`, `merge` or `replace`.
    pub mode: String,
    pub vars: Vec<EnvVar>,
    pub inherited: Vec<InheritedVar>,
}

struct Level {
    label: Option<String>,
    mode: String,
    vars: Vec<EnvVar>,
}

fn load_level(conn: &Connection, profile_id: &str, label: Option<String>) -> AppResult<Level> {
    let mode: Option<String> = conn.query_row(
        "SELECT env_mode FROM connection_profiles WHERE id = ?1",
        [profile_id],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT name, COALESCE(value, '') FROM profile_env_vars WHERE profile_id = ?1 ORDER BY sort, name",
    )?;
    let vars = stmt
        .query_map([profile_id], |r| {
            Ok(EnvVar {
                name: r.get(0)?,
                value: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Level {
        label,
        mode: mode.unwrap_or_else(|| "inherit".into()),
        vars,
    })
}

/// The item itself first, then its groups, nearest first.
fn levels(conn: &Connection, vault_id: &str, kind: &str, id: &str) -> AppResult<Vec<Level>> {
    groups::profile_chain(conn, vault_id, kind, id)?
        .into_iter()
        .map(|(label, pid)| load_level(conn, &pid, label))
        .collect()
}

/// Applies levels from the farthest group down to the item, per their modes.
fn merge(levels: &[Level]) -> Vec<InheritedVar> {
    let mut acc: Vec<InheritedVar> = Vec::new();
    for l in levels.iter().rev() {
        let own = l.vars.iter().map(|v| InheritedVar {
            name: v.name.clone(),
            value: v.value.clone(),
            from: l.label.clone(),
        });
        match l.mode.as_str() {
            "replace" => acc = own.collect(),
            "merge" => {
                for v in own {
                    match acc.iter_mut().find(|a| a.name == v.name) {
                        Some(a) => *a = v,
                        None => acc.push(v),
                    }
                }
            }
            _ => {}
        }
    }
    acc
}

pub fn get(conn: &Connection, vault_id: &str, kind: &str, id: &str) -> AppResult<EnvConfig> {
    let lv = levels(conn, vault_id, kind, id)?;
    let inherited = merge(&lv[1..]);
    let own = &lv[0];
    Ok(EnvConfig {
        mode: own.mode.clone(),
        vars: own.vars.clone(),
        inherited,
    })
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    n.len() <= MAX_NAME
        && c.next()
            .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|f| f.is_ascii_alphanumeric() || f == '_')
}

pub fn set(
    conn: &mut Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
    mode: &str,
    vars: &[EnvVar],
) -> AppResult<()> {
    if !["inherit", "merge", "replace"].contains(&mode) {
        return Err(AppError::validation("Unknown variables mode."));
    }
    if vars.len() > MAX_VARS {
        return Err(AppError::validation(format!("Up to {MAX_VARS} variables.")));
    }
    let mut seen = std::collections::HashSet::new();
    for v in vars {
        if !valid_name(&v.name) {
            return Err(AppError::validation(format!(
                "“{}” isn't a valid name. Use letters, digits and _, not starting with a digit.",
                v.name
            )));
        }
        if v.value.len() > MAX_VALUE || v.value.contains('\0') {
            return Err(AppError::validation(format!(
                "The value of {} is too long.",
                v.name
            )));
        }
        if !seen.insert(v.name.as_str()) {
            return Err(AppError::validation(format!("{} is listed twice.", v.name)));
        }
    }
    let table = if kind == "group" { "groups" } else { "hosts" };
    let tx = conn.transaction()?;
    let profile: String = tx
        .query_row(
            &format!("SELECT profile_id FROM {table} WHERE id = ?1 AND vault_id = ?2"),
            params![id, vault_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That item no longer exists."))?;
    tx.execute(
        "UPDATE connection_profiles SET env_mode = ?1, updated_at = ?2 WHERE id = ?3",
        params![
            if mode == "inherit" { None } else { Some(mode) },
            now_ms(),
            profile
        ],
    )?;
    tx.execute(
        "DELETE FROM profile_env_vars WHERE profile_id = ?1",
        [&profile],
    )?;
    for (i, v) in vars.iter().enumerate() {
        tx.execute(
            "INSERT INTO profile_env_vars (profile_id, name, value, sort) VALUES (?1, ?2, ?3, ?4)",
            params![profile, v.name, v.value, i as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// What to send for a host's shell, after inheritance.
pub fn resolve_for_host(
    conn: &Connection,
    vault_id: &str,
    host_id: &str,
) -> AppResult<Vec<EnvVar>> {
    Ok(merge(&levels(conn, vault_id, "host", host_id)?)
        .into_iter()
        .map(|v| EnvVar {
            name: v.name,
            value: v.value,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::groups::{self, GroupInput};
    use crate::hosts::{self, HostInput};
    use crate::vault::{self, kdf::KdfParams};

    fn var(n: &str, v: &str) -> EnvVar {
        EnvVar {
            name: n.into(),
            value: v.into(),
        }
    }

    #[test]
    fn modes_inherit_merge_replace_and_validation() {
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let g = |parent: Option<&str>, label: &str| GroupInput {
            id: None,
            parent_id: parent.map(String::from),
            label: label.into(),
            port: None,
            username: None,
            password: None,
            clear_password: false,
            key_id: None,
            identity_id: None,
            startup_snippet_id: None,
            jump_mode: "inherit".into(),
            jump_host_ids: vec![],
        };
        let prod = groups::save(&mut c, &s, &g(None, "Prod")).unwrap();
        let web = groups::save(&mut c, &s, &g(Some(&prod.id), "Web")).unwrap();
        let host = hosts::save(
            &mut c,
            &s,
            &HostInput {
                id: None,
                label: "w1".into(),
                address: "10.0.0.1".into(),
                port: None,
                username: None,
                password: None,
                clear_password: false,
                group_id: Some(web.id.clone()),
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
        set(
            &mut c,
            v,
            "group",
            &prod.id,
            "replace",
            &[var("LANG", "C"), var("ENVNAME", "prod")],
        )
        .unwrap();
        set(
            &mut c,
            v,
            "group",
            &web.id,
            "merge",
            &[var("ENVNAME", "web"), var("ROLE", "www")],
        )
        .unwrap();
        // The host ignores its own rows while in `inherit` mode.
        set(
            &mut c,
            v,
            "host",
            &host.id,
            "inherit",
            &[var("IGNORED", "1")],
        )
        .unwrap();
        let r = resolve_for_host(&c, v, &host.id).unwrap();
        assert_eq!(
            r,
            vec![var("LANG", "C"), var("ENVNAME", "web"), var("ROLE", "www")]
        );
        // The editor's hint says which group each inherited value came from.
        let cfg = get(&c, v, "host", &host.id).unwrap();
        assert_eq!(
            cfg.inherited
                .iter()
                .find(|i| i.name == "LANG")
                .unwrap()
                .from
                .as_deref(),
            Some("Prod")
        );
        assert_eq!(cfg.vars, vec![var("IGNORED", "1")]); // kept, just not used

        set(&mut c, v, "host", &host.id, "merge", &[var("ROLE", "api")]).unwrap();
        assert_eq!(
            resolve_for_host(&c, v, &host.id).unwrap().last().unwrap(),
            &var("ROLE", "api")
        );
        set(&mut c, v, "host", &host.id, "replace", &[var("ONLY", "me")]).unwrap();
        assert_eq!(
            resolve_for_host(&c, v, &host.id).unwrap(),
            vec![var("ONLY", "me")]
        );

        assert!(set(&mut c, v, "host", &host.id, "merge", &[var("1BAD", "x")]).is_err());
        assert!(set(
            &mut c,
            v,
            "host",
            &host.id,
            "merge",
            &[var("A", "1"), var("A", "2")]
        )
        .is_err());
        assert!(set(&mut c, v, "host", &host.id, "bogus", &[]).is_err());
        assert!(set(&mut c, v, "host", "nope", "merge", &[]).is_err());
    }
}
