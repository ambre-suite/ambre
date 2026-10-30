//! The shaped run: what pagination shaped, handed to the view as it is
//! (`features/ambre-spec-prose-page-rendering.md section 2`).
//!
//! Pagination shapes every block to find where it breaks. The view draws and
//! hit-tests that same shape rather than shaping a second time on the thread
//! that paints (`PROSE-UI-006`, `PROSE-UI-114`). A run holds no GPU handle
//! and nothing bound to a thread, so it crosses by move (`PROSE-UI-113`).

use ambre_bus::app::grid::{Row, RowSpan, RowVec};
use ambre_bus::app::prose::{DocCount, DocOffset, PageIndex, PageSpan};
use ambre_core::Rgba;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use ambre_core::{
    ByteCount, ByteDelta, ByteOffset, ByteSpan, CharCount, DocPoint, DocPx, FootnoteId,
};
use ambre_draw::{BoundingBox, Layout, ParleyBrush};
use loro::TreeID;
use parley::{Affinity as ParleyAffinity, Cursor, Selection};
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::measure::{BlockLine, EffectiveStyle, MarkedSpan};
use crate::layout::pass::PageLayout;
use crate::layout::place::BlockLayout;

ambre_core::define_index! {
    /// A code-point offset into the run `ProseUiState::run_of` returns. It
    /// equals the document offset while no draft is pending, or while a fresh
    /// draft run is drawn. `ProseUiState::to_doc` is the bridge otherwise.
    pub struct RunOffset(u32);
    /// A number of code points in one run.
    pub struct RunCount;
    /// The signed gap between two offsets of one run.
    pub struct RunDelta(i64);
    /// A range of code points in one run.
    pub struct RunSpan;
    /// A list indexed by code-point offset in one run.
    pub struct RunVec;
}

impl RunOffset {
    /// The document offset of the same code point, where the run counts the
    /// text the document counts. That holds while no draft is pending, for a
    /// provisional run that holds the draft, and for an offset the Logic
    /// thread answered. `ProseUiState::to_doc` is the bridge otherwise.
    #[must_use]
    pub const fn as_doc(self) -> DocOffset {
        DocOffset::new(self.get())
    }

    /// The run offset of the same code point, under the condition that
    /// [`Self::as_doc`] states.
    #[must_use]
    pub const fn from_doc(offset: DocOffset) -> Self {
        Self::new(offset.get())
    }
}

impl RunCount {
    /// The same number of code points as the document counts them, under the
    /// condition that [`RunOffset::as_doc`] states.
    #[must_use]
    pub const fn as_doc(self) -> DocCount {
        DocCount::new(self.get())
    }

    /// The same number of code points as a run counts them, for a text the
    /// run holds whole.
    #[must_use]
    pub const fn from_chars(count: CharCount) -> Self {
        Self::new(count.get())
    }
}

ambre_core::define_index! {
    /// A block's place in the flow of the document, the first being zero.
    /// A window of the layout carries a stretch of them.
    pub struct FlowIndex(usize);
    /// A number of blocks of the flow.
    pub struct FlowCount;
    /// The signed gap between two places of the flow.
    pub struct FlowDelta(isize);
    /// A stretch of the flow.
    pub struct FlowSpan;
    /// A list indexed by place in the flow.
    pub struct FlowVec;
}

ambre_core::define_index! {
    /// A cell of one table shape, in the order `TableShape::cells` holds.
    pub struct ShapeCell(usize);
    /// A number of cells of one table shape.
    pub struct ShapeCellCount;
    /// The signed gap between two cells of one table shape.
    pub struct ShapeCellDelta(isize);
    /// A run of cells of one table shape.
    pub struct ShapeCellSpan;
    /// A list indexed by cell of one table shape.
    pub struct ShapeCellVec;
}

ambre_core::define_index! {
    /// A text block of one cell shape, the cell's first being zero.
    pub struct CellBlock(usize);
    /// A number of text blocks of one cell.
    pub struct CellBlockCount;
    /// The signed gap between two text blocks of one cell.
    pub struct CellBlockDelta(isize);
    /// A run of text blocks of one cell.
    pub struct CellBlockSpan;
    /// A list indexed by text block of one cell.
    pub struct CellBlockVec;
}

/// The side of a wrap point a caret stands on. One offset at a soft wrap
/// is both the end of one line and the start of the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    /// The caret stands at the start of the line after the offset.
    #[default]
    Downstream,
    /// The caret stands at the end of the line before the offset.
    Upstream,
}

/// One end of a wrapped line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineEdge {
    Start,
    End,
}

impl Affinity {
    /// The affinity parley answers, read into this crate's own type.
    fn of(affinity: ParleyAffinity) -> Self {
        match affinity {
            ParleyAffinity::Upstream => Self::Upstream,
            ParleyAffinity::Downstream => Self::Downstream,
        }
    }

    /// The affinity parley takes, at the one call into it.
    fn to_parley(self) -> ParleyAffinity {
        match self {
            Self::Upstream => ParleyAffinity::Upstream,
            Self::Downstream => ParleyAffinity::Downstream,
        }
    }
}

ambre_core::define_index! {
    /// A byte of the text one shaped segment holds, which starts where the
    /// segment was shaped from. `Segment::to_run` takes it to the run's text.
    pub struct LocalByte(usize);
    /// A number of bytes in one segment's text.
    pub struct LocalByteCount;
    /// The signed gap between two bytes of one segment's text.
    pub struct LocalByteDelta(isize);
    /// A range of bytes in one segment's text.
    pub struct LocalByteSpan;
    /// A list indexed by byte in one segment's text.
    pub struct LocalByteVec;
}

ambre_core::define_index! {
    /// A line of one run, counted across its segments, the first being zero.
    pub struct RunLine(usize);
    /// A number of lines of one run.
    pub struct RunLineCount;
    /// The signed gap between two lines of one run.
    pub struct RunLineDelta(isize);
    /// A run of lines of one run.
    pub struct RunLineSpan;
    /// A list indexed by line of one run.
    pub struct RunLineVec;
}

