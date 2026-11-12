//! The edit and the delete of a message against their contract rows
//! (`features/ambre-spec-murmure-messages.md`).

use crate::Receipt;
use crate::message::{Mention, MentionTarget, MessageDraft, MessageView, Reaction};
use ambre_bus::Char;
use ambre_bus::{ActivityKind, Authored, Instant};
use ambre_core::id::{Channel, Document, Id, Message, Segment, User};
use ambre_core::index::Span;
use ambre_core::{Relay, Seq};

use crate::logic::schema::{self, Root};
use crate::{ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Sent, Signer};

const NOW: Instant<Authored> = Instant::<Authored>::new(1_700_000_000_000);
const THUMB: &str = "\u{1F44D}";

/// One member of the channel: its state, and the user it writes for.
struct Member {
    state: ChannelState,
    user: Id<User>,
}

impl Member {
    fn segment(&self) -> Id<Segment> {
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
                Seq::<Relay>::new(seq),
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

    /// The map of the one message, read in the segment itself.
    fn message_map(&self) -> crate::logic::schema::Fields<crate::logic::schema::MessageKey> {
        let doc = self.state.doc(self.segment().document_id()).expect("held");
        let Some(map) = crate::logic::schema::root_list(doc, Root::Messages)
            .get(0)
            .and_then(crate::logic::schema::fields_of)
        else {
            panic!("a message is a map container");
        };
        map
    }

    fn edit(&mut self, message: Id<Message>, draft: &MessageDraft) -> Sent {
        self.state
            .edit(self.segment(), message, draft, self.user)
            .expect("edit")
            .expect("a new body writes")
    }
}

fn plain(text: &str) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles: Vec::new(),
    }
}

