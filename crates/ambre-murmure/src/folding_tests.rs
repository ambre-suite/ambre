//! The fold of the amendments of a final segment against its contract rows.
//! They hold when the fold lease is asked, the fold snapshot the holder
//! builds, the check another member runs, and the fold every member settles.

use ambre_bus::CharSpan;
use std::collections::HashMap;

use ambre_bus::{
    Authored, FoldAnswer, Instant, MessageDraft, MessageView, MurmureEvent, SnapshotVerdict,
    StyleMark, TextStyle,
};
use ambre_core::id::{Channel, Document, Id, Lease, Message, Segment, User};
use ambre_core::{DeltaBytes, Payload, PeerEntryBytes, Seq, SnapshotBytes};
use loro::{Container, ExportMode, LoroDoc, ValueOrContainer};

use crate::error::MurmureError;
use crate::schema::{MessageKey, Root};
use crate::session::next_peer;
use crate::{ChangeTarget, ChannelState, ChannelType, FINALIZE_RETRY, FOLD_DELAY, Replay, Signer};
use ambre_core::{Ms, TimeSpan};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);
const LEASE_MS: u64 = 30_000;

fn at(ms: u64) -> Instant<Authored> {
    Instant::<Authored>::new(NOW.get() + ms)
}

/// The style the amendments put on the edited message.
fn bold() -> StyleMark {
    StyleMark {
        style: TextStyle::Bold,
        range: CharSpan::from_usize_range(1..3),
    }
}