impl RunLine {
    /// The line of the block whose whole text this run shapes. A text block
    /// is one run, so the paginator cuts it at the run's own lines.
    #[must_use]
    pub(crate) fn in_block(self) -> BlockLine {
        BlockLine::from_usize(self.to_usize())
    }
}

ambre_core::define_index! {
    /// A line of one segment's layout, as the shaper numbers it. A segment
    /// may keep its layout's lines from a later one on, so it is no line of
    /// the run. `ParleyRunData::lines` maps one to the other.
    pub struct LocalLine(usize);
    /// A number of lines of one segment's layout.
    pub struct LocalLineCount;
    /// The signed gap between two lines of one segment's layout.
    pub struct LocalLineDelta(isize);
    /// A run of lines of one segment's layout.
    pub struct LocalLineSpan;
    /// A list indexed by line of one segment's layout.
    pub struct LocalLineVec;
}

ambre_core::define_index! {
    /// A segment of one run, the first being zero.
    pub struct RunSegment(usize);
    /// A number of segments of one run.
    pub struct RunSegmentCount;
    /// The signed gap between two segments of one run.
    pub struct RunSegmentDelta(isize);
    /// A stretch of segments of one run.
    pub struct RunSegmentSpan;
    /// A list indexed by segment of one run.
    pub struct RunSegmentVec;
}

/// One stretch of a run's lines, shaped on its own (`PROSE-128`).
///
/// A run shaped whole is one segment. An edit re-shapes a window from the
/// line before the one it touched, and keeps the lines around it where they
/// were shaped.
#[derive(Clone)]
struct Segment {
    /// The glyphs, their clusters and the line boxes, as the shaper left them.
    layout: Arc<Layout<ParleyBrush>>,
    /// The byte the first kept line starts at, in the run's text.
    start: ByteOffset,
    /// The same byte in the layout's own text.
    local_start: LocalByte,
    /// The layout's first kept line. The lines above it were shaped before
    /// an edit, and an earlier segment holds what replaced them.
    skip: LocalLine,
    /// How many of the layout's lines the run keeps from `skip` on. The
    /// lines after them were replaced the same way.
    lines: LocalLineCount,
    /// Its first kept line, among the run's lines.
    first_line: RunLine,
}

impl Segment {
    /// A byte of the layout's text, in the run's text.
    fn to_run(&self, local: LocalByte) -> ByteOffset {
        self.start + ByteCount::new(local.count_from(self.local_start).get())
    }

    /// A byte of the run's text, in the layout's text.
    fn to_local(&self, byte: ByteOffset) -> LocalByte {
        self.local_start + LocalByteCount::new(byte.count_from(self.start).get())
    }

    /// The line of the layout's text, `parley` numbering it.
    fn layout_line(&self, line: LocalLine) -> Option<parley::Line<'_, ParleyBrush>> {
        self.layout.get(line.to_usize())
    }

    /// The lines of the layout the run keeps.
    fn kept(&self) -> LocalLineSpan {
        LocalLineSpan::from_count(self.skip, self.lines)
    }
}

/// Where a window shaped after an edit meets the lines shaped before it
/// (`PROSE-UI-215`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Resume {
    /// How many of the window's lines the run keeps.
    pub kept: LocalLineCount,
    /// The first line of the previous run kept after them.
    pub line: RunLine,
    /// How many bytes the edit added to the text, negative when it removed.
    pub delta: ByteDelta,
}

/// Where one line of the run is: its segment, its index there, its top, and
/// the shift that moves its segment's coordinates to the run's.
#[derive(Clone, Copy, Debug)]
struct LineAt {
    segment: RunSegment,
    local: LocalLine,
    top: DocPx,
    shift: DocPx,
}

/// One block's text, shaped and broken into lines: enough to draw and to
/// hit-test.
pub struct ParleyRunData {
    /// The shaped stretches of the text, in order. Never empty.
    segments: RunSegmentVec<Segment>,
    /// Every line of the run, in order, across the segments.
    lines: RunLineVec<LineAt>,
    /// The height every line takes, summed as the shaper sums it.
    height: DocPx,
    /// The byte offset each code point starts at, the text's length last. An
    /// offset in the document counts code points, and the shaper counts bytes.
    char_starts: RunVec<ByteOffset>,
    /// How far the text sits from the left of the content box. It is the
    /// block's left indent, which the width it was broken to left out.
    indent: DocPx,
    /// The text the glyphs were shaped from. The caret walks its graphemes
    /// and its words, which the glyphs alone do not tell apart.
    text: String,
    /// The style the text was shaped in, so a draft typed into the block is
    /// shaped alike (`PROSE-UI-134`). `None` for a run shaped without one.
    style: Option<EffectiveStyle>,
    /// The spans shaped bold or italic, in bytes, so a draft typed after one
    /// takes its marks (`PROSE-UI-208`).
    marks: Vec<MarkedSpan>,
    /// The width the text was broken to, when pagination shaped it. Only such
    /// a run is re-shaped in part (`PROSE-128`).
    width: Option<DocPx>,
}

impl ParleyRunData {
    /// The run of `text` as `layout` shaped it, `indent` from the left of
    /// the content box.
    #[must_use]
    pub fn new(text: &str, layout: Layout<ParleyBrush>, indent: DocPx) -> Self {
        let mut char_starts: RunVec<ByteOffset> = text
            .char_indices()
            .map(|(at, _)| ByteOffset::from_usize(at))
            .collect();
        char_starts.push(ByteCount::from_len(text.len()).as_end());
        let lines = LocalLineCount::from_len(layout.len());
        let mut data = Self {
            segments: RunSegmentVec::from(vec![Segment {
                layout: Arc::new(layout),
                start: ByteOffset::ZERO,
                local_start: LocalByte::ZERO,
                skip: LocalLine::ZERO,
                lines,
                first_line: RunLine::ZERO,
            }]),
            lines: RunLineVec::new(),
            height: DocPx::ZERO,
            char_starts,
            indent,
            text: text.to_owned(),
            style: None,
            marks: Vec::new(),
            width: None,
        };
        data.index_lines();
        data
    }

