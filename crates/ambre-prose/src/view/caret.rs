//! The caret and the selection: where they go, and what a press or a drag
//! makes of them (`features/ambre-spec-prose-selection.md section 2`).
//!
//! Everything here reads the runs of the layout in force and writes view
//! state alone. Nothing shapes a block, and nothing is sent: a selection never
//! enters the document (`PROSE-UI-010`).

use crate::events::{BlockId, CursorPosition, block_id, tree_id};
use crate::layout::run::ParleyRunData;
use crate::ui::{ProseUiState, chars};
use crate::view::hit::fragment;

/// A walk of the caret with the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    PageUp,
    PageDown,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

/// A place in the text: a block and a code-point offset.
pub(crate) type Spot = (BlockId, u32);

impl ProseUiState {
    /// The run a text block was shaped into, in the layout in force.
    pub(crate) fn run_of(&self, block: BlockId) -> Option<&ParleyRunData> {
        self.shaped
            .blocks
            .get(&tree_id(block))
            .map(|run| run.data())
    }

    /// The first and the last text block of the document, the last with its
    /// length in the layout in force. They hold whether or not the layout in
    /// force carries them (`PROSE-UI-167`).
    pub(crate) fn text_ends(&self) -> Option<(BlockId, BlockId, u32)> {
        let ends = self.shaped.ends()?;
        let last = block_id(ends.last);
        let len = self.run_of(last).map_or(ends.last_len, ParleyRunData::len);
        Some((block_id(ends.first), last, len))
    }

    /// The text block before `block` in the flow, or after it.
    pub(crate) fn neighbour(&self, block: BlockId, after: bool) -> Option<BlockId> {
        let index = self.flow_index(&tree_id(block))?;
        let is_text = |(at, b): &(usize, &crate::layout::place::BlockLayout)| {
            (if after { *at > index } else { *at < index })
                && self.shaped.blocks.contains_key(&b.block_id)
        };
        let found = if after {
            self.shaped.window_blocks().find(is_text)
        } else {
            self.shaped.window_blocks().rev().find(is_text)
        };
        found.map(|(_, b)| block_id(b.block_id))
    }

    /// How many code points a block holds, the draft typed into it counted.
    pub(crate) fn block_len(&self, block: BlockId) -> u32 {
        let laid = self.run_of(block).map_or(0, ParleyRunData::len);
        match self.draft.shown() {
            Some((anchored, ..)) if anchored == block => laid.saturating_add(self.draft.unlaid()),
            _ => laid,
        }
    }

    /// A document offset, as the layout in force counts it: the draft it
    /// does not carry yet is taken out.
    pub(crate) fn to_laid(&self, block: BlockId, offset: u32) -> u32 {
        match self.draft.shown() {
            Some((anchored, at, text)) if anchored == block => {
                let n = chars(&text);
                if offset <= at {
                    offset
                } else if offset >= at.saturating_add(n) {
                    offset - n
                } else {
                    at
                }
            }
            _ => offset,
        }
    }

    /// An offset of the layout in force, as the document counts it.
    pub(crate) fn to_doc(&self, block: BlockId, laid: u32) -> u32 {
        match self.draft.shown() {
            Some((anchored, at, text)) if anchored == block && laid > at => {
                laid.saturating_add(chars(&text))
            }
            _ => laid,
        }
    }

    /// The two ends of the selection in flow order, or `None` for a caret
    /// alone or an empty range (`PROSE-UI-151`).
    #[must_use]
    pub fn selection(&self) -> Option<(Spot, Spot)> {
        let cursor = self.cursor.as_ref()?;
        let anchor = cursor.anchor?;
        let focus = (cursor.block_id, cursor.offset);
        if anchor == focus {
            return None;
        }
        let rank = |spot: Spot| (self.flow_index(&tree_id(spot.0)), spot.1);
        if rank(anchor) <= rank(focus) {
            Some((anchor, focus))
        } else {
            Some((focus, anchor))
        }
    }

