//! The pages on the canvas: the column, the layout in force, and what the view
//! keeps to paint it (`features/ambre-spec-prose-page-rendering.md`,
//! `features/ambre-spec-prose-viewport.md`).
//!
//! Everything here runs on the UI thread and reads what the Logic thread
//! handed over. A layout arrives shaped (`PROSE-UI-006`). Only the paragraphs an
//! edit in flight changes are shaped here (`provisional`), and nothing reads the
//! document or sends a command.

pub mod a11y;
pub mod annotate;
pub mod cache;
pub mod caret;
pub mod column;
pub mod fling;
pub mod hit;
pub mod paint;
pub mod provisional;
pub mod ruler;
pub mod slash;
pub mod toolbar;
pub mod walk;

use std::collections::HashMap;
use std::sync::Arc;

use loro::TreeID;

use ambre_bus::app::prose::{DocOffset, ProseCommand};
use ambre_core::{DocPx, ViewPx};

use ambre_draw::kinetic::ScrollStep;

use crate::events::{BlockId, PageCount, TocEntry, tree_id};
use crate::layout::run::{RunOffset, RunSpan, ShapedLayout};
use crate::ui::ProseUiState;

/// The pages asked for beyond those a frame builds, on each side, so a
/// scroll finds its next pages laid out (`PROSE-UI-211`).
pub const WINDOW_SLACK: PageCount = PageCount::new(4);

/// What an inline highlight marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightKind {
    Comment,
    Insertion,
    Deletion,
    Find,
}

/// One inline highlight, on the laid run of its block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub kind: HighlightKind,
    pub span: RunSpan,
}

/// Where an offset the Logic thread answered falls on the laid run of its
/// block. It answers the comments, suggestions, links, find matches and
/// peers' positions. Those lists and the layout in force count the text it
/// holds, so the number carries over. The local draft is not in either.
pub(crate) const fn answered(offset: DocOffset) -> RunOffset {
    RunOffset::new(offset.get())
}

