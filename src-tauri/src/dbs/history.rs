//! What was run on a database (S37 "History" side panel): the text of each run, encrypted like terminal history with
//! the row id as associated data. Redis keeps its own console history in `redis/console.rs` (kind `command`); SQL uses
//! kind `sql`. The same text twice in a row is kept once, 500 rows per database and kind, and a statement that carries
//! a password is never stored.

use rusqlite::{params, Connection};

use crate::db::now_ms;
use crate::error::AppResult;
use crate::history::{open_command, seal_command};
use crate::vault::VaultSession;

pub const KEEP: i64 = 500;

/// `true` for text that may be written to history: nothing that sets a password.
pub fn keep_sql(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
    let has = |w: &str| words.contains(&w);
    // Whole words: `password_hash` and `users` are not a password being set.
    let sets_password = (has("password") || has("passwd") || has("identified"))
        && (has("role") || has("user") || has("alter") || has("set") || has("create") || has("identified"));
    !sets_password && !has("crypt") && !has("gen_salt")
}

/// Saves `text` for the database and returns the ones newest first. `kind` is `sql` (or `pipeline`, `filter`).
pub fn remember(
    conn: &Connection,
    s: &VaultSession,
    db_id: &str,
    kind: &str,
    text: &str,
) -> AppResult<()> {
    let text = text.trim();
    if text.is_empty() || !keep_sql(text) {
        return Ok(());
    }
    let last: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT h.id, h.body_enc FROM database_history h JOIN databases d ON d.id = h.database_id
             WHERE h.database_id = ?1 AND h.kind = ?2 AND d.vault_id = ?3 ORDER BY h.ran_at DESC, h.rowid DESC LIMIT 1",
            params![db_id, kind, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    if let Some((id, enc)) = last {
        if open_command(s, &id, &enc).as_deref() == Some(text) {
            return Ok(());
        }
    }
    // The database must be the vault's own: the join above and this check keep one vault from writing in another's.
    let own: i64 = conn.query_row(
        "SELECT count(*) FROM databases WHERE id = ?1 AND vault_id = ?2",
        params![db_id, s.vault_id],
        |r| r.get(0),
    )?;
    if own == 0 {
        return Ok(());
    }
    let id = uuid::Uuid::now_v7().to_string();
    conn.execute(
        "INSERT INTO database_history (id, database_id, kind, body_enc, ran_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, db_id, kind, seal_command(s, &id, text)?, now_ms()],
    )?;
    conn.execute(
        "DELETE FROM database_history WHERE database_id = ?1 AND kind = ?2 AND rowid NOT IN
         (SELECT rowid FROM database_history WHERE database_id = ?1 AND kind = ?2 ORDER BY ran_at DESC, rowid DESC LIMIT ?3)",
        params![db_id, kind, KEEP],
    )?;
    Ok(())
}

pub fn list(
    conn: &Connection,
    s: &VaultSession,
    db_id: &str,
    kind: &str,
) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT h.id, h.body_enc FROM database_history h JOIN databases d ON d.id = h.database_id
         WHERE h.database_id = ?1 AND h.kind = ?2 AND d.vault_id = ?3 ORDER BY h.ran_at DESC, h.rowid DESC",
    )?;
    let rows = stmt.query_map(params![db_id, kind, s.vault_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, enc) = row?;
        if let Some(t) = open_command(s, &id, &enc) {
            out.push(t);
        }
    }
    Ok(out)
}

pub fn clear(conn: &Connection, s: &VaultSession, db_id: &str, kind: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM database_history WHERE kind = ?2 AND database_id IN (SELECT id FROM databases WHERE id = ?1 AND vault_id = ?3)",
        params![db_id, kind, s.vault_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_with_passwords_are_not_kept() {
        assert!(keep_sql("select * from users"));
        assert!(keep_sql("select password_hash from users limit 1"));
        assert!(!keep_sql("select crypt('x', gen_salt('bf'))"));
        assert!(!keep_sql("create role r login password 'x'"));
        assert!(!keep_sql("alter user bob with password 'x'"));
        assert!(!keep_sql("set password for bob = 'x'"));
        assert!(!keep_sql("create user b identified by 'x'"));
    }
}
