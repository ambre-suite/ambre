//! Two peers, one document: the convergence belt of the Prose engine
//! (`1-Foundations/ambre-spec-testing.md`, "Reference model").
//!
//! Each test drives two `ProseState`s through their own commands, exchanges the
//! deltas both ways, and asserts on what the two hold afterwards. No worker, no
//! browser and no channel: a handler answers with a value, which is what makes
//! this shape reachable at all (`PROSE-024`).

use crate::events::{
    Align, BlockInit, ListStyle, OrderedStyle, ParagraphStyle, ProseCommand, ProseEvent,
};
use crate::logic::ProseState;

/// Two peers over one seeded document, so both start from the same block ids.
///
/// The seed travels as a snapshot, which is how a second peer joins a document
/// in the live system.
fn two_peers() -> (ProseState, ProseState, crate::events::BlockId) {
    let alice = ProseState::with_peer(1).expect("a document is built");
    let block = alice.root_blocks()[0];
    let seed = alice.export_snapshot();
    let mut bob = ProseState::with_peer(2).expect("a document is built");
    bob.import_snapshot(&seed).expect("the seed imports");
    (alice, bob, block)
}

/// Hand each peer what the other wrote.
fn exchange(alice: &mut ProseState, bob: &mut ProseState, from_alice: &[u8], from_bob: &[u8]) {
    bob.apply_remote(from_alice);
    alice.apply_remote(from_bob);
}

/// The text of every live root block, in document order.
fn rendered(state: &ProseState) -> Vec<String> {
    state
        .root_blocks()
        .into_iter()
        .map(|id| state.text_of(&id).unwrap_or_default())
        .collect()
}

#[test]
fn two_peers_typing_in_one_paragraph_keep_both_sentences() {
    let (mut alice, mut bob, block) = two_peers();
    let from_alice = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "Alice writes.".to_owned(),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "Bob writes.".to_owned(),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let text = alice.text_of(&block).expect("a body");
    assert_eq!(text, bob.text_of(&block).expect("a body"));
    // Neither sentence is lost, and neither is interleaved into the other
    // (`PROSE-089`).
    assert!(text.contains("Alice writes."), "{text}");
    assert!(text.contains("Bob writes."), "{text}");
}

#[test]
fn two_peers_setting_two_paragraph_properties_lose_neither() {
    let (mut alice, mut bob, block) = two_peers();
    let from_alice = alice
        .handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: ParagraphStyle {
                align: Some(Align::Center),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: ParagraphStyle {
                space_after: Some(12.0),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    for state in [&alice, &bob] {
        let style = state.paragraph_style_of(&block);
        assert_eq!(style.align, Some(Align::Center));
        assert_eq!(style.space_after, Some(12.0));
    }
}

#[test]
fn two_peers_writing_one_size_merge_to_one_value() {
    let (mut alice, mut bob, block) = two_peers();
    let seed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "sized".to_owned(),
        })
        .update
        .expect("a delta");
    bob.apply_remote(&seed);

    // One peer spells the size with a trailing zero and the other without. The
    // canonical form makes both write the same bytes (`PROSE-008`,
    // `PROSE-091`).
    let from_alice = alice
        .handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "fs".to_owned(),
            value: Some("11.0".to_owned()),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: (0, 5),
            key: "fs".to_owned(),
            value: Some("11".to_owned()),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let alice_marks = format!("{:?}", alice.marks_of(&block));
    assert_eq!(alice_marks, format!("{:?}", bob.marks_of(&block)));
    assert!(alice_marks.contains("\"11\""), "{alice_marks}");
    assert!(!alice_marks.contains("11.0"), "{alice_marks}");
}

#[test]
fn two_peers_emptying_the_tree_converge_on_one_block() {
    let (mut alice, mut bob, block) = two_peers();
    // Both delete the document's only block at the same moment. Neither may
    // recreate one locally, or the merge would hold two (`PROSE-094`).
    let from_alice = alice
        .handle_command(ProseCommand::DeleteBlock { block_id: block })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::DeleteBlock { block_id: block })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert_eq!(alice.root_blocks().len(), 1);
    assert_eq!(alice.root_blocks(), bob.root_blocks());
}

