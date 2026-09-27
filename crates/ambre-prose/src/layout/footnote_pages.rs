//! Where the footnotes land on the page
//! (`features/ambre-spec-prose-pagination.md section 2`).
//!
//! Driven through the engine: a note is written by its command, and the
//! layout read back is the one the view is handed.

use ambre_core::Uuid;
use loro::TreeID;

use crate::events::{BlockId, BlockInit, ProseCommand, tree_id};
use crate::layout::pass::PageLayout;
use crate::layout::reflow::shipped_paginator;
use crate::logic::ProseState;
use crate::note;

/// A paragraph long enough to cross a page.
fn long_text() -> String {
    "The quick brown fox jumps over the lazy dog. ".repeat(160)
}

/// A document of `texts`, one paragraph each, measured on the shipped faces.
fn document(texts: &[String]) -> (ProseState, Vec<BlockId>) {
    let mut state = ProseState::with_peer(1).expect("a document");
    state.load_faces(&crate::layout::fonts::shipped_faces());
    let mut ids = state.root_blocks();
    for _ in 1..texts.len() {
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: ids.last().copied(),
            init: BlockInit::Paragraph,
            style_id: None,
        });
        ids = state.root_blocks();
    }
    for (id, text) in ids.iter().zip(texts) {
        state.handle_command(ProseCommand::InsertText {
            block_id: *id,
            pos: 0,
            text: text.clone(),
        });
    }
    (state, ids)
}

fn add(state: &mut ProseState, block: BlockId, pos: u32, text: &str) -> Uuid {
    let before: Vec<Uuid> = note::ordinals(state.document())
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    state.handle_command(ProseCommand::InsertFootnote {
        block_id: block,
        pos,
        note_text: text.to_owned(),
    });
    note::all(state.document())
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("a new note")
}

/// The page a line of a block was placed on.
fn page_of_line(layout: &PageLayout, block: &TreeID, line: u32) -> Option<u32> {
    let entry = layout
        .blocks
        .iter()
        .find(|entry| entry.block_id == *block)?;
    let carried = entry
        .split_points
        .iter()
        .filter(|point| **point <= line)
        .count();
    Some(entry.page_start + u32::try_from(carried).ok()?)
}

/// Every note the layout placed, by identity: its ordinal and its page.
fn placed(layout: &PageLayout) -> Vec<(Uuid, u32, u32)> {
    let mut out: Vec<(Uuid, u32, u32)> = (0..layout.page_count)
        .flat_map(|page| layout.footnotes_on_page(page))
        .map(|note| (note.footnote_id, note.ordinal, note.page))
        .collect();
    out.sort_by_key(|(_, ordinal, _)| *ordinal);
    out
}

/// No page's body text runs into the notes at its foot.
fn nothing_runs_into_the_notes(layout: &PageLayout) {
    let body = layout.body_height_px();
    for page in 0..layout.page_count {
        let foot: f32 = layout
            .footnotes_on_page(page)
            .iter()
            .map(|note| note.height_px)
            .sum();
        if foot <= 0.0 {
            continue;
        }
        let foot = foot + crate::layout::place::NOTE_SEPARATOR_PX;
        for entry in layout.blocks_on_page(page) {
            let bottom = if entry.page_start == page {
                entry.y_in_page + entry.height_px
            } else {
                let index = (page - entry.page_start - 1) as usize;
                entry
                    .continuation_heights
                    .get(index)
                    .copied()
                    .unwrap_or(0.0)
            };
            assert!(
                bottom <= body - foot + 0.01,
                "page {page}: text reaches {bottom}, the notes start at {}",
                body - foot
            );
        }
    }
}

#[test]
fn every_note_is_numbered_from_its_call_before_any_block_is_placed() {
    let (mut state, ids) = document(&["first".to_owned(), "second".to_owned()]);
    let later = add(&mut state, ids[1], 6, "later");
    let earlier = add(&mut state, ids[0], 5, "earlier");
    let layout = shipped_paginator().full(state.document()).clone();
    let ordinals: Vec<(Uuid, u32)> = placed(&layout).iter().map(|(id, n, _)| (*id, *n)).collect();
    assert_eq!(ordinals, vec![(earlier, 1), (later, 2)]);
    assert_eq!(ordinals, note::ordinals(state.document()));
}

