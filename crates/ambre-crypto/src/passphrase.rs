//! Argon2id passphrase-based key derivation.
//!
//! Derives the user key that unwraps the server-stored master key at login. The
//! parameters are explicit and versioned so they can evolve without breaking
//! existing accounts (the salt and version travel with the stored key material).

use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::CryptoError;

/// Length of the derived user key, in bytes.
pub const USER_KEY_LEN: usize = 32;

/// Argon2id parameters, v1. Tuned for a mid-range 2024 mobile SoC: ~64 MiB,
/// 3 passes, single lane.
const MEM_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const LANES: u32 = 1;

/// Derives a 32-byte user key from `passphrase` and `salt` using Argon2id v1.
pub fn derive_user_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; USER_KEY_LEN], CryptoError> {
    let params = Params::new(MEM_KIB, ITERATIONS, LANES, Some(USER_KEY_LEN))
        .map_err(|_| CryptoError::Argon2Failed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; USER_KEY_LEN];
    argon2
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|_| CryptoError::Argon2Failed)?;
    Ok(out)
}
