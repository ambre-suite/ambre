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

use std::collections::BTreeMap;

use ambre_core::DocType;
use ambre_core::DocumentId;
use ambre_core::UserId;
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

/// A frame's identity in a presentation sequence (§5). A frame is an ordinary
/// element (`type = "frame"`), so this is a plain [`ElementId`] alias -- a
/// naming aid at call sites, not a distinct type.
pub type FrameId = ElementId;

/// The element an auto-layout / boolean / reaction command targets (§5). Aliased
/// to [`ElementId`] for the same reason as [`FrameId`].
pub type TargetId = ElementId;

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

/// Vector boolean rule for [`ToileCommand::BooleanOp`] (toile-features §3), run
/// over the selected shape / path elements. `Subtract` treats the first element
/// as the subject and removes the union of the rest; the others fold left to
/// right over all selected elements.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
    Exclude,
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
        document_id: DocumentId,
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

/// A single char-CRDT splice on a `LoroText` (§1.D speaker notes). Encodes any
/// editor edit as "at codepoint `at`, remove `delete` codepoints, then insert
/// `insert`". Indices are loro unicode-codepoint positions; the engine clamps
/// them to the current length. This is the `TextDelta` the specs defer for
/// element text -- introduced here for notes only, where char-CRDT convergence
/// is required (§6).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextDelta {
    pub at: usize,
    pub delete: usize,
    pub insert: String,
}

/// One user's persisted reaction on a target (`toile-features` §4.A): an emoji
/// OR a signed vote weight, plus a wall-clock `ts` kept only as a display
/// tiebreak. The LWW on a user's own key is handled by Loro (the map key is the
/// canonical `UserId`), not by this `ts`. Postcard-encoded into the reactions
/// sub-map value (§4.A).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub kind: ReactionKind,
    pub ts: i64,
}

/// The two reaction flavours (§4.A): a free-form emoji or a signed vote weight
/// (e.g. `+1` / `-1` / a weighted ballot).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionKind {
    Emoji(String),
    Vote(i32),
}

/// Read-time tally derived from a target's reaction sub-map (§4.A). Never stored
/// -- recomputed on read so it can never drift from the per-user entry set.
/// `emoji` is sorted for a stable render order; `vote_total` / `vote_count`
/// aggregate the `Vote` entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReactionTally {
    pub emoji: Vec<(String, u32)>,
    pub vote_total: i64,
    pub vote_count: u32,
}

// -----------------------------------------------------------------------------
// Auto-layout spec (§2) -- the CRDT design intent carried by a frame. The pure
// `solve_layout` solver that turns this spec + child sizes into positions lives
// in the engine (`ambre_toile::logic::autolayout`); only the shared value types
// live here so `SetAutoLayout` can carry them across `AppCommand::Toile`.
// -----------------------------------------------------------------------------

/// Main-axis flow of an [`AutoLayout`] (§2). `Wrap` is a horizontal flow that
/// breaks into rows when a child would overflow the content box width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Horizontal,
    Vertical,
    Wrap,
}

/// Distribution of children along the main axis within the free space (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    Start,
    Center,
    End,
    SpaceBetween,
}

/// How a child is sized on the main axis (§2). `Hug` and `Fill` are the Figma
/// resize modes; from the pure solver's view `Hug` == `Fixed` (the frame rect is
/// an input, so "hug content" changes nothing about the children).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sizing {
    Hug,
    Fill,
    Fixed,
}

/// Auto-layout spec carried by a frame (§2). Serialized with `postcard` into the
/// frame element's `autolayout` CRDT register (absent = free positioning). The
/// spec is shared design intent; the solved positions are derived, never
/// persisted (§8).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoLayout {
    /// Main-axis flow.
    pub direction: Direction,
    /// Space between consecutive children on the main axis.
    pub gap: f64,
    /// `[top, right, bottom, left]` inset of the content box inside the frame.
    pub padding: [f64; 4],
    /// Main-axis distribution of children in the free space.
    pub align: Align,
    /// Main-axis child sizing.
    pub sizing: Sizing,
}

/// Per-viewer presentation playback (§1.B). Purely local, momentary-gesture
/// state: which slide this viewer is on and whether focus (present) mode hides
/// the chrome. Never a CRDT value and never synced -- only the slide *sequence*
/// (which frames, in what order) is shared (§0/§1). Mirrors the durable-intent
/// vs momentary-gesture split already used for presence (`toile-logic §7`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackState {
    /// Index into the shared slide `order`; clamped to the deck length.
    pub active: usize,
    /// Focus (present) mode: the UI hides its chrome and fits the active frame.
    pub focus: bool,
}

// =============================================================================
// Smart Animate snapshots (toile-features §1.C)
// =============================================================================

