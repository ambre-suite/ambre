// ambre-trame/src/render/headers.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;

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
    layout: &GridLayout<'_>,
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
            font_cx,
            &label,
            HEADER_SIZE,
            layout.col_width as f32,
            Alignment::Center,
            false,
            false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((hh - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > hh;
        if needs_clip {
            surface.push_clip(Rect::new(col_x, top, col_x + layout.col_width, top + hh));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
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
    layout: &GridLayout<'_>,
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

    // Active-row tint, over the bg and under the numbers. Positioned in collapsed
    // (filtered) row space; skipped when the active row is itself hidden.
    if let Some(row) = active_row
        && row >= layout.origin_row
        && !layout.is_hidden(row)
    {
        let y = cells_top + layout.visible_offset(row) as f64 * layout.row_height;
        if y < h {
            surface.fill_rect(
                Rect::new(0.0, y, hw, y + layout.row_height),
                palette.header_active,
            );
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
    while let Some(row) = layout.row_at_slot(i) {
        // The label at visible slot `i` is the i-th non-hidden row from the
        // origin: filtered rows are collapsed out of the number strip too.
        let row_y = cells_top + i as f64 * layout.row_height;
        if row_y >= h {
            break;
        }
        let label = (row + 1).to_string();
        let lyt: &Layout<ParleyBrush> = text_cache.get_or_build(
            font_cx,
            &label,
            HEADER_SIZE,
            max_width,
            Alignment::End,
            false,
            false,
        );
        let text_h = lyt.height() as f64;
        let y_off = ((layout.row_height - text_h) / 2.0).max(0.0);
        let needs_clip = text_h > layout.row_height;
        if needs_clip {
            surface.push_clip(Rect::new(0.0, row_y, hw, row_y + layout.row_height));
        }
        for line in lyt.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
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
