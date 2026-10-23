//! Secret key types and master-seed derivation.
//!
//! [`MasterSeed`] is the single root secret derived from the user's passphrase
//! at login. It deterministically yields the Ed25519 signing key and the X25519
//! exchange key via domain-separated BLAKE2b. [`DocumentKey`] is the symmetric
//! per-document key used for AEAD and the HMAC commitment tag.
//!
//! [`FileKey`] is the symmetric key of one passphrase-sealed `.ambre` file,
//! derived from the passphrase the user typed and the file's own salt. It is a
//! type of its own, so that no local file takes the document key the server
//! rotates.
//!
//! All three are `ZeroizeOnDrop`: their bytes are wiped when they leave scope.

use ambre_core::{FileSalt, KeyId, SALT_LEN, WrappedMasterKey};
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
/// Domain-separation tag for the identifier of a document key.
const KEY_ID_DOMAIN: &[u8] = b"ambre-key-id-v1";
/// Domain-separation tag for the key that seals the local store.
const STORE_DOMAIN: &[u8] = b"ambre-store-v1";

/// Derives 32 bytes from `domain || seed` via BLAKE2b, truncating the 64-byte
/// digest. Distinct domains yield independent keys from the same seed.
fn derive32(domain: &[u8], seed: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Blake2b512::new();
    hasher.update(domain);
    hasher.update(seed);
    crate::hash::prefix(&hasher.finalize())
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

    /// Reconstructs a master seed from raw bytes (for example after decryption).
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

    /// Derives the key that seals the rows of the local store
    /// (`ambre-spec-residency.md`, `RESID-025`). The same seed derives it on
    /// every device and every sign-in, so nothing of it is ever written down.
    pub fn derive_store_key(&self) -> StoreKey {
        StoreKey(derive32(STORE_DOMAIN, &self.0))
    }

    /// Returns the raw seed bytes for recovery-word encoding. This is the one
    /// sanctioned escape hatch that exposes the root seed. Its name flags that
    /// the caller is handling recovery material and must not retain it.
    pub fn recovery_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Seals the raw seed under `key` (the passphrase-derived user key) with the
    /// given `aad`, for storage as the server-side passphrase-wrapped master key.
    /// The raw seed never leaves this crate on this path.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EncryptionFailed`] from the AEAD.
    pub fn wrap_for_storage(
        &self,
        key: &DocumentKey,
        aad: &ambre_core::Aad,
    ) -> Result<WrappedMasterKey, CryptoError> {
        let (nonce, ciphertext) = aead::seal(&self.0, aad, key)?;
        let mut wrapped = nonce.0.to_vec();
        wrapped.extend_from_slice(&ciphertext.0);
        Ok(WrappedMasterKey::new(wrapped))
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

    /// Builds a document key from raw bytes (for example after X25519 unwrap).
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw key bytes. Restricted to crate use (AEAD, HMAC, wrap).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The identifier of this key: `BLAKE2b(domain || key)`, truncated
    /// (`SYNC-115`). It is derived from the key, so two keys never share one
    /// in practice, and it reveals nothing of the key it names.
    pub fn id(&self) -> KeyId {
        let digest = Blake2b512::new()
            .chain_update(KEY_ID_DOMAIN)
            .chain_update(self.0)
            .finalize();
        KeyId::from_array(crate::hash::prefix(&digest))
    }

    /// Seals `previous` under this key and signs it: one link of the key
    /// chain (`SYNC-116`). The full envelope, so the link opens only under this
    /// key and names its author.
    ///
    /// # Errors
    ///
    /// [`CryptoError::EncryptionFailed`] from the AEAD, which a 32-byte
    /// plaintext never reaches.
    pub fn seal_previous(
        &self,
        previous: &DocumentKey,
        aad: &ambre_core::Aad,
        signing_key: &SigningKey,
    ) -> Result<crate::Sealed, CryptoError> {
        crate::envelope::seal(&previous.0, aad, self, signing_key)
    }

    /// Opens what [`Self::seal_previous`] sealed, against the author's key.
    /// The caller still checks the key against the identifier the link names
    /// (`SYNC-120`). This proves who sealed it under which key, not which key
    /// it is.
    ///
    /// # Errors
    ///
    /// Every error of [`crate::envelope::open`], and
    /// [`CryptoError::InvalidKeyLength`] when what opens is not a key.
    pub fn open_previous(
        &self,
        sealed: &crate::Sealed,
        aad: &ambre_core::Aad,
        author: &ambre_core::PublicKey,
    ) -> Result<DocumentKey, CryptoError> {
        let mut bytes = crate::envelope::open(sealed, aad, self, author)?;
        let key = <[u8; 32]>::try_from(bytes.as_slice())
            .map(DocumentKey)
            .map_err(|_| CryptoError::InvalidKeyLength);
        bytes.zeroize();
        key
    }
}

/// The 32-byte key that seals the rows of this user's local store, derived
/// from the [`MasterSeed`] and never stored.
///
/// A type of its own, like [`FileKey`]. A row sealed under a document key
/// would stop opening at the next rotation, and no AEAD entry point takes the
/// other's key. It carries no [`Clone`], so the one copy is the one wiped.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct StoreKey([u8; 32]);

impl StoreKey {
    /// Returns the raw key bytes. Restricted to crate use (AEAD).
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
    ///
    /// # Errors
    ///
    /// [`CryptoError::Argon2Failed`] from the first step, on a `salt` Argon2id
    /// refuses. The BLAKE2b pass is infallible.
    pub fn from_passphrase(
        passphrase: &ambre_core::Passphrase,
        salt: &FileSalt,
    ) -> Result<Self, CryptoError> {
        let mut intermediate = crate::passphrase::argon2id(passphrase, salt.as_array())?;
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
    ///
    /// # Errors
    ///
    /// [`CryptoError::Argon2Failed`] from [`Self::from_passphrase`]. The salt it
    /// draws is [`SALT_LEN`] bytes wide, which Argon2id accepts.
    pub fn for_new_file(
        passphrase: &ambre_core::Passphrase,
    ) -> Result<(Self, FileSalt), CryptoError> {
        let salt = FileSalt::from_array(random::bytes::<SALT_LEN>());
        let key = Self::from_passphrase(passphrase, &salt)?;
        Ok((key, salt))
    }

    /// Returns the raw key bytes. Restricted to crate use (AEAD).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
