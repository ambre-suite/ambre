//! Toile (whiteboard) data plane (`ambre-spec-toile-schema.md`): the whole
//! typed content vocabulary of a board, both directions. It lives in the
//! application's crate, beside the engine and the view that use it, and the
//! Sync worker never builds against it (`BUS-032`, `ARCH-070`). The Logic
//! runtime carries it as `AppCommand::Toile` and `AppEvent::Toile`. The
//! stateful engine (`ToileState`) and its `ToileOutcome` wrapper sit beside it
//! under `logic`.
//!
//! * Pure data only: identity, element kinds, the single per-element style
//!   register, freehand strokes, connector anchoring, the culled
//!   `DisplayViewport`, and the command / event enums.
//! * No `LoroDoc` handle, no `unsafe`, US-keyboard characters only.

use ambre_core::crdt::CrdtName;
use std::collections::BTreeMap;
use std::fmt;

pub use ambre_core::FontFamily;
use ambre_core::id::{Connector, Document, Element, Frame, Id, Thread, User};
use ambre_core::index::{Axis, Len, Numbered, Pos, Shown};
use ambre_core::{
    Board, Bounded, Count, DocType, FontSize, Px, Rgba, Rule, Slant, StoredBoard, Strikethrough,
    Underline, Validated,
};
use kurbo::{Affine, BezPath, Point, Rect, Size, Vec2};
use serde::{Deserialize, Serialize};

// =============================================================================
// Identity -- minted, move-invariant ids (`ambre-spec-toile-schema.md`)
// =============================================================================

// A node, a connector, a frame and a comment thread each carry an `Id` of
// `ambre_core::id`, minted once as `"{peer:016x}_{counter:08x}"` and never
// reused. The id is a value, never a Loro op id, so a `LoroMovableList::mov`
// reorder leaves every reference valid. Two peers that create a node at the
// same time mint distinct ids (distinct `PeerID`), and both nodes survive
// (`ambre-spec-toile-schema.md`).

/// Z-order node identity (`toile-z-order`): `layers` interleaves
/// elements and connectors in a single back-to-front paint order, so a connector
/// can sit behind an element.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum NodeId {
    Element(Id<Element>),
    Connector(Id<Connector>),
}

impl NodeId {
    /// Encode for storage in the `layers` `LoroMovableList` as a plain string:
    /// `"E:{id}"` for an element, `"C:{id}"` for a connector. Chosen over a Loro
    /// sub-map so the list stays a flat, order-preserving sequence.
    pub fn encode(&self) -> String {
        match self {
            NodeId::Element(id) => format!("E:{id}"),
            NodeId::Connector(id) => format!("C:{id}"),
        }
    }

    /// Inverse of [`encode`](Self::encode). Returns `None` on a malformed tag
    /// (a defensive guard, never expected on a well-formed document).
    pub fn decode(s: &str) -> Option<Self> {
        let (tag, rest) = s.split_once(':')?;
        match tag {
            "E" => Some(NodeId::Element(Id::from_text(rest.to_owned()))),
            "C" => Some(NodeId::Connector(Id::from_text(rest.to_owned()))),
            _ => None,
        }
    }
}

// =============================================================================
// Element kinds (`toile-elements`)
// =============================================================================

/// Discriminant of a board node. Also tags a [`DisplayElement`].
///
/// `Drawing` and `Group` are valid *types* but are **not** creatable via
/// `CreateElement` (`toile-elements`): only `CommitStroke` creates a stroke, and
/// only `GroupElements` creates a group. `handle_command` rejects such a `kind`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ElementKind {
    Sticky,
    Shape(ShapeKind),
    Text,
    Image,
    Frame,
    Group,
    Drawing,
    Embed,
    DocEmbed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Triangle,
    Line,
    Path,
}

/// Vector boolean rule for [`ToileCommand::BooleanOp`]
/// (`toile-vector-paths`), run over the selected shape / path elements.
/// `Subtract` treats the first element as the subject and removes the union of
/// the rest; the others fold left to right over all selected elements.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

/// What a [`ToileCommand::BooleanOp`] does to the elements it combines.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Sources {
    /// The inputs are deleted in the same commit as the result.
    #[default]
    Consume,
    /// The inputs stay beside the result.
    Keep,
}

impl ElementKind {
    /// The persisted `"type"` tag (`ambre-spec-toile-schema.md`). A shape stores its
    /// `ShapeKind` separately in the payload, so every shape maps to `"shape"`
    /// here.
    pub fn type_tag(&self) -> &'static str {
        match self {
            ElementKind::Sticky => "sticky",
            ElementKind::Shape(_) => "shape",
            ElementKind::Text => "text",
            ElementKind::Image => "image",
            ElementKind::Frame => "frame",
            ElementKind::Group => "group",
            ElementKind::Drawing => "drawing",
            ElementKind::Embed => "embed",
            ElementKind::DocEmbed => "doc_embed",
        }
    }
}

impl ShapeKind {
    pub fn tag(&self) -> &'static str {
        match self {
            ShapeKind::Rect => "rect",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Triangle => "triangle",
            ShapeKind::Line => "line",
            ShapeKind::Path => "path",
        }
    }

    pub fn from_tag(s: &str) -> Option<Self> {
        match s {
            "rect" => Some(ShapeKind::Rect),
            "ellipse" => Some(ShapeKind::Ellipse),
            "triangle" => Some(ShapeKind::Triangle),
            "line" => Some(ShapeKind::Line),
            "path" => Some(ShapeKind::Path),
            _ => None,
        }
    }
}

// =============================================================================
// Style register -- one LWW binary per element (`toile-styles`)
// =============================================================================

/// Font attributes for sticky / text bodies. `underline` / `strikethrough` are
/// the text decorations, and they match Trame's cell font attributes. The shared
/// format controls (`ambre_chrome`) therefore drive one field set in both apps.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct FontSpec {
    /// The family the user picked, or `None` when the style names none. The
    /// two draw the same face, and only the picker tells them apart.
    pub family: Option<FontFamily>,
    /// The size, or automatic: a note's body then tracks its card.
    pub size: FontSizeChoice,
    pub weight: ambre_core::Bounded<ambre_core::FontWeight>,
    pub italic: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: None,
            size: FontSizeChoice::DEFAULT,
            weight: ambre_core::Bounded::<ambre_core::FontWeight>::REGULAR,
            italic: Slant::Upright,
            underline: Underline::Plain,
            strikethrough: Strikethrough::Plain,
        }
    }
}

/// The face a style that names no family draws in.
static DEFAULT_FAMILY: FontFamily = FontFamily::Sans;

impl FontSpec {
    /// The family the renderer resolves to a face. A style that names no
    /// family draws in the default one.
    #[must_use]
    pub fn face(&self) -> &FontFamily {
        self.family.as_ref().unwrap_or(&DEFAULT_FAMILY)
    }

    /// The label the font picker shows, or `None` for a family it does not
    /// offer. A style that names no family shows the "Default" entry, and a
    /// named family shows its own label (`TOILE-UI-397`).
    #[must_use]
    pub fn picker_label(&self) -> Option<&'static str> {
        self.family
            .as_ref()
            .map_or(Some("Default"), FontFamily::picker_label)
    }
}

/// Optional drop shadow. Its offset and its blur are board lengths, so the
/// camera scales them with the shape. Each encodes as the bare `f32` it was.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Shadow {
    pub color: Rgba,
    pub dx: Px<StoredBoard>,
    pub dy: Px<StoredBoard>,
    pub blur: Px<StoredBoard>,
}

/// The whole visual style of an element, held as **one** postcard-encoded LWW
/// register (`toile-styles`). One register means a fill update racing a stroke
/// update cannot leave a half-applied tear.
///
/// Schema note: postcard decodes the blob whole and tolerates no missing or
/// extra field. ANY layout change here or in `FontSpec` therefore makes a blob
/// from an older binary fall back to `default()` (`from_bytes` ->
/// `unwrap_or_default`). That is acceptable pre-1.0, since no persisted-format
/// guarantee holds yet. GA needs a versioned register.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ElementStyle {
    pub fill: Option<Rgba>, // None = no fill
    pub stroke: Option<Rgba>,
    pub stroke_w: Px<StoredBoard>,
    pub opacity: ambre_core::Bounded<ambre_core::Opacity>,
    /// The corner radius of a rectangle or a sticky.
    pub corner: Px<StoredBoard>,
    pub font: FontSpec,
    pub shadow: Option<Shadow>,
    /// Text (foreground) colour, RGBA. `None` = the kind's default ink (a
    /// sticky's fixed readable ink, or the theme text colour for a free text
    /// element). Distinct from `fill`, which is the card / shape background.
    pub text_color: Option<Rgba>,
}

impl Default for ElementStyle {
    fn default() -> Self {
        Self {
            fill: Some(Rgba::from_bits(0xFFFF_FFFF)),
            stroke: Some(Rgba::from_bits(0x3333_33FF)),
            stroke_w: Px::<StoredBoard>::new(1.0),
            opacity: ambre_core::Bounded::<ambre_core::Opacity>::OPAQUE,
            corner: Px::<StoredBoard>::ZERO,
            font: FontSpec::default(),
            shadow: None,
            text_color: None,
        }
    }
}

/// Longest frame title the document carries, in characters (`TOILE-249`). A
/// title names a region of a board and is read in a one-line field, so nothing
/// legible reaches this. The cut is what keeps an unbounded register out of a
/// document every peer replicates.
pub const FRAME_NAME_MAX: Len<ambre_core::Char> = Len::<ambre_core::Char>::new(120);

/// The default sticky-note card colour (RGBA): a warm pastel yellow, the Miro /
/// Figma-whiteboard default. Used as the seeded fill for a `sticky` element and
/// as the Sticky tool's default colour (`toile-tools`).
pub const STICKY_DEFAULT_FILL: Rgba = Rgba::from_bits(0xFEF0_8AFF);

/// Corner radius (world units) of a sticky-note card. A sticky is a plain
/// rectangle (Miro-style), so this is zero. A named constant keeps the card's
/// shape spec-anchored and easy to revisit.
pub const STICKY_CORNER: Px<StoredBoard> = Px::<StoredBoard>::ZERO;

/// The size of a text body: automatic, or a positive size the style names.
///
/// Automatic sizes a note's body from its card (`toile-styles`). A free text
/// has no card, so it draws an automatic size at [`FontSizeChoice::DEFAULT`].
/// A named size is never zero, so the two meanings cannot collide. The type is
/// stored as the bare `f32` of points, where zero is automatic, so the style
/// register keeps its layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct FontSizeChoice(Option<FontSize>);

/// The size behind [`FontSizeChoice::DEFAULT`].
const DEFAULT_FONT_SIZE: FontSize = FontSize::from_whole_points(12);

/// The largest named size a board carries: 4000 points, the cap of the
/// presentation formats (`CANVAS-088`).
pub const TEXT_FONT_SIZE_MAX: FontSize = FontSize::from_whole_points(4000);

impl FontSizeChoice {
    /// The automatic size.
    pub const AUTO: Self = Self(None);

    /// The size a fresh element names, and the one an automatic free text
    /// draws at: 12 points.
    pub const DEFAULT: Self = Self(Some(DEFAULT_FONT_SIZE));

    /// The named size `size`, or `None` for a zero size, which names nothing.
    #[must_use]
    pub fn fixed(size: FontSize) -> Option<Self> {
        (size.points() > 0.0).then_some(Self(Some(size)))
    }

    /// The choice a size picker holds: `None` is automatic, and so is a
    /// zero, which no picker lists.
    #[must_use]
    pub fn picked(size: Option<FontSize>) -> Self {
        size.and_then(Self::fixed).unwrap_or(Self::AUTO)
    }

    /// The choice a whole-point picker holds: `None` is automatic, and so is
    /// a zero, which no picker lists.
    #[must_use]
    pub fn from_whole_points(points: Option<u16>) -> Self {
        Self::picked(points.map(FontSize::from_whole_points))
    }

    /// The named size, or `None` when the size is automatic.
    #[must_use]
    pub const fn as_fixed(self) -> Option<FontSize> {
        self.0
    }

    /// The size rounded to whole points, as a picker lists it, or `None`
    /// when the size is automatic.
    #[must_use]
    pub fn whole_points(self) -> Option<u16> {
        self.0.map(FontSize::whole_points)
    }

    /// The size a free text draws at: the named size, or the default when
    /// the size is automatic. The board and the export both read it, so a
    /// text leaves at the size it was drawn with.
    #[must_use]
    pub fn or_default(self) -> FontSize {
        self.0.unwrap_or(DEFAULT_FONT_SIZE)
    }
}

impl TryFrom<f32> for FontSizeChoice {
    type Error = ambre_core::Refused<ambre_core::FontPoints>;

    /// Zero points read as automatic; a negative or non-finite number is
    /// refused. A size past [`TEXT_FONT_SIZE_MAX`] reads as it (`CANVAS-088`).
    fn try_from(points: f32) -> Result<Self, Self::Error> {
        let size = FontSize::try_from(points)?.at_most(TEXT_FONT_SIZE_MAX);
        Ok(Self::fixed(size).unwrap_or(Self::AUTO))
    }
}

impl From<FontSizeChoice> for f32 {
    fn from(choice: FontSizeChoice) -> Self {
        choice.0.map_or(0.0, FontSize::points)
    }
}

ambre_core::define_layout_version! {
    /// The style register's own layout version, its first byte (`TOILE-UI-071`).
    /// A later layout change adds a variant and a migration row, so an older
    /// build's style survives instead of resetting to the default.
    pub enum StyleLayout {
        /// The postcard value of [`ElementStyle`] behind the version byte.
        V1 = 1,
    }
    current = V1;
}

impl ElementStyle {
    /// The style seeded at element birth (`toile-styles`, "Per-kind
    /// defaults"). Every kind uses [`ElementStyle::default`] EXCEPT `sticky`. A
    /// sticky gets the sticky yellow, no border, `corner = 0` (a plain card) and
    /// an automatic font size. A fresh, still-empty note is therefore
    /// visible at once, and its text tracks the card size. The Sticky tool
    /// overrides the fill per placement via `CreateElement.fill`.
    pub fn for_kind(kind: ElementKind) -> Self {
        match kind {
            ElementKind::Sticky => Self {
                fill: Some(STICKY_DEFAULT_FILL),
                stroke: None,
                stroke_w: Px::<StoredBoard>::ZERO,
                corner: STICKY_CORNER,
                font: FontSpec {
                    size: FontSizeChoice::AUTO,
                    ..FontSpec::default()
                },
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    /// Serialize for the element's `"style"` binary register. Infallible in
    /// practice (a fixed, all-owned struct); a failure logs a line and yields an
    /// empty blob, which decodes back to `default()`.
    ///
    /// The first byte is [`StyleLayout`], the register's own layout version
    /// (`TOILE-UI-071`). It lets a later layout change add a migration instead
    /// of resetting every element an older build drew.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![StyleLayout::CURRENT.byte()];
        out.extend_from_slice(&postcard::to_allocvec(self).unwrap_or_default());
        out
    }

    /// Decode a `"style"` register, saying so when the bytes will not decode.
    ///
    /// `None` marks a register this build does not understand, that is a layout
    /// a newer client wrote. The caller then owes the user a word about it
    /// (`SCHEMA-003`). An **empty** register is not that case. An element with
    /// no style carries no bytes and reads as the default, so it comes back as
    /// `Some(default)`.
    ///
    /// The first byte selects the layout (`TOILE-UI-071`). A version this build
    /// does not know is reported, never silently reset. A later layout change
    /// then adds a migration row that decodes the older version.
    pub fn try_from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes.split_first() {
            None => Some(Self::default()),
            Some((&first, body)) => match StyleLayout::from_byte(first)? {
                StyleLayout::V1 => postcard::from_bytes(body).ok(),
            },
        }
    }

    /// Merge a whole-register restyle value over one element's own style
    /// (`TOILE-UI-174`). The fields a surface shows come from `sent`. The
    /// fields no surface shows -- the corner radius, the shadow, a non-bold
    /// exact weight -- come from `self`. A restyle of a mixed selection then
    /// cannot square off one member's corner or drop its shadow.
    pub fn merged_over(&self, sent: &Self) -> Self {
        let mut out = sent.clone();
        out.corner = self.corner;
        out.shadow = self.shadow;
        // The bold toggle writes 700 or 400. A sent non-bold weight must not
        // collapse an element's own non-bold weight (500, 600), which the
        // surface folds to "not bold" and never shows exactly.
        if !sent.font.weight.is_bold() && !self.font.weight.is_bold() {
            out.font.weight = self.font.weight;
        }
        out
    }
}

// =============================================================================
// Links on an element (`toile-links`)
// =============================================================================

/// Where a link on an element goes. Three navigations and no more: an address,
/// a place on this board, and another document. Anything else is a command, and
/// a command belongs to a menu (`toile-links` section 5).
///
/// The document arm carries its type beside its id, like an embed's does.
/// Without the type no route builds, and asking the server for it turns a
/// follow into a round trip that can fail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkTarget {
    /// A web address, opened by the shell in a sandboxed tab. The board never
    /// fetches it (`TOILE-110`).
    Url(ambre_core::LinkUrl),
    /// A frame on this board: the camera fits to it (`TOILE-113`).
    Frame(Id<Element>),
    /// Another Ambre document, opened by the shell (`TOILE-114`).
    Document {
        document_id: Id<Document>,
        doc_type: DocType,
    },
}

