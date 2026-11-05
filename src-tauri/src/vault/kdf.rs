//! Master password -> 32-byte vault key with Argon2id.

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};

pub const KEY_LEN: usize = 32;
/// `Zeroizing` wipes the bytes when the key is dropped (on lock).
pub type Key = Zeroizing<[u8; KEY_LEN]>;

/// Stored as JSON in `vaults.kdf_params`, so old vaults keep working if the defaults change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub alg: String,
    /// Memory in KiB.
    pub m: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            alg: "argon2id".into(),
            m: 65536,
            t: 3,
            p: 1,
        }
    }
}

impl KdfParams {
    /// Fast parameters so unit tests don't take seconds each. Never used outside tests.
    #[cfg(test)]
    pub fn cheap() -> Self {
        Self {
            alg: "argon2id".into(),
            m: 8,
            t: 1,
            p: 1,
        }
    }
}

pub fn derive_key(password: &[u8], salt: &[u8], params: &KdfParams) -> AppResult<Key> {
    if params.alg != "argon2id" {
        return Err(AppError::internal(format!(
            "unsupported KDF: {}",
            params.alg
        )));
    }
    let p = Params::new(params.m, params.t, params.p, Some(KEY_LEN))
        .map_err(|e| AppError::internal(format!("bad KDF params: {e}")))?;
    let mut out: Key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|e| AppError::internal(format!("key derivation failed: {e}")))?;
    Ok(out)
}
