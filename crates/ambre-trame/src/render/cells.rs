// ambre-trame/src/render/cells.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;
use super::content::{cell_alignment, cell_v_offset, rgb};

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
    layout: &GridLayout<'_>,
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
    let cell_max_width = (layout.col_width - 8.0) as f32;
    let mut drawn = 0u32;

    for (row, col, content) in cells {
        let rect = layout.cell_rect(row, col);
        // Lift the cell out of the pending grey with its fill (the manual fill
        // colour if set, else the opaque content background).
        let bg = content.bg.map(rgb).unwrap_or(palette.content_bg);
        surface.fill_rect(rect, bg);

        // Borders are a separate pass (draw_cell_borders_on) AFTER the grid lines,
        // so a deliberate border is never overpainted by a faint separator.

        if !text_enabled || content.text.is_empty() {
            continue;
        }

        // Wrap constrains the layout to the cell width so parley breaks lines;
        // otherwise the text lays out unbounded (Excel-style overflow). We always
        // shape left-aligned and position the run horizontally ourselves: parley's
        // own alignment needs a finite advance, and an unbounded (non-wrap) layout
        // would push centred/right text off-screen.
        let max_w = if content.wrap { cell_max_width } else { f32::MAX };
        let font_size = content.size.map(|s| s as f32).unwrap_or(BODY_SIZE);
        let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build_font(
            font_cx,
            content.text,
            font_size,
            max_w,
            Alignment::Start,
            content.bold,
            content.italic,
            content.font,
        );
        let text_h = layout_text.height() as f64;
        let text_w = layout_text.width() as f64;
        let y_off = cell_v_offset(content.v_align, layout.row_height, text_h);
        // Horizontal alignment within the column content width (Auto resolves
        // numbers right, text left). Overflow keeps the text pinned to the start.
        let avail = cell_max_width as f64;
        let x_off = match cell_alignment(content.h_align, content.text) {
            Alignment::Center => ((avail - text_w) / 2.0).max(0.0),
            Alignment::End | Alignment::Right => (avail - text_w).max(0.0),
            _ => 0.0,
        };
        // Clip wrapped or vertically oversized text to the cell box; a non-wrapped
        // single line is allowed to overflow into empty neighbours.
        let needs_clip = text_h > layout.row_height || content.wrap;
        if needs_clip {
            surface.push_clip(rect);
        }
        let brush = Brush::Solid(content.fg.map(rgb).unwrap_or(palette.cell_text));
        for line in layout_text.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = rect.x0 + CELL_PAD_X + x_off + run.offset() as f64;
                let ty = rect.y0 + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
                let dec_color = content.fg.map(rgb).unwrap_or(palette.cell_text);
                let adv = run.advance() as f64;
                // Underline run: a 1px line just below the glyph baseline.
                if content.underline {
                    surface.stroke_line(tx, ty + 2.0, tx + adv, ty + 2.0, dec_color, 1.0);
                }
                // Strikethrough run: a 1px line through the x-height.
                if content.strikethrough {
                    let sy = ty - (font_size as f64) * 0.3;
                    surface.stroke_line(tx, sy, tx + adv, sy, dec_color, 1.0);
                }
            }
        }
        if needs_clip {
            surface.pop_clip();
        }
        drawn += 1;
    }

    drawn
}

