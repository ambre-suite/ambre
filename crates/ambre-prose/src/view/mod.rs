//! The pages on the canvas: the column, the layout in force, and what the view
//! keeps to paint it (`features/ambre-spec-prose-page-rendering.md`,
//! `features/ambre-spec-prose-viewport.md`).
//!
//! Everything here runs on the UI thread and reads what the Logic thread
//! handed over. A layout arrives shaped (`PROSE-UI-006`), and nothing in this
//! module shapes a block, reads the document or sends a command.

pub mod annotate;
pub mod cache;
pub mod caret;
pub mod column;
pub mod hit;
pub mod paint;
pub mod slash;
pub mod toolbar;

use std::collections::HashMap;

use loro::TreeID;

use crate::events::{BlockId, TocEntry, tree_id};
use crate::layout::run::ShapedLayout;
use crate::ui::ProseUiState;

/// What an inline highlight marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightKind {
    Comment,
    Insertion,
    Deletion,
    Find,
}

/// One inline highlight, in its block's own code-point offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub kind: HighlightKind,
    pub start: u32,
    pub end: u32,
}

impl ProseUiState {
    /// Take a layout the Logic thread computed.
    ///
    /// A layout older than the one in force is dropped, so a slow
    /// recomputation never repaints over a newer one (`PROSE-UI-016`). A newer
    /// one replaces it, and its version becomes the view's (`PROSE-UI-121`).
    /// The column is rebuilt from it (`PROSE-UI-131`) and the offset clamped
    /// to the new column (`PROSE-UI-025`). The zoom stays (`PROSE-UI-132`),
    /// and nothing is shaped: the runs came with the layout (`PROSE-UI-122`).
    ///
    /// Answers whether the layout was taken.
    pub fn apply_layout(&mut self, shaped: ShapedLayout) -> bool {
        if shaped.version <= self.layout_version {
            return false;
        }
        self.layout_version = shaped.version;
        self.column = column::PageColumn::of(&shaped.layout);
        self.order = shaped
            .layout
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.block_id, index))
            .collect();
        let mut page_blocks = vec![Vec::new(); shaped.layout.page_count as usize];
        for (index, block) in shaped.layout.blocks.iter().enumerate() {
            for page in block.page_span() {
                if let Some(blocks) = page_blocks.get_mut(page as usize) {
                    blocks.push(index);
                }
            }
        }
        self.page_blocks = page_blocks;
        self.shaped = shaped;
        self.clamp_scroll();
        // What the draft confirmed is in these runs now.
        self.draft.laid();
        self.settle_awaited_block();
        true
    }

    /// Scroll by a distance on the canvas, in screen pixels, down for a
    /// positive one.
    ///
    /// The column moves by what the gesture asks, up to its ends
    /// (`PROSE-UI-126`). The zoom and the column stay as they were
    /// (`PROSE-UI-127`).
    pub fn scroll_by(&mut self, dy_px: f32) {
        let moved = self.viewport.scroll_y + dy_px / self.viewport.scale();
        self.viewport.scroll_y = self.column.clamp_scroll(&self.viewport, moved);
    }

    /// The canvas took a new size. The offset and the zoom stay
    /// (`PROSE-UI-130`): what was at the top of the canvas still is.
    pub fn resize(&mut self, width_px: f32, height_px: f32) {
        self.viewport.width_px = width_px.max(0.0);
        self.viewport.height_px = height_px.max(0.0);
    }

    /// Bring the heading a table-of-contents entry reads to the top of the
    /// canvas (`PROSE-UI-082`).
    ///
    /// Where this reader looks is theirs alone: the document is not touched
    /// and nothing is sent (`PROSE-UI-083`). An entry whose heading is not in
    /// the layout in force moves nothing.
    pub fn follow_toc_entry(&mut self, entry: &TocEntry) {
        self.reveal_block(entry.block_id);
    }

    /// Scroll so a block's top shows at the top of the canvas.
    pub(crate) fn reveal_block(&mut self, block: BlockId) {
        let Some(layout) = self.block_layout(block) else {
            return;
        };
        let top = self.column.page_top_y(layout.page_start)
            + self.shaped.layout.body_top_px()
            + layout.y_in_page;
        self.viewport.scroll_y = self.column.clamp_scroll(&self.viewport, top);
    }

    /// Where a block sits in the layout in force.
    fn block_layout(&self, block: BlockId) -> Option<&crate::layout::place::BlockLayout> {
        let index = *self.order.get(&tree_id(block))?;
        self.shaped.layout.blocks.get(index)
    }

    /// The position of a block in the flow of the layout in force.
    pub(crate) fn flow_index(&self, block: &TreeID) -> Option<usize> {
        self.order.get(block).copied()
    }

    fn clamp_scroll(&mut self) {
        self.viewport.scroll_y = self
            .column
            .clamp_scroll(&self.viewport, self.viewport.scroll_y);
    }

    /// Index every inline highlight by the block it sits in, so a paint reads
    /// the highlights of the blocks it draws and no other (`PROSE-UI-020`).
    pub(crate) fn index_highlights(&mut self) {
        let mut marks: HashMap<BlockId, Vec<Highlight>> = HashMap::new();
        let mut push = |block: BlockId, kind: HighlightKind, start: u32, end: u32| {
            if start < end {
                marks
                    .entry(block)
                    .or_default()
                    .push(Highlight { kind, start, end });
            }
        };
        for thread in &self.comments {
            if let Some(block) = thread.anchor_block
                && !thread.orphaned
                && !thread.resolved
            {
                push(
                    block,
                    HighlightKind::Comment,
                    thread.anchor_start,
                    thread.anchor_end,
                );
            }
        }
        for suggestion in &self.suggestions {
            let kind = match suggestion.kind {
                crate::events::SuggestionKind::Insertion => HighlightKind::Insertion,
                crate::events::SuggestionKind::Deletion => HighlightKind::Deletion,
            };
            for span in &suggestion.spans {
                push(span.block_id, kind, span.start, span.end);
            }
        }
        for found in &self.find_matches {
            push(
                found.block_id,
                HighlightKind::Find,
                found.range.0,
                found.range.1,
            );
        }
        self.highlights = marks;
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::events::block_id;
    use crate::events::{BlockInit, ProseCommand};
    use crate::layout::pass::PageLayout;
    use crate::logic::ProseState;

    /// Append one block at the root, after the last one, holding `text`.
    pub(crate) fn append(state: &mut ProseState, init: BlockInit, text: &str) -> BlockId {
        let last = *state.root_blocks().last().expect("a root block");
        state.handle_command(ProseCommand::InsertBlock {
            parent_id: None,
            after_id: Some(last),
            init,
            style_id: None,
        });
        let id = *state.root_blocks().last().expect("the block was inserted");
        if !text.is_empty() {
            state.handle_command(ProseCommand::InsertText {
                block_id: id,
                pos: 0,
                text: text.to_owned(),
            });
        }
        id
    }

    /// A document of `paragraphs` paragraphs of `words` words each, laid out
    /// on the faces the application ships.
    pub(crate) fn laid_out(paragraphs: usize, words: usize) -> (ProseState, ShapedLayout) {
        let mut state = ProseState::with_peer(1).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let text = vec!["word"; words].join(" ");
        for _ in 0..paragraphs {
            append(&mut state, BlockInit::Paragraph, &text);
        }
        let shaped = state.shaped_layout();
        (state, shaped)
    }

    fn a_layout(version: u64, pages: u32) -> ShapedLayout {
        ShapedLayout {
            version,
            layout: Arc::new(PageLayout {
                page_width_px: 800.0,
                page_height_px: 1000.0,
                page_count: pages,
                ..PageLayout::default()
            }),
            ..ShapedLayout::default()
        }
    }

    fn a_view() -> ProseUiState {
        let mut ui = ProseUiState::default();
        ui.resize(1200.0, 900.0);
        ui
    }

    #[test]
    fn a_newer_layout_replaces_the_one_in_force_and_rebuilds_the_column() {
        let mut ui = a_view();
        assert!(ui.apply_layout(a_layout(1, 3)));
        assert!(ui.apply_layout(a_layout(2, 7)));
        assert_eq!(ui.layout_version, 2);
        assert_eq!(ui.shaped.layout.page_count, 7);
        assert_eq!(ui.column, column::PageColumn::of(&ui.shaped.layout));
        assert_eq!(ui.column.page_count, 7);
    }

    #[test]
    fn a_late_layout_is_dropped_and_the_newer_one_stays() {
        let mut ui = a_view();
        assert!(ui.apply_layout(a_layout(5, 9)));
        assert!(!ui.apply_layout(a_layout(4, 2)));
        assert!(!ui.apply_layout(a_layout(5, 2)));
        assert_eq!(ui.layout_version, 5);
        assert_eq!(ui.shaped.layout.page_count, 9);
    }

    #[test]
    fn applying_a_layout_keeps_the_zoom_shapes_nothing_and_writes_nothing() {
        let (logic, shaped) = laid_out(40, 60);
        let before = logic.export_snapshot();
        let shaped_before = logic.shaped_count();
        let mut ui = a_view();
        ui.viewport.zoom = 1.5;
        assert!(ui.apply_layout(shaped));
        assert_eq!(ui.viewport.zoom, 1.5);
        assert_eq!(logic.shaped_count(), shaped_before);
        assert_eq!(logic.export_snapshot(), before);
        // The view holds the runs it was handed and built none of them.
        assert_eq!(ui.runs.built(), 0);
    }

    #[test]
    fn a_shorter_layout_brings_the_offset_back_inside_the_column() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 30));
        ui.scroll_by(1.0e9);
        assert_eq!(ui.viewport.scroll_y, ui.column.max_scroll(&ui.viewport));
        ui.apply_layout(a_layout(2, 2));
        assert_eq!(ui.viewport.scroll_y, ui.column.max_scroll(&ui.viewport));
        ui.apply_layout(a_layout(3, 0));
        assert_eq!(ui.viewport.scroll_y, 0.0);
    }

    #[test]
    fn a_scroll_moves_by_the_gesture_up_to_the_ends_and_nothing_else() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 10));
        ui.viewport.zoom = 2.0;
        let (column, version) = (ui.column, ui.layout_version);
        ui.scroll_by(300.0);
        assert_eq!(ui.viewport.scroll_y, 150.0);
        ui.scroll_by(-100.0);
        assert_eq!(ui.viewport.scroll_y, 100.0);
        ui.scroll_by(-1.0e6);
        assert_eq!(ui.viewport.scroll_y, 0.0);
        assert_eq!(ui.viewport.zoom, 2.0);
        assert_eq!(ui.column, column);
        assert_eq!(ui.layout_version, version);
    }

    #[test]
    fn a_resize_keeps_the_offset_and_the_zoom() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 10));
        ui.viewport.zoom = 0.8;
        ui.scroll_by(2000.0);
        let scroll_y = ui.viewport.scroll_y;
        ui.resize(300.0, 4000.0);
        assert_eq!(ui.viewport.scroll_y, scroll_y);
        assert_eq!(ui.viewport.zoom, 0.8);
    }

    #[test]
    fn following_an_entry_brings_its_heading_to_the_top_and_sends_nothing() {
        let (logic, shaped) = laid_out(60, 60);
        let before = logic.export_snapshot();
        let mut ui = a_view();
        ui.apply_layout(shaped);
        let far = ui.shaped.layout.blocks.last().expect("a block").clone();
        assert!(far.page_start > 0);
        let entry = TocEntry {
            block_id: block_id(far.block_id),
            level: 1,
            text: String::new(),
            section_number: None,
            page: far.page_start,
        };
        ui.follow_toc_entry(&entry);
        let top =
            ui.column.page_top_y(far.page_start) + ui.shaped.layout.body_top_px() + far.y_in_page;
        assert_eq!(
            ui.viewport.scroll_y,
            ui.column.clamp_scroll(&ui.viewport, top)
        );
        let visible = ui.column.visible_pages(&ui.viewport).expect("pages");
        assert!(visible.contains(&far.page_start));
        assert_eq!(logic.export_snapshot(), before);
    }

    #[test]
    fn a_block_not_edited_hands_the_view_the_run_it_was_measured_into() {
        let (mut state, first) = laid_out(4, 30);
        let edited = *state.root_blocks().last().expect("a block");
        state.handle_command(ProseCommand::InsertText {
            block_id: edited,
            pos: 0,
            text: "more ".to_owned(),
        });
        // No caller names the block: the document reports what it was written.
        let second = state.shaped_layout();
        let edited = crate::events::tree_id(edited);
        for (id, run) in &first.blocks {
            let same = second.blocks.get(id) == Some(run);
            assert_eq!(same, *id != edited, "block {id:?}");
        }
    }

    #[test]
    fn an_edit_from_another_peer_reshapes_the_block_it_wrote() {
        let (mut alice, _) = laid_out(4, 30);
        let mut bob = ProseState::with_peer(2).expect("a document is built");
        bob.load_faces(&crate::layout::fonts::shipped_faces());
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        let edited = *alice.root_blocks().last().expect("a block");
        let type_one = |alice: &mut ProseState, bob: &mut ProseState, pos: u32| {
            let delta = alice
                .handle_command(ProseCommand::InsertText {
                    block_id: edited,
                    pos,
                    text: "x".to_owned(),
                })
                .update
                .expect("the edit exports");
            bob.apply_remote(&delta);
        };
        // Each peer grew a paragraph on opening. Until Alice holds Bob's
        // removal of his, every import rewinds the tree and names each block.
        type_one(&mut alice, &mut bob, 1);
        let back = bob
            .handle_command(ProseCommand::InsertText {
                block_id: edited,
                pos: 0,
                text: "y".to_owned(),
            })
            .update
            .expect("the edit exports");
        alice.apply_remote(&back);
        type_one(&mut alice, &mut bob, 2);
        let before = bob.shaped_layout();
        type_one(&mut alice, &mut bob, 3);
        let after = bob.shaped_layout();
        let edited = crate::events::tree_id(edited);
        let old = before.blocks[&edited].data().len();
        assert_eq!(after.blocks[&edited].data().len(), old + 1);
        for (id, run) in &before.blocks {
            if *id != edited {
                assert!(after.blocks.get(id) == Some(run), "block {id:?}");
            }
        }
    }
}
