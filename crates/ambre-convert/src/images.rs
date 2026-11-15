//! The drawing the images module answers for an SVG file
//! (`ambre-spec-architecture.md section 2`, the converter modules).
//!
//! `usvg` reads the file into a normalized tree: every primitive is a path,
//! and every transform is resolved to an absolute one. The module steps the
//! source text beside it, with `roxmltree`, because the usvg font stack is off
//! and its tree carries no text. Each `<text>` lands in the drawing where the
//! source put it: in document order, inside the group that holds it.
//!
//! The drawing is format-level and flat. Toile maps it onto its elements in
//! the Main instance, with its own limits, clamps and report. A node names its
//! children by position, so postcard never recurses on the Logic thread. A
//! tree nested past the module's depth bound stops there and says so.

use ambre_core::index::{Axis, IdxVec, Pos};
use ambre_core::{FileBytes, Image, Ms, Payload, Px, Rgb, RgbaPixels, Size, TimeSpan};
use serde::{Deserialize, Serialize};

/// The places of a drawing: a [`Pos<SvgNode>`] names a node. Only the images
/// module hands one out.
#[derive(PartialEq, Eq)]
pub enum SvgNode {}

impl Axis for SvgNode {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SvgNode");
}

/// The drawing of one SVG file. Node 0 is the root group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgTree {
    nodes: IdxVec<SvgNode, SvgItem>,
    /// Whether the text pass could read the source.
    pub source: SvgSource,
    /// Whether a group sat deeper than the module's depth bound.
    pub depth: SvgDepth,
}

impl SvgTree {
    /// The root group, the drawing itself.
    pub const ROOT: Pos<SvgNode> = Pos::ZERO;

    /// The drawing `nodes` hold, the root group at position 0.
    #[must_use]
    pub const fn new(nodes: IdxVec<SvgNode, SvgItem>, source: SvgSource, depth: SvgDepth) -> Self {
        Self {
            nodes,
            source,
            depth,
        }
    }

    /// The node at `pos`, or `None` past the end of the arena.
    #[must_use]
    pub fn get(&self, pos: Pos<SvgNode>) -> Option<&SvgItem> {
        self.nodes.get(pos)
    }
}

/// Whether the text pass read the source. A source that is not UTF-8 or not
/// well-formed XML costs the drawing its text, and Toile reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SvgSource {
    Read,
    Unreadable,
}

/// Whether every group of the file sat within the module's depth bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SvgDepth {
    Within,
    Exceeded,
}

/// One node of the drawing and its children, in document order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgItem {
    pub children: Vec<Pos<SvgNode>>,
    pub kind: SvgKind,
}

/// What a node draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SvgKind {
    /// A group, under its absolute transform.
    Group(SvgTransform),
    Path(SvgPath),
    Image(SvgImage),
    Text(SvgText),
}

/// An affine transform, in the row order of `tiny-skia`.
///
/// The image of `(x, y)` is `(sx x + kx y + tx, ky x + sy y + ty)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SvgTransform {
    pub sx: f32,
    pub kx: f32,
    pub tx: f32,
    pub ky: f32,
    pub sy: f32,
    pub ty: f32,
}

impl SvgTransform {
    /// The transform that moves nothing.
    pub const IDENTITY: Self = Self {
        sx: 1.0,
        kx: 0.0,
        tx: 0.0,
        ky: 0.0,
        sy: 1.0,
        ty: 0.0,
    };
}

/// A point of a path, in the path's own coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SvgPoint {
    pub x: f32,
    pub y: f32,
}

/// One segment of a path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SvgSegment {
    MoveTo(SvgPoint),
    LineTo(SvgPoint),
    QuadTo(SvgPoint, SvgPoint),
    CubicTo(SvgPoint, SvgPoint, SvgPoint),
    Close,
}

/// A path: its segments in its own coordinates, its absolute transform, and
/// its paint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgPath {
    pub segments: Vec<SvgSegment>,
    pub transform: SvgTransform,
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgStroke>,
}

/// How a fill or a stroke paints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SvgPaint {
    /// One colour, at an opacity from 0 to 1.
    Solid { color: Rgb, opacity: f32 },
    /// A gradient or a pattern.
    Other,
}

