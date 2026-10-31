//! The key chain of one document (`ambre-spec-key-rotation.md section 3`).
//!
//! The relay keeps one wrapped copy per member, the current one. Every older
//! key comes back through the links the rotations left: each seals the key it
//! replaced under the key it installed. A walk starts at the current key and
//! steps back one link at a time, so it reaches only what the current key
//! opens (`SYNC-119`).
//!
//! A recovered key opens history and nothing else. The walk hands it back to
//! one open, and never stores it where a seal reads its key (`SYNC-122`).

use std::collections::{HashMap, HashSet};

use ambre_core::KeyId;
use ambre_core::id::{Document, Id};
use ambre_crypto::DocumentKey;
use ambre_protocol::KeyLink;

use crate::pipeline;

/// The links of one document, indexed by the key each starts from.
#[derive(Clone, Default)]
pub(crate) struct Chain {
    links: HashMap<KeyId, KeyLink>,
}

impl Chain {
    /// The links of `document_id` among `links`. A link of another document
    /// is dropped, and the first link starting from a key is the one kept.
    pub(crate) fn new(document_id: Id<Document>, links: impl IntoIterator<Item = KeyLink>) -> Self {
        let mut chain = Self::default();
        for link in links {
            chain.push(document_id, link);
        }
        chain
    }

    /// Adds one link, the one this client just posted or one the relay served.
    pub(crate) fn push(&mut self, document_id: Id<Document>, link: KeyLink) {
        if link.document_id == document_id {
            self.links.entry(link.key).or_insert(link);
        }
    }

    /// How many links the chain holds.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.links.len()
    }

    /// The key `wanted` names, walked back from `current` (`SYNC-119`).
    ///
    /// Each step opens one link under the key it starts from, and checks that
    /// what opens hashes to the identifier the link names. A link that fails
    /// either check ends the walk, so no key past it is reached (`SYNC-120`).
    /// `None` when the walk ends before `wanted`.
    pub(crate) fn recover(
        &self,
        document_id: Id<Document>,
        current: &DocumentKey,
        wanted: KeyId,
    ) -> Option<DocumentKey> {
        let mut key = current.clone();
        // Each step consumes one link, so a walk longer than the chain is a
        // loop, which no honest chain holds.
        for _ in 0..=self.links.len() {
            if key.id() == wanted {
                return Some(key);
            }
            let link = self.links.get(&key.id())?;
            key = pipeline::open_link(link, document_id, &key).ok()?;
        }
        None
    }
}

/// The links among `links` signed by a key the membership log granted a
/// writing role (`SYNC-121`). `signers` is `None` for a document older than
/// the log. Every link is kept then, and its signature still holds under the
/// key its header names.
pub(crate) fn signed_by_writers(
    links: Vec<KeyLink>,
    signers: Option<&HashSet<ambre_core::PublicKey>>,
) -> Vec<KeyLink> {
    let Some(signers) = signers else {
        return links;
    };
    links
        .into_iter()
        .filter(|link| pipeline::object_author(&link.sealed).is_ok_and(|a| signers.contains(&a)))
        .collect()
}

/// What one document's keys open: the current key, the cache and the chain.
/// It is taken out of the document state for a task that outlives the borrow.
#[derive(Clone)]
pub(crate) struct Keyring {
    pub(crate) document_id: Id<Document>,
    pub(crate) current: DocumentKey,
    pub(crate) previous: Vec<DocumentKey>,
    pub(crate) chain: Chain,
}

impl Keyring {
    /// The key `wanted` names, as [`find_key`] reaches it.
    pub(crate) fn find(&self, wanted: KeyId) -> Option<DocumentKey> {
        find_key(
            self.document_id,
            &self.current,
            &self.previous,
            &self.chain,
            wanted,
        )
    }
}

