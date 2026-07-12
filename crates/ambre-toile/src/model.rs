//! Core value types of the Toile CRDT model (`ambre-spec-toile-logic.md` §2).
//!
//! Identity, element kinds, the single per-element style register, freehand
//! stroke samples, and connector anchoring. These types are pure data: they
//! carry no `LoroDoc` handle and are equally usable from the engine
//! ([`crate::state`]), the derived display structures ([`crate::command`]), and
//! the tests. No `unsafe`, US-keyboard characters only.

use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Identity -- minted, move-invariant ids (§2.A)
// -----------------------------------------------------------------------------

/// Stable identity of an element / connector / frame / group, minted once as
/// `"{peer:016x}_{counter:08x}"` and never reused. Owned as a value (never a
/// Loro op id), so `LoroMovableList::mov` reorders leave every reference valid.
///
/// Two peers that concurrently create a node mint distinct ids (distinct
/// `PeerID`): both survive, neither overwrites the other (§2.A / §6).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ElementId(pub String);

/// Stable identity of a connector. Same minting scheme as [`ElementId`]; a
/// distinct type so the compiler rejects passing one where the other is meant.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ConnectorId(pub String);

impl ElementId {
    /// Mint from the document `PeerID` and a per-session monotonic counter.
    pub fn mint(peer: u64, counter: u32) -> Self {
        Self(format!("{peer:016x}_{counter:08x}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ConnectorId {
    pub fn mint(peer: u64, counter: u32) -> Self {
        Self(format!("{peer:016x}_{counter:08x}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Z-order node identity (§2.G): `layers` interleaves elements and connectors in
/// a single back-to-front paint order, so a connector can sit behind an element.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum NodeId {
    Element(ElementId),
    Connector(ConnectorId),
}

impl NodeId {
    /// Encode for storage in the `layers` `LoroMovableList` as a plain string:
    /// `"E:{id}"` for an element, `"C:{id}"` for a connector. Chosen over a Loro
    /// sub-map so the list stays a flat, order-preserving sequence.
    pub fn encode(&self) -> String {
        match self {
            NodeId::Element(id) => format!("E:{}", id.0),
            NodeId::Connector(id) => format!("C:{}", id.0),
        }
    }

    /// Inverse of [`encode`](Self::encode). Returns `None` on a malformed tag
    /// (a defensive guard, never expected on a well-formed document).
    pub fn decode(s: &str) -> Option<Self> {
        let (tag, rest) = s.split_once(':')?;
        match tag {
            "E" => Some(NodeId::Element(ElementId(rest.to_owned()))),
            "C" => Some(NodeId::Connector(ConnectorId(rest.to_owned()))),
            _ => None,
        }
    }
}

// -----------------------------------------------------------------------------
// Element kinds (§2.C)
// -----------------------------------------------------------------------------

/// Discriminant of a board node. Also tags a [`crate::command::DisplayElement`].
///
/// `Drawing` and `Group` are valid *types* but are **not** creatable via
/// `CreateElement` (§8): a stroke is born only through `CommitStroke`, a group
/// only through `GroupElements`. `handle_command` rejects such a `kind`.
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

impl ElementKind {
    /// The persisted `"type"` tag (§2.B). A shape stores its `ShapeKind`
    /// separately in the payload, so every shape maps to `"shape"` here.
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

// -----------------------------------------------------------------------------
// Style register -- one LWW binary per element (§2.E)
// -----------------------------------------------------------------------------

/// Font attributes for sticky / text bodies.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    pub size: f32,
    pub weight: u16, // CSS weight scale, 400 = regular, 700 = bold
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "sans-serif".to_owned(),
            size: 16.0,
            weight: 400,
            italic: false,
        }
    }
}

/// Optional drop shadow.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Shadow {
    pub color: u32, // ARGB
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
}

/// The whole visual style of an element, stored as **one** postcard-encoded LWW
/// register (§2.E) so a fill update racing a stroke update cannot leave a
/// half-applied tear. Extended fields are append-only: postcard tolerates
/// trailing additions on read.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ElementStyle {
    pub fill: Option<u32>, // ARGB, None = no fill
    pub stroke: Option<u32>,
    pub stroke_w: f32,
    pub opacity: f32, // 0.0 ..= 1.0
    pub corner: f32,  // corner radius for rect / sticky
    pub font: FontSpec,
    pub shadow: Option<Shadow>,
}

impl Default for ElementStyle {
    fn default() -> Self {
        Self {
            fill: Some(0xFFFF_FFFF),
            stroke: Some(0xFF33_3333),
            stroke_w: 1.0,
            opacity: 1.0,
            corner: 0.0,
            font: FontSpec::default(),
            shadow: None,
        }
    }
}

impl ElementStyle {
    /// Serialize for the element's `"style"` binary register. Infallible in
    /// practice (a fixed, all-owned struct); a failure is logged and yields an
    /// empty blob, which decodes back to `default()`.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    /// Decode a `"style"` register. An empty / malformed blob falls back to the
    /// default style rather than dropping the element.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        postcard::from_bytes(bytes).unwrap_or_default()
    }
}

// -----------------------------------------------------------------------------
// Freehand stroke sample (§2.C / §4)
// -----------------------------------------------------------------------------

/// One sampled point of a freehand stroke. 12 bytes, fixed layout.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32, // 0.0 ..= 1.0, 1.0 when the input device reports none
}

// -----------------------------------------------------------------------------
// Connector anchoring (§5)
// -----------------------------------------------------------------------------

/// Where a connector attaches on an element's AABB. `Free` is a normalized
/// `(u, v)` in `0.0 ..= 1.0` from the top-left.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Anchor {
    Top,
    Bottom,
    Left,
    Right,
    Center,
    Free { u: f32, v: f32 },
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum PathType {
    Straight,
    Elbow,
    Curved,
}

/// Arrowheads per connector end -- a restyle-class LWW field, persisted in the
/// CRDT and surfaced on the `DisplayConnector`.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct EndCaps {
    pub start: bool,
    pub end: bool,
}
