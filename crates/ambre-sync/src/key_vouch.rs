//! Whether the key a members answer hands this client is one a writer chose
//! (`ambre-spec-membership.md section 3`).
//!
//! The relay names the wrapper of each copy, so it can wrap a key of its own
//! under an exchange key of its own. The unwrap then succeeds, and every
//! delta this client seals after it opens for the relay. Before a key becomes
//! current, the client checks three things against the signed membership
//! log, and not against the relay's word:
//!
//! 1. the wrapper is this client, or a member the log verifies (`SYNC-201`);
//! 2. a writer the log verifies named the key: in a log entry, or as the key
//!    a link it signed starts from (`SYNC-200`);
//! 3. no link a writer signed replaced the key (`SYNC-202`).
//!
//! A document of a Murmure channel is out of reach of the three checks. Its
//! rows and its links are those of the channel's `meta`, and its log is its
//! own. So no log the open walks names its key. The open does not yet name
//! the `meta` whose log would.

use std::collections::HashSet;

use ambre_bus::{DocType, MemberStanding};
use ambre_core::id::{Document, Id};
use ambre_core::{ExchangePublicKey, FixedBytes, KeyId};
use ambre_crypto::DocumentKey;
use ambre_protocol::KeyLink;

use crate::http;
use crate::membership_log::{Log, LogRule};
use crate::pipeline;

/// Why a key a members answer hands this client cannot become current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyRefusal {
    /// The copy carried no anchor of the log. No entry is then checked, so
    /// no writer the log verifies can name the key.
    Unanchored,
    /// The copy was wrapped under an exchange key that neither this client
    /// nor a member the log verifies holds.
    WrapperUnverified,
    /// No writer the log verifies named the key, in an entry or in a link.
    Unvouched,
    /// A link a writer signed starts from a newer key and seals this one.
    Superseded,
}

/// Who wrapped the copy of this client, as the log sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrapper {
    /// This client wrapped its own copy. The relay cannot build such a copy:
    /// the exchange needs the secret half of the key on both sides.
    Holder,
    /// A row the log verifies as a member carries the wrapper's key.
    Member,
    /// No row carries the wrapper's key, and the log of a space governs this
    /// one. A space writer re-keys a channel without being a member of it.
    Space,
    /// Any other wrapper, this client's own row absent included.
    Unverified,
}

/// What one members answer and its log say about the key it hands over.
pub(crate) struct KeyVouch {
    /// The keys a writer named in a valid entry. `None` when the log carries
    /// no anchor.
    named: Option<HashSet<FixedBytes<KeyId>>>,
    wrapper: Wrapper,
}

impl KeyVouch {
    /// Reads `log`, walked under `rule`, and the rows of `members`, for the
    /// copy carried by the row of `own`, this client's exchange key.
    pub(crate) fn of(
        log: &Log,
        rule: &LogRule,
        own: &FixedBytes<ExchangePublicKey>,
        members: &[http::MemberEntry],
    ) -> Self {
        // The same wrapper the unwrap runs against (`SYNC-034`).
        let wrapper_key = members
            .iter()
            .find(|m| m.exchange_pub_key.key() == Some(own))
            .map(|m| m.wrapper_exchange_pub_key.key().unwrap_or(own));
        let wrapper = match wrapper_key {
            None => Wrapper::Unverified,
            Some(key) if key == own => Wrapper::Holder,
            Some(key) => {
                let mut rows = members
                    .iter()
                    .filter(|m| m.exchange_pub_key.key() == Some(key))
                    .peekable();
                let carried = rows.peek().is_some();
                if rows.any(|m| log.standing(m.user_id).0 == MemberStanding::Verified) {
                    Wrapper::Member
                } else if !carried && rule.space.is_some() {
                    Wrapper::Space
                } else {
                    Wrapper::Unverified
                }
            }
        };
        Self {
            named: log.named_keys().cloned(),
            wrapper,
        }
    }

