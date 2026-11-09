//! Stack VM for evaluating formula bytecode.
//!
//! The VM executes a [`CellBytecode`] sequence on a value stack. It detects
//! dynamic cycles through an `eval_stack`: a cycle through a range or a
//! cross-sheet reference is invisible in the static cell_dag. It memoizes
//! results in `resolved` to avoid redundant work within one recomputation pass.
//!
//! References are uid-anchored ([`CellId`] / [`RangeId`]). At evaluation each
//! reference is resolved against the current per-sheet [`SheetIndex`]. A bound
//! `Uid` that is missing yields `#REF!`; a virtual position beyond the used
//! range reads `Empty`. The lookup is normalized to the bound `(row_uid,
//! col_uid)` form so it matches the keys fed by the host.

use crate::hash::{HashMap, HashSet};
use ambre_core::index::{Len, Pos};

use ambre_bus::{Col, Row};

use super::cache::BytecodeCache;
use super::functions::call_builtin;
use super::text::fold_cmp;
use super::types::{
    CellId, CellValue, EngineSheet, FormulaError, Op, RangeId, RangeValue, nearly_equal,
};
use crate::model::SheetIndex;
use ambre_core::{Cells, Count, Instant, SheetDateTime};

/// Upper bound on the number of cells a single range reference may materialize
/// during evaluation. Protects the WASM Logic worker from a peer-supplied
/// formula (for example `=SUM(A1:ZZ100000)`) whose range would allocate gigabytes.
/// 1M cells covers any realistic used area while capping the blast radius.
const MAX_RANGE_CELLS: Count<Cells> = Count::<Cells>::new(1_000_000);

/// Ceiling on the range cells one recomputation pass may read, all ranges
/// together (`TRAME-210`). `MAX_RANGE_CELLS` bounds the memory of one range,
/// which is freed after its call. It leaves the time unbounded: a hundred
/// formulas each just under it cost a hundred million reads on every recompute.
/// The ceiling clears a running total over ten thousand rows (`SUM(A$1:An)`,
/// fifty million reads), the heaviest shape a real sheet takes.
pub(crate) const MAX_PASS_RANGE_CELLS: Count<Cells> = Count::<Cells>::new(64_000_000);

/// Leave to materialize one range: the count of member cells it may hold.
/// Only [`grant_range`] makes one, and the range buffer is sized by it alone.
/// So no range over the caps is ever allocated (`TRAME-209`).
#[derive(Debug, PartialEq, Eq)]
struct RangeGrant(Count<Cells>);

/// Grant a `rows` x `cols` range against the per-range cap and what the pass
/// has `left`. A grant debits `left`. A refusal leaves it whole and allocates
/// nothing (`TRAME-209`, `TRAME-210`).
fn grant_range(rows: Len<Row>, cols: Len<Col>, left: &mut Count<Cells>) -> Option<RangeGrant> {
    let cells = Count::<Cells>::new(u64::from(rows.get()) * u64::from(cols.get()));
    if cells > MAX_RANGE_CELLS || cells > *left {
        return None;
    }
    *left = left.minus(cells);
    Some(RangeGrant(cells))
}

/// Holds the mutable evaluation state shared across all cell evaluations in a
/// single recomputation pass (`compute_all_dirty`).
pub struct EvalContext<'a> {
    /// Cells currently on the evaluation stack -- used for dynamic cycle
    /// detection.
    pub eval_stack: HashSet<CellId>,
    /// Results already computed during this pass, keyed by `CellId`.
    pub resolved: HashMap<CellId, CellValue>,
    /// Borrowed reference to the bytecode cache.
    pub bc: &'a BytecodeCache,
    /// Borrowed reference to the persistent formula result cache so recursive
    /// `eval_cell` calls for non-dirty cells can reuse prior results.
    pub formula_cache: &'a HashMap<CellId, CellValue>,
    /// Per-sheet position <-> uid indices, used to resolve references at eval.
    pub indices: &'a HashMap<Pos<EngineSheet>, SheetIndex>,
    /// Evaluation clock as an Excel serial number. Shared by every cell in the
    /// pass so `TODAY()`/`NOW()` are stable within one recomputation.
    pub now_serial: Instant<SheetDateTime>,
    /// Row of the cell currently being evaluated, used by the no-argument
    /// `ROW()`. `None` means "unknown" (for example a bare `execute` call in
    /// tests). Set by [`EvalContext::eval_cell`].
    pub current_row: Option<Pos<Row>>,
    /// Column of the cell currently being evaluated, for `COLUMN()`.
    pub current_col: Option<Pos<Col>>,
    /// Range cells this pass may still read ([`MAX_PASS_RANGE_CELLS`]). A range
    /// that would overdraw it reads as `#REF!`, like one over `MAX_RANGE_CELLS`.
    pub range_cells_left: Count<Cells>,
    /// Whether the evaluation in progress met the evaluation stack, through
    /// any cell it read (`TRAME-281`).
    met_loop: MetLoop,
    /// The cells this pass evaluated to `#CIRCULAR!` because they met a loop.
    /// A later read of one of them meets the loop too.
    looped: HashSet<CellId>,
}

