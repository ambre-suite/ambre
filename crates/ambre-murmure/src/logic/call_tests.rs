//! The call message of a channel against its contract rows
//! (`features/ambre-spec-murmure-calls.md`).

use crate::Receipt;
use crate::message::{CallEnd, MessageView};
use ambre_bus::{Authored, CallMembers, Instant};
use ambre_core::id::{Call, Channel, Id, Segment, User};
use ambre_core::{DeltaBytes, Payload, Relay, Seq};
use loro::{Container, LoroMap, ValueOrContainer, VersionVector};

use crate::logic::schema::{MessageKey, Root};
use crate::{
    ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Refusal, Sent, Signer,
};
use ambre_core::crdt::CrdtName as _;

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);

/// One member of the channel: its state, and the user it writes for.
struct Member {
    state: ChannelState,
    user: Id<User>,
}

impl Member {
    fn segment(&self) -> Id<Segment> {
        self.state.active_id()
    }

    fn signer(&self) -> Signer {
        Signer {
            nonce: crate::logic::delta::TEST_NONCE,
            peer: self
                .state
                .peer(self.segment().document_id())
                .expect("a session"),
            user: Some(self.user),
        }
    }

    /// Take what `from` wrote, as the relay delivers it under number `seq`.
    fn receive(&mut self, from: &Member, sent: &Sent, seq: u64) {
        self.state
            .apply_signed(
                sent.segment.document_id(),
                Seq::<Relay>::new(seq),
                &sent.delta,
                from.signer(),
            )
            .expect("a delta the check passes");
    }

    fn messages(&self) -> Vec<MessageView> {
        self.state.messages(self.segment()).expect("held")
    }

    fn post(&mut self, call: Id<Call>) -> Sent {
        self.state.post_call(call, self.user, NOW).expect("a post")
    }

    fn end(&mut self, call: Id<Call>) -> Option<Sent> {
        self.state.end_call(call, self.user, NOW).expect("an end")
    }

    fn doc(&self) -> &loro::LoroDoc {
        self.state.doc(self.segment().document_id()).expect("held")
    }
}

/// Alice's empty channel, and Bob, who opened it.
fn pair() -> (Member, Member) {
    let alice = Member {
        state: ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
            .expect("create")
            .identify(),
        user: Id::<User>::mint(),
    };
    let bob = Member {
        state: open_from(&alice.state),
        user: Id::<User>::mint(),
    };
    (alice, bob)
}

/// A member opening `state`'s channel from its snapshots, with a session.
fn open_from(state: &ChannelState) -> ChannelState {
    let meta_id = state.meta_id();
    let active = state.active_id().document_id();
    let mut opening = OpeningChannel::new(state.channel_id(), meta_id);
    let meta = state.snapshot(meta_id.document_id()).expect("meta");
    assert_eq!(
        opening.receive(meta_id.document_id(), (&meta).into()),
        Receipt::Kept
    );
    assert!(matches!(
        opening.ready(meta_id.document_id()),
        Ok(Opening::OpenSegments(_))
    ));
    let segment = state.snapshot(active).expect("segment");
    assert_eq!(opening.receive(active, (&segment).into()), Receipt::Kept);
    match opening.ready(active) {
        Ok(Opening::Open(state)) => state.identify(),
        other => panic!("the channel opens: {other:?}"),
    }
}

/// The `call_end` map of the first message `member` holds.
fn ends_map(member: &Member) -> LoroMap {
    let Some(message) = crate::logic::schema::root_list(member.doc(), Root::Messages)
        .get(0)
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("a message is a map");
    };
    let Some(ValueOrContainer::Container(Container::Map(map))) = message.get(MessageKey::CallEnd)
    else {
        panic!("a call message carries its end map");
    };
    map
}

/// What `member` wrote since `before`, as a segment delta travels.
fn written_since(member: &Member, before: &VersionVector) -> Payload<DeltaBytes> {
    let doc = member.doc();
    doc.commit();
    crate::logic::delta::encode(
        &doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()),
    )
    .expect("encode")
}

// --- MURMURE-408, MURMURE-410 ---

