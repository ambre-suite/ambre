//! Two peers, one document: the convergence belt of the Prose engine
//! (`1-Foundations/ambre-spec-testing.md`, "Reference model").
//!
//! Each test drives two `ProseState`s through their own commands, exchanges the
//! deltas both ways, and asserts on what the two hold afterwards. No worker, no
//! browser and no channel: a handler answers with a value, which is what makes
//! this shape reachable at all (`PROSE-024`).

use crate::events::HeadingLevel;
use crate::events::{
    Align, BlockInit, ListStyle, OrderedStyle, ParagraphStyle, ProseCommand, ProseEvent,
};
use crate::logic::ProseState;
use crate::logic::tables::tests::{cell_at, document_with_table, texts, type_in};
use crate::message::DocChar;
use ambre_core::Flag as _;
use ambre_core::index::{Delta, Pos, Span};
use ambre_core::{PrintLength, Pt};

/// Two peers over one seeded document, so both start from the same block ids.
///
/// The seed travels as a snapshot, which is how a second peer joins a document
/// in the live system.
fn two_peers() -> (ProseState, ProseState, crate::events::BlockId) {
    let alice = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
    let block = alice.root_blocks()[0];
    let seed = alice.export_snapshot();
    let mut bob = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
    bob.import_snapshot(&seed).expect("the seed imports");
    (alice, bob, block)
}

/// Hand each peer what the other wrote.
fn exchange(
    alice: &mut ProseState,
    bob: &mut ProseState,
    from_alice: &ambre_core::Payload<ambre_core::DeltaBytes>,
    from_bob: &ambre_core::Payload<ambre_core::DeltaBytes>,
) {
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
            pos: Pos::<DocChar>::ZERO,
            text: "Alice writes.".to_owned(),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
                space_after: Some(PrintLength::<Pt>::new(12.0)),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);

    for state in [&alice, &bob] {
        let style = state.paragraph_style_of(&block);
        assert_eq!(style.align, Some(Align::Center));
        assert_eq!(style.space_after, Some(PrintLength::<Pt>::new(12.0)));
    }
}

#[test]
fn two_peers_writing_one_size_merge_to_one_value() {
    let (mut alice, mut bob, block) = two_peers();
    let seed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "fs".into(),
            value: Some(crate::mark::test_value("11.0")),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::from_usize_range(0..5),
            key: "fs".into(),
            value: Some(crate::mark::test_value("11")),
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
            pos: Pos::<DocChar>::ZERO,
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
            init: BlockInit::Heading {
                level: HeadingLevel::H2,
            },
            style_id: None,
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(first),
            init: BlockInit::CodeBlock {
                lang: ambre_core::Validated::<crate::message::CodeLang>::parse("rust").ok(),
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
                    pos: alice.text_of(&block).map_or(Pos::<DocChar>::ZERO, |t| {
                        ambre_core::char_len(&t).as_axis::<DocChar>().as_end()
                    }),
                    text: text.to_owned(),
                })
                .update
                .expect("a delta"),
        );
    }
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
            pos: Pos::<DocChar>::ZERO,
            text: "hello".to_owned(),
        })
        .update
        .expect("a delta");
    // Applying it answers display events alone: nothing here can reach Sync
    // (`SYNC-101`).
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
            pos: Pos::<DocChar>::new(5),
            text: "!".to_owned(),
        })
        .update
        .expect("a delta");
    let mut third = ProseState::with_peer(ambre_core::PeerId::new(3)).expect("a document is built");
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
        pos: Pos::<DocChar>::ZERO,
        text: "carried".to_owned(),
    });
    let update = outcome.update.clone().expect("a delta");
    let confirmed = outcome
        .display
        .iter()
        .find_map(|e| match e {
            ProseEvent::DocUpdated { payload } => Some(payload.clone()),
            _ => None,
        })
        .expect("the confirmed change");
    // The event carries the exported bytes, never a re-export (`PROSE-023`).
    assert_eq!(ambre_core::LoroPayload::Delta(update.clone()), confirmed);
    let echoed = bob.apply_remote(&update);
    let remote_confirmed = echoed
        .iter()
        .find_map(|e| match e {
            ProseEvent::DocUpdated { payload } => Some(payload.clone()),
            _ => None,
        })
        .expect("the confirmed change");
    // And on the remote path, the payload that arrived (`PROSE-048`).
    assert_eq!(ambre_core::LoroPayload::Delta(update), remote_confirmed);
}