    /// Move the caret, or grow the selection to it when `extend`
    /// (`features/ambre-spec-prose-selection.md section 2`).
    ///
    /// A vertical move keeps the column the run of them started from
    /// (`PROSE-UI-034`); any other move ends the run.
    pub fn move_cursor(&mut self, direction: Direction, extend: bool) {
        let Some(cursor) = self.cursor else {
            return;
        };
        let block = cursor.block_id;
        let laid = self.to_laid(block, cursor.offset);
        let upstream = self.caret_upstream;
        let vertical = matches!(
            direction,
            Direction::Up | Direction::Down | Direction::PageUp | Direction::PageDown
        );
        let Some(run) = self.run_of(block) else {
            return;
        };
        let x_goal = if vertical {
            let caret = run.caret_box_at(laid, upstream);
            Some(self.x_goal.unwrap_or(run.indent_px() + caret.x0 as f32))
        } else {
            None
        };
        let next = match direction {
            Direction::Left => self.step(block, laid, false, false),
            Direction::Right => self.step(block, laid, true, false),
            Direction::WordLeft => self.step(block, laid, false, true),
            Direction::WordRight => self.step(block, laid, true, true),
            Direction::Up | Direction::Down => {
                self.line_step(block, laid, upstream, direction == Direction::Down, x_goal)
            }
            Direction::PageUp | Direction::PageDown => {
                let down = direction == Direction::PageDown;
                let caret = run.caret_box_at(laid, upstream);
                let line = ((caret.y1 - caret.y0) as f32).max(1.0);
                let lines = (self.viewport.visible_height() / line).floor().max(1.0) as usize;
                let mut here = (block, laid, upstream);
                for _ in 0..lines {
                    let moved = self.line_step(here.0, here.1, here.2, down, x_goal);
                    if moved == here {
                        break;
                    }
                    here = moved;
                }
                here
            }
            Direction::LineStart | Direction::LineEnd => {
                let (offset, up) = run.line_edge(laid, upstream, direction == Direction::LineEnd);
                (block, offset, up)
            }
            Direction::DocStart => match self.text_ends() {
                Some((first, ..)) => (first, 0, false),
                None => (block, laid, upstream),
            },
            Direction::DocEnd => match self.text_ends() {
                Some((_, last, len)) => (last, len, false),
                None => (block, laid, upstream),
            },
        };
        let offset = self.to_doc(next.0, next.1);
        // A document end may sit on a page the layout does not carry.
        if !self.shaped.carries(&tree_id(next.0)) {
            self.awaited_reveal = Some(next.0);
        }
        self.apply_caret((next.0, offset), next.2, x_goal, extend);
    }

    /// One grapheme or one word along the text, into the next block at an
    /// edge. Offsets count the layout in force.
    fn step(&self, block: BlockId, laid: u32, forward: bool, word: bool) -> (BlockId, u32, bool) {
        let Some(run) = self.run_of(block) else {
            return (block, laid, false);
        };
        let inside = match (forward, word) {
            (false, false) => run.grapheme_before(laid),
            (true, false) => run.grapheme_after(laid),
            (false, true) => run.word_before(laid),
            (true, true) => run.word_after(laid),
        };
        if let Some(offset) = inside {
            return (block, offset, false);
        }
        match self.neighbour(block, forward) {
            Some(next) if forward => (next, 0, false),
            Some(next) => (next, self.run_of(next).map_or(0, ParleyRunData::len), false),
            None => (block, laid, false),
        }
    }

    /// One wrapped line up or down, at the column `x_goal` holds, into the
    /// next block past the first line or the last. At the ends of the
    /// document it goes to the start or the end.
    fn line_step(
        &self,
        block: BlockId,
        laid: u32,
        upstream: bool,
        down: bool,
        x_goal: Option<f32>,
    ) -> (BlockId, u32, bool) {
        let Some(run) = self.run_of(block) else {
            return (block, laid, upstream);
        };
        let x = x_goal.unwrap_or(0.0);
        let line = run.line_of(laid, upstream);
        let inside = if down {
            (line + 1 < run.line_count()).then_some(line + 1)
        } else {
            line.checked_sub(1)
        };
        if let Some(target) = inside {
            let (offset, up) = run.at_line(target, x - run.indent_px());
            return (block, offset, up);
        }
        match self.neighbour(block, down) {
            Some(next) => {
                let Some(next_run) = self.run_of(next) else {
                    return (block, laid, upstream);
                };
                let target = if down { 0 } else { next_run.line_count() - 1 };
                let (offset, up) = next_run.at_line(target, x - next_run.indent_px());
                (next, offset, up)
            }
            None if down => (block, run.len(), false),
            None => (block, 0, false),
        }
    }

