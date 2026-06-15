// ambre-trame/src/render.rs
// ============================================================================
// Shared, surface-generic rendering primitives for the Trame grid.
//
// These functions are parameterized over the `DrawSurface` trait (defined in
// `ambre-draw`) so the *same* drawing code serves both:
//
//   - the UI thread, via `WgpuSurface` (live GPU rendering), called from
//     `ui::renderer` inside its dirty-rect / layer orchestration;
//   - the Logic thread, via `KrillaSurface` (vector PDF export, lazy-loaded),
//     once the PDF pipeline lands (see spec-ambre-trame-logic.md 9 / 14).
//
// Everything here is geometry-driven through `GridLayout` and plain cell data
// (`CellContent`), with no knowledge of scroll state, dirty rects, the HUD, or
// any other screen-only concern. Those live in `ui::renderer`, which composes
// these primitives in the right z-order.
//
// `GridLayout` is uniform-metrics for the MVP (one column width, one row
// height) so it stays O(1) over the virtual 100k grid -- never materializing a
// per-index offset vector. Variable column widths / row heights (resize) will
// extend `GridLayout` later without changing these primitive signatures.
// ============================================================================

use ambre_canvas::{Palette, TextCache};
use ambre_draw::{
    Affine, Alignment, Brush, Color, DrawSurface, FontContext, Layout, ParleyBrush,
    PositionedLayoutItem, Rect,
};

/// Cell text left padding (also the caret's x origin inside a cell).
pub const CELL_PAD_X: f64 = 4.0;
/// Body font size, shared by the cell renderer and the edit overlay.
pub const BODY_SIZE: f32 = 12.0;
/// Header label font size (column letters + row numbers).
pub const HEADER_SIZE: f32 = 11.0;

// -----------------------------------------------------------------------------
// GridLayout -- index <-> pixel mapping
// -----------------------------------------------------------------------------

/// Maps absolute grid indices to pixel rectangles on a target surface.
///
/// `origin_row` / `origin_col` are the absolute grid indices pinned to the
/// top-left corner of the content area (i.e. the scroll position, cell-aligned
/// for the MVP). For PDF export the origin is `(0, 0)`.
///
/// MVP: uniform `col_width` / `row_height`. The fields are deliberately shaped
/// so a future variable-size layout can add per-index offset slices without
/// breaking callers.
#[derive(Clone, Copy, Debug)]
pub struct GridLayout {
    /// Width of the fixed row-number strip on the left.
    pub header_width: f64,
    /// Height of the fixed column-letter strip on the top.
    pub header_height: f64,
    /// Vertical offset (physical px) of the whole grid from the surface top:
    /// the height of any band reserved above the column-header strip (the
    /// formula bar on screen, `0.0` for PDF export).
    pub top: f64,
    /// Uniform column width (content area).
    pub col_width: f64,
    /// Uniform row height (content area).
    pub row_height: f64,
    /// Absolute grid column at the content area's left edge.
    pub origin_col: u32,
    /// Absolute grid row at the content area's top edge.
    pub origin_row: u32,
    /// Full surface width in device pixels (header strip included).
    pub width: f64,
    /// Full surface height in device pixels (header strip included).
    pub height: f64,
    /// Total columns in the grid (iteration bound).
    pub total_cols: u32,
    /// Total rows in the grid (iteration bound).
    pub total_rows: u32,
}

impl GridLayout {
    /// The scrollable content area (everything below + right of the headers).
    pub fn content_area(&self) -> Rect {
        Rect::new(self.header_width, self.top + self.header_height, self.width, self.height)
    }

    /// Pixel rect of cell `(row, col)` relative to the surface top-left.
    ///
    /// The result is unclipped: a cell scrolled left of / above the origin has
    /// a negative-offset (smaller-than-header) coordinate. Callers clip against
    /// `content_area()` or push their own clip.
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        let x = self.header_width + (col as i64 - self.origin_col as i64) as f64 * self.col_width;
        let y = self.top
            + self.header_height
            + (row as i64 - self.origin_row as i64) as f64 * self.row_height;
        Rect::new(x, y, x + self.col_width, y + self.row_height)
    }
}

