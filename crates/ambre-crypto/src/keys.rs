//! Secret key types and master-seed derivation.
//!
//! [`MasterSeed`] is the single root secret derived from the user's passphrase
//! at login; it deterministically yields the Ed25519 signing key and the X25519
//! exchange key via domain-separated BLAKE2b. [`DocumentKey`] is the symmetric
//! per-document key used for AEAD and the HMAC commitment tag.
//!
//! [`FileKey`] is the symmetric key of one passphrase-sealed `.ambre` file,
//! derived from the passphrase the user typed and the file's own salt. It is a
//! type of its own, so that no local file takes the document key the server
//! rotates.
//!
//! All three are `ZeroizeOnDrop`: their bytes are wiped when they leave scope.

use ambre_core::SALT_LEN;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::SigningKey;
use x25519_dalek::StaticSecret;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::aead;
use crate::error::CryptoError;
use crate::random;

/// Domain-separation tag for the Ed25519 signing key derivation.
const SIGN_DOMAIN: &[u8] = b"ambre-sign-v1";
/// Domain-separation tag for the X25519 exchange key derivation.
const EXCH_DOMAIN: &[u8] = b"ambre-exch-v1";
/// Domain-separation tag for the passphrase-sealed file key.
const FILE_DOMAIN: &[u8] = b"ambre-file-v1";

/// Derives 32 bytes from `domain || seed` via BLAKE2b, truncating the 64-byte
/// digest. Distinct domains yield independent keys from the same seed.
fn derive32(domain: &[u8], seed: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Blake2b512::new();
    hasher.update(domain);
    hasher.update(seed);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    out
}

/// The root secret of a user, decrypted at login from the server-stored,
/// passphrase-wrapped master key. Source of both keypairs.
///
/// Deliberately not `Clone`: the root seed is the single most sensitive secret
/// (recovery-word equivalent), so silent duplication of copies to zeroize is
/// disallowed. It is moved, never copied.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterSeed([u8; 32]);

impl MasterSeed {
    /// Generates a fresh master seed from the platform CSPRNG (account setup).
    pub fn generate() -> Self {
        Self(random::bytes::<32>())
    }

    /// Reconstructs a master seed from raw bytes (e.g. after decryption).
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derives the Ed25519 signing key (deterministic for a given seed).
    pub fn derive_signing_key(&self) -> SigningKey {
        let mut sk = derive32(SIGN_DOMAIN, &self.0);
        let key = SigningKey::from_bytes(&sk);
        sk.zeroize();
        key
    }

    /// Derives the X25519 exchange key (deterministic for a given seed).
    pub fn derive_exchange_key(&self) -> StaticSecret {
        let sk = derive32(EXCH_DOMAIN, &self.0);
        StaticSecret::from(sk)
    }

    /// Returns the raw seed bytes for recovery-word encoding. This is the one
    /// sanctioned escape hatch that exposes the root seed; its name flags that
    /// the caller is handling recovery material and must not retain it.
    pub fn recovery_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Seals the raw seed under `key` (the passphrase-derived user key) with the
    /// given `aad`, for storage as the server-side passphrase-wrapped master key.
    /// The raw seed never leaves this crate on this path.
    pub fn wrap_for_storage(
        &self,
        key: &DocumentKey,
        aad: &[u8],
    ) -> Result<([u8; aead::NONCE_LEN], Vec<u8>), CryptoError> {
        aead::seal(&self.0, aad, key)
    }
}

/// A 32-byte symmetric key scoped to a single document. Used for AEAD encryption
/// and to compute the HMAC-SHA256 key-commitment tag.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DocumentKey([u8; 32]);

impl DocumentKey {
    /// Generates a fresh document key from the platform CSPRNG.
    pub fn generate() -> Self {
        Self(random::bytes::<32>())
    }

    /// Builds a document key from raw bytes (e.g. after X25519 unwrap).
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw key bytes. Restricted to crate use (AEAD, HMAC, wrap).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A 32-byte symmetric key derived from a passphrase, scoped to a single sealed
/// `.ambre` file. Used for AEAD and nothing else.
///
/// **A distinct type from [`DocumentKey`], and that is a rule rather than a
/// preference** (`ambre-spec-docfile.md section 5`). Sealing a local file under
/// the key Sync holds for that document would tie its readability to a
/// server-side lifecycle. Every revocation rotates the document key, so a
/// backup under a since-rotated key stops opening, silently and months late.
/// Two types with no conversion between them means the mistake does not
/// compile -- neither AEAD entry point accepts the other's key.
///
/// It carries no [`Clone`]: a file key exists for one seal or one open, and a
/// copy is one more buffer to wipe.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct FileKey([u8; 32]);

impl FileKey {
    /// Derives the key that seals one file, from `passphrase` and that file's
    /// own random `salt`.
    ///
    /// Argon2id v1 at the login work factor (`crate::passphrase`), then one
    /// domain-separated BLAKE2b pass. The second step is what makes this key
    /// *not* the account's user key, which the same inputs would otherwise
    /// produce. Sealing a file under the account passphrase would put a value
    /// derived from the login secret on the disk, beside the ciphertext it
    /// protects.
    ///
    /// The cost is the point and it is not small: ~64 MiB and a few hundred
    /// milliseconds per call. It runs in the Sync worker, which holds an event
    /// loop of its own, so the interface keeps painting while it works
    /// (`ARCH-004`).
    pub fn from_passphrase(passphrase: &[u8], salt: &[u8]) -> Result<Self, CryptoError> {
        let mut intermediate = crate::passphrase::derive_user_key(passphrase, salt)?;
        let key = Self(derive32(FILE_DOMAIN, &intermediate));
        intermediate.zeroize();
        Ok(key)
    }

    /// Draws a fresh salt from the platform CSPRNG, derives the key of a file
    /// about to go out, and returns both. The sealed header carries the salt in
    /// the clear, and the caller needs it there.
    ///
    /// Sealing needs a salt no caller may choose, so the draw belongs on this
    /// side of the crate boundary, not behind a public CSPRNG accessor.
    /// The length is [`ambre_core::SALT_LEN`], the header field it fills.
    pub fn for_new_file(passphrase: &[u8]) -> Result<(Self, [u8; SALT_LEN]), CryptoError> {
        let salt = random::bytes::<SALT_LEN>();
        let key = Self::from_passphrase(passphrase, &salt)?;
        Ok((key, salt))
    }

    /// Returns the raw key bytes. Restricted to crate use (AEAD).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
