// ambre-app/src/trame/renderer.rs
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
// The orchestrator (UiState::render) detects what changed since the previous
// frame and selects the appropriate combination:
//   - Full repaint:      draw_cells(strip=None) + draw_overlay
//   - Pure scroll:       draw_cells(strip=Some(strip_rect)) + draw_overlay
//   - Overlay-only diff: draw_overlay
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
    /// Overlay layer HUD overlay itself.
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

/// Compute the inclusive cell-coordinate range that intersects `strip` in the
/// cell area. Returns `(row_start, row_end_exclusive, col_start, col_end_exclusive)`.
fn rows_cols_in_strip(
    state: &TrameUiState,
    strip: Rect,
) -> (u32, u32, u32, u32) {
    let row_start = if strip.y0 > HEADER_HEIGHT {
        ((strip.y0 - HEADER_HEIGHT) / ROW_HEIGHT).floor() as u32 + state.top_row
    } else {
        state.top_row
    };
    let row_end_excl = (((strip.y1 - HEADER_HEIGHT) / ROW_HEIGHT).ceil() as u32
        + state.top_row)
        .min(TOTAL_ROWS);
    let col_start = if strip.x0 > HEADER_WIDTH {
        ((strip.x0 - HEADER_WIDTH) / COL_WIDTH).floor() as u32 + state.left_col
    } else {
        state.left_col
    };
    let col_end_excl = (((strip.x1 - HEADER_WIDTH) / COL_WIDTH).ceil() as u32
        + state.left_col)
        .min(TOTAL_COLS);
    (row_start, row_end_excl, col_start, col_end_excl)
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
    surface:      &mut S,
    state:        &TrameUiState,
    font_cx:      &mut FontContext,
    text_cache:   &mut TextCache,
    text_enabled: bool,
    strip_rect:   Option<Rect>,
    perf:         &web_sys::Performance,
    stats:        &mut DrawStats,
) {
    let w = state.canvas_width;
    let h = state.canvas_height;

    // Cell area = full canvas minus the fixed-position header strip.
    let cell_area = Rect::new(HEADER_WIDTH, HEADER_HEIGHT, w, h);
    let strip = match strip_rect {
        Some(r) => r.intersect(cell_area),
        None    => cell_area,
    };
    if strip.width() <= 0.0 || strip.height() <= 0.0 {
        stats.bg_ms      = 0.0;
        stats.cells_ms   = 0.0;
        stats.cells_drawn = 0;
        return;
    }

    let t0 = perf.now();

    // -- 1. Pending grey over the strip. ---------------------------------------
    surface.fill_rect(strip, COLOR_PENDING);

    // -- 2. White fill for received cells in the strip. ------------------------
    let (row_start, row_end, col_start, col_end) = rows_cols_in_strip(state, strip);
    for row in row_start..row_end {
        for col in col_start..col_end {
            if state.cell_cache.contains_key(&(row, col)) {
                let x = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                let y = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                surface.fill_rect(Rect::new(x, y, x + COL_WIDTH, y + ROW_HEIGHT), COLOR_WHITE);
            }
        }
    }

    // -- 3. Vertical grid lines inside the strip. ------------------------------
    // Walk every visible column boundary, drawing line segments bounded by
    // the strip's y range.
    let line_y0 = strip.y0.max(HEADER_HEIGHT);
    let line_y1 = strip.y1.min(h);
    if line_y1 > line_y0 {
        for i in 0..=(state.viewport_cols + 2) {
            let col = state.left_col + i;
            if col > TOTAL_COLS { break; }
            let x = HEADER_WIDTH + i as f64 * COL_WIDTH;
            if x > w { break; }
            if x < strip.x0 || x > strip.x1 { continue; }
            surface.stroke_line(x, line_y0, x, line_y1, COLOR_GRID_LINE, 1.0);
        }
    }

    // -- 4. Horizontal grid lines inside the strip. ----------------------------
    let line_x0 = strip.x0.max(HEADER_WIDTH);
    let line_x1 = strip.x1.min(w);
    if line_x1 > line_x0 {
        for i in 0..=(state.viewport_rows + 2) {
            let row = state.top_row + i;
            if row > TOTAL_ROWS { break; }
            let y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
            if y > h { break; }
            if y < strip.y0 || y > strip.y1 { continue; }
            surface.stroke_line(line_x0, y, line_x1, y, COLOR_GRID_LINE, 1.0);
        }
    }

    let t_bg = perf.now();

    // -- 5. Cell body text. ----------------------------------------------------
    const BODY_SIZE: f32 = 12.0;
    let mut cells_drawn = 0u32;

    if text_enabled {
        let cell_brush     = Brush::Solid(COLOR_CELL_TEXT);
        let cell_max_width = (COL_WIDTH - 8.0) as f32;
        for row in row_start..row_end {
            for col in col_start..col_end {
                if let Some(content) = state.cell_cache.get(&(row, col)) {
                    if content.is_empty() { continue; }
                    let cx = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                    let cy = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                    let layout: &Layout<ParleyBrush> = text_cache.get_or_build(
                        font_cx, content, BODY_SIZE, cell_max_width, Alignment::Start,
                    );
                    let text_h  = layout.height() as f64;
                    let y_off   = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
                    let needs_clip = text_h > ROW_HEIGHT;
                    if needs_clip {
                        surface.push_clip(Rect::new(cx, cy, cx + COL_WIDTH, cy + ROW_HEIGHT));
                    }
                    for line in layout.lines() {
                        for item in line.items() {
                            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                            let tx = cx + 4.0 + run.offset() as f64;
                            let ty = cy + y_off + run.baseline() as f64;
                            surface.draw_glyphs(&run, &cell_brush, Affine::translate((tx, ty)));
                        }
                    }
                    if needs_clip {
                        surface.pop_clip();
                    }
                    cells_drawn += 1;
                }
            }
        }
    }

    let t_cells = perf.now();
    stats.bg_ms       = (t_bg - t0) as f32;
    stats.cells_ms    = (t_cells - t_bg) as f32;
    stats.cells_drawn = cells_drawn;
}

