//! PDF export for Toile decks (`toile-pdf-export`), gated behind the `pdf`
//! feature.
//!
//! One presentation frame becomes one PDF page. The export drives the **same**
//! [`BoardWalk`](crate::ui::walk::BoardWalk) the screen uses (`CANVAS-074`). It
//! records the walk through a [`ListRecorder`] instead of the GPU surface, and
//! the converter module writes the recorded pages
//! (`ambre-spec-architecture.md section 2`, the converter module). Screen and
//! PDF therefore render identically, and the Main instance links no PDF writer. This is why
//! `logic::interop` reaches into `ui::walk` / `ui::viewport`: both are pure,
//! wgpu-free geometry.
//!
//! Each page is culled and fitted to its frame's spatial rect (a [`PdfPage`],
//! built by `ToileState::pdf_pages`). The walk's region is that rect, and the
//! walk clips to it. The clip is the fix for the aspect-ratio bleed. `fit_target`
//! scales to the tighter axis (`ui::viewport`), so slack on the wider axis
//! would otherwise pull neighbouring board content into the page. The page
//! draws no board grid and no frame title, so it shows only slide content.

use ambre_core::hash::HashMap;

use ambre_bus::interop::{ConversionProgress, Direction, Steps};
use ambre_canvas::{Palette, TextCache};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{Count, PrintLength, Pt};
use ambre_draw::list::{ListRecorder, PrintDocument, PrintPage, RecorderTables};
use ambre_draw::{DocumentWalk, DrawSurface, FontContext, Rect};

use crate::logic::command::DisplayViewport;
use crate::logic::state::ToileState;
use crate::message::{DisplayElement, DisplayGeom, DisplayNode};
use crate::ui::grid;
use crate::ui::viewport::ViewportTransform;
use crate::ui::walk::{BoardView, BoardWalk, DecodedImage, StoredPicture, StoredPictures};
use ambre_bus::Request;
use ambre_core::{ContentHash, MediaBytes, Octets, Payload, Seq, Validated};

/// Output page dimensions, in PDF points (1 pt = 1/72 inch). The export camera's
/// pixel size equals these points 1:1, so the on-screen `fit_bounds` math maps
/// straight onto the page with no extra scaling. Defaults to a 16:9 slide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: PrintLength<Pt>,
    pub height: PrintLength<Pt>,
}

impl PageSize {
    /// A page of `width` x `height` points, each clamped to a positive minimum so
    /// the writer never rejects it.
    pub fn new(width: PrintLength<Pt>, height: PrintLength<Pt>) -> Self {
        let least = PrintLength::<Pt>::new(1.0);
        Self {
            width: width.max(least),
            height: height.max(least),
        }
    }
}

impl Default for PageSize {
    fn default() -> Self {
        // 1280x720: a 16:9 slide, matching the default presentation aspect.
        Self::new(
            PrintLength::<Pt>::new(1280.0),
            PrintLength::<Pt>::new(720.0),
        )
    }
}

/// One rendered page. That is the culled, z-ordered viewport already derived at
/// the page camera (`ToileState::derive_viewport_at`). Then the camera that
/// fits the slide into the page, and the frame's **world-space** rect to clip
/// to. `clip` is `None` for the whole-content fallback page of an empty deck.
/// There is no frame to clip to, so the fitted content is shown unclipped.
#[derive(Debug, Clone)]
pub struct PdfPage {
    pub viewport: DisplayViewport,
    pub camera: ViewportTransform,
    pub clip: Option<Rect>,
}

/// The frame of one page before its content is derived: the camera that fits
/// the slide into the page, and the clip of a [`PdfPage`]. A deck export frames
/// and derives each page in the step that records it.
#[derive(Debug, Clone, Copy)]
pub struct PdfPagePlan {
    pub(crate) camera: ViewportTransform,
    pub(crate) clip: Option<Rect>,
}

impl PdfPagePlan {
    /// The page whose camera centres `center` at `zoom` over `size` page
    /// points, clipped to `clip`. A raster export frames its page here, so it
    /// names no view type.
    pub(crate) fn framed(
        center: ambre_draw::Point,
        zoom: ambre_core::Scale<ambre_core::Board, ambre_core::View>,
        size: ambre_draw::Size,
        clip: Option<Rect>,
    ) -> Self {
        Self {
            camera: ViewportTransform { center, zoom, size },
            clip,
        }
    }

    /// The camera of the page, for a test that maps its corners.
    #[cfg(test)]
    pub(crate) fn camera(&self) -> ViewportTransform {
        self.camera
    }
}

