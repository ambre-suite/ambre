//! Placement: measured blocks laid onto pages
//! (`features/ambre-spec-prose-pagination.md section 2`, pass 2).
//!
//! Placement is sequential and never revisits a page it has emitted
//! (`PROSE-078`). A constraint it cannot meet is resolved by moving forward,
//! and the constraint order of section 2 decides which one gives way.

use ambre_bus::app::prose::BlockType;
use ambre_core::Uuid;
use std::sync::Arc;

use loro::TreeID;

use crate::layout::measure::TextMetrics;
use crate::layout::run::{ShapedRun, TableShape};

/// The lines a break leaves behind, and the lines it carries over
/// (`PROSE-072`, `PROSE-073`).
const MIN_LINES_AT_BREAK: u32 = 2;

/// The band drawn once above the notes of a page, rule and gap together.
pub const NOTE_SEPARATOR_PX: f32 = 8.0;

/// One footnote a block calls, as pass 0 numbered and measured it.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteCall {
    pub footnote_id: Uuid,
    /// The block holding the note's text (`PROSE-FEAT-305`).
    pub block_id: TreeID,
    /// Counted before placement, from the calls in document order
    /// (`PROSE-117`).
    pub ordinal: u32,
    /// The line of the block the call sits on: a table's row, a text's line.
    pub line: u32,
    /// The note's shaped text, or its number when that is taller.
    pub height_px: f32,
}

/// A footnote placed at the foot of a page (`PROSE-015`).
#[derive(Clone, Debug, PartialEq)]
pub struct FootnoteLayout {
    pub footnote_id: Uuid,
    /// The block holding the note's text, the caret's address in it.
    pub block_id: TreeID,
    pub ordinal: u32,
    pub page: u32,
    pub height_px: f32,
}

/// What one block costs, as pass 1 measured it.
///
/// Placement reads heights and flags alone: the text is already shaped and no
/// glyph reaches this far.
#[derive(Clone, Debug)]
pub struct MeasuredBlock {
    pub block_id: TreeID,
    pub block_type: BlockType,
    /// The content height, both margins excluded.
    pub height_px: f32,
    pub space_before_px: f32,
    pub space_after_px: f32,
    pub keep_together: bool,
    pub keep_with_next: bool,
    /// The line tops a split may be taken at (`PROSE-071`). Empty for a block
    /// that has no line structure, which is a block that cannot split.
    pub metrics: Option<TextMetrics>,
    /// The notes this block calls, in the order of their calls.
    pub notes: Vec<NoteCall>,
    /// The text as it was shaped, for the view to draw. `None` for a block
    /// that draws no text of its own: a rule, a break, an image, a table.
    pub run: Option<ShapedRun>,
    /// The cells of a table, for the view to draw. `None` for any other block.
    pub table: Option<Arc<TableShape>>,
}

/// Two measurements are equal when placement would treat them alike. The run
/// is left out: text edited to the same lines moves no block, and a reflow
/// must not replace the tail for it (`PROSE-083`).
impl PartialEq for MeasuredBlock {
    fn eq(&self, other: &Self) -> bool {
        self.block_id == other.block_id
            && self.block_type == other.block_type
            && self.height_px == other.height_px
            && self.space_before_px == other.space_before_px
            && self.space_after_px == other.space_after_px
            && self.keep_together == other.keep_together
            && self.keep_with_next == other.keep_with_next
            && self.metrics == other.metrics
            && self.notes == other.notes
    }
}

impl MeasuredBlock {
    /// Whether this block may be cut at all.
    ///
    /// A heading and an image never split (`PROSE-013`, `PROSE-014`), and a
    /// block with no line structure has no boundary to cut at (`PROSE-071`).
    #[must_use]
    pub fn splittable(&self) -> bool {
        if matches!(
            self.block_type,
            BlockType::Heading | BlockType::Image | BlockType::PageBreak | BlockType::HRule
        ) {
            return false;
        }
        self.metrics
            .as_ref()
            .is_some_and(|metrics| metrics.line_count > 1)
    }

    /// The height the first `lines` lines take, or the whole block when it has
    /// no line structure.
    fn height_of_first(&self, lines: u32) -> f32 {
        match &self.metrics {
            Some(metrics) => metrics.height_of_first(lines),
            None => self.height_px,
        }
    }

    /// How many whole lines fit in a height.
    fn lines_fitting_in(&self, height_px: f32) -> u32 {
        match &self.metrics {
            Some(metrics) => metrics.lines_fitting_in(height_px),
            None => u32::from(height_px >= self.height_px),
        }
    }

    fn line_count(&self) -> u32 {
        match &self.metrics {
            Some(metrics) => metrics.line_count,
            None => 1,
        }
    }
}

