//! The byte newtypes of the encrypted envelope: nonce, ciphertext, commitment
//! tag and signature.
//!
//! `ambre-crypto` computes them and `ambre-protocol` carries them on the wire.
//! Both depend on this crate alone, so the types live here. They hold bytes
//! only: no primitive of this module encrypts, hashes or signs.

use crate::fixed_bytes::{DebugForm, FixedRole};

/// The role of a 24-byte nonce used for XChaCha20-Poly1305 encryption:
/// `FixedBytes<Nonce>`.
pub enum Nonce {}

impl FixedRole for Nonce {
    const NAME: &'static str = "Nonce";
    type Array = [u8; 24];
}

/// The role of a 32-byte HMAC-SHA256 commitment tag:
/// `FixedBytes<CommitmentTag>`.
pub enum CommitmentTag {}

impl FixedRole for CommitmentTag {
    const NAME: &'static str = "CommitmentTag";
    type Array = [u8; 32];
}

/// The role of a 64-byte Ed25519 signature: `FixedBytes<Signature>`.
pub enum Signature {}

impl FixedRole for Signature {
    const NAME: &'static str = "Signature";
    const DEBUG: DebugForm = DebugForm::HexPrefix;
    type Array = [u8; 64];
}

/// The role of AEAD ciphertext: `Payload<Ciphertext>`. `Debug` prints the
/// length and none of the bytes, so a logged message logs no ciphertext
/// (`ZK-006`). Serde sees the bare byte string, as the `serde_bytes` form it
/// replaced: postcard and JSON write the same bytes.
pub enum Ciphertext {}

impl crate::PayloadKind for Ciphertext {
    const NAME: &'static str = "Ciphertext";
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

/// The role of one link of the snapshot integrity chain:
/// `FixedBytes<ChainHash>`. It folds a parent proof with the bytes of a sealed
/// snapshot, so a reordered or substituted snapshot breaks the chain. Distinct
/// from [`EntryDigest`] and from any keyless digest, so no call can pass one
/// where the other is meant.
pub enum ChainHash {}

impl FixedRole for ChainHash {
    const NAME: &'static str = "ChainHash";
    type Array = [u8; 32];
}

/// The role of the digest of one membership-log entry, which the next entry
/// names: `FixedBytes<EntryDigest>`. A wrapped document key also seals the
/// digest of the first entry, as the anchor of the log (`SYNC-104`).
pub enum EntryDigest {}

impl FixedRole for EntryDigest {
    const NAME: &'static str = "EntryDigest";
    type Array = [u8; 32];
}

#[cfg(test)]
mod tests {
    use super::{Ciphertext, CommitmentTag, Nonce, Signature};
    use crate::authorship::SessionNonce;
    use crate::{ByteSize, FixedBytes, KeyId, Payload, PublicKey};

    /// A ciphertext prints its length and none of its bytes (`ZK-006`).
    #[test]
    fn a_ciphertext_debug_prints_no_byte() {
        let ciphertext = Payload::<Ciphertext>::new(vec![0xab; 3]);
        let debug = format!("{ciphertext:?}");
        assert_eq!(debug, "Ciphertext(3 bytes)");
        assert!(!debug.contains("171") && !debug.contains("ab"));
    }

    /// A ciphertext writes the bytes the `serde_bytes` form wrote, in postcard
    /// and in JSON, and reads that form back.
    #[test]
    fn a_ciphertext_keeps_its_serde_bytes() {
        let bytes = vec![0, 1, 0xfe, 0xff];
        let old = serde_bytes::Bytes::new(&bytes);
        let ciphertext = Payload::<Ciphertext>::new(bytes.clone());
        let wire = postcard::to_allocvec(&ciphertext).unwrap();
        assert_eq!(wire, postcard::to_allocvec(&old).unwrap());
        let back: Payload<Ciphertext> = postcard::from_bytes(&wire).unwrap();
        assert_eq!(back, ciphertext);
        let json = serde_json::to_string(&ciphertext).unwrap();
        assert_eq!(json, serde_json::to_string(&old).unwrap());
        let back: Payload<Ciphertext> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ciphertext);
    }

    /// Each fixed-size type states the size it frames, the length of its
    /// array.
    #[test]
    fn a_fixed_size_type_states_the_length_of_its_array() {
        assert_eq!(FixedBytes::<Nonce>::SIZE, ByteSize::new(24));
        assert_eq!(FixedBytes::<CommitmentTag>::SIZE, ByteSize::new(32));
        assert_eq!(FixedBytes::<Signature>::SIZE, ByteSize::new(64));
        assert_eq!(FixedBytes::<KeyId>::SIZE, ByteSize::new(16));
        assert_eq!(FixedBytes::<SessionNonce>::SIZE, ByteSize::new(16));
        assert_eq!(FixedBytes::<PublicKey>::SIZE, ByteSize::new(32));
    }

    /// A split reads one value from the front and hands back the bytes after
    /// it. Bytes short of one value give nothing.
    #[test]
    fn a_split_reads_one_value_from_the_front_and_refuses_short_bytes() {
        let nonce = FixedBytes::<Nonce>::from_array([7; 24]);
        let mut bytes = vec![7; 24];
        bytes.extend_from_slice(b"rest");
        assert_eq!(
            FixedBytes::<Nonce>::split_from(&bytes),
            Some((nonce, &b"rest"[..]))
        );
        assert_eq!(
            FixedBytes::<Nonce>::split_from(&[7; 24]),
            Some((nonce, &[][..]))
        );
        assert_eq!(FixedBytes::<Nonce>::split_from(&[7; 23]), None);
        let (key_id, rest) = FixedBytes::<KeyId>::split_from(&[3; 20]).unwrap();
        assert_eq!(
            (key_id, rest),
            (FixedBytes::from_array([3; 16]), &[3; 4][..])
        );
        assert_eq!(FixedBytes::<PublicKey>::split_from(&[1; 31]), None);
        assert_eq!(FixedBytes::<Signature>::split_from(&[]), None);
    }

    /// A push appends the bytes a split reads back, after what `out` held.
    #[test]
    fn a_push_appends_the_bytes_a_split_reads_back() {
        let nonce = FixedBytes::<Nonce>::from_array([7; 24]);
        let mut out = b"head".to_vec();
        FixedBytes::<KeyId>::from_array([3; 16]).push_to(&mut out);
        nonce.push_to(&mut out);
        let rest = out.strip_prefix(b"head").unwrap();
        let (key_id, rest) = FixedBytes::<KeyId>::split_from(rest).unwrap();
        assert_eq!(key_id, FixedBytes::from_array([3; 16]));
        assert_eq!(
            FixedBytes::<Nonce>::split_from(rest),
            Some((nonce, &[][..]))
        );
    }
}
