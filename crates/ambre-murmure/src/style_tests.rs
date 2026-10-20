//! The style marks of a message body against their contract rows
//! (`features/ambre-spec-murmure-composer.md`).

use ambre_bus::{MessageDraft, MessageView, StyleMark, TextStyle, Timestamp};
use ambre_core::{ChannelId, Seq, UserId};
use loro::{Container, LoroText, LoroValue, TextDelta, ValueOrContainer};

use crate::schema::{BOLD, BULLETS, CODE_BLOCK, LINK, MESSAGE_CONTENT, MESSAGES, QUOTE};
use crate::{ChannelState, ChannelType, MurmureError, Opening, OpeningChannel, Sent, Signer};

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
    assert!(opening.receive(meta_id.document_id(), &meta));
    assert!(matches!(
        opening.ready(meta_id.document_id()),
        Ok(Opening::OpenSegments(_))
    ));
    let segment = state.snapshot(active).expect("segment");
    assert!(opening.receive(active, &segment));
    match opening.ready(active) {
        Ok(Opening::Open(state)) => state.identify(),
        other => panic!("the channel opens: {other:?}"),
    }
}

fn mark(style: TextStyle, range: std::ops::Range<usize>) -> StyleMark {
    StyleMark { style, range }
}

fn link(address: &str) -> TextStyle {
    TextStyle::Link(address.to_owned())
}

