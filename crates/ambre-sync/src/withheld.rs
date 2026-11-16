//! A log entry the relay withholds from this client
//! (`ambre-spec-membership.md section 3`, `SYNC-206`, `SYNC-207`).
//!
//! The relay can hide a revoke and its rotation from one member, and then
//! forward that member's deltas to the member it removed. Every update carries
//! the head of the log its author last read, under the author's signature,
//! and the entry that head names (`SYNC-218`). A head that the log this client
//! read does not pass through names an entry this client has not seen. The
//! client reads the log again only when a member its log allows signed that
//! entry (`SYNC-219`). A log that still does not pass through the head is a
//! log the relay withholds, and the user is told.

use ambre_bus::LogicEvent;
use ambre_bus::sync_warning::SyncWarningCode;
use ambre_core::Seq;
use ambre_core::id::{Document, Id};
use ambre_protocol::{LogHead, MembershipEntry, Update};

use crate::generation::Listing;
use crate::membership_log::ServedHeads;
use crate::{SyncState, pipeline};

/// Where the check of a signed head stands for one document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeadCheck {
    /// Every signed head this client read is one its log passes through.
    Clear,
    /// A peer signed `head`, and this client asked for the log again with
    /// `listing`. An answer to an older listing proves nothing: it may have
    /// left before the entry landed.
    Asked {
        head: LogHead,
        listing: Seq<Listing>,
    },
    /// The log served after the ask did not pass through `head`.
    Withheld(LogHead),
}

/// What a fresh members answer says of the head under check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Nothing for the user: the head is in the log, or nothing was asked.
    Nothing,
    /// The relay served a log that does not pass through a signed head.
    /// The relay withholds an entry, or the member who signed that entry
    /// never sent it.
    Withheld,
}

/// The log heads one document checks against (`SYNC-207`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeadWatch {
    /// The heads of the log the relay last served this client.
    served: ServedHeads,
    check: HeadCheck,
}

impl Default for HeadWatch {
    fn default() -> Self {
        Self {
            served: ServedHeads::default(),
            check: HeadCheck::Clear,
        }
    }
}

impl HeadWatch {
    /// The log an open read. No check is under way yet.
    pub(crate) fn read(&mut self, served: ServedHeads) {
        self.served = served;
    }

    /// The head of the log the relay last served: the head this client
    /// signs into each update (`SYNC-206`).
    pub(crate) fn head(&self) -> LogHead {
        self.served.last()
    }

    /// The entry [`HeadWatch::head`] names, which each update carries as
    /// its proof (`SYNC-218`). Absent on an empty log.
    pub(crate) fn head_entry(&self) -> Option<Box<MembershipEntry>> {
        self.served.last_entry().cloned().map(Box::new)
    }

    /// Whether a signed `head` asks for the log again. A head the log passes
    /// through asks nothing. While one listing is out, no other leaves, and a
    /// head already found withheld does not ask twice.
    pub(crate) fn asks(&self, head: LogHead) -> bool {
        !self.served.holds(head)
            && match self.check {
                HeadCheck::Clear => true,
                HeadCheck::Asked { .. } => false,
                HeadCheck::Withheld(withheld) => withheld != head,
            }
    }

    /// Records that `listing` left to check `head`.
    pub(crate) fn asked(&mut self, head: LogHead, listing: Seq<Listing>) {
        self.check = HeadCheck::Asked { head, listing };
    }

    /// A fresh members answer to `listing`, which served `served`. A head
    /// asked about is withheld when an answer that left after the ask still
    /// does not pass through it. A withheld head clears once a log holds it.
    pub(crate) fn served(&mut self, listing: Seq<Listing>, served: ServedHeads) -> Verdict {
        self.served = served;
        match self.check {
            HeadCheck::Asked { head, listing: ask } if listing >= ask => {
                if self.served.holds(head) {
                    self.check = HeadCheck::Clear;
                    Verdict::Nothing
                } else {
                    self.check = HeadCheck::Withheld(head);
                    Verdict::Withheld
                }
            }
            HeadCheck::Withheld(head) if self.served.holds(head) => {
                self.check = HeadCheck::Clear;
                Verdict::Nothing
            }
            _ => Verdict::Nothing,
        }
    }
}

