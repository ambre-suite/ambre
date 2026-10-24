//! The page column and the camera over it
//! (`features/ambre-spec-prose-viewport.md section 2`).
//!
//! Pages of one size, stacked with a fixed gutter, centred across the canvas.
//! Everything here is arithmetic over the layout in force: nothing shapes,
//! nothing reads the document, and nothing is sent.

use ambre_bus::app::prose::{PageCount, PageIndex, PageSpan};

use ambre_core::{DocPx, ViewPoint, ViewPx};

use crate::layout::pass::PageLayout;

/// The gap between two pages, in document pixels.
pub const GUTTER: DocPx = DocPx::new(24.0);

/// Where the reader is looking. Local, and never sent (`PROSE-UI-010`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// From the top of the column, in document pixels.
    pub scroll_y: DocPx,
    /// The canvas, in screen pixels.
    pub width: ViewPx,
    pub height: ViewPx,
    /// One at a hundred per cent.
    pub zoom: ambre_core::ZoomFactor,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            scroll_y: DocPx::ZERO,
            width: ViewPx::ZERO,
            height: ViewPx::ZERO,
            zoom: ambre_core::ZoomFactor::ONE,
        }
    }
}

impl Viewport {
    /// The zoom a conversion divides by. The factor is positive and finite.
    /// One that an `f32` cannot hold reads as a hundred per cent, so nothing
    /// divides by zero. Clamping the zoom into a range lands with the "One
    /// zoom model" Story (`ambre-roadmap.md` E8).
    #[must_use]
    pub fn scale(&self) -> f32 {
        let zoom = self.zoom.get() as f32;
        if zoom.is_finite() && zoom > 0.0 {
            zoom
        } else {
            1.0
        }
    }

    /// How much of the column the canvas shows, in document pixels.
    #[must_use]
    pub fn visible_height(&self) -> DocPx {
        DocPx::new((self.height.get() / self.scale()).max(0.0))
    }
}

/// A point of the document, resolved to the page it falls on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DocPos {
    OnPage {
        page: PageIndex,
        x_in_page: DocPx,
        y_in_page: DocPx,
    },
    /// The layout has no page at all.
    Empty,
}

/// The geometry of the page column, rebuilt from each layout applied
/// (`PROSE-UI-131`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageColumn {
    pub page_width: DocPx,
    pub page_height: DocPx,
    pub gutter: DocPx,
    /// Zero until a layout has arrived.
    pub page_count: PageCount,
    pub total_height: DocPx,
}

impl PageColumn {
    /// The column a layout describes.
    #[must_use]
    pub fn of(layout: &PageLayout) -> Self {
        let page_height = layout.page_height.max(DocPx::ZERO);
        let count = layout.page_count.get() as f32;
        let total_height = if layout.page_count.is_zero() {
            DocPx::ZERO
        } else {
            page_height * count + GUTTER * (count - 1.0)
        };
        Self {
            page_width: layout.page_width.max(DocPx::ZERO),
            page_height,
            gutter: GUTTER,
            page_count: layout.page_count,
            total_height,
        }
    }

    fn stride(&self) -> DocPx {
        self.page_height + self.gutter
    }

    /// Where a page starts, from the top of the column.
    #[must_use]
    pub fn page_top_y(&self, page: PageIndex) -> DocPx {
        self.stride() * page.get() as f32
    }

    /// The last offset the column scrolls to: its bottom at the canvas's.
    #[must_use]
    pub fn max_scroll(&self, viewport: &Viewport) -> DocPx {
        (self.total_height - viewport.visible_height()).max(DocPx::ZERO)
    }

    /// An offset brought inside the column (`PROSE-UI-025`). One that is not a
    /// number goes back to the top.
    #[must_use]
    pub fn clamp_scroll(&self, viewport: &Viewport, scroll_y: DocPx) -> DocPx {
        if scroll_y.get().is_nan() {
            return DocPx::ZERO;
        }
        scroll_y.clamp(DocPx::ZERO, self.max_scroll(viewport))
    }