fn draft(text: &str, styles: Vec<StyleMark>) -> MessageDraft {
    MessageDraft {
        text: text.to_owned(),
        mentions: Vec::new(),
        styles,
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

/// The body of message `n`, as the segment holds it.
fn body(state: &ChannelState, n: usize) -> LoroText {
    let doc = state.doc(state.active_id().document_id()).expect("held");
    let Some(ValueOrContainer::Container(Container::Map(message))) = doc.get_list(MESSAGES).get(n)
    else {
        panic!("a message is a map");
    };
    let Some(ValueOrContainer::Container(Container::Text(content))) = message.get(MESSAGE_CONTENT)
    else {
        panic!("a body is a text");
    };
    content
}

/// The value each span of the body of message `n` holds under `key`.
fn stored(state: &ChannelState, n: usize, key: &str) -> Vec<(String, Option<LoroValue>)> {
    body(state, n)
        .to_delta()
        .into_iter()
        .filter_map(|span| match span {
            TextDelta::Insert { insert, attributes } => Some((
                insert,
                attributes.and_then(|attributes| attributes.get(key).cloned()),
            )),
            _ => None,
        })
        .collect()
}

// --- MURMURE-562 ---

/// Each style is a mark on the body, beside the mentions, and a member reads
/// it from the delta alone.
#[test]
fn a_style_is_a_mark_on_the_body_that_travels_with_it() {
    let (mut alice, author) = alice();
    let styles = vec![
        mark(TextStyle::Bold, 0..4),
        mark(TextStyle::Italic, 2..8),
        mark(TextStyle::Code, 10..14),
        mark(link("https://ambre.dev"), 15..19),
    ];
    let text = "very much code site";
    let mut bob = open_from(&alice);
    let sent = alice
        .send(&draft(text, styles.clone()), author, NOW)
        .expect("send");
    assert_eq!(messages(&alice)[0].styles, styles);
    deliver(&mut bob, &alice, author, &sent, 1);
    assert_eq!(messages(&bob)[0].styles, styles, "the delta carries them");

    let marked = |key: &str, value: LoroValue| -> String {
        stored(&alice, 0, key)
            .into_iter()
            .filter(|(_, held)| held.as_ref() == Some(&value))
            .map(|(text, _)| text)
            .collect()
    };
    assert_eq!(marked(BOLD, LoroValue::Bool(true)), "very");
    assert_eq!(marked(LINK, LoroValue::from("https://ambre.dev")), "site");
}

// --- MURMURE-194 ---

/// A code block, a quote and a list are marks on the body like the inline
/// styles. A member reads them from the delta alone. Text put at the edge
/// of a block stays out of it.
#[test]
fn a_block_mark_travels_with_the_body() {
    let (mut alice, author) = alice();
    let styles = vec![
        mark(TextStyle::Quote, 0..5),
        mark(TextStyle::CodeBlock, 5..10),
        mark(TextStyle::List, 10..14),
    ];
    let text = "said\nfn a\none\ntwo";
    let mut bob = open_from(&alice);
    let sent = alice
        .send(&draft(text, styles.clone()), author, NOW)
        .expect("send");
    deliver(&mut bob, &alice, author, &sent, 1);
    assert_eq!(messages(&bob)[0].styles, styles, "the delta carries them");

    let marked = |state: &ChannelState, key: &str| -> String {
        stored(state, 0, key)
            .into_iter()
            .filter(|(_, held)| held == &Some(LoroValue::Bool(true)))
            .map(|(text, _)| text)
            .collect()
    };
    assert_eq!(marked(&alice, QUOTE), "said\n");
    assert_eq!(marked(&alice, CODE_BLOCK), "fn a\n");
    assert_eq!(marked(&alice, BULLETS), "one\n");
    body(&alice, 0).insert(5, "x").expect("an insert");
    assert_eq!(marked(&alice, QUOTE), "said\n", "a block does not grow");
    assert_eq!(marked(&alice, CODE_BLOCK), "fn a\n");
}

/// The runs a draft gives that touch or overlap read as one, in text order.
#[test]
fn runs_of_one_style_read_as_one() {
    let (mut alice, author) = alice();
    let styles = vec![
        mark(TextStyle::Bold, 3..6),
        mark(TextStyle::Bold, 0..3),
        mark(TextStyle::Bold, 5..7),
    ];
    alice
        .send(&draft("bold it", styles.clone()), author, NOW)
        .expect("send");
    assert_eq!(messages(&alice)[0].styles, [mark(TextStyle::Bold, 0..7)]);
    assert_eq!(
        crate::style::normalized(&styles),
        messages(&alice)[0].styles,
        "a read gives what the draft normalizes to"
    );
}

/// An edit rewrites the styles of the body, and one that changes neither
/// the text nor its styles writes nothing.
#[test]
fn an_edit_rewrites_the_styles_of_the_body() {
    let (mut alice, author) = alice();
    let mut bob = open_from(&alice);
    let sent = alice
        .send(
            &draft("one two", vec![mark(TextStyle::Bold, 0..3)]),
            author,
            NOW,
        )
        .expect("send");
    deliver(&mut bob, &alice, author, &sent, 1);
    let (segment, id) = (alice.active_id(), messages(&alice)[0].id);

    let same = draft("one two", vec![mark(TextStyle::Bold, 0..3)]);
    assert!(matches!(alice.edit(segment, id, &same, author), Ok(None)));

    let moved = draft("one two", vec![mark(TextStyle::Italic, 4..7)]);
    let edited = alice
        .edit(segment, id, &moved, author)
        .expect("edit")
        .expect("the styles changed");
    deliver(&mut bob, &alice, author, &edited, 2);
    let read = &messages(&bob)[0];
    assert_eq!((read.text.as_str(), read.edited), ("one two", true));
    assert_eq!(read.styles, [mark(TextStyle::Italic, 4..7)], "bold is gone");

    let plain = draft("one two three", Vec::new());
    alice
        .edit(segment, id, &plain, author)
        .expect("edit")
        .expect("a change");
    assert!(messages(&alice)[0].styles.is_empty());
}

// --- MURMURE-561 ---

/// A send, an edit and a reply refuse a style outside the body and write
/// nothing. So do a link to another scheme and two links over one char.
#[test]
fn a_style_outside_the_body_or_a_bad_link_is_refused() {
    let (mut alice, author) = alice();
    alice
        .send(&draft("base", Vec::new()), author, NOW)
        .expect("send");
    let (segment, id) = (alice.active_id(), messages(&alice)[0].id);
    let before = messages(&alice);

    let refused = [
        vec![mark(TextStyle::Bold, 2..2)],
        vec![mark(TextStyle::Italic, 3..9)],
        vec![mark(link("javascript:alert(1)"), 0..4)],
        vec![mark(link("https://"), 0..4)],
        vec![mark(link("https://a b"), 0..4)],
        vec![
            mark(link("https://one.example"), 0..3),
            mark(link("https://two.example"), 2..4),
        ],
    ];
    for styles in refused {
        let bad = draft("text", styles.clone());
        assert!(
            matches!(alice.send(&bad, author, NOW), Err(MurmureError::BadStyle)),
            "send {styles:?}"
        );
        assert!(
            matches!(
                alice.edit(segment, id, &bad, author),
                Err(MurmureError::BadStyle)
            ),
            "edit {styles:?}"
        );
        assert!(
            matches!(
                alice.reply(id, &bad, author, NOW, false),
                Err(MurmureError::BadStyle)
            ),
            "reply {styles:?}"
        );
    }
    assert_eq!(messages(&alice), before, "nothing was written");

    let fine = draft(
        "text",
        vec![
            mark(link("HTTP://one.example/a?b=c"), 0..2),
            mark(link("https://two.example"), 2..4),
        ],
    );
    alice
        .send(&fine, author, NOW)
        .expect("two links side by side");
}

/// A link mark whose value is no `http` or `https` address, as another
/// writer could put on the body, reads as text.
#[test]
fn a_link_to_another_scheme_reads_as_text() {
    let (mut alice, author) = alice();
    alice
        .send(
            &draft("click here", vec![mark(TextStyle::Bold, 0..5)]),
            author,
            NOW,
        )
        .expect("send");
    let doc = alice.doc(alice.active_id().document_id()).expect("held");
    let content = body(&alice, 0);
    content
        .mark(6..10, LINK, LoroValue::from("javascript:alert(1)"))
        .expect("mark");
    content
        .mark(0..5, LINK, LoroValue::Bool(true))
        .expect("mark");
    doc.commit();
    assert_eq!(messages(&alice)[0].styles, [mark(TextStyle::Bold, 0..5)]);
}
