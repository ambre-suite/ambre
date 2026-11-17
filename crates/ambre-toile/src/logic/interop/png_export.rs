//! The PNG export of a board or of one frame (`toile-import-export`).
//!
//! The Logic thread derives one page. It records the page through the
//! screen's own walk in one step, as the PDF export records a page
//! (`TOILE-IO-099`). The recording is a [`PrintDocument`] that names no
//! writer. The PDF module rasterizes and encodes it on the converter thread.
//! Neither the UI thread nor the Logic thread waits for a pixel
//! (`TOILE-IO-017`, `TOILE-IO-097`).
//!
//! The page camera maps the board onto raster pixels at [`PNG_ZOOM`], capped
//! so the longest edge stays within [`ambre_convert::RASTER_EDGE`]. The page
//! points then equal the raster pixels, and the module fills the raster with
//! the page one for one.

use ambre_core::{Board, Image, Px, Scale, Size, View};
use ambre_draw::Rect;
use ambre_draw::list::PrintDocument;

use crate::logic::command::DisplayViewport;
use crate::message::Viewport;

use super::pdf_export::{PageSize, PdfPage};

/// The raster pixels a PNG export gives one board unit: twice the screen at
/// 100%, so the image stays sharp on a dense display.
pub const PNG_ZOOM: Scale<Board, View> = Scale::<Board, View>::positive(2.0);

/// The board units a board export keeps around its content, so a stroke on
/// the outer edge is not cut.
const BOARD_MARGIN: Px<Board> = Px::new(16.0);

/// What a PNG export rasterizes, read off the board.
#[derive(Debug, Clone)]
pub struct PngPage {
    /// The page, derived at the raster camera, and clipped to a frame.
    pub page: PdfPage,
    /// The page size in points, which equal the raster pixels.
    pub points: PageSize,
    /// The raster size the module writes.
    pub size: Size<Px<Image>>,
}

/// Why a board yields no PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PngExportError {
    /// The board holds no element.
    #[error("the board holds nothing to export")]
    Empty,
    /// The element asked for is not a frame on the board.
    #[error("no such frame on the board")]
    NoFrame,
}

/// The request the converter thread runs: the recorded page and the raster
/// size.
#[derive(Debug, Clone)]
pub struct PngRequest {
    pub document: PrintDocument,
    pub size: Size<Px<Image>>,
}

/// The page that shows `region` of the board, clipped to `clip`. `derive`
/// culls the board at the page camera. `None` for a region with no area or
/// a corner that is no number.
pub(crate) fn png_page(
    region: Rect,
    clip: Option<Rect>,
    derive: impl FnOnce(Viewport) -> DisplayViewport,
) -> Option<PngPage> {
    let (width, height) = (region.width(), region.height());
    if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let edge = f64::from(ambre_convert::RASTER_EDGE.get());
    let zoom = PNG_ZOOM.get().min(edge / width.max(height));
    let zoom = Scale::<Board, View>::new(zoom)?;
    // Each edge rounds up, so the page holds the whole region, and stays
    // within the bound the zoom was capped by.
    let pixels = |length: f64| {
        let pixels = (length * zoom.get()).ceil().clamp(1.0, edge);
        Px::<Image>::new(u32::try_from(pixels as i64).unwrap_or(1))
    };
    let size = Size::new(pixels(width), pixels(height));
    let points = PageSize::new(
        ambre_core::PrintLength::<ambre_core::Pt>::new(size.width.get() as f32),
        ambre_core::PrintLength::<ambre_core::Pt>::new(size.height.get() as f32),
    );
    Some(PngPage {
        page: PdfPage::framed(
            region.center(),
            zoom,
            ambre_draw::Size::new(f64::from(size.width.get()), f64::from(size.height.get())),
            clip,
            derive,
        ),
        points,
        size,
    })
}

/// The region a board export shows: its content and a margin around it.
pub(crate) fn board_region(content: Rect) -> Rect {
    content.inflate(BOARD_MARGIN.get(), BOARD_MARGIN.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty(_: Viewport) -> DisplayViewport {
        DisplayViewport {
            reactions: Default::default(),
            pins: Vec::new(),
            viewport: Default::default(),
            nodes: Vec::new(),
            version: ambre_core::Seq::ZERO,
            grid_unit: Px::<Board>::new(20.0),
        }
    }

    /// `TOILE-IO-186`: a region rasterizes at twice its board size, and the
    /// camera maps its corners onto the corners of the raster.
    #[test]
    fn a_region_rasterizes_at_twice_its_size_corner_to_corner() {
        let region = Rect::new(-50.0, 10.0, 150.0, 110.0);
        let page = png_page(region, None, empty).expect("the region has an area");
        assert_eq!(page.size, Size::new(Px::new(400), Px::new(200)));
        let to_screen = page.page.camera.world_to_screen();
        let origin = to_screen * kurbo::Point::new(region.x0, region.y0);
        let corner = to_screen * kurbo::Point::new(region.x1, region.y1);
        assert!(origin.distance(kurbo::Point::ZERO) < 1e-9, "{origin:?}");
        assert!(
            corner.distance(kurbo::Point::new(400.0, 200.0)) < 1e-9,
            "{corner:?}"
        );
    }

    /// `TOILE-IO-187`: a region whose doubled edge passes the bound is scaled
    /// down until its longest edge meets it, and keeps its aspect.
    #[test]
    fn a_large_region_is_scaled_down_to_the_bound() {
        let region = Rect::new(0.0, 0.0, 10_000.0, 2_500.0);
        let page = png_page(region, None, empty).expect("the region has an area");
        assert_eq!(page.size.width, ambre_convert::RASTER_EDGE);
        assert_eq!(page.size.height, Px::new(1024));
    }

    #[test]
    fn a_region_with_no_area_yields_no_page() {
        assert!(png_page(Rect::new(5.0, 5.0, 5.0, 9.0), None, empty).is_none());
    }
}