impl LinkTarget {
    /// Encode for the `"link"` register.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    /// Decode a `"link"` register. A malformed blob reads as no link rather
    /// than as a link nobody can follow.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        postcard::from_bytes(bytes).ok()
    }
}

/// The rule of the address of a web page that an embed frames
/// (`toile-embeds`): an `https` address with a host. A
/// `Validated<WebAddress>` holds one, with its scheme written out.
///
/// The rule is the only way in, for a typed address and a stored one alike.
/// Deserialisation goes through it too. A peer's `javascript:` or `data:`
/// text therefore never becomes an address, and never reaches a frame
/// (`TOILE-088`, `TOILE-267`). A user typed the address, so `Debug` prints
/// its length alone.
///
/// **Only `https` survives.** A bare `example.com` reads as
/// `https://example.com`, and every explicit scheme other than `https` gets
/// a refusal, `http` included. Two reasons, and neither is taste.
///
/// * A `javascript:`, `data:` or `blob:` frame executes with the embedder's
///   own origin as its opener: the attack `TOILE-088` names.
/// * The browser blocks a plain-`http` frame inside a page served over TLS
///   as mixed content. Accepting one would put a box on the board that never
///   paints.
///
/// The rule parses by hand rather than through a URL crate. The board needs
/// the scheme and a non-empty host and nothing else. A conforming parser
/// would carry the IDNA tables into the wasm bundle for the rest. A parsed
/// address parses again to itself, so a stored address reads back as it was
/// written.
///
/// # Errors
///
/// * [`AddressError::Empty`] on an empty text.
/// * [`AddressError::NotHttps`] on any scheme other than `https`.
/// * [`AddressError::NoHost`] on a missing host, or a host holding a
///   character no host may carry.
pub enum WebAddress {}

impl Rule for WebAddress {
    type Error = AddressError;

    fn admit(text: String) -> Result<String, AddressError> {
        // A control character or a space inside an address is never legitimate.
        // It is a paste accident, or an attempt to hide a scheme from a naive
        // prefix test (`java\nscript:`). Strip the ends, and refuse the middle.
        let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c.is_control());
        if trimmed.is_empty() {
            return Err(AddressError::Empty);
        }
        if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(AddressError::NoHost);
        }

        let rest = match split_scheme(trimmed) {
            // An explicit scheme: `https` and nothing else.
            Some((scheme, rest)) => {
                if !scheme.eq_ignore_ascii_case("https") {
                    return Err(AddressError::NotHttps);
                }
                rest.strip_prefix("//").unwrap_or(rest)
            }
            // No scheme at all: the user typed a host, and https is what they get.
            None => trimmed,
        };

        // The authority ends at the first `/`, `?` or `#`; what is left must be
        // a host. A leading `@` would make the host the part after it
        // (userinfo), so an empty one there is as empty as no host at all.
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or_default();
        if host.is_empty() || host.starts_with(':') {
            return Err(AddressError::NoHost);
        }

        Ok(format!("https://{rest}"))
    }
}

/// Why a text is not an address a frame may load. One variant per message
/// the user reads, so the wording lives with the rule rather than at the
/// dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressError {
    /// The text is empty.
    Empty,
    /// A scheme other than `https`. This is the security-bearing refusal: a
    /// `javascript:` or a `data:` address in a frame runs in the embedder's
    /// stead, which is exactly what `TOILE-088` forbids.
    NotHttps,
    /// `https://` with nothing after it, or a host carrying a character no host
    /// may hold.
    NoHost,
}

impl AddressError {
    /// What the dialog shows. Says what to do, not what went wrong.
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "Type a web address",
            Self::NotHttps => "Only https addresses can be embedded",
            Self::NoHost => "That address has no site in it",
        }
    }
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Split `s` at an explicit URL scheme, returning it and the remainder. `None`
/// when the text carries no scheme. A `host:port` deliberately carries none:
/// what follows its colon is a port rather than a path.
fn split_scheme(s: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = s.split_once(':')?;
    // RFC 3986: a scheme starts with a letter and holds letters, digits and
    // `+ - .`. A `host:8080` matches that shape too, so the digits after the
    // colon are what tells the two apart.
    let shaped = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let is_port = {
        let digits = rest.split(['/', '?', '#']).next().unwrap_or_default();
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
    };
    (shaped && !is_port).then_some((scheme, rest))
}

/// The link target of a web page, once its address passed the rule of a
/// [`WebAddress`].
#[must_use]
pub fn link_url(address: Validated<WebAddress>) -> ambre_core::LinkUrl {
    ambre_core::LinkUrl::new(address.into_string())
}

// =============================================================================
// Freehand stroke sample (`toile-freehand`)
// =============================================================================

/// One sampled point of a freehand stroke. 12 bytes, fixed layout: three
/// `f32`s, the coordinates in board units then the pressure.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: Px<StoredBoard>,
    pub y: Px<StoredBoard>,
    /// How hard the pen pressed, [`Bounded::<UnitFraction>::ONE`] when the input device
    /// reports none. The type refuses a value outside `0 ..= 1`; the board
    /// reader clamps a stored one (`TOILE-268`).
    pub pressure: Bounded<UnitFraction>,
}

impl StrokePoint {
    /// A sample at `(x, y)` on the board, pressed as hard as a device that
    /// reports no pressure reads.
    #[must_use]
    pub const fn full_pressure(x: Px<StoredBoard>, y: Px<StoredBoard>) -> Self {
        Self {
            x,
            y,
            pressure: Bounded::<UnitFraction>::ONE,
        }
    }
}

// =============================================================================
// Connector anchoring (`toile-connectors`)
// =============================================================================

pub use ambre_core::UnitFraction;

/// Where a connector attaches on an element's AABB. `Free` is a fraction of
/// the box's width and height, from the top-left.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Anchor {
    Top,
    Bottom,
    Left,
    Right,
    Center,
    Free {
        u: Bounded<UnitFraction>,
        v: Bounded<UnitFraction>,
    },
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum PathType {
    Straight,
    Elbow,
    Curved,
}

ambre_core::define_flag! {
    /// What one end of a connector draws.
    pub enum Cap {
        /// The line ends with no head.
        Plain,
        /// The line ends in an arrowhead.
        Arrow,
    }

    /// Whether a nested document or a web embed takes the input
    /// (`toile-embeds`, "Input focus"). A click enters it, and a drag moves it.
    pub enum InputFocus {
        /// The board takes the input, so the pointer passes through the embed.
        Board,
        /// The embed takes the input.
        Entered,
    }
}

/// Arrowheads per connector end -- a restyle-class LWW field, persisted in the
/// CRDT and surfaced on the `DisplayConnector`. Each [`Cap`] writes the byte
/// of a `bool`.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct EndCaps {
    pub start: Cap,
    pub end: Cap,
}

// =============================================================================
// Viewport / camera (`toile-viewport`)
// =============================================================================

/// The smallest zoom the camera allows, and the floor every reader clamps to.
pub const MIN_ZOOM: ambre_core::Scale<ambre_core::Board, ambre_core::View> =
    ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(0.1);
/// The largest zoom the camera allows, and the ceiling every reader clamps to.
pub const MAX_ZOOM: ambre_core::Scale<ambre_core::Board, ambre_core::View> =
    ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(10.0);

/// The camera: a world-space center, a zoom factor, and the physical screen
/// size. Drives culling via [`to_world_rect`](Viewport::to_world_rect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub center: Point,
    /// Read through [`clamped_zoom`](Viewport::clamped_zoom), never raw.
    pub zoom: ambre_core::Scale<ambre_core::Board, ambre_core::View>,
    pub size: Size, // screen size in physical pixels
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            center: Point::ZERO,
            zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::ONE,
            size: Size::new(1280.0, 800.0),
        }
    }
}

impl Viewport {
    /// The zoom inside `MIN_ZOOM ..= MAX_ZOOM`, so a reader can divide by it
    /// (`TOILE-UI-281`). The fields are public, so the clamp sits on the read.
    pub fn clamped_zoom(&self) -> ambre_core::Scale<ambre_core::Board, ambre_core::View> {
        self.zoom.clamped(MIN_ZOOM, MAX_ZOOM)
    }

    /// The world-space rectangle currently visible: the screen box mapped back
    /// through the inverse view transform.
    pub fn to_world_rect(&self) -> Rect {
        let half = Vec2::new(self.size.width, self.size.height) / (2.0 * self.clamped_zoom().get());
        Rect::from_center_size(self.center, Size::new(2.0 * half.x, 2.0 * half.y))
    }
}

// =============================================================================
// Clipboard payload -- id-free, slot-indexed, portable across boards
// (`toile-elements`)
// =============================================================================

/// One captured CRDT scalar field value. It covers the four `LoroValue` leaf
/// kinds an element / connector map stores (`ambre-spec-toile-schema.md`). A
/// container field, such as a group's `children` list, never travels here: the
/// capture maps membership onto slots instead. A text field holds board
/// content, such as a sticky body or a frame name, so `Debug` prints its
/// length alone.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipValue {
    F64(f64),
    Bool(bool),
    Str(String),
    Bin(ambre_core::Payload<ClipBytes>),
}

impl std::fmt::Debug for ClipValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::F64(value) => f.debug_tuple("F64").field(value).finish(),
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Str(text) => f
                .debug_tuple("Str")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
            Self::Bin(bytes) => f.debug_tuple("Bin").field(bytes).finish(),
        }
    }
}

/// The role of the bytes of one captured binary field, such as a style
/// blob: `Payload<ClipBytes>`. They travel as the bare bytes a `Vec<u8>`
/// serializes to, so a payload already on the clipboard reads back the same.
/// They may hold board content, so `Debug` prints the length alone.
pub enum ClipBytes {}

impl ambre_core::PayloadKind for ClipBytes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ClipBytes");
}

ambre_core::crdt_names! {
    /// The fields of one element map. Each kind writes a part of the set.
    pub enum ElementKey: FieldKey {
        /// The element type tag, as `ElementKind::type_tag` spells it.
        Type = "type",
        /// The world AABB, top-left, before the rotation.
        X = "x",
        Y = "y",
        W = "w",
        H = "h",
        /// The rotation in radians, about the AABB centre.
        Rot = "rot",
        /// The advisory lock.
        Locked = "locked",
        /// The whole style, one register (`TOILE-018`).
        Style = "style",
        /// The link target. An absent key means no link.
        Link = "link",
        /// The frame title.
        Name = "name",
        /// The body of a sticky or a text element, a text container
        /// (`TOILE-030`).
        Body = "text",
        /// The shape kind tag of a shape.
        Shape = "shape",
        /// The SVG path data of a vector node.
        Path = "path",
        /// The raw stroke samples of a drawing.
        Points = "points",
        /// The key of an image in `media`.
        Hash = "hash",
        /// The address of an embed.
        Url = "url",
        /// The reference of a nested document.
        DocId = "doc_id",
        DocType = "doc_type",
        /// The auto-layout register of a frame. An absent key means free
        /// positioning.
        Autolayout = "autolayout",
        /// The member list of a group or a frame.
        Children = "children",
        /// The speaker notes of a frame, a text container.
        Notes = "notes",
    }
}

ambre_core::crdt_names! {
    /// The fields of one connector map.
    pub enum ConnectorKey: FieldKey {
        /// The endpoint element ids.
        From = "from",
        To = "to",
        /// The endpoint anchors.
        FromAnch = "from_anch",
        ToAnch = "to_anch",
        PathType = "path_type",
        /// The two end caps.
        ArrowStart = "arrow_start",
        ArrowEnd = "arrow_end",
        /// The edge caption, a text container seeded at birth (`TOILE-047`).
        Label = "label",
        /// The whole style, one register.
        Style = "style",
    }
}

/// A schema of map fields a copied element or connector carries. A paste
/// reads only the keys the schema names.
trait ClipKey: Sized {
    fn parse_key(word: &str) -> Option<Self>;
}

macro_rules! clip_key {
    ($($name:ident),+) => {$(
        impl ClipKey for $name {
            fn parse_key(word: &str) -> Option<Self> {
                Self::parse(word)
            }
        }

        /// A key travels as the word the board stores, as the `String` it
        /// replaced did, so a payload already on the clipboard reads the same.
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
    )+};
}

clip_key!(ElementKey, ConnectorKey);

/// Reads captured fields, skipping each one whose key the schema `K` does not
/// name. A payload a newer board wrote still pastes what this one reads.
fn known_fields<'de, D, K>(deserializer: D) -> Result<Vec<(K, ClipValue)>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: ClipKey,
{
    let fields: Vec<(String, ClipValue)> = Deserialize::deserialize(deserializer)?;
    Ok(fields
        .into_iter()
        .filter_map(|(key, value)| Some((K::parse_key(&key)?, value)))
        .collect())
}

/// An element captured id-free: the full scalar field set of its map (type,
/// geometry, style blob, per-type payload). Referenced elsewhere in the payload
/// by its **slot** (index in [`ClipboardPayload::elements`]), never by id, so an
/// old id can never leak into the paste. `Debug` prints the length of the
/// notes, and none of their text.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipElement {
    #[serde(deserialize_with = "known_fields")]
    pub fields: Vec<(ElementKey, ClipValue)>,
    /// A frame's speaker notes, captured as their text (`TOILE-137`). They are
    /// a shared text container on the board, not a scalar field. So they travel
    /// beside the fields, and are written back into a container of their own.
    /// `None` for every other kind.
    pub notes: Option<String>,
}

impl std::fmt::Debug for ClipElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipElement")
            .field("fields", &self.fields)
            .field(
                "notes",
                &self.notes.as_deref().map(ambre_core::RedactedChars::of),
            )
            .finish()
    }
}

/// A message's place in its comment thread, 0 for the opening message. It
/// means nothing outside that thread.
#[derive(PartialEq, Eq)]
pub enum ThreadMessage {}

impl Axis for ThreadMessage {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ThreadMessage");
}

/// The rank of an element in one clipboard payload: its place in
/// [`ClipboardPayload::elements`]. It means nothing outside that payload.
#[derive(PartialEq, Eq)]
pub enum ClipSlot {}

impl Axis for ClipSlot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ClipSlot");
}

/// A connector captured with both endpoints remapped to element slot indices.
/// A connector whose partner was not copied is never captured (it would dangle),
/// so both slots always resolve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipConnector {
    pub from_slot: Pos<ClipSlot>,
    pub to_slot: Pos<ClipSlot>,
    /// Non-endpoint scalar fields (anchors, path type, arrows).
    #[serde(deserialize_with = "known_fields")]
    pub fields: Vec<(ConnectorKey, ClipValue)>,
}

