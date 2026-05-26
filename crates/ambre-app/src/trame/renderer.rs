// ambre-app/src/trame/renderer.rs
// ============================================================================
// Trame grid draw_grid -- UI Thread, WgpuSurface backend.
//
// draw_grid is generic over DrawSurface so the same code path drives both
// WgpuSurface (screen) and KrillaSurface (PDF export, ambre-logic).
//
// Text layout is provided by TextCache (cached Parley layouts) and glyph
// rasterisation by SwashScaler into the GlyphAtlas (R8Unorm GPU texture).
// ============================================================================

use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

use ambre_ui::TextCache;

use super::grid::{COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TOTAL_COLS, TOTAL_ROWS};
use super::TrameUiState;

// -----------------------------------------------------------------------------
// Colors
// -----------------------------------------------------------------------------

const COLOR_WHITE:         Color = Color::from_rgba8(255, 255, 255, 255);
const COLOR_PENDING:       Color = Color::from_rgba8(235, 237, 240, 255);
const COLOR_HEADER_BG:     Color = Color::from_rgba8(242, 244, 248, 255);
const COLOR_HEADER_ACTIVE: Color = Color::from_rgba8(207, 220, 240, 255);
const COLOR_GRID_LINE:     Color = Color::from_rgba8(210, 213, 218, 255);
const COLOR_HEADER_BORDER: Color = Color::from_rgba8(178, 183, 193, 255);
const COLOR_ACTIVE_BORDER: Color = Color::from_rgba8(60,  120, 210, 255);
const COLOR_CELL_TEXT:     Color = Color::from_rgba8(51,  51,  51,  255);
const COLOR_HEADER_TEXT:   Color = Color::from_rgba8(85,  85,  85,  255);

// -----------------------------------------------------------------------------
// DrawStats — per-frame breakdown of draw_grid
// -----------------------------------------------------------------------------

/// Sub-phase timings for one `draw_grid` invocation.
///
/// All times are wall-clock milliseconds measured via `performance.now()`,
/// so they include CPU time spent in lyon tessellation, parley shaping,
/// glyph rasterisation, vertex batching, and our own iteration overhead.
/// The GPU `flush` / `present` happens *after* `draw_grid` returns and is
/// measured separately as `last_gpu_ms` in `UiState`.
#[derive(Default, Clone, Copy)]
pub struct DrawStats {
    /// Phases 1-10: white background, pending area, received-cell fills,
    /// grid lines, header backgrounds + active-row/col tints, active-cell
    /// border. All `fill_rect` / `stroke_line` / `stroke_rect`.
    pub bg_ms: f32,
    /// Phase 11a: cell body text (parley shaping + push_clip + draw_glyphs
    /// + pop_clip per non-empty cell). Zero when `text_enabled` is false.
    pub cells_ms: f32,
    /// Phases 11b + 11c: column header labels (A, B, ...) and row header
    /// numbers (1, 2, ...). Zero when `text_enabled` is false.
    pub hdrs_ms: f32,
    /// Phase 12: the on-canvas HUD overlay itself.
    pub hud_ms: f32,
    /// Number of cells with non-empty text actually drawn this frame.
    pub cells_drawn: u32,
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

fn col_label(col: u32) -> String {
    let mut label = String::new();
    let mut n = col + 1;
    while n > 0 {
        n -= 1;
        label.push(char::from_u32(b'A' as u32 + n % 26).unwrap());
        n /= 26;
    }
    label.chars().rev().collect()
}

/// Iterate the positioned glyph runs in a Parley layout and call draw_glyphs
/// for each run. This is the correct DrawSurface path; text_cache no longer
/// holds a direct reference to any rendering backend.
#[allow(clippy::too_many_arguments)]
fn draw_text<S: DrawSurface>(
    surface:   &mut S,
    font_cx:   &mut FontContext,
    cache:     &mut TextCache,
    text:      &str,
    font_size: f32,
    max_width: f32,
    align:     Alignment,
    origin_x:  f64,
    origin_y:  f64,
    color:     Color,
) {
    if text.is_empty() {
        return;
    }
    let layout: &Layout<ParleyBrush> =
        cache.get_or_build(font_cx, text, font_size, max_width, align);
    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            let tx = origin_x + run.offset() as f64;
            let ty = origin_y + run.baseline() as f64;
            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
        }
    }
}

