//! The shaped run: what pagination shaped, handed to the view as it is
//! (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! Pagination shapes every block to find where it breaks. The view draws and
//! hit-tests that same shape rather than shaping a second time on the thread
//! that paints (`PROSE-UI-006`, `PROSE-UI-114`). A run holds no GPU handle
//! and nothing bound to a thread, so it crosses by move (`PROSE-UI-113`).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use ambre_core::Uuid;
use ambre_draw::{BoundingBox, Layout, ParleyBrush};
use loro::TreeID;
use parley::{Affinity, Cursor, Selection};
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::measure::{EffectiveStyle, MarkedSpan};
use crate::layout::pass::PageLayout;
use crate::layout::place::BlockLayout;

/// One block's text, shaped and broken into lines: enough to draw and to
/// hit-test.
pub struct ParleyRunData {
    /// The glyphs, their clusters and the line boxes, as the shaper left them.
    layout: Layout<ParleyBrush>,
    /// The byte offset each code point starts at, the text's length last. An
    /// offset in the document counts code points, and the shaper counts bytes.
    char_starts: Vec<u32>,
    /// How far the text sits from the left of the content box. It is the
    /// block's left indent, which the width it was broken to left out.
    indent_px: f32,
    /// The text the glyphs were shaped from. The caret walks its graphemes
    /// and its words, which the glyphs alone do not tell apart.
    text: String,
    /// The style the text was shaped in, so a draft typed into the block is
    /// shaped alike (`PROSE-UI-134`). `None` for a run shaped without one.
    style: Option<EffectiveStyle>,
    /// The spans shaped bold or italic, in bytes, so a draft typed after one
    /// takes its marks (`PROSE-UI-208`).
    marks: Vec<MarkedSpan>,
}

impl ParleyRunData {
    /// The run of `text` as `layout` shaped it, `indent_px` from the left of
    /// the content box.
    #[must_use]
    pub fn new(text: &str, layout: Layout<ParleyBrush>, indent_px: f32) -> Self {
        let mut char_starts: Vec<u32> = text
            .char_indices()
            .map(|(at, _)| u32::try_from(at).unwrap_or(u32::MAX))
            .collect();
        char_starts.push(u32::try_from(text.len()).unwrap_or(u32::MAX));
        Self {
            layout,
            char_starts,
            indent_px,
            text: text.to_owned(),
            style: None,
            marks: Vec::new(),
        }
    }

    /// The marks of the code point before `offset`, its range aside. Text
    /// typed at `offset` takes them, since each expands after its range
    /// (`ambre-spec-prose-schema.md section 3`).
    #[must_use]
    pub fn marks_before(&self, offset: u32) -> MarkedSpan {
        let Some(previous) = offset.checked_sub(1) else {
            return MarkedSpan::default();
        };
        let byte = self.byte_of(previous);
        self.marks
            .iter()
            .find(|span| span.start <= byte && byte < span.end)
            .copied()
            .unwrap_or_default()
    }

    /// The marked spans the text was shaped with, in bytes.
    #[must_use]
    pub fn marks(&self) -> &[MarkedSpan] {
        &self.marks
    }

    /// The style the text was shaped in, when the shaper recorded it.
    #[must_use]
    pub fn style(&self) -> Option<&EffectiveStyle> {
        self.style.as_ref()
    }

    /// The text the glyphs were shaped from.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// How far the text sits from the left of the content box.
    #[must_use]
    pub fn indent_px(&self) -> f32 {
        self.indent_px
    }

