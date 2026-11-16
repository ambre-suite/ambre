//! Whether the key a members answer hands this client is one a writer chose
//! (`ambre-spec-membership.md section 3`).
//!
//! The relay names the wrapper of each copy, so it can wrap a key of its own
//! under an exchange key of its own. The unwrap then succeeds, and every
//! delta this client seals after it opens for the relay. Before a key becomes
//! current, the client checks five things against the signed membership
//! log, and not against the relay's word:
//!
//! 1. the wrapper is this client, or a member the log verifies (`SYNC-201`);
//! 2. a writer the log verifies named the key: in a log entry, or as the key
//!    a link it signed starts from (`SYNC-200`);
//! 3. no link a writer signed replaced the key (`SYNC-202`);
//! 4. a key only a link names counts when no writer named another key at or
//!    past the head the link signs (`SYNC-221`);
//! 5. a key in force before the latest departure counts only while the
//!    members answer says the rotation is owed (`SYNC-222`).
//!
//! A link counts only when its signer held a writing role at the log head it
//! names (`SYNC-204`). A revoked writer can still name a head before its
//! revoke, which the fourth check answers. A departure names no key, so the
//! fourth check misses a leaver's link, and the fifth one answers it.
//!
//! The anchor of the log travels inside the copy, which the relay may have
//! wrapped. So the five checks read a log the relay could root at a creator
//! of its own. An open that names a pinned inviter, from the invite link,
//! first checks that the inviter signed an entry of that log (`SYNC-205`).
//! An open with no link fails closed (`SYNC-210`). It passes only on a copy
//! this client wrapped, or on a log it signed an entry of itself.
//!
//! A document of a Murmure channel is out of reach of the five checks. Its
//! rows and its links are those of the channel's `meta`, and its log is its
//! own. So no log the open walks names its key. The open does not yet name
//! the `meta` whose log would.

use ambre_bus::DocType;
use ambre_core::id::{Document, Id};
use ambre_core::{ExchangePublicKey, FixedBytes, InviterPin, PublicKey, RotationDebt, Seq};
use ambre_crypto::DocumentKey;
use ambre_protocol::{KeyLink, LogPlaces};

use crate::http;
use crate::membership_log::{Log, LogRule, NamedKey};
use crate::pipeline;

/// Why a key a members answer hands this client cannot become current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyRefusal {
    /// The copy carried no anchor of the log. No entry is then checked, so
    /// no writer the log verifies can name the key.
    Unanchored,
    /// The open names a pinned inviter, and no valid entry of the log is
    /// signed by the key the pin names.
    NotFromInviter,
    /// The open names no pinned inviter, and this client did not wrap its own
    /// copy. Nor did its own signing key sign a valid entry of the log, so
    /// nothing roots the log but the relay's word.
    Unpinned,
    /// The copy was wrapped under an exchange key that neither this client
    /// nor a member the log verifies holds.
    WrapperUnverified,
    /// No writer the log verifies named the key, in an entry or in a link.
    Unvouched,
    /// Only a link vouches for the key, and a writer named another key at or
    /// past the head the link names. The link predates the log it claims.
    Backdated,
    /// The key was in force before a member left on their own, so the leaver
    /// holds it. The members answer says no rotation is owed, yet a debt
    /// paid leaves a key a writer chose past the departure.
    Departed,
    /// A link a writer signed starts from a newer key and seals this one.
    Superseded,
}

impl KeyRefusal {
    /// The failure the open reports for this refusal (`SYNC-173`, `SYNC-212`).
    ///
    /// Two refusals mean that no member signed the key: no writer named it,
    /// or the pinned inviter signed no entry of the log. The relay chose that
    /// key, and the open names the fault. An honest relay can cause each other
    /// refusal, so each stays a key not received. Examples are an invitee with
    /// no link, a copy a newer key replaced, and a wrapper an old log does not
    /// verify.
    pub(crate) fn open_failure(self) -> ambre_bus::OpenFailure {
        match self {
            Self::NotFromInviter | Self::Unvouched => ambre_bus::OpenFailure::KeyUnsigned,
            Self::Unanchored
            | Self::Unpinned
            | Self::WrapperUnverified
            | Self::Superseded
            | Self::Backdated
            | Self::Departed => {
                ambre_bus::OpenFailure::KeyNotReceived
            }
        }
    }
}