/// A PDF export failure. `Convert` wraps the converter module's refusal; the
/// walk itself cannot fail (it only emits draw calls).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfExportError {
    #[error("no pages to export")]
    Empty,
    #[error("{0}")]
    Convert(#[from] ConvertError),
}

/// Render `pages` into a single PDF document (`toile-pdf-export`).
///
/// Every page runs the screen content walk against a fresh page of a
/// recorded [`PrintDocument`], clipped to its frame rect. `converter` writes
/// the recorded pages as PDF bytes. `font_cx` must already carry the base
/// faces (`LogicState::export_cx`, seeded at boot) or text emits no glyphs.
/// `images` carries the decoded media pixels an `Image` element paints; an
/// absent hash falls back to a placeholder box. `editing` and Smart Animate
/// overrides have no meaning off-screen, so the walk runs with none.
///
/// Deterministic: the writer assembles the same bytes for the same input, so
/// two runs of identical `pages` are byte-stable.
///
/// # Errors
///
/// [`PdfExportError::Empty`] on an empty `pages`, or
/// [`PdfExportError::Convert`] when the converter refuses or cannot run.
pub fn export_pdf(
    pages: &[PdfPage],
    page: PageSize,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    images: &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
    converter: &Converter,
) -> Result<ambre_core::Payload<ambre_client_core::FileBytes>, PdfExportError> {
    let document = record_pdf(pages, page, palette, font_cx, text_cache, images)?;
    write_pdf(document, converter)
}

/// Write a recorded document as PDF bytes, through the PDF module.
///
/// # Errors
///
/// [`PdfExportError::Convert`] when the converter refuses, cannot run, or
/// answers something other than a file.
pub fn write_pdf(
    document: PrintDocument,
    converter: &Converter,
) -> Result<ambre_core::Payload<ambre_client_core::FileBytes>, PdfExportError> {
    match converter.convert(&Conversion::WritePdf { document })? {
        Converted::File(bytes) => Ok(bytes),
        _ => Err(ConvertError::Malformed.into()),
    }
}

/// Record `pages` as the document the converter writes: one op list per
/// page, every face and picture once.
///
/// # Errors
///
/// [`PdfExportError::Empty`] on an empty `pages`.
pub fn record_pdf(
    pages: &[PdfPage],
    page: PageSize,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    images: &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
) -> Result<PrintDocument, PdfExportError> {
    if pages.is_empty() {
        return Err(PdfExportError::Empty);
    }
    let mut document = PrintDocument::new(PrintPage {
        width: page.width,
        height: page.height,
    });
    let mut tables = RecorderTables::default();
    let files = StoredPictures::default();
    for pdf_page in pages {
        record_page(
            &mut document,
            &mut tables,
            pdf_page,
            palette,
            font_cx,
            text_cache,
            images,
            &files,
        );
    }
    Ok(document)
}

/// Record one page at the end of `document`.
#[expect(
    clippy::too_many_arguments,
    reason = "the page, its sinks and the two picture stores of one walk"
)]
fn record_page(
    document: &mut PrintDocument,
    tables: &mut RecorderTables,
    pdf_page: &PdfPage,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    images: &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
    files: &StoredPictures,
) {
    let mut recorder = ListRecorder::new(document, tables);
    paint_page(
        &mut recorder,
        pdf_page,
        palette,
        font_cx,
        text_cache,
        (images, files),
    );
    recorder.finish();
}

/// The stored thumbnail of each picture a deck may draw, by content hash,
/// still encoded.
pub type Thumbnails = HashMap<
    ambre_core::Validated<ambre_core::ContentHash>,
    ambre_core::Payload<ambre_core::ThumbnailPng>,
>;

/// The pages of a deck export, as the request reads them (`TOILE-IO-099`).
pub(crate) enum DeckPages {
    /// Each page framed already: the page of a board with no deck, or the
    /// one page of a raster export.
    Planned(Vec<PdfPagePlan>),
    /// The frame of each slide, in deck order. Each page is framed in the
    /// step that records it, so the request reads no frame.
    Slides(Vec<ambre_core::id::Id<ambre_core::id::Element>>),
}

impl DeckPages {
    /// How many pages the export records: one step each.
    fn steps(&self) -> Count<Steps> {
        Count::<Steps>::of(match self {
            Self::Planned(plans) => plans.len(),
            Self::Slides(slides) => slides.len(),
        })
    }
}

/// The pages a recording has still to record, in deck order.
enum PageQueue {
    Planned(std::vec::IntoIter<PdfPagePlan>),
    Slides(std::vec::IntoIter<ambre_core::id::Id<ambre_core::id::Element>>),
}

