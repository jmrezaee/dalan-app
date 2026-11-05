//! The keychain (S05-S08, F02): SSH keys and identities. A private key lives in `secrets` as
//! OpenSSH text (encrypted with the user's passphrase if they gave one). Only the public part,
//! the fingerprint and flags ever reach the UI.

use rusqlite::{params, Connection, OptionalExtension};
use russh::keys::ssh_key::certificate::CertType;
use russh::keys::ssh_key::{Algorithm, Certificate, EcdsaCurve, LineEnding, PrivateKey, PublicKey};
use russh::keys::{decode_secret_key, HashAlg};
use serde::{Deserialize, Serialize};
use specta::Type;
use zeroize::Zeroizing;

use crate::db::now_ms;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::groups::next_secret;
use crate::vault::secrets::{self, SecretKind};
use crate::vault::VaultSession;

const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    Rsa4096,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct KeySummary {
    pub id: String,
    pub label: String,
    /// `ed25519`, `ecdsa-p256`, `ecdsa-p384`, `ecdsa-p521` or `rsa`.
    pub key_type: String,
    pub bits: Option<u32>,
    /// OpenSSH one-line public key. Safe to show and copy.
    pub public_key: String,
    pub fingerprint: String,
    /// The private key is protected by a passphrase.
    pub encrypted: bool,
    /// ...and Dalan remembers it (otherwise it asks on every connect).
    pub passphrase_saved: bool,
    pub certificate: bool,
    /// A line about the attached certificate: who it is for and when it ends.
    pub certificate_note: Option<String>,
}

#[derive(Debug, Deserialize, Type)]
pub struct GenerateInput {
    pub label: String,
    pub kind: KeyKind,
    pub passphrase: Option<String>,
    pub save_passphrase: bool,
}

#[derive(Debug, Deserialize, Type)]
pub struct ImportInput {
    pub label: String,
    /// Pasted key text.
    pub pem: Option<String>,
    /// A file chosen in the dialog. Read here in Rust, not in the webview.
    pub path: Option<String>,
    pub passphrase: Option<String>,
    pub save_passphrase: bool,
}

/// A key ready to store: everything computed, nothing written yet. Built outside the DB lock
/// because RSA generation can take seconds.
pub struct NewKey {
    label: String,
    key_type: &'static str,
    bits: Option<u32>,
    public_key: String,
    fingerprint: String,
    private_text: Zeroizing<String>,
    passphrase: Option<Zeroizing<String>>,
    save_passphrase: bool,
}

fn clean_label(label: &str) -> AppResult<String> {
    let l = label.trim();
    if l.is_empty() {
        return Err(AppError::validation("Give the key a label."));
    }
    Ok(l.to_string())
}

fn describe(k: &PrivateKey) -> AppResult<(&'static str, Option<u32>)> {
    Ok(match k.algorithm() {
        Algorithm::Ed25519 => ("ed25519", None),
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        } => ("ecdsa-p256", None),
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        } => ("ecdsa-p384", None),
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        } => ("ecdsa-p521", None),
        Algorithm::Rsa { .. } => ("rsa", k.public_key().key_data().rsa().map(|r| r.key_size())),
        _ => {
            return Err(AppError::new(
                ErrorCode::InvalidKey,
                "That key type isn't supported yet.",
            ))
        }
    })
}

/// Turns a (decrypted) private key into a storable record, re-encrypting it if a passphrase was given.
fn package(
    mut key: PrivateKey,
    label: &str,
    passphrase: Option<&str>,
    save: bool,
) -> AppResult<NewKey> {
    let label = clean_label(label)?;
    let (key_type, bits) = describe(&key)?;
    key.set_comment(label.as_str());
    let pass = passphrase.filter(|p| !p.is_empty());
    let stored = match pass {
        Some(p) => key
            .encrypt(&mut rand::rng(), p)
            .map_err(|e| AppError::internal(format!("couldn't encrypt the key: {e}")))?,
        None => key.clone(),
    };
    let private_text = stored
        .to_openssh(LineEnding::LF)
        .map_err(|e| AppError::internal(format!("couldn't encode the key: {e}")))?;
    Ok(NewKey {
        label,
        key_type,
        bits,
        public_key: key
            .public_key()
            .to_openssh()
            .map_err(|e| AppError::internal(e.to_string()))?,
        fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        private_text,
        passphrase: pass.map(|p| Zeroizing::new(p.to_string())),
        save_passphrase: save,
    })
}