#[test]
fn a_note_moved_to_another_page_keeps_its_ordinal() {
    let (mut state, ids) = document(&["intro".to_owned(), "body".to_owned()]);
    let first = add(&mut state, ids[0], 5, "one");
    let second = add(&mut state, ids[1], 4, "two");
    let before = placed(&shipped_paginator().full(state.document()).clone());
    assert_eq!(before, vec![(first, 1, 0), (second, 2, 0)]);
    state.handle_command(ProseCommand::InsertBlock {
        parent_id: None,
        after_id: Some(ids[0]),
        init: BlockInit::PageBreak,
        style_id: None,
    });
    let after = placed(&shipped_paginator().full(state.document()).clone());
    assert_eq!(after, vec![(first, 1, 0), (second, 2, 1)]);
}

#[test]
fn a_note_sits_on_the_page_of_the_line_that_calls_it() {
    let text = long_text();
    let end = u32::try_from(text.chars().count()).expect("a short text");
    let (mut state, ids) = document(&[text]);
    let early = add(&mut state, ids[0], 3, "early");
    let late = add(&mut state, ids[0], end, "late");
    let block = tree_id(ids[0]);
    let check = |state: &mut ProseState| {
        let layout = state.paginate(&[block]).clone();
        let entry = layout
            .blocks
            .iter()
            .find(|e| e.block_id == block)
            .expect("placed");
        assert!(entry.is_split, "the paragraph crosses a page");
        for (id, _, page) in placed(&layout) {
            let call = entry
                .notes
                .iter()
                .find(|n| n.footnote_id == id)
                .expect("the call's block carries the note");
            assert_eq!(Some(page), Some(call.page));
        }
        let pages: Vec<u32> = placed(&layout).iter().map(|(_, _, page)| *page).collect();
        let last_line = entry.split_points.last().copied().unwrap_or(0);
        assert_eq!(pages[0], page_of_line(&layout, &block, 0).expect("line 0"));
        assert_eq!(
            pages[1],
            page_of_line(&layout, &block, last_line).expect("a line")
        );
        assert!(pages[1] > pages[0]);
        nothing_runs_into_the_notes(&layout);
    };
    check(&mut state);
    assert_eq!(placed(state.paginate(&[]))[0].0, early);
    assert_eq!(placed(state.paginate(&[]))[1].0, late);
    // The calling paragraph moves down a page, and its notes follow it.
    state.handle_command(ProseCommand::InsertBlock {
        parent_id: None,
        after_id: Some(ids[0]),
        init: BlockInit::PageBreak,
        style_id: None,
    });
    let pause = state.root_blocks()[1];
    state.handle_command(ProseCommand::MoveBlock {
        block_id: ids[0],
        new_parent: None,
        after_id: Some(pause),
    });
    let moved = state.paginate(&[]).clone();
    let start = moved
        .blocks
        .iter()
        .find(|e| e.block_id == block)
        .map(|e| e.page_start);
    assert_eq!(
        start,
        Some(1),
        "the paragraph now starts on the second page"
    );
    check(&mut state);
}

#[test]
fn a_call_added_or_removed_lays_out_what_a_full_recomputation_would() {
    let (mut state, ids) = document(&["alpha".to_owned(), "beta".to_owned(), "gamma".to_owned()]);
    let _ = add(&mut state, ids[2], 5, "last");
    let _ = state.paginate(&[]);
    let first = add(&mut state, ids[0], 5, "first");
    let reflowed = state.paginate(&[tree_id(ids[0])]).clone();
    let full = shipped_paginator().full(state.document()).clone();
    assert_eq!(placed(&reflowed), placed(&full));
    assert_eq!(reflowed.blocks, full.blocks);
    state.handle_command(ProseCommand::DeleteFootnote { footnote_id: first });
    let reflowed = state.paginate(&[tree_id(ids[0])]).clone();
    let full = shipped_paginator().full(state.document()).clone();
    assert_eq!(placed(&reflowed), placed(&full));
    assert_eq!(placed(&reflowed)[0].1, 1, "the last note is note one again");
}

#[test]
fn text_typed_anywhere_lays_out_what_a_full_recomputation_would() {
    let (mut state, ids) = document(&[long_text(), "beta".to_owned(), long_text()]);
    let call = add(&mut state, ids[1], 2, "a note");
    let _ = state.paginate(&[]);
    let body = note::block(state.document(), &call).expect("a note block");
    let typed_in = [ids[0], ids[1], crate::events::block_id(body), ids[2]];
    for block in typed_in {
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: 0,
            text: "Typed at the head, long enough to push a line further on. ".repeat(3),
        });
        let reflowed = state.paginate(&[]).clone();
        let full = shipped_paginator().full(state.document()).clone();
        assert_eq!(placed(&reflowed), placed(&full), "{block:?}");
        assert_eq!(reflowed.blocks, full.blocks, "{block:?}");
    }
}
