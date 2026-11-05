//! The vault: master password -> Argon2id -> key held in memory only while unlocked (F10, S24).
//! Everything secret in the database is XChaCha20-Poly1305 ciphertext under that key.

pub mod crypto;
pub mod kdf;
pub mod secrets;

use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroize;

use self::kdf::{derive_key, KdfParams, Key};
use crate::db::now_ms;
use crate::error::{AppError, AppResult};

const KEY_CHECK_AAD: &[u8] = b"dalan:key-check:v1";
const KEY_CHECK_PLAINTEXT: &[u8] = b"dalan-vault-ok";
const SALT_LEN: usize = 16;
pub const MIN_PASSWORD_CHARS: usize = 8;
pub const DEFAULT_AUTO_LOCK_MINUTES: u32 = 15;

/// What "unlocked" means: the vault id and its key. The key is wiped when this is dropped.
pub struct VaultSession {
    pub vault_id: String,
    pub key: Key,
}

pub fn exists(conn: &Connection) -> AppResult<bool> {
    Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM vaults)", [], |r| r.get(0))?)
}

fn random_salt() -> [u8; SALT_LEN] {
    use chacha20poly1305::aead::{rand_core::RngCore, OsRng};
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// key_check blob = nonce || ciphertext of a known value. Opening it proves the password was right.
fn make_key_check(key: &Key) -> AppResult<Vec<u8>> {
    let (nonce, ct) = crypto::seal(key, KEY_CHECK_AAD, KEY_CHECK_PLAINTEXT)?;
    Ok([nonce, ct].concat())
}

fn key_check_ok(key: &Key, blob: &[u8]) -> bool {
    if blob.len() <= crypto::NONCE_LEN {
        return false;
    }
    let (nonce, ct) = blob.split_at(crypto::NONCE_LEN);
    crypto::open(key, KEY_CHECK_AAD, nonce, ct).is_some_and(|p| p.as_slice() == KEY_CHECK_PLAINTEXT)
}

/// First run (F10). Creates the vault row and returns an unlocked session.
pub fn create(conn: &Connection, password: &str, params: KdfParams) -> AppResult<VaultSession> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::validation(format!(
            "Use at least {MIN_PASSWORD_CHARS} characters."
        )));
    }
    if exists(conn)? {
        return Err(AppError::validation("A vault already exists."));
    }
    let salt = random_salt();
    let key = derive_key(password.as_bytes(), &salt, &params)?;
    let key_check = make_key_check(&key)?;
    let id = uuid::Uuid::now_v7().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO vaults (id, name, kind, kdf_salt, kdf_params, key_check, created_at, updated_at) VALUES (?1, 'Personal', 'local', ?2, ?3, ?4, ?5, ?5)",
        params![id, salt.to_vec(), serde_json::to_string(&params).map_err(|e| AppError::internal(e.to_string()))?, key_check, now],
    )?;
    Ok(VaultSession { vault_id: id, key })
}

struct VaultRow {
    id: String,
    salt: Vec<u8>,
    params: KdfParams,
    key_check: Vec<u8>,
}

