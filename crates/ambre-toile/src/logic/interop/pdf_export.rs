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

use std::collections::HashMap;

use ambre_canvas::{Palette, TextCache};
use ambre_convert::{Conversion, ConvertError, Converted, Converter};
use ambre_core::{PrintLength, Pt};
use ambre_draw::list::{ListRecorder, PrintDocument, PrintPage, RecorderTables};
use ambre_draw::{DocumentWalk, DrawSurface, FontContext, Rect};

use crate::logic::command::DisplayViewport;
use crate::ui::grid;
use crate::ui::viewport::ViewportTransform;
use crate::ui::walk::{BoardView, BoardWalk, DecodedImage};

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
) -> Result<ambre_core::Payload<ambre_core::FileBytes>, PdfExportError> {
    let document = record_pdf(pages, page, palette, font_cx, text_cache, images)?;
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
    for pdf_page in pages {
        let mut recorder = ListRecorder::new(&mut document, &mut tables);
        paint_page(
            &mut recorder,
            pdf_page,
            palette,
            font_cx,
            text_cache,
            images,
        );
        recorder.finish();
    }
    Ok(document)
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
    images: &HashMap<ambre_core::Validated<ambre_core::ContentHash>, DecodedImage>,
) {
    grid::paint_background(s, &pdf_page.camera, palette);
    // The region is the frame, so the page carries only slide content. The
    // fallback page of an empty deck has no frame and shows what its camera
    // sees.
    let region = pdf_page
        .clip
        .unwrap_or_else(|| pdf_page.camera.visible_world());
    // Nothing is under inline edit off-screen, and no Smart Animate
    // transition runs on export.
    let anim = HashMap::new();
    let view = BoardView {
        viewport: &pdf_page.viewport,
        palette,
        images,
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
    ) -> Result<ambre_core::Payload<ambre_core::FileBytes>, PdfExportError> {
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
            &HashMap::new(),
        )
        .expect("export must succeed");
        assert!(
            bytes.as_bytes().starts_with(b"%PDF"),
            "output must be a PDF"
        );
        assert!(
            bytes.size().get() > 400,
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
                &HashMap::new(),
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
                ambre_draw::register_face(&mut font_cx, ambre_core::Payload::new(bytes.clone()));
            }
            let mut tc = TextCache::new();
            export_pdf(
                &[a_page_with_text()],
                PageSize::default(),
                palette,
                &mut font_cx,
                &mut tc,
                &HashMap::new(),
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
            &HashMap::new(),
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
            &HashMap::new(),
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
            &HashMap::new(),
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
            ambre_draw::register_face(&mut font_cx, ambre_core::Payload::new(bytes));
        }
        let mut tc = TextCache::new();
        let images = HashMap::new();

        let mut printed = RecordingSurface::new();
        paint_page(&mut printed, &page, palette, &mut font_cx, &mut tc, &images);
        let mut screen = RecordingSurface::new();
        crate::ui::render::paint_content(
            &mut screen,
            &page.camera,
            crate::ui::render::Scene {
                viewport: &page.viewport,
                palette,
                images: &images,
                anim: &HashMap::new(),
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
                ambre_draw::register_face(&mut font_cx, ambre_core::Payload::new(bytes));
            }
            let mut tc = TextCache::new();
            let images = HashMap::new();
            let anim = HashMap::new();
            let camera = ViewportTransform {
                center: ambre_draw::Point::new(x, y),
                zoom: ambre_core::Scale::<ambre_core::Board, ambre_core::View>::positive(zoom),
                size: ambre_draw::Size::new(width, height),
            };

            let mut screen = RecordingSurface::new();
            crate::ui::render::paint_content(&mut screen, &camera, crate::ui::render::Scene { viewport: &page.viewport, palette, images: &images, anim: &anim }, BoardWalk { font_cx: &mut font_cx, text_cache: &mut tc }, None, if present {
                    crate::message::PresentMode::Presenting
                } else {
                    crate::message::PresentMode::Editing
                });
            let view = BoardView {
                viewport: &page.viewport,
                palette,
                images: &images,
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
            ambre_draw::register_face(&mut font_cx, ambre_core::Payload::new(bytes));
        }
        let mut tc = TextCache::new();
        let mut page = RecordingSurface::new();
        paint_page(
            &mut page,
            &a_page_with_text(),
            palette,
            &mut font_cx,
            &mut tc,
            &HashMap::new(),
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