/// CPU-heavy for RSA: call from `spawn_blocking`, not under the DB lock.
pub fn generate(input: &GenerateInput) -> AppResult<NewKey> {
    clean_label(&input.label)?;
    let algo = match input.kind {
        KeyKind::Ed25519 => Algorithm::Ed25519,
        KeyKind::EcdsaP256 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        KeyKind::EcdsaP384 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        },
        KeyKind::EcdsaP521 => Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        },
        KeyKind::Rsa4096 => Algorithm::Rsa { hash: None },
    };
    let key = PrivateKey::random(&mut rand::rng(), algo)
        .map_err(|e| AppError::internal(format!("couldn't generate a key: {e}")))?;
    package(
        key,
        &input.label,
        input.passphrase.as_deref(),
        input.save_passphrase,
    )
}

/// Parses pasted text or a chosen file. Encrypted keys need `passphrase`, else `needs_passphrase`.
pub fn parse_import(input: &ImportInput) -> AppResult<NewKey> {
    clean_label(&input.label)?;
    let text: Zeroizing<String> = match (&input.pem, &input.path) {
        (Some(p), _) if !p.trim().is_empty() => Zeroizing::new(p.clone()),
        (_, Some(path)) => {
            let meta = std::fs::metadata(path)
                .map_err(|_| AppError::validation("Couldn't read that file."))?;
            if meta.len() > MAX_KEY_FILE_BYTES {
                return Err(AppError::new(
                    ErrorCode::InvalidKey,
                    "That file is too big to be a private key.",
                ));
            }
            Zeroizing::new(std::fs::read_to_string(path).map_err(|_| {
                AppError::new(ErrorCode::InvalidKey, "That file isn't a private key.")
            })?)
        }
        _ => {
            return Err(AppError::validation(
                "Paste a private key or choose a file.",
            ))
        }
    };
    let pass = input.passphrase.as_deref().filter(|p| !p.is_empty());
    let key = decode_secret_key(&text, pass).map_err(|e| match (&e, pass) {
        (russh::keys::Error::KeyIsEncrypted, None) => AppError::new(
            ErrorCode::NeedsPassphrase,
            "This key is protected by a passphrase.",
        ),
        (_, Some(_))
            if text.contains("ENCRYPTED") || text.contains("BEGIN OPENSSH PRIVATE KEY") =>
        {
            AppError::new(ErrorCode::WrongPassword, "That passphrase didn't work.")
        }
        (_, None) if text.contains("ENCRYPTED") => AppError::new(
            ErrorCode::NeedsPassphrase,
            "This key is protected by a passphrase.",
        ),
        _ => AppError::new(
            ErrorCode::InvalidKey,
            "That doesn't look like a private key Dalan can read.",
        ),
    })?;
    package(key, &input.label, pass, input.save_passphrase)
}

/// Writes the key (and its saved passphrase, if asked) in one transaction.
pub fn store(conn: &mut Connection, s: &VaultSession, k: &NewKey) -> AppResult<KeySummary> {
    let dup: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM keys WHERE vault_id = ?1 AND fingerprint_sha256 = ?2)",
        params![s.vault_id, k.fingerprint],
        |r| r.get(0),
    )?;
    if dup {
        return Err(AppError::validation(
            "That key is already in your keychain.",
        ));
    }
    let id = uuid::Uuid::now_v7().to_string();
    let now = now_ms();
    let tx = conn.transaction()?;
    let private_id = secrets::put(&tx, s, SecretKind::PrivateKey, k.private_text.as_bytes())?;
    let pass_id = match (&k.passphrase, k.save_passphrase) {
        (Some(p), true) => Some(secrets::put(&tx, s, SecretKind::Passphrase, p.as_bytes())?),
        _ => None,
    };
    tx.execute(
        "INSERT INTO keys (id, vault_id, label, key_type, bits, public_key, fingerprint_sha256, private_key_secret_id, passphrase_secret_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![id, s.vault_id, k.label, k.key_type, k.bits, k.public_key, k.fingerprint, private_id, pass_id, now],
    )?;
    tx.commit()?;
    get(conn, s, &id)
}