/// Who wrapped the copy of this client, as the log sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wrapper {
    /// This client wrapped its own copy. The relay cannot build such a copy:
    /// the exchange needs the secret half of the key on both sides.
    Holder,
    /// A valid entry signs the wrapper's key as the exchange key of a member
    /// the log keeps. The key the relay serves on a row counts for nothing.
    Member,
    /// No row carries the wrapper's key, and the log of a space governs this
    /// one. A space writer re-keys a channel without being a member of it.
    Space,
    /// Any other wrapper, this client's own row absent included.
    Unverified,
}

/// What the log says of the inviter the open pinned out of band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inviter {
    /// The open names no pin, and this client's own signing key signed no
    /// valid entry of the log.
    Unpinned,
    /// The open names no pin, and this client's own signing key signed a
    /// valid entry of the log. The relay cannot sign such an entry either.
    Own,
    /// The key the pin names signed a valid entry of the log.
    Signed,
    /// No valid entry of the log is signed by the key the pin names.
    Absent,
}

/// What one members answer and its log say about the key it hands over.
pub(crate) struct KeyVouch {
    /// The keys a writer named in a valid entry, each at its place. `None`
    /// when the log carries no anchor.
    named: Option<Vec<NamedKey>>,
    wrapper: Wrapper,
    inviter: Inviter,
    /// The place of the latest departure the log holds (`SYNC-222`).
    departed: Option<Seq<LogPlaces>>,
    /// Whether the members answer says a rotation is owed.
    debt: RotationDebt,
}

