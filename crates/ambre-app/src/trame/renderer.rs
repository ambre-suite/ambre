// ambre-app/src/trame/renderer.rs
// ============================================================================
// Trame grid renderer -- UI Thread, Vello backend.
//
// `VelloSurface` wraps a `&mut vello::Scene` and implements `DrawSurface`.
//
// `draw_grid` renders one frame via `TextCache` (cached Parley layouts) so
// that shaping only happens on cache misses -- not every frame.
// ============================================================================

use ambre_draw::{Affine, Alignment, Brush, Color, DrawSurface, Fill, FontContext, GlyphRun,
                 ParleyBrush, Rect, Stroke};
use vello::kurbo::Affine as VelloAffine;

use super::grid::{COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TOTAL_COLS, TOTAL_ROWS};
use super::text_cache::{TextCache, render_layout};
use super::TrameUiState;

// -----------------------------------------------------------------------------
// Colors
// -----------------------------------------------------------------------------

const COLOR_WHITE: Color = Color::from_rgba8(255, 255, 255, 255);
const COLOR_PENDING: Color = Color::from_rgba8(235, 237, 240, 255);
const COLOR_HEADER_BG: Color = Color::from_rgba8(242, 244, 248, 255);
const COLOR_HEADER_ACTIVE: Color = Color::from_rgba8(207, 220, 240, 255);
const COLOR_GRID_LINE: Color = Color::from_rgba8(210, 213, 218, 255);
const COLOR_HEADER_BORDER: Color = Color::from_rgba8(178, 183, 193, 255);
const COLOR_ACTIVE_BORDER: Color = Color::from_rgba8(60, 120, 210, 255);
const COLOR_CELL_TEXT: Color = Color::from_rgba8(51, 51, 51, 255);
const COLOR_HEADER_TEXT: Color = Color::from_rgba8(85, 85, 85, 255);

// -----------------------------------------------------------------------------
// VelloSurface
// -----------------------------------------------------------------------------

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
        self.scene.fill(Fill::NonZero, VelloAffine::IDENTITY, color, None, &rect);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        self.scene.stroke(&Stroke::new(width), VelloAffine::IDENTITY, color, None, &rect);
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        use ambre_draw::Line;
        self.scene.stroke(
            &Stroke::new(width),
            VelloAffine::IDENTITY,
            color,
            None,
            &Line::new((x0, y0), (x1, y1)),
        );
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        // This path is kept for DrawSurface completeness (e.g. KrillaSurface).
        // The hot path in draw_grid goes through TextCache::render_cell_text
        // which calls render_layout() directly on the scene -- bypassing the
        // trait to avoid re-wrapping the cached layout.
        let mut pen_x = 0.0_f32;
        let glyphs = run.glyphs().map(move |g| {
            let out = vello::Glyph { id: g.id, x: pen_x + g.x, y: g.y };
            pen_x += g.advance;
            out
        });
        let coords: Vec<_> = run.run().normalized_coords().to_vec();
        self.scene
            .draw_glyphs(run.run().font())
            .font_size(run.run().font_size())
            .transform(transform)
            .glyph_transform(None)
            .normalized_coords(&coords)
            .brush(brush)
            .hint(true)
            .draw(Fill::NonZero, glyphs);
    }

    fn push_clip(&mut self, rect: Rect) {
        self.scene.push_clip_layer(Fill::NonZero, VelloAffine::IDENTITY, &rect);
    }

    fn pop_clip(&mut self) {
        self.scene.pop_layer();
    }
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

// -----------------------------------------------------------------------------
// draw_grid
// -----------------------------------------------------------------------------