// -----------------------------------------------------------------------------
// draw_grid
// -----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn draw_grid<S: DrawSurface>(
    surface:        &mut S,
    state:          &TrameUiState,
    font_cx:        &mut FontContext,
    text_cache:     &mut TextCache,
    text_enabled:   bool,
    render_fps:     f32,
    last_frame_ms:  f32,
    last_drain_ms:  f32,
    last_draw_ms:   f32,
    last_gpu_ms:    f32,
    perf:           &web_sys::Performance,
    stats:          &mut DrawStats,
) {
    let w = state.canvas_width;
    let h = state.canvas_height;

    // -------------------------------------------------------------------------
    // Phase BG: backgrounds, fills, grid lines, header tints, active border.
    // No text, no parley, no atlas. Pure lyon tessellation via DrawSurface.
    // -------------------------------------------------------------------------
    let t0 = perf.now();

    // 1. White background.
    surface.fill_rect(Rect::new(0.0, 0.0, w, h), COLOR_WHITE);

    // 2. Grey "pending / not-yet-received" cell area.
    surface.fill_rect(Rect::new(HEADER_WIDTH, HEADER_HEIGHT, w, h), COLOR_PENDING);

    // 3. White fill for received cells.
    let row_end = (state.top_row + state.viewport_rows + 1).min(TOTAL_ROWS);
    let col_end = (state.left_col + state.viewport_cols + 1).min(TOTAL_COLS);

    for row in state.top_row..row_end {
        for col in state.left_col..col_end {
            if state.cell_cache.contains_key(&(row, col)) {
                let x = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                let y = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                surface.fill_rect(Rect::new(x, y, x + COL_WIDTH, y + ROW_HEIGHT), COLOR_WHITE);
            }
        }
    }

    // 4. Vertical grid lines.
    for i in 0..=(state.viewport_cols + 2) {
        let col = state.left_col + i;
        if col > TOTAL_COLS { break; }
        let x = HEADER_WIDTH + i as f64 * COL_WIDTH;
        if x > w { break; }
        surface.stroke_line(x, HEADER_HEIGHT, x, h, COLOR_GRID_LINE, 1.0);
    }

    // 5. Horizontal grid lines.
    for i in 0..=(state.viewport_rows + 2) {
        let row = state.top_row + i;
        if row > TOTAL_ROWS { break; }
        let y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
        if y > h { break; }
        surface.stroke_line(HEADER_WIDTH, y, w, y, COLOR_GRID_LINE, 1.0);
    }

    // 6a. Active column header tint.
    let active_col_visible = state.active_col >= state.left_col
        && state.active_col < state.left_col + state.viewport_cols;
    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        surface.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 6b. Active row header tint.
    let active_row_visible = state.active_row >= state.top_row
        && state.active_row < state.top_row + state.viewport_rows;
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        surface.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 7. Column header background (over the tints).
    surface.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);

    // 8. Row header background.
    surface.fill_rect(Rect::new(0.0, 0.0, HEADER_WIDTH, h), COLOR_HEADER_BG);

    // Re-apply active tints on top of the opaque header backgrounds.
    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        surface.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        surface.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 9. Header separator lines.
    surface.stroke_line(0.0, HEADER_HEIGHT, w, HEADER_HEIGHT, COLOR_HEADER_BORDER, 1.0);
    surface.stroke_line(HEADER_WIDTH, 0.0, HEADER_WIDTH, h, COLOR_HEADER_BORDER, 1.0);

    // 10. Active cell border.
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        surface.stroke_rect(Rect::new(x, y, x + cw, y + ch), COLOR_ACTIVE_BORDER, 2.0);
    }

    let t_bg = perf.now();

    // -------------------------------------------------------------------------
    // Phases CELLS + HDRS: text.
    // -------------------------------------------------------------------------

    const BODY_SIZE:   f32 = 12.0;
    const HEADER_SIZE: f32 = 11.0;

    let mut cells_drawn = 0u32;

    let t_hdrs = if text_enabled {
        // 11a. Cell body text.
        for row in state.top_row..row_end {
            for col in state.left_col..col_end {
                if let Some(content) = state.cell_cache.get(&(row, col)) {
                    if content.is_empty() { continue; }
                    let cx = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                    let cy = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                    let layout: &Layout<ParleyBrush> = text_cache.get_or_build(
                        font_cx, content, BODY_SIZE, (COL_WIDTH - 8.0) as f32, Alignment::Start,
                    );
                    let text_h  = layout.height() as f64;
                    let y_off   = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
                    let brush   = Brush::Solid(COLOR_CELL_TEXT);
                    surface.push_clip(Rect::new(cx, cy, cx + COL_WIDTH, cy + ROW_HEIGHT));
                    for line in layout.lines() {
                        for item in line.items() {
                            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                            let tx = cx + 4.0 + run.offset() as f64;
                            let ty = cy + y_off + run.baseline() as f64;
                            surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                        }
                    }
                    surface.pop_clip();
                    cells_drawn += 1;
                }
            }
        }
        let t_cells = perf.now();
        stats.cells_ms = (t_cells - t_bg) as f32;

        // 11b. Column header labels (A, B, C...).
        for i in 0..=(state.viewport_cols + 1) {
            let col = state.left_col + i;
            if col >= TOTAL_COLS { break; }
            let col_x = HEADER_WIDTH + i as f64 * COL_WIDTH;
            if col_x >= w { break; }
            let label = col_label(col);
            let layout: &Layout<ParleyBrush> = text_cache.get_or_build(
                font_cx, &label, HEADER_SIZE, COL_WIDTH as f32, Alignment::Center,
            );
            let text_h = layout.height() as f64;
            let y_off  = ((HEADER_HEIGHT - text_h) / 2.0).max(0.0);
            let brush  = Brush::Solid(COLOR_HEADER_TEXT);
            surface.push_clip(Rect::new(col_x, 0.0, col_x + COL_WIDTH, HEADER_HEIGHT));
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let tx = col_x + run.offset() as f64;
                    let ty = y_off + run.baseline() as f64;
                    surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                }
            }
            surface.pop_clip();
        }

        // 11c. Row header numbers (1, 2, 3...).
        for i in 0..=(state.viewport_rows + 1) {
            let row = state.top_row + i;
            if row >= TOTAL_ROWS { break; }
            let row_y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
            if row_y >= h { break; }
            let label = (row + 1).to_string();
            let layout: &Layout<ParleyBrush> = text_cache.get_or_build(
                font_cx, &label, HEADER_SIZE, (HEADER_WIDTH - 8.0) as f32, Alignment::End,
            );
            let text_h = layout.height() as f64;
            let y_off  = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
            let brush  = Brush::Solid(COLOR_HEADER_TEXT);
            surface.push_clip(Rect::new(0.0, row_y, HEADER_WIDTH, row_y + ROW_HEIGHT));
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let tx = 4.0 + run.offset() as f64;
                    let ty = row_y + y_off + run.baseline() as f64;
                    surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                }
            }
            surface.pop_clip();
        }
        let t_hdrs = perf.now();
        stats.hdrs_ms = (t_hdrs - t_cells) as f32;
        t_hdrs
    } else {
        stats.cells_ms = 0.0;
        stats.hdrs_ms  = 0.0;
        t_bg
    };

    // -------------------------------------------------------------------------
    // Phase HUD: FPS counter overlay (2 lines: high-level + sub-phase).
    // -------------------------------------------------------------------------

    let fps_bg = Rect::new(4.0, h - 38.0, 520.0, h - 4.0);
    surface.fill_rect(fps_bg, Color::from_rgba8(0, 0, 0, 160));

    let text_tag  = if text_enabled { "" } else { " [T=text]" };
    let line1 = format!(
        "{render_fps:.0}fps {last_frame_ms:.1}ms  \
         drain:{last_drain_ms:.1} draw:{last_draw_ms:.1} gpu:{last_gpu_ms:.1}  \
         miss:{}{text_tag}",
        text_cache.frame_misses,
    );
    let line2 = format!(
        "bg:{:.2} cells:{:.2} hdrs:{:.2} hud:{:.2}  drawn:{}",
        stats.bg_ms, stats.cells_ms, stats.hdrs_ms, stats.hud_ms, cells_drawn,
    );
    draw_text(
        surface, font_cx, text_cache,
        &line1, 11.0, 514.0, Alignment::Start,
        6.0, h - 37.0,
        Color::from_rgba8(255, 220, 0, 255),
    );
    draw_text(
        surface, font_cx, text_cache,
        &line2, 11.0, 514.0, Alignment::Start,
        6.0, h - 21.0,
        Color::from_rgba8(180, 220, 255, 255),
    );

    let t_hud_end = perf.now();
    stats.bg_ms      = (t_bg - t0) as f32;
    stats.hud_ms     = (t_hud_end - t_hdrs) as f32;
    stats.cells_drawn = cells_drawn;
}
