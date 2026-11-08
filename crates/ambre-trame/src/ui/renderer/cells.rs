// ambre-trame/src/ui/renderer/cells.rs
// Cell-layer rendering: full/scroll/reuse repaint of the persistent grid.

use std::collections::HashMap;

use ambre_draw::{Affine, DocumentWalk};

use super::super::grid::CellData;
use super::*;
use crate::render::{CellContent, SheetAxes, SheetCell, SheetCells, SheetView, SheetWalk};
use crate::{CellPos, MergeSpan};
use ambre_core::{FracMs, TimeSpan};

/// The screen cache as the sheet walk reads it.
pub(super) struct CacheCells<'a>(&'a HashMap<CellPos, CellData>);

impl<'a> CacheCells<'a> {
    /// The walk's view of the screen cache `cache`.
    pub(super) fn new(cache: &'a HashMap<CellPos, CellData>) -> Self {
        Self(cache)
    }
}

impl SheetCells for CacheCells<'_> {
    fn cell(&self, at: CellPos) -> Option<SheetCell<'_>> {
        self.0.get(&at).map(|c| SheetCell {
            content: c.content(&c.text),
            merge_span: c.merge_span,
        })
    }

    fn for_each_merge(&self, f: &mut dyn FnMut(CellPos, MergeSpan, CellContent<'_>)) {
        for (&at, c) in self.0 {
            if let Some(span) = c.merge_span {
                f(at, span, c.content(&c.text));
            }
        }
    }
}

/// The sheet's axes on screen: the default sizes, the resized entries and the
/// rows the shared filter hides.
pub(crate) fn sheet_axes(state: &TrameUiState) -> SheetAxes<'_> {
    SheetAxes {
        col_width: COL_WIDTH,
        row_height: ROW_HEIGHT,
        col_widths: &state.col_widths,
        row_heights: &state.row_heights,
        hidden_rows: &state.hidden_rows,
    }
}

/// The camera from sheet coordinates to the screen: the zoom, then the scroll
/// origin pinned at the corner of the header strips.
pub(crate) fn sheet_camera(state: &TrameUiState) -> Affine {
    let axes = sheet_axes(state);
    let zoom = state.zoom.get();
    Affine::translate((
        state.header_w().get() - zoom * axes.col_x(state.left_col).get(),
        state.header_h().get() - zoom * axes.row_y(state.top_row).get(),
    )) * Affine::scale(zoom)
}

