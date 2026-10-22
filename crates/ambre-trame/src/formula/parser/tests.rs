// ambre-trame/src/formula/parser/tests.rs
// Parser unit tests (descendant module: sees the private combinators).

use super::*;

fn parse(input: &str) -> Expr {
    parse_formula(input).unwrap_or_else(|e| panic!("parse failed: {e}"))
}

#[test]
fn literal_number() {
    assert!(matches!(
        parse("42"),
        Expr::Literal(CellValue::Number(42.0))
    ));
    assert!(matches!(parse("-5"), Expr::UnaryOp { .. }));
    assert!(
        matches!(parse("1.23"), Expr::Literal(CellValue::Number(v)) if (v - 1.23).abs() < 0.001)
    );
}

#[test]
fn literal_string() {
    assert!(matches!(parse(r#""hello""#), Expr::Literal(CellValue::Text(s)) if s == "hello"));
}

#[test]
fn literal_bool() {
    assert!(matches!(
        parse("TRUE"),
        Expr::Literal(CellValue::Boolean(true))
    ));
    assert!(matches!(
        parse("FALSE"),
        Expr::Literal(CellValue::Boolean(false))
    ));
}

#[test]
fn cell_reference() {
    assert!(matches!(
        parse("A1"),
        Expr::CellRef(CellRef { row: 0, col: 0, .. })
    ));
    assert!(matches!(
        parse("B2"),
        Expr::CellRef(CellRef { row: 1, col: 1, .. })
    ));
    assert!(matches!(
        parse("AA100"),
        Expr::CellRef(CellRef {
            row: 99,
            col: 26,
            ..
        })
    ));
}

#[test]
fn cell_range() {
    match parse("A1:B10") {
        Expr::Range(r) => {
            assert_eq!(r.row_start, 0);
            assert_eq!(r.row_end, 9);
            assert_eq!(r.col_start, 0);
            assert_eq!(r.col_end, 1);
        }
        other => panic!("expected Range, got {other:?}"),
    }
}

#[test]
fn arithmetic_add() {
    match parse("A1+B2") {
        Expr::BinOp {
            kind: BinOpKind::Add,
            ..
        } => {}
        other => panic!("expected Add, got {other:?}"),
    }
}

#[test]
fn arithmetic_precedence() {
    let ast = parse("1+2*3");
    match &ast {
        Expr::BinOp {
            kind: BinOpKind::Add,
            rhs,
            ..
        } => {
            assert!(matches!(
                **rhs,
                Expr::BinOp {
                    kind: BinOpKind::Mul,
                    ..
                }
            ));
        }
        other => panic!("expected Add with Mul rhs, got {other:?}"),
    }
}

#[test]
fn parens_override_precedence() {
    let ast = parse("(1+2)*3");
    match &ast {
        Expr::BinOp {
            kind: BinOpKind::Mul,
            lhs,
            ..
        } => {
            assert!(matches!(
                **lhs,
                Expr::BinOp {
                    kind: BinOpKind::Add,
                    ..
                }
            ));
        }
        other => panic!("expected Mul with Add lhs, got {other:?}"),
    }
}

#[test]
fn comparison_eq() {
    match parse("A1=B1") {
        Expr::BinOp {
            kind: BinOpKind::Eq,
            ..
        } => {}
        other => panic!("expected Eq, got {other:?}"),
    }
}

#[test]
fn comparison_chain() {
    match parse("A1<B1") {
        Expr::BinOp {
            kind: BinOpKind::Lt,
            ..
        } => {}
        other => panic!("expected Lt, got {other:?}"),
    }
}

#[test]
fn concatenation() {
    match parse(r#"A1&"hello""#) {
        Expr::BinOp {
            kind: BinOpKind::Concat,
            ..
        } => {}
        other => panic!("expected Concat, got {other:?}"),
    }
}

#[test]
fn function_no_args() {
    match parse("PI()") {
        Expr::FnCall { name, args } => {
            assert_eq!(name, "PI");
            assert!(args.is_empty());
        }
        other => panic!("expected FnCall, got {other:?}"),
    }
}

#[test]
fn function_single_arg() {
    match parse("SUM(A1:A10)") {
        Expr::FnCall { name, args } => {
            assert_eq!(name, "SUM");
            assert_eq!(args.len(), 1);
            assert!(matches!(&args[0], Expr::Range(_)));
        }
        other => panic!("expected FnCall SUM, got {other:?}"),
    }
}

// -- Sheet-qualified references & named ranges ----------------------------

use super::super::types::{EngineSheet, FormulaCtx, NameTarget, RangeDep};

const S0: EngineSheet = EngineSheet::ACTIVE;
const S2: EngineSheet = EngineSheet::new(2);
const S3: EngineSheet = EngineSheet::new(3);
const S5: EngineSheet = EngineSheet::new(5);

/// Test context: `Sheet2` -> index 2, name `TOTAL` -> `B1:B3` on sheet 2,
/// canonical id `#s2` -> index 2 (`#gone` resolves to nothing).
struct TestCtx;
impl FormulaCtx for TestCtx {
    fn resolve_sheet(&self, name: &str) -> Option<EngineSheet> {
        match name {
            "Sheet1" => Some(S0),
            "Sheet2" => Some(S2),
            "My Sheet" => Some(S5),
            _ => None,
        }
    }
    fn resolve_sheet_id(&self, id: &str) -> Option<EngineSheet> {
        match id {
            "s2" => Some(S2),
            _ => None,
        }
    }
    fn resolve_name(&self, name: &str) -> Option<NameTarget> {
        match name {
            "TOTAL" => Some(NameTarget::Range(RangeDep::with_sheet(S2, 0, 2, 1, 1))),
            "PIVOT" => Some(NameTarget::Cell(CellRef::with_sheet(S0, 4, 0))),
            _ => None,
        }
    }
}

fn resolve(src: &str, default_sheet: EngineSheet) -> Result<Expr, String> {
    resolve_refs(parse(src), default_sheet, &TestCtx)
}

#[test]
fn sheet_qualified_cell_parses() {
    assert!(matches!(parse("Sheet2!A1"), Expr::SheetRef { .. }));
}

#[test]
fn sheet_qualified_resolves_index() {
    match resolve("Sheet2!A1", S0).unwrap() {
        Expr::CellRef(CellRef {
            sheet: S2,
            row: 0,
            col: 0,
        }) => {}
        other => panic!("expected sheet-2 cell, got {other:?}"),
    }
}

#[test]
fn quoted_sheet_name_with_space() {
    match resolve("'My Sheet'!B2", S0).unwrap() {
        Expr::CellRef(CellRef {
            sheet: S5,
            row: 1,
            col: 1,
        }) => {}
        other => panic!("expected sheet-5 cell, got {other:?}"),
    }
}

#[test]
fn sheet_qualified_range_resolves() {
    match resolve("Sheet2!A1:A10", S0).unwrap() {
        Expr::Range(RangeDep {
            sheet: S2,
            row_start: 0,
            row_end: 9,
            ..
        }) => {}
        other => panic!("expected sheet-2 range, got {other:?}"),
    }
}

#[test]
fn id_qualified_cell_resolves_index() {
    match resolve("#s2!A1", S0).unwrap() {
        Expr::CellRef(CellRef {
            sheet: S2,
            row: 0,
            col: 0,
        }) => {}
        other => panic!("expected sheet-2 cell, got {other:?}"),
    }
}

#[test]
fn id_qualified_with_underscore_and_leading_digit() {
    // A peer-scoped id can start with a hex digit and contain '_'.
    assert!(matches!(
        parse("#00a_2f!A1"),
        Expr::SheetRef { sheet: SheetQualifier::Id(id), .. } if id == "00a_2f"
    ));
}

#[test]
fn deleted_sheet_id_yields_broken_ref() {
    match resolve("#gone!A1", S0).unwrap() {
        Expr::Literal(CellValue::Error(FormulaError::BrokenRef)) => {}
        other => panic!("expected #REF! literal, got {other:?}"),
    }
}

#[test]
fn unknown_sheet_name_is_parse_error() {
    assert!(resolve("Nope!A1", S0).is_err());
}

#[test]
fn unqualified_ref_takes_default_sheet() {
    match resolve("A1", S3).unwrap() {
        Expr::CellRef(CellRef {
            sheet: S3,
            row: 0,
            col: 0,
        }) => {}
        other => panic!("expected default-sheet cell, got {other:?}"),
    }
}

#[test]
fn unknown_sheet_is_error() {
    assert!(resolve("Nope!A1", S0).is_err());
}

#[test]
fn named_range_resolves() {
    match resolve("SUM(TOTAL)", S0).unwrap() {
        Expr::FnCall { name, args } => {
            assert_eq!(name, "SUM");
            assert!(matches!(
                &args[0],
                Expr::Range(RangeDep {
                    sheet: S2,
                    row_start: 0,
                    row_end: 2,
                    ..
                })
            ));
        }
        other => panic!("expected SUM(range), got {other:?}"),
    }
}

#[test]
fn named_cell_in_arithmetic() {
    // PIVOT + 1 -> CellRef(sheet 0, E1) added to 1.
    match resolve("PIVOT+1", S0).unwrap() {
        Expr::BinOp { lhs, .. } => {
            assert!(matches!(
                *lhs,
                Expr::CellRef(CellRef {
                    sheet: S0,
                    row: 4,
                    col: 0
                })
            ));
        }
        other => panic!("expected BinOp, got {other:?}"),
    }
}

#[test]
fn undefined_name_is_error() {
    assert!(matches!(
        resolve("UNKNOWN", S0).unwrap(),
        Expr::Literal(CellValue::Error(FormulaError::NameError))
    ));
}

#[test]
fn bareword_name_does_not_shadow_cell_ref() {
    // `A1` is a valid cell ref, never treated as a name.
    assert!(matches!(resolve("A1", S0).unwrap(), Expr::CellRef(_)));
}

#[test]
fn function_multiple_args() {
    match parse("IF(A1>5, 1, 0)") {
        Expr::FnCall { name, args } => {
            assert_eq!(name, "IF");
            assert_eq!(args.len(), 3);
        }
        other => panic!("expected FnCall IF, got {other:?}"),
    }
}

#[test]
fn power_right_associative() {
    let ast = parse("2^3^2");
    match &ast {
        Expr::BinOp {
            kind: BinOpKind::Pow,
            lhs,
            rhs,
        } => {
            assert!(matches!(**lhs, Expr::Literal(CellValue::Number(2.0))));
            assert!(matches!(
                **rhs,
                Expr::BinOp {
                    kind: BinOpKind::Pow,
                    ..
                }
            ));
        }
        other => panic!("expected right-assoc pow, got {other:?}"),
    }
}

#[test]
fn empty_formula() {
    assert!(parse_formula("").is_err());
    assert!(parse_formula("   ").is_err());
}

#[test]
fn invalid_formula() {
    assert!(parse_formula("=1").is_err());
    assert!(parse_formula("SUM(").is_err());
}

#[test]
fn error_literal_reparses() {
    // The renderer emits `#REF!` whenever a paste drives a reference off the
    // grid, and the Logic thread re-parses that text on write. Without an error
    // literal in the grammar the cell would settle on `#PARSE!`, not `#REF!`.
    assert!(matches!(
        parse("#REF!"),
        Expr::Literal(CellValue::Error(FormulaError::BrokenRef))
    ));
    assert!(matches!(parse("1+#REF!"), Expr::BinOp { .. }));
    assert!(matches!(
        parse("#N/A"),
        Expr::Literal(CellValue::Error(FormulaError::NaError))
    ));
    assert!(matches!(
        parse("#DIV/0!"),
        Expr::Literal(CellValue::Error(FormulaError::DivisionByZero))
    ));
}

#[test]
fn sheet_qualifier_still_wins_over_the_error_literal() {
    // Both start with `#`: `#s0!A1` must stay a sheet-qualified reference.
    assert!(matches!(parse("#s0!A1"), Expr::SheetRef { .. }));
}

#[test]
fn row_zero_is_not_a_reference() {
    // Rows are 1-based in A1 syntax, so `A0` names nothing. It must fall through
    // to the named-range parser rather than silently aliasing row 1.
    assert!(matches!(parse("A0"), Expr::Name(n) if n == "A0"));
    assert!(matches!(parse("A1"), Expr::CellRef(r) if r.row == 0));
}

#[test]
fn bool_literal_matches_only_a_whole_word() {
    // A named range that merely starts with TRUE/FALSE must stay a bareword;
    // consuming the prefix left ":VALUE" as unparsed trailing input.
    assert!(matches!(parse("TRUEVALUE"), Expr::Name(n) if n == "TRUEVALUE"));
    assert!(matches!(parse("FALSEY"), Expr::Name(n) if n == "FALSEY"));
    assert!(matches!(
        parse("TRUE"),
        Expr::Literal(CellValue::Boolean(true))
    ));
    assert!(matches!(parse("TRUE+1"), Expr::BinOp { .. }));
}

/// `TRAME-013`: the formula the fuzzer found, 2 356 nested parentheses,
/// overflowed the stack. Past each bound the formula is refused as an error.
#[test]
fn a_formula_past_a_bound_is_refused_before_it_recurses() {
    let nested = format!("{}1{}", "(".repeat(2_356), ")".repeat(2_356));
    assert!(parse_formula(&nested).is_err());
    let long = "1".repeat(MAX_FORMULA_CHARS + 1);
    assert!(parse_formula(&long).is_err());
    let chain = vec!["1"; MAX_FORMULA_OPERATORS + 2].join("+");
    assert!(parse_formula(&chain).is_err());
    // Inside a string an operator or a parenthesis is text.
    let text = format!("\"{}\"", "(-".repeat(MAX_FORMULA_OPERATORS));
    assert!(parse_formula(&text).is_ok());
}

/// `TRAME-013`: the worst formula each bound still admits parses, compiles,
/// renders and drops on a 1 MiB stack, the default of a wasm thread. A debug
/// build spends several times the stack per level, so it gets 16 MiB.
#[test]
fn the_worst_admitted_formula_fits_a_small_stack() {
    let worst = [
        format!(
            "{}1{}",
            "SUM(".repeat(MAX_FORMULA_NESTING),
            ")".repeat(MAX_FORMULA_NESTING)
        ),
        vec!["1"; MAX_FORMULA_OPERATORS + 1].join("^"),
        vec!["1"; MAX_FORMULA_OPERATORS + 1].join("+"),
        format!("{}1", "-".repeat(MAX_FORMULA_OPERATORS)),
    ];
    let run = std::thread::Builder::new()
        .stack_size(if cfg!(debug_assertions) {
            16 << 20
        } else {
            1 << 20
        })
        .spawn(move || {
            let mut indices = crate::formula::compile::SheetIndices::new();
            indices.insert(S0, crate::formula::test_index::index_with(4, 4));
            for formula in &worst {
                let ast = parse_formula(formula).expect("admitted");
                let _ = crate::formula::compile::compile(&ast, &indices);
                let _ = crate::formula::render::render(&ast);
            }
        })
        .expect("thread spawns");
    assert!(run.join().is_ok());
}