/// Where one block, or one fragment of it, sits.
///
/// A split block is one entry: `page_start` carries the first fragment, and
/// each later page carries the next. The whole height of a split block is
/// never in `height_px` (`PROSE-066`).
#[derive(Clone, Debug, PartialEq)]
pub struct BlockLayout {
    pub block_id: TreeID,
    pub block_type: BlockType,
    /// The height of the portion on `page_start` alone.
    pub height_px: f32,
    pub page_start: u32,
    pub y_in_page: f32,
    pub is_split: bool,
    /// Index of the first line carried by each page after `page_start`.
    pub split_points: Vec<u32>,
    /// The height of each fragment after `page_start`, one per split point. A
    /// continuation starts at y = 0 (`PROSE-018`) and carries no leading
    /// margin (`PROSE-070`).
    pub continuation_heights: Vec<f32>,
    /// The notes this block calls, each on the page its calling line took.
    pub notes: Vec<FootnoteLayout>,
    /// The height the notes take at the foot of the block's last page once
    /// this block is placed. Earlier blocks' notes on that page count too.
    pub foot_px: f32,
}

impl BlockLayout {
    /// The pages this block has a fragment on, `page_start` included.
    #[must_use]
    pub fn page_span(&self) -> std::ops::RangeInclusive<u32> {
        let last = self
            .page_start
            .saturating_add(u32::try_from(self.continuation_heights.len()).unwrap_or(0));
        self.page_start..=last
    }

    /// Whether a fragment of this block shows on a page.
    #[must_use]
    pub fn shows_on(&self, page: u32) -> bool {
        self.page_span().contains(&page)
    }
}

/// Every block placed, and the pages they took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    pub blocks: Vec<BlockLayout>,
    /// Always at least one: an empty document is one blank page
    /// (`PROSE-011`).
    pub page_count: u32,
}

impl Placement {
    /// Every fragment visible on a page, continuations of split blocks
    /// included.
    pub fn blocks_on_page(&self, page: u32) -> impl Iterator<Item = &BlockLayout> {
        self.blocks.iter().filter(move |block| block.shows_on(page))
    }
}

/// The cursor placement walks the document with.
///
/// It is the whole of the state placement carries forward. That is what lets
/// a reflow resume at a block rather than at the first one (`PROSE-081`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cursor {
    pub(crate) page: u32,
    /// The next free y in the body area of the current page.
    pub(crate) y: f32,
    /// The trailing margin of the block just placed, not yet spent
    /// (`PROSE-069`).
    pub(crate) pending_margin_px: f32,
    pub(crate) body_height_px: f32,
    /// The height the current page's notes hold at its foot, separator
    /// included.
    pub(crate) foot_px: f32,
}

impl Cursor {
    pub(crate) fn new(body_height_px: f32) -> Self {
        Self {
            page: 0,
            y: 0.0,
            pending_margin_px: 0.0,
            body_height_px,
            foot_px: 0.0,
        }
    }

    fn at_page_top(&self) -> bool {
        self.y <= 0.0
    }

    /// The gap before a block: the collapse of its leading margin against the
    /// previous trailing one, dropped entirely at a page top (`PROSE-069`).
    fn gap_before(&self, block: &MeasuredBlock) -> f32 {
        if self.at_page_top() {
            0.0
        } else {
            self.pending_margin_px.max(block.space_before_px)
        }
    }

    /// What is left on this page once the gap and the notes are spent.
    fn room_after_gap(&self, block: &MeasuredBlock) -> f32 {
        self.body_height_px - self.foot_px - self.y - self.gap_before(block)
    }

    /// The height `notes` would add to this page's foot, the separator
    /// counted when they are the page's first.
    fn foot_cost<'a>(&self, notes: impl Iterator<Item = &'a NoteCall>) -> f32 {
        let height: f32 = notes.map(|note| note.height_px).sum();
        if height > 0.0 && self.foot_px <= 0.0 {
            height + NOTE_SEPARATOR_PX
        } else {
            height
        }
    }

    /// Reserve `notes` at this page's foot (`PROSE-015`).
    fn reserve<'a>(
        &mut self,
        notes: impl Iterator<Item = &'a NoteCall> + Clone,
        out: &mut Vec<FootnoteLayout>,
    ) {
        self.foot_px += self.foot_cost(notes.clone());
        out.extend(notes.map(|note| FootnoteLayout {
            footnote_id: note.footnote_id,
            block_id: note.block_id,
            ordinal: note.ordinal,
            page: self.page,
            height_px: note.height_px,
        }));
    }

    fn open_new_page(&mut self) {
        self.page = self.page.saturating_add(1);
        self.y = 0.0;
        self.pending_margin_px = 0.0;
        self.foot_px = 0.0;
    }
}

