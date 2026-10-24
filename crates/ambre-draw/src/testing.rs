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
    Affine, BezPath, Brush, Clock, Color, DrawSurface, GlyphRun, ImageSource, Line, ParleyBrush,
    Rect, Stroke,
};

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
        width: f64,
    },
    StrokeLine {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        color: Color,
        width: f64,
    },
    /// A pre-shaped glyph run: how many glyphs, the solid brush colour (if any),
    /// and the run origin transform.
    Glyphs {
        glyph_count: usize,
        color: Option<Color>,
        transform: Affine,
    },
    FillPath {
        color: Option<Color>,
    },
    /// A raster image paint: its native pixel dimensions and the destination
    /// rect it was scaled into. The pixels themselves are not captured -- tests
    /// assert on placement and that a real image (not a placeholder) was drawn.
    DrawImage {
        width: u32,
        height: u32,
        dest: Rect,
    },
    /// A stroked path: its colour, the box of its control points in the
    /// coordinates it was handed, and the stroke width in those coordinates.
    StrokePath {
        color: Option<Color>,
        bounds: Rect,
        width: f64,
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
    clip_depth: usize,
    /// Current transform-stack depth, same contract as `clip_depth`.
    transform_depth: usize,
}

impl RecordingSurface {
    pub fn new() -> Self {
        Self::default()
    }

    /// The ordered log of every draw call since construction.
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }

    /// Clip-stack depth right now (0 at a balanced frame boundary).
    pub fn clip_depth(&self) -> usize {
        self.clip_depth
    }

    /// Transform-stack depth right now (0 at a balanced frame boundary).
    pub fn transform_depth(&self) -> usize {
        self.transform_depth
    }

    /// Count the ops matching `pred`, for example
    /// `count(|op| matches!(op, DrawOp::FillRect { .. }))`.
    pub fn count(&self, pred: impl Fn(&DrawOp) -> bool) -> usize {
        self.ops.iter().filter(|op| pred(op)).count()
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

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        self.ops.push(DrawOp::StrokeRect { rect, color, width });
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: f64) {
        self.ops.push(DrawOp::StrokeLine {
            x0: line.p0.x,
            y0: line.p0.y,
            x1: line.p1.x,
            y1: line.p1.y,
            color,
            width,
        });
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        self.ops.push(DrawOp::Glyphs {
            glyph_count: run.glyphs().count(),
            color: brush_color(brush),
            transform,
        });
    }

    fn push_clip(&mut self, rect: Rect) {
        self.clip_depth += 1;
        self.ops.push(DrawOp::PushClip { rect });
    }

    fn pop_clip(&mut self) {
        self.clip_depth = self
            .clip_depth
            .checked_sub(1)
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
            width: stroke.width,
        });
    }

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        self.ops.push(DrawOp::DrawImage {
            width: image.size.width,
            height: image.size.height,
            dest,
        });
    }

    fn push_transform(&mut self, transform: Affine) {
        self.transform_depth += 1;
        self.ops.push(DrawOp::PushTransform { transform });
    }

    fn pop_transform(&mut self) {
        self.transform_depth = self
            .transform_depth
            .checked_sub(1)
            .expect("pop_transform without a matching push_transform");
        self.ops.push(DrawOp::PopTransform);
    }
}

/// `Clock` stub returning a constant. Renderer sub-phase timings become
/// deterministic zero-deltas in headless tests, which never assert on them.
#[derive(Clone, Copy, Default)]
pub struct FixedClock {
    ms: f64,
}

impl FixedClock {
    pub fn new(ms: f64) -> Self {
        Self { ms }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> ambre_core::PageTime {
        ambre_core::PageTime(self.ms)
    }
}