/// Group / frame membership remapped from ids to element slot indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipMembership {
    pub parent_slot: Pos<ClipSlot>,
    pub child_slots: Vec<Pos<ClipSlot>>,
}

/// Self-contained, id-free snapshot of a selection produced by `CopyElements`
/// (`toile-clipboard`). Portable across boards and sessions (postcard
/// round-trips for the OS clipboard). The payload names media by content hash
/// only, and the blobs stay in the store (`toile-media`). A `doc_embed` copies the
/// same `document_id`, that is a second reference to the one nested document,
/// not a clone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPayload {
    pub elements: Vec<ClipElement>,
    pub connectors: Vec<ClipConnector>,
    pub membership: Vec<ClipMembership>,
    /// Source union bounds, for the paste-at-cursor offset.
    pub origin: Rect,
}

// =============================================================================
// Commands (UI/Logic -> ToileState)
// =============================================================================

/// Type-specific seed passed to `CreateElement`. The variant must match `kind`
/// (the handler rejects a mismatch). `Debug` prints the length of a text
/// body alone.
#[derive(Clone)]
pub enum InitContent {
    None,
    Path(BezPath),
    Image(MediaRef),              // the picture's blob, key into `media` once stored
    Embed(Validated<WebAddress>), // web url (sandboxed iframe)
    DocEmbed {
        document_id: Id<Document>,
        doc_type: DocType,
    }, // nested Ambre document (`toile-embeds`)
    Text(String),                 // initial plain body (sticky / text)
}

impl fmt::Debug for InitContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Image(media) => f.debug_tuple("Image").field(media).finish(),
            Self::Embed(address) => f.debug_tuple("Embed").field(address).finish(),
            Self::DocEmbed {
                document_id,
                doc_type,
            } => f
                .debug_struct("DocEmbed")
                .field("document_id", document_id)
                .field("doc_type", doc_type)
                .finish(),
            Self::Text(text) => f
                .debug_tuple("Text")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
        }
    }
}

/// The rank of a media blob in one import batch (`toile-media`). It names
/// the blob until Sync returns the blob's address, and it means nothing
/// outside that batch.
#[derive(PartialEq, Eq)]
pub enum ImportSlot {}

impl Axis for ImportSlot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportSlot");
}

/// The blob a picture names: one Sync stored, or one an import still holds.
///
/// An import names its blobs by slot until Sync answers. A pending blob is
/// therefore never spelled as an address, and never lands in the document
/// (`TOILE-IO-059`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaRef {
    /// The content address Sync returned.
    Stored(ambre_core::Validated<ambre_core::ContentHash>),
    /// The blob at this slot of an import, before Sync answers.
    Pending(Pos<ImportSlot>),
}

impl MediaRef {
    /// The address, or `None` while the blob is pending. An empty address
    /// is no address either.
    #[must_use]
    pub fn stored(&self) -> Option<&ambre_core::Validated<ambre_core::ContentHash>> {
        match self {
            Self::Stored(hash) if !hash.as_str().is_empty() => Some(hash),
            Self::Stored(_) | Self::Pending(_) => None,
        }
    }
}

/// A place in the board's z-order, 0 at the back.
#[derive(PartialEq, Eq)]
pub enum ZOrder {}

impl Axis for ZOrder {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ZOrder");
}

/// Z-order target for `Reorder` (`toile-z-order`).
#[derive(Clone, Copy, Debug)]
pub enum ZTarget {
    Front,
    Back,
    Forward,
    Backward,
    Index(Pos<ZOrder>),
}

/// Which endpoint of a connector to retarget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    From,
    To,
}

/// Options for a board `Find` (`toile-find`). Search runs against
/// each element's plaintext body (sticky / text) and frame title; `case`
/// toggles case folding, `word` may require a word-boundary match. Replace is
/// out of MVP scope (`toile-find`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case: ambre_core::CaseMatch,
    pub word: ambre_core::WordMatch,
}

/// Result of a board `Find` (`toile-find`). `hits` are the matching
/// elements in stable z-order; `focus` is the element to recenter the camera on
/// (the next hit after `from`, wrapping around). Purely local view state -- never
/// synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindResults {
    pub hits: Vec<Id<Element>>,
    pub focus: Option<Id<Element>>,
}

/// A single char-CRDT splice on a `LoroText` (`toile-text-editing`,
/// speaker notes). Encodes any editor edit as "at codepoint `at`, remove `delete`
/// codepoints, then insert `insert`". Indices are loro unicode-codepoint
/// positions; the engine clamps them to the current length. This is the
/// `TextDelta` the specs defer for element text -- introduced here for notes
/// only, where the spec demands char-CRDT convergence (`toile-text-editing`).
/// `Debug` prints the length of `insert` alone.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct TextDelta {
    pub at: Pos<ambre_core::Char>,
    pub delete: Len<ambre_core::Char>,
    pub insert: String,
}

impl fmt::Debug for TextDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextDelta")
            .field("at", &self.at)
            .field("delete", &self.delete)
            .field("insert", &ambre_core::RedactedChars::of(&self.insert))
            .finish()
    }
}

impl TextDelta {
    /// Where the delta lands in a text of `len` code points, and how many it
    /// removes there. Both are clamped to that text, so neither runs past its
    /// end.
    pub fn clamped(
        &self,
        len: Len<ambre_core::Char>,
    ) -> (Pos<ambre_core::Char>, Len<ambre_core::Char>) {
        let at = self.at.min(len.as_end());
        (at, self.delete.min(len.as_end().len_from(at)))
    }
}

/// One user's persisted reaction on a target (`toile-reactions`): an
/// emoji OR a signed vote weight, plus a wall-clock `ts` kept only as a display
/// tiebreak. Loro handles the LWW on a user's own key, since the map key is the
/// canonical `Id<User>`. This `ts` does not. Postcard-encoded into the reactions
/// sub-map value (`toile-reactions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub kind: ReactionKind,
    /// Stored as the `i64` the reactions sub-map was frozen with.
    #[serde(with = "ambre_core::span::stored_as_i64")]
    pub ts: Instant<Authored>,
}

/// The two reaction flavours (`toile-reactions`): a free-form emoji or
/// a signed vote weight (for example `+1` / `-1` / a weighted ballot).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionKind {
    Emoji(ambre_bus::Emoji),
    Vote(VoteWeight),
}

/// The signed weight one vote casts, for example `+1`, `-1` or a weighted
/// ballot. It travels and is stored as the bare `i32` it was.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct VoteWeight(i32);

impl VoteWeight {
    /// A vote of weight `weight`.
    pub const fn new(weight: i32) -> Self {
        Self(weight)
    }

    /// The weight itself.
    pub const fn get(self) -> i32 {
        self.0
    }
}

/// The sum of the vote weights cast on one target. A sum has no rank of its
/// own: it only grows or shrinks by a weight, and is read or printed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoteTotal(i64);

impl VoteTotal {
    /// No vote cast.
    pub const ZERO: Self = Self(0);

    /// A sum of `total`.
    pub const fn new(total: i64) -> Self {
        Self(total)
    }

    /// The sum itself.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// The sum with one more vote of `weight`, held at either end of `i64`.
    #[must_use]
    pub const fn plus(self, weight: VoteWeight) -> Self {
        Self(self.0.saturating_add(weight.0 as i64))
    }
}

/// Prints the bare sum, and honours the sign flag: `{:+}` prints `+3`.
impl std::fmt::Display for VoteTotal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

pub use ambre_core::Elements;

/// The shared import and export command, over what Toile reads and writes
/// (`INTEROP-001`).
pub type ToileInterop = ambre_bus::interop::InteropRequest<ToileImport, ToileExport>;

/// A file dropped or picked onto the board.
#[derive(Clone, Debug, PartialEq)]
pub struct ToileImport {
    pub format: ToileImportFormat,
    pub data: ambre_core::Payload<ambre_client_core::FileBytes>,
    /// The world point the file lands on: the drop point, or the centre of the
    /// viewport when the picker started the import. The whole batch moves so
    /// its top-left corner sits there. A file lands where the user put it,
    /// never at a fixed point of the world (`TOILE-IO-023`).
    pub at: Point,
}

/// The formats a file enters the board in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToileImportFormat {
    Svg,
    /// A raster picture, PNG or JPEG, under its media type.
    Image(ambre_core::MediaType),
    /// A board exported as JSON from Figma or from Miro. The boards module
    /// tells the two apart by the file's own keys (`toile-import-export`).
    Board,
}

/// The formats a board leaves in.
#[derive(Clone, Debug)]
pub enum ToileExport {
    /// A standalone SVG document of the whole board (`toile-import-export`).
    Svg,
    /// The deck, one page per slide (`toile-pdf-export`). The render walk needs
    /// the base font faces the Main thread fetched at boot, so the Logic thread
    /// serves it, not the board alone.
    Pdf,
    /// The deck as a PowerPoint file, one slide per slide of the deck
    /// (`toile-pptx-export`). The board maps itself onto the deck, and the
    /// Office converter module writes the package.
    Pptx,
    /// A standalone HTML page that draws the board with no relay and no
    /// script. It is the picture alone, and embeds no board state
    /// (`TOILE-IO-018`). It is plaintext for anyone who holds it, which the
    /// gesture that offers it must say (`TOILE-IO-096`, `ZK-037`).
    Showcase,
    /// The native `.ambre` file (`ambre-spec-docfile.md`). It drops nothing:
    /// the payload **is** the document (`DOCFILE-001`). It is also plaintext,
    /// which the surface that offers it must say (`DOCFILE-009`). The Logic
    /// thread serves it, as the container's header carries the identity only
    /// the registry knows.
    Ambre,
    /// The native file **sealed under `passphrase`** (`ambre-spec-docfile.md
    /// section 5`). The Logic thread writes the plaintext container and the
    /// Sync worker seals it (`ARCH-004`). A failure leaves no file: a partial
    /// seal is not a file the user can open.
    AmbreSealed {
        /// The passphrase the user typed for this file, and not the account's:
        /// nothing on this path can reach a session (`DOCFILE-014`).
        passphrase: ambre_core::Confidential<ambre_core::Passphrase>,
    },
}

/// A construct a board conversion carries or loses, as its report names it
/// (`INTEROP-003`). It names the thing on the board, never a variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToileConstruct {
    /// Any element, the unit an import counts what it carried in.
    Element,
    StickyNote,
    Group,
    Frame,
    Embed,
    EmbeddedDocument,
    FreehandStroke,
    Connector,
    /// A picture, as an element of the board.
    Picture,
    /// A text node the reader found no string in.
    Text,
    /// A raster picture the reader does not decode, GIF or WebP.
    UndecodedPicture,
    /// An SVG nested in an SVG, which the reader does not inline.
    NestedSvg,
    /// A fill or a stroke with a gradient or a pattern.
    NonSolidPaint,
    /// The speaker notes of one slide.
    SpeakerNotes,
    /// The caption a connector carries on its line.
    ConnectorLabel,
    /// A vector outline of a Figma file, or a boolean operation.
    VectorPath,
    /// A Miro card.
    Card,
    /// A table of a Figma or a Miro board.
    Table,
    /// A Figma widget or stamp.
    Widget,
    /// A node of a type the reader does not know.
    UnknownElement,
    /// A shadow or a blur.
    Effect,
    /// The rotation of a Figma node, which lands upright in its box.
    Rotation,
    /// The link of a Figma instance to its component.
    ComponentLink,
    /// The runs of a text styled apart: a bold word, a link.
    TextFormatting,
    /// An outline the board has no shape for, which lands as a rectangle.
    ForeignShape,
    /// A Figma page, laid beside the page before it.
    Page,
}

impl ambre_bus::interop::Construct for ToileConstruct {
    fn label(self) -> &'static str {
        match self {
            Self::Element => "element",
            Self::StickyNote => "sticky note",
            Self::Group => "group",
            Self::Frame => "frame",
            Self::Embed => "embed",
            Self::EmbeddedDocument => "embedded document",
            Self::FreehandStroke => "freehand stroke",
            Self::Connector => "connector",
            Self::Picture => "picture",
            Self::Text => "text",
            Self::UndecodedPicture => "GIF or WebP picture",
            Self::NestedSvg => "nested SVG",
            Self::NonSolidPaint => "gradient or pattern paint",
            Self::SpeakerNotes => "speaker notes",
            Self::ConnectorLabel => "connector label",
            Self::VectorPath => "vector path",
            Self::Card => "card",
            Self::Table => "table",
            Self::Widget => "widget",
            Self::UnknownElement => "unknown element",
            Self::Effect => "shadow or blur",
            Self::Rotation => "rotation",
            Self::ComponentLink => "component link",
            Self::TextFormatting => "text formatting",
            Self::ForeignShape => "shape with no board counterpart",
            Self::Page => "page",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Element => "elements",
            Self::Card => "cards",
            Self::Table => "tables",
            Self::Widget => "widgets",
            Self::UnknownElement => "unknown elements",
            Self::Rotation => "rotations",
            Self::ComponentLink => "component links",
            Self::ForeignShape => "shapes with no board counterpart",
            Self::Page => "pages",
            other => other.label(),
        }
    }
}

/// What a finished board conversion carried and lost.
pub type ToileReport = ambre_bus::interop::ConversionReport<ToileConstruct>;

/// How many votes were cast on one target.
pub enum Votes {}

impl ambre_core::NarrowCount for Votes {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("VoteCount");
}

/// How many users reacted to one target with one emoji.
pub enum Reactions {}

impl ambre_core::NarrowCount for Reactions {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ReactionCount");
}

pub use ambre_core::IdCounter;
use ambre_core::{Authored, Instant, PageLoad};

/// Read-time count derived from a target's reaction sub-map
/// (`toile-reactions`). Never stored
/// -- recomputed on read so it can never drift from the per-user entry set.
/// `emoji` is sorted for a stable render order; `vote_total` / `vote_count`
/// aggregate the `Vote` entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReactionCounts {
    pub emoji: Vec<(ambre_bus::Emoji, Count<Reactions>)>,
    pub vote_total: VoteTotal,
    pub vote_count: Count<Votes>,
}

// -----------------------------------------------------------------------------
// Comment threads (`toile-comments`) -- the asynchronous half of collaborating
// on a board. A thread is its own entry under a minted id, so two peers
// commenting on one spot mint distinct threads (`TOILE-FEAT-011`).
// -----------------------------------------------------------------------------

/// What a comment thread hangs on (`toile-comments`, `TOILE-024`). An element
/// anchor tracks the element through every move; a point anchor carries a remark
/// on empty board. A viewport coordinate is never an anchor: it means nothing on
/// the second screen.
///
/// Postcard-encoded into the thread map's `anchor` key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommentAnchor {
    Element(Id<Element>),
    Point(kurbo::Point),
}

/// One message of a thread, read from the CRDT (`toile-comments`). The
/// `author` is the canonical `Id<User>`, never a `PeerID` (`TOILE-FEAT-014`). The
/// reader sees the `ts`, and nothing orders by it (`TOILE-FEAT-012`).
/// `Debug` prints the length of the body alone.
#[derive(Clone, PartialEq, Eq)]
pub struct CommentMessage {
    pub author: Id<User>,
    pub ts: Instant<Authored>,
    pub body: String,
}

impl fmt::Debug for CommentMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommentMessage")
            .field("author", &self.author)
            .field("ts", &self.ts)
            .field("body", &ambre_core::RedactedChars::of(&self.body))
            .finish()
    }
}

/// A whole thread, read on demand (`toile-comments`). Derived on read, never
/// stored: the display viewport carries the pins alone, so a body reaches the UI
/// only through this projection (`TOILE-026`). `messages` is in the
/// `LoroMovableList` order.
#[derive(Clone, Debug, PartialEq)]
pub struct CommentThread {
    pub id: Id<Thread>,
    pub anchor: CommentAnchor,
    pub resolved: ambre_core::ThreadState,
    pub messages: Vec<CommentMessage>,
}