#[test]
fn a_split_and_a_concurrent_insertion_both_survive() {
    let (mut alice, mut bob, block) = two_peers();
    let seed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "hello world".to_owned(),
        })
        .update
        .expect("a delta");
    bob.apply_remote(&seed);

    let from_alice = alice
        .handle_command(ProseCommand::SplitBlock {
            block_id: block,
            pos: Pos::<DocChar>::new(5),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
                pos: Pos::<DocChar>::ZERO,
                text: text.to_owned(),
            })
            .update
            .expect("a delta");
        bob.apply_remote(&delta);
    }

    let from_alice = alice
        .handle_command(ProseCommand::DeleteRange {
            anchor: (first, Pos::<DocChar>::new(4)),
            focus: (second, Pos::<DocChar>::ZERO),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: Pos::<DocChar>::new(4),
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
    assert_eq!(ordinals(&alice), vec![None, Some(1), Some(2)]);
}

/// The ordinal of every live root block, in document order.
fn ordinals(state: &ProseState) -> Vec<Option<u32>> {
    let tree = crate::doc::blocks(state.document());
    state
        .root_blocks()
        .into_iter()
        .map(|id| {
            crate::numbering::compute_ordinal(&tree, &crate::events::tree_id(id))
                .map(ambre_core::Ordinal::<crate::numbering::ListItem>::get)
        })
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
            .filter(|(_, style)| style.bold.as_bool())
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
    assert_eq!(first.script, crate::mark::Script::Raised);
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
    assert!(size.points() == 12.0 || size.points() == 14.0, "{size}");
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
                space_before: Some(PrintLength::<Pt>::new(6.0)),
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
    assert_eq!(style.space_before, Some(PrintLength::<Pt>::new(6.0)));
    assert_eq!(style.align, Some(Align::Center));
}

/// One sentence in both peers, so a range means the same thing on each.
fn seed(alice: &mut ProseState, bob: &mut ProseState, block: crate::events::BlockId, text: &str) {
    let typed = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
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
) -> ambre_core::Payload<ambre_core::DeltaBytes> {
    state
        .handle_command(ProseCommand::ApplyMark {
            block_id: block,
            range: Span::<DocChar>::new(Pos::<DocChar>::new(range.0), Pos::<DocChar>::new(range.1)),
            key: key.into(),
            value: value.map(crate::mark::test_value),
        })
        .update
        .expect("a delta")
}

/// The bold mark over one range, and the delta it exported.
fn bold(
    state: &mut ProseState,
    block: crate::events::BlockId,
    range: (u32, u32),
) -> ambre_core::Payload<ambre_core::DeltaBytes> {
    mark(state, block, range, "b", None)
}

// -- tables (`features/ambre-spec-prose-tables.md`) ----------------------------

/// Two peers over one document holding a table of `rows` by `cols`.
fn two_peers_with_table(rows: u32, cols: u32) -> (ProseState, ProseState, loro::TreeID) {
    let (alice, table) = document_with_table(ambre_core::PeerId::new(1), rows, cols);
    let seed = alice.export_snapshot();
    let mut bob = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
    bob.import_snapshot(&seed).expect("the seed imports");
    (alice, bob, table)
}

fn merge_cells(
    state: &mut ProseState,
    table: loro::TreeID,
    start: (u32, u32),
    end: (u32, u32),
) -> ambre_core::Payload<ambre_core::DeltaBytes> {
    state
        .handle_command(ProseCommand::MergeTableCells {
            table_id: crate::events::block_id(table),
            start: crate::logic::tables::tests::pos(start),
            end: crate::logic::tables::tests::pos(end),
        })
        .update
        .expect("a delta")
}

/// The row count of a table, and the rectangle of every region that stands.
fn grid_and_regions(
    state: &ProseState,
    table: loro::TreeID,
) -> (usize, Vec<(usize, usize, usize, usize)>) {
    let rows = state.grid_of(&table).map_or(0, |grid| grid.rows.len());
    let rects = state
        .merged_rects(&table)
        .into_iter()
        .map(|(_, rect)| {
            let (top_left, bottom_right) = (rect.top_left, rect.bottom_right);
            (
                top_left.row.to_usize(),
                top_left.col.to_usize(),
                bottom_right.row.to_usize(),
                bottom_right.col.to_usize(),
            )
        })
        .collect();
    (rows, rects)
}

#[test]
fn a_row_insert_and_a_concurrent_edit_in_the_table_converge() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 2);
    let first_row = alice.grid_of(&table).expect("a table").rows[0];
    let from_alice = alice
        .handle_command(ProseCommand::InsertTableRow {
            table_id: crate::events::block_id(table),
            after_row: Some(crate::events::block_id(first_row)),
        })
        .update
        .expect("a delta");
    let cell = cell_at(&bob, table, 1, 1);
    let from_bob = type_in(&mut bob, cell, "Bob's cell").expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    assert_eq!(texts(&alice, table), texts(&bob, table));
    // The row landed between the two, and the text moved down with its cell.
    assert_eq!(texts(&alice, table).len(), 3);
    assert_eq!(texts(&alice, table)[2][1], "Bob's cell");
}