// -----------------------------------------------------------------------------
// Cell content view
// -----------------------------------------------------------------------------

/// A borrowed view of one cell's renderable content. Both the screen cache
/// (`ui::grid::CellData`) and a future `DisplayViewport` map onto this, so the
/// primitives never depend on either concrete type.
#[derive(Clone, Copy)]
pub struct CellContent<'a> {
    pub text: &'a str,
    pub bold: bool,
    pub italic: bool,
}

// -----------------------------------------------------------------------------
// Text helper
// -----------------------------------------------------------------------------

/// Shape `text` and emit its glyph runs at `(origin_x, origin_y)` (the layout's
/// top-left). Shared by the headers, the HUD, and any one-shot label.
#[allow(clippy::too_many_arguments)]
pub fn draw_text<S: DrawSurface>(
    surface: &mut S,
    font_cx: &mut FontContext,
    cache: &mut TextCache,
    text: &str,
    font_size: f32,
    max_width: f32,
    align: Alignment,
    origin_x: f64,
    origin_y: f64,
    color: Color,
) {
    if text.is_empty() {
        return;
    }
    let layout: &Layout<ParleyBrush> =
        cache.get_or_build(font_cx, text, font_size, max_width, align, false, false);
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

/// Spreadsheet column label for a 0-based column index (0 -> "A", 26 -> "AA").
pub fn col_label(col: u32) -> String {
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
// draw_cells_on -- cell backgrounds + body text
// -----------------------------------------------------------------------------

/// Draw the background fill and body text for each supplied cell.
///
/// `cells` yields `(row, col, content)` for the cells that exist (on screen:
/// the cells received from the Logic thread; for PDF: every used cell). Each
/// gets an opaque `palette.content_bg` fill -- on screen this is what lifts a
/// cell out of the "pending" grey -- followed by its text when `text_enabled`.
///
/// Grid lines are *not* drawn here: call [`draw_grid_lines_on`] afterwards so
/// the separators sit on top of the fills. Returns the number of cells that
/// actually had non-empty text drawn (font-independent tally for stats/tests).
#[allow(clippy::too_many_arguments)]
pub fn draw_cells_on<'a, S, I>(
    surface: &mut S,
    layout: &GridLayout,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    cells: I,
) -> u32
where
    S: DrawSurface,
    I: IntoIterator<Item = (u32, u32, CellContent<'a>)>,
{
    let cell_brush = Brush::Solid(palette.cell_text);
    let cell_max_width = (layout.col_width - 8.0) as f32;
    let mut drawn = 0u32;

    for (row, col, content) in cells {
        let rect = layout.cell_rect(row, col);
        // Lift the cell out of the pending grey with an opaque fill.
        surface.fill_rect(rect, palette.content_bg);

        if !text_enabled || content.text.is_empty() {
            continue;
        }

        let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx,
            content.text,
            BODY_SIZE,
            cell_max_width,
            Alignment::Start,
            content.bold,
            content.italic,
        );
        let text_h = layout_text.height() as f64;
        let y_off = ((layout.row_height - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > layout.row_height;
        if needs_clip {
            surface.push_clip(rect);
        }
        for line in layout_text.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = rect.x0 + CELL_PAD_X + run.offset() as f64;
                let ty = rect.y0 + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &cell_brush, Affine::translate((tx, ty)));
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        drawn += 1;
    }

    drawn
}

// -----------------------------------------------------------------------------
// draw_grid_lines_on -- 1px cell separators
// -----------------------------------------------------------------------------

/// Stroke the grid separators inside `clip`, intersected with the content area.
///
/// Drawn *after* the cell fills so the 1px boundaries stay visible between
/// adjacent filled cells.
pub fn draw_grid_lines_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout,
    color: Color,
    clip: Rect,
) {
    let area = clip.intersect(layout.content_area());
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return;
    }

    // Vertical lines at each visible column boundary.
    let cells_top = layout.top + layout.header_height;
    let line_y0 = area.y0.max(cells_top);
    let line_y1 = area.y1.min(layout.height);
    if line_y1 > line_y0 {
        let mut i = 0u32;
        loop {
            let col = layout.origin_col + i;
            if col > layout.total_cols {
                break;
            }
            let x = layout.header_width + i as f64 * layout.col_width;
            if x > layout.width {
                break;
            }
            if x >= area.x0 && x <= area.x1 {
                surface.stroke_line(x, line_y0, x, line_y1, color, 1.0);
            }
            i += 1;
        }
    }

    // Horizontal lines at each visible row boundary.
    let line_x0 = area.x0.max(layout.header_width);
    let line_x1 = area.x1.min(layout.width);
    if line_x1 > line_x0 {
        let mut i = 0u32;
        loop {
            let row = layout.origin_row + i;
            if row > layout.total_rows {
                break;
            }
            let y = cells_top + i as f64 * layout.row_height;
            if y > layout.height {
                break;
            }
            if y >= area.y0 && y <= area.y1 {
                surface.stroke_line(line_x0, y, line_x1, y, color, 1.0);
            }
            i += 1;
        }
    }
}

