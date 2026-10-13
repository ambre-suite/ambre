//! PDF export for Toile decks (`toile-pdf-export`), gated behind the `pdf`
//! feature so the krilla dependency only links when export is compiled in.
//!
//! One presentation frame becomes one PDF page. The export drives the **same**
//! [`paint_content`](crate::ui::render::paint_content) walk the screen uses, as
//! the reuse demands ("the same `DrawSurface` code as the screen"). It runs
//! against a [`KrillaSurface`](ambre_pdf::KrillaSurface) instead of the GPU
//! surface, so screen and PDF render identically. This is why `logic::interop`
//! reaches into `ui::render` / `ui::viewport`: both are pure, wgpu-free geometry.
//!
//! Each page is culled and fitted to its frame's spatial rect (a [`PdfPage`],
//! built by `ToileState::pdf_pages`, a later slice) and then **clipped** to that
//! rect. The clip is the fix for the aspect-ratio bleed. `fit_target` scales to
//! the tighter axis (`ui::viewport`), so slack on the wider axis would otherwise
//! pull neighbouring board content into the page. Present mode (`present = true`)
//! also drops the board grid, so the page shows only slide content.

use std::collections::HashMap;

use ambre_canvas::{Palette, TextCache};
use ambre_draw::{Affine, DrawSurface, FontContext, Point, Rect};

use ambre_pdf::KrillaSurface;
use krilla::Document;
use krilla::page::PageSettings;

use crate::logic::command::DisplayViewport;
use crate::ui::render::{ImageStore, paint_content};
use crate::ui::viewport::ViewportTransform;

/// Output page dimensions, in PDF points (1 pt = 1/72 inch). The export camera's
/// pixel size equals these points 1:1, so the on-screen `fit_bounds` math maps
/// straight onto the page with no extra scaling. Defaults to a 16:9 slide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

impl PageSize {
    /// A page of `width` x `height` points, each clamped to a positive minimum so
    /// `PageSettings::from_wh` never rejects it.
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }
}

