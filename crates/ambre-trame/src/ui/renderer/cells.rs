// ambre-trame/src/ui/renderer/cells.rs
// Cell-layer rendering: full/scroll/reuse repaint of the persistent grid.

use crate::hash::HashMap;

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
/// origin pinned at the corner the frozen bands leave. With no frozen pane,
/// that corner is the corner of the header strips.
#[cfg(test)]
pub(crate) fn sheet_camera(state: &TrameUiState) -> Affine {
    pane_camera(state, Pane::SCROLLED)
}

/// One of the four panes a frozen pane splits the cell area into. Each axis
/// of a pane is frozen, drawn from the first line, or scrolled, drawn from
/// the scroll origin past the frozen band.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pane {
    rows: Band,
    cols: Band,
}

/// Whether the lines of one axis of a pane are frozen or scrolled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Band {
    Frozen,
    Scrolled,
}

impl Pane {
    /// The pane that scrolls on both axes.
    pub(crate) const SCROLLED: Self = Self {
        rows: Band::Scrolled,
        cols: Band::Scrolled,
    };

    /// The four panes, the scrolled one first.
    const ALL: [Self; 4] = [
        Self::SCROLLED,
        Self {
            rows: Band::Frozen,
            cols: Band::Scrolled,
        },
        Self {
            rows: Band::Scrolled,
            cols: Band::Frozen,
        },
        Self {
            rows: Band::Frozen,
            cols: Band::Frozen,
        },
    ];
}

/// The camera of `pane`. A frozen axis lays the sheet out from its first
/// line at the header strip. A scrolled axis lays it out from the scroll
/// origin, past the frozen band (`TRAME-UI-192`).
fn pane_camera(state: &TrameUiState, pane: Pane) -> Affine {
    let axes = sheet_axes(state);
    let zoom = state.zoom.get();
    let x = match pane.cols {
        Band::Frozen => state.header_w().get(),
        Band::Scrolled => {
            (state.header_w() + state.frozen_w()).get() - zoom * axes.col_x(state.left_col).get()
        }
    };
    let y = match pane.rows {
        Band::Frozen => state.header_h().get(),
        Band::Scrolled => {
            (state.header_h() + state.frozen_h()).get() - zoom * axes.row_y(state.top_row).get()
        }
    };
    Affine::translate((x, y)) * Affine::scale(zoom)
}

/// The screen rectangle of `pane` inside the cell area `cell_area`. Empty
/// for a frozen band of no line.
fn pane_rect(state: &TrameUiState, pane: Pane, cell_area: Rect) -> Rect {
    let split_x = cell_area.x0 + state.frozen_w().get();
    let split_y = cell_area.y0 + state.frozen_h().get();
    let (x0, x1) = match pane.cols {
        Band::Frozen => (cell_area.x0, split_x.min(cell_area.x1)),
        Band::Scrolled => (split_x.min(cell_area.x1), cell_area.x1),
    };
    let (y0, y1) = match pane.rows {
        Band::Frozen => (cell_area.y0, split_y.min(cell_area.y1)),
        Band::Scrolled => (split_y.min(cell_area.y1), cell_area.y1),
    };
    Rect::new(x0, y0, x1, y1)
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

    // -- 2. The sheet's walk under each pane's camera, over the part of the
    //       strip the pane holds. With no frozen pane, the scrolled pane is
    //       the whole cell area (`TRAME-UI-192`). --------------------------------
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
    let mut drawn = Vec::with_capacity(Pane::ALL.len());
    for pane in Pane::ALL {
        let shown = pane_rect(state, pane, cell_area).intersect(strip);
        if shown.width() <= 0.0 || shown.height() <= 0.0 {
            continue;
        }
        let camera = pane_camera(state, pane);
        let region = camera.inverse().transform_rect_bbox(shown);
        surface.push_clip(shown);
        surface.push_transform(camera);
        walk.walk(&view, region, surface);
        surface.pop_transform();
        surface.pop_clip();
        drawn.push(walk.cells_drawn);
    }
    walk.cells_drawn = drawn.into_iter().sum();
    draw_frozen_edges(surface, state, palette, cell_area);
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

/// Stroke one line along the far edge of each frozen band, so the reader
/// sees where the scrolled cells pass under it.
fn draw_frozen_edges<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    cell_area: Rect,
) {
    let width = ambre_core::Px::<ambre_core::Surface>::new(2.0);
    if !state.frozen.rows.is_zero() {
        let y = cell_area.y0 + state.frozen_h().get();
        if y < cell_area.y1 {
            surface.stroke_line(
                ambre_draw::Line::new((cell_area.x0, y), (cell_area.x1, y)),
                palette.header_border,
                width,
            );
        }
    }
    if !state.frozen.cols.is_zero() {
        let x = cell_area.x0 + state.frozen_w().get();
        if x < cell_area.x1 {
            surface.stroke_line(
                ambre_draw::Line::new((x, cell_area.y0), (x, cell_area.y1)),
                palette.header_border,
                width,
            );
        }
    }
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
    // A cell shows when the strip holds it or a frozen band does, and no
    // frozen band covers it.
    let shows = |at: CellPos| {
        let row_frozen = at.row < state.frozen.rows.as_end();
        let col_frozen = at.col < state.frozen.cols.as_end();
        let row_on = row_frozen || at.row >= state.top_row;
        let col_on = col_frozen || at.col >= state.left_col;
        (span.contains(at) || row_frozen || col_frozen) && row_on && col_on
    };
    for &at in &state.dirty_cells {
        if shows(at) && !state.is_row_hidden(at.row) {
            surface.fill_rect(layout.cell_rect(at), veil);
        }
    }

    // Error marker: a filled right-triangle in the cell's top-right corner,
    // assembled from 1px horizontal lines. It then renders on the GPU
    // surface, since `WgpuSurface` does not implement fill_path.
    let err = palette.cell_error;
    for &at in &state.formula_errors {
        if !shows(at) || state.is_row_hidden(at.row) {
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