fn load_row(conn: &Connection) -> AppResult<VaultRow> {
    let (id, salt, params_json, key_check): (String, Vec<u8>, String, Vec<u8>) = conn
        .query_row(
            "SELECT id, kdf_salt, kdf_params, key_check FROM vaults ORDER BY created_at LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("No vault yet. Set one up first."))?;
    let params = serde_json::from_str(&params_json)
        .map_err(|e| AppError::internal(format!("bad kdf_params: {e}")))?;
    Ok(VaultRow {
        id,
        salt,
        params,
        key_check,
    })
}

/// Derives the key and checks it against `key_check`. Wrong password -> `wrong_password`.
pub fn unlock(conn: &Connection, password: &str) -> AppResult<VaultSession> {
    let row = load_row(conn)?;
    let key = derive_key(password.as_bytes(), &row.salt, &row.params)?;
    if !key_check_ok(&key, &row.key_check) {
        return Err(AppError::wrong_password());
    }
    Ok(VaultSession {
        vault_id: row.id,
        key,
    })
}

/// Re-derives with a new salt and re-encrypts every secret in one transaction.
pub fn change_password(
    conn: &mut Connection,
    old: &str,
    new: &str,
    params: KdfParams,
) -> AppResult<VaultSession> {
    if new.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AppError::validation(format!(
            "Use at least {MIN_PASSWORD_CHARS} characters."
        )));
    }
    let old_session = unlock(conn, old)?;
    let salt = random_salt();
    let new_key = derive_key(new.as_bytes(), &salt, &params)?;

    let tx = conn.transaction()?;
    let rows: Vec<(String, Vec<u8>, Vec<u8>)> = {
        let mut stmt =
            tx.prepare("SELECT id, nonce, ciphertext FROM secrets WHERE vault_id = ?1")?;
        let mapped = stmt.query_map([&old_session.vault_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        mapped.collect::<Result<_, _>>()?
    };
    for (id, nonce, ct) in rows {
        let mut plain = crypto::open(&old_session.key, id.as_bytes(), &nonce, &ct)
            .ok_or_else(|| AppError::internal("a stored secret could not be decrypted"))?;
        let (new_nonce, new_ct) = crypto::seal(&new_key, id.as_bytes(), &plain)?;
        plain.zeroize();
        tx.execute(
            "UPDATE secrets SET nonce = ?1, ciphertext = ?2 WHERE id = ?3",
            params![new_nonce, new_ct, id],
        )?;
    }
    tx.execute(
        "UPDATE vaults SET kdf_salt = ?1, kdf_params = ?2, key_check = ?3, updated_at = ?4 WHERE id = ?5",
        params![
            salt.to_vec(),
            serde_json::to_string(&params).map_err(|e| AppError::internal(e.to_string()))?,
            make_key_check(&new_key)?,
            now_ms(),
            old_session.vault_id
        ],
    )?;
    tx.commit()?;
    Ok(VaultSession {
        vault_id: old_session.vault_id,
        key: new_key,
    })
}

/// Idle minutes before auto-lock, from settings.security.auto_lock_minutes. 0 = never.
pub fn auto_lock_minutes(conn: &Connection) -> u32 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'security'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
    .and_then(|v| v.get("auto_lock_minutes")?.as_u64())
    .map_or(DEFAULT_AUTO_LOCK_MINUTES, |m| m as u32)
}

