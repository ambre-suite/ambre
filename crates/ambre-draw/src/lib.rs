// ambre-draw/src/lib.rs
// ============================================================================
// Abstraction trait for shared rendering in Ambre.
//
// `DrawSurface` is the GPU- and PDF-agnostic drawing interface used by
// application-level rendering code (Trame grid, Prose page, Toile canvas, ...).
//
// Implementations:
//   - `WgpuSurface`   in `ambre-canvas` (lyon tessellation + wgpu, on-screen) -- live.
//   - `KrillaSurface` in `ambre-pdf` (krilla, PDF export, Logic thread, lazy-loaded) -- live.
//
// `kinetic` is the one kinetic scroll the applications share: pure arithmetic
// over timed pointer positions, beside the drawing primitives they share.
// `wheel` is the wheel delta with its unit, which every view reads as pixels.
// `consumed` is the answer every input handler gives: whether it took the event.
// `hit` is the trait each view's content hit-test implements, and `walk` the
// trait each application's document walk implements.
//
// Dependency rule: ambre-draw depends only on kurbo + peniko + parley.
// No GPU code, no WASM glue, no wgpu, no lyon, no krilla. This keeps the core
// drawing logic portable and testable.
// ============================================================================

pub mod types;
pub use types::*;

pub mod kinetic;

pub mod wheel;
pub use wheel::{WHEEL_LINE, WheelAmount, WheelDelta, WheelDeltaMode, WheelVector};

pub mod consumed;
pub use consumed::Consumed;

pub mod hit;
pub use hit::HitTest;

pub mod peer;
pub use peer::{peer_color, peer_rgb};

pub mod text;
pub use text::{BundledFamily, LazyFont, LazyScripts, resolve_family, sealed_font_context};

pub mod walk;
pub use walk::DocumentWalk;

/// The pixels of an image, and the pixel lengths of an image and of a stroke.
/// The size of an image is an `ambre_core::Size`, which this crate does not
/// re-export: its own `Size` is the kurbo one.
pub use ambre_core::{Image, Payload, Px, RgbaPixels, Surface};
use ambre_core::{Instant, PageLoad};

#[cfg(feature = "testing")]
pub mod testing;

/// A borrowed view over already-decoded raster pixels, passed to
/// [`DrawSurface::draw_image`]. RGBA8, row-major, no stride padding: `pixels` is
/// exactly `width * height * 4` bytes.
///
/// It never owns or decodes pixels. Decoding is the caller's job: Toile interop
/// decodes on import, and the screen renderer decodes from its media cache. That
/// keeps `ambre-draw` codec-free (kurbo / peniko / parley only).
#[derive(Clone, Copy, Debug)]
pub struct ImageSource<'a> {
    /// A stable identity for *these* pixels. A back end that uploads them can
    /// recognise the same image on a later frame rather than hashing the buffer
    /// on every draw.
    ///
    /// The caller mints it from whatever already identifies the image -- Toile
    /// folds the leading bytes of the blake2b address it stores in the document.
    /// Two sources sharing a key MUST carry the same pixels. A back end is free
    /// to serve the first upload for both, and a key reused for different
    /// content shows the wrong picture. Distinct content that happens to share a
    /// key is the caller's bug, not the surface's.
    pub key: ImageKey,
    /// Non-premultiplied RGBA8 samples, row-major, `width * height * 4` bytes.
    pub pixels: &'a Payload<RgbaPixels>,
    /// Native pixel size of the image.
    pub size: ambre_core::Size<Px<Image>>,
}

/// The identity of a decoded image, as its caller knows it (`ImageSource::key`).
///
/// It is deliberately not a hash of the pixels: hashing megabytes per frame is
/// the cost this type exists to avoid. It carries no meaning of its own -- only
/// equality -- so any stable derivation the caller already has will do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageKey(u64);

impl ImageKey {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Fold the leading bytes of a content address, its hex digest, into a
    /// key. Two different addresses collide only if their first bytes do.
    pub fn from_address(address: &ambre_core::Validated<ambre_core::ContentHash>) -> Self {
        let mut acc = 0u64;
        for b in address.as_str().as_bytes().iter().take(16) {
            acc = acc.rotate_left(8) ^ u64::from(*b);
        }
        Self(acc)
    }
}

/// Monotonic millisecond clock, abstracted so rendering code can measure its
/// sub-phase timings without depending on `web_sys::Performance` directly.
///
/// On the UI Thread the implementation wraps `performance.now()`; headless
/// tests pass a deterministic stub (see [`testing::FixedClock`]). This keeps the
/// renderers callable off the browser main thread, which is what makes the
/// mock-surface tests possible.
pub trait Clock {
    /// The page's monotonic clock. Sub-millisecond precision is expected on
    /// screen but never relied upon for correctness.
    fn now(&self) -> Instant<PageLoad>;
}

