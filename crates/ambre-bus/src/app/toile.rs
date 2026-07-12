//! Toile (whiteboard) IPC contract (`ambre-spec-toile-logic.md` §2 / §8 / §9).
//!
//! Mirrors the Trame split: the typed command / event / display data-plane types
//! live here in `ambre-bus` so the `ambre-logic` dispatcher can match on
//! `(DocState, AppCommand::Toile)` without a dependency cycle (the engine crate
//! `ambre-toile` depends on `ambre-bus`, never the reverse). The stateful engine
//! (`ToileState`) and its `ToileOutcome` wrapper stay in `ambre-toile`, exactly
//! as `TrameState` / `TrameOutcome` stay in `ambre-trame`.
//!
//! These are pure data: identity, element kinds, the single per-element style
//! register, freehand strokes, connector anchoring, the culled `DisplayViewport`,
//! and the command / event enums. No `LoroDoc` handle, no `unsafe`, US-keyboard
//! characters only.

use ambre_core::DocType;
use ambre_core::uuid::Uuid;
use kurbo::{Affine, BezPath, Point, Rect, Size, Vec2};
use serde::{Deserialize, Serialize};

// =============================================================================
// Identity -- minted, move-invariant ids (§2.A)
// =============================================================================

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
    /// Mint from the document `PeerID` and a per-session monotonic counter.
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

// =============================================================================
// Element kinds (§2.C)
// =============================================================================

/// Discriminant of a board node. Also tags a [`DisplayElement`].
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

// =============================================================================
// Style register -- one LWW binary per element (§2.E)
// =============================================================================

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

// =============================================================================
// Freehand stroke sample (§2.C / §4)
// =============================================================================

/// One sampled point of a freehand stroke. 12 bytes, fixed layout.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32, // 0.0 ..= 1.0, 1.0 when the input device reports none
}

// =============================================================================
// Connector anchoring (§5)
// =============================================================================

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

// =============================================================================
// Viewport / camera (§3)
// =============================================================================

/// The camera: a world-space center, a zoom factor, and the physical screen
/// size. Drives culling via [`to_world_rect`](Viewport::to_world_rect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub center: Point,
    pub zoom: f64,  // 0.1 ..= 10.0
    pub size: Size, // screen size in physical pixels
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            center: Point::ZERO,
            zoom: 1.0,
            size: Size::new(1280.0, 800.0),
        }
    }
}

impl Viewport {
    /// The world-space rectangle currently visible: the screen box mapped back
    /// through the inverse view transform.
    pub fn to_world_rect(&self) -> Rect {
        let half = Vec2::new(self.size.width, self.size.height) / (2.0 * self.zoom);
        Rect::from_center_size(self.center, Size::new(2.0 * half.x, 2.0 * half.y))
    }
}

// =============================================================================
// Clipboard payload -- id-free, slot-indexed, portable across boards (§8 ter)
// =============================================================================

/// One captured CRDT scalar field value. Covers the four `LoroValue` leaf kinds
/// an element / connector map stores (§2.B); container fields (a group's
/// `children` list) are never captured here -- membership is remapped to slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipValue {
    F64(f64),
    Bool(bool),
    Str(String),
    Bin(Vec<u8>),
}

/// An element captured id-free: the full scalar field set of its map (type,
/// geometry, style blob, per-type payload). Referenced elsewhere in the payload
/// by its **slot** (index in [`ClipboardPayload::elements`]), never by id, so an
/// old id can never leak into the paste.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipElement {
    pub fields: Vec<(String, ClipValue)>,
}

/// A connector captured with both endpoints remapped to element slot indices.
/// A connector whose partner was not copied is never captured (it would dangle),
/// so both slots always resolve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipConnector {
    pub from_slot: usize,
    pub to_slot: usize,
    /// Non-endpoint scalar fields (anchors, path type, arrows).
    pub fields: Vec<(String, ClipValue)>,
}

/// Group / frame membership remapped from ids to element slot indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipMembership {
    pub parent_slot: usize,
    pub child_slots: Vec<usize>,
}