/// Animatable properties of one element captured for a slide (§1.C). A frame's
/// snapshot holds one of these per declared child, in `children` order. Pure
/// data: it lives here so the enriched [`SlideChange`] event can carry both
/// frame snapshots across `AppEvent::Toile` to the render thread, which owns no
/// board state. The `smart_animate` solver that consumes these stays in the
/// engine (`ambre_toile::logic::animate`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapElement {
    pub id: ElementId,
    /// Position + size, world space (the element AABB).
    pub bounds: Rect,
    /// Rotation in radians (the element `rot` field).
    pub rotation: f64,
    /// Opacity `0.0 ..= 1.0` (from [`ElementStyle::opacity`]).
    pub opacity: f32,
    /// Fill color as ARGB, `None` = no fill (from [`ElementStyle::fill`]).
    pub fill: Option<u32>,
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
    /// the test-facing accessor mirroring the §1.C pseudo-code.
    pub fn by_id(&self, id: &ElementId) -> Option<&SnapElement> {
        self.elements.iter().find(|e| &e.id == id)
    }
}

/// Payload of a [`ToileEvent::SlideChanged`]: everything the `toile-ui` camera
/// pass needs to glide to the new slide and Smart Animate its content (§1.C /
/// §1.D). The Logic thread owns the board, so it captures the two frame
/// snapshots and the target frame rect here in one shot; the UI holds no board
/// state and cannot derive them itself. Local view state, never crosses the wire.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlideChange {
    /// The new `0`-based active slide index.
    pub index: usize,
    /// The previously active frame's children (empty on `EnterPresent`, or when
    /// the source frame is absent / dangling).
    pub from: FrameSnapshot,
    /// The new active frame's children.
    pub to: FrameSnapshot,
    /// The new active frame element's world rect: the camera fit target. `None`
    /// when the slide has no resolvable frame (dangling id), leaving the camera
    /// where it is.
    pub to_rect: Option<Rect>,
    /// Total number of slides in the deck: drives the presenter panel's `k / n`
    /// counter (§1.D).
    pub count: usize,
    /// The new active frame's element id, or `None` when the slide has no
    /// resolvable frame (dangling id / empty deck). The presenter panel uses it
    /// as the target of `EditNotes`.
    pub frame: Option<ElementId>,
    /// The new active frame's speaker notes as plain text, seeding the presenter
    /// panel's editor (§1.D). Empty when the frame has no notes yet or is absent.
    pub notes: String,
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
    /// Replace the plaintext body of a sticky / text element (§6). Phase 0 stores
    /// the body as a single LWW string field, so a text edit is a full-body
    /// overwrite -- the inline Parley editor holds the authoritative buffer and
    /// commits the final string here. The char-wise `LoroText` + `TextDelta`
    /// upgrade (a `EditText { id, delta }` variant) is a later slice (see the
    /// `DisplayGeom::Text` note). A non-text element id is rejected.
    SetText {
        id: ElementId,
        text: String,
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

    // -- Presentation (§1 / §5) --
    /// Set the shared slide sequence: the frames, in playback order (§1.A). CRDT
    /// (a `LoroMovableList<FrameId>`), convergent and undoable (§8 bis). Repeated
    /// ids are de-duplicated; a dangling FrameId is skipped at derivation like a
    /// dangling connector (§5.A), so no frame-existence check happens here.
    SetSlideOrder {
        order: Vec<FrameId>,
    },
    /// Step the local playback cursor to the next / previous slide, saturating at
    /// the deck bounds (§1.B). Local only -- no CRDT delta, not an undo item.
    NextSlide,
    PrevSlide,
    /// Jump the local playback cursor to `0`-based slide index, clamped to the
    /// deck length (§1.B). Local only.
    GotoSlide(usize),
    /// Enter presentation (focus) mode, resetting the cursor to the first slide;
    /// `focus` drives whether the UI hides its chrome (§1.B). Local only.
    EnterPresent {
        focus: bool,
    },
    /// Leave presentation mode (§1.B). Local only.
    ExitPresent,
    /// Toggle whether this viewer broadcasts its active slide index to peers
    /// (§1.B "Follow presenter"). Local role only -- no CRDT delta. When on, the
    /// playback commands additionally emit an ephemeral presence frame carrying
    /// `present_slide`.
    SetPresentBroadcast(bool),
    /// Follow the given presenter's broadcast playback position, or stop following
    /// with `None` (§1.B "Follow presenter"). Local role only -- no CRDT delta.
    /// While following, an inbound presence frame from that `UserId` moves this
    /// viewer's active slide; a manual navigation clears the follow ("take
    /// control").
    FollowPresenter(Option<UserId>),
    /// Apply a char-CRDT edit to a frame's collaborative speaker notes (§1.D).
    /// Notes live in the frame element's payload as a LoroText, seeded on first
    /// edit. CRDT, convergent, one commit = one undo item. A missing / non-frame
    /// id is a rejected no-op.
    EditNotes {
        frame: FrameId,
        delta: TextDelta,
    },
    /// Set (or clear, with `None`) a frame's auto-layout spec (§2). The spec is a
    /// single LWW CRDT register on the frame element; the solved child positions
    /// are derived on render, never persisted (§8). Convergent and undoable; a
    /// missing / non-frame id is a rejected no-op.
    SetAutoLayout {
        frame: FrameId,
        layout: Option<AutoLayout>,
    },

    // -- Reactions / votes (§4.A) --
    /// Place, change or remove the local user's reaction on a target (element or
    /// frame). Shared, persisted CRDT: a sub-map keyed by target + canonical
    /// `UserId`, so one user has exactly one entry per target (a re-vote is LWW on
    /// their own key). `reaction: None` removes the user's own entry. Tallies are
    /// derived on read, never stored. One commit = one undo item.
    React {
        target: TargetId,
        reaction: Option<Reaction>,
    },

    // -- Vector boolean ops (§3) --
    /// Combine the selected shape / path elements with a boolean rule (§3),
    /// producing one new `shape:path` element whose geometry is the result. The
    /// inputs are flattened to polygon contours and combined client-side (kurbo
    /// has no path booleans; see toile-features §3). Needs at least two eligible
    /// (shape or path) elements; other kinds are ignored. If `keep_sources` is
    /// false the inputs are deleted in the same commit, so the whole operation is
    /// one undo item. An empty result (fully cancelled geometry) is a no-op.
    BooleanOp {
        op: BoolOp,
        ids: Vec<ElementId>,
        keep_sources: bool,
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
    /// bodies, frame titles); answered by `ToileEvent::FindResults`. `from` is the
    /// current match to resume from; `backward` picks the previous hit rather than
    /// the next (both wrapping). A `None` `from` starts a fresh search focusing the
    /// first hit. No CRDT effect.
    Find {
        query: String,
        options: FindOptions,
        from: Option<ElementId>,
        backward: bool,
    },

    // -- Selection aggregates for the status bar (§8 quinquies) --
    RequestSelectionStats {
        ids: Vec<ElementId>,
    },

    // -- Interop import (`toile-interop` §1/§5) --
    /// Import an SVG document dropped onto the board. The bytes are converted to
    /// an `ImportBatch` and applied as one undo group; any embedded raster media
    /// is encrypted + uploaded via the Sync media plane before the batch lands
    /// (§5). Answered by [`ToileEvent::ImportReport`] on success or
    /// [`ToileEvent::ImportFailed`] if the file never mapped.
    ImportSvg {
        bytes: Vec<u8>,
    },
    /// Import a raster image (PNG / JPG) dropped onto the board.
    ImportImage {
        bytes: Vec<u8>,
        mime: String,
    },
    /// Import a Figma document (a `.fig` JSON export).
    ImportFigma {
        bytes: Vec<u8>,
    },
    /// Import a Miro board (a JSON export).
    ImportMiro {
        bytes: Vec<u8>,
    },

    // -- Persistence / compaction (no CRDT mutation) --
    RequestSnapshot,

    // -- Viewport (drives culling; no CRDT effect) --
    SetViewport(Viewport),

    // -- Ephemeral presence (never persisted, never in history; §7) --
    /// A local presence sample (cursor move, live-stroke sample, laser point,
    /// selection) to broadcast to peers. Produces a `SendEphemeral` payload and
    /// no display event / CRDT delta: the local cursor is drawn by the UI itself.
    Presence(PresencePayload),
}

impl ToileCommand {
    /// Whether this command can change the board's shared CRDT state
    /// (architecture §2.D).
    ///
    /// Only the commands documented above as local-only or read-only are listed
    /// here; everything else is treated as mutating. That is the safe direction
    /// for a UX gate -- a new variant left unclassified is withheld from a
    /// viewer rather than wrongly offered, and the server rejects the write
    /// regardless.
    pub fn is_mutating(&self) -> bool {
        !matches!(
            self,
            // Local playback / presentation state (§1.B): no CRDT delta.
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
                | Self::RequestSnapshot
                | Self::CopyElements { .. }
                // Ephemeral only: a viewer still shows a cursor (§7).
                | Self::Presence(_)
        )
    }
}

// =============================================================================
// Presence (ephemeral, §7) -- never persisted, never in Loro history.
// =============================================================================

/// One ephemeral presence sample the local peer broadcasts (§7): pointer
/// position, current selection, an in-flight stroke, and/or a laser point. All
/// fields are optional so a bare cursor move need not resend the selection.
/// Postcard-encoded into the `SendEphemeral` payload. Identity is deliberately
/// NOT carried here: the receiver keys presence off the sender's canonical
/// `UserId`, resolved from the authenticated `pub_key` in `ambre-sync`, so a peer
/// cannot spoof another user's cursor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresencePayload {
    /// World-space pointer; `None` means the pointer left the board.
    pub cursor: Option<Point>,
    /// The peer's current selection, drawn as halos. Empty = nothing selected.
    pub selection: Vec<ElementId>,
    /// In-flight freehand stroke samples (world space), streamed before the
    /// stroke commits to a `"drawing"` element (§4). `None` = not drawing.
    pub live_stroke: Option<Vec<StrokePoint>>,
    /// Laser / highlighter point (a momentary, fading trail on the render side,
    /// `toile-ui`). `None` = laser off.
    pub laser: Option<Point>,
    /// The sender's broadcast active slide index while presenting (§1.B "Follow
    /// presenter"). `Some(idx)` means the sender is broadcasting this position; a
    /// viewer following the sender adopts it. `None` = not broadcasting. Purely
    /// ephemeral, like every other field here (the playback position is never
    /// CRDT -- see the rejected-decisions table).
    pub present_slide: Option<usize>,
}

