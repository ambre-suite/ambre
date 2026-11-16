//! The deck the writer reads: slides of shapes, pictures and connectors, in
//! slide coordinates, with their speaker notes.
//!
//! The model knows nothing of a board. An application maps its own document
//! onto it and reports what it approximated or dropped on the way
//! (`1-Foundations/ambre-spec-interop.md`). The writer then carries every
//! item of the model, so the report of the application is the whole report.
//!
//! Every length is a print length in points, from the top-left corner of the
//! slide, y down. The writer turns each one into the English Metric Units of
//! the file. A model crosses from the Logic thread to the converter module as
//! postcard bytes, so every type here has a serde form.

use ambre_core::index::{Axis, Pos};
use ambre_core::{
    Becomes, Payload, PayloadKind, PrintLength, Pt, Radians, Rgba, Slant, Strikethrough, Underline,
    Weight,
};
use serde::{Deserialize, Serialize};

/// A whole deck: one slide size for every slide, and the slides in play order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Deck {
    pub size: SlideSize,
    pub slides: Vec<Slide>,
}

/// The size of every slide of a deck. A presentation holds one size alone.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlideSize {
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
}

impl SlideSize {
    /// The 16:9 slide PowerPoint opens a new deck on: 13.333 in by 7.5 in.
    pub const WIDESCREEN: Self = Self {
        width: PrintLength::<Pt>::new(960.0),
        height: PrintLength::<Pt>::new(540.0),
    };

    /// The smallest edge a presentation accepts: one inch.
    pub const MIN_EDGE: PrintLength<Pt> = PrintLength::<Pt>::new(72.0);

    /// The largest edge a presentation accepts: 56 in.
    pub const MAX_EDGE: PrintLength<Pt> = PrintLength::<Pt>::new(4032.0);

    /// The slide `width` wide whose height keeps the ratio of `height` to
    /// `width` of a source box. Each edge lands inside the bounds a
    /// presentation accepts, and a ratio that is no number gives the
    /// widescreen slide.
    #[must_use]
    pub fn with_aspect_of(width: PrintLength<Pt>, source: SlideSize) -> Self {
        let ratio = source.height.get() / source.width.get();
        if !ratio.is_finite() || ratio <= 0.0 {
            return Self::WIDESCREEN;
        }
        let width = width.clamp(Self::MIN_EDGE, Self::MAX_EDGE);
        Self {
            width,
            height: PrintLength::<Pt>::new(width.get() * ratio)
                .clamp(Self::MIN_EDGE, Self::MAX_EDGE),
        }
    }
}

impl Default for SlideSize {
    fn default() -> Self {
        Self::WIDESCREEN
    }
}

/// One slide: its items in paint order, bottom first, and its notes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    /// The name PowerPoint shows in its outline, empty for an untitled slide.
    pub name: String,
    pub items: Vec<Item>,
    /// What the presenter reads beside the slide. An empty text writes no
    /// notes page.
    pub notes: String,
    /// The colour behind every item. `None` leaves the slide to the
    /// background of its master.
    pub background: Option<Rgba>,
}

/// The items of one slide, the axis a connector names its ends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlideItems {}

impl Axis for SlideItems {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("SlideItem");
}

/// One thing a slide draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    Shape(Shape),
    Picture(Picture),
    Connector(Connector),
}

/// Where an item sits: the box it fills, unrotated, and the rotation about
/// its centre, clockwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub origin: SlidePoint,
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
    pub rotation: Radians,
}

/// A point of the slide, from its top-left corner, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlidePoint {
    pub x: PrintLength<Pt>,
    pub y: PrintLength<Pt>,
}

/// A shape: its outline, its paint, and the text it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// The name the selection pane of PowerPoint lists the shape under.
    pub name: String,
    pub at: Placement,
    pub geometry: Geometry,
    /// `None` leaves the shape hollow.
    pub fill: Option<Rgba>,
    /// `None` draws no outline.
    pub line: Option<Line>,
    /// `None` holds no text. A text box is a shape with no fill, no line
    /// and a text.
    pub text: Option<TextBody>,
}