impl KeyVouch {
    /// Reads `log`, walked under `rule`, and the rows of `members`, for the
    /// copy carried by the row of `own`, this client's exchange key. `pin`
    /// is the inviter this client holds out of band for the document, and
    /// `signer` this client's own signing key. `debt` is what the same
    /// answer says of the rotation a departure owes.
    pub(crate) fn of(
        log: &Log,
        rule: &LogRule,
        own: &FixedBytes<ExchangePublicKey>,
        members: &[http::MemberEntry],
        pin: Option<&FixedBytes<InviterPin>>,
        signer: Option<&FixedBytes<PublicKey>>,
        debt: RotationDebt,
    ) -> Self {
        // The same wrapper the unwrap runs against (`SYNC-034`).
        let wrapper_key = members
            .iter()
            .find(|m| m.exchange_pub_key.key() == Some(own))
            .map(|m| m.wrapper_exchange_pub_key.key().unwrap_or(own));
        let wrapper = match wrapper_key {
            None => Wrapper::Unverified,
            Some(key) if key == own => Wrapper::Holder,
            // The exchange key a row serves carries no signature, so the
            // relay can put its own on the row of a verified member. Only the
            // key the log signs for a member vouches for the wrapper.
            Some(key) if log.signs_exchange_key(key) => Wrapper::Member,
            Some(key) => {
                let carried = members
                    .iter()
                    .any(|m| m.exchange_pub_key.key() == Some(key));
                if !carried && rule.space.is_some() {
                    Wrapper::Space
                } else {
                    Wrapper::Unverified
                }
            }
        };
        let inviter = match pin {
            None if signer.is_some_and(|key| log.signed_under(&ambre_crypto::inviter_pin(key))) => {
                Inviter::Own
            }
            None => Inviter::Unpinned,
            Some(pin) if log.signed_under(pin) => Inviter::Signed,
            Some(_) => Inviter::Absent,
        };
        Self {
            named: log.named_at().map(<[NamedKey]>::to_vec),
            wrapper,
            inviter,
            departed: log.departed_at(),
            debt,
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
        if self.inviter == Inviter::Absent {
            return Err(KeyRefusal::NotFromInviter);
        }
        // An open with no link fails closed (`SYNC-210`).
        if self.inviter == Inviter::Unpinned && self.wrapper != Wrapper::Holder {
            return Err(KeyRefusal::Unpinned);
        }
        if self.wrapper == Wrapper::Unverified {
            return Err(KeyRefusal::WrapperUnverified);
        }
        let id = key.id();
        let vouching = || {
            links.iter().filter(move |link| {
                link.key == id && pipeline::open_link(link, document_id, key).is_ok()
            })
        };
        if !named.iter().any(|n| n.key == id) {
            if vouching().next().is_none() {
                return Err(KeyRefusal::Unvouched);
            }
            // A writer that installed another key at or past the head the
            // link names left this one behind (`SYNC-221`).
            let stale = |link: &KeyLink| {
                named
                    .iter()
                    .any(|n| n.place >= link.head.position && n.key != link.key)
            };
            if vouching().all(stale) {
                return Err(KeyRefusal::Backdated);
            }
        }
        // A leaver holds every key in force before it left, and names no
        // key to leave a backdated link behind (`SYNC-222`). Only a debt
        // still owed lets such a key stand: a writer pays it before it seals.
        if self.debt == RotationDebt::Settled
            && let Some(left) = self.departed
        {
            let held = named.iter().any(|n| n.key == id && n.place < left)
                || vouching().any(|link| link.head.position <= left);
            if held {
                return Err(KeyRefusal::Departed);
            }
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
            Self::created_by(Id::<Document>::mint(), Member::new(0x21))
        }

        /// The document `document_id` that `alice` created and shared with
        /// Bob, whoever holds her keys.
        fn created_by(document_id: Id<Document>, alice: Member) -> Self {
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
            self.entry_naming(member, act, role, self.k0.id())
        }

        /// The entry after the last one, signed by Alice, naming `key`.
        fn entry_naming(
            &self,
            member: &Member,
            act: MembershipAct,
            role: Role,
            key: FixedBytes<ambre_core::KeyId>,
        ) -> MembershipEntry {
            self.entry_by(&self.alice, member, act, role, Some(key))
        }

        /// The entry after the last one, signed by `author`, naming `key`.
        fn entry_by(
            &self,
            author: &Member,
            member: &Member,
            act: MembershipAct,
            role: Role,
            key: Option<FixedBytes<ambre_core::KeyId>>,
        ) -> MembershipEntry {
            let head = self
                .entries
                .iter()
                .fold(LogHead::EMPTY, |head, e| head.after(digest(e).unwrap()));
            let draft = Draft {
                document: self.document_id,
                member: member.id(),
                member_key: member.person.public(),
                member_exchange_key: member.exchange(),
                act,
                role,
                key,
            };
            sign(draft, head, author.id(), &author.person.key).unwrap()
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

        /// What Bob's client decides of `key`, the open pinning `pin`.
        fn bob_checks_pinned(
            &self,
            log: &Log,
            members: &[http::MemberEntry],
            key: &DocumentKey,
            pin: &FixedBytes<InviterPin>,
        ) -> Result<(), KeyRefusal> {
            let own = self.bob.exchange();
            KeyVouch::of(
                log,
                &LogRule::document(),
                &own,
                members,
                Some(pin),
                None,
                RotationDebt::Settled,
            )
            .check(DocType::Prose, self.document_id, key, &[])
        }

        /// The pin of Alice's signing key, which her invite link carries.
        fn alice_pin(&self) -> FixedBytes<InviterPin> {
            ambre_crypto::inviter_pin(&self.alice.person.public())
        }

        /// What Bob's client decides of `key`, served with `links`, opened
        /// from the link Alice sent him.
        fn bob_checks(
            &self,
            log: &Log,
            members: &[http::MemberEntry],
            key: &DocumentKey,
            links: &[KeyLink],
        ) -> Result<(), KeyRefusal> {
            KeyVouch::of(
                log,
                &LogRule::document(),
                &self.bob.exchange(),
                members,
                Some(&self.alice_pin()),
                None,
                RotationDebt::Settled,
            )
            .check(DocType::Prose, self.document_id, key, links)
        }

        /// What Bob's client decides of `key`, opened from Alice's link, in
        /// a document the log of a space governs.
        fn bob_checks_under_space(
            &self,
            log: &Log,
            members: &[http::MemberEntry],
            key: &DocumentKey,
            links: &[KeyLink],
        ) -> Result<(), KeyRefusal> {
            let mut rule = LogRule::document();
            rule.space = Some(crate::membership_log::SpaceWord::default());
            KeyVouch::of(
                log,
                &rule,
                &self.bob.exchange(),
                members,
                Some(&self.alice_pin()),
                None,
                RotationDebt::Settled,
            )
            .check(DocType::Prose, self.document_id, key, links)
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

    /// Holds `SYNC-212`: only a key no member signed fails the open as the
    /// relay's fault. A refusal an honest relay can cause stays a key not
    /// received, so the user sees no false alarm.
    #[test]
    fn only_a_key_no_member_signed_fails_as_the_relays_fault() {
        use ambre_bus::OpenFailure;
        let cases = [
            (KeyRefusal::Unvouched, OpenFailure::KeyUnsigned),
            (KeyRefusal::NotFromInviter, OpenFailure::KeyUnsigned),
            (KeyRefusal::Unanchored, OpenFailure::KeyNotReceived),
            (KeyRefusal::Unpinned, OpenFailure::KeyNotReceived),
            (KeyRefusal::WrapperUnverified, OpenFailure::KeyNotReceived),
            (KeyRefusal::Superseded, OpenFailure::KeyNotReceived),
            (KeyRefusal::Backdated, OpenFailure::KeyNotReceived),
            (KeyRefusal::Departed, OpenFailure::KeyNotReceived),
        ];
        for (refusal, failure) in cases {
            assert_eq!(refusal.open_failure(), failure, "{refusal:?}");
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
        let link = pipeline::seal_link(
            doc.document_id,
            &doc.k0,
            &k1,
            log.next(),
            &doc.alice.person.key,
        )
        .unwrap();
        let rotated = doc.rows(&k1, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &rotated, &k1, std::slice::from_ref(&link)),
            Ok(())
        );
        assert_eq!(
            doc.bob_checks(&log, &rotated, &k1, &[]),
            Err(KeyRefusal::Unvouched)
        );

        // The relay re-wraps a key of its own for Bob. Under a space, a
        // wrapper no row carries passes as a space writer, so the key check
        // alone stands between the relay and the open.
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let chosen = DocumentKey::generate();
        let forged = doc.rows(&chosen, &relay_xk);
        assert_eq!(
            doc.bob_checks_under_space(&log, &forged, &chosen, std::slice::from_ref(&link)),
            Err(KeyRefusal::Unvouched)
        );
        // A link the relay seals from its own key and signs itself counts for
        // nothing: no writer of the log holds its signing key.
        let relay_signer = ambre_crypto::MasterSeed::from_array([0x67; 32]).derive_signing_key();
        let own_link =
            pipeline::seal_link(doc.document_id, &k1, &chosen, log.next(), &relay_signer).unwrap();
        let kept = crate::chain::signed_by_writers(vec![own_link], &log.link_signers());
        assert_eq!(
            doc.bob_checks_under_space(&log, &forged, &chosen, &kept),
            Err(KeyRefusal::Unvouched)
        );
    }

    /// Holds `SYNC-201` against a swap. The relay wraps Bob's copy under its
    /// own exchange key, and serves that key on the row of a verified member.
    /// The check reads the key the log signs, so the swap is refused, and a
    /// withheld row takes no vouch away.
    #[test]
    fn a_wrapper_key_the_relay_swaps_on_a_row_is_refused() {
        let doc = Shared::new();
        let log = doc.log();
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        // The relay wraps Bob's copy under its own exchange key, and serves
        // that key on Alice's row. Alice is a verified member, but the log
        // signs her own key: the swap is refused, even for a named key.
        let mut swapped = doc.rows(&doc.k0, &relay_xk);
        swapped[0].exchange_pub_key =
            http::ServedKey::of(crate::sharing::own_exchange_key(&relay_xk));
        assert_eq!(
            doc.bob_checks(&log, &swapped, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );
        assert_eq!(
            doc.bob_checks_under_space(&log, &swapped, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified),
            "a key a row carries is no space writer's"
        );

        // A verified member whose row the relay withholds still vouches:
        // the log signs her key, and the row was never the evidence.
        let alice_wraps = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &alice_wraps[1..], &doc.k0, &[]),
            Ok(())
        );
    }

    /// Holds `SYNC-201`. A copy Bob wrapped himself, or one wrapped under the
    /// exchange key the log signs for a verified member, passes. A copy under
    /// any other exchange key is refused, whatever key it carries and
    /// whichever row the relay serves that exchange key on.
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
        let pin = doc.alice_pin();
        let vouch = KeyVouch::of(
            &log,
            &rule,
            &doc.bob.exchange(),
            &stranger,
            Some(&pin),
            None,
            RotationDebt::Settled,
        );
        assert_eq!(
            vouch.check(DocType::Prose, doc.document_id, &doc.k0, &[]),
            Ok(())
        );
        let vouch = KeyVouch::of(
            &log,
            &rule,
            &doc.bob.exchange(),
            &with_carol,
            Some(&pin),
            None,
            RotationDebt::Settled,
        );
        assert_eq!(
            vouch.check(DocType::Prose, doc.document_id, &doc.k0, &[]),
            Err(KeyRefusal::WrapperUnverified)
        );
    }

    /// Holds `SYNC-204` and `SYNC-221`, audit `N14`. Alice revokes Bob, an
    /// editor, and her revoke names `k1`. Bob keeps his signing key and, with
    /// the relay, hands Carol a key of his own. His link at the head past his
    /// revoke is dropped, so the key is unvouched. His link at a head before
    /// his revoke is kept, and refused as backdated. The wrapper check is held
    /// apart, so Alice's row wraps every copy here.
    #[test]
    fn a_key_a_revoked_writer_links_is_refused() {
        let mut doc = Shared::new();
        let carol = Member::new(0x23);
        let invite = doc.entry(&carol, MembershipAct::Invite, Role::Editor);
        doc.entries.push(invite);
        let before_revoke = doc.log().next();
        let k1 = DocumentKey::generate();
        let revoke = doc.entry_naming(&doc.bob, MembershipAct::Revoke, Role::Editor, k1.id());
        doc.entries.push(revoke);
        let log = doc.log();
        let carol_checks = |key: &DocumentKey, links: Vec<KeyLink>| {
            let kept = crate::chain::signed_by_writers(links, &log.link_signers());
            let rows = vec![
                row(&doc.alice, key, &doc.alice.xk),
                row(&carol, key, &doc.alice.xk),
            ];
            let pin = doc.alice_pin();
            KeyVouch::of(
                &log,
                &LogRule::document(),
                &carol.exchange(),
                &rows,
                Some(&pin),
                None,
                RotationDebt::Settled,
            )
            .check(DocType::Prose, doc.document_id, key, &kept)
        };
        let seal = |previous: &DocumentKey, next: &DocumentKey, head, by: &Member| {
            pipeline::seal_link(doc.document_id, previous, next, head, &by.person.key).unwrap()
        };

        // Alice's rotation signs the head she read before her revoke.
        assert_eq!(
            carol_checks(&k1, vec![seal(&doc.k0, &k1, before_revoke, &doc.alice)]),
            Ok(())
        );
        let chosen = DocumentKey::generate();
        assert_eq!(
            carol_checks(&chosen, vec![seal(&k1, &chosen, log.next(), &doc.bob)]),
            Err(KeyRefusal::Unvouched)
        );
        let backdated = seal(&doc.k0, &chosen, before_revoke, &doc.bob);
        let kept = crate::chain::signed_by_writers(vec![backdated.clone()], &log.link_signers());
        assert_eq!(kept.len(), 1, "Bob wrote at that head");
        assert_eq!(
            carol_checks(&chosen, vec![backdated]),
            Err(KeyRefusal::Backdated)
        );
    }

    /// Holds `SYNC-222`. Alice rotates to `k1`, then Bob, an editor, leaves,
    /// and his departure names no key. With the relay, he hands Carol a key
    /// of his own. His link names a head before he left, so no writer named
    /// another key past it. While the answer owes no rotation, that key is refused, and so
    /// are `k0` and `k1`, which Bob held. A debt still owed lets each one
    /// stand, and the key a writer pays the debt with stands once settled.
    #[test]
    fn a_key_held_before_a_departure_is_refused_unless_the_debt_is_owed() {
        let mut doc = Shared::new();
        let carol = Member::new(0x23);
        let invite = doc.entry(&carol, MembershipAct::Invite, Role::Editor);
        doc.entries.push(invite);
        let before_leave = doc.log().next();
        let leave = doc.entry_by(
            &doc.bob,
            &doc.bob,
            MembershipAct::Revoke,
            Role::Editor,
            None,
        );
        doc.entries.push(leave);
        let log = doc.log();
        let carol_checks = |key: &DocumentKey, links: &[KeyLink], debt| {
            let kept = crate::chain::signed_by_writers(links.to_vec(), &log.link_signers());
            let rows = vec![
                row(&doc.alice, key, &doc.alice.xk),
                row(&carol, key, &doc.alice.xk),
            ];
            let pin = doc.alice_pin();
            KeyVouch::of(
                &log,
                &LogRule::document(),
                &carol.exchange(),
                &rows,
                Some(&pin),
                None,
                debt,
            )
            .check(DocType::Prose, doc.document_id, key, &kept)
        };
        let seal = |previous: &DocumentKey, next: &DocumentKey, head, by: &Member| {
            pipeline::seal_link(doc.document_id, previous, next, head, &by.person.key).unwrap()
        };
        let k1 = DocumentKey::generate();
        let rotation = seal(&doc.k0, &k1, before_leave, &doc.alice);
        let chosen = DocumentKey::generate();
        let backdated = seal(&k1, &chosen, before_leave, &doc.bob);
        let served = [rotation.clone(), backdated];
        let kept = crate::chain::signed_by_writers(served.to_vec(), &log.link_signers());
        assert_eq!(kept.len(), 2, "Bob wrote at that head");

        assert_eq!(
            carol_checks(&chosen, &served, RotationDebt::Settled),
            Err(KeyRefusal::Departed)
        );
        assert_eq!(carol_checks(&chosen, &served, RotationDebt::Owed), Ok(()));
        let alone = std::slice::from_ref(&rotation);
        assert_eq!(
            carol_checks(&k1, alone, RotationDebt::Settled),
            Err(KeyRefusal::Departed)
        );
        assert_eq!(carol_checks(&k1, alone, RotationDebt::Owed), Ok(()));
        assert_eq!(
            carol_checks(&doc.k0, &[], RotationDebt::Settled),
            Err(KeyRefusal::Departed)
        );
        assert_eq!(carol_checks(&doc.k0, &[], RotationDebt::Owed), Ok(()));

        // Carol pays the debt at the head past the departure.
        let k2 = DocumentKey::generate();
        let paid = seal(&k1, &k2, log.next(), &carol);
        assert_eq!(
            carol_checks(&k2, &[rotation, paid], RotationDebt::Settled),
            Ok(())
        );
    }

    /// Holds `SYNC-222`. Bob leaves with an entry that names a key of his
    /// own. A departure names no key, so the key stays one no writer named,
    /// whatever the answer says of the debt.
    #[test]
    fn a_departure_names_no_key() {
        let mut doc = Shared::new();
        let chosen = DocumentKey::generate();
        let leave = doc.entry_by(
            &doc.bob,
            &doc.bob,
            MembershipAct::Revoke,
            Role::Editor,
            Some(chosen.id()),
        );
        let left = leave.position;
        doc.entries.push(leave);
        let log = doc.log();
        assert_eq!(log.departed_at(), Some(left));
        let rows = vec![row(&doc.alice, &chosen, &doc.alice.xk)];
        let pin = doc.alice_pin();
        for debt in [RotationDebt::Settled, RotationDebt::Owed] {
            let vouch = KeyVouch::of(
                &log,
                &LogRule::document(),
                &doc.alice.exchange(),
                &rows,
                Some(&pin),
                None,
                debt,
            );
            assert_eq!(
                vouch.check(DocType::Prose, doc.document_id, &chosen, &[]),
                Err(KeyRefusal::Unvouched)
            );
        }
    }

    /// Holds `SYNC-202`. A key that a writer's link replaced is refused, so
    /// the relay cannot hand back a copy it kept from before a rotation.
    #[test]
    fn a_key_a_writer_replaced_is_refused() {
        let doc = Shared::new();
        let log = doc.log();
        let k1 = DocumentKey::generate();
        let link = pipeline::seal_link(
            doc.document_id,
            &doc.k0,
            &k1,
            log.next(),
            &doc.alice.person.key,
        )
        .unwrap();
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
        let vouch = KeyVouch::of(
            &log,
            &LogRule::document(),
            &doc.bob.exchange(),
            &stranger,
            Some(&doc.alice_pin()),
            None,
            RotationDebt::Settled,
        );
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
        // That log walks no entry, so the inviter the link pins signed none.
        assert_eq!(
            doc.bob_checks(&elsewhere, &members, &doc.k0, &[]),
            Err(KeyRefusal::NotFromInviter)
        );
    }

    /// Holds `SYNC-205`. The relay builds a whole log of its own. A creator
    /// whose keys it holds invites Bob, and the relay wraps a key of its own
    /// for him. Every check passes on the relay's word alone. The pin of Alice,
    /// which Bob holds from her link, refuses that log and admits hers.
    #[test]
    fn a_log_the_pinned_inviter_did_not_sign_is_refused() {
        let doc = Shared::new();
        let alice_pin = ambre_crypto::inviter_pin(&doc.alice.person.public());
        let honest = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks_pinned(&doc.log(), &honest, &doc.k0, &alice_pin),
            Ok(())
        );

        // Pinned to the creator the relay made, the forged log passes every
        // other check.
        let forged = Shared::created_by(doc.document_id, Member::new(0x66));
        let rows = forged.rows(&forged.k0, &forged.alice.xk);
        assert_eq!(
            forged.bob_checks(&forged.log(), &rows, &forged.k0, &[]),
            Ok(())
        );
        assert_eq!(
            forged.bob_checks_pinned(&forged.log(), &rows, &forged.k0, &alice_pin),
            Err(KeyRefusal::NotFromInviter)
        );

        // A pin on a log with no anchor walks nothing, so it signs nothing.
        let unanchored = Log::walk(doc.document_id, None, &doc.entries, no_self_join);
        assert!(!unanchored.signed_under(&alice_pin));
        // The pin of Bob, who signed no entry, admits nothing either.
        let bob_pin = ambre_crypto::inviter_pin(&doc.bob.person.public());
        assert_eq!(
            doc.bob_checks_pinned(&doc.log(), &honest, &doc.k0, &bob_pin),
            Err(KeyRefusal::NotFromInviter)
        );
    }

    /// Holds `SYNC-210`. An open with no link fails closed. Bob, invited by
    /// Alice, is refused without her link, on her log and on a log the relay
    /// rooted alike. A copy he wrapped himself still opens. Alice signed the
    /// creation, so she opens unpinned a copy Bob wrapped for her. She is
    /// refused when the open knows no signing key of hers.
    #[test]
    fn an_open_with_no_link_is_refused_unless_this_client_vouches_for_the_log() {
        let doc = Shared::new();
        let log = doc.log();
        let unpinned = |own: &Member, members: &[http::MemberEntry], signer: Option<&Member>| {
            let signer = signer.map(|member| member.person.public());
            KeyVouch::of(
                &log,
                &LogRule::document(),
                &own.exchange(),
                members,
                None,
                signer.as_ref(),
                RotationDebt::Settled,
            )
            .check(DocType::Prose, doc.document_id, &doc.k0, &[])
        };
        let honest = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(
            unpinned(&doc.bob, &honest, Some(&doc.bob)),
            Err(KeyRefusal::Unpinned)
        );
        let forged = Shared::created_by(doc.document_id, Member::new(0x66));
        let rows = forged.rows(&forged.k0, &forged.alice.xk);
        let bob_key = doc.bob.person.public();
        assert_eq!(
            KeyVouch::of(
                &forged.log(),
                &LogRule::document(),
                &doc.bob.exchange(),
                &rows,
                None,
                Some(&bob_key),
                RotationDebt::Settled,
            )
            .check(DocType::Prose, doc.document_id, &forged.k0, &[]),
            Err(KeyRefusal::Unpinned)
        );

        let own = doc.rows(&doc.k0, &doc.bob.xk);
        assert_eq!(unpinned(&doc.bob, &own, None), Ok(()));

        let for_alice = vec![
            row(&doc.alice, &doc.k0, &doc.bob.xk),
            row(&doc.bob, &doc.k0, &doc.bob.xk),
        ];
        assert_eq!(unpinned(&doc.alice, &for_alice, Some(&doc.alice)), Ok(()));
        assert_eq!(
            unpinned(&doc.alice, &for_alice, None),
            Err(KeyRefusal::Unpinned)
        );
    }
}
