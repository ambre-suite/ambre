//! One axis of a sheet, named by its position type.
//!
//! Much of the engine reads rows and columns the same way: find the line at a
//! position, find a line's position, count the lines. [`Line`] writes that once
//! per axis, so generic code takes a [`Row`] or a [`Col`] and never an axis
//! flag beside a bare integer.

use std::fmt::Debug;
use std::hash::Hash;
use std::ops::Add;

use ambre_bus::{Col, ColCount, Row, RowCount};
use loro::LoroMovableList;

use super::{SheetIndex, Uid};

/// A position on one axis of a sheet: a [`Row`] or a [`Col`].
pub trait Line: Copy + Ord + Hash + Debug {
    /// A number of lines on this axis.
    type Count: Copy + Ord + Debug + Add<Output = Self::Count>;

    /// The first position of the axis.
    const FIRST: Self;

    /// One line.
    const ONE: Self::Count;

    /// Insert the line `uid` at this position of `list`, the list that orders
    /// the axis. `None` when the list refuses it, or the position lies past
    /// the indices the list takes.
    fn insert_into(self, list: &LoroMovableList, uid: &Uid) -> Option<()>;

    /// Delete the line at this position of `list`. `None` as for
    /// [`insert_into`](Self::insert_into).
    fn delete_from(self, list: &LoroMovableList) -> Option<()>;

    /// Move the line at this position of `list` to the position `to`. `None`
    /// as for [`insert_into`](Self::insert_into).
    fn move_in(self, list: &LoroMovableList, to: Self) -> Option<()>;

    /// The number of lines in a slice of `len` items, saturating.
    fn count_of(len: usize) -> Self::Count;

    /// The line at `pos`, or `None` past the used range.
    fn uid(index: &SheetIndex, pos: Self) -> Option<Uid>;

    /// Where the line `uid` sits, or `None` once it is gone.
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Self>;

    /// How many lines the axis holds.
    fn len(index: &SheetIndex) -> Self::Count;

    /// The first position past the used range.
    fn end(index: &SheetIndex) -> Self;

    /// The last line of the used range, or `None` when the axis is empty.
    fn last(index: &SheetIndex) -> Option<Self>;

    /// The position `n` lines further, saturating at the last position.
    fn forward(self, n: Self::Count) -> Self;

    /// The position `n` lines back, or `None` before the first one.
    fn back(self, n: Self::Count) -> Option<Self>;

    /// The position just before this one, or `None` at the first.
    fn before(self) -> Option<Self>;

    /// The lines from `start` up to this position, zero when `start` is not
    /// before it.
    fn count_from(self, start: Self) -> Self::Count;

    /// The position halfway from this one to `end`, rounded towards this one.
    fn midpoint(self, end: Self) -> Self;
}

impl Line for Row {
    type Count = RowCount;
    const FIRST: Self = Row::ZERO;
    const ONE: RowCount = RowCount::ONE;

    fn insert_into(self, list: &LoroMovableList, uid: &Uid) -> Option<()> {
        list.insert(self.to_index()?, uid.to_key_part().as_str())
            .ok()
    }
    fn delete_from(self, list: &LoroMovableList) -> Option<()> {
        list.delete(self.to_index()?, 1).ok()
    }
    fn move_in(self, list: &LoroMovableList, to: Self) -> Option<()> {
        list.mov(self.to_index()?, to.to_index()?).ok()
    }
    fn count_of(len: usize) -> RowCount {
        RowCount::from_len(len)
    }

    fn uid(index: &SheetIndex, pos: Self) -> Option<Uid> {
        index.row_id(pos)
    }
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Self> {
        index.row_pos(uid)
    }
    fn len(index: &SheetIndex) -> RowCount {
        index.row_len()
    }
    fn end(index: &SheetIndex) -> Self {
        index.row_len().as_end()
    }
    fn last(index: &SheetIndex) -> Option<Self> {
        index.row_len().as_end().prev()
    }
    fn forward(self, n: RowCount) -> Self {
        self + n
    }
    fn back(self, n: RowCount) -> Option<Self> {
        self.checked_sub(n)
    }
    fn before(self) -> Option<Self> {
        self.prev()
    }
    fn count_from(self, start: Self) -> RowCount {
        Row::count_from(self, start)
    }
    fn midpoint(self, end: Self) -> Self {
        self + RowCount::new(end.count_from(self).get() / 2)
    }
}

impl Line for Col {
    type Count = ColCount;
    const FIRST: Self = Col::ZERO;
    const ONE: ColCount = ColCount::ONE;

    fn insert_into(self, list: &LoroMovableList, uid: &Uid) -> Option<()> {
        list.insert(self.to_index()?, uid.to_key_part().as_str())
            .ok()
    }
    fn delete_from(self, list: &LoroMovableList) -> Option<()> {
        list.delete(self.to_index()?, 1).ok()
    }
    fn move_in(self, list: &LoroMovableList, to: Self) -> Option<()> {
        list.mov(self.to_index()?, to.to_index()?).ok()
    }
    fn count_of(len: usize) -> ColCount {
        ColCount::from_len(len)
    }

    fn uid(index: &SheetIndex, pos: Self) -> Option<Uid> {
        index.col_id(pos)
    }
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Self> {
        index.col_pos(uid)
    }
    fn len(index: &SheetIndex) -> ColCount {
        index.col_len()
    }
    fn end(index: &SheetIndex) -> Self {
        index.col_len().as_end()
    }
    fn last(index: &SheetIndex) -> Option<Self> {
        index.col_len().as_end().prev()
    }
    fn forward(self, n: ColCount) -> Self {
        self + n
    }
    fn back(self, n: ColCount) -> Option<Self> {
        self.checked_sub(n)
    }
    fn before(self) -> Option<Self> {
        self.prev()
    }
    fn count_from(self, start: Self) -> ColCount {
        Col::count_from(self, start)
    }
    fn midpoint(self, end: Self) -> Self {
        self + ColCount::new(end.count_from(self).get() / 2)
    }
}