    /// The run `previous` holds with its lines from `cut` on replaced by
    /// `window`, which is `text` shaped from the start of that line
    /// (`PROSE-128`). The lines above `cut` keep their glyphs.
    ///
    /// With `resume`, the run keeps only the window's first `kept` lines.
    /// The lines of `previous` from `line` on follow them, moved by `delta`
    /// bytes (`PROSE-UI-215`). Without it, the window runs to the end.
    ///
    /// The caller has checked that `text` and `previous` agree up to `cut`,
    /// and from `line` on, in text and in marks.
    pub(crate) fn with_window(
        previous: &Self,
        cut: RunLine,
        (text, marks): (&str, &[MarkedSpan]),
        window: Layout<ParleyBrush>,
        resume: Option<Resume>,
    ) -> Option<Self> {
        let start = previous.line_start(cut)?;
        let mut segments =
            RunSegmentVec::from(Vec::with_capacity(previous.segments.len().to_usize() + 2));
        for segment in &previous.segments {
            if segment.first_line >= cut {
                break;
            }
            let mut kept = segment.clone();
            let above = LocalLineCount::new(cut.count_from(segment.first_line).get());
            kept.lines = kept.lines.min(above);
            segments.push(kept);
        }
        let lines = resume.map_or(LocalLineCount::from_len(window.len()), |resume| resume.kept);
        segments.push(Segment {
            layout: Arc::new(window),
            start,
            local_start: LocalByte::ZERO,
            skip: LocalLine::ZERO,
            lines,
            first_line: cut,
        });
        if let Some(resume) = resume {
            let at = previous.lines.get(resume.line)?;
            let later = previous.segments.iter_enumerated();
            for (index, segment) in later.filter(|(index, _)| *index >= at.segment) {
                let mut kept = segment.clone();
                if index == at.segment {
                    if at.local < segment.skip {
                        return None;
                    }
                    let dropped = at.local.count_from(segment.skip);
                    if dropped > segment.lines {
                        return None;
                    }
                    kept.skip = at.local;
                    kept.lines = segment.lines.saturating_sub(dropped);
                    kept.local_start =
                        LocalByte::from_usize(segment.layout_line(at.local)?.text_range().start);
                    kept.start = segment.to_run(kept.local_start);
                }
                kept.start = kept.start.checked_offset(resume.delta)?;
                segments.push(kept);
            }
        }
        let mut data = Self::new(text, Layout::new(), previous.indent);
        data.segments = segments;
        data.style = previous.style.clone();
        data.marks = marks.to_vec();
        data.width = previous.width;
        data.index_lines();
        Some(data)
    }

    /// Record the style, the marks and the width the text was shaped in.
    pub(crate) fn shaped_in(&mut self, style: &EffectiveStyle, marks: &[MarkedSpan], width: DocPx) {
        self.style = Some(style.clone());
        self.marks = marks.to_vec();
        self.width = Some(width);
    }

    /// The width the text was broken to, when pagination shaped it.
    pub(crate) fn width(&self) -> Option<DocPx> {
        self.width
    }

    /// How many shaped stretches the run holds: one for a run shaped whole,
    /// or shaped again from its first line to its end.
    #[must_use]
    pub fn segment_count(&self) -> RunSegmentCount {
        self.segments.len()
    }

    /// How many lines the run keeps of the layouts `previous` does not hold:
    /// the lines an edit shaped.
    #[cfg(test)]
    pub(crate) fn fresh_lines(&self, previous: &Self) -> usize {
        self.segments
            .iter()
            .filter(|segment| {
                !previous
                    .segments
                    .iter()
                    .any(|old| Arc::ptr_eq(&old.layout, &segment.layout))
            })
            .map(|segment| segment.lines.to_usize())
            .sum()
    }

    /// Number the lines across the segments, and sum their heights.
    ///
    /// The shaper rounds each baseline from the height summed above it. A
    /// line keeps its pixel only when its shift is rounded the same way. It
    /// is the rounded top in the run less the rounded top in its segment.
    fn index_lines(&mut self) {
        let mut lines = RunLineVec::new();
        let mut top = DocPx::ZERO;
        let mut y = 0.0_f64;
        for index in self.segments.positions() {
            let Some(segment) = self.segments.get_mut(index) else {
                continue;
            };
            segment.first_line = lines.len().as_end();
            let mut local_y: f64 = segment
                .layout
                .lines()
                .take(segment.skip.to_usize())
                .map(|line| f64::from(line.metrics().line_height))
                .sum();
            let kept = segment
                .kept()
                .into_iter()
                .filter_map(|local| segment.layout_line(local).map(|line| (local, line)));
            for (local, line) in kept {
                let height = line.metrics().line_height;
                lines.push(LineAt {
                    segment: index,
                    local,
                    top,
                    shift: DocPx::new((y.round() - local_y.round()) as f32),
                });
                top += DocPx::new(height);
                y += f64::from(height);
                local_y += f64::from(height);
            }
        }
        self.lines = lines;
        self.height = DocPx::new(y as f32);
    }

    /// The byte line `line` starts at, in the run's text.
    pub(crate) fn line_start(&self, line: RunLine) -> Option<ByteOffset> {
        let at = self.lines.get(line)?;
        let segment = self.segments.get(at.segment)?;
        let start = segment.layout_line(at.local)?.text_range().start;
        Some(segment.to_run(LocalByte::from_usize(start)))
    }

    /// The line a byte of the text falls on, the last one past the text.
    pub(crate) fn line_at_byte(&self, byte: ByteOffset) -> RunLine {
        let last = self.last_line();
        self.lines
            .iter()
            .position(|at| {
                self.segments.get(at.segment).is_some_and(|segment| {
                    segment.layout_line(at.local).is_some_and(|line| {
                        let range = LocalByteSpan::from_usize_range(line.text_range());
                        ByteSpan::new(segment.to_run(range.start), segment.to_run(range.end))
                            .contains(byte)
                    })
                })
            })
            .map_or(last, RunLine::from_usize)
    }

