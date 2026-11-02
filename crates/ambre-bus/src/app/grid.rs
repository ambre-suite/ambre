//! Grid coordinates: a row, a column, the cell at their crossing and the
//! extent of a merge. Trame's sheet (`trame-rows-and-columns`) and Prose's
//! tables (`features/ambre-spec-prose-tables.md`) both count cells this way.
//!
//! A row and a column are both small unsigned integers, and most signatures of
//! a grid carry two of them side by side. One family per axis keeps a column
//! out of a row's place, and one struct per pair fixes the order of the pair.
//! Every position is 0-based.

use ambre_core::index::{Axis, Count, Pos, Span};
use serde::{Deserialize, Serialize};

/// A grid row, 0-based.
///
/// The axis derives the traits a type generic over it derives, such as the
/// structural edit of `ambre-trame`. It holds no value, so none of them runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {}

impl Axis for Row {
    const NAME: &'static str = "Row";
}

/// A grid column, 0-based.
///
/// The axis derives the traits a type generic over it derives, such as the
/// structural edit of `ambre-trame`. It holds no value, so none of them runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {}

impl Axis for Col {
    const NAME: &'static str = "Col";
}

/// The cell at the crossing of a row and a column.
///
/// The derived order is row-major: rows first, then columns.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct CellPos {
    pub row: Pos<Row>,
    pub col: Pos<Col>,
}

impl CellPos {
    /// The cell at `row` and `col`.
    pub const fn new(row: Pos<Row>, col: Pos<Col>) -> Self {
        Self { row, col }
    }

    /// The top-left cell of the grid.
    pub const ORIGIN: Self = Self::new(Pos::<Row>::ZERO, Pos::<Col>::ZERO);
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
    pub const fn rows(self) -> Span<Row> {
        self.top_left.row.through(self.bottom_right.row)
    }

    /// The columns the rectangle covers.
    pub const fn cols(self) -> Span<Col> {
        self.top_left.col.through(self.bottom_right.col)
    }

    /// How many rows the rectangle covers.
    pub const fn row_count(self) -> Count<Row> {
        self.rows().len()
    }

    /// How many columns the rectangle covers.
    pub const fn col_count(self) -> Count<Col> {
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
    pub rows: Count<Row>,
    pub cols: Count<Col>,
}

impl MergeSpan {
    /// The extent of one cell alone.
    pub const CELL: Self = Self::new(Count::<Row>::ONE, Count::<Col>::ONE);

    /// The extent of `rows` rows by `cols` columns.
    pub const fn new(rows: Count<Row>, cols: Count<Col>) -> Self {
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