#[test]
fn a_column_insert_and_a_concurrent_edit_in_the_table_converge() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 2);
    let from_alice = alice
        .handle_command(ProseCommand::InsertTableCol {
            table_id: crate::events::block_id(table),
            at_col: Pos::<crate::events::grid::Col>::new(1),
            width: None,
        })
        .update
        .expect("a delta");
    let cell = cell_at(&bob, table, 0, 1);
    let from_bob = type_in(&mut bob, cell, "moved right").expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    assert_eq!(texts(&alice, table), texts(&bob, table));
    assert_eq!(texts(&alice, table)[0], ["", "", "moved right"]);
    assert_eq!(alice.column_widths(&table), bob.column_widths(&table));
    assert_eq!(alice.column_widths(&table).len(), 3);
}

#[test]
fn a_row_inserted_between_the_corners_widens_the_region_on_both_peers() {
    let (mut alice, mut bob, table) = two_peers_with_table(3, 3);
    let merged = merge_cells(&mut alice, table, (0, 0), (1, 1));
    bob.apply_remote(&merged);
    let first_row = alice.grid_of(&table).expect("a table").rows[0];
    let from_alice = alice
        .handle_command(ProseCommand::InsertTableRow {
            table_id: crate::events::block_id(table),
            after_row: Some(crate::events::block_id(first_row)),
        })
        .update
        .expect("a delta");
    let anchor = cell_at(&bob, table, 0, 0);
    let from_bob = type_in(&mut bob, anchor, "anchor text").expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    let seen = grid_and_regions(&alice, table);
    assert_eq!(seen, grid_and_regions(&bob, table));
    // One region, on the same two corners, now three rows tall.
    assert_eq!(seen, (4, vec![(0, 0, 2, 1)]));
    assert_eq!(alice.regions_of(&table), bob.regions_of(&table));
}

