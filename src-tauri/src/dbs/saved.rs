//! Saved queries (S34 "Saved queries" entry, S37 and S45 side panels): SQL, pipelines, filters and commands the
//! user keeps. The body is encrypted with the vault's data key like history, with the row id as the associated
//! data, so a body moved to another row does not open.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::Engine;
use crate::db::now_ms;
use crate::error::{AppError, AppResult};
use crate::history::{open_command, seal_command};
use crate::vault::VaultSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum SavedKind {
    Sql,
    Pipeline,
    Filter,
    Command,
}

impl SavedKind {
    fn as_str(self) -> &'static str {
        match self {
            SavedKind::Sql => "sql",
            SavedKind::Pipeline => "pipeline",
            SavedKind::Filter => "filter",
            SavedKind::Command => "command",
        }
    }
    fn parse(s: &str) -> Option<SavedKind> {
        [
            SavedKind::Sql,
            SavedKind::Pipeline,
            SavedKind::Filter,
            SavedKind::Command,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SavedQuery {
    pub id: String,
    /// `None`: usable on any database of that engine.
    pub database_id: Option<String>,
    pub database_name: Option<String>,
    pub engine: Engine,
    pub kind: SavedKind,
    pub folder: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub body: String,
    #[specta(type = f64)]
    pub updated_at: i64,
}

#[derive(Debug, Deserialize, Type)]
pub struct SavedQueryInput {
    pub id: Option<String>,
    pub database_id: Option<String>,
    pub engine: Engine,
    pub kind: SavedKind,
    pub folder: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub body: String,
}

fn clean(s: &Option<String>) -> Option<String> {
    crate::redis::store::clean(s)
}

/// Every saved query of the vault: by folder, then name. A body that no longer opens is left out.
pub fn list(conn: &Connection, s: &VaultSession) -> AppResult<Vec<SavedQuery>> {
    let mut st = conn.prepare(
        "SELECT q.id, q.database_id, d.name, q.engine, q.kind, q.folder, q.name, q.description, q.body_enc, q.updated_at
         FROM saved_queries q LEFT JOIN databases d ON d.id = q.database_id
         WHERE q.vault_id = ?1 ORDER BY q.folder COLLATE NOCASE, q.name COLLATE NOCASE",
    )?;
    let rows = st.query_map([&s.vault_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, Option<String>>(7)?,
            r.get::<_, Vec<u8>>(8)?,
            r.get::<_, i64>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            id,
            database_id,
            database_name,
            engine,
            kind,
            folder,
            name,
            description,
            enc,
            updated_at,
        ) = row?;
        let (Some(engine), Some(kind), Some(body)) = (
            Engine::parse(&engine),
            SavedKind::parse(&kind),
            open_command(s, &id, &enc),
        ) else {
            continue;
        };
        out.push(SavedQuery {
            id,
            database_id,
            database_name,
            engine,
            kind,
            folder,
            name,
            description,
            body,
            updated_at,
        });
    }
    Ok(out)
}

pub fn save(
    conn: &mut Connection,
    s: &VaultSession,
    input: SavedQueryInput,
) -> AppResult<SavedQuery> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::validation(
            "Give the query a name of up to 80 characters.",
        ));
    }
    if input.body.trim().is_empty() {
        return Err(AppError::validation("There is nothing to save."));
    }
    if input.body.len() > 1_000_000 {
        return Err(AppError::validation(
            "That query is too long to save (over 1 MB).",
        ));
    }
    // A kind only makes sense for some engines: SQL for the SQL engines, pipelines and filters for MongoDB.
    let fits = match input.kind {
        SavedKind::Sql => matches!(
            input.engine,
            Engine::Postgres | Engine::Mysql | Engine::Mariadb
        ),
        SavedKind::Pipeline | SavedKind::Filter => input.engine == Engine::Mongodb,
        SavedKind::Command => matches!(input.engine, Engine::Redis | Engine::Mongodb),
    };
    if !fits {
        return Err(AppError::validation(
            "That kind of query isn't for this engine.",
        ));
    }
    let tx = conn.transaction()?;
    let database_id = clean(&input.database_id);
    if let Some(d) = &database_id {
        let engine: Option<String> = tx
            .query_row(
                "SELECT engine FROM databases WHERE id = ?1 AND vault_id = ?2",
                params![d, s.vault_id],
                |r| r.get(0),
            )
            .optional()?;
        match engine.as_deref().and_then(Engine::parse) {
            None => return Err(AppError::validation("That database is gone.")),
            Some(e) if e != input.engine => {
                return Err(AppError::validation(
                    "The query is for another engine than that database.",
                ))
            }
            Some(_) => {}
        }
    }
    let now = now_ms();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let body = seal_command(s, &id, &input.body)?;
    if input.id.is_some() {
        let changed = tx.execute(
            "UPDATE saved_queries SET database_id = ?3, engine = ?4, kind = ?5, folder = ?6, name = ?7, description = ?8,
               body_enc = ?9, updated_at = ?10 WHERE id = ?1 AND vault_id = ?2",
            params![
                id,
                s.vault_id,
                database_id,
                input.engine.as_str(),
                input.kind.as_str(),
                clean(&input.folder),
                name,
                clean(&input.description),
                body,
                now
            ],
        )?;
        if changed == 0 {
            return Err(AppError::not_found("That saved query is gone."));
        }
    } else {
        tx.execute(
            "INSERT INTO saved_queries (id, vault_id, database_id, engine, kind, folder, name, description, body_enc, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                id,
                s.vault_id,
                database_id,
                input.engine.as_str(),
                input.kind.as_str(),
                clean(&input.folder),
                name,
                clean(&input.description),
                body,
                now
            ],
        )?;
    }
    tx.commit()?;
    list(conn, s)?
        .into_iter()
        .find(|q| q.id == id)
        .ok_or_else(|| AppError::internal("the saved query did not read back"))
}