/// Whether an evaluation met a loop the evaluation stack caught.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetLoop {
    No,
    Yes,
}

impl<'a> EvalContext<'a> {
    /// A pass is given its clock here, so no context exists without one
    /// (`TRAME-198`).
    pub fn new(
        bc: &'a BytecodeCache,
        formula_cache: &'a HashMap<CellId, CellValue>,
        indices: &'a HashMap<Pos<EngineSheet>, SheetIndex>,
        now_serial: Instant<SheetDateTime>,
    ) -> Self {
        Self {
            eval_stack: HashSet::default(),
            resolved: HashMap::default(),
            bc,
            formula_cache,
            indices,
            now_serial,
            current_row: None,
            current_col: None,
            range_cells_left: MAX_PASS_RANGE_CELLS,
            met_loop: MetLoop::No,
            looped: HashSet::default(),
        }
    }

    /// Resolve the row and column of a cell against its sheet index. A bound
    /// id that has been deleted, or a missing sheet, yields `None` on its axis.
    fn grid_pos_of(&self, cell: CellId) -> (Option<Pos<Row>>, Option<Pos<Col>>) {
        let Some(idx) = self.indices.get(&cell.sheet) else {
            return (None, None);
        };
        (cell.row.pos(idx), cell.col.pos(idx))
    }

    /// Evaluate a single cell (already in canonical bound form), detecting
    /// dynamic cycles and caching results.
    ///
    /// A cell whose evaluation meets the evaluation stack, itself or through
    /// any cell it reads, shows `#CIRCULAR!`. Another error the same pass
    /// meets first never replaces it. Each cell on a loop and each cell
    /// downstream of one then shows the same error in every pass order, on
    /// every peer (`TRAME-281`).
    pub fn eval_cell(&mut self, cell: CellId) -> CellValue {
        if self.eval_stack.contains(&cell) {
            self.met_loop = MetLoop::Yes;
            return CellValue::Error(FormulaError::CircularReference);
        }
        if let Some(v) = self.resolved.get(&cell) {
            if self.looped.contains(&cell) {
                self.met_loop = MetLoop::Yes;
            }
            return v.clone();
        }
        self.eval_stack.insert(cell);
        let outer = std::mem::replace(&mut self.met_loop, MetLoop::No);
        let prev = (self.current_row, self.current_col);
        (self.current_row, self.current_col) = self.grid_pos_of(cell);
        let mut result = match self.bc.get(&cell) {
            Some(bc) => self.execute(bc),
            None => self
                .formula_cache
                .get(&cell)
                .cloned()
                .unwrap_or(CellValue::Empty),
        };
        self.eval_stack.remove(&cell);
        (self.current_row, self.current_col) = prev;
        if self.met_loop == MetLoop::Yes {
            result = CellValue::Error(FormulaError::CircularReference);
            self.looped.insert(cell);
        } else {
            self.met_loop = outer;
        }
        self.resolved.insert(cell, result.clone());
        result
    }

    /// Resolve a [`CellId`] to the value it points at. `#REF!` for a deleted
    /// bound id, `Empty` for a virtual position beyond the used range. Otherwise
    /// the evaluated value of the canonical bound cell at that position.
    fn read_cell(&mut self, r: CellId) -> CellValue {
        let Some(idx) = self.indices.get(&r.sheet) else {
            return CellValue::Error(FormulaError::BrokenRef);
        };
        // A bound id no longer in the index reads `#REF!`.
        let (Some(rp), Some(cp)) = (r.row.pos(idx), r.col.pos(idx)) else {
            return CellValue::Error(FormulaError::BrokenRef);
        };
        match (idx.row_id(rp), idx.col_id(cp)) {
            (Some(row), Some(col)) => self.eval_cell(CellId::bound(r.sheet, row, col)),
            // Virtual position beyond the materialized range: an empty cell.
            _ => CellValue::Empty,
        }
    }