const SELECT_KEY: &str = "SELECT id, label, key_type, bits, public_key, fingerprint_sha256, private_key_secret_id, passphrase_secret_id, certificate FROM keys";

fn row_to_summary(conn: &Connection, s: &VaultSession, r: &rusqlite::Row) -> AppResult<KeySummary> {
    let private_id: Option<String> = r.get(6)?;
    let encrypted = match private_id {
        Some(id) => {
            let text = secrets::get(conn, s, &id)?;
            PrivateKey::from_openssh(text.as_slice())
                .map(|k| k.is_encrypted())
                .unwrap_or(false)
        }
        None => false,
    };
    let cert: Option<String> = r.get(8)?;
    Ok(KeySummary {
        id: r.get(0)?,
        label: r.get(1)?,
        key_type: r.get(2)?,
        bits: r.get(3)?,
        public_key: r.get(4)?,
        fingerprint: r.get(5)?,
        encrypted,
        passphrase_saved: r.get::<_, Option<String>>(7)?.is_some(),
        certificate: cert.is_some(),
        certificate_note: cert
            .as_deref()
            .and_then(|c| Certificate::from_openssh(c).ok())
            .map(|c| certificate_note(&c)),
    })
}

pub fn list(conn: &Connection, s: &VaultSession) -> AppResult<Vec<KeySummary>> {
    let mut st = conn.prepare(&format!(
        "{SELECT_KEY} WHERE vault_id = ?1 ORDER BY label COLLATE NOCASE"
    ))?;
    let mut rows = st.query([&s.vault_id])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(row_to_summary(conn, s, r)?);
    }
    Ok(out)
}