    /// The common tail of every move: place the caret, keep or clear the
    /// column, extend or collapse the selection, then reveal the caret.
    ///
    /// A move without extension collapses the selection (`PROSE-UI-036`), and
    /// one with it keeps the anchor where it is (`PROSE-UI-160`). Every move
    /// ends here, so none can leave the caret off screen (`PROSE-UI-035`).
    fn apply_caret(&mut self, next: Spot, upstream: bool, x_goal: Option<f32>, extend: bool) {
        let anchor = match (&self.cursor, extend) {
            (Some(cursor), true) => Some(cursor.anchor.unwrap_or((cursor.block_id, cursor.offset))),
            _ => None,
        };
        self.cursor = Some(CursorPosition {
            block_id: next.0,
            offset: next.1,
            anchor: anchor.filter(|spot| *spot != next),
        });
        self.caret_upstream = upstream;
        self.x_goal = x_goal;
        self.reveal_caret();
    }

    /// After a local insertion, advance the caret past it and reveal it. The
    /// column is dropped: an edit ends a run of vertical moves.
    pub(crate) fn advance_cursor(&mut self, n: u32) {
        let Some(cursor) = &mut self.cursor else {
            return;
        };
        cursor.offset = cursor.offset.saturating_add(n);
        cursor.anchor = None;
        self.caret_upstream = false;
        self.x_goal = None;
        self.reveal_caret();
    }

    /// Put the caret at a place the view decided, with no selection.
    pub(crate) fn set_caret(&mut self, spot: Spot) {
        self.cursor = Some(CursorPosition {
            block_id: spot.0,
            offset: spot.1,
            anchor: None,
        });
        self.caret_upstream = false;
        self.x_goal = None;
    }

    /// Where the caret is drawn, from the top of the column: its top and its
    /// bottom, in document pixels.
    pub(crate) fn caret_span(&self) -> Option<(f32, f32)> {
        let cursor = self.cursor.as_ref()?;
        let laid = self.to_laid(cursor.block_id, cursor.offset);
        let Some(index) = self.flow_index(&tree_id(cursor.block_id)) else {
            return self.band_caret_span(cursor.block_id, laid);
        };
        let block = self.shaped.block(index)?;
        let run = self.run_of(cursor.block_id)?;
        let caret = run.caret_box_at(laid, self.caret_upstream);
        let (y0, y1) = (caret.y0 as f32, caret.y1 as f32);
        let tops = run.line_tops();
        let body_top = self.shaped.layout.body_top_px();
        let last = *block.page_span().end();
        for page in block.page_span() {
            let Some(piece) = fragment(block, page, &tops) else {
                continue;
            };
            if page == last || y0 < piece.run_top + piece.height {
                let base = self.column.page_top_y(page) + body_top + piece.top - piece.run_top;
                return Some((base + y0, base + y1));
            }
        }
        None
    }

    /// Where the caret is drawn in a band, on the page it was placed on, or
    /// in a note, on the page that carries it.
    fn band_caret_span(&self, block: BlockId, laid: u32) -> Option<(f32, f32)> {
        let id = tree_id(block);
        let (page, top) = match self.note_page(block) {
            Some(page) => {
                let slot = self
                    .note_slots(page)
                    .into_iter()
                    .find(|slot| slot.block == id)?;
                (page, slot.top)
            }
            None => {
                let page = self.band_page?;
                let slot = self
                    .band_slots(page)
                    .into_iter()
                    .find(|slot| slot.block == id)?;
                (page, slot.top)
            }
        };
        let caret = self.run_of(block)?.caret_box_at(laid, self.caret_upstream);
        let base = self.column.page_top_y(page) + top;
        Some((base + caret.y0 as f32, base + caret.y1 as f32))
    }

    /// Whether a selection may run from `anchor` to `focus`: a header or a
    /// footer holds its own text, and no selection crosses its edge.
    pub(crate) fn joins(&self, anchor: Spot, focus: Spot) -> bool {
        anchor.0 == focus.0 || !(self.is_band(anchor.0) || self.is_band(focus.0))
    }