/// One pin of the derived viewport (`toile-comments`, `TOILE-026`). Carries
/// where the pin sits, how many messages the thread holds and the resolved
/// flag -- and no body (`TOILE-205`).
///
/// `at` is a world point: the anchor point itself, or the top-right corner of
/// the anchored element, which the pin follows through every move. The painter
/// maps it to screen and draws a constant-size marker there (`TOILE-UI-022`).
#[derive(Clone, Debug, PartialEq)]
pub struct CommentPin {
    pub thread: Id<Thread>,
    pub at: Point,
    pub messages: Len<ThreadMessage>,
    pub resolved: ambre_core::ThreadState,
}

/// One row of the board-wide thread list (`toile-comments`). The panel shows
/// every thread, including one whose anchor element is gone
/// (`TOILE-FEAT-016`). This list therefore reads the whole document, not the
/// culled viewport. `preview` is the opening message, so a reader recognises
/// the thread before opening it. The rest of the conversation arrives only when
/// the reader opens the thread (`TOILE-205`). `Debug` prints the length of
/// the preview alone.
#[derive(Clone, PartialEq)]
pub struct CommentSummary {
    pub id: Id<Thread>,
    pub anchor: CommentAnchor,
    pub resolved: ambre_core::ThreadState,
    pub messages: Len<ThreadMessage>,
    pub preview: String,
}

impl fmt::Debug for CommentSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommentSummary")
            .field("id", &self.id)
            .field("anchor", &self.anchor)
            .field("resolved", &self.resolved)
            .field("messages", &self.messages)
            .field("preview", &ambre_core::RedactedChars::of(&self.preview))
            .finish()
    }
}

// -----------------------------------------------------------------------------
// Deck (`toile-presentation`) -- the projection the ordering panel reads.
// -----------------------------------------------------------------------------

/// One frame the deck panel lists (`toile-presentation`). `name` is the frame's
/// title, empty until the user names one. The panel then labels an unnamed
/// frame from its rank, a presentation concern that stays out of the engine.
/// `Debug` prints the length of the name alone.
#[derive(Clone, PartialEq)]
pub struct DeckFrame {
    pub id: Id<Frame>,
    pub name: String,
}

impl fmt::Debug for DeckFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeckFrame")
            .field("id", &self.id)
            .field("name", &ambre_core::RedactedChars::of(&self.name))
            .finish()
    }
}

/// The board's frames, split by whether the deck names them
/// (`toile-presentation`): the answer to a [`ReadDeck`](ToileCommand::ReadDeck).
/// `slides` is the deck in playback order. A slide whose frame is gone drops
/// out, so the deck holds only live frames (`TOILE-FEAT-027`). `off_deck`
/// is every other frame, in paint order, so the panel can offer them for adding
/// without deriving an order of its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeckView {
    pub slides: Vec<DeckFrame>,
    pub off_deck: Vec<DeckFrame>,
}

// -----------------------------------------------------------------------------
// Auto-layout spec (`toile-auto-layout`) -- the CRDT design intent carried
// by a frame. The pure `solve_layout` solver that turns this spec + child sizes
// into positions lives in the engine (`ambre_toile::logic::autolayout`); only the
// shared value types live here so `SetAutoLayout` can carry them across
// `AppCommand::Toile`.
// -----------------------------------------------------------------------------

/// Main-axis flow of an [`AutoLayout`] (`toile-auto-layout`). `Wrap` is a
/// horizontal flow that breaks into rows when a child would overflow the content
/// box width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Horizontal,
    Vertical,
    Wrap,
}

/// What an [`ToileCommand::AlignElements`] does to the selection it names
/// (`toile-elements section 1.7`).
///
/// The six alignments put every box on one edge or centre line of the
/// selection's union box. The two distributions pin the extreme boxes and
/// equalize the gaps between the rest. Distinct from [`Align`], which is a
/// persisted auto-layout rule on a frame. This one is a one-shot batch of
/// coordinates, and the document keeps no trace of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignOp {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
    DistributeH,
    DistributeV,
}

/// Distribution of children along the main axis within the free space
/// (`toile-auto-layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    Start,
    Center,
    End,
    SpaceBetween,
}

/// How the solver sizes a child on the main axis (`toile-auto-layout`). `Hug`
/// and `Fill` are the Figma resize modes. From the pure solver's view `Hug` ==
/// `Fixed`, since the frame rect is an input and "hug content" changes nothing
/// about the children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sizing {
    Hug,
    Fill,
    Fixed,
}

/// Auto-layout spec carried by a frame (`toile-auto-layout`). Serialized
/// with `postcard` into the frame element's `autolayout` CRDT register (absent =
/// free positioning). The spec carries shared design intent. The solver derives
/// the positions and never persists them (`toile-auto-layout`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoLayout {
    /// Main-axis flow.
    pub direction: Direction,
    /// Space between consecutive children on the main axis.
    pub gap: Px<Board>,
    /// The inset of the content box inside the frame.
    pub padding: Padding,
    /// Main-axis distribution of children in the free space.
    pub align: Align,
    /// Main-axis child sizing.
    pub sizing: Sizing,
}

/// The inset of a frame's content box, one length a side, in board units.
///
/// The sides are named, so none can be read as another. Postcard encodes them
/// in the order `top`, `right`, `bottom`, `left`, the bytes of the
/// `[f64; 4]` the register held before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Padding {
    pub top: Px<Board>,
    pub right: Px<Board>,
    pub bottom: Px<Board>,
    pub left: Px<Board>,
}

impl Padding {
    /// No inset.
    pub const ZERO: Self = Self::uniform(Px::<Board>::ZERO);

    /// The same inset on all four sides.
    #[must_use]
    pub const fn uniform(side: Px<Board>) -> Self {
        Self {
            top: side,
            right: side,
            bottom: side,
            left: side,
        }
    }
}

/// Per-viewer presentation playback (`toile-presentation`). Purely local,
/// momentary-gesture state: which slide this viewer is on and whether focus
/// (present) mode hides the chrome. Never a CRDT value and never synced -- only
/// the slide *sequence* (which frames, in what order) travels between peers
/// (`toile-presentation`). Mirrors the durable-intent vs
/// momentary-gesture split already used for presence (`toile-presence`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackState {
    /// Index into the shared slide `order`; clamped to the deck length.
    pub active: Pos<Slide>,
    /// Whether the presentation hides the editing chrome.
    pub focus: PresentFocus,
}

/// A slide's zero-based rank in the shared slide order
/// (`toile-presentation`). The deck names frames by id, and this rank
/// only says which of them a viewer is on.
#[derive(PartialEq, Eq)]
pub enum Slide {}

impl Axis for Slide {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> = ambre_core::StaticKey::new("Slide");
}

/// The number a person reads for a slide: the first slide is 1. It is the
/// count of slides up to and with this one, so it saturates with the axis.
impl Numbered for Slide {
    type Number = Len<Slide>;

    fn number_of(pos: Pos<Slide>) -> Len<Slide> {
        Len::through(pos)
    }
}

/// A deck shows its number of slides to a reader.
impl Shown for Slide {}

/// Whether a presentation hides the editing chrome (`toile-presentation`,
/// `EnterPresent { focus }`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentFocus {
    /// The editing chrome stays on screen around the slide.
    #[default]
    WithChrome,
    /// Focus mode: the UI hides its chrome and fits the active frame.
    Focus,
}

// =============================================================================
// Smart Animate snapshots (`toile-presentation`)
// =============================================================================

/// Animatable properties of one element captured for a slide
/// (`toile-presentation`). A frame's snapshot holds one of these per
/// declared child, in `children` order. Pure data. It lives here so the enriched
/// [`SlideChange`] event can carry both frame snapshots across `AppEvent::Toile`
/// to the render thread, which owns no board state. The `smart_animate` solver
/// that consumes these stays in the engine (`ambre_toile::logic::animate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapElement {
    pub id: Id<Element>,
    /// Position + size, world space (the element AABB).
    pub bounds: Rect,
    /// The rotation (the element `rot` field).
    pub rotation: ambre_core::Radians,
    /// Opacity (from [`ElementStyle::opacity`]).
    pub opacity: ambre_core::Bounded<ambre_core::Opacity>,
    /// Fill color, `None` = no fill (from [`ElementStyle::fill`]).
    pub fill: Option<Rgba>,
}

/// The animatable set of a frame for a transition: its declared children as
/// [`SnapElement`]s. Derived by the engine (`ToileState::frame_snapshot`), fed
/// to the Smart Animate solver.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameSnapshot {
    pub elements: Vec<SnapElement>,
}

impl FrameSnapshot {
    /// The captured element with `id`, if this frame contains it. Linear scan --
    /// the test-facing accessor mirroring the `toile-presentation`
    /// pseudo-code.
    pub fn by_id(&self, id: &Id<Element>) -> Option<&SnapElement> {
        self.elements.iter().find(|e| &e.id == id)
    }
}

/// Whether this viewer edits the board or presents it (`toile-presentation`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresentMode {
    /// The board is open for editing.
    #[default]
    Editing,
    /// The board is in presentation (focus) mode.
    Presenting,
}

impl PresentMode {
    /// The board is in presentation mode.
    pub fn is_presenting(self) -> bool {
        self == Self::Presenting
    }
}

/// Whether this viewer shares its slide position with its peers
/// (`toile-presentation`, "Follow presenter").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Broadcast {
    /// The position stays on this client.
    #[default]
    Private,
    /// Each slide change travels to the peers.
    Broadcasting,
}

impl Broadcast {
    /// The position travels to the peers.
    pub fn is_on(self) -> bool {
        self == Self::Broadcasting
    }
}

/// The slide on screen: a zero-based index into a deck of `count` slides.
/// The index is below the count, and both read 0 on an empty deck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlidePosition {
    index: Pos<Slide>,
    count: Len<Slide>,
}

impl SlidePosition {
    /// Slide `index` of a deck of `count`, the index pulled back into the deck.
    pub fn new(index: Pos<Slide>, count: Len<Slide>) -> Self {
        Self {
            index: index.within(count),
            count,
        }
    }

    /// The zero-based index of the slide on screen.
    pub fn index(self) -> Pos<Slide> {
        self.index
    }

    /// How many slides the deck holds.
    pub fn count(self) -> Len<Slide> {
        self.count
    }
}

/// Payload of a [`ToileEvent::SlideChanged`]: everything the `toile-presentation` camera
/// pass needs to glide to the new slide and Smart Animate its content
/// (`toile-presentation`). The Logic thread owns the
/// board, so it captures the two frame snapshots and the target frame rect here
/// in one shot. The UI holds no board state and cannot derive them itself. Local
/// view state, never crosses the wire.
#[derive(Clone, Default, PartialEq)]
pub struct SlideChange {
    /// The new active slide and the deck's length: the presenter panel's
    /// `k / n` counter (`toile-presentation`).
    pub position: SlidePosition,
    /// The previously active frame's children (empty on `EnterPresent`, or when
    /// the source frame is absent / dangling).
    pub from: FrameSnapshot,
    /// The new active frame's children.
    pub to: FrameSnapshot,
    /// The new active frame element's world rect: the camera fit target. `None`
    /// when the slide has no resolvable frame (dangling id), leaving the camera
    /// where it is.
    pub to_rect: Option<Rect>,
    /// The new active frame, or `None` when the slide has no resolvable frame
    /// (dangling id / empty deck). The presenter panel uses it as the target
    /// of `EditNotes`.
    pub frame: Option<Id<Frame>>,
    /// The new active frame's speaker notes as plain text, seeding the presenter
    /// panel's editor (`toile-presentation`). Empty when the frame has no
    /// notes yet or is absent.
    pub notes: String,
    /// How long this transition lasts: the deck's own `transition_ms`, read
    /// off the document (`TOILE-FEAT-028`). The render thread holds no board
    /// data, so the length rides the event like the snapshots beside it.
    pub transition: std::time::Duration,
}

// The speaker notes are the author's: a log carries their length alone.
impl fmt::Debug for SlideChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlideChange")
            .field("position", &self.position)
            .field("from", &self.from)
            .field("to", &self.to)
            .field("to_rect", &self.to_rect)
            .field("frame", &self.frame)
            .field("notes", &ambre_core::RedactedChars::of(&self.notes))
            .field("transition", &self.transition)
            .finish()
    }
}

/// The family of the token a `CreateElement` carries and its
/// `ElementCreated` echoes. The view that asked claims the element it
/// created and no other (`TOILE-UI-194`).
///
/// No token is zero, so "no deferred open" is an `Option` and never a
/// reserved value. The view mints the tokens, one after another.
pub enum ElementCreation {}

impl ambre_bus::token::TokenKind for ElementCreation {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("CorrelationToken");
}

/// Whether an element refuses every write (`TOILE-033`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LockState {
    #[default]
    Unlocked,
    Locked,
}

impl LockState {
    /// [`LockState::Locked`] when `locked` holds.
    #[must_use]
    pub fn locked_if(locked: bool) -> Self {
        if locked { Self::Locked } else { Self::Unlocked }
    }

    /// The flag as the board stores it.
    #[must_use]
    pub fn is_locked(self) -> bool {
        self == Self::Locked
    }
}

/// Command from the UI / Logic thread to `ToileState` (in `ambre-toile`).
/// `Debug` prints a text a user typed by its length alone.
#[derive(Clone)]
pub enum ToileCommand {
    // -- Element lifecycle --
    CreateElement {
        kind: ElementKind,
        at: Point,
        size: Size,
        content: InitContent,
        /// Optional creation-time override of the seeded `style.fill` (RGBA). The
        /// engine seeds a per-kind default style at birth (sticky = yellow,
        /// others = white). `Some(rgba)` overwrites just that fill and leaves
        /// the rest of the default alone. The Sticky tool uses it to place a
        /// note in the picked palette colour atomically (one element, one undo
        /// item). `None` keeps the per-kind default. See `toile-styles`.
        fill: Option<Rgba>,
        /// A request token the engine echoes in `ElementCreated`, so a
        /// deferred open claims the element this command created and no other
        /// (`TOILE-UI-194`). `None` when no deferred open is pending.
        correlate: Option<ambre_bus::token::Token<ElementCreation>>,
    },
    MoveElements {
        ids: Vec<Id<Element>>,
        delta: Vec2,
    },
    /// Align or distribute a selection (`toile-elements section 1.7`). One
    /// batch, one undo item. The handler computes every translation from the
    /// boxes of the named nodes, then writes them under a single commit. No
    /// peer therefore ever sees half an alignment.
    ///
    /// `ids` are the selected nodes, not their leaves: a group aligns by its own
    /// box and carries its subtree along. The handler skips an id nobody minted,
    /// and a locked element takes no part -- it neither moves nor counts in the
    /// union (`TOILE-234`).
    AlignElements {
        ids: Vec<Id<Element>>,
        op: AlignOp,
    },
    ResizeElement {
        id: Id<Element>,
        new: Rect,
    },
    RotateElement {
        id: Id<Element>,
        radians: ambre_core::Radians,
    },
    /// Ask for the pixels behind one or more media addresses
    /// (`toile-media`). The UI thread sends it when the walk meets a picture
    /// whose pixels it does not hold. Only what is on screen therefore crosses
    /// the bus: a board of two hundred photographs pays for the ones in view.
    ///
    /// The command only reads, and the board answers it per address with
    /// `MediaDecoded`. It answers an address the document does not carry with
    /// nothing, and the element keeps the placeholder it already paints.
    RequestMedia {
        hashes: Vec<ambre_core::Validated<ambre_core::ContentHash>>,
    },
    RestyleElements {
        ids: Vec<Id<Element>>,
        style: ElementStyle,
    },
    DeleteElements {
        ids: Vec<Id<Element>>,
    },
    /// Write the plaintext body of a sticky / text element
    /// (`toile-text-editing`). The inline editor commits its whole buffer as
    /// `text`, and `base` is the body it opened on. The handler merges the
    /// change from `base` to `text` into the stored body, so a peer's edit
    /// that arrived meanwhile survives (`TOILE-UI-381`). The handler rejects a
    /// non-text element id.
    SetText {
        id: Id<Element>,
        base: String,
        text: String,
    },
    LockElements {
        ids: Vec<Id<Element>>,
        lock: LockState,
    },