impl From<DeckPages> for PageQueue {
    fn from(pages: DeckPages) -> Self {
        match pages {
            DeckPages::Planned(plans) => Self::Planned(plans.into_iter()),
            DeckPages::Slides(slides) => Self::Slides(slides.into_iter()),
        }
    }
}

/// A deck export in progress: its pages are recorded one per step
/// (`TOILE-IO-099`).
///
/// The recording owns the slides and the stored pictures, read at the
/// request. Each step frames and derives its own page alone, so
/// the request costs one page and not the deck (`TOILE-IO-099`). The Logic
/// thread applies a local edit or a remote delta between two steps. The file
/// still shows the deck as it stood at the request (`TOILE-IO-133`). A step
/// derives from the board while the board holds the version of the request.
/// Once the board moved on, it derives from a private copy at that version.
///
/// The output is a recorded [`PrintDocument`], which names no writer. The
/// PDF module writes it today through [`write_pdf`]. Another sink of a draw
/// list takes the same steps unchanged.
pub struct DeckRecording {
    /// The pages still to record, in deck order.
    pages: PageQueue,
    /// The size of every page.
    page: PageSize,
    /// The board the pages are derived from: the version of the request.
    source: DeckSource,
    /// The address of each picture the pages draw, listed at the request.
    pictures: Vec<Validated<ContentHash>>,
    /// Every page of the export: one step each.
    total: Count<Steps>,
    /// The pages recorded so far.
    done: Count<Steps>,
    /// The pictures no recorded page has drawn yet, still encoded.
    thumbs: Thumbnails,
    /// The pictures a recorded page has drawn, decoded once for every page
    /// after it.
    images: HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
    /// The downloads of full blobs still owed, by the request id this
    /// recording allocated (`TOILE-IO-100`).
    waiting: HashMap<Seq<Request>, Validated<ContentHash>>,
    /// The stored files the Sync worker opened, each drawn whole.
    files: StoredPictures,
    /// The bytes `files` holds, under [`EXPORT_BYTES_CAP`].
    held: ambre_core::Count<Octets>,
    /// The pictures a page drew without their stored file: from the
    /// thumbnail, or as the placeholder box when that did not decode.
    approximated: ambre_core::hash::HashSet<Validated<ContentHash>>,
    palette: &'static Palette,
    /// One cache for the whole export, so a run shaped on one page is not
    /// shaped again on the next. It holds no loader: a codepoint no held face
    /// covers draws the missing glyph (`PDF-020`).
    text_cache: TextCache,
    document: PrintDocument,
    tables: RecorderTables,
}

impl DeckRecording {
    /// A recording of `pages` at `page` size, read off `board` as it stands
    /// now, with no page recorded yet. A picture of
    /// `thumbs` is decoded by the step of the first page that draws it, so
    /// the start decodes none. The start derives no page either.
    ///
    /// # Errors
    ///
    /// [`PdfExportError::Empty`] on empty `pages` (`TOILE-IO-048`).
    pub(crate) fn new(
        pages: DeckPages,
        board: &ToileState,
        page: PageSize,
        palette: &'static Palette,
        thumbs: Thumbnails,
    ) -> Result<Self, PdfExportError> {
        let total = pages.steps();
        if total == Count::ZERO {
            return Err(PdfExportError::Empty);
        }
        Ok(Self {
            total,
            pictures: board.pictures_of_pages(&pages, page),
            pages: pages.into(),
            page,
            source: DeckSource::AtRequest(board.frontiers()),
            done: Count::ZERO,
            thumbs,
            images: HashMap::default(),
            waiting: HashMap::default(),
            files: StoredPictures::default(),
            held: ambre_core::Count::ZERO,
            approximated: ambre_core::hash::HashSet::default(),
            palette,
            // The Logic thread has no window to fetch a face through, so the
            // export shapes on the faces it holds and arms no fetch
            // (`PDF-019`).
            text_cache: TextCache::over_held_faces(),
            document: PrintDocument::new(PrintPage {
                width: page.width,
                height: page.height,
            }),
            tables: RecorderTables::default(),
        })
    }

    /// Record the next page, if one is left: one step, through the screen's
    /// walk. `board` is the board the export was asked of, as it stands now;
    /// the page shows it as it stood at the request. `font_cx` carries the
    /// base faces, as for [`export_pdf`].
    pub fn record_next(&mut self, board: &ToileState, font_cx: &mut FontContext) {
        let source = self.source.board(board);
        let plan = match &mut self.pages {
            PageQueue::Planned(plans) => plans.next(),
            // The frame is read off the board at the request, like the
            // content of its page (`TOILE-IO-133`).
            PageQueue::Slides(slides) => match slides.next() {
                Some(frame) => source.slide_plan(&frame, self.page),
                None => return,
            },
        };
        let Some(plan) = plan else {
            // A slide whose frame the version of the request lacks: its step
            // counts and records no page. The request listed held frames
            // only, so no deck reaches this.
            if matches!(self.pages, PageQueue::Slides(_)) {
                self.done = self.done.incremented();
            }
            return;
        };
        let pdf_page = source.derive_pdf_page(plan);
        self.decode_pictures_of(&pdf_page);
        record_page(
            &mut self.document,
            &mut self.tables,
            &pdf_page,
            self.palette,
            font_cx,
            &mut self.text_cache,
            &self.images,
            &self.files,
        );
        self.done = self.done.incremented();
    }