/// Draw the cells layer (white bg + pending fill + cell text + grid lines).
///
/// `strip_rect == None`: full-canvas repaint. The caller must have started the
/// cells pass with LoadOp::Clear(WHITE), so the background is already white.
/// This function only adds the pending grey + cell fills + lines + text.
///
/// `strip_rect == Some(r)`: only the area inside `r` is repainted, on top of
/// existing (shifted) pixels. Pending grey covers the strip first, then cells
/// and grid lines are drawn over.
///
/// The cells, lines, merges and borders come from the sheet's walk, under the
/// camera (`CANVAS-074`). The base fill and the per-cell UI layers are the
/// screen's own, drawn before and after it.
pub fn draw_cells<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    pen: TextPen<'_>,
    strip_rect: Option<Rect>,
    clock: &dyn Clock,
    stats: &mut DrawStats,
) {
    let TextPen {
        font_cx,
        cache: text_cache,
        pass: text_pass,
    } = pen;
    let w = state.canvas_width.get();
    let h = state.grid_bottom().get();

    // Cell area = canvas minus the formula bar band + fixed header strip
    // (top/left) and the bottom sheet-tab strip.
    let cell_area = Rect::new(state.header_w().get(), state.header_h().get(), w, h);
    // The base fill under the cells differs by repaint kind:
    //  - Full (strip_rect == None): the grey "pending" veil, the first-load cue
    //    shown until the initial ViewportData lands.
    //  - Scroll (strip_rect == Some): the band entering the viewport sits on top
    //    of the shifted pixels. Fill it with the blank content background, NOT
    //    grey. The prefetch margin is meant to have these cells cached before
    //    they scroll in. But it can never win the async Logic round-trip every
    //    time (fast wheel, held arrow, a full-viewport Page Up jump). When it
    //    loses the race a grey block flashes for a frame -- the flicker. White is
    //    the graceful fallback: a not-yet-received cell reads as empty (which it
    //    usually is) until its data lands, never a flashing grey strip.
    let (strip, base_fill) = match strip_rect {
        Some(r) => (r.intersect(cell_area), palette.content_bg),
        None => (cell_area, palette.content_pending),
    };
    if strip.width() <= 0.0 || strip.height() <= 0.0 {
        stats.background = TimeSpan::<FracMs>::ZERO;
        stats.cells = TimeSpan::<FracMs>::ZERO;
        stats.cells_drawn = ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO;
        return;
    }

    let t0 = clock.now();

    // -- 1. Base fill over the strip (see `base_fill` above). -------------------
    surface.fill_rect(strip, base_fill);
    let t_pending = clock.now();

    // -- 2. The sheet's walk under the camera, over the strip's sheet region. --
    let camera = sheet_camera(state);
    let region = camera.inverse().transform_rect_bbox(strip);
    let view = SheetView {
        cells: &CacheCells::new(&state.cell_cache),
        axes: sheet_axes(state),
        palette,
        text_pass,
    };
    let mut walk = SheetWalk {
        font_cx,
        text_cache,
        cells_drawn: ambre_core::Count::<ambre_canvas::CellsDrawn>::ZERO,
    };
    surface.push_transform(camera);
    walk.walk(&view, region, surface);
    surface.pop_transform();
    let t_cells = clock.now();

    // -- 3. UI-only layers tied to cell content (dirty veil + error marker). ---
    // These live on the persistent cells layer so they scroll with the data,
    // and they are intentionally absent from the walk: a page never shows
    // "pending recompute" or transient error state.
    let layout = screen_layout(state);
    draw_ui_cell_layers(
        surface,
        state,
        &layout,
        palette,
        rows_cols_in_strip(state, strip),
    );
    let t_lines = clock.now();

    stats.background = (t_pending - t0) + (t_lines - t_cells);
    stats.cells = t_cells - t_pending;
    stats.cells_drawn = walk.cells_drawn;
}

/// Paint the per-cell UI-only layers for the cells intersecting the strip:
/// a reduced-opacity veil over dirty cells, and a corner marker over errored
/// cells. Iterates the (small) dirty/error sets rather than the visible grid,
/// rejecting entries outside the `span` already computed by the caller.
pub(crate) fn draw_ui_cell_layers<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    layout: &GridLayout,
    palette: &Palette,
    span: CellSpan,
) {
    // Dirty veil: a translucent content_bg quad fades the optimistic value.
    let veil = palette.content_bg.with_alpha(DIRTY_VEIL_ALPHA);
    for &at in &state.dirty_cells {
        if span.contains(at) && !state.is_row_hidden(at.row) {
            surface.fill_rect(layout.cell_rect(at), veil);
        }
    }

    // Error marker: a filled right-triangle in the cell's top-right corner,
    // assembled from 1px horizontal lines. It then renders on the GPU
    // surface, since `WgpuSurface` does not implement fill_path.
    let err = palette.cell_error;
    for &at in &state.formula_errors {
        if !span.contains(at) || state.is_row_hidden(at.row) {
            continue;
        }
        let rect = layout.cell_rect(at);
        // The surface draws in bare f64: the leg unwraps once, here.
        let leg = ERROR_MARK_SIZE.get();
        let mut dy = 0.0;
        while dy < leg {
            // Row `dy` from the top spans the hypotenuse to the right edge.
            let x0 = rect.x1 - (leg - dy);
            let y = rect.y0 + dy + 0.5;
            surface.stroke_line(
                ambre_draw::Line::new((x0, y), (rect.x1, y)),
                err,
                ambre_core::Px::<ambre_core::Surface>::new(1.0),
            );
            dy += 1.0;
        }
    }
}