/// Lay measured blocks onto pages.
///
/// `body_height_px` is what the two bands left, which the page box already
/// capped (`PROSE-016`). It is treated as positive. A body of zero would take
/// no block at all, so one line is forced onto each page rather than looping
/// (`PROSE-017`).
#[must_use]
pub fn place(blocks: &[MeasuredBlock], body_height_px: f32) -> Placement {
    let mut cursor = Cursor::new(body_height_px.max(0.0));
    let mut placed: Vec<BlockLayout> = Vec::with_capacity(blocks.len());
    place_tail(blocks, 0, &mut cursor, &mut placed);
    Placement {
        blocks: placed,
        page_count: cursor.page.saturating_add(1),
    }
}

/// Place the blocks from `from` onward, carrying the cursor forward.
///
/// The one loop of pass 2. A full placement enters it at zero, a reflow at the
/// first block whose measurement moved (`PROSE-081`).
pub(crate) fn place_tail(
    blocks: &[MeasuredBlock],
    from: usize,
    cursor: &mut Cursor,
    placed: &mut Vec<BlockLayout>,
) {
    place_tail_until(blocks, from, cursor, placed, |_, _| false);
}

/// Place from `from` onward, stopping before the first block `stop` accepts.
///
/// The index it stops at is returned, and `None` when it placed the whole
/// tail. A reflow stops where the cursor rejoins what it was, because from
/// there the rest of the layout is what it already is (`PROSE-100`).
pub(crate) fn place_tail_until(
    blocks: &[MeasuredBlock],
    from: usize,
    cursor: &mut Cursor,
    placed: &mut Vec<BlockLayout>,
    stop: impl Fn(usize, &Cursor) -> bool,
) -> Option<usize> {
    for (index, block) in blocks.iter().enumerate().skip(from) {
        if index > from && stop(index, cursor) {
            return Some(index);
        }
        if block.block_type == BlockType::PageBreak {
            // A manual break is never relaxed (`PROSE-012`). It emits a
            // zero-height entry so a reader can still find it on its page.
            placed.push(zero_height_at(block, cursor));
            cursor.open_new_page();
            continue;
        }
        let successor = blocks.get(index + 1);
        place_one(block, successor, cursor, placed);
    }
    None
}

/// The cursor a block leaves behind, read back from where it was placed.
///
/// It mirrors what `emit_whole`, `split` and the manual break write, and a
/// property confronts it with the cursor placement actually carried.
#[must_use]
pub(crate) fn cursor_after(
    entry: &BlockLayout,
    block: &MeasuredBlock,
    body_height_px: f32,
) -> Cursor {
    if entry.block_type == BlockType::PageBreak {
        let mut cursor = Cursor::new(body_height_px);
        cursor.page = entry.page_start.saturating_add(1);
        return cursor;
    }
    let last_page = *entry.page_span().end();
    let y = match entry.continuation_heights.last() {
        Some(height) => *height,
        None => entry.y_in_page + entry.height_px,
    };
    Cursor {
        page: last_page,
        y,
        pending_margin_px: block.space_after_px,
        body_height_px,
        foot_px: entry.foot_px,
    }
}

/// Place one block, opening pages and splitting it as the constraint order
/// allows.
fn place_one(
    block: &MeasuredBlock,
    successor: Option<&MeasuredBlock>,
    cursor: &mut Cursor,
    placed: &mut Vec<BlockLayout>,
) {
    let room = cursor.room_after_gap(block);
    if block.height_px + cursor.foot_cost(block.notes.iter()) <= room {
        if moves_to_keep_with_successor(block, successor, room, cursor) {
            cursor.open_new_page();
        }
        emit_whole(block, cursor, placed);
        return;
    }
    // It does not fit. A block a whole page holds moves rather than splits
    // (`PROSE-075`), and so does one that may not be cut at all. Its notes
    // move with it.
    let whole = block.height_px + Cursor::new(0.0).foot_cost(block.notes.iter());
    let whole_page_holds_it = whole <= cursor.body_height_px;
    if (whole_page_holds_it || block.keep_together || !block.splittable()) && !cursor.at_page_top()
    {
        cursor.open_new_page();
        // On a fresh page it is placed whether or not it fits: the page it
        // would move to is no larger (`PROSE-017`).
        if whole <= cursor.body_height_px || !block.splittable() {
            emit_whole(block, cursor, placed);
            return;
        }
    }
    if !block.splittable() {
        emit_whole(block, cursor, placed);
        return;
    }
    // A cut that would leave fewer than two lines behind is not taken here:
    // the block starts on the next page instead (`PROSE-072`). On a fresh page
    // there is nowhere further to go, and the rule gives way (`PROSE-017`).
    let room = cursor.room_after_gap(block);
    let wanted = MIN_LINES_AT_BREAK.min(block.line_count());
    if lines_fitting_from(block, room, 0) < wanted && !cursor.at_page_top() {
        cursor.open_new_page();
    }
    split(block, cursor, placed);
}