    /// Execute a compiled bytecode sequence.
    pub fn execute(&mut self, bc: &super::types::CellBytecode) -> CellValue {
        let mut stack: Vec<CellValue> = Vec::with_capacity(8);

        for op in &bc.ops {
            match op {
                Op::PushNum(n) => stack.push(CellValue::Number(*n)),
                Op::PushStr(s) => stack.push(CellValue::Text(s.as_str().to_owned())),
                Op::PushBool(b) => stack.push(CellValue::Boolean(*b)),
                Op::PushEmpty => stack.push(CellValue::Empty),
                Op::PushError(e) => stack.push(CellValue::Error(e.clone())),

                Op::PushCell(r) => {
                    let v = self.read_cell(*r);
                    stack.push(v);
                }

                Op::EvalRange(dep) => {
                    stack.push(CellValue::Range(self.eval_range(dep)));
                }

                Op::Add => self.binop(&mut stack, |a, b| a + b),
                Op::Sub => self.binop(&mut stack, |a, b| a - b),
                Op::Mul => self.binop(&mut stack, |a, b| a * b),
                Op::Div => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match (a.as_number(), b.as_number()) {
                        // An operand's own error outranks the division rules:
                        // `=1/A1` on a `#REF!` cell is `#REF!`, not `#TYPE!`.
                        (Err(e), _) | (_, Err(e)) => CellValue::Error(e),
                        (Ok(_), Ok(0.0)) => CellValue::Error(FormulaError::DivisionByZero),
                        (Ok(x), Ok(y)) => finite(x / y),
                    });
                }
                Op::Pow => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match (a.as_number(), b.as_number()) {
                        (Err(e), _) | (_, Err(e)) => CellValue::Error(e),
                        // Guard NaN/inf (for example `(-1)^0.5`, `0^-1`): a bare
                        // `powf` result would propagate silently through later
                        // arithmetic. Excel surfaces these as `#NUM!`.
                        (Ok(x), Ok(y)) => finite(x.powf(y)),
                    });
                }
                Op::Eq => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match first_error(&a, &b) {
                        Some(e) => e,
                        None => CellValue::Boolean(values_equal(&a, &b)),
                    });
                }
                Op::Ne => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match first_error(&a, &b) {
                        Some(e) => e,
                        None => CellValue::Boolean(!values_equal(&a, &b)),
                    });
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    if let Some(e) = first_error(&a, &b) {
                        stack.push(e);
                        continue;
                    }
                    stack.push(match compare_values(&a, &b) {
                        Some(ord) => {
                            let ok = match op {
                                Op::Lt => ord == std::cmp::Ordering::Less,
                                Op::Le => ord != std::cmp::Ordering::Greater,
                                Op::Gt => ord == std::cmp::Ordering::Greater,
                                Op::Ge => ord != std::cmp::Ordering::Less,
                                _ => false,
                            };
                            CellValue::Boolean(ok)
                        }
                        None => CellValue::Error(FormulaError::TypeError),
                    });
                }
                Op::Concat => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    // Without this an error operand would be stringified into the
                    // result ("x#REF!") instead of surfacing as the cell's error.
                    stack.push(match first_error(&a, &b) {
                        Some(e) => e,
                        None => CellValue::Text(a.to_display() + &b.to_display()),
                    });
                }
                Op::Neg => {
                    let v = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match v.as_number() {
                        Ok(n) => finite(-n),
                        Err(e) => CellValue::Error(e),
                    });
                }
                Op::Not => {
                    let v = stack.pop().unwrap_or(CellValue::Empty);
                    if let CellValue::Error(_) = v {
                        stack.push(v);
                        continue;
                    }
                    let truthy = match &v {
                        CellValue::Boolean(false) => false,
                        CellValue::Number(n) if *n == 0.0 => false,
                        CellValue::Text(s) if s.is_empty() => false,
                        CellValue::Empty => false,
                        _ => true,
                    };
                    stack.push(CellValue::Boolean(!truthy));
                }
                Op::CallFn { name, n_args } => {
                    let split = stack.len().saturating_sub(n_args.to_usize());
                    let args: Vec<CellValue> = stack.drain(split..).collect();
                    let fnctx = super::functions::FnCtx {
                        now_serial: self.now_serial,
                        cell_row: self.current_row,
                        cell_col: self.current_col,
                    };
                    stack.push(call_builtin(name, &args, fnctx));
                }
            }
        }

        stack.pop().unwrap_or(CellValue::Empty)
    }

    /// Evaluate every cell in a range, returning a row-major [`RangeValue`] that
    /// preserves the 2D shape (needed by `VLOOKUP`/`INDEX`/`MATCH`). Endpoints
    /// resolve through the current index; cells beyond the used range read
    /// `Empty`. A deleted bound endpoint yields a single `#REF!` value.
    fn eval_range(&mut self, dep: &RangeId) -> RangeValue {
        let Some(idx) = self.indices.get(&dep.sheet) else {
            return RangeValue::broken_ref();
        };
        let (rs, re) = match dep.row_range(idx) {
            Some(r) => r,
            None => return RangeValue::broken_ref(),
        };
        let (cs, ce) = match dep.col_range(idx) {
            Some(r) => r,
            None => return RangeValue::broken_ref(),
        };
        if re < rs || ce < cs {
            return RangeValue::new(Len::<Row>::ZERO, Len::<Col>::ZERO, Vec::new());
        }
        let rows = re.len_from(rs) + Len::<Row>::ONE;
        let cols = ce.len_from(cs) + Len::<Col>::ONE;
        // Bound the materialized range. A compiled range stores only its two
        // endpoints (O(1) to register), but evaluating it allocates one
        // `CellValue` per member cell. With remote apply live, a malicious peer
        // can push `=SUM(A1:ZZ100000)` and force every collaborator to allocate
        // ~70M values on each recompute. Cap the cell count and surface `#REF!`
        // rather than let a single formula OOM the WASM Logic worker. The
        // ceiling is generous relative to any real spreadsheet's used area.
        // The whole pass shares one budget (`TRAME-210`).
        let Some(RangeGrant(cells)) = grant_range(rows, cols, &mut self.range_cells_left) else {
            return RangeValue::broken_ref();
        };
        let mut values = Vec::with_capacity(usize::try_from(cells.get()).unwrap_or_default());
        for rp in rs.through(re) {
            for cp in cs.through(ce) {
                let v = match (idx.row_id(rp), idx.col_id(cp)) {
                    (Some(row), Some(col)) => self.eval_cell(CellId::bound(dep.sheet, row, col)),
                    _ => CellValue::Empty,
                };
                values.push(v);
            }
        }
        RangeValue::new(rows, cols, values)
    }

    /// Pop two values, apply a numeric binary operation, push the result.
    fn binop<F>(&mut self, stack: &mut Vec<CellValue>, f: F)
    where
        F: Fn(f64, f64) -> f64,
    {
        let b = stack.pop().unwrap_or(CellValue::Empty);
        let a = stack.pop().unwrap_or(CellValue::Empty);
        match (a.as_number(), b.as_number()) {
            (Ok(x), Ok(y)) => stack.push(finite(f(x, y))),
            (Err(e), _) | (_, Err(e)) => stack.push(CellValue::Error(e)),
        }
    }
}

