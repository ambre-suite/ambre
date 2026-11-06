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
use ambre_core::index::Len;
use ambre_core::index::Pos;

use crate::model::{Line, SheetIndex};

use super::types::{
    AxisSpan, CellRef, CellValue, Expr, FormulaError, RangeDep, SheetQualifier, SpanLines,
    StoredAxis, StoredCell,
};

/// The structural edit a [`reanchor`] follows on one axis, in positions of the
/// old layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEdit<L: Line> {
    /// `n` lines inserted before position `at`.
    Insert { at: Pos<L>, n: Len<L> },
    /// The `n` lines from position `at` removed.
    Delete { at: Pos<L>, n: Len<L> },
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
fn stored_node(start: StoredCell, end: StoredCell, shape: NodeShape) -> Expr {
    if shape == NodeShape::Single {
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

/// Whether a reference node names one cell or a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeShape {
    Single,
    Range,
}

fn shape_of(node: &Expr) -> NodeShape {
    if matches!(node, Expr::CellRef(_) | Expr::StoredCell(_)) {
        NodeShape::Single
    } else {
        NodeShape::Range
    }
}

fn broken() -> Expr {
    Expr::Literal(CellValue::Error(FormulaError::BrokenRef))
}

/// One axis of a stored cell, read and written by its position type.
trait CellAxis: Line {
    /// The axis a structural edit of this one leaves in place.
    type Other: CellAxis;
    fn get(cell: &StoredCell) -> StoredAxis<Pos<Self>>;
    fn set(cell: &mut StoredCell, value: StoredAxis<Pos<Self>>);
}

impl CellAxis for Row {
    type Other = Col;
    fn get(cell: &StoredCell) -> StoredAxis<Pos<Self>> {
        cell.row
    }
    fn set(cell: &mut StoredCell, value: StoredAxis<Pos<Self>>) {
        cell.row = value;
    }
}

impl CellAxis for Col {
    type Other = Row;
    fn get(cell: &StoredCell) -> StoredAxis<Pos<Self>> {
        cell.col
    }
    fn set(cell: &mut StoredCell, value: StoredAxis<Pos<Self>>) {
        cell.col = value;
    }
}

/// The visible element of the line at `pos`, or the position itself past the
/// used range.
fn bind_at<L: Line>(index: &SheetIndex, pos: Pos<L>) -> StoredAxis<Pos<L>> {
    L::element(index, pos).map_or(StoredAxis::At(pos), StoredAxis::Line)
}

/// The position of a stored axis, `None` for a line that is gone. The whole
/// axis has no single position, so it is `None` too.
fn position<L: Line>(index: &SheetIndex, axis: StoredAxis<Pos<L>>) -> Option<Pos<L>> {
    match axis {
        StoredAxis::Line(element) => L::pos_of_element(index, &element),
        StoredAxis::At(pos) => Some(pos),
        StoredAxis::Whole => None,
    }
}

/// A stored axis in today's layout: a line becomes its position, and `None`
/// once it is gone. A position and the whole axis stay as they are.
fn place<L: Line>(index: &SheetIndex, axis: StoredAxis<Pos<L>>) -> Option<StoredAxis<Pos<L>>> {
    match axis {
        StoredAxis::Line(element) => L::pos_of_element(index, &element).map(StoredAxis::At),
        other => Some(other),
    }
}

/// Whether axis `L` of `cell` names an element `index` no longer holds.
fn gone<L: CellAxis>(index: &SheetIndex, cell: &StoredCell) -> bool {
    match L::get(cell) {
        StoredAxis::Line(element) => L::pos_of_element(index, &element).is_none(),
        StoredAxis::At(_) | StoredAxis::Whole => false,
    }
}

/// Name each position of one axis of both corners by the line now at it.
fn bind_axis<L: CellAxis>(index: &SheetIndex, start: &mut StoredCell, end: &mut StoredCell) {
    for cell in [start, end] {
        if let StoredAxis::At(pos) = L::get(cell) {
            L::set(cell, bind_at(index, pos));
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
        stored_node(start, end, shape_of(&node))
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
        stored_node(a, b, shape_of(&node))
    })
}

/// Whether a structural edit can change `expr`, its compiled form or its value
/// only through the cells it reads.
///
/// That holds when each reference of `expr` names a cell by two lines that
/// `index_of` still holds, and `expr` calls neither `ROW` nor `COLUMN`. Such a
/// formula follows its cells. [`reanchor`] rewrites it only when one of those
/// lines goes, and its value moves only when a cell it reads does. Anything
/// else reads positions. That covers a range, a whole-line span, a position
/// past the used range, a defined name, its own position and a line already
/// gone. A structural edit re-reads each of those (`TRAME-280`).
pub fn follows_its_lines<'a>(
    expr: &Expr,
    index_of: &impl Fn(Option<&SheetQualifier>) -> Option<&'a SheetIndex>,
) -> bool {
    follows_on(expr, None, index_of)
}