/// Background task (architecture.md > Background tasks): locks the vault after idle time.
pub fn spawn_auto_lock(app: tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tauri::Manager;
    use tauri_specta::Event;

    use crate::events::{AutoLockWarning, LockReason, VaultLocked};
    use crate::state::AppState;

    const WARN_MS: i64 = 30_000;

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let state = app.state::<AppState>();
            if !state.is_unlocked() {
                continue;
            }
            let Ok(minutes) = state.with_db(|c| Ok(auto_lock_minutes(c))).await else {
                continue;
            };
            if minutes == 0 {
                continue;
            }
            let limit = i64::from(minutes) * 60_000;
            let idle = state.idle_ms();
            if idle >= limit {
                state.set_session(None);
                VaultLocked {
                    reason: LockReason::Idle,
                }
                .emit(&app)
                .ok();
            } else if idle >= limit - WARN_MS && !state.warned.swap(true, Ordering::Relaxed) {
                AutoLockWarning {
                    seconds_left: ((limit - idle) / 1000) as u32,
                }
                .emit(&app)
                .ok();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::secrets::{self, SecretKind};
    use super::*;
    use crate::db::open_in_memory;
    use crate::error::ErrorCode;

    fn fresh() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    #[test]
    fn create_then_unlock_roundtrip() {
        let (conn, s) = fresh();
        let again = unlock(&conn, "correct-horse-staple").unwrap();
        assert_eq!(again.vault_id, s.vault_id);
        assert_eq!(again.key.as_slice(), s.key.as_slice());
    }

    #[test]
    fn wrong_password_is_rejected() {
        let (conn, _) = fresh();
        let err = unlock(&conn, "nope-nope-nope").err().unwrap();
        assert_eq!(err.code, ErrorCode::WrongPassword);
    }

    #[test]
    fn short_password_and_second_vault_are_refused() {
        let conn = open_in_memory().unwrap();
        assert_eq!(
            create(&conn, "short", KdfParams::cheap())
                .err()
                .unwrap()
                .code,
            ErrorCode::Validation
        );
        create(&conn, "long-enough-pw", KdfParams::cheap()).unwrap();
        assert_eq!(
            create(&conn, "long-enough-pw", KdfParams::cheap())
                .err()
                .unwrap()
                .code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn secrets_roundtrip_and_are_not_plaintext_on_disk() {
        let (conn, s) = fresh();
        let id = secrets::put(&conn, &s, SecretKind::Password, b"hunter2").unwrap();
        assert_eq!(secrets::get(&conn, &s, &id).unwrap().as_slice(), b"hunter2");
        let raw: Vec<u8> = conn
            .query_row("SELECT ciphertext FROM secrets WHERE id = ?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(!raw.windows(7).any(|w| w == b"hunter2"));
        secrets::replace(&conn, &s, &id, b"new-secret").unwrap();
        assert_eq!(
            secrets::get(&conn, &s, &id).unwrap().as_slice(),
            b"new-secret"
        );
        secrets::delete(&conn, &s, &id).unwrap();
        assert_eq!(
            secrets::get(&conn, &s, &id).err().unwrap().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn ciphertext_moved_to_another_row_does_not_decrypt() {
        let (conn, s) = fresh();
        let a = secrets::put(&conn, &s, SecretKind::Password, b"aaa").unwrap();
        let b = secrets::put(&conn, &s, SecretKind::Password, b"bbb").unwrap();
        conn.execute(
            "UPDATE secrets SET nonce = (SELECT nonce FROM secrets WHERE id = ?1), ciphertext = (SELECT ciphertext FROM secrets WHERE id = ?1) WHERE id = ?2",
            [&a, &b],
        )
        .unwrap();
        assert!(secrets::get(&conn, &s, &b).is_err());
    }

    #[test]
    fn change_password_reencrypts_secrets() {
        let (mut conn, s) = fresh();
        let id = secrets::put(&conn, &s, SecretKind::Passphrase, b"my-passphrase").unwrap();
        let new_s = change_password(
            &mut conn,
            "correct-horse-staple",
            "a-brand-new-password",
            KdfParams::cheap(),
        )
        .unwrap();
        assert_eq!(
            unlock(&conn, "correct-horse-staple").err().unwrap().code,
            ErrorCode::WrongPassword
        );
        let again = unlock(&conn, "a-brand-new-password").unwrap();
        assert_eq!(
            secrets::get(&conn, &again, &id).unwrap().as_slice(),
            b"my-passphrase"
        );
        assert_eq!(new_s.key.as_slice(), again.key.as_slice());
    }

    #[test]
    fn change_password_with_wrong_old_password_changes_nothing() {
        let (mut conn, _) = fresh();
        let err = change_password(
            &mut conn,
            "wrong-old-password",
            "a-brand-new-password",
            KdfParams::cheap(),
        )
        .err()
        .unwrap();
        assert_eq!(err.code, ErrorCode::WrongPassword);
        assert!(unlock(&conn, "correct-horse-staple").is_ok());
    }

    #[test]
    fn auto_lock_defaults_to_15_and_reads_the_setting() {
        let conn = open_in_memory().unwrap();
        assert_eq!(auto_lock_minutes(&conn), 15);
        conn.execute("INSERT INTO settings (key, value, updated_at) VALUES ('security', '{\"auto_lock_minutes\":0}', 0)", []).unwrap();
        assert_eq!(auto_lock_minutes(&conn), 0);
    }
}