pub fn get(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<KeySummary> {
    let mut st = conn.prepare(&format!("{SELECT_KEY} WHERE id = ?1 AND vault_id = ?2"))?;
    let mut rows = st.query(params![id, s.vault_id])?;
    match rows.next()? {
        Some(r) => row_to_summary(conn, s, r),
        None => Err(AppError::not_found("That key no longer exists.")),
    }
}

#[derive(Debug, Deserialize, Type)]
pub struct KeyUpdate {
    pub id: String,
    pub label: String,
    /// Remember this passphrase (checked against the key first). `None` leaves the saved one alone.
    pub save_passphrase: Option<String>,
    pub forget_passphrase: bool,
}

pub fn update(conn: &mut Connection, s: &VaultSession, u: &KeyUpdate) -> AppResult<KeySummary> {
    let label = clean_label(&u.label)?;
    let (private_id, old_pass): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT private_key_secret_id, passphrase_secret_id FROM keys WHERE id = ?1 AND vault_id = ?2",
            params![u.id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That key no longer exists."))?;
    let new_pass = u.save_passphrase.as_deref().filter(|p| !p.is_empty());
    if let (Some(p), Some(pid)) = (new_pass, &private_id) {
        // Only remember a passphrase that actually opens the key.
        let text = secrets::get(conn, s, pid)?;
        decode_secret_key(&String::from_utf8_lossy(&text), Some(p))
            .map_err(|_| AppError::new(ErrorCode::WrongPassword, "That passphrase didn't work."))?;
    }
    let tx = conn.transaction()?;
    let (pass_id, orphan) = next_secret(&tx, s, old_pass, new_pass, u.forget_passphrase)?;
    tx.execute(
        "UPDATE keys SET label = ?1, passphrase_secret_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![label, pass_id, now_ms(), u.id],
    )?;
    if let Some(o) = orphan {
        secrets::delete(&tx, s, &o)?;
    }
    tx.commit()?;
    get(conn, s, &u.id)
}

/// Removes the key and its secrets. Profiles and identities that used it fall back to inheriting (`SET NULL`).
pub fn delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let (private_id, pass_id): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT private_key_secret_id, passphrase_secret_id FROM keys WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That key no longer exists."))?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM keys WHERE id = ?1", [id])?;
    for sid in private_id.into_iter().chain(pass_id) {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

/// What a session needs to sign in with a key. The private text and passphrase are wiped on drop.
pub struct LoadedKey {
    pub label: String,
    pub private_text: Zeroizing<String>,
    pub saved_passphrase: Option<Zeroizing<String>>,
    pub encrypted: bool,
    /// An OpenSSH user certificate for this key, if one is attached.
    pub certificate: Option<String>,
}

pub fn load_for_auth(conn: &Connection, s: &VaultSession, id: &str) -> AppResult<LoadedKey> {
    let (label, private_id, pass_id, certificate): (String, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT label, private_key_secret_id, passphrase_secret_id, certificate FROM keys WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That key no longer exists."))?;
    let private_id = private_id.ok_or_else(|| AppError::internal("the key has no private part"))?;
    let text = secrets::get(conn, s, &private_id)?;
    let private_text = Zeroizing::new(String::from_utf8_lossy(&text).into_owned());
    let encrypted = PrivateKey::from_openssh(private_text.as_bytes())
        .map(|k| k.is_encrypted())
        .unwrap_or(false);
    let saved_passphrase = match pass_id {
        Some(p) => Some(Zeroizing::new(
            String::from_utf8_lossy(&secrets::get(conn, s, &p)?).into_owned(),
        )),
        None => None,
    };
    Ok(LoadedKey {
        label,
        private_text,
        saved_passphrase,
        encrypted,
        certificate,
    })
}

/// Decodes the stored key with a passphrase (when it has one).
pub fn open_private(k: &LoadedKey, passphrase: Option<&str>) -> AppResult<PrivateKey> {
    decode_secret_key(&k.private_text, passphrase.filter(|p| !p.is_empty()))
        .map_err(|_| AppError::new(ErrorCode::WrongPassword, "That passphrase didn't work."))
}

// ------------------------------------------------------------------ certificates

/// "For deploy, ops. Expires in 12 days." (an empty principal list means any user).
fn certificate_note(c: &Certificate) -> String {
    let who = if c.valid_principals().is_empty() {
        "any user".to_string()
    } else {
        c.valid_principals().join(", ")
    };
    let end = match c.valid_before() {
        u64::MAX => "No expiry.".to_string(),
        vb => {
            let left = i128::from(vb) - i128::from(now_ms() / 1000);
            if left <= 0 {
                "Expired.".to_string()
            } else if left < 86_400 {
                "Expires today.".to_string()
            } else {
                format!("Expires in {} days.", left / 86_400)
            }
        }
    };
    format!("For {who}. {end}")
}

/// True when the certificate's validity window has ended.
pub fn certificate_expired(c: &Certificate) -> bool {
    c.valid_before() != u64::MAX && c.valid_before() <= (now_ms() / 1000) as u64
}

/// Attaches an OpenSSH user certificate (`id_ed25519-cert.pub`) to a key, or removes it with
/// `None`. The certificate must be for this key and for logging in (not a host certificate).
pub fn set_certificate(
    conn: &Connection,
    s: &VaultSession,
    id: &str,
    text: Option<&str>,
) -> AppResult<KeySummary> {
    let public: String = conn
        .query_row(
            "SELECT public_key FROM keys WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That key no longer exists."))?;
    let stored = match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => None,
        Some(text) => {
            let cert = Certificate::from_openssh(text).map_err(|_| {
                AppError::validation("That isn't an OpenSSH certificate. It should start with something like ssh-ed25519-cert-v01@openssh.com.")
            })?;
            if cert.cert_type() != CertType::User {
                return Err(AppError::validation(
                    "That is a host certificate. Sign-in needs a user certificate.",
                ));
            }
            let key = PublicKey::from_openssh(&public)
                .map_err(|_| AppError::internal("the stored public key is unreadable"))?;
            if cert.public_key() != key.key_data() {
                return Err(AppError::validation(
                    "That certificate was made for a different key.",
                ));
            }
            if certificate_expired(&cert) {
                return Err(AppError::validation(
                    "That certificate has already expired.",
                ));
            }
            Some(
                cert.to_openssh()
                    .map_err(|e| AppError::internal(e.to_string()))?,
            )
        }
    };
    conn.execute(
        "UPDATE keys SET certificate = ?1, updated_at = ?2 WHERE id = ?3 AND vault_id = ?4",
        params![stored, now_ms(), id, s.vault_id],
    )?;
    get(conn, s, id)
}

// ------------------------------------------------------------------ identities (S07)

#[derive(Debug, Clone, Serialize, Type)]
pub struct IdentitySummary {
    pub id: String,
    pub label: String,
    pub username: String,
    pub has_password: bool,
    pub key_id: Option<String>,
}

#[derive(Debug, Deserialize, Type)]
pub struct IdentityInput {
    pub id: Option<String>,
    pub label: String,
    pub username: String,
    pub password: Option<String>,
    pub clear_password: bool,
    pub key_id: Option<String>,
}

pub fn identities_list(conn: &Connection, vault_id: &str) -> AppResult<Vec<IdentitySummary>> {
    let mut st = conn.prepare(
        "SELECT id, label, username, password_secret_id IS NOT NULL, key_id FROM identities WHERE vault_id = ?1 ORDER BY label COLLATE NOCASE",
    )?;
    let rows = st
        .query_map([vault_id], |r| {
            Ok(IdentitySummary {
                id: r.get(0)?,
                label: r.get(1)?,
                username: r.get(2)?,
                has_password: r.get(3)?,
                key_id: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

pub fn identity_save(
    conn: &mut Connection,
    s: &VaultSession,
    i: &IdentityInput,
) -> AppResult<IdentitySummary> {
    let label = i.label.trim().to_string();
    let username = i.username.trim().to_string();
    if label.is_empty() || username.is_empty() {
        return Err(AppError::validation(
            "Give the identity a label and a username.",
        ));
    }
    let password = i.password.as_deref().filter(|p| !p.is_empty());
    let now = now_ms();
    let tx = conn.transaction()?;
    let id = match &i.id {
        None => {
            let id = uuid::Uuid::now_v7().to_string();
            let (secret, _) = next_secret(&tx, s, None, password, false)?;
            tx.execute(
                "INSERT INTO identities (id, vault_id, label, username, password_secret_id, key_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id, s.vault_id, label, username, secret, i.key_id, now],
            )?;
            id
        }
        Some(id) => {
            let old: Option<String> = tx
                .query_row(
                    "SELECT password_secret_id FROM identities WHERE id = ?1 AND vault_id = ?2",
                    params![id, s.vault_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| AppError::not_found("That identity no longer exists."))?;
            let (secret, orphan) = next_secret(&tx, s, old, password, i.clear_password)?;
            tx.execute(
                "UPDATE identities SET label = ?1, username = ?2, password_secret_id = ?3, key_id = ?4, updated_at = ?5 WHERE id = ?6",
                params![label, username, secret, i.key_id, now, id],
            )?;
            if let Some(o) = orphan {
                secrets::delete(&tx, s, &o)?;
            }
            id.clone()
        }
    };
    tx.commit()?;
    identities_list(conn, &s.vault_id)?
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| AppError::internal("saved identity is missing"))
}

pub fn identity_delete(conn: &mut Connection, s: &VaultSession, id: &str) -> AppResult<()> {
    let old: Option<String> = conn
        .query_row(
            "SELECT password_secret_id FROM identities WHERE id = ?1 AND vault_id = ?2",
            params![id, s.vault_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("That identity no longer exists."))?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM identities WHERE id = ?1", [id])?;
    if let Some(sid) = old {
        secrets::delete(&tx, s, &sid)?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};

    fn fresh() -> (Connection, VaultSession) {
        let conn = open_in_memory().unwrap();
        let s = vault::create(&conn, "correct-horse-staple", KdfParams::cheap()).unwrap();
        (conn, s)
    }

    fn gen(label: &str, kind: KeyKind, pass: Option<&str>, save: bool) -> NewKey {
        generate(&GenerateInput {
            label: label.into(),
            kind,
            passphrase: pass.map(String::from),
            save_passphrase: save,
        })
        .unwrap()
    }

    #[test]
    fn generated_key_is_stored_encrypted_and_summarised_without_secrets() {
        let (mut c, s) = fresh();
        let k = store(&mut c, &s, &gen("deploy", KeyKind::Ed25519, None, false)).unwrap();
        assert_eq!(k.key_type, "ed25519");
        assert!(k.public_key.starts_with("ssh-ed25519 ") && k.public_key.ends_with(" deploy"));
        assert!(k.fingerprint.starts_with("SHA256:"));
        assert!(!k.encrypted && !k.passphrase_saved);
        let raw: Vec<u8> = c
            .query_row("SELECT ciphertext FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&raw).contains("OPENSSH PRIVATE KEY"),
            "private key must be ciphertext at rest"
        );
        // The stored key still signs in: it decodes back to the same public key.
        let loaded = load_for_auth(&c, &s, &k.id).unwrap();
        let back = open_private(&loaded, None).unwrap();
        assert_eq!(
            back.public_key().fingerprint(HashAlg::Sha256).to_string(),
            k.fingerprint
        );
    }

    #[test]
    fn passphrase_protects_the_key_and_can_be_saved_or_forgotten() {
        let (mut c, s) = fresh();
        let k = store(
            &mut c,
            &s,
            &gen("locked", KeyKind::EcdsaP256, Some("hunter2-hunter2"), true),
        )
        .unwrap();
        assert!(k.encrypted && k.passphrase_saved);
        let loaded = load_for_auth(&c, &s, &k.id).unwrap();
        assert!(
            open_private(&loaded, None).is_err(),
            "no passphrase, no key"
        );
        assert!(open_private(&loaded, Some("wrong")).is_err());
        assert!(open_private(
            &loaded,
            loaded.saved_passphrase.as_deref().map(String::as_str)
        )
        .is_ok());
        let k = update(
            &mut c,
            &s,
            &KeyUpdate {
                id: k.id.clone(),
                label: "locked2".into(),
                save_passphrase: None,
                forget_passphrase: true,
            },
        )
        .unwrap();
        assert_eq!((k.label.as_str(), k.passphrase_saved), ("locked2", false));
        // Saving a wrong passphrase is refused; the right one is accepted.
        let bad = update(
            &mut c,
            &s,
            &KeyUpdate {
                id: k.id.clone(),
                label: "x".into(),
                save_passphrase: Some("nope".into()),
                forget_passphrase: false,
            },
        );
        assert_eq!(bad.err().unwrap().code, ErrorCode::WrongPassword);
        let ok = update(
            &mut c,
            &s,
            &KeyUpdate {
                id: k.id,
                label: "x".into(),
                save_passphrase: Some("hunter2-hunter2".into()),
                forget_passphrase: false,
            },
        )
        .unwrap();
        assert!(ok.passphrase_saved);
    }

    #[test]
    fn import_handles_encrypted_keys_and_garbage() {
        let (mut c, s) = fresh();
        // Export a generated encrypted key as text, then import it the way a user would paste it.
        let g = gen("src", KeyKind::Ed25519, Some("pw-pw-pw-pw"), false);
        let text = g.private_text.to_string();
        let imp = |pass: Option<&str>| {
            parse_import(&ImportInput {
                label: "imported".into(),
                pem: Some(text.clone()),
                path: None,
                passphrase: pass.map(String::from),
                save_passphrase: false,
            })
        };
        assert_eq!(imp(None).err().unwrap().code, ErrorCode::NeedsPassphrase);
        assert_eq!(
            imp(Some("bad")).err().unwrap().code,
            ErrorCode::WrongPassword
        );
        let k = store(&mut c, &s, &imp(Some("pw-pw-pw-pw")).unwrap()).unwrap();
        assert!(k.encrypted);
        // Same key again is a duplicate.
        assert!(store(&mut c, &s, &imp(Some("pw-pw-pw-pw")).unwrap()).is_err());
        let junk = parse_import(&ImportInput {
            label: "j".into(),
            pem: Some("not a key".into()),
            path: None,
            passphrase: None,
            save_passphrase: false,
        });
        assert_eq!(junk.err().unwrap().code, ErrorCode::InvalidKey);
    }

    #[test]
    fn deleting_a_key_removes_its_secrets_and_detaches_users() {
        let (mut c, s) = fresh();
        let k = store(
            &mut c,
            &s,
            &gen("k", KeyKind::Ed25519, Some("pw-pw-pw-pw"), true),
        )
        .unwrap();
        let ident = identity_save(
            &mut c,
            &s,
            &IdentityInput {
                id: None,
                label: "deploy".into(),
                username: "deploy".into(),
                password: None,
                clear_password: false,
                key_id: Some(k.id.clone()),
            },
        )
        .unwrap();
        assert_eq!(ident.key_id.as_deref(), Some(k.id.as_str()));
        delete(&mut c, &s, &k.id).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert_eq!(identities_list(&c, &s.vault_id).unwrap()[0].key_id, None);
    }

    #[test]
    fn identity_password_replace_and_clear() {
        let (mut c, s) = fresh();
        let mut i = IdentityInput {
            id: None,
            label: "a".into(),
            username: "u".into(),
            password: Some("secret-pw-1".into()),
            clear_password: false,
            key_id: None,
        };
        let made = identity_save(&mut c, &s, &i).unwrap();
        assert!(made.has_password);
        i.id = Some(made.id.clone());
        i.password = Some("secret-pw-2".into());
        identity_save(&mut c, &s, &i).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        i.password = None;
        i.clear_password = true;
        assert!(!identity_save(&mut c, &s, &i).unwrap().has_password);
        identity_delete(&mut c, &s, &made.id).unwrap();
        assert!(identities_list(&c, &s.vault_id).unwrap().is_empty());
    }

    #[test]
    fn identity_credentials_are_inherited_like_group_ones() {
        use crate::groups::{chain, load, load_identities, resolve, Level};
        let (mut c, s) = fresh();
        let k = store(&mut c, &s, &gen("k", KeyKind::Ed25519, None, false)).unwrap();
        let ident = identity_save(
            &mut c,
            &s,
            &IdentityInput {
                id: None,
                label: "deploy".into(),
                username: "deploy".into(),
                password: None,
                clear_password: false,
                key_id: Some(k.id.clone()),
            },
        )
        .unwrap();
        // A group that sets only an identity supplies both username and key to hosts below it.
        c.execute("INSERT INTO connection_profiles (id, vault_id, identity_id, created_at, updated_at) VALUES ('p1', ?1, ?2, 0, 0)", params![s.vault_id, ident.id]).unwrap();
        c.execute("INSERT INTO groups (id, vault_id, profile_id, label, created_at, updated_at) VALUES ('g1', ?1, 'p1', 'Prod', 0, 0)", [&s.vault_id]).unwrap();
        let (map, ids) = (
            load(&c, &s.vault_id).unwrap(),
            load_identities(&c, &s.vault_id).unwrap(),
        );
        let mut levels = vec![Level::default()]; // a host that sets nothing itself
        levels.extend(chain(&map, Some("g1")));
        let r = resolve(&levels, &ids);
        assert_eq!(r.username.unwrap().value, "deploy");
        let key = r.key_id.unwrap();
        assert_eq!(
            (key.value.as_str(), key.from.as_deref()),
            (k.id.as_str(), Some("Prod"))
        );
    }

    #[test]
    fn certificates_are_checked_before_they_are_attached() {
        use crate::ssh::tests::sign_user_cert;
        let (mut c, s) = fresh();
        let mk = |c: &mut Connection, label: &str| {
            let new = generate(&GenerateInput {
                label: label.into(),
                kind: KeyKind::Ed25519,
                passphrase: None,
                save_passphrase: false,
            })
            .unwrap();
            store(c, &s, &new).unwrap()
        };
        let (a, b) = (mk(&mut c, "a"), mk(&mut c, "b"));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let good = sign_user_cert(
            &a.public_key,
            (now - 60, now + 5 * 86_400 + 3_600),
            CertType::User,
        );
        let err = |r: AppResult<KeySummary>| r.err().unwrap().message;
        assert!(err(set_certificate(&c, &s, &a.id, Some("nonsense")))
            .contains("isn't an OpenSSH certificate"));
        assert!(err(set_certificate(&c, &s, &b.id, Some(&good))).contains("different key")); // made for a
        let host = sign_user_cert(&a.public_key, (now - 60, now + 3600), CertType::Host);
        assert!(err(set_certificate(&c, &s, &a.id, Some(&host))).contains("host certificate"));
        let old = sign_user_cert(&a.public_key, (now - 7200, now - 3600), CertType::User);
        assert!(err(set_certificate(&c, &s, &a.id, Some(&old))).contains("expired"));
        let k = set_certificate(&c, &s, &a.id, Some(&good)).unwrap();
        assert!(k.certificate);
        let note = k.certificate_note.unwrap();
        assert!(
            note.contains("For test") && note.contains("Expires in 5 days"),
            "{note}"
        );
        assert!(load_for_auth(&c, &s, &a.id).unwrap().certificate.is_some());
        assert!(!set_certificate(&c, &s, &a.id, None).unwrap().certificate); // removed
    }
}