/// A remote peer's latest presence, keyed by the canonical `UserId` (§7). The UI
/// derives the peer color / name label by hashing `user_id` with the shared
/// cross-app algorithm (`prose-ui::peer_color`), so a collaborator keeps the same
/// color across Prose, Trame and Toile. Emitted as `ToileEvent::PeerPresence`;
/// the render side is `toile-ui`'s overlay pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresenceUpdate {
    pub user_id: UserId,
    pub presence: PresencePayload,
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
    #[error("boolean op needs at least two eligible shape / path elements")]
    BooleanNeedsTwo,
    #[error("boolean op produced an empty result")]
    BooleanEmpty,
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
    /// A remote peer's cursor / live stroke / laser / selection to render,
    /// keyed by the canonical `UserId` (§7). Ephemeral, never persisted.
    PeerPresence(PresenceUpdate),
    /// Local undo/redo stack availability, driving the chrome affordances (§8 bis).
    UndoState {
        can_undo: bool,
        can_redo: bool,
    },
    /// The local playback cursor moved to a new slide (§5). Carries the enriched
    /// [`SlideChange`] the `toile-ui` camera pass consumes to glide + Smart
    /// Animate. Local view state: emitted by the playback commands, never crosses
    /// the wire.
    SlideChanged(SlideChange),
    /// Presentation (focus) mode was entered (`true`) or left (`false`) (§5).
    /// Local view state, like [`SlideChanged`](Self::SlideChanged).
    PresentModeChanged(bool),
    /// This viewer's slide-position broadcast toggle changed (§1.B "Follow
    /// presenter"): the presenter panel reflects it. Local view state.
    PresentBroadcastChanged(bool),
    /// This viewer's follow target changed (§1.B "Follow presenter"): `Some(u)`
    /// = now following `u`, `None` = stopped following (e.g. after taking control
    /// with a manual navigation). Local view state.
    FollowingChanged(Option<UserId>),
    /// A target's reaction set changed (§4.A): the UI re-reads the derived tally
    /// for this target. Emitted on a local `React`; the derived tally is never
    /// carried in the CRDT, only recomputed on read.
    ReactionsUpdated(TargetId),
    /// A foreign-file import applied successfully (`toile-interop` §2.C). Carries
    /// the lossy-import summary the UI surfaces: how many elements landed and
    /// which unsupported constructs were dropped. The whole import is one undo
    /// item (§1).
    ImportReport {
        imported: usize,
        skipped: Vec<String>,
    },
    /// A foreign-file import aborted before touching the board (a parse error, a
    /// safety-limit breach, or a media upload failure): the board is unchanged
    /// (`toile-interop` §1 atomicity). Carries a human-facing reason.
    ImportFailed(String),
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
    /// Derived reaction tallies (§4.A), keyed by target, for the visible,
    /// still-existing targets that carry at least one reaction. Derived on read
    /// (never stored in the CRDT); a deleted target's orphan entries are skipped.
    /// The reaction badge pass paints one chip row per entry. A `BTreeMap` (not
    /// `HashMap`) so iteration order is deterministic -- the derived viewport
    /// must equal a full rebuild byte-for-byte (convergence tests).
    pub reactions: BTreeMap<TargetId, ReactionTally>,
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
    /// The immediate enclosing group node, if this element is a group member
    /// (§2.F bis / §7). `None` for a top-level element. Group-selection resolution
    /// is a pure-UI concern (walk this edge upward through the group chain to the
    /// top-most enclosing group), so the display carries the edge rather than a
    /// command round-trip; a group node itself carries its own parent group here.
    pub group: Option<ElementId>,
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
        document_id: DocumentId,
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
