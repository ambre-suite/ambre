//! The stored form of a reference (`TRAME-228`).
//!
//! A user types positions: `=A2`. The document keeps the lines at those
//! positions when the formula was written: `={row_uid|col_uid}`. So an insert,
//! a delete or a peer's concurrent edit cannot move the reference off its cell.
//! Three rewrites of the AST carry a formula between the two forms:
//!
//! - [`bind`] turns each position into the line now at it, before a write;
//! - [`unbind`] turns each line back into its position, for the editor and the
//!   engine. A line that no longer exists reads `#REF!` (`TRAME-008`);
//! - [`reanchor`] follows a structural edit: a range keeps its rectangle of
//!   positions through a sort, widens on an insert and shrinks on a delete.
//!
//! A position past the used range names no line, and stays a position
//! (`TRAME-006`). Each rewrite takes the host's answer for "which sheet index
//! does this reference read": `None` for the formula's own sheet, else its
//! qualifier. A sheet the host cannot index is left untouched.

use crate::model::{SheetIndex, Uid};

use super::types::{
    CellRef, CellValue, Expr, FormulaError, RangeDep, SheetQualifier, StoredAxis, StoredCell,
};

/// Which axis a structural edit moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineAxis {
    Row,
    Col,
}

/// The structural edit a [`reanchor`] follows, in positions of the old layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEdit {
    /// `n` lines inserted before position `at`.
    Insert { at: u32, n: u32 },
    /// The `n` lines from position `at` removed.
    Delete { at: u32, n: u32 },
    /// The lines permuted, with none added or removed: a sort or a move.
    Reorder,
}

/// Rewrite every reference of `expr` through `f`, which receives the reference
/// node and its qualifier. A qualified reference that `f` turns into an error
/// drops its qualifier, so the text renders as `#REF!` rather than `#s1!#REF!`.
fn map_refs(expr: Expr, f: &impl Fn(Option<&SheetQualifier>, Expr) -> Expr) -> Expr {
    match expr {
        node
        @ (Expr::CellRef(_) | Expr::Range(_) | Expr::StoredCell(_) | Expr::StoredRange(..)) => {
            f(None, node)
        }
        Expr::SheetRef { sheet, inner } => match f(Some(&sheet), *inner) {
            lit @ Expr::Literal(_) => lit,
            inner => Expr::SheetRef {
                sheet,
                inner: Box::new(inner),
            },
        },
        Expr::BinOp { kind, lhs, rhs } => Expr::BinOp {
            kind,
            lhs: Box::new(map_refs(*lhs, f)),
            rhs: Box::new(map_refs(*rhs, f)),
        },
        Expr::UnaryOp { kind, operand } => Expr::UnaryOp {
            kind,
            operand: Box::new(map_refs(*operand, f)),
        },
        Expr::FnCall { name, args } => Expr::FnCall {
            name,
            args: args.into_iter().map(|a| map_refs(a, f)).collect(),
        },
        leaf => leaf,
    }
}

/// The two corners of a reference node, or `None` for a node that is not one.
fn corners(node: &Expr) -> Option<(StoredCell, StoredCell)> {
    match node {
        Expr::CellRef(r) => Some((StoredCell::at(*r), StoredCell::at(*r))),
        Expr::Range(r) => Some((
            StoredCell::at(CellRef::new(r.row_start, r.col_start)),
            StoredCell::at(CellRef::new(r.row_end, r.col_end)),
        )),
        Expr::StoredCell(c) => Some((*c, *c)),
        Expr::StoredRange(a, b) => Some((*a, *b)),
        _ => None,
    }
}

/// The node for a cell or a range in stored form, plain `A1` when no axis
/// names a line.
fn stored_node(start: StoredCell, end: StoredCell, single: bool) -> Expr {
    if single {
        return match start.as_position() {
            Some(at) => Expr::CellRef(at),
            None => Expr::StoredCell(start),
        };
    }
    match (start.as_position(), end.as_position()) {
        (Some(a), Some(b)) => Expr::Range(RangeDep::new(
            a.row.min(b.row),
            a.row.max(b.row),
            a.col.min(b.col),
            a.col.max(b.col),
        )),
        _ => Expr::StoredRange(start, end),
    }
}

fn is_single(node: &Expr) -> bool {
    matches!(node, Expr::CellRef(_) | Expr::StoredCell(_))
}