    // -- Z-order (over the unified NodeId layer list,
    // `toile-elements`) --
    Reorder {
        node: NodeId,
        to: ZTarget,
    },

    // -- Groups (`toile-groups`) --
    /// Bind a set of elements into a fresh group element (selects/transforms as
    /// one). One undo item.
    GroupElements {
        ids: Vec<Id<Element>>,
    },
    /// Drop each group's binding, re-parenting its children to the group's own
    /// parent. The children survive, and only the group nodes go away. A
    /// multi-selection is one gesture, so the whole list is one undo item
    /// (`TOILE-094`).
    UngroupElements {
        groups: Vec<Id<Element>>,
    },

    // -- Frames (`toile-frames`) --
    /// Set a frame's membership: add / remove children on the frame's `children`
    /// list. The frame itself is an ordinary element (created via
    /// `CreateElement { kind: Frame, .. }`); only its content is frame-specific.
    /// The engine keeps the `toile-frames` invariant (a child paints
    /// after its frame). One undo item.
    SetFrameChildren {
        frame: Id<Element>,
        add: Vec<Id<Element>>,
        remove: Vec<Id<Element>>,
    },
    /// Take each listed element out of the frame that holds it, whichever that
    /// is. The engine resolves the membership, because the display carries no
    /// parent-frame edge and the frame that contains a box need not be its
    /// parent (`toile-menus`). An id no frame holds is skipped. One undo item.
    RemoveFromFrames {
        ids: Vec<Id<Element>>,
    },
    /// Name a frame (`toile-frames`). The title is one LWW register, not a
    /// shared text. A title is short and written by one person at a time. Two
    /// writers converge on one whole name, never interleaving into a third
    /// (`TOILE-252`). Trimmed and cut to
    /// [`FRAME_NAME_MAX`](crate::message::FRAME_NAME_MAX) by the engine.
    /// One undo item.
    RenameFrame {
        frame: Id<Element>,
        name: String,
    },

    /// Set the board's locale, one register of `meta` (`LOCALE-001`). No
    /// element moves with it (`LOCALE-009`). An edit, so a viewer's is
    /// dropped (`LOCALE-007`).
    SetDocumentLocale {
        locale: ambre_client_core::DocumentLocale,
    },

    // -- Presentation (`toile-presentation`) --
    /// Set the shared slide sequence: the frames, in playback order
    /// (`toile-presentation`). CRDT (a `LoroMovableList<Id<Frame>>`),
    /// convergent and undoable (`toile-undo`). Repeated ids are
    /// de-duplicated; a dangling Id<Frame> is skipped at derivation like a dangling
    /// connector (`toile-z-order`), so no frame-existence check happens
    /// here.
    SetSlideOrder {
        order: Vec<Id<Frame>>,
    },
    /// Step the local playback cursor to the next / previous slide, saturating at
    /// the deck bounds (`toile-presentation`). Local only -- no CRDT
    /// delta, not an undo item.
    NextSlide,
    PrevSlide,
    /// Jump the local playback cursor to `0`-based slide index, clamped to the
    /// deck length (`toile-presentation`). Local only.
    GotoSlide(Pos<Slide>),
    /// Enter presentation (focus) mode, resetting the cursor to the first slide;
    /// `focus` drives whether the UI hides its chrome
    /// (`toile-presentation`). Local only.
    EnterPresent {
        focus: PresentFocus,
    },
    /// Leave presentation mode (`toile-presentation`). Local only.
    ExitPresent,
    /// Toggle whether this viewer broadcasts its active slide index to peers
    /// (`toile-presentation`, "Follow presenter"). Local role only -- no
    /// CRDT delta. When on, the playback commands additionally emit a presence
    /// sample carrying `present_slide`.
    SetPresentBroadcast(Broadcast),
    /// Follow the given presenter's broadcast playback position, or stop following
    /// with `None` (`toile-presentation`, "Follow presenter"). Local role
    /// only -- no CRDT delta. While following, an inbound presence sample from that
    /// `Id<User>` moves this viewer's active slide; a manual navigation clears the
    /// follow ("take control").
    FollowPresenter(Option<Id<User>>),
    /// Apply a char-CRDT edit to a frame's collaborative speaker notes
    /// (`toile-presentation`). Notes live in the frame element's payload as
    /// a LoroText, seeded on first edit. CRDT, convergent, one commit = one undo
    /// item. A missing / non-frame id is a rejected no-op. `delta` is measured
    /// against `base`, the note the panel held. When a peer moved the note
    /// since, the handler merges the change into the stored note instead
    /// (`TOILE-FEAT-088`).
    EditNotes {
        frame: Id<Frame>,
        base: String,
        delta: TextDelta,
    },
    /// Set (or clear, with `None`) a frame's auto-layout spec
    /// (`toile-auto-layout`). The spec is a single LWW CRDT register on the
    /// frame element, and the renderer derives the child positions afresh and
    /// never persists them (`toile-auto-layout`). Convergent and undoable; a
    /// missing / non-frame id is a rejected no-op.
    SetAutoLayout {
        frame: Id<Frame>,
        layout: Option<AutoLayout>,
    },

    // -- Reactions / votes (`toile-reactions`) --
    /// Place, change or remove the local user's reaction on a target (element or
    /// frame). Shared, persisted CRDT: a sub-map keyed by target + canonical
    /// `Id<User>`, so one user has exactly one entry per target (a re-vote is LWW on
    /// their own key). `reaction: None` removes the user's own entry. Counts are
    /// derived on read, never stored. One commit = one undo item.
    React {
        target: Id<Element>,
        reaction: Option<Reaction>,
    },

    // -- Comment threads (`toile-comments`) --
    /// Open a thread at `anchor`, seeded with its first message
    /// (`TOILE-024`). The thread id is minted by the engine, so two peers
    /// commenting on one spot mint distinct threads (`TOILE-FEAT-011`). `ts` is
    /// the author's wall clock, shown to the reader and used for nothing else:
    /// the Logic thread has no clock, and a clock orders nothing
    /// (`TOILE-FEAT-012`). Outside the undo stack (`TOILE-FEAT-017`); a
    /// no-op before the session carries a `Id<User>`.
    CreateComment {
        anchor: CommentAnchor,
        body: String,
        ts: Instant<Authored>,
    },
    /// Append a message to `thread`. Same clock rule and same history exemption
    /// as [`CreateComment`](Self::CreateComment). A missing thread is a rejected
    /// no-op.
    ReplyComment {
        thread: Id<Thread>,
        body: String,
        ts: Instant<Authored>,
    },
    /// Splice the body of the author's own message `message` (its index in the
    /// thread's message list). The body is a `LoroText`, so a self-edit
    /// converges character by character (`TOILE-FEAT-013`); the delta is the
    /// `EditNotes` one. Another author's message, or a missing index, is a
    /// rejected no-op.
    EditComment {
        thread: Id<Thread>,
        message: Pos<ThreadMessage>,
        delta: TextDelta,
    },
    /// Resolve `thread`, or reopen it. Idempotent and convergent, and it
    /// destroys neither the thread nor its messages (`TOILE-FEAT-015`).
    ResolveThread {
        thread: Id<Thread>,
        state: ambre_core::ThreadState,
    },
    /// Read comments back for the panel, which holds no state of its own
    /// (`TOILE-UI-023`). `Some(thread)` reads that one thread whole and answers
    /// [`CommentThreadData`](ToileEvent::CommentThreadData); `None` reads the
    /// board-wide list and answers [`CommentList`](ToileEvent::CommentList).
    /// A pure read: no commit, no delta, no undo item.
    ReadComments {
        thread: Option<Id<Thread>>,
    },
    /// Read the board's frames back for the deck panel, which holds no state of
    /// its own (`toile-presentation`). Answers
    /// [`DeckData`](ToileEvent::DeckData). A pure read: no commit, no delta, no
    /// undo item.
    ReadDeck,

    // -- Vector boolean ops (`toile-vector-paths`) --
    /// Combine the selected shape / path elements with a boolean rule
    /// (`toile-vector-paths`), producing one new `shape:path` element whose
    /// geometry is the result. The inputs are flattened to polygon contours and
    /// combined client-side (kurbo has no path booleans). Needs at least two
    /// eligible (shape or path) elements; other kinds are ignored. If
    /// `sources` is `Consume` the inputs are deleted in the same commit, so the
    /// whole operation is one undo item. An empty result (fully cancelled
    /// geometry) is a no-op.
    BooleanOp {
        op: BoolOp,
        ids: Vec<Id<Element>>,
        sources: Sources,
    },

    // -- Undo / redo (local-only, `toile-undo`) --
    Undo,
    Redo,

    // -- Connectors --
    CreateConnector {
        from: Id<Element>,
        from_anch: Anchor,
        to: Id<Element>,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    },
    RetargetConnector {
        id: Id<Connector>,
        endpoint: End,
        to: Id<Element>,
        anch: Anchor,
    },
    DeleteConnector {
        id: Id<Connector>,
    },
    /// Set or clear the link register of one element (`toile-links`). `None`
    /// clears it. One register write and one undo item; nothing else on the
    /// element moves (`TOILE-066`, `TOILE-111`).
    SetLink {
        id: Id<Element>,
        link: Option<LinkTarget>,
    },
    /// Overwrite an arrow's style register, whole, like `RestyleElements` does
    /// for an element (`toile-connectors`).
    RestyleConnector {
        id: Id<Connector>,
        style: ElementStyle,
    },
    /// Write an arrow's caption. The whole caption travels as `text`, with
    /// `base`, the caption the editor opened on. The handler merges the change
    /// into the `label` char-CRDT, so two peers captioning one arrow converge
    /// character by character, instead of one body winning
    /// (`toile-connectors`).
    SetConnectorLabel {
        id: Id<Connector>,
        base: String,
        text: String,
    },

    // -- Freehand --
    CommitStroke {
        points: Vec<StrokePoint>,
        style: ElementStyle,
    },

    // -- Clipboard (`toile-clipboard`) --
    /// Read the selection into a self-contained, id-free [`ClipboardPayload`]
    /// (does not mutate the doc); answered by `ToileEvent::Clipboard`. Cut is the
    /// UI issuing this then `DeleteElements` on the same selection.
    CopyElements {
        ids: Vec<Id<Element>>,
    },
    /// Re-mint every node in `payload`, offset so `payload.origin` lands at `at`,
    /// and insert above the current top of the z-order. One undo item.
    PasteElements {
        payload: ClipboardPayload,
        at: Point,
    },
    /// Copy + paste fused into a single undo item, offset by `delta` (the
    /// alt-drag / Ctrl+D nudge).
    DuplicateElements {
        ids: Vec<Id<Element>>,
        delta: Vec2,
    },

    // -- Find (search on board, `toile-find`) --
    /// Non-mutating text search over the text-bearing elements (sticky / text
    /// bodies, frame titles); answered by `ToileEvent::FindResults`. `from` is the
    /// current match to resume from; a `Backward` direction picks the previous hit rather than
    /// the next (both wrapping). A `None` `from` starts a fresh search focusing
    /// the first hit. A `None` `query` is the emptied field, answered with no hit
    /// (`FIND-005`). No CRDT effect.
    Find {
        query: Option<ambre_bus::app::find::FindQuery<FindOptions>>,
        from: Option<Id<Element>>,
        direction: ambre_core::StepDirection,
    },

    // -- Selection aggregates for the status bar (`toile-status-bar`) --
    RequestSelectionStats {
        ids: Vec<Id<Element>>,
        kind: StatsRequestKind,
    },
    /// Ask Logic for the full display data of these ids, off-screen included
    /// (`toile-menus`, `toile-styles`, `toile-text-editing`). The UI thread's
    /// culled set omits the selection past the edge. The menu scope, the style
    /// surfaces and the inline editor read this instead of the viewport
    /// (`TOILE-UI-070`, `TOILE-UI-134`).
    RequestSelectionData {
        ids: Vec<Id<Element>>,
    },
    /// Ask Logic for every element id on the board, off-screen included, which
    /// select-all needs because the UI thread holds the culled set alone
    /// (`toile-selection`, `TOILE-UI-033`).
    RequestBoardIds,

    // -- Interop import (`toile-import-export`) --
    /// The shared import and export shape (`INTEROP-001`), answered by one
    /// [`ToileEvent::Interop`]. An import converts the bytes to an
    /// `ImportBatch` and applies it as one undo group; embedded raster media
    /// is encrypted and uploaded through the Sync media plane before the batch
    /// lands (`toile-import-export`). An export only reads: no commit, no
    /// delta, no undo item (`TOILE-IO-024`).
    Interop(ToileInterop),

    // -- Viewport (drives culling; no CRDT effect) --
    SetViewport(Viewport),

    // -- Ephemeral presence (never persisted, never in history;
    // `toile-engine`) --
    /// A local presence sample (cursor move, live-stroke sample, laser point,
    /// selection) to broadcast to peers. Produces a `SendEphemeral` payload and
    /// no CRDT delta: the local cursor is drawn by the UI itself.
    ///
    /// `now` is the UI thread's wall clock, which the Logic thread does not
    /// hold (`features/ambre-spec-toile-comments.md section 5`, the same
    /// argument). It never travels: it ages the peers this client knows about,
    /// so one that stopped sending leaves the live set (`TOILE-185`). The UI
    /// sends a sample on every gesture and a heartbeat while idle, so the clock
    /// always advances.
    Presence {
        payload: PresencePayload,
        now: Instant<PageLoad>,
    },
}