    /// Whether `key` may become the current key of `document_id`, a document
    /// of `doc_type`. A Murmure document passes unchecked.
    ///
    /// `links` are the links of the document that a writer the log verifies
    /// signed (`SYNC-121`). A link that names `key` counts only when it opens
    /// under `key`, so its signature and its key commitment hold.
    ///
    /// # Errors
    ///
    /// The first [`KeyRefusal`] that applies, in the order the variants are
    /// declared.
    pub(crate) fn check(
        &self,
        doc_type: DocType,
        document_id: Id<Document>,
        key: &DocumentKey,
        links: &[KeyLink],
    ) -> Result<(), KeyRefusal> {
        // A Murmure document is a channel's, which no log the open walks
        // describes.
        if doc_type == DocType::Murmure {
            return Ok(());
        }
        let named = self.named.as_ref().ok_or(KeyRefusal::Unanchored)?;
        if self.wrapper == Wrapper::Unverified {
            return Err(KeyRefusal::WrapperUnverified);
        }
        let id = key.id();
        let vouched = named.contains(&id)
            || links
                .iter()
                .any(|link| link.key == id && pipeline::open_link(link, document_id, key).is_ok());
        if !vouched {
            return Err(KeyRefusal::Unvouched);
        }
        // This client cannot open a link from a key it lacks. A forged link
        // here costs the open and nothing more, which the relay can cost
        // anyway by serving nothing.
        if links.iter().any(|link| link.previous == id) {
            return Err(KeyRefusal::Superseded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership_log::tests::Person;
    use crate::membership_log::{Draft, LogHead, digest, no_self_join, sign};
    use ambre_core::Role;
    use ambre_core::id::User;
    use ambre_protocol::{MembershipAct, MembershipEntry};
    use x25519_dalek::StaticSecret;

    /// One member of a test document: a signing identity and an exchange key.
    struct Member {
        person: Person,
        xk: StaticSecret,
    }

    impl Member {
        fn new(seed: u8) -> Self {
            Self {
                person: Person::new(seed),
                xk: ambre_crypto::MasterSeed::from_array([seed; 32]).derive_exchange_key(),
            }
        }

        fn id(&self) -> Id<User> {
            self.person.id
        }

        fn exchange(&self) -> FixedBytes<ExchangePublicKey> {
            crate::sharing::own_exchange_key(&self.xk)
        }
    }

    /// A document Alice created under `k0` and shared with Bob as an editor.
    struct Shared {
        document_id: Id<Document>,
        alice: Member,
        bob: Member,
        k0: DocumentKey,
        entries: Vec<MembershipEntry>,
    }

    impl Shared {
        fn new() -> Self {
            let document_id = Id::<Document>::mint();
            let alice = Member::new(0x21);
            let bob = Member::new(0x22);
            let k0 = DocumentKey::generate();
            let mut shared = Self {
                document_id,
                alice,
                bob,
                k0,
                entries: Vec::new(),
            };
            let create = shared.entry(&shared.alice, MembershipAct::Create, Role::Owner);
            shared.entries.push(create);
            let invite = shared.entry(&shared.bob, MembershipAct::Invite, Role::Editor);
            shared.entries.push(invite);
            shared
        }

        /// The entry after the last one, signed by Alice, naming `k0`.
        fn entry(&self, member: &Member, act: MembershipAct, role: Role) -> MembershipEntry {
            let head = self
                .entries
                .iter()
                .fold(LogHead::EMPTY, |head, e| head.after(digest(e).unwrap()));
            let draft = Draft {
                document: self.document_id,
                member: member.id(),
                member_key: member.person.public(),
                member_exchange_key: member.person.exchange(),
                act,
                role,
                key: Some(self.k0.id()),
            };
            sign(draft, head, self.alice.id(), &self.alice.person.key).unwrap()
        }

        fn log(&self) -> Log {
            let anchor = self.entries.first().and_then(digest);
            Log::walk(self.document_id, anchor, &self.entries, no_self_join)
        }

        /// The rows of Alice and Bob, Bob's copy of `key` wrapped by `wrapper`.
        fn rows(&self, key: &DocumentKey, wrapper: &StaticSecret) -> Vec<http::MemberEntry> {
            vec![
                row(&self.alice, key, &self.alice.xk),
                row(&self.bob, key, wrapper),
            ]
        }

        /// What Bob's client decides of `key`, served with `links`.
        fn bob_checks(
            &self,
            log: &Log,
            members: &[http::MemberEntry],
            key: &DocumentKey,
            links: &[KeyLink],
        ) -> Result<(), KeyRefusal> {
            KeyVouch::of(log, &LogRule::document(), &self.bob.exchange(), members).check(
                DocType::Prose,
                self.document_id,
                key,
                links,
            )
        }
    }

    /// The row of `member`, its copy of `key` wrapped by `wrapper`.
    fn row(member: &Member, key: &DocumentKey, wrapper: &StaticSecret) -> http::MemberEntry {
        let holder = member.exchange();
        http::MemberEntry {
            user_id: member.id(),
            signing_pub_key: http::ServedKey::of(member.person.public()),
            exchange_pub_key: http::ServedKey::of(holder),
            doc_key_enc: crate::sharing::wrap_anchored_b64(wrapper, &holder, key, None).unwrap(),
            role: Role::Editor,
            wrapper_exchange_pub_key: http::ServedKey::of(crate::sharing::own_exchange_key(
                wrapper,
            )),
        }
    }

    /// Holds `SYNC-200`. The key the creation and the invite name opens, as
    /// does a key a writer's link starts from. A key the relay made, wrapped
    /// under a key of its own and served under the genuine log, is refused.
    #[test]
    fn a_key_no_writer_named_is_refused() {
        let doc = Shared::new();
        let log = doc.log();
        let members = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(doc.bob_checks(&log, &members, &doc.k0, &[]), Ok(()));

        // Alice rotates: the new key is named by the link she signed alone.
        let k1 = DocumentKey::generate();
        let link =
            pipeline::seal_link(doc.document_id, &doc.k0, &k1, &doc.alice.person.key).unwrap();
        let rotated = doc.rows(&k1, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &rotated, &k1, std::slice::from_ref(&link)),
            Ok(())
        );
        assert_eq!(
            doc.bob_checks(&log, &rotated, &k1, &[]),
            Err(KeyRefusal::Unvouched)
        );

        // The relay re-wraps a key of its own for Bob. It puts its exchange
        // key on Alice's row, so the wrapper reads as a verified member.
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let chosen = DocumentKey::generate();
        let mut forged = doc.rows(&chosen, &relay_xk);
        forged[0].exchange_pub_key =
            http::ServedKey::of(crate::sharing::own_exchange_key(&relay_xk));
        assert_eq!(
            doc.bob_checks(&log, &forged, &chosen, std::slice::from_ref(&link)),
            Err(KeyRefusal::Unvouched)
        );
        // A link the relay seals from its own key and signs itself counts for
        // nothing: no writer of the log holds its signing key.
        let relay_signer = ambre_crypto::MasterSeed::from_array([0x67; 32]).derive_signing_key();
        let own_link = pipeline::seal_link(doc.document_id, &k1, &chosen, &relay_signer).unwrap();
        let writers = log.signers().cloned();
        let kept = crate::chain::signed_by_writers(vec![own_link], writers.as_ref());
        assert_eq!(
            doc.bob_checks(&log, &forged, &chosen, &kept),
            Err(KeyRefusal::Unvouched)
        );
    }

    /// Holds `SYNC-201`. A copy Bob wrapped himself, or one a member the log
    /// verifies wrapped, passes. A copy under an exchange key no verified
    /// member holds is refused, whatever key it carries.
    #[test]
    fn a_wrapper_the_log_does_not_verify_is_refused() {
        let doc = Shared::new();
        let log = doc.log();
        let own = doc.rows(&doc.k0, &doc.bob.xk);
        assert_eq!(doc.bob_checks(&log, &own, &doc.k0, &[]), Ok(()));

        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let stranger = doc.rows(&doc.k0, &relay_xk);
        assert_eq!(
            doc.bob_checks(&log, &stranger, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );

        // A row the log never verified carries the wrapper's key.
        let carol = Member::new(0x23);
        let mut with_carol = doc.rows(&doc.k0, &carol.xk);
        with_carol.push(row(&carol, &doc.k0, &carol.xk));
        assert_eq!(
            doc.bob_checks(&log, &with_carol, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );

        // Under a space, a wrapper no row carries is a space writer.
        let mut rule = LogRule::document();
        rule.space = Some(crate::membership_log::SpaceWord::default());
        let vouch = KeyVouch::of(&log, &rule, &doc.bob.exchange(), &stranger);
        assert_eq!(
            vouch.check(DocType::Prose, doc.document_id, &doc.k0, &[]),
            Ok(())
        );
        let vouch = KeyVouch::of(&log, &rule, &doc.bob.exchange(), &with_carol);
        assert_eq!(
            vouch.check(DocType::Prose, doc.document_id, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );
    }

    /// Holds `SYNC-202`. A key that a writer's link replaced is refused, so
    /// the relay cannot hand back a copy it kept from before a rotation.
    #[test]
    fn a_key_a_writer_replaced_is_refused() {
        let doc = Shared::new();
        let log = doc.log();
        let k1 = DocumentKey::generate();
        let link =
            pipeline::seal_link(doc.document_id, &doc.k0, &k1, &doc.alice.person.key).unwrap();
        let replayed = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &replayed, &doc.k0, &[link]),
            Err(KeyRefusal::Superseded)
        );
    }

    /// A Murmure document skips the checks, and every other kind runs them.
    #[test]
    fn a_channel_document_skips_the_checks() {
        let doc = Shared::new();
        let log = doc.log();
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let stranger = doc.rows(&doc.k0, &relay_xk);
        let vouch = KeyVouch::of(&log, &LogRule::document(), &doc.bob.exchange(), &stranger);
        let checked = |doc_type| vouch.check(doc_type, doc.document_id, &doc.k0, &[]);
        assert_eq!(checked(DocType::Murmure), Ok(()));
        for doc_type in [DocType::Prose, DocType::Trame, DocType::Toile] {
            assert_eq!(checked(doc_type), Err(KeyRefusal::WrapperUnverified));
        }
    }

    /// Holds `SYNC-200` for a log with no anchor: no entry is checked, so no
    /// key is named, and the copy is refused. A log whose first entry does not
    /// hash to the anchor names no key either.
    #[test]
    fn a_copy_with_no_anchored_log_is_refused() {
        let doc = Shared::new();
        let members = doc.rows(&doc.k0, &doc.alice.xk);
        let unanchored = Log::walk(doc.document_id, None, &doc.entries, no_self_join);
        assert_eq!(
            doc.bob_checks(&unanchored, &members, &doc.k0, &[]),
            Err(KeyRefusal::Unanchored)
        );
        let elsewhere = Log::walk(
            doc.document_id,
            Some(FixedBytes::from_array([9; 32])),
            &doc.entries,
            no_self_join,
        );
        assert_eq!(
            doc.bob_checks(&elsewhere, &members, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );
    }
}