pub fn delete(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM saved_queries WHERE id = ?1 AND vault_id = ?2",
        params![id, s.vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That saved query is gone."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn input(name: &str, body: &str) -> SavedQueryInput {
        SavedQueryInput {
            id: None,
            database_id: None,
            engine: Engine::Postgres,
            kind: SavedKind::Sql,
            folder: Some("orders".into()),
            name: name.into(),
            description: None,
            body: body.into(),
        }
    }

    #[test]
    fn a_saved_query_is_encrypted_at_rest_and_round_trips() {
        let mut conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let q = save(
            &mut conn,
            &s,
            input("open orders", "select * from orders where status = 'open'"),
        )
        .unwrap();
        assert_eq!(q.body, "select * from orders where status = 'open'");
        let raw: Vec<u8> = conn
            .query_row("SELECT body_enc FROM saved_queries", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("orders"));
        // Editing keeps the id and replaces the body.
        let mut again = input("open orders", "select 1");
        again.id = Some(q.id.clone());
        assert_eq!(save(&mut conn, &s, again).unwrap().body, "select 1");
        assert_eq!(list(&conn, &s).unwrap().len(), 1);
        delete(&conn, &s, &q.id).unwrap();
        assert!(list(&conn, &s).unwrap().is_empty());
        assert!(delete(&conn, &s, &q.id).is_err());
    }

    #[test]
    fn a_query_is_tied_to_a_database_of_the_same_engine_only() {
        let mut conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        conn.execute(
            "INSERT INTO databases (id, vault_id, engine, name, route, address, created_at, updated_at) VALUES ('d1', ?1, 'mongodb', 'logs', 'direct', 'h', 0, 0)",
            [&s.vault_id],
        )
        .unwrap();
        let mut q = input("x", "select 1");
        q.database_id = Some("d1".into());
        assert!(save(&mut conn, &s, q).is_err());
        let mut ok = input("y", "{}");
        ok.engine = Engine::Mongodb;
        ok.kind = SavedKind::Pipeline;
        ok.database_id = Some("d1".into());
        assert_eq!(
            save(&mut conn, &s, ok).unwrap().database_name.as_deref(),
            Some("logs")
        );
        assert!(save(&mut conn, &s, input("  ", "select 1")).is_err());
        let mut wrong = input("z", "{}");
        wrong.kind = SavedKind::Pipeline;
        assert!(
            save(&mut conn, &s, wrong).is_err(),
            "a pipeline is not for PostgreSQL"
        );
    }
}
