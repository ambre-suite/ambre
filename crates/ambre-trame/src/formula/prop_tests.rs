// ambre-trame/src/formula/prop_tests.rs
// ============================================================================
// Property tests for the formula stack (parser/render, compile, VM, engine,
// functions). The crate's convergence proptest (`logic/convergence_tests.rs`)
// covers the CRDT contract. This module covers the *algebraic* contracts of the
// most structural subsystem: a parser/printer, a compiler, a stack VM and a
// dependency-driven engine. Those were exercised by examples only.
//
// Host-only for the same reason as `convergence_tests`: `proptest` is a
// host-target dev-dependency (its `wait-timeout` transitive dep does not build
// on wasm32). The properties are pure logic, so they run under `just
// test-native`.
//
// Properties asserted:
//  * parser/printer agreement  -- render is a fixpoint of `parse . render`:
//    `render(parse(render(ast))) == render(ast)` over a generated AST.
//  * parser totality           -- `parse_formula` never panics on any input.
//  * compiler well-formedness  -- emitted bytecode is stack-balanced (never
//    underflows, leaves exactly one result) for any AST.
//  * VM totality + determinism -- `execute` never panics and is referentially
//    transparent for a given bytecode + state.
//  * engine oracle             -- `SUM` over a grid of plain cells equals the
//    naive sum, and a recompute is deterministic.
//  * function algebra          -- order-independence, flattening, and the
//    AVERAGE = SUM/COUNT / MIN<=x<=MAX / COUNT invariants.
// ============================================================================

use std::collections::HashMap;

use proptest::prelude::*;

use super::cache::BytecodeCache;
use super::compile::compile;
use super::functions::{FnCtx, call_builtin};
use super::parser::parse_formula;
use super::render::render;
use super::test_index::{function, indices_with, stored};
use super::types::EngineSheet;
use super::types::{
    BinOpKind, CellId, CellRef, CellValue, Expr, Op, RangeDep, RangeValue, UnaryOpKind,
};
use super::vm::EvalContext;
use crate::model::Uid;
use crate::{Col, ColCount, Row, RowCount};

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// Bound cell at grid `(row, col)` on sheet 0, matching the `test_index` uid
/// scheme (rows on peer 1, columns on peer 2, counter == position).
fn cell(row: u32, col: u32) -> CellId {
    CellId::bound(
        EngineSheet::ACTIVE,
        Uid {
            peer: ambre_core::PeerId::new(1),
            counter: ambre_core::IdCounter::new(row),
        },
        Uid {
            peer: ambre_core::PeerId::new(2),
            counter: ambre_core::IdCounter::new(col),
        },
    )
}

/// A no-clock, position-less function context (the algebraic functions under
/// test ignore all three fields).
fn fnctx() -> FnCtx {
    FnCtx {
        now_serial: ambre_bus::DateSerial::new(0.0),
        cell_row: None,
        cell_col: None,
    }
}

/// Extract the numeric payload of a `CellValue`, failing the property otherwise.
fn as_num(v: &CellValue) -> f64 {
    match v {
        CellValue::Number(n) => *n,
        other => panic!("expected Number, got {other:?}"),
    }
}

/// Wrap a slice of integers as numeric `CellValue` arguments.
fn nums(xs: &[i64]) -> Vec<CellValue> {
    xs.iter().map(|&n| CellValue::Number(n as f64)).collect()
}

// -----------------------------------------------------------------------------
// AST generation
// -----------------------------------------------------------------------------

/// Numeric literals restricted to values whose `render` output the (exponent-free)
/// number grammar can read back exactly: non-negative integers and two-decimal
/// fractions. This keeps the round-trip property about parser/printer *structure*
/// rather than float formatting.
fn num_leaf() -> impl Strategy<Value = f64> {
    prop_oneof![
        (0u32..100_000).prop_map(|n| n as f64),
        (0u32..100_000).prop_map(|n| n as f64 / 100.0),
    ]
}

/// Cell references inside a generous positional window.
fn cellref() -> impl Strategy<Value = CellRef> {
    (0u32..1_000, 0u32..1_000).prop_map(|(r, c)| CellRef::new(Row::new(r), Col::new(c)))
}

/// Normalized ranges (`start <= end` on both axes), as the parser emits.
fn rangedep() -> impl Strategy<Value = RangeDep> {
    (0u32..500, 0u32..500, 0u32..500, 0u32..500).prop_map(|(a, b, c, d)| {
        RangeDep::new(
            Row::new(a.min(b)),
            Row::new(a.max(b)),
            Col::new(c.min(d)),
            Col::new(c.max(d)),
        )
    })
}

