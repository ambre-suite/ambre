//! The page zoom: the range of the page, through the one zoom engine
//! (`features/ambre-spec-prose-viewport.md section 2`, `UI-THREAD-012`).
//!
//! The zoom is local view state (`PROSE-UI-010`). Nothing here reads the
//! document, moves the caret or sends a command (`PROSE-UI-129`).

use ambre_core::{Doc, Point, Px, Scale, View, Zoom, ZoomRange};

pub use ambre_core::ZoomStep;

use crate::ui::ProseUiState;

/// The factors the page supports (`PROSE-UI-024`). The chip and a zoom key
/// read the stops of the one ladder inside it (`ZOOM-006`).
pub const ZOOM_RANGE: ZoomRange<Doc> = ZoomRange::new(
    Scale::<Doc, View>::positive(0.25),
    Scale::<Doc, View>::positive(4.0),
);

/// The stop `index` of the page's presets, low to high, for the tests.
#[cfg(test)]
pub(crate) fn zoom_stop(index: usize) -> Scale<Doc, View> {
    ZOOM_RANGE.presets().nth(index).unwrap_or(Scale::ONE)
}

/// What a zoom request did to the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomChange {
    /// The factor was already the one asked for, or a step found no preset.
    Same,
    /// The factor moved, so the pages repaint.
    Moved,
}

impl ProseUiState {
    /// Show the pages at `zoom`, clamped to the preset range (`PROSE-UI-024`).
    ///
    /// The document line at the top of the canvas stays there, so the first
    /// line of a short document never leaves the screen. The offset is then
    /// clamped to the column (`PROSE-UI-025`). The column recentres by
    /// itself (`PROSE-UI-026`).
    pub fn zoom_to(&mut self, zoom: Scale<Doc, View>) -> ZoomChange {
        let next = ZOOM_RANGE.clamp(zoom);
        if next == self.viewport.zoom {
            return ZoomChange::Same;
        }
        self.viewport.zoom = next;
        self.clamp_scroll();
        ZoomChange::Moved
    }

    /// Zoom to `asked` about a canvas point, through the one engine
    /// (`ZOOM-003`, `PROSE-UI-128`). The document point under `at` stays
    /// there. The clamp of the offsets (`PROSE-UI-025`) and the centring of
    /// a page that fits (`PROSE-UI-026`) may move it.
    pub fn zoom_about(&mut self, asked: Scale<Doc, View>, at: Point<Px<View>>) -> ZoomChange {
        let zoom = Zoom::at(ZOOM_RANGE, self.viewport.zoom);
        if zoom.to(asked).factor() == self.viewport.zoom {
            return ZoomChange::Same;
        }
        // The column coordinate at the canvas's left edge.
        let left = self.column.left(&self.viewport);
        let origin_x = Px::<View>::ZERO - left;
        let across = zoom.about(asked, origin_x / zoom.factor(), at.x);
        let down = zoom.about(asked, self.viewport.scroll_y, at.y);
        self.viewport.zoom = down.zoom.factor();
        self.viewport.scroll_y = down.origin;
        self.viewport.scroll_x = across.origin;
        self.clamp_scroll();
        ZoomChange::Moved
    }

    /// The factor a wheel step held with `Ctrl` asks for, a trackpad pinch
    /// included (`ZOOM-004`).
    #[must_use]
    pub fn wheel_zoom(&self, dy: Px<ambre_core::Css>) -> Scale<Doc, View> {
        Zoom::at(ZOOM_RANGE, self.viewport.zoom).wheeled(dy)
    }

