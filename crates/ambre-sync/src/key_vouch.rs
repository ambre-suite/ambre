//! Whether the key a members answer hands this client is one a writer chose
//! (`ambre-spec-membership.md section 3`).
//!
//! The relay names the wrapper of each copy, so it can wrap a key of its own
//! under an exchange key of its own. The unwrap then succeeds, and every
//! delta this client seals after it opens for the relay. Before a key becomes
//! current, the client checks seven things against the signed membership
//! log, and not against the relay's word:
//!
//! 1. the wrapper is this client, or a member the log verifies (`SYNC-201`);
//! 2. a writer the log verifies named the key: in a log entry, or as the key
//!    a link it signed starts from (`SYNC-200`);
//! 3. no link a writer signed replaced the key (`SYNC-202`);
//! 4. a key only a link names counts when no writer named another key at or
//!    past the head the link signs (`SYNC-221`);
//! 5. a key in force before the latest departure counts only while the
//!    members answer says the rotation is owed (`SYNC-222`);
//! 6. a key counts only when it is the key the latest revoke installed, or
//!    the chain leads back from it to that key (`SYNC-225`);
//! 7. the links a writer signed walk back from the key to the latest key the
//!    log names (`SYNC-226`). A relay that serves a genuine copy from before
//!    a revoke and withholds the link past it fails here, at a first open
//!    too.
//!
//! A link counts only when its signer held a writing role at the log head it
//! names (`SYNC-204`). A revoked writer can still name a head before its
//! revoke, which the fourth check answers. A departure names no key, so the
//! fourth check misses a leaver's link, and the fifth one answers it.
//!
//! The anchor of the log travels inside the copy, which the relay may have
//! wrapped. So the seven checks read a log the relay could root at a creator
//! of its own. An open that names a pinned inviter, from the invite link,
//! first checks that the inviter signed an entry of that log (`SYNC-205`).
//! An open with no link fails closed (`SYNC-210`). It passes only on a copy
//! this client wrapped, or on a log it signed an entry of itself.
//!
//! A document of a Murmure channel takes its rows and its links from the
//! channel's `meta`. Its open names that `meta`, read from the sealed state
//! of the channel, and the checks read the `meta`'s log and links
//! (`SYNC-229`). The log of a channel a space lists must start at a writer of
//! that space (audit `N16`). So a viewer's catalogue entry roots no log.
//!
//! A Murmure page binds no document, so no Murmure open names a pinned
//! inviter of its own. A Murmure open checks its log against the roots of
//! the session instead (`SYNC-235`). The roots hold the inviter a Murmure
//! link pins, and the members of each Murmure log an open accepted.

use ambre_core::id::{Document, Id};
use ambre_core::{ExchangePublicKey, FixedBytes, InviterPin, PublicKey, RotationDebt, Seq};
use ambre_crypto::DocumentKey;
use ambre_protocol::{KeyLink, LogPlaces};

use crate::chain::Chain;
use crate::http;
use crate::membership_log::{Log, LogRule, NamedKey, Root};
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
    /// copy. No valid entry of the log is signed by its own signing key or by
    /// a root of the session. Nothing roots the log but the relay's word.
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
    /// The log holds a revoke that installed another key, and no link leads
    /// back from this key to that one. The revoked member may hold this key.
    Unfollowed,
    /// The links a writer signed do not walk back from the key to the latest
    /// key the log names. The key is older than that one, or off its chain.
    Behind,
    /// The log heeds a space, and no writer of that space signed its
    /// creation (audit `N16`).
    RootOutsideSpace,
}