/// GPU- and PDF-agnostic drawing interface.
///
/// All coordinates are in logical pixels (f64). The origin is the top-left
/// corner of the rendering surface; Y increases downward.
///
/// Each `draw_*` call appends a draw command -- the surface is append-only
/// within a frame. The only stateful mechanism is the clip stack
/// (`push_clip`/`pop_clip`) and the transform stack
/// (`push_transform`/`pop_transform`), which must always be balanced.
///
/// ## Text rendering
///
/// Text layout (Parley `Layout<ParleyBrush>`) is the caller's responsibility.
/// Shape once, cache the `Layout`, then call `draw_glyphs` for each `GlyphRun`
/// every frame. This keeps shaping out of the trait and enables per-frame
/// zero-allocation rendering on cache hits.
///
/// ## Path rendering
///
/// `fill_path` and `stroke_path` accept `kurbo::BezPath`. On screen this is
/// tessellated by lyon; in PDF it maps to krilla path primitives. Callers never
/// depend on lyon directly.
pub trait DrawSurface {
    // -------------------------------------------------------------------------
    // Geometry -- primitives
    // -------------------------------------------------------------------------

    /// Fill a rectangle with a solid color.
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Stroke a rectangle outline, `width` across.
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: Px<Surface>);

    /// Stroke a line segment, `width` across.
    fn stroke_line(&mut self, line: Line, color: Color, width: Px<Surface>);

    // -------------------------------------------------------------------------
    // Text
    // -------------------------------------------------------------------------

    /// Render a pre-shaped Parley glyph run at the given transform.
    ///
    /// `transform` positions the run origin (typically
    /// `Affine::translate((x, baseline_y))`). The run's own advance offsets
    /// are embedded in `GlyphRun::positioned_glyphs()`.
    ///
    /// On screen: glyphs are rasterized via Swash into a GPU atlas and drawn
    /// as textured quads. In PDF: mapped to krilla glyph placement.
    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine);

    // -------------------------------------------------------------------------
    // Clip stack
    // -------------------------------------------------------------------------

    /// Push a rectangular clip. All subsequent draw calls are scissored to
    /// `rect` until the matching `pop_clip`. Calls must be balanced.
    fn push_clip(&mut self, rect: Rect);

    /// Pop the most recently pushed clip region.
    fn pop_clip(&mut self);

    // -------------------------------------------------------------------------
    // Geometry -- arbitrary paths (default no-ops until lyon is wired)
    // -------------------------------------------------------------------------

    /// Fill an arbitrary path with a solid color. Default no-op; overridden by
    /// `WgpuSurface` (lyon tessellation) and `KrillaSurface` (PDF paths).
    #[allow(unused_variables)]
    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {}

    /// Stroke an arbitrary path. Default no-op; overridden by `WgpuSurface`
    /// (lyon tessellation) and `KrillaSurface` (PDF paths).
    #[allow(unused_variables)]
    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {}

    // -------------------------------------------------------------------------
    // Raster images (default no-op until a surface supports raster)
    // -------------------------------------------------------------------------

    /// Paint a decoded raster image into the world-space `dest` rect (the element
    /// AABB after the active transform). The image is scaled to fill `dest`;
    /// preserving aspect ratio is the caller's responsibility (Toile stores the
    /// element rect at the imported aspect).
    ///
    /// Default no-op: a surface without raster support (for example a base
    /// `KrillaSurface` built without the raster feature) still renders text and
    /// vector geometry. `KrillaSurface` hands the pixels to krilla as a raster
    /// image.
    ///
    /// `WgpuSurface` draws the image as a textured quad. Each corner of `dest`
    /// maps through the active transform, so a rotated element turns its
    /// picture with it. A clip rejects the quad by its bounding box and never
    /// trims it (`ambre-spec-drawing-surface.md` section 5).
    ///
    /// Per `toile-media`, the caller paints a Toile image as a placeholder rect
    /// (via `fill_rect`) until its pixels arrive from the media cache. It is
    /// repainted through this method once the pixels are known. The trait carries
    /// no placeholder logic itself.
    #[allow(unused_variables)]
    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {}

    // -------------------------------------------------------------------------
    // Transform stack (default no-ops until Toile needs it)
    // -------------------------------------------------------------------------

    /// Push an affine transform. Subsequent draw calls are in the transformed
    /// coordinate space. Calls must be balanced with `pop_transform`.
    ///
    /// Used by Toile (infinite canvas pan/zoom) and Prose (page margins).
    /// Default no-op -- override when transform support is implemented.
    #[allow(unused_variables)]
    fn push_transform(&mut self, transform: Affine) {}

    /// Pop the most recently pushed transform. Default no-op.
    fn pop_transform(&mut self) {}
}