    /// Move the zoom one preset up or down, or back to a hundred per cent
    /// (`PROSE-UI-250`). A step past either end of the ladder moves nothing.
    pub fn step_zoom(&mut self, step: ZoomStep) -> ZoomChange {
        let next = Zoom::at(ZOOM_RANGE, self.viewport.zoom).step(step).factor();
        self.zoom_to(next)
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::layout::pass::PageLayout;
    use crate::message::Page;
    use crate::view::column::PageColumn;
    use ambre_core::index::Len;
    use ambre_core::{Px, View};
    use proptest::prelude::*;

    fn a_view(pages: u32, width: f32) -> ProseUiState {
        let mut ui = ProseUiState::default();
        ui.resize(Px::<View>::new(width), Px::<View>::new(900.0));
        ui.column = PageColumn::of(&PageLayout {
            page_width: Px::<Doc>::new(816.0),
            page_height: Px::<Doc>::new(1056.0),
            page_count: Len::<Page>::new(pages),
            ..PageLayout::default()
        });
        ui
    }

    proptest! {
        /// `PROSE-UI-024`, `PROSE-UI-025`: whatever factor is asked, the zoom
        /// lands inside the preset range and the offset inside the column.
        #[test]
        fn a_zoom_stays_in_range_and_keeps_the_offset_in_the_column(
            asked in prop_oneof![
                -10.0f64..10.0,
                Just(f64::NAN),
                Just(f64::INFINITY),
                Just(0.0),
                Just(1.0e9),
            ],
            scroll in 0.0f32..400_000.0,
            pages in 1u32..300,
        ) {
            let mut ui = a_view(pages, 1200.0);
            ui.viewport.scroll_y = ui.column.clamp_scroll(&ui.viewport, Px::<Doc>::new(scroll));
            let _ = ui.zoom_to(Scale::<Doc, View>::positive(asked));
            let zoom = ui.viewport.zoom;
            prop_assert!(zoom >= ZOOM_RANGE.floor() && zoom <= ZOOM_RANGE.ceiling(), "{zoom:?}");
            prop_assert!(ui.viewport.scroll_y >= Px::<Doc>::ZERO);
            prop_assert!(ui.viewport.scroll_y <= ui.column.max_scroll(&ui.viewport));
        }

        /// `PROSE-UI-026`, `PROSE-UI-125`: at every zoom and every width, the
        /// column is centred when it fits, and pinned to the left edge when
        /// it does not.
        #[test]
        fn the_column_stays_centred_or_pinned_at_every_zoom(
            asked in 0.25f64..4.0,
            width in 0.0f32..4000.0,
        ) {
            let mut ui = a_view(3, width);
            let _ = ui.zoom_to(Scale::<Doc, View>::positive(asked));
            let left = ui.column.left(&ui.viewport).get();
            let page = ui.column.page_width.get() * ui.viewport.scale();
            if page <= width {
                prop_assert!(left >= 0.0);
                prop_assert!((left * 2.0 + page - width).abs() < 1e-2, "{left} {page} {width}");
            } else {
                prop_assert!(left == 0.0);
            }
        }
    }

    proptest! {
        /// `PROSE-UI-128`, `ZOOM-003`: the document point under the pointer
        /// stays under it, on each axis the clamp and the centring leave
        /// free. The zoom stays in range whatever factor is asked.
        #[test]
        fn a_zoom_about_a_point_keeps_the_point_under_it(
            from in 0.25f64..4.0,
            asked in 0.05f64..8.0,
            scroll in 0.0f32..300_000.0,
            across in 0.0f32..3000.0,
            width in 200.0f32..2400.0,
            x in 0.0f32..1.0,
            y in 0.0f32..1.0,
        ) {
            let mut ui = a_view(300, width);
            let _ = ui.zoom_to(Scale::<Doc, View>::positive(from));
            ui.viewport.scroll_y = Px::<Doc>::new(scroll);
            ui.viewport.scroll_x = Px::<Doc>::new(across);
            ui.clamp_scroll();
            let at = ambre_core::Point::new(Px::<View>::new(x * width), Px::<View>::new(y * 900.0));
            let crate::view::column::DocPos::OnPage { page, x_in_page, y_in_page } =
                ui.column.screen_to_doc(&ui.viewport, at)
            else {
                return Err(TestCaseError::fail("no page"));
            };
            // A point in a gutter resolves to a page edge (`PROSE-UI-124`), so
            // it is not the point under the pointer: leave it out.
            let under = ui.viewport.scroll_y.get() + at.y.get() / ui.viewport.scale();
            let resolved = ui.column.page_top_y(page).get() + y_in_page.get();
            prop_assume!((under - resolved).abs() <= 0.05);
            let _ = ui.zoom_about(Scale::<Doc, View>::positive(asked), at);
            let zoom = ui.viewport.zoom;
            prop_assert!(zoom >= ZOOM_RANGE.floor() && zoom <= ZOOM_RANGE.ceiling());
            let now = ui.column.doc_to_screen(&ui.viewport, page, x_in_page, y_in_page);
            let free_y = ui.viewport.scroll_y > Px::<Doc>::ZERO
                && ui.viewport.scroll_y < ui.column.max_scroll(&ui.viewport);
            let free_x = ui.viewport.scroll_x > Px::<Doc>::ZERO
                && ui.viewport.scroll_x < ui.column.max_scroll_x(&ui.viewport);
            // `f32` offsets of a 300-page column: a few steps of 300 000.
            let grain = 0.25 * ui.viewport.scale().max(1.0);
            if free_y {
                prop_assert!((now.y - at.y).get().abs() <= grain, "y {:?} -> {:?}", at.y, now.y);
            }
            if free_x {
                prop_assert!((now.x - at.x).get().abs() <= 1e-2, "x {:?} -> {:?}", at.x, now.x);
            }
        }

        /// `PROSE-UI-251`: a page wider than the canvas can be reached edge
        /// to edge, and never leaves a gap on either side of it.
        #[test]
        fn a_wide_page_is_reachable_edge_to_edge(
            zoom in 0.25f64..4.0,
            width in 100.0f32..3000.0,
            dx in -5000.0f32..5000.0,
        ) {
            let mut ui = a_view(3, width);
            let _ = ui.zoom_to(Scale::<Doc, View>::positive(zoom));
            ui.scroll_across(Px::<View>::new(dx));
            let left = ui.column.left(&ui.viewport).get();
            let page = ui.column.page_width.get() * ui.viewport.scale();
            if page > width {
                prop_assert!(left <= 0.0 && left + page >= width - 1e-2, "{left} {page} {width}");
                ui.scroll_across(Px::<View>::new(1.0e7));
                let right = ui.column.left(&ui.viewport).get() + page;
                prop_assert!((right - width).abs() <= 1e-2 * page.max(1.0) / 100.0 + 1e-2);
                ui.scroll_across(Px::<View>::new(-1.0e7));
                prop_assert!(ui.column.left(&ui.viewport).get() == 0.0);
            } else {
                prop_assert!((left * 2.0 + page - width).abs() < 1e-2);
            }
        }
    }

    /// `PROSE-UI-250`: a key steps to the next preset, stops at either end,
    /// and `Ctrl+0` returns to a hundred per cent.
    #[test]
    fn a_step_moves_one_preset_and_stops_at_the_ends() {
        let mut ui = a_view(10, 1200.0);
        assert_eq!(ui.step_zoom(ZoomStep::In), ZoomChange::Moved);
        assert_eq!(ui.viewport.zoom, zoom_stop(4));
        for _ in 0..20 {
            let _ = ui.step_zoom(ZoomStep::In);
        }
        assert_eq!(ui.viewport.zoom, zoom_stop(8));
        assert_eq!(ui.step_zoom(ZoomStep::In), ZoomChange::Same);
        assert_eq!(ui.step_zoom(ZoomStep::Out), ZoomChange::Moved);
        assert_eq!(ui.viewport.zoom, zoom_stop(7));
        assert_eq!(ui.step_zoom(ZoomStep::Reset), ZoomChange::Moved);
        assert_eq!(ui.viewport.zoom, Scale::<Doc, View>::ONE);
        // A factor between two presets steps to the nearer one on its side.
        let _ = ui.zoom_to(Scale::<Doc, View>::positive(1.1));
        assert_eq!(ui.step_zoom(ZoomStep::Out), ZoomChange::Moved);
        assert_eq!(ui.viewport.zoom, Scale::<Doc, View>::ONE);
        for _ in 0..20 {
            let _ = ui.step_zoom(ZoomStep::Out);
        }
        assert_eq!(ui.viewport.zoom, zoom_stop(0));
        assert_eq!(ui.step_zoom(ZoomStep::Out), ZoomChange::Same);
    }

    /// The text at the top of the canvas stays there across a zoom. The first
    /// line of a document read from its top stays in view.
    #[test]
    fn a_zoom_keeps_the_top_of_the_canvas_in_place() {
        let mut ui = a_view(50, 1200.0);
        ui.viewport.scroll_y = Px::<Doc>::new(10_000.0);
        let _ = ui.zoom_to(Scale::<Doc, View>::positive(2.0));
        assert_eq!(ui.viewport.scroll_y, Px::<Doc>::new(10_000.0));
        let _ = ui.zoom_to(Scale::<Doc, View>::positive(0.5));
        assert_eq!(ui.viewport.scroll_y, Px::<Doc>::new(10_000.0));
        let mut short = a_view(1, 1200.0);
        let _ = short.zoom_to(Scale::<Doc, View>::positive(2.0));
        assert_eq!(short.viewport.scroll_y, Px::<Doc>::ZERO);
    }
}