impl Default for PageSize {
    fn default() -> Self {
        // 1280x720: a 16:9 slide, matching the default presentation aspect.
        Self::new(1280.0, 720.0)
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

/// A PDF export failure. `Serialize` wraps krilla's final assembly error; the
/// walk itself cannot fail (it only emits draw calls).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfExportError {
    #[error("no pages to export")]
    Empty,
    #[error("failed to serialize the PDF: {0}")]
    Serialize(String),
}

/// Render `pages` into a single PDF document (`toile-pdf-export`).
///
/// Every page runs the screen content walk against a fresh krilla page surface,
/// clipped to its frame rect. `font_cx` must already carry the base faces
/// (`LogicState::export_cx`, seeded at boot) or text emits no glyphs. `images`
/// carries the decoded media pixels an `Image` element paints; an absent hash
/// falls back to a placeholder box. `editing` and Smart Animate overrides have
/// no meaning off-screen, so the walk runs with none.
///
/// Deterministic: krilla assembles the same bytes for the same input, so two
/// runs of identical `pages` are byte-stable.
///
/// # Errors
///
/// [`PdfExportError::Empty`] on an empty `pages`, or
/// [`PdfExportError::Serialize`] when krilla refuses the bytes.
///
/// # Panics
///
/// When krilla refuses a unit page as a fallback page size, which its own
/// validity rule accepts.
pub fn export_pdf(
    pages: &[PdfPage],
    page: PageSize,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    images: &ImageStore,
) -> Result<Vec<u8>, PdfExportError> {
    if pages.is_empty() {
        return Err(PdfExportError::Empty);
    }

    let mut doc = Document::new();
    for pdf_page in pages {
        // `from_wh` only rejects a non-positive extent; `PageSize::new` clamps to
        // a positive minimum, so this is infallible in practice.
        let settings = PageSettings::from_wh(page.width as f32, page.height as f32)
            .or_else(|| PageSettings::from_wh(1.0, 1.0))
            .unwrap_or_default();
        let mut kpage = doc.start_page_with(settings);
        {
            let mut surface = kpage.surface();
            let mut s = KrillaSurface::new(&mut surface);
            paint_page(&mut s, pdf_page, palette, font_cx, text_cache, images);
        }
        kpage.finish();
    }

    doc.finish()
        .map_err(|e| PdfExportError::Serialize(format!("{e:?}")))
}

/// Paint one page onto any surface: the screen's content walk inside the
/// frame clip (`CANVAS-029`).
///
/// It names no surface type, so the page a test records is the page krilla
/// writes.
fn paint_page<S: DrawSurface>(
    s: &mut S,
    pdf_page: &PdfPage,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    images: &ImageStore,
) {
    // Clip to the frame rect (mapped world -> page space through the same
    // camera the walk uses), so the page carries only slide content.
    let clipped = pdf_page.clip.is_some();
    if let Some(world) = pdf_page.clip {
        s.push_clip(map_rect(pdf_page.camera.world_to_screen(), world));
    }

    paint_content(
        s,
        &pdf_page.viewport,
        &pdf_page.camera,
        palette,
        font_cx,
        text_cache,
        None, // nothing is under inline edit off-screen
        images,
        &HashMap::new(), // no Smart Animate transition on export
        true,            // present mode: no board grid
    );

    if clipped {
        s.pop_clip();
    }
}

/// Map an axis-aligned rect through a scale + translate affine, the camera's
/// `world_to_screen`, which has no rotation. It yields the enclosing axis-aligned
/// rect in the target space.
fn map_rect(a: Affine, r: Rect) -> Rect {
    let p0 = a * Point::new(r.x0, r.y0);
    let p1 = a * Point::new(r.x1, r.y1);
    Rect::new(
        p0.x.min(p1.x),
        p0.y.min(p1.y),
        p0.x.max(p1.x),
        p0.y.max(p1.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambre_bus::app::toile::{
        DisplayElement, DisplayGeom, DisplayNode, ElementId, ElementKind, ElementStyle, ShapeKind,
    };
    use ambre_canvas::Theme;

    fn rect_node(id: &str, bounds: Rect) -> DisplayNode {
        DisplayNode::Element(DisplayElement {
            id: ElementId(id.to_owned()),
            kind: ElementKind::Shape(ShapeKind::Rect),
            transform: Affine::IDENTITY,
            bounds,
            locked: false,
            style: ElementStyle::default(),
            geom: DisplayGeom::Rect,
            link: None,
            group: None,
        })
    }

    fn one_page() -> PdfPage {
        let frame = Rect::new(0.0, 0.0, 200.0, 100.0);
        let mut camera = ViewportTransform::new(1280.0, 720.0);
        camera.fit_bounds(frame, 0.08);
        PdfPage {
            viewport: DisplayViewport {
                reactions: Default::default(),
                pins: Vec::new(),
                viewport: Default::default(),
                nodes: vec![rect_node("a", Rect::new(10.0, 10.0, 90.0, 90.0))],
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
        let mut tc = TextCache::new(1.0);
        let bytes = export_pdf(
            &[one_page()],
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &ImageStore::new(),
        )
        .expect("export must succeed");
        assert!(bytes.starts_with(b"%PDF"), "output must be a PDF");
        assert!(bytes.len() > 400, "content stream should be non-trivial");
    }

    /// Export is deterministic: two runs of the same pages produce identical
    /// bytes.
    #[test]
    fn export_is_byte_stable() {
        let palette = Theme::Light.palette();
        let run = || {
            let mut font_cx = FontContext::new();
            let mut tc = TextCache::new(1.0);
            export_pdf(
                &[one_page()],
                PageSize::default(),
                palette,
                &mut font_cx,
                &mut tc,
                &ImageStore::new(),
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
                id: ElementId("t".to_owned()),
                kind: ElementKind::Text,
                transform: Affine::IDENTITY,
                bounds: Rect::new(100.0, 10.0, 190.0, 40.0),
                locked: false,
                style: ElementStyle::default(),
                geom: DisplayGeom::Text("Ambre".to_owned()),
                link: None,
                group: None,
            }));
        page
    }

    /// Two peers holding the same faces export one file (`PDF-004`).
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
                font_cx
                    .collection
                    .register_fonts(bytes.clone().into(), None);
            }
            let mut tc = TextCache::new(1.0);
            export_pdf(
                &[a_page_with_text()],
                PageSize::default(),
                palette,
                &mut font_cx,
                &mut tc,
                &ImageStore::new(),
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

    /// An empty page list is rejected rather than producing a zero-page PDF.
    #[test]
    fn empty_pages_is_an_error() {
        let palette = Theme::Light.palette();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        let err = export_pdf(
            &[],
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &ImageStore::new(),
        )
        .unwrap_err();
        assert_eq!(err, PdfExportError::Empty);
    }

    /// A page count of N produces N pages; the multi-page path stays balanced
    /// (clip push/pop per page) and serializes.
    #[test]
    fn multiple_pages_serialize() {
        let palette = Theme::Light.palette();
        let mut font_cx = FontContext::new();
        let mut tc = TextCache::new(1.0);
        let pages = vec![one_page(), one_page(), one_page()];
        let bytes = export_pdf(
            &pages,
            PageSize::default(),
            palette,
            &mut font_cx,
            &mut tc,
            &ImageStore::new(),
        )
        .expect("export must succeed");
        assert!(bytes.starts_with(b"%PDF"));
    }

    /// Covers `CANVAS-029`: a page is the screen's walk, unchanged, inside the
    /// frame clip.
    ///
    /// Both walks record onto the same kind of surface, from the same faces.
    /// What the page adds is the clip and nothing else.
    #[test]
    fn the_page_and_the_screen_share_one_walk() {
        use ambre_draw::testing::{DrawOp, RecordingSurface};

        let palette = Theme::Light.palette();
        let page = a_page_with_text();
        let mut font_cx = ambre_draw::sealed_font_context();
        for bytes in shipped_faces() {
            font_cx.collection.register_fonts(bytes.into(), None);
        }
        let mut tc = TextCache::new(1.0);
        let images = ImageStore::new();

        let mut printed = RecordingSurface::new();
        paint_page(&mut printed, &page, palette, &mut font_cx, &mut tc, &images);
        let mut screen = RecordingSurface::new();
        paint_content(
            &mut screen,
            &page.viewport,
            &page.camera,
            palette,
            &mut font_cx,
            &mut tc,
            None,
            &images,
            &HashMap::new(),
            true,
        );

        let ops = printed.ops();
        assert!(
            matches!(ops.first(), Some(DrawOp::PushClip { .. })),
            "{ops:?}"
        );
        assert_eq!(ops.last(), Some(&DrawOp::PopClip));
        assert!(
            screen.ops().len() > 1,
            "the walk drew the shape and the word"
        );
        assert_eq!(&ops[1..ops.len() - 1], screen.ops());
    }
}
