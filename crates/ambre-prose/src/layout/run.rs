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

use crate::layout::pass::PageLayout;

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
        }
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

/// One layout as the view receives it: the pages, and the runs to draw on
/// them.
///
/// It is a whole value, stamped with the version the paginator gave it. The
/// view keeps the newest it has seen and drops an older one
/// (`PROSE-UI-016`).
#[derive(Clone, Debug, Default)]
pub struct ShapedLayout {
    pub version: u64,
    pub layout: Arc<PageLayout>,
    /// The run of every block that draws text, body and bands alike.
    pub blocks: HashMap<TreeID, ShapedRun>,
    /// The run of every footnote, its ordinal in front.
    pub notes: HashMap<Uuid, ShapedRun>,
}

// `PROSE-UI-113`: the run crosses from the thread that shapes to the thread
// that draws. A field bound to one thread or to a device fails this build.
const _: () = {
    const fn crosses<T: Send + Sync + 'static>() {}
    crosses::<ShapedRun>();
    crosses::<ParleyRunData>();
    crosses::<ShapedLayout>();
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