/// Self-contained, id-free snapshot of a selection produced by `CopyElements`
/// (§8 ter). Portable across boards and sessions (postcard round-trips for the
/// OS clipboard). Media are referenced by content hash only (blobs stay on
/// MinIO, `toile-interop §5`); a `doc_embed` copies the same `document_id` (a
/// second reference to the one nested document, not a clone).
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
/// (a mismatch is rejected).
#[derive(Clone, Debug)]
pub enum InitContent {
    None,
    Path(BezPath),
    Image(String), // blake2b plaintext hash, key into `media`
    Embed(String), // web url (sandboxed iframe)
    DocEmbed {
        document_id: Uuid,
        doc_type: DocType,
    }, // nested Ambre document (§2.I)
    Text(String),  // initial plain body (sticky / text)
}

/// Z-order target for `Reorder` (§2.G).
#[derive(Clone, Copy, Debug)]
pub enum ZTarget {
    Front,
    Back,
    Forward,
    Backward,
    Index(usize),
}

/// Which endpoint of a connector to retarget.
#[derive(Clone, Copy, Debug)]
pub enum End {
    From,
    To,
}

/// Options for a board `Find` (§8 quater). Search runs against each element's
/// plaintext body (sticky / text) and frame title; `match_case` toggles case
/// folding, `whole_word` requires a word-boundary match. Replace is out of MVP
/// scope (§12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

/// Result of a board `Find` (§8 quater). `hits` are the matching elements in
/// stable z-order; `focus` is the element to recenter the camera on (the next
/// hit after `from`, wrapping around). Purely local view state -- never synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindResults {
    pub hits: Vec<ElementId>,
    pub focus: Option<ElementId>,
}

/// Command from the UI / Logic thread to `ToileState` (in `ambre-toile`).
#[derive(Clone, Debug)]
pub enum ToileCommand {
    // -- Element lifecycle --
    CreateElement {
        kind: ElementKind,
        at: Point,
        size: Size,
        content: InitContent,
    },
    MoveElements {
        ids: Vec<ElementId>,
        delta: Vec2,
    },
    ResizeElement {
        id: ElementId,
        new: Rect,
    },
    RotateElement {
        id: ElementId,
        radians: f64,
    },
    RestyleElements {
        ids: Vec<ElementId>,
        style: ElementStyle,
    },
    DeleteElements {
        ids: Vec<ElementId>,
    },
    LockElements {
        ids: Vec<ElementId>,
        locked: bool,
    },

    // -- Z-order (over the unified NodeId layer list, §2.G) --
    Reorder {
        node: NodeId,
        to: ZTarget,
    },

    // -- Groups (§2.F bis) --
    /// Bind a set of elements into a fresh group element (selects/transforms as
    /// one). One undo item.
    GroupElements {
        ids: Vec<ElementId>,
    },
    /// Drop a group's binding, re-parenting its children to the group's own
    /// parent. The children survive; only the group node is removed. One undo item.
    UngroupElements {
        group: ElementId,
    },

    // -- Frames (§2.F) --
    /// Set a frame's membership: add / remove children on the frame's `children`
    /// list. The frame itself is an ordinary element (created via
    /// `CreateElement { kind: Frame, .. }`); only its content is frame-specific.
    /// The engine keeps the §2.G invariant (a child paints after its frame).
    /// One undo item.
    SetFrameChildren {
        frame: ElementId,
        add: Vec<ElementId>,
        remove: Vec<ElementId>,
    },

    // -- Undo / redo (local-only, §8 bis) --
    Undo,
    Redo,

    // -- Connectors --
    CreateConnector {
        from: ElementId,
        from_anch: Anchor,
        to: ElementId,
        to_anch: Anchor,
        path_type: PathType,
        arrows: EndCaps,
    },
    RetargetConnector {
        id: ConnectorId,
        endpoint: End,
        to: ElementId,
        anch: Anchor,
    },
    DeleteConnector {
        id: ConnectorId,
    },

    // -- Freehand --
    CommitStroke {
        points: Vec<StrokePoint>,
        style: ElementStyle,
    },

    // -- Clipboard (§8 ter) --
    /// Read the selection into a self-contained, id-free [`ClipboardPayload`]
    /// (does not mutate the doc); answered by `ToileEvent::Clipboard`. Cut is the
    /// UI issuing this then `DeleteElements` on the same selection.
    CopyElements {
        ids: Vec<ElementId>,
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
        ids: Vec<ElementId>,
        delta: Vec2,
    },

