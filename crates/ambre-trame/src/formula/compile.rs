//! AST-to-bytecode compilation.
//!
//! Transforms an [`Expr`] AST into a flat postfix sequence of [`Op`]
//! instructions suitable for stack-VM execution. Extracts cell and range
//! dependencies in the same pass so the [`DependencyGraph`] can be updated
//! without a second traversal.
//!
//! Compilation is also where the positional AST references (`A1`, `A1:B10`) are
//! resolved. They resolve against the current per-sheet [`SheetIndex`], into
//! the uid-anchored engine forms ([`CellId`] / [`RangeId`]). A position inside
//! the used range binds to the row/column `Uid`, so it follows the data. A
//! position beyond it stays a [`AxisId::Virtual`] grid coordinate
//! (`trame-formula-engine`).

use ambre_core::index::Len;
use ambre_core::index::Pos;
use std::collections::HashMap;

use crate::model::{Line, SheetIndex};

use super::types::{
    AxisId, AxisSpan, CellBytecode, CellId, CellRef, EngineSheet, Expr, Op, RangeDep,
    RangeEndpoint, RangeId, SpanLines,
};

/// Compile an AST into a [`CellBytecode`] (ops + dependencies), resolving
/// positional references against `indices`. The indices are per sheet, keyed
/// by the engine's local sheet index. They bind positional references to
/// row and column `Uid`s.
pub fn compile(ast: &Expr, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) -> CellBytecode {
    let mut ops = Vec::new();
    let mut cell_deps = Vec::new();
    let mut range_deps = Vec::new();
    compile_into(ast, indices, &mut ops, &mut cell_deps, &mut range_deps);
    CellBytecode {
        ops,
        cell_deps,
        range_deps,
    }
}

fn resolve_cell(r: &CellRef, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) -> CellId {
    let idx = indices.get(&r.sheet);
    let row = bind(idx, r.row);
    let col = bind(idx, r.col);
    CellId {
        sheet: r.sheet,
        row,
        col,
    }
}

/// Compile a positional range into a uid-anchored [`RangeId`]. **Range endpoints
/// stay positional** (`AxisId::Virtual`), so a range denotes a rectangular
/// *region* (a block of positions), not a set of specific rows/columns. This is
/// the only well-defined choice under a sort. The two endpoints can otherwise
/// scatter and invert, collapsing the range. It also matches Excel/Sheets, where
/// sorting the rows inside `SUM(A1:A3)` leaves the block (and its total) intact.
/// Single-cell references, by contrast, bind to their row/column `Uid` and follow
/// the data (see [`resolve_cell`] and `trame-formula-engine`).
fn resolve_range(r: &RangeDep, _indices: &HashMap<Pos<EngineSheet>, SheetIndex>) -> RangeId {
    RangeId {
        sheet: r.sheet,
        row_start: at(r.row_start),
        row_end: at(r.row_end),
        col_start: at(r.col_start),
        col_end: at(r.col_end),
    }
}

/// Compile a range of whole lines (`TRAME-048`). Its own axis stays positional,
/// as a range does. The other axis is [`RangeEndpoint::EntireAxis`], which
/// reads the used part of the sheet when the range is evaluated.
fn resolve_span(s: &AxisSpan) -> RangeId {
    match s.lines {
        SpanLines::Rows { first, last } => RangeId {
            sheet: s.sheet,
            row_start: at(first),
            row_end: at(last),
            col_start: RangeEndpoint::EntireAxis,
            col_end: RangeEndpoint::EntireAxis,
        },
        SpanLines::Cols { first, last } => RangeId {
            sheet: s.sheet,
            row_start: RangeEndpoint::EntireAxis,
            row_end: RangeEndpoint::EntireAxis,
            col_start: at(first),
            col_end: at(last),
        },
    }
}

/// A range endpoint that stays at the position `pos`.
fn at<P>(pos: P) -> RangeEndpoint<P> {
    RangeEndpoint::Specific(AxisId::Virtual(pos))
}

/// Bind a positional axis coordinate: the materialized `Uid` if present (a
/// reference inside the used range, follows the data), else a virtual position.
fn bind<L: Line>(index: Option<&SheetIndex>, pos: Pos<L>) -> AxisId<Pos<L>> {
    match index.and_then(|i| L::uid(i, pos)) {
        Some(u) => AxisId::Bound(u),
        None => AxisId::Virtual(pos),
    }
}

