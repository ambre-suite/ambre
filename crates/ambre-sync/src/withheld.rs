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
//!
//! Every presence sample carries the same head and entry (`SYNC-262`), and
//! the same proof decides it (`SYNC-263`). Presence flows while nobody
//! writes, so a relay that drops every update still lets a head through.
//!
//! A client that read the removal sees the other side of the same gap. It
//! opens a member's delta under the key the removal retired, with a head
//! before it. The relay
//! numbered that delta after one under the current key, so the member writes
//! behind the log, and the user is told (`SYNC-298`).
//!
//! The warning alone stops nothing: an edit sealed under the old key still
//! reaches the removed member through the relay. So from the ask on, each
//! edit waits unsealed, and leaves once a log that passes through the head is
//! served (`SYNC-220`). The hold starts at the ask, not at the warning: a
//! relay that never answers the listing would otherwise keep the window open.

use ambre_bus::LogicEvent;
use ambre_bus::sync_warning::SyncWarningCode;
use std::collections::HashSet;

use ambre_core::id::{Document, Id};
use ambre_core::{FixedBytes, KeyId, PublicKey, Relay, Seq};
use ambre_protocol::{Ephemeral, LogHead, MembershipEntry, Update};

use crate::generation::Listing;
use crate::membership_log::ServedHeads;
use crate::pipeline::UpdateFrame;
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
    /// The listing that left to check `head` failed. The next signed head
    /// asks again, and an answer to any later listing settles the check.
    Unanswered {
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

/// The lowest relay number this client opened a delta at, under one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeySince {
    key: FixedBytes<KeyId>,
    since: Seq<Relay>,
}

/// One delta this client opened, as the check of a member writing behind
/// the log reads it (`SYNC-298`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct OpenedFrame {
    /// The number the relay gave the delta.
    pub(crate) seq: Seq<Relay>,
    /// The key that sealed the delta.
    pub(crate) key: FixedBytes<KeyId>,
    /// The current key of the document when it opened.
    pub(crate) current: FixedBytes<KeyId>,
    /// The log head its author signed (`SYNC-206`).
    pub(crate) head: LogHead,
    /// The key that signed it.
    pub(crate) signer: FixedBytes<PublicKey>,
}

/// The log heads one document checks against (`SYNC-207`), and the edits
/// that wait while a head is under check (`SYNC-220`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeadWatch {
    /// The heads of the log the relay last served this client.
    served: ServedHeads,
    check: HeadCheck,
    /// Edits made while a head is under check, unsealed and in order.
    held: Vec<UpdateFrame>,
    /// The lowest relay number of a delta opened under the current key.
    /// A rotation leaves it naming the replaced key (`SYNC-298`).
    current_since: Option<KeySince>,
    /// The signers already found writing behind the log. Each one warns
    /// once, and only a key that signed a delta that opened joins it, so
    /// the set grows with the writers alone.
    behind: HashSet<FixedBytes<PublicKey>>,
}