fn draft(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// A second member's copy of `state`, opened from its live segments.
fn peer_of(state: &ChannelState) -> ChannelState {
    let segments: HashMap<Id<Segment>, Payload<SnapshotBytes>> = state
        .held_segments()
        .into_iter()
        .map(|segment| {
            let bytes = state.snapshot(segment.document_id()).expect("segment");
            (segment, bytes)
        })
        .collect();
    let meta = state.snapshot(state.meta_id().document_id()).expect("meta");
    ChannelState::open(state.channel_id(), state.meta_id(), &meta, &segments)
        .expect("peer")
        .identify()
}

/// Alice's channel with a final segment holding "one", "two" and "three".
/// Its amendments edit the first and delete the second, and the relay
/// numbered their three deltas, header first.
struct Amended {
    alice: ChannelState,
    author: Id<User>,
    segment: Id<Segment>,
    amendments: Id<Document>,
    ids: Vec<Id<Message>>,
    /// The `meta` delta that names the amendments.
    meta: Payload<DeltaBytes>,
    /// The three amendment deltas, in the relay's order.
    deltas: Vec<Payload<DeltaBytes>>,
    /// The final snapshot of the segment, as the relay holds it.
    sealed: Payload<SnapshotBytes>,
}

fn amended() -> Amended {
    let author = Id::<User>::mint();
    let mut alice = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .with_segment_size(crate::MessageCount::new(3))
        .identify();
    for text in ["one", "two", "three"] {
        alice.send(&draft(text), author, NOW).expect("send");
    }
    let segment = alice.wants_close().expect("full");
    let successor = Id::<Segment>::mint();
    alice
        .close_into(segment, successor, crate::Successor::Created, NOW)
        .expect("close");
    let peer = next_peer();
    alice
        .adopt_session(
            successor.document_id(),
            peer,
            PeerEntryBytes::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt");
    assert_eq!(alice.lease_to_ask(NOW), Some(segment));
    alice.lease_granted(
        segment,
        Id::<Lease>::mint(),
        ambre_core::Seq::new(0),
        TimeSpan::<Ms>::new(LEASE_MS),
    );
    let sealed = alice
        .final_snapshot(NOW, ambre_core::Seq::new(0))
        .expect("export")
        .expect("in");
    alice.mark_final(segment).expect("mark");
    assert_eq!(alice.final_segments(), vec![segment]);

    let ChangeTarget::Create(amendments) = alice.change_target(segment, NOW).expect("target")
    else {
        panic!("a final segment with no amendments creates them");
    };
    let peer = next_peer();
    let first = alice
        .adopt_session(
            amendments,
            peer,
            PeerEntryBytes::new(peer.get().to_le_bytes().to_vec()),
        )
        .expect("adopt")
        .expect("the header travels");
    let meta = alice
        .amendments_linked(segment, amendments)
        .expect("link")
        .meta
        .expect("meta names them");
    let ids: Vec<Id<Message>> = alice
        .messages(segment)
        .expect("held")
        .iter()
        .map(|m| m.id)
        .collect();
    let styled = MessageDraft {
        styles: vec![bold()],
        ..draft("uno")
    };
    let edited = alice
        .edit(segment, ids[0], &styled, author)
        .expect("edit")
        .expect("a change");
    let deleted = alice.delete(segment, ids[1], author).expect("delete");
    let deltas = vec![first, edited.delta, deleted.delta];
    let mut amended = Amended {
        alice,
        author,
        segment,
        amendments,
        ids,
        meta,
        deltas,
        sealed: sealed.payload,
    };
    // The relay echoes each delta to its author.
    let alice_signer = amended.signer();
    for (seq, delta) in (1..).zip(amended.deltas.clone()) {
        amended
            .alice
            .apply_signed(amendments, Seq::new(seq), &delta, alice_signer)
            .expect("echo");
    }
    amended
}

impl Amended {
    fn signer(&self) -> Signer {
        Signer {
            peer: self.alice.peer(self.amendments).expect("a session"),
            user: Some(self.author),
        }
    }

    /// A member's copy holding the final segment, whose `meta` names the
    /// amendments.
    fn loading(&self) -> ChannelState {
        let mut copy = peer_of(&self.alice);
        copy.apply_remote(copy.meta_id().document_id(), (&self.meta).into())
            .expect("meta");
        copy.load_older(self.segment, &self.sealed)
            .expect("the final snapshot loads");
        copy
    }

    /// Bob's copy, holding the first `received` amendment deltas.
    fn bob(&self, received: usize) -> ChannelState {
        let mut bob = self.loading();
        assert_eq!(bob.amendments_to_open(), [self.amendments]);
        let signer = self.signer();
        for (seq, delta) in (1..).zip(&self.deltas).take(received) {
            bob.apply_signed(self.amendments, Seq::new(seq), delta, signer)
                .expect("waits");
        }
        bob.older_ready(self.amendments).expect("admitted");
        assert_eq!(bob.amendments_id(self.segment), Some(self.amendments));
        bob
    }

    /// Alice takes the fold lease and builds the fold snapshot.
    fn fold(&mut self) -> crate::FoldSnapshot {
        assert_eq!(self.alice.fold_lease_to_ask(NOW), Some(self.segment));
        self.alice.fold_answered(
            self.segment,
            FoldAnswer::Granted {
                lease: Id::<Lease>::mint(),
                horizon: ambre_core::Seq::new(0),
                amendments: self.amendments,
                amendments_seq: ambre_core::Seq::new(3),
                expires_in: TimeSpan::<Ms>::new(LEASE_MS),
            },
        );
        self.alice
            .fold_snapshot(NOW)
            .expect("fold")
            .expect("every amendment is in")
    }
}

fn messages_of(event: &MurmureEvent) -> &[MessageView] {
    let MurmureEvent::Messages { messages, .. } = event else {
        panic!("messages");
    };
    messages
}

fn holds(bytes: &[u8], word: &str) -> bool {
    bytes.windows(word.len()).any(|w| w == word.as_bytes())
}

// --- MURMURE-075 ---

/// The fold lease is asked at the first tick past the link of the
/// amendments, then as late as the relay says. A failure waits a delay that
/// doubles, and an accepted fold waits out the fold delay.
#[test]
fn the_fold_lease_is_asked_as_the_relay_says() {
    let Amended {
        mut alice, segment, ..
    } = amended();
    assert_eq!(
        alice.fold_lease_to_ask(NOW),
        Some(segment),
        "the first tick"
    );
    assert_eq!(alice.fold_lease_to_ask(NOW), None, "asked once");

    alice.fold_answered(
        segment,
        FoldAnswer::Deferred {
            retry_in: TimeSpan::<Ms>::new(5_000),
        },
    );
    assert_eq!(alice.fold_lease_to_ask(NOW), None, "the tick dates it");
    assert_eq!(alice.fold_lease_to_ask(at(4_999)), None);
    assert_eq!(alice.fold_lease_to_ask(at(5_000)), Some(segment));

    alice.fold_failed(segment);
    let base = 5_000;
    assert_eq!(alice.fold_lease_to_ask(at(base)), None, "the tick dates it");
    assert_eq!(
        alice.fold_lease_to_ask(at(base + FINALIZE_RETRY.get())),
        Some(segment)
    );
    alice.fold_failed(segment);
    let base = base + FINALIZE_RETRY.get();
    assert_eq!(alice.fold_lease_to_ask(at(base)), None);
    assert_eq!(
        alice.fold_lease_to_ask(at(base + FINALIZE_RETRY.get())),
        None,
        "the delay doubled"
    );
    assert_eq!(
        alice.fold_lease_to_ask(at(base + 2 * FINALIZE_RETRY.get())),
        Some(segment)
    );

    alice.fold_committed(segment);
    let base = base + 2 * FINALIZE_RETRY.get();
    assert_eq!(alice.fold_lease_to_ask(at(base)), None, "the tick dates it");
    assert_eq!(
        alice.fold_lease_to_ask(at(base + FOLD_DELAY.get() - 1)),
        None
    );
    assert_eq!(
        alice.fold_lease_to_ask(at(base + FOLD_DELAY.get())),
        Some(segment)
    );
}

/// A channel with no amendments asks for no fold lease.
#[test]
fn a_segment_without_amendments_asks_for_no_fold_lease() {
    let mut state = ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    assert_eq!(state.fold_lease_to_ask(NOW), None);
    assert!(state.fold_snapshot(NOW).expect("fold").is_none());
}

/// The lease holder folds the amendments into a shallow snapshot of the
/// segment, once it holds them up to the number the relay returned. A new
/// copy loading it reads the folded messages, and fetches no amendments.
#[test]
fn the_lease_holder_folds_the_amendments_into_the_segment() {
    let mut amended = amended();
    let (segment, amendments) = (amended.segment, amended.amendments);
    let before = amended.alice.messages(segment).expect("held");
    assert_eq!(amended.alice.fold_lease_to_ask(NOW), Some(segment));
    let lease = Id::<Lease>::mint();
    amended.alice.fold_answered(
        segment,
        FoldAnswer::Granted {
            lease,
            horizon: ambre_core::Seq::new(0),
            amendments,
            amendments_seq: ambre_core::Seq::new(4),
            expires_in: TimeSpan::<Ms>::new(LEASE_MS),
        },
    );
    assert!(
        amended.alice.fold_snapshot(NOW).expect("fold").is_none(),
        "an amendment the relay numbered is missing"
    );
    amended.alice.fold_answered(
        segment,
        FoldAnswer::Granted {
            lease,
            horizon: ambre_core::Seq::new(0),
            amendments,
            amendments_seq: ambre_core::Seq::new(3),
            expires_in: TimeSpan::<Ms>::new(LEASE_MS),
        },
    );
    let fold = amended
        .alice
        .fold_snapshot(at(1))
        .expect("fold")
        .expect("in");
    assert_eq!((fold.segment, fold.lease), (segment, lease));
    assert_eq!(
        (fold.horizon, fold.amendments),
        (ambre_core::Seq::new(0), amendments)
    );
    assert_eq!(fold.amendments_seq, ambre_core::Seq::new(3));
    assert!(holds(fold.payload.as_bytes(), "uno"));
    assert!(
        !holds(fold.payload.as_bytes(), "two"),
        "shallow: the deleted body is gone"
    );
    assert!(
        amended.alice.fold_snapshot(at(2)).expect("fold").is_none(),
        "built once"
    );
    assert_eq!(
        amended.alice.messages(segment).expect("held"),
        before,
        "this copy stays as it is"
    );

    // Carol opens the channel once `meta` marks the amendments folded.
    let mut bob = amended.bob(3);
    let marked = bob
        .mark_folded(segment, amendments)
        .expect("mark")
        .expect("a delta");
    let mut carol = peer_of(&bob);
    carol
        .apply_remote(carol.meta_id().document_id(), (&marked).into())
        .expect("meta");
    let loaded = carol
        .load_older(segment, &fold.payload)
        .expect("the fold loads");
    assert_eq!(carol.amendments_to_open(), [], "the fold holds them");
    let read = messages_of(&loaded).to_vec();
    assert_eq!(
        read.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["uno", "", "three"]
    );
    assert_eq!(read[0].edited, ambre_bus::Revision::Edited);
    assert_eq!(read[1].deleted, ambre_bus::Deletion::Deleted);
    // `MURMURE-562`: the fold keeps the styles of the edit.
    assert_eq!(read[0].styles, [bold()]);
    assert_eq!(read, before, "the fold reads as the amendments did");
}

/// A lease naming other amendments than this copy holds fails, and is asked
/// again past the retry delay. One that runs out is asked again.
#[test]
fn a_fold_lease_at_odds_with_this_copy_is_asked_again() {
    let Amended {
        mut alice,
        segment,
        amendments,
        ..
    } = amended();
    assert_eq!(alice.fold_lease_to_ask(NOW), Some(segment));
    alice.fold_answered(
        segment,
        FoldAnswer::Granted {
            lease: Id::<Lease>::mint(),
            horizon: ambre_core::Seq::new(0),
            amendments: Id::<Document>::mint(),
            amendments_seq: ambre_core::Seq::new(1),
            expires_in: TimeSpan::<Ms>::new(LEASE_MS),
        },
    );
    assert!(alice.fold_snapshot(NOW).expect("fold").is_none());
    assert_eq!(alice.fold_lease_to_ask(NOW), None, "the tick dates it");
    assert_eq!(
        alice.fold_lease_to_ask(at(FINALIZE_RETRY.get())),
        Some(segment)
    );

    alice.fold_answered(
        segment,
        FoldAnswer::Granted {
            lease: Id::<Lease>::mint(),
            horizon: ambre_core::Seq::new(0),
            amendments,
            amendments_seq: ambre_core::Seq::new(9),
            expires_in: TimeSpan::<Ms>::new(LEASE_MS),
        },
    );
    let base = FINALIZE_RETRY.get();
    assert!(alice.fold_snapshot(at(base)).expect("fold").is_none());
    assert_eq!(alice.fold_lease_to_ask(at(base + LEASE_MS - 1)), None);
    assert_eq!(
        alice.fold_lease_to_ask(at(base + LEASE_MS)),
        Some(segment),
        "the lease ran out"
    );
}

/// Bob holds the amendments up to the number the fold names: he folds his
/// own copy and finds the same. A forged fold is a mismatch, and a copy
/// holding another number of amendments leaves it unchecked.
#[test]
fn another_member_checks_the_fold_by_folding_its_own_copy() {
    let mut amended = amended();
    let segment = amended.segment;
    let fold = amended.fold();
    let bob = amended.bob(3);
    let document_id = segment.document_id();
    let before = bob.messages(segment).expect("held");
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(0), &fold.payload)
            .expect("check"),
        SnapshotVerdict::Match { extras: Vec::new() }
    );
    assert_eq!(bob.messages(segment).expect("held"), before, "only read");

    let forged = LoroDoc::new();
    crate::sequence::import(&forged, (&fold.payload).into()).expect("import");
    let Some(message) = crate::schema::root_list(&forged, Root::Messages)
        .get(2)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message");
    };
    let Some(ValueOrContainer::Container(Container::Text(body))) = message.get(MessageKey::Content)
    else {
        panic!("a body");
    };
    body.insert(0, "planted ").expect("forge");
    forged.commit();
    let forged = Payload::<SnapshotBytes>::new(
        forged
            .export(ExportMode::shallow_snapshot(&forged.oplog_frontiers()))
            .expect("export"),
    );
    assert_eq!(
        bob.check_snapshot(document_id, ambre_core::Seq::new(0), &forged)
            .expect("check"),
        SnapshotVerdict::Mismatch
    );

    let short = amended.bob(2);
    assert_eq!(
        short
            .check_snapshot(document_id, ambre_core::Seq::new(0), &fold.payload)
            .expect("check"),
        SnapshotVerdict::Unchecked,
        "he misses an amendment the fold holds"
    );
}