pub fn draw_grid(
    scene: &mut vello::Scene,
    state: &TrameUiState,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    render_fps: f32,
    last_frame_ms: f32,
) {
    let mut s = VelloSurface::new(scene);
    let w = state.canvas_width;
    let h = state.canvas_height;

    // 1. White background.
    s.fill_rect(Rect::new(0.0, 0.0, w, h), COLOR_WHITE);

    // 2. Grey pending area.
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
    for i in 0..=(state.viewport_cols + 2) {
        let col = state.left_col + i;
        if col > TOTAL_COLS { break; }
        let x = HEADER_WIDTH + i as f64 * COL_WIDTH;
        if x > w { break; }
        s.stroke_line(x, HEADER_HEIGHT, x, h, COLOR_GRID_LINE, 1.0);
    }

    // 5. Horizontal grid lines.
    for i in 0..=(state.viewport_rows + 2) {
        let row = state.top_row + i;
        if row > TOTAL_ROWS { break; }
        let y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
        if y > h { break; }
        s.stroke_line(HEADER_WIDTH, y, w, y, COLOR_GRID_LINE, 1.0);
    }

    // 6a. Active column tint.
    let active_col_visible = state.active_col >= state.left_col
        && state.active_col < state.left_col + state.viewport_cols;
    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        s.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 6b. Active row tint.
    let active_row_visible = state.active_row >= state.top_row
        && state.active_row < state.top_row + state.viewport_rows;
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        s.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 7. Column header background.
    s.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);

    // 8. Row header background.
    s.fill_rect(Rect::new(0.0, 0.0, HEADER_WIDTH, h), COLOR_HEADER_BG);

    // Re-apply active tints on top of header backgrounds.
    if active_col_visible {
        let x = HEADER_WIDTH + (state.active_col - state.left_col) as f64 * COL_WIDTH;
        s.fill_rect(Rect::new(x, 0.0, x + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
    }
    if active_row_visible {
        let y = HEADER_HEIGHT + (state.active_row - state.top_row) as f64 * ROW_HEIGHT;
        s.fill_rect(Rect::new(0.0, y, HEADER_WIDTH, y + ROW_HEIGHT), COLOR_HEADER_ACTIVE);
    }

    // 9. Header separator lines.
    s.stroke_line(0.0, HEADER_HEIGHT, w, HEADER_HEIGHT, COLOR_HEADER_BORDER, 1.0);
    s.stroke_line(HEADER_WIDTH, 0.0, HEADER_WIDTH, h, COLOR_HEADER_BORDER, 1.0);

    // 10. Active cell border.
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        s.stroke_rect(Rect::new(x, y, x + cw, y + ch), COLOR_ACTIVE_BORDER, 2.0);
    }

    // -------------------------------------------------------------------------
    // 11. Text via TextCache (cached Parley layouts -- no per-frame shaping)
    // -------------------------------------------------------------------------

    const BODY_SIZE: f32 = 12.0;
    const HEADER_SIZE: f32 = 11.0;

    if text_enabled {
        // 11a. Cell body text.
        for row in state.top_row..row_end {
            for col in state.left_col..col_end {
                if let Some(content) = state.cell_cache.get(&(row, col)) {
                    if content.is_empty() { continue; }
                    let cx = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                    let cy = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;
                    s.push_clip(Rect::new(cx, cy, cx + COL_WIDTH, cy + ROW_HEIGHT));
                    let layout = text_cache.get_or_build(
                        font_cx, content, BODY_SIZE, (COL_WIDTH - 8.0) as f32, Alignment::Start,
                    );
                    let text_h = layout.height() as f64;
                    let y_off = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
                    render_layout(
                        s.scene, layout,
                        VelloAffine::translate((cx + 4.0, cy + y_off)),
                        &Brush::Solid(COLOR_CELL_TEXT),
                    );
                    s.pop_clip();
                }
            }
        }

        // 11b. Column header labels.
        for i in 0..=(state.viewport_cols + 1) {
            let col = state.left_col + i;
            if col >= TOTAL_COLS { break; }
            let col_x = HEADER_WIDTH + i as f64 * COL_WIDTH;
            if col_x >= w { break; }
            let label = col_label(col);
            s.push_clip(Rect::new(col_x, 0.0, col_x + COL_WIDTH, HEADER_HEIGHT));
            let layout = text_cache.get_or_build(
                font_cx, &label, HEADER_SIZE, COL_WIDTH as f32, Alignment::Center,
            );
            let text_h = layout.height() as f64;
            let y_off = ((HEADER_HEIGHT - text_h) / 2.0).max(0.0);
            render_layout(
                s.scene, layout,
                VelloAffine::translate((col_x, y_off)),
                &Brush::Solid(COLOR_HEADER_TEXT),
            );
            s.pop_clip();
        }

        // 11c. Row header numbers.
        for i in 0..=(state.viewport_rows + 1) {
            let row = state.top_row + i;
            if row >= TOTAL_ROWS { break; }
            let row_y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
            if row_y >= h { break; }
            let label = (row + 1).to_string();
            s.push_clip(Rect::new(0.0, row_y, HEADER_WIDTH, row_y + ROW_HEIGHT));
            let layout = text_cache.get_or_build(
                font_cx, &label, HEADER_SIZE, (HEADER_WIDTH - 8.0) as f32, Alignment::End,
            );
            let text_h = layout.height() as f64;
            let y_off = ((ROW_HEIGHT - text_h) / 2.0).max(0.0);
            render_layout(
                s.scene, layout,
                VelloAffine::translate((4.0, row_y + y_off)),
                &Brush::Solid(COLOR_HEADER_TEXT),
            );
            s.pop_clip();
        }
    }

    // -------------------------------------------------------------------------
    // 12. FPS counter (always visible)
    // -------------------------------------------------------------------------

    let text_tag = if text_enabled { "" } else { " [T=text]" };
    let fps_label = format!("{render_fps:.0} fps  {last_frame_ms:.1}ms{text_tag}");
    let fps_bg = Rect::new(4.0, h - 20.0, 180.0, h - 4.0);
    s.fill_rect(fps_bg, Color::from_rgba8(0, 0, 0, 160));
    let layout = text_cache.get_or_build(
        font_cx, &fps_label, 11.0, 174.0, Alignment::Start,
    );
    render_layout(
        s.scene, layout,
        VelloAffine::translate((6.0, h - 19.0)),
        &Brush::Solid(Color::from_rgba8(255, 220, 0, 255)),
    );
}
