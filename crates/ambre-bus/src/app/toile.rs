//! Toile (whiteboard) IPC contract (`ambre-spec-toile-schema.md`).
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
// Identity -- minted, move-invariant ids (`ambre-spec-toile-schema.md`)
// =============================================================================

/// Stable identity of an element / connector / frame / group, minted once as
/// `"{peer:016x}_{counter:08x}"` and never reused. Owned as a value (never a
/// Loro op id), so `LoroMovableList::mov` reorders leave every reference valid.
///
/// Two peers that concurrently create a node mint distinct ids (distinct
/// `PeerID`): both survive, neither overwrites the other
/// (`ambre-spec-toile-schema.md`).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ElementId(pub String);

/// Stable identity of a connector. Same minting scheme as [`ElementId`]; a
/// distinct type so the compiler rejects passing one where the other is meant.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ConnectorId(pub String);

/// A frame's identity in a presentation sequence (`toile-presentation`). A
/// frame is an ordinary element (`type = "frame"`), so this is a plain
/// [`ElementId`] alias -- a naming aid at call sites, not a distinct type.
pub type FrameId = ElementId;

/// The element an auto-layout / boolean / reaction command targets
/// (`toile-presentation`). Aliased to [`ElementId`] for the same reason as
/// [`FrameId`].
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

/// Z-order node identity (`toile-z-order`): `layers` interleaves
/// elements and connectors in a single back-to-front paint order, so a connector
/// can sit behind an element.
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
// Element kinds (`toile-elements`)
// =============================================================================

/// Discriminant of a board node. Also tags a [`DisplayElement`].
///
/// `Drawing` and `Group` are valid *types* but are **not** creatable via
/// `CreateElement` (`toile-elements`): a stroke is born only through
/// `CommitStroke`, a group only through `GroupElements`. `handle_command` rejects
/// such a `kind`.
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
/// the text decorations, matching Trame's cell font attributes so the shared
/// format controls (`ambre_chrome`) drive the same field set in both apps.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
    pub size: f32,
    pub weight: u16, // CSS weight scale, 400 = regular, 700 = bold
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: "sans-serif".to_owned(),
            size: 16.0,
            weight: 400,
            italic: false,
            underline: false,
            strikethrough: false,
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
/// register (`toile-styles`) so a fill update racing a stroke update
/// cannot leave a half-applied tear. Schema note: postcard decodes the blob whole
/// and does NOT tolerate a missing or extra field, so ANY layout change (a new
/// field here or in `FontSpec`) makes a blob written by an older binary fall back
/// to `default()` (`from_bytes` -> `unwrap_or_default`). Acceptable pre-1.0 (no
/// persisted-format guarantee yet); a versioned register is owed before GA.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ElementStyle {
    pub fill: Option<u32>, // ARGB, None = no fill
    pub stroke: Option<u32>,
    pub stroke_w: f32,
    pub opacity: f32, // 0.0 ..= 1.0
    pub corner: f32,  // corner radius for rect / sticky
    pub font: FontSpec,
    pub shadow: Option<Shadow>,
    /// Text (foreground) colour, ARGB. `None` = the kind's default ink (a
    /// sticky's fixed readable ink, or the theme text colour for a free text
    /// element). Distinct from `fill`, which is the card / shape background.
    pub text_color: Option<u32>,
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
            text_color: None,
        }
    }
}

/// The default sticky-note card colour (ARGB): a warm pastel yellow, the Miro /
/// Figma-whiteboard default. Used as the seeded fill for a `sticky` element and
/// as the Sticky tool's default colour (`toile-tools`).
pub const STICKY_DEFAULT_FILL: u32 = 0xFFFE_F08A;

/// Corner radius (world units) of a sticky-note card. A sticky is a plain
/// rectangle (Miro-style), so this is zero; kept as a named constant so the
/// card's shape is spec-anchored and easy to revisit.
pub const STICKY_CORNER: f32 = 0.0;

/// Sentinel `FontSpec.size` meaning "auto-size to the note" for a sticky: the
/// renderer computes the body size from the card bounds instead of a fixed value
/// (`toile-styles`). A positive size set from the style inspector overrides
/// it.
pub const STICKY_AUTO_FONT_SIZE: f32 = 0.0;

impl ElementStyle {
    /// The style seeded at element birth (`toile-styles`, "Per-kind
    /// defaults"). Every kind uses [`ElementStyle::default`] EXCEPT `sticky`,
    /// which gets the sticky yellow, no border, `corner = 0` (a plain card) and
    /// the auto-size font sentinel so a fresh, still-empty note is immediately
    /// visible and its text tracks the card size. The Sticky tool overrides the
    /// fill per placement via `CreateElement.fill`.
    pub fn for_kind(kind: ElementKind) -> Self {
        match kind {
            ElementKind::Sticky => Self {
                fill: Some(STICKY_DEFAULT_FILL),
                stroke: None,
                stroke_w: 0.0,
                corner: STICKY_CORNER,
                font: FontSpec {
                    size: STICKY_AUTO_FONT_SIZE,
                    ..FontSpec::default()
                },
                ..Self::default()
            },
            _ => Self::default(),
        }
    }