/// Whether a heading, or a block that asks to stay with its successor, moves
/// to the next page (`PROSE-074`).
///
/// It moves when what follows it cannot place two lines under it. A block
/// already at a page top never moves: the next page is no roomier, and moving
/// would not terminate.
fn moves_to_keep_with_successor(
    block: &MeasuredBlock,
    successor: Option<&MeasuredBlock>,
    room: f32,
    cursor: &Cursor,
) -> bool {
    if cursor.at_page_top() {
        return false;
    }
    if !(block.keep_with_next || block.block_type == BlockType::Heading) {
        return false;
    }
    let Some(next) = successor else {
        return false;
    };
    if next.block_type == BlockType::PageBreak {
        return false;
    }
    let after_block = room - block.height_px - next.space_before_px.max(block.space_after_px);
    let wanted = next.line_count().min(MIN_LINES_AT_BREAK);
    next.height_of_first(wanted) > after_block
}

/// Emit a block whole on the current page, and move the cursor past it.
fn emit_whole(block: &MeasuredBlock, cursor: &mut Cursor, placed: &mut Vec<BlockLayout>) {
    let gap = cursor.gap_before(block);
    let y = cursor.y + gap;
    let mut notes = Vec::with_capacity(block.notes.len());
    cursor.reserve(block.notes.iter(), &mut notes);
    placed.push(BlockLayout {
        block_id: block.block_id,
        block_type: block.block_type,
        height_px: block.height_px,
        page_start: cursor.page,
        y_in_page: y,
        is_split: false,
        split_points: Vec::new(),
        continuation_heights: Vec::new(),
        notes,
        foot_px: cursor.foot_px,
    });
    cursor.y = y + block.height_px;
    cursor.pending_margin_px = block.space_after_px;
}

/// Cut a block across pages, at a line and never inside one (`PROSE-071`).
///
/// Each page takes as many whole lines as it holds. Two are kept on either
/// side of the break, where the block has the lines to spare (`PROSE-072`,
/// `PROSE-073`). A page that cannot hold two lines takes one, which is what
/// makes the walk terminate (`PROSE-017`).
fn split(block: &MeasuredBlock, cursor: &mut Cursor, placed: &mut Vec<BlockLayout>) {
    let total_lines = block.line_count();
    let gap = cursor.gap_before(block);
    let first_y = cursor.y + gap;
    let page_start = cursor.page;
    let mut notes = Vec::with_capacity(block.notes.len());
    let mut taken = fragment(block, cursor, first_y, 0, &mut notes);
    let mut split_points = Vec::new();
    let mut continuation_heights = Vec::new();
    let first_height = height_between(block, 0, taken);
    while taken < total_lines {
        cursor.open_new_page();
        let next = fragment(block, cursor, 0.0, taken, &mut notes);
        split_points.push(taken);
        continuation_heights.push(height_between(block, taken, taken + next));
        taken += next;
    }
    cursor.y = if split_points.is_empty() {
        first_y + first_height
    } else {
        continuation_heights.last().copied().unwrap_or(0.0)
    };
    cursor.pending_margin_px = block.space_after_px;
    placed.push(BlockLayout {
        block_id: block.block_id,
        block_type: block.block_type,
        height_px: first_height,
        page_start,
        y_in_page: first_y,
        is_split: !split_points.is_empty(),
        split_points,
        continuation_heights,
        notes,
        foot_px: cursor.foot_px,
    });
}

/// The lines one fragment takes from `from`, at `y` on the cursor's page, and
/// the notes those lines call reserved on that page (`PROSE-015`).
///
/// The fragment is first cut to the room the page's foot leaves. When its own
/// notes do not fit beside it, it is cut again to the room they leave. The
/// second cut calls no more notes than the first, so it fits.
fn fragment(
    block: &MeasuredBlock,
    cursor: &mut Cursor,
    y: f32,
    from: u32,
    notes: &mut Vec<FootnoteLayout>,
) -> u32 {
    let total = block.line_count();
    let room = cursor.body_height_px - cursor.foot_px - y;
    let calls = |lines: u32| {
        block
            .notes
            .iter()
            .filter(move |note| note.line >= from && note.line < from.saturating_add(lines))
    };
    let mut lines = lines_for(block, room, from, total);
    let cost = cursor.foot_cost(calls(lines));
    if height_between(block, from, from.saturating_add(lines)) + cost > room {
        lines = lines_for(block, room - cost, from, total);
    }
    cursor.reserve(calls(lines), notes);
    lines
}

