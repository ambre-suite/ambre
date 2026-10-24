//! Argon2id passphrase-based key derivation.
//!
//! Derives the user key that unwraps the server-stored master key at login. The
//! parameters are explicit and versioned so they can evolve without breaking
//! existing accounts (the salt and version travel with the stored key material).
//!
//! The same work factor derives the key of a passphrase-sealed file
//! ([`crate::keys::FileKey::from_passphrase`]). That is the other place a
//! passphrase becomes a key, against the same offline guessing attack.

use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::CryptoError;

/// Length of the derived user key, in bytes.
pub const USER_KEY_LEN: usize = 32;

/// Argon2id parameters, v1. Tuned for a mid-range 2024 mobile SoC: ~64 MiB, 3
/// passes, single lane. Reviewed against `argon2` 0.5.3 (no relevant RUSTSEC
/// advisory). The single lane is deliberate: the KDF runs in a single-threaded
/// WASM Sync worker, where p > 1 adds sequential work and no speedup. Argon2's
/// memory-hardness is set by `MEM_KIB` (total memory), not by the lane count.
/// Parameters are versioned, so the lane count can be raised without breaking
/// existing accounts if a threaded target lands.
const MEM_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const LANES: u32 = 1;

/// Derives a 32-byte user key from `passphrase` and `salt` using Argon2id v1.
///
/// # Errors
///
/// [`CryptoError::Argon2Failed`] when Argon2id refuses the parameters or the
/// inputs. A [`MasterKeySalt`](ambre_core::MasterKeySalt) holds its fixed
/// width, so neither is reachable from a well-built salt.
pub fn derive_user_key(
    passphrase: &ambre_core::Passphrase,
    salt: &ambre_core::MasterKeySalt,
) -> Result<[u8; USER_KEY_LEN], CryptoError> {
    argon2id(passphrase, salt.as_bytes())
}

/// Argon2id v1 over raw salt bytes, under the parameters above. The kernel of
/// [`derive_user_key`] and of [`crate::keys::FileKey::from_passphrase`], each
/// of which names its own salt type.
pub(crate) fn argon2id(
    passphrase: &ambre_core::Passphrase,
    salt: &[u8],
) -> Result<[u8; USER_KEY_LEN], CryptoError> {
    let params = Params::new(MEM_KIB, ITERATIONS, LANES, Some(USER_KEY_LEN))
        .map_err(|_| CryptoError::Argon2Failed)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; USER_KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|_| CryptoError::Argon2Failed)?;
    Ok(out)
}
