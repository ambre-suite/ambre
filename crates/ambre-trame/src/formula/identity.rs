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

use ambre_bus::{Col, Row};

use crate::model::{Line, SheetIndex};

use super::types::{
    AxisSpan, CellRef, CellValue, Expr, FormulaError, RangeDep, SheetQualifier, SpanLines,
    StoredAxis, StoredCell,
};

/// The structural edit a [`reanchor`] follows on one axis, in positions of the
/// old layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEdit<P: Line> {
    /// `n` lines inserted before position `at`.
    Insert { at: P, n: P::Count },
    /// The `n` lines from position `at` removed.
    Delete { at: P, n: P::Count },
    /// The lines permuted, with none added or removed: a sort or a move.
    Reorder,
}

/// A structural edit and the axis it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisEdit {
    Rows(LineEdit<Row>),
    Cols(LineEdit<Col>),
}

/// Rewrite every reference of `expr` through `f`, which receives the reference
/// node and its qualifier. A qualified reference that `f` turns into an error
/// drops its qualifier, so the text renders as `#REF!` rather than `#s1!#REF!`.
fn map_refs(expr: Expr, f: &impl Fn(Option<&SheetQualifier>, Expr) -> Expr) -> Expr {
    match expr {
        node @ (Expr::CellRef(_)
        | Expr::Range(_)
        | Expr::AxisSpan(_)
        | Expr::StoredCell(_)
        | Expr::StoredRange(..)) => f(None, node),
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
        Expr::AxisSpan(s) => Some(match s.lines {
            SpanLines::Cols { first, last } => {
                let corner = |col| StoredCell {
                    row: StoredAxis::Whole,
                    col: StoredAxis::At(col),
                };
                (corner(first), corner(last))
            }
            SpanLines::Rows { first, last } => {
                let corner = |row| StoredCell {
                    row: StoredAxis::At(row),
                    col: StoredAxis::Whole,
                };
                (corner(first), corner(last))
            }
        }),
        Expr::StoredCell(c) => Some((*c, *c)),
        Expr::StoredRange(a, b) => Some((*a, *b)),
        _ => None,
    }
}

/// The range of whole lines two corners name, when neither names a line.
fn span_of(start: StoredCell, end: StoredCell) -> Option<AxisSpan> {
    use StoredAxis::{At, Whole};
    match (start.row, start.col, end.row, end.col) {
        (Whole, At(a), Whole, At(b)) => Some(AxisSpan::cols(a, b)),
        (At(a), Whole, At(b), Whole) => Some(AxisSpan::rows(a, b)),
        _ => None,
    }
}

/// The node for a cell or a range in stored form, plain `A1` or `A:C` when no
/// axis names a line.
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
        _ => span_of(start, end).map_or(Expr::StoredRange(start, end), Expr::AxisSpan),
    }
}

fn is_single(node: &Expr) -> bool {
    matches!(node, Expr::CellRef(_) | Expr::StoredCell(_))
}

fn broken() -> Expr {
    Expr::Literal(CellValue::Error(FormulaError::BrokenRef))
}

/// One axis of a stored cell, read and written by its position type.
trait CellAxis: Line {
    fn get(cell: &StoredCell) -> StoredAxis<Self>;
    fn set(cell: &mut StoredCell, value: StoredAxis<Self>);
}

impl CellAxis for Row {
    fn get(cell: &StoredCell) -> StoredAxis<Self> {
        cell.row
    }
    fn set(cell: &mut StoredCell, value: StoredAxis<Self>) {
        cell.row = value;
    }
}

impl CellAxis for Col {
    fn get(cell: &StoredCell) -> StoredAxis<Self> {
        cell.col
    }
    fn set(cell: &mut StoredCell, value: StoredAxis<Self>) {
        cell.col = value;
    }
}

/// The line at `pos`, or the position itself past the used range.
fn bind_at<P: Line>(index: &SheetIndex, pos: P) -> StoredAxis<P> {
    P::uid(index, pos).map_or(StoredAxis::At(pos), StoredAxis::Line)
}

/// The position of a stored axis, `None` for a line that is gone. The whole
/// axis has no single position, so it is `None` too.
fn position<P: Line>(index: &SheetIndex, axis: StoredAxis<P>) -> Option<P> {
    match axis {
        StoredAxis::Line(uid) => P::pos(index, &uid),
        StoredAxis::At(pos) => Some(pos),
        StoredAxis::Whole => None,
    }
}

/// A stored axis in today's layout: a line becomes its position, and `None`
/// once it is gone. A position and the whole axis stay as they are.
fn place<P: Line>(index: &SheetIndex, axis: StoredAxis<P>) -> Option<StoredAxis<P>> {
    match axis {
        StoredAxis::Line(uid) => P::pos(index, &uid).map(StoredAxis::At),
        other => Some(other),
    }
}