    /// The height the shaped text takes.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.layout.height()
    }

    /// The shaped layout, for the drawing to walk.
    #[must_use]
    pub fn layout(&self) -> &Layout<ParleyBrush> {
        &self.layout
    }

    /// How many code points the run holds.
    #[must_use]
    pub fn len(&self) -> u32 {
        u32::try_from(self.char_starts.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// Whether the run holds no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The top of each line, relative to the top of the run, the run's height
    /// last.
    #[must_use]
    pub fn line_tops(&self) -> Vec<f32> {
        let mut tops = Vec::with_capacity(self.layout.len() + 1);
        let mut top = 0.0;
        for line in self.layout.lines() {
            tops.push(top);
            top += line.metrics().line_height;
        }
        tops.push(self.layout.height());
        tops
    }

    /// The code-point offset under a point inside the run.
    #[must_use]
    pub fn offset_at(&self, x: f32, y: f32) -> u32 {
        let byte = Cursor::from_point(&self.layout, x, y).index();
        self.offset_of_byte(byte)
    }

    /// Where a code-point offset sits inside the run: the caret's box.
    #[must_use]
    pub fn caret_box(&self, offset: u32) -> BoundingBox {
        Cursor::from_byte_index(&self.layout, self.byte_of(offset), Affinity::Downstream)
            .geometry(&self.layout, 1.0)
    }

    /// The boxes a code-point range covers, one per line it crosses. Empty
    /// for an empty range.
    #[must_use]
    pub fn range_boxes(&self, start: u32, end: u32) -> Vec<BoundingBox> {
        if start >= end {
            return Vec::new();
        }
        let from = Cursor::from_byte_index(&self.layout, self.byte_of(start), Affinity::Downstream);
        let to = Cursor::from_byte_index(&self.layout, self.byte_of(end), Affinity::Upstream);
        Selection::new(from, to)
            .geometry(&self.layout)
            .into_iter()
            .map(|(rect, _)| rect)
            .collect()
    }

    /// Where a code-point offset sits, on the side of a wrap point the
    /// affinity names: the caret's box.
    #[must_use]
    pub fn caret_box_at(&self, offset: u32, upstream: bool) -> BoundingBox {
        self.cursor(offset, upstream).geometry(&self.layout, 1.0)
    }

    /// The line a caret sits on, the first one being zero.
    #[must_use]
    pub fn line_of(&self, offset: u32, upstream: bool) -> usize {
        let caret = self.caret_box_at(offset, upstream);
        let middle = ((caret.y0 + caret.y1) / 2.0) as f32;
        let tops = self.line_tops();
        let lines = self.layout.len().max(1);
        tops.partition_point(|top| *top <= middle)
            .saturating_sub(1)
            .min(lines - 1)
    }

    /// How many lines the run is broken into, never fewer than one.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.layout.len().max(1)
    }

    /// The caret on line `line` nearest to `x`, and its affinity.
    #[must_use]
    pub fn at_line(&self, line: usize, x: f32) -> (u32, bool) {
        let tops = self.line_tops();
        let top = tops.get(line).copied().unwrap_or(0.0);
        let bottom = tops.get(line + 1).copied().unwrap_or(top);
        let cursor = Cursor::from_point(&self.layout, x, (top + bottom) / 2.0);
        (
            self.offset_of_byte(cursor.index()),
            cursor.affinity() == Affinity::Upstream,
        )
    }

    /// The start or the end of the wrapped line under a caret, and the
    /// affinity that keeps the caret on it (`PROSE-UI-159`).
    #[must_use]
    pub fn line_edge(&self, offset: u32, upstream: bool, end: bool) -> (u32, bool) {
        let here = Selection::new(self.cursor(offset, upstream), self.cursor(offset, upstream));
        let moved = if end {
            here.line_end(&self.layout, false)
        } else {
            here.line_start(&self.layout, false)
        };
        let focus = moved.focus();
        (
            self.offset_of_byte(focus.index()),
            focus.affinity() == Affinity::Upstream,
        )
    }

    /// The grapheme boundary before an offset, or `None` at the start.
    #[must_use]
    pub fn grapheme_before(&self, offset: u32) -> Option<u32> {
        let byte = self.byte_of(offset);
        let head = self.text.get(..byte)?;
        let (at, _) = head.grapheme_indices(true).next_back()?;
        Some(self.offset_of_byte(at))
    }

    /// The grapheme boundary after an offset, or `None` at the end.
    #[must_use]
    pub fn grapheme_after(&self, offset: u32) -> Option<u32> {
        let byte = self.byte_of(offset);
        let tail = self.text.get(byte..)?;
        let first = tail.graphemes(true).next()?;
        Some(self.offset_of_byte(byte + first.len()))
    }

    /// The start of the word before an offset, or `None` at the start.
    #[must_use]
    pub fn word_before(&self, offset: u32) -> Option<u32> {
        let byte = self.byte_of(offset);
        let head = self.text.get(..byte)?;
        if head.is_empty() {
            return None;
        }
        let at = head
            .split_word_bound_indices()
            .rev()
            .find(|(_, word)| word.chars().any(char::is_alphanumeric))
            .map_or(0, |(at, _)| at);
        Some(self.offset_of_byte(at))
    }

    /// The end of the word after an offset, or `None` at the end.
    #[must_use]
    pub fn word_after(&self, offset: u32) -> Option<u32> {
        let byte = self.byte_of(offset);
        let tail = self.text.get(byte..)?;
        if tail.is_empty() {
            return None;
        }
        let end = tail
            .split_word_bound_indices()
            .find(|(_, word)| word.chars().any(char::is_alphanumeric))
            .map_or(tail.len(), |(at, word)| at + word.len());
        Some(self.offset_of_byte(byte + end))
    }

    /// The word under an offset, as a code-point range. A space or a
    /// punctuation mark is a word of its own, as a double click takes it.
    #[must_use]
    pub fn word_around(&self, offset: u32) -> (u32, u32) {
        let byte = self.byte_of(offset);
        for (at, word) in self.text.split_word_bound_indices() {
            if byte < at + word.len() {
                return (
                    self.offset_of_byte(at),
                    self.offset_of_byte(at + word.len()),
                );
            }
        }
        (self.len(), self.len())
    }

    fn cursor(&self, offset: u32, upstream: bool) -> Cursor {
        let affinity = if upstream {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        Cursor::from_byte_index(&self.layout, self.byte_of(offset), affinity)
    }

    fn byte_of(&self, offset: u32) -> usize {
        let last = self.char_starts.last().copied().unwrap_or(0);
        self.char_starts
            .get(offset as usize)
            .copied()
            .unwrap_or(last) as usize
    }

    fn offset_of_byte(&self, byte: usize) -> u32 {
        let byte = u32::try_from(byte).unwrap_or(u32::MAX);
        let at = self.char_starts.partition_point(|start| *start < byte);
        u32::try_from(at).unwrap_or(u32::MAX).min(self.len())
    }
}

impl fmt::Debug for ParleyRunData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParleyRunData")
            .field("lines", &self.layout.len())
            .field("height", &self.layout.height())
            .field("chars", &self.len())
            .finish()
    }
}