/// Text literals over a quote-free, exponent-free charset (escaping has its own
/// example test; here we want broad structural coverage).
fn string_leaf() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 ]{0,6}").unwrap()
}

fn binop_kind() -> impl Strategy<Value = BinOpKind> {
    prop_oneof![
        Just(BinOpKind::Add),
        Just(BinOpKind::Sub),
        Just(BinOpKind::Mul),
        Just(BinOpKind::Div),
        Just(BinOpKind::Pow),
        Just(BinOpKind::Eq),
        Just(BinOpKind::Ne),
        Just(BinOpKind::Lt),
        Just(BinOpKind::Le),
        Just(BinOpKind::Gt),
        Just(BinOpKind::Ge),
        Just(BinOpKind::Concat),
    ]
}

/// A small set of real function names. Unknown args are fine: the VM-totality
/// property wants these to *degrade*, never panic. (Index-style lookups are kept
/// out so generation stays focused on the arithmetic/aggregation core.)
fn fn_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("SUM".to_string()),
        Just("AVERAGE".to_string()),
        Just("MIN".to_string()),
        Just("MAX".to_string()),
        Just("ABS".to_string()),
        Just("ROUND".to_string()),
        Just("IF".to_string()),
        Just("CONCAT".to_string()),
    ]
}

/// A parser-expressible `Expr` AST: every node the context-free parser can emit
/// except `SheetRef`/`Name` (which need a resolution context). Recursion is
/// bounded so the suite stays fast.
fn expr_strategy() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        num_leaf().prop_map(|n| Expr::Literal(CellValue::Number(n))),
        string_leaf().prop_map(|s| Expr::Literal(CellValue::Text(s))),
        any::<bool>().prop_map(|b| Expr::Literal(CellValue::Boolean(b))),
        cellref().prop_map(Expr::CellRef),
        rangedep().prop_map(Expr::Range),
    ];
    leaf.prop_recursive(5, 48, 3, |inner| {
        prop_oneof![
            (binop_kind(), inner.clone(), inner.clone()).prop_map(|(kind, lhs, rhs)| {
                Expr::BinOp {
                    kind,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }
            }),
            inner.clone().prop_map(|e| Expr::UnaryOp {
                kind: UnaryOpKind::Neg,
                operand: Box::new(e),
            }),
            (fn_name(), proptest::collection::vec(inner, 0..3)).prop_map(|(name, args)| {
                Expr::FnCall {
                    name: function(&name),
                    args,
                }
            }),
        ]
    })
}

// -----------------------------------------------------------------------------
// Bytecode well-formedness
// -----------------------------------------------------------------------------

/// Simulate the stack depth produced by a bytecode sequence using each op's
/// static arity. A well-formed program never reads below the operands it needs
/// and finishes with exactly one result on the stack.
fn stack_balanced(ops: &[Op]) -> bool {
    let mut depth: i64 = 0;
    for op in ops {
        let (needs, delta): (i64, i64) = match op {
            Op::PushNum(_)
            | Op::PushStr(_)
            | Op::PushBool(_)
            | Op::PushEmpty
            | Op::PushError(_)
            | Op::PushCell(_)
            | Op::EvalRange(_) => (0, 1),
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Pow
            | Op::Eq
            | Op::Ne
            | Op::Lt
            | Op::Le
            | Op::Gt
            | Op::Ge
            | Op::Concat => (2, -1),
            Op::Neg | Op::Not => (1, 0),
            Op::CallFn { n_args, .. } => {
                let n = i64::from(n_args.get());
                (n, 1 - n)
            }
        };
        if depth < needs {
            return false;
        }
        depth += delta;
    }
    depth == 1
}

