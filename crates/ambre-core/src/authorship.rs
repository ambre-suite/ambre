//! The types that bind a Loro PeerID to the signing key of its author
//! (`ambre-spec-op-authorship.md`).
//!
//! A session draws a [`SessionNonce`] each time it opens a document. Its PeerID
//! is a hash of the author's signing key and that nonce, computed in
//! `ambre-crypto`. The document carries a peer registry, one [`PeerEntry`] per
//! PeerID, so a reader can resolve an operation to its user.

use serde::{Deserialize, Serialize};

use crate::UserId;
use crate::envelope::Signature;
use crate::fixed_bytes::{FixedBytes, FixedRole};

/// Length of a session nonce, in bytes.
pub const SESSION_NONCE_LEN: usize = 16;

/// Length of an Ed25519 signature, in bytes.
pub const ENTRY_SIGNATURE_LEN: usize = 64;

/// Length of an encoded [`PeerEntry`]: the user, the nonce, the signature.
pub const PEER_ENTRY_LEN: usize = 16 + SESSION_NONCE_LEN + ENTRY_SIGNATURE_LEN;

/// The Loro PeerID of one writing session (`AUTHOR-001`). A distinct type,
/// so an op counter, a sequence number or a lamport stamp never takes its
/// place. `#[serde(transparent)]`, so it reads and writes the bare integer.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PeerId(pub u64);

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The spelling an element id and a registry key carry.
impl std::fmt::LowerHex for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::LowerHex::fmt(&self.0, f)
    }
}

/// The counter of one operation under a PeerID: a Loro `ID` is the pair of
/// the two. A distinct type, so a PeerID, a sequence number or a length never
/// takes its place. `#[serde(transparent)]`, so it reads and writes the bare
/// integer.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpCounter(pub i32);

/// The part of a minted id after its PeerID: a Toile element or connector
/// id, or a Trame row or column uid. One session mints each value once, so
/// the pair with its PeerID names one thing.
///
/// An id part, not a tally: two mints of one value would give two things
/// one id. So it has no step that holds at the top or wraps, only
/// [`next`](Self::next), which ends at `u32::MAX`.
/// `#[serde(transparent)]`, so it reads and writes the bare integer.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct IdCounter(u32);

impl IdCounter {
    /// The first value of a session.
    pub const ZERO: Self = Self(0);

    /// The id part `n`, as a stored id spells it.
    pub const fn new(n: u32) -> Self {
        Self(n)
    }

    /// The value itself, for the hex an id spells.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The value after this one, or `None` past `u32::MAX`: the session has
    /// no fresh id part left.
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(n) => Some(Self(n)),
            None => None,
        }
    }
}

/// The role of the 128 random bits drawn when a client opens a document
/// (`AUTHOR-002`): `FixedBytes<SessionNonce>`.
pub enum SessionNonce {}

impl FixedRole for SessionNonce {
    const NAME: &'static str = "SessionNonce";
    type Array = [u8; SESSION_NONCE_LEN];
}

/// One row of the peer registry: who wrote under a PeerID, and with which
/// nonce. It carries that user's signature over the document, the PeerID and
/// the nonce (`AUTHOR-019`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PeerEntry {
    /// The user the entry claims.
    pub user: UserId,
    /// The nonce the PeerID derives from.
    pub nonce: FixedBytes<SessionNonce>,
    /// The signature of `user` over the document, the PeerID and the nonce.
    pub signature: FixedBytes<Signature>,
}

impl PeerEntry {
    /// The entry as the registry stores it: the user, the nonce, then the
    /// signature.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PEER_ENTRY_LEN);
        out.extend_from_slice(self.user.as_bytes());
        self.nonce.push_to(&mut out);
        self.signature.push_to(&mut out);
        out
    }

    /// Reads an entry back. `None` when `bytes` is not exactly one entry, which
    /// a reader treats as an absent entry (`AUTHOR-006`).
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (user, rest) = bytes.split_first_chunk::<16>()?;
        let (nonce, rest) = FixedBytes::<SessionNonce>::split_from(rest)?;
        let signature = FixedBytes::<Signature>::from_slice(rest).ok()?;
        Some(Self {
            user: UserId::from_bytes(*user),
            nonce,
            signature,
        })
    }
}

