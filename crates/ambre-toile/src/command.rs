//! Toile IPC contract (§8) and the data-oriented display structures (§9).
//!
//! **Scope note (Phase 0).** This is the *core standalone* slice of the
//! contract: the element / connector / stroke lifecycle and the `DisplayViewport`
//! derivation. Text bodies, frames content, groups, clipboard, find, nested
//! `doc_embed` and undo/redo are later Phase-0 / Phase-1 slices and their
//! commands are added as they land, keeping every shipped variant fully handled
//! (no `todo!()` per the project constitution).
//!
//! **Outcome shape.** [`ToileOutcome`] mirrors `TrameOutcome` rather than the
//! spec's literal `sync: Vec<SyncCommand>`: it returns raw CRDT / ephemeral
//! bytes, and the `ambre-logic` dispatcher wraps them into `SyncCommand`s with
//! the owning `document_id` (which the standalone engine does not hold). Same
//! division of labour as Trame.

use ambre_core::DocType;
use ambre_core::uuid::Uuid;
use kurbo::{Affine, BezPath, Point, Rect, Size, Vec2};

use crate::index::Viewport;
use crate::model::{
    Anchor, ConnectorId, ElementId, ElementKind, ElementStyle, EndCaps, NodeId, PathType,
    StrokePoint,
};

// -----------------------------------------------------------------------------
// Commands (UI/Logic -> ToileState)
// -----------------------------------------------------------------------------

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

/// Command from the UI / Logic thread to [`ToileState`](crate::state::ToileState).
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

    // -- Selection aggregates for the status bar (§8 quinquies) --
    RequestSelectionStats {
        ids: Vec<ElementId>,
    },

    // -- Persistence / compaction (no CRDT mutation) --
    RequestSnapshot,

    // -- Viewport (drives culling; no CRDT effect) --
    SetViewport(Viewport),
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Events (ToileState -> UI)
// -----------------------------------------------------------------------------

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
    SelectionStats(SelectionStats),
    DanglingConnector(ConnectorId),
    Error(ToileError),
}

// -----------------------------------------------------------------------------
// Outcome (§8.C)
// -----------------------------------------------------------------------------

/// Result of processing a [`ToileCommand`]: display events for the UI plus
/// optional CRDT / ephemeral byte payloads the dispatcher forwards to Sync.
pub struct ToileOutcome {
    pub display: Vec<ToileEvent>,
    /// Binary CRDT delta (`ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<Vec<u8>>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<Vec<u8>>,
}

impl ToileOutcome {
    pub fn display_only(display: Vec<ToileEvent>) -> Self {
        Self {
            display,
            update: None,
            ephemeral: None,
        }
    }
}

// -----------------------------------------------------------------------------
// Data-oriented display structures (§9)
// -----------------------------------------------------------------------------

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
