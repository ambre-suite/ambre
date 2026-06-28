// ambre-trame/src/ui/renderer.rs
// ============================================================================
// Trame grid renderer -- UI Thread.
//
// Split into two layers to support dirty-rect rendering:
//
//   draw_cells (CELLS layer, persistent offscreen):
//       white bg + pending bg + received-cell fills + grid lines + cell text.
//       Either full-canvas redraw or strip-only (during scroll), controlled by
//       the `strip_rect` argument.
//
//   draw_overlay (OVERLAY layer, swapchain):
//       column/row header backgrounds + active tints + separator lines
//       + active cell border + column/row header labels + on-canvas HUD.
//
// The orchestrator (`TrameView::plan_repaint`, driven by the launcher's FrameHost)
// detects what changed since the previous frame and selects the appropriate
// combination:
//   - Full repaint:      draw_cells(strip=None) + draw_overlay
//   - Pure scroll:       draw_cells(strip=Some(strip_rect)) + draw_overlay
//   - Overlay-only diff: draw_overlay
// ============================================================================

use ambre_draw::{
    Affine, Alignment, Brush, Clock, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

use ambre_canvas::{Palette, TextCache};

use super::TrameUiState;
use super::grid::{
    self, ADD_BTN_WIDTH, COL_WIDTH, CellEditor, HEADER_HEIGHT, HEADER_WIDTH, HitKind, ROW_HEIGHT,
    TAB_MENU_ITEM_HEIGHT, TAB_MENU_WIDTH, TAB_STRIP_HEIGHT, TAB_WIDTH, TabDrag, TabMenu, TabRename,
};
use crate::render::{self, BODY_SIZE, CELL_PAD_X, GridLayout};
use crate::{TOTAL_COLS, TOTAL_ROWS};

/// Build the shared [`GridLayout`] for the current screen viewport. Carries the
/// per-axis width/height maps (sec.4bis.C) so resized columns/rows reflow; the
/// cell-aligned scroll position is the layout origin.
fn screen_layout(state: &TrameUiState) -> GridLayout<'_> {
    GridLayout {
        // Collapse the row axis over filter-hidden rows (sec.5.D).
        hidden_rows: &state.hidden_rows,
        header_width: HEADER_WIDTH,
        header_height: HEADER_HEIGHT,
        top: 0.0,
        col_width: COL_WIDTH,
        row_height: ROW_HEIGHT,
        col_widths: &state.col_widths,
        row_heights: &state.row_heights,
        origin_col: state.left_col,
        origin_row: state.top_row,
        width: state.canvas_width,
        // The grid stops above the bottom sheet-tab strip (sec.13).
        height: state.grid_bottom(),
        total_cols: TOTAL_COLS,
        total_rows: TOTAL_ROWS,
    }
}

// -----------------------------------------------------------------------------
// Colors
// -----------------------------------------------------------------------------
//
// Product colours come from the active `Palette` (generated from tokens.ron,
// theme-switched at runtime). Only the debug HUD keeps hardcoded colours -- it
// is a developer overlay, not themed product chrome.

// -----------------------------------------------------------------------------
// DrawStats — per-frame breakdown
// -----------------------------------------------------------------------------

/// Sub-phase timings for one frame.
///
/// All times are wall-clock milliseconds measured via `performance.now()`.
/// The GPU work (copy_texture, blit pass, present) happens after these CPU
/// phases return and is measured separately as `last_gpu_ms` in `UiState`.
#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    /// Cells layer non-text geometry: pending bg, received-cell fills,
    /// grid lines. Zero on overlay-only frames.
    pub bg_ms: f32,
    /// Cells layer text (parley shaping + glyph atlas + push_glyph per cell).
    /// Zero on overlay-only frames or when `text_enabled` is false.
    pub cells_ms: f32,
    /// Overlay layer non-HUD: header backgrounds, active tints, separator
    /// lines, active border, header labels.
    pub hdrs_ms: f32,
    /// Number of cells with non-empty text actually drawn this frame.
    pub cells_drawn: u32,
    /// Latest measured input->caret latency (keydown to caret paint), in ms.
    /// Target < 50 ms, acceptable < 100 ms (ambre-spec-chrome 7, Phase 3).
    pub input_latency_ms: f32,
}

/// Opacity of the veil painted over a dirty (awaiting-recompute) cell. The
/// content_bg-coloured veil fades the optimistic value toward the background.
const DIRTY_VEIL_ALPHA: f32 = 0.5;
/// Leg length, in physical pixels, of the right-triangle error marker drawn in
/// a cell's top-right corner.
const ERROR_MARK_SIZE: f64 = 7.0;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Half-open `[row_start, row_end) x [col_start, col_end)` cell range covering a
/// repaint strip. Bundling the four bounds keeps the per-cell helpers' argument
/// lists short and the `in_strip` test self-documenting.
#[derive(Clone, Copy)]
struct CellSpan {
    row_start: u32,
    row_end: u32,
    col_start: u32,
    col_end: u32,
}

impl CellSpan {
    /// True when `(row, col)` falls inside the half-open span.
    fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.row_start && row < self.row_end && col >= self.col_start && col < self.col_end
    }
}

/// Compute the cell-coordinate range that intersects `strip` in the cell area.
fn rows_cols_in_strip(state: &TrameUiState, strip: Rect) -> CellSpan {
    let cells_top = HEADER_HEIGHT;
    // Walk the variable column widths from the scroll origin: skip columns wholly
    // left of the strip, then extend until the strip's right edge is covered.
    let mut col_start = state.left_col;
    let mut cx = HEADER_WIDTH;
    while col_start + 1 < TOTAL_COLS && cx + state.col_width_of(col_start) <= strip.x0 {
        cx += state.col_width_of(col_start);
        col_start += 1;
    }
    let mut col_end = col_start;
    let mut ex = cx;
    while col_end < TOTAL_COLS && ex < strip.x1 {
        ex += state.col_width_of(col_end);
        col_end += 1;
    }
    let col_end = col_end.min(TOTAL_COLS);

    // Same walk over the *visible* rows' variable heights (hidden rows take no
    // vertical space; the cell iterator skips them too).
    let mut row_start = state.top_row;
    let mut cy = cells_top;
    while row_start + 1 < TOTAL_ROWS {
        if state.is_row_hidden(row_start) {
            row_start += 1;
            continue;
        }
        let h = state.row_height_of(row_start);
        if cy + h <= strip.y0 {
            cy += h;
            row_start += 1;
        } else {
            break;
        }
    }
    let mut row_end = row_start;
    let mut ey = cy;
    while row_end < TOTAL_ROWS && ey < strip.y1 {
        if !state.is_row_hidden(row_end) {
            ey += state.row_height_of(row_end);
        }
        row_end += 1;
    }
    let row_end = row_end.min(TOTAL_ROWS);
    CellSpan {
        row_start,
        row_end,
        col_start,
        col_end,
    }
}