/// Name each position of one axis of both corners by the line now at it.
fn bind_axis<P: CellAxis>(index: &SheetIndex, start: &mut StoredCell, end: &mut StoredCell) {
    for cell in [start, end] {
        if let StoredAxis::At(pos) = P::get(cell) {
            P::set(cell, bind_at(index, pos));
        }
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
        bind_axis::<Row>(index, &mut start, &mut end);
        bind_axis::<Col>(index, &mut start, &mut end);
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
        let at = |c: StoredCell| {
            Some(StoredCell {
                row: place(index, c.row)?,
                col: place(index, c.col)?,
            })
        };
        let (Some(a), Some(b)) = (at(start), at(end)) else {
            return broken();
        };
        stored_node(a, b, is_single(&node))
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
    old: &SheetIndex,
    new: &SheetIndex,
    edit: AxisEdit,
) -> Expr {
    match edit {
        AxisEdit::Rows(edit) => reanchor_on(expr, edited, old, new, edit),
        AxisEdit::Cols(edit) => reanchor_on(expr, edited, old, new, edit),
    }
}

/// The side a removed position moves to, to its nearest survivor.
#[derive(Clone, Copy)]
enum Survivor {
    /// The first survivor past the removed lines.
    After,
    /// The last survivor before the removed lines.
    Before,
}

/// Where a position of the old layout sits in the new one. A removed one moves
/// to the nearest survivor on the side `side` names.
fn moved<P: Line>(edit: LineEdit<P>, pos: P, side: Survivor) -> Option<P> {
    match edit {
        LineEdit::Reorder => Some(pos),
        LineEdit::Insert { at, n } => Some(if pos >= at { pos.forward(n) } else { pos }),
        LineEdit::Delete { at, n } if pos >= at.forward(n) => pos.back(n),
        LineEdit::Delete { at, .. } if pos >= at => match side {
            Survivor::After => Some(at),
            Survivor::Before => at.before(),
        },
        LineEdit::Delete { .. } => Some(pos),
    }
}

/// [`reanchor`] on the axis whose positions are `P`.
fn reanchor_on<P: CellAxis>(
    expr: Expr,
    edited: &impl Fn(Option<&SheetQualifier>) -> bool,
    old: &SheetIndex,
    new: &SheetIndex,
    edit: LineEdit<P>,
) -> Expr {
    map_refs(expr, &|q, node| {
        if !edited(q) {
            return node;
        }
        let Some((mut start, mut end)) = corners(&node) else {
            return node;
        };
        if is_single(&node) {
            let next = match P::get(&start) {
                // A line follows itself; one that is gone already reads `#REF!`.
                StoredAxis::Line(_) | StoredAxis::Whole => return node,
                StoredAxis::At(pos) if pos < P::end(old) => bind_at(old, pos),
                StoredAxis::At(pos) => match moved(edit, pos, Survivor::After) {
                    Some(to) => StoredAxis::At(to),
                    None => return broken(),
                },
            };
            P::set(&mut start, next);
            return stored_node(start, end, true);
        }
        // A range of whole lines on the edited axis keeps covering them all.
        if P::get(&start) == StoredAxis::Whole {
            return node;
        }
        let (Some(a), Some(b)) = (position(old, P::get(&start)), position(old, P::get(&end)))
        else {
            // A corner whose line is already gone: it reads `#REF!` as it is.
            return node;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let (Some(lo), Some(hi)) = (
            moved(edit, lo, Survivor::After),
            moved(edit, hi, Survivor::Before),
        ) else {
            return broken();
        };
        if lo > hi {
            return broken();
        }
        let encode = |pos: P| {
            if pos < P::end(new) {
                bind_at(new, pos)
            } else {
                StoredAxis::At(pos)
            }
        };
        P::set(&mut start, encode(lo));
        P::set(&mut end, encode(hi));
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
    fn whole_lines_store_their_lines_and_read_back_as_typed() {
        let index = index_with(5, 3);
        let text = stored("SUM(A:B)+SUM(2:3)", &index);
        assert!(text.starts_with("SUM({*|"), "{text}");
        assert!(text.contains("|*}"), "{text}");
        assert_eq!(render(&parse_formula(&text).unwrap()), text);
        assert_eq!(shown(&text, &index), "SUM(A:B)+SUM(2:3)");
        // Past the used range a whole line stays a position.
        assert_eq!(stored("SUM(Z:Z)", &index), "SUM(Z:Z)");
    }

    #[test]
    fn a_stored_reference_reparses_to_itself() {
        let index = index_with(5, 3);
        let text = stored("SUM(A1:B3)*C2", &index);
        assert_eq!(render(&parse_formula(&text).unwrap()), text);
    }
}