#[test]
fn a_column_inserted_between_the_corners_widens_the_region_on_both_peers() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 3);
    let merged = merge_cells(&mut alice, table, (0, 0), (1, 1));
    bob.apply_remote(&merged);
    let from_alice = alice
        .handle_command(ProseCommand::InsertTableCol {
            table_id: crate::events::block_id(table),
            at_col: Pos::<crate::events::grid::Col>::new(1),
            width: None,
        })
        .update
        .expect("a delta");
    let anchor = cell_at(&bob, table, 0, 0);
    let from_bob = type_in(&mut bob, anchor, "anchor text").expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    let seen = grid_and_regions(&alice, table);
    assert_eq!(seen, grid_and_regions(&bob, table));
    assert_eq!(seen, (2, vec![(0, 0, 1, 2)]));
}

#[test]
fn two_merges_over_one_cell_at_once_resolve_to_one_region_on_both_peers() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 2);
    let from_alice = merge_cells(&mut alice, table, (0, 0), (0, 1));
    let from_bob = merge_cells(&mut bob, table, (0, 1), (1, 1));
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    let seen = grid_and_regions(&alice, table);
    assert_eq!(seen, grid_and_regions(&bob, table));
    assert_eq!(seen.1.len(), 1);
    // Each peer dropped the losing record on its own, and the two drops
    // agree: the stored map holds the same single entry on both.
    assert_eq!(alice.regions_of(&table), bob.regions_of(&table));
    assert_eq!(alice.regions_of(&table).len(), 1);
}

#[test]
fn a_merge_over_text_typed_at_once_masks_it_and_reports_it() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 2);
    let from_alice = merge_cells(&mut alice, table, (0, 0), (0, 1));
    let covered = cell_at(&bob, table, 0, 1);
    let from_bob = type_in(&mut bob, covered, "typed meanwhile").expect("a delta");
    let seen_by_bob = bob.apply_remote(&from_alice);
    let seen_by_alice = alice.apply_remote(&from_bob);
    for events in [&seen_by_alice, &seen_by_bob] {
        // The change is applied whatever the pass finds, and the masked cell
        // is named (`PROSE-FEAT-255`, `PROSE-FEAT-256`).
        assert!(matches!(
            events.first(),
            Some(ProseEvent::DocUpdated { .. })
        ));
        let reported = events.iter().any(|event| {
            matches!(event, ProseEvent::MergeConflictDetected { cell_id }
                if *cell_id == crate::events::block_id(covered))
        });
        assert!(reported, "{events:?}");
    }
    // The region stands and the text under it survives, on both peers.
    assert_eq!(grid_and_regions(&alice, table), (2, vec![(0, 0, 0, 1)]));
    assert_eq!(
        grid_and_regions(&alice, table),
        grid_and_regions(&bob, table)
    );
    assert_eq!(texts(&alice, table)[0][1], "typed meanwhile");
    assert_eq!(texts(&alice, table), texts(&bob, table));
}

#[test]
fn unmerging_brings_a_masked_text_back_verbatim() {
    let (mut alice, mut bob, table) = two_peers_with_table(2, 2);
    let from_alice = merge_cells(&mut alice, table, (0, 0), (0, 1));
    let covered = cell_at(&bob, table, 0, 1);
    let from_bob = type_in(&mut bob, covered, "Kept, verbatim.").expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    let anchor = cell_at(&alice, table, 0, 0);
    alice.handle_command(ProseCommand::UnmergeTableCell {
        cell_id: crate::events::block_id(anchor),
    });
    assert!(alice.merged_rects(&table).is_empty());
    assert_eq!(texts(&alice, table)[0][1], "Kept, verbatim.");
}

/// What a peer's change did to the text, as the view is told it.
fn moved(
    events: &[ProseEvent],
) -> Option<(
    crate::events::ChangeOrigin,
    Vec<crate::events::TextEdit>,
    Vec<crate::events::BlockId>,
)> {
    events.iter().find_map(|event| match event {
        ProseEvent::TextChanged {
            origin,
            edits,
            removed,
            ..
        } => Some((*origin, edits.clone(), removed.clone())),
        _ => None,
    })
}

