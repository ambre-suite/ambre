//! The byte newtypes of the encrypted envelope: nonce, ciphertext, commitment
//! tag and signature.
//!
//! `ambre-crypto` computes them and `ambre-protocol` carries them on the wire.
//! Both depend on this crate alone, so the types live here. They hold bytes
//! only: no primitive of this module encrypts, hashes or signs.

use serde::{Deserialize, Serialize};

/// Gives a fixed-size byte type its size and its typed split. A crate that
/// frames one in a layout then never counts its bytes by hand. The type is a tuple
/// struct over `[u8; $len]`, and the macro runs in its own module, which
/// builds it. A const assert holds the declared length to the array.
macro_rules! fixed_bytes {
    ($name:ident, $len:expr) => {
        impl $name {
            /// Bytes of one value where a layout frames it.
            pub const SIZE: $crate::ByteSize = $crate::ByteSize($len as u64);

            /// Reads one value from the front of `bytes`, and returns the
            /// bytes after it. `None` when `bytes` is shorter than
            /// [`Self::SIZE`].
            #[must_use]
            pub fn split_from(bytes: &[u8]) -> Option<(Self, &[u8])> {
                let (head, rest) = bytes.split_first_chunk::<{ $len }>()?;
                Some((Self(*head), rest))
            }
        }

        const _: () = assert!(::core::mem::size_of::<$name>() == $len);
    };
}
pub(crate) use fixed_bytes;

/// 24-byte nonce used for XChaCha20-Poly1305 encryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nonce(#[serde(with = "serde_bytes")] pub [u8; 24]);

/// 32-byte HMAC-SHA256 commitment tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitmentTag(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// 64-byte Ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes")] pub [u8; 64]);

fixed_bytes!(Nonce, 24);
fixed_bytes!(CommitmentTag, 32);
fixed_bytes!(Signature, 64);

/// AEAD ciphertext wrapper. `Debug` prints its length and none of its bytes,
/// so a logged message logs no ciphertext (`ZK-006`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ciphertext(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl Ciphertext {
    /// The length in bytes, the one reading of a ciphertext a log may carry.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether it holds no byte.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Ciphertext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ciphertext({} bytes)", self.0.len())
    }
}

/// The associated data a seal binds its ciphertext to: the public context
/// (document, place, version) the ciphertext must open under.
///
/// A type of its own, so a call that takes plaintext, ciphertext and
/// associated data side by side cannot take one for another. Every byte
/// string is valid associated data, so the constructor takes any; the bytes
/// are the ones the seal authenticates, unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Aad(Vec<u8>);

impl Aad {
    /// The associated data `bytes` spell.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The associated data of a fixed domain tag.
    pub fn of(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    /// The bytes the seal authenticates.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One link of the snapshot integrity chain. It folds a parent proof with
/// the bytes of a sealed snapshot, so a reordered or substituted snapshot
/// breaks the chain. Distinct from [`EntryDigest`] and from any
/// keyless digest, so no call can pass one where the other is meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChainHash(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// The digest of one membership-log entry, which the next entry names. A
/// wrapped document key also seals the digest of the first entry, as the
/// anchor of the log (`SYNC-104`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntryDigest(pub [u8; 32]);

#[cfg(test)]
mod tests {
    use super::{CommitmentTag, Nonce, Signature};
    use crate::authorship::SessionNonce;
    use crate::{ByteSize, KeyId, PublicKey};

    /// Each fixed-size type states the size it frames, the length of its
    /// array.
    #[test]
    fn a_fixed_size_type_states_the_length_of_its_array() {
        assert_eq!(Nonce::SIZE, ByteSize(24));
        assert_eq!(CommitmentTag::SIZE, ByteSize(32));
        assert_eq!(Signature::SIZE, ByteSize(64));
        assert_eq!(KeyId::SIZE, ByteSize(16));
        assert_eq!(SessionNonce::SIZE, ByteSize(16));
        assert_eq!(PublicKey::SIZE, ByteSize(32));
    }

    /// A split reads one value from the front and hands back the bytes after
    /// it. Bytes short of one value give nothing.
    #[test]
    fn a_split_reads_one_value_from_the_front_and_refuses_short_bytes() {
        let mut bytes = vec![7; 24];
        bytes.extend_from_slice(b"rest");
        assert_eq!(
            Nonce::split_from(&bytes),
            Some((Nonce([7; 24]), &b"rest"[..]))
        );
        assert_eq!(Nonce::split_from(&[7; 24]), Some((Nonce([7; 24]), &[][..])));
        assert_eq!(Nonce::split_from(&[7; 23]), None);
        let (key_id, rest) = KeyId::split_from(&[3; 20]).unwrap();
        assert_eq!((key_id, rest), (KeyId::from_array([3; 16]), &[3; 4][..]));
        assert_eq!(PublicKey::split_from(&[1; 31]), None);
        assert_eq!(Signature::split_from(&[]), None);
    }
}
