// ambre-trame/src/render/cells.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::content::{TextFit, cell_alignment, cell_v_offset, rgb};
use super::*;
use ambre_core::SurfacePx;

// -----------------------------------------------------------------------------
// draw_cells_on -- cell backgrounds + body text
// -----------------------------------------------------------------------------

/// Width (logical px) of the consecutive *empty* neighbour columns a non-wrapped
/// overflowing cell may spill its text over. The walk starts at `at` and steps
/// by `step` (`+1` right, `-1` left). It stops at whichever comes first: the
/// first non-empty neighbour, the grid edge, or `need` pixels of spill covered
/// (Excel/Sheets overflow, `trame-cell-rendering`). The result never exceeds
/// `need`.
pub(super) fn spill_width<E: Fn(CellPos) -> bool>(
    layout: &GridLayout<'_>,
    at: CellPos,
    step: ColDelta,
    need: SurfacePx,
    is_empty: &E,
) -> SurfacePx {
    let mut acc = SurfacePx::ZERO;
    let mut next = at.col.checked_offset(step);
    while let Some(c) = next
        && acc < need
        && c < layout.total_cols.as_end()
    {
        if !is_empty(CellPos::new(at.row, c)) {
            break;
        }
        acc += layout.col_w(c);
        next = c.checked_offset(step);
    }
    acc.min(need)
}

/// Draw the background fill and body text for each supplied cell.
///
/// `cells` yields `(at, content)` for the cells that exist (on screen: the
/// cells received from the Logic thread; for PDF: every used cell). Each gets an
/// opaque `palette.content_bg` fill, followed by its text when `text_pass`. On
/// screen that fill is what lifts a cell out of the "pending" grey.
///
/// `is_empty(at)` reports whether a neighbour cell carries no visible text,
/// so a non-wrapped overflowing line may spill over it. Overflow direction
/// follows the resolved alignment: left text spills right, right text spills
/// left, centred text both ways. The spill stops at the first non-empty neighbour
/// and the text is clipped there (no ellipsis in the MVP,
/// `trame-cell-rendering`).
///
/// Grid lines are *not* drawn here: call [`draw_grid_lines_on`] afterwards so the
/// separators sit on top of the fills. Returns the number of cells that actually
/// had non-empty text drawn (font-independent tally for stats/tests).
#[allow(clippy::too_many_arguments)]
pub fn draw_cells_on<'a, S, I, E>(
    surface: &mut S,
    layout: &GridLayout<'_>,
    palette: &Palette,
    font_cx: &mut FontContext,
    text_cache: &mut TextCache,
    text_pass: TextPass,
    cells: I,
    is_empty: E,
) -> ambre_canvas::CellsDrawn
where
    S: DrawSurface,
    I: IntoIterator<Item = (CellPos, CellContent<'a>)>,
    E: Fn(CellPos) -> bool,
{
    let mut drawn = ambre_canvas::CellsDrawn::ZERO;

    for (at, content) in cells {
        let CellPos { row, col } = at;
        let rect = layout.cell_rect(at);
        // Per-cell metrics (variable column width / row height).
        // The surface draws in bare f64: the lengths unwrap once, here.
        let col_w = layout.col_w(col).get();
        let row_h = layout.row_h(row);
        let cell_max_width = (col_w - 8.0) as f32;
        // Lift the cell out of the pending grey with its fill (the manual fill
        // colour if set, else the opaque content background).
        let bg = content.bg.map(rgb).unwrap_or(palette.content_bg);
        surface.fill_rect(rect, bg);

        // Borders are a separate pass (draw_cell_borders_on) AFTER the grid lines,
        // so a deliberate border is never overpainted by a faint separator.

        if text_pass == TextPass::Shapes || content.text.is_empty() {
            continue;
        }

        // Wrap constrains the layout to the cell width so parley breaks lines;
        // otherwise the text lays out unbounded (Excel-style overflow). We always
        // shape left-aligned and position the run horizontally ourselves. Parley's
        // own alignment needs a finite advance, and an unbounded (non-wrap) layout
        // would push centred/right text off-screen.
        let wraps = content.wrap == ambre_canvas::WordBreak::Anywhere;
        let max_w = if wraps { cell_max_width } else { f32::MAX };
        // True zoom: the glyph size scales with the grid (the per-axis metrics
        // already do via `col_w`/`row_h`).
        let font_size = super::cell_font_size(content.size).zoomed(layout.zoom);
        let layout_text: &Layout<ParleyBrush> = text_cache.get_or_build_font(
            font_cx,
            content.text,
            font_size,
            ViewPx::new(max_w),
            Alignment::Start,
            content.bold,
            content.italic,
            content.font,
            content.wrap,
        );
        let text_h = layout_text.height() as f64;
        let text_w = layout_text.width() as f64;
        let fit = TextFit {
            box_height: row_h,
            text_height: SurfacePx::new(text_h),
        };
        let y_off = cell_v_offset(content.v_align, fit).get();
        let avail = cell_max_width as f64;
        // A non-wrapped single line wider than its column spills into empty
        // neighbour cells (`trame-cell-rendering`); wrapped or multi-line text is
        // clipped to its own box. `single_line` guards against newlines pushing a
        // non-wrap value taller than the row -- that case clips like wrap does.
        let align = cell_alignment(content.h_align, content.numeric);
        let single_line = text_h <= row_h.get();
        let overflow = !wraps && single_line && text_w > avail;
        let (x_off, clip_rect, needs_clip) = if overflow {
            // Extend the clip rect into the empty neighbours in the spill
            // direction(s); the text is shifted so its anchored edge stays put.
            let need = SurfacePx::new(text_w - avail);
            match align {
                Alignment::Center => {
                    let half = need / 2.0;
                    let left = spill_width(layout, at, ColDelta::new(-1), half, &is_empty).get();
                    let right = spill_width(layout, at, ColDelta::new(1), half, &is_empty).get();
                    let clip = Rect::new(rect.x0 - left, rect.y0, rect.x1 + right, rect.y1);
                    (-left, clip, true)
                }
                Alignment::End | Alignment::Right => {
                    let left = spill_width(layout, at, ColDelta::new(-1), need, &is_empty).get();
                    let clip = Rect::new(rect.x0 - left, rect.y0, rect.x1, rect.y1);
                    // Right-anchored: keep the right edge inside the cell content
                    // box (negative offset), clipped on the left at the spill.
                    (avail - text_w, clip, true)
                }
                _ => {
                    let right = spill_width(layout, at, ColDelta::new(1), need, &is_empty).get();
                    let clip = Rect::new(rect.x0, rect.y0, rect.x1 + right, rect.y1);
                    (0.0, clip, true)
                }
            }
        } else {
            // Fits, or wrapped / multi-line: position within the cell content box
            // and clip only the cases that can overshoot it vertically.
            let x = match align {
                Alignment::Center => ((avail - text_w) / 2.0).max(0.0),
                Alignment::End | Alignment::Right => (avail - text_w).max(0.0),
                _ => 0.0,
            };
            (x, rect, wraps || !single_line)
        };
        if needs_clip {
            surface.push_clip(clip_rect);
        }
        let fg = content.fg.map(rgb).unwrap_or(palette.cell_text);
        let brush = Brush::Solid(fg);
        let origin_x = rect.x0 + layout.pad_x().get() + x_off;
        draw_layout_runs_decorated(
            surface,
            layout_text,
            Point::new(origin_x, rect.y0 + y_off),
            &brush,
            fg,
            font_size,
            Decoration::of(&content),
        );
        if needs_clip {
            surface.pop_clip();
        }
        drawn = drawn.incremented();
    }

    drawn
}