// A text a user typed prints its length alone, and every other field its own
// `Debug`. A log of a command carries none of the board's text.
impl fmt::Debug for ToileCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fields {
            ($name:literal $(, $field:ident)*) => {
                f.debug_struct($name)$(.field(stringify!($field), $field))*.finish()
            };
        }
        macro_rules! tuple {
            ($name:literal, $value:expr) => {
                f.debug_tuple($name).field($value).finish()
            };
        }
        match self {
            Self::CreateElement {
                kind,
                at,
                size,
                content,
                fill,
                correlate,
            } => fields!("CreateElement", kind, at, size, content, fill, correlate),
            Self::MoveElements { ids, delta } => fields!("MoveElements", ids, delta),
            Self::AlignElements { ids, op } => fields!("AlignElements", ids, op),
            Self::ResizeElement { id, new } => fields!("ResizeElement", id, new),
            Self::RotateElement { id, radians } => fields!("RotateElement", id, radians),
            Self::RequestMedia { hashes } => fields!("RequestMedia", hashes),
            Self::RestyleElements { ids, style } => fields!("RestyleElements", ids, style),
            Self::DeleteElements { ids } => fields!("DeleteElements", ids),
            Self::SetText { id, base, text } => f
                .debug_struct("SetText")
                .field("id", id)
                .field("base", &ambre_core::RedactedChars::of(base))
                .field("text", &ambre_core::RedactedChars::of(text))
                .finish(),
            Self::LockElements { ids, lock } => fields!("LockElements", ids, lock),
            Self::Reorder { node, to } => fields!("Reorder", node, to),
            Self::GroupElements { ids } => fields!("GroupElements", ids),
            Self::UngroupElements { groups } => fields!("UngroupElements", groups),
            Self::SetFrameChildren { frame, add, remove } => {
                fields!("SetFrameChildren", frame, add, remove)
            }
            Self::RemoveFromFrames { ids } => fields!("RemoveFromFrames", ids),
            Self::RenameFrame { frame, name } => f
                .debug_struct("RenameFrame")
                .field("frame", frame)
                .field("name", &ambre_core::RedactedChars::of(name))
                .finish(),
            Self::SetSlideOrder { order } => fields!("SetSlideOrder", order),
            Self::SetDocumentLocale { locale } => fields!("SetDocumentLocale", locale),
            Self::NextSlide => f.write_str("NextSlide"),
            Self::PrevSlide => f.write_str("PrevSlide"),
            Self::GotoSlide(slide) => tuple!("GotoSlide", slide),
            Self::EnterPresent { focus } => fields!("EnterPresent", focus),
            Self::ExitPresent => f.write_str("ExitPresent"),
            Self::SetPresentBroadcast(broadcast) => tuple!("SetPresentBroadcast", broadcast),
            Self::FollowPresenter(user) => tuple!("FollowPresenter", user),
            Self::EditNotes { frame, base, delta } => f
                .debug_struct("EditNotes")
                .field("frame", frame)
                .field("base", &ambre_core::RedactedChars::of(base))
                .field("delta", delta)
                .finish(),
            Self::SetAutoLayout { frame, layout } => fields!("SetAutoLayout", frame, layout),
            Self::React { target, reaction } => fields!("React", target, reaction),
            Self::CreateComment { anchor, body, ts } => f
                .debug_struct("CreateComment")
                .field("anchor", anchor)
                .field("body", &ambre_core::RedactedChars::of(body))
                .field("ts", ts)
                .finish(),
            Self::ReplyComment { thread, body, ts } => f
                .debug_struct("ReplyComment")
                .field("thread", thread)
                .field("body", &ambre_core::RedactedChars::of(body))
                .field("ts", ts)
                .finish(),
            Self::EditComment {
                thread,
                message,
                delta,
            } => fields!("EditComment", thread, message, delta),
            Self::ResolveThread { thread, state } => fields!("ResolveThread", thread, state),
            Self::ReadComments { thread } => fields!("ReadComments", thread),
            Self::ReadDeck => f.write_str("ReadDeck"),
            Self::BooleanOp { op, ids, sources } => fields!("BooleanOp", op, ids, sources),
            Self::Undo => f.write_str("Undo"),
            Self::Redo => f.write_str("Redo"),
            Self::CreateConnector {
                from,
                from_anch,
                to,
                to_anch,
                path_type,
                arrows,
            } => fields!(
                "CreateConnector",
                from,
                from_anch,
                to,
                to_anch,
                path_type,
                arrows
            ),
            Self::RetargetConnector {
                id,
                endpoint,
                to,
                anch,
            } => fields!("RetargetConnector", id, endpoint, to, anch),
            Self::DeleteConnector { id } => fields!("DeleteConnector", id),
            Self::SetLink { id, link } => fields!("SetLink", id, link),
            Self::RestyleConnector { id, style } => fields!("RestyleConnector", id, style),
            Self::SetConnectorLabel { id, base, text } => f
                .debug_struct("SetConnectorLabel")
                .field("id", id)
                .field("base", &ambre_core::RedactedChars::of(base))
                .field("text", &ambre_core::RedactedChars::of(text))
                .finish(),
            Self::CommitStroke { points, style } => fields!("CommitStroke", points, style),
            Self::CopyElements { ids } => fields!("CopyElements", ids),
            Self::PasteElements { payload, at } => fields!("PasteElements", payload, at),
            Self::DuplicateElements { ids, delta } => fields!("DuplicateElements", ids, delta),
            Self::Find {
                query,
                from,
                direction,
            } => fields!("Find", query, from, direction),
            Self::RequestSelectionStats { ids, kind } => {
                fields!("RequestSelectionStats", ids, kind)
            }
            Self::RequestSelectionData { ids } => fields!("RequestSelectionData", ids),
            Self::RequestBoardIds => f.write_str("RequestBoardIds"),
            Self::Interop(command) => tuple!("Interop", command),
            Self::SetViewport(viewport) => tuple!("SetViewport", viewport),
            Self::Presence { payload, now } => fields!("Presence", payload, now),
        }
    }
}

impl ambre_bus::Mutation for ToileCommand {
    fn is_mutating(&self) -> bool {
        ToileCommand::is_mutating(self)
    }

    /// The viewport fetch, the media fetch and the presence beat are lossy:
    /// the board offers each again on a later frame (`BUS-024`). Every other
    /// command carries intent. The match is exhaustive, so a new command does
    /// not build until it is classed.
    fn traffic(&self) -> ambre_bus::Traffic {
        match self {
            Self::SetViewport(_) | Self::RequestMedia { .. } | Self::Presence { .. } => {
                ambre_bus::Traffic::Lossy
            }
            Self::CreateElement { .. }
            | Self::MoveElements { .. }
            | Self::AlignElements { .. }
            | Self::ResizeElement { .. }
            | Self::RotateElement { .. }
            | Self::RestyleElements { .. }
            | Self::DeleteElements { .. }
            | Self::SetText { .. }
            | Self::LockElements { .. }
            | Self::Reorder { .. }
            | Self::GroupElements { .. }
            | Self::UngroupElements { .. }
            | Self::SetFrameChildren { .. }
            | Self::RemoveFromFrames { .. }
            | Self::RenameFrame { .. }
            | Self::SetDocumentLocale { .. }
            | Self::SetSlideOrder { .. }
            | Self::NextSlide
            | Self::PrevSlide
            | Self::GotoSlide(..)
            | Self::EnterPresent { .. }
            | Self::ExitPresent
            | Self::SetPresentBroadcast(..)
            | Self::FollowPresenter(..)
            | Self::EditNotes { .. }
            | Self::SetAutoLayout { .. }
            | Self::React { .. }
            | Self::CreateComment { .. }
            | Self::ReplyComment { .. }
            | Self::EditComment { .. }
            | Self::ResolveThread { .. }
            | Self::ReadComments { .. }
            | Self::ReadDeck
            | Self::BooleanOp { .. }
            | Self::Undo
            | Self::Redo
            | Self::CreateConnector { .. }
            | Self::RetargetConnector { .. }
            | Self::DeleteConnector { .. }
            | Self::SetLink { .. }
            | Self::RestyleConnector { .. }
            | Self::SetConnectorLabel { .. }
            | Self::CommitStroke { .. }
            | Self::CopyElements { .. }
            | Self::PasteElements { .. }
            | Self::DuplicateElements { .. }
            | Self::Find { .. }
            | Self::RequestSelectionStats { .. }
            | Self::RequestSelectionData { .. }
            | Self::RequestBoardIds
            | Self::Interop(_) => ambre_bus::Traffic::Lossless,
        }
    }
}

impl ToileCommand {
    /// Bring `data`, read as `format`, onto the board at `at`.
    #[must_use]
    pub const fn import(
        format: ToileImportFormat,
        data: ambre_core::Payload<ambre_client_core::FileBytes>,
        at: Point,
    ) -> Self {
        Self::Interop(ambre_bus::interop::InteropRequest::Import(ToileImport {
            format,
            data,
            at,
        }))
    }

    /// Write the board out as `format`.
    #[must_use]
    pub const fn export(format: ToileExport) -> Self {
        Self::Interop(ambre_bus::interop::InteropRequest::Export(format))
    }

    /// Whether this command can change the board's shared CRDT state
    /// (`architecture`).
    ///
    /// This list holds only the commands documented above as local-only or
    /// read-only. Everything else counts as mutating. That is the safe
    /// direction for a UX gate. An unclassified new variant stays out of a
    /// viewer's reach rather than reaching it wrongly, and the server rejects
    /// the write regardless.
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            // Local playback / presentation state
            // (`toile-presentation`): no CRDT delta.
            Self::NextSlide
                | Self::PrevSlide
                | Self::GotoSlide(_)
                | Self::EnterPresent { .. }
                | Self::ExitPresent
                | Self::SetPresentBroadcast(_)
                | Self::FollowPresenter(_)
                // Local view state and read-only queries.
                | Self::SetViewport(_)
                | Self::Find { .. }
                | Self::RequestSelectionStats { .. }
                | Self::RequestSelectionData { .. }
                | Self::RequestBoardIds
                | Self::CopyElements { .. }
                | Self::ReadComments { .. }
                | Self::ReadDeck
                | Self::RequestMedia { .. }
                // Ephemeral only: a viewer still shows a cursor
                // (`toile-presence`).
                | Self::Presence { .. }
        ) && match self {
            // An import writes and an export reads (`INTEROP-005`,
            // `INTEROP-006`).
            Self::Interop(command) => command.is_mutating(),
            _ => true,
        }
    }
}

// =============================================================================
// Presence (ephemeral, `toile-presence`) -- never persisted, never in Loro
// history.
// =============================================================================

/// A sample's name, `None` when it carried none or one that breaks the
/// display-name rule. A forged name never drops the cursor it labels.
fn lenient_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ambre_core::Validated<ambre_core::DisplayName>>, D::Error> {
    let name = Option::<String>::deserialize(deserializer)?;
    Ok(name.and_then(|text| ambre_core::Validated::<ambre_core::DisplayName>::parse(&text).ok()))
}

/// One ephemeral presence sample the local peer broadcasts
/// (`toile-presence`): pointer position, current selection, an in-flight
/// stroke, and/or a laser point. All fields are optional so a bare cursor move
/// need not resend the selection. Postcard-encoded into the `SendEphemeral`
/// payload. This payload deliberately carries NO identity. The receiver keys
/// presence off the sender's canonical `Id<User>`, which `ambre-sync` resolves
/// from the authenticated `pub_key`, so a peer cannot spoof another user's
/// cursor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresencePayload {
    /// The sender's display name, cosmetic and forgeable
    /// (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-004`). It labels a
    /// cursor and decides neither identity, nor colour, nor permission. Both
    /// come from the `Id<User>` the transport resolved. `None` falls back to a
    /// short handle derived from that id. So does a name that breaks the
    /// display-name rule: the sample stays, the name is dropped.
    #[serde(deserialize_with = "lenient_name")]
    pub name: Option<ambre_core::Validated<ambre_core::DisplayName>>,
    /// World-space pointer; `None` means the pointer left the board.
    pub cursor: Option<Point>,
    /// The peer's current selection, drawn as halos. Empty = nothing selected.
    pub selection: Vec<Id<Element>>,
    /// In-flight freehand stroke samples (world space), streamed before the
    /// stroke commits to a `"drawing"` element (`toile-freehand`). `None` =
    /// not drawing.
    pub live_stroke: Option<Vec<StrokePoint>>,
    /// Laser / highlighter point (a momentary, fading trail on the render side,
    /// `toile-presence`). `None` = laser off.
    pub laser: Option<Point>,
    /// The sender's broadcast active slide index while presenting
    /// (`toile-presentation`, "Follow presenter"). `Some(idx)` means the
    /// sender is broadcasting this position; a viewer following the sender adopts
    /// it. `None` = not broadcasting. Purely ephemeral, like every other field
    /// here (the playback position is never CRDT -- see the rejected-decisions
    /// table).
    pub present_slide: Option<Pos<Slide>>,
}

/// A remote peer's latest presence, keyed by the canonical `Id<User>`
/// (`toile-presence`). The UI derives the peer color / name label by
/// hashing `user_id` with the shared cross-app algorithm
/// (`ambre_draw::peer::peer_color`). A collaborator therefore keeps the same
/// color across Prose, Trame and Toile. Emitted as `ToileEvent::PeerPresence`;
/// the render side is `toile-presence`'s overlay pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub user_id: Id<User>,
    pub presence: PresencePayload,
}

// =============================================================================
// Errors
// =============================================================================

/// Why the board refused a command. The reason of a failed write may quote
/// the board, so `Debug` prints its length alone.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToileError {
    #[error("element {} not found", .0.as_str())]
    NoSuchElement(Id<Element>),
    #[error("connector {} not found", .0.as_str())]
    NoSuchConnector(Id<Connector>),
    #[error("comment thread {} not found", .0.as_str())]
    NoSuchThread(Id<Thread>),
    #[error("{} cannot be created via CreateElement", .kind.type_tag())]
    NotCreatable { kind: ElementKind },
    #[error("InitContent does not match the requested element kind")]
    ContentKindMismatch,
    #[error("an element needs a finite size that is not empty")]
    UnusableSize,
    #[error("a group needs at least two distinct members")]
    GroupNeedsTwo,
    #[error("element {} is not a group", .0.as_str())]
    NotAGroup(Id<Element>),
    #[error("a collaborator's change could not be applied; the board may be behind")]
    RemoteImportFailed,
    #[error("boolean op needs at least two eligible shape / path elements")]
    BooleanNeedsTwo,
    #[error("boolean op produced an empty result")]
    BooleanEmpty,
    #[error("the board engine could not write the change")]
    WriteFailed,
    /// The session has minted every id its counter holds. A fresh id would
    /// repeat one, and overwrite the thing that holds it. A reopened board
    /// mints under a new PeerID.
    #[error("this session has no fresh id left; reopen the board to create more")]
    IdsExhausted,
}

impl fmt::Debug for ToileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchElement(id) => f.debug_tuple("NoSuchElement").field(id).finish(),
            Self::NoSuchConnector(id) => f.debug_tuple("NoSuchConnector").field(id).finish(),
            Self::NoSuchThread(id) => f.debug_tuple("NoSuchThread").field(id).finish(),
            Self::NotCreatable { kind } => {
                f.debug_struct("NotCreatable").field("kind", kind).finish()
            }
            Self::ContentKindMismatch => f.write_str("ContentKindMismatch"),
            Self::UnusableSize => f.write_str("UnusableSize"),
            Self::GroupNeedsTwo => f.write_str("GroupNeedsTwo"),
            Self::NotAGroup(id) => f.debug_tuple("NotAGroup").field(id).finish(),
            Self::RemoteImportFailed => f.write_str("RemoteImportFailed"),
            Self::BooleanNeedsTwo => f.write_str("BooleanNeedsTwo"),
            Self::BooleanEmpty => f.write_str("BooleanEmpty"),
            Self::WriteFailed => f.write_str("WriteFailed"),
            Self::IdsExhausted => f.write_str("IdsExhausted"),
        }
    }
}

// =============================================================================
// Events (ToileState -> UI)
// =============================================================================

/// Which surface asked for a selection aggregate, so the reply can name the
/// request it answers (`TOILE-UI-108`). The Logic thread echoes it back, so the
/// shell and the view each spend only the reply that matches the question they
/// asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatsRequestKind {
    /// The status bar asked for the selection's count and bounds.
    #[default]
    Band,
    /// A fit / zoom-to-selection / frame link asked for bounds to frame.
    Fit,
    /// A find step asked for the focus element's bounds.
    FindFocus,
    /// A comment thread asked for its anchor element's bounds.
    CommentFocus,
}

/// Selection aggregates (`toile-status-bar`): element count + union AABB
/// (world).
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionStats {
    pub count: Count<Elements>,
    pub bounds: Option<Rect>,
    pub kind: StatsRequestKind,
}