/// `PROSE-UI-028`: the view is told each run a change inserted or deleted, in
/// code points of the text the runs before it left. It also learns whether it was its own.
#[test]
fn a_change_tells_the_view_what_it_did_to_the_text() {
    let (mut alice, mut bob, block) = two_peers();
    let typed = alice.handle_command(ProseCommand::InsertText {
        block_id: block,
        pos: Pos::<DocChar>::ZERO,
        text: "\u{e9}t\u{e9}".to_owned(),
    });
    let (origin, edits, removed) = moved(&typed.display).expect("the text moved");
    assert_eq!(
        origin,
        crate::events::ChangeOrigin::Local,
        "the command is this peer's own"
    );
    assert_eq!(edits.len(), 1);
    assert_eq!(
        (edits[0].block_id, edits[0].at, edits[0].delta),
        (block, Pos::<DocChar>::ZERO, Delta::<DocChar>::new(3))
    );
    assert!(removed.is_empty());

    let answered = bob.apply_remote(&typed.update.expect("a delta"));
    let (origin, edits, _) = moved(&answered).expect("the text moved");
    assert_eq!(
        origin,
        crate::events::ChangeOrigin::Remote,
        "the change came from a peer"
    );
    assert_eq!(
        (edits[0].at, edits[0].delta),
        (Pos::<DocChar>::ZERO, Delta::<DocChar>::new(3))
    );

    // A deletion after the accented run counts code points, not bytes.
    let cut = alice.handle_command(ProseCommand::DeleteText {
        block_id: block,
        range: Span::<DocChar>::from_usize_range(2..3),
    });
    let answered = bob.apply_remote(&cut.update.expect("a delta"));
    let (_, edits, _) = moved(&answered).expect("the text moved");
    assert_eq!(
        (edits[0].at, edits[0].delta),
        (Pos::<DocChar>::new(2), Delta::<DocChar>::new(-1))
    );

    let gone = alice.handle_command(ProseCommand::DeleteBlock { block_id: block });
    let answered = bob.apply_remote(&gone.update.expect("a delta"));
    let (_, _, removed) = moved(&answered).expect("a block went");
    assert!(removed.contains(&block), "the deleted block is named");
}

#[test]
fn an_echo_of_this_peers_own_change_answers_nothing() {
    let (mut alice, _bob, block) = two_peers();
    let own = alice
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "hello".to_owned(),
        })
        .update
        .expect("a delta");
    // The relay hands the writer its own change back.
    assert!(alice.apply_remote(&own).is_empty());
}

/// A landscape page with margins of `margin` millimetres.
fn landscape(margin: f32) -> crate::doc::PageFormat {
    use ambre_core::Mm;
    crate::doc::PageFormat {
        width: PrintLength::<Mm>::new(297.0),
        height: PrintLength::<Mm>::new(210.0),
        margin_top: PrintLength::<Mm>::new(margin),
        margin_bottom: PrintLength::<Mm>::new(margin),
        margin_left: PrintLength::<Mm>::new(margin),
        margin_right: PrintLength::<Mm>::new(margin),
    }
}

/// Two peers change the page at once, one of them further along its own
/// history. Both end on one page, whose orientation agrees with its
/// dimensions (`PROSE-145`).
#[test]
fn two_peers_setting_the_page_at_once_converge_on_one_agreeing_page() {
    use ambre_core::Mm;
    let (mut alice, mut bob, block) = two_peers();
    // Bob writes first, so his clock runs ahead of Alice's.
    let typed = bob
        .handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: Pos::<DocChar>::ZERO,
            text: "Ahead".to_owned(),
        })
        .update
        .expect("a delta");
    let tall = crate::doc::PageFormat {
        width: PrintLength::<Mm>::new(200.0),
        height: PrintLength::<Mm>::new(320.0),
        ..crate::doc::PageFormat::default()
    };
    let from_alice = alice
        .handle_command(ProseCommand::SetPageFormat {
            format: landscape(15.0),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::SetPageFormat { format: tall })
        .update
        .expect("a delta");
    alice.apply_remote(&typed);
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    let (a, b) = (alice.page_format(), bob.page_format());
    assert_eq!(a, b, "both peers hold one page");
    for state in [&alice, &bob] {
        let orientation = state.page_orientation().expect("an orientation is stored");
        assert!(
            orientation.agrees_with(a.width, a.height),
            "{orientation:?} for {a:?}"
        );
    }
}