/// Compile `ast` over a 10x10 sheet and execute it against an empty cell cache,
/// returning the result value. Mirrors the `vm` unit-test harness.
fn eval_ast(ast: &Expr) -> CellValue {
    let indices = indices_with(10, 10);
    let bc = compile(ast, &indices);
    let mut bcc = BytecodeCache::new();
    let c = cell(0, 0);
    bcc.insert(c, bc);
    let owned = bcc.get(&c).expect("just inserted").clone();
    let cache: HashMap<CellId, CellValue> = HashMap::new();
    let mut ctx = EvalContext::new(&bcc, &cache, &indices, ambre_bus::DateSerial::new(0.0));
    ctx.execute(&owned)
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    // 256 cases per property keeps the whole module well under a second on host.
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Parser/printer agreement: rendering an AST, parsing it, and rendering
    /// again is a fixpoint. Catches any precedence/associativity/escaping
    /// mismatch between `render` and `parse_formula`.
    #[test]
    fn render_parse_render_is_a_fixpoint(ast in expr_strategy()) {
        let once = render(&ast);
        let parsed = parse_formula(&once);
        prop_assert!(
            parsed.is_ok(),
            "rendered text did not re-parse: {once:?} -> {:?}",
            parsed.err()
        );
        let twice = render(&parsed.unwrap());
        prop_assert_eq!(once, twice);
    }

    /// Parser totality over arbitrary text: never panics, only Ok/Err.
    #[test]
    fn parser_never_panics_on_arbitrary_text(s in ".*") {
        let _ = parse_formula(&s);
    }

    /// Parser totality over formula-shaped noise: stress the grammar with
    /// strings drawn from the operator/identifier/quote alphabet.
    #[test]
    fn parser_never_panics_on_formula_noise(
        s in proptest::string::string_regex("[-+*/^()=<>&,:A-Za-z0-9 .\"']{0,24}").unwrap()
    ) {
        let _ = parse_formula(&s);
    }

    /// The compiler emits stack-balanced bytecode for any AST: the VM can never
    /// underflow, and execution leaves exactly one result.
    #[test]
    fn compile_emits_balanced_bytecode(ast in expr_strategy()) {
        let indices = indices_with(10, 10);
        let bc = compile(&ast, &indices);
        prop_assert!(
            stack_balanced(&bc.ops),
            "unbalanced bytecode for {ast:?}: {:?}",
            bc.ops
        );
    }

    /// VM totality + determinism: executing compiled bytecode never panics, and
    /// the same bytecode against the same state yields the same value.
    #[test]
    fn vm_is_total_and_deterministic(ast in expr_strategy()) {
        let v1 = eval_ast(&ast).to_display();
        let v2 = eval_ast(&ast).to_display();
        prop_assert_eq!(v1, v2);
    }
}

