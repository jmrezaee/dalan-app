//! App settings (S23): a few named JSON documents in the `settings` table, validated on write.
//! The UI owns the shape of `appearance` and `terminal`; Rust reads `security` (auto-lock).

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

const KEYS: [&str; 9] = [
    "appearance",
    "terminal",
    "security",
    "shortcuts",
    "logs",
    "history",
    "hosts",
    "ai",
    "sftp",
];
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_AUTO_LOCK_MINUTES: u64 = 24 * 60;

/// One field of the `hosts` settings document (`detect_os`, `post_quantum`, `local_shell`), if set.
pub fn hosts_field(conn: &Connection, name: &str) -> Option<serde_json::Value> {
    let all = get_all(conn).ok()?;
    let doc: serde_json::Value = serde_json::from_str(all.get("hosts")?).ok()?;
    doc.get(name).cloned()
}

/// Remembered "open with" apps for the SFTP editor: file extension -> app path. Only Rust writes
/// this (`settings_set` refuses the `sftp` key), so the webview can't make us launch a program.
pub fn open_with_all(conn: &Connection) -> HashMap<String, String> {
    get_all(conn)
        .ok()
        .and_then(|a| {
            a.get("sftp")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        })
        .and_then(|d| serde_json::from_value(d.get("open_with")?.clone()).ok())
        .unwrap_or_default()
}

pub fn open_with_set(conn: &Connection, ext: &str, app: &str) -> AppResult<()> {
    let mut map = open_with_all(conn);
    map.insert(ext.to_string(), app.to_string());
    set(
        conn,
        "sftp",
        &serde_json::json!({ "open_with": map }).to_string(),
    )
}

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
    fn open_with_is_remembered_per_extension() {
        let c = open_in_memory().unwrap();
        assert!(open_with_all(&c).is_empty());
        open_with_set(&c, "txt", "/Apps/Edit.app").unwrap();
        open_with_set(&c, "md", "/Apps/Other.app").unwrap();
        open_with_set(&c, "txt", "/Apps/New.app").unwrap();
        let m = open_with_all(&c);
        assert_eq!(m["txt"], "/Apps/New.app");
        assert_eq!(m["md"], "/Apps/Other.app");
    }

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