#[test]
fn a_block_a_peer_moved_keeps_every_reference_to_it() {
    let (mut alice, mut bob, first) = two_peers();
    let seeded = alice
        .handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        })
        .update
        .expect("a delta");
    bob.apply_remote(&seeded);
    let second = alice.root_blocks()[1];

    // Alice moves the block while Bob types in it. The move keeps the block's
    // identity, so Bob's text lands in the block Alice moved (`PROSE-086`).
    let from_alice = alice
        .handle_command(ProseCommand::MoveBlock {
            block_id: first,
            new_parent: None,
            after_id: Some(second),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: 0,
            text: "typed".to_owned(),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert_eq!(alice.text_of(&second).as_deref(), Some("typed"));
    assert_eq!(rendered(&alice), rendered(&bob));
}

#[test]
fn a_converged_document_holds_only_the_parentages_the_format_declares() {
    let (mut alice, mut bob, first) = two_peers();
    let from_alice = alice
        .handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Heading { level: 2 },
            style_id: None,
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::CodeBlock {
                lang: "rust".to_owned(),
            },
            style_id: None,
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert!(alice.parentages_are_declared());
    assert!(bob.parentages_are_declared());
    assert_eq!(alice.root_blocks(), bob.root_blocks());
}

#[test]
fn an_offline_edit_and_a_concurrent_one_both_land() {
    let (mut alice, mut bob, block) = two_peers();
    // Alice works offline: three commands, three deltas, none delivered yet.
    let mut offline = Vec::new();
    for text in ["one ", "two ", "three"] {
        offline.push(
            alice
                .handle_command(ProseCommand::InsertText {
                    block_id: block,
                    pos: alice
                        .text_of(&block)
                        .map(|t| t.chars().count())
                        .unwrap_or(0) as u32,
                    text: text.to_owned(),
                })
                .update
                .expect("a delta"),
        );
    }
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "meanwhile ".to_owned(),
        })
        .update
        .expect("a delta");
    for delta in &offline {
        bob.apply_remote(delta);
    }
    alice.apply_remote(&from_bob);

    let text = alice.text_of(&block).expect("a body");
    assert_eq!(text, bob.text_of(&block).expect("a body"));
    assert!(text.contains("one two three"), "{text}");
    assert!(text.contains("meanwhile"), "{text}");
}

#[test]
fn a_remote_change_is_never_re_broadcast() {
    let (mut alice, mut bob, block) = two_peers();
    let from_alice = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "hello".to_owned(),
        })
        .update
        .expect("a delta");
    // Applying it answers display events alone: nothing here can reach Sync
    // (`PROSE-049`).
    let events = bob.apply_remote(&from_alice);
    assert!(matches!(
        events.first(),
        Some(ProseEvent::DocUpdated { .. })
    ));
    // The next local command of Bob's exports his own operation and not
    // Alice's, because the import advanced his export cursor.
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 5,
            text: "!".to_owned(),
        })
        .update
        .expect("a delta");
    let mut third = ProseState::with_peer(3).expect("a document is built");
    third
        .import_snapshot(&alice.export_snapshot())
        .expect("the seed imports");
    third.apply_remote(&from_bob);
    assert_eq!(third.text_of(&block).as_deref(), Some("hello!"));
}

#[test]
fn the_confirmed_change_carries_the_bytes_that_were_applied() {
    let (mut alice, mut bob, block) = two_peers();
    let outcome = alice.handle_command(ProseCommand::InsertText {
        block_id: block,
        pos: 0,
        text: "carried".to_owned(),
    });
    let update = outcome.update.clone().expect("a delta");
    let confirmed = outcome
        .display
        .iter()
        .find_map(|e| match e {
            ProseEvent::DocUpdated { delta } => Some(delta.clone()),
            _ => None,
        })
        .expect("the confirmed change");
    // The event carries the exported bytes, never a re-export (`PROSE-023`).
    assert_eq!(confirmed, update);
    let echoed = bob.apply_remote(&update);
    let remote_confirmed = echoed
        .iter()
        .find_map(|e| match e {
            ProseEvent::DocUpdated { delta } => Some(delta.clone()),
            _ => None,
        })
        .expect("the confirmed change");
    // And on the remote path, the payload that arrived (`PROSE-048`).
    assert_eq!(remote_confirmed, update);
}

