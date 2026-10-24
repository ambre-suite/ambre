// ambre-trame/src/render/layout.rs.rs
//
// Part of the Trame software renderer, split out of render.rs by concern.
// Submodule of `render`; reaches sibling helpers, constants and imports through
// the `use super::*` glob (the parent re-exports every renderer item).

use std::collections::HashMap;

use ambre_core::{DocPx, SurfacePx};

use super::*;

// -----------------------------------------------------------------------------
// GridLayout -- index <-> pixel mapping
// -----------------------------------------------------------------------------

/// Maps absolute grid indices to pixel rectangles on a target surface.
///
/// `origin_row` / `origin_col` are the absolute grid indices pinned to the
/// top-left corner of the content area: the scroll position, cell-aligned for
/// the MVP. For PDF export the origin is `(0, 0)`.
///
/// MVP: uniform `col_width` / `row_height`. The fields are deliberately shaped
/// so a future variable-size layout can add per-index offset slices without
/// breaking callers.
#[derive(Clone, Copy, Debug)]
pub struct GridLayout<'a> {
    /// Absolute grid rows hidden by the shared filter (`trame-shared-filter`),
    /// sorted
    /// ascending. Such rows occupy no vertical space: the row axis collapses over
    /// them, so a hidden row's successors slide up by one `row_height`. Empty
    /// (the common case and PDF export) means a plain uniform layout. The slice
    /// only needs to cover the visible window -- rows beyond it are treated as
    /// visible.
    pub hidden_rows: &'a [Row],
    /// Width of the fixed row-number strip on the left.
    pub header_width: SurfacePx,
    /// Height of the fixed column-letter strip on the top.
    pub header_height: SurfacePx,
    /// Vertical offset (logical px) of the whole grid from the surface top.
    /// This is the height of any band reserved above the column-header
    /// strip: the formula bar on screen, `0.0` for PDF export.
    pub top: f64,
    /// Default column width (content area), used for any column without an
    /// explicit entry in [`col_widths`](Self::col_widths).
    pub col_width: DocPx,
    /// Default row height (content area), used for any row without an explicit
    /// entry in [`row_heights`](Self::row_heights).
    pub row_height: DocPx,
    /// Per-column widths (logical px) keyed by absolute grid column. A column
    /// with no entry (the common case, and PDF export with an empty map) falls
    /// back to the uniform [`col_width`](Self::col_width). Fed by the Logic
    /// thread's `col_widths_{sheet}` map (`trame-resize`).
    pub col_widths: &'a HashMap<Col, DocPx>,
    /// Per-row heights keyed by absolute grid row. Row analogue of
    /// [`col_widths`](Self::col_widths).
    pub row_heights: &'a HashMap<Row, DocPx>,
    /// Absolute grid column at the content area's left edge.
    pub origin_col: Col,
    /// Absolute grid row at the content area's top edge.
    pub origin_row: Row,
    /// Full surface width in device pixels (header strip included).
    pub width: f64,
    /// Full surface height in device pixels (header strip included).
    pub height: f64,
    /// Total columns in the grid (iteration bound).
    pub total_cols: ColCount,
    /// Total rows in the grid (iteration bound).
    pub total_rows: RowCount,
    /// Application zoom factor (1.0 = 100%). Scales the content-area metrics
    /// (`col_width`/`row_height` and the per-axis maps, which store BASE px). The
    /// header strip sizes (`header_width`/`header_height`) are passed already
    /// scaled by the caller, so they are not re-multiplied here.
    pub zoom: ambre_core::ZoomFactor,
}