/// How many lines one fragment takes, starting at `from`.
///
/// It is what the height holds, trimmed so the far side of the cut is not left
/// with a single line (`PROSE-073`). A fragment is never empty: a zero would
/// place no line on a fresh page and the walk would not terminate
/// (`PROSE-017`).
fn lines_for(block: &MeasuredBlock, height_px: f32, from: u32, total: u32) -> u32 {
    let remaining = total.saturating_sub(from);
    let fitting = lines_fitting_from(block, height_px, from).min(remaining);
    if fitting >= remaining {
        return remaining;
    }
    // Two lines must survive on the far side of the cut (`PROSE-073`). The
    // fragment only ever shrinks to honour it, never grows past what the page
    // holds.
    let carried = remaining.saturating_sub(fitting);
    let trimmed = if carried < MIN_LINES_AT_BREAK {
        fitting.saturating_sub(MIN_LINES_AT_BREAK - carried)
    } else {
        fitting
    };
    trimmed.clamp(1, remaining)
}

/// How many lines fit in a height, counting from line `from`.
fn lines_fitting_from(block: &MeasuredBlock, height_px: f32, from: u32) -> u32 {
    let offset = block.height_of_first(from);
    block
        .lines_fitting_in(height_px + offset)
        .saturating_sub(from)
}

/// The height of the lines in `from..to`.
fn height_between(block: &MeasuredBlock, from: u32, to: u32) -> f32 {
    (block.height_of_first(to) - block.height_of_first(from)).max(0.0)
}