    /// The last line of the run, the first when it holds none.
    fn last_line(&self) -> RunLine {
        self.lines.len().as_end().prev().unwrap_or(RunLine::ZERO)
    }

    /// The marks of the code point before `offset`, its range aside. Text
    /// typed at `offset` takes them, since each expands after its range
    /// (`ambre-spec-prose-schema.md section 3`).
    #[must_use]
    pub fn marks_before(&self, offset: RunOffset) -> MarkedSpan {
        let Some(previous) = offset.checked_sub(RunCount::ONE) else {
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
    pub fn indent(&self) -> DocPx {
        self.indent
    }

    /// The height the shaped text takes.
    #[must_use]
    pub fn height(&self) -> DocPx {
        self.height
    }

    /// The widest line, its trailing spaces counted.
    #[must_use]
    pub fn full_width(&self) -> DocPx {
        DocPx::new(
            self.lines()
                .map(|(line, _)| line.metrics().advance)
                .fold(0.0, f32::max),
        )
    }

    /// Every line, in order, with the vertical shift that moves its glyphs
    /// from where its segment put them to where the run draws them.
    pub fn lines(&self) -> impl Iterator<Item = (parley::Line<'_, ParleyBrush>, DocPx)> + '_ {
        self.lines
            .positions()
            .into_iter()
            .filter_map(|index| self.line(index))
    }

    /// Line `index`, with its vertical shift.
    #[must_use]
    pub fn line(&self, index: RunLine) -> Option<(parley::Line<'_, ParleyBrush>, DocPx)> {
        let at = self.lines.get(index)?;
        let line = self.segments.get(at.segment)?.layout_line(at.local)?;
        Some((line, at.shift))
    }

    /// How many code points the run holds.
    #[must_use]
    pub fn len(&self) -> RunCount {
        self.char_starts.len().saturating_sub(RunCount::ONE)
    }

    /// Whether the run holds no text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len().is_zero()
    }

    /// The top of each line, relative to the top of the run, the run's height
    /// last.
    #[must_use]
    pub fn line_tops(&self) -> Vec<DocPx> {
        let mut tops: Vec<DocPx> = self.lines.iter().map(|at| at.top).collect();

        tops.push(self.height);
        tops
    }

    /// The code-point offset under a point inside the run, measured from the
    /// run's top-left corner.
    #[must_use]
    pub fn offset_at(&self, at: DocPoint) -> RunOffset {
        let index = self
            .lines
            .as_slice()
            .partition_point(|line| line.top <= at.y)
            .saturating_sub(1);
        let Some((segment, local_y)) = self.on_line(RunLine::from_usize(index), at.y) else {
            return RunOffset::ZERO;
        };
        let cursor = Cursor::from_point(&segment.layout, at.x.get(), local_y.get());
        self.offset_of_byte(segment.to_run(LocalByte::from_usize(cursor.index())))
    }

    /// Where a code-point offset sits inside the run: the caret's box.
    #[must_use]
    pub fn caret_box(&self, offset: RunOffset) -> BoundingBox {
        self.caret_box_at(offset, Affinity::Downstream)
    }

    /// The boxes a code-point range covers, one per line it crosses. Empty
    /// for an empty range.
    #[must_use]
    pub fn range_boxes(&self, span: RunSpan) -> Vec<BoundingBox> {
        if span.is_empty() {
            return Vec::new();
        }
        let (from, to) = (self.byte_of(span.start), self.byte_of(span.end));
        let mut boxes = Vec::new();
        for (index, segment) in self.segments.iter_enumerated() {
            let segment_end = self.segment_end(index);
            let local_from = from.max(segment.start);
            let local_to = to.min(segment_end);
            if local_from >= local_to {
                continue;
            }
            let anchor = Cursor::from_byte_index(
                &segment.layout,
                segment.to_local(local_from).to_usize(),
                ParleyAffinity::Downstream,
            );
            let focus = Cursor::from_byte_index(
                &segment.layout,
                segment.to_local(local_to).to_usize(),
                ParleyAffinity::Upstream,
            );
            for (rect, _) in Selection::new(anchor, focus).geometry(&segment.layout) {
                if let Some(line) = self.line_under(index, &rect) {
                    boxes.push(shifted(rect, line.shift));
                }
            }
        }
        boxes
    }

    /// Where a code-point offset sits, on the side of a wrap point the
    /// affinity names: the caret's box.
    #[must_use]
    pub fn caret_box_at(&self, offset: RunOffset, affinity: Affinity) -> BoundingBox {
        let (index, cursor) = self.cursor(offset, affinity);
        let Some(segment) = self.segments.get(index) else {
            return BoundingBox::default();
        };
        let caret = cursor.geometry(&segment.layout, 1.0);
        let shift = self
            .line_under(index, &caret)
            .map_or(DocPx::ZERO, |line| line.shift);
        self.past_indent(shifted(caret, shift))
    }

    /// A caret in an empty run, moved past the first-line indent: the run
    /// has no character for the caret to stand before (`PROSE-FEAT-309`).
    fn past_indent(&self, mut caret: BoundingBox) -> BoundingBox {
        if !self.text.is_empty() {
            return caret;
        }
        let indent = self
            .line(RunLine::ZERO)
            .and_then(|(line, _)| {
                line.items().find_map(|item| match item {
                    ambre_draw::PositionedLayoutItem::InlineBox(b) => Some(b.x + b.width),
                    ambre_draw::PositionedLayoutItem::GlyphRun(_) => None,
                })
            })
            .unwrap_or(0.0);
        let shift = f64::from(indent) - caret.x0;
        if shift > 0.0 {
            caret.x0 += shift;
            caret.x1 += shift;
        }
        caret
    }

    /// The line a caret sits on, the first one being zero.
    #[must_use]
    pub fn line_of(&self, offset: RunOffset, affinity: Affinity) -> RunLine {
        let caret = self.caret_box_at(offset, affinity);
        let middle = DocPx::new(((caret.y0 + caret.y1) / 2.0) as f32);
        let under = self
            .lines
            .as_slice()
            .partition_point(|at| at.top <= middle)
            .saturating_sub(1);
        RunLine::from_usize(under).min(self.last_line())
    }

    /// How many lines the run is broken into, never fewer than one.
    #[must_use]
    pub fn line_count(&self) -> RunLineCount {
        self.lines.len().max(RunLineCount::ONE)
    }

    /// The caret on line `line` nearest to `x`, and its affinity.
    #[must_use]
    pub fn at_line(&self, line: RunLine, x: DocPx) -> (RunOffset, Affinity) {
        let index = line.min(self.last_line());
        let Some(at) = self.lines.get(index) else {
            return (RunOffset::ZERO, Affinity::Downstream);
        };
        let Some(segment) = self.segments.get(at.segment) else {
            return (RunOffset::ZERO, Affinity::Downstream);
        };
        let Some(local) = segment.layout_line(at.local) else {
            return (RunOffset::ZERO, Affinity::Downstream);
        };
        let metrics = local.metrics();
        let cursor = Cursor::from_point(
            &segment.layout,
            x.get(),
            (metrics.block_min_coord + metrics.block_max_coord) / 2.0,
        );
        (
            self.offset_of_byte(segment.to_run(LocalByte::from_usize(cursor.index()))),
            Affinity::of(cursor.affinity()),
        )
    }

    /// The start or the end of the wrapped line under a caret, and the
    /// affinity that keeps the caret on it (`PROSE-UI-159`).
    #[must_use]
    pub fn line_edge(
        &self,
        offset: RunOffset,
        affinity: Affinity,
        edge: LineEdge,
    ) -> (RunOffset, Affinity) {
        let (index, cursor) = self.cursor(offset, affinity);
        let Some(segment) = self.segments.get(index) else {
            return (offset, affinity);
        };
        let here = Selection::new(cursor, cursor);
        let moved = match edge {
            LineEdge::End => here.line_end(&segment.layout, false),
            LineEdge::Start => here.line_start(&segment.layout, false),
        };
        let focus = moved.focus();
        (
            self.offset_of_byte(segment.to_run(LocalByte::from_usize(focus.index()))),
            Affinity::of(focus.affinity()),
        )
    }

    /// The grapheme boundary before an offset, or `None` at the start.
    #[must_use]
    pub fn grapheme_before(&self, offset: RunOffset) -> Option<RunOffset> {
        let byte = self.byte_of(offset);
        let head = self.text.get(..byte.to_usize())?;
        let (at, _) = head.grapheme_indices(true).next_back()?;
        Some(self.offset_of_byte(ByteOffset::from_usize(at)))
    }

    /// The grapheme boundary after an offset, or `None` at the end.
    #[must_use]
    pub fn grapheme_after(&self, offset: RunOffset) -> Option<RunOffset> {
        let byte = self.byte_of(offset);
        let tail = self.text.get(byte.to_usize()..)?;
        let first = tail.graphemes(true).next()?;
        Some(self.offset_of_byte(byte + ByteCount::from_len(first.len())))
    }

    /// The start of the word before an offset, or `None` at the start.
    #[must_use]
    pub fn word_before(&self, offset: RunOffset) -> Option<RunOffset> {
        let byte = self.byte_of(offset);
        let head = self.text.get(..byte.to_usize())?;
        if head.is_empty() {
            return None;
        }
        let at = head
            .split_word_bound_indices()
            .rev()
            .find(|(_, word)| word.chars().any(char::is_alphanumeric))
            .map_or(0, |(at, _)| at);
        Some(self.offset_of_byte(ByteOffset::from_usize(at)))
    }

    /// The end of the word after an offset, or `None` at the end.
    #[must_use]
    pub fn word_after(&self, offset: RunOffset) -> Option<RunOffset> {
        let byte = self.byte_of(offset);
        let tail = self.text.get(byte.to_usize()..)?;
        if tail.is_empty() {
            return None;
        }
        let end = tail
            .split_word_bound_indices()
            .find(|(_, word)| word.chars().any(char::is_alphanumeric))
            .map_or(tail.len(), |(at, word)| at + word.len());
        Some(self.offset_of_byte(byte + ByteCount::from_len(end)))
    }

    /// The word under an offset, as a code-point range. A space or a
    /// punctuation mark is a word of its own, as a double click takes it.
    #[must_use]
    pub fn word_around(&self, offset: RunOffset) -> RunSpan {
        let byte = self.byte_of(offset).to_usize();
        for (at, word) in self.text.split_word_bound_indices() {
            if byte < at + word.len() {
                return RunSpan::new(
                    self.offset_of_byte(ByteOffset::from_usize(at)),
                    self.offset_of_byte(ByteOffset::from_usize(at + word.len())),
                );
            }
        }
        let end = self.len().as_end();
        RunSpan::new(end, end)
    }

    /// The segment a caret falls in, and the caret there. A byte where one
    /// segment ends and the next starts is the end of the first upstream,
    /// and the start of the second downstream.
    fn cursor(&self, offset: RunOffset, affinity: Affinity) -> (RunSegment, Cursor) {
        let upstream = affinity == Affinity::Upstream;
        let byte = self.byte_of(offset);
        let after = self
            .segments
            .as_slice()
            .partition_point(|segment| segment.start <= byte);
        let mut index = RunSegment::from_usize(after.saturating_sub(1));
        let starts_here = self.segments.get(index).is_some_and(|s| s.start == byte);
        if let Some(before) = index.prev().filter(|_| upstream && starts_here) {
            index = before;
        }
        // `segments` is never empty, so the first one always answers. The
        // default caret only stands in for a run built without one.
        let Some(segment) = self
            .segments
            .get(index)
            .or_else(|| self.segments.get(RunSegment::ZERO))
        else {
            return (RunSegment::ZERO, Cursor::default());
        };
        let local = segment.to_local(byte);
        (
            index,
            Cursor::from_byte_index(&segment.layout, local.to_usize(), affinity.to_parley()),
        )
    }

    /// The byte segment `index` stops at: where the next one starts, or the
    /// end of the text.
    fn segment_end(&self, index: RunSegment) -> ByteOffset {
        self.segments
            .get(index.next())
            .map_or(ByteCount::from_len(self.text.len()).as_end(), |next| {
                next.start
            })
    }

    /// The run's line that a box of segment `index` sits on. The middle of
    /// the box picks it, among the lines the run keeps of that segment.
    fn line_under(&self, index: RunSegment, rect: &BoundingBox) -> Option<LineAt> {
        let segment = self.segments.get(index)?;
        let middle = ((rect.y0 + rect.y1) / 2.0) as f32;
        let kept = LocalLineSpan::from_count(segment.skip, segment.lines.max(LocalLineCount::ONE));
        let local = kept
            .into_iter()
            .find(|&local| {
                segment
                    .layout_line(local)
                    .is_some_and(|line| middle < line.metrics().block_max_coord)
            })
            .or(kept.last())?;
        let below = RunLineCount::new(local.count_from(segment.skip).get());
        self.lines.get(segment.first_line + below).copied()
    }

    /// Segment of line `index`, and `y` moved into its coordinates and held
    /// inside that line.
    fn on_line(&self, index: RunLine, y: DocPx) -> Option<(&Segment, DocPx)> {
        let last = self.lines.len().as_end().prev()?;
        let at = self.lines.get(index.min(last))?;
        let segment = self.segments.get(at.segment)?;
        let metrics = *segment.layout_line(at.local)?.metrics();
        let local = y - at.shift;
        let low = DocPx::new(metrics.block_min_coord);
        let high = DocPx::new((metrics.block_max_coord - 0.01).max(metrics.block_min_coord));
        Some((segment, local.clamp(low, high)))
    }

    /// The byte a code point of the run starts at, the text's length past
    /// the end.
    pub(crate) fn byte_of(&self, offset: RunOffset) -> ByteOffset {
        let last = self.char_starts.iter().last().copied().unwrap_or_default();
        self.char_starts.get(offset).copied().unwrap_or(last)
    }

    /// The code point a byte of the run's text falls in, held to the end.
    pub(crate) fn offset_of_byte(&self, byte: ByteOffset) -> RunOffset {
        let at = self
            .char_starts
            .as_slice()
            .partition_point(|start| *start < byte);
        RunOffset::from_usize(at).min(self.len().as_end())
    }
}

/// A box moved down by `shift`.
fn shifted(mut rect: BoundingBox, shift: DocPx) -> BoundingBox {
    rect.y0 += f64::from(shift.get());
    rect.y1 += f64::from(shift.get());
    rect
}

impl fmt::Debug for ParleyRunData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParleyRunData")
            .field("lines", &self.lines.len())
            .field("segments", &self.segments.len())
            .field("height", &self.height)
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
    /// The run of `text` as `layout` shaped it, `indent` from the left of
    /// the content box.
    #[must_use]
    pub fn new(text: &str, layout: Layout<ParleyBrush>, indent: DocPx) -> Self {
        Self(Arc::new(ParleyRunData::new(text, layout, indent)))
    }

    /// The same, recording the style the text was shaped in.
    #[must_use]
    pub fn styled(text: &str, layout: Layout<ParleyBrush>, style: &EffectiveStyle) -> Self {
        let mut data = ParleyRunData::new(text, layout, style.indent_left);
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
        let mut data = ParleyRunData::new(text, layout, style.indent_left);
        data.style = Some(style.clone());
        data.marks = marks.to_vec();
        Self(Arc::new(data))
    }

    /// A run over data the measurer assembled.
    pub(crate) fn from_data(data: ParleyRunData) -> Self {
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

/// The stamp the paginator puts on each layout it shapes. It only grows, so
/// the view keeps the newest layout it was handed and drops an older one
/// (`PROSE-UI-016`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutVersion(u64);

impl LayoutVersion {
    /// The version before any layout was shaped.
    pub const FIRST: Self = Self(0);

    /// The version `value`.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The number the version holds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The version after this one, held at the last one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
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
    pub version: LayoutVersion,
    /// The page geometry of the document, with the blocks of the window.
    pub layout: Arc<PageLayout>,
    /// The run of every block the window draws text for: body, bands and
    /// notes alike.
    pub blocks: HashMap<TreeID, ShapedRun>,
    /// The number of every footnote the window draws, beside its text.
    pub notes: HashMap<FootnoteId, ShapedRun>,
    /// The cells of every table of the window, placed and shaped.
    pub tables: HashMap<TreeID, Arc<TableShape>>,
    /// Where each text block a cell of `tables` holds sits: its table, its
    /// cell and its place in the cell.
    cells: HashMap<TreeID, CellAt>,
    /// Every block of the document, by its place in the flow. The Logic
    /// thread shares one map between the layouts it hands, and builds it
    /// again only when a block is added, removed or moved.
    order: Arc<HashMap<TreeID, FlowIndex>>,
    /// The place in the flow of the first block of `layout`.
    first_index: FlowIndex,
    /// The pages the window carries.
    pages: PageSpan,
    /// The first and the last block of the document that hold text.
    ends: Option<TextEnds>,
    /// The blocks each page of the window shows, by their place in the flow.
    /// A frame reads its pages and never the whole document (`PROSE-UI-007`).
    page_blocks: WindowPageVec<Vec<FlowIndex>>,
}

ambre_core::define_index! {
    /// A page of the window a layout carries, its first page being zero.
    pub struct WindowPage(usize);
    /// A number of pages of the window.
    pub struct WindowPageCount;
    /// The signed gap between two pages of the window.
    pub struct WindowPageDelta(isize);
    /// A stretch of pages of the window.
    pub struct WindowPageSpan;
    /// A list indexed by page of the window.
    pub struct WindowPageVec;
}

/// Where a text block sits in the tables of a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellAt {
    table: TreeID,
    cell: ShapeCell,
    block: CellBlock,
}

/// The two ends of the text of a document, which a selection of the whole
/// document runs between (`PROSE-UI-167`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEnds {
    pub first: TreeID,
    pub last: TreeID,
    /// The length of the last block, in code points.
    pub last_len: RunCount,
}

/// Where a window sits in the document it was cut from.
#[derive(Clone, Debug, Default)]
pub struct Window {
    /// The place in the flow of the first block the window carries.
    pub first_index: FlowIndex,
    /// The pages the window carries.
    pub pages: PageSpan,
    /// Every block of the document, by its place in the flow.
    pub order: Arc<HashMap<TreeID, FlowIndex>>,
    pub ends: Option<TextEnds>,
}

impl ShapedLayout {
    /// A layout of the whole document and its runs, with the indices built
    /// from them.
    #[must_use]
    pub fn new(
        version: LayoutVersion,
        layout: Arc<PageLayout>,
        blocks: HashMap<TreeID, ShapedRun>,
        notes: HashMap<FootnoteId, ShapedRun>,
        tables: HashMap<TreeID, Arc<TableShape>>,
    ) -> Self {
        let ends = text_ends(layout.blocks.iter().map(|block| {
            let run = blocks.get(&block.block_id);
            (block.block_id, run.map(|run| run.data().len()))
        }));
        let window = Window {
            first_index: FlowIndex::ZERO,
            pages: layout.page_count.span(),
            order: Arc::new(order_of(&layout.blocks)),
            ends,
        };
        let cells = tables
            .iter()
            .flat_map(|(table, shape)| {
                shape
                    .cells
                    .iter()
                    .enumerate()
                    .flat_map(move |(cell, held)| {
                        held.blocks
                            .iter()
                            .enumerate()
                            .map(move |(block, (id, _, _))| {
                                (
                                    *id,
                                    CellAt {
                                        table: *table,
                                        cell: ShapeCell::from_usize(cell),
                                        block: CellBlock::from_usize(block),
                                    },
                                )
                            })
                    })
            })
            .collect();
        Self {
            version,
            layout,
            blocks,
            notes,
            tables,
            cells,
            ..Self::default()
        }
        .with_window(window)
    }