/// `Debug` prints the length alone of the speaker notes and of an import
/// failure, which may quote the foreign file. Every other text is the engine's.
#[derive(Clone)]
pub enum ToileEvent {
    ViewportData(DisplayViewport),
    /// Decoded pixels for one media address (`toile-media`), answering a
    /// `RequestMedia`. RGBA8, row-major, `width * height * 4` bytes.
    ///
    /// These are the *thumbnail's* pixels: it is what the document carries, and
    /// the full blob is ciphertext on the relay that nothing here fetches. So a
    /// picture reaches the screen at thumbnail resolution -- readable, not
    /// printable (`TOILE-IO-100`).
    MediaDecoded {
        hash: ambre_core::Validated<ambre_core::ContentHash>,
        size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
        pixels: ambre_core::Payload<ambre_core::RgbaPixels>,
    },
    /// A freshly-created element, with the `correlate` token its `CreateElement`
    /// carried (`TOILE-UI-194`). `None` when the create requested no deferred
    /// open.
    ElementCreated {
        id: Id<Element>,
        correlate: Option<ambre_bus::token::Token<ElementCreation>>,
    },
    /// Result of `CopyElements`: the UI stashes it and may mirror it to the OS
    /// clipboard for cross-tab paste (`toile-clipboard`).
    Clipboard(ClipboardPayload),
    SelectionStats(SelectionStats),
    /// The board's content changed, by a local write or a peer's. It carries
    /// nothing: a surface holding data it fetched by request asks again. A
    /// camera move is no change, so a pan never sends it (`TOILE-UI-384`).
    BoardChanged,
    /// Full display data for the requested ids, off-screen included
    /// (`TOILE-UI-070`, `TOILE-UI-134`).
    SelectionData {
        elements: Vec<DisplayElement>,
        /// The requested ids a frame's children list holds, whatever their
        /// box. The menu offers removal from a frame by this, not by geometry
        /// (`TOILE-UI-384`).
        framed: Vec<Id<Element>>,
        /// The requested ids the board no longer holds, a peer's deletion
        /// included. The view drops them from its selection (`TOILE-UI-385`).
        missing: Vec<Id<Element>>,
    },
    /// Every element id on the board, for select-all (`TOILE-UI-033`).
    BoardIds(Vec<Id<Element>>),
    /// Text-search hits + recenter target (`toile-find`). Local view
    /// state, never synced and never an undo item.
    FindResults(FindResults),
    DanglingConnector(Id<Connector>),
    /// The arrows the board just lost, whoever deleted them. The view drops a
    /// selection or a caption editor on one of them. It never reads an arrow's
    /// absence from the culled frame that way, since panning one off screen
    /// is not deleting it (`TOILE-UI-386`, `TOILE-UI-388`).
    ArrowsGone(Vec<Id<Connector>>),
    /// The whole live peer set to render, each entry keyed by the canonical
    /// `Id<User>` (`toile-presence`, `PRESENCE-009`). Ephemeral, never persisted.
    ///
    /// The set, not one peer: the view replaces its mirror with it. A peer who
    /// stopped sending therefore leaves the set, and the overlay pass stops
    /// painting it. No departure signal travels anywhere in the workspace
    /// (`TOILE-UI-075`, `TOILE-185`).
    PeerPresence(Vec<PresenceUpdate>),
    /// Local undo/redo stack availability, driving the chrome affordances
    /// (`toile-undo`).
    UndoState {
        undo: ambre_core::Availability,
        redo: ambre_core::Availability,
    },
    /// The local playback cursor moved to a new slide
    /// (`toile-presentation`). Carries the enriched [`SlideChange`] the
    /// `toile-presentation` camera pass consumes to glide + Smart Animate. Local view state:
    /// emitted by the playback commands, never crosses the wire.
    SlideChanged(SlideChange),
    /// The viewer entered or left presentation (focus) mode
    /// (`toile-presentation`). Local view state, like
    /// [`SlideChanged`](Self::SlideChanged).
    PresentModeChanged(PresentMode),
    /// This viewer's slide-position broadcast toggle changed
    /// (`toile-presentation`, "Follow presenter"): the presenter panel
    /// reflects it. Local view state.
    PresentBroadcastChanged(Broadcast),
    /// This viewer's follow target changed (`toile-presentation`, "Follow
    /// presenter"). `Some(u)` = now following `u`, `None` = stopped following
    /// (for example after taking control with a manual navigation). Local view
    /// state.
    FollowingChanged(Option<Id<User>>),
    /// A target's reaction set changed (`toile-reactions`): the UI
    /// re-reads the derived count for this target. Emitted on a local `React`; the
    /// derived count is never carried in the CRDT, only recomputed on read.
    ReactionsUpdated(Id<Element>),
    /// A thread changed (`toile-comments`): created, replied to, edited or
    /// resolved. The panel holds no authoritative state, so it re-reads the
    /// thread on this event (`TOILE-UI-023`). The bodies never ride the display
    /// viewport (`TOILE-026`), so the event carries the id alone.
    CommentsUpdated(Id<Thread>),
    /// A thread must take the focus (`toile-comments`): the comment tool just
    /// dropped it, or a click landed on its pin. The panel opens on it.
    CommentThreadOpened(Id<Thread>),
    /// Every thread of the board, in id order (`toile-comments`): the answer to
    /// a `ReadComments { thread: None }`. A thread whose anchor element is gone
    /// is in it, because the discussion outlives the drawing
    /// (`TOILE-FEAT-016`).
    CommentList(Vec<CommentSummary>),
    /// One whole thread, bodies included (`toile-comments`): the answer to a
    /// `ReadComments { thread: Some(..) }`. This is the only path a message body
    /// takes to the UI (`TOILE-205`).
    CommentThreadData(CommentThread),
    /// The board's frames, deck first: the answer to a
    /// [`ReadDeck`](ToileCommand::ReadDeck). Goes straight to the deck panel's
    /// signals, which are the panel's only source (`toile-presentation`).
    DeckData(DeckView),
    /// The shared deck changed under this viewer (`toile-presentation`): a
    /// local write reordered it, or a deleted frame left it. The panel holds no
    /// authoritative state, so it re-reads on this event.
    DeckChanged,
    /// The speaker notes of the slide on screen changed under this viewer: a
    /// peer wrote into them, or a history step did (`TOILE-UI-316`). Carries
    /// the whole note, because the presenter panel's editor holds a body and
    /// not a position. Emitted only when the text actually differs.
    NotesChanged {
        frame: Id<Frame>,
        notes: String,
    },
    /// The shared answer of an import or an export (`INTEROP-001`). A
    /// finished import carries what landed and what the converter left behind,
    /// as one undo item; a running one reports each slice (`TOILE-IO-028`). A
    /// finished export carries the file, which the UI thread turns into a
    /// browser download (`TOILE-IO-085`), and what the format could not express
    /// (`TOILE-IO-025`). A conversion that produced nothing leaves the board as
    /// it was and says why.
    Interop(ambre_bus::interop::InteropAnswer<ToileConstruct>),
    /// A newer client wrote the board, and the gate refused it whole
    /// (`SCHEMA-008`). Nothing reached the state, and the board on screen is the
    /// one the user already had open. Carries the refusal, whose words the
    /// view makes. A refusal nobody explains looks exactly like a board that
    /// simply failed to load.
    DocumentRefused(ambre_core::SchemaRefusal),
    /// The reader could not decode at least one record in the board, and it drew
    /// that record with its default style instead (`SCHEMA-003`). A fact rather
    /// than a count: every later read meets the same unreadable register again.
    /// Any number here would therefore measure how often the board redrew,
    /// nothing more.
    UndecodableRecords,
    Error(ToileError),
}

impl fmt::Debug for ToileEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ambre_core::RedactedChars;
        macro_rules! tuple {
            ($name:literal, $value:expr) => {
                f.debug_tuple($name).field($value).finish()
            };
        }
        match self {
            Self::ViewportData(viewport) => tuple!("ViewportData", viewport),
            Self::MediaDecoded { hash, size, pixels } => f
                .debug_struct("MediaDecoded")
                .field("hash", hash)
                .field("size", size)
                .field("pixels", pixels)
                .finish(),
            Self::ElementCreated { id, correlate } => f
                .debug_struct("ElementCreated")
                .field("id", id)
                .field("correlate", correlate)
                .finish(),
            Self::Clipboard(payload) => tuple!("Clipboard", payload),
            Self::SelectionStats(stats) => tuple!("SelectionStats", stats),
            Self::BoardChanged => f.write_str("BoardChanged"),
            Self::SelectionData {
                elements,
                framed,
                missing,
            } => f
                .debug_struct("SelectionData")
                .field("elements", elements)
                .field("framed", framed)
                .field("missing", missing)
                .finish(),
            Self::BoardIds(ids) => tuple!("BoardIds", ids),
            Self::FindResults(results) => tuple!("FindResults", results),
            Self::DanglingConnector(id) => tuple!("DanglingConnector", id),
            Self::ArrowsGone(ids) => tuple!("ArrowsGone", ids),
            Self::PeerPresence(peers) => tuple!("PeerPresence", peers),
            Self::UndoState { undo, redo } => f
                .debug_struct("UndoState")
                .field("undo", undo)
                .field("redo", redo)
                .finish(),
            Self::SlideChanged(change) => tuple!("SlideChanged", change),
            Self::PresentModeChanged(mode) => tuple!("PresentModeChanged", mode),
            Self::PresentBroadcastChanged(broadcast) => {
                tuple!("PresentBroadcastChanged", broadcast)
            }
            Self::FollowingChanged(user) => tuple!("FollowingChanged", user),
            Self::ReactionsUpdated(target) => tuple!("ReactionsUpdated", target),
            Self::CommentsUpdated(thread) => tuple!("CommentsUpdated", thread),
            Self::CommentThreadOpened(thread) => tuple!("CommentThreadOpened", thread),
            Self::CommentList(threads) => tuple!("CommentList", threads),
            Self::CommentThreadData(thread) => tuple!("CommentThreadData", thread),
            Self::DeckData(deck) => tuple!("DeckData", deck),
            Self::DeckChanged => f.write_str("DeckChanged"),
            Self::NotesChanged { frame, notes } => f
                .debug_struct("NotesChanged")
                .field("frame", frame)
                .field("notes", &RedactedChars::of(notes))
                .finish(),
            Self::Interop(event) => tuple!("Interop", event),
            Self::DocumentRefused(refusal) => tuple!("DocumentRefused", refusal),
            Self::UndecodableRecords => f.write_str("UndecodableRecords"),
            Self::Error(error) => tuple!("Error", error),
        }
    }
}

// =============================================================================
// Data-oriented display structures (`toile-engine`)
// =============================================================================

/// One frame of "what to paint", already culled and z-ordered. The CRDT visible
/// set only -- presence rides its own event path (`toile-engine`).
#[derive(Clone, Debug)]
pub struct DisplayViewport {
    /// How many viewport requests the Logic thread had applied when it built
    /// this frame. The view drops a frame older than the one it last accepted
    /// (`BUS-017`).
    pub version: ambre_core::Seq<ambre_bus::ViewportVersion>,
    pub viewport: Viewport,
    /// The board's grid unit in world units, as its `board` register holds it.
    /// The grid step is a power-of-two multiple of it (`TOILE-UI-279`). The
    /// value rides each frame as written: the view replaces a unit that is
    /// not finite and positive.
    pub grid_unit: Px<Board>,
    /// Back-to-front sequence mirroring the `layers` walk
    /// (`toile-z-order`): elements and connectors interleaved by
    /// z-order, painted in one pass.
    pub nodes: Vec<DisplayNode>,
    /// Derived reaction counts (`toile-reactions`), keyed by target,
    /// for the visible, still-existing targets that carry at least one reaction.
    /// Derived on read
    /// (never stored in the CRDT); the derivation skips a deleted target's
    /// orphan entries.
    /// The reaction badge pass paints one chip row per entry. A `BTreeMap` (not
    /// `HashMap`) so iteration order is deterministic -- the derived viewport
    /// must equal a full rebuild byte-for-byte (convergence tests).
    pub reactions: BTreeMap<Id<Element>, ReactionCounts>,
    /// The comment pins to paint (`toile-comments`, `TOILE-026`): one entry per
    /// thread whose anchor resolves inside this camera. Message bodies never
    /// ride here: the reader fetches a thread when it opens one (`TOILE-205`). A
    /// thread whose anchor element is gone carries no pin (`TOILE-025`).
    /// Ordered by thread id, so the derived viewport equals a full rebuild.
    pub pins: Vec<CommentPin>,
}

#[derive(Clone, Debug)]
pub enum DisplayNode {
    Element(DisplayElement),
    Connector(DisplayConnector),
}

#[derive(Clone, Debug)]
pub struct DisplayElement {
    pub id: Id<Element>,
    pub kind: ElementKind,
    pub transform: Affine, // world transform incl. rotation
    pub bounds: Rect,
    /// Top-level element flag (`toile-elements`), NOT from
    /// `ElementStyle`.
    pub locked: LockState,
    pub style: ElementStyle,
    pub geom: DisplayGeom,
    /// The element's link, when it carries one (`toile-links`). The painted
    /// element carries it so the surface can show the glyph and follow the
    /// link without asking the board.
    pub link: Option<LinkTarget>,
    /// The immediate enclosing group node, if this element is a group member
    /// (`toile-groups`). `None` for a top-level
    /// element. Group-selection resolution is a pure-UI concern: walk this edge
    /// upward through the group chain to the top-most enclosing group. The
    /// display therefore carries the edge, not a command round-trip. A group
    /// node itself carries its own parent group here.
    pub group: Option<Id<Element>>,
}

#[derive(Clone)]
pub enum DisplayGeom {
    Rect,
    Ellipse,
    Triangle,
    Path(BezPath),
    /// Sticky note: a filled, rounded card (background = `style.fill`) with the
    /// body string drawn on top in a fixed readable ink. Distinct from `Text`
    /// (which has no card). Phase 0 carries a plain string; a later slice upgrades
    /// it to `LoroText` + `Vec<TextRun>` for char-wise convergence
    /// (`toile-text-editing`).
    Sticky(String),
    /// Free text body, no card background. Phase 0 carries a plain string, a
    /// single LWW field. A later slice upgrades it to `LoroText` +
    /// `Vec<TextRun>` for char-wise convergence (`toile-text-editing`) and
    /// Parley-ready runs.
    Text(String),
    /// Media blob hash (`toile-media` resolves the pixels). `None` when the
    /// stored text spells no address: a peer may write any text, and the
    /// image then paints its placeholder.
    Image(Option<ambre_core::Validated<ambre_core::ContentHash>>),
    /// Logic-smoothed polyline (`toile-freehand`).
    Stroke(Vec<Point>),
    /// Background + border, and the frame's title (`TOILE-253`). Children are
    /// separate `DisplayElement`s. The title is carried here because the render
    /// thread holds no board data: what it paints, it was handed.
    Frame(String),
    /// Paints nothing; carried for group-selection resolution
    /// (`toile-embeds`).
    Group,
    /// Web url (sandboxed iframe). `None` when the document holds no address
    /// that the rule of a [`WebAddress`] accepts: the placeholder paints, and no
    /// frame loads (`TOILE-267`).
    Embed(Option<Validated<WebAddress>>),
    /// Nested Ambre document (`toile-embeds`).
    DocEmbed {
        document_id: Id<Document>,
        doc_type: DocType,
        focused: InputFocus,
    },
}

// A sticky's body, a text box and a frame's title are the author's, and
// print their length alone.
impl fmt::Debug for DisplayGeom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rect => f.write_str("Rect"),
            Self::Ellipse => f.write_str("Ellipse"),
            Self::Triangle => f.write_str("Triangle"),
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Sticky(text) => f
                .debug_tuple("Sticky")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
            Self::Text(text) => f
                .debug_tuple("Text")
                .field(&ambre_core::RedactedChars::of(text))
                .finish(),
            Self::Image(hash) => f.debug_tuple("Image").field(hash).finish(),
            Self::Stroke(points) => f.debug_tuple("Stroke").field(points).finish(),
            Self::Frame(title) => f
                .debug_tuple("Frame")
                .field(&ambre_core::RedactedChars::of(title))
                .finish(),
            Self::Group => f.write_str("Group"),
            Self::Embed(address) => f.debug_tuple("Embed").field(address).finish(),
            Self::DocEmbed {
                document_id,
                doc_type,
                focused,
            } => f
                .debug_struct("DocEmbed")
                .field("document_id", document_id)
                .field("doc_type", doc_type)
                .field("focused", focused)
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct DisplayConnector {
    pub id: Id<Connector>,
    /// Already routed against the current bounds (`toile-connectors`).
    pub path: BezPath,
    pub style: ElementStyle,
    pub arrows: EndCaps,
    /// The edge caption, empty when the arrow carries none. Read from the
    /// connector's `label` char-CRDT (`toile-connectors`).
    pub label: String,
}