/// The key `wanted` names: the current one, a cached previous one, or one the
/// chain reaches from the current key. Never by trying every key in turn
/// (`SYNC-045`).
pub(crate) fn find_key(
    document_id: Id<Document>,
    current: &DocumentKey,
    previous: &[DocumentKey],
    chain: &Chain,
    wanted: KeyId,
) -> Option<DocumentKey> {
    std::iter::once(current)
        .chain(previous)
        .find(|key| key.id() == wanted)
        .cloned()
        .or_else(|| chain.recover(document_id, current, wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_crypto::MasterSeed;
    use ed25519_dalek::SigningKey;

    fn rotator() -> SigningKey {
        MasterSeed::from_array([0x44; 32]).derive_signing_key()
    }

    /// `n + 1` keys, oldest first, and the `n` links between them.
    fn rotations(document_id: Id<Document>, n: usize) -> (Vec<DocumentKey>, Vec<KeyLink>) {
        let keys: Vec<_> = (0..=n).map(|_| DocumentKey::generate()).collect();
        let links = keys
            .windows(2)
            .map(|pair| pipeline::seal_link(document_id, &pair[0], &pair[1], &rotator()).unwrap())
            .collect();
        (keys, links)
    }

    /// Past the tenth rotation, the current key alone still reaches every
    /// older key, one link at a time (`SYNC-119`).
    #[test]
    fn the_current_key_walks_back_to_every_older_key() {
        let document_id = Id::<Document>::mint();
        let (keys, links) = rotations(document_id, 12);
        let chain = Chain::new(document_id, links);
        let current = keys.last().unwrap();
        for key in &keys {
            let found = chain.recover(document_id, current, key.id()).unwrap();
            assert_eq!(found.id(), key.id());
        }
        // An older key walks back, never forward.
        assert!(chain.recover(document_id, &keys[3], keys[5].id()).is_none());
        // A key no rotation left is out of reach.
        assert!(
            chain
                .recover(document_id, current, DocumentKey::generate().id())
                .is_none()
        );
    }

    /// A link whose key does not hash to the identifier it names is refused,
    /// and so is every link past it. The links before it still open
    /// (`SYNC-120`).
    #[test]
    fn a_link_naming_another_key_stops_the_walk_there() {
        let document_id = Id::<Document>::mint();
        let (keys, mut links) = rotations(document_id, 4);
        // The link from keys[3] seals a key that is not keys[2], and names
        // keys[2]: what the relay would serve to swap a key in.
        let forged =
            pipeline::seal_link(document_id, &DocumentKey::generate(), &keys[3], &rotator())
                .unwrap();
        links[2] = KeyLink {
            previous: keys[2].id(),
            ..forged
        };
        let chain = Chain::new(document_id, links);
        let current = &keys[4];
        assert!(chain.recover(document_id, current, keys[3].id()).is_some());
        for older in &keys[..3] {
            assert!(chain.recover(document_id, current, older.id()).is_none());
        }
    }

    /// A link the relay re-sealed, or whose signature does not hold, opens
    /// nothing; a link of another document is not kept.
    #[test]
    fn a_link_tampered_or_misfiled_opens_nothing() {
        let document_id = Id::<Document>::mint();
        let (keys, links) = rotations(document_id, 1);
        let mut tampered = links[0].clone();
        let mut sealed = tampered.sealed.into_bytes();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        tampered.sealed = ambre_core::SealedPreviousKey::new(sealed);
        let chain = Chain::new(document_id, [tampered]);
        assert!(chain.recover(document_id, &keys[1], keys[0].id()).is_none());

        let elsewhere = Chain::new(Id::<Document>::mint(), links);
        assert_eq!(elsewhere.len(), 0);
    }

    /// Holds `SYNC-121`. The walk drops a link whose signer holds no writing
    /// role in the log, so the key it seals is out of reach.
    /// A document with no log keeps every link.
    #[test]
    fn a_link_no_writer_signed_is_dropped() {
        let document_id = Id::<Document>::mint();
        let (keys, mut links) = rotations(document_id, 2);
        let stranger = MasterSeed::from_array([0x55; 32]).derive_signing_key();
        if let Some(link) = links.get_mut(0) {
            *link = pipeline::seal_link(document_id, &keys[0], &keys[1], &stranger).unwrap();
        }
        let writers = HashSet::from([ambre_core::PublicKey::from_array(
            rotator().verifying_key().to_bytes(),
        )]);
        let kept = signed_by_writers(links.clone(), Some(&writers));
        assert_eq!(kept.len(), 1);
        let chain = Chain::new(document_id, kept);
        assert!(chain.recover(document_id, &keys[2], keys[1].id()).is_some());
        assert!(chain.recover(document_id, &keys[2], keys[0].id()).is_none());
        assert_eq!(signed_by_writers(links, None).len(), 2);
    }

    /// A key the walk recovers opens history and is handed back alone: the
    /// key the caller holds as current is untouched (`SYNC-122`).
    #[test]
    fn a_recovered_key_never_becomes_the_current_one() {
        let document_id = Id::<Document>::mint();
        let (keys, links) = rotations(document_id, 2);
        let chain = Chain::new(document_id, links);
        let current = keys[2].clone();
        let found = find_key(document_id, &current, &[], &chain, keys[0].id()).unwrap();
        assert_eq!(found.id(), keys[0].id());
        assert_eq!(current.id(), keys[2].id());
    }
}