impl Default for HeadWatch {
    fn default() -> Self {
        Self {
            served: ServedHeads::default(),
            check: HeadCheck::Clear,
            held: Vec::new(),
            current_since: None,
            behind: HashSet::new(),
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

    /// Whether the delta `frame` shows its signer writing behind the log
    /// (`SYNC-298`), the first time for that signer.
    ///
    /// Three facts hold together. The delta is sealed under a key other than
    /// the current one. The relay numbered it after a delta under the
    /// current key, so it is no delta in flight before the rotation. Its
    /// head is one the valid walk passes through, short of the head this
    /// client read: the removal the rotation answered is past it.
    ///
    /// A delta under the current key only records its number. History
    /// replayed under an older key carries lower numbers, so it warns
    /// nobody.
    pub(crate) fn writes_behind(&mut self, frame: OpenedFrame) -> bool {
        if frame.key == frame.current {
            self.current_since = Some(match self.current_since {
                Some(held) if held.key == frame.current => KeySince {
                    key: held.key,
                    since: held.since.min(frame.seq),
                },
                _ => KeySince {
                    key: frame.current,
                    since: frame.seq,
                },
            });
            return false;
        }
        let after_successor = self
            .current_since
            .is_some_and(|held| held.key == frame.current && held.since < frame.seq);
        let short_of_log =
            frame.head.position < self.head().position && self.served.walked_through(frame.head);
        after_successor && short_of_log && self.behind.insert(frame.signer)
    }

    /// Whether a signed `head` asks for the log again. A head the valid walk
    /// passes through asks nothing: a served entry the walk refused proves
    /// nothing (audit `W202`). While one listing is out, no other leaves, and
    /// a head already found withheld does not ask twice.
    pub(crate) fn asks(&self, head: LogHead) -> bool {
        !self.served.walked_through(head)
            && match self.check {
                HeadCheck::Clear | HeadCheck::Unanswered { .. } => true,
                HeadCheck::Asked { .. } => false,
                HeadCheck::Withheld(withheld) => withheld != head,
            }
    }

    /// Whether an edit waits instead of leaving (`SYNC-220`): a signed head
    /// is asked about, unanswered or withheld.
    pub(crate) fn holds_edits(&self) -> bool {
        self.check != HeadCheck::Clear
    }

    /// Keeps an edit made while a head is under check, unsealed.
    pub(crate) fn hold(&mut self, body: UpdateFrame) {
        self.held.push(body);
    }

    /// The held edits, in order, once the check is clear. Nothing while a
    /// head is still under check.
    pub(crate) fn release(&mut self) -> Vec<UpdateFrame> {
        if self.holds_edits() {
            Vec::new()
        } else {
            std::mem::take(&mut self.held)
        }
    }

    /// Whether no edit waits here, so the session may turn light.
    pub(crate) fn settled(&self) -> bool {
        self.held.is_empty()
    }

    /// The listing `listing` failed. A check waiting on it asks again at the
    /// next signed head, and its edits keep waiting.
    pub(crate) fn failed(&mut self, listing: Seq<Listing>) {
        if let HeadCheck::Asked { head, listing: ask } = self.check
            && ask == listing
        {
            self.check = HeadCheck::Unanswered { head, listing };
        }
    }

    /// Records that `listing` left to check `head`.
    pub(crate) fn asked(&mut self, head: LogHead, listing: Seq<Listing>) {
        self.check = HeadCheck::Asked { head, listing };
    }

    /// A fresh members answer to `listing`, which served `served`. A head
    /// asked about is withheld when an answer that left after the ask still
    /// does not pass through it. A withheld head clears once a log holds it.
    /// Only the heads of the valid walk clear a head: the relay can serve a
    /// refused entry before the true one (audit `W202`).
    pub(crate) fn served(&mut self, listing: Seq<Listing>, served: ServedHeads) -> Verdict {
        self.served = served;
        match self.check {
            HeadCheck::Asked { head, listing: ask }
            | HeadCheck::Unanswered { head, listing: ask }
                if listing >= ask =>
            {
                if self.served.walked_through(head) {
                    self.check = HeadCheck::Clear;
                    Verdict::Nothing
                } else {
                    self.check = HeadCheck::Withheld(head);
                    Verdict::Withheld
                }
            }
            HeadCheck::Withheld(head) if self.served.walked_through(head) => {
                self.check = HeadCheck::Clear;
                Verdict::Nothing
            }
            _ => Verdict::Nothing,
        }
    }
}

/// A frame that carries a log head and its entry under its author's
/// signature: an update (`SYNC-206`) or a presence sample (`SYNC-262`).
#[derive(Clone, Copy)]
pub(crate) enum HeadFrame<'a> {
    Update(&'a Update),
    Ephemeral(&'a Ephemeral),
}

impl HeadFrame<'_> {
    fn document_id(self) -> Id<Document> {
        match self {
            Self::Update(update) => update.public_data.document_id,
            Self::Ephemeral(ephemeral) => ephemeral.public_data.document_id,
        }
    }

    fn head(self) -> LogHead {
        match self {
            Self::Update(update) => update.public_data.log_head,
            Self::Ephemeral(ephemeral) => ephemeral.public_data.log_head,
        }
    }

    fn entry(&self) -> Option<&MembershipEntry> {
        match self {
            Self::Update(update) => update.public_data.log_entry.as_deref(),
            Self::Ephemeral(ephemeral) => ephemeral.public_data.log_entry.as_deref(),
        }
    }

    fn signer(self) -> FixedBytes<PublicKey> {
        match self {
            Self::Update(update) => update.public_data.pub_key,
            Self::Ephemeral(ephemeral) => ephemeral.public_data.pub_key,
        }
    }

    /// Whether the signer the frame names signed it, checked alone.
    fn signed(self) -> bool {
        match self {
            Self::Update(update) => pipeline::update_signed(update),
            Self::Ephemeral(ephemeral) => pipeline::ephemeral_signed(ephemeral),
        }
    }
}

impl SyncState {
    /// Reads the log head `frame` carries (`SYNC-207`, `SYNC-263`). A head of
    /// a log this client has not read asks for the log again. The head counts
    /// only when a member of the directory signed the frame. The signature is
    /// checked here, so a frame under a key this client does not hold still
    /// counts. That frame is the one a hidden rotation brings.
    ///
    /// The head also counts only when the entry it names checks against this
    /// client's log (`SYNC-219`), on an update and on a sample alike. A member
    /// cannot invent a head and have the relay blamed.
    pub(crate) fn note_signed_head(&mut self, frame: HeadFrame<'_>) {
        let document_id = frame.document_id();
        let head = frame.head();
        // The call of a channel heeds the head first, open document or not (`CALL-137`).
        self.heed_call_head(document_id, head, frame.entry());
        let Some(doc_state) = self.documents.get(&document_id) else {
            return;
        };
        // The cheap checks run first: a current head costs one comparison,
        // and only a head past the log pays for the signature.
        let asks = doc_state.head_watch.asks(head)
            && doc_state
                .head_watch
                .served
                .vouches(document_id, head, frame.entry())
            && doc_state.signing_keys.contains_key(&frame.signer())
            && frame.signed();
        if !asks {
            return;
        }
        crate::log::warning!(
            "[Sync] a member signed a log head this client has not read doc={document_id} place={}",
            head.position.get()
        );
        self.handle_list_document_members(None, document_id);
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            let listing = doc_state.directory_asked.latest();
            doc_state.head_watch.asked(head, listing);
        }
    }

    /// Reads a peer's delta that opened at `seq` (`SYNC-298`). A member
    /// that writes under a retired key, behind the log this client read,
    /// raises a warning for the user. Either the relay withholds the removal
    /// from that member, or the member has not read it yet. Under the key it
    /// retired, the member who left may read that delta. This client's own
    /// delta under a retired key is a resend, and warns nobody.
    pub(crate) fn note_writer_behind(&mut self, seq: Seq<Relay>, update: &Update) {
        let document_id = update.public_data.document_id;
        let signer = update.public_data.pub_key;
        let peer = self.author_pub_key() != Some(signer);
        let Some(doc_state) = self.documents.get_mut(&document_id) else {
            return;
        };
        let frame = OpenedFrame {
            seq,
            key: update.public_data.key_id,
            current: doc_state.doc_key.id(),
            head: update.public_data.log_head,
            signer,
        };
        if (frame.key == frame.current || peer) && doc_state.head_watch.writes_behind(frame) {
            crate::log::warning!(
                "[Sync] a member writes under a retired key, behind the log doc={document_id} seq={}",
                seq.get()
            );
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::WriterBehindLog,
            });
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
        let verdict = doc_state.head_watch.served(listing, served);
        let released = doc_state.head_watch.release();
        if verdict == Verdict::Withheld {
            crate::log::warning!(
                "[Sync] the relay withholds a log entry a member signed doc={document_id}"
            );
            self.send_to_logic(LogicEvent::SyncWarning {
                document_id,
                code: SyncWarningCode::LogEntryWithheld,
            });
        }
        if !released.is_empty() {
            crate::log::info!(
                "[Sync] the log passes through the signed head, {} held edit(s) leave doc={document_id}",
                released.len()
            );
        }
        // Each edit takes the ordinary path again: an owed rotation or a new
        // key the answer brought still applies (`SYNC-128`).
        for body in released {
            self.send_body(document_id, body);
        }
    }

