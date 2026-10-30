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
    // The surface draws in bare f64: the lengths unwrap once, here.
    let cells_top = layout.cells_top().get();
    let (width, height) = (layout.width.get(), layout.height.get());
    let line_y0 = area.y0.max(cells_top);
    let line_y1 = area.y1.min(height);
    if line_y1 > line_y0 {
        // Walk the variable column widths, stroking a separator at each left edge.
        let mut col = layout.origin_col;
        let mut x = layout.header_width.get();
        loop {
            if col > layout.total_cols.as_end() || x > width {
                break;
            }
            if x >= area.x0 && x <= area.x1 {
                surface.stroke_line(
                    ambre_draw::Line::new((x, line_y0), (x, line_y1)),
                    color,
                    ambre_core::SurfacePx::new(1.0),
                );
            }
            x += layout.col_w(col).get();
            col = col.next();
        }
    }

    // Horizontal lines at each visible row boundary.
    let line_x0 = area.x0.max(layout.header_width.get());
    let line_x1 = area.x1.min(width);
    if line_x1 > line_x0 {
        // Walk the *visible* rows' variable heights (hidden rows occupy no space).
        let mut row = layout.origin_row;
        let mut y = cells_top;
        loop {
            if row > layout.total_rows.as_end() || y > height {
                break;
            }
            if y >= area.y0 && y <= area.y1 {
                surface.stroke_line(
                    ambre_draw::Line::new((line_x0, y), (line_x1, y)),
                    color,
                    ambre_core::SurfacePx::new(1.0),
                );
            }
            if !layout.is_hidden(row) {
                y += layout.row_h(row).get();
            }
            row = row.next();
        }
    }
}
