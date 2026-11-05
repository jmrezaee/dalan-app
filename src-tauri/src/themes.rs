//! Terminal theme per host or group (S03, S04). The theme is just a name (`midnight`, ...) stored in
//! the profile's `terminal_theme` column; the colours live in the UI. NULL = inherit, so a host
//! gets its own, else its group's, else the parent group's, else the app default.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

#[derive(Debug, Serialize, Type, PartialEq)]
pub struct ProfileTheme {
    /// Host or group id.
    pub id: String,
    /// Set on the host or group itself.
    pub own: Option<String>,
    /// What it would get from the groups above, and the group that sets it.
    pub inherited: Option<String>,
    pub inherited_from: Option<String>,
}

#[derive(Debug, Serialize, Type)]
pub struct Themes {
    pub hosts: Vec<ProfileTheme>,
    pub groups: Vec<ProfileTheme>,
}

struct GroupNode {
    parent: Option<String>,
    label: String,
    theme: Option<String>,
}

/// Nearest group (starting at `start`) that sets a theme. The step limit guards against a cycle.
fn from_groups(
    map: &HashMap<String, GroupNode>,
    start: Option<&str>,
) -> (Option<String>, Option<String>) {
    let mut cur = start.and_then(|id| map.get(id));
    for _ in 0..=map.len() {
        let Some(g) = cur else { break };
        if g.theme.is_some() {
            return (g.theme.clone(), Some(g.label.clone()));
        }
        cur = g.parent.as_deref().and_then(|p| map.get(p));
    }
    (None, None)
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Themes> {
    let mut stmt = conn.prepare(
        "SELECT g.id, g.parent_id, g.label, p.terminal_theme FROM groups g JOIN connection_profiles p ON p.id = g.profile_id WHERE g.vault_id = ?1",
    )?;
    let map: HashMap<String, GroupNode> = stmt
        .query_map([vault_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                GroupNode {
                    parent: r.get(1)?,
                    label: r.get(2)?,
                    theme: r.get(3)?,
                },
            ))
        })?
        .collect::<Result<_, _>>()?;
    let groups = map
        .iter()
        .map(|(id, g)| {
            let (inherited, inherited_from) = from_groups(&map, g.parent.as_deref());
            ProfileTheme {
                id: id.clone(),
                own: g.theme.clone(),
                inherited,
                inherited_from,
            }
        })
        .collect();
    let mut stmt = conn.prepare(
        "SELECT h.id, h.group_id, p.terminal_theme FROM hosts h JOIN connection_profiles p ON p.id = h.profile_id WHERE h.vault_id = ?1",
    )?;
    let hosts = stmt
        .query_map([vault_id], |r| {
            let group: Option<String> = r.get(1)?;
            let (inherited, inherited_from) = from_groups(&map, group.as_deref());
            Ok(ProfileTheme {
                id: r.get(0)?,
                own: r.get(2)?,
                inherited,
                inherited_from,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Themes { hosts, groups })
}

/// `kind` is `host` or `group`. `None` (or blank) goes back to inheriting.
pub fn set(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    id: &str,
    theme: Option<&str>,
) -> AppResult<()> {
    let theme = theme.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = theme {
        if t.len() > 40 || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(AppError::validation("That theme name isn't valid."));
        }
    }
    let table = match kind {
        "host" => "hosts",
        "group" => "groups",
        _ => return Err(AppError::validation("Unknown item type.")),
    };
    let n = conn.execute(
        &format!("UPDATE connection_profiles SET terminal_theme = ?1, updated_at = ?2 WHERE id = (SELECT profile_id FROM {table} WHERE id = ?3 AND vault_id = ?4)"),
        params![theme, now_ms(), id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That item no longer exists."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::groups::{self, GroupInput};
    use crate::hosts::{self, HostInput};
    use crate::vault::{self, kdf::KdfParams};

    fn group(parent: Option<&str>, label: &str) -> GroupInput {
        GroupInput {
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
        }
    }

    #[test]
    fn host_inherits_nearest_group_theme_and_can_override() {
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let prod = groups::save(&mut c, &s, &group(None, "Prod")).unwrap();
        let web = groups::save(&mut c, &s, &group(Some(&prod.id), "Web")).unwrap();
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
        set(&c, &s.vault_id, "group", &prod.id, Some("ember")).unwrap();
        let t = list(&c, &s.vault_id).unwrap();
        let h = t.hosts.iter().find(|h| h.id == host.id).unwrap();
        assert_eq!(
            (
                h.own.as_deref(),
                h.inherited.as_deref(),
                h.inherited_from.as_deref()
            ),
            (None, Some("ember"), Some("Prod"))
        );
        // The child group sees the parent's; the parent sees nothing above it.
        let w = t.groups.iter().find(|g| g.id == web.id).unwrap();
        assert_eq!(w.inherited.as_deref(), Some("ember"));
        assert!(t
            .groups
            .iter()
            .find(|g| g.id == prod.id)
            .unwrap()
            .inherited
            .is_none());

        set(&c, &s.vault_id, "group", &web.id, Some("forest")).unwrap(); // nearest wins
        set(&c, &s.vault_id, "host", &host.id, Some("midnight")).unwrap(); // own wins
        let t = list(&c, &s.vault_id).unwrap();
        let h = t.hosts.iter().find(|h| h.id == host.id).unwrap();
        assert_eq!(
            (h.own.as_deref(), h.inherited.as_deref()),
            (Some("midnight"), Some("forest"))
        );
        set(&c, &s.vault_id, "host", &host.id, None).unwrap();
        assert!(list(&c, &s.vault_id).unwrap().hosts[0].own.is_none());

        assert!(set(&c, &s.vault_id, "host", &host.id, Some("bad name!")).is_err());
        assert!(set(&c, &s.vault_id, "host", "nope", Some("ember")).is_err());
    }
}