/// A shaped run, shared between the side that shaped it and the side that
/// draws it.
///
/// Both threads share one heap, so the hand-over is a reference count and not
/// a copy. Two runs are equal when they are one shape. A block that was not
/// shaped again keeps its run, and the view keeps what it built from it.
#[derive(Clone, Debug)]
pub struct ShapedRun(Arc<ParleyRunData>);

impl ShapedRun {
    /// The run of `text` as `layout` shaped it, `indent_px` from the left of
    /// the content box.
    #[must_use]
    pub fn new(text: &str, layout: Layout<ParleyBrush>, indent_px: f32) -> Self {
        Self(Arc::new(ParleyRunData::new(text, layout, indent_px)))
    }

    /// The same, recording the style the text was shaped in.
    #[must_use]
    pub fn styled(text: &str, layout: Layout<ParleyBrush>, style: &EffectiveStyle) -> Self {
        let mut data = ParleyRunData::new(text, layout, style.indent_left_px);
        data.style = Some(style.clone());
        Self(Arc::new(data))
    }

    /// The same, recording the spans shaped bold or italic as well.
    #[must_use]
    pub fn marked(
        text: &str,
        layout: Layout<ParleyBrush>,
        style: &EffectiveStyle,
        marks: &[MarkedSpan],
    ) -> Self {
        let mut data = ParleyRunData::new(text, layout, style.indent_left_px);
        data.style = Some(style.clone());
        data.marks = marks.to_vec();
        Self(Arc::new(data))
    }