    /// Serialize for the element's `"style"` binary register. Infallible in
    /// practice (a fixed, all-owned struct); a failure is logged and yields an
    /// empty blob, which decodes back to `default()`.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    /// Decode a `"style"` register, saying so when it cannot be read.
    ///
    /// `None` is a register this build does not understand -- a layout written
    /// by a newer client -- and the caller owes the user a word about it
    /// (`SCHEMA-003`). An **empty** register is not that case: an element with
    /// no style carries no bytes and reads as the default, so it comes back as
    /// `Some(default)`.
    pub fn try_from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return Some(Self::default());
        }
        postcard::from_bytes(bytes).ok()
    }
}

// =============================================================================
// Links on an element (`toile-links`)
// =============================================================================

/// Where a link on an element goes. Three navigations and no more: an address,
/// a place on this board, and another document; anything else is a command, and
/// a command belongs to a menu (`toile-links` section 5).
///
/// The document arm carries its type beside its id, like an embed's does: a
/// route cannot be built without it, and asking the server for it turns a
/// follow into a round trip that can fail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkTarget {
    /// A web address, opened by the shell in a sandboxed tab. The board never
    /// fetches it (`TOILE-110`).
    Url(String),
    /// A frame on this board: the camera fits to it (`TOILE-113`).
    Frame(ElementId),
    /// Another Ambre document, opened by the shell (`TOILE-114`).
    Document {
        document_id: DocumentId,
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

// =============================================================================
// Freehand stroke sample (`toile-freehand`)
// =============================================================================

/// One sampled point of a freehand stroke. 12 bytes, fixed layout.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32, // 0.0 ..= 1.0, 1.0 when the input device reports none
}

// =============================================================================
// Connector anchoring (`toile-connectors`)
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
// Viewport / camera (`toile-viewport`)
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
// Clipboard payload -- id-free, slot-indexed, portable across boards
// (`toile-elements`)
// =============================================================================

/// One captured CRDT scalar field value. Covers the four `LoroValue` leaf kinds
/// an element / connector map stores (`ambre-spec-toile-schema.md`); container
/// fields (a group's `children` list) are never captured here -- membership is
/// remapped to slots.
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
/// (`toile-clipboard`). Portable across boards and sessions (postcard
/// round-trips for the OS clipboard). Media are referenced by content hash only
/// (blobs stay on MinIO, `toile-media`); a `doc_embed` copies the
/// same `document_id` (a second reference to the one nested document, not a
/// clone).
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
    }, // nested Ambre document (`toile-embeds`)
    Text(String),  // initial plain body (sticky / text)
}

/// Z-order target for `Reorder` (`toile-z-order`).
#[derive(Clone, Copy, Debug)]
pub enum ZTarget {
    Front,
    Back,
    Forward,
    Backward,
    Index(usize),
}

/// Which endpoint of a connector to retarget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    From,
    To,
}

/// Options for a board `Find` (`toile-find`). Search runs against
/// each element's plaintext body (sticky / text) and frame title; `match_case`
/// toggles case folding, `whole_word` requires a word-boundary match. Replace is
/// out of MVP scope (`toile-find`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_word: bool,
}

/// Result of a board `Find` (`toile-find`). `hits` are the matching
/// elements in stable z-order; `focus` is the element to recenter the camera on
/// (the next hit after `from`, wrapping around). Purely local view state -- never
/// synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindResults {
    pub hits: Vec<ElementId>,
    pub focus: Option<ElementId>,
}

/// A single char-CRDT splice on a `LoroText` (`toile-text-editing`,
/// speaker notes). Encodes any editor edit as "at codepoint `at`, remove `delete`
/// codepoints, then insert `insert`". Indices are loro unicode-codepoint
/// positions; the engine clamps them to the current length. This is the
/// `TextDelta` the specs defer for element text -- introduced here for notes
/// only, where char-CRDT convergence is required (`toile-text-editing`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextDelta {
    pub at: usize,
    pub delete: usize,
    pub insert: String,
}

/// One user's persisted reaction on a target (`toile-reactions`): an
/// emoji OR a signed vote weight, plus a wall-clock `ts` kept only as a display
/// tiebreak. The LWW on a user's own key is handled by Loro (the map key is the
/// canonical `UserId`), not by this `ts`. Postcard-encoded into the reactions
/// sub-map value (`toile-reactions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub kind: ReactionKind,
    pub ts: i64,
}

/// The two reaction flavours (`toile-reactions`): a free-form emoji or
/// a signed vote weight (e.g. `+1` / `-1` / a weighted ballot).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReactionKind {
    Emoji(String),
    Vote(i32),
}

/// Read-time tally derived from a target's reaction sub-map
/// (`toile-reactions`). Never stored
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
// Comment threads (`toile-comments`) -- the asynchronous half of collaborating
// on a board. A thread is its own entry under a minted id, so two peers
// commenting on one spot mint distinct threads (`TOILE-FEAT-011`).
// -----------------------------------------------------------------------------

/// A comment thread's identity (`toile-comments`). Minted like an
/// [`ElementId`], and aliased to it for the same reason as [`FrameId`]: one
/// identity type serves elements, connectors, frames and threads
/// (`ambre-spec-toile-schema.md section 2`).
pub type ThreadId = ElementId;