// -----------------------------------------------------------------------------
// draw_cells -- persistent CELLS layer
// -----------------------------------------------------------------------------

/// Draw the cells layer (white bg + pending fill + cell text + grid lines).
///
/// `strip_rect == None`: full-canvas repaint. The caller must have started
/// the cells pass with LoadOp::Clear(WHITE) so the background is already
/// white; this function only adds the pending grey + cell fills + lines + text.
///
/// `strip_rect == Some(r)`: only the area inside `r` is repainted, on top of
/// existing (shifted) pixels. Pending grey covers the strip first, then cells
/// and grid lines are drawn over.
#[allow(clippy::too_many_arguments)]
pub fn draw_cells<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    strip_rect: Option<Rect>,
    clock: &dyn Clock,
    stats: &mut DrawStats,
) {
    let w = state.canvas_width;
    let h = state.grid_bottom();

    // Cell area = canvas minus the formula bar band + fixed header strip
    // (top/left) and the bottom sheet-tab strip.
    let cell_area = Rect::new(HEADER_WIDTH, HEADER_HEIGHT, w, h);
    // The base fill under the cells differs by repaint kind:
    //  - Full (strip_rect == None): the grey "pending" veil, the first-load cue
    //    shown until the initial ViewportData lands.
    //  - Scroll (strip_rect == Some): the band entering the viewport sits on top
    //    of the shifted pixels. Fill it with the blank content background, NOT
    //    grey. The prefetch margin is meant to have these cells cached before
    //    they scroll in, but it can never win the async Logic round-trip every
    //    time (fast wheel, held arrow, a full-viewport Page Up jump). When it
    //    loses the race a grey block flashes for a frame -- the flicker. White is
    //    the graceful fallback: a not-yet-received cell reads as empty (which it
    //    usually is) until its data lands, never a flashing grey strip.
    let (strip, base_fill) = match strip_rect {
        Some(r) => (r.intersect(cell_area), palette.content_bg),
        None => (cell_area, palette.content_pending),
    };
    if strip.width() <= 0.0 || strip.height() <= 0.0 {
        stats.bg_ms = 0.0;
        stats.cells_ms = 0.0;
        stats.cells_drawn = 0;
        return;
    }

    let layout = screen_layout(state);
    let t0 = clock.now_ms();

    // -- 1. Base fill over the strip (see `base_fill` above). -------------------
    surface.fill_rect(strip, base_fill);
    let t_pending = clock.now_ms();

    // -- 2. Received-cell backgrounds + body text, via the shared primitive. ---
    // The cell area is virtual, so we feed only the cached cells inside the
    // strip rather than a full DisplayViewport.
    let span = rows_cols_in_strip(state, strip);
    let cells = (span.row_start..span.row_end)
        // A collapsed (filtered) row occupies no slot: skip it so its successors
        // are not double-drawn over the next visible row.
        .filter(move |row| !state.is_row_hidden(*row))
        .flat_map(move |row| {
            (span.col_start..span.col_end).filter_map(move |col| {
                state.cell_cache.get(&(row, col)).map(|c| {
                    // A merge master's text is laid out by the merge pass over the
                    // full span; drawing it here too would double it (single-cell
                    // baseline + span baseline). Pass empty text so only its bg fill
                    // lands here. Covered cells already carry empty text.
                    let text = if c.merge_span.is_some() {
                        ""
                    } else {
                        c.text.as_str()
                    };
                    (row, col, c.content(text))
                })
            })
        });
    // A neighbour is "empty" -- so an overflowing non-wrapped line may spill over
    // it -- when the cache has no entry for it or its text is blank. A merge
    // master keeps its (non-empty) text, so overflow stops at a merged block.
    let is_empty = |row: u32, col: u32| {
        state
            .cell_cache
            .get(&(row, col))
            .is_none_or(|c| c.text.is_empty())
    };
    let cells_drawn = render::draw_cells_on(
        surface,
        &layout,
        palette,
        font_cx,
        text_cache,
        text_enabled,
        cells,
        is_empty,
    );
    let t_cells = clock.now_ms();

    // -- 3. Grid lines on top of the fills, so separators stay visible. --------
    render::draw_grid_lines_on(surface, &layout, palette.grid_line, strip);

    // -- 3bis. Merge masters: paint over the interior separators of the covered
    // cells, lay the master's text across the span, re-stroke the outer border.
    // Iterates the (bounded) cache rather than the strip so a master pinned just
    // outside the strip still repaints the part of its span that is inside.
    let merges = state.cell_cache.iter().filter_map(|(&(row, col), c)| {
        c.merge_span.and_then(|(col_span, row_span)| {
            let hit = layout
                .merge_rect(row, col, col_span, row_span)
                .intersect(strip);
            (hit.width() > 0.0 && hit.height() > 0.0).then_some((
                row,
                col,
                col_span,
                row_span,
                c.content(&c.text),
            ))
        })
    });
    render::draw_merges_on(
        surface,
        &layout,
        palette,
        font_cx,
        text_cache,
        text_enabled,
        merges,
    );

    // -- 3ter. Cell borders LAST, so a deliberate border is never overpainted by a
    // faint grid separator nor by the merge repaint (`-ui.md` sec.5bis). A merged
    // master's border wraps the whole span (merge_rect); plain cells use their own
    // rect. Second pass over the strip (the first iterator was consumed by
    // draw_cells_on); plus the cache for merge masters pinned just outside it.
    let bspan = rows_cols_in_strip(state, strip);
    let strip_borders = (bspan.row_start..bspan.row_end)
        .filter(move |row| !state.is_row_hidden(*row))
        .flat_map(move |row| {
            (bspan.col_start..bspan.col_end).filter_map(move |col| {
                let c = state.cell_cache.get(&(row, col))?;
                // Merge masters are handled by the cache pass below (full span);
                // skip them here so a single-cell rect is not drawn under the span.
                if c.merge_span.is_some() {
                    return None;
                }
                Some((layout.cell_rect(row, col), c.content("")))
            })
        });
    render::draw_cell_borders_on(surface, strip_borders);
    // Merge masters: border around the full span rect.
    let merge_borders = state.cell_cache.iter().filter_map(|(&(row, col), c)| {
        let (col_span, row_span) = c.merge_span?;
        let rect = layout.merge_rect(row, col, col_span, row_span);
        let hit = rect.intersect(strip);
        (hit.width() > 0.0 && hit.height() > 0.0).then(|| (rect, c.content("")))
    });
    render::draw_cell_borders_on(surface, merge_borders);

    // -- 4. UI-only layers tied to cell content (dirty veil + error marker). ---
    // These live on the persistent cells layer so they scroll with the data,
    // and they are intentionally absent from the shared `render` primitives:
    // a PDF export never shows "pending recompute" or transient error state.
    draw_ui_cell_layers(surface, state, &layout, palette, span);
    let t_lines = clock.now_ms();

    stats.bg_ms = ((t_pending - t0) + (t_lines - t_cells)) as f32;
    stats.cells_ms = (t_cells - t_pending) as f32;
    stats.cells_drawn = cells_drawn;
}

