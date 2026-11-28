//! Schema snapshots (D16, E3): one schema's catalog at a moment, to compare against later. The catalog is sealed with
//! the vault's data key (it holds view and function bodies), the row id as associated data, like history.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use crate::db::now_ms;
use crate::dbs::schemadiff::{Catalog, Family};
use crate::error::{AppError, AppResult};
use crate::history::{open_command, seal_command};
use crate::vault::VaultSession;

#[derive(Debug, Clone, Serialize, Type)]
pub struct SnapshotRow {
    pub id: String,
    pub database_id: String,
    pub database_name: String,
    pub schema: String,
    pub family: Family,
    pub captured_at: f64,
    pub tables: f64,
}

pub fn save(
    conn: &Connection,
    s: &VaultSession,
    database_id: &str,
    cat: &Catalog,
) -> AppResult<SnapshotRow> {
    let id = uuid::Uuid::now_v7().to_string();
    let json = serde_json::to_string(cat).map_err(|e| AppError::internal(e.to_string()))?;
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM databases WHERE id = ?1 AND vault_id = ?2",
            params![database_id, s.vault_id],
            |r| r.get(0),
        )
        .optional()?;
    let name = found.ok_or_else(|| AppError::not_found("That database is no longer saved."))?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO schema_snapshots (id, database_id, captured_at, catalog_enc) VALUES (?1, ?2, ?3, ?4)",
        params![id, database_id, now, seal_command(s, &id, &json)?],
    )?;
    Ok(SnapshotRow {
        id,
        database_id: database_id.into(),
        database_name: name,
        schema: cat.schema.clone(),
        family: cat.family,
        captured_at: now as f64,
        tables: cat.tables.len() as f64,
    })
}

fn open(s: &VaultSession, id: &str, blob: &[u8]) -> Option<Catalog> {
    serde_json::from_str(&open_command(s, id, blob)?).ok()
}

/// Every snapshot of this vault, newest first.
pub fn list(conn: &Connection, s: &VaultSession) -> AppResult<Vec<SnapshotRow>> {
    let mut st = conn.prepare(
        "SELECT x.id, x.database_id, d.name, x.captured_at, x.catalog_enc FROM schema_snapshots x JOIN databases d ON d.id = x.database_id
         WHERE d.vault_id = ?1 ORDER BY x.captured_at DESC",
    )?;
    let rows = st.query_map([&s.vault_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Vec<u8>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, database_id, database_name, at, blob) = row?;
        if let Some(cat) = open(s, &id, &blob) {
            out.push(SnapshotRow {
                id,
                database_id,
                database_name,
                schema: cat.schema,
                family: cat.family,
                captured_at: at as f64,
                tables: cat.tables.len() as f64,
            });
        }
    }
    Ok(out)
}

pub fn load(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<(Catalog, String)> {
    let (blob, name, at): (Vec<u8>, String, i64) = conn
        .query_row(
            "SELECT x.catalog_enc, d.name, x.captured_at FROM schema_snapshots x JOIN databases d ON d.id = x.database_id
             WHERE x.id = ?1 AND d.vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That snapshot is no longer saved."))?;
    let cat = open(s, id, &blob)
        .ok_or_else(|| AppError::internal("The snapshot can't be read with this vault's key."))?;
    Ok((cat, format!("{name} · snapshot {}", at)))
}

pub fn delete(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM schema_snapshots WHERE id = ?1 AND database_id IN (SELECT id FROM databases WHERE vault_id = ?2)",
        params![id, s.vault_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_snapshot_is_sealed_listed_and_read_back() {
        let conn = crate::db::open_in_memory().unwrap();
        let s = crate::vault::create(
            &conn,
            "correct-horse-staple",
            crate::vault::kdf::KdfParams::cheap(),
        )
        .unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO databases (id, vault_id, engine, name, route, address, created_at, updated_at) VALUES ('d1', '{}', 'postgres', 'shop', 'direct', 'localhost', 1, 1);",
            s.vault_id
        ))
        .unwrap();
        let cat = crate::dbs::schemadiff::Catalog {
            family: crate::dbs::schemadiff::Family::Pg,
            schema: "public".into(),
            tables: vec![],
            objects: vec![],
        };
        let row = super::save(&conn, &s, "d1", &cat).unwrap();
        let blob: Vec<u8> = conn
            .query_row("SELECT catalog_enc FROM schema_snapshots", [], |r| r.get(0))
            .unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("public"), "sealed");
        assert_eq!(super::list(&conn, &s).unwrap().len(), 1);
        assert_eq!(super::load(&conn, &s, &row.id).unwrap().0, cat);
        super::delete(&conn, &s, &row.id).unwrap();
        assert!(super::list(&conn, &s).unwrap().is_empty());
    }
}
