//! The PeerID of a session, derived from its author's signing key and a session
//! nonce, and the signed registry entry that names its user
//! (`ambre-spec-op-authorship.md`).
//!
//! ```text
//! peer_id = first 64 bits of BLAKE2b(b"ambre-peer-v1" || signing_pubkey || nonce)
//! ```

use ambre_core::authorship::{
    ENTRY_SIGNATURE_LEN, PeerEntry, PeerId, SESSION_NONCE_LEN, SessionNonce,
};
use ambre_core::{DocumentId, PublicKey, UserId};
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// The derivation domain of a PeerID. Frozen: a new value would move every
/// PeerID a registry already holds (`AUTHOR-003`).
pub const PEER_DOMAIN: &[u8] = b"ambre-peer-v1";

/// The domain of the signed transcript of a registry entry.
const ENTRY_DOMAIN: &[u8] = b"ambre-peer-entry-v1";

/// Draws a fresh session nonce from the operating system generator.
#[must_use]
pub fn draw_nonce() -> SessionNonce {
    SessionNonce(crate::random::bytes::<SESSION_NONCE_LEN>())
}

/// The PeerID of the session that `signing_pub` opened with `nonce`
/// (`AUTHOR-001`).
///
/// The CRDT engine reserves `u64::MAX`, so a hash that lands on it takes the
/// value below. Both ends derive alike, so the check still holds.
#[must_use]
pub fn derive_peer_id(signing_pub: &PublicKey, nonce: &SessionNonce) -> PeerId {
    let digest = Blake2b512::new()
        .chain_update(PEER_DOMAIN)
        .chain_update(signing_pub.as_array())
        .chain_update(nonce.0)
        .finalize();
    let mut head = [0u8; 8];
    for (out, byte) in head.iter_mut().zip(digest.iter()) {
        *out = *byte;
    }
    PeerId(u64::from_le_bytes(head).min(u64::MAX - 1))
}

/// The bytes a registry entry signs: the document, the PeerID and the nonce,
/// behind their own domain. Every field has a fixed length.
fn entry_transcript(document: DocumentId, peer: PeerId, nonce: &SessionNonce) -> Vec<u8> {
    let mut msg = Vec::with_capacity(ENTRY_DOMAIN.len() + 16 + 8 + SESSION_NONCE_LEN);
    msg.extend_from_slice(ENTRY_DOMAIN);
    msg.extend_from_slice(document.as_bytes());
    msg.extend_from_slice(&peer.0.to_le_bytes());
    msg.extend_from_slice(&nonce.0);
    msg
}

/// The PeerID of this session and its signed registry entry (`AUTHOR-019`).
#[must_use]
pub fn session_identity(
    signing_key: &SigningKey,
    user: UserId,
    document: DocumentId,
    nonce: SessionNonce,
) -> (PeerId, PeerEntry) {
    let own = PublicKey::from_array(signing_key.verifying_key().to_bytes());
    let peer = derive_peer_id(&own, &nonce);
    let signature = signing_key
        .sign(&entry_transcript(document, peer, &nonce))
        .to_bytes();
    let entry = PeerEntry {
        user,
        nonce,
        signature: ambre_core::Signature(signature),
    };
    (peer, entry)
}

/// Whether `entry` resolves `peer` to its user in `document`. `user_key` is the
/// signing key the membership log holds for that user.
///
/// It resolves only when the PeerID derives from that key and the nonce
/// (`AUTHOR-005`), and the signature verifies under that key (`AUTHOR-020`).
#[must_use]
pub fn entry_resolves(
    entry: &PeerEntry,
    document: DocumentId,
    peer: PeerId,
    user_key: &PublicKey,
) -> bool {
    if derive_peer_id(user_key, &entry.nonce) != peer {
        return false;
    }
    let Ok(verifying_key) = VerifyingKey::from_bytes(user_key.as_array()) else {
        return false;
    };
    let signature: &[u8; ENTRY_SIGNATURE_LEN] = &entry.signature.0;
    verifying_key
        .verify_strict(
            &entry_transcript(document, peer, &entry.nonce),
            &Signature::from_bytes(signature),
        )
        .is_ok()
}