/// What a comment thread hangs on (`toile-comments`, `TOILE-024`). An element
/// anchor tracks the element through every move; a point anchor carries a remark
/// on empty board. A viewport coordinate is never an anchor: it means nothing on
/// the second screen.
///
/// Postcard-encoded into the thread map's `anchor` key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommentAnchor {
    Element(ElementId),
    Point { x: f64, y: f64 },
}

/// One message of a thread, read from the CRDT (`toile-comments`). The
/// `author` is the canonical `UserId`, never a `PeerID` (`TOILE-FEAT-014`). The
/// reader sees the `ts`, and nothing orders by it (`TOILE-FEAT-012`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentMessage {
    pub author: UserId,
    pub ts: i64,
    pub body: String,
}

/// A whole thread, read on demand (`toile-comments`). Derived on read, never
/// stored: the display viewport carries the pins alone, so a body reaches the UI
/// only through this projection (`TOILE-026`). `messages` is in the
/// `LoroMovableList` order.
#[derive(Clone, Debug, PartialEq)]
pub struct CommentThread {
    pub id: ThreadId,
    pub anchor: CommentAnchor,
    pub resolved: bool,
    pub messages: Vec<CommentMessage>,
}

/// One pin of the derived viewport (`toile-comments`, `TOILE-026`). Carries
/// where the pin sits, how many messages the thread holds and whether it is
/// resolved -- and no body (`TOILE-205`).
///
/// `at` is a world point: the anchor point itself, or the top-right corner of
/// the anchored element, which the pin follows through every move. The painter
/// maps it to screen and draws a constant-size marker there (`TOILE-UI-022`).
#[derive(Clone, Debug, PartialEq)]
pub struct CommentPin {
    pub thread: ThreadId,
    pub at: Point,
    pub messages: u32,
    pub resolved: bool,
}

/// One row of the board-wide thread list (`toile-comments`). The panel shows
/// every thread, including one whose anchor element is gone
/// (`TOILE-FEAT-016`), so this list is read from the whole document and not
/// from the culled viewport. `preview` is the opening message, so a reader
/// recognises the thread before opening it; the rest of the conversation
/// arrives only when the thread is opened (`TOILE-205`).
#[derive(Clone, Debug, PartialEq)]
pub struct CommentSummary {
    pub id: ThreadId,
    pub anchor: CommentAnchor,
    pub resolved: bool,
    pub messages: u32,
    pub preview: String,
}

// -----------------------------------------------------------------------------
// Deck (`toile-presentation`) -- the projection the ordering panel reads.
// -----------------------------------------------------------------------------

/// One frame the deck panel lists (`toile-presentation`). `name` is the frame's
/// title, empty until the user names one: the panel labels an unnamed frame
/// from its rank, which is a presentation concern and stays out of the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct DeckFrame {
    pub id: FrameId,
    pub name: String,
}

/// The board's frames, split by whether the deck names them
/// (`toile-presentation`): the answer to a [`ReadDeck`](ToileCommand::ReadDeck).
/// `slides` is the deck in playback order, and a slide whose frame is gone is
/// not in it -- the deck holds only live frames (`TOILE-FEAT-027`). `off_deck`
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
/// selection's union box; the two distributions pin the extreme boxes and
/// equalize the gaps between the rest. Distinct from [`Align`], which is a
/// persisted auto-layout rule on a frame: this one is a one-shot batch of
/// coordinates and the document keeps no trace of it.
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

/// How a child is sized on the main axis (`toile-auto-layout`). `Hug` and
/// `Fill` are the Figma resize modes; from the pure solver's view `Hug` ==
/// `Fixed` (the frame rect is an input, so "hug content" changes nothing about
/// the children).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sizing {
    Hug,
    Fill,
    Fixed,
}

/// Auto-layout spec carried by a frame (`toile-auto-layout`). Serialized
/// with `postcard` into the frame element's `autolayout` CRDT register (absent =
/// free positioning). The spec is shared design intent; the solved positions are
/// derived, never persisted (`toile-auto-layout`).
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

/// Per-viewer presentation playback (`toile-presentation`). Purely local,
/// momentary-gesture state: which slide this viewer is on and whether focus
/// (present) mode hides the chrome. Never a CRDT value and never synced -- only
/// the slide *sequence* (which frames, in what order) is shared
/// (`toile-presentation`). Mirrors the durable-intent vs
/// momentary-gesture split already used for presence (`toile-presence`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackState {
    /// Index into the shared slide `order`; clamped to the deck length.
    pub active: usize,
    /// Focus (present) mode: the UI hides its chrome and fits the active frame.
    pub focus: bool,
}

// =============================================================================
// Smart Animate snapshots (`toile-presentation`)
// =============================================================================

/// Animatable properties of one element captured for a slide
/// (`toile-presentation`). A frame's snapshot holds one of these per
/// declared child, in `children` order. Pure data: it lives here so the enriched
/// [`SlideChange`] event can carry both frame snapshots across `AppEvent::Toile`
/// to the render thread, which owns no board state. The `smart_animate` solver
/// that consumes these stays in the engine (`ambre_toile::logic::animate`).
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
    /// the test-facing accessor mirroring the `toile-presentation`
    /// pseudo-code.
    pub fn by_id(&self, id: &ElementId) -> Option<&SnapElement> {
        self.elements.iter().find(|e| &e.id == id)
    }
}