/// One end of a selection: a block and an offset in it.
type Spot = (crate::events::BlockId, Pos<DocChar>);

/// Two peers, the two blocks they share, and the selection over both.
type TwoParagraphs = (
    ProseState,
    ProseState,
    (crate::events::BlockId, crate::events::BlockId),
    (Spot, Spot),
);

/// Two peers over one seeded document of two paragraphs, "hello" and
/// "world", and the selection over both.
fn two_peers_two_paragraphs() -> TwoParagraphs {
    let mut alice = ProseState::with_peer(ambre_core::PeerId::new(1)).expect("a document is built");
    let (first, second, anchor, focus) = crate::logic::selection::tests::two_paragraphs(&mut alice);
    let seed = alice.export_snapshot();
    let mut bob = ProseState::with_peer(ambre_core::PeerId::new(2)).expect("a document is built");
    bob.import_snapshot(&seed).expect("the seed imports");
    (alice, bob, (first, second), (anchor, focus))
}

/// One value mark over a selection, and the delta it exported.
fn mark_selection(
    state: &mut ProseState,
    (anchor, focus): (Spot, Spot),
    key: crate::mark::MarkRoot,
    raw: &str,
) -> ambre_core::Payload<ambre_core::DeltaBytes> {
    state
        .handle_command(ProseCommand::MarkSelection {
            anchor,
            focus,
            key: key.key(),
            value: crate::mark::validate_value(&key.key(), Some(raw)).ok(),
        })
        .update
        .expect("a delta")
}

/// Two peers setting two fonts over overlapping selections of two blocks
/// read one font on each character, the same on both peers
/// (`PROSE-FEAT-337`).
#[test]
fn two_peers_setting_two_fonts_over_overlapping_selections_converge() {
    let (mut alice, mut bob, (first, second), whole) = two_peers_two_paragraphs();
    let from_alice = mark_selection(&mut alice, whole, crate::mark::MarkRoot::FontFamily, "Lora");
    let tail = ((first, Pos::<DocChar>::new(3)), whole.1);
    let from_bob = mark_selection(&mut bob, tail, crate::mark::MarkRoot::FontFamily, "Inter");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    for block in [first, second] {
        assert_eq!(alice.resolved_runs(&block), bob.resolved_runs(&block));
    }
    let read =
        |state: &mut ProseState| crate::logic::selection::tests::values(state, whole.0, whole.1);
    assert_eq!(read(&mut alice), read(&mut bob));
    assert_eq!(rendered(&alice), rendered(&bob));
}