/// Paint the per-cell UI-only layers for the cells intersecting the strip:
/// a reduced-opacity veil over dirty cells, and a corner marker over errored
/// cells. Iterates the (small) dirty/error sets rather than the visible grid,
/// rejecting entries outside the `span` already computed by the caller.
fn draw_ui_cell_layers<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    layout: &GridLayout,
    palette: &Palette,
    span: CellSpan,
) {
    // Dirty veil: a translucent content_bg quad fades the optimistic value.
    let veil = palette.content_bg.with_alpha(DIRTY_VEIL_ALPHA);
    for &(row, col) in &state.dirty_cells {
        if span.contains(row, col) && !state.is_row_hidden(row) {
            surface.fill_rect(layout.cell_rect(row, col), veil);
        }
    }

    // Error marker: a filled right-triangle in the cell's top-right corner,
    // assembled from 1px horizontal lines so it renders on the GPU surface
    // (WgpuSurface does not implement fill_path).
    let err = palette.cell_error;
    for &(row, col) in state.formula_errors.keys() {
        if !span.contains(row, col) || state.is_row_hidden(row) {
            continue;
        }
        let rect = layout.cell_rect(row, col);
        let mut dy = 0.0;
        while dy < ERROR_MARK_SIZE {
            // Row `dy` from the top spans the hypotenuse to the right edge.
            let x0 = rect.x1 - (ERROR_MARK_SIZE - dy);
            let y = rect.y0 + dy + 0.5;
            surface.stroke_line(x0, y, rect.x1, y, err, 1.0);
            dy += 1.0;
        }
    }
}

// -----------------------------------------------------------------------------
// draw_overlay -- transient OVERLAY layer
// -----------------------------------------------------------------------------

/// Marching-ants dash geometry (physical px): 6px dash + 4px gap. The animation
/// crawls the dashes by advancing a phase across this period each frame.
const MARCHING_DASH: f64 = 6.0;
const MARCHING_GAP: f64 = 4.0;
const MARCHING_PERIOD: f64 = MARCHING_DASH + MARCHING_GAP;

/// Draw a small filled "+" badge centred on `(cx, cy)`: the inline-insert
/// affordance shown on a hovered header boundary (sec.4bis.D).
fn draw_plus<S: DrawSurface>(s: &mut S, cx: f64, cy: f64, c: Color) {
    let arm = 4.0;
    let t = 1.5;
    s.fill_rect(Rect::new(cx - arm, cy - t, cx + arm, cy + t), c);
    s.fill_rect(Rect::new(cx - t, cy - arm, cx + t, cy + arm), c);
}

/// Stroke a horizontal dashed line from `x0` to `x1` at height `y`, the dashes
/// offset left by `phase` (the marching-ants crawl, sec.5.E).
fn dashed_h<S: DrawSurface>(s: &mut S, x0: f64, x1: f64, y: f64, c: Color, phase: f64) {
    let mut t = x0 - phase;
    while t < x1 {
        let a = t.max(x0);
        let b = (t + MARCHING_DASH).min(x1);
        if b > a {
            s.stroke_line(a, y, b, y, c, 1.0);
        }
        t += MARCHING_PERIOD;
    }
}

/// Stroke a vertical dashed line from `y0` to `y1` at column `x` (see [`dashed_h`]).
fn dashed_v<S: DrawSurface>(s: &mut S, y0: f64, y1: f64, x: f64, c: Color, phase: f64) {
    let mut t = y0 - phase;
    while t < y1 {
        let a = t.max(y0);
        let b = (t + MARCHING_DASH).min(y1);
        if b > a {
            s.stroke_line(x, a, x, b, c, 1.0);
        }
        t += MARCHING_PERIOD;
    }
}