/// Payload of a [`ToileEvent::SlideChanged`]: everything the `toile-presentation` camera
/// pass needs to glide to the new slide and Smart Animate its content
/// (`toile-presentation`). The Logic thread owns the
/// board, so it captures the two frame snapshots and the target frame rect here
/// in one shot; the UI holds no board state and cannot derive them itself. Local
/// view state, never crosses the wire.
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
    /// counter (`toile-presentation`).
    pub count: usize,
    /// The new active frame's element id, or `None` when the slide has no
    /// resolvable frame (dangling id / empty deck). The presenter panel uses it
    /// as the target of `EditNotes`.
    pub frame: Option<ElementId>,
    /// The new active frame's speaker notes as plain text, seeding the presenter
    /// panel's editor (`toile-presentation`). Empty when the frame has no
    /// notes yet or is absent.
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
        /// Optional creation-time override of the seeded `style.fill` (ARGB). The
        /// engine seeds a per-kind default style at birth (sticky = yellow,
        /// others = white); `Some(argb)` overwrites just that fill, the rest of
        /// the default is kept. The Sticky tool uses it to place a note in the
        /// picked palette colour atomically (one element, one undo item). `None`
        /// keeps the per-kind default. See `toile-styles`.
        fill: Option<u32>,
    },
    MoveElements {
        ids: Vec<ElementId>,
        delta: Vec2,
    },
    /// Align or distribute a selection (`toile-elements section 1.7`). One
    /// batch, one undo item: the handler computes every translation from the
    /// boxes of the named nodes and writes them under a single commit, so no
    /// peer ever sees half an alignment.
    ///
    /// `ids` are the selected nodes, not their leaves: a group aligns by its own
    /// box and carries its subtree along. An id nobody minted is skipped, and a
    /// locked element takes no part -- neither moved nor counted in the union
    /// (`TOILE-234`).
    AlignElements {
        ids: Vec<ElementId>,
        op: AlignOp,
    },
    ResizeElement {
        id: ElementId,
        new: Rect,
    },
    RotateElement {
        id: ElementId,
        radians: f64,
    },
    /// Ask for the pixels behind one or more media addresses
    /// (`toile-media`). Sent by the UI thread when the walk meets a picture
    /// whose pixels it does not hold, so only what is on screen crosses the
    /// bus: a board of two hundred photographs pays for the ones in view.
    ///
    /// Read-only, and answered per address with `MediaDecoded`. An address the
    /// document does not carry is answered with nothing -- the element keeps its
    /// placeholder, which is what it was already painting.
    RequestMedia {
        hashes: Vec<String>,
    },
    RestyleElements {
        ids: Vec<ElementId>,
        style: ElementStyle,
    },
    DeleteElements {
        ids: Vec<ElementId>,
    },
    /// Replace the plaintext body of a sticky / text element
    /// (`toile-text-editing`). Phase 0 stores the body as a single LWW string
    /// field, so a text edit is a full-body overwrite -- the inline Parley editor
    /// holds the authoritative buffer and commits the final string here. The
    /// char-wise `LoroText` + `TextDelta` upgrade (a `EditText { id, delta }`
    /// variant) is a later slice (see the `DisplayGeom::Text` note). A non-text
    /// element id is rejected.
    SetText {
        id: ElementId,
        text: String,
    },
    LockElements {
        ids: Vec<ElementId>,
        locked: bool,
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
        ids: Vec<ElementId>,
    },
    /// Drop a group's binding, re-parenting its children to the group's own
    /// parent. The children survive; only the group node is removed. One undo
    /// item.
    UngroupElements {
        group: ElementId,
    },

    // -- Frames (`toile-frames`) --
    /// Set a frame's membership: add / remove children on the frame's `children`
    /// list. The frame itself is an ordinary element (created via
    /// `CreateElement { kind: Frame, .. }`); only its content is frame-specific.
    /// The engine keeps the `toile-frames` invariant (a child paints
    /// after its frame). One undo item.
    SetFrameChildren {
        frame: ElementId,
        add: Vec<ElementId>,
        remove: Vec<ElementId>,
    },

    // -- Presentation (`toile-presentation`) --
    /// Set the shared slide sequence: the frames, in playback order
    /// (`toile-presentation`). CRDT (a `LoroMovableList<FrameId>`),
    /// convergent and undoable (`toile-undo`). Repeated ids are
    /// de-duplicated; a dangling FrameId is skipped at derivation like a dangling
    /// connector (`toile-z-order`), so no frame-existence check happens
    /// here.
    SetSlideOrder {
        order: Vec<FrameId>,
    },
    /// Step the local playback cursor to the next / previous slide, saturating at
    /// the deck bounds (`toile-presentation`). Local only -- no CRDT
    /// delta, not an undo item.
    NextSlide,
    PrevSlide,
    /// Jump the local playback cursor to `0`-based slide index, clamped to the
    /// deck length (`toile-presentation`). Local only.
    GotoSlide(usize),
    /// Enter presentation (focus) mode, resetting the cursor to the first slide;
    /// `focus` drives whether the UI hides its chrome
    /// (`toile-presentation`). Local only.
    EnterPresent {
        focus: bool,
    },
    /// Leave presentation mode (`toile-presentation`). Local only.
    ExitPresent,
    /// Toggle whether this viewer broadcasts its active slide index to peers
    /// (`toile-presentation`, "Follow presenter"). Local role only -- no
    /// CRDT delta. When on, the playback commands additionally emit an ephemeral
    /// presence frame carrying `present_slide`.
    SetPresentBroadcast(bool),
    /// Follow the given presenter's broadcast playback position, or stop following
    /// with `None` (`toile-presentation`, "Follow presenter"). Local role
    /// only -- no CRDT delta. While following, an inbound presence frame from that
    /// `UserId` moves this viewer's active slide; a manual navigation clears the
    /// follow ("take control").
    FollowPresenter(Option<UserId>),
    /// Apply a char-CRDT edit to a frame's collaborative speaker notes
    /// (`toile-presentation`). Notes live in the frame element's payload as
    /// a LoroText, seeded on first edit. CRDT, convergent, one commit = one undo
    /// item. A missing / non-frame id is a rejected no-op.
    EditNotes {
        frame: FrameId,
        delta: TextDelta,
    },
    /// Set (or clear, with `None`) a frame's auto-layout spec
    /// (`toile-auto-layout`). The spec is a single LWW CRDT register on the
    /// frame element; the solved child positions are derived on render, never
    /// persisted (`toile-auto-layout`). Convergent and undoable; a missing /
    /// non-frame id is a rejected no-op.
    SetAutoLayout {
        frame: FrameId,
        layout: Option<AutoLayout>,
    },

    // -- Reactions / votes (`toile-reactions`) --
    /// Place, change or remove the local user's reaction on a target (element or
    /// frame). Shared, persisted CRDT: a sub-map keyed by target + canonical
    /// `UserId`, so one user has exactly one entry per target (a re-vote is LWW on
    /// their own key). `reaction: None` removes the user's own entry. Tallies are
    /// derived on read, never stored. One commit = one undo item.
    React {
        target: TargetId,
        reaction: Option<Reaction>,
    },

    // -- Comment threads (`toile-comments`) --
    /// Open a thread at `anchor`, seeded with its first message
    /// (`TOILE-024`). The thread id is minted by the engine, so two peers
    /// commenting on one spot mint distinct threads (`TOILE-FEAT-011`). `ts` is
    /// the author's wall clock, shown to the reader and used for nothing else:
    /// the Logic thread has no clock, and a clock orders nothing
    /// (`TOILE-FEAT-012`). Outside the undo stack (`TOILE-FEAT-017`); a
    /// no-op before the session carries a `UserId`.
    CreateComment {
        anchor: CommentAnchor,
        body: String,
        ts: i64,
    },
    /// Append a message to `thread`. Same clock rule and same history exemption
    /// as [`CreateComment`](Self::CreateComment). A missing thread is a rejected
    /// no-op.
    ReplyComment {
        thread: ThreadId,
        body: String,
        ts: i64,
    },
    /// Splice the body of the author's own message `message` (its index in the
    /// thread's message list). The body is a `LoroText`, so a self-edit
    /// converges character by character (`TOILE-FEAT-013`); the delta is the
    /// `EditNotes` one. Another author's message, or a missing index, is a
    /// rejected no-op.
    EditComment {
        thread: ThreadId,
        message: usize,
        delta: TextDelta,
    },
    /// Resolve `thread`, or reopen it. Idempotent and convergent, and it
    /// destroys neither the thread nor its messages (`TOILE-FEAT-015`).
    ResolveThread {
        thread: ThreadId,
        resolved: bool,
    },
    /// Read comments back for the panel, which holds no state of its own
    /// (`TOILE-UI-023`). `Some(thread)` reads that one thread whole and answers
    /// [`CommentThreadData`](ToileEvent::CommentThreadData); `None` reads the
    /// board-wide list and answers [`CommentList`](ToileEvent::CommentList).
    /// A pure read: no commit, no delta, no undo item.
    ReadComments {
        thread: Option<ThreadId>,
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
    /// `keep_sources` is false the inputs are deleted in the same commit, so the
    /// whole operation is one undo item. An empty result (fully cancelled
    /// geometry) is a no-op.
    BooleanOp {
        op: BoolOp,
        ids: Vec<ElementId>,
        keep_sources: bool,
    },

    // -- Undo / redo (local-only, `toile-undo`) --
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
    /// Set or clear the link register of one element (`toile-links`). `None`
    /// clears it. One register write and one undo item; nothing else on the
    /// element moves (`TOILE-066`, `TOILE-111`).
    SetLink {
        id: ElementId,
        link: Option<LinkTarget>,
    },
    /// Overwrite an arrow's style register, whole, like `RestyleElements` does
    /// for an element (`toile-connectors`).
    RestyleConnector {
        id: ConnectorId,
        style: ElementStyle,
    },
    /// Overwrite an arrow's caption. The whole caption travels, and the handler
    /// diffs it into the `label` char-CRDT, so two peers captioning one arrow
    /// converge character by character instead of one body winning
    /// (`toile-connectors`).
    SetConnectorLabel {
        id: ConnectorId,
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

    // -- Find (search on board, `toile-find`) --
    /// Non-mutating text search over the text-bearing elements (sticky / text
    /// bodies, frame titles); answered by `ToileEvent::FindResults`. `from` is the
    /// current match to resume from; `backward` picks the previous hit rather than
    /// the next (both wrapping). A `None` `from` starts a fresh search focusing
    /// the first hit. No CRDT effect.
    Find {
        query: String,
        options: FindOptions,
        from: Option<ElementId>,
        backward: bool,
    },

    // -- Selection aggregates for the status bar (`toile-status-bar`) --
    RequestSelectionStats {
        ids: Vec<ElementId>,
    },

    // -- Interop import (`toile-import-export`) --
    /// Import an SVG document dropped onto the board. The bytes are converted to
    /// an `ImportBatch` and applied as one undo group; any embedded raster media
    /// is encrypted + uploaded via the Sync media plane before the batch lands
    /// (`toile-import-export`). Answered by [`ToileEvent::ImportReport`] on
    /// success or [`ToileEvent::ImportFailed`] if the file never mapped.
    /// `at` is the world point the file lands on: the drop point, or the centre
    /// of the viewport when the picker started the import. The whole batch is
    /// translated so its top-left corner sits there, because a file must land
    /// where the user put it and never at a fixed point of the world
    /// (`TOILE-IO-023`).
    ImportSvg {
        bytes: Vec<u8>,
        at: Point,
    },
    /// Import a raster image (PNG / JPG) dropped onto the board. `at` as above.
    ImportImage {
        bytes: Vec<u8>,
        mime: String,
        at: Point,
    },
    /// Import a Figma document (a `.fig` JSON export). `at` as above.
    ImportFigma {
        bytes: Vec<u8>,
        at: Point,
    },
    /// Import a Miro board (a JSON export). `at` as above.
    ImportMiro {
        bytes: Vec<u8>,
        at: Point,
    },
    /// Serialize the board to an SVG document and hand the bytes back
    /// (`toile-import-export`). Read-only: no commit, no delta, no undo item
    /// (`TOILE-IO-024`). Answered by [`ExportComplete`](ToileEvent::ExportComplete),
    /// which the UI thread turns into a browser download (`TOILE-IO-085`).
    ExportSvg,
    /// Render the deck to a PDF, one page per slide, and hand the bytes back
    /// (`toile-pdf-export`). Read-only, like [`ExportSvg`](Self::ExportSvg), and
    /// answered the same way -- but the render walk needs the base font faces the
    /// Main thread fetched at boot, so this one is served by the Logic thread
    /// itself rather than by the board alone. Answered by
    /// [`ExportComplete`](ToileEvent::ExportComplete), or by
    /// [`ExportFailed`](ToileEvent::ExportFailed) when there is no page to draw
    /// or no font to draw it with.
    ExportPdf,
    /// Write the whole board to a native `.ambre` file and hand the bytes back
    /// (`ambre-spec-docfile.md`). Read-only, and served by the Logic thread
    /// rather than by the board: the container's header carries the document's
    /// identity, which the board does not know -- only the registry that keyed
    /// it does. Answered by [`ExportComplete`](ToileEvent::ExportComplete).
    ///
    /// Unlike every other export, this one drops nothing: the payload **is** the
    /// document (`DOCFILE-001`). It is also plaintext, which the surface that
    /// offers it must say (`DOCFILE-009`).
    ExportAmbreFile,
    /// Write the whole board to a native `.ambre` file **sealed under
    /// `passphrase`**, and hand the sealed bytes back
    /// (`ambre-spec-docfile.md section 5`). Read-only, and answered by the same
    /// [`ExportComplete`](ToileEvent::ExportComplete) as the plaintext export --
    /// what differs is the bytes, which the surface never has to tell apart.
    ///
    /// Served neither by the board nor by the Logic thread alone. The Logic
    /// thread writes the plaintext container, the Sync worker seals it
    /// (`ARCH-004`), and the answer comes back through the service plane. A
    /// failure answers [`ExportFailed`](ToileEvent::ExportFailed) and leaves no
    /// file. That is the only safe direction: a partial seal is not a file the
    /// user can open.
    ExportAmbreFileSealed {
        /// The passphrase the user typed for this file, and not the account's:
        /// nothing on this path can reach a session (`DOCFILE-014`).
        passphrase: crate::rest::SecretBytes,
    },

    // -- Persistence / compaction (no CRDT mutation) --
    RequestSnapshot,

    // -- Viewport (drives culling; no CRDT effect) --
    SetViewport(Viewport),

    // -- Ephemeral presence (never persisted, never in history;
    // `toile-engine`) --
    /// A local presence sample (cursor move, live-stroke sample, laser point,
    /// selection) to broadcast to peers. Produces a `SendEphemeral` payload and
    /// no CRDT delta: the local cursor is drawn by the UI itself.
    ///
    /// `now_ms` is the UI thread's wall clock, which the Logic thread does not
    /// hold (`features/ambre-spec-toile-comments.md section 5`, the same
    /// argument). It never travels: it ages the peers this client knows about,
    /// so one that stopped sending leaves the live set (`TOILE-185`). The UI
    /// sends a sample on every gesture and a heartbeat while idle, so the clock
    /// always advances.
    Presence {
        payload: PresencePayload,
        now_ms: i64,
    },
}

impl ToileCommand {
    /// Whether this command can change the board's shared CRDT state
    /// (`architecture`).
    ///
    /// Only the commands documented above as local-only or read-only are listed
    /// here; everything else is treated as mutating. That is the safe direction
    /// for a UX gate -- a new variant left unclassified is withheld from a
    /// viewer rather than wrongly offered, and the server rejects the write
    /// regardless.
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
                | Self::RequestSnapshot
                | Self::CopyElements { .. }
                | Self::ExportSvg
                | Self::ExportPdf
                | Self::ExportAmbreFile
                | Self::ExportAmbreFileSealed { .. }
                | Self::ReadComments { .. }
                | Self::ReadDeck
                | Self::RequestMedia { .. }
                // Ephemeral only: a viewer still shows a cursor
                // (`toile-presence`).
                | Self::Presence { .. }
        )
    }
}