    /// The page a column offset belongs to, and the offset within it.
    ///
    /// An offset in a gutter goes to the nearer of the two page edges
    /// (`PROSE-UI-124`). One above or below the column goes to the edge of the
    /// first or the last page. `None` only for a layout with no page
    /// (`PROSE-UI-022`).
    #[must_use]
    pub fn doc_y_to_page(&self, doc_y: DocPx) -> Option<(PageIndex, DocPx)> {
        let last = self.page_count.last()?;
        let stride = self.stride();
        if doc_y.get().is_nan() || doc_y <= DocPx::ZERO || stride <= DocPx::ZERO {
            return Some((PageIndex::ZERO, DocPx::ZERO));
        }
        let index = (doc_y / stride).floor();
        if index >= last.get() as f32 {
            let within = (doc_y - self.page_top_y(last)).min(self.page_height);
            return Some((last, within));
        }
        // `index` is below `last`, so it fits a page index.
        let page = PageIndex::new(index as u32);
        let within = doc_y - self.page_top_y(page);
        if within <= self.page_height {
            return Some((page, within));
        }
        let past_bottom = within - self.page_height;
        if past_bottom * 2.0 <= self.gutter {
            Some((page, self.page_height))
        } else {
            Some((page.next(), DocPx::ZERO))
        }
    }

    /// The pages the canvas shows, first to last. `None` for a layout with no
    /// page; never inverted, and never a page the layout lacks
    /// (`PROSE-UI-021`, `PROSE-UI-123`).
    #[must_use]
    pub fn visible_pages(&self, viewport: &Viewport) -> Option<PageSpan> {
        let top = viewport.scroll_y;
        let (first, _) = self.doc_y_to_page(top)?;
        let (last, _) = self.doc_y_to_page(top + viewport.visible_height())?;
        Some(first.min(last).through(last.max(first)))
    }

    /// The pages a frame builds: the visible ones and one more on each side,
    /// and never the whole document (`PROSE-UI-007`).
    #[must_use]
    pub fn built_pages(&self, viewport: &Viewport) -> Option<PageSpan> {
        let visible = self.visible_pages(viewport)?;
        let last = self.page_count.last()?;
        let first = visible.start - PageCount::ONE;
        let end = visible.last()?.next().min(last);
        Some(first.through(end))
    }

    /// The left edge of the column on the canvas, in screen pixels. A page
    /// wider than the canvas pins the column to the left edge.
    #[must_use]
    pub fn left(&self, viewport: &Viewport) -> ViewPx {
        ViewPx::new(
            ((viewport.width.get() - self.page_width.get() * viewport.scale()) / 2.0).max(0.0),
        )
    }

