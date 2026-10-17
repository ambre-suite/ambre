// ambre-trame/src/render/walk.rs
//
// The sheet's document walk (`CANVAS-074`): cell fills, cell text, grid lines,
// merges and borders, laid out in sheet coordinates at zoom 1. The screen
// pushes its camera before the call; the walk knows no scroll position, no
// zoom, no header strip and no dirty cell.

use std::collections::HashMap;

use ambre_draw::DocumentWalk;

use super::*;
use crate::{TOTAL_COLS, TOTAL_ROWS};

/// One cell as the walk reads it.
#[derive(Clone, Copy)]
pub struct SheetCell<'a> {
    /// What the cell shows. A merge master carries its own text here.
    pub content: CellContent<'a>,
    /// `Some((col_span, row_span))` on a merge master.
    pub merge_span: Option<(u32, u32)>,
}

/// The cells a sheet view holds. The screen reads its cache through it.
pub trait SheetCells {
    /// The cell at `(row, col)`, or `None` if the view holds none there.
    fn cell(&self, row: u32, col: u32) -> Option<SheetCell<'_>>;

    /// Call `f` with `(row, col, col_span, row_span, content)` for each merge
    /// master the view holds.
    fn for_each_merge(&self, f: &mut dyn FnMut(u32, u32, u32, u32, CellContent<'_>));
}

/// The two axes of a sheet, in sheet coordinates at zoom 1.
///
/// Column `0` starts at `x = 0` and row `0` at `y = 0`. A hidden row takes no
/// height. Each position sums the explicit sizes only: it costs the number of
/// resized columns or rows, never the column or row index.
#[derive(Clone, Copy)]
pub struct SheetAxes<'v> {
    /// Width of a column with no entry in `col_widths`.
    pub col_width: f64,
    /// Height of a row with no entry in `row_heights`.
    pub row_height: f64,
    /// Explicit column widths, keyed by column.
    pub col_widths: &'v HashMap<u32, f32>,
    /// Explicit row heights, keyed by row.
    pub row_heights: &'v HashMap<u32, f32>,
    /// Rows hidden by the shared filter, sorted ascending.
    pub hidden_rows: &'v [u32],
}

impl SheetAxes<'_> {
    /// True if `row` is hidden by the shared filter.
    pub fn is_hidden(&self, row: u32) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// X of column `col`'s left edge.
    pub fn col_x(&self, col: u32) -> f64 {
        let resized: f64 = self
            .col_widths
            .iter()
            .filter(|&(&c, _)| c < col)
            .map(|(_, &w)| w as f64 - self.col_width)
            .sum();
        col as f64 * self.col_width + resized
    }

    /// Y of row `row`'s top edge. The hidden rows above it take no height.
    pub fn row_y(&self, row: u32) -> f64 {
        let hidden = self.hidden_rows.partition_point(|&r| r < row) as u32;
        let resized: f64 = self
            .row_heights
            .iter()
            .filter(|&(&r, _)| r < row && !self.is_hidden(r))
            .map(|(_, &h)| h as f64 - self.row_height)
            .sum();
        (row - hidden) as f64 * self.row_height + resized
    }

    /// The column whose band holds `x`, clamped to the grid.
    pub fn col_at(&self, x: f64) -> u32 {
        first(TOTAL_COLS, |c| self.col_x(c + 1) > x).min(TOTAL_COLS - 1)
    }

    /// The row whose band holds `y`, clamped to the grid.
    pub fn row_at(&self, y: f64) -> u32 {
        first(TOTAL_ROWS, |r| self.row_y(r + 1) > y).min(TOTAL_ROWS - 1)
    }
}

/// The first index in `0..n` where the monotone `pred` holds, or `n`.
fn first(n: u32, pred: impl Fn(u32) -> bool) -> u32 {
    let (mut lo, mut hi) = (0u32, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// A sheet as the walk reads it.
pub struct SheetView<'v> {
    /// The cells.
    pub cells: &'v dyn SheetCells,
    /// The axes the cells are laid out on.
    pub axes: SheetAxes<'v>,
    /// The paint of the fills, lines and text.
    pub palette: &'v Palette,
    /// Draw the cell text. Off, only the fills, lines and borders land.
    pub text_enabled: bool,
}

/// The sheet's walk: the shaping context and the caches it keeps between
/// calls.
pub struct SheetWalk<'t> {
    /// The shaping context.
    pub font_cx: &'t mut FontContext,
    /// The shaped-text cache.
    pub text_cache: &'t mut TextCache,
    /// Cells whose text the last call drew.
    pub cells_drawn: u32,
}