impl KeyRefusal {
    /// The failure the open reports for this refusal (`SYNC-173`, `SYNC-212`).
    ///
    /// Two refusals mean that no member signed the key: no writer named it,
    /// or the pinned inviter signed no entry of the log. The relay chose that
    /// key, and the open names the fault. An honest relay can cause each other
    /// refusal, so each stays a key not received. Examples are an invitee with
    /// no link, a copy a newer key replaced, and a wrapper an old log does not
    /// verify. A log rooted outside its space fails the open as a later walk
    /// does, so the Logic thread ends the whole channel (`SYNC-231`).
    pub(crate) fn open_failure(self) -> ambre_bus::OpenFailure {
        match self {
            Self::NotFromInviter | Self::Unvouched => ambre_bus::OpenFailure::KeyUnsigned,
            Self::RootOutsideSpace => ambre_bus::OpenFailure::RootOutsideSpace,
            Self::Unanchored
            | Self::Unpinned
            | Self::WrapperUnverified
            | Self::Superseded
            | Self::Backdated
            | Self::Departed
            | Self::Unfollowed
            | Self::Behind => ambre_bus::OpenFailure::KeyNotReceived,
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
    /// The open names no pin, and neither this client's own signing key nor
    /// a root of the session signed a valid entry of the log.
    Unpinned,
    /// The open names no pin, and this client's own signing key signed a
    /// valid entry of the log. The relay cannot sign such an entry either.
    Own,
    /// The open names no pin, and the root of the session it holds signed a
    /// valid entry of the log (`SYNC-235`).
    Rooted(FixedBytes<InviterPin>),
    /// The key the pin names signed a valid entry of the log.
    Signed(FixedBytes<InviterPin>),
    /// No valid entry of the log is signed by the key the pin names.
    Absent,
}

/// What this client holds out of band to root the log of one open on. The
/// relay can forge none of it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Roots {
    /// The inviter the invite link of the document pinned (`SYNC-205`).
    pub(crate) pin: Option<FixedBytes<InviterPin>>,
    /// The roots of the session, for a Murmure open alone (`SYNC-235`). They
    /// hold the inviter a Murmure link pins, and each member of a Murmure log
    /// an earlier open accepted.
    pub(crate) session: Vec<FixedBytes<InviterPin>>,
    /// This client's own signing key (`SYNC-210`).
    pub(crate) signer: Option<FixedBytes<PublicKey>>,
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
    /// The latest revoke the log holds, and the key it installed
    /// (`SYNC-225`).
    revoked: Option<NamedKey>,
    /// Whether the members answer says a rotation is owed.
    debt: RotationDebt,
    root: Root,
}

impl KeyVouch {
    /// Reads `log`, walked under `rule`, and the rows of `members`, for the
    /// copy carried by the row of `own`, this client's exchange key. `roots`
    /// is what this client holds out of band for the open. `debt` is what
    /// the same answer says of the rotation a departure owes.
    pub(crate) fn of(
        log: &Log,
        rule: &LogRule,
        own: &FixedBytes<ExchangePublicKey>,
        members: &[http::MemberEntry],
        roots: &Roots,
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
        let inviter = match &roots.pin {
            Some(pin) if log.signed_under(pin) => Inviter::Signed(*pin),
            Some(_) => Inviter::Absent,
            None if roots
                .signer
                .as_ref()
                .is_some_and(|key| log.signed_under(&ambre_crypto::inviter_pin(key))) =>
            {
                Inviter::Own
            }
            None => roots
                .session
                .iter()
                .find(|root| log.signed_under(root))
                .map_or(Inviter::Unpinned, |root| Inviter::Rooted(*root)),
        };
        let root = rule.root_of(log);
        Self {
            named: log.named_at().map(<[NamedKey]>::to_vec),
            wrapper,
            inviter,
            departed: log.departed_at(),
            revoked: log.revoked_at(),
            debt,
            root,
        }
    }

    /// Raises the latest departure to `known`, one this client walked
    /// before. A log served without it then leaves the leaver no key it
    /// held (`SYNC-223`).
    pub(crate) fn knowing(mut self, known: Option<Seq<LogPlaces>>) -> Self {
        self.departed = self.departed.max(known);
        self
    }

    /// The latest departure the check reads (`SYNC-223`).
    pub(crate) fn departed(&self) -> Option<Seq<LogPlaces>> {
        self.departed
    }

    /// The pin or the root of the session that signed a valid entry of the
    /// log. The local store keeps it for the sessions after, once the open
    /// passes (`SYNC-238`, `SYNC-242`). `None` when only this client vouches
    /// for the log, or when nothing does.
    pub(crate) fn proved(&self) -> Option<FixedBytes<InviterPin>> {
        match self.inviter {
            Inviter::Signed(pin) | Inviter::Rooted(pin) => Some(pin),
            Inviter::Unpinned | Inviter::Own | Inviter::Absent => None,
        }
    }