/// A start posts one call message in the active segment, which names the call, carries an
/// empty end and reaches Bob. The `meta` document, which holds the members, stays as it was.
#[test]
fn a_start_posts_one_call_message_and_leaves_the_members() {
    let (mut alice, mut bob) = pair();
    let meta = alice.state.meta_id().document_id();
    let members = alice.state.snapshot(meta).expect("meta");
    let call = Id::<Call>::mint();
    let sent = alice.post(call);
    assert_eq!(sent.document, alice.segment().document_id());
    assert_eq!(
        alice.state.snapshot(meta).expect("meta"),
        members,
        "a start writes nothing to the members"
    );
    bob.receive(&alice, &sent, 1);
    let messages = bob.messages();
    assert_eq!(messages.len(), 1);
    let view = messages[0].call.clone().expect("a call message");
    assert_eq!(
        (view.call, view.end, view.members),
        (call, CallEnd::Open, None)
    );
    assert_eq!(messages[0].author, alice.user);
    assert!(ends_map(&bob).is_empty(), "the end map comes empty");
    assert_eq!(bob.state.live_calls(), [call]);
}

// --- MURMURE-591, MURMURE-598 ---

/// The last leave writes the end of its member: one entry under its own key, and the rest
/// of the message stays. Every member reads the call ended, and no tick asks about it again.
/// A second end of the same member writes nothing.
#[test]
fn the_last_leave_writes_an_end_every_member_reads() {
    let (mut alice, mut bob) = pair();
    let call = Id::<Call>::mint();
    let sent = alice.post(call);
    bob.receive(&alice, &sent, 1);
    let before = bob.messages();
    let ended = bob.end(call).expect("an end writes");
    assert!(bob.end(call).is_none(), "one end per member");
    alice.receive(&bob, &ended, 2);
    let after = alice.messages();
    let view = after[0].call.clone().expect("a call message");
    assert_eq!(view.end, CallEnd::Written);
    assert!(view.has_ended());
    let keys: Vec<String> = ends_map(&alice).keys().map(|k| k.to_string()).collect();
    assert_eq!(keys, [bob.user.to_string()]);
    let (mut a, mut b) = (after[0].clone(), before[0].clone());
    a.call = None;
    b.call = None;
    assert_eq!(a, b, "the end changes nothing else of the message");
    assert!(alice.state.live_calls().is_empty());
}

// --- MURMURE-593 ---

/// A delta that writes an end under another member's key, removes an end, or replaces the end
/// map of a sent message is refused whole.
#[test]
fn an_end_under_another_members_key_is_refused() {
    let (mut alice, mut bob) = pair();
    let call = Id::<Call>::mint();
    let sent = alice.post(call);
    bob.receive(&alice, &sent, 1);
    let ended = alice.end(call).expect("an end");
    bob.receive(&alice, &ended, 2);
    let document_id = alice.segment().document_id();
    let held = alice.doc().get_deep_value();

    // Bob's own end first, so his registry entry rides with it.
    let own = bob.end(call).expect("an end");
    let before = bob.doc().oplog_vv();
    ends_map(&bob)
        .insert(&Id::<User>::mint().to_string(), 1_i64)
        .expect("write");
    let foreign = written_since(&bob, &before);
    let before = bob.doc().oplog_vv();
    ends_map(&bob)
        .delete(&alice.user.to_string())
        .expect("delete");
    let removed = written_since(&bob, &before);
    for (seq, forged) in [(3, &foreign), (4, &removed)] {
        let verdict =
            alice
                .state
                .apply_signed(document_id, Seq::<Relay>::new(seq), forged, bob.signer());
        assert_eq!(
            verdict.map(|_| ()),
            Err(MurmureError::Refused(Refusal::ForeignCallEnd))
        );
    }
    assert_eq!(
        alice.doc().get_deep_value(),
        held,
        "a refused delta changes nothing"
    );
    alice.receive(&bob, &own, 5);

    // The author of the call message may not replace its end map either.
    let before = alice.doc().oplog_vv();
    let Some(first) = alice
        .doc()
        .get_list(Root::Messages.as_str())
        .get(0)
        .and_then(crate::logic::schema::fields_of)
    else {
        panic!("a message is a map");
    };
    first
        .insert_container(MessageKey::CallEnd, LoroMap::new())
        .expect("replace");
    let replaced = written_since(&alice, &before);
    assert_eq!(
        bob.state
            .apply_signed(document_id, Seq::<Relay>::new(6), &replaced, alice.signer())
            .map(|_| ()),
        Err(MurmureError::Refused(Refusal::ForeignCallEnd))
    );
}