impl SyncState {
    /// Reads the log head `update` carries (`SYNC-207`). A head of a log this
    /// client has not read asks for the log again. The head counts only when
    /// a member of the directory signed the frame. The signature is checked
    /// here, so a frame under a key this client does not hold still counts.
    /// That frame is the one a hidden rotation brings.
    ///
    /// The head also counts
    /// only when the entry it names checks against this client's log
    /// (`SYNC-219`). A member cannot invent a head and have the relay blamed.
    pub(crate) fn note_signed_head(&mut self, update: &Update) {
        let document_id = update.public_data.document_id;
        let head = update.public_data.log_head;
        let Some(doc_state) = self.documents.get(&document_id) else {
            return;
        };
        let asks = doc_state.head_watch.asks(head)
            && doc_state.head_watch.served.vouches(
                document_id,
                head,
                update.public_data.log_entry.as_deref(),
            )
            && doc_state
                .signing_keys
                .contains_key(&update.public_data.pub_key)
            && pipeline::update_signed(update);
        if !asks {
            return;
        }
        tracing::warn!(
            "[Sync] a member signed a log head this client has not read doc={document_id} place={}",
            head.position.get()
        );
        self.handle_list_document_members(None, document_id);
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            let listing = doc_state.directory_asked.latest();
            doc_state.head_watch.asked(head, listing);
        }
    }

    /// Checks a signed head against the log a fresh members answer served.
    /// A log the relay withholds raises a warning for the user (`SYNC-207`).
    pub(crate) fn check_served_heads(
        &mut self,
        document_id: Id<Document>,
        listing: Seq<Listing>,
        served: ServedHeads,
    ) {
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        if doc_state.head_watch.served(listing, served) == Verdict::Withheld {
            tracing::warn!(
                "[Sync] the relay withholds a log entry a member signed doc={document_id}"
            );
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::LogEntryWithheld,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership_log::tests::{DOC, Person, append, created, walk};
    use ambre_core::Role;
    use ambre_protocol::MembershipAct;

    /// The heads of the first `n` entries of a log.
    fn heads(entries: &[ambre_protocol::MembershipEntry], n: usize) -> ServedHeads {
        ServedHeads::of(entries.get(..n).unwrap_or(entries))
    }

    #[test]
    fn a_served_log_holds_each_head_it_passes_through_and_no_other() {
        let alice = Person::new(1);
        let bob = Person::new(2);
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let all = ServedHeads::of(&entries);
        let short = heads(&entries, entries.len() - 1);
        // Every head of the shorter log is held by the longer one.
        assert!(all.holds(short.last()));
        assert!(all.holds(LogHead::EMPTY));
        assert!(all.holds(all.last()));
        // The head past the shorter log is not held by it.
        assert!(!short.holds(all.last()));
        // A head at a held place, with another digest, forks: not held.
        let fork = LogHead {
            position: short.last().position,
            previous: all.last().previous,
        };
        assert!(!all.holds(fork));
        assert!(!short.holds(fork));
    }

    #[test]
    fn a_head_past_the_log_asks_once_and_a_log_without_it_is_withheld() {
        let alice = Person::new(1);
        let bob = Person::new(2);
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let hidden = heads(&entries, entries.len() - 1);
        let signed = ServedHeads::of(&entries).last();

        let mut watch = HeadWatch::default();
        watch.read(hidden.clone());
        assert!(!watch.asks(hidden.last()), "a held head asks nothing");
        assert!(watch.asks(signed));
        watch.asked(signed, Seq::nth(3));
        assert!(!watch.asks(signed), "one listing out at a time");
        // An answer to a listing that left before the ask proves nothing.
        assert_eq!(watch.served(Seq::nth(2), hidden.clone()), Verdict::Nothing);
        assert!(!watch.asks(signed));
        // The answer to the ask still hides the entry.
        assert_eq!(watch.served(Seq::nth(3), hidden.clone()), Verdict::Withheld);
        assert!(!watch.asks(signed), "a withheld head does not ask twice");
        // A later log that holds the head clears the check.
        assert_eq!(
            watch.served(Seq::nth(4), ServedHeads::of(&entries)),
            Verdict::Nothing
        );
        assert!(!watch.asks(signed));
    }

    #[test]
    fn a_log_that_serves_the_signed_head_is_not_withheld() {
        let alice = Person::new(1);
        let bob = Person::new(2);
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let mut watch = HeadWatch::default();
        watch.read(heads(&entries, entries.len() - 1));
        let signed = ServedHeads::of(&entries).last();
        watch.asked(signed, Seq::nth(1));
        assert_eq!(
            watch.served(Seq::nth(1), ServedHeads::of(&entries)),
            Verdict::Nothing
        );
        assert!(!watch.asks(signed));
    }

    /// Bob's Sync worker, a reader of a shared document. It holds `key`,
    /// and Alice's signing key is in its directory. Its open read the first
    /// `read` entries of `entries`.
    fn bob_reads(
        alice: &Person,
        key: &crate::DocumentKey,
        entries: &[ambre_protocol::MembershipEntry],
        read: usize,
    ) -> (
        SyncState,
        ambre_bus::conduit::Receiver<LogicEvent>,
        Id<Document>,
        Id<ambre_core::id::Snapshot>,
    ) {
        let (sink, rx) = crate::LogicSink::channel();
        let mut state = SyncState::new(
            sink,
            crate::http::Url::<crate::RelayHost>::new(String::new()),
        );
        let doc = DOC;
        let mut doc_state = crate::SyncDocumentState::new(
            doc,
            key.clone(),
            ambre_bus::DocType::Trame,
            crate::Existence::OnRelay,
        );
        doc_state.set_signing_keys(vec![(alice.public(), alice.id)]);
        let snapshot = doc_state.current_snapshot_id;
        state.documents.insert(doc, doc_state);
        state.handle_sync_event(crate::SyncEvent::LogWalked {
            document_id: doc,
            word: crate::membership_log::SpaceWord::default(),
            heads: walk(entries.get(..read).unwrap_or(entries)).heads(),
        });
        (state, rx, doc, snapshot)
    }

    /// An update Alice seals under `key`, signing the head of `entries` and
    /// the last entry, which proves it.
    fn alice_writes(
        alice: &Person,
        doc: Id<Document>,
        snapshot: Id<ambre_core::id::Snapshot>,
        key: &crate::DocumentKey,
        entries: &[ambre_protocol::MembershipEntry],
    ) -> Update {
        writes(
            alice,
            (doc, snapshot),
            key,
            ServedHeads::of(entries).last(),
            entries.last().cloned(),
        )
    }

    /// An update `author` seals under `key` for a document and its
    /// snapshot, announcing `log_head` and the entry `log_entry`.
    fn writes(
        author: &Person,
        (doc, snapshot): (Id<Document>, Id<ambre_core::id::Snapshot>),
        key: &crate::DocumentKey,
        log_head: LogHead,
        log_entry: Option<MembershipEntry>,
    ) -> Update {
        let public_data = ambre_protocol::UpdatePublicData {
            document_id: doc,
            pub_key: author.public(),
            key_id: key.id(),
            snapshot_id: snapshot,
            clock: Seq::nth(1),
            log_head,
            log_entry: log_entry.map(Box::new),
        };
        let nonce = ambre_core::FixedBytes::from_array([6; 16]);
        let body = pipeline::update_body(&nonce, None, &b"after the revoke".to_vec().into())
            .expect("no summary");
        pipeline::seal_update(&body, public_data, key, &author.key).expect("seal")
    }

    /// The members answer to the latest listing, serving `served`.
    fn answer(state: &mut SyncState, doc: Id<Document>, alice: &Person, served: ServedHeads) {
        let asked = state
            .documents
            .get(&doc)
            .expect("open")
            .directory_asked
            .latest();
        state.handle_sync_event(crate::SyncEvent::DocumentMembersReady {
            id: None,
            asked,
            document_id: doc,
            word: crate::membership_log::SpaceWord::default(),
            rotation_owed: ambre_core::RotationDebt::Settled,
            members: Vec::new(),
            signing_keys: vec![(alice.public(), alice.id)],
            heads: served,
            writers: Default::default(),
        });
    }

    fn codes(rx: &ambre_bus::conduit::Receiver<LogicEvent>) -> Vec<SyncWarningCode> {
        rx.try_iter()
            .filter_map(|event| match event {
                LogicEvent::SyncWarning { code, .. } => Some(code),
                _ => None,
            })
            .collect()
    }

    /// The relay hides from Bob that Alice revoked Carol and rotated the
    /// key. Alice's next delta is under the new key, which Bob does not hold.
    /// Its signed head still counts, so Bob reads the log again. The relay
    /// serves it without the revoke, and Bob is warned (`SYNC-207`).
    #[test]
    fn a_peer_detects_a_revoke_the_relay_hid_from_it() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Revoke,
            Role::Editor,
        );
        let old_key = crate::DocumentKey::generate();
        let new_key = crate::DocumentKey::generate();
        let (mut state, rx, doc, snapshot) = bob_reads(&alice, &old_key, &entries, 3);

        state.receive_update(
            ambre_core::Seq::nth(1),
            alice_writes(&alice, doc, snapshot, &new_key, &entries),
        );
        assert_eq!(codes(&rx), vec![SyncWarningCode::KeyNotHeld]);
        let listing = state
            .documents
            .get(&doc)
            .expect("open")
            .directory_asked
            .latest();
        assert_eq!(
            listing,
            Seq::nth(1),
            "the signed head asks for the log again"
        );

        answer(&mut state, doc, &alice, heads(&entries, 3));
        assert_eq!(codes(&rx), vec![SyncWarningCode::LogEntryWithheld]);

        // The same head again asks nothing more, and warns no more.
        state.receive_update(
            ambre_core::Seq::nth(2),
            alice_writes(&alice, doc, snapshot, &new_key, &entries),
        );
        assert_eq!(codes(&rx), vec![SyncWarningCode::KeyNotHeld]);
        let doc_state = state.documents.get(&doc).expect("open");
        assert_eq!(doc_state.directory_asked.latest(), listing);
    }

    /// An honest relay serves the revoke on the second read: no warning,
    /// and Bob's next delta signs the head he now holds (`SYNC-206`,
    /// `SYNC-207`).
    #[test]
    fn a_log_served_whole_clears_the_check_and_heads_the_next_delta() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Revoke,
            Role::Editor,
        );
        let key = crate::DocumentKey::generate();
        let (mut state, rx, doc, snapshot) = bob_reads(&alice, &key, &entries, 3);
        state.signing_key = Some(bob.key.clone());
        state
            .session_nonces
            .insert(doc, ambre_core::FixedBytes::from_array([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);

        state.receive_update(
            ambre_core::Seq::nth(1),
            alice_writes(&alice, doc, snapshot, &key, &entries),
        );
        answer(&mut state, doc, &alice, ServedHeads::of(&entries));
        assert!(codes(&rx).is_empty());

        state.handle_send_update(doc, b"bob".to_vec());
        let heads: Vec<(LogHead, Option<MembershipEntry>)> = ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .filter_map(|msg| match msg {
                ambre_protocol::ws::WsClientMsg::Update(update) => Some((
                    update.public_data.log_head,
                    update.public_data.log_entry.map(|e| *e),
                )),
                _ => None,
            })
            .collect();
        // The head, and the entry it names as its proof (`SYNC-218`).
        assert_eq!(
            heads,
            vec![(ServedHeads::of(&entries).last(), entries.last().cloned())]
        );
    }

    /// A head signed by a key outside the directory, or under a broken
    /// signature, asks for nothing (`SYNC-207`).
    #[test]
    fn a_head_no_member_signed_asks_nothing() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        let key = crate::DocumentKey::generate();
        let (mut state, _rx, doc, snapshot) = bob_reads(&alice, &key, &entries, 2);

        let stranger = alice_writes(&carol, doc, snapshot, &key, &entries);
        state.receive_update(ambre_core::Seq::nth(1), stranger);
        let mut forged = alice_writes(&alice, doc, snapshot, &key, &entries);
        forged.public_data.log_head =
            LogHead::EMPTY.after(ambre_core::FixedBytes::from_array([9; 32]));
        state.receive_update(ambre_core::Seq::nth(2), forged);
        let doc_state = state.documents.get(&doc).expect("open");
        assert_eq!(doc_state.directory_asked.latest(), Seq::ZERO);
    }

    /// An empty log names no entry, and a head names the last entry served,
    /// valid or not (`SYNC-218`).
    #[test]
    fn a_head_names_the_last_entry_served_and_an_empty_log_none() {
        let mut watch = HeadWatch::default();
        assert_eq!(watch.head(), LogHead::EMPTY);
        assert_eq!(watch.head_entry(), None);
        let entries = created(&Person::new(1), &Person::new(2));
        watch.read(ServedHeads::of(&entries));
        assert_eq!(watch.head_entry().as_deref(), entries.last());
        assert_eq!(
            watch.head().previous,
            entries.last().and_then(crate::membership_log::digest)
        );
    }

    /// Carol, an editor, announces heads past Bob's log. No head whose
    /// entry fails to check asks for the log, so the relay is never blamed
    /// for an entry a member invented. An entry Alice, the owner, signed
    /// at that place asks at once (`SYNC-219`).
    #[test]
    fn a_head_whose_entry_no_allowed_member_signed_asks_nothing() {
        use crate::membership_log::{Draft, digest, sign};
        let (alice, bob, carol, dave) = (
            Person::new(1),
            Person::new(2),
            Person::new(3),
            Person::new(4),
        );
        let mut entries = created(&alice, &bob);
        append(
            &mut entries,
            &alice,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        let key = crate::DocumentKey::generate();
        let (mut state, _rx, doc, snapshot) = bob_reads(&alice, &key, &entries, 3);
        if let Some(doc_state) = state.documents.get_mut(&doc) {
            doc_state
                .set_signing_keys(vec![(alice.public(), alice.id), (carol.public(), carol.id)]);
        }
        let next = ServedHeads::of(&entries).last();
        let past = |entry: &MembershipEntry| next.after(digest(entry).expect("encodes"));
        let asked = |state: &SyncState| {
            state
                .documents
                .get(&doc)
                .expect("open")
                .directory_asked
                .latest()
        };
        let draft = |member: &Person, act| Draft {
            document: DOC,
            member: member.id,
            member_key: member.public(),
            member_exchange_key: member.exchange(),
            act,
            role: Role::Editor,
            key: None,
        };
        let signed = |member: &Person, act, author: &Person, by: &Person| {
            sign(draft(member, act), next, author.id, &by.key).expect("signs")
        };
        let honest = signed(&carol, MembershipAct::Revoke, &alice, &alice);
        let demoted = signed(&alice, MembershipAct::Revoke, &carol, &carol);
        let borrowed = signed(&bob, MembershipAct::Revoke, &alice, &carol);
        let joined = signed(&dave, MembershipAct::SelfJoin, &dave, &dave);
        let elsewhere = MembershipEntry {
            document: Id::<Document>::from_bytes([8; 16]),
            ..honest.clone()
        };
        let refused = [
            ("no entry", past(&honest), None),
            ("an editor revokes the owner", past(&demoted), Some(demoted)),
            (
                "the owner named, under another key",
                past(&borrowed),
                Some(borrowed),
            ),
            (
                "an entry of another head",
                past(&honest).after(digest(&honest).expect("encodes")),
                Some(honest.clone()),
            ),
            (
                "an entry of another document",
                past(&honest),
                Some(elsewhere),
            ),
            ("a self-join", past(&joined), Some(joined)),
        ];
        for (clock, (case, head, entry)) in (1..).zip(refused) {
            state.receive_update(
                Seq::nth(clock),
                writes(&carol, (doc, snapshot), &key, head, entry),
            );
            assert_eq!(asked(&state), Seq::ZERO, "{case} asks nothing");
        }
        state.receive_update(
            Seq::nth(10),
            writes(
                &carol,
                (doc, snapshot),
                &key,
                past(&honest),
                Some(honest.clone()),
            ),
        );
        assert_eq!(asked(&state), Seq::nth(1), "the owner's entry asks once");
    }
}