// The caption is the author's: a log carries its length alone.
impl fmt::Debug for DisplayConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisplayConnector")
            .field("id", &self.id)
            .field("path", &self.path)
            .field("style", &self.style)
            .field("arrows", &self.arrows)
            .field("label", &ambre_core::RedactedChars::of(&self.label))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_core::{Authored, Instant, Payload};

    /// A minted id spells its peer and its counter in fixed-width hex, as the
    /// board stores it. An id written before the counter was typed reads the
    /// same.
    #[test]
    fn a_minted_id_spells_its_peer_and_counter_in_hex() {
        let peer = ambre_core::PeerId::new(0x1f);
        let counter = IdCounter::new(0x2a);
        assert_eq!(
            Id::<Element>::mint_from(peer, counter).as_str(),
            "000000000000001f_0000002a"
        );
        assert_eq!(
            Id::<Connector>::mint_from(peer, IdCounter::new(0x2b)).as_str(),
            "000000000000001f_0000002b"
        );
    }

    /// A binary clipboard field encodes as the bare bytes a `Vec<u8>` did.
    /// A payload already on the clipboard reads back, and `Debug` prints the
    /// length alone. The JSON form is the role-blind one
    /// `ambre_core::payload` proves.
    #[test]
    fn a_binary_clipboard_field_keeps_the_bytes_of_a_vec() {
        let bytes = vec![0xde, 0xad, 0x00, 0x01];
        let field = ClipValue::Bin(Payload::<ClipBytes>::new(bytes.clone()));
        let encoded = postcard::to_allocvec(&field).expect("a field encodes");
        let mut expected = vec![3];
        expected.extend(postcard::to_allocvec(&bytes).expect("a vec encodes"));
        assert_eq!(encoded, expected, "variant index 3, then the bare bytes");
        assert_eq!(postcard::from_bytes::<ClipValue>(&encoded), Ok(field));
        let clip = Payload::<ClipBytes>::new(bytes.clone());
        assert_eq!(format!("{clip:?}"), "ClipBytes(4 bytes)");
    }

    /// Every board type and command that holds a text a user typed prints its
    /// length in `Debug`, and none of the text.
    #[test]
    fn a_typed_text_prints_its_length_alone() {
        let id = Id::<Element>::mint_from(ambre_core::PeerId::new(1), IdCounter::new(2));
        let frame_id = Id::<Frame>::mint_from(ambre_core::PeerId::new(1), IdCounter::new(5));
        let thread = Id::<Thread>::mint_from(ambre_core::PeerId::new(1), IdCounter::new(6));
        let secret = "Q3 layoffs".to_owned();
        let delta = TextDelta {
            insert: secret.clone(),
            ..TextDelta::default()
        };
        let address = Validated::<WebAddress>::parse("https://a.example").expect("an address");
        let ts = Instant::<Authored>::default();
        let message = CommentMessage {
            author: Id::<User>::mint(),
            ts,
            body: secret.clone(),
        };
        let summary = CommentSummary {
            id: thread.clone(),
            anchor: CommentAnchor::Element(id.clone()),
            resolved: ambre_core::ThreadState::default(),
            messages: Len::<ThreadMessage>::new(1),
            preview: secret.clone(),
        };
        let frame = DeckFrame {
            id: frame_id.clone(),
            name: secret.clone(),
        };
        // A copied frame carries its notes, and a copied sticky its body.
        let payload = ClipboardPayload {
            elements: vec![ClipElement {
                fields: vec![(ElementKey::Body, ClipValue::Str(secret.clone()))],
                notes: Some(secret.clone()),
            }],
            connectors: Vec::new(),
            membership: Vec::new(),
            origin: Rect::ZERO,
        };
        let commands = [
            ToileCommand::SetText {
                id: id.clone(),
                base: secret.clone(),
                text: secret.clone(),
            },
            ToileCommand::RenameFrame {
                frame: id.clone(),
                name: secret.clone(),
            },
            ToileCommand::EditNotes {
                frame: frame_id.clone(),
                base: secret.clone(),
                delta: delta.clone(),
            },
            ToileCommand::CreateComment {
                anchor: CommentAnchor::Element(id.clone()),
                body: secret.clone(),
                ts,
            },
            ToileCommand::ReplyComment {
                thread: thread.clone(),
                body: secret.clone(),
                ts,
            },
            ToileCommand::SetConnectorLabel {
                id: Id::<Connector>::mint_from(ambre_core::PeerId::new(1), IdCounter::new(3)),
                base: secret.clone(),
                text: secret.clone(),
            },
            ToileCommand::CreateElement {
                kind: ElementKind::Sticky,
                at: Point::ZERO,
                size: Size::ZERO,
                content: InitContent::Text(secret.clone()),
                fill: None,
                correlate: None,
            },
            ToileCommand::PasteElements {
                payload: payload.clone(),
                at: Point::ZERO,
            },
        ];
        let printed = [
            format!("{delta:?}"),
            format!("{message:?}"),
            format!("{summary:?}"),
            format!("{frame:?}"),
            format!("{:?}", InitContent::Text(secret.clone())),
            format!("{:?}", ToileEvent::Clipboard(payload)),
            format!(
                "{:?}",
                ToileEvent::SlideChanged(SlideChange {
                    notes: secret.clone(),
                    ..SlideChange::default()
                })
            ),
            format!(
                "{:?}",
                ToileEvent::NotesChanged {
                    frame: frame_id.clone(),
                    notes: secret.clone(),
                }
            ),
            format!("{:?}", DisplayGeom::Sticky(secret.clone())),
            format!("{:?}", DisplayGeom::Text(secret.clone())),
            format!("{:?}", DisplayGeom::Frame(secret.clone())),
            format!(
                "{:?}",
                DisplayConnector {
                    id: Id::<Connector>::mint_from(ambre_core::PeerId::new(1), IdCounter::new(4)),
                    path: BezPath::new(),
                    style: ElementStyle::default(),
                    arrows: EndCaps {
                        start: Cap::Plain,
                        end: Cap::Arrow,
                    },
                    label: secret.clone(),
                }
            ),
        ]
        .into_iter()
        .chain(commands.iter().map(|command| format!("{command:?}")));
        for line in printed {
            assert!(line.contains("10 chars"), "{line}");
            assert!(!line.contains("Q3 layoffs"), "{line}");
        }
        assert_eq!(format!("{address:?}"), "17 chars");
        // A failure names its kind and quotes no text (`INTEROP-004`).
        let failed = ToileEvent::Interop(ambre_bus::interop::InteropAnswer::failed(
            ambre_bus::interop::Direction::Import,
            ambre_bus::interop::InteropError::Stopped(ambre_core::StopCause::PastBound),
        ));
        assert!(format!("{failed:?}").contains("Stopped(the file passes a safety limit)"));
        assert_eq!(format!("{:?}", ToileError::WriteFailed), "WriteFailed");
        assert_eq!(format!("{:?}", ToileCommand::Undo), "Undo");
    }

    /// A stroke sample keeps the twelve bytes of three bare `f32`s, and a
    /// stored one pressed harder than full does not decode.
    #[test]
    fn a_stroke_sample_keeps_three_floats_and_refuses_a_pressure_past_one() {
        let sample = StrokePoint {
            x: Px::<StoredBoard>::new(1.5),
            y: Px::<StoredBoard>::new(-2.0),
            pressure: Bounded::<UnitFraction>::HALF,
        };
        let encoded = postcard::to_allocvec(&sample).expect("a sample encodes");
        let bare = postcard::to_allocvec(&(1.5_f32, -2.0_f32, 0.5_f32)).expect("floats encode");
        assert_eq!(encoded, bare);
        assert_eq!(postcard::from_bytes::<StrokePoint>(&encoded), Ok(sample));
        for pressure in [2.0_f32, -0.5, f32::NAN] {
            let stored = postcard::to_allocvec(&(1.5_f32, -2.0_f32, pressure)).expect("encodes");
            assert!(
                postcard::from_bytes::<StrokePoint>(&stored).is_err(),
                "pressure {pressure} is refused"
            );
        }
    }

    /// A copied element's field keys encode as the words the board stores,
    /// the bytes a `String` key gave. A key no schema names is skipped as the
    /// bytes are read, and the fields beside it are kept.
    #[test]
    fn a_clipboard_key_travels_as_its_word_and_an_unknown_one_is_skipped() {
        let element = ClipElement {
            fields: vec![(ElementKey::X, ClipValue::F64(1.5))],
            notes: None,
        };
        let as_words = (vec![("x".to_owned(), ClipValue::F64(1.5))], None::<String>);
        let bytes = postcard::to_allocvec(&element).expect("an element encodes");
        assert_eq!(
            bytes,
            postcard::to_allocvec(&as_words).expect("words encode")
        );

        let stray = (
            vec![
                ("stray".to_owned(), ClipValue::Bool(true)),
                ("x".to_owned(), ClipValue::F64(1.5)),
            ],
            None::<String>,
        );
        let bytes = postcard::to_allocvec(&stray).expect("words encode");
        assert_eq!(postcard::from_bytes::<ClipElement>(&bytes), Ok(element));
    }

    /// `TOILE-267`: an address that crosses a channel is parsed again on the
    /// far side. A safe one keeps its bytes, and an unsafe one never decodes.
    #[test]
    fn a_received_address_is_checked_again() {
        let safe = Validated::<WebAddress>::parse("https://example.com/a").expect("a safe address");
        let bytes = postcard::to_allocvec(&safe).expect("encodes");
        assert_eq!(
            postcard::to_allocvec(&"https://example.com/a").expect("encodes"),
            bytes,
            "the wire carries the bare text"
        );
        assert_eq!(
            postcard::from_bytes::<Validated<WebAddress>>(&bytes),
            Ok(safe)
        );
        for hostile in [
            "javascript:alert(1)",
            "data:text/html,x",
            "http://example.com",
        ] {
            let bytes = postcard::to_allocvec(&hostile).expect("encodes");
            assert!(
                postcard::from_bytes::<Validated<WebAddress>>(&bytes).is_err(),
                "{hostile}"
            );
        }
    }

    /// A sample whose name breaks the display-name rule keeps
    /// its cursor, and loses the name alone.
    #[test]
    fn a_blank_sample_name_decodes_as_no_name() {
        let plain = PresencePayload {
            cursor: Some(Point { x: 1.0, y: 2.0 }),
            ..PresencePayload::default()
        };
        let bytes = postcard::to_allocvec(&plain).unwrap();
        // The name leads the sample: swap its `None` for a blank text.
        let mut forged = postcard::to_allocvec(&Some("   ")).unwrap();
        forged.extend_from_slice(&bytes[1..]);
        let read: PresencePayload = postcard::from_bytes(&forged).unwrap();
        assert_eq!(read, plain);
        let named = PresencePayload {
            name: ambre_core::Validated::<ambre_core::DisplayName>::parse("Ada").ok(),
            ..plain
        };
        let bytes = postcard::to_allocvec(&named).unwrap();
        assert_eq!(
            postcard::from_bytes::<PresencePayload>(&bytes).unwrap(),
            named
        );
    }

    /// `TOILE-UI-281`: whatever zoom a producer writes, the culling rectangle
    /// stays finite and no larger than the widest zoom-out shows.
    #[test]
    fn any_zoom_culls_a_finite_rect() {
        let widest = Viewport {
            zoom: MIN_ZOOM,
            ..Viewport::default()
        }
        .to_world_rect();
        for zoom in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            1e-300,
        ] {
            let r = Viewport {
                zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom),
                ..Viewport::default()
            }
            .to_world_rect();
            assert!(
                r.width().is_finite() && r.height().is_finite(),
                "zoom {zoom}"
            );
            assert!(
                r.width() > 0.0 && r.width() <= widest.width(),
                "zoom {zoom}"
            );
        }
    }

    /// `TOILE-UI-071`: the register carries its layout version, and a style
    /// written under the current layout reads back whole.
    #[test]
    fn a_style_round_trips_through_its_version() {
        let style = ElementStyle {
            fill: Some(Rgba::from_bits(0xaabb_ccff)),
            corner: Px::<StoredBoard>::new(8.0),
            shadow: Some(Shadow {
                color: Rgba::from_bits(0x0000_00ff),
                dx: Px::<StoredBoard>::new(1.0),
                dy: Px::<StoredBoard>::new(2.0),
                blur: Px::<StoredBoard>::new(3.0),
            }),
            font: FontSpec {
                family: Some(FontFamily::Serif),
                size: FontSizeChoice::from_whole_points(Some(24)),
                weight: ambre_core::Bounded::<ambre_core::FontWeight>::new(600).unwrap(),
                italic: Slant::Italic,
                underline: Underline::Plain,
                strikethrough: Strikethrough::Struck,
            },
            ..ElementStyle::default()
        };
        let bytes = style.to_bytes();
        assert_eq!(StyleLayout::from_byte(bytes[0]), Some(StyleLayout::CURRENT));
        assert_eq!(ElementStyle::try_from_bytes(&bytes), Some(style));
    }

    /// A size choice is stored as the bare points it names, zero for
    /// automatic, and a named size is never zero.
    #[test]
    fn a_size_choice_is_stored_as_its_points() {
        let twelve = FontSizeChoice::from_whole_points(Some(12));
        let bytes = postcard::to_allocvec(&twelve).expect("encodes");
        assert_eq!(bytes, postcard::to_allocvec(&12.0_f32).expect("encodes"));
        assert_eq!(postcard::from_bytes::<FontSizeChoice>(&bytes), Ok(twelve));
        let auto = postcard::to_allocvec(&FontSizeChoice::AUTO).expect("encodes");
        assert_eq!(auto, postcard::to_allocvec(&0.0_f32).expect("encodes"));
        assert_eq!(
            postcard::from_bytes::<FontSizeChoice>(&auto),
            Ok(FontSizeChoice::AUTO)
        );
        let negative = postcard::to_allocvec(&-1.0_f32).expect("encodes");
        assert!(postcard::from_bytes::<FontSizeChoice>(&negative).is_err());
        assert_eq!(FontSizeChoice::fixed(FontSize::from_whole_points(0)), None);
        assert_eq!(
            FontSizeChoice::from_whole_points(Some(0)),
            FontSizeChoice::AUTO
        );
        assert_eq!(
            FontSizeChoice::AUTO.or_default(),
            FontSize::from_whole_points(12)
        );
        assert_eq!(twelve.whole_points(), Some(12));
        assert_eq!(FontSizeChoice::AUTO.whole_points(), None);
    }

    /// A size a peer stored past the cap reads as the cap, and the style
    /// still decodes (`CANVAS-088`).
    #[test]
    fn a_size_choice_past_the_cap_reads_as_the_cap() {
        let huge = postcard::to_allocvec(&1.0e6_f32).expect("encodes");
        assert_eq!(
            postcard::from_bytes::<FontSizeChoice>(&huge),
            Ok(FontSizeChoice::fixed(TEXT_FONT_SIZE_MAX).expect("a named size"))
        );
        assert_eq!(TEXT_FONT_SIZE_MAX.points(), 4000.0);
        let past = postcard::to_allocvec(&4001.0_f32).expect("encodes");
        let read = postcard::from_bytes::<FontSizeChoice>(&past).expect("decodes");
        assert_eq!(read.whole_points(), Some(4000));
        let in_range = postcard::to_allocvec(&7.5_f32).expect("encodes");
        let read = postcard::from_bytes::<FontSizeChoice>(&in_range).expect("decodes");
        assert_eq!(postcard::to_allocvec(&read).expect("encodes"), in_range);
    }

    /// `TOILE-UI-071`: a register carrying a layout this build does not know is
    /// reported, never silently reset to the default.
    #[test]
    fn a_style_from_an_unknown_layout_is_reported() {
        let mut bytes = vec![StyleLayout::CURRENT.byte() + 1];
        bytes.extend_from_slice(&postcard::to_allocvec(&ElementStyle::default()).unwrap());
        assert_eq!(ElementStyle::try_from_bytes(&bytes), None);
    }

    /// An empty register is a style never written, and it reads as the default.
    #[test]
    fn an_empty_register_reads_as_the_default() {
        assert_eq!(
            ElementStyle::try_from_bytes(&[]),
            Some(ElementStyle::default())
        );
    }
}