    /// The cell that holds text block `block`, and its run there.
    #[must_use]
    pub fn cell_run(&self, block: &TreeID) -> Option<(&CellShape, &ShapedRun)> {
        let at = self.cells.get(block)?;
        let cell = self.tables.get(&at.table)?.cells.get(at.cell.to_usize())?;
        let (_, _, run) = cell.blocks.get(at.block.to_usize())?;
        Some((cell, run))
    }

    /// The same layout, read as the window `window` describes.
    #[must_use]
    pub fn with_window(mut self, window: Window) -> Self {
        let first = window.pages.start;
        let mut page_blocks: WindowPageVec<Vec<FlowIndex>> =
            vec![Vec::new(); window.pages.len().to_usize()].into();
        for (local, block) in self.layout.blocks.iter().enumerate() {
            for page in block.page_span() {
                let at = page_offset(first, page);
                if let Some(blocks) = at.and_then(|at| page_blocks.get_mut(at)) {
                    blocks.push(window.first_index + FlowCount::from_len(local));
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
    pub fn index_of(&self, block: &TreeID) -> Option<FlowIndex> {
        self.order.get(block).copied()
    }

    /// The block at a place in the flow, when the window carries it.
    #[must_use]
    pub fn block(&self, index: FlowIndex) -> Option<&BlockLayout> {
        if index < self.first_index {
            return None;
        }
        self.layout
            .blocks
            .get(index.count_from(self.first_index).to_usize())
    }

    /// The blocks the window carries, each with its place in the flow.
    pub fn window_blocks(&self) -> impl DoubleEndedIterator<Item = (FlowIndex, &BlockLayout)> {
        let first = self.first_index;
        self.layout
            .blocks
            .iter()
            .enumerate()
            .map(move |(local, block)| (first + FlowCount::from_len(local), block))
    }

    /// How many blocks the document holds, carried or not.
    #[must_use]
    pub fn block_count(&self) -> FlowCount {
        FlowCount::from_len(self.order.len())
    }

    /// Whether the window carries a block.
    #[must_use]
    pub fn carries(&self, block: &TreeID) -> bool {
        self.index_of(block)
            .and_then(|index| self.block(index))
            .is_some()
    }

    /// The pages the window carries.
    #[must_use]
    pub fn pages(&self) -> PageSpan {
        self.pages
    }

    /// The two ends of the text of the document.
    #[must_use]
    pub fn ends(&self) -> Option<TextEnds> {
        self.ends
    }

    /// The indices of the blocks a page shows, in flow order. Empty for a
    /// page the window does not carry.
    #[must_use]
    pub fn blocks_on(&self, page: PageIndex) -> &[FlowIndex] {
        page_offset(self.pages.start, page)
            .and_then(|at| self.page_blocks.get(at))
            .map_or(&[], Vec::as_slice)
    }
}

/// The place of `page` in a window that starts at `first`, `None` before it.
fn page_offset(first: PageIndex, page: PageIndex) -> Option<WindowPage> {
    (page >= first).then(|| WindowPage::from_usize(page.count_from(first).to_usize()))
}

/// Every block of `blocks`, by its place in the flow.
#[must_use]
pub fn order_of(blocks: &[BlockLayout]) -> HashMap<TreeID, FlowIndex> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.block_id, FlowIndex::from_usize(index)))
        .collect()
}

/// The first and the last block holding text, from blocks in flow order,
/// each with the length of its text when it has one.
///
/// It reads from both ends, so a document whose ends hold text answers at
/// once.
pub fn text_ends<I>(blocks: I) -> Option<TextEnds>
where
    I: DoubleEndedIterator<Item = (TreeID, Option<RunCount>)> + Clone,
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
    pub row_tops: RowVec<DocPx>,
    pub cells: Vec<CellShape>,
}

impl TableShape {
    /// The top of `row` from the table's top. The row past the last one
    /// answers the bottom of the table, and a row past that `None`.
    pub(crate) fn row_top(&self, row: Row) -> Option<DocPx> {
        self.row_tops.get(row).copied()
    }
}

/// One cell drawn, or one merged region drawn as a single cell.
#[derive(Clone, Debug, PartialEq)]
pub struct CellShape {
    /// The cell the text sits in: a region's top-left cell.
    pub cell: TreeID,
    pub x: DocPx,
    pub width: DocPx,
    /// The rows the cell spans.
    pub rows: RowSpan,
    /// The text blocks the cell holds, each with its top from the top of the
    /// padded box, before the vertical alignment moves them.
    pub blocks: Vec<(TreeID, DocPx, ShapedRun)>,
    /// The height of that text, padding excluded: what the alignment places.
    pub content_height: DocPx,
    pub style: CellStyle,
}

/// What a cell stores about its own look (`ambre-spec-prose-schema.md
/// section 3`), in pixels and packed colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStyle {
    /// The fill. None leaves the sheet showing.
    pub fill: Option<Rgba>,
    /// The stored border. None draws the theme's hairline (`PROSE-UI-181`).
    pub border: Option<CellBorderPx>,
    /// The inset on each of the four sides.
    pub padding: DocPx,
    pub valign: ambre_bus::app::prose::VAlign,
}