/// A font over a selection while the other peer types inside it: both
/// peers hold the same text and the same runs (`PROSE-FEAT-337`).
#[test]
fn a_font_over_a_selection_and_typing_inside_it_converge() {
    let (mut alice, mut bob, (first, second), whole) = two_peers_two_paragraphs();
    let from_alice = mark_selection(&mut alice, whole, crate::mark::MarkRoot::Ink, "#2f6fd6");
    let from_bob = bob
        .handle_command(ProseCommand::InsertText {
            block_id: second,
            pos: Pos::<DocChar>::new(2),
            text: "XY".to_owned(),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    assert_eq!(rendered(&alice), rendered(&bob));
    for block in [first, second] {
        assert_eq!(alice.resolved_runs(&block), bob.resolved_runs(&block));
    }
    assert_eq!(alice.text_of(&second).as_deref(), Some("woXYrld"));
}

/// One peer makes a selection a heading while the other makes it a list.
/// Both peers hold one kind per block, the same one, and a declared tree
/// (`PROSE-FEAT-338`).
#[test]
fn two_peers_making_one_selection_a_heading_and_a_list_converge() {
    let (mut alice, mut bob, (first, second), (anchor, focus)) = two_peers_two_paragraphs();
    let from_alice = alice
        .handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: crate::message::ParagraphKind::Heading(HeadingLevel::H2),
        })
        .update
        .expect("a delta");
    let from_bob = bob
        .handle_command(ProseCommand::SetSelectionKind {
            anchor,
            focus,
            kind: crate::message::ParagraphKind::List(ListStyle::Bullet),
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    for block in [first, second] {
        assert_eq!(alice.block_type_of(&block), bob.block_type_of(&block));
        assert_eq!(alice.heading_level_of(&block), bob.heading_level_of(&block));
        assert_eq!(alice.list_style_of(&block), bob.list_style_of(&block));
    }
    let read =
        |state: &mut ProseState| crate::logic::selection::tests::values(state, anchor, focus);
    assert_eq!(read(&mut alice).kind, read(&mut bob).kind);
    assert!(alice.parentages_are_declared() && bob.parentages_are_declared());
    assert_eq!(rendered(&alice), rendered(&bob));
}

/// One peer aligns a selection while the other sets its line spacing:
/// both properties land on every block, on both peers (`PROSE-FEAT-338`).
#[test]
fn two_peers_setting_two_paragraph_properties_of_one_selection_keep_both() {
    let (mut alice, mut bob, (first, second), (anchor, focus)) = two_peers_two_paragraphs();
    let from_alice = alice
        .handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: ParagraphStyle {
                align: Some(Align::Justify),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    let double =
        crate::events::LineSpacingMultiple::new(2.0).map(crate::events::LineHeight::Multiple);
    let from_bob = bob
        .handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: ParagraphStyle {
                line_height: double,
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    for block in [first, second] {
        let style = alice.paragraph_style_of(&block);
        assert_eq!(style, bob.paragraph_style_of(&block));
        assert_eq!(style.align, Some(Align::Justify));
        assert_eq!(style.line_height, double);
    }
}

/// Two peers setting two body fonts at once converge on one, and a body
/// font keeps a paragraph property set beside it (`PROSE-FEAT-338`).
#[test]
fn two_peers_setting_the_body_font_converge() {
    let (mut alice, mut bob, (first, _), (anchor, focus)) = two_peers_two_paragraphs();
    let set = |state: &mut ProseState, label: &str| {
        state
            .handle_command(ProseCommand::SetDefaultFont {
                font: crate::message::DefaultFont::Body(ambre_core::FontFamily::from_label(label)),
            })
            .update
            .expect("a delta")
    };
    let from_alice = set(&mut alice, "Lora");
    let from_bob = set(&mut bob, "Noto Serif");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    assert_eq!(alice.defaults(), bob.defaults());
    assert_eq!(alice.resolved_runs(&first), bob.resolved_runs(&first));

    let from_alice = set(&mut alice, "Inter");
    let from_bob = bob
        .handle_command(ProseCommand::SetSelectionParagraph {
            anchor,
            focus,
            style: ParagraphStyle {
                align: Some(Align::Right),
                ..ParagraphStyle::default()
            },
        })
        .update
        .expect("a delta");
    exchange(&mut alice, &mut bob, &from_alice, &from_bob);
    assert_eq!(
        alice.defaults().font_family,
        ambre_core::FontFamily::from_label("Inter")
    );
    assert_eq!(alice.defaults(), bob.defaults());
    assert_eq!(alice.paragraph_style_of(&first).align, Some(Align::Right));
}
