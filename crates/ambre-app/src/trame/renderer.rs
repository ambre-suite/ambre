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
//   8. Text: cell content, column letters, row numbers (via Parley)
//
// Text layout (Parley) is performed in `draw_grid` and passed to `VelloSurface`
// via `DrawSurface::draw_glyphs`.  `VelloSurface` itself holds only a scene
// reference -- no font or layout state.
// ============================================================================

use ambre_draw::{
    Affine, Alignment, AlignmentOptions, Brush, Color, DrawSurface, Fill, FontContext,
    GlyphRun, Layout, LayoutContext, Line, ParleyBrush, PositionedLayoutItem, Rect, Stroke,
    StyleProperty,
};
use vello::kurbo::Affine as VelloAffine;

use super::grid::{
    COL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH, ROW_HEIGHT, TOTAL_COLS, TOTAL_ROWS,
};
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
const COLOR_CELL_TEXT: Color = Color::from_rgba8(51, 51, 51, 255);   // #333333
const COLOR_HEADER_TEXT: Color = Color::from_rgba8(85, 85, 85, 255); // #555555

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
        self.scene.fill(Fill::NonZero, VelloAffine::IDENTITY, color, None, &rect);
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64) {
        self.scene.stroke(
            &Stroke::new(width),
            VelloAffine::IDENTITY,
            color,
            None,
            &rect,
        );
    }

    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64) {
        self.scene.stroke(
            &Stroke::new(width),
            VelloAffine::IDENTITY,
            color,
            None,
            &Line::new((x0, y0), (x1, y1)),
        );
    }

    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine) {
        // Map parley::layout::Glyph -> vello::Glyph (same fields, no advance in vello).
        let glyphs = run
            .positioned_glyphs()
            .map(|g| vello::Glyph { id: g.id, x: g.x, y: g.y });

        self.scene
            .draw_glyphs(run.run().font())
            .font_size(run.run().font_size())
            .transform(transform)
            .brush(brush)
            .hint(false)
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

/// Convert a zero-based column index to its spreadsheet label ("A", "B", ..., "Z", "AA", ...).
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

/// Shape `text` with Parley and call `draw_glyphs` for each glyph run.
///
/// `max_advance`: container width for line-breaking and alignment (pass `None`
/// to disable wrapping).  `align` controls horizontal alignment within the
/// container.  `transform` positions the top-left of the text area on the
/// surface.
#[allow(clippy::too_many_arguments)]
fn render_text<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
    text: &str,
    size: f32,
    color: Color,
    max_advance: Option<f32>,
    align: Alignment,
    transform: Affine,
) {
    if text.is_empty() {
        return;
    }

    // ParleyBrush ([u8; 4]) is a placeholder -- the actual render color is
    // passed as &Brush below.  We store [0, 0, 0, 255] so parley has a valid
    // value, but it is never read by our rendering path.
    let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::Brush([0, 0, 0, 255]));
    let mut layout: Layout<ParleyBrush> = builder.build(text);
    layout.break_all_lines(max_advance);
    layout.align(max_advance, align, AlignmentOptions::default());

    let brush = Brush::Solid(color);
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else { continue };
            surface.draw_glyphs(&run, &brush, transform);
        }
    }
}

// -----------------------------------------------------------------------------
// draw_grid
// -----------------------------------------------------------------------------

