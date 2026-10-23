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

/// Length of a session nonce, in bytes.
pub const SESSION_NONCE_LEN: usize = 16;

/// Length of an Ed25519 signature, in bytes.
pub const ENTRY_SIGNATURE_LEN: usize = 64;

/// Length of an encoded [`PeerEntry`]: the user, the nonce, the signature.
pub const PEER_ENTRY_LEN: usize = 16 + SESSION_NONCE_LEN + ENTRY_SIGNATURE_LEN;

/// The commit origin of a registry write. An application keeps it off its undo
/// stack: the entry is no gesture of the user.
pub const PEERS_ORIGIN: &str = "ambre-peers";

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

/// 128 random bits drawn when a client opens a document (`AUTHOR-002`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct SessionNonce(pub [u8; SESSION_NONCE_LEN]);

/// One row of the peer registry: who wrote under a PeerID, and with which
/// nonce. It carries that user's signature over the document, the PeerID and
/// the nonce (`AUTHOR-019`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PeerEntry {
    /// The user the entry claims.
    pub user: UserId,
    /// The nonce the PeerID derives from.
    pub nonce: SessionNonce,
    /// The signature of `user` over the document, the PeerID and the nonce.
    pub signature: Signature,
}

impl PeerEntry {
    /// The entry as the registry stores it: the user, the nonce, then the
    /// signature.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PEER_ENTRY_LEN);
        out.extend_from_slice(self.user.as_bytes());
        out.extend_from_slice(&self.nonce.0);
        out.extend_from_slice(&self.signature.0);
        out
    }

    /// Reads an entry back. `None` when `bytes` is not exactly one entry, which
    /// a reader treats as an absent entry (`AUTHOR-006`).
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let (user, rest) = bytes.split_first_chunk::<16>()?;
        let (nonce, rest) = rest.split_first_chunk::<SESSION_NONCE_LEN>()?;
        let signature: [u8; ENTRY_SIGNATURE_LEN] = rest.try_into().ok()?;
        Some(Self {
            user: UserId::from_bytes(*user),
            nonce: SessionNonce(*nonce),
            signature: Signature(signature),
        })
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

    #[test]
    fn an_entry_reads_back_and_a_truncated_one_does_not() {
        let entry = PeerEntry {
            user: UserId::from_bytes([3; 16]),
            nonce: SessionNonce([5; SESSION_NONCE_LEN]),
            signature: Signature([7; ENTRY_SIGNATURE_LEN]),
        };
        let bytes = entry.to_bytes();
        assert_eq!(bytes.len(), PEER_ENTRY_LEN);
        assert_eq!(PeerEntry::from_bytes(&bytes), Some(entry));
        assert_eq!(PeerEntry::from_bytes(&bytes[1..]), None);
        assert_eq!(PeerKey::of(PeerId(0xab)).as_str(), "00000000000000ab");
    }
}
