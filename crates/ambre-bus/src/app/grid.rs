//! Grid coordinates: a row, a column, the cell at their crossing and the
//! extent of a merge. Trame's sheet (`trame-rows-and-columns`) and Prose's
//! tables (`features/ambre-spec-prose-tables.md`) both count cells this way.
//!
//! A row and a column are both small unsigned integers, and most signatures of
//! a grid carry two of them side by side. One family per axis keeps a column
//! out of a row's place, and one struct per pair fixes the order of the pair.
//! Every position is 0-based.

use ambre_core::define_index;
use serde::{Deserialize, Serialize};

define_index! {
    /// A grid row, 0-based.
    pub struct Row(u32);
    /// A number of rows.
    pub struct RowCount;
    /// The signed gap between two rows.
    pub struct RowDelta(i64);
    /// A run of rows.
    pub struct RowSpan;
    /// A list indexed by row.
    pub struct RowVec;
}

define_index! {
    /// A grid column, 0-based.
    pub struct Col(u32);
    /// A number of columns.
    pub struct ColCount;
    /// The signed gap between two columns.
    pub struct ColDelta(i64);
    /// A run of columns.
    pub struct ColSpan;
    /// A list indexed by column.
    pub struct ColVec;
}

/// The cell at the crossing of a row and a column.
///
/// The derived order is row-major: rows first, then columns.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CellPos {
    pub row: Row,
    pub col: Col,
}

impl CellPos {
    /// The cell at `row` and `col`.
    pub const fn new(row: Row, col: Col) -> Self {
        Self { row, col }
    }

    /// The top-left cell of the grid.
    pub const ORIGIN: Self = Self::new(Row::ZERO, Col::ZERO);
}

/// A rectangle of cells, both corners included. The top-left corner is never
/// below or right of the bottom-right one.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CellRect {
    pub top_left: CellPos,
    pub bottom_right: CellPos,
}

impl CellRect {
    /// The rectangle between two corners, in any order.
    pub fn spanning(a: CellPos, b: CellPos) -> Self {
        Self {
            top_left: CellPos::new(a.row.min(b.row), a.col.min(b.col)),
            bottom_right: CellPos::new(a.row.max(b.row), a.col.max(b.col)),
        }
    }

    /// The rectangle of the one cell `at`.
    pub const fn cell(at: CellPos) -> Self {
        Self {
            top_left: at,
            bottom_right: at,
        }
    }

    /// The rows the rectangle covers.
    pub const fn rows(self) -> RowSpan {
        self.top_left.row.through(self.bottom_right.row)
    }

    /// The columns the rectangle covers.
    pub const fn cols(self) -> ColSpan {
        self.top_left.col.through(self.bottom_right.col)
    }

    /// How many rows the rectangle covers.
    pub const fn row_count(self) -> RowCount {
        self.rows().len()
    }

    /// How many columns the rectangle covers.
    pub const fn col_count(self) -> ColCount {
        self.cols().len()
    }

    /// Whether `at` lies inside the rectangle.
    pub const fn contains(self, at: CellPos) -> bool {
        self.rows().contains(at.row) && self.cols().contains(at.col)
    }

    /// Whether the rectangle holds one cell alone.
    pub fn is_single_cell(self) -> bool {
        self.top_left == self.bottom_right
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Self) -> Self {
        Self {
            top_left: CellPos::new(
                self.top_left.row.min(other.top_left.row),
                self.top_left.col.min(other.top_left.col),
            ),
            bottom_right: CellPos::new(
                self.bottom_right.row.max(other.bottom_right.row),
                self.bottom_right.col.max(other.bottom_right.col),
            ),
        }
    }

    /// Whether the two rectangles share a cell.
    pub fn overlaps(self, other: Self) -> bool {
        self.top_left.row <= other.bottom_right.row
            && other.top_left.row <= self.bottom_right.row
            && self.top_left.col <= other.bottom_right.col
            && other.top_left.col <= self.bottom_right.col
    }

    /// Every cell of the rectangle, row by row.
    pub fn cells(self) -> impl Iterator<Item = CellPos> {
        let cols = self.cols();
        self.rows()
            .into_iter()
            .flat_map(move |row| cols.into_iter().map(move |col| CellPos::new(row, col)))
    }

    /// The extent of the rectangle as a merge.
    pub const fn extent(self) -> MergeSpan {
        MergeSpan::new(self.row_count(), self.col_count())
    }
}

/// The extent of a merged block from its top-left cell: how many rows and
/// columns it covers, the top-left cell included. A 1 x 1 extent is no merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MergeSpan {
    pub rows: RowCount,
    pub cols: ColCount,
}

impl MergeSpan {
    /// The extent of one cell alone.
    pub const CELL: Self = Self::new(RowCount::ONE, ColCount::ONE);

    /// The extent of `rows` rows by `cols` columns.
    pub const fn new(rows: RowCount, cols: ColCount) -> Self {
        Self { rows, cols }
    }

    /// The bottom-right cell of the block whose top-left cell is `master`.
    pub fn last_from(self, master: CellPos) -> CellPos {
        CellPos::new(
            (master.row + self.rows).prev().unwrap_or(master.row),
            (master.col + self.cols).prev().unwrap_or(master.col),
        )
    }

    /// The block whose top-left cell is `master`.
    pub fn block_at(self, master: CellPos) -> CellRect {
        CellRect {
            top_left: master,
            bottom_right: self.last_from(master),
        }
    }
}
