//! The A1 notation of a cell and of a range, as a worksheet part spells them.
//!
//! Positions are 0-based on two axes of this crate. The text form is the
//! file's, 1-based, with the column in letters: `A1` is row 0, column 0.

use std::fmt;

use ambre_core::index::{Axis, Pos};
use serde::{Deserialize, Serialize};

/// A worksheet row, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {}

impl Axis for Row {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("XlsxRow");
}

/// A worksheet column, 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Col {}

impl Axis for Col {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("XlsxCol");
}

/// The last row a worksheet holds: row 1 048 576, the cap of the format.
pub const LAST_ROW: Pos<Row> = Pos::new(1_048_575);

/// The last column a worksheet holds: column `XFD`, the cap of the format.
pub const LAST_COL: Pos<Col> = Pos::new(16_383);

/// One cell of a sheet. The derived order is row-major, the order a worksheet
/// part writes its cells in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellRef {
    pub row: Pos<Row>,
    pub col: Pos<Col>,
}

impl CellRef {
    /// The cell at `row` and `col`, or `None` past the last row or column of
    /// the format.
    #[must_use]
    pub fn new(row: Pos<Row>, col: Pos<Col>) -> Option<Self> {
        (row <= LAST_ROW && col <= LAST_COL).then_some(Self { row, col })
    }

    /// The cell an A1 text names, such as `B3`. A `$` before either part is
    /// read and ignored: a position has no anchor.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('$').unwrap_or(text);
        let split = text.find(|c: char| !c.is_ascii_alphabetic())?;
        let (letters, rest) = text.split_at(split);
        let digits = rest.strip_prefix('$').unwrap_or(rest);
        Self::new(row_from_number(digits)?, col_from_letters(letters)?)
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_col_letters(f, self.col)?;
        write!(f, "{}", u64::from(self.row.get()) + 1)
    }
}

/// A rectangle of cells, both corners included. The first corner is the top
/// left one, whatever order the file wrote them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CellRange {
    first: CellRef,
    last: CellRef,
}

impl CellRange {
    /// The rectangle with `a` and `b` at two opposite corners.
    #[must_use]
    pub fn spanning(a: CellRef, b: CellRef) -> Self {
        Self {
            first: CellRef {
                row: a.row.min(b.row),
                col: a.col.min(b.col),
            },
            last: CellRef {
                row: a.row.max(b.row),
                col: a.col.max(b.col),
            },
        }
    }

    /// The range an A1 text names: `A1:C4`, or one cell `B2`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.split_once(':') {
            Some((a, b)) => Some(Self::spanning(CellRef::parse(a)?, CellRef::parse(b)?)),
            None => CellRef::parse(text).map(|cell| Self::spanning(cell, cell)),
        }
    }

    /// The top left cell.
    #[must_use]
    pub const fn first(self) -> CellRef {
        self.first
    }

    /// The bottom right cell.
    #[must_use]
    pub const fn last(self) -> CellRef {
        self.last
    }

    /// Whether the range covers one cell alone.
    #[must_use]
    pub fn is_single(self) -> bool {
        self.first == self.last
    }

    /// Whether `cell` lies inside the range.
    #[must_use]
    pub fn contains(self, cell: CellRef) -> bool {
        (self.first.row..=self.last.row).contains(&cell.row)
            && (self.first.col..=self.last.col).contains(&cell.col)
    }
}

impl fmt::Display for CellRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_single() {
            write!(f, "{}", self.first)
        } else {
            write!(f, "{}:{}", self.first, self.last)
        }
    }
}

/// The column the letters name: `A` is 0, `XFD` the last. Case is ignored, as
/// Excel ignores it in a formula.
#[must_use]
pub fn col_from_letters(letters: &str) -> Option<Pos<Col>> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let number = letters.bytes().try_fold(0u32, |acc, b| {
        b.is_ascii_alphabetic()
            .then(|| acc * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1)
    })?;
    let col = Pos::new(number.checked_sub(1)?);
    (col <= LAST_COL).then_some(col)
}

/// The row a 1-based decimal text names: `1` is row 0.
#[must_use]
pub fn row_from_number(digits: &str) -> Option<Pos<Row>> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    let row = Pos::new(number.checked_sub(1)?);
    (row <= LAST_ROW).then_some(row)
}

/// The letters of `col`, written to `out`.
///
/// # Errors
///
/// The error of `out`.
pub fn write_col_letters(out: &mut impl fmt::Write, col: Pos<Col>) -> fmt::Result {
    let mut letters = [0u8; 3];
    let mut n = col.get() + 1;
    let mut start = letters.len();
    while n > 0 && start > 0 {
        start -= 1;
        let rem = (n - 1) % 26;
        if let Some(slot) = letters.get_mut(start) {
            *slot = b'A' + u8::try_from(rem).unwrap_or(0);
        }
        n = (n - 1) / 26;
    }
    letters
        .get(start..)
        .unwrap_or_default()
        .iter()
        .try_for_each(|&b| out.write_char(char::from(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a1_names_the_first_cell_and_xfd_the_last_column() {
        assert_eq!(
            CellRef::parse("A1"),
            Some(CellRef {
                row: Pos::new(0),
                col: Pos::new(0)
            })
        );
        assert_eq!(col_from_letters("XFD"), Some(LAST_COL));
        assert_eq!(col_from_letters("XFE"), None);
        assert_eq!(CellRef::parse("A1048577"), None);
        assert_eq!(
            CellRef::parse("$B$3").map(|c| c.to_string()),
            Some("B3".to_owned())
        );
        assert_eq!(CellRef::parse("A0"), None);
        assert_eq!(CellRef::parse("1A"), None);
    }

    #[test]
    fn a_range_normalizes_its_corners() {
        let range = CellRange::parse("C4:A1");
        assert_eq!(range.map(|r| r.to_string()), Some("A1:C4".to_owned()));
        assert_eq!(
            CellRange::parse("B2").map(|r| r.to_string()),
            Some("B2".to_owned())
        );
    }

    proptest! {
        #[test]
        fn a_cell_reads_back_its_own_text(row in 0u32..=1_048_575, col in 0u32..=16_383) {
            let cell = CellRef { row: Pos::new(row), col: Pos::new(col) };
            prop_assert_eq!(CellRef::parse(&cell.to_string()), Some(cell));
        }
    }
}