// =============================================================================
// Presence (ephemeral, `toile-presence`) -- never persisted, never in Loro
// history.
// =============================================================================

/// One ephemeral presence sample the local peer broadcasts
/// (`toile-presence`): pointer position, current selection, an in-flight
/// stroke, and/or a laser point. All fields are optional so a bare cursor move
/// need not resend the selection. Postcard-encoded into the `SendEphemeral`
/// payload. Identity is deliberately NOT carried here: the receiver keys presence
/// off the sender's canonical `UserId`, resolved from the authenticated `pub_key`
/// in `ambre-sync`, so a peer cannot spoof another user's cursor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresencePayload {
    /// The sender's display name, cosmetic and forgeable
    /// (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-004`): it labels a
    /// cursor and decides neither identity, nor colour, nor permission -- both
    /// come from the `UserId` the transport resolved. `None` falls back to a
    /// short handle derived from that id.
    pub name: Option<String>,
    /// World-space pointer; `None` means the pointer left the board.
    pub cursor: Option<Point>,
    /// The peer's current selection, drawn as halos. Empty = nothing selected.
    pub selection: Vec<ElementId>,
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
    pub present_slide: Option<usize>,
}

/// A remote peer's latest presence, keyed by the canonical `UserId`
/// (`toile-presence`). The UI derives the peer color / name label by
/// hashing `user_id` with the shared cross-app algorithm
/// (`ambre_draw::peer::peer_color`), so a collaborator keeps the same color across
/// Prose, Trame and Toile. Emitted as `ToileEvent::PeerPresence`; the render side
/// is `toile-presence`'s overlay pass.
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

