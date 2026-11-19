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

use ambre_core::Count;
use ambre_core::index::{Axis, Pos};
use kurbo::{BezPath, Point, Rect};

use crate::logic::command::{MediaRef, ToileCommand};
use crate::logic::model::{Anchor, ElementKind, ElementStyle, EndCaps, PathType};

mod board_import;
mod deck_import;
/// The native `.ambre` container (`ambre-spec-docfile.md`). Lossless, so it sits
/// apart from the converters here: no `ImportReport`, no warning list.
pub mod docfile;
pub mod limits;
pub mod media;
#[cfg(feature = "pdf")]
mod pdf_export;
#[cfg(feature = "pdf")]
mod png_export;
mod pptx_export;
mod raster;
mod showcase;
mod svg_export;
mod svg_import;

#[cfg(test)]
mod tests;

pub use board_import::import_board;
pub use deck_import::import_deck;
pub use limits::{CoordBound, ImportDepth, Limits};
pub use media::{
    CachedBlobs, ContentHash, MediaCache, MediaCacheCapacity, MediaState, RequestOutcome,
};
#[cfg(feature = "pdf")]
pub(crate) use pdf_export::DeckPages;
#[cfg(all(test, feature = "pdf"))]
pub(crate) use pdf_export::paint_page;
#[cfg(feature = "pdf")]
pub use pdf_export::{
    DeckRecording, DeckStage, EXPORT_BYTES_CAP, PageSize, PdfExportError, PdfPage, PdfPagePlan,
    Thumbnails, export_pdf, record_pdf, stored_picture, write_pdf,
};
#[cfg(feature = "pdf")]
pub use png_export::{PNG_ZOOM, PngExportError, PngPage, PngRequest};
#[cfg(feature = "pdf")]
pub(crate) use png_export::{board_region, png_page};
pub use pptx_export::{PptxConnector, PptxElement, PptxExport, PptxSlideSource, export_pptx};
pub(crate) use raster::decode_thumbnail;
pub use raster::{import_raster, jpeg_blob, png_of, raster_batch};
pub use showcase::showcase_page;
pub use svg_export::{SvgExport, export_svg};
pub use svg_import::import_svg;

/// A reference to something the apply pass mints, used by converters that cannot
/// name `Id<Element>`s, which are minted only when `CreateElement` is applied. It
/// is either a `CreateElement` slot (an index into [`ImportBatch::commands`]) or
/// a group produced by an earlier [`ImportGroup`] (an index into
/// [`ImportBatch::groups`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRef {
    Slot(Pos<ImportElement>),
    Group(Pos<ImportGroupRank>),
}

/// The ranks of the `CreateElement`s in [`ImportBatch::commands`]. A rank
/// means nothing outside its batch. The importer takes the next rank from
/// the batch's length, and the resolver reads the create at that rank.
#[derive(PartialEq, Eq)]
pub enum ImportElement {}

impl Axis for ImportElement {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportElement");
}

/// The ranks of the groups in [`ImportBatch::groups`]. A rank means nothing
/// outside its batch, as for [`ImportElement`].
#[derive(PartialEq, Eq)]
pub enum ImportGroupRank {}

impl Axis for ImportGroupRank {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportGroupRank");
}

/// The ranks of the frames in [`ImportBatch::frames`]. A rank means nothing
/// outside its batch, as for [`ImportElement`].
#[derive(PartialEq, Eq)]
pub enum ImportFrameRank {}

impl Axis for ImportFrameRank {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("ImportFrameRank");
}

/// A `<g>` (or a Figma/Miro group) to rebuild via `GroupElements` after the
/// creates land. Emitted innermost-first (post-order) so a member
/// `Group(i)` always refers to an already-resolved entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    pub members: Vec<ImportRef>,
}

/// A frame to fill after the creates and the groups land: its members,
/// named by slot or by group, and its title (`toile-frames`). Emitted
/// innermost-first, as groups are, so a nested frame holds its own members
/// before its parent takes it. A frame of a deck carries its speaker notes
/// and joins the end of the deck, in batch order (`toile-pptx-import`).
#[derive(Clone, PartialEq, Eq)]
pub struct ImportFrame {
    /// The create slot of the frame.
    pub frame: Pos<ImportElement>,
    /// What the frame holds, in paint order.
    pub members: Vec<ImportRef>,
    pub name: String,
    /// The speaker notes of a slide: `Some` for a frame that joins the deck.
    pub notes: Option<String>,
}

/// The title and the notes print as their lengths alone (`ZK-006`).
impl std::fmt::Debug for ImportFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportFrame")
            .field("frame", &self.frame)
            .field("members", &self.members)
            .field("name", &ambre_core::Redacted::new(self.name.as_str()))
            .field(
                "notes",
                &self.notes.as_deref().map(ambre_core::Redacted::new),
            )
            .finish()
    }
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
#[derive(Debug)]
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
    /// Index-aligned with `commands`, and shorter when the tail is upright:
    /// the rotation of that slot, which a create cannot carry.
    pub rotations: Vec<ambre_core::Radians>,
    /// Groups to rebuild after the creates land, innermost-first.
    pub groups: Vec<ImportGroup>,
    /// Frames to title and fill after the groups land, innermost-first.
    pub frames: Vec<ImportFrame>,
    /// Connectors to create after the elements land (`toile-connectors`).
    pub connectors: Vec<ImportConnector>,
    /// Index-aligned with `commands`, or shorter: the turn of the element at
    /// that slot. A slot past its end keeps the unturned box.
    pub turns: Vec<ambre_core::Radians>,
    /// Plaintext media referenced by the commands (an `image` element's hash is a
    /// key here). Handed to Sync for encrypt-then-upload; interop holds no keys.
    pub media: Vec<MediaBlob>,
    /// What was imported and what was dropped (unsupported constructs).
    pub report: ImportReport,
}

