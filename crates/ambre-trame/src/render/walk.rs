// ambre-trame/src/render/walk.rs
//
// The sheet's document walk (`CANVAS-074`): cell fills, cell text, grid lines,
// merges and borders, laid out in sheet coordinates at zoom 1. The screen
// pushes its camera before the call; the walk knows no scroll position, no
// zoom, no header strip and no dirty cell.

use ambre_core::index::{Pos, Span};
use std::collections::HashMap;

use ambre_core::{Doc, Px, Surface};
use ambre_draw::DocumentWalk;

use super::*;
use crate::{Col, LAST_COL, LAST_ROW, Row, TOTAL_COLS, TOTAL_ROWS};

/// One cell as the walk reads it.
#[derive(Clone, Copy)]
pub struct SheetCell<'a> {
    /// What the cell shows. A merge master carries its own text here.
    pub content: CellContent<'a>,
    /// The merged block's extent on a merge master.
    pub merge_span: Option<MergeSpan>,
}

/// The cells a sheet view holds. The screen reads its cache through it.
pub trait SheetCells {
    /// The cell at `at`, or `None` if the view holds none there.
    fn cell(&self, at: CellPos) -> Option<SheetCell<'_>>;

    /// Call `f` with `(master, span, content)` for each merge master the view
    /// holds.
    fn for_each_merge(&self, f: &mut dyn FnMut(CellPos, MergeSpan, CellContent<'_>));
}

/// The two axes of a sheet, in sheet coordinates at zoom 1.
///
/// Column `0` starts at `x = 0` and row `0` at `y = 0`. A hidden row takes no
/// height. Each position sums the explicit sizes only: it costs the number of
/// resized columns or rows, never the column or row index.
#[derive(Clone, Copy)]
pub struct SheetAxes<'v> {
    /// Width of a column with no entry in `col_widths`.
    pub col_width: Px<Doc>,
    /// Height of a row with no entry in `row_heights`.
    pub row_height: Px<Doc>,
    /// Explicit column widths, keyed by column.
    pub col_widths: &'v HashMap<Pos<Col>, Px<Doc>>,
    /// Explicit row heights, keyed by row.
    pub row_heights: &'v HashMap<Pos<Row>, Px<Doc>>,
    /// Rows hidden by the shared filter, sorted ascending.
    pub hidden_rows: &'v [Pos<Row>],
}

impl SheetAxes<'_> {
    /// True if `row` is hidden by the shared filter.
    pub fn is_hidden(&self, row: Pos<Row>) -> bool {
        self.hidden_rows.binary_search(&row).is_ok()
    }

    /// X of column `col`'s left edge.
    pub fn col_x(&self, col: Pos<Col>) -> Px<Surface> {
        let resized: f64 = self
            .col_widths
            .iter()
            .filter(|&(&c, _)| c < col)
            .map(|(_, &w)| f64::from(w.get()) - f64::from(self.col_width.get()))
            .sum();
        Px::<Surface>::new(f64::from(col.get()) * f64::from(self.col_width.get()) + resized)
    }

    /// Y of row `row`'s top edge. The hidden rows above it take no height.
    pub fn row_y(&self, row: Pos<Row>) -> Px<Surface> {
        let hidden = self.hidden_rows.partition_point(|&r| r < row) as f64;
        let resized: f64 = self
            .row_heights
            .iter()
            .filter(|&(&r, _)| r < row && !self.is_hidden(r))
            .map(|(_, &h)| f64::from(h.get()) - f64::from(self.row_height.get()))
            .sum();
        Px::<Surface>::new(
            (f64::from(row.get()) - hidden) * f64::from(self.row_height.get()) + resized,
        )
    }

    /// The column whose band holds `x`, clamped to the grid.
    pub fn col_at(&self, x: Px<Surface>) -> Pos<Col> {
        Span::<Col>::from_count(Pos::<Col>::ZERO, TOTAL_COLS)
            .first_where(|c| self.col_x(c.next()) > x)
            .min(LAST_COL)
    }

    /// The row whose band holds `y`, clamped to the grid.
    pub fn row_at(&self, y: Px<Surface>) -> Pos<Row> {
        Span::<Row>::from_count(Pos::<Row>::ZERO, TOTAL_ROWS)
            .first_where(|r| self.row_y(r.next()) > y)
            .min(LAST_ROW)
    }
}

/// A sheet as the walk reads it.
pub struct SheetView<'v> {
    /// The cells.
    pub cells: &'v dyn SheetCells,
    /// The axes the cells are laid out on.
    pub axes: SheetAxes<'v>,
    /// The paint of the fills, lines and text.
    pub palette: &'v Palette,
    /// Whether the cell text is drawn, or only the fills, lines and borders.
    pub text_pass: TextPass,
}