/// Selection aggregates (`toile-status-bar`): element tally + union AABB
/// (world).
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionStats {
    pub count: u32,
    pub bounds: Option<Rect>,
}

#[derive(Clone, Debug)]
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
        hash: String,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    ElementCreated(ElementId),
    /// Result of `CopyElements`: the UI stashes it and may mirror it to the OS
    /// clipboard for cross-tab paste (`toile-clipboard`).
    Clipboard(ClipboardPayload),
    SelectionStats(SelectionStats),
    /// Text-search hits + recenter target (`toile-find`). Local view
    /// state, never synced and never an undo item.
    FindResults(FindResults),
    DanglingConnector(ConnectorId),
    /// The whole live peer set to render, each entry keyed by the canonical
    /// `UserId` (`toile-presence`, `PRESENCE-009`). Ephemeral, never persisted.
    ///
    /// The set, not one peer: the view replaces its mirror with it, so a peer
    /// who stopped sending is an absent entry and stops being painted, with no
    /// departure signal anywhere in the workspace (`TOILE-UI-075`,
    /// `TOILE-185`).
    PeerPresence(Vec<PresenceUpdate>),
    /// Local undo/redo stack availability, driving the chrome affordances
    /// (`toile-undo`).
    UndoState {
        can_undo: bool,
        can_redo: bool,
    },
    /// The local playback cursor moved to a new slide
    /// (`toile-presentation`). Carries the enriched [`SlideChange`] the
    /// `toile-presentation` camera pass consumes to glide + Smart Animate. Local view state:
    /// emitted by the playback commands, never crosses the wire.
    SlideChanged(SlideChange),
    /// Presentation (focus) mode was entered (`true`) or left (`false`)
    /// (`toile-presentation`). Local view state, like
    /// [`SlideChanged`](Self::SlideChanged).
    PresentModeChanged(bool),
    /// This viewer's slide-position broadcast toggle changed
    /// (`toile-presentation`, "Follow presenter"): the presenter panel
    /// reflects it. Local view state.
    PresentBroadcastChanged(bool),
    /// This viewer's follow target changed (`toile-presentation`, "Follow
    /// presenter"): `Some(u)` = now following `u`, `None` = stopped following
    /// (e.g. after taking control with a manual navigation). Local view state.
    FollowingChanged(Option<UserId>),
    /// A target's reaction set changed (`toile-reactions`): the UI
    /// re-reads the derived tally for this target. Emitted on a local `React`; the
    /// derived tally is never carried in the CRDT, only recomputed on read.
    ReactionsUpdated(TargetId),
    /// A thread changed (`toile-comments`): created, replied to, edited or
    /// resolved. The panel holds no authoritative state, so it re-reads the
    /// thread on this event (`TOILE-UI-023`). The bodies never ride the display
    /// viewport (`TOILE-026`), so the event carries the id alone.
    CommentsUpdated(ThreadId),
    /// A thread must take the focus (`toile-comments`): the comment tool just
    /// dropped it, or a click landed on its pin. The panel opens on it.
    CommentThreadOpened(ThreadId),
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
    /// A foreign-file import applied successfully
    /// (`toile-import-export`). Carries the lossy-import summary the UI
    /// surfaces: how many elements landed and which unsupported constructs were
    /// dropped. The whole import is one undo item (`toile-import-export`).
    ImportReport {
        imported: usize,
        skipped: Vec<String>,
    },
    /// A foreign-file import aborted before touching the board (a parse error, a
    /// safety-limit breach, or a media upload failure): the board is unchanged
    /// (`toile-import-export`, atomicity). Carries a human-facing reason.
    ImportFailed(String),
    /// The board was written by a newer client and was refused whole
    /// (`SCHEMA-008`): nothing was applied, and the board on screen is the one
    /// that was already open. Carries the reason in the user's terms -- a
    /// refusal the user is not told about is indistinguishable from a board that
    /// simply failed to load.
    DocumentRefused(String),
    /// At least one record in the board could not be decoded and was drawn with
    /// its default style instead (`SCHEMA-003`). A fact rather than a count: the
    /// same unreadable register is met again on every later read, so any number
    /// reported here would be an artefact of how often the board was redrawn.
    UndecodableRecords,
    /// An export produced nothing (`toile-pdf-export`): a board with no content,
    /// or a session whose fonts have not landed yet. The board is unchanged, so
    /// without a word the user cannot tell the refusal from a lost click.
    ExportFailed(String),
    /// An export produced a file (`toile-import-export`). The Logic thread holds
    /// the board, so it serializes the bytes; the UI thread owns the DOM, so it
    /// triggers the download (`TOILE-IO-085`). `filename` already carries the
    /// right extension.
    ExportComplete {
        filename: String,
        data: Vec<u8>,
    },
    Error(ToileError),
}