/// Draw the overlay layer (headers + active border + header labels + HUD).
/// This is re-rendered every dirty frame onto the swapchain after the cells
/// layer has been blitted from its persistent offscreen.
#[allow(clippy::too_many_arguments)]
pub fn draw_overlay<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    clock: &dyn Clock,
    stats: &mut DrawStats,
) {
    let t0 = clock.now_ms();

    let layout = screen_layout(state);

    // -- 1. Fixed header strips (bg + active tint + separator + labels), via
    //       the shared primitives. The primitives clip the active tint to the
    //       visible range, so passing the raw active index is safe. -----------
    render::draw_col_headers_on(
        surface,
        &layout,
        palette,
        Some(state.active_col()),
        font_cx,
        text_cache,
        text_enabled,
    );
    render::draw_row_headers_on(
        surface,
        &layout,
        palette,
        Some(state.active_row()),
        font_cx,
        text_cache,
        text_enabled,
    );

    // -- 1b. Funnel badge on the filtered column's header (sec.5.D). Drawn only
    //        when the column is within the visible span; a small funnel glyph
    //        (trapezoid + stem) using stroke primitives so every surface renders
    //        it (fill_path is optional). -------------------------------------
    if let Some(fcol) = state.filtered_col
        && fcol >= layout.origin_col
        && fcol < layout.origin_col + state.viewport_cols + 1
    {
        let cell = layout.cell_rect(layout.origin_row, fcol);
        // Anchor the badge to the header strip, right-aligned in the cell.
        let w = 8.0;
        let h = 7.0;
        let bx = (cell.x1 - w - 3.0).max(cell.x0 + 2.0);
        let by = layout.top + (layout.header_height - h) / 2.0;
        let cx = bx + w / 2.0;
        let mid = by + h * 0.55;
        let c = palette.selection;
        // Top edge (wide mouth), then both sides converge to the centre, then
        // a short stem downward.
        surface.stroke_line(bx, by, bx + w, by, c, 1.0);
        surface.stroke_line(bx, by, cx, mid, c, 1.0);
        surface.stroke_line(bx + w, by, cx, mid, c, 1.0);
        surface.stroke_line(cx, mid, cx, by + h, c, 1.0);
    }

    // -- 1c. Find matches (sec.5.G): a translucent highlight behind each matched
    //        cell so every hit is visible at a glance; the focused match is the
    //        active cell, so it also carries the active border drawn below.
    if !state.find_matches.is_empty() {
        for &(row, col) in &state.find_matches {
            if let Some((x, y, cw, ch)) = state.cell_pixel_rect(row, col) {
                surface.fill_rect(
                    Rect::new(x, y, x + cw, y + ch),
                    palette.selection.with_alpha(0.28),
                );
            }
        }
    }

    // -- 1d. Whole-axis selection band (sec.4bis.A). A virtual column/row
    //        selection has no on-screen rectangle bounding it, so fill the whole
    //        band over the grid area and tint its headers opaquely.
    let sel = &state.selection;
    if sel.kind != grid::SelKind::Cells {
        let (band_x0, band_x1, band_y0, band_y1, hx0, hx1, hy0, hy1) = match sel.kind {
            grid::SelKind::Cols => {
                let (c1, c2) = (
                    sel.anchor.1.min(sel.active.1),
                    sel.anchor.1.max(sel.active.1),
                );
                let x0 = state.col_x(c1).max(HEADER_WIDTH);
                let x1 = (state.col_x(c2) + state.col_width_of(c2)).min(state.canvas_width);
                // Band over the grid body, and the header tint over the strip.
                (
                    x0,
                    x1,
                    HEADER_HEIGHT,
                    state.grid_bottom(),
                    x0,
                    x1,
                    0.0,
                    HEADER_HEIGHT,
                )
            }
            _ => {
                let (r1, r2) = (
                    sel.anchor.0.min(sel.active.0),
                    sel.anchor.0.max(sel.active.0),
                );
                let y0 = state.row_y(r1).max(HEADER_HEIGHT);
                let y1 = (state.row_y(r2) + state.row_height_of(r2)).min(state.grid_bottom());
                (
                    HEADER_WIDTH,
                    state.canvas_width,
                    y0,
                    y1,
                    0.0,
                    HEADER_WIDTH,
                    y0,
                    y1,
                )
            }
        };
        if band_x1 > band_x0 && band_y1 > band_y0 {
            surface.fill_rect(
                Rect::new(band_x0, band_y0, band_x1, band_y1),
                palette.selection.with_alpha(0.16),
            );
        }
        if hx1 > hx0 && hy1 > hy0 {
            surface.fill_rect(Rect::new(hx0, hy0, hx1, hy1), palette.header_active);
        }
    }

    // -- 2. Selection (range fills + outlines) and active-cell border. --------
    // Lives in the overlay so it tracks the selection on every frame regardless
    // of scroll shifts in the persistent layer. A lone single cell shows only
    // the active border (no fill); any multi-cell or disjoint selection fills
    // each rectangle with a translucent tint and outlines it. Skipped for a
    // whole-axis band (drawn above): its `bounds()` collapses to row/col 0.
    if sel.kind == grid::SelKind::Cells && sel.is_multi() {
        for (r1, c1, r2, c2) in state.selection_rects() {
            if let Some((x, y, w, ht)) = state.range_pixel_rect(r1, c1, r2, c2) {
                let rect = Rect::new(x, y, x + w, y + ht);
                surface.fill_rect(rect, palette.selection.with_alpha(0.16));
                surface.stroke_rect(rect, palette.selection, 1.0);
            }
        }
    }
    // The active-cell 2px border is drawn only for a lone single-cell selection
    // (Excel behaviour): in a multi-cell / disjoint selection the range fill +
    // outline already mark the selection as one block, so a separate border on
    // the active cell would read as a spurious second selection following the
    // cursor. When the active cell sits in a merged block the border wraps the
    // whole block, so the merge reads as one selected cell.
    if sel.kind == grid::SelKind::Cells
        && !sel.is_multi()
        && let Some((x, y, cw, ch)) = state
            .active_merge_rect()
            .or_else(|| state.cell_pixel_rect(state.active_row(), state.active_col()))
    {
        surface.stroke_rect(Rect::new(x, y, x + cw, y + ch), palette.selection, 2.0);
    }

    // -- 2b. Clipboard marching-ants marquee (sec.5.E). A dashed rectangle around
    //        the copied/cut range, its dashes crawling so the marquee animates.
    if let Some((r1, c1, r2, c2)) = state.clipboard_anchor
        && let Some((x, y, w, ht)) = state.range_pixel_rect(r1, c1, r2, c2)
    {
        let phase = (clock.now_ms() / 40.0) % MARCHING_PERIOD;
        let c = palette.selection;
        dashed_h(surface, x, x + w, y, c, phase);
        dashed_h(surface, x, x + w, y + ht, c, phase);
        dashed_v(surface, y, y + ht, x, c, phase);
        dashed_v(surface, y, y + ht, x + w, c, phase);
    }

    // -- 2c. Autofill (sec.5.F): a dashed preview outline of the destination
    //        rectangle while the handle is dragged, then the small blue handle
    //        square at the selection's bottom-right corner.
    if let Some((r1, c1, r2, c2)) = state.fill_preview
        && let Some((x, y, w, ht)) = state.range_pixel_rect(r1, c1, r2, c2)
    {
        let c = palette.selection;
        dashed_h(surface, x, x + w, y, c, 0.0);
        dashed_h(surface, x, x + w, y + ht, c, 0.0);
        dashed_v(surface, y, y + ht, x, c, 0.0);
        dashed_v(surface, y, y + ht, x + w, c, 0.0);
    }
    if let Some((hx, hy, hw, hh)) = state.fill_handle_rect() {
        let r = Rect::new(hx, hy, hx + hw, hy + hh);
        surface.fill_rect(r, palette.selection);
        surface.stroke_rect(r, palette.content_bg, 1.0);
    }

    // -- 2d. Header resize guide (sec.4bis.C): a full-height/width line at the
    //        prospective boundary while a header border is dragged.
    match state.resize_preview {
        Some(grid::ResizePreview::Col(x)) => {
            surface.stroke_line(
                x,
                layout.top,
                x,
                state.grid_bottom(),
                palette.selection,
                1.0,
            );
        }
        Some(grid::ResizePreview::Row(y)) => {
            surface.stroke_line(0.0, y, state.canvas_width, y, palette.selection, 1.0);
        }
        None => {}
    }

    // -- 2e. Inline insert "+" badge on the hovered header boundary (sec.4bis.D).
    match state.insert_hint {
        // ColInsert(i) inserts left of column i, i.e. at column i's left edge.
        Some(grid::HeaderHit::ColInsert(i)) => {
            let bx = state.col_x(i);
            let cy = layout.top + layout.header_height * 0.5;
            draw_plus(surface, bx, cy, palette.selection);
        }
        Some(grid::HeaderHit::RowInsert(i)) => {
            let by = state.row_y(i);
            let cx = HEADER_WIDTH * 0.5;
            draw_plus(surface, cx, by, palette.selection);
        }
        _ => {}
    }
    let t_after_hdrs = clock.now_ms();

    // The perf HUD no longer paints here: fps + frame timings are published to
    // the DOM status bar (`ambre_canvas::PerfPublisher`) and rendered as plain
    // text, so we never shape debug glyphs on the GPU. Only the timing the view
    // feeds the snapshot is recorded.
    stats.hdrs_ms = (t_after_hdrs - t0) as f32;
}

