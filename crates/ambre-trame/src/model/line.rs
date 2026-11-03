//! One axis of a sheet, named by its axis marker.
//!
//! Much of the engine reads rows and columns the same way: find the line at a
//! position, find a line's position, count the lines. [`Line`] writes the part
//! that reads the sheet once per axis. The index family owns the arithmetic
//! of a position. Generic code takes a `Pos<L>` for `L: Line`, and never an
//! axis flag beside a bare integer.

use std::fmt::Debug;

use ambre_bus::{Col, Row};
use ambre_core::index::{Axis, Len, Pos};
use loro::LoroMovableList;

use super::{SheetIndex, Uid};

/// One axis of a sheet: [`Row`] or [`Col`].
///
/// The marker is `Copy` and `Debug`. A type generic over the axis, such as a
/// structural edit, then derives those traits with no bound of its own.
pub trait Line: Axis + Copy + Debug {
    /// The line at `pos`, or `None` past the used range.
    fn uid(index: &SheetIndex, pos: Pos<Self>) -> Option<Uid>;

    /// Where the line `uid` sits, or `None` once it is gone.
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Pos<Self>>;

    /// How many lines the axis holds.
    fn len(index: &SheetIndex) -> Len<Self>;
}

impl Line for Row {
    fn uid(index: &SheetIndex, pos: Pos<Row>) -> Option<Uid> {
        index.row_id(pos)
    }
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Pos<Row>> {
        index.row_pos(uid)
    }
    fn len(index: &SheetIndex) -> Len<Row> {
        index.row_len()
    }
}

impl Line for Col {
    fn uid(index: &SheetIndex, pos: Pos<Col>) -> Option<Uid> {
        index.col_id(pos)
    }
    fn pos(index: &SheetIndex, uid: &Uid) -> Option<Pos<Col>> {
        index.col_pos(uid)
    }
    fn len(index: &SheetIndex) -> Len<Col> {
        index.col_len()
    }
}

/// Insert the line `uid` at `at` of `list`, the list that orders an axis.
/// `None` when the list refuses it, or `at` lies past the indices the list
/// takes.
pub(crate) fn insert_line<A: Axis>(list: &LoroMovableList, at: Pos<A>, uid: &Uid) -> Option<()> {
    list.insert(at.to_index()?, uid.to_key_part().as_str()).ok()
}

/// Delete the line at `at` of `list`. `None` as for [`insert_line`].
pub(crate) fn delete_line<A: Axis>(list: &LoroMovableList, at: Pos<A>) -> Option<()> {
    list.delete(at.to_index()?, 1).ok()
}

/// Move the line at `from` of `list` to `to`. `None` as for [`insert_line`].
pub(crate) fn move_line<A: Axis>(list: &LoroMovableList, from: Pos<A>, to: Pos<A>) -> Option<()> {
    list.mov(from.to_index()?, to.to_index()?).ok()
}