    /// Whether `key` may become the current key of a document whose log and
    /// links are those of `log_document` (`SYNC-229`).
    ///
    /// `links` are the links of `log_document` that a writer the log verifies
    /// signed (`SYNC-121`). A link that names `key` counts only when it opens
    /// under `key`, so its signature and its key commitment hold.
    ///
    /// # Errors
    ///
    /// The first [`KeyRefusal`] that applies, in the order the variants are
    /// declared.
    pub(crate) fn check(
        &self,
        log_document: Id<Document>,
        key: &DocumentKey,
        links: &[KeyLink],
    ) -> Result<(), KeyRefusal> {
        let named = self.named.as_ref().ok_or(KeyRefusal::Unanchored)?;
        if self.inviter == Inviter::Absent {
            return Err(KeyRefusal::NotFromInviter);
        }
        // An open with no link and no root fails closed (`SYNC-210`,
        // `SYNC-235`).
        if self.inviter == Inviter::Unpinned && self.wrapper != Wrapper::Holder {
            return Err(KeyRefusal::Unpinned);
        }
        if self.wrapper == Wrapper::Unverified {
            return Err(KeyRefusal::WrapperUnverified);
        }
        let id = key.id();
        let vouching = || {
            links.iter().filter(move |link| {
                link.key == id && pipeline::open_link(link, log_document, key).is_ok()
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
        // A revoke installs its key in the request that removes the member,
        // so every key past it chains back to it. A key that does not is one
        // the revoked member may hold (`SYNC-225`).
        if let Some(revoke) = self.revoked
            && Chain::new(log_document, links.iter().cloned())
                .recover(key, revoke.key)
                .is_none()
        {
            return Err(KeyRefusal::Unfollowed);
        }
        // Each step opens a link under the key it starts from, so the relay
        // cannot fake the walk. A key older than the latest one a writer
        // named reaches it by no link (`SYNC-226`).
        if let Some(latest) = named.last()
            && Chain::new(log_document, links.iter().cloned())
                .recover(key, latest.key)
                .is_none()
        {
            return Err(KeyRefusal::Behind);
        }
        if self.root == Root::Stranger {
            return Err(KeyRefusal::RootOutsideSpace);
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

    /// The roots of an open that the link pinned to `pin` drives.
    fn pinned(pin: &FixedBytes<InviterPin>) -> Roots {
        Roots {
            pin: Some(*pin),
            ..Roots::default()
        }
    }

    /// The roots of an open with no link, that `signer` signs for.
    fn signed(signer: Option<FixedBytes<PublicKey>>) -> Roots {
        Roots {
            signer,
            ..Roots::default()
        }
    }

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
                &pinned(pin),
                RotationDebt::Settled,
            )
            .check(self.document_id, key, &[])
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
                &pinned(&self.alice_pin()),
                RotationDebt::Settled,
            )
            .check(self.document_id, key, links)
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
                &pinned(&self.alice_pin()),
                RotationDebt::Settled,
            )
            .check(self.document_id, key, links)
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
            (KeyRefusal::Unfollowed, OpenFailure::KeyNotReceived),
            (KeyRefusal::Behind, OpenFailure::KeyNotReceived),
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
            &pinned(&pin),
            RotationDebt::Settled,
        );
        assert_eq!(vouch.check(doc.document_id, &doc.k0, &[]), Ok(()));
        let vouch = KeyVouch::of(
            &log,
            &rule,
            &doc.bob.exchange(),
            &with_carol,
            &pinned(&pin),
            RotationDebt::Settled,
        );
        assert_eq!(
            vouch.check(doc.document_id, &doc.k0, &[]),
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
                &crate::key_vouch::Roots {
                    pin: Some(pin),
                    signer: None,
                    ..Default::default()
                },
                RotationDebt::Settled,
            )
            .check(doc.document_id, key, &kept)
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

    /// Holds `SYNC-225`. Alice revokes Carol and installs `k1`, which her
    /// link chains to `k0`. With the link withheld, the relay serves Bob `k0`
    /// again: it passes every other check, and is refused. So is a fresh key
    /// whose link leads back to `k0` alone. `k1`, and a key whose link leads
    /// back to `k1`, stand.
    #[test]
    fn a_key_that_does_not_follow_the_latest_revoke_is_refused() {
        let mut doc = Shared::new();
        let carol = Member::new(0x23);
        let invite = doc.entry(&carol, MembershipAct::Invite, Role::Editor);
        doc.entries.push(invite);
        let before_revoke = doc.log().next();
        let k1 = DocumentKey::generate();
        let revoke = doc.entry_naming(&carol, MembershipAct::Revoke, Role::Editor, k1.id());
        doc.entries.push(revoke);
        let log = doc.log();
        let past_revoke = log.next();
        let seal = |previous: &DocumentKey, next: &DocumentKey, head, by: &Member| {
            pipeline::seal_link(doc.document_id, previous, next, head, &by.person.key).unwrap()
        };
        let checks = |key: &DocumentKey, links: Vec<KeyLink>| {
            let kept = crate::chain::signed_by_writers(links, &log.link_signers());
            doc.bob_checks(&log, &doc.rows(key, &doc.alice.xk), key, &kept)
        };
        let revoke_link = seal(&doc.k0, &k1, before_revoke, &doc.alice);

        assert_eq!(checks(&doc.k0, Vec::new()), Err(KeyRefusal::Unfollowed));
        assert_eq!(checks(&k1, vec![revoke_link.clone()]), Ok(()));
        assert_eq!(checks(&k1, Vec::new()), Ok(()), "the revoke names k1");

        let later = DocumentKey::generate();
        let paid = seal(&k1, &later, past_revoke, &doc.bob);
        assert_eq!(checks(&later, vec![revoke_link, paid]), Ok(()));
        let astray = DocumentKey::generate();
        let from_k0 = seal(&doc.k0, &astray, past_revoke, &doc.bob);
        assert_eq!(
            checks(&astray, vec![from_k0]),
            Err(KeyRefusal::Unfollowed),
            "a key past the revoke that leads to k0 alone"
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
                &crate::key_vouch::Roots {
                    pin: Some(pin),
                    signer: None,
                    ..Default::default()
                },
                debt,
            )
            .check(doc.document_id, key, &kept)
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

    /// Holds `SYNC-223`. Bob, an editor, leaves after Alice rotated to
    /// `k1`. A payment the relay answers settled serves `k1` again, with a
    /// log cut before the departure. The cut log passes `k1`. The departure
    /// this client walked before still refuses it, and the key a writer
    /// paid with stands.
    #[test]
    fn a_departure_walked_before_stands_when_the_log_omits_it() {
        let mut doc = Shared::new();
        let carol = Member::new(0x23);
        let invite = doc.entry(&carol, MembershipAct::Invite, Role::Editor);
        doc.entries.push(invite);
        let cut = doc.log();
        let before_leave = cut.next();
        let leave = doc.entry_by(
            &doc.bob,
            &doc.bob,
            MembershipAct::Revoke,
            Role::Editor,
            None,
        );
        let left = leave.position;
        doc.entries.push(leave);
        let full = doc.log();
        assert_eq!(full.departed_at(), Some(left));
        assert_eq!(cut.departed_at(), None);
        let k1 = DocumentKey::generate();
        let rotation = pipeline::seal_link(
            doc.document_id,
            &doc.k0,
            &k1,
            before_leave,
            &doc.alice.person.key,
        )
        .unwrap();
        let k2 = DocumentKey::generate();
        let paid =
            pipeline::seal_link(doc.document_id, &k1, &k2, full.next(), &carol.person.key).unwrap();
        let carol_checks = |log: &Log, key: &DocumentKey, links: &[KeyLink], known| {
            let kept = crate::chain::signed_by_writers(links.to_vec(), &log.link_signers());
            let rows = vec![
                row(&doc.alice, key, &doc.alice.xk),
                row(&carol, key, &doc.alice.xk),
            ];
            let pin = doc.alice_pin();
            let vouch = KeyVouch::of(
                log,
                &LogRule::document(),
                &carol.exchange(),
                &rows,
                &crate::key_vouch::Roots {
                    pin: Some(pin),
                    signer: None,
                    ..Default::default()
                },
                RotationDebt::Settled,
            )
            .knowing(known);
            vouch.check(doc.document_id, key, &kept)
        };
        let alone = std::slice::from_ref(&rotation);
        assert_eq!(
            carol_checks(&cut, &k1, alone, None),
            Ok(()),
            "the cut hides it"
        );
        assert_eq!(
            carol_checks(&cut, &k1, alone, Some(left)),
            Err(KeyRefusal::Departed)
        );
        assert_eq!(
            carol_checks(&full, &k2, &[rotation, paid], Some(left)),
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
                &crate::key_vouch::Roots {
                    pin: Some(pin),
                    signer: None,
                    ..Default::default()
                },
                debt,
            );
            assert_eq!(
                vouch.check(doc.document_id, &chosen, &[]),
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

    /// Holds `SYNC-235`. A Murmure open names no pin of its own. The relay
    /// roots a whole log at a creator of its own, and wraps a key of its own
    /// for Bob. Every other check passes on its word.
    ///
    /// With no root, Bob refuses both that log and Alice's. A root of the
    /// session that signed Alice's log admits hers and refuses the relay's.
    /// The wrapper and the naming checks still run under a root.
    #[test]
    fn a_murmure_log_no_root_of_the_session_signed_is_refused() {
        let doc = Shared::new();
        let forged = Shared::created_by(doc.document_id, Member::new(0x66));
        let rooted = |shared: &Shared, members: &[http::MemberEntry], key, session| {
            KeyVouch::of(
                &shared.log(),
                &LogRule::document(),
                &doc.bob.exchange(),
                members,
                &Roots {
                    session,
                    ..Roots::default()
                },
                RotationDebt::Settled,
            )
            .check(shared.document_id, key, &[])
        };
        let honest = doc.rows(&doc.k0, &doc.alice.xk);
        let relayed = forged.rows(&forged.k0, &forged.alice.xk);
        assert_eq!(
            rooted(&doc, &honest, &doc.k0, Vec::new()),
            Err(KeyRefusal::Unpinned)
        );
        assert_eq!(
            rooted(&forged, &relayed, &forged.k0, Vec::new()),
            Err(KeyRefusal::Unpinned)
        );
        let roots = vec![doc.alice_pin()];
        assert_eq!(rooted(&doc, &honest, &doc.k0, roots.clone()), Ok(()));
        assert_eq!(
            rooted(&forged, &relayed, &forged.k0, roots.clone()),
            Err(KeyRefusal::Unpinned)
        );
        // A root of Bob himself, who signed no entry, admits nothing.
        let bob_root = vec![ambre_crypto::inviter_pin(&doc.bob.person.public())];
        assert_eq!(
            rooted(&doc, &honest, &doc.k0, bob_root),
            Err(KeyRefusal::Unpinned)
        );
        let relay_xk = ambre_crypto::MasterSeed::from_array([0x66; 32]).derive_exchange_key();
        let stranger = doc.rows(&doc.k0, &relay_xk);
        assert_eq!(
            rooted(&doc, &stranger, &doc.k0, roots.clone()),
            Err(KeyRefusal::WrapperUnverified)
        );
        let chosen = DocumentKey::generate();
        let own = doc.rows(&chosen, &doc.bob.xk);
        assert_eq!(
            rooted(&doc, &own, &chosen, roots),
            Err(KeyRefusal::Unvouched)
        );
    }

    /// Holds `SYNC-242` and `SYNC-241` on the vouch. Bob opens Alice's space
    /// on the root her Murmure link gave the session, and the vouch names that
    /// root for the local store to keep. A log the relay rooted names none.
    ///
    /// After a restart the session holds no root, and Alice's log is refused.
    /// The kept root comes back as the pin of the document: it admits her log
    /// again, and refuses the log the relay rooted.
    #[test]
    fn a_kept_murmure_root_admits_its_log_after_a_restart_and_no_relay_log() {
        let doc = Shared::new();
        let forged = Shared::created_by(doc.document_id, Member::new(0x66));
        let honest = doc.rows(&doc.k0, &doc.alice.xk);
        let relayed = forged.rows(&forged.k0, &forged.alice.xk);
        let vouch = |shared: &Shared, members: &[http::MemberEntry], roots: &Roots| {
            KeyVouch::of(
                &shared.log(),
                &LogRule::document(),
                &doc.bob.exchange(),
                members,
                roots,
                RotationDebt::Settled,
            )
        };
        let session = Roots {
            session: vec![doc.alice_pin()],
            ..Roots::default()
        };
        let first = vouch(&doc, &honest, &session);
        assert_eq!(first.check(doc.document_id, &doc.k0, &[]), Ok(()));
        let kept = first.proved();
        assert_eq!(kept, Some(doc.alice_pin()));
        assert_eq!(vouch(&forged, &relayed, &session).proved(), None);

        // The restart: no root of the session, then the kept one as the pin.
        assert_eq!(
            vouch(&doc, &honest, &Roots::default()).check(doc.document_id, &doc.k0, &[]),
            Err(KeyRefusal::Unpinned)
        );
        let restarted = Roots {
            pin: kept,
            ..Roots::default()
        };
        assert_eq!(
            vouch(&doc, &honest, &restarted).check(doc.document_id, &doc.k0, &[]),
            Ok(())
        );
        assert_eq!(
            vouch(&forged, &relayed, &restarted).check(forged.document_id, &forged.k0, &[]),
            Err(KeyRefusal::NotFromInviter)
        );
    }

    /// Holds `SYNC-237`. Bob accepted the space log of Alice from her link.
    /// Its members root his next Murmure opens: a direct message Carol
    /// created opens on them, and not on Alice's pin alone. A log the relay
    /// created stays refused. Once Alice revokes Carol, the log keeps her no
    /// more, so her pin roots nothing.
    #[test]
    fn a_member_of_an_accepted_murmure_log_roots_the_next_open() {
        let mut space = Shared::new();
        let carol = Member::new(0x23);
        let invite = space.entry(&carol, MembershipAct::Invite, Role::Editor);
        space.entries.push(invite);
        let opens = |shared: &Shared, session: Vec<FixedBytes<InviterPin>>| {
            let rows = shared.rows(&shared.k0, &shared.alice.xk);
            KeyVouch::of(
                &shared.log(),
                &LogRule::document(),
                &shared.bob.exchange(),
                &rows,
                &Roots {
                    session,
                    ..Roots::default()
                },
                RotationDebt::Settled,
            )
            .check(shared.document_id, &shared.k0, &[])
        };
        assert_eq!(opens(&space, vec![space.alice_pin()]), Ok(()));
        let members = space.log().member_pins();
        let carol_pin = ambre_crypto::inviter_pin(&carol.person.public());
        assert!(members.contains(&carol_pin), "{members:?}");

        let direct = Shared::created_by(Id::<Document>::mint(), Member::new(0x23));
        assert_eq!(
            opens(&direct, vec![space.alice_pin()]),
            Err(KeyRefusal::Unpinned)
        );
        assert_eq!(opens(&direct, members.clone()), Ok(()));
        let forged = Shared::created_by(Id::<Document>::mint(), Member::new(0x66));
        assert_eq!(opens(&forged, members), Err(KeyRefusal::Unpinned));

        let revoke = space.entry(&carol, MembershipAct::Revoke, Role::Editor);
        space.entries.push(revoke);
        let kept = space.log().member_pins();
        assert!(!kept.contains(&carol_pin), "{kept:?}");
        assert_eq!(opens(&direct, kept), Err(KeyRefusal::Unpinned));
        // A log with no anchor verifies nobody, so it roots nobody.
        let unanchored = Log::walk(space.document_id, None, &space.entries, no_self_join);
        assert!(unanchored.member_pins().is_empty());
    }

    /// Audit `N16`, at the open. A viewer of a space puts a key and an anchor
    /// of its own in the catalogue. The relay serves a log the viewer
    /// created. Bob joins alone, so he wraps his own copy, and every other
    /// check passes. No writer of the space created that log, so the key is
    /// refused.
    ///
    /// The genuine log, created by a space writer, opens. A space whose word
    /// the open does not hold yet decides nothing.
    #[test]
    fn a_channel_log_no_space_writer_created_is_refused() {
        let doc = Shared::new();
        let viewer = Member::new(0x6a);
        let forged = Shared::created_by(doc.document_id, viewer);
        let mut rule = LogRule::document();
        let mut word = crate::membership_log::SpaceWord::default();
        word.writers.insert(doc.alice.person.public());
        rule.space = Some(word);
        let joins = |shared: &Shared, rule: &LogRule| {
            let own = shared.rows(&shared.k0, &shared.bob.xk);
            KeyVouch::of(
                &shared.log(),
                rule,
                &shared.bob.exchange(),
                &own,
                &Roots::default(),
                RotationDebt::Settled,
            )
            .check(shared.document_id, &shared.k0, &[])
        };
        assert_eq!(joins(&doc, &rule), Ok(()));
        assert_eq!(joins(&forged, &rule), Err(KeyRefusal::RootOutsideSpace));
        assert_eq!(
            KeyRefusal::RootOutsideSpace.open_failure(),
            ambre_bus::OpenFailure::RootOutsideSpace,
            "the first open fails as the later walk does (SYNC-231)"
        );
        rule.space = Some(crate::membership_log::SpaceWord::default());
        assert_eq!(joins(&forged, &rule), Ok(()));
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
                &signed(signer),
                RotationDebt::Settled,
            )
            .check(doc.document_id, &doc.k0, &[])
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
                &signed(Some(bob_key)),
                RotationDebt::Settled
            )
            .check(doc.document_id, &forged.k0, &[]),
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

    /// Holds `SYNC-226`. Alice invites Dave and names `k1` in his entry, with
    /// no revoke. The relay serves Bob, at his first open, his genuine copy
    /// of `k0` and withholds the link to `k1`. Every other check passes, and
    /// the key is refused. The honest answer opens, and so does a later key
    /// whose links walk back to `k1`, while one whose walk stops short is
    /// refused.
    #[test]
    fn a_key_behind_the_latest_key_the_log_names_is_refused() {
        let mut doc = Shared::new();
        let dave = Member::new(0x24);
        let k1 = DocumentKey::generate();
        let invite = doc.entry_naming(&dave, MembershipAct::Invite, Role::Editor, k1.id());
        doc.entries.push(invite);
        let log = doc.log();
        let latest = log.named_at().and_then(<[NamedKey]>::last).map(|n| n.key);
        assert_eq!(latest, Some(k1.id()));
        assert_eq!(log.revoked_at(), None, "no revoke: the seventh check alone");
        let head = log.next();
        let alice = &doc.alice.person.key;

        let stale = doc.rows(&doc.k0, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &stale, &doc.k0, &[]),
            Err(KeyRefusal::Behind)
        );

        let back = pipeline::seal_link(doc.document_id, &doc.k0, &k1, head, alice).unwrap();
        let current = doc.rows(&k1, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &current, &k1, std::slice::from_ref(&back)),
            Ok(())
        );
        assert_eq!(doc.bob_checks(&log, &current, &k1, &[]), Ok(()));

        let k2 = DocumentKey::generate();
        let to_k1 = pipeline::seal_link(doc.document_id, &k1, &k2, head, alice).unwrap();
        let later = doc.rows(&k2, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &later, &k2, &[to_k1, back.clone()]),
            Ok(())
        );

        // A writer's link names `k3` and starts from `k0`, so it never
        // reaches `k1`.
        let k3 = DocumentKey::generate();
        let to_k0 = pipeline::seal_link(doc.document_id, &doc.k0, &k3, head, alice).unwrap();
        let forked = doc.rows(&k3, &doc.alice.xk);
        assert_eq!(
            doc.bob_checks(&log, &forked, &k3, &[to_k0]),
            Err(KeyRefusal::Behind)
        );
    }
}