// -----------------------------------------------------------------------------
// draw_col_headers_on / draw_row_headers_on -- fixed header strips
// -----------------------------------------------------------------------------

/// Draw the top column-header strip: opaque background, the active-column tint
/// (when `active_col` is visible), the bottom separator, then the A/B/C labels.
///
/// `active_col == None` (PDF) skips the highlight.
#[allow(clippy::too_many_arguments)]
pub fn draw_col_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout,
    palette: &Palette,
    active_col: Option<u32>,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
) {
    let w = layout.width;
    let hh = layout.header_height;
    let top = layout.top;

    // Opaque background over the full top strip (corner included).
    surface.fill_rect(Rect::new(0.0, top, w, top + hh), palette.header_bg);

    // Active-column tint, over the bg and under the labels.
    if let Some(col) = active_col
        && col >= layout.origin_col
    {
        let x = layout.header_width + (col - layout.origin_col) as f64 * layout.col_width;
        if x < w {
            surface.fill_rect(
                Rect::new(x, top, x + layout.col_width, top + hh),
                palette.header_active,
            );
        }
    }

    // Bottom separator.
    surface.stroke_line(0.0, top + hh, w, top + hh, palette.header_border, 1.0);

    if !text_enabled {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let mut i = 0u32;
    loop {
        let col = layout.origin_col + i;
        if col >= layout.total_cols {
            break;
        }
        let col_x = layout.header_width + i as f64 * layout.col_width;
        if col_x >= w {
            break;
        }
        let label = col_label(col);
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx, &label, HEADER_SIZE, layout.col_width as f32, Alignment::Center, false, false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((hh - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > hh;
        if needs_clip {
            surface.push_clip(Rect::new(col_x, top, col_x + layout.col_width, top + hh));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = col_x + run.offset() as f64;
                let ty = top + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        i += 1;
    }
}

/// Draw the left row-header strip: opaque background, the active-row tint (when
/// `active_row` is visible), the right separator, then the 1/2/3 numbers.
///
/// `active_row == None` (PDF) skips the highlight.
#[allow(clippy::too_many_arguments)]
pub fn draw_row_headers_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout,
    palette: &Palette,
    active_row: Option<u32>,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
) {
    let h = layout.height;
    let hw = layout.header_width;
    let top = layout.top;
    let cells_top = top + layout.header_height;

    // Opaque background over the full left strip (below the formula bar band).
    surface.fill_rect(Rect::new(0.0, top, hw, h), palette.header_bg);

    // Active-row tint, over the bg and under the numbers.
    if let Some(row) = active_row
        && row >= layout.origin_row
    {
        let y = cells_top + (row - layout.origin_row) as f64 * layout.row_height;
        if y < h {
            surface.fill_rect(Rect::new(0.0, y, hw, y + layout.row_height), palette.header_active);
        }
    }

    // Right separator.
    surface.stroke_line(hw, top, hw, h, palette.header_border, 1.0);

    if !text_enabled {
        return;
    }
    let brush = Brush::Solid(palette.header_text);
    let max_width = (hw - 8.0) as f32;
    let mut i = 0u32;
    loop {
        let row = layout.origin_row + i;
        if row >= layout.total_rows {
            break;
        }
        let row_y = cells_top + i as f64 * layout.row_height;
        if row_y >= h {
            break;
        }
        let label = (row + 1).to_string();
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx, &label, HEADER_SIZE, max_width, Alignment::End, false, false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((layout.row_height - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > layout.row_height;
        if needs_clip {
            surface.push_clip(Rect::new(0.0, row_y, hw, row_y + layout.row_height));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = 4.0 + run.offset() as f64;
                let ty = row_y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        i += 1;
    }
}

// -----------------------------------------------------------------------------
// Headless tests -- mock DrawSurface.
// -----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use ambre_canvas::{TextCache, Theme};
    use ambre_draw::testing::{DrawOp, RecordingSurface};
    use ambre_draw::FontContext;

    /// 800x600 surface, uniform 80x20 cells, 50x24 headers, origin at (0,0).
    fn layout() -> GridLayout {
        GridLayout {
            header_width: 50.0,
            header_height: 24.0,
            top: 0.0,
            col_width: 80.0,
            row_height: 20.0,
            origin_col: 0,
            origin_row: 0,
            width: 800.0,
            height: 600.0,
            total_cols: 1000,
            total_rows: 100_000,
        }
    }

    fn harness() -> (RecordingSurface, FontContext, TextCache) {
        (RecordingSurface::new(), FontContext::new(), TextCache::new(1.0))
    }

    #[test]
    fn cell_rect_maps_index_to_pixels() {
        let l = layout();
        let r = l.cell_rect(0, 0);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
        let r = l.cell_rect(2, 1);
        assert_eq!((r.x0, r.y0), (50.0 + 80.0, 24.0 + 2.0 * 20.0));
    }

    #[test]
    fn cell_rect_accounts_for_scroll_origin() {
        let mut l = layout();
        l.origin_col = 3;
        l.origin_row = 5;
        // The origin cell sits at the content top-left.
        let r = l.cell_rect(5, 3);
        assert_eq!((r.x0, r.y0), (50.0, 24.0));
    }

    #[test]
    fn draw_cells_on_fills_each_and_counts_nonempty_text() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        let cells = vec![
            (0, 0, CellContent { text: "hi", bold: false, italic: false }),
            (1, 1, CellContent { text: "", bold: false, italic: false }),
        ];
        let drawn = draw_cells_on(&mut s, &l, palette, &mut fc, &mut tc, true, cells);

        // Both cells get a content_bg fill...
        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.content_bg)),
            2,
        );
        // ...but only the non-empty one is tallied.
        assert_eq!(drawn, 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn draw_grid_lines_on_strokes_with_grid_color() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, _fc, _tc) = harness();

        draw_grid_lines_on(&mut s, &l, palette.grid_line, l.content_area());

        assert!(
            s.count(|op| matches!(op, DrawOp::StrokeLine { color, .. } if *color == palette.grid_line)) > 0
        );
    }

    #[test]
    fn col_headers_draw_bg_separator_and_active_tint() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        draw_col_headers_on(&mut s, &l, palette, Some(0), &mut fc, &mut tc, true);

        // The full-width header strip.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        // The active-column tint.
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_active
        )) >= 1);
        // The bottom separator.
        assert!(s.count(|op| matches!(
            op, DrawOp::StrokeLine { color, .. } if *color == palette.header_border
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn row_headers_skip_tint_when_inactive() {
        let l = layout();
        let palette = Theme::Light.palette();
        let (mut s, mut fc, mut tc) = harness();

        // No active row -> no header_active fill, just bg + separator.
        draw_row_headers_on(&mut s, &l, palette, None, &mut fc, &mut tc, true);

        assert_eq!(
            s.count(|op| matches!(op, DrawOp::FillRect { color, .. } if *color == palette.header_active)),
            0,
        );
        assert!(s.count(|op| matches!(
            op, DrawOp::FillRect { color, .. } if *color == palette.header_bg
        )) >= 1);
        assert_eq!(s.clip_depth(), 0);
    }

    #[test]
    fn col_label_matches_spreadsheet_convention() {
        assert_eq!(col_label(0), "A");
        assert_eq!(col_label(25), "Z");
        assert_eq!(col_label(26), "AA");
        assert_eq!(col_label(701), "ZZ");
        assert_eq!(col_label(702), "AAA");
    }
}