/// Whether a paint of the sheet draws its text (the debug HUD toggles it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextPass {
    /// Only the fills, lines and borders land.
    Shapes,
    /// The text lands over them.
    #[default]
    WithText,
}

/// The sheet's walk: the shaping context and the caches it keeps between
/// calls.
pub struct SheetWalk<'t> {
    /// The shaping context.
    pub font_cx: &'t mut FontContext,
    /// The shaped-text cache.
    pub text_cache: &'t mut TextCache,
    /// Cells whose text the last call drew.
    pub cells_drawn: ambre_core::Tally<ambre_canvas::CellsDrawn>,
}

impl DocumentWalk for SheetWalk<'_> {
    type View<'v> = SheetView<'v>;

    fn walk<S: DrawSurface>(&mut self, view: &SheetView<'_>, region: Rect, surface: &mut S) {
        let axes = &view.axes;
        let palette = view.palette;
        let (c0, r0) = (
            axes.col_at(Px::<Surface>::new(region.x0)),
            axes.row_at(Px::<Surface>::new(region.y0)),
        );
        let c1 = axes
            .col_at(Px::<Surface>::new(region.x1))
            .next()
            .min(TOTAL_COLS.as_end());
        let r1 = axes
            .row_at(Px::<Surface>::new(region.y1))
            .next()
            .min(TOTAL_ROWS.as_end());
        let (cols, rows) = (Span::<Col>::new(c0, c1), Span::<Row>::new(r0, r1));
        // The primitives lay out from an origin cell: pin it at its own sheet
        // position, so every rect they produce is in sheet coordinates.
        let layout = GridLayout {
            hidden_rows: axes.hidden_rows,
            header_width: axes.col_x(c0),
            header_height: axes.row_y(r0),
            top: Px::<Surface>::ZERO,
            col_width: axes.col_width,
            row_height: axes.row_height,
            col_widths: axes.col_widths,
            row_heights: axes.row_heights,
            origin_col: c0,
            origin_row: r0,
            width: Px::<Surface>::new(region.x1),
            height: Px::<Surface>::new(region.y1),
            total_cols: TOTAL_COLS,
            total_rows: TOTAL_ROWS,
            zoom: ambre_core::Scale::<ambre_core::Doc, ambre_core::View>::positive(1.0),
        };
        let hits = |rect: Rect| {
            let hit = rect.intersect(region);
            hit.width() > 0.0 && hit.height() > 0.0
        };

        surface.push_clip(region);

        // Fills and text. A merge master's text is laid out by the merge pass
        // over its span, so this pass draws its fill only.
        let cells = rows
            .into_iter()
            .filter(|&row| !axes.is_hidden(row))
            .flat_map(|row| {
                cols.into_iter().filter_map(move |col| {
                    let at = CellPos::new(row, col);
                    let cell = view.cells.cell(at)?;
                    let mut content = cell.content;
                    if cell.merge_span.is_some() {
                        content.text = "";
                    }
                    Some((at, content))
                })
            });
        // An overflowing line spills over a neighbour with no text. A merge
        // master keeps its text, so overflow stops at a merged block.
        let is_empty = |at: CellPos| {
            view.cells
                .cell(at)
                .is_none_or(|c| c.content.text.is_empty())
        };
        self.cells_drawn = draw_cells_on(
            surface,
            &layout,
            palette,
            self.font_cx,
            self.text_cache,
            view.text_pass,
            cells,
            is_empty,
        );

        // Grid lines over the fills.
        draw_grid_lines_on(surface, &layout, palette.grid_line, region);

        // Merge masters over the separators of the cells they cover. A master
        // outside the region still draws the part of its span inside it.
        view.cells.for_each_merge(&mut |master, span, content| {
            if hits(layout.merge_rect(master, span)) {
                draw_merges_on(
                    surface,
                    &layout,
                    palette,
                    self.font_cx,
                    self.text_cache,
                    view.text_pass,
                    std::iter::once((master, span, content)),
                );
            }
        });

        // Borders last, so no separator and no merge fill paints over one. A
        // plain cell's border wraps its rect, a master's wraps its span.
        let borders = rows
            .into_iter()
            .filter(|&row| !axes.is_hidden(row))
            .flat_map(|row| {
                cols.into_iter().filter_map(move |col| {
                    let at = CellPos::new(row, col);
                    let cell = view.cells.cell(at)?;
                    cell.merge_span
                        .is_none()
                        .then(|| (layout.cell_rect(at), cell.content))
                })
            });
        draw_cell_borders_on(surface, borders, palette.cell_border);
        view.cells.for_each_merge(&mut |master, span, content| {
            let rect = layout.merge_rect(master, span);
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