// -----------------------------------------------------------------------------
// draw_overlay -- transient OVERLAY layer
// -----------------------------------------------------------------------------

/// Draw the overlay layer (headers + active border + header labels + HUD).
/// This is re-rendered every dirty frame onto the swapchain after the cells
/// layer has been blitted from its persistent offscreen.
#[allow(clippy::too_many_arguments)]
pub fn draw_overlay<S: DrawSurface>(
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

    let t0 = perf.now();

    // -- 1. Active column / row header tints (drawn behind the bg first so
    //       they show through any alpha-blended pixels). -----------------------
    let active_col_visible = state.active_col >= state.left_col
        && state.active_col < state.left_col + state.viewport_cols;
    let active_row_visible = state.active_row >= state.top_row
        && state.active_row < state.top_row + state.viewport_rows;

    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        surface.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        surface.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // -- 2. Header backgrounds (cover the tint outside the active cells). -----
    surface.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);
    surface.fill_rect(Rect::new(0.0, 0.0, HEADER_WIDTH, h), COLOR_HEADER_BG);

    // Re-apply active tints over the opaque header backgrounds.
    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        surface.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        surface.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // -- 3. Header separator lines. -------------------------------------------
    surface.stroke_line(0.0, HEADER_HEIGHT, w, HEADER_HEIGHT, COLOR_HEADER_BORDER, 1.0);
    surface.stroke_line(HEADER_WIDTH, 0.0, HEADER_WIDTH, h, COLOR_HEADER_BORDER, 1.0);

    // -- 4. Active cell border. -----------------------------------------------
    // Lives in the overlay so it tracks the active cell on every frame
    // regardless of scroll shifts in the persistent layer.
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        surface.stroke_rect(Rect::new(x, y, x + cw, y + ch), COLOR_ACTIVE_BORDER, 2.0);
    }

    // -- 5. Header text labels. -----------------------------------------------
    const HEADER_SIZE: f32 = 11.0;
    if text_enabled {
        let header_brush = Brush::Solid(COLOR_HEADER_TEXT);

        // 5a. Column header labels (A, B, C...).
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
            let needs_clip = text_h > HEADER_HEIGHT;
            if needs_clip {
                surface.push_clip(Rect::new(col_x, 0.0, col_x + COL_WIDTH, HEADER_HEIGHT));
            }
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let tx = col_x + run.offset() as f64;
                    let ty = y_off + run.baseline() as f64;
                    surface.draw_glyphs(&run, &header_brush, Affine::translate((tx, ty)));
                }
            }
            if needs_clip {
                surface.pop_clip();
            }
        }

        // 5b. Row header numbers (1, 2, 3...).
        let row_hdr_max_width = (HEADER_WIDTH - 8.0) as f32;
        for i in 0..=(state.viewport_rows + 1) {
            let row = state.top_row + i;
            if row >= TOTAL_ROWS { break; }
            let row_y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
            if row_y >= h { break; }
            let label = (row + 1).to_string();
            let layout: &Layout<ParleyBrush> = text_cache.get_or_build(
                font_cx, &label, HEADER_SIZE, row_hdr_max_width, Alignment::End,
            );
            let text_h = layout.height() as f64;
            let y_off  = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
            let needs_clip = text_h > ROW_HEIGHT;
            if needs_clip {
                surface.push_clip(Rect::new(0.0, row_y, HEADER_WIDTH, row_y + ROW_HEIGHT));
            }
            for line in layout.lines() {
                for item in line.items() {
                    let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                    let tx = 4.0 + run.offset() as f64;
                    let ty = row_y + y_off + run.baseline() as f64;
                    surface.draw_glyphs(&run, &header_brush, Affine::translate((tx, ty)));
                }
            }
            if needs_clip {
                surface.pop_clip();
            }
        }
    }
    let t_after_hdrs = perf.now();

    // -- 6. HUD overlay. ------------------------------------------------------
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
        stats.bg_ms, stats.cells_ms, stats.hdrs_ms, stats.hud_ms, stats.cells_drawn,
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

    let t_end = perf.now();
    stats.hdrs_ms = (t_after_hdrs - t0) as f32;
    stats.hud_ms  = (t_end - t_after_hdrs) as f32;
}
