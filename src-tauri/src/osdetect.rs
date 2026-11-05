//! Remote OS detection for the host icon (S02). On the first connect to a host, one short
//! read-only command (`uname`, `/etc/os-release`) is run on a separate channel and the answer is
//! kept in `hosts.os_hint`. Switchable in Settings; on by default. Nothing else is read or sent.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppResult;

/// What runs on the server. Read-only, with output capped.
pub const COMMAND: &str =
    "uname -s 2>/dev/null; sed -n 's/^ID=//p' /etc/os-release 2>/dev/null | head -n 1";

/// Stored when the server answered but we couldn't tell, so it isn't asked again on every connect.
pub const UNKNOWN: &str = "unknown";

/// `Darwin` -> `macos`; `Linux` + `ID=ubuntu` -> `ubuntu`; other systems by their `uname` name.
pub fn parse_os(output: &str) -> Option<String> {
    let mut lines = output
        .lines()
        .map(|l| l.trim().trim_matches('"').to_lowercase())
        .filter(|l| !l.is_empty());
    let uname = lines.next()?;
    let clean = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            && s.len() <= 24
    };
    if !clean(&uname) {
        return None;
    }
    Some(match uname.as_str() {
        "darwin" => "macos".to_string(),
        "linux" => match lines.next() {
            Some(id) if clean(&id) => id,
            _ => "linux".into(),
        },
        // Windows shells (Git Bash, Cygwin) report names like `mingw64_nt-10.0`.
        u if u.starts_with("mingw") || u.starts_with("msys") || u.starts_with("cygwin") => {
            "windows".into()
        }
        other => other.to_string(),
    })
}

/// Settings -> Terminal -> "Detect server OS". On unless switched off.
pub fn enabled(conn: &Connection) -> bool {
    crate::settings::get_all(conn)
        .ok()
        .and_then(|m| {
            m.get("hosts")
                .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
        })
        .and_then(|v| v.get("detect_os").and_then(serde_json::Value::as_bool))
        .unwrap_or(true)
}

/// True when detection is on and this host has never been checked.
pub fn needed(conn: &Connection, vault_id: &str, host_id: &str) -> bool {
    if !enabled(conn) {
        return false;
    }
    conn.query_row(
        "SELECT os_hint IS NULL FROM hosts WHERE id = ?1 AND vault_id = ?2",
        params![host_id, vault_id],
        |r| r.get::<_, bool>(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(false)
}

pub fn set(conn: &Connection, vault_id: &str, host_id: &str, hint: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE hosts SET os_hint = ?1 WHERE id = ?2 AND vault_id = ?3",
        params![hint, host_id, vault_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_becomes_a_short_name() {
        assert_eq!(parse_os("Darwin\n").as_deref(), Some("macos"));
        assert_eq!(parse_os("Linux\nubuntu\n").as_deref(), Some("ubuntu"));
        assert_eq!(parse_os("Linux\n\"debian\"\n").as_deref(), Some("debian")); // quoted ID
        assert_eq!(parse_os("Linux\n").as_deref(), Some("linux")); // no os-release
        assert_eq!(parse_os("FreeBSD\n").as_deref(), Some("freebsd"));
        assert_eq!(
            parse_os("MINGW64_NT-10.0-19045\n").as_deref(),
            Some("windows")
        );
        assert_eq!(parse_os(""), None);
        assert_eq!(
            parse_os("bash: uname: command not found something long and odd!\n"),
            None
        );
    }
}