    /// The shaped data.
    #[must_use]
    pub fn data(&self) -> &ParleyRunData {
        &self.0
    }
}

impl PartialEq for ShapedRun {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One layout as the view receives it: the pages, the runs to draw on
/// them, and the indices the view reads them through.
///
/// It is a whole value, stamped with the version the paginator gave it. The
/// view keeps the newest it has seen and drops an older one
/// (`PROSE-UI-016`). The indices are built here, on the thread that lays
/// out, so taking a layout costs the view a pointer (`PROSE-UI-205`).
///
/// It carries the pages the view asked for, not the document
/// (`PROSE-125`). The page geometry and the page count are the document's,
/// so the column and the scroll range stay whole. The blocks, the runs and
/// the tables are those of the pages in [`ShapedLayout::pages`] alone.
#[derive(Clone, Debug, Default)]
pub struct ShapedLayout {
    pub version: u64,
    /// The page geometry of the document, with the blocks of the window.
    pub layout: Arc<PageLayout>,
    /// The run of every block the window draws text for: body, bands and
    /// notes alike.
    pub blocks: HashMap<TreeID, ShapedRun>,
    /// The number of every footnote the window draws, beside its text.
    pub notes: HashMap<Uuid, ShapedRun>,
    /// The cells of every table of the window, placed and shaped.
    pub tables: HashMap<TreeID, Arc<TableShape>>,
    /// Every block of the document, by its place in the flow. The Logic
    /// thread shares one map between the layouts it hands, and builds it
    /// again only when a block is added, removed or moved.
    order: Arc<HashMap<TreeID, usize>>,
    /// The place in the flow of the first block of `layout`.
    first_index: usize,
    /// The first and the last page the window carries.
    pages: (u32, u32),
    /// The first and the last block of the document that hold text.
    ends: Option<TextEnds>,
    /// The blocks each page of the window shows, by their place in the flow.
    /// A frame reads its pages and never the whole document (`PROSE-UI-007`).
    page_blocks: Vec<Vec<usize>>,
}

/// The two ends of the text of a document, which a selection of the whole
/// document runs between (`PROSE-UI-167`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEnds {
    pub first: TreeID,
    pub last: TreeID,
    /// The length of the last block, in code points.
    pub last_len: u32,
}

/// Where a window sits in the document it was cut from.
#[derive(Clone, Debug, Default)]
pub struct Window {
    /// The place in the flow of the first block the window carries.
    pub first_index: usize,
    /// The first and the last page the window carries.
    pub pages: (u32, u32),
    /// Every block of the document, by its place in the flow.
    pub order: Arc<HashMap<TreeID, usize>>,
    pub ends: Option<TextEnds>,
}

impl ShapedLayout {
    /// A layout of the whole document and its runs, with the indices built
    /// from them.
    #[must_use]
    pub fn new(
        version: u64,
        layout: Arc<PageLayout>,
        blocks: HashMap<TreeID, ShapedRun>,
        notes: HashMap<Uuid, ShapedRun>,
        tables: HashMap<TreeID, Arc<TableShape>>,
    ) -> Self {
        let ends = text_ends(layout.blocks.iter().map(|block| {
            let run = blocks.get(&block.block_id);
            (block.block_id, run.map(|run| run.data().len()))
        }));
        let window = Window {
            first_index: 0,
            pages: (0, layout.page_count.saturating_sub(1)),
            order: Arc::new(order_of(&layout.blocks)),
            ends,
        };
        Self {
            version,
            layout,
            blocks,
            notes,
            tables,
            ..Self::default()
        }
        .with_window(window)
    }