// -----------------------------------------------------------------------------
// draw_edit -- in-progress cell edit (overlay, above everything)
// -----------------------------------------------------------------------------

/// Width in pixels of `text` shaped at the body size, or 0 for empty text.
fn measure(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    bold: bool,
    italic: bool,
) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let layout: &Layout<ParleyBrush> = cache.get_or_build(
        font_cx,
        text,
        BODY_SIZE,
        f32::MAX,
        Alignment::Start,
        bold,
        italic,
    );
    layout.width() as f64
}

/// Map a text-relative pixel x to the nearest char boundary in `text`. Walks
/// boundaries left to right and returns the one whose mid-glyph crossing the
/// target is closest -- i.e. the caret snaps to whichever gap the click is
/// nearer to. `text` is short (one cell), so the linear measure scan is cheap.
fn offset_at_x(
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    bold: bool,
    italic: bool,
    target: f64,
) -> usize {
    let mut prev_w = 0.0;
    let mut prev_off = 0;
    for (off, c) in text.char_indices() {
        let next_off = off + c.len_utf8();
        let w = measure(font_cx, cache, &text[..next_off], bold, italic);
        if target <= (prev_w + w) / 2.0 {
            return prev_off;
        }
        prev_w = w;
        prev_off = next_off;
    }
    text.len()
}

/// Draw the active edit session over the cell: a white field, the buffer with
/// its IME composition preview, the caret, and a dotted underline under the
/// composition. Called on the overlay layer after `draw_overlay`, so it sits
/// above the active-cell border and the grid. Clipped to the cell, which is
/// itself clipped to the scrollable area (never under the header strip).
pub fn draw_edit<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    editor: &mut CellEditor,
    caret_visible: bool,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
) {
    let Some((x, y, cw, ch)) = state.cell_pixel_rect(editor.row, editor.col) else {
        return; // active cell scrolled out of view; nothing to draw
    };

    // Match the cell's pinned formatting so the edit looks like the result.
    let (bold, italic) = state
        .cell_cache
        .get(&(editor.row, editor.col))
        .map(|c| (c.bold, c.italic))
        .unwrap_or((false, false));

    // Resolve a deferred pointer hit now that we have font metrics: map its
    // physical x to a byte offset and apply the click/drag/word-select intent.
    if let Some(hit) = editor.pending.take() {
        let text_x = (hit.x - (x + CELL_PAD_X)).max(0.0);
        let off = offset_at_x(font_cx, cache, &editor.buffer, bold, italic, text_x);
        match hit.kind {
            HitKind::Place => editor.place_caret(off),
            HitKind::Extend => editor.extend_to(off),
            HitKind::Word => editor.select_word(off),
        }
    }

    let cell = Rect::new(x, y, x + cw, y + ch);
    surface.push_clip(cell);

    // Content-coloured field over the underlying cell + active border interior.
    surface.fill_rect(cell, palette.content_bg);

    // Selection highlight behind the text: a translucent band from the start to
    // the end of the selected range.
    if let Some((s, e)) = editor.selection() {
        let sx = x + CELL_PAD_X + measure(font_cx, cache, &editor.buffer[..s], bold, italic);
        let ex = x + CELL_PAD_X + measure(font_cx, cache, &editor.buffer[..e], bold, italic);
        surface.fill_rect(
            Rect::new(sx, y + 2.0, ex, y + ch - 2.0),
            palette.selection.with_alpha(0.30),
        );
    }

    // Text: prefix + preview + suffix, baseline-centred in the row.
    let display = editor.display();
    if !display.is_empty() {
        let layout: &Layout<ParleyBrush> = cache.get_or_build(
            font_cx,
            &display,
            BODY_SIZE,
            f32::MAX,
            Alignment::Start,
            bold,
            italic,
        );
        let text_h = layout.height() as f64;
        let y_off = ((ch - text_h) / 2.0).max(0.0);
        let brush = Brush::Solid(palette.cell_text);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let tx = x + CELL_PAD_X + run.offset() as f64;
                let ty = y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
    }

    // Composition underline: a dotted line under the preview run.
    if !editor.preview.is_empty() {
        let pre_w = measure(font_cx, cache, &editor.buffer[..editor.caret], bold, italic);
        let prev_w = measure(font_cx, cache, &editor.preview, bold, italic);
        let ux0 = x + CELL_PAD_X + pre_w;
        let ux1 = ux0 + prev_w;
        let uy = (y + ch - 3.0).min(y + ch);
        // Dotted: 3px dash, 2px gap.
        let mut dx = ux0;
        while dx < ux1 {
            let seg_end = (dx + 3.0).min(ux1);
            surface.stroke_line(dx, uy, seg_end, uy, palette.selection, 1.0);
            dx += 5.0;
        }
    }

    // Caret: a 1px vertical bar after the prefix + preview, drawn only on the
    // visible half of the blink cycle.
    if caret_visible {
        let caret_w = measure(font_cx, cache, &editor.before_caret(), bold, italic);
        let caret_x = x + CELL_PAD_X + caret_w;
        surface.stroke_line(caret_x, y + 3.0, caret_x, y + ch - 3.0, palette.caret, 1.0);
    }

    surface.pop_clip();
}