    /// Decode each picture `pdf_page` draws that no page before it drew and
    /// that has no stored file. A thumbnail that does not decode is dropped,
    /// so its element paints the placeholder box rather than failing the
    /// export.
    fn decode_pictures_of(&mut self, pdf_page: &PdfPage) {
        for hash in pictures_of(pdf_page) {
            if self.files.contains_key(hash) {
                continue;
            }
            self.approximated.insert(hash.clone());
            if let Some(thumb) = self.thumbs.remove(hash)
                && let Some(image) = super::raster::decode_thumbnail(&thumb)
            {
                self.images.insert(hash.clone(), image);
            }
        }
    }

    /// Ask for the stored file of every picture a page draws, one download
    /// per address (`TOILE-IO-100`). The request listed them off the image
    /// elements, so no page is derived before the downloads. The recording
    /// waits until each answered or [`stop_waiting`](Self::stop_waiting)
    /// gives up on them. A deck with no picture asks for nothing and waits
    /// for nothing. A second call asks for nothing.
    pub fn fetch_full(&mut self) -> Vec<crate::logic::command::ToileRequest> {
        let addresses = std::mem::take(&mut self.pictures);
        let mut requests = Vec::with_capacity(addresses.len());
        for address in addresses {
            let id = ambre_bus::next_request();
            self.waiting.insert(id, address.clone());
            requests.push(crate::logic::command::ToileRequest::DownloadMedia {
                id,
                content_hash: address,
            });
        }
        requests
    }

    /// Where the recording stands: still fetching stored files, or
    /// recording pages. No page is recorded while a download is owed.
    #[must_use]
    pub fn stage(&self) -> DeckStage {
        if self.waiting.is_empty() {
            DeckStage::Recording
        } else {
            DeckStage::Fetching
        }
    }

    /// The address download `id` asks for, while this recording still owes
    /// it. `None` for an id it never asked for, or already answered.
    #[must_use]
    pub fn owes(&self, id: Seq<Request>) -> Option<&Validated<ContentHash>> {
        self.waiting.get(&id)
    }

    /// Take the answer to download `id`: the opened bytes, or `None` when the
    /// Sync worker refused or failed it. An id this recording does not owe
    /// is dropped.
    ///
    /// The bytes are kept only when they spell a PNG or a JPEG whose header
    /// declares a size, and fit under [`EXPORT_BYTES_CAP`]. Anything else
    /// leaves the picture to its thumbnail, and the report counts it. The
    /// fallback is never silent, and the PDF module never gets a file it
    /// would draw broken.
    pub fn answer(&mut self, id: Seq<Request>, data: Option<Payload<MediaBytes>>) {
        let Some(address) = self.waiting.remove(&id) else {
            return;
        };
        if let Some(file) = data.and_then(stored_picture)
            && self.held.plus(file.bytes.len()) <= EXPORT_BYTES_CAP
        {
            self.held = self.held.plus(file.bytes.len());
            self.files.insert(address, file);
        }
    }

    /// Give up on every download still owed: their pictures go out from the
    /// thumbnail. A second export of the same board settles the first this
    /// way (`TOILE-IO-135`).
    pub fn stop_waiting(&mut self) {
        self.waiting.clear();
    }

    /// The report of the file: one approximated picture for each picture a
    /// page drew without its stored file.
    #[must_use]
    pub fn report(&self) -> crate::message::ToileReport {
        let mut report = crate::message::ToileReport::new(
            Direction::Export,
            ambre_bus::interop::FileFormat::Pdf,
        );
        report.record_count(
            crate::message::ToileConstruct::Picture,
            ambre_bus::interop::Outcome::Approximated,
            ambre_core::Count::of(self.approximated.len()),
        );
        report
    }

    /// Whether every page is recorded, so the document is ready to write.
    #[must_use]
    pub fn is_recorded(&self) -> bool {
        self.done >= self.total
    }

    /// How far the recording got, in pages (`TOILE-IO-134`).
    #[must_use]
    pub fn progress(&self) -> ConversionProgress {
        ConversionProgress::new(Direction::Export, self.done, self.total)
    }

