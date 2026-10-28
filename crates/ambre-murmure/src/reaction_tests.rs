//! The reactions of a message against their contract rows
//! (`features/ambre-spec-murmure-reactions.md`).

use crate::Receipt;
use ambre_bus::{ActivityKind, MessageDraft, MessageView, Reaction, Timestamp, UnreadCount};
use ambre_core::{ChannelId, DeltaBytes, MessageId, SegmentId, Seq, UserId};
use loro::{Container, LoroMap, ValueOrContainer, VersionVector};

use crate::schema::{MessageKey, Root};
use crate::{
    ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Refusal, Sent, Signer,
};

const NOW: Timestamp = Timestamp(1_700_000_000_000);
const THUMB: &str = "\u{1F44D}";

/// One member of the channel: its state, and the user it writes for.
struct Member {
    state: ChannelState,
    user: UserId,
}

impl Member {
    fn segment(&self) -> SegmentId {
        self.state.active_id()
    }

    /// Who signs what this member writes in the active segment.
    fn signer(&self) -> Signer {
        Signer {
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
                Seq(seq),
                &sent.delta,
                from.signer(),
            )
            .expect("a delta the check passes");
    }

    fn only_message(&self) -> MessageView {
        let messages = self.state.messages(self.segment()).expect("held");
        assert_eq!(messages.len(), 1);
        messages[0].clone()
    }

    fn react(&mut self, message: MessageId, emoji: &str) -> Sent {
        self.state
            .react(
                self.segment(),
                message,
                &ambre_bus::Emoji::from(emoji),
                self.user,
                NOW,
            )
            .expect("react")
            .expect("a new reaction writes")
    }
}

/// Alice's channel with one message of hers, and Bob, who opened it after.
fn pair() -> (Member, Member, MessageId) {
    let mut alice = Member {
        state: ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
            .expect("create")
            .identify(),
        user: UserId::new(),
    };
    let draft = MessageDraft {
        text: "lunch?".to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    };
    alice.state.send(&draft, alice.user, NOW).expect("send");
    let bob = Member {
        state: open_from(&alice.state),
        user: UserId::new(),
    };
    let message = alice.only_message().id;
    (alice, bob, message)
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

fn one(emoji: &str, mut users: Vec<UserId>) -> Reaction {
    users.sort_by_key(ToString::to_string);
    Reaction {
        emoji: ambre_bus::Emoji::from(emoji),
        users,
    }
}

// --- MURMURE-092, MURMURE-093, MURMURE-095, MURMURE-256 ---

/// A sent message carries an empty `reactions` map. A reaction adds one
/// entry under its member, summarised as a reaction, and the body stays.
#[test]
fn a_reaction_adds_one_entry_and_leaves_the_message() {
    let (mut alice, mut bob, message) = pair();
    let before = alice.only_message();
    assert!(before.reactions.is_empty());
    assert!(reactions_map(&alice).is_empty(), "the map is created empty");

    let sent = bob.react(message, THUMB);
    assert_eq!(sent.summary.kind, ActivityKind::Reaction);
    assert!(sent.summary.mentions.is_empty());
    assert_eq!(sent.summary.mentions_all, ambre_bus::MentionScope::Named);
    alice.receive(&bob, &sent, 2);

    let after = alice.only_message();
    assert_eq!(after.reactions, vec![one(THUMB, vec![bob.user])]);
    assert_eq!(
        (after.id, after.author, after.ts, &after.text),
        (before.id, before.author, before.ts, &before.text),
        "the message itself is unchanged"
    );
    assert!(
        bob.state
            .react(
                bob.segment(),
                message,
                &ambre_bus::Emoji::from(THUMB),
                bob.user,
                NOW
            )
            .expect("react")
            .is_none(),
        "a reaction already there writes nothing"
    );
    assert_eq!(
        bob.state
            .react(
                bob.segment(),
                message,
                &ambre_bus::Emoji::from("ok"),
                bob.user,
                NOW
            )
            .map(|_| ()),
        Err(MurmureError::NotEmoji)
    );
}

// --- MURMURE-006, MURMURE-007 ---

/// Two members reacting with one emoji at once write two keys of one flat
/// map, and the merge keeps both.
#[test]
fn two_members_reacting_at_once_both_stay() {
    let (mut alice, mut bob, message) = pair();
    let from_alice = alice.react(message, THUMB);
    let from_bob = bob.react(message, THUMB);
    alice.receive(&bob, &from_bob, 2);
    bob.receive(&alice, &from_alice, 3);

    let both = vec![one(THUMB, vec![alice.user, bob.user])];
    assert_eq!(alice.only_message().reactions, both);
    assert_eq!(bob.only_message().reactions, both);
}

// --- MURMURE-096, MURMURE-097 ---

/// Removing a reaction deletes the member's own entry, and the entry of the
/// other member under the same emoji stays.
#[test]
fn removing_a_reaction_deletes_only_ones_own() {
    let (mut alice, mut bob, message) = pair();
    let from_bob = bob.react(message, THUMB);
    alice.receive(&bob, &from_bob, 2);
    let from_alice = alice.react(message, THUMB);
    bob.receive(&alice, &from_alice, 3);

    let removed = alice
        .state
        .unreact(
            alice.segment(),
            message,
            &ambre_bus::Emoji::from(THUMB),
            alice.user,
        )
        .expect("unreact")
        .expect("an entry to remove");
    assert_eq!(removed.summary.kind, ActivityKind::Reaction);
    bob.receive(&alice, &removed, 4);

    let left = vec![one(THUMB, vec![bob.user])];
    assert_eq!(alice.only_message().reactions, left);
    assert_eq!(bob.only_message().reactions, left);
    assert!(
        alice
            .state
            .unreact(
                alice.segment(),
                message,
                &ambre_bus::Emoji::from(THUMB),
                alice.user
            )
            .expect("unreact")
            .is_none(),
        "no entry of hers is left, so nothing travels"
    );
}

/// The `reactions` map of the first message, as `member` holds it.
fn reactions_map(member: &Member) -> LoroMap {
    let doc = member
        .state
        .doc(member.segment().document_id())
        .expect("held");
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message is a map");
    };
    let Some(ValueOrContainer::Container(Container::Map(map))) = message.get(MessageKey::Reactions)
    else {
        panic!("a message carries its reactions map");
    };
    map
}

