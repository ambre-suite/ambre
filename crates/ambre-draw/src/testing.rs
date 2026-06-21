// ambre-draw/src/testing.rs
// ============================================================================
// Headless test doubles for the rendering layer (feature = "testing").
//
// `RecordingSurface` is the mock `DrawSurface`: it tessellates nothing and
// touches no GPU, it just appends every draw call to an ordered log. Tests
// drive the real application renderers (Trame grid, later Prose/Toile) against
// it and assert on the recorded `DrawOp` sequence -- geometry, colours, clip
// balance, glyph-run counts -- entirely off the browser main thread.
//
// `FixedClock` is the matching `Clock` stub: a renderer needs a clock for its
// sub-phase timings, and on native there is no `performance.now()`. The stub
// returns a constant so timings are deterministic (and irrelevant) in tests.
// ============================================================================

use crate::{
    Affine, BezPath, Brush, Clock, Color, DrawSurface, GlyphRun, ParleyBrush, Rect, Stroke,
};

/// One recorded drawing operation. Glyph runs are summarised by their glyph
/// count and transform rather than captured verbatim -- the run borrows from a
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
    StrokePath {
        color: Option<Color>,
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

    /// Count the ops matching `pred` -- e.g. `count(|op| matches!(op, DrawOp::FillRect { .. }))`.
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
}

impl DrawSurface for RecordingSurface {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.ops.push(DrawOp::FillRect { rect, color });
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        self.ops.push(DrawOp::StrokeRect { rect, color, width });
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        self.ops.push(DrawOp::StrokeLine {
            x0,
            y0,
            x1,
            y1,
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

    fn stroke_path(&mut self, _path: &BezPath, brush: &Brush, _stroke: &Stroke) {
        self.ops.push(DrawOp::StrokePath {
            color: brush_color(brush),
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
    fn now_ms(&self) -> f64 {
        self.ms
    }
}
