// ambre-trame/src/render/layout.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use super::*;

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
pub struct GridLayout<'a> {
    /// Absolute grid rows hidden by the shared filter (`-ui.md` sec.5.D), sorted
    /// ascending. Such rows occupy no vertical space: the row axis collapses over
    /// them, so a hidden row's successors slide up by one `row_height`. Empty
    /// (the common case and PDF export) means a plain uniform layout. The slice
    /// only needs to cover the visible window -- rows beyond it are treated as
    /// visible.
    pub hidden_rows: &'a [u32],
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

impl GridLayout<'_> {
    /// The scrollable content area (everything below + right of the headers).
    pub fn content_area(&self) -> Rect {
        Rect::new(
            self.header_width,
            self.top + self.header_height,
            self.width,
            self.height,
        )
    }

    /// True if `row` is collapsed by the shared filter (binary search over the
    /// sorted [`hidden_rows`](Self::hidden_rows) slice).
    pub fn is_hidden(&self, row: u32) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Signed number of *visible* row slots between [`origin_row`](Self::origin_row)
    /// and `row`: the vertical offset (in `row_height` units) at which `row` is
    /// drawn, with the collapsed rows removed. Equals `row - origin_row` when no
    /// row in between is hidden.
    pub fn visible_offset(&self, row: u32) -> i64 {
        if self.hidden_rows.is_empty() {
            return row as i64 - self.origin_row as i64;
        }
        if row >= self.origin_row {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= self.origin_row && r < row)
                .count() as i64;
            (row - self.origin_row) as i64 - hidden
        } else {
            let hidden = self
                .hidden_rows
                .iter()
                .filter(|&&r| r >= row && r < self.origin_row)
                .count() as i64;
            -((self.origin_row - row) as i64 - hidden)
        }
    }

    /// The absolute grid row drawn at visible slot `slot` (0 = the row at
    /// `origin_row`, skipping hidden rows). `None` once the grid is exhausted.
    pub fn row_at_slot(&self, slot: u32) -> Option<u32> {
        let mut row = self.origin_row;
        let mut seen = 0u32;
        while row < self.total_rows {
            if !self.is_hidden(row) {
                if seen == slot {
                    return Some(row);
                }
                seen += 1;
            }
            row += 1;
        }
        None
    }

    /// Pixel rect of cell `(row, col)` relative to the surface top-left.
    ///
    /// The result is unclipped: a cell scrolled left of / above the origin has
    /// a negative-offset (smaller-than-header) coordinate. Callers clip against
    /// `content_area()` or push their own clip. The vertical position accounts
    /// for collapsed (filtered) rows via [`visible_offset`](Self::visible_offset).
    pub fn cell_rect(&self, row: u32, col: u32) -> Rect {
        let x = self.header_width + (col as i64 - self.origin_col as i64) as f64 * self.col_width;
        let y = self.top + self.header_height + self.visible_offset(row) as f64 * self.row_height;
        Rect::new(x, y, x + self.col_width, y + self.row_height)
    }

    /// Pixel rect of a merge master `(row, col)` spanning `col_span` columns and
    /// `row_span` rows (both >= 1). Like [`cell_rect`](Self::cell_rect) the
    /// result is unclipped. The height counts only the *visible* rows in the span
    /// (a merge across a filtered row shrinks accordingly).
    pub fn merge_rect(&self, row: u32, col: u32, col_span: u32, row_span: u32) -> Rect {
        let base = self.cell_rect(row, col);
        let vis_rows = (row..row + row_span.max(1))
            .filter(|&r| !self.is_hidden(r))
            .count()
            .max(1) as f64;
        Rect::new(
            base.x0,
            base.y0,
            base.x0 + col_span.max(1) as f64 * self.col_width,
            base.y0 + vis_rows * self.row_height,
        )
    }
}
