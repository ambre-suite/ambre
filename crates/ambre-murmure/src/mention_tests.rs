//! The mentions of a message against their contract rows
//! (`features/ambre-spec-murmure-mentions.md`).

use crate::Receipt;
use ambre_bus::CharSpan;
use ambre_bus::{Mention, MentionCount, MentionTarget, MessageDraft, MessageView, Timestamp};
use ambre_core::{ChannelId, Seq, UserId};
use loro::{Container, LoroValue, TextDelta, ValueOrContainer};

use crate::schema::{Mark, MessageKey, Root};
use crate::{
    CONFIRM_CHANNEL_PAST, ChannelState, ChannelType, MurmureError, Opening, OpeningChannel,
    Screened, Sent, Signer, screen,
};

const NOW: Timestamp = Timestamp(1_700_000_000_000);

/// Alice's channel, created empty, and the user she writes for.
fn alice() -> (ChannelState, UserId) {
    let state = ChannelState::create(ChannelId::new(), ChannelType::Public, None, NOW)
        .expect("create")
        .identify();
    (state, UserId::new())
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

fn at(target: MentionTarget, range: std::ops::Range<usize>) -> Mention {
    Mention {
        target,
        range: CharSpan::from_usize_range(range),
    }
}

fn draft(text: &str, mentions: Vec<Mention>) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions,
        styles: Vec::new(),
    }
}

/// Hand what `from` wrote, signed by `author`, to `to` under number `seq`.
fn deliver(to: &mut ChannelState, from: &ChannelState, author: UserId, sent: &Sent, seq: u64) {
    let document_id = sent.segment.document_id();
    let signer = Signer {
        peer: from.peer(document_id).expect("a session"),
        user: Some(author),
    };
    to.apply_signed(document_id, Seq(seq), &sent.delta, signer)
        .expect("a delta the check passes");
}

fn messages(state: &ChannelState) -> Vec<MessageView> {
    state.messages(state.active_id()).expect("held")
}

/// The value of every `mention` mark over the body of message `n`.
fn stored_marks(state: &ChannelState, n: usize) -> Vec<LoroValue> {
    let doc = state.doc(state.active_id().document_id()).expect("held");
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(n)
        .and_then(crate::schema::fields_of)
    else {
        panic!("a message is a map");
    };
    let Some(ValueOrContainer::Container(Container::Text(content))) =
        message.get(MessageKey::Content)
    else {
        panic!("a body is a text");
    };
    content
        .to_delta()
        .into_iter()
        .filter_map(|span| match span {
            TextDelta::Insert {
                attributes: Some(attributes),
                ..
            } => attributes.get(Mark::Mention.as_str()).cloned(),
            _ => None,
        })
        .collect()
}

// --- MURMURE-008, MURMURE-100, MURMURE-103 ---

/// The composer's mention becomes a mark over the name, carrying the user
/// identifier. The receiver finds it by the mark: an `@` name typed with no
/// mark is no mention.
#[test]
fn a_mention_is_a_mark_that_carries_its_member() {
    let (mut state, author) = alice();
    let mut bob_state = open_from(&state);
    let bob = UserId::new();
    let sent = state
        .send(
            &draft(
                "hi @bob and @carol",
                vec![at(MentionTarget::User(bob), 3..7)],
            ),
            author,
            NOW,
        )
        .expect("send");
    deliver(&mut bob_state, &state, author, &sent, 2);

    assert_eq!(
        stored_marks(&bob_state, 0),
        vec![LoroValue::from(bob.to_string())]
    );
    let shown = messages(&bob_state);
    assert_eq!(
        shown[0].mentions,
        vec![at(MentionTarget::User(bob), 3..7)],
        "the unmarked @carol is text, not a mention"
    );
    assert_eq!(shown[0].text, "hi @bob and @carol");
}

// --- MURMURE-098 ---

/// A mention reads as the current name of its member, so a rename shows in
/// every message that names them. A name not known keeps the typed text.
#[test]
fn a_mention_shows_the_current_name_of_its_member() {
    let (mut state, author) = alice();
    let bob = UserId::new();
    state
        .send(
            &draft("hi @bob", vec![at(MentionTarget::User(bob), 3..7)]),
            author,
            NOW,
        )
        .expect("send");
    let message = &messages(&state)[0];
    let named = |name: &'static str| {
        move |user: UserId| (user == bob).then(|| ambre_core::DisplayName::new(name).unwrap())
    };
    assert_eq!(message.shown(named("Bob")), "hi @Bob");
    assert_eq!(message.shown(named("Robert")), "hi @Robert");
    assert_eq!(message.shown(|_| None), "hi @bob");
}

// --- MURMURE-229, MURMURE-230, MURMURE-232 ---

/// An `@channel` is one mark naming the channel. Its summary says so and
/// lists no member, and the message holds no list of members.
#[test]
fn an_at_channel_names_the_channel_and_no_member() {
    let (mut state, author) = alice();
    let sent = state
        .send(
            &draft("@channel lunch", vec![at(MentionTarget::Channel, 0..8)]),
            author,
            NOW,
        )
        .expect("send");
    assert!(sent.summary.mentions_all);
    assert!(sent.summary.mentions.is_empty());
    assert_eq!(
        stored_marks(&state, 0),
        vec![LoroValue::from(state.channel_id().to_string())]
    );
    assert_eq!(
        messages(&state)[0].mentions,
        vec![at(MentionTarget::Channel, 0..8)]
    );
    let doc = state.doc(state.active_id().document_id()).expect("held");
    let Some(message) = crate::schema::root_list(doc, Root::Messages)
        .get(0)
        .and_then(crate::schema::fields_of::<MessageKey>)
    else {
        panic!("a message is a map");
    };
    let mut keys = message
        .map()
        .keys()
        .map(|k| k.to_string())
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        ["author", "content", "id", "reactions", "ts"],
        "no field lists the members"
    );
}

