//! Trusted server keys (S14, F01, F12). Rule: unknown key -> ask; changed key -> block.
//! There is no "accept all" anywhere in this module, by design (CLAUDE.md hard rule 5).

use rusqlite::{params, Connection};
use serde::Serialize;
use specta::Type;

use crate::db::now_ms;
use crate::error::{AppError, AppResult};

/// "address:port" exactly as connected; IPv6 literals are bracketed, like OpenSSH: "[::1]:2222".
pub fn pattern(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    /// Same key already trusted.
    Known,
    /// Nothing saved for this address and key type: ask the user (trust on first use).
    Unknown,
    /// A different key is saved for this address and key type: block.
    Changed { old_fingerprint: String },
}

pub fn check(
    conn: &Connection,
    vault_id: &str,
    pattern: &str,
    key_type: &str,
    public_key: &str,
) -> AppResult<Check> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT public_key, fingerprint_sha256 FROM known_hosts WHERE vault_id = ?1 AND host_pattern = ?2 AND key_type = ?3",
            params![vault_id, pattern, key_type],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(match row {
        None => Check::Unknown,
        Some((saved, _)) if saved == public_key => {
            conn.execute(
                "UPDATE known_hosts SET last_seen_at = ?1 WHERE vault_id = ?2 AND host_pattern = ?3 AND key_type = ?4",
                params![now_ms(), vault_id, pattern, key_type],
            )?;
            Check::Known
        }
        Some((_, old_fingerprint)) => Check::Changed { old_fingerprint },
    })
}

/// Saves a key, replacing any earlier one for the same address and key type.
/// Callers must only reach this after the user said yes.
pub fn trust(
    conn: &Connection,
    vault_id: &str,
    pattern: &str,
    key_type: &str,
    public_key: &str,
    fingerprint: &str,
) -> AppResult<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO known_hosts (id, vault_id, host_pattern, key_type, public_key, fingerprint_sha256, source, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'tofu', ?7, ?7)
         ON CONFLICT(vault_id, host_pattern, key_type) DO UPDATE SET public_key = ?5, fingerprint_sha256 = ?6, created_at = ?7, last_seen_at = ?7",
        params![uuid::Uuid::now_v7().to_string(), vault_id, pattern, key_type, public_key, fingerprint, now],
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Type)]
pub struct KnownHost {
    pub id: String,
    pub host_pattern: String,
    pub key_type: String,
    pub fingerprint: String,
    /// Epoch ms. Exported as `number` (fits a JS number safely).
    #[specta(type = f64)]
    pub created_at: i64,
    #[specta(type = Option<f64>)]
    pub last_seen_at: Option<i64>,
}

pub fn list(conn: &Connection, vault_id: &str) -> AppResult<Vec<KnownHost>> {
    let mut st = conn.prepare(
        "SELECT id, host_pattern, key_type, fingerprint_sha256, created_at, last_seen_at FROM known_hosts WHERE vault_id = ?1 ORDER BY host_pattern, key_type",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok(KnownHost {
                id: r.get(0)?,
                host_pattern: r.get(1)?,
                key_type: r.get(2)?,
                fingerprint: r.get(3)?,
                created_at: r.get(4)?,
                last_seen_at: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn delete(conn: &Connection, vault_id: &str, id: &str) -> AppResult<()> {
    let n = conn.execute(
        "DELETE FROM known_hosts WHERE id = ?1 AND vault_id = ?2",
        params![id, vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("That entry no longer exists."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    #[test]
    fn unknown_then_known_then_changed() {
        let c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let p = pattern("10.0.0.5", 22);
        assert_eq!(
            check(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA1").unwrap(),
            Check::Unknown
        );
        trust(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA1", "SHA256:one").unwrap();
        assert_eq!(
            check(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA1").unwrap(),
            Check::Known
        );
        // Same address, different key: blocked, and the old fingerprint is reported.
        assert_eq!(
            check(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA2").unwrap(),
            Check::Changed {
                old_fingerprint: "SHA256:one".into()
            }
        );
        // Another port is a different server.
        assert_eq!(
            check(
                &c,
                &v.vault_id,
                &pattern("10.0.0.5", 2222),
                "ssh-ed25519",
                "AAAA2"
            )
            .unwrap(),
            Check::Unknown
        );
        // Replacing is explicit: trust() overwrites, and nothing else does.
        trust(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA2", "SHA256:two").unwrap();
        assert_eq!(
            check(&c, &v.vault_id, &p, "ssh-ed25519", "AAAA2").unwrap(),
            Check::Known
        );
        assert_eq!(list(&c, &v.vault_id).unwrap().len(), 1);
    }

    #[test]
    fn ipv6_is_bracketed_and_delete_works() {
        assert_eq!(pattern("::1", 2222), "[::1]:2222");
        let c = open_in_memory().unwrap();
        let v = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        trust(&c, &v.vault_id, "h:22", "ssh-ed25519", "K", "SHA256:x").unwrap();
        let id = list(&c, &v.vault_id).unwrap()[0].id.clone();
        delete(&c, &v.vault_id, &id).unwrap();
        assert!(list(&c, &v.vault_id).unwrap().is_empty());
        assert!(delete(&c, &v.vault_id, &id).is_err());
    }
}
