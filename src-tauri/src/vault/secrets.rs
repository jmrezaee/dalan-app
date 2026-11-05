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
