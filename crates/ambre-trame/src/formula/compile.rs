//! AST-to-bytecode compilation.
//!
//! Transforms an [`Expr`] AST into a flat postfix sequence of [`Op`]
//! instructions suitable for stack-VM execution. Extracts cell and range
//! dependencies in the same pass so the [`DependencyGraph`] can be updated
//! without a second traversal.

use super::types::{CellBytecode, CellRef, Expr, Op, RangeDep};

/// Compile an AST into a [`CellBytecode`] (ops + dependencies).
pub fn compile(ast: &Expr) -> CellBytecode {
    let mut ops = Vec::new();
    let mut cell_deps = Vec::new();
    let mut range_deps = Vec::new();
    compile_into(ast, &mut ops, &mut cell_deps, &mut range_deps);
    CellBytecode { ops, cell_deps, range_deps }
}

fn compile_into(
    ast: &Expr,
    ops: &mut Vec<Op>,
    cell_deps: &mut Vec<CellRef>,
    range_deps: &mut Vec<RangeDep>,
) {
    match ast {
        Expr::Literal(val) => {
            match val {
                super::types::CellValue::Number(n) => ops.push(Op::PushNum(*n)),
                super::types::CellValue::Text(s) => ops.push(Op::PushStr(s.clone())),
                super::types::CellValue::Boolean(b) => ops.push(Op::PushBool(*b)),
                super::types::CellValue::Empty => ops.push(Op::PushEmpty),
                super::types::CellValue::Range(_) => {
                    // Literal ranges cannot appear in source code; this branch
                    // is unreachable from the parser.
                    ops.push(Op::PushEmpty);
                }
                super::types::CellValue::Error(e) => {
                    // Error literals come from `resolve_refs` (e.g. a deleted
                    // cross-sheet target -> `#REF!`); push the error so it
                    // propagates through any surrounding operators.
                    ops.push(Op::PushError(e.clone()));
                }
            }
        }
        Expr::CellRef(r) => {
            cell_deps.push(*r);
            ops.push(Op::PushCell(*r));
        }
        Expr::Range(r) => {
            range_deps.push(r.clone());
            ops.push(Op::EvalRange(r.clone()));
        }
        Expr::BinOp { kind, lhs, rhs } => {
            compile_into(lhs, ops, cell_deps, range_deps);
            compile_into(rhs, ops, cell_deps, range_deps);
            ops.push(kind.to_op());
        }
        Expr::UnaryOp { kind, operand } => {
            compile_into(operand, ops, cell_deps, range_deps);
            match kind {
                super::types::UnaryOpKind::Neg => ops.push(Op::Neg),
                super::types::UnaryOpKind::Not => ops.push(Op::Not),
            }
        }
        Expr::FnCall { name, args } => {
            for arg in args {
                compile_into(arg, ops, cell_deps, range_deps);
            }
            ops.push(Op::CallFn { name: name.clone(), n_args: args.len() as u8 });
        }
        // `SheetRef`/`Name` are eliminated by `parser::resolve_refs` before
        // compilation. Reaching here means the caller skipped resolution; emit a
        // harmless empty so a misuse degrades to a blank value rather than a panic.
        Expr::SheetRef { .. } | Expr::Name(_) => {
            debug_assert!(false, "unresolved SheetRef/Name reached compile()");
            ops.push(Op::PushEmpty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parser::parse_formula;

    fn compile_str(input: &str) -> CellBytecode {
        let ast = parse_formula(input).unwrap();
        compile(&ast)
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
    fn cell_ref_extracts_dep() {
        let bc = compile_str("A1");
        assert!(matches!(&bc.ops[0], Op::PushCell(CellRef { row: 0, col: 0, .. })));
        assert_eq!(bc.cell_deps.len(), 1);
        assert_eq!(bc.cell_deps[0], CellRef::new(0, 0));
    }

    #[test]
    fn range_extracts_dep() {
        let bc = compile_str("A1:B10");
        assert!(matches!(&bc.ops[0], Op::EvalRange(_)));
        assert_eq!(bc.range_deps.len(), 1);
        assert_eq!(bc.range_deps[0], RangeDep::new(0, 9, 0, 1));
    }

    #[test]
    fn binary_op_postfix_order() {
        // "A1+B2" → push A1, push B2, Add
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
        assert!(matches!(&bc.ops[1], Op::CallFn { name, n_args: 1 } if name == "SUM"));
    }
}
