//! The page column and the camera over it
//! (`features/ambre-spec-prose-viewport.md section 2`).
//!
//! Pages of one size, stacked with a fixed gutter, centred across the canvas.
//! Everything here is arithmetic over the layout in force: nothing shapes,
//! nothing reads the document, and nothing is sent.

use std::ops::RangeInclusive;

use crate::layout::pass::PageLayout;

/// The gap between two pages, in document pixels.
pub const GUTTER_PX: f32 = 24.0;

/// Where the reader is looking. Local, and never sent (`PROSE-UI-010`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// From the top of the column, in document pixels.
    pub scroll_y: f32,
    /// The canvas, in screen pixels.
    pub width_px: f32,
    pub height_px: f32,
    /// One at a hundred per cent.
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            scroll_y: 0.0,
            width_px: 0.0,
            height_px: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// The zoom a conversion divides by. A zoom of zero or less, or one that
    /// is not a number, reads as a hundred per cent rather than dividing by
    /// nothing. Clamping the zoom itself lands with the "One zoom model"
    /// Story (`ambre-roadmap.md` E8).
    #[must_use]
    pub fn scale(&self) -> f32 {
        if self.zoom.is_finite() && self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        }
    }

    /// How much of the column the canvas shows, in document pixels.
    #[must_use]
    pub fn visible_height(&self) -> f32 {
        (self.height_px / self.scale()).max(0.0)
    }
}

/// A point of the document, resolved to the page it falls on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DocPos {
    OnPage {
        page: u32,
        x_in_page: f32,
        y_in_page: f32,
    },
    /// The layout has no page at all.
    Empty,
}

/// The geometry of the page column, rebuilt from each layout applied
/// (`PROSE-UI-131`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageColumn {
    pub page_width_px: f32,
    pub page_height_px: f32,
    pub gutter_px: f32,
    /// Zero until a layout has arrived.
    pub page_count: u32,
    pub total_height_px: f32,
}

impl PageColumn {
    /// The column a layout describes.
    #[must_use]
    pub fn of(layout: &PageLayout) -> Self {
        let page_height_px = layout.page_height_px.max(0.0);
        let count = layout.page_count as f32;
        let total_height_px = if layout.page_count == 0 {
            0.0
        } else {
            count * page_height_px + (count - 1.0) * GUTTER_PX
        };
        Self {
            page_width_px: layout.page_width_px.max(0.0),
            page_height_px,
            gutter_px: GUTTER_PX,
            page_count: layout.page_count,
            total_height_px,
        }
    }

    fn stride(&self) -> f32 {
        self.page_height_px + self.gutter_px
    }

    /// Where a page starts, from the top of the column.
    #[must_use]
    pub fn page_top_y(&self, page: u32) -> f32 {
        page as f32 * self.stride()
    }

    /// The last offset the column scrolls to: its bottom at the canvas's.
    #[must_use]
    pub fn max_scroll(&self, viewport: &Viewport) -> f32 {
        (self.total_height_px - viewport.visible_height()).max(0.0)
    }

    /// An offset brought inside the column (`PROSE-UI-025`). One that is not a
    /// number goes back to the top.
    #[must_use]
    pub fn clamp_scroll(&self, viewport: &Viewport, scroll_y: f32) -> f32 {
        if scroll_y.is_nan() {
            return 0.0;
        }
        scroll_y.clamp(0.0, self.max_scroll(viewport))
    }

    /// The page a column offset belongs to, and the offset within it.
    ///
    /// An offset in a gutter goes to the nearer of the two page edges
    /// (`PROSE-UI-124`). One above or below the column goes to the edge of the
    /// first or the last page. `None` only for a layout with no page
    /// (`PROSE-UI-022`).
    #[must_use]
    pub fn doc_y_to_page(&self, doc_y: f32) -> Option<(u32, f32)> {
        let last = self.page_count.checked_sub(1)?;
        let stride = self.stride();
        if doc_y.is_nan() || doc_y <= 0.0 || stride <= 0.0 {
            return Some((0, 0.0));
        }
        let index = (doc_y / stride).floor();
        if index >= last as f32 {
            let within = (doc_y - self.page_top_y(last)).min(self.page_height_px);
            return Some((last, within));
        }
        // `index` is below `last`, so it fits a page index.
        let page = index as u32;
        let within = doc_y - self.page_top_y(page);
        if within <= self.page_height_px {
            return Some((page, within));
        }
        let past_bottom = within - self.page_height_px;
        if past_bottom * 2.0 <= self.gutter_px {
            Some((page, self.page_height_px))
        } else {
            Some((page + 1, 0.0))
        }
    }

