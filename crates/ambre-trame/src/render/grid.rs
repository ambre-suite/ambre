// ambre-trame/src/render/grid.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;

// -----------------------------------------------------------------------------
// draw_grid_lines_on -- 1px cell separators
// -----------------------------------------------------------------------------

/// Stroke the grid separators inside `clip`, intersected with the content area.
///
/// Drawn *after* the cell fills so the 1px boundaries stay visible between
/// adjacent filled cells.
pub fn draw_grid_lines_on<S: DrawSurface>(
    surface: &mut S,
    layout: &GridLayout<'_>,
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