// -----------------------------------------------------------------------------
// draw_sheet_tabs -- bottom tab strip (overlay, spec sec.13)
// -----------------------------------------------------------------------------

/// Vertical padding from the tab top to the label baseline area.
const TAB_LABEL_PAD_X: f64 = 8.0;

/// Draw the bottom sheet-tab strip: one slot per sheet (active highlighted),
/// the trailing "+" button, an optional in-place rename field, and a drop marker
/// while a tab is being dragged. Lives on the overlay so it redraws with the
/// chrome each dirty frame.
#[allow(clippy::too_many_arguments)]
pub fn draw_sheet_tabs<S: DrawSurface>(
    surface: &mut S,
    state: &TrameUiState,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    rename: Option<&TabRename>,
    drag: Option<&TabDrag>,
    caret_on: bool,
) {
    let strip_y = state.grid_bottom();
    let w = state.canvas_width;
    let bottom = state.canvas_height;

    // Strip background across the full width.
    surface.fill_rect(Rect::new(0.0, strip_y, w, bottom), palette.header_bg);
    // Top hairline separating the strip from the grid.
    surface.stroke_line(0.0, strip_y, w, strip_y, palette.grid_line, 1.0);

    for (i, (id, name)) in state.sheets.iter().enumerate() {
        let (x0, x1) = state.tab_slot_x(i);
        if x0 >= w {
            break; // off the right edge; no horizontal tab scroll in the MVP
        }
        let active = *id == state.active_sheet;
        let bg = if active {
            palette.content_bg
        } else {
            palette.header_bg
        };
        surface.fill_rect(Rect::new(x0, strip_y, x1, bottom), bg);

        // Active tab carries a thicker accent on its top edge.
        let (border_color, border_w) = if active {
            (palette.selection, 2.0)
        } else {
            (palette.grid_line, 1.0)
        };
        surface.stroke_line(x0, strip_y, x1, strip_y, border_color, border_w);
        // Right separator between slots.
        surface.stroke_line(x1, strip_y, x1, bottom, palette.grid_line, 1.0);

        // Label, or the rename field when this tab is being renamed.
        let label_y = strip_y + (TAB_STRIP_HEIGHT - BODY_SIZE as f64) / 2.0;
        if let Some(r) = rename.filter(|r| r.sheet_id == *id) {
            let text = r.editor.display();
            if text_enabled && !text.is_empty() {
                render::draw_text(
                    surface,
                    font_cx,
                    text_cache,
                    &text,
                    BODY_SIZE,
                    (TAB_WIDTH - TAB_LABEL_PAD_X * 2.0) as f32,
                    Alignment::Start,
                    x0 + TAB_LABEL_PAD_X,
                    label_y,
                    palette.cell_text,
                );
            }
            // Caret after the committed prefix + IME preview.
            if caret_on {
                let cx = x0
                    + TAB_LABEL_PAD_X
                    + measure(font_cx, text_cache, &r.editor.before_caret(), false, false);
                surface.stroke_line(cx, strip_y + 4.0, cx, bottom - 4.0, palette.caret, 1.0);
            }
        } else if text_enabled && !name.is_empty() {
            let color = if active {
                palette.cell_text
            } else {
                palette.header_text
            };
            render::draw_text(
                surface,
                font_cx,
                text_cache,
                name,
                BODY_SIZE,
                (TAB_WIDTH - TAB_LABEL_PAD_X * 2.0) as f32,
                Alignment::Start,
                x0 + TAB_LABEL_PAD_X,
                label_y,
                color,
            );
        }
    }

    // Trailing "+" button.
    let (ax0, ax1) = state.add_button_x();
    if ax0 < w {
        surface.fill_rect(Rect::new(ax0, strip_y, ax1, bottom), palette.header_bg);
        surface.stroke_line(ax1, strip_y, ax1, bottom, palette.grid_line, 1.0);
        if text_enabled {
            let label_y = strip_y + (TAB_STRIP_HEIGHT - BODY_SIZE as f64) / 2.0;
            render::draw_text(
                surface,
                font_cx,
                text_cache,
                "+",
                BODY_SIZE,
                ADD_BTN_WIDTH as f32,
                Alignment::Center,
                ax0,
                label_y,
                palette.header_text,
            );
        }
    }

    // Drop marker: a vertical accent at the slot the dragged tab would land on.
    if let Some(d) = drag.filter(|d| d.moved) {
        let drop = state.drop_index_at(d.cur_x) as usize;
        let (mx, _) = state.tab_slot_x(drop);
        surface.stroke_line(mx, strip_y, mx, bottom, palette.selection, 2.0);
    }
}

/// Draw an open tab context menu (`{ Rename, Delete }`) over everything.
pub fn draw_tab_menu<S: DrawSurface>(
    surface: &mut S,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    menu: &TabMenu,
) {
    let h = TAB_MENU_ITEM_HEIGHT * 2.0;
    let area = Rect::new(menu.x, menu.y, menu.x + TAB_MENU_WIDTH, menu.y + h);
    surface.fill_rect(area, palette.content_bg);
    surface.stroke_rect(area, palette.grid_line, 1.0);

    for (i, label) in ["Rename", "Delete"].into_iter().enumerate() {
        let y = menu.y + i as f64 * TAB_MENU_ITEM_HEIGHT;
        render::draw_text(
            surface,
            font_cx,
            text_cache,
            label,
            BODY_SIZE,
            (TAB_MENU_WIDTH - 16.0) as f32,
            Alignment::Start,
            menu.x + 8.0,
            y + (TAB_MENU_ITEM_HEIGHT - BODY_SIZE as f64) / 2.0,
            palette.cell_text,
        );
    }
}

