// ambre-trame/src/render/merges.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::content::{cell_alignment, cell_v_offset, rgb};
use super::*;

// -----------------------------------------------------------------------------
// draw_merges_on -- merge masters spanning their covered area
// -----------------------------------------------------------------------------

/// Draw the merge masters over the base cell + grid-line layer.
///
/// `merges` yields `(row, col, col_span, row_span, content)` for the visible
/// merge masters. A covered cell is *not* a master and must not appear here:
/// it renders blank under the master's span. For each master:
///
///   1. the full span gets an opaque `content_bg` fill, painting over the
///      interior grid separators drawn through the covered cells;
///   2. the master's text is laid out across the spanned width, vertically
///      centered in the spanned height;
///   3. the span's outer border is stroked in `grid_line` so the merged block
///      reads as one cell.
///
/// Call *after* [`draw_cells_on`] + [`draw_grid_lines_on`]. Returns the number
/// of masters that had non-empty text drawn (font-independent tally).
#[allow(clippy::too_many_arguments)]
pub fn draw_merges_on<'a, S, I>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_enabled: bool,
    merges: I,
) -> u32
where
    S: DrawSurface,
    I: IntoIterator<Item = (u32, u32, u32, u32, CellContent<'a>)>,
{
    let mut drawn = 0u32;

    for (row, col, col_span, row_span, content) in merges {
        let rect = layout.merge_rect(row, col, col_span, row_span);

        // 1. Fill the whole span with the master's fill (or the themed bg),
        // hiding the interior separators drawn through the covered cells.
        let bg = content.bg.map(rgb).unwrap_or(palette.content_bg);
        surface.fill_rect(rect, bg);
        // Re-stroke the span perimeter in the grid-line colour so the block reads
        // as one cell (the fill above paints over the perimeter separators). A
        // user-set border draws on top of this in the dedicated border pass.
        surface.stroke_rect(rect, palette.grid_line, 1.0);

        if text_enabled && !content.text.is_empty() {
            // Honour the master's full style (size, font, colour, alignment,
            // underline/strike) exactly like a plain cell, but always clipped to
            // and aligned within the span rect.
            let avail = (rect.width() - 2.0 * CELL_PAD_X * layout.zoom).max(0.0);
            // True zoom: scale the glyph size with the grid (span metrics already
            // scale via `cell_rect`/`merge_rect`).
            let font_size = super::cell_font_px(content.size) * layout.zoom as f32;
            // Wrap constrains the layout to the span width, breaking long tokens
            // within the word. Otherwise the line lays out unbounded, and is
            // clipped to the span rect below (`trame-cell-rendering`).
            let max_w = if content.wrap { avail as f32 } else { f32::MAX };
            let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build_font(
                font_cx,
                content.text,
                font_size,
                max_w,
                Alignment::Start,
                content.bold,
                content.italic,
                content.font,
                content.wrap,
            );
            let text_h = layout_text.height() as f64;
            let text_w = layout_text.width() as f64;
            let y_off = cell_v_offset(content.v_align, rect.height(), text_h);
            let x_off = match cell_alignment(content.h_align, content.numeric) {
                Alignment::Center => ((avail - text_w) / 2.0).max(0.0),
                Alignment::End | Alignment::Right => (avail - text_w).max(0.0),
                _ => 0.0,
            };
            let fg = content.fg.map(rgb).unwrap_or(palette.cell_text);
            let brush = Brush::Solid(fg);
            surface.push_clip(rect);
            let origin_x = rect.x0 + CELL_PAD_X * layout.zoom + x_off;
            draw_layout_runs_decorated(
                surface,
                layout_text,
                origin_x,
                rect.y0 + y_off,
                &brush,
                fg,
                font_size,
                content.underline,
                content.strikethrough,
            );
            surface.pop_clip();
            drawn += 1;
        }
    }

    drawn
}