impl GridLayout<'_> {
    /// The scrollable content area (everything below + right of the headers).
    pub fn content_area(&self) -> Rect {
        Rect::new(
            self.header_width.get(),
            self.top + self.header_height.get(),
            self.width,
            self.height,
        )
    }

    /// True if `row` is collapsed by the shared filter (binary search over the
    /// sorted [`hidden_rows`](Self::hidden_rows) slice).
    pub fn is_hidden(&self, row: Row) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// Width (logical px) of column `col`: its per-column entry, else the
    /// uniform [`col_width`](Self::col_width).
    pub fn col_w(&self, col: Col) -> f64 {
        let width = self.col_widths.get(&col).copied().unwrap_or(self.col_width);
        f64::from(width.get()) * self.zoom.get()
    }

    /// Height (logical px) of row `row`: its per-row entry, else the uniform
    /// [`row_height`](Self::row_height). A hidden row still reports its natural
    /// height (callers skip it explicitly).
    pub fn row_h(&self, row: Row) -> f64 {
        let height = self
            .row_heights
            .get(&row)
            .copied()
            .unwrap_or(self.row_height);
        f64::from(height.get()) * self.zoom.get()
    }

    /// X of column `col`'s left edge relative to the surface, summing the
    /// variable widths from [`origin_col`](Self::origin_col). Columns left of the
    /// origin yield a coordinate smaller than [`header_width`](Self::header_width)
    /// (callers clip).
    pub fn col_left(&self, col: Col) -> f64 {
        let mut x = self.header_width.get();
        if col >= self.origin_col {
            for c in ColSpan::new(self.origin_col, col) {
                x += self.col_w(c);
            }
        } else {
            for c in ColSpan::new(col, self.origin_col) {
                x -= self.col_w(c);
            }
        }
        x
    }

    /// Y of row `row`'s top edge relative to the surface, summing the variable
    /// heights of the *visible* rows from [`origin_row`](Self::origin_row). The
    /// row axis collapses over filtered rows (`trame-shared-filter`).
    pub fn row_top(&self, row: Row) -> f64 {
        let mut y = self.top + self.header_height.get();
        if row >= self.origin_row {
            for r in RowSpan::new(self.origin_row, row) {
                if !self.is_hidden(r) {
                    y += self.row_h(r);
                }
            }
        } else {
            for r in RowSpan::new(row, self.origin_row) {
                if !self.is_hidden(r) {
                    y -= self.row_h(r);
                }
            }
        }
        y
    }

    /// Absolute grid column whose band contains surface x `px` (in the content
    /// area), walking the variable widths from the origin. `None` left of the
    /// header strip or past the grid's last column.
    pub fn col_at(&self, px: f64) -> Option<Col> {
        if px < self.header_width.get() {
            return None;
        }
        let mut x = self.header_width.get();
        let mut col = self.origin_col;
        while col < self.total_cols.as_end() {
            let w = self.col_w(col);
            if px < x + w {
                return Some(col);
            }
            x += w;
            col = col.next();
        }
        None
    }

    /// Absolute grid row whose band contains surface y `py` (in the content
    /// area), walking the *visible* variable heights from the origin. `None`
    /// above the header strip or past the grid's last row.
    pub fn row_at(&self, py: f64) -> Option<Row> {
        let cells_top = self.top + self.header_height.get();
        if py < cells_top {
            return None;
        }
        let mut y = cells_top;
        let mut row = self.origin_row;
        while row < self.total_rows.as_end() {
            if !self.is_hidden(row) {
                let h = self.row_h(row);
                if py < y + h {
                    return Some(row);
                }
                y += h;
            }
            row = row.next();
        }
        None
    }

    /// Signed number of *visible* row slots between [`origin_row`](Self::origin_row)
    /// and `row`. This is the vertical offset (in `row_height` units) at which `row`
    /// is drawn, with the collapsed rows removed. Equals `row - origin_row` when no
    /// row in between is hidden.
    pub fn visible_offset(&self, row: Row) -> RowDelta {
        let between = RowSpan::new(row.min(self.origin_row), row.max(self.origin_row));
        let hidden = self
            .hidden_rows
            .iter()
            .filter(|&&r| between.contains(r))
            .count() as i64;
        let slots = RowDelta::new((row - self.origin_row).get().abs() - hidden);
        if row >= self.origin_row {
            slots
        } else {
            slots.reversed()
        }
    }

    /// The absolute grid row drawn at visible slot `slot` (0 = the row at
    /// `origin_row`, skipping hidden rows). `None` once the grid is exhausted.
    pub fn row_at_slot(&self, slot: RowCount) -> Option<Row> {
        let mut row = self.origin_row;
        let mut seen = RowCount::ZERO;
        while row < self.total_rows.as_end() {
            if !self.is_hidden(row) {
                if seen == slot {
                    return Some(row);
                }
                seen += RowCount::ONE;
            }
            row = row.next();
        }
        None
    }

    /// Pixel rect of cell `at` relative to the surface top-left.
    ///
    /// The result is unclipped: a cell scrolled left of / above the origin has
    /// a negative-offset (smaller-than-header) coordinate. Callers clip against
    /// `content_area()` or push their own clip. The vertical position accounts
    /// for collapsed (filtered) rows via [`visible_offset`](Self::visible_offset).
    pub fn cell_rect(&self, at: CellPos) -> Rect {
        let x = self.col_left(at.col);
        let y = self.row_top(at.row);
        Rect::new(x, y, x + self.col_w(at.col), y + self.row_h(at.row))
    }

    /// Pixel rect of a merge master `master` spanning `span` (at least one row
    /// and one column). Like [`cell_rect`](Self::cell_rect) the
    /// result is unclipped. The height counts only the *visible* rows in the span
    /// (a merge across a filtered row shrinks accordingly).
    pub fn merge_rect(&self, master: CellPos, span: MergeSpan) -> Rect {
        let base = self.cell_rect(master);
        // Variable-metric span: sum the covered columns' widths and the visible
        // covered rows' heights (a merge across a filtered row shrinks).
        let width: f64 = ColSpan::from_count(master.col, span.cols.max(ColCount::ONE))
            .into_iter()
            .map(|c| self.col_w(c))
            .sum();
        let height: f64 = RowSpan::from_count(master.row, span.rows.max(RowCount::ONE))
            .into_iter()
            .filter(|&r| !self.is_hidden(r))
            .map(|r| self.row_h(r))
            .sum::<f64>()
            .max(self.row_h(master.row));
        Rect::new(base.x0, base.y0, base.x0 + width, base.y0 + height)
    }
}
