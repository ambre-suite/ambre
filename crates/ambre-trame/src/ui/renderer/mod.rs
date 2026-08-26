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

use ambre_draw::{Clock, Color, DrawSurface, FontContext, Rect};

use ambre_canvas::{Palette, TextCache};

use super::TrameUiState;
use super::grid::{self, COL_WIDTH, CellEditor, ROW_HEIGHT};
use crate::render::{self, BODY_SIZE, CELL_PAD_X, GridLayout};
use crate::{TOTAL_COLS, TOTAL_ROWS};

// Rendering entry points are split by layer; shared layout/stats/helpers stay
// here and reach the sub-modules via the glob mesh.
mod cells;
mod edit;
mod overlay;

pub use cells::draw_cells;
pub use edit::draw_edit;
pub use overlay::draw_overlay;
// Exposed to the test module (which calls it as `super::mdi_filter_polygon`).
#[cfg(test)]
use overlay::mdi_filter_polygon;

/// Build the shared [`GridLayout`] for the current screen viewport. Carries the
/// per-axis width/height maps (`trame-resize`) so resized axes reflow; the
/// cell-aligned scroll position is the layout origin.
fn screen_layout(state: &TrameUiState) -> GridLayout<'_> {
    GridLayout {
        // Collapse the row axis over filter-hidden rows (`trame-shared-filter`).
        hidden_rows: &state.hidden_rows,
        // Header strips zoom with the grid: pass the already-scaled sizes (the
        // layout multiplies only the content-area metrics by `zoom`).
        header_width: state.header_w(),
        header_height: state.header_h(),
        top: 0.0,
        col_width: COL_WIDTH,
        row_height: ROW_HEIGHT,
        col_widths: &state.col_widths,
        row_heights: &state.row_heights,
        origin_col: state.left_col,
        origin_row: state.top_row,
        width: state.canvas_width,
        // The grid stops above the bottom sheet-tab strip (`trame-sheets`).
        height: state.grid_bottom(),
        total_cols: TOTAL_COLS,
        total_rows: TOTAL_ROWS,
        zoom: state.zoom,
    }
}

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
    /// Read against the keystroke-to-draft budget of `ambre-spec-budgets.md`.
    pub input_latency_ms: f32,
}

/// Opacity of the veil painted over a dirty (awaiting-recompute) cell. The
/// content_bg-coloured veil fades the optimistic value toward the background.
const DIRTY_VEIL_ALPHA: f32 = 0.5;

/// Leg length, in physical pixels, of the right-triangle error marker drawn in
/// a cell's top-right corner.
const ERROR_MARK_SIZE: f64 = 7.0;

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
    let cells_top = state.header_h();
    // Walk the variable column widths from the scroll origin: skip columns wholly
    // left of the strip, then extend until the strip's right edge is covered.
    let mut col_start = state.left_col;
    let mut cx = state.header_w();
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

#[cfg(test)]
mod tests {
    use super::*;
    // Header strip metrics: only the tests still reference the raw constants
    // (production code goes through `state.header_w()` / `header_h()`).
    use super::grid::{HEADER_HEIGHT, HEADER_WIDTH};
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
    fn autofilter_buttons_drawn_on_region_header_row() {
        // A two-column data block around the active cell (0,0): each column draws
        // one autofilter button (bordered with the grid colour) on the band row.
        let state = fixture(&[(0, 0, "a"), (0, 1, "b")]);
        assert_eq!(state.current_region(), Some((0, 0, 0, 1)));
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

        // One bordered button per region column (cell grid lines are stroke_line,
        // so a grid-colour StrokeRect is unambiguously an autofilter button).
        assert_eq!(
            surface.count(|op| matches!(
                op, DrawOp::StrokeRect { color, .. } if *color == palette.grid_line
            )),
            2,
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn autofilter_button_shows_funnel_on_filtered_column() {
        // With a filter active on column 0, that button paints the MdiFilter
        // funnel glyph as selection-coloured scanline FillRects (the GPU surface
        // has no fill_path); the secondary letter-header badge fills it too. An
        // unfiltered fixture draws no selection-coloured fills here.
        let mut state = fixture(&[(0, 0, "a"), (0, 1, "b")]);
        state.filtered_col = Some(0);
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

        // Several selection-coloured fill spans make up the funnel silhouette.
        assert!(
            surface.count(|op| matches!(
                op, DrawOp::FillRect { color, .. } if *color == palette.selection
            )) > 2
        );
        assert_eq!(surface.clip_depth(), 0);
    }

    #[test]
    fn mdi_filter_polygon_parses_into_unit_box() {
        // The embedded MdiFilter path flattens to at least one non-empty contour,
        // normalized inside the unit box so the scanline fill maps cleanly.
        let contours = super::mdi_filter_polygon();
        assert!(contours.iter().any(|c| c.len() >= 3));
        for c in contours {
            for &(x, y) in c {
                assert!((-1e-6..=1.0 + 1e-6).contains(&x), "x out of unit box: {x}");
                assert!((-1e-6..=1.0 + 1e-6).contains(&y), "y out of unit box: {y}");
            }
        }
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