// --- MURMURE-233, MURMURE-104 ---

/// A message counts as a mention for the member its mark names, and an
/// `@channel` for every member. The marks decide: a summary that claims a
/// mention the message does not carry counts nothing once it is loaded.
#[test]
fn a_mention_counts_for_its_member_and_an_at_channel_for_all() {
    let (mut state, author) = alice();
    let mut held = open_from(&state);
    let (bob, carol) = (UserId::new(), UserId::new());
    let count = |held: &ChannelState, user| {
        held.mentions(held.active_id(), Seq::ZERO, user)
            .expect("count")
    };

    let mut claimed = state
        .send(&draft("no mark here", Vec::new()), author, NOW)
        .expect("send");
    claimed.summary.mentions = vec![bob];
    deliver(&mut held, &state, author, &claimed, 2);
    assert_eq!(
        count(&held, bob),
        MentionCount(0),
        "the claim reads as nothing"
    );

    let to_bob = state
        .send(
            &draft("@bob look", vec![at(MentionTarget::User(bob), 0..4)]),
            author,
            NOW,
        )
        .expect("send");
    deliver(&mut held, &state, author, &to_bob, 3);
    assert_eq!(
        (count(&held, bob), count(&held, carol)),
        (MentionCount(1), MentionCount(0))
    );

    let to_all = state
        .send(
            &draft("@channel", vec![at(MentionTarget::Channel, 0..8)]),
            author,
            NOW,
        )
        .expect("send");
    deliver(&mut held, &state, author, &to_all, 4);
    assert_eq!(
        (count(&held, bob), count(&held, carol)),
        (MentionCount(2), MentionCount(1))
    );
    assert_eq!(
        count(&held, author),
        MentionCount(0),
        "her own messages never count"
    );
}

// --- MURMURE-152 ---

/// Reading the mentions of a message, or counting them, writes nothing.
#[test]
fn reading_the_mentions_changes_nothing() {
    let (mut state, author) = alice();
    let bob = UserId::new();
    state
        .send(
            &draft("@bob", vec![at(MentionTarget::User(bob), 0..4)]),
            author,
            NOW,
        )
        .expect("send");
    let doc = state.doc(state.active_id().document_id()).expect("held");
    let (version, value) = (doc.oplog_vv(), doc.get_deep_value());
    assert_eq!(messages(&state)[0].mentions.len(), 1);
    state
        .mentions(state.active_id(), Seq::ZERO, bob)
        .expect("count");
    let doc = state.doc(state.active_id().document_id()).expect("held");
    assert_eq!((doc.oplog_vv(), doc.get_deep_value()), (version, value));
}

/// A mention outside the body is refused before anything is written.
#[test]
fn a_mention_outside_the_body_writes_nothing() {
    let (mut state, author) = alice();
    let before = state
        .doc(state.active_id().document_id())
        .expect("held")
        .oplog_vv();
    for range in [3..9, 2..2] {
        assert_eq!(
            state
                .send(
                    &draft("hi @x", vec![at(MentionTarget::User(author), range)]),
                    author,
                    NOW,
                )
                .map(|_| ()),
            Err(MurmureError::BadMention)
        );
    }
    let doc = state.doc(state.active_id().document_id()).expect("held");
    assert_eq!(doc.oplog_vv(), before);
    assert!(messages(&state).is_empty());
}

// --- MURMURE-099 ---

/// A mention of a user outside the channel leaves the draft as no mark, its
/// text untouched. Before the members are listed, every mention stays.
#[test]
fn a_mention_of_a_non_member_is_no_mark() {
    let (bob, eve) = (UserId::new(), UserId::new());
    let both = draft(
        "@bob @eve",
        vec![
            at(MentionTarget::User(bob), 0..4),
            at(MentionTarget::User(eve), 5..9),
        ],
    );
    assert_eq!(
        screen(both.clone(), Some(&[bob]), false),
        Screened::Send(draft("@bob @eve", vec![at(MentionTarget::User(bob), 0..4)]))
    );
    assert_eq!(screen(both.clone(), None, false), Screened::Send(both));
}

// --- MURMURE-231 ---

/// An `@channel` past the member threshold waits for the author's
/// confirmation. At the threshold, or once confirmed, it is sent.
#[test]
fn an_at_channel_past_the_threshold_waits_for_confirmation() {
    let all = draft("@channel", vec![at(MentionTarget::Channel, 0..8)]);
    let members: Vec<UserId> = (0..=CONFIRM_CHANNEL_PAST).map(|_| UserId::new()).collect();
    let at_threshold = &members[..CONFIRM_CHANNEL_PAST];
    assert_eq!(
        screen(all.clone(), Some(at_threshold), false),
        Screened::Send(all.clone())
    );
    assert_eq!(
        screen(all.clone(), Some(&members), false),
        Screened::Confirm {
            members: CONFIRM_CHANNEL_PAST + 1,
            draft: all.clone(),
        }
    );
    assert_eq!(
        screen(all.clone(), Some(&members), true),
        Screened::Send(all)
    );
    let plain = draft("hello", vec![]);
    assert_eq!(
        screen(plain.clone(), Some(&members), false),
        Screened::Send(plain)
    );
}
