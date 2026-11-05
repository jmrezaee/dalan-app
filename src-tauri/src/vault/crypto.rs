//! XChaCha20-Poly1305. The 24-byte random nonce is safe to generate per message.
//! `aad` (additional authenticated data) is not encrypted but is checked on open. We bind each
//! secret to its own row id, so a ciphertext copied into another row fails to decrypt.

use chacha20poly1305::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    XChaCha20Poly1305,
};
use zeroize::Zeroizing;

use super::kdf::Key;
use crate::error::{AppError, AppResult};

pub const NONCE_LEN: usize = 24;

pub fn seal(key: &Key, aad: &[u8], plaintext: &[u8]) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AppError::internal("encryption failed"))?;
    Ok((nonce.to_vec(), ct))
}

/// `None` means the key, nonce, aad or ciphertext is wrong. Callers decide which error that becomes
/// (wrong password at unlock, internal error for a stored secret).
pub fn open(key: &Key, aad: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if nonce.len() != NONCE_LEN {
        return None;
    }
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    cipher
        .decrypt(
            nonce.into(),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
        .map(Zeroizing::new)
}