    /// Scroll the least that brings the caret back into the window
    /// (`PROSE-UI-158`). A caret already in it moves nothing.
    pub(crate) fn reveal_caret(&mut self) {
        let Some((top, bottom)) = self.caret_span() else {
            return;
        };
        let visible = self.viewport.visible_height();
        let scroll = self.viewport.scroll_y;
        let wanted = if top < scroll {
            top
        } else if bottom > scroll + visible {
            bottom - visible
        } else {
            return;
        };
        self.viewport.scroll_y = self.column.clamp_scroll(&self.viewport, wanted);
    }

    /// Select the whole document: the anchor at the start of the first block,
    /// the moving end at the end of the last (`PROSE-UI-167`). The window
    /// stays where it is (`PROSE-UI-168`).
    pub fn select_all(&mut self) {
        let Some((first, last, len)) = self.text_ends() else {
            return;
        };
        let end = self.to_doc(last, len);
        self.cursor = Some(CursorPosition {
            block_id: last,
            offset: end,
            anchor: Some((first, 0)).filter(|anchor| *anchor != (last, end)),
        });
        self.caret_upstream = false;
        self.x_goal = None;
    }

    /// Escape: collapse the selection to the caret and cancel a drag in
    /// flight (`PROSE-UI-164`, `PROSE-UI-165`). The caret and the window stay
    /// where they are (`PROSE-UI-166`).
    pub fn escape(&mut self) {
        if let Some(cursor) = &mut self.cursor {
            cursor.anchor = None;
        }
        self.dragging = None;
    }

    /// The place under a canvas point, in document offsets.
    ///
    /// A point on a block without text resolves to the edge on the pointer's
    /// side (`PROSE-UI-032`). That is the end of the text before it, or the
    /// start of the text after it.
    pub(crate) fn place_at(&self, x: f32, y: f32) -> Option<Spot> {
        let (block, laid) = self.hit_test(x, y)?;
        if self.run_of(block).is_some() {
            return Some((block, self.to_doc(block, laid)));
        }
        let x_in_page = (x - self.column.left_px(&self.viewport)) / self.viewport.scale();
        let middle = self.shaped.layout.margin_left_px + self.shaped.layout.content_width_px / 2.0;
        let after = x_in_page >= middle;
        let edge = |next: BlockId, after: bool| {
            if after {
                (next, 0)
            } else {
                (next, self.block_len(next))
            }
        };
        self.neighbour(block, after)
            .map(|next| edge(next, after))
            .or_else(|| self.neighbour(block, !after).map(|next| edge(next, !after)))
    }

    /// The press of a drag: capture its anchor where it landed
    /// (`PROSE-UI-037`).
    pub(crate) fn start_drag(&mut self, anchor: Spot) {
        self.dragging = Some(anchor);
    }

    /// Extend a drag to the point under the pointer. The anchor is the one
    /// the press captured (`PROSE-UI-161`), and the window does not scroll
    /// (`PROSE-UI-162`).
    pub fn update_drag(&mut self, x: f32, y: f32) {
        let Some(anchor) = self.dragging else {
            return;
        };
        let Some(focus) = self.place_at(x, y) else {
            return;
        };
        if !self.joins(anchor, focus) {
            return;
        }
        self.cursor = Some(CursorPosition {
            block_id: focus.0,
            offset: focus.1,
            anchor: Some(anchor).filter(|anchor| *anchor != focus),
        });
        self.caret_upstream = false;
        self.x_goal = None;
    }

    /// The word under a place, or the whole paragraph, as a selection
    /// (`PROSE-UI-155`, `PROSE-UI-171`).
    pub(crate) fn select_around(&mut self, spot: Spot, paragraph: bool) {
        let Some(run) = self.run_of(spot.0) else {
            return;
        };
        let (start, end) = if paragraph {
            (0, run.len())
        } else {
            run.word_around(self.to_laid(spot.0, spot.1))
        };
        let (start, end) = (self.to_doc(spot.0, start), self.to_doc(spot.0, end));
        self.cursor = Some(CursorPosition {
            block_id: spot.0,
            offset: end,
            anchor: Some((spot.0, start)).filter(|_| start != end),
        });
        self.caret_upstream = false;
        self.x_goal = None;
    }
}