// --- MURMURE-409, MURMURE-592 ---

/// The list shows on the call message who the relay seats in the call. It shows the call
/// ended once the relay holds no state of it, though no end was written. A call message that
/// arrives after the answer shows it too.
#[test]
fn a_call_message_shows_who_is_in_it_then_ended_once_the_relay_holds_no_state() {
    use crate::message::{MurmureEvent, UiEvent};
    use crate::ui::list::{CALL_ENDED, CALL_LIVE, Conversations};
    let (mut alice, _) = pair();
    let call = Id::<Call>::mint();
    let channel_id = alice.state.channel_id();
    let sent = alice.post(call);
    let mut lists = Conversations::default();
    let ada = Id::<User>::mint();
    let name = move |user: Id<User>| {
        (user == ada).then(|| ambre_core::Validated::parse("Ada").expect("a name"))
    };
    let body = |lists: &Conversations| {
        let list = lists.channel(channel_id).expect("a list");
        let rows = list.rows();
        let row = rows.iter().last().expect("a row");
        row.words(name, |_| String::new()).body
    };
    lists.absorb(UiEvent::Channel {
        channel_id,
        event: sent.event.clone(),
    });
    assert_eq!(body(&lists), ambre_commands::shown(&CALL_LIVE));
    assert!(lists.absorb(UiEvent::Channel {
        channel_id,
        event: MurmureEvent::CallMembers {
            call,
            members: CallMembers::Live(vec![ada]),
        },
    }));
    assert_eq!(
        body(&lists),
        format!("{}: Ada", ambre_commands::shown(&CALL_LIVE))
    );
    lists.absorb(UiEvent::Channel {
        channel_id,
        event: MurmureEvent::CallMembers {
            call,
            members: CallMembers::Gone,
        },
    });
    assert_eq!(body(&lists), ambre_commands::shown(&CALL_ENDED));
    let mut fresh = Conversations::default();
    fresh.absorb(UiEvent::Channel {
        channel_id,
        event: MurmureEvent::CallMembers {
            call,
            members: CallMembers::Gone,
        },
    });
    fresh.absorb(UiEvent::Channel {
        channel_id,
        event: sent.event,
    });
    assert_eq!(body(&fresh), ambre_commands::shown(&CALL_ENDED));
}

// --- The ring (the Murmure calls spec) ---

/// The summary of a start cues a ring of its call. The pages of the other members read it
/// with no document loaded.
#[test]
fn a_start_cues_a_ring_of_its_call_in_the_summary() {
    let (mut alice, _bob) = pair();
    let call = Id::<Call>::mint();
    let sent = alice.post(call);
    assert_eq!(
        sent.summary.call,
        Some(ambre_bus::CallCue {
            call,
            kind: ambre_bus::CueKind::Ring,
            at: NOW,
        })
    );
}

/// A decline writes the end of its member under its own key, as a leave does, and its
/// summary cues a decline of the call. A second decline of the same member writes nothing.
#[test]
fn a_decline_writes_the_end_of_its_member_and_cues_a_decline() {
    let (mut alice, mut bob) = pair();
    let call = Id::<Call>::mint();
    let sent = alice.post(call);
    bob.receive(&alice, &sent, 1);
    let declined = bob
        .state
        .decline_call(call, bob.user, NOW)
        .expect("a decline")
        .expect("the decline writes an end");
    assert_eq!(
        declined.summary.call,
        Some(ambre_bus::CallCue {
            call,
            kind: ambre_bus::CueKind::Decline,
            at: NOW,
        })
    );
    let keys: Vec<String> = ends_map(&bob).keys().map(|k| k.to_string()).collect();
    assert_eq!(keys, [bob.user.to_string()]);
    assert!(
        bob.state
            .decline_call(call, bob.user, NOW)
            .expect("a decline")
            .is_none(),
        "one end per member"
    );
}

/// The segment of a call is the held segment its call message sits in, and no segment for a
/// call no message names.
#[test]
fn the_segment_of_a_call_is_the_one_its_message_sits_in() {
    let (mut alice, _bob) = pair();
    let call = Id::<Call>::mint();
    assert_eq!(alice.state.call_segment(call), None);
    alice.post(call);
    assert_eq!(alice.state.call_segment(call), Some(alice.segment()));
    assert_eq!(alice.state.call_segment(Id::<Call>::mint()), None);
}