    /// The recorded document. A page not yet recorded is absent from it.
    #[must_use]
    pub fn into_document(self) -> PrintDocument {
        self.document
    }
}

/// Where a deck export stands (`TOILE-IO-100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckStage {
    /// A download of a stored file is still owed: no page is recorded.
    Fetching,
    /// Every download answered or was given up: pages are recorded.
    Recording,
}

/// The encoded picture bytes one deck export holds (`TOILE-IO-100`,
/// `ambre-spec-budgets.md section 6`). A picture past it goes out from its
/// thumbnail, and the report counts it.
pub const EXPORT_BYTES_CAP: ambre_core::Count<Octets> = ambre_core::Count::new(32 * 1024 * 1024);

/// The address of each picture `pdf_page` draws, in walk order.
fn pictures_of(pdf_page: &PdfPage) -> impl Iterator<Item = &Validated<ContentHash>> {
    pdf_page
        .viewport
        .nodes
        .iter()
        .filter_map(|node| match node {
            DisplayNode::Element(DisplayElement {
                geom: DisplayGeom::Image(Some(hash)),
                ..
            }) => Some(hash),
            _ => None,
        })
}

/// A stored file as a page draws it. `None` when the bytes are neither a
/// PNG nor a JPEG, or when their header declares no size or one past the
/// import bound. Only the header is read: a JPEG is walked by `ambre_convert::jpeg`, since Main links no JPEG
/// decoder (`TOILE-IO-131`).
#[must_use]
pub fn stored_picture(bytes: Payload<MediaBytes>) -> Option<StoredPicture> {
    let (format, size) = if ambre_convert::jpeg::is_jpeg(&bytes) {
        (
            ambre_draw::EncodedFormat::Jpeg,
            ambre_convert::jpeg::stored_size(&bytes)?,
        )
    } else if image::guess_format(bytes.as_bytes()).ok()? == image::ImageFormat::Png {
        let (width, height) = image::ImageReader::with_format(
            std::io::Cursor::new(bytes.as_bytes()),
            image::ImageFormat::Png,
        )
        .into_dimensions()
        .ok()?;
        (
            ambre_draw::EncodedFormat::Png,
            ambre_core::Size::new(ambre_core::Px::new(width), ambre_core::Px::new(height)),
        )
    } else {
        return None;
    };
    // A peer wrote the blob, so a header past the import bound is refused
    // here too: the PDF module would expand it (`TOILE-IO-052`).
    super::Limits::default().check_image_dims(size).ok()?;
    Some(StoredPicture {
        format,
        bytes,
        size,
    })
}

/// The board a deck export derives its pages from.
enum DeckSource {
    /// No change reached the board since the request, at this version: the
    /// board itself is the deck as it stood.
    AtRequest(loro::Frontiers),
    /// The board moved on: a private copy of it at the version of the
    /// request, built once, in the first step after the change.
    Frozen(Box<ToileState>),
}

impl DeckSource {
    /// The board as it stood at the request. `live` is the board now.
    fn board<'a>(&'a mut self, live: &'a ToileState) -> &'a ToileState {
        if let Self::AtRequest(at) = self {
            if live.frontiers() == *at {
                return live;
            }
            match live.frozen_at(at) {
                Some(frozen) => *self = Self::Frozen(Box::new(frozen)),
                None => {
                    // Unreachable while the board keeps its history, which it
                    // trims nowhere. The page then shows the board as it is.
                    tracing::warn!("Toile: the PDF export lost the version it was asked at");
                    return live;
                }
            }
        }
        match self {
            Self::Frozen(frozen) => frozen,
            Self::AtRequest(_) => live,
        }
    }
}