// -----------------------------------------------------------------------------
// Engine oracle
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    /// `SUM(A1:C5)` over a grid of plain numeric cells equals the naive sum of
    /// those cells, regardless of the insertion order. A forced full recompute
    /// reproduces the result (recompute determinism).
    #[test]
    fn engine_sum_matches_naive_sum(
        entries in proptest::collection::vec((0u32..5, 0u32..3, -1_000i64..1_000), 0..16)
    ) {
        use super::FormulaEngine;
        use super::types::SingleSheetCtx;

        // Dedup to one value per cell (a later write wins, as in the engine).
        let mut grid: HashMap<(u32, u32), i64> = HashMap::new();
        for (r, c, v) in &entries {
            grid.insert((*r, *c), *v);
        }
        let oracle: i64 = grid.values().sum();

        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        for ((r, c), v) in &grid {
            engine.on_cell_changed(cell(*r, *c), stored(&v.to_string()), &idx, &SingleSheetCtx);
        }
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        // Formula lives outside the summed range (row 6).
        let f = cell(6, 0);
        engine.on_cell_changed(f, stored("=SUM(A1:C5)"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        prop_assert_eq!(engine.evaluated_text(f).unwrap(), oracle.to_string());

        // Recompute everything from scratch: same answer.
        engine.mark_all_dirty();
        engine.compute_all_dirty(&idx);
        prop_assert_eq!(engine.evaluated_text(f).unwrap(), oracle.to_string());
    }
}

// -----------------------------------------------------------------------------
// Function algebra
//
// Integer-valued inputs keep every sum/product exact (within 2^53), so
// reordering equalities hold without a float tolerance.
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// SUM equals the naive total and is invariant under argument order.
    #[test]
    fn sum_is_exact_and_order_independent(xs in proptest::collection::vec(-100_000i64..100_000, 0..24)) {
        let forward = as_num(&call_builtin(&function("SUM"), &nums(&xs), fnctx()));
        let mut reversed = xs.clone();
        reversed.reverse();
        let backward = as_num(&call_builtin(&function("SUM"), &nums(&reversed), fnctx()));
        prop_assert_eq!(forward, backward);
        prop_assert_eq!(forward, xs.iter().sum::<i64>() as f64);
    }

    /// A total of decimals of mixed magnitudes is the same in every order:
    /// `f64` addition is not associative, so a plain fold fails this
    /// (`TRAME-199`).
    #[test]
    fn a_total_of_decimals_is_the_same_in_any_order(
        (xs, shuffled) in proptest::collection::vec(
            (-1_000_000_000i64..1_000_000_000, 0i32..18).prop_map(|(m, e)| m as f64 * 10f64.powi(e - 9)),
            0..32,
        )
        .prop_flat_map(|xs| (Just(xs.clone()), Just(xs).prop_shuffle())),
    ) {
        let total = |v: &[f64]| {
            let args: Vec<CellValue> = v.iter().map(|&n| CellValue::Number(n)).collect();
            as_num(&call_builtin(&function("SUM"), &args, fnctx()))
        };
        let mut reversed = xs.clone();
        reversed.reverse();
        prop_assert_eq!(total(&xs).to_bits(), total(&shuffled).to_bits());
        prop_assert_eq!(total(&xs).to_bits(), total(&reversed).to_bits());
    }

    /// Padding the arguments with zeros does not change SUM.
    #[test]
    fn sum_ignores_appended_zeros(
        xs in proptest::collection::vec(-100_000i64..100_000, 0..16),
        zeros in 0usize..8,
    ) {
        let base = as_num(&call_builtin(&function("SUM"), &nums(&xs), fnctx()));
        let mut padded = xs.clone();
        padded.extend(std::iter::repeat_n(0, zeros));
        let padded_sum = as_num(&call_builtin(&function("SUM"), &nums(&padded), fnctx()));
        prop_assert_eq!(base, padded_sum);
    }

    /// A range argument flattens to the same SUM as the equivalent scalar list:
    /// `SUM(range) == SUM(v1, v2, ...)`.
    #[test]
    fn sum_flattens_range_like_scalars(xs in proptest::collection::vec(-100_000i64..100_000, 0..24)) {
        let scalar = as_num(&call_builtin(&function("SUM"), &nums(&xs), fnctx()));
        let range = CellValue::Range(RangeValue::new(
            RowCount::ONE,
            ColCount::from_len(xs.len()),
            nums(&xs),
        ));
        let ranged = as_num(&call_builtin(&function("SUM"), &[range], fnctx()));
        prop_assert_eq!(scalar, ranged);
    }

    /// AVERAGE * COUNT == SUM for any non-empty numeric input.
    #[test]
    fn average_times_count_equals_sum(xs in proptest::collection::vec(-100_000i64..100_000, 1..24)) {
        let args = nums(&xs);
        let avg = as_num(&call_builtin(&function("AVERAGE"), &args, fnctx()));
        let count = as_num(&call_builtin(&function("COUNT"), &args, fnctx()));
        let sum = as_num(&call_builtin(&function("SUM"), &args, fnctx()));
        prop_assert_eq!(count, xs.len() as f64);
        // avg*count reconstructs sum; integers keep this within a tiny epsilon.
        prop_assert!((avg * count - sum).abs() < 1e-6);
    }

    /// MIN and MAX bound every element, and MIN <= MAX.
    #[test]
    fn min_max_bound_every_element(xs in proptest::collection::vec(-100_000i64..100_000, 1..24)) {
        let args = nums(&xs);
        let lo = as_num(&call_builtin(&function("MIN"), &args, fnctx()));
        let hi = as_num(&call_builtin(&function("MAX"), &args, fnctx()));
        prop_assert!(lo <= hi);
        for &x in &xs {
            prop_assert!(lo <= x as f64);
            prop_assert!((x as f64) <= hi);
        }
        prop_assert_eq!(lo, *xs.iter().min().unwrap() as f64);
        prop_assert_eq!(hi, *xs.iter().max().unwrap() as f64);
    }

    /// PRODUCT is invariant under argument order (bounded magnitude keeps the
    /// product exact in f64).
    #[test]
    fn product_is_order_independent(xs in proptest::collection::vec(-10i64..10, 0..10)) {
        let forward = as_num(&call_builtin(&function("PRODUCT"), &nums(&xs), fnctx()));
        let mut reversed = xs.clone();
        reversed.reverse();
        let backward = as_num(&call_builtin(&function("PRODUCT"), &nums(&reversed), fnctx()));
        prop_assert_eq!(forward, backward);
    }
}