/// A change written after the fold began reaches Bob before his check. His
/// copy holds more than the fold, so the check leaves it unchecked, and the
/// relay keeps the amendments.
#[test]
fn a_change_past_the_fold_leaves_it_unchecked() {
    let mut amended = amended();
    let (segment, amendments) = (amended.segment, amended.amendments);
    let fold = amended.fold();
    let mut bob = amended.bob(3);
    let later = amended
        .alice
        .edit(segment, amended.ids[2], &draft("tres"), amended.author)
        .expect("edit")
        .expect("a change");
    bob.apply_signed(amendments, Seq::new(4), &later.delta, amended.signer())
        .expect("applies");
    assert_eq!(
        bob.check_snapshot(
            segment.document_id(),
            ambre_core::Seq::new(0),
            &fold.payload
        )
        .expect("check"),
        SnapshotVerdict::Unchecked
    );
    assert_eq!(bob.amendments_id(segment), Some(amendments));
}

// --- MURMURE-076, MURMURE-077 ---

/// `meta` marks the amendments folded, and every member folds its own copy
/// and drops them. A change of Alice's the relay never sequenced comes back
/// as a replay, and the next change creates new amendments.
#[test]
fn a_marked_fold_settles_every_copy_and_replays_what_never_left() {
    let mut amended = amended();
    let (segment, amendments) = (amended.segment, amended.amendments);
    amended.fold();
    let mut bob = amended.bob(3);
    // Alice edits the third message; the relay never sequences it.
    amended
        .alice
        .edit(segment, amended.ids[2], &draft("tres"), amended.author)
        .expect("edit")
        .expect("a change");
    assert_eq!(
        bob.settle_folds().expect("settle").len(),
        0,
        "not marked yet"
    );

    let meta = bob
        .mark_folded(segment, amendments)
        .expect("mark")
        .expect("a delta");
    assert_eq!(bob.mark_folded(segment, amendments).expect("mark"), None);
    let settled = bob.settle_folds().expect("settle");
    assert_eq!(settled.len(), 1);
    assert_eq!(
        (settled[0].segment, settled[0].amendments),
        (segment, amendments)
    );
    assert!(settled[0].replay.is_empty(), "Bob changed nothing");
    assert_eq!(bob.amendments_id(segment), None, "dropped");
    let read = messages_of(&settled[0].event);
    assert_eq!(
        read.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        ["uno", "", "three"]
    );

    amended
        .alice
        .apply_remote(amended.alice.meta_id().document_id(), (&meta).into())
        .expect("meta");
    let settled = amended.alice.settle_folds().expect("settle");
    assert_eq!(settled.len(), 1);
    assert_eq!(
        settled[0].replay,
        [Replay {
            segment,
            message: amended.ids[2],
            draft: Some(draft("tres")),
        }]
    );
    assert_eq!(messages_of(&settled[0].event), read, "the fold, as Bob's");
    assert_eq!(amended.alice.amendments_id(segment), None);
    assert_eq!(
        amended.alice.fold_lease_to_ask(NOW),
        None,
        "nothing to fold"
    );
    assert!(
        amended.alice.settle_folds().expect("settle").is_empty(),
        "once"
    );
    assert!(matches!(
        amended.alice.change_target(segment, NOW),
        Ok(ChangeTarget::Create(other)) if other != amendments
    ));
}