// -----------------------------------------------------------------------------
// Headless tests -- mock DrawSurface (ambre-spec-canvas P0 "Headless tests").
// -----------------------------------------------------------------------------
//
// These drive the real grid renderer against `RecordingSurface` (a recording
// `DrawSurface`) and `FixedClock`, entirely off the browser main thread. They
// assert on the *sequence* of draw calls -- geometry, palette colours, clip
// balance -- not on pixels. This is the unit-test counterpart to the on-screen
// wgpu path; it needs no GPU, no canvas, no `performance.now()`.
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::FontContext;
    use ambre_draw::testing::{DrawOp, FixedClock, RecordingSurface};

    use super::super::grid::CellData;

    /// A view state large enough to expose several rows/cols, with the given
    /// cells pre-populated in the cache.
    fn fixture(cells: &[(u32, u32, &str)]) -> TrameUiState {
        let mut state = TrameUiState::new(800.0, 600.0);
        for &(row, col, text) in cells {
            state.cell_cache.insert(
                (row, col),
                CellData {
                    text: text.to_owned(),
                    raw: text.to_owned(),
                    ..Default::default()
                },
            );
        }
        state
    }

    /// Shared harness: a recording surface, a fixed clock, a real (empty) font
    /// context and text cache. Text shaping may yield zero glyphs on a host with
    /// no fonts, so tests never assert glyph *counts* -- only structural ops and
    /// the renderer's own `cells_drawn` tally, which is font-independent.
    fn harness() -> (
        RecordingSurface,
        FixedClock,
        FontContext,
        TextCache,
        DrawStats,
    ) {
        (
            RecordingSurface::new(),
            FixedClock::default(),
            FontContext::new(),
            TextCache::new(1.0),
            DrawStats::default(),
        )
    }

    #[test]
    fn draw_cells_full_repaint_paints_pending_then_grid() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        // First op is the full-area pending-grey fill.
        assert_eq!(
            surface.ops().first(),
            Some(&DrawOp::FillRect {
                rect: Rect::new(HEADER_WIDTH, HEADER_HEIGHT, 800.0, state.grid_bottom(),),
                color: palette.content_pending,
            }),
        );
        // Grid lines are stroked with the palette grid colour.
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line
            )) > 0
        );
        // The clip stack is balanced and no cell bg was painted (empty cache).
        assert_eq!(surface.clip_depth(), 0);
        assert_eq!(stats.cells_drawn, 0);
    }

    #[test]
    fn draw_cells_scroll_strip_base_is_blank_not_pending_grey() {
        // A scroll strip repaints a band entering the viewport on top of the
        // shifted pixels. Its base fill must be the blank content background, not
        // the grey "pending" veil: a cell scrolled in faster than the Logic
        // round-trip should read as empty until its data lands, never flash grey
        // (the source of the Arrow/Page Up/wheel scroll flicker).
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        // One row entering at the top: [HEADER_HEIGHT, HEADER_HEIGHT + ROW_HEIGHT].
        let strip = Rect::new(
            HEADER_WIDTH,
            HEADER_HEIGHT,
            800.0,
            HEADER_HEIGHT + ROW_HEIGHT,
        );
        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            Some(strip),
            &clock,
            &mut stats,
        );

        // The strip base fill is content_bg, and the grey pending veil never
        // appears during a scroll repaint.
        assert_eq!(
            surface.ops().first(),
            Some(&DrawOp::FillRect {
                rect: strip,
                color: palette.content_bg,
            }),
        );
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.content_pending
            )),
            0,
        );
    }

    #[test]
    fn draw_cells_fills_cached_cell_and_counts_text() {
        // Two cells in view: one with text, one empty (pending stays, but its bg
        // is filled because it is "received").
        let state = fixture(&[(0, 0, "hello"), (1, 1, "")]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        // Both received cells get a content_bg fill...
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.content_bg
            )),
            2,
        );
        // ...but only the non-empty one is tallied as drawn text.
        assert_eq!(stats.cells_drawn, 1);
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn draw_cells_renders_merge_master_over_its_span() {
        // A 2x2 merge master at (0,0) plus its three blank covered cells.
        let mut state = fixture(&[(0, 0, "merged"), (0, 1, ""), (1, 0, ""), (1, 1, "")]);
        state.cell_cache.get_mut(&(0, 0)).unwrap().merge_span = Some((2, 2));
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        // The merge pass strokes the span's outer border with the grid colour as
        // a rect (the per-cell separators are only ever stroked as lines).
        let layout = screen_layout(&state);
        let span_rect = layout.merge_rect(0, 0, 2, 2);
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeRect { rect, color, .. }
                    if *color == palette.grid_line && *rect == span_rect
            )) == 1
        );
        // The span gets an opaque content_bg fill covering its full rectangle.
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { rect, color }
                    if *color == palette.content_bg && *rect == span_rect
            )) == 1
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn draw_cells_strip_skips_when_outside_grid() {
        // A strip entirely inside the header corner produces no cell drawing.
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        let header_corner = Rect::new(0.0, 0.0, HEADER_WIDTH, HEADER_HEIGHT);
        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            Some(header_corner),
            &clock,
            &mut stats,
        );

        assert!(surface.ops().is_empty());
        assert_eq!(stats.cells_drawn, 0);
    }

    #[test]
    fn draw_overlay_draws_headers_and_active_border() {
        let state = fixture(&[]);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            &clock,
            &mut stats,
        );

        // Header backgrounds use the palette header colour.
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
            )) >= 2
        );
        // The active cell (0,0) border is stroked with the selection colour.
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeRect { color, width, .. }
                    if *color == palette.selection && *width == 2.0
            )),
            1,
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn multi_cell_selection_fills_and_outlines_each_range() {
        // A live range (0,0)..=(1,1) plus a disjoint single cell (3,3): both get
        // a translucent selection fill, and no 2px active-cell border is drawn
        // (Excel behaviour: the range outline alone marks the selection).
        let mut state = fixture(&[]);
        state.select(0, 0);
        state.select_extend(1, 1);
        state.select_disjoint(3, 3);
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_overlay(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            &clock,
            &mut stats,
        );

        // One translucent fill per selected rectangle (2 rects).
        let tint = palette.selection.with_alpha(0.16);
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == tint
            )),
            2,
        );
        // No 2px active-cell border in a multi-cell selection.
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeRect { color, width, .. }
                    if *color == palette.selection && *width == 2.0
            )),
            0,
        );
    }

    #[test]
    fn dirty_cell_gets_a_translucent_veil() {
        // An optimistic edit marks (0,0) dirty -> a content_bg veil at reduced
        // alpha is painted over it, on top of the opaque cell fill.
        let mut state = fixture(&[(0, 0, "x")]);
        state.set_cell_text(0, 0, "5".to_owned());
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        let veil = palette.content_bg.with_alpha(super::DIRTY_VEIL_ALPHA);
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == veil
            )),
            1,
        );
        assert_ne!(
            veil, palette.content_bg,
            "veil must differ from the opaque fill"
        );
    }

    #[test]
    fn errored_cell_gets_a_corner_marker() {
        // A formula error on (1,1) draws the corner marker with the error colour;
        // a clean cell draws none.
        let mut state = fixture(&[(1, 1, "")]);
        state.set_formula_error(1, 1, "#DIV/0!".to_owned());
        let palette = Theme::Light.palette();
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        assert!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeLine { color, .. } if *color == palette.cell_error
            )) > 0
        );
    }

    #[test]
    fn sheet_tabs_fill_active_slot_and_draw_strip() {
        // Two sheets, the second active: the active slot is filled with
        // content_bg and accented with the selection colour on its top edge.
        let mut state = fixture(&[]);
        state.on_sheet_list(
            vec![("s0".into(), "One".into()), ("s1".into(), "Two".into())],
            "s1".into(),
        );
        let palette = Theme::Light.palette();
        let (mut surface, _clock, mut font_cx, mut cache, _stats) = harness();

        draw_sheet_tabs(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            None,
            true,
        );

        // The active slot gets a content_bg fill (inactive slots use header_bg).
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.content_bg
            )) >= 1
        );
        // The active accent uses the selection colour at 2px.
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeLine { color, width, .. }
                    if *color == palette.selection && *width == 2.0
            )) >= 1
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn sheet_tab_rename_draws_caret_over_the_target_tab() {
        // An active rename on "s0" draws a caret (palette.caret) in the strip.
        let mut state = fixture(&[]);
        state.on_sheet_list(vec![("s0".into(), "One".into())], "s0".into());
        let palette = Theme::Light.palette();
        let (mut surface, _clock, mut font_cx, mut cache, _stats) = harness();

        let rename = super::TabRename {
            sheet_id: "s0".into(),
            editor: CellEditor::new(0, 0, "On".to_owned()),
        };
        draw_sheet_tabs(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            Some(&rename),
            None,
            /* caret_on */ true,
        );
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeLine { color, .. } if *color == palette.caret
            )) >= 1
        );

        // With the caret off (blink), no caret line is drawn.
        let (mut surface, _c, mut f, mut ch, _s) = harness();
        draw_sheet_tabs(
            &mut surface,
            &state,
            palette,
            &mut f,
            &mut ch,
            true,
            Some(&rename),
            None,
            false,
        );
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeLine { color, .. } if *color == palette.caret
            )),
            0,
        );
    }

    #[test]
    fn sheet_tab_drag_draws_drop_marker_only_when_moved() {
        let mut state = fixture(&[]);
        state.on_sheet_list(
            vec![("s0".into(), "A".into()), ("s1".into(), "B".into())],
            "s0".into(),
        );
        let palette = Theme::Light.palette();

        let strip_y = state.grid_bottom();
        let bottom = state.canvas_height;
        // The drop marker is the *vertical* (x0 == x1) 2px selection line that
        // spans the strip -- distinct from the active tab's horizontal accent,
        // which is also a 2px selection line but along the top edge.
        let drop_marker = move |op: &DrawOp| {
            matches!(
                op, DrawOp::StrokeLine { x0, y0, x1, y1, color, width }
                    if *color == palette.selection && *width == 2.0
                        && (*x0 - *x1).abs() < 0.01 && *y0 == strip_y && *y1 == bottom
            )
        };

        // A drag that has moved draws the marker at the drop slot's left edge.
        let moved = super::TabDrag {
            sheet_id: "s0".into(),
            grab_x: 5.0,
            cur_x: 150.0,
            moved: true,
        };
        let (mut surface, _c, mut f, mut ch, _s) = harness();
        draw_sheet_tabs(
            &mut surface,
            &state,
            palette,
            &mut f,
            &mut ch,
            true,
            None,
            Some(&moved),
            true,
        );
        let expected_x = state.tab_slot_x(state.drop_index_at(150.0) as usize).0;
        assert_eq!(surface.count(drop_marker), 1);
        assert!(surface.ops().iter().any(|op| matches!(
            op, DrawOp::StrokeLine { x0, .. } if (*x0 - expected_x).abs() < 0.01
        )));

        // A grab that has not moved yet draws no drop marker.
        let still = super::TabDrag {
            sheet_id: "s0".into(),
            grab_x: 5.0,
            cur_x: 6.0,
            moved: false,
        };
        let (mut surface, _c, mut f, mut ch, _s) = harness();
        draw_sheet_tabs(
            &mut surface,
            &state,
            palette,
            &mut f,
            &mut ch,
            true,
            None,
            Some(&still),
            true,
        );
        assert_eq!(surface.count(drop_marker), 0);
    }

    #[test]
    fn renderer_uses_only_palette_colors_for_fills() {
        // Guards the "no hardcoded product colour" invariant on the cells layer:
        // every fill colour must come from the active palette (the HUD, which
        // does use literals, lives only in the overlay).
        let state = fixture(&[(0, 0, "x")]);
        let palette = Theme::Light.palette();
        let allowed = [palette.content_pending, palette.content_bg];
        let (mut surface, clock, mut font_cx, mut cache, mut stats) = harness();

        draw_cells(
            &mut surface,
            &state,
            palette,
            &mut font_cx,
            &mut cache,
            true,
            None,
            &clock,
            &mut stats,
        );

        for color in surface.fill_colors() {
            assert!(allowed.contains(&color), "unexpected fill colour {color:?}");
        }
    }
}