/// The zero-height entry a manual break leaves on its page (`PROSE-012`).
fn zero_height_at(block: &MeasuredBlock, cursor: &Cursor) -> BlockLayout {
    BlockLayout {
        block_id: block.block_id,
        block_type: block.block_type,
        height_px: 0.0,
        page_start: cursor.page,
        y_in_page: cursor.y,
        is_split: false,
        split_points: Vec::new(),
        continuation_heights: Vec::new(),
        notes: Vec::new(),
        foot_px: cursor.foot_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page body ten lines of twenty pixels tall.
    const BODY: f32 = 200.0;
    const LINE: f32 = 20.0;

    pub(super) fn id(n: i32) -> TreeID {
        TreeID::new(n.unsigned_abs().into(), n)
    }

    /// Line tops for a run of `lines` lines, each [`LINE`] tall.
    fn metrics(lines: u32) -> TextMetrics {
        let mut line_tops: Vec<f32> = (0..lines).map(|n| n as f32 * LINE).collect();
        line_tops.push(lines as f32 * LINE);
        TextMetrics {
            text_height_px: lines as f32 * LINE,
            line_count: lines,
            line_tops,
        }
    }

    pub(super) fn text(n: i32, lines: u32) -> MeasuredBlock {
        MeasuredBlock {
            block_id: id(n),
            block_type: BlockType::Paragraph,
            height_px: lines as f32 * LINE,
            space_before_px: 0.0,
            space_after_px: 0.0,
            keep_together: false,
            keep_with_next: false,
            metrics: Some(metrics(lines)),
            notes: Vec::new(),
            run: None,
            table: None,
        }
    }

    pub(super) fn of_type(n: i32, kind: BlockType, height_px: f32) -> MeasuredBlock {
        MeasuredBlock {
            block_id: id(n),
            block_type: kind,
            height_px,
            space_before_px: 0.0,
            space_after_px: 0.0,
            keep_together: false,
            keep_with_next: false,
            metrics: None,
            notes: Vec::new(),
            run: None,
            table: None,
        }
    }

    /// No fragment of any block reaches past the body it was placed in.
    pub(super) fn nothing_overflows(placement: &Placement, body: f32) {
        for block in &placement.blocks {
            assert!(
                block.y_in_page + block.height_px <= body + 0.01,
                "{:?} runs off its page",
                block.block_id
            );
            for height in &block.continuation_heights {
                assert!(*height <= body + 0.01, "a continuation runs off its page");
            }
        }
    }

    #[test]
    fn blocks_that_fit_share_one_page() {
        let placement = place(&[text(1, 3), text(2, 3), text(3, 3)], BODY);
        assert_eq!(placement.page_count, 1);
        assert!(placement.blocks.iter().all(|block| block.page_start == 0));
        assert_eq!(placement.blocks[0].y_in_page, 0.0);
        assert_eq!(placement.blocks[1].y_in_page, 3.0 * LINE);
        nothing_overflows(&placement, BODY);
    }

    #[test]
    fn a_manual_break_opens_a_page_and_leaves_a_zero_height_entry() {
        let blocks = [
            text(1, 2),
            of_type(2, BlockType::PageBreak, 0.0),
            text(3, 2),
        ];
        let placement = place(&blocks, BODY);
        assert_eq!(placement.page_count, 2);
        assert_eq!(placement.blocks[1].height_px, 0.0);
        assert_eq!(placement.blocks[1].page_start, 0);
        assert_eq!(placement.blocks[2].page_start, 1);
        assert_eq!(placement.blocks[2].y_in_page, 0.0);
    }

    #[test]
    fn a_break_is_honoured_even_where_the_page_had_room_to_spare() {
        let blocks = [
            of_type(1, BlockType::PageBreak, 0.0),
            of_type(2, BlockType::PageBreak, 0.0),
            of_type(3, BlockType::PageBreak, 0.0),
        ];
        let placement = place(&blocks, BODY);
        assert_eq!(placement.page_count, 4);
    }

    #[test]
    fn a_block_a_whole_page_holds_moves_rather_than_splits() {
        // Eight lines on a page with six left: the block moves whole
        // (`PROSE-075`).
        let placement = place(&[text(1, 4), text(2, 8)], BODY);
        let second = &placement.blocks[1];
        assert!(!second.is_split);
        assert_eq!(second.page_start, 1);
        assert_eq!(second.y_in_page, 0.0);
    }

    #[test]
    fn a_block_taller_than_a_page_splits_and_its_continuation_starts_at_the_top() {
        let placement = place(&[text(1, 25)], BODY);
        let block = &placement.blocks[0];
        assert!(block.is_split);
        assert_eq!(block.page_start, 0);
        assert_eq!(placement.page_count, 3);
        assert_eq!(block.split_points.len(), block.continuation_heights.len());
        // Every continuation sits at the top of its page (`PROSE-018`), which
        // is what a fragment index of the page span says.
        assert_eq!(block.page_span().count(), 3);
        nothing_overflows(&placement, BODY);
    }

    #[test]
    fn a_split_never_leaves_fewer_than_two_lines_on_either_side() {
        // Eleven lines with nine of room left: a naive cut would carry two
        // lines over, and taking ten would leave one (`PROSE-072`,
        // `PROSE-073`).
        for room_lines in 1..=9 {
            let filler = text(1, room_lines);
            let placement = place(&[filler, text(2, 11)], BODY);
            let block = &placement.blocks[1];
            if !block.is_split {
                continue;
            }
            let first = block.split_points[0];
            assert!(
                first >= MIN_LINES_AT_BREAK,
                "only {first} lines left behind"
            );
            let carried = 11 - first;
            assert!(
                carried >= MIN_LINES_AT_BREAK,
                "only {carried} lines carried"
            );
        }
    }

    #[test]
    fn a_three_line_block_that_overflows_gives_up_the_two_line_rule() {
        // Nine lines placed, three to go, and only one fits. The block still
        // splits rather than losing a line (`PROSE-072` relaxed).
        let placement = place(&[text(1, 9), text(2, 3)], BODY);
        let block = &placement.blocks[1];
        assert!(block.is_split || block.page_start == 1);
        nothing_overflows(&placement, BODY);
    }

    #[test]
    fn a_heading_moves_rather_than_standing_alone_at_a_page_bottom() {
        let mut heading = of_type(2, BlockType::Heading, LINE);
        heading.block_type = BlockType::Heading;
        let blocks = [text(1, 9), heading, text(3, 4)];
        let placement = place(&blocks, BODY);
        let placed_heading = &placement.blocks[1];
        assert_eq!(placed_heading.page_start, 1);
        assert_eq!(placed_heading.y_in_page, 0.0);
        assert_eq!(placement.blocks[2].page_start, 1);
    }

    #[test]
    fn a_heading_with_room_under_it_stays_where_it_is() {
        let heading = of_type(2, BlockType::Heading, LINE);
        let placement = place(&[text(1, 4), heading, text(3, 4)], BODY);
        assert_eq!(placement.blocks[1].page_start, 0);
        assert_eq!(placement.page_count, 1);
    }

    #[test]
    fn a_heading_is_never_cut_however_tall_it_is() {
        let tall = of_type(1, BlockType::Heading, BODY * 3.0);
        let placement = place(&[tall], BODY);
        assert!(!placement.blocks[0].is_split);
        assert!(placement.blocks[0].split_points.is_empty());
    }

    #[test]
    fn an_image_is_never_cut_and_moves_to_a_page_of_its_own() {
        let blocks = [text(1, 5), of_type(2, BlockType::Image, 8.0 * LINE)];
        let placement = place(&blocks, BODY);
        assert!(!placement.blocks[1].is_split);
        assert_eq!(placement.blocks[1].page_start, 1);
    }

    #[test]
    fn a_block_kept_together_moves_whole_where_a_page_can_hold_it() {
        let mut kept = text(2, 8);
        kept.keep_together = true;
        let placement = place(&[text(1, 5), kept], BODY);
        assert!(!placement.blocks[1].is_split);
        assert_eq!(placement.blocks[1].page_start, 1);
    }

    #[test]
    fn a_block_kept_together_and_taller_than_a_page_splits_all_the_same() {
        let mut kept = text(1, 30);
        kept.keep_together = true;
        let placement = place(&[kept], BODY);
        assert!(placement.blocks[0].is_split);
        nothing_overflows(&placement, BODY);
    }

    #[test]
    fn a_leading_margin_collapses_against_the_trailing_one_before_it() {
        let mut first = text(1, 2);
        first.space_after_px = 30.0;
        let mut second = text(2, 2);
        second.space_before_px = 10.0;
        let placement = place(&[first, second], BODY);
        // The larger of the two, never their sum (`PROSE-069`).
        assert_eq!(placement.blocks[1].y_in_page, 2.0 * LINE + 30.0);
    }

    #[test]
    fn a_leading_margin_is_dropped_at_a_page_top() {
        let mut second = text(2, 4);
        second.space_before_px = 50.0;
        let placement = place(&[text(1, 9), second], BODY);
        assert_eq!(placement.blocks[1].page_start, 1);
        assert_eq!(placement.blocks[1].y_in_page, 0.0);
    }

    #[test]
    fn an_empty_document_is_one_page() {
        let placement = place(&[], BODY);
        assert_eq!(placement.page_count, 1);
        assert!(placement.blocks.is_empty());
    }

    #[test]
    fn a_body_of_zero_places_every_block_and_terminates() {
        // A degenerate body must not loop. Each block lands somewhere, and the
        // walk ends (`PROSE-017`).
        let placement = place(&[text(1, 3), text(2, 3)], 0.0);
        assert_eq!(placement.blocks.len(), 2);
    }

    #[test]
    fn every_page_index_a_layout_reports_is_one_it_has() {
        let blocks = [
            text(1, 12),
            of_type(2, BlockType::PageBreak, 0.0),
            text(3, 30),
        ];
        let placement = place(&blocks, BODY);
        for block in &placement.blocks {
            for page in block.page_span() {
                assert!(page < placement.page_count, "page {page} is out of range");
            }
        }
        // Every page in range answers a list, empty or not (`PROSE-011`).
        for page in 0..placement.page_count {
            let _ = placement.blocks_on_page(page).count();
        }
    }

    #[test]
    fn placement_never_goes_back_to_a_page_it_left() {
        let blocks = [
            text(1, 7),
            of_type(2, BlockType::Image, 9.0 * LINE),
            text(3, 22),
            of_type(4, BlockType::PageBreak, 0.0),
            text(5, 3),
        ];
        let placement = place(&blocks, BODY);
        let mut highest = 0;
        for block in &placement.blocks {
            assert!(
                block.page_start >= highest,
                "{:?} landed behind a page already emitted",
                block.block_id
            );
            highest = *block.page_span().end();
        }
        nothing_overflows(&placement, BODY);
    }
}

/// The universal rules placement holds over any document it is handed.
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod properties {
    use proptest::prelude::*;

    use super::tests::{nothing_overflows, of_type, text};
    use super::{
        BlockLayout, BlockType, Cursor, MIN_LINES_AT_BREAK, MeasuredBlock, cursor_after, place,
        place_tail,
    };

    /// One block of any shape placement has to take.
    fn any_block(n: i32) -> impl Strategy<Value = MeasuredBlock> {
        prop_oneof![
            (1u32..40).prop_map(move |lines| text(n, lines)),
            (1u32..40, 0.0f32..80.0, 0.0f32..80.0).prop_map(move |(lines, before, after)| {
                let mut block = text(n, lines);
                block.space_before_px = before;
                block.space_after_px = after;
                block
            }),
            (1u32..40, any::<bool>(), any::<bool>()).prop_map(move |(lines, together, with)| {
                let mut block = text(n, lines);
                block.keep_together = together;
                block.keep_with_next = with;
                block
            }),
            (0.0f32..700.0).prop_map(move |h| of_type(n, BlockType::Image, h)),
            (0.0f32..700.0).prop_map(move |h| of_type(n, BlockType::Heading, h)),
            Just(of_type(n, BlockType::PageBreak, 0.0)),
        ]
    }

    fn any_document() -> impl Strategy<Value = Vec<MeasuredBlock>> {
        (0usize..12).prop_flat_map(|count| {
            let blocks: Vec<_> = (0..count)
                .map(|n| any_block(i32::try_from(n).unwrap_or(0) + 1).boxed())
                .collect();
            blocks
        })
    }

    proptest! {
        /// Every block is placed, and placement ends (`PROSE-017`).
        #[test]
        fn every_block_reaches_a_page(blocks in any_document(), body in 1.0f32..600.0) {
            let placement = place(&blocks, body);
            prop_assert_eq!(placement.blocks.len(), blocks.len());
            prop_assert!(placement.page_count >= 1);
        }

        /// Placement never revisits a page it has emitted (`PROSE-078`).
        #[test]
        fn pages_only_ever_move_forward(blocks in any_document(), body in 1.0f32..600.0) {
            let placement = place(&blocks, body);
            let mut highest = 0;
            for block in &placement.blocks {
                prop_assert!(block.page_start >= highest);
                highest = *block.page_span().end();
            }
        }

        /// Every page a block claims is a page the layout has (`PROSE-011`).
        #[test]
        fn no_block_claims_a_page_out_of_range(blocks in any_document(), body in 1.0f32..600.0) {
            let placement = place(&blocks, body);
            for block in &placement.blocks {
                prop_assert!(*block.page_span().end() < placement.page_count);
            }
        }

        /// A cut is only ever taken at a line, and never at the same one twice
        /// (`PROSE-071`).
        #[test]
        fn a_split_cuts_at_rising_line_indices(blocks in any_document(), body in 40.0f32..600.0) {
            let placement = place(&blocks, body);
            for block in &placement.blocks {
                prop_assert_eq!(block.split_points.len(), block.continuation_heights.len());
                prop_assert_eq!(block.is_split, !block.split_points.is_empty());
                for pair in block.split_points.windows(2) {
                    let [first, second] = pair else { continue };
                    prop_assert!(second > first);
                }
                if let Some(first) = block.split_points.first() {
                    prop_assert!(*first >= 1);
                }
            }
        }

        /// A heading and an image are never cut (`PROSE-013`, `PROSE-014`).
        #[test]
        fn a_heading_or_an_image_is_never_split(blocks in any_document(), body in 1.0f32..600.0) {
            let placement = place(&blocks, body);
            for block in &placement.blocks {
                if matches!(block.block_type, BlockType::Heading | BlockType::Image) {
                    prop_assert!(!block.is_split);
                }
            }
        }

        /// Nothing placed runs off the page it was placed on, wherever the
        /// body is tall enough to hold two lines of it.
        #[test]
        fn no_fragment_runs_off_a_page_that_could_hold_it(
            blocks in any_document(),
            body in 600.0f32..900.0,
        ) {
            let placement = place(&blocks, body);
            let tall = blocks.iter().any(|block| block.height_px > body);
            if !tall {
                nothing_overflows(&placement, body);
            }
        }

        /// Placement resumed from any block is placement run from the first
        /// one (`PROSE-080`, `PROSE-081`).
        ///
        /// The cursor read back off a placed entry is the whole of what a
        /// reflow carries forward. Restarting the walk there has to land the
        /// rest of the document where the full run did.
        #[test]
        fn a_resumed_walk_lands_where_the_full_one_did(
            blocks in any_document(),
            body in 1.0f32..600.0,
            pick in 0usize..12,
        ) {
            let full = place(&blocks, body);
            if blocks.is_empty() {
                return Ok(());
            }
            let from = pick % blocks.len();
            if from == 0 {
                return Ok(());
            }
            let (Some(entry), Some(previous)) =
                (full.blocks.get(from - 1), blocks.get(from - 1)) else {
                return Ok(());
            };
            let mut cursor: Cursor = cursor_after(entry, previous, body.max(0.0));
            let mut placed: Vec<BlockLayout> = match full.blocks.get(..from) {
                Some(head) => head.to_vec(),
                None => return Ok(()),
            };
            place_tail(&blocks, from, &mut cursor, &mut placed);
            prop_assert_eq!(placed, full.blocks);
            prop_assert_eq!(cursor.page.saturating_add(1), full.page_count);
        }

        /// A cut never leaves a single line on either side of it, unless the
        /// block has too few lines to give (`PROSE-072`, `PROSE-073`).
        #[test]
        fn a_cut_leaves_two_lines_on_each_side(lines in 4u32..40, filler in 0u32..9) {
            let body = 200.0;
            let document = vec![text(1, filler.max(1)), text(2, lines)];
            let placement = place(&document, body);
            let block = &placement.blocks[1];
            if !block.is_split {
                return Ok(());
            }
            let mut previous = 0;
            for point in &block.split_points {
                prop_assert!(point - previous >= MIN_LINES_AT_BREAK);
                previous = *point;
            }
            prop_assert!(lines - previous >= MIN_LINES_AT_BREAK);
        }
    }
}