/// Crosses the bus as the bytes [`PeerEntry::to_bytes`] writes, so the event
/// carries one length-prefixed byte string. Bytes that are not exactly one
/// entry fail to deserialize.
impl Serialize for PeerEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_bytes::Bytes::new(&self.to_bytes()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PeerEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(deserializer)?;
        Self::from_bytes(&bytes).ok_or_else(|| serde::de::Error::custom("not one peer entry"))
    }
}

/// A registry entry as a snapshot holds it, not read yet. The snapshot came
/// from another writer, so its bytes may not be an entry at all. The reader
/// that verifies the entry is the one that reads it (`AUTHOR-021`), and a
/// malformed one fails there. `#[serde(transparent)]`: the bare byte string.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerEntryBytes(Vec<u8>);

impl PeerEntryBytes {
    /// The bytes as the registry stores them.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The entry, or `None` when the bytes are not exactly one.
    #[must_use]
    pub fn read(&self) -> Option<PeerEntry> {
        PeerEntry::from_bytes(&self.0)
    }

    /// The bytes, owned, for the registry map that stores them.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<&PeerEntry> for PeerEntryBytes {
    fn from(entry: &PeerEntry) -> Self {
        Self(entry.to_bytes())
    }
}

/// The key of a PeerID in the peer registry,
/// [`crate::crdt::FoundationRoot::Peers`]: sixteen lowercase hex digits
/// (`AUTHOR-004`). Built from the PeerID alone, so no call site spells the
/// format.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PeerKey(String);

impl PeerKey {
    /// The registry key of `peer`.
    #[must_use]
    pub fn of(peer: PeerId) -> Self {
        Self(format!("{peer:016x}"))
    }

    /// The key as the registry stores it, for the call into the map.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An id part steps to the next value and ends at the top, never back to
    /// a value it gave. It crosses the wire as the bare number.
    #[test]
    fn an_id_counter_ends_at_the_top_and_crosses_the_wire_bare() {
        assert_eq!(IdCounter::ZERO.next(), Some(IdCounter::new(1)));
        assert_eq!(
            IdCounter::new(u32::MAX - 1).next(),
            Some(IdCounter::new(u32::MAX))
        );
        assert_eq!(IdCounter::new(u32::MAX).next(), None);
        let bytes = postcard::to_allocvec(&IdCounter::new(7)).unwrap();
        assert_eq!(bytes, postcard::to_allocvec(&7_u32).unwrap());
    }

    #[test]
    fn an_entry_reads_back_and_a_truncated_one_does_not() {
        let entry = PeerEntry {
            user: UserId::from_bytes([3; 16]),
            nonce: FixedBytes::from_array([5; SESSION_NONCE_LEN]),
            signature: FixedBytes::from_array([7; ENTRY_SIGNATURE_LEN]),
        };
        let bytes = entry.to_bytes();
        assert_eq!(bytes.len(), PEER_ENTRY_LEN);
        assert_eq!(PeerEntry::from_bytes(&bytes), Some(entry));
        assert_eq!(PeerEntry::from_bytes(&bytes[1..]), None);
        assert_eq!(PeerKey::of(PeerId(0xab)).as_str(), "00000000000000ab");
    }

    /// An entry crosses the bus as the byte string the registry stores, and
    /// bytes that are not one entry do not decode as one. An entry a snapshot
    /// holds is read only by the reader that verifies it.
    #[test]
    fn an_entry_crosses_the_bus_as_its_registry_bytes() {
        let entry = PeerEntry {
            user: UserId::from_bytes([3; 16]),
            nonce: FixedBytes::from_array([5; SESSION_NONCE_LEN]),
            signature: FixedBytes::from_array([7; ENTRY_SIGNATURE_LEN]),
        };
        let wire = postcard::to_allocvec(&entry).unwrap();
        assert_eq!(wire, postcard::to_allocvec(&entry.to_bytes()).unwrap());
        assert_eq!(postcard::from_bytes::<PeerEntry>(&wire).unwrap(), entry);
        let short = postcard::to_allocvec(&entry.to_bytes()[1..].to_vec()).unwrap();
        assert!(postcard::from_bytes::<PeerEntry>(&short).is_err());
        assert_eq!(PeerEntryBytes::from(&entry).read(), Some(entry));
        assert_eq!(PeerEntryBytes::new(vec![7; 4]).read(), None);
    }
}