/// An operator's numeric result, or `#NUM!` when it is not finite
/// (`TRAME-244`).
fn finite(n: f64) -> CellValue {
    if n.is_finite() {
        CellValue::Number(n)
    } else {
        CellValue::Error(FormulaError::NumError)
    }
}

// -----------------------------------------------------------------------------
// Value comparison helpers
// -----------------------------------------------------------------------------

/// The leftmost error among two operands, if either carries one. Every operator
/// consults this before coercing. An error is a value that must reach the cell,
/// not one to coerce into a number, a boolean or text.
fn first_error(a: &CellValue, b: &CellValue) -> Option<CellValue> {
    match (a, b) {
        (CellValue::Error(_), _) => Some(a.clone()),
        (_, CellValue::Error(_)) => Some(b.clone()),
        _ => None,
    }
}

/// An empty operand takes the zero of whatever type it is weighed against: `0`
/// for a number, `""` for text, `false` for a boolean. [`CellValue::as_number`]
/// already reads `Empty` as `0`. Without this, the same blank cell answered `1`
/// in `=A1+1` and `#TYPE!` in `=A1>0`. That is the case a formula hits the moment
/// it is dragged one row past the end of its data. Excel coerces the same way.
fn values_equal(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => nearly_equal(*x, *y),
        // Text meets text whatever its case, as Excel compares it (`TRAME-252`).
        (CellValue::Text(x), CellValue::Text(y)) => fold_cmp(x, y).is_eq(),
        (CellValue::Boolean(x), CellValue::Boolean(y)) => x == y,
        (CellValue::Empty, CellValue::Empty) => true,
        (CellValue::Empty, CellValue::Text(s)) | (CellValue::Text(s), CellValue::Empty) => {
            s.is_empty()
        }
        (CellValue::Empty, CellValue::Number(n)) | (CellValue::Number(n), CellValue::Empty) => {
            nearly_equal(0.0, *n)
        }
        (CellValue::Empty, CellValue::Boolean(b)) | (CellValue::Boolean(b), CellValue::Empty) => {
            !*b
        }
        _ => false,
    }
}

