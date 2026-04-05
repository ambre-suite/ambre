// ambre-app/src/trame/renderer.rs
// ============================================================================
// Trame grid renderer -- UI Thread, Vello backend.
//
// `VelloSurface` wraps a `&mut vello::Scene` and implements `DrawSurface`
// (the GPU-agnostic trait from ambre-draw). This is the concrete implementation
// for the screen; a `KrillaSurface` (for PDF export) will live in the Logic
// Worker when that feature is implemented.
//
// `draw_grid` renders one frame of the Trame grid into a `vello::Scene`:
//   1. White background
//   2. Grey "pending" area for the grid
//   3. White fill for cells that are in the cache (data received)
//   4. Vertical and horizontal grid lines
//   5. Column/row header strips
//   6. Active cell/header highlights
//   7. Active cell border (drawn last, always on top)
//
// No text is rendered in this first iteration -- text requires a parley
// FontContext with loaded font bytes and will be added in a follow-up.
// The grey → white transition is sufficient to observe the IPC latency.
// ============================================================================

use ambre_draw::{Color, DrawSurface, Fill, Line, Rect, Stroke};
use vello::kurbo::Affine;

use super::grid::{
    COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TOTAL_COLS, TOTAL_ROWS,
};
use super::TrameUiState;

// -----------------------------------------------------------------------------
// Colors
// -----------------------------------------------------------------------------

const COLOR_WHITE: Color = Color::from_rgba8(255, 255, 255, 255);
const COLOR_PENDING: Color = Color::from_rgba8(235, 237, 240, 255); // light grey
const COLOR_HEADER_BG: Color = Color::from_rgba8(242, 244, 248, 255); // blue-grey tint
const COLOR_HEADER_ACTIVE: Color = Color::from_rgba8(207, 220, 240, 255); // active col/row tint
const COLOR_GRID_LINE: Color = Color::from_rgba8(210, 213, 218, 255);
const COLOR_HEADER_BORDER: Color = Color::from_rgba8(178, 183, 193, 255);
const COLOR_ACTIVE_BORDER: Color = Color::from_rgba8(60, 120, 210, 255); // blue

// -----------------------------------------------------------------------------
// VelloSurface
// -----------------------------------------------------------------------------

/// Wraps a `vello::Scene` and implements the `DrawSurface` trait so that
/// rendering code written against `DrawSurface` can target Vello without
/// knowing about the underlying GPU pipeline.
pub struct VelloSurface<'s> {
    pub scene: &'s mut vello::Scene,
}

impl<'s> VelloSurface<'s> {
    pub fn new(scene: &'s mut vello::Scene) -> Self {
        Self { scene }
    }
}

impl DrawSurface for VelloSurface<'_> {
    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        self.scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &rect);
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        self.scene.stroke(
            &Stroke::new(width),
            Affine::IDENTITY,
            color,
            None,
            &Line::new((x0, y0), (x1, y1)),
        );
    }
}

// -----------------------------------------------------------------------------
// draw_grid
// -----------------------------------------------------------------------------

/// Render one frame of the Trame grid into `scene`.
///
/// Called every `requestAnimationFrame` from the render loop in `ambre-app/src/lib.rs`.
pub fn draw_grid(scene: &mut vello::Scene, state: &TrameUiState) {
    let mut s = VelloSurface::new(scene);
    let w = state.canvas_width;
    let h = state.canvas_height;

    // 1. White canvas background.
    s.fill_rect(Rect::new(0.0, 0.0, w, h), COLOR_WHITE);

    // 2. Grey "pending" fill for the entire grid area.
    //    Cells that arrive from the Logic Worker will overwrite this with white.
    s.fill_rect(Rect::new(HEADER_WIDTH, HEADER_HEIGHT, w, h), COLOR_PENDING);

    // 3. White fill for received cells.
    let row_end = (state.top_row + state.viewport_rows + 1).min(TOTAL_ROWS);
    let col_end = (state.left_col + state.viewport_cols + 1).min(TOTAL_COLS);

    for row in state.top_row..row_end {
        for col in state.left_col..col_end {
            if state.cell_cache.contains_key(&(row, col)) {
                let x = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                let y = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                s.fill_rect(Rect::new(x, y, x + COL_WIDTH, y + ROW_HEIGHT), COLOR_WHITE);
            }
        }
    }

    // 4. Vertical grid lines.
    let n_vcols = state.viewport_cols + 2;
    for i in 0..=n_vcols {
        let col = state.left_col + i;
        if col > TOTAL_COLS {
            break;
        }
        let x = HEADER_WIDTH + i as f64 * COL_WIDTH;
        if x > w {
            break;
        }
        s.stroke_line(x, HEADER_HEIGHT, x, h, COLOR_GRID_LINE, 1.0);
    }

    // 5. Horizontal grid lines.
    let n_hrows = state.viewport_rows + 2;
    for i in 0..=n_hrows {
        let row = state.top_row + i;
        if row > TOTAL_ROWS {
            break;
        }
        let y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
        if y > h {
            break;
        }
        s.stroke_line(HEADER_WIDTH, y, w, y, COLOR_GRID_LINE, 1.0);
    }

    // 6a. Active column header tint (drawn before the header background covers it).
    //     We paint into the header strip area directly.
    let active_col_visible = state.active_col >= state.left_col
        && state.active_col < state.left_col + state.viewport_cols;
    if active_col_visible {
        let col_off = (state.active_col - state.left_col) as f64;
        let x = HEADER_WIDTH + col_off * COL_WIDTH;
        s.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 6b. Active row header tint.
    let active_row_visible = state.active_row >= state.top_row
        && state.active_row < state.top_row + state.viewport_rows;
    if active_row_visible {
        let row_off = (state.active_row - state.top_row) as f64;
        let y = HEADER_HEIGHT + row_off * ROW_HEIGHT;
        s.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 7. Column header background strip (covers grid lines at the top).
    s.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);

    // 8. Row header background strip (covers grid lines on the left).
    s.fill_rect(Rect::new(0.0, 0.0, HEADER_WIDTH, h), COLOR_HEADER_BG);

    // Re-apply active column/row tints on top of the header background.
    if active_col_visible {
        let col_off = (state.active_col - state.left_col) as f64;
        let x = HEADER_WIDTH + col_off * COL_WIDTH;
        s.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }
    if active_row_visible {
        let row_off = (state.active_row - state.top_row) as f64;
        let y = HEADER_HEIGHT + row_off * ROW_HEIGHT;
        s.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 9. Header separator lines (bottom of column strip, right of row strip).
    s.stroke_line(0.0, HEADER_HEIGHT, w, HEADER_HEIGHT, COLOR_HEADER_BORDER, 1.0);
    s.stroke_line(HEADER_WIDTH, 0.0, HEADER_WIDTH, h, COLOR_HEADER_BORDER, 1.0);

    // 10. Active cell blue border (drawn last so it is always on top).
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        s.stroke_rect(Rect::new(x, y, x + cw, y + ch), COLOR_ACTIVE_BORDER, 2.0);
    }
}