impl Default for CellStyle {
    fn default() -> Self {
        Self {
            fill: None,
            border: None,
            padding: DocPx::ZERO,
            valign: ambre_bus::app::prose::VAlign::Top,
        }
    }
}

/// The four border widths of a cell, and their colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellBorderPx {
    pub top: DocPx,
    pub bottom: DocPx,
    pub left: DocPx,
    pub right: DocPx,

    /// None takes the theme's border colour.
    pub color: Option<Rgba>,
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

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use crate::doc;
    use crate::layout::measure::{EffectiveStyle, shipped_measurer};
    use crate::layout::run::RunOffset;
    use crate::layout::run::RunSpan;
    use ambre_bus::app::prose::ParagraphStyle;
    use ambre_core::{DocPoint, DocPx};

    fn style() -> EffectiveStyle {
        EffectiveStyle::from_parts(
            &doc::DocDefaults::default(),
            &ParagraphStyle::default(),
            ambre_core::Weight::Regular,
        )
    }

    /// A shaped run is document text, so its `Debug` prints counts and
    /// never the text.
    #[test]
    fn a_shaped_run_prints_counts_and_never_its_text() {
        let mut measurer = shipped_measurer();
        let (_, run) = measurer.measure_run("confidential", &style(), DocPx::new(400.0));
        let printed = format!("{run:?} {:?}", run.data());
        assert!(!printed.contains("confidential"), "{printed}");
        assert!(printed.contains("chars: RunCount(12)"), "{printed}");
    }

    #[test]
    fn a_point_resolves_to_the_offset_the_caret_box_puts_there() {
        let mut measurer = shipped_measurer();
        let (_, run) = measurer.measure_run("h\u{e9}llo world", &style(), DocPx::new(400.0));
        let run = run.data();
        for offset in (0..=run.len().get()).map(RunOffset::new) {
            let caret = run.caret_box(offset);
            let back = run.offset_at(DocPoint::new(
                DocPx::new(caret.x0 as f32 + 0.1),
                DocPx::new((caret.y0 + caret.y1) as f32 / 2.0),
            ));
            assert_eq!(back, offset, "offset {}", offset.get());
        }
    }

    /// `PROSE-FEAT-309`: the first line starts at its indent, and the lines
    /// after it at the left edge. A caret in an empty paragraph sits where
    /// its text will start.
    #[test]
    fn the_first_line_starts_at_its_indent() {
        let mut measurer = shipped_measurer();
        let mut indented = style();
        indented.indent_first = DocPx::new(40.0);
        let text = "one two three four five six seven eight nine ten eleven";
        let (plain_metrics, plain) = measurer.measure_run(text, &style(), DocPx::new(120.0));
        let (metrics, run) = measurer.measure_run(text, &indented, DocPx::new(120.0));
        let run = run.data();
        assert!(metrics.line_count >= plain_metrics.line_count);
        let lines = run.range_boxes(RunSpan::from_count(RunOffset::ZERO, run.len()));
        assert!((lines[0].x0 - 40.0).abs() < 1e-3);
        assert!(lines[1..].iter().all(|line| line.x0.abs() < 1e-3));
        assert!((run.caret_box(RunOffset::ZERO).x0 - 40.0).abs() < 1e-3);
        // A click inside the indent lands before the first character.
        assert_eq!(
            run.offset_at(DocPoint::new(DocPx::new(5.0), DocPx::new(5.0))),
            RunOffset::ZERO
        );
        // The plain run is untouched.
        assert!(plain.data().caret_box(RunOffset::ZERO).x0.abs() < 1e-3);
        let (_, empty) = measurer.measure_run("", &indented, DocPx::new(120.0));
        assert!((empty.data().caret_box(RunOffset::ZERO).x0 - 40.0).abs() < 1e-3);
        assert!(
            (empty
                .data()
                .caret_box_at(RunOffset::ZERO, super::Affinity::Upstream)
                .x0
                - 40.0)
                .abs()
                < 1e-3
        );
    }

    #[test]
    fn a_range_covers_one_box_per_line_it_crosses() {
        let mut measurer = shipped_measurer();
        let text = "one two three four five six seven eight nine ten eleven";
        let (metrics, run) = measurer.measure_run(text, &style(), DocPx::new(90.0));
        assert!(metrics.line_count.get() > 2);
        let whole = run
            .data()
            .range_boxes(RunSpan::from_count(RunOffset::ZERO, run.data().len()));
        assert_eq!(whole.len(), metrics.line_count.to_usize());
        assert!(
            run.data()
                .range_boxes(RunSpan::new(RunOffset::new(3), RunOffset::new(3)))
                .is_empty()
        );
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
