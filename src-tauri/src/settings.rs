//! App settings (S23): a few named JSON documents in the `settings` table, validated on write.
//! The UI owns the shape of `appearance` and `terminal`; Rust reads `security` (auto-lock).

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

const KEYS: [&str; 7] = [
    "appearance",
    "terminal",
    "security",
    "shortcuts",
    "logs",
    "history",
    "hosts",
];
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_AUTO_LOCK_MINUTES: u64 = 24 * 60;

pub fn get_all(conn: &Connection) -> AppResult<HashMap<String, String>> {
    let mut st = conn.prepare("SELECT key, value FROM settings")?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// Stores one settings document. `value` must be a JSON object, and `security` is range-checked.
pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    if !KEYS.contains(&key) {
        return Err(AppError::validation("Unknown setting."));
    }
    if value.len() > MAX_VALUE_BYTES {
        return Err(AppError::validation("That setting is too large."));
    }
    let json: serde_json::Value =
        serde_json::from_str(value).map_err(|_| AppError::validation("Settings must be JSON."))?;
    if !json.is_object() {
        return Err(AppError::validation("Settings must be a JSON object."));
    }
    if key == "security" {
        if let Some(m) = json.get("auto_lock_minutes") {
            if !m.as_u64().is_some_and(|m| m <= MAX_AUTO_LOCK_MINUTES) {
                return Err(AppError::validation(
                    "Lock after idle must be 0 to 1440 minutes.",
                ));
            }
        }
    }
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        params![key, value, now_ms()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::auto_lock_minutes;

    #[test]
    fn roundtrip_and_validation() {
        let c = open_in_memory().unwrap();
        set(&c, "terminal", r#"{"font_size":16}"#).unwrap();
        set(&c, "terminal", r#"{"font_size":18}"#).unwrap(); // upsert
        assert_eq!(get_all(&c).unwrap()["terminal"], r#"{"font_size":18}"#);
        assert!(set(&c, "nope", "{}").is_err());
        assert!(set(&c, "terminal", "[1]").is_err());
        assert!(set(&c, "terminal", "not json").is_err());
        assert!(set(&c, "security", r#"{"auto_lock_minutes":-1}"#).is_err());
        assert!(set(&c, "security", r#"{"auto_lock_minutes":99999}"#).is_err());
    }

    #[test]
    fn auto_lock_setting_reaches_the_vault_reader() {
        let c = open_in_memory().unwrap();
        set(&c, "security", r#"{"auto_lock_minutes":5}"#).unwrap();
        assert_eq!(auto_lock_minutes(&c), 5);
    }
}