// --- MURMURE-483 ---

/// Amendments the relay reports gone are marked folded, once. The change
/// it refused is replayed, and only linked amendments are marked so.
#[test]
fn amendments_reported_gone_are_marked_folded_and_replay_the_refused_change() {
    let mut amended = amended();
    let (segment, amendments) = (amended.segment, amended.amendments);
    let mut bob = amended.bob(3);
    assert!(matches!(
        amended.alice.amendments_gone(Id::<Document>::mint()),
        Err(MurmureError::NotHeld(_))
    ));
    // The relay refuses this edit: it erased the amendments meanwhile.
    amended
        .alice
        .edit(segment, amended.ids[2], &draft("tres"), amended.author)
        .expect("edit")
        .expect("a change");

    let meta = amended
        .alice
        .amendments_gone(amendments)
        .expect("gone")
        .expect("a delta");
    assert_eq!(
        amended.alice.amendments_gone(amendments).expect("gone"),
        None
    );
    let settled = amended.alice.settle_folds().expect("settle");
    assert_eq!(
        settled[0].replay,
        [Replay {
            segment,
            message: amended.ids[2],
            draft: Some(draft("tres")),
        }]
    );
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert_eq!(bob.settle_folds().expect("settle").len(), 1, "every copy");
}

/// Amendments `meta` marks folded while they still arrive are dropped
/// unopened.
#[test]
fn amendments_still_arriving_are_dropped_once_folded() {
    let amended = amended();
    let (segment, amendments) = (amended.segment, amended.amendments);
    let mut bob = amended.loading();
    assert_eq!(bob.amendments_to_open(), [amendments]);
    let mut carol = amended.bob(3);
    let meta = carol
        .mark_folded(segment, amendments)
        .expect("mark")
        .expect("a delta");
    bob.apply_remote(bob.meta_id().document_id(), (&meta).into())
        .expect("meta");
    assert!(bob.settle_folds().expect("settle").is_empty());
    assert!(bob.older_ready(amendments).is_err(), "no longer arriving");
}