/// Paint one page onto any surface: the board background, then the screen's
/// board walk under the page camera, over the frame's region (`CANVAS-074`).
///
/// It names no surface type, so the page a test records is the page krilla
/// writes.
pub(crate) fn paint_page<S: DrawSurface>(
    s: &mut S,
    pdf_page: &PdfPage,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    (images, files): (
        &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
        &StoredPictures,
    ),
) {
    grid::paint_background(
        s,
        &pdf_page.camera,
        grid::GroundColours::of(palette, pdf_page.viewport.ground),
    );
    // The region is the frame, so the page carries only slide content. The
    // fallback page of an empty deck has no frame and shows what its camera
    // sees.
    let region = pdf_page
        .clip
        .unwrap_or_else(|| pdf_page.camera.visible_world());
    // Nothing is under inline edit off-screen, and no Smart Animate
    // transition runs on export.
    let anim = HashMap::default();
    let view = BoardView {
        viewport: &pdf_page.viewport,
        palette,
        images,
        files,
        anim: &anim,
        muted_body: None,
        muted_caption: None,
    };
    s.push_transform(pdf_page.camera.world_to_screen());
    BoardWalk {
        font_cx,
        text_cache,
    }
    .walk(&view, region, s);
    s.pop_transform();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{
        DisplayElement, DisplayGeom, DisplayNode, ElementKind, ElementStyle, ShapeKind,
    };
    use ambre_canvas::Theme;
    use ambre_core::id::{Element, Id};
    use ambre_core::{Board, Px};
    use ambre_draw::Affine;

    /// The converter the Logic thread's host build hands in: the engine,
    /// linked in place.
    fn converter() -> Converter {
        Converter::load(
            ambre_core::ConverterFamily::Pdf,
            &ambre_core::Payload::new(Vec::new()),
        )
        .expect("the host links the engine")
    }

    /// The export, written through [`converter`].
    fn export_pdf(
        pages: &[PdfPage],
        page: PageSize,
        palette: &Palette,
        font_cx: &mut FontContext,
        text_cache: &mut TextCache,
        images: &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
    ) -> Result<ambre_core::Payload<ambre_client_core::FileBytes>, PdfExportError> {
        super::export_pdf(
            pages,
            page,
            palette,
            font_cx,
            text_cache,
            images,
            &converter(),
        )
    }

    fn rect_node(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: Id::<Element>::from_text(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: crate::message::LockState::Unlocked,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn one_page() -> PdfPage {
        let frame = Rect::new(0.0, 0.0, 200.0, 100.0);
        let mut camera = ViewportTransform::new(ambre_draw::Size::new(1280.0, 720.0));
        camera.fit_bounds(frame, 0.08);
        PdfPage {
            viewport: DisplayViewport {
                reactions: Default::default(),
                pins: Vec::new(),
                viewport: Default::default(),
                nodes: vec![rect_node("a", Rect::new(10.0, 10.0, 90.0, 90.0))],
                version: ambre_core::Seq::ZERO,
                grid_unit: Px::<Board>::new(20.0),
                snap: crate::message::GridSnap::Free,
                ground: crate::message::BoardGround::Theme,
            },
            camera,
            clip: Some(frame),
        }
    }

    /// A single page renders to a well-formed, non-trivial PDF. No fonts are
    /// registered here, so text (none present) is irrelevant; the rect proves the
    /// content walk reached the krilla surface.
    #[test]
    fn export_yields_a_pdf() {
        let palette = Theme::Light.palette();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
        let bytes = export_pdf(
            &[one_page()],
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &HashMap::default(),
        )
        .expect("export must succeed");
        assert!(
            bytes.as_bytes().starts_with(b"%PDF"),
            "output must be a PDF"
        );
        assert!(
            bytes.len().get() > 400,
            "content stream should be non-trivial"
        );
    }

    /// Export is deterministic: two runs of the same pages produce identical
    /// bytes.
    #[test]
    fn export_is_byte_stable() {
        let palette = Theme::Light.palette();
        let run = || {
            let mut font_cx = FontContext::new();
            let mut tc = TextCache::new();
            export_pdf(
                &[one_page()],
                PageSize::default(),
                palette,
                &mut font_cx,
                &mut tc,
                &HashMap::default(),
            )
            .expect("export must succeed")
        };
        assert_eq!(run(), run(), "identical input must yield identical bytes");
    }

    /// The faces the application ships, read off the tree. A running peer gets
    /// the same bytes from the Main thread (`LogicCommand::LoadFonts`).
    fn shipped_faces() -> Vec<Vec<u8>> {
        const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ambre-app/fonts/");
        ["NotoSans-Regular.ttf", "DejaVuSerif.ttf"]
            .iter()
            .filter_map(|file| std::fs::read(format!("{DIR}{file}")).ok())
            .collect()
    }

    /// One page holding a word, so the export has something to shape.
    fn a_page_with_text() -> PdfPage {
        let mut page = one_page();
        page.viewport
            .nodes
            .push(DisplayNode::Element(DisplayElement {
                id: Id::<Element>::from_text("t".to_owned()),
                kind: ElementKind::Text,
                transform: Affine::IDENTITY,
                bounds: Rect::new(100.0, 10.0, 190.0, 40.0),
                locked: crate::message::LockState::Unlocked,
                style: ElementStyle::default(),
                geom: DisplayGeom::Text("Ambre".to_owned()),
                link: None,
                group: None,
            }));
        page
    }

    /// Two peers holding the same faces export one file (`PDF-010`).
    ///
    /// Each run builds its own sealed context, as a second peer would, and
    /// feeds it the shipped faces. A sealed context discovers nothing of the
    /// machine, so what shapes the word is what was registered and nothing
    /// else. The empty context is the control: its file differs, so the faces
    /// really reached the page and the equality is not two empty pages.
    #[test]
    fn two_peers_holding_the_same_faces_export_one_file() {
        let palette = Theme::Light.palette();
        let faces = shipped_faces();
        assert!(!faces.is_empty(), "the workspace ships its faces");
        let run = |faces: &[Vec<u8>]| {
            let mut font_cx = ambre_draw::sealed_font_context();
            for bytes in faces {
                ambre_draw::register_face(
                    &mut font_cx,
                    &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes.clone())),
                );
            }
            let mut tc = TextCache::new();
            export_pdf(
                &[a_page_with_text()],
                PageSize::default(),
                palette,
                &mut font_cx,
                &mut tc,
                &HashMap::default(),
            )
            .expect("export must succeed")
        };
        let alice = run(&faces);
        let bob = run(&faces);
        assert_eq!(
            alice, bob,
            "one document and one set of faces make one file"
        );
        assert_ne!(alice, run(&[]), "the shipped faces shaped the word");
    }

    /// A word on an exported slide reads back as its characters (`PDF-005`).
    ///
    /// The body is set in the shipped serif, which ligates "ffi", and carries
    /// a combining acute. An independent extractor reads the written file, so
    /// a page of shapes alone reads back empty. The extractor is a host
    /// dev-dependency.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_exported_word_reads_back_as_its_characters() {
        let text = "office cafe\u{301}";
        let mut page = one_page();
        let mut style = ElementStyle::default();
        style.font.family = Some(ambre_core::FontFamily::Serif);
        page.viewport
            .nodes
            .push(DisplayNode::Element(DisplayElement {
                id: Id::<Element>::from_text("t".to_owned()),
                kind: ElementKind::Text,
                transform: Affine::IDENTITY,
                bounds: Rect::new(10.0, 10.0, 190.0, 60.0),
                locked: crate::message::LockState::Unlocked,
                style,
                geom: DisplayGeom::Text(text.to_owned()),
                link: None,
                group: None,
            }));
        let mut font_cx = ambre_draw::testing::shipped_font_context();
        let bytes = export_pdf(
            &[page],
            PageSize::default(),
            Theme::Light.palette(),
            &mut font_cx,
            &mut TextCache::new(),
            &HashMap::default(),
        )
        .expect("export must succeed");
        let read = pdf_extract::extract_text_from_mem(bytes.as_bytes()).expect("the file reads");
        let compact = |t: &str| t.split_whitespace().collect::<String>();
        assert_eq!(compact(&read), compact(text));
    }

    /// An empty page list is rejected rather than producing a zero-page PDF.
    #[test]
    fn empty_pages_is_an_error() {
        let palette = Theme::Light.palette();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
        let err = export_pdf(
            &[],
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &HashMap::default(),
        )
        .unwrap_err();
        assert_eq!(err, PdfExportError::Empty);
    }

    /// `TOILE-IO-038`: the bytes of a multi-page export are a PDF document. A
    /// page count of N produces N pages; the multi-page path stays balanced
    /// (clip push/pop per page) and serializes.
    #[test]
    fn multiple_pages_serialize() {
        let palette = Theme::Light.palette();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new();
        let pages = vec![one_page(), one_page(), one_page()];
        let bytes = export_pdf(
            &pages,
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &HashMap::default(),
        )
        .expect("export must succeed");
        assert!(bytes.as_bytes().starts_with(b"%PDF"));
    }

    /// Covers `CANVAS-029`: a page is the screen's walk, unchanged, over the
    /// frame's region.
    ///
    /// Both record onto the same kind of surface, from the same faces, at the
    /// same camera. The region the walk clips to is the one thing that differs:
    /// the frame on the page, the visible board on the screen.
    #[test]
    fn the_page_and_the_screen_share_one_walk() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let palette = Theme::Light.palette();
        let page = a_page_with_text();
        let mut font_cx = ambre_draw::sealed_font_context();
        for bytes in shipped_faces() {
            ambre_draw::register_face(
                &mut font_cx,
                &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes)),
            );
        }
        let mut tc = TextCache::new();
        let images = HashMap::default();

        let mut printed = RecordingSurface::new();
        paint_page(
            &mut printed,
            &page,
            palette,
            &mut font_cx,
            &mut tc,
            (&images, &StoredPictures::default()),
        );
        let mut screen = RecordingSurface::new();
        crate::ui::render::paint_content(
            &mut screen,
            &page.camera,
            crate::ui::render::Scene {
                grid: crate::ui::grid::GridShown::Shown,
                viewport: &page.viewport,
                palette,
                images: &images,
                anim: &HashMap::default(),
            },
            BoardWalk {
                font_cx: &mut font_cx,
                text_cache: &mut tc,
            },
            None,
            crate::message::PresentMode::Presenting,
        );

        let frame = page.clip.expect("the page has a frame");
        let regions = |ops: &[DrawOp]| -> Vec<Rect> {
            ops.iter()
                .filter_map(|op| match op {
                    DrawOp::PushClip { rect } => Some(*rect),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(regions(printed.ops()), [frame]);
        assert_eq!(regions(screen.ops()), [page.camera.visible_world()]);
        let rest = |ops: &[DrawOp]| -> Vec<DrawOp> {
            ops.iter()
                .filter(|op| !matches!(op, DrawOp::PushClip { .. }))
                .cloned()
                .collect()
        };
        assert!(
            screen.ops().len() > 4,
            "the walk drew the shape and the word"
        );
        assert_eq!(rest(printed.ops()), rest(screen.ops()));
    }

    // proptest is a host dev-dependency: it pulls `wait-timeout`, which does
    // not build on wasm32. The other tests still compile for wasm.
    #[cfg(not(target_arch = "wasm32"))]
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(32))]

        /// Whatever the camera, the zoom and the canvas size, the screen
        /// draws the board's walk for the region it shows, call for call
        /// (`CANVAS-074`). The surface exposes no density, so the walk
        /// cannot read one.
        #[test]
        fn the_screen_draws_the_walk_at_any_camera(
            x in -400.0f64..600.0,
            y in -300.0f64..400.0,
            zoom in 0.1f64..8.0,
            width in 200.0f64..2000.0,
            height in 200.0f64..1400.0,
            present: bool,
        ) {
            use ambre_draw::testing::RecordingSurface;

            let palette = Theme::Light.palette();
            let page = a_page_with_text();
            let mut font_cx = ambre_draw::sealed_font_context();
            for bytes in shipped_faces() {
                ambre_draw::register_face(&mut font_cx, &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes)));
            }
            let mut tc = TextCache::new();
            let images = HashMap::default();
            let anim = HashMap::default();
            let camera = ViewportTransform {
                center: ambre_draw::Point::new(x, y),
                zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom),
                size: ambre_draw::Size::new(width, height),
            };

            let mut screen = RecordingSurface::new();
            crate::ui::render::paint_content(&mut screen, &camera, crate::ui::render::Scene { grid: crate::ui::grid::GridShown::Shown, viewport: &page.viewport, palette, images: &images, anim: &anim }, BoardWalk { font_cx: &mut font_cx, text_cache: &mut tc }, None, if present {
                    crate::message::PresentMode::Presenting
                } else {
                    crate::message::PresentMode::Editing
                });
            let view = BoardView {
                viewport: &page.viewport,
                palette,
                images: &images,
                files: &StoredPictures::default(),
                anim: &anim,
                muted_body: None,
                muted_caption: None,
            };
            let mut walked = RecordingSurface::new();
            BoardWalk {
                font_cx: &mut font_cx,
                text_cache: &mut tc,
            }
            .walk(&view, camera.visible_world(), &mut walked);
            proptest::prop_assert!(screen.holds_walk(&walked));
        }
    }

    /// A page with a shape and a word draws no image. The text is glyphs and
    /// the shape is a path, both vector (`PDF-012`). A walk that
    /// rasterized either one would record an image paint here.
    #[test]
    fn a_page_without_a_picture_paints_no_image() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let palette = Theme::Light.palette();
        let mut font_cx = ambre_draw::sealed_font_context();
        for bytes in shipped_faces() {
            ambre_draw::register_face(
                &mut font_cx,
                &ambre_draw::FontFace::new(ambre_core::Payload::new(bytes)),
            );
        }
        let mut tc = TextCache::new();
        let mut page = RecordingSurface::new();
        paint_page(
            &mut page,
            &a_page_with_text(),
            palette,
            &mut font_cx,
            &mut tc,
            (&HashMap::default(), &StoredPictures::default()),
        );
        assert!(
            page.count(|op| matches!(op, DrawOp::Glyphs { .. })).get() >= 1,
            "the word is drawn as glyphs"
        );
        assert!(
            page.count(|op| matches!(op, DrawOp::FillPath { .. } | DrawOp::FillRect { .. }))
                .get()
                >= 1,
            "the shape is drawn as a vector fill"
        );
        assert_eq!(
            page.count(|op| matches!(op, DrawOp::DrawImage { .. }))
                .get(),
            0,
            "a page with no picture rasterized something"
        );
    }
}