fn broken() -> Expr {
    Expr::Literal(CellValue::Error(FormulaError::BrokenRef))
}

/// One axis of a sheet's layout, read through a [`SheetIndex`].
#[derive(Clone, Copy)]
struct Lines<'a> {
    index: &'a SheetIndex,
    axis: LineAxis,
}

impl Lines<'_> {
    fn uid(&self, pos: u32) -> Option<Uid> {
        match self.axis {
            LineAxis::Row => self.index.row_id(pos as usize),
            LineAxis::Col => self.index.col_id(pos as usize),
        }
    }

    fn pos(&self, uid: &Uid) -> Option<u32> {
        match self.axis {
            LineAxis::Row => self.index.row_pos(uid),
            LineAxis::Col => self.index.col_pos(uid),
        }
        .map(|p| p as u32)
    }

    fn len(&self) -> u32 {
        (match self.axis {
            LineAxis::Row => self.index.row_len(),
            LineAxis::Col => self.index.col_len(),
        }) as u32
    }

    /// The line at `pos`, or the position itself past the used range.
    fn bind(&self, pos: u32) -> StoredAxis {
        self.uid(pos).map_or(StoredAxis::At(pos), StoredAxis::Line)
    }

    /// The position of a stored axis, `None` for a line that is gone.
    fn position(&self, axis: StoredAxis) -> Option<u32> {
        match axis {
            StoredAxis::Line(uid) => self.pos(&uid),
            StoredAxis::At(pos) => Some(pos),
        }
    }
}

fn get(cell: &StoredCell, axis: LineAxis) -> StoredAxis {
    match axis {
        LineAxis::Row => cell.row,
        LineAxis::Col => cell.col,
    }
}

fn set(cell: &mut StoredCell, axis: LineAxis, value: StoredAxis) {
    match axis {
        LineAxis::Row => cell.row = value,
        LineAxis::Col => cell.col = value,
    }
}

/// Name each position of `expr` by the line now at it (`TRAME-228`). A position
/// past the used range stays a position.
pub fn bind<'a>(
    expr: Expr,
    index_of: &impl Fn(Option<&SheetQualifier>) -> Option<&'a SheetIndex>,
) -> Expr {
    map_refs(expr, &|q, node| {
        let (Some(index), Some((mut start, mut end))) = (index_of(q), corners(&node)) else {
            return node;
        };
        for axis in [LineAxis::Row, LineAxis::Col] {
            let lines = Lines { index, axis };
            for cell in [&mut start, &mut end] {
                if let StoredAxis::At(pos) = get(cell, axis) {
                    set(cell, axis, lines.bind(pos));
                }
            }
        }
        stored_node(start, end, is_single(&node))
    })
}

/// Turn each line of `expr` back into its position. A reference naming a line
/// that is gone reads `#REF!` (`TRAME-008`).
pub fn unbind<'a>(
    expr: Expr,
    index_of: &impl Fn(Option<&SheetQualifier>) -> Option<&'a SheetIndex>,
) -> Expr {
    map_refs(expr, &|q, node| {
        if !matches!(node, Expr::StoredCell(_) | Expr::StoredRange(..)) {
            return node;
        }
        let (Some(index), Some((start, end))) = (index_of(q), corners(&node)) else {
            return node;
        };
        let rows = Lines {
            index,
            axis: LineAxis::Row,
        };
        let cols = Lines {
            index,
            axis: LineAxis::Col,
        };
        let at = |c: StoredCell| Some(CellRef::new(rows.position(c.row)?, cols.position(c.col)?));
        let (Some(a), Some(b)) = (at(start), at(end)) else {
            return broken();
        };
        if is_single(&node) {
            Expr::CellRef(a)
        } else {
            Expr::Range(RangeDep::new(
                a.row.min(b.row),
                a.row.max(b.row),
                a.col.min(b.col),
                a.col.max(b.col),
            ))
        }
    })
}