    /// The pages the canvas shows, first to last. `None` for a layout with no
    /// page; never inverted, and never a page the layout lacks
    /// (`PROSE-UI-021`, `PROSE-UI-123`).
    #[must_use]
    pub fn visible_pages(&self, viewport: &Viewport) -> Option<RangeInclusive<u32>> {
        let top = viewport.scroll_y;
        let (first, _) = self.doc_y_to_page(top)?;
        let (last, _) = self.doc_y_to_page(top + viewport.visible_height())?;
        Some(first.min(last)..=last.max(first))
    }

    /// The pages a frame builds: the visible ones and one more on each side,
    /// and never the whole document (`PROSE-UI-007`).
    #[must_use]
    pub fn built_pages(&self, viewport: &Viewport) -> Option<RangeInclusive<u32>> {
        let visible = self.visible_pages(viewport)?;
        let last = self.page_count.saturating_sub(1);
        let first = visible.start().saturating_sub(1);
        let end = visible.end().saturating_add(1).min(last);
        Some(first..=end)
    }

    /// The left edge of the column on the canvas, in screen pixels. A page
    /// wider than the canvas pins the column to the left edge.
    #[must_use]
    pub fn left_px(&self, viewport: &Viewport) -> f32 {
        ((viewport.width_px - self.page_width_px * viewport.scale()) / 2.0).max(0.0)
    }

    /// A canvas point as a point of the document.
    #[must_use]
    pub fn screen_to_doc(&self, viewport: &Viewport, x: f32, y: f32) -> DocPos {
        let scale = viewport.scale();
        let doc_y = y / scale + viewport.scroll_y;
        match self.doc_y_to_page(doc_y) {
            Some((page, y_in_page)) => DocPos::OnPage {
                page,
                x_in_page: (x - self.left_px(viewport)) / scale,
                y_in_page,
            },
            None => DocPos::Empty,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn column(pages: u32) -> PageColumn {
        PageColumn::of(&PageLayout {
            page_width_px: 800.0,
            page_height_px: 1000.0,
            page_count: pages,
            ..PageLayout::default()
        })
    }

    fn viewport(scroll_y: f32, height_px: f32, zoom: f32) -> Viewport {
        Viewport {
            scroll_y,
            width_px: 1200.0,
            height_px,
            zoom,
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
                    prop_assert!(range.start() <= range.end());
                    prop_assert!(*range.end() < pages);
                }
            }
        }

        #[test]
        fn every_position_resolves_to_a_page_unless_there_is_none(
            pages in 0u32..40,
            doc_y in -5000.0f32..60000.0,
        ) {
            let column = column(pages);
            match column.doc_y_to_page(doc_y) {
                None => prop_assert_eq!(pages, 0),
                Some((page, within)) => {
                    prop_assert!(page < pages);
                    prop_assert!((0.0..=column.page_height_px).contains(&within));
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
            prop_assert_eq!(*built.start(), visible.start().saturating_sub(1));
            prop_assert_eq!(*built.end(), (visible.end() + 1).min(pages - 1));
            // A canvas of at most three pages builds at most five, whatever
            // the length of the document.
            prop_assert!(built.end() - built.start() <= 5);
        }

        #[test]
        fn a_clamped_offset_stays_inside_the_column(
            pages in 0u32..40,
            scroll in -1.0e7f32..1.0e7,
            height in 0.0f32..4000.0,
        ) {
            let column = column(pages);
            let view = viewport(0.0, height, 1.0);
            let clamped = column.clamp_scroll(&view, scroll);
            prop_assert!(clamped >= 0.0);
            prop_assert!(clamped <= column.max_scroll(&view));
        }
    }

    #[test]
    fn a_position_in_a_gutter_goes_to_the_nearer_page_edge() {
        let column = column(3);
        let bottom = column.page_height_px;
        assert_eq!(column.doc_y_to_page(bottom + 5.0), Some((0, bottom)));
        let next = column.page_top_y(1);
        assert_eq!(column.doc_y_to_page(next - 5.0), Some((1, 0.0)));
    }

    #[test]
    fn a_layout_with_no_page_resolves_nothing() {
        let column = column(0);
        assert_eq!(column.total_height_px, 0.0);
        assert_eq!(column.doc_y_to_page(10.0), None);
        assert_eq!(
            column.screen_to_doc(&viewport(0.0, 600.0, 1.0), 10.0, 10.0),
            DocPos::Empty
        );
    }

    #[test]
    fn a_screen_point_lands_on_the_page_drawn_under_it() {
        let column = column(3);
        let view = viewport(column.page_top_y(1), 800.0, 2.0);
        let left = column.left_px(&view);
        let DocPos::OnPage {
            page,
            x_in_page,
            y_in_page,
        } = column.screen_to_doc(&view, left + 100.0, 60.0)
        else {
            panic!("a page is under the point");
        };
        assert_eq!(page, 1);
        assert!((x_in_page - 50.0).abs() < 1e-3);
        assert!((y_in_page - 30.0).abs() < 1e-3);
    }
}
