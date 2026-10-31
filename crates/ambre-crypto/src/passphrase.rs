//! Argon2id passphrase-based key derivation.
//!
//! Derives the user key that unwraps the server-stored master key at login. The
//! parameters are explicit and versioned so they can evolve without breaking
//! existing accounts (the salt and version travel with the stored key material).
//!
//! The same work factor derives the key of a passphrase-sealed file
//! ([`crate::keys::FileKey::from_passphrase`]). That is the other place a
//! passphrase becomes a key, against the same offline guessing attack.

use ambre_core::Secret;
use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::CryptoError;
use crate::keys::DocumentKey;

/// Length of the derived user key, in bytes.
pub const USER_KEY_LEN: usize = 32;

/// The Argon2id work factor of one parameter version: its memory, its passes,
/// its lanes and its output length. The salt and the version travel with the
/// stored key material, so a version never changes once an account uses it.
#[derive(Debug, Clone)]
struct Argon2Cost(Params);

impl Argon2Cost {
    /// Argon2id parameters, v1. Tuned for a mid-range 2024 mobile SoC: 64 MiB
    /// of memory, 3 passes, a single lane. Reviewed against `argon2` 0.5.3 (no
    /// relevant RUSTSEC advisory).
    ///
    /// The single lane is deliberate: the KDF runs in a single-threaded WASM
    /// Sync worker, where p > 1 adds sequential work and no speedup. Argon2's
    /// memory-hardness is set by the total memory, not by the lane count.
    /// Parameters are versioned, so the lane count can be raised without
    /// breaking existing accounts if a threaded target lands.
    ///
    /// The match runs at compile time, so a refused parameter is a build error.
    const V1: Self = match Params::new(64 * 1024, 3, 1, Some(USER_KEY_LEN)) {
        Ok(params) => Self(params),
        Err(_) => panic!("the v1 Argon2id parameters are in range"),
    };
}

/// The key a passphrase derives to unwrap the server-stored master key. It is
/// wiped on drop and prints none of its bytes.
#[derive(Debug)]
pub struct UserKey(Secret<[u8; USER_KEY_LEN]>);

impl UserKey {
    /// The key as the AEAD that wraps the master seed reads it.
    pub fn document_key(&self) -> DocumentKey {
        DocumentKey::from_secret(self.0.clone_secret())
    }

    /// The raw key bytes, for a test that compares two derivations.
    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8; USER_KEY_LEN] {
        self.0.expose()
    }
}

/// Derives a 32-byte user key from `passphrase` and `salt` using Argon2id v1.
///
/// # Errors
///
/// [`CryptoError::Argon2Failed`] when Argon2id refuses the inputs. The
/// parameters are checked at compile time, and a
/// [`MasterKeySalt`](ambre_core::MasterKeySalt) holds its fixed width, so
/// neither failure is reachable from a well-built salt.
pub fn derive_user_key(
    passphrase: &ambre_core::Passphrase,
    salt: &ambre_core::FixedBytes<ambre_core::MasterKeySalt>,
) -> Result<UserKey, CryptoError> {
    argon2id(passphrase, salt.as_slice()).map(UserKey)
}

/// Argon2id v1 over raw salt bytes, under the parameters above. The kernel of
/// [`derive_user_key`] and of [`crate::keys::FileKey::from_passphrase`], each
/// of which names its own salt type. The output is secret, so it is wiped
/// when the caller drops it.
pub(crate) fn argon2id(
    passphrase: &ambre_core::Passphrase,
    salt: &[u8],
) -> Result<Secret<[u8; USER_KEY_LEN]>, CryptoError> {
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Argon2Cost::V1.0);
    Secret::try_filled(|out: &mut [u8; USER_KEY_LEN]| {
        argon2
            .hash_password_into(passphrase.secret().expose().as_bytes(), salt, out)
            .map_err(|_| CryptoError::Argon2Failed)
    })
}