/// Carry the references of `expr` that read the edited sheet across one
/// structural edit, from the `old` layout to the `new` one.
///
/// A cell keeps naming its line, so it follows that line. It reads `#REF!` once
/// the line is gone. A position inside the old used range is bound to its line
/// first. A position past the used range moves as the lines before it do.
///
/// A range is a rectangle of positions. Through a reorder each corner is
/// re-anchored to the line now at its position (`TRAME-227`). Through an insert
/// each corner keeps its line, so lines inserted inside it widen it
/// (`TRAME-171`). Through a delete a removed corner moves to the nearest line
/// left inside it, and a range with none left reads `#REF!`.
pub fn reanchor(
    expr: Expr,
    edited: &impl Fn(Option<&SheetQualifier>) -> bool,
    axis: LineAxis,
    old: &SheetIndex,
    new: &SheetIndex,
    edit: LineEdit,
) -> Expr {
    let old = Lines { index: old, axis };
    let new = Lines { index: new, axis };
    // Where a position of the old layout sits in the new one. A removed one
    // moves to the nearest survivor on the side `towards_end` names.
    let moved = |pos: u32, towards_end: bool| -> Option<u32> {
        match edit {
            LineEdit::Reorder => Some(pos),
            LineEdit::Insert { at, n } => Some(if pos >= at { pos + n } else { pos }),
            LineEdit::Delete { at, n } if pos >= at + n => Some(pos - n),
            LineEdit::Delete { at, .. } if pos >= at => {
                if towards_end {
                    Some(at)
                } else {
                    at.checked_sub(1)
                }
            }
            LineEdit::Delete { .. } => Some(pos),
        }
    };
    map_refs(expr, &|q, node| {
        if !edited(q) {
            return node;
        }
        let Some((mut start, mut end)) = corners(&node) else {
            return node;
        };
        if is_single(&node) {
            let next = match get(&start, axis) {
                // A line follows itself; one that is gone already reads `#REF!`.
                StoredAxis::Line(_) => return node,
                StoredAxis::At(pos) if pos < old.len() => old.bind(pos),
                StoredAxis::At(pos) => match moved(pos, true) {
                    Some(to) => StoredAxis::At(to),
                    None => return broken(),
                },
            };
            set(&mut start, axis, next);
            return stored_node(start, end, true);
        }
        let (Some(a), Some(b)) = (
            old.position(get(&start, axis)),
            old.position(get(&end, axis)),
        ) else {
            // A corner whose line is already gone: it reads `#REF!` as it is.
            return node;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let (Some(lo), Some(hi)) = (moved(lo, true), moved(hi, false)) else {
            return broken();
        };
        if lo > hi {
            return broken();
        }
        let encode = |pos: u32| {
            if pos < new.len() {
                new.bind(pos)
            } else {
                StoredAxis::At(pos)
            }
        };
        set(&mut start, axis, encode(lo));
        set(&mut end, axis, encode(hi));
        stored_node(start, end, false)
    })
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse_formula;
    use super::super::render::render;
    use super::super::test_index::index_with;
    use super::*;

    fn own<'a>(
        index: &'a SheetIndex,
    ) -> impl Fn(Option<&SheetQualifier>) -> Option<&'a SheetIndex> {
        move |q| q.is_none().then_some(index)
    }

    fn stored(text: &str, index: &SheetIndex) -> String {
        render(&bind(parse_formula(text).unwrap(), &own(index)))
    }

    fn shown(text: &str, index: &SheetIndex) -> String {
        render(&unbind(parse_formula(text).unwrap(), &own(index)))
    }

    #[test]
    fn a_stored_formula_names_lines_and_reads_back_as_typed() {
        let index = index_with(5, 3);
        let text = stored("SUM(A1:B3)+C2", &index);
        assert!(!text.contains("A1"), "no position is stored: {text}");
        assert!(text.contains('{'), "each reference names its lines: {text}");
        assert_eq!(shown(&text, &index), "SUM(A1:B3)+C2");
    }

    #[test]
    fn a_position_past_the_used_range_stays_a_position() {
        let index = index_with(5, 3);
        assert_eq!(stored("Z99", &index), "Z99");
        // The far corner keeps its row as a position and names its column.
        let text = stored("A1:A99", &index);
        assert!(text.contains(":{99|"), "{text}");
        assert_eq!(shown(&text, &index), "A1:A99");
    }

    #[test]
    fn a_stored_reference_reparses_to_itself() {
        let index = index_with(5, 3);
        let text = stored("SUM(A1:B3)*C2", &index);
        assert_eq!(render(&parse_formula(&text).unwrap()), text);
    }
}
