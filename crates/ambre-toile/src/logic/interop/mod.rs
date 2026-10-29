//! Toile interop -- import / export converters (`toile-import-export`, Phase 0).
//!
//! Pure, host-testable converters between foreign formats and the Toile element
//! model. Phase 0 covers **SVG import/export** (primitives + paths + text) and
//! **raster import** (PNG/JPG). It also covers the **safety limits** that make a
//! foreign file untrusted-input-safe, and the **media placeholder state machine**.
//! Everything here runs on the Logic thread and never emits plaintext to the
//! server. An import is turned into a batch of [`ToileCommand`]s the dispatcher
//! applies in one undo group (Phase 1 wiring). Media plaintext is handed up for
//! the Sync thread to encrypt: interop never hashes or encrypts.
//!
//! **Atomicity.** Each `import_*` entry point is all-or-nothing: it parses and
//! converts fully in memory and only returns `Ok(ImportBatch)` when the whole file
//! mapped within limits. Any parse failure or exceeded bound returns `Err` and
//! yields **no** commands, so a caller applying the batch can never land a partial
//! board.

use kurbo::{BezPath, Point, Rect};

use crate::logic::command::{MediaRef, ToileCommand};
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType};

/// The native `.ambre` container (`ambre-spec-docfile.md`). Lossless, so it sits
/// apart from the converters here: no `ImportReport`, no warning list.
pub mod docfile;
pub mod limits;
pub mod media;
#[cfg(feature = "pdf")]
mod pdf_export;
mod raster;
mod svg_export;
mod svg_import;

#[cfg(test)]
mod tests;

pub use limits::{CoordBound, ImportDepth, Limits};
pub use media::{CachedBlobs, ContentHash, MediaCache, MediaState, RequestOutcome};
#[cfg(all(test, feature = "pdf"))]
pub(crate) use pdf_export::paint_page;
#[cfg(feature = "pdf")]
pub use pdf_export::{PageSize, PdfExportError, PdfPage, export_pdf};
pub use raster::import_raster;
pub use svg_export::export_svg;
pub use svg_import::import_svg;

/// A reference to something the apply pass mints, used by converters that cannot
/// name `ElementId`s, which are minted only when `CreateElement` is applied. It
/// is either a `CreateElement` slot (an index into [`ImportBatch::commands`]) or
/// a group produced by an earlier [`ImportGroup`] (an index into
/// [`ImportBatch::groups`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRef {
    Slot(ImportElementSlot),
    Group(ImportGroupSlot),
}

ambre_core::define_index! {
    /// The rank of a `CreateElement` in [`ImportBatch::commands`]. It means
    /// nothing outside that batch.
    pub struct ImportElementSlot(u32);
    /// A number of element creates in one import batch.
    pub struct ImportElementSlotCount;
    /// The signed gap between two element slots.
    pub struct ImportElementSlotDelta(i64);
    /// A run of element slots.
    pub struct ImportElementSlotSpan;
    /// A list indexed by element slot.
    pub struct ImportElementSlotVec;
}

ambre_core::define_index! {
    /// The rank of a group in [`ImportBatch::groups`]. It means nothing
    /// outside that batch.
    pub struct ImportGroupSlot(u32);
    /// A number of groups in one import batch.
    pub struct ImportGroupSlotCount;
    /// The signed gap between two group slots.
    pub struct ImportGroupSlotDelta(i64);
    /// A run of group slots.
    pub struct ImportGroupSlotSpan;
    /// A list indexed by group slot.
    pub struct ImportGroupSlotVec;
}

/// A `<g>` (or a Figma/Miro group) to rebuild via `GroupElements` after the
/// creates land. Emitted innermost-first (post-order) so a member
/// `Group(i)` always refers to an already-resolved entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    pub members: Vec<ImportRef>,
}

/// A connector to create after the elements land, endpoints named by slot
/// (`toile-connectors`). A Miro connector whose endpoint did not map to an imported
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

/// Everything one import produced. That is the CRDT commands to apply
/// atomically, and the media plaintext blobs the Sync layer will
/// encrypt/upload. Plus the human-facing report of what was and was not
/// mapped (import report).
#[derive(Debug, Default)]
pub struct ImportBatch {
    /// The `CreateElement` batch, applied in one undo group by the
    /// dispatcher. Parent-before-child order, so a group/frame is created
    /// before its members (the z-order / membership invariant).
    pub commands: Vec<ToileCommand>,
    /// Index-aligned with `commands`: the imported style for that slot, or `None`
    /// to keep the default. Style is an independent LWW register, so it cannot
    /// ride `CreateElement`; the apply pass restyles inside the same undo group
    /// (`toile-elements`).
    pub styles: Vec<Option<ElementStyle>>,
    /// Groups to rebuild after the creates land, innermost-first.
    pub groups: Vec<ImportGroup>,
    /// Connectors to create after the elements land (`toile-connectors`).
    pub connectors: Vec<ImportConnector>,
    /// Plaintext media referenced by the commands (an `image` element's hash is a
    /// key here). Handed to Sync for encrypt-then-upload; interop holds no keys.
    pub media: Vec<MediaBlob>,
    /// What was imported and what was dropped (unsupported constructs,).
    pub report: ImportReport,
}