impl DocumentWalk for SheetWalk<'_> {
    type View<'v> = SheetView<'v>;

    fn walk<S: DrawSurface>(&mut self, view: &SheetView<'_>, region: Rect, surface: &mut S) {
        let axes = &view.axes;
        let palette = view.palette;
        let (c0, r0) = (axes.col_at(region.x0), axes.row_at(region.y0));
        let c1 = (axes.col_at(region.x1) + 1).min(TOTAL_COLS);
        let r1 = (axes.row_at(region.y1) + 1).min(TOTAL_ROWS);
        // The primitives lay out from an origin cell: pin it at its own sheet
        // position, so every rect they produce is in sheet coordinates.
        let layout = GridLayout {
            hidden_rows: axes.hidden_rows,
            header_width: axes.col_x(c0),
            header_height: axes.row_y(r0),
            top: 0.0,
            col_width: axes.col_width,
            row_height: axes.row_height,
            col_widths: axes.col_widths,
            row_heights: axes.row_heights,
            origin_col: c0,
            origin_row: r0,
            width: region.x1,
            height: region.y1,
            total_cols: TOTAL_COLS,
            total_rows: TOTAL_ROWS,
            zoom: 1.0,
        };
        let hits = |rect: Rect| {
            let hit = rect.intersect(region);
            hit.width() > 0.0 && hit.height() > 0.0
        };

        surface.push_clip(region);

        // Fills and text. A merge master's text is laid out by the merge pass
        // over its span, so this pass draws its fill only.
        let cells = (r0..r1)
            .filter(|&row| !axes.is_hidden(row))
            .flat_map(|row| {
                (c0..c1).filter_map(move |col| {
                    let cell = view.cells.cell(row, col)?;
                    let mut content = cell.content;
                    if cell.merge_span.is_some() {
                        content.text = "";
                    }
                    Some((row, col, content))
                })
            });
        // An overflowing line spills over a neighbour with no text. A merge
        // master keeps its text, so overflow stops at a merged block.
        let is_empty = |row: u32, col: u32| {
            view.cells
                .cell(row, col)
                .is_none_or(|c| c.content.text.is_empty())
        };
        self.cells_drawn = draw_cells_on(
            surface,
            &layout,
            palette,
            self.font_cx,
            self.text_cache,
            view.text_enabled,
            cells,
            is_empty,
        );

        // Grid lines over the fills.
        draw_grid_lines_on(surface, &layout, palette.grid_line, region);

        // Merge masters over the separators of the cells they cover. A master
        // outside the region still draws the part of its span inside it.
        view.cells
            .for_each_merge(&mut |row, col, col_span, row_span, content| {
                if hits(layout.merge_rect(row, col, col_span, row_span)) {
                    draw_merges_on(
                        surface,
                        &layout,
                        palette,
                        self.font_cx,
                        self.text_cache,
                        view.text_enabled,
                        std::iter::once((row, col, col_span, row_span, content)),
                    );
                }
            });

        // Borders last, so no separator and no merge fill paints over one. A
        // plain cell's border wraps its rect, a master's wraps its span.
        let borders = (r0..r1)
            .filter(|&row| !axes.is_hidden(row))
            .flat_map(|row| {
                (c0..c1).filter_map(move |col| {
                    let cell = view.cells.cell(row, col)?;
                    cell.merge_span
                        .is_none()
                        .then(|| (layout.cell_rect(row, col), cell.content))
                })
            });
        draw_cell_borders_on(surface, borders, palette.cell_border);
        view.cells
            .for_each_merge(&mut |row, col, col_span, row_span, content| {
                let rect = layout.merge_rect(row, col, col_span, row_span);
                if hits(rect) {
                    draw_cell_borders_on(
                        surface,
                        std::iter::once((rect, content)),
                        palette.cell_border,
                    );
                }
            });

        surface.pop_clip();
    }
}
