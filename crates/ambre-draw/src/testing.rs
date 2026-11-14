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
#![expect(
    clippy::expect_used,
    reason = "a test double fails the test that misuses it"
)]

use crate::{
    Affine, BezPath, Brush, Clock, Color, DrawSurface, GlyphRun, ImageSource, Line, ParleyBrush,
    Rect, Stroke,
};
use ambre_core::{Count, Image, Instant, PageLoad, Px, Size, Surface};

/// How many glyphs one recorded run holds.
pub enum Glyphs {}

impl ambre_core::CountKind for Glyphs {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("GlyphCount");
}

/// How many recorded operations match a predicate.
pub enum Ops {}

impl ambre_core::CountKind for Ops {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("OpCount");
}

/// The pushes of one stack that no pop has closed yet.
pub enum OpenPushes {}

impl ambre_core::CountKind for OpenPushes {
    type Repr = u32;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("StackDepth");
}

/// The depth after one pop, or `None` when no push is open. A pop with no
/// push is the renderer bug the double exists to catch. So it is refused
/// here, where `minus` alone would hold the depth at zero.
fn popped(depth: Count<OpenPushes>) -> Option<Count<OpenPushes>> {
    (!depth.is_zero()).then(|| depth.minus(Count::new(1)))
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
        width: Px<Surface>,
    },
    StrokeLine {
        line: Line,
        color: Color,
        width: Px<Surface>,
    },
    /// A pre-shaped glyph run: how many glyphs, the solid brush colour (if any),
    /// and the run origin transform.
    Glyphs {
        glyph_count: Count<Glyphs>,
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
        size: Size<Px<Image>>,
        dest: Rect,
    },
    /// A stroked path: its colour, the box of its control points in the
    /// coordinates it was handed, and the stroke width in those coordinates.
    StrokePath {
        color: Option<Color>,
        bounds: Rect,
        width: Px<Surface>,
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
    clip_depth: Count<OpenPushes>,
    /// Current transform-stack depth, same contract as `clip_depth`.
    transform_depth: Count<OpenPushes>,
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
        self.clip_depth.is_zero()
    }

    /// Whether every transform pushed so far is popped: true at a balanced
    /// frame boundary.
    pub fn transforms_balanced(&self) -> bool {
        self.transform_depth.is_zero()
    }

    /// Count the ops matching `pred`, for example
    /// `count(|op| matches!(op, DrawOp::FillRect { .. }))`.
    pub fn count(&self, pred: impl Fn(&DrawOp) -> bool) -> Count<Ops> {
        Count::<Ops>::of(self.ops.iter().filter(|op| pred(op)).count())
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

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: Px<Surface>) {
        self.ops.push(DrawOp::StrokeRect { rect, color, width });
    }

    fn stroke_line(&mut self, line: Line, color: Color, width: Px<Surface>) {
        self.ops.push(DrawOp::StrokeLine { line, color, width });
    }

    fn draw_glyphs(
        &mut self,
        run: &GlyphRun<'_, ParleyBrush>,
        _text: crate::LayoutText<'_>,
        brush: &Brush,
        transform: Affine,
    ) {
        self.ops.push(DrawOp::Glyphs {
            glyph_count: Count::<Glyphs>::of(run.glyphs().count()),
            color: brush_color(brush),
            transform,
        });
    }

    fn push_clip(&mut self, rect: Rect) {
        self.clip_depth = self.clip_depth.incremented();
        self.ops.push(DrawOp::PushClip { rect });
    }

    fn pop_clip(&mut self) {
        self.clip_depth = popped(self.clip_depth).expect("pop_clip without a matching push_clip");
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
            width: Px::<Surface>::new(stroke.width),
        });
    }

    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {
        self.ops.push(DrawOp::DrawImage {
            size: image.size,
            dest,
        });
    }

    fn push_transform(&mut self, transform: Affine) {
        self.transform_depth = self.transform_depth.incremented();
        self.ops.push(DrawOp::PushTransform { transform });
    }

    fn pop_transform(&mut self) {
        self.transform_depth =
            popped(self.transform_depth).expect("pop_transform without a matching push_transform");
        self.ops.push(DrawOp::PopTransform);
    }
}

/// Every face the application ships, base and lazy, read off the tree.
const SHIPPED_FACES: [&str; 10] = [
    "NotoSans-Regular.ttf",
    "NotoSans-Bold.ttf",
    "NotoSans-Italic.ttf",
    "NotoSans-BoldItalic.ttf",
    "DejaVuSerif.ttf",
    "DejaVuSansMono.ttf",
    "NotoSansArabic-Regular.ttf",
    "NotoSansDevanagari-Regular.ttf",
    "NotoSansSC-Regular.otf",
    "NotoColorEmoji.ttf",
];

/// A sealed context holding every face the application ships, as a peer holds
/// them once each lazy script arrived. A test then shapes ligatures, marks,
/// right-to-left words and emoji on the faces a user sees, never on the
/// machine's. A face the tree lacks is skipped, so the caller asserts on what
/// it shaped.
pub fn shipped_font_context() -> parley::FontContext {
    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
    let mut font_cx = crate::sealed_font_context();
    for file in SHIPPED_FACES {
        if let Ok(bytes) = std::fs::read(format!("{DIR}{file}")) {
            crate::register_face(&mut font_cx, ambre_core::Payload::new(bytes));
        }
    }
    font_cx
}

/// `Clock` stub returning a constant. Renderer sub-phase timings become
/// deterministic zero-deltas in headless tests, which never assert on them.
#[derive(Clone, Copy, Default)]
pub struct FixedClock {
    at: Instant<PageLoad>,
}

impl FixedClock {
    /// The clock that reads `at` on every call.
    pub fn new(at: Instant<PageLoad>) -> Self {
        Self { at }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Instant<PageLoad> {
        self.at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A depth never goes below zero: a pop with no push has no depth.
    #[test]
    fn a_pop_with_no_push_has_no_depth() {
        assert_eq!(popped(Count::ZERO), None);
        let one = Count::<OpenPushes>::ZERO.incremented();
        assert!(!one.is_zero());
        assert_eq!(popped(one).map(Count::<OpenPushes>::is_zero), Some(true));
    }

    /// The double stops the test at a pop that no push opened.
    #[test]
    #[should_panic(expected = "pop_clip without a matching push_clip")]
    fn an_unbalanced_pop_stops_the_test() {
        RecordingSurface::new().pop_clip();
    }
}
