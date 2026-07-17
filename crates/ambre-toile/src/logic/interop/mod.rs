//! Toile interop -- import / export converters (`toile-interop`, Phase 0).
//!
//! Pure, host-testable converters between foreign formats and the Toile element
//! model. Phase 0 covers **SVG import/export** (primitives + paths + text),
//! **raster import** (PNG/JPG), the **safety limits** that make a foreign file
//! untrusted-input-safe, and the **media placeholder state machine**. Everything
//! here runs on the Logic thread and never emits plaintext to the server (§1/§6):
//! an import is turned into a batch of [`ToileCommand`]s the dispatcher applies in
//! one undo group (Phase 1 wiring), and media plaintext is handed up for the Sync
//! thread to encrypt (§5) -- interop never hashes or encrypts.
//!
//! **Atomicity (§1).** Each `import_*` entry point is all-or-nothing: it parses
//! and converts fully in memory and only returns `Ok(ImportBatch)` when the whole
//! file mapped within limits. Any parse failure or exceeded bound returns `Err`
//! and yields **no** commands, so a caller applying the batch can never land a
//! partial board.

use kurbo::{BezPath, Rect};

use crate::logic::command::ToileCommand;
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType};

pub mod limits;
pub mod media;
#[cfg(feature = "pdf")]
mod pdf_export;
mod raster;
mod svg_export;
mod svg_import;

#[cfg(test)]
mod tests;

pub use limits::Limits;
pub use media::{ContentHash, MediaCache, MediaState, RequestOutcome};
#[cfg(feature = "pdf")]
pub use pdf_export::{PageSize, PdfExportError, PdfPage, export_pdf};
pub use raster::import_raster;
pub use svg_export::export_svg;
pub use svg_import::import_svg;

/// A reference to something the apply pass mints, used by converters that cannot
/// name `ElementId`s (those are minted only when `CreateElement` is applied,
/// §2.A): either a `CreateElement` slot (an index into
/// [`ImportBatch::commands`]) or a group produced by an earlier
/// [`ImportGroup`] (an index into [`ImportBatch::groups`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRef {
    Slot(usize),
    Group(usize),
}

/// A `<g>` (or a Figma/Miro group) to rebuild via `GroupElements` after the
/// creates land (§2.A). Emitted innermost-first (post-order) so a member
/// `Group(i)` always refers to an already-resolved entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    pub members: Vec<ImportRef>,
}

/// A connector to create after the elements land, endpoints named by slot
/// (`toile-logic §5`). A Miro connector whose endpoint did not map to an imported
/// element is dropped before this is emitted (recorded in the report).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportConnector {
    pub from: ImportRef,
    pub from_anch: Anchor,
    pub to: ImportRef,
    pub to_anch: Anchor,
    pub path_type: PathType,
    pub arrows: EndCaps,
}

/// Everything one import produced: the CRDT commands to apply atomically, the
/// media plaintext blobs the Sync layer will encrypt/upload (§5.A), and the
/// human-facing report of what was and was not mapped (§2.C import report).
#[derive(Debug, Default)]
pub struct ImportBatch {
    /// The `CreateElement` batch, applied in one undo group by the dispatcher
    /// (§2.E). Parent-before-child order so a group/frame is created before its
    /// members (the §2.G z-order / membership invariant).
    pub commands: Vec<ToileCommand>,
    /// Index-aligned with `commands`: the imported style for that slot, or `None`
    /// to keep the default. Style is an independent LWW register, so it cannot
    /// ride `CreateElement`; the apply pass restyles inside the same undo group
    /// (§2.E).
    pub styles: Vec<Option<ElementStyle>>,
    /// Groups to rebuild after the creates land, innermost-first (§2.A).
    pub groups: Vec<ImportGroup>,
    /// Connectors to create after the elements land (`toile-logic §5`).
    pub connectors: Vec<ImportConnector>,
    /// Plaintext media referenced by the commands (an `image` element's hash is a
    /// key here). Handed to Sync for encrypt-then-upload; interop holds no keys.
    pub media: Vec<MediaBlob>,
    /// What was imported and what was dropped (unsupported constructs, §2.C).
    pub report: ImportReport,
}

/// Import outcome summary surfaced to the UI (§2.C). `imported` counts created
/// elements; `skipped` lists dropped/unsupported constructs (a gradient fill, a
/// pattern, an unhandled node) so the user knows the import was lossy.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: Vec<String>,
}

impl ImportReport {
    /// Record a dropped/unsupported construct in the report.
    pub fn skip(&mut self, reason: impl Into<String>) {
        self.skipped.push(reason.into());
    }
}

/// One plaintext media blob produced by an import (§5.A). The `hash` is the
/// content address (blake2b) the Sync thread computes; Phase 0 leaves it empty
/// (`ContentHash::default`-like) unless a caller injects one, since interop never
/// hashes. `width`/`height` are the native resolution (§ media map value), `thumb`
/// is a small PNG-encoded placeholder for the §5.B first paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBlob {
    pub hash: ContentHash,
    pub plaintext: Vec<u8>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub thumb: Vec<u8>,
}

/// A converter / limit failure. Any variant aborts the import atomically (§1):
/// no command is emitted. `Parse`/`Decode` are foreign-format failures; the rest
/// are the §2.D safety bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("failed to parse SVG: {0}")]
    Parse(String),
    #[error("failed to parse {format}: {msg}")]
    Json { format: &'static str, msg: String },
    #[error("failed to decode image: {0}")]
    Decode(String),
    #[error("import exceeds the element limit ({limit})")]
    TooManyElements { limit: usize },
    #[error("import exceeds the nesting-depth limit ({limit})")]
    TooDeep { limit: usize },
    #[error("decoded image {width}x{height} exceeds the dimension limit ({limit})")]
    ImageTooLarge { width: u32, height: u32, limit: u32 },
    #[error("import produced no elements")]
    Empty,
}

/// A single element to serialize on the **export** path (§3.B high-res SVG). A
/// round-trippable view model decoupled from the CRDT `DisplayElement`, so the
/// SVG serializer stays pure and host-testable; a Phase-1 adapter maps a
/// `DisplayViewport` into `[ExportElement]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportElement {
    pub kind: ElementKind,
    /// World-space AABB (unrotated) the element occupies.
    pub rect: Rect,
    /// Rotation in radians about the rect center (0.0 = axis-aligned).
    pub rotation: f64,
    pub style: ElementStyle,
    /// For `Shape(Path)`: the vector geometry (`toile-logic` stores it as SVG,
    /// so this round-trips through `BezPath::to_svg`). `None` for other kinds.
    pub path: Option<BezPath>,
    /// For `Text` / `Sticky`: the plain body.
    pub text: Option<String>,
    /// For `Image`: the media content hash referencing a `MediaBlob`.
    pub media_hash: Option<String>,
}

impl ExportElement {
    /// A minimal element of the given kind at `rect` with the default style.
    pub fn new(kind: ElementKind, rect: Rect) -> Self {
        Self {
            kind,
            rect,
            rotation: 0.0,
            style: ElementStyle::default(),
            path: None,
            text: None,
            media_hash: None,
        }
    }
}