/// Translate every element of `batch` so the batch's top-left corner sits at
/// `at` (`TOILE-IO-023`). A file lands where the user dropped it, whatever
/// coordinates it carried. An SVG drawn around its own origin and a raster that
/// has none both arrive under the pointer.
///
/// Pure and total: a batch that creates nothing is left alone. The offset is the
/// same for every slot, so the drawing keeps its internal geometry. The groups,
/// the connectors and the media all name slots, never coordinates.
pub fn place_batch_at(batch: &mut ImportBatch, at: Point) {
    let mut origin: Option<Point> = None;
    for cmd in &batch.commands {
        if let ToileCommand::CreateElement { at: p, .. } = cmd {
            origin = Some(match origin {
                Some(o) => Point::new(o.x.min(p.x), o.y.min(p.y)),
                None => *p,
            });
        }
    }
    let Some(origin) = origin else {
        return;
    };
    let delta = at - origin;
    for cmd in &mut batch.commands {
        if let ToileCommand::CreateElement { at: p, .. } = cmd {
            *p += delta;
        }
    }
}

/// Import outcome summary surfaced to the UI. `imported` counts created
/// elements; `skipped` lists dropped/unsupported constructs (a gradient fill, a
/// pattern, an unhandled node) so the user knows the import was lossy.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: ambre_bus::app::toile::ElementCount,
    pub skipped: Vec<ambre_bus::app::toile::SkipReason>,
}

impl ImportReport {
    /// Record a dropped/unsupported construct in the report.
    pub fn skip(&mut self, reason: impl Into<String>) {
        self.skipped
            .push(ambre_bus::app::toile::SkipReason::new(reason));
    }
}

/// One plaintext media blob produced by an import. `media` names it by its
/// import slot until Sync returns the content address (blake2b), since interop
/// never hashes. `size` is the native resolution (the media map value), `thumb` is a small PNG-encoded placeholder for the first paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBlob {
    pub media: MediaRef,
    pub plaintext: ambre_core::MediaBytes,
    pub mime: ambre_core::MediaType,
    pub size: ambre_core::ImageSize,
    pub thumb: ambre_core::ThumbnailPng,
}

/// A converter / limit failure. Any variant aborts the import atomically:
/// no command is emitted. `Parse`/`Decode` are foreign-format failures; the rest
/// are the safety bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("failed to parse SVG: {0}")]
    Parse(String),
    #[error("failed to parse {format}: {msg}")]
    Json { format: &'static str, msg: String },
    #[error("failed to decode image: {0}")]
    Decode(String),
    #[error("import exceeds the element limit ({limit})")]
    TooManyElements { limit: ambre_core::ElementCount },
    #[error("import exceeds the nesting-depth limit ({limit})")]
    TooDeep { limit: ImportDepth },
    #[error(
        "decoded image {}x{} exceeds the dimension limit ({}x{})",
        size.width,
        size.height,
        limit.width,
        limit.height
    )]
    ImageTooLarge {
        size: ambre_core::ImageSize,
        limit: ambre_core::ImageSize,
    },
    #[error("import produced no elements")]
    Empty,
}

/// A single element to serialize on the **export** path (high-res SVG). A
/// round-trippable view model decoupled from the CRDT `DisplayElement`, so the
/// SVG serializer stays pure and host-testable; a Phase-1 adapter maps a
/// `DisplayViewport` into `[ExportElement]`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportElement {
    pub kind: ElementKind,
    /// World-space AABB (unrotated) the element occupies.
    pub rect: Rect,
    /// The rotation about the rect center (zero is axis-aligned).
    pub rotation: ambre_core::Radians,
    pub style: ElementStyle,
    /// For `Shape(Path)`: the vector geometry (`toile-vector-paths` stores it as SVG,
    /// so this round-trips through `BezPath::to_svg`). `None` for other kinds.
    pub path: Option<BezPath>,
    /// For `Text` / `Sticky`: the plain body.
    pub text: Option<String>,
    /// For `Image`: the media content hash referencing a `MediaBlob`.
    pub media_hash: Option<ambre_core::ContentHash>,
    /// For `Image`: the PNG bytes the export writes inline, so a reader outside
    /// Ambre resolves the picture (`TOILE-IO-087`). This is the thumbnail the
    /// board holds in plaintext; the full blob is ciphertext on the relay and
    /// no path here fetches it (`TOILE-IO-100`).
    pub image_png: Option<ambre_core::ThumbnailPng>,
}

impl ExportElement {
    /// A minimal element of the given kind at `rect` with the default style.
    pub fn new(kind: ElementKind, rect: Rect) -> Self {
        Self {
            kind,
            rect,
            rotation: ambre_core::Radians::ZERO,
            style: ElementStyle::default(),
            path: None,
            text: None,
            media_hash: None,
            image_png: None,
        }
    }
}