    /// The same layout, read as the window `window` describes.
    #[must_use]
    pub fn with_window(mut self, window: Window) -> Self {
        let (first, last) = window.pages;
        let mut page_blocks = vec![Vec::new(); last.saturating_sub(first) as usize + 1];
        for (local, block) in self.layout.blocks.iter().enumerate() {
            for page in block.page_span() {
                let at = page.checked_sub(first).map(|at| at as usize);
                if let Some(blocks) = at.and_then(|at| page_blocks.get_mut(at)) {
                    blocks.push(window.first_index + local);
                }
            }
        }
        self.order = window.order;
        self.first_index = window.first_index;
        self.pages = window.pages;
        self.ends = window.ends;
        self.page_blocks = page_blocks;
        self
    }

    /// The place of a block in the flow of the document, whether or not the
    /// window carries it.
    #[must_use]
    pub fn index_of(&self, block: &TreeID) -> Option<usize> {
        self.order.get(block).copied()
    }

    /// The block at a place in the flow, when the window carries it.
    #[must_use]
    pub fn block(&self, index: usize) -> Option<&BlockLayout> {
        self.layout.blocks.get(index.checked_sub(self.first_index)?)
    }

    /// The blocks the window carries, each with its place in the flow.
    pub fn window_blocks(&self) -> impl DoubleEndedIterator<Item = (usize, &BlockLayout)> {
        let first = self.first_index;
        self.layout
            .blocks
            .iter()
            .enumerate()
            .map(move |(local, block)| (first + local, block))
    }

    /// How many blocks the document holds, carried or not.
    #[must_use]
    pub fn block_count(&self) -> usize {
        self.order.len()
    }

    /// Whether the window carries a block.
    #[must_use]
    pub fn carries(&self, block: &TreeID) -> bool {
        self.index_of(block)
            .and_then(|index| self.block(index))
            .is_some()
    }

    /// The first and the last page the window carries.
    #[must_use]
    pub fn pages(&self) -> std::ops::RangeInclusive<u32> {
        self.pages.0..=self.pages.1
    }

    /// The two ends of the text of the document.
    #[must_use]
    pub fn ends(&self) -> Option<TextEnds> {
        self.ends
    }

    /// The indices of the blocks a page shows, in flow order. Empty for a
    /// page the window does not carry.
    #[must_use]
    pub fn blocks_on(&self, page: u32) -> &[usize] {
        page.checked_sub(self.pages.0)
            .and_then(|at| self.page_blocks.get(at as usize))
            .map_or(&[], Vec::as_slice)
    }
}

/// Every block of `blocks`, by its place in the flow.
#[must_use]
pub fn order_of(blocks: &[BlockLayout]) -> HashMap<TreeID, usize> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.block_id, index))
        .collect()
}

/// The first and the last block holding text, from blocks in flow order,
/// each with the length of its text when it has one.
///
/// It reads from both ends, so a document whose ends hold text answers at
/// once.
pub fn text_ends<I>(blocks: I) -> Option<TextEnds>
where
    I: DoubleEndedIterator<Item = (TreeID, Option<u32>)> + Clone,
{
    let first = blocks.clone().find_map(|(id, len)| len.map(|_| id))?;
    let (last, last_len) = blocks.rev().find_map(|(id, len)| Some((id, len?)))?;
    Some(TextEnds {
        first,
        last,
        last_len,
    })
}

/// A table as the view draws it: where each cell sits, and the text it holds
/// (`features/ambre-spec-prose-tables.md section 3`).
///
/// A merged region is one cell here, anchored at its top-left cell and as wide
/// and tall as the rectangle it covers. A covered cell is absent, so it is
/// neither measured nor painted (`PROSE-FEAT-048`, `PROSE-UI-147`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableShape {
    /// The top of each row from the table's top, and the bottom of the last.
    pub row_tops: Vec<f32>,
    pub cells: Vec<CellShape>,
}