/// The span `start..end` the Logic thread answered, on the laid run of its
/// block (see [`answered`]).
pub(crate) const fn answered_span(start: DocOffset, end: DocOffset) -> RunSpan {
    RunSpan::new(answered(start), answered(end))
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
    pub fn apply_layout(&mut self, shaped: impl Into<Arc<ShapedLayout>>) -> bool {
        let shaped = shaped.into();
        if shaped.version <= self.layout_version {
            return false;
        }
        self.layout_version = shaped.version;
        // Built on the Logic thread: taking the layout copies nothing and
        // walks no block (`PROSE-UI-205`).
        self.column = column::PageColumn::of(&shaped.layout);
        self.shaped = shaped;
        self.clamp_scroll();
        // What the draft confirmed is in these runs now.
        self.draft.laid();
        if self
            .splitting
            .and_then(|split| split.new_block)
            .is_some_and(|new_block| self.shaped.index_of(&tree_id(new_block)).is_some())
        {
            self.splitting = None;
        }
        self.settle_awaited_block();
        // A request lost on a full channel is made again from here.
        self.asked_pages = None;
        if let Some(block) = self.awaited_reveal
            && self.shaped.carries(&tree_id(block))
        {
            self.awaited_reveal = None;
            self.asked_block = None;
            self.reveal_block(block);
        }
        true
    }

    /// What to ask the Logic thread so the next frames find their pages laid
    /// out (`PROSE-UI-211`, `PROSE-UI-212`). `None` when the layout in force
    /// carries them.
    ///
    /// The host calls it once a frame and sends what it returns. A request
    /// is made once: the same pages are not asked for again until a layout
    /// comes back without them.
    pub fn window_request(&mut self) -> Option<ProseCommand> {
        if let Some(block) = self.awaited_reveal {
            if self.asked_block == Some(block) {
                return None;
            }
            self.asked_block = Some(block);
            return Some(ProseCommand::ShowBlock { block_id: block });
        }
        let built = self.column.built_pages(&self.viewport)?;
        let carried = self.shaped.pages();
        let built_last = built.last()?;
        if carried.contains(built.start) && carried.contains(built_last) {
            return None;
        }
        let first = built.start - WINDOW_SLACK;
        let last = built_last + WINDOW_SLACK;
        if self.asked_pages == Some((first, last)) {
            return None;
        }
        self.asked_pages = Some((first, last));
        Some(ProseCommand::ShowPages { first, last })
    }

    /// Scroll by a step of the shared model, down for a positive one.
    ///
    /// The column moves by what the gesture asks, up to its ends
    /// (`PROSE-UI-126`). The zoom and the column stay as they were
    /// (`PROSE-UI-127`). Only the model builds a step, so a distance the view
    /// computed itself does not reach the column (`KINETIC-001`).
    pub fn scroll_by(&mut self, step: ScrollStep) {
        let moved = self.viewport.scroll_y + DocPx::new(step.y().get() / self.viewport.scale());
        self.viewport.scroll_y = self.column.clamp_scroll(&self.viewport, moved);
    }

    /// The canvas took a new size. The offset and the zoom stay
    /// (`PROSE-UI-130`): what was at the top of the canvas still is.
    pub fn resize(&mut self, width_px: ViewPx, height_px: ViewPx) {
        self.viewport.width_px = width_px.max(ViewPx::ZERO);
        self.viewport.height_px = height_px.max(ViewPx::ZERO);
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
    ///
    /// A block the layout in force does not carry is asked for, and shown
    /// once a layout carries it (`PROSE-UI-212`).
    pub(crate) fn reveal_block(&mut self, block: BlockId) {
        let Some(layout) = self.block_layout(block) else {
            self.awaited_reveal = Some(block);
            return;
        };
        let top = self.column.page_top_y(layout.page_start)
            + self.shaped.layout.body_top_px()
            + layout.y_in_page;
        self.viewport.scroll_y = self.column.clamp_scroll(&self.viewport, top);
    }

    /// Where a block sits in the layout in force.
    fn block_layout(&self, block: BlockId) -> Option<&crate::layout::place::BlockLayout> {
        let index = self.shaped.index_of(&tree_id(block))?;
        self.shaped.block(index)
    }

    /// The position of a block in the flow of the layout in force.
    pub(crate) fn flow_index(&self, block: &TreeID) -> Option<usize> {
        self.shaped.index_of(block)
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
        let mut push = |block: BlockId, kind: HighlightKind, start: DocOffset, end: DocOffset| {
            if start < end {
                marks.entry(block).or_default().push(Highlight {
                    kind,
                    span: answered_span(start, end),
                });
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
                found.range.start,
                found.range.end,
            );
        }
        self.highlights = marks;
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod tests {
    use crate::layout::run::RunCount;
    use std::sync::Arc;

    use super::*;
    use crate::events::block_id;
    use crate::events::{BlockInit, PageIndex, ProseCommand};
    use crate::layout::pass::PageLayout;
    use crate::logic::ProseState;

    /// The step of a finger dragged up by `dy` screen pixels, which scrolls
    /// the column down by as much.
    /// A point of the view, from the canvas's corner.
    pub(crate) fn view_at(x: f32, y: f32) -> ambre_core::ViewPoint {
        ambre_core::ViewPoint::new(ambre_core::ViewPx::new(x), ambre_core::ViewPx::new(y))
    }

    pub(crate) fn down(dy: f32) -> ScrollStep {
        ScrollStep::dragged(view_at(0.0, dy), view_at(0.0, 0.0))
    }

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
                pos: DocOffset::ZERO,
                text: text.to_owned(),
            });
        }
        id
    }

    /// A document of `paragraphs` paragraphs of `words` words each, laid out
    /// on the faces the application ships.
    pub(crate) fn laid_out(paragraphs: usize, words: usize) -> (ProseState, ShapedLayout) {
        let mut state = ProseState::with_peer(ambre_core::PeerId(1)).expect("a document is built");
        state.load_faces(&crate::layout::fonts::shipped_faces());
        let text = vec!["word"; words].join(" ");
        for _ in 0..paragraphs {
            append(&mut state, BlockInit::Paragraph, &text);
        }
        let shaped = state.shaped_layout();
        (state, shaped)
    }

    /// Answer every page the view asks for, as the host does once a frame,
    /// until the view asks for nothing more.
    pub(crate) fn follow(ui: &mut ProseUiState, state: &mut ProseState) {
        while let Some(command) = ui.window_request() {
            let _ = state.handle_command(command);
            if let Some(event) = state.layout_event() {
                ui.apply_event(&event);
            }
        }
    }

    /// `PROSE-125`: a layout handed to the view carries the pages it asked
    /// for, and no block of another page.
    #[test]
    fn a_layout_carries_the_pages_asked_for_alone() {
        let (mut state, _) = laid_out(400, 60);
        let _ = state.handle_command(ProseCommand::ShowPages {
            first: PageIndex::new(10),
            last: PageIndex::new(12),
        });
        let Some(crate::events::ProseEvent::LayoutShaped { layout }) = state.layout_event() else {
            panic!("a new window is handed over");
        };
        let shaped = layout.shared::<ShapedLayout>().expect("a shaped layout");
        assert!(shaped.layout.page_count.get() > 20);
        assert_eq!(
            shaped.pages(),
            PageIndex::new(10).through(PageIndex::new(12))
        );
        assert!(!shaped.layout.blocks.is_empty());
        for block in &shaped.layout.blocks {
            let span = block.page_span();
            assert!(
                block.last_page() >= PageIndex::new(10) && span.start <= PageIndex::new(12),
                "{span:?} is outside"
            );
        }
        // Every run and table is one of a block the window carries, or a band.
        let carried: std::collections::HashSet<_> =
            shaped.layout.blocks.iter().map(|b| b.block_id).collect();
        let bands = shaped
            .blocks
            .keys()
            .filter(|id| !carried.contains(id))
            .count();
        assert!(bands <= 4, "{bands} runs of blocks outside the window");
    }

    /// `PROSE-UI-211`: scrolling past the pages carried asks for the pages
    /// around the canvas, and once they come back asks for nothing more.
    #[test]
    fn a_scroll_asks_for_the_pages_around_the_canvas() {
        let (mut state, _) = laid_out(400, 60);
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
        ui.apply_event(&state.layout_event().expect("a layout"));
        assert_eq!(ui.window_request(), None, "the first pages are carried");
        ui.scroll_by(down(
            ui.column.page_top_y(PageIndex::new(30)).get() * ui.viewport.scale(),
        ));
        let built = ui.column.built_pages(&ui.viewport).expect("a page");
        let Some(ProseCommand::ShowPages { first, last }) = ui.window_request() else {
            panic!("the pages under the canvas are asked for");
        };
        let built_last = built.last().expect("a page");
        assert!(first <= built.start && last >= built_last);
        assert_eq!(ui.window_request(), None, "a request is made once");
        let _ = state.handle_command(ProseCommand::ShowPages { first, last });
        let event = state.layout_event().expect("a layout");
        ui.apply_event(&event);
        assert!(ui.shaped.pages().contains(built.start));
        assert!(ui.shaped.pages().contains(built_last));
        assert_eq!(ui.window_request(), None);
    }

    /// `PROSE-UI-212`: revealing a block the layout does not carry asks for
    /// its pages, then scrolls to it once a layout carries it.
    #[test]
    fn revealing_a_block_not_carried_asks_for_it_then_shows_it() {
        let (mut state, _) = laid_out(400, 60);
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
        ui.apply_event(&state.layout_event().expect("a layout"));
        let far = state
            .full_layout()
            .and_then(|p| p.blocks.last().map(|b| (b.block_id, b.page_start)))
            .expect("a block");
        let block = crate::events::block_id(far.0);
        assert!(!ui.shaped.carries(&far.0));
        ui.reveal_block(block);
        assert_eq!(
            ui.viewport.scroll_y,
            DocPx::new(0.0),
            "nothing moves before the block comes"
        );
        let Some(command) = ui.window_request() else {
            panic!("the block is asked for");
        };
        assert_eq!(command, ProseCommand::ShowBlock { block_id: block });
        let _ = state.handle_command(command);
        let event = state.layout_event().expect("a layout");
        ui.apply_event(&event);
        assert!(ui.shaped.carries(&far.0));
        assert!(
            ui.column
                .visible_pages(&ui.viewport)
                .expect("a page")
                .contains(far.1)
        );
    }

    /// `FIND-004`: each step of a search brings its current match into the
    /// window. When the layout in force does not hold the match, the view
    /// asks for its pages. A match already in the window moves nothing.
    #[test]
    fn each_step_of_a_search_brings_its_match_into_view() {
        let (mut state, _) = laid_out(0, 0);
        append(&mut state, BlockInit::Paragraph, "needle at the top");
        let body = vec!["word"; 60].join(" ");
        for _ in 0..400 {
            append(&mut state, BlockInit::Paragraph, &body);
        }
        append(&mut state, BlockInit::Paragraph, "needle at the end");
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
        ui.apply_event(&state.layout_event().expect("a layout"));
        let in_view = |ui: &ProseUiState| {
            let hit = ui.find_matches[ui.find_current.expect("a current match").to_usize()];
            let (top, bottom) = ui
                .flow_span(
                    hit.block_id,
                    hit.range.start,
                    crate::layout::run::Affinity::Downstream,
                )
                .expect("the match is laid out");
            let scroll = ui.viewport.scroll_y;
            top >= scroll && bottom <= scroll + ui.viewport.visible_height()
        };
        let mut step = |ui: &mut ProseUiState, command| {
            for event in state.handle_command(command).display {
                ui.apply_event(&event);
            }
            follow(ui, &mut state);
        };

        step(
            &mut ui,
            ProseCommand::FindStart {
                query: crate::events::FindQuery {
                    text: "needle".to_owned(),
                    ..Default::default()
                },
            },
        );
        assert_eq!(
            ui.viewport.scroll_y,
            DocPx::new(0.0),
            "the first match is in view"
        );
        step(&mut ui, ProseCommand::FindNext);
        assert!(ui.viewport.scroll_y > DocPx::new(0.0));
        assert!(in_view(&ui), "the last match is brought into view");
        step(&mut ui, ProseCommand::FindNext);
        assert!(in_view(&ui), "the wrap brings the first one back");
    }

    /// `PROSE-UI-205`: the view takes the layout the Logic thread handed by
    /// its pointer, with the indices built beside it, and copies nothing.
    #[test]
    fn a_layout_is_taken_by_its_pointer() {
        let (mut state, _) = laid_out(3, 5);
        let Some(event) = state.layout_event() else {
            panic!("a layout is handed over");
        };
        let crate::events::ProseEvent::LayoutShaped { layout } = &event else {
            panic!("the event carries a layout");
        };
        let handed = layout.shared::<ShapedLayout>().expect("a shaped layout");
        let mut ui = a_view();
        ui.apply_event(&event);
        assert!(
            Arc::ptr_eq(&ui.shaped, &handed),
            "the view holds the value sent"
        );
        let last = *state.root_blocks().last().expect("a block");
        assert_eq!(ui.flow_index(&crate::events::tree_id(last)), Some(3));
        assert_eq!(
            ui.blocks_on(crate::events::PageIndex::new(0)).len(),
            4,
            "the page index came with it"
        );
    }

    /// `PROSE-UI-205`: a layout the view lets go of is freed on the Logic
    /// thread, never by the view.
    #[test]
    fn a_layout_the_view_drops_is_freed_by_the_logic_thread() {
        let (mut state, _) = laid_out(1, 5);
        let block = *state.root_blocks().last().expect("a block");
        let mut ui = a_view();
        let hand = |state: &mut ProseState, ui: &mut ProseUiState| {
            let event = state.layout_event().expect("a layout is handed over");
            ui.apply_event(&event);
        };
        hand(&mut state, &mut ui);
        let first = Arc::downgrade(&ui.shaped);
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "a".to_owned(),
        });
        hand(&mut state, &mut ui);
        assert!(
            first.upgrade().is_some(),
            "the view let go without freeing it"
        );
        state.handle_command(ProseCommand::InsertText {
            block_id: block,
            pos: DocOffset::ZERO,
            text: "b".to_owned(),
        });
        hand(&mut state, &mut ui);
        assert!(first.upgrade().is_none(), "the Logic thread freed it");
    }

    fn a_layout(version: u64, pages: u32) -> ShapedLayout {
        ShapedLayout::new(
            crate::layout::run::LayoutVersion::new(version),
            Arc::new(PageLayout {
                page_width_px: DocPx::new(800.0),
                page_height_px: DocPx::new(1000.0),
                page_count: PageCount::new(pages),
                ..PageLayout::default()
            }),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
    }

    fn a_view() -> ProseUiState {
        let mut ui = ProseUiState::default();
        ui.resize(ViewPx::new(1200.0), ViewPx::new(900.0));
        ui
    }

    #[test]
    fn a_newer_layout_replaces_the_one_in_force_and_rebuilds_the_column() {
        let mut ui = a_view();
        assert!(ui.apply_layout(a_layout(1, 3)));
        assert!(ui.apply_layout(a_layout(2, 7)));
        assert_eq!(ui.layout_version, crate::layout::run::LayoutVersion::new(2));
        assert_eq!(
            ui.shaped.layout.page_count,
            crate::events::PageCount::new(7)
        );
        assert_eq!(ui.column, column::PageColumn::of(&ui.shaped.layout));
        assert_eq!(ui.column.page_count, crate::events::PageCount::new(7));
    }

    #[test]
    fn a_late_layout_is_dropped_and_the_newer_one_stays() {
        let mut ui = a_view();
        assert!(ui.apply_layout(a_layout(5, 9)));
        assert!(!ui.apply_layout(a_layout(4, 2)));
        assert!(!ui.apply_layout(a_layout(5, 2)));
        assert_eq!(ui.layout_version, crate::layout::run::LayoutVersion::new(5));
        assert_eq!(
            ui.shaped.layout.page_count,
            crate::events::PageCount::new(9)
        );
    }

    #[test]
    fn applying_a_layout_keeps_the_zoom_shapes_nothing_and_writes_nothing() {
        let (logic, shaped) = laid_out(40, 60);
        let before = logic.export_snapshot();
        let shaped_before = logic.shaped_count();
        let mut ui = a_view();
        ui.viewport.zoom = ambre_core::ZoomFactor::positive(1.5);
        assert!(ui.apply_layout(shaped));
        assert_eq!(ui.viewport.zoom.get(), 1.5);
        assert_eq!(logic.shaped_count(), shaped_before);
        assert_eq!(logic.export_snapshot(), before);
        // The view holds the runs it was handed and built none of them.
        assert_eq!(ui.runs.built(), 0);
    }

    #[test]
    fn a_shorter_layout_brings_the_offset_back_inside_the_column() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 30));
        ui.scroll_by(down(1.0e9));
        assert_eq!(ui.viewport.scroll_y, ui.column.max_scroll(&ui.viewport));
        ui.apply_layout(a_layout(2, 2));
        assert_eq!(ui.viewport.scroll_y, ui.column.max_scroll(&ui.viewport));
        ui.apply_layout(a_layout(3, 0));
        assert_eq!(ui.viewport.scroll_y, DocPx::new(0.0));
    }

    #[test]
    fn a_scroll_moves_by_the_gesture_up_to_the_ends_and_nothing_else() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 10));
        ui.viewport.zoom = ambre_core::ZoomFactor::positive(2.0);
        let (column, version) = (ui.column, ui.layout_version);
        ui.scroll_by(down(300.0));
        assert_eq!(ui.viewport.scroll_y, DocPx::new(150.0));
        ui.scroll_by(down(-100.0));
        assert_eq!(ui.viewport.scroll_y, DocPx::new(100.0));
        ui.scroll_by(down(-1.0e6));
        assert_eq!(ui.viewport.scroll_y, DocPx::new(0.0));
        assert_eq!(ui.viewport.zoom.get(), 2.0);
        assert_eq!(ui.column, column);
        assert_eq!(ui.layout_version, version);
    }

    #[test]
    fn a_resize_keeps_the_offset_and_the_zoom() {
        let mut ui = a_view();
        ui.apply_layout(a_layout(1, 10));
        ui.viewport.zoom = ambre_core::ZoomFactor::positive(0.8);
        ui.scroll_by(down(2000.0));
        let scroll_y = ui.viewport.scroll_y;
        ui.resize(ViewPx::new(300.0), ViewPx::new(4000.0));
        assert_eq!(ui.viewport.scroll_y, scroll_y);
        assert_eq!(ui.viewport.zoom.get(), 0.8);
    }

    #[test]
    fn following_an_entry_brings_its_heading_to_the_top_and_sends_nothing() {
        let (logic, shaped) = laid_out(60, 60);
        let before = logic.export_snapshot();
        let mut ui = a_view();
        ui.apply_layout(shaped);
        let far = ui.shaped.layout.blocks.last().expect("a block").clone();
        assert!(far.page_start > PageIndex::ZERO);
        let entry = TocEntry {
            block_id: block_id(far.block_id),
            level: crate::events::HeadingLevel::H1,
            text: String::new(),
            section_number: None,
            page: Some(far.page_start.number()),
        };
        ui.follow_toc_entry(&entry);
        let top =
            ui.column.page_top_y(far.page_start) + ui.shaped.layout.body_top_px() + far.y_in_page;
        assert_eq!(
            ui.viewport.scroll_y,
            ui.column.clamp_scroll(&ui.viewport, top)
        );
        let visible = ui.column.visible_pages(&ui.viewport).expect("pages");
        assert!(visible.contains(far.page_start));
        assert_eq!(logic.export_snapshot(), before);
    }

    #[test]
    fn a_block_not_edited_hands_the_view_the_run_it_was_measured_into() {
        let (mut state, first) = laid_out(4, 30);
        let edited = *state.root_blocks().last().expect("a block");
        state.handle_command(ProseCommand::InsertText {
            block_id: edited,
            pos: DocOffset::ZERO,
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
        let mut bob = ProseState::with_peer(ambre_core::PeerId(2)).expect("a document is built");
        bob.load_faces(&crate::layout::fonts::shipped_faces());
        bob.import_snapshot(&alice.export_snapshot())
            .expect("the seed imports");
        let edited = *alice.root_blocks().last().expect("a block");
        let type_one = |alice: &mut ProseState, bob: &mut ProseState, pos: DocOffset| {
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
        type_one(&mut alice, &mut bob, DocOffset::new(1));
        let back = bob
            .handle_command(ProseCommand::InsertText {
                block_id: edited,
                pos: DocOffset::ZERO,
                text: "y".to_owned(),
            })
            .update
            .expect("the edit exports");
        alice.apply_remote(&back);
        type_one(&mut alice, &mut bob, DocOffset::new(2));
        let before = bob.shaped_layout();
        type_one(&mut alice, &mut bob, DocOffset::new(3));
        let after = bob.shaped_layout();
        let edited = crate::events::tree_id(edited);
        let old = before.blocks[&edited].data().len();
        assert_eq!(after.blocks[&edited].data().len(), old + RunCount::new(1));
        for (id, run) in &before.blocks {
            if *id != edited {
                assert!(after.blocks.get(id) == Some(run), "block {id:?}");
            }
        }
    }
}
