// ambre-draw/src/testing.rs
// ============================================================================
// Headless test doubles for the rendering layer (feature = "testing").
//
// `RecordingSurface` is the mock `DrawSurface`: it tessellates nothing and
// touches no GPU, it just appends every draw call to an ordered log. Tests
// drive the real application renderers (Trame grid, later Prose/Toile) against
// it, entirely off the browser main thread. They assert on the recorded
// `DrawOp` sequence: geometry, colours, clip balance, glyph-run counts.
//
// `FixedClock` is the matching `Clock` stub: a renderer needs a clock for its
// sub-phase timings, and on native there is no `performance.now()`. The stub
// returns a constant so timings are deterministic (and irrelevant) in tests.
// ============================================================================

// A test double. An unbalanced clip or transform is the bug it exists to
// catch, so it stops the test at the call that caused it.
#![allow(
    clippy::expect_used,
    reason = "a test double fails the test that misuses it"
)]

use crate::{
    Affine, BezPath, Brush, Clock, Color, DrawSurface, GlyphRun, ImageSize, ImageSource, Line,
    ParleyBrush, Rect, Stroke, SurfacePx,
};

ambre_core::define_tally! {
    /// How many glyphs one recorded run holds.
    pub struct GlyphCount;
    /// How many recorded operations match a predicate.
    pub struct OpCount;
}

/// How many pushes of one stack no pop has closed yet. It never goes below
/// zero: a pop with no push is the renderer bug the double exists to catch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StackDepth(u32);

impl StackDepth {
    /// The depth after one more push.
    fn pushed(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The depth after one pop, or `None` when no push is open.
    fn popped(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }

    /// Whether every push has its pop.
    fn is_balanced(self) -> bool {
        self.0 == 0
    }
}

/// One recorded drawing operation. Glyph runs are summarised by their glyph
/// count and transform rather than captured verbatim. The run borrows from a
/// Parley layout owned by the caller, and the count is what tests assert on.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    FillRect {
        rect: Rect,
        color: Color,
    },
    StrokeRect {
        rect: Rect,
        color: Color,
        width: SurfacePx,
    },
    StrokeLine {
        line: Line,
        color: Color,
        width: SurfacePx,
    },
    /// A pre-shaped glyph run: how many glyphs, the solid brush colour (if any),
    /// and the run origin transform.
    Glyphs {
        glyph_count: GlyphCount,
        color: Option<Color>,
        transform: Affine,
    },
    FillPath {
        color: Option<Color>,
    },
    /// A raster image paint: its native pixel size and the destination rect
    /// it was scaled into. The pixels themselves are not captured -- tests
    /// assert on placement and that a real image (not a placeholder) was drawn.
    DrawImage {
        size: ImageSize,
        dest: Rect,
    },
    /// A stroked path: its colour, the box of its control points in the
    /// coordinates it was handed, and the stroke width in those coordinates.
    StrokePath {
        color: Option<Color>,
        bounds: Rect,
        width: SurfacePx,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
    PushTransform {
        transform: Affine,
    },
    PopTransform,
}

/// Solid colour behind a `Brush`, or `None` for gradient/image brushes (which
/// the canvas renderers never use today).
fn brush_color(brush: &Brush) -> Option<Color> {
    match brush {
        Brush::Solid(c) => Some(*c),
        _ => None,
    }
}

/// Mock `DrawSurface` that records the ordered sequence of draw calls.
///
/// Construct with [`RecordingSurface::new`], pass `&mut` it to a renderer, then
/// inspect [`RecordingSurface::ops`]. Helper counters keep the common
/// assertions (clip balance, op counts) terse.
#[derive(Default)]
pub struct RecordingSurface {
    ops: Vec<DrawOp>,
    /// Current clip-stack depth, updated live so tests can assert balance even
    /// mid-frame. Underflow (pop without push) panics -- that is a renderer bug.
    clip_depth: StackDepth,
    /// Current transform-stack depth, same contract as `clip_depth`.
    transform_depth: StackDepth,
}