/// One cell drawn, or one merged region drawn as a single cell.
#[derive(Clone, Debug, PartialEq)]
pub struct CellShape {
    /// The cell the text sits in: a region's top-left cell.
    pub cell: TreeID,
    pub x_px: f32,
    pub width_px: f32,
    /// The rows the cell spans, the last one excluded.
    pub rows: std::ops::Range<usize>,
    /// The text blocks the cell holds, each with its top from the top of the
    /// padded box, before the vertical alignment moves them.
    pub blocks: Vec<(TreeID, f32, ShapedRun)>,
    /// The height of that text, padding excluded: what the alignment places.
    pub content_px: f32,
    pub style: CellStyle,
}

/// What a cell stores about its own look (`ambre-spec-prose-schema.md
/// section 3`), in pixels and packed colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStyle {
    /// The fill as `0xRRGGBBAA`. None leaves the sheet showing.
    pub fill: Option<u32>,
    /// The stored border. None draws the theme's hairline (`PROSE-UI-181`).
    pub border: Option<CellBorderPx>,
    /// The inset on each of the four sides.
    pub padding_px: f32,
    pub valign: ambre_bus::app::prose::VAlign,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fill: None,
            border: None,
            padding_px: 0.0,
            valign: ambre_bus::app::prose::VAlign::Top,
        }
    }
}

/// The four border widths of a cell, and their colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellBorderPx {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    /// As `0xRRGGBBAA`. None takes the theme's border colour.
    pub color: Option<u32>,
}

// `PROSE-UI-113`: the run crosses from the thread that shapes to the thread
// that draws. A field bound to one thread or to a device fails this build.
const _: () = {
    const fn crosses<T: Send + Sync + 'static>() {}
    crosses::<ShapedRun>();
    crosses::<ParleyRunData>();
    crosses::<ShapedLayout>();
    crosses::<TableShape>();
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::doc;
    use crate::layout::measure::{EffectiveStyle, shipped_measurer};
    use ambre_bus::app::prose::ParagraphStyle;

    fn style() -> EffectiveStyle {
        EffectiveStyle::from_parts(
            &doc::DocDefaults::default(),
            &ParagraphStyle::default(),
            false,
        )
    }

    #[test]
    fn a_point_resolves_to_the_offset_the_caret_box_puts_there() {
        let mut measurer = shipped_measurer();
        let (_, run) = measurer.measure_run("h\u{e9}llo world", &style(), 400.0);
        let run = run.data();
        for offset in 0..=run.len() {
            let caret = run.caret_box(offset);
            let back = run.offset_at(caret.x0 as f32 + 0.1, (caret.y0 + caret.y1) as f32 / 2.0);
            assert_eq!(back, offset, "offset {offset}");
        }
    }

    #[test]
    fn a_range_covers_one_box_per_line_it_crosses() {
        let mut measurer = shipped_measurer();
        let text = "one two three four five six seven eight nine ten eleven";
        let (metrics, run) = measurer.measure_run(text, &style(), 90.0);
        assert!(metrics.line_count > 2);
        let whole = run.data().range_boxes(0, run.data().len());
        assert_eq!(whole.len(), metrics.line_count as usize);
        assert!(run.data().range_boxes(3, 3).is_empty());
    }

    #[test]
    fn a_shaped_layout_moves_to_another_thread_and_draws_there() {
        let (_, shaped) = crate::view::tests::laid_out(3, 10);
        let lines = std::thread::spawn(move || {
            shaped
                .blocks
                .values()
                .map(|run| run.data().line_tops().len())
                .sum::<usize>()
        })
        .join()
        .expect("the drawing thread returns");
        assert!(lines >= 3);
    }
}