impl ImportBatch {
    /// An empty batch read from a file of `format`.
    #[must_use]
    pub const fn new(format: ambre_bus::interop::FileFormat) -> Self {
        Self {
            commands: Vec::new(),
            styles: Vec::new(),
            rotations: Vec::new(),
            groups: Vec::new(),
            frames: Vec::new(),
            connectors: Vec::new(),
            turns: Vec::new(),
            media: Vec::new(),
            report: ImportReport::new(ambre_bus::interop::Direction::Import, format),
        }
    }
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

/// What an import carried and what it left behind (`INTEROP-003`): the
/// shared report over the board's constructs. The converter records what it
/// could not map, and the apply pass adds what landed.
pub type ImportReport = crate::message::ToileReport;

/// One plaintext media blob produced by an import. `media` names it by its
/// import slot until Sync returns the content address (blake2b), since interop
/// never hashes. `size` is the native resolution (the media map value), `thumb` is a small PNG-encoded placeholder for the first paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBlob {
    pub media: MediaRef,
    pub plaintext: ambre_core::Payload<ambre_core::MediaBytes>,
    pub mime: ambre_core::MediaType,
    pub size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    pub thumb: ambre_core::Payload<ambre_core::ThumbnailPng>,
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
    #[error("failed to decode image: {0}")]
    Jpeg(ambre_convert::jpeg::JpegRefusal),
    #[error("import exceeds the element limit ({limit})")]
    TooManyElements {
        limit: ambre_core::Count<ambre_core::Elements>,
    },
    #[error("import exceeds the nesting-depth limit ({limit})")]
    TooDeep { limit: Count<ImportDepth> },
    #[error(
        "decoded image {}x{} exceeds the dimension limit ({}x{})",
        size.width,
        size.height,
        limit.width,
        limit.height
    )]
    ImageTooLarge {
        size: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
        limit: ambre_core::Size<ambre_core::Px<ambre_core::Image>>,
    },
    #[error("import produced no elements")]
    Empty,
}

impl ImportError {
    /// The error as the bus carries it. The parser's words go to the log here
    /// (`BUS-046`, `INTEROP-004`).
    #[must_use]
    pub fn interop(&self) -> ambre_bus::interop::InteropError {
        use ambre_bus::interop::InteropError;
        use ambre_core::{FileRefusal, StopCause};
        tracing::warn!("Toile: an import failed: {self}");
        match self {
            Self::Parse(_) | Self::Json { .. } | Self::Decode(_) => {
                InteropError::ParseFailed(FileRefusal::Malformed)
            }
            Self::Jpeg(ambre_convert::jpeg::JpegRefusal::NotJpeg) => {
                InteropError::ParseFailed(FileRefusal::OtherDocument)
            }
            Self::Jpeg(ambre_convert::jpeg::JpegRefusal::PastBound { .. })
            | Self::TooManyElements { .. }
            | Self::TooDeep { .. }
            | Self::ImageTooLarge { .. } => InteropError::Stopped(StopCause::PastBound),
            Self::Jpeg(_) => InteropError::ParseFailed(FileRefusal::Malformed),
            Self::Empty => InteropError::Stopped(StopCause::NothingToConvert),
        }
    }
}

/// A single element to serialize on the **export** path (high-res SVG). A
/// round-trippable view model decoupled from the CRDT `DisplayElement`, so the
/// SVG serializer stays pure and host-testable; a Phase-1 adapter maps a
/// `DisplayViewport` into `[ExportElement]`.
#[derive(Clone, PartialEq)]
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
    pub media_hash: Option<ambre_core::Validated<ambre_core::ContentHash>>,
    /// For `Image`: the PNG bytes the export writes inline, so a reader outside
    /// Ambre resolves the picture (`TOILE-IO-087`). This is the thumbnail the
    /// board holds in plaintext; the full blob is ciphertext on the relay and
    /// no path here fetches it (`TOILE-IO-100`).
    pub image_png: Option<ambre_core::Payload<ambre_core::ThumbnailPng>>,
}

/// The body and the outline print as their sizes alone (`ZK-006`); the
/// thumbnail prints its own length.
impl std::fmt::Debug for ExportElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::logic::PathLen;
        f.debug_struct("ExportElement")
            .field("kind", &self.kind)
            .field("rect", &self.rect)
            .field("rotation", &self.rotation)
            .field("style", &self.style)
            .field("path", &self.path.as_ref().map(PathLen::new))
            .field("text", &self.text.as_deref().map(ambre_core::Redacted::new))
            .field("media_hash", &self.media_hash)
            .field("image_png", &self.image_png)
            .finish()
    }
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