fn compile_into(
    ast: &Expr,
    indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
    ops: &mut Vec<Op>,
    cell_deps: &mut Vec<CellId>,
    range_deps: &mut Vec<RangeId>,
) {
    match ast {
        Expr::Literal(val) => {
            match val {
                super::types::CellValue::Number(n) => ops.push(Op::PushNum(*n)),
                super::types::CellValue::Text(s) => {
                    ops.push(Op::PushStr(super::text::LiteralText::new(s.clone())))
                }
                super::types::CellValue::Boolean(b) => ops.push(Op::PushBool(*b)),
                super::types::CellValue::Empty => ops.push(Op::PushEmpty),
                super::types::CellValue::Range(_) => {
                    // Literal ranges cannot appear in source code; this branch
                    // is unreachable from the parser.
                    ops.push(Op::PushEmpty);
                }
                super::types::CellValue::Error(e) => {
                    // Error literals come from `resolve_refs` (for example a deleted
                    // cross-sheet target -> `#REF!`); push the error so it
                    // propagates through any surrounding operators.
                    ops.push(Op::PushError(e.clone()));
                }
            }
        }
        Expr::CellRef(r) => {
            let id = resolve_cell(r, indices);
            cell_deps.push(id);
            ops.push(Op::PushCell(id));
        }
        Expr::Range(r) => {
            let id = resolve_range(r, indices);
            range_deps.push(id.clone());
            ops.push(Op::EvalRange(id));
        }
        Expr::AxisSpan(s) => {
            let id = resolve_span(s);
            range_deps.push(id.clone());
            ops.push(Op::EvalRange(id));
        }
        Expr::BinOp { kind, lhs, rhs } => {
            compile_into(lhs, indices, ops, cell_deps, range_deps);
            compile_into(rhs, indices, ops, cell_deps, range_deps);
            ops.push(kind.to_op());
        }
        Expr::UnaryOp { kind, operand } => {
            compile_into(operand, indices, ops, cell_deps, range_deps);
            match kind {
                super::types::UnaryOpKind::Neg => ops.push(Op::Neg),
                super::types::UnaryOpKind::Not => ops.push(Op::Not),
            }
        }
        Expr::FnCall { name, args } => {
            for arg in args {
                compile_into(arg, indices, ops, cell_deps, range_deps);
            }
            ops.push(Op::CallFn {
                name: name.clone(),
                n_args: Len::<super::types::Arg>::of(args.len()),
            });
        }
        // `SheetRef`/`Name` are eliminated by `parser::resolve_refs` before
        // compilation, and the stored forms by `identity::unbind` before it.
        // Reaching here means the caller skipped a pass; emit a harmless empty
        // so a misuse degrades to a blank value rather than a panic.
        Expr::SheetRef { .. } | Expr::Name(_) | Expr::StoredCell(_) | Expr::StoredRange(..) => {
            debug_assert!(false, "unresolved reference reached compile()");
            ops.push(Op::PushEmpty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse_formula;
    use super::super::test_index::index_with;
    use super::super::types::ACTIVE_SHEET;
    use crate::Col;
    use ambre_core::index::Pos;

    const COL_25: Pos<Col> = Pos::<Col>::new(25);
    use super::*;

    fn compile_str(input: &str) -> CellBytecode {
        let ast = parse_formula(input).unwrap();
        let mut indices = HashMap::new();
        indices.insert(ACTIVE_SHEET, index_with(20, 10));
        compile(&ast, &indices)
    }

    #[test]
    fn literal_number() {
        let bc = compile_str("42");
        assert_eq!(bc.ops.len(), 1);
        assert!(matches!(&bc.ops[0], Op::PushNum(42.0)));
        assert!(bc.cell_deps.is_empty());
        assert!(bc.range_deps.is_empty());
    }

    #[test]
    fn cell_ref_extracts_bound_dep() {
        let bc = compile_str("A1");
        assert!(matches!(
            &bc.ops[0],
            Op::PushCell(CellId {
                row: AxisId::Bound(_),
                col: AxisId::Bound(_),
                ..
            })
        ));
        assert_eq!(bc.cell_deps.len(), 1);
    }

    #[test]
    fn range_extracts_dep() {
        let bc = compile_str("A1:B10");
        assert!(matches!(&bc.ops[0], Op::EvalRange(_)));
        assert_eq!(bc.range_deps.len(), 1);
    }

    #[test]
    fn binary_op_postfix_order() {
        let bc = compile_str("A1+B2");
        assert_eq!(bc.ops.len(), 3);
        assert!(matches!(&bc.ops[0], Op::PushCell(_)));
        assert!(matches!(&bc.ops[1], Op::PushCell(_)));
        assert!(matches!(&bc.ops[2], Op::Add));
    }

    #[test]
    fn function_call() {
        let bc = compile_str("SUM(A1:B10)");
        assert_eq!(bc.ops.len(), 2); // EvalRange + CallFn
        assert!(matches!(&bc.ops[0], Op::EvalRange(_)));
        assert!(
            matches!(&bc.ops[1], Op::CallFn { name, n_args } if name.as_str() == "SUM" && *n_args == Len::<super::super::types::Arg>::new(1))
        );
    }

    #[test]
    fn ref_beyond_used_range_is_virtual() {
        // Column Z (25) is beyond the 10 materialized columns -> virtual.
        let bc = compile_str("Z1");
        assert!(matches!(
            &bc.ops[0],
            Op::PushCell(CellId {
                col: AxisId::Virtual(COL_25),
                ..
            })
        ));
    }
}