/// Render one frame of the Trame grid into `scene`.
///
/// Called every `requestAnimationFrame` from the render loop in `ambre-app/src/lib.rs`.
pub fn draw_grid(
    scene: &mut vello::Scene,
    state: &TrameUiState,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ParleyBrush>,
) {
    let mut s = VelloSurface::new(scene);
    let w = state.canvas_width;
    let h = state.canvas_height;

    // 1. White canvas background.
    s.fill_rect(Rect::new(0.0, 0.0, w, h), COLOR_WHITE);

    // 2. Grey "pending" fill for the entire grid area.
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

    // 6a. Active column header tint.
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

    // 7. Column header background strip.
    s.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);

    // 8. Row header background strip.
    s.fill_rect(Rect::new(0.0, 0.0, HEADER_WIDTH, h), COLOR_HEADER_BG);

    // Re-apply active tints on top of the header backgrounds.
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

    // 9. Header separator lines.
    s.stroke_line(0.0, HEADER_HEIGHT, w, HEADER_HEIGHT, COLOR_HEADER_BORDER, 1.0);
    s.stroke_line(HEADER_WIDTH, 0.0, HEADER_WIDTH, h, COLOR_HEADER_BORDER, 1.0);

    // 10. Active cell blue border (drawn last so it is always on top).
    if let Some((x, y, cw, ch)) = state.cell_pixel_rect(state.active_row, state.active_col) {
        s.stroke_rect(Rect::new(x, y, x + cw, y + ch), COLOR_ACTIVE_BORDER, 2.0);
    }

    // -------------------------------------------------------------------------
    // 11. Text rendering
    // -------------------------------------------------------------------------

    // Vertical center offset for 12px body text in a ROW_HEIGHT row.
    // Parley places the baseline relative to the layout top, so this positions
    // the text area such that the visual center aligns with the cell center.
    const BODY_SIZE: f32 = 12.0;
    const HEADER_SIZE: f32 = 11.0;
    let body_text_dy = (ROW_HEIGHT - BODY_SIZE as f64) / 2.0;
    let header_text_dy = (HEADER_HEIGHT - HEADER_SIZE as f64) / 2.0;

    // 11a. Cell body text (only for cells that have data).
    for row in state.top_row..row_end {
        for col in state.left_col..col_end {
            if let Some(content) = state.cell_cache.get(&(row, col)) {
                if content.is_empty() {
                    continue;
                }
                let cell_x = HEADER_WIDTH + (col - state.left_col) as f64 * COL_WIDTH;
                let cell_y = HEADER_HEIGHT + (row - state.top_row) as f64 * ROW_HEIGHT;

                // Clip text to cell bounds.
                let clip = Rect::new(cell_x, cell_y, cell_x + COL_WIDTH, cell_y + ROW_HEIGHT);
                s.push_clip(clip);

                let text_x = cell_x + 4.0;
                let text_y = cell_y + body_text_dy;
                render_text(
                    &mut s,
                    font_cx,
                    layout_cx,
                    content,
                    BODY_SIZE,
                    COLOR_CELL_TEXT,
                    Some((COL_WIDTH - 8.0) as f32),
                    Alignment::Start,
                    Affine::translate((text_x, text_y)),
                );

                s.pop_clip();
            }
        }
    }

    // 11b. Column header labels ("A", "B", ..., "AA", ...).
    for i in 0..=(state.viewport_cols + 1) {
        let col = state.left_col + i;
        if col >= TOTAL_COLS {
            break;
        }
        let col_x = HEADER_WIDTH + i as f64 * COL_WIDTH;
        if col_x >= w {
            break;
        }
        let label = col_label(col);
        let clip = Rect::new(col_x, 0.0, col_x + COL_WIDTH, HEADER_HEIGHT);
        s.push_clip(clip);
        render_text(
            &mut s,
            font_cx,
            layout_cx,
            &label,
            HEADER_SIZE,
            COLOR_HEADER_TEXT,
            Some(COL_WIDTH as f32),
            Alignment::Center,
            Affine::translate((col_x, header_text_dy)),
        );
        s.pop_clip();
    }

    // 11c. Row header numbers ("1", "2", ...).
    for i in 0..=(state.viewport_rows + 1) {
        let row = state.top_row + i;
        if row >= TOTAL_ROWS {
            break;
        }
        let row_y = HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
        if row_y >= h {
            break;
        }
        let label = (row + 1).to_string();
        let clip = Rect::new(0.0, row_y, HEADER_WIDTH, row_y + ROW_HEIGHT);
        s.push_clip(clip);
        // Right-align: layout within (HEADER_WIDTH - 8px), translate from x=4.0.
        render_text(
            &mut s,
            font_cx,
            layout_cx,
            &label,
            HEADER_SIZE,
            COLOR_HEADER_TEXT,
            Some((HEADER_WIDTH - 8.0) as f32),
            Alignment::End,
            Affine::translate((4.0, row_y + header_text_dy)),
        );
        s.pop_clip();
    }
}
