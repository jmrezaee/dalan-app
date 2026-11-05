//! The `secrets` table: every password, passphrase and private key lives here as ciphertext.
//! Plaintext only exists in Rust memory, wrapped in `Zeroizing`. Commands return flags, never values.

use rusqlite::{params, Connection};
use zeroize::Zeroizing;

use super::{crypto, VaultSession};
use crate::db::now_ms;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy)]
pub enum SecretKind {
    Password,
    Passphrase,
    PrivateKey,
    EnvValue,
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Passphrase => "passphrase",
            Self::PrivateKey => "private_key",
            Self::EnvValue => "env_value",
        }
    }
}

/// Encrypts and inserts a secret. Returns its id, to store in the owning row.
pub fn put(
    conn: &Connection,
    s: &VaultSession,
    kind: SecretKind,
    plaintext: &[u8],
) -> AppResult<String> {
    let id = uuid::Uuid::now_v7().to_string();
    let (nonce, ct) = crypto::seal(&s.key, id.as_bytes(), plaintext)?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO secrets (id, vault_id, kind, nonce, ciphertext, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, s.vault_id, kind.as_str(), nonce, ct, now],
    )?;
    Ok(id)
}

/// Replaces the value of an existing secret (new nonce every time).
pub fn replace(conn: &Connection, s: &VaultSession, id: &str, plaintext: &[u8]) -> AppResult<()> {
    let (nonce, ct) = crypto::seal(&s.key, id.as_bytes(), plaintext)?;
    let n = conn.execute(
        "UPDATE secrets SET nonce = ?1, ciphertext = ?2, updated_at = ?3 WHERE id = ?4 AND vault_id = ?5",
        params![nonce, ct, now_ms(), id, s.vault_id],
    )?;
    if n == 0 {
        return Err(AppError::not_found("secret not found"));
    }
    Ok(())
}

pub fn get(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<Zeroizing<Vec<u8>>> {
    let (nonce, ct): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT nonce, ciphertext FROM secrets WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::not_found("secret not found"),
            other => other.into(),
        })?;
    crypto::open(&s.key, id.as_bytes(), &nonce, &ct)
        .ok_or_else(|| AppError::internal("a stored secret could not be decrypted"))
}

pub fn delete(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM secrets WHERE id = ?1 AND vault_id = ?2",
        params![id, s.vault_id],
    )?;
    Ok(())
}

/// Deletes secrets that no row points at any more (a host or key was removed in a way that left its
/// secret behind, or a save failed half way). Run at unlock, before anything else can be saving.
/// Secrets referenced from a settings value (the AI key) count as used. Returns how many went.
pub fn sweep_orphans(conn: &Connection, vault_id: &str) -> AppResult<usize> {
    let ids: Vec<String> = {
        let mut st = conn.prepare(
            "SELECT id FROM secrets WHERE vault_id = ?1
               AND id NOT IN (SELECT private_key_secret_id FROM keys WHERE private_key_secret_id IS NOT NULL)
               AND id NOT IN (SELECT passphrase_secret_id FROM keys WHERE passphrase_secret_id IS NOT NULL)
               AND id NOT IN (SELECT password_secret_id FROM identities WHERE password_secret_id IS NOT NULL)
               AND id NOT IN (SELECT password_secret_id FROM connection_profiles WHERE password_secret_id IS NOT NULL)
               AND id NOT IN (SELECT proxy_secret_id FROM connection_profiles WHERE proxy_secret_id IS NOT NULL)
               AND id NOT IN (SELECT value_secret_id FROM profile_env_vars WHERE value_secret_id IS NOT NULL)",
        )?;
        let mapped = st.query_map([vault_id], |r| r.get(0))?;
        mapped.collect::<Result<_, _>>()?
    };
    let mut removed = 0;
    for id in ids {
        let in_settings: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM settings WHERE instr(value, ?1) > 0)",
            [&id],
            |r| r.get(0),
        )?;
        if !in_settings {
            removed += conn.execute("DELETE FROM secrets WHERE id = ?1", [&id])?;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod sweep_tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    #[test]
    fn only_unreferenced_secrets_are_swept() {
        let c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let orphan = put(&c, &s, SecretKind::Password, b"x").unwrap();
        let in_settings = put(&c, &s, SecretKind::Password, b"y").unwrap();
        c.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('ai', ?1, 0)",
            [format!("{{\"key_secret_id\":\"{in_settings}\"}}")],
        )
        .unwrap();
        assert_eq!(sweep_orphans(&c, &s.vault_id).unwrap(), 1);
        assert!(get(&c, &s, &orphan).is_err());
        assert!(get(&c, &s, &in_settings).is_ok());
    }
}
