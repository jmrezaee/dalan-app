//! Workspace templates (S19, F09): a saved set of panes (which hosts, in what order) and the layout
//! mode. The layout and pane list are JSON owned by the UI; Rust checks they are well formed and sane.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

pub const MAX_PANES: usize = 16;
const MAX_JSON_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Type)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    /// `split` or `focus`.
    pub mode: String,
    /// JSON: the split layout.
    pub layout: String,
    /// JSON array: `[{ "host_id": "...", "title": "..." }]`.
    pub panes: String,
}

#[derive(Debug, Deserialize, Type)]
pub struct TemplateInput {
    pub id: Option<String>,
    pub name: String,
    pub mode: String,
    pub layout: String,
    pub panes: String,
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Vec<TemplateSummary>> {
    let mut st = conn.prepare("SELECT id, name, mode, layout, panes FROM workspace_templates WHERE vault_id = ?1 ORDER BY name COLLATE NOCASE")?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok(TemplateSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                mode: r.get(2)?,
                layout: r.get(3)?,
                panes: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn save(conn: &Connection, vault_id: &str, i: &TemplateInput) -> AppResult<String> {
    let name = i.name.trim();
    if name.is_empty() {
        return Err(AppError::validation("Give the template a name."));
    }
    if !matches!(i.mode.as_str(), "split" | "focus") {
        return Err(AppError::validation("Layout must be split or focus."));
    }
    if i.layout.len() > MAX_JSON_BYTES || i.panes.len() > MAX_JSON_BYTES {
        return Err(AppError::validation("That template is too large."));
    }
    serde_json::from_str::<serde_json::Value>(&i.layout)
        .map_err(|_| AppError::validation("The layout isn't valid."))?;
    let panes: serde_json::Value = serde_json::from_str(&i.panes)
        .map_err(|_| AppError::validation("The pane list isn't valid."))?;
    match panes.as_array() {
        Some(a) if !a.is_empty() && a.len() <= MAX_PANES => {}
        _ => {
            return Err(AppError::validation(format!(
                "A template needs 1 to {MAX_PANES} panes."
            )))
        }
    }
    let now = now_ms();
    match &i.id {
        None => {
            let id = uuid::Uuid::now_v7().to_string();
            conn.execute(
                "INSERT INTO workspace_templates (id, vault_id, name, mode, layout, panes, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, vault_id, name, i.mode, i.layout, i.panes, now],
            )?;
            Ok(id)
        }
        Some(id) => {
            let n = conn.execute(
                "UPDATE workspace_templates SET name = ?1, mode = ?2, layout = ?3, panes = ?4, updated_at = ?5 WHERE id = ?6 AND vault_id = ?7",
                params![name, i.mode, i.layout, i.panes, now, id, vault_id],
            )?;
            if n == 0 {
                return Err(AppError::not_found("That template no longer exists."));
            }
            Ok(id.clone())
        }
    }
}

pub fn delete(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM workspace_templates WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That template no longer exists."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn input(name: &str, panes: &str) -> TemplateInput {
        TemplateInput {
            id: None,
            name: name.into(),
            mode: "split".into(),
            layout: r#"{"cols":2}"#.into(),
            panes: panes.into(),
        }
    }

    #[test]
    fn templates_roundtrip_and_are_validated() {
        let c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap())
            .unwrap()
            .vault_id;
        let id = save(
            &c,
            &v,
            &input(
                "Staging",
                r#"[{"host_id":"a","title":"web-1"},{"host_id":"b","title":"web-2"}]"#,
            ),
        )
        .unwrap();
        let mut update = input("Staging x2", r#"[{"host_id":"a","title":"web-1"}]"#);
        update.id = Some(id.clone());
        update.mode = "focus".into();
        save(&c, &v, &update).unwrap();
        let all = list(&c, &v).unwrap();
        assert_eq!(
            (all.len(), all[0].name.as_str(), all[0].mode.as_str()),
            (1, "Staging x2", "focus")
        );
        assert!(save(&c, &v, &input(" ", "[1]")).is_err());
        assert!(
            save(&c, &v, &input("x", "[]")).is_err(),
            "needs at least one pane"
        );
        assert!(save(&c, &v, &input("x", "not json")).is_err());
        let too_many = format!("[{}]", vec!["{}"; MAX_PANES + 1].join(","));
        assert!(save(&c, &v, &input("x", &too_many)).is_err());
        let mut bad_mode = input("x", "[{}]");
        bad_mode.mode = "grid".into();
        assert!(save(&c, &v, &bad_mode).is_err());
        delete(&c, &v, &id).unwrap();
        assert!(list(&c, &v).unwrap().is_empty());
        assert!(delete(&c, &v, &id).is_err());
    }
}