    /// The listing `listing` of `document_id` failed. A head under check asks
    /// again at the next signed head, and its edits keep waiting (`SYNC-220`).
    pub(crate) fn listing_failed(&mut self, document_id: Id<Document>, listing: Seq<Listing>) {
        if let Some(doc_state) = self.documents.get_mut(&document_id) {
            doc_state.head_watch.failed(listing);
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
        walk(entries.get(..n).unwrap_or(entries)).heads()
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
        let all = walk(&entries).heads();
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
        let signed = walk(&entries).heads().last();

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
        // A later log that still hides the entry keeps the check.
        assert_eq!(watch.served(Seq::nth(4), hidden.clone()), Verdict::Nothing);
        assert!(watch.holds_edits(), "the head stays withheld");
        // A later log that holds the head clears the check.
        assert_eq!(
            watch.served(Seq::nth(5), walk(&entries).heads()),
            Verdict::Nothing
        );
        assert!(!watch.asks(signed));
        assert!(!watch.holds_edits());
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
        let signed = walk(&entries).heads().last();
        watch.asked(signed, Seq::nth(1));
        assert_eq!(
            watch.served(Seq::nth(1), walk(&entries).heads()),
            Verdict::Nothing
        );
        assert!(!watch.asks(signed));
    }

    /// The relay serves an entry the walk refuses, then the entry the signed
    /// head names. The served heads hold that head and the valid walk does
    /// not, so the check does not clear (audit `W202`).
    #[test]
    fn a_served_entry_the_walk_refuses_clears_no_head_under_check() {
        let (alice, bob, carol) = (Person::new(1), Person::new(2), Person::new(3));
        let mut entries = created(&alice, &bob);
        // Carol is no member, so the walk refuses her invite and stops.
        append(
            &mut entries,
            &carol,
            &carol,
            MembershipAct::Invite,
            Role::Editor,
        );
        append(
            &mut entries,
            &alice,
            &bob,
            MembershipAct::Revoke,
            Role::Viewer,
        );
        let served = walk(&entries).heads();
        let signed = served.last();
        assert!(served.holds(signed), "the served heads hold the head");
        assert!(!served.walked_through(signed), "the valid walk does not");

        let mut watch = HeadWatch::default();
        watch.read(served.clone());
        assert!(watch.asks(signed), "a refused entry proves no head");
        watch.asked(signed, Seq::nth(1));
        assert_eq!(watch.served(Seq::nth(1), served), Verdict::Withheld);
        assert!(watch.holds_edits(), "the edits keep waiting");
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
            walk(entries).heads().last(),
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
            departed: None,
            revoked: None,
            root: Default::default(),
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

    /// Holds `SYNC-298`: Alice revoked Carol and rotated the key, and the
    /// relay hides it from Bob. Bob's delta under the retired key, with the
    /// head before the revoke, opens on Alice's client. Numbered after a
    /// delta under the current key, it warns once for Bob. Three deltas
    /// under the retired key warn nobody:
    /// * history numbered before it;
    /// * a delta in flight before the rotation;
    /// * a delta whose head holds the revoke.
    #[test]
    fn a_member_writing_behind_the_log_is_seen_by_a_peer() {
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
        let retired = crate::DocumentKey::generate();
        let current = crate::DocumentKey::generate();
        // Alice's client: it read the whole log, and the revoke rotated the
        // key, so the retired one sits in the cache.
        let (mut state, rx, doc, snapshot) = bob_reads(&alice, &retired, &entries, entries.len());
        state
            .documents
            .get_mut(&doc)
            .expect("open")
            .rotate_key(current.clone());
        state.signing_key = Some(alice.key.clone());
        let behind = walk(&entries[..3]).heads().last();
        let bob_writes = |key: &crate::DocumentKey, head: LogHead, entry: usize| {
            writes(
                &bob,
                (doc, snapshot),
                key,
                head,
                entries.get(entry).cloned(),
            )
        };

        // History: Bob wrote under the retired key before the revoke.
        state.receive_update(Seq::nth(1), bob_writes(&retired, behind, 2));
        assert!(codes(&rx).is_empty(), "history warns nobody");
        // Alice's own delta under the current key, which the relay numbers 5.
        state.receive_update(
            Seq::nth(5),
            alice_writes(&alice, doc, snapshot, &current, &entries),
        );
        // A delta in flight before the rotation, numbered before it.
        state.receive_update(Seq::nth(3), bob_writes(&retired, behind, 2));
        assert!(codes(&rx).is_empty(), "a delta in flight warns nobody");

        // Bob writes behind the log, after the current key was in use.
        state.receive_update(Seq::nth(6), bob_writes(&retired, behind, 2));
        assert_eq!(codes(&rx), vec![SyncWarningCode::WriterBehindLog]);
        state.receive_update(Seq::nth(7), bob_writes(&retired, behind, 2));
        assert!(codes(&rx).is_empty(), "one warning per member");

        // A delta under the retired key whose head holds the revoke is not
        // behind the log, so it warns nobody here.
        let (mut state, rx, doc, snapshot) = bob_reads(&alice, &retired, &entries, entries.len());
        state
            .documents
            .get_mut(&doc)
            .expect("open")
            .rotate_key(current.clone());
        state.receive_update(
            Seq::nth(1),
            alice_writes(&alice, doc, snapshot, &current, &entries),
        );
        state.receive_update(
            Seq::nth(2),
            writes(
                &bob,
                (doc, snapshot),
                &retired,
                walk(&entries).heads().last(),
                entries.last().cloned(),
            ),
        );
        assert!(codes(&rx).is_empty(), "a head that holds the removal");
    }

    /// `SYNC-298`: the current key is in use from the lowest number of a
    /// delta opened under it. A later delta under that key keeps the number,
    /// so a delta under the retired key numbered between them still warns. A
    /// rotation counts the next key from its own first delta.
    #[test]
    fn the_current_key_is_in_use_from_its_lowest_delta() {
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
        let mut watch = HeadWatch::default();
        watch.read(walk(&entries).heads());
        let behind = walk(&entries[..3]).heads().last();
        let (retired, current, next) = (
            crate::DocumentKey::generate().id(),
            crate::DocumentKey::generate().id(),
            crate::DocumentKey::generate().id(),
        );
        let frame = |seq: u64, key, current, signer: &Person| OpenedFrame {
            seq: Seq::nth(seq),
            key,
            current,
            head: behind,
            signer: signer.public(),
        };
        assert!(!watch.writes_behind(frame(5, current, current, &alice)));
        // A number the current key already holds is no delta after it.
        assert!(!watch.writes_behind(frame(5, retired, current, &bob)));
        assert!(!watch.writes_behind(frame(10, current, current, &alice)));
        assert!(watch.writes_behind(frame(7, retired, current, &bob)));

        assert!(!watch.writes_behind(frame(12, next, next, &alice)));
        assert!(watch.writes_behind(frame(15, current, next, &carol)));
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
        answer(&mut state, doc, &alice, walk(&entries).heads());
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
            vec![(walk(&entries).heads().last(), entries.last().cloned())]
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

    #[test]
    fn a_head_under_check_holds_edits_until_a_log_passes_through_it() {
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
        let signed = walk(&entries).heads().last();
        let edit = |byte: u8| UpdateFrame::from_bytes(vec![byte]);

        let mut watch = HeadWatch::default();
        watch.read(hidden.clone());
        assert!(!watch.holds_edits(), "a clear check holds nothing");
        watch.asked(signed, Seq::nth(1));
        assert!(watch.holds_edits(), "the hold starts at the ask");
        watch.hold(edit(1));
        assert!(watch.release().is_empty());
        assert!(!watch.settled());
        // A failed listing keeps the hold, and the next signed head asks.
        watch.failed(Seq::nth(1));
        assert!(watch.holds_edits());
        assert!(watch.asks(signed));
        watch.asked(signed, Seq::nth(2));
        assert_eq!(watch.served(Seq::nth(2), hidden.clone()), Verdict::Withheld);
        watch.hold(edit(2));
        assert!(watch.release().is_empty(), "a withheld head holds");
        // A log that passes through the head lets every edit go, in order.
        assert_eq!(
            watch.served(Seq::nth(3), walk(&entries).heads()),
            Verdict::Nothing
        );
        assert!(!watch.holds_edits());
        assert_eq!(watch.release(), vec![edit(1), edit(2)]);
        assert!(watch.settled());
    }

    #[test]
    fn a_failed_listing_of_another_check_changes_nothing() {
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
        let signed = walk(&entries).heads().last();
        let mut watch = HeadWatch::default();
        watch.read(heads(&entries, entries.len() - 1));
        watch.asked(signed, Seq::nth(2));
        watch.failed(Seq::nth(1));
        assert!(!watch.asks(signed), "the ask in flight still stands");
    }

    /// The Update frames Bob sent, by clock.
    fn sent_clocks(
        ws_rx: &ambre_bus::conduit::Receiver<ambre_core::Payload<ambre_protocol::WireFrame>>,
    ) -> Vec<Seq<ambre_protocol::messages::DeltaAuthor>> {
        ws_rx
            .try_iter()
            .filter_map(|wire| ambre_protocol::ws::decode_client_msg(&wire).ok())
            .filter_map(|msg| match msg {
                ambre_protocol::ws::WsClientMsg::Update(update) => Some(update.public_data.clock),
                _ => None,
            })
            .collect()
    }

    /// The relay hides from Bob that Alice revoked Carol. From the ask on,
    /// Bob's edits wait unsealed. None leaves while the log is withheld.
    /// Each leaves in order once a log holds Alice's head (`SYNC-220`).
    #[test]
    fn a_warned_member_seals_nothing_until_the_log_passes_through_the_head() {
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
        state.handle_send_update(doc, b"while asked".to_vec());
        assert!(
            sent_clocks(&ws_rx).is_empty(),
            "an edit waits during the ask"
        );

        answer(&mut state, doc, &alice, heads(&entries, 3));
        assert_eq!(codes(&rx), vec![SyncWarningCode::LogEntryWithheld]);
        state.handle_send_update(doc, b"once warned".to_vec());
        assert!(
            sent_clocks(&ws_rx).is_empty(),
            "a warned member seals nothing"
        );
        let doc_state = state.documents.get(&doc).expect("open");
        assert!(!doc_state.head_watch.settled());
        assert_eq!(doc_state.rekey.sent_len(), 0);
        // An eviction keeps the full session while an edit waits here.
        state.handle_release_document(doc, Seq::ZERO);
        assert!(
            state.documents.contains_key(&doc),
            "the session stays heavy"
        );

        // A later listing, such as the member panel's, serves the revoke.
        state.handle_list_document_members(None, doc);
        answer(&mut state, doc, &alice, walk(&entries).heads());
        assert!(codes(&rx).is_empty());
        assert_eq!(
            sent_clocks(&ws_rx),
            vec![Seq::nth(1), Seq::nth(2)],
            "both edits leave, in order"
        );
        state.handle_send_update(doc, b"after".to_vec());
        assert_eq!(sent_clocks(&ws_rx), vec![Seq::nth(3)]);
    }

    /// The listing a signed head asked for fails. Bob's edits keep waiting,
    /// and Alice's next frame asks again (`SYNC-220`).
    #[test]
    fn a_failed_listing_keeps_the_hold_and_the_next_head_asks_again() {
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
        let (mut state, _rx, doc, snapshot) = bob_reads(&alice, &key, &entries, 1);
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
        state.handle_sync_event(crate::SyncEvent::DocumentMembersFailed {
            document_id: doc,
            asked: Seq::nth(1),
        });
        state.handle_send_update(doc, b"held".to_vec());
        assert!(sent_clocks(&ws_rx).is_empty());

        state.receive_update(
            ambre_core::Seq::nth(2),
            alice_writes(&alice, doc, snapshot, &key, &entries),
        );
        let asked = state
            .documents
            .get(&doc)
            .expect("open")
            .directory_asked
            .latest();
        assert_eq!(asked, Seq::nth(2), "the next signed head asks again");
        answer(&mut state, doc, &alice, walk(&entries).heads());
        assert_eq!(sent_clocks(&ws_rx), vec![Seq::nth(1)]);
    }

    /// An empty log names no entry, and a head names the last entry served,
    /// valid or not (`SYNC-218`).
    #[test]
    fn a_head_names_the_last_entry_served_and_an_empty_log_none() {
        let mut watch = HeadWatch::default();
        assert_eq!(watch.head(), LogHead::EMPTY);
        assert_eq!(watch.head_entry(), None);
        let entries = created(&Person::new(1), &Person::new(2));
        watch.read(walk(&entries).heads());
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
        let next = walk(&entries).heads().last();
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

    /// A presence sample `author` seals under `key`, announcing `log_head`
    /// and the entry `log_entry`, at `counter`.
    fn points(
        author: &Person,
        (doc, key): (Id<Document>, &crate::DocumentKey),
        log_head: LogHead,
        log_entry: Option<MembershipEntry>,
        counter: u64,
    ) -> Ephemeral {
        let public_data = ambre_protocol::EphemeralPublicData {
            document_id: doc,
            pub_key: author.public(),
            key_id: key.id(),
            counter: ambre_protocol::messages::Seq::new(counter),
            log_head,
            log_entry: log_entry.map(Box::new),
        };
        let payload = ambre_core::Payload::new(b"a cursor".to_vec());
        pipeline::seal_ephemeral(&payload, public_data, key, &author.key).expect("seal")
    }

    /// The listing Bob last sent for `doc`.
    fn asked_of(state: &SyncState, doc: Id<Document>) -> Seq<Listing> {
        state
            .documents
            .get(&doc)
            .expect("open")
            .directory_asked
            .latest()
    }

    /// `SYNC-274`, audit `W205`: while a head is under check, or a rotation
    /// is owed, Bob's presence samples are dropped, as his edits wait. Once
    /// the check clears, they leave again.
    #[test]
    fn presence_is_dropped_while_an_edit_would_wait() {
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
        state.signing_key = Some(bob.key.clone());
        // A sample names the open it comes from (`SYNC-287`).
        state
            .session_nonces
            .insert(doc, ambre_core::FixedBytes::from_array([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        state.set_mock_ws(ws_tx);
        let samples = |state: &mut SyncState| {
            state.handle_send_ephemeral(doc, ambre_core::Payload::new(b"a cursor".to_vec()));
            ws_rx
                .try_iter()
                .filter(|wire| {
                    matches!(
                        ambre_protocol::ws::decode_client_msg(wire),
                        Ok(ambre_protocol::ws::WsClientMsg::Ephemeral(_))
                    )
                })
                .count()
        };
        assert_eq!(samples(&mut state), 1, "control: a clear check sends");

        // Alice signs a head past Bob's log: the check starts.
        state.receive_update(
            ambre_core::Seq::nth(1),
            alice_writes(&alice, doc, snapshot, &key, &entries),
        );
        assert_eq!(
            samples(&mut state),
            0,
            "a head under check drops the sample"
        );
        answer(&mut state, doc, &alice, walk(&entries).heads());
        assert_eq!(samples(&mut state), 1, "a cleared check sends again");

        if let Some(doc_state) = state.documents.get_mut(&doc) {
            doc_state.rekey.found_debt();
        }
        assert_eq!(samples(&mut state), 0, "an owed rotation drops the sample");
    }

    /// The relay hides from Bob that Alice revoked Carol and rotated the
    /// key. A scripted relay then drops each update Alice sends and forwards
    /// her presence alone. Her cursor carries her head and its entry, so Bob
    /// reads the log again and is warned once (`SYNC-262`, `SYNC-263`).
    #[test]
    fn a_relay_that_drops_updates_but_forwards_presence_is_caught() {
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
        let (mut bob_state, rx, doc, _snapshot) = bob_reads(&alice, &old_key, &entries, 3);
        // Alice read the whole log and seals under the key of the rotation.
        let (mut alice_state, _alice_rx, _, _) =
            bob_reads(&alice, &new_key, &entries, entries.len());
        alice_state.signing_key = Some(alice.key.clone());
        alice_state
            .session_nonces
            .insert(doc, ambre_core::FixedBytes::from_array([7; 16]));
        let (ws_tx, ws_rx) = ambre_bus::conduit::unbounded();
        alice_state.set_mock_ws(ws_tx);

        // The relay of the script: each update dies, each sample passes.
        let relay = |alice_state: &mut SyncState, bob_state: &mut SyncState| {
            alice_state.handle_send_update(doc, b"after the revoke".to_vec());
            let sample = ambre_core::Payload::new(b"a cursor".to_vec());
            alice_state.handle_send_ephemeral(doc, sample);
            let mut dropped = 0;
            for wire in ws_rx.try_iter() {
                match ambre_protocol::ws::decode_client_msg(&wire) {
                    Ok(ambre_protocol::ws::WsClientMsg::Update(_)) => dropped += 1,
                    Ok(ambre_protocol::ws::WsClientMsg::Ephemeral(ephemeral)) => {
                        assert_eq!(
                            (
                                ephemeral.public_data.log_head,
                                ephemeral.public_data.log_entry.as_deref()
                            ),
                            (walk(&entries).heads().last(), entries.last()),
                            "the sample carries the head and the entry Alice read"
                        );
                        bob_state.handle_ws_message(ambre_protocol::ws::WsServerMsg::Ephemeral(
                            ephemeral,
                        ));
                    }
                    _ => {}
                }
            }
            assert_eq!(dropped, 1, "the relay drops Alice's update");
        };

        relay(&mut alice_state, &mut bob_state);
        let listing = asked_of(&bob_state, doc);
        assert_eq!(listing, Seq::nth(1), "the sample asks for the log again");
        answer(&mut bob_state, doc, &alice, heads(&entries, 3));
        assert!(codes(&rx).contains(&SyncWarningCode::LogEntryWithheld));

        // The next sample names the same head: no second read or warning.
        relay(&mut alice_state, &mut bob_state);
        assert_eq!(asked_of(&bob_state, doc), listing);
        assert!(!codes(&rx).contains(&SyncWarningCode::LogEntryWithheld));
    }

    /// A sample with no entry, with an entry no allowed member signed, or
    /// rewritten in transit asks for no log. The same proof as an update's
    /// decides it: the honest sample asks once (`SYNC-219`, `SYNC-263`).
    #[test]
    fn a_forged_presence_entry_asks_nothing() {
        use crate::membership_log::{Draft, digest, sign};
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
        let (mut state, _rx, doc, _snapshot) = bob_reads(&alice, &key, &entries, 3);
        if let Some(doc_state) = state.documents.get_mut(&doc) {
            doc_state
                .set_signing_keys(vec![(alice.public(), alice.id), (carol.public(), carol.id)]);
        }
        let next = walk(&entries).heads().last();
        let past = |entry: &MembershipEntry| next.after(digest(entry).expect("encodes"));
        let signed = |member: &Person, author: &Person| {
            let draft = Draft {
                document: DOC,
                member: member.id,
                member_key: member.public(),
                member_exchange_key: member.exchange(),
                act: MembershipAct::Revoke,
                role: Role::Editor,
                key: None,
            };
            sign(draft, next, author.id, &author.key).expect("signs")
        };
        let honest = signed(&carol, &alice);
        let demoted = signed(&alice, &carol);
        let mut rewritten = points(&carol, (doc, &key), past(&honest), Some(honest.clone()), 3);
        rewritten.public_data.counter = ambre_protocol::messages::Seq::new(9);
        let refused = [
            (
                "no entry",
                points(&carol, (doc, &key), past(&honest), None, 1),
            ),
            (
                "an editor revokes the owner",
                points(&carol, (doc, &key), past(&demoted), Some(demoted), 2),
            ),
            ("a sample rewritten in transit", rewritten),
        ];
        for (case, sample) in refused {
            state.handle_ws_message(ambre_protocol::ws::WsServerMsg::Ephemeral(sample));
            assert_eq!(asked_of(&state, doc), Seq::ZERO, "{case} asks nothing");
        }
        let proved = points(&carol, (doc, &key), past(&honest), Some(honest), 4);
        state.handle_ws_message(ambre_protocol::ws::WsServerMsg::Ephemeral(proved));
        assert_eq!(
            asked_of(&state, doc),
            Seq::nth(1),
            "the owner's entry asks once"
        );
    }
}