// =============================================================================
// Data-oriented display structures (`toile-engine`)
// =============================================================================

/// One frame of "what to paint", already culled and z-ordered. The CRDT visible
/// set only -- presence rides its own event path (`toile-engine`).
#[derive(Clone, Debug)]
pub struct DisplayViewport {
    pub viewport: Viewport,
    /// Back-to-front sequence mirroring the `layers` walk
    /// (`toile-z-order`): elements and connectors interleaved by
    /// z-order, painted in one pass.
    pub nodes: Vec<DisplayNode>,
    /// Derived reaction tallies (`toile-reactions`), keyed by target,
    /// for the visible, still-existing targets that carry at least one reaction.
    /// Derived on read
    /// (never stored in the CRDT); a deleted target's orphan entries are skipped.
    /// The reaction badge pass paints one chip row per entry. A `BTreeMap` (not
    /// `HashMap`) so iteration order is deterministic -- the derived viewport
    /// must equal a full rebuild byte-for-byte (convergence tests).
    pub reactions: BTreeMap<TargetId, ReactionTally>,
    /// The comment pins to paint (`toile-comments`, `TOILE-026`): one entry per
    /// thread whose anchor resolves inside this camera. Message bodies never
    /// ride here -- a thread is read when it is opened (`TOILE-205`) -- and a
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
    pub id: ElementId,
    pub kind: ElementKind,
    pub transform: Affine, // world transform incl. rotation
    pub bounds: Rect,
    /// Top-level element flag (`toile-elements`), NOT from
    /// `ElementStyle`.
    pub locked: bool,
    pub style: ElementStyle,
    pub geom: DisplayGeom,
    /// The element's link, when it carries one (`toile-links`). The painted
    /// element carries it so the surface can show the glyph and follow the
    /// link without asking the board.
    pub link: Option<LinkTarget>,
    /// The immediate enclosing group node, if this element is a group member
    /// (`toile-groups`). `None` for a top-level
    /// element. Group-selection resolution is a pure-UI concern (walk this edge
    /// upward through the group chain to the top-most enclosing group), so the
    /// display carries the edge rather than a command round-trip; a group node
    /// itself carries its own parent group here.
    pub group: Option<ElementId>,
}

