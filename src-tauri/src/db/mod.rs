//! SQLite access. One file in the app data dir, opened only by Rust.
//! Schema source of truth is replica/schema.sql (migration 0001_init). Change it only by
//! adding a new migration below, never by editing 0001 once it has shipped.

use std::path::Path;

use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};

use crate::error::AppResult;

pub(crate) const MIGRATION_0001: &str = include_str!("../../../replica/schema.sql");
const MIGRATION_0002: &str = include_str!("../../migrations/0002_protocol.sql");
const MIGRATION_0003: &str = include_str!("../../migrations/0003_serial.sql");
const MIGRATION_0004: &str = include_str!("../../migrations/0004_mosh.sql");
const MIGRATION_0005: &str = include_str!("../../migrations/0005_data_key.sql");

/// Every migration's SQL, oldest first. The fixtures test builds older schemas from prefixes of this list.
#[cfg(test)]
pub(crate) fn migration_sources() -> [&'static str; 5] {
    [
        MIGRATION_0001,
        MIGRATION_0002,
        MIGRATION_0003,
        MIGRATION_0004,
        MIGRATION_0005,
    ]
}

#[cfg(test)]
pub(crate) fn migration_count() -> usize {
    migration_sources().len()
}

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(MIGRATION_0001),
        M::up(MIGRATION_0002),
        M::up(MIGRATION_0003),
        M::up(MIGRATION_0004),
        M::up(MIGRATION_0005),
    ])
}

/// Time is stored as UTC epoch milliseconds everywhere.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Pragmas must be set on every open, outside the migration transaction.
pub(crate) fn configure(conn: &Connection) -> AppResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

pub fn open(path: &Path) -> AppResult<Connection> {
    let mut conn = Connection::open(path)?;
    // journal_mode returns a row, so it needs pragma_update_and_check.
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    configure(&conn)?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> AppResult<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrations().to_latest(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_creates_all_tables_and_is_valid() {
        assert!(migrations().validate().is_ok());
        let conn = open_in_memory().unwrap();
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 22); // 21 from 0001 + serial_settings
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }
}
