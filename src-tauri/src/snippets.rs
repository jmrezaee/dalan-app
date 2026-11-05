//! Snippets and packages (S11, S12). A snippet is a labelled script. Running one is just writing
//! `script + "\n"` to a session, so there is no run command; a host can also pick one to run on connect.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

const MAX_SCRIPT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Type)]
pub struct SnippetSummary {
    pub id: String,
    pub label: String,
    pub script: String,
    pub package_id: Option<String>,
    /// Hosts this snippet is normally run on (pre-ticked in the multi-host picker, P11).
    pub target_host_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct PackageSummary {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Serialize, Type)]
pub struct SnippetsAll {
    pub packages: Vec<PackageSummary>,
    pub snippets: Vec<SnippetSummary>,
}

#[derive(Debug, Deserialize, Type)]
pub struct SnippetInput {
    pub id: Option<String>,
    pub label: String,
    pub script: String,
    pub package_id: Option<String>,
    pub target_host_ids: Vec<String>,
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<SnippetsAll> {
    let mut targets: HashMap<String, Vec<String>> = HashMap::new();
    let mut st = conn.prepare(
        "SELECT t.snippet_id, t.host_id FROM snippet_targets t JOIN snippets s ON s.id = t.snippet_id WHERE s.vault_id = ?1",
    )?;
    for row in st.query_map([vault_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (s, h) = row?;
        targets.entry(s).or_default().push(h);
    }
    let mut st = conn.prepare(
        "SELECT id, label, script, package_id FROM snippets WHERE vault_id = ?1 ORDER BY label COLLATE NOCASE",
    )?;
    let snippets = st
        .query_map([vault_id], |r| {
            Ok(SnippetSummary {
                id: r.get(0)?,
                label: r.get(1)?,
                script: r.get(2)?,
                package_id: r.get(3)?,
                target_host_ids: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|mut s| {
            s.target_host_ids = targets.remove(&s.id).unwrap_or_default();
            s
        })
        .collect();
    let mut st = conn.prepare(
        "SELECT id, label FROM snippet_packages WHERE vault_id = ?1 ORDER BY sort, label COLLATE NOCASE",
    )?;
    let packages = st
        .query_map([vault_id], |r| {
            Ok(PackageSummary {
                id: r.get(0)?,
                label: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(SnippetsAll { packages, snippets })
}

pub fn save(conn: &mut Connection, vault_id: &str, i: &SnippetInput) -> AppResult<()> {
    let label = i.label.trim();
    if label.is_empty() {
        return Err(AppError::validation("Give the snippet a name."));
    }
    if i.script.trim().is_empty() {
        return Err(AppError::validation("Write the script to run."));
    }
    if i.script.len() > MAX_SCRIPT_BYTES {
        return Err(AppError::validation("That script is too long."));
    }
    let now = now_ms();
    let tx = conn.transaction()?;
    if let Some(p) = &i.package_id {
        let ok: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM snippet_packages WHERE id = ?1 AND vault_id = ?2)",
            params![p, vault_id],
            |r| r.get(0),
        )?;
        if !ok {
            return Err(AppError::validation("That package no longer exists."));
        }
    }
    let id = match &i.id {
        None => {
            let id = uuid::Uuid::now_v7().to_string();
            tx.execute(
                "INSERT INTO snippets (id, vault_id, package_id, label, script, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![id, vault_id, i.package_id, label, i.script, now],
            )?;
            id
        }
        Some(id) => {
            let n = tx.execute(
                "UPDATE snippets SET package_id = ?1, label = ?2, script = ?3, updated_at = ?4 WHERE id = ?5 AND vault_id = ?6",
                params![i.package_id, label, i.script, now, id, vault_id],
            )?;
            if n == 0 {
                return Err(AppError::not_found("That snippet no longer exists."));
            }
            id.clone()
        }
    };
    tx.execute("DELETE FROM snippet_targets WHERE snippet_id = ?1", [&id])?;
    for h in &i.target_host_ids {
        // Hosts that vanished are skipped rather than failing the whole save.
        tx.execute(
            "INSERT OR IGNORE INTO snippet_targets (snippet_id, host_id) SELECT ?1, id FROM hosts WHERE id = ?2 AND vault_id = ?3",
            params![id, h, vault_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn delete(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM snippets WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That snippet no longer exists."));
    }
    Ok(())
}

/// Moves snippets into a package (`None` = out of any package).
pub fn snippets_move(
    conn: &mut Connection,
    vault_id: &str,
    ids: &[String],
    package_id: Option<&str>,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    if let Some(p) = package_id {
        let ok: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM snippet_packages WHERE id = ?1 AND vault_id = ?2)",
            params![p, vault_id],
            |r| r.get(0),
        )?;
        if !ok {
            return Err(AppError::not_found("That package no longer exists."));
        }
    }
    for id in ids {
        tx.execute(
            "UPDATE snippets SET package_id = ?1, updated_at = ?2 WHERE id = ?3 AND vault_id = ?4",
            params![package_id, now_ms(), id, vault_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn package_save(
    conn: &Connection,
    vault_id: &str,
    id: Option<&str>,
    label: &str,
) -> AppResult<()> {
    let label = label.trim();
    if label.is_empty() {
        return Err(AppError::validation("Give the package a name."));
    }
    let now = now_ms();
    match id {
        None => {
            conn.execute(
                "INSERT INTO snippet_packages (id, vault_id, label, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![uuid::Uuid::now_v7().to_string(), vault_id, label, now],
            )?;
        }
        Some(id) => {
            let n = conn.execute(
                "UPDATE snippet_packages SET label = ?1, updated_at = ?2 WHERE id = ?3 AND vault_id = ?4",
                params![label, now, id, vault_id],
            )?;
            if n == 0 {
                return Err(AppError::not_found("That package no longer exists."));
            }
        }
    }
    Ok(())
}

/// Deleting a package keeps its snippets (they move to "No package").
pub fn package_delete(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM snippet_packages WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That package no longer exists."));
    }
    Ok(())
}

/// The script of a snippet, for a host's "run on connect".
pub fn script(conn: &Connection, vault_id: &str, id: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT script FROM snippets WHERE id = ?1 AND vault_id = ?2",
            params![id, vault_id],
            |r| r.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    #[test]
    fn snippets_move_into_and_out_of_a_package() {
        let mut c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap())
            .unwrap()
            .vault_id;
        package_save(&c, &v, None, "Deploy").unwrap();
        let pkg = list(&c, &v).unwrap().packages[0].id.clone();
        let input = SnippetInput {
            id: None,
            label: "Disk".into(),
            script: "df -h".into(),
            package_id: None,
            target_host_ids: vec![],
        };
        save(&mut c, &v, &input).unwrap();
        let id = list(&c, &v).unwrap().snippets[0].id.clone();
        snippets_move(&mut c, &v, std::slice::from_ref(&id), Some(&pkg)).unwrap();
        assert_eq!(
            list(&c, &v).unwrap().snippets[0].package_id.as_deref(),
            Some(pkg.as_str())
        );
        snippets_move(&mut c, &v, &[id], None).unwrap();
        assert!(list(&c, &v).unwrap().snippets[0].package_id.is_none());
        assert!(snippets_move(&mut c, &v, &[], Some("nope")).is_err());
    }

    #[test]
    fn snippets_and_packages_roundtrip() {
        let mut c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap())
            .unwrap()
            .vault_id;
        package_save(&c, &v, None, "Deploy").unwrap();
        let pkg = list(&c, &v).unwrap().packages[0].id.clone();
        let input = |id: Option<String>, label: &str, pkg: Option<String>| SnippetInput {
            id,
            label: label.into(),
            script: "df -h".into(),
            package_id: pkg,
            target_host_ids: vec![],
        };
        save(&mut c, &v, &input(None, "Disk", Some(pkg.clone()))).unwrap();
        let s = list(&c, &v).unwrap().snippets[0].clone();
        assert_eq!(s.package_id.as_deref(), Some(pkg.as_str()));
        save(&mut c, &v, &input(Some(s.id.clone()), "Disk usage", None)).unwrap();
        assert_eq!(list(&c, &v).unwrap().snippets[0].label, "Disk usage");
        // Deleting the package keeps the snippet.
        save(
            &mut c,
            &v,
            &input(Some(s.id.clone()), "Disk usage", Some(pkg.clone())),
        )
        .unwrap();
        package_delete(&c, &v, &pkg).unwrap();
        assert_eq!(list(&c, &v).unwrap().snippets[0].package_id, None);
        assert_eq!(script(&c, &v, &s.id).unwrap().as_deref(), Some("df -h"));
        delete(&c, &v, &s.id).unwrap();
        assert!(list(&c, &v).unwrap().snippets.is_empty());
    }

    #[test]
    fn rejects_empty_label_script_and_unknown_package() {
        let mut c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap())
            .unwrap()
            .vault_id;
        let mk = |label: &str, script: &str, pkg: Option<&str>| SnippetInput {
            id: None,
            label: label.into(),
            script: script.into(),
            package_id: pkg.map(String::from),
            target_host_ids: vec![],
        };
        assert!(save(&mut c, &v, &mk(" ", "x", None)).is_err());
        assert!(save(&mut c, &v, &mk("a", "  ", None)).is_err());
        assert!(save(&mut c, &v, &mk("a", "x", Some("nope"))).is_err());
    }
}