#[derive(Clone, Debug)]
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
    /// Free text body, no card background. Phase 0 carries a plain string (a
    /// single LWW field); a later slice upgrades it to `LoroText` + `Vec<TextRun>`
    /// for char-wise convergence (`toile-text-editing`) and Parley-ready
    /// runs.
    Text(String),
    /// Media blob hash (`toile-media` resolves the pixels).
    Image(String),
    /// Logic-smoothed polyline (`toile-freehand`).
    Stroke(Vec<Point>),
    /// Background + border only; children are separate `DisplayElement`s.
    Frame,
    /// Paints nothing; carried for group-selection resolution
    /// (`toile-embeds`).
    Group,
    /// Web url (sandboxed iframe).
    Embed(String),
    /// Nested Ambre document (`toile-embeds`).
    DocEmbed {
        document_id: DocumentId,
        doc_type: DocType,
        focused: bool,
    },
}

#[derive(Clone, Debug)]
pub struct DisplayConnector {
    pub id: ConnectorId,
    /// Already routed against the current bounds (`toile-connectors`).
    pub path: BezPath,
    pub style: ElementStyle,
    pub arrows: EndCaps,
    /// The edge caption, empty when the arrow carries none. Read from the
    /// connector's `label` char-CRDT (`toile-connectors`).
    pub label: String,
}