/// What `member` wrote since `before`, as a segment delta travels.
fn written_since(member: &Member, before: &VersionVector) -> DeltaBytes {
    let doc = member
        .state
        .doc(member.segment().document_id())
        .expect("held");
    doc.commit();
    crate::delta::encode(&doc.export_json_updates_without_peer_compression(before, &doc.oplog_vv()))
        .expect("encode")
}

// --- MURMURE-094 ---

/// A delta that writes or removes a reaction under another member's key is
/// refused whole. So is one that replaces the map of a sent message.
#[test]
fn a_reaction_under_another_members_key_is_refused() {
    let (mut alice, mut bob, message) = pair();
    let from_alice = alice.react(message, THUMB);
    bob.receive(&alice, &from_alice, 2);
    let doc = |alice: &Member| {
        alice
            .state
            .doc(alice.segment().document_id())
            .expect("held")
            .get_deep_value()
    };
    let held = doc(&alice);

    // Bob's own first write, so the registry entry rides with it.
    let own = bob.react(message, "\u{1F389}");
    let before = bob
        .state
        .doc(bob.segment().document_id())
        .expect("held")
        .oplog_vv();
    let forged = crate::reaction::ReactionKey::of(alice.user, &ambre_bus::Emoji::from("\u{1F44E}"));
    reactions_map(&bob)
        .insert(forged.as_str(), 1_i64)
        .expect("write");
    let inserted = written_since(&bob, &before);
    let removal_from = bob
        .state
        .doc(bob.segment().document_id())
        .expect("held")
        .oplog_vv();
    reactions_map(&bob)
        .delete(
            crate::reaction::ReactionKey::of(alice.user, &ambre_bus::Emoji::from(THUMB)).as_str(),
        )
        .expect("delete");
    let removed = written_since(&bob, &removal_from);

    let document_id = alice.segment().document_id();
    for (seq, forged) in [(3, &inserted), (4, &removed)] {
        let verdict = alice
            .state
            .apply_signed(document_id, Seq(seq), forged, bob.signer());
        assert_eq!(
            verdict.map(|_| ()),
            Err(MurmureError::Refused(Refusal::ForeignReaction))
        );
    }
    assert_eq!(doc(&alice), held, "a refused delta changes nothing");
    alice.receive(&bob, &own, 5);
    assert_eq!(
        alice.only_message().reactions,
        vec![
            one("\u{1F389}", vec![bob.user]),
            one(THUMB, vec![alice.user])
        ]
    );

    // The author of the message may not replace its map either.
    let before = alice.state.doc(document_id).expect("held").oplog_vv();
    let Some(first) = alice
        .state
        .doc(document_id)
        .expect("held")
        .get_list(Root::Messages.as_str())
        .get(0)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message is a map");
    };
    first
        .insert_container(MessageKey::Reactions, LoroMap::new())
        .expect("replace");
    let replaced = written_since(&alice, &before);
    assert_eq!(
        bob.state
            .apply_signed(document_id, Seq(6), &replaced, alice.signer())
            .map(|_| ()),
        Err(MurmureError::Refused(Refusal::ForeignReaction))
    );
}

// --- MURMURE-091 ---

/// A reaction delivered past the cursor creates no message, so the unread
/// count of the segment does not move.
#[test]
fn a_reaction_never_counts_as_unread() {
    let (mut alice, mut bob, message) = pair();
    let segment = bob.segment();
    let count = |bob: &Member| bob.state.unread(segment, Seq(1), bob.user).expect("count");
    assert_eq!(count(&bob), UnreadCount(1), "Alice's message is unread");
    let sent = alice.react(message, THUMB);
    bob.receive(&alice, &sent, 2);
    assert_eq!(count(&bob), UnreadCount(1), "her reaction adds nothing");
}