/// Alice's channel with one message of hers, `draft`, and Bob, who opened
/// it after.
fn pair(draft: &MessageDraft) -> (Member, Member, Id<Message>) {
    let mut alice = Member {
        state: ChannelState::create(Id::<Channel>::mint(), ChannelType::Public, None, NOW)
            .expect("create")
            .identify(),
        user: Id::<User>::mint(),
    };
    alice.state.send(draft, alice.user, NOW).expect("send");
    let bob = Member {
        state: open_from(&alice.state),
        user: Id::<User>::mint(),
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

// --- MURMURE-084 ---

/// An edit replaces the body and marks the message edited, here and at a
/// peer. The marks follow the draft: a mention the edit took out is gone,
/// and the one it added is there.
#[test]
fn an_edit_changes_the_body_and_marks_the_message_edited() {
    let carol = Id::<User>::mint();
    let dave = Id::<User>::mint();
    let (mut alice, mut bob, message) = pair(&MessageDraft {
        text: "@carol lunch?".to_owned(),
        styles: Vec::new(),
        mentions: vec![Mention {
            target: MentionTarget::User(carol),
            range: Span::<Char>::from_usize_range(0..6),
        }],
    });
    assert_eq!(
        alice.only_message().edited,
        crate::message::Revision::Original
    );
    let draft = MessageDraft {
        text: "lunch at noon, @dave?".to_owned(),
        styles: Vec::new(),
        mentions: vec![Mention {
            target: MentionTarget::User(dave),
            range: Span::<Char>::from_usize_range(15..20),
        }],
    };

    let sent = alice.edit(message, &draft);
    assert_eq!(sent.summary.kind, ActivityKind::Edit);
    assert!(sent.summary.mentions.is_empty());
    assert_eq!(sent.summary.mentions_all, ambre_bus::MentionScope::Named);
    bob.receive(&alice, &sent, 2);

    for member in [&alice, &bob] {
        let shown = member.only_message();
        assert_eq!(
            (
                shown.text.as_str(),
                &shown.mentions,
                shown.edited,
                shown.deleted
            ),
            (
                "lunch at noon, @dave?",
                &draft.mentions,
                crate::message::Revision::Edited,
                crate::message::Deletion::Live
            )
        );
    }
    assert!(
        alice
            .state
            .edit(alice.segment(), message, &draft, alice.user)
            .expect("edit")
            .is_none(),
        "the body already reads the draft, so nothing travels"
    );
}

/// An edit that takes a mention out and keeps its text leaves no mark on
/// that text, here and at a peer.
#[test]
fn an_edit_that_drops_a_mention_leaves_its_text_unmarked() {
    let carol = Id::<User>::mint();
    let (mut alice, mut bob, message) = pair(&MessageDraft {
        text: "@carol lunch?".to_owned(),
        styles: Vec::new(),
        mentions: vec![Mention {
            target: MentionTarget::User(carol),
            range: Span::<Char>::from_usize_range(0..6),
        }],
    });
    let sent = alice.edit(message, &plain("@carol lunch?"));
    bob.receive(&alice, &sent, 2);
    for member in [&alice, &bob] {
        let shown = member.only_message();
        assert_eq!(shown.text, "@carol lunch?");
        assert!(shown.mentions.is_empty(), "the mark went with the edit");
    }
}

/// A mention past the draft is refused before any write.
#[test]
fn an_edit_with_a_mention_past_its_body_is_refused() {
    let (mut alice, _bob, message) = pair(&plain("lunch?"));
    let draft = MessageDraft {
        text: "hi".to_owned(),
        styles: Vec::new(),
        mentions: vec![Mention {
            target: MentionTarget::Channel,
            range: Span::<Char>::from_usize_range(0..9),
        }],
    };
    let refused = alice
        .state
        .edit(alice.segment(), message, &draft, alice.user);
    assert!(matches!(refused, Err(MurmureError::BadMention)));
    assert_eq!(alice.only_message().text, "lunch?");
}

// --- MURMURE-086 ---

/// Alice edits while Bob reacts: each takes the other's delta, and both read
/// the edited body with Bob's reaction, under the same identity and author.
#[test]
fn an_edit_and_a_concurrent_reaction_both_hold() {
    let (mut alice, mut bob, message) = pair(&plain("lunch?"));
    let before = alice.only_message();

    let edited = alice.edit(message, &plain("lunch at noon?"));
    let reacted = bob
        .state
        .react(
            bob.segment(),
            message,
            &ambre_bus::Emoji::from(THUMB),
            bob.user,
            NOW,
        )
        .expect("react")
        .expect("a new reaction writes");
    alice.receive(&bob, &reacted, 2);
    bob.receive(&alice, &edited, 3);

    let reaction = Reaction {
        emoji: ambre_bus::Emoji::from(THUMB),
        users: vec![bob.user],
    };
    for member in [&alice, &bob] {
        let shown = member.only_message();
        assert_eq!(
            (shown.id, shown.author, shown.ts, shown.text.as_str()),
            (before.id, before.author, before.ts, "lunch at noon?")
        );
        assert_eq!(shown.reactions, vec![reaction.clone()]);
        assert_eq!(shown.edited, crate::message::Revision::Edited);
    }
}

// --- MURMURE-087 ---

/// A delete empties the body and marks the message deleted, here and at a
/// peer. A deleted message takes no edit and no second delete.
#[test]
fn a_delete_empties_the_body_and_marks_the_message_deleted() {
    let (mut alice, mut bob, message) = pair(&plain("my card number"));

    let sent = alice
        .state
        .delete(alice.segment(), message, alice.user)
        .expect("delete");
    assert_eq!(sent.summary.kind, ActivityKind::Edit);
    bob.receive(&alice, &sent, 2);

    for member in [&alice, &bob] {
        let shown = member.only_message();
        assert_eq!(
            (shown.text.as_str(), shown.deleted),
            ("", crate::message::Deletion::Deleted)
        );
    }
    let segment = alice.segment();
    assert!(matches!(
        alice.state.edit(segment, message, &plain("back"), alice.user),
        Err(MurmureError::Deleted(id)) if id == message
    ));
    assert!(matches!(
        alice.state.delete(segment, message, alice.user),
        Err(MurmureError::Deleted(id)) if id == message
    ));
}

/// Only the author edits or deletes: the message of another user, or one the
/// segment does not hold, is refused and nothing is written.
#[test]
fn an_edit_or_a_delete_of_a_message_not_ones_own_is_refused() {
    let (alice, mut bob, message) = pair(&plain("lunch?"));
    let segment = bob.segment();
    assert!(matches!(
        bob.state.edit(segment, message, &plain("dinner?"), bob.user),
        Err(MurmureError::NotAuthor(id)) if id == message
    ));
    assert!(matches!(
        bob.state.delete(segment, message, bob.user),
        Err(MurmureError::NotAuthor(id)) if id == message
    ));
    let absent = Id::<Message>::mint();
    assert!(matches!(
        bob.state.delete(segment, absent, bob.user),
        Err(MurmureError::NoMessage(id)) if id == absent
    ));
    let elsewhere = Id::<Segment>::from_document(Id::<Document>::mint());
    assert!(matches!(
        bob.state.delete(elsewhere, message, bob.user),
        Err(MurmureError::NotHeld(_))
    ));
    assert_eq!(bob.only_message(), alice.only_message());
}

// --- MURMURE-090 ---

/// A delete leaves the `thread` field and the reactions of its message, so
/// the replies stay reachable.
#[test]
fn a_delete_keeps_the_thread_and_the_reactions() {
    let (mut alice, mut bob, message) = pair(&plain("who is in?"));
    let reacted = bob
        .state
        .react(
            bob.segment(),
            message,
            &ambre_bus::Emoji::from(THUMB),
            bob.user,
            NOW,
        )
        .expect("react")
        .expect("a new reaction writes");
    alice.receive(&bob, &reacted, 2);
    // The thread document the first reply would name (the Murmure threads
    // spec), written by the author herself until the threads land.
    let thread = Id::<Document>::mint().to_string();
    alice
        .message_map()
        .insert(crate::logic::schema::MessageKey::Thread, thread.as_str())
        .expect("thread");

    let sent = alice
        .state
        .delete(alice.segment(), message, alice.user)
        .expect("delete");
    bob.receive(&alice, &sent, 3);

    for member in [&alice, &bob] {
        assert_eq!(
            schema::string(
                &member.message_map(),
                crate::logic::schema::MessageKey::Thread
            )
            .as_deref(),
            Some(thread.as_str())
        );
        let shown = member.only_message();
        assert_eq!(shown.deleted, crate::message::Deletion::Deleted);
        assert_eq!(shown.reactions.len(), 1, "the reaction stays");
    }
}