#[test]
fn a_split_and_a_concurrent_insertion_both_survive() {
    let (mut alice, mut bob, block) = two_peers();
    let seed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "hello world".to_owned(),
        })
        .update
        .expect("a delta");
    bob.apply_remote(&seed);

    let from_alice = alice
        .handle_command(ProseCommand::SplitBlock {
            block_id: block,
            pos: 5,
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "oh ".to_owned(),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert_eq!(rendered(&alice), rendered(&bob));
    assert_eq!(alice.root_blocks().len(), 2);
}

#[test]
fn a_join_and_a_concurrent_edit_converge() {
    let (mut alice, mut bob, first) = two_peers();
    let seeded = alice
        .handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::Paragraph,
            style_id: None,
        })
        .update
        .expect("a delta");
    bob.apply_remote(&seeded);
    let second = alice.root_blocks()[1];
    for (id, text) in [(first, "head"), (second, "tail")] {
        let delta = alice
            .handle_command(ProseCommand::InsertText {
                block_id: id,
                pos: 0,
                text: text.to_owned(),
            })
            .update
            .expect("a delta");
        bob.apply_remote(&delta);
    }

    let from_alice = alice
        .handle_command(ProseCommand::DeleteRange {
            anchor: (first, 4),
            focus: (second, 0),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: 4,
            text: "!".to_owned(),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert_eq!(rendered(&alice), rendered(&bob));
    assert_eq!(alice.root_blocks(), vec![first]);
}

/// Two peers appending to one list read the same ordinal on every item
/// (`PROSE-FEAT-137`).
///
/// Neither peer wrote a number, so there is none to merge. Both read the tree
/// the two of them converged on, and the walk over one order has one answer.
#[test]
fn two_peers_appending_to_one_list_draw_the_same_ordinals() {
    let (mut alice, mut bob, block) = two_peers();
    let append = |state: &mut ProseState| {
        state
            .handle_command(ProseCommand::InsertBlock {
                parent_id: None,
                after_id: Some(block),
                init: BlockInit::ListItem {
                    style: ListStyle::Ordered(OrderedStyle::Decimal),
                },
                style_id: None,
            })
            .update
            .expect("a delta")
    };
    let from_alice = append(&mut alice);
    let from_bob = append(&mut bob);
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    assert_eq!(ordinals(&alice), ordinals(&bob));
    // Both items landed, and they are numbered one after the other.
    assert_eq!(ordinals(&alice), vec![0, 1, 2]);
}

/// The ordinal of every live root block, in document order.
fn ordinals(state: &ProseState) -> Vec<u32> {
    let tree = crate::doc::blocks(state.document());
    state
        .root_blocks()
        .into_iter()
        .map(|id| crate::numbering::compute_ordinal(&tree, &crate::events::tree_id(id)))
        .collect()
}

/// Two overlapping applications of one mark converge on one span
/// (`PROSE-FEAT-008`).
#[test]
fn two_peers_bolding_overlapping_ranges_keep_one_run() {
    let (mut alice, mut bob, block) = two_peers();
    seed(&mut alice, &mut bob, block, "one two three four");
    let from_alice = bold(&mut alice, block, (4, 11));
    let from_bob = bold(&mut bob, block, (8, 18));
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let bolded = |state: &ProseState| -> String {
        state
            .resolved_runs(&block)
            .into_iter()
            .filter(|(_, style)| style.bold)
            .map(|(text, _)| text)
            .collect()
    };
    assert_eq!(bolded(&alice), bolded(&bob));
    // One continuous run over the union of the two ranges, with no seam.
    assert_eq!(bolded(&alice), "two three four");
}

/// Two peers holding both raised and lowered resolve one style, whichever
/// order the marks arrived in (`PROSE-FEAT-173`, `PROSE-FEAT-021`).
#[test]
fn two_peers_raising_and_lowering_one_word_resolve_one_style() {
    let (mut alice, mut bob, block) = two_peers();
    seed(&mut alice, &mut bob, block, "one two");
    let from_alice = mark(&mut alice, block, (0, 3), "sup", None);
    let from_bob = mark(&mut bob, block, (0, 3), "sub", None);
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let first = alice.resolved_runs(&block).remove(0).1;
    assert_eq!(first, bob.resolved_runs(&block).remove(0).1);
    // Neither write cleared the other, and the raised one wins.
    assert!(first.superscript);
    assert!(!first.subscript);
}

/// Two values of one key converge on one, and both peers resolve that one
/// (`PROSE-FEAT-184`).
#[test]
fn two_peers_setting_two_sizes_resolve_the_same_one() {
    let (mut alice, mut bob, block) = two_peers();
    seed(&mut alice, &mut bob, block, "one two");
    let from_alice = mark(&mut alice, block, (0, 3), "fs", Some("12"));
    let from_bob = mark(&mut bob, block, (0, 3), "fs", Some("14"));
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let size = alice.resolved_runs(&block).remove(0).1.font_size;
    assert_eq!(size, bob.resolved_runs(&block).remove(0).1.font_size);
    assert!(size == 12.0 || size == 14.0, "{size}");
}

/// Two authors setting two properties of one paragraph both keep theirs
/// (`PROSE-FEAT-176`).
#[test]
fn two_peers_setting_two_paragraph_properties_both_keep_theirs() {
    let (mut alice, mut bob, block) = two_peers();
    let from_alice = alice
        .handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: ParagraphStyle {
                space_before: Some(6.0),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::SetParagraphStyle {
            block_id: block,
            style: ParagraphStyle {
                align: Some(Align::Center),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    let style = alice.paragraph_style_of(&block);
    assert_eq!(style, bob.paragraph_style_of(&block));
    assert_eq!(style.space_before, Some(6.0));
    assert_eq!(style.align, Some(Align::Center));
}

/// One sentence in both peers, so a range means the same thing on each.
fn seed(alice: &mut ProseState, bob: &mut ProseState, block: crate::events::BlockId, text: &str) {
    let typed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: text.to_owned(),
        })
        .update
        .expect("a delta");
    bob.apply_remote(&typed);
}

/// One mark, and the delta it exported.
fn mark(
    state: &mut ProseState,
    block: crate::events::BlockId,
    range: (u32, u32),
    key: &str,
    value: Option<&str>,
) -> Vec<u8> {
    state
        .handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range,
            key: key.to_owned(),
            value: value.map(str::to_owned),
        })
        .update
        .expect("a delta")
}

/// The bold mark over one range, and the delta it exported.
fn bold(state: &mut ProseState, block: crate::events::BlockId, range: (u32, u32)) -> Vec<u8> {
    mark(state, block, range, "b", None)
}