/// The outline of a shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    Rect,
    /// A rectangle whose corners round by the radius, clamped to half the
    /// shorter edge.
    RoundRect {
        radius: PrintLength<Pt>,
    },
    Ellipse,
    /// An isosceles triangle, its apex at the middle of the top edge.
    Triangle,
    /// A line from the top-left corner of the box to its bottom-right one.
    Line,
    /// A path, in slide coordinates. The writer draws it in the box of the
    /// placement, which the caller sets to the bounds of the path.
    Freeform(Vec<PathSegment>),
}

/// One step of a freeform path.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathSegment {
    MoveTo(SlidePoint),
    LineTo(SlidePoint),
    QuadTo {
        control: SlidePoint,
        end: SlidePoint,
    },
    CubicTo {
        first: SlidePoint,
        second: SlidePoint,
        end: SlidePoint,
    },
    Close,
}

/// A stroke: its colour and its width.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub colour: Rgba,
    pub width: PrintLength<Pt>,
}

/// The text a shape holds, one paragraph per line of `text`. It stays text
/// the reader edits, in one face, one size and one colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBody {
    pub text: String,
    /// The name of the face, empty for the face of the theme.
    pub typeface: String,
    pub size: PrintLength<Pt>,
    pub colour: Rgba,
    pub weight: Weight,
    pub slant: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    pub anchor: TextAnchor,
    pub align: TextAlign,
    /// The space between each edge of the box and the text.
    pub inset: PrintLength<Pt>,
}

/// Where the text sits in its box, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAnchor {
    Top,
    Middle,
}

/// How each line of the text sits in its box, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Centre,
}

/// The encoded bytes of a picture, a PNG or a JPEG file:
/// `Payload<PictureBytes>`.
pub enum PictureBytes {}

impl PayloadKind for PictureBytes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("PictureBytes");
}

/// The thumbnail a board holds of a picture is a PNG file the deck carries.
impl Becomes<PictureBytes> for ambre_core::ThumbnailPng {}

/// A picture a reader takes out of a file is a file a decoder reads.
#[cfg(feature = "codec")]
impl Becomes<ambre_client_core::FileBytes> for PictureBytes {}

/// A picture, stretched to the box of its placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Picture {
    pub name: String,
    pub at: Placement,
    pub format: PictureFormat,
    /// A file of `format`. The writer refuses other bytes, so a reader of
    /// the file never meets a part its type misnames.
    pub bytes: Payload<PictureBytes>,
}

/// The encoding of a picture's bytes. A presentation embeds others, a
/// metafile or a TIFF, which the reader names in its report instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PictureFormat {
    Png,
    Jpeg,
}

/// A line between two points, bound to the shapes at its ends where it has
/// them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connector {
    pub name: String,
    pub route: Route,
    pub start: ConnectorEnd,
    pub end: ConnectorEnd,
    pub line: Line,
    pub start_cap: Cap,
    pub end_cap: Cap,
}

/// The path a connector takes between its ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    Straight,
    /// Two right angles, PowerPoint's elbow connector.
    Elbow,
    Curved,
}

/// One end of a connector: where it lies, and the shape it holds on to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectorEnd {
    pub at: SlidePoint,
    /// `None` leaves the end free: PowerPoint keeps it where it lies when a
    /// shape moves.
    pub attached: Option<Attachment>,
}

/// The shape an end of a connector holds on to, and at which site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The item of the same slide, which must be a shape. The writer leaves
    /// an end free that names a picture, a connector or no item.
    pub item: Pos<SlideItems>,
    pub site: Site,
}

/// The middle of one edge of a box, where a connector holds on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Site {
    Top,
    Left,
    Bottom,
    Right,
}

/// What one end of a connector draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cap {
    Plain,
    Arrow,
}

/// A deck a reader read from a file, and what the file held that the deck
/// could not hold whole. It crosses out of the converter module as postcard
/// bytes, and an application maps it onto its own document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportedDeck {
    pub deck: Deck,
    pub report: crate::report::DeckReport,
}