    // -- Find (search on board, §8 quater) --
    /// Non-mutating text search over the text-bearing elements (sticky / text
    /// bodies, frame titles); answered by `ToileEvent::FindResults`. `from`
    /// resumes after the current match (wrapping). No CRDT effect.
    Find {
        query: String,
        options: FindOptions,
        from: Option<ElementId>,
    },

    // -- Selection aggregates for the status bar (§8 quinquies) --
    RequestSelectionStats {
        ids: Vec<ElementId>,
    },

    // -- Persistence / compaction (no CRDT mutation) --
    RequestSnapshot,

    // -- Viewport (drives culling; no CRDT effect) --
    SetViewport(Viewport),
}

// =============================================================================
// Errors
// =============================================================================

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToileError {
    #[error("element {0} not found")]
    NoSuchElement(String),
    #[error("connector {0} not found")]
    NoSuchConnector(String),
    #[error("{kind} cannot be created via CreateElement")]
    NotCreatable { kind: &'static str },
    #[error("InitContent does not match the requested element kind")]
    ContentKindMismatch,
}

// =============================================================================
// Events (ToileState -> UI)
// =============================================================================

/// Selection aggregates (§8 quinquies): element tally + union AABB (world).
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionStats {
    pub count: u32,
    pub bounds: Option<Rect>,
}

#[derive(Clone, Debug)]
pub enum ToileEvent {
    ViewportData(DisplayViewport),
    ElementCreated(ElementId),
    /// Result of `CopyElements`: the UI stashes it and may mirror it to the OS
    /// clipboard for cross-tab paste (§8 ter).
    Clipboard(ClipboardPayload),
    SelectionStats(SelectionStats),
    /// Text-search hits + recenter target (§8 quater). Local view state, never
    /// synced and never an undo item.
    FindResults(FindResults),
    DanglingConnector(ConnectorId),
    /// Local undo/redo stack availability, driving the chrome affordances (§8 bis).
    UndoState {
        can_undo: bool,
        can_redo: bool,
    },
    Error(ToileError),
}

// =============================================================================
// Data-oriented display structures (§9)
// =============================================================================

/// One frame of "what to paint", already culled and z-ordered. The CRDT visible
/// set only -- presence rides its own event path (§8.B).
#[derive(Clone, Debug)]
pub struct DisplayViewport {
    pub viewport: Viewport,
    /// Back-to-front sequence mirroring the `layers` walk (§2.G): elements and
    /// connectors interleaved by z-order, painted in one pass.
    pub nodes: Vec<DisplayNode>,
}

#[derive(Clone, Debug)]
pub enum DisplayNode {
    Element(DisplayElement),
    Connector(DisplayConnector),
}

#[derive(Clone, Debug)]
pub struct DisplayElement {
    pub id: ElementId,
    pub kind: ElementKind,
    pub transform: Affine, // world transform incl. rotation
    pub bounds: Rect,
    pub locked: bool, // top-level element flag (§2.B), NOT from ElementStyle
    pub style: ElementStyle,
    pub geom: DisplayGeom,
}

#[derive(Clone, Debug)]
pub enum DisplayGeom {
    Rect,
    Ellipse,
    Triangle,
    Path(BezPath),
    /// Sticky / text body. Phase 0 carries a plain string (a single LWW field);
    /// a later slice upgrades it to `LoroText` + `Vec<TextRun>` for char-wise
    /// convergence (§2.D) and Parley-ready runs.
    Text(String),
    Image(String),      // media blob hash (toile-interop resolves the pixels)
    Stroke(Vec<Point>), // Logic-smoothed polyline (§4)
    Frame,              // background + border only; children are separate DisplayElements
    Group,              // paints nothing; carried for group-selection resolution (§2.F bis)
    Embed(String),      // web url (sandboxed iframe)
    DocEmbed {
        document_id: Uuid,
        doc_type: DocType,
        focused: bool,
    }, // nested Ambre doc (§2.I)
}

#[derive(Clone, Debug)]
pub struct DisplayConnector {
    pub id: ConnectorId,
    pub path: BezPath, // already routed against current bounds (§5)
    pub style: ElementStyle,
    pub arrows: EndCaps,
}