/// Order two operands, or `None` when the pair has no ordering (`#TYPE!`). Blanks
/// coerce as in [`values_equal`]; booleans order `false` before `true`. Mixed
/// number/text/boolean pairs stay unordered: Excel ranks them by type, we would
/// rather surface the mistake than answer a comparison nobody meant to write.
fn compare_values(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => x.partial_cmp(y),
        (CellValue::Text(x), CellValue::Text(y)) => Some(fold_cmp(x, y)),
        (CellValue::Boolean(x), CellValue::Boolean(y)) => Some(x.cmp(y)),
        (CellValue::Empty, CellValue::Empty) => Some(Ordering::Equal),
        (CellValue::Empty, CellValue::Number(y)) => 0.0f64.partial_cmp(y),
        (CellValue::Number(x), CellValue::Empty) => x.partial_cmp(&0.0),
        (CellValue::Empty, CellValue::Text(y)) => Some("".cmp(y.as_str())),
        (CellValue::Text(x), CellValue::Empty) => Some(x.as_str().cmp("")),
        (CellValue::Empty, CellValue::Boolean(y)) => Some(false.cmp(y)),
        (CellValue::Boolean(x), CellValue::Empty) => Some(x.cmp(&false)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::compile;
    use super::super::parser::parse_formula;
    use super::super::test_index::{index_with, indices_with};
    use super::super::types::ACTIVE_SHEET;
    use super::super::types::CellValue;
    use super::*;
    use crate::model::Uid;

    fn cell(row: u32, col: u32) -> CellId {
        CellId::bound(
            ACTIVE_SHEET,
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

    /// Compile `src` for the cell at `(0, fcol)` over a 10x10 sheet. Evaluate it
    /// against `cache` (plain cell values keyed by their bound `CellId`) and
    /// return the result text. Exercises `PushCell`/`EvalRange` resolution.
    fn eval_with_cells(src: &str, fcol: u32, cache: HashMap<CellId, CellValue>) -> String {
        let indices = indices_with(10, 10);
        let bc = compile(&parse_formula(src).unwrap(), &indices);
        let mut bcc = BytecodeCache::new();
        bcc.insert(cell(0, fcol), bc);
        let mut ctx = EvalContext::new(&bcc, &cache, &indices, Instant::<SheetDateTime>::new(0.0));
        ctx.eval_cell(cell(0, fcol)).to_display()
    }

    #[test]
    fn push_cell_reads_bound_value() {
        let mut cache = HashMap::default();
        cache.insert(cell(0, 0), CellValue::Number(7.0)); // A1 = 7
        assert_eq!(eval_with_cells("A1*2", 1, cache), "14");
    }

    #[test]
    fn push_cell_virtual_beyond_used_range_reads_empty() {
        // Column Z (25) is past the 10 materialized columns -> virtual -> Empty,
        // so the sum is just the literal addend.
        assert_eq!(eval_with_cells("Z1+5", 1, HashMap::default()), "5");
    }

    #[test]
    fn push_cell_deleted_bound_yields_ref_error() {
        // Compile `=A1` (binds A1 -> Bound(row0,col0)) then evaluate against an
        // index whose row 0 has been deleted: the bound uid is gone -> #REF!.
        let indices_full = indices_with(10, 10);
        let bc = compile(&parse_formula("A1").unwrap(), &indices_full);
        let mut bcc = BytecodeCache::new();
        bcc.insert(cell(0, 1), bc);

        let mut idx = index_with(10, 10);
        idx.apply_delete_row(Pos::<Row>::new(0)); // removes row_uid(0)
        let mut indices = HashMap::default();
        indices.insert(ACTIVE_SHEET, idx);

        let cache = HashMap::default();
        let mut ctx = EvalContext::new(&bcc, &cache, &indices, Instant::<SheetDateTime>::new(0.0));
        assert!(matches!(
            ctx.eval_cell(cell(0, 1)),
            CellValue::Error(FormulaError::BrokenRef)
        ));
    }

    #[test]
    fn every_operator_propagates_an_operand_error() {
        // An operand carrying `#REF!` must surface `#REF!`, whatever the operator.
        // `Div`/`Pow` used to collapse it into `#TYPE!`, `Concat` rendered it as
        // the text "#REF!", and the comparisons answered a plain boolean.
        for src in [
            "1/A1", "A1/1", "A1^2", "2^A1", "1+A1", "\"x\"&A1", "A1&\"x\"", "A1=1", "A1<>1",
            "A1<1", "A1>=1",
        ] {
            let mut cache = HashMap::default();
            cache.insert(cell(0, 0), CellValue::Error(FormulaError::BrokenRef));
            assert_eq!(eval_with_cells(src, 1, cache), "#REF!", "formula: ={src}");
        }
    }

    #[test]
    fn unary_not_propagates_an_operand_error() {
        // The parser never emits a unary NOT today, so drive the op directly.
        let bc = super::super::types::CellBytecode {
            ops: vec![
                Op::PushError(FormulaError::BrokenRef),
                super::super::types::Op::Not,
            ],
            cell_deps: vec![],
            range_deps: vec![],
        };
        let bcc = BytecodeCache::new();
        let cache = HashMap::default();
        let indices = indices_with(10, 10);
        let mut ctx = EvalContext::new(&bcc, &cache, &indices, Instant::<SheetDateTime>::new(0.0));
        assert!(matches!(
            ctx.execute(&bc),
            CellValue::Error(FormulaError::BrokenRef)
        ));
    }

    #[test]
    fn an_empty_operand_compares_at_its_peer_zero() {
        // `as_number` reads `Empty` as 0, so `=A1+1` answers 1 on a blank A1. The
        // comparisons refused that same operand and answered `#TYPE!`. A formula
        // dragged one row past the end of its data (`=IF(A1>5,..)`, `=A1=0`)
        // broke on the blank cell instead of reading it as zero.
        for (src, expect) in [
            ("A1=0", "true"),
            ("A1<>0", "false"),
            ("A1<1", "true"),
            ("A1>0", "false"),
            ("A1>=0", "true"),
            ("A1<=0", "true"),
            ("IF(A1>5,\"big\",\"small\")", "small"),
            // Against text, the zero is the empty string.
            ("A1<\"x\"", "true"),
            ("A1=\"\"", "true"),
            // Against a boolean, it is `false`.
            ("A1=FALSE", "true"),
            ("A1<TRUE", "true"),
        ] {
            assert_eq!(
                eval_with_cells(src, 1, HashMap::default()),
                expect,
                "formula: ={src}"
            );
        }
        // Two blanks are equal, and order as equal.
        assert_eq!(eval_with_cells("A1=B1", 2, HashMap::default()), "true");
        assert_eq!(eval_with_cells("A1<B1", 2, HashMap::default()), "false");
        assert_eq!(eval_with_cells("A1>=B1", 2, HashMap::default()), "true");
    }

    /// The operators compare text whatever its case, as Excel does, through
    /// the fold `FIND`, `SEARCH` and the lookups share (`TRAME-252`).
    #[test]
    fn text_compares_whatever_its_case() {
        for (src, expect) in [
            ("\"abc\"=\"ABC\"", "true"),
            ("\"abc\"<>\"ABC\"", "false"),
            ("\"a\"<\"B\"", "true"),
            ("\"B\">\"a\"", "true"),
            ("\"a\"<=\"A\"", "true"),
            ("\"A\">=\"a\"", "true"),
            ("\"\u{3C3}\"=\"\u{3C2}\"", "true"),
            ("\"a\"=\"b\"", "false"),
        ] {
            assert_eq!(
                eval_with_cells(src, 1, HashMap::default()),
                expect,
                "formula: ={src}"
            );
        }
    }

    #[test]
    fn booleans_order_false_before_true() {
        // Ordering two booleans used to be `#TYPE!`, which left `=A1<TRUE` on a
        // blank cell answering a boolean while `=FALSE<TRUE` errored.
        assert_eq!(eval_formula("FALSE<TRUE").to_display(), "true");
        assert_eq!(eval_formula("TRUE>FALSE").to_display(), "true");
        assert_eq!(eval_formula("TRUE>=TRUE").to_display(), "true");
    }

    #[test]
    fn equality_is_relative_not_absolute() {
        // An absolute `f64::EPSILON` tolerance calls every pair of tiny numbers
        // equal: 1e-16 and 2e-16 differ by less than EPSILON yet are distinct.
        assert_eq!(
            eval_formula("0.0000000000000001=0.0000000000000002").to_display(),
            "false"
        );
        // ... while remaining tolerant of the usual binary-fraction drift.
        assert_eq!(eval_formula("0.1+0.2=0.3").to_display(), "true");
    }

    #[test]
    fn call_with_more_than_255_arguments_keeps_them_all() {
        // `n_args` used to be a `u8`: the 256th argument wrapped the count to 0.
        // `SUM` was then called with no argument, and the formula silently
        // answered the last operand left on the stack instead of the total.
        let args = std::iter::repeat_n("1", 256).collect::<Vec<_>>().join(",");
        assert_eq!(eval_formula(&format!("SUM({args})")).to_display(), "256");
    }

    #[test]
    fn eval_range_sums_bound_cells_and_ignores_virtual_tail() {
        // A1=1, A2=2, A3=3; SUM(A1:A100) spans far past the used range but the
        // virtual tail reads Empty, so the result is 6.
        let mut cache = HashMap::default();
        cache.insert(cell(0, 0), CellValue::Number(1.0));
        cache.insert(cell(1, 0), CellValue::Number(2.0));
        cache.insert(cell(2, 0), CellValue::Number(3.0));
        assert_eq!(eval_with_cells("SUM(A1:A100)", 2, cache), "6");
    }

    #[test]
    fn eval_range_over_cell_cap_yields_ref_error() {
        // `A1:ZZ100000` spans 702 cols x 100000 rows (~70M cells), well past
        // MAX_RANGE_CELLS. The evaluator must refuse to materialize it and
        // collapse the range to a single #REF! value instead of allocating
        // gigabytes on a co-editor's box. A bare range surfaces the raw
        // `CellValue::Range`, so we can assert on its shape and content.
        match eval_formula("A1:ZZ100000") {
            CellValue::Range(r) => {
                assert_eq!((r.rows, r.cols), (Len::<Row>::ONE, Len::<Col>::ONE));
                assert!(matches!(
                    r.values.as_slice(),
                    [CellValue::Error(FormulaError::BrokenRef)]
                ));
            }
            other => panic!("expected a collapsed error range, got {other:?}"),
        }
    }

    fn eval_formula(src: &str) -> CellValue {
        let ast = parse_formula(src).unwrap();
        let indices = indices_with(10, 10);
        let bc = compile(&ast, &indices);
        let cache = HashMap::default();
        let mut bc_cache_mut = BytecodeCache::new();
        bc_cache_mut.insert(cell(0, 0), bc);
        let bc_ref = bc_cache_mut.get(&cell(0, 0)).unwrap();
        let mut ctx = EvalContext::new(
            &bc_cache_mut,
            &cache,
            &indices,
            Instant::<SheetDateTime>::new(0.0),
        );
        // SAFETY-free: execute borrows bc_ref immutably; ctx borrows the same
        // cache immutably. Clone the bytecode to avoid the double borrow.
        let owned = bc_ref.clone();
        ctx.execute(&owned)
    }

    /// A range over the cap gets no grant, so no buffer is sized for it, and
    /// the pass keeps its whole budget (`TRAME-209`).
    #[test]
    fn a_range_over_the_cap_is_granted_nothing() {
        let mut left = MAX_PASS_RANGE_CELLS;
        assert_eq!(
            grant_range(Len::<Row>::new(100_000), Len::<Col>::new(702), &mut left),
            None
        );
        assert_eq!(
            grant_range(
                Len::<Row>::new(u32::MAX),
                Len::<Col>::new(u32::MAX),
                &mut left
            ),
            None
        );
        assert_eq!(
            grant_range(
                Len::<Row>::new(
                    u32::try_from(MAX_RANGE_CELLS.get() + 1).expect("fits a row count")
                ),
                Len::<Col>::ONE,
                &mut left
            ),
            None
        );
        assert_eq!(left, MAX_PASS_RANGE_CELLS, "a refusal debits nothing");
        assert_eq!(
            grant_range(Len::<Row>::new(1_000), Len::<Col>::new(1_000), &mut left),
            Some(RangeGrant(MAX_RANGE_CELLS))
        );
        assert_eq!(left, MAX_PASS_RANGE_CELLS.minus(MAX_RANGE_CELLS));
        // Through the evaluator: the over-cap range leaves the budget whole.
        let ast = parse_formula("SUM(A1:ZZ100000)").unwrap();
        let indices = indices_with(10, 10);
        let bc = compile(&ast, &indices);
        let (cache, bc_cache) = (HashMap::default(), BytecodeCache::new());
        let mut ctx = EvalContext::new(
            &bc_cache,
            &cache,
            &indices,
            Instant::<SheetDateTime>::new(0.0),
        );
        assert_eq!(ctx.execute(&bc).to_display(), "#REF!");
        assert_eq!(ctx.range_cells_left, MAX_PASS_RANGE_CELLS);
    }

    /// The ranges of one pass share one budget (`TRAME-210`). Each range below
    /// is under the per-range cap. The one that overdraws the pass reads `#REF!`.
    #[test]
    fn ranges_of_one_pass_share_one_budget() {
        let ast = parse_formula("SUM(A1:J10)").unwrap();
        let indices = indices_with(10, 10);
        let bc = compile(&ast, &indices);
        let cache = HashMap::default();
        let bc_cache = BytecodeCache::new();
        let mut ctx = EvalContext::new(
            &bc_cache,
            &cache,
            &indices,
            Instant::<SheetDateTime>::new(0.0),
        );
        assert_eq!(ctx.range_cells_left, MAX_PASS_RANGE_CELLS);
        // Room for two reads of the hundred cells, not three.
        ctx.range_cells_left = Count::<Cells>::new(250);
        assert_eq!(ctx.execute(&bc).to_display(), "0");
        assert_eq!(ctx.execute(&bc).to_display(), "0");
        assert_eq!(ctx.range_cells_left, Count::<Cells>::new(50));
        assert_eq!(ctx.execute(&bc).to_display(), "#REF!");
    }

    #[test]
    fn literal_number() {
        assert_eq!(eval_formula("42").to_display(), "42");
    }

    #[test]
    fn literal_string() {
        assert_eq!(eval_formula(r#""hello""#).to_display(), "hello");
    }

    #[test]
    fn arithmetic() {
        assert_eq!(eval_formula("2+3").to_display(), "5");
        assert_eq!(eval_formula("10-3").to_display(), "7");
        assert_eq!(eval_formula("4*5").to_display(), "20");
        assert_eq!(eval_formula("10/2").to_display(), "5");
        assert_eq!(eval_formula("2^3").to_display(), "8");
    }

    #[test]
    fn division_by_zero() {
        assert!(matches!(
            eval_formula("1/0"),
            CellValue::Error(FormulaError::DivisionByZero)
        ));
    }

    /// Every operator answers `#NUM!` for a result that is not a finite
    /// number, division and negation included. `IFERROR` then catches it, and
    /// `ISERROR` sees it (`TRAME-244`).
    #[test]
    fn an_operator_whose_result_is_not_finite_answers_num() {
        let cells = || {
            HashMap::from_iter([
                (cell(0, 0), CellValue::Number(1e308)),
                (cell(1, 0), CellValue::Number(0.1)),
                (cell(2, 0), CellValue::Number(f64::INFINITY)),
            ])
        };
        for (src, reads) in [
            ("A1/A2", "#NUM!"),
            ("A3/A3", "#NUM!"),
            ("-A3", "#NUM!"),
            ("A1*10", "#NUM!"),
            ("IFERROR(A1/A2, 0)", "0"),
            ("ISERROR(A1/A2)", "true"),
        ] {
            assert_eq!(eval_with_cells(src, 1, cells()), reads, "formula: ={src}");
        }
    }

    #[test]
    fn concatenation() {
        assert_eq!(
            eval_formula(r#""hello"&" world""#).to_display(),
            "hello world"
        );
    }

    #[test]
    fn comparisons() {
        assert_eq!(eval_formula("1=1").to_display(), "true");
        assert_eq!(eval_formula("1=2").to_display(), "false");
        assert_eq!(eval_formula("1>0").to_display(), "true");
        assert_eq!(eval_formula("1<0").to_display(), "false");
    }

    #[test]
    fn negate() {
        assert_eq!(eval_formula("-5").to_display(), "-5");
        assert_eq!(eval_formula("--5").to_display(), "5");
    }

    #[test]
    fn precedence() {
        assert_eq!(eval_formula("2+3*4").to_display(), "14");
        assert_eq!(eval_formula("(2+3)*4").to_display(), "20");
    }
}