impl RecordingSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// The ordered log of every draw call since construction.
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }

    /// Whether every clip pushed so far is popped: true at a balanced frame
    /// boundary.
    pub fn clips_balanced(&self) -> bool {
        self.clip_depth.is_balanced()
    }

    /// Whether every transform pushed so far is popped: true at a balanced
    /// frame boundary.
    pub fn transforms_balanced(&self) -> bool {
        self.transform_depth.is_balanced()
    }

    /// Count the ops matching `pred`, for example
    /// `count(|op| matches!(op, DrawOp::FillRect { .. }))`.
    pub fn count(&self, pred: impl Fn(&DrawOp) -> bool) -> OpCount {
        OpCount::of_len(self.ops.iter().filter(|op| pred(op)).count())
    }

    /// Every distinct fill colour seen, in first-seen order. Handy for asserting
    /// that a renderer drew with the active palette and not a hardcoded colour.
    pub fn fill_colors(&self) -> Vec<Color> {
        let mut seen = Vec::new();
        for op in &self.ops {
            if let DrawOp::FillRect { color, .. } = op
                && !seen.contains(color)
            {
                seen.push(*color);
            }
        }
        seen
    }

    /// The rect of every clip pushed, in order.
    pub fn clip_rects(&self) -> Vec<Rect> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                DrawOp::PushClip { rect } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    /// Whether these ops hold every op of `walk`, in order and contiguous: a
    /// walk drawn unchanged among the screen's own layers (`CANVAS-074`).
    pub fn holds_walk(&self, walk: &RecordingSurface) -> bool {
        let walk = walk.ops();
        !walk.is_empty() && self.ops.windows(walk.len()).any(|ops| ops == walk)
    }
}

impl DrawSurface for RecordingSurface {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.ops.push(DrawOp::FillRect { rect, color });
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: SurfacePx) {
        self.ops.push(DrawOp::StrokeRect { rect, color, width });
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: SurfacePx) {
        self.ops.push(DrawOp::StrokeLine { line, color, width });
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        self.ops.push(DrawOp::Glyphs {
            glyph_count: GlyphCount::of_len(run.glyphs().count()),
            color: brush_color(brush),
            transform,
        });
    }

    fn push_clip(&mut self, rect: Rect) {
        self.clip_depth = self.clip_depth.pushed();
        self.ops.push(DrawOp::PushClip { rect });
    }

    fn pop_clip(&mut self) {
        self.clip_depth = self
            .clip_depth
            .popped()
            .expect("pop_clip without a matching push_clip");
        self.ops.push(DrawOp::PopClip);
    }

    fn fill_path(&mut self, _path: &BezPath, brush: &Brush) {
        self.ops.push(DrawOp::FillPath {
            color: brush_color(brush),
        });
    }

    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {
        self.ops.push(DrawOp::StrokePath {
            color: brush_color(brush),
            bounds: path.control_box(),
            width: SurfacePx::new(stroke.width),
        });
    }

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        self.ops.push(DrawOp::DrawImage {
            size: image.size,
            dest,
        });
    }

    fn push_transform(&mut self, transform: Affine) {
        self.transform_depth = self.transform_depth.pushed();
        self.ops.push(DrawOp::PushTransform { transform });
    }

    fn pop_transform(&mut self) {
        self.transform_depth = self
            .transform_depth
            .popped()
            .expect("pop_transform without a matching push_transform");
        self.ops.push(DrawOp::PopTransform);
    }
}

/// `Clock` stub returning a constant. Renderer sub-phase timings become
/// deterministic zero-deltas in headless tests, which never assert on them.
#[derive(Clone, Copy, Default)]
pub struct FixedClock {
    at: ambre_core::PageTime,
}

impl FixedClock {
    /// The clock that reads `at` on every call.
    pub fn new(at: ambre_core::PageTime) -> Self {
        Self { at }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> ambre_core::PageTime {
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A depth never goes below zero: a pop with no push has no depth.
    #[test]
    fn a_pop_with_no_push_has_no_depth() {
        assert_eq!(StackDepth::default().popped(), None);
        let one = StackDepth::default().pushed();
        assert!(!one.is_balanced());
        assert_eq!(one.popped().map(StackDepth::is_balanced), Some(true));
    }

    /// The double stops the test at a pop that no push opened.
    #[test]
    #[should_panic(expected = "pop_clip without a matching push_clip")]
    fn an_unbalanced_pop_stops_the_test() {
        RecordingSurface::new().pop_clip();
    }
}