/// [`follows_its_lines`] under the qualifier `q` of the enclosing reference.
fn follows_on<'a>(
    expr: &Expr,
    q: Option<&SheetQualifier>,
    index_of: &impl Fn(Option<&SheetQualifier>) -> Option<&'a SheetIndex>,
) -> bool {
    match expr {
        Expr::Literal(_) => true,
        Expr::StoredCell(cell) => match (cell.row, cell.col, index_of(q)) {
            (StoredAxis::Line(row), StoredAxis::Line(col), Some(index)) => {
                Row::pos_of_element(index, &row).is_some()
                    && Col::pos_of_element(index, &col).is_some()
            }
            _ => false,
        },
        Expr::SheetRef { sheet, inner } => follows_on(inner, Some(sheet), index_of),
        Expr::BinOp { lhs, rhs, .. } => {
            follows_on(lhs, q, index_of) && follows_on(rhs, q, index_of)
        }
        Expr::UnaryOp { operand, .. } => follows_on(operand, q, index_of),
        Expr::FnCall { name, args } => {
            !matches!(name.as_str(), "ROW" | "COLUMN")
                && args.iter().all(|a| follows_on(a, q, index_of))
        }
        Expr::CellRef(_)
        | Expr::Range(_)
        | Expr::StoredRange(..)
        | Expr::AxisSpan(_)
        | Expr::Name(_) => false,
    }
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
///
/// A reference to a line the new layout lacks is frozen into a literal
/// `#REF!`. No element of a deleted line then stays in the text (`TRAME-277`).
///
/// Returns `None` when no reference moved, so the walk renders and writes
/// nothing for the formula (`TRAME-277`).
pub fn reanchor(
    expr: Expr,
    edited: &impl Fn(Option<&SheetQualifier>) -> bool,
    old: &SheetIndex,
    new: &SheetIndex,
    edit: AxisEdit,
) -> Option<Expr> {
    let moved = std::cell::Cell::new(Moved::No);
    let after = match edit {
        AxisEdit::Rows(edit) => reanchor_on(expr, edited, old, new, edit, &moved),
        AxisEdit::Cols(edit) => reanchor_on(expr, edited, old, new, edit, &moved),
    };
    (moved.get() == Moved::Yes).then_some(after)
}

/// Whether a walk changed a reference of the formula it read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Moved {
    No,
    Yes,
}

/// Whether `before` and `after` name the same reference, by kind and corners.
fn same_reference(before: &Expr, after: &Expr) -> bool {
    std::mem::discriminant(before) == std::mem::discriminant(after)
        && corners(before) == corners(after)
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
fn moved<L: Line>(edit: LineEdit<L>, pos: Pos<L>, side: Survivor) -> Option<Pos<L>> {
    match edit {
        LineEdit::Reorder => Some(pos),
        LineEdit::Insert { at, n } => Some(if pos >= at { pos + n } else { pos }),
        LineEdit::Delete { at, n } if pos >= at + n => pos.checked_sub(n),
        LineEdit::Delete { at, .. } if pos >= at => match side {
            Survivor::After => Some(at),
            Survivor::Before => at.prev(),
        },
        LineEdit::Delete { .. } => Some(pos),
    }
}

/// [`reanchor`] on the axis whose positions are `P`.
fn reanchor_on<L: CellAxis>(
    expr: Expr,
    edited: &impl Fn(Option<&SheetQualifier>) -> bool,
    old: &SheetIndex,
    new: &SheetIndex,
    edit: LineEdit<L>,
    moved: &std::cell::Cell<Moved>,
) -> Expr {
    map_refs(expr, &|q, node| {
        let before = node.clone();
        let after = reanchor_node(q, node, edited, old, new, edit);
        if !same_reference(&before, &after) {
            moved.set(Moved::Yes);
        }
        after
    })
}

/// [`reanchor_on`] for one reference node.
fn reanchor_node<L: CellAxis>(
    q: Option<&SheetQualifier>,
    node: Expr,
    edited: &impl Fn(Option<&SheetQualifier>) -> bool,
    old: &SheetIndex,
    new: &SheetIndex,
    edit: LineEdit<L>,
) -> Expr {
    {
        if !edited(q) {
            return node;
        }
        let Some((mut start, mut end)) = corners(&node) else {
            return node;
        };
        // A reference to a line the new layout lacks freezes into a literal
        // `#REF!`: the deleted line, or one a concurrent delete removed. A
        // formula then holds no element of a deleted line (`TRAME-277`).
        if [start, end].iter().any(|c| gone::<L::Other>(new, c)) {
            return broken();
        }
        if shape_of(&node) == NodeShape::Single {
            if gone::<L>(new, &start) {
                return broken();
            }
            let next = match L::get(&start) {
                // A line follows itself.
                StoredAxis::Line(_) | StoredAxis::Whole => return node,
                StoredAxis::At(pos) if pos < L::len(old).as_end() => bind_at(old, pos),
                StoredAxis::At(pos) => match moved(edit, pos, Survivor::After) {
                    Some(to) => StoredAxis::At(to),
                    None => return broken(),
                },
            };
            L::set(&mut start, next);
            return stored_node(start, end, NodeShape::Single);
        }
        // A range of whole lines on the edited axis keeps covering them all.
        if L::get(&start) == StoredAxis::Whole {
            return node;
        }
        let (Some(a), Some(b)) = (position(old, L::get(&start)), position(old, L::get(&end)))
        else {
            // A corner whose line was gone before this edit freezes too.
            return broken();
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
        let encode = |pos: Pos<L>| {
            if pos < L::len(new).as_end() {
                bind_at(new, pos)
            } else {
                StoredAxis::At(pos)
            }
        };
        L::set(&mut start, encode(lo));
        L::set(&mut end, encode(hi));
        stored_node(start, end, NodeShape::Range)
    }
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