/// The stroke of a path: its paint and its width, in the path's coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SvgStroke {
    pub paint: SvgPaint,
    pub width: f32,
}

/// A rectangle in absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SvgRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// An `<image>`: the picture it embeds and its absolute bounds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgImage {
    pub picture: SvgPicture,
    pub bounds: SvgRect,
}

/// The picture an `<image>` embeds. Toile decodes PNG itself. The module
/// decodes a WebP picture into its pixels; `Webp` is one it could not decode.
/// It decodes the thumbnail of a JPEG picture, since Main links no JPEG
/// decoder. The other kinds carry no bytes, since nothing reads them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SvgPicture {
    Png(Payload<FileBytes>),
    /// A JPEG picture, its thumbnail decoded by the module.
    Jpeg(SvgJpeg),
    /// A JPEG picture the decoder refused, and why.
    JpegRefused(crate::jpeg::JpegRefusal),
    /// A WebP picture, decoded by the module.
    WebpPixels(Pixels),
    Gif,
    /// A WebP picture the decoder refused.
    Webp,
    /// A nested SVG document.
    Svg,
}

/// A JPEG picture an SVG file embeds: the file Toile stores, the size its
/// header declares, and its thumbnail at [`thumb_size`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgJpeg {
    pub data: Payload<FileBytes>,
    pub size: Size<Px<Image>>,
    pub thumb: Pixels,
}

/// The longer edge of the thumbnail Toile stores beside a picture, inline in
/// its media entry. Small enough to paint at once, large enough to preview.
pub const THUMB_EDGE: Px<Image> = Px::new(256);

/// The size of the thumbnail of a picture of `size`: within [`THUMB_EDGE`],
/// its ratio kept. A picture already within the edge keeps its size.
#[must_use]
pub fn thumb_size(size: Size<Px<Image>>) -> Size<Px<Image>> {
    let (width, height) = (size.width.get(), size.height.get());
    if width.max(height) <= THUMB_EDGE.get() {
        return size;
    }
    let edge = f64::from(THUMB_EDGE.get());
    let scale = (edge / f64::from(width.max(1))).min(edge / f64::from(height.max(1)));
    // A side scaled under the thumbnail edge fits a `u32`.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a side scaled under the thumbnail edge"
    )]
    let fit = |side: u32| Px::new(((f64::from(side) * scale).round() as u32).max(1));
    Size::new(fit(width), fit(height))
}

/// A `<text>` element, read from the source. The body is content, so `Debug`
/// prints its length alone. `x` and `y` place the baseline in the
/// coordinates of the group around it, whose absolute transform rides along.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SvgText {
    /// The text of its descendants, trimmed. Empty when its content sits in
    /// a construct the pass does not follow.
    pub body: String,
    pub x: f64,
    pub y: f64,
    /// The `font-size` in pixels: 16 when absent or unreadable.
    pub size: f64,
    pub transform: SvgTransform,
}

impl std::fmt::Debug for SvgText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgText")
            .field("body", &ambre_core::Redacted::new(&self.body))
            .field("x", &self.x)
            .field("y", &self.y)
            .field("size", &self.size)
            .field("transform", &self.transform)
            .finish()
    }
}

/// A decoded picture: non-premultiplied RGBA8 samples, row-major, four bytes
/// a pixel, and its size. The module answers it for a WebP picture, which
/// the Main instance encodes as PNG for the board.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pixels {
    pub size: Size<Px<Image>>,
    pub rgba: Payload<RgbaPixels>,
}

/// The frames of a decoded animation, each composed onto the logical screen
/// and scaled to `size`. The module answers it for a GIF picture. A still
/// picture is one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frames {
    pub size: Size<Px<Image>>,
    /// At least one frame, in the order the file plays them.
    pub frames: Vec<Frame>,
}

impl Frames {
    /// The first frame alone, as the pixels of a still picture.
    #[must_use]
    pub fn into_first(self) -> Option<Pixels> {
        let size = self.size;
        self.frames.into_iter().next().map(|frame| Pixels {
            size,
            rgba: frame.rgba,
        })
    }
}

/// One frame of an animation: its samples, as [`Pixels`] holds them, and how
/// long it shows, as the file states it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub rgba: Payload<RgbaPixels>,
    pub delay: TimeSpan<Ms>,
}
