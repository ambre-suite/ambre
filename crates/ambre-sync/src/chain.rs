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

use std::collections::HashMap;

use ambre_core::id::{Document, Id};
use ambre_core::{FixedBytes, KeyId};
use ambre_crypto::DocumentKey;
use ambre_protocol::KeyLink;

use crate::membership_log::LinkSigners;
use crate::pipeline;

/// The links of one document, indexed by the key each starts from.
#[derive(Clone, Default)]
pub(crate) struct Chain {
    links: HashMap<FixedBytes<KeyId>, KeyLink>,
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
        wanted: FixedBytes<KeyId>,
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

/// The links among `links` whose signer held a writing role at the log head
/// the link names (`SYNC-121`, `SYNC-204`). A document older than the log
/// keeps every link, and its signature still holds under the key its header
/// names.
pub(crate) fn signed_by_writers(links: Vec<KeyLink>, signers: &LinkSigners) -> Vec<KeyLink> {
    links
        .into_iter()
        .filter(|link| {
            pipeline::object_author(&link.sealed).is_ok_and(|a| signers.signs(&a, link.head))
        })
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
    pub(crate) fn find(&self, wanted: FixedBytes<KeyId>) -> Option<DocumentKey> {
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
    wanted: FixedBytes<KeyId>,
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
    use crate::membership_log::tests::Person;
    use crate::membership_log::{Log, LogHead, digest, no_self_join};
    use ambre_core::Role;
    use ambre_crypto::MasterSeed;
    use ambre_protocol::MembershipAct;
    use ed25519_dalek::SigningKey;

    fn rotator() -> SigningKey {
        MasterSeed::from_array([0x44; 32]).derive_signing_key()
    }

    /// `n + 1` keys, oldest first, and the `n` links between them.
    fn rotations(document_id: Id<Document>, n: usize) -> (Vec<DocumentKey>, Vec<KeyLink>) {
        let keys: Vec<_> = (0..=n).map(|_| DocumentKey::generate()).collect();
        let links = keys
            .windows(2)
            .map(|pair| {
                pipeline::seal_link(
                    document_id,
                    &pair[0],
                    &pair[1],
                    ambre_protocol::LogHead::EMPTY,
                    &rotator(),
                )
                .unwrap()
            })
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
        let forged = pipeline::seal_link(
            document_id,
            &DocumentKey::generate(),
            &keys[3],
            ambre_protocol::LogHead::EMPTY,
            &rotator(),
        )
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
        tampered.sealed = ambre_core::Payload::new(sealed);
        let chain = Chain::new(document_id, [tampered]);
        assert!(chain.recover(document_id, &keys[1], keys[0].id()).is_none());

        let elsewhere = Chain::new(Id::<Document>::mint(), links);
        assert_eq!(elsewhere.len(), 0);
    }

    /// The log Alice created, where she invited Bob as an editor, then
    /// revoked him. Its heads, in order, are the empty head and the head
    /// after each of the three entries.
    fn revoked_log() -> (Person, Person, Vec<LogHead>, Log) {
        use crate::membership_log::tests::{DOC, append, created};
        let (alice, bob) = (Person::new(1), Person::new(2));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Editor,
        );
        let heads = std::iter::once(LogHead::EMPTY)
            .chain(entries.iter().scan(LogHead::EMPTY, |head, e| {
                *head = head.after(digest(e).unwrap());
                Some(*head)
            }))
            .collect();
        let anchor = entries.first().and_then(digest);
        let log = Log::walk(DOC, anchor, &entries, no_self_join);
        (alice, bob, heads, log)
    }

    /// Holds `SYNC-121`. The walk drops a link whose signer holds no writing
    /// role in the log, so the key it seals is out of reach.
    /// A document with no log keeps every link.
    #[test]
    fn a_link_no_writer_signed_is_dropped() {
        let (alice, _bob, heads, log) = revoked_log();
        let document_id = Id::<Document>::mint();
        let keys: Vec<_> = (0..3).map(|_| DocumentKey::generate()).collect();
        let stranger = MasterSeed::from_array([0x55; 32]).derive_signing_key();
        let links = vec![
            pipeline::seal_link(document_id, &keys[0], &keys[1], heads[3], &stranger).unwrap(),
            pipeline::seal_link(document_id, &keys[1], &keys[2], heads[3], &alice.key).unwrap(),
        ];
        let kept = signed_by_writers(links.clone(), &log.link_signers());
        assert_eq!(kept.len(), 1);
        let chain = Chain::new(document_id, kept);
        assert!(chain.recover(document_id, &keys[2], keys[1].id()).is_some());
        assert!(chain.recover(document_id, &keys[2], keys[0].id()).is_none());
        assert_eq!(signed_by_writers(links, &LinkSigners::Unchecked).len(), 2);
    }

    /// Holds `SYNC-204`. Bob signs a link at the head before Alice revoked
    /// him, and the walk keeps it. The same link at the head past his revoke
    /// is dropped, as is a link at a head no valid entry reaches. Alice still
    /// signs at that head.
    #[test]
    fn a_link_signed_past_its_signers_revoke_is_dropped() {
        let (alice, bob, heads, log) = revoked_log();
        let document_id = Id::<Document>::mint();
        let (k0, k1) = (DocumentKey::generate(), DocumentKey::generate());
        let link = |head, signer: &Person| {
            pipeline::seal_link(document_id, &k0, &k1, head, &signer.key).unwrap()
        };
        let signers = log.link_signers();
        let kept = |link| signed_by_writers(vec![link], &signers).len();
        assert_eq!(
            kept(link(heads[2], &bob)),
            1,
            "Bob writes before his revoke"
        );
        assert_eq!(kept(link(heads[3], &bob)), 0, "Bob writes no more after it");
        assert_eq!(
            kept(link(heads[1], &bob)),
            0,
            "Bob writes only once invited"
        );
        assert_eq!(kept(link(heads[3], &alice)), 1);
        assert_eq!(kept(link(LogHead::EMPTY, &alice)), 0);
        let forged = LogHead {
            previous: Some(FixedBytes::from_array([9; 32])),
            ..heads[3]
        };
        assert_eq!(
            kept(link(forged, &alice)),
            0,
            "a head the log never reached"
        );
        let beyond = heads[3].after(FixedBytes::from_array([9; 32]));
        assert_eq!(kept(link(beyond, &alice)), 0, "a head past the walk");
    }

    /// Holds `SYNC-204`. The seal binds the head the link names: a link whose
    /// head the relay moved opens nothing.
    #[test]
    fn a_link_whose_head_moved_opens_nothing() {
        let (_, _, heads, _) = revoked_log();
        let document_id = Id::<Document>::mint();
        let (k0, k1) = (DocumentKey::generate(), DocumentKey::generate());
        let link = pipeline::seal_link(document_id, &k0, &k1, heads[3], &rotator()).unwrap();
        assert!(pipeline::open_link(&link, document_id, &k1).is_ok());
        let moved = KeyLink {
            head: heads[2],
            ..link
        };
        assert!(pipeline::open_link(&moved, document_id, &k1).is_err());
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