/// The user an operation under `peer` resolves to, read from the registry.
///
/// `entry` is the registry row for `peer`, `None` when absent. `key_of` returns
/// the signing key of a user. Any failed step resolves to no user
/// (`AUTHOR-006`).
#[must_use]
pub fn resolve_peer(
    entry: Option<&PeerEntry>,
    document: DocumentId,
    peer: PeerId,
    key_of: impl Fn(UserId) -> Option<PublicKey>,
) -> Option<UserId> {
    let entry = entry?;
    let key = key_of(entry.user)?;
    entry_resolves(entry, document, peer, &key).then_some(entry.user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(key: &SigningKey) -> PublicKey {
        PublicKey::from_array(key.verifying_key().to_bytes())
    }

    /// The derivation is pinned to a value, so another build derives the same
    /// PeerID from the same key and nonce.
    #[test]
    fn a_peer_id_derives_from_the_key_and_the_nonce() {
        let alice = public(&key(1));
        let nonce = SessionNonce([9; SESSION_NONCE_LEN]);
        let peer = derive_peer_id(&alice, &nonce);
        assert_eq!(peer, derive_peer_id(&alice, &nonce));
        assert_ne!(peer, derive_peer_id(&public(&key(2)), &nonce));
        assert_ne!(
            peer,
            derive_peer_id(&alice, &SessionNonce([8; SESSION_NONCE_LEN]))
        );
        assert_eq!(PEER_DOMAIN, b"ambre-peer-v1");
        // Computed outside Rust: the first eight bytes, little-endian, of
        // Python's `hashlib.blake2b(b"ambre-peer-v1" + bytes([1]*32) + bytes([9]*16))`.
        let pinned = derive_peer_id(&PublicKey::from_array([1; 32]), &nonce);
        assert_eq!(pinned, ambre_core::PeerId(0x280d_a166_ca77_5390));
    }

    #[test]
    fn two_drawn_nonces_differ() {
        assert_ne!(draw_nonce(), draw_nonce());
    }

    /// An entry resolves under its own user's key, and under no other: a
    /// foreign key neither derives the PeerID nor verifies the signature.
    #[test]
    fn an_entry_resolves_to_the_user_who_signed_it_alone() {
        let alice = key(1);
        let mallory = key(2);
        let alice_id = UserId::from_bytes([1; 16]);
        let mallory_id = UserId::from_bytes([2; 16]);
        let document = DocumentId::from_bytes([4; 16]);
        let (peer, entry) = session_identity(&alice, alice_id, document, draw_nonce());
        let keys = |user: UserId| {
            if user == alice_id {
                Some(public(&alice))
            } else if user == mallory_id {
                Some(public(&mallory))
            } else {
                None
            }
        };
        assert_eq!(
            resolve_peer(Some(&entry), document, peer, keys),
            Some(alice_id)
        );
        // An absent entry resolves to nobody.
        assert_eq!(resolve_peer(None, document, peer, keys), None);
        // The same entry under another document does not verify.
        let other = DocumentId::from_bytes([5; 16]);
        assert_eq!(resolve_peer(Some(&entry), other, peer, keys), None);
        // Mallory claims Alice for one of her own PeerIDs: the PeerID does not
        // derive from Alice's key.
        let (mallory_peer, mut forged) =
            session_identity(&mallory, mallory_id, document, draw_nonce());
        forged.user = alice_id;
        assert_eq!(
            resolve_peer(Some(&forged), document, mallory_peer, keys),
            None
        );
        // Mallory derives a PeerID from Alice's public key, which anyone can:
        // the signature is the part she cannot make.
        let nonce = draw_nonce();
        let alice_peer = derive_peer_id(&public(&alice), &nonce);
        let (_, by_mallory) = session_identity(&mallory, mallory_id, document, nonce);
        let claim = PeerEntry {
            user: alice_id,
            ..by_mallory
        };
        assert_eq!(resolve_peer(Some(&claim), document, alice_peer, keys), None);
    }
}