    /// A canvas point as a point of the document.
    #[must_use]
    pub fn screen_to_doc(&self, viewport: &Viewport, at: ViewPoint) -> DocPos {
        let scale = viewport.scale();
        let doc_y = DocPx::new(at.y.get() / scale) + viewport.scroll_y;
        match self.doc_y_to_page(doc_y) {
            Some((page, y_in_page)) => DocPos::OnPage {
                page,
                x_in_page: DocPx::new((at.x - self.left(viewport)).get() / scale),

                y_in_page,
            },
            None => DocPos::Empty,
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn point(x: f32, y: f32) -> ViewPoint {
        ViewPoint::new(ambre_core::ViewPx::new(x), ambre_core::ViewPx::new(y))
    }

    fn column(pages: u32) -> PageColumn {
        PageColumn::of(&PageLayout {
            page_width: DocPx::new(800.0),
            page_height: DocPx::new(1000.0),
            page_count: PageCount::new(pages),
            ..PageLayout::default()
        })
    }

    fn viewport(scroll_y: f32, height_px: f32, zoom: f32) -> Viewport {
        Viewport {
            scroll_y: DocPx::new(scroll_y),
            width: ViewPx::new(1200.0),
            height: ViewPx::new(height_px),
            zoom: ambre_core::ZoomFactor::positive(f64::from(zoom)),
        }
    }

    proptest! {
        #[test]
        fn the_visible_range_is_never_inverted_and_names_only_pages_the_layout_has(
            pages in 0u32..40,
            scroll in -5000.0f32..60000.0,
            height in 0.0f32..4000.0,
            zoom in 0.1f32..5.0,
        ) {
            let column = column(pages);
            let range = column.visible_pages(&viewport(scroll, height, zoom));
            match range {
                None => prop_assert_eq!(pages, 0),
                Some(range) => {
                    prop_assert!(!range.is_empty());
                    prop_assert!(range.end.get() <= pages);
                }
            }
        }

        #[test]
        fn every_position_resolves_to_a_page_unless_there_is_none(
            pages in 0u32..40,
            doc_y in -5000.0f32..60000.0,
        ) {
            let column = column(pages);
            match column.doc_y_to_page(DocPx::new(doc_y)) {
                None => prop_assert_eq!(pages, 0),
                Some((page, within)) => {
                    prop_assert!(page.get() < pages);
                    prop_assert!((0.0..=column.page_height.get()).contains(&within.get()));
                }
            }
        }

        #[test]
        fn a_frame_builds_the_visible_pages_and_one_more_on_each_side(
            pages in 1u32..200,
            scroll in 0.0f32..300000.0,
            height in 0.0f32..3000.0,
        ) {
            let column = column(pages);
            let view = viewport(scroll, height, 1.0);
            let visible = column.visible_pages(&view).expect("a page");
            let built = column.built_pages(&view).expect("a page");
            prop_assert_eq!(built.start, visible.start - PageCount::ONE);
            prop_assert_eq!(
                built.last().expect("a page").get(),
                (visible.last().expect("a page").get() + 1).min(pages - 1)
            );
            // A canvas of at most three pages builds at most five, whatever
            // the length of the document.
            prop_assert!(built.len().get() <= 6);
        }

        #[test]
        fn a_clamped_offset_stays_inside_the_column(
            pages in 0u32..40,
            scroll in -1.0e7f32..1.0e7,
            height in 0.0f32..4000.0,
        ) {
            let column = column(pages);
            let view = viewport(0.0, height, 1.0);
            let clamped = column.clamp_scroll(&view, DocPx::new(scroll));
            prop_assert!(clamped >= DocPx::new(0.0));
            prop_assert!(clamped <= column.max_scroll(&view));
        }
    }

    #[test]
    fn a_position_in_a_gutter_goes_to_the_nearer_page_edge() {
        let column = column(3);
        let bottom = column.page_height;
        assert_eq!(
            column.doc_y_to_page(bottom + DocPx::new(5.0)),
            Some((PageIndex::ZERO, bottom))
        );
        let next = column.page_top_y(PageIndex::new(1)).get();
        assert_eq!(
            column.doc_y_to_page(DocPx::new(next - 5.0)),
            Some((PageIndex::new(1), DocPx::ZERO))
        );
    }

    #[test]
    fn a_layout_with_no_page_resolves_nothing() {
        let column = column(0);
        assert_eq!(column.total_height, DocPx::new(0.0));
        assert_eq!(column.doc_y_to_page(DocPx::new(10.0)), None);
        assert_eq!(
            column.screen_to_doc(&viewport(0.0, 600.0, 1.0), point(10.0, 10.0)),
            DocPos::Empty
        );
    }

    #[test]
    fn a_screen_point_lands_on_the_page_drawn_under_it() {
        let column = column(3);
        let view = viewport(column.page_top_y(PageIndex::new(1)).get(), 800.0, 2.0);
        let left = column.left(&view);
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = column.screen_to_doc(&view, point(left.get() + 100.0, 60.0))
        else {
            panic!("a page is under the point");
        };
        assert_eq!(page, PageIndex::new(1));
        assert!((x_in_page - DocPx::new(50.0)).abs() < DocPx::new(1e-3));
        assert!((y_in_page - DocPx::new(30.0)).abs() < DocPx::new(1e-3));
    }
}
