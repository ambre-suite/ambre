//! Stack VM for evaluating formula bytecode.
//!
//! The VM executes a [`CellBytecode`] sequence on a value stack. It detects
//! dynamic cycles (not visible in the static cell_dag, e.g. cycles through
//! ranges or cross-sheet references) via an `eval_stack` and memoizes results
//! in `resolved` to avoid redundant work within a single recomputation pass.
//!
//! References are uid-anchored ([`CellId`] / [`RangeId`]). At evaluation each
//! reference is resolved against the current per-sheet [`SheetIndex`]: a bound
//! `Uid` that is missing yields `#REF!`; a virtual position beyond the used
//! range reads `Empty`. The lookup is normalized to the bound `(row_uid,
//! col_uid)` form so it matches the keys fed by the host.

use std::collections::{HashMap, HashSet};

use super::cache::BytecodeCache;
use super::compile::SheetIndices;
use super::functions::call_builtin;
use super::types::{AxisId, CellId, CellValue, FormulaError, Op, RangeId, RangeValue};

/// Upper bound on the number of cells a single range reference may materialize
/// during evaluation. Protects the WASM Logic worker from a peer-supplied
/// formula (e.g. `=SUM(A1:ZZ100000)`) whose range would allocate gigabytes.
/// 1M cells covers any realistic used area while capping the blast radius.
const MAX_RANGE_CELLS: usize = 1_000_000;

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
    pub indices: &'a SheetIndices,
    /// Evaluation clock as an Excel serial number. Shared by every cell in the
    /// pass so `TODAY()`/`NOW()` are stable within one recomputation.
    pub now_serial: f64,
    /// 1-based grid position of the cell currently being evaluated, used by the
    /// no-argument `ROW()`/`COLUMN()`. `(0, 0)` means "unknown" (e.g. a bare
    /// `execute` call in tests). Set by [`EvalContext::eval_cell`].
    pub current_pos: (u32, u32),
}

/// Resolving one axis of a reference to a grid position.
enum AxisPos {
    /// A concrete grid position (may be beyond the materialized range).
    Pos(usize),
    /// A bound `Uid` that is no longer in the index -> `#REF!`.
    Broken,
}

impl<'a> EvalContext<'a> {
    pub fn new(
        bc: &'a BytecodeCache,
        formula_cache: &'a HashMap<CellId, CellValue>,
        indices: &'a SheetIndices,
    ) -> Self {
        Self {
            eval_stack: HashSet::new(),
            resolved: HashMap::new(),
            bc,
            formula_cache,
            indices,
            now_serial: 0.0,
            current_pos: (0, 0),
        }
    }

    /// Resolve the 1-based grid position of a cell against its sheet index.
    /// A bound id that has been deleted, or a missing sheet, yields `(0, 0)`.
    fn grid_pos_of(&self, cell: CellId) -> (u32, u32) {
        let Some(idx) = self.indices.get(&cell.sheet) else {
            return (0, 0);
        };
        let axis = |a: AxisId, pos_of: &dyn Fn(&crate::model::Uid) -> Option<usize>| match a {
            AxisId::Bound(u) => pos_of(&u).map(|p| p as u32 + 1).unwrap_or(0),
            AxisId::Virtual(p) => p + 1,
        };
        (
            axis(cell.row, &|u| idx.row_pos(u)),
            axis(cell.col, &|u| idx.col_pos(u)),
        )
    }

    /// Set the evaluation clock (see [`EvalContext::now_serial`]).
    pub fn with_now(mut self, now_serial: f64) -> Self {
        self.now_serial = now_serial;
        self
    }

    /// Evaluate a single cell (already in canonical bound form), detecting
    /// dynamic cycles and caching results.
    pub fn eval_cell(&mut self, cell: CellId) -> CellValue {
        if self.eval_stack.contains(&cell) {
            return CellValue::Error(FormulaError::CircularReference);
        }
        if let Some(v) = self.resolved.get(&cell) {
            return v.clone();
        }
        self.eval_stack.insert(cell);
        let prev_pos = self.current_pos;
        self.current_pos = self.grid_pos_of(cell);
        let result = match self.bc.get(&cell) {
            Some(bc) => self.execute(bc),
            None => self
                .formula_cache
                .get(&cell)
                .cloned()
                .unwrap_or(CellValue::Empty),
        };
        self.eval_stack.remove(&cell);
        self.current_pos = prev_pos;
        self.resolved.insert(cell, result.clone());
        result
    }

    /// Resolve one axis of a reference to a grid position against `idx`.
    fn axis_pos(axis: AxisId, pos_of: impl Fn(&crate::model::Uid) -> Option<usize>) -> AxisPos {
        match axis {
            AxisId::Bound(u) => match pos_of(&u) {
                Some(p) => AxisPos::Pos(p),
                None => AxisPos::Broken,
            },
            AxisId::Virtual(p) => AxisPos::Pos(p as usize),
        }
    }

    /// Resolve a [`CellId`] to the value it points at: `#REF!` for a deleted
    /// bound id, `Empty` for a virtual position beyond the used range, otherwise
    /// the evaluated value of the canonical bound cell at that position.
    fn read_cell(&mut self, r: CellId) -> CellValue {
        let Some(idx) = self.indices.get(&r.sheet) else {
            return CellValue::Error(FormulaError::BrokenRef);
        };
        let rp = match Self::axis_pos(r.row, |u| idx.row_pos(u)) {
            AxisPos::Pos(p) => p,
            AxisPos::Broken => return CellValue::Error(FormulaError::BrokenRef),
        };
        let cp = match Self::axis_pos(r.col, |u| idx.col_pos(u)) {
            AxisPos::Pos(p) => p,
            AxisPos::Broken => return CellValue::Error(FormulaError::BrokenRef),
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
                Op::PushStr(s) => stack.push(CellValue::Text(s.clone())),
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
                        (Ok(_), Ok(0.0)) => CellValue::Error(FormulaError::DivisionByZero),
                        (Ok(x), Ok(y)) => CellValue::Number(x / y),
                        _ => CellValue::Error(FormulaError::TypeError),
                    });
                }
                Op::Pow => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match (a.as_number(), b.as_number()) {
                        // Guard NaN/inf (e.g. `(-1)^0.5`, `0^-1`): a bare
                        // `powf` result would propagate silently through later
                        // arithmetic. Excel surfaces these as `#NUM!`.
                        (Ok(x), Ok(y)) => match x.powf(y) {
                            r if r.is_finite() => CellValue::Number(r),
                            _ => CellValue::Error(FormulaError::NumError),
                        },
                        _ => CellValue::Error(FormulaError::TypeError),
                    });
                }
                Op::Eq => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(CellValue::Boolean(values_equal(&a, &b)));
                }
                Op::Ne => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(CellValue::Boolean(!values_equal(&a, &b)));
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
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
                    let b = stack.pop().unwrap_or(CellValue::Empty).to_display();
                    let a = stack.pop().unwrap_or(CellValue::Empty).to_display();
                    stack.push(CellValue::Text(a + &b));
                }
                Op::Neg => {
                    let v = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match v.as_number() {
                        Ok(n) => CellValue::Number(-n),
                        Err(e) => CellValue::Error(e),
                    });
                }
                Op::Not => {
                    let v = stack.pop().unwrap_or(CellValue::Empty);
                    let truthy = match &v {
                        CellValue::Boolean(false) => false,
                        CellValue::Number(n) if *n == 0.0 => false,
                        CellValue::Text(s) if s.is_empty() => false,
                        CellValue::Empty => false,
                        CellValue::Error(_) => false,
                        _ => true,
                    };
                    stack.push(CellValue::Boolean(!truthy));
                }
                Op::CallFn { name, n_args } => {
                    let split = stack.len().saturating_sub(*n_args as usize);
                    let args: Vec<CellValue> = stack.drain(split..).collect();
                    let fnctx = super::functions::FnCtx {
                        now_serial: self.now_serial,
                        cell_row: self.current_pos.0,
                        cell_col: self.current_pos.1,
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
            return RangeValue::new(1, 1, vec![CellValue::Error(FormulaError::BrokenRef)]);
        };
        let row_len = idx.row_len();
        let col_len = idx.col_len();
        let (rs, re) = match dep.row_range(idx) {
            Some(r) => r,
            None => return RangeValue::new(1, 1, vec![CellValue::Error(FormulaError::BrokenRef)]),
        };
        let (cs, ce) = match dep.col_range(idx) {
            Some(r) => r,
            None => return RangeValue::new(1, 1, vec![CellValue::Error(FormulaError::BrokenRef)]),
        };
        if re < rs || ce < cs {
            return RangeValue::new(0, 0, Vec::new());
        }
        let rows = re - rs + 1;
        let cols = ce - cs + 1;
        // Bound the materialized range. A compiled range stores only its two
        // endpoints (O(1) to register), but evaluating it allocates one
        // `CellValue` per member cell. With remote apply live, a malicious peer
        // can push `=SUM(A1:ZZ100000)` and force every collaborator to allocate
        // ~70M values on each recompute. Cap the cell count and surface `#REF!`
        // rather than let a single formula OOM the WASM Logic worker. The
        // ceiling is generous relative to any real spreadsheet's used area.
        if rows.saturating_mul(cols) > MAX_RANGE_CELLS {
            return RangeValue::new(1, 1, vec![CellValue::Error(FormulaError::BrokenRef)]);
        }
        let mut values = Vec::with_capacity(rows * cols);
        for rp in rs..=re {
            for cp in cs..=ce {
                let v = match (rp < row_len, cp < col_len) {
                    (true, true) => {
                        let row = idx.row_id(rp).expect("rp < row_len");
                        let col = idx.col_id(cp).expect("cp < col_len");
                        self.eval_cell(CellId::bound(dep.sheet, row, col))
                    }
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
            (Ok(x), Ok(y)) => {
                let result = f(x, y);
                if result.is_finite() {
                    stack.push(CellValue::Number(result));
                } else {
                    stack.push(CellValue::Error(FormulaError::NumError));
                }
            }
            (Err(e), _) | (_, Err(e)) => stack.push(CellValue::Error(e)),
        }
    }
}

// -----------------------------------------------------------------------------
// Value comparison helpers
// -----------------------------------------------------------------------------

fn values_equal(a: &CellValue, b: &CellValue) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => (x - y).abs() < f64::EPSILON,
        (CellValue::Text(x), CellValue::Text(y)) => x == y,
        (CellValue::Boolean(x), CellValue::Boolean(y)) => x == y,
        (CellValue::Empty, CellValue::Empty) => true,
        (CellValue::Empty, CellValue::Text(s)) | (CellValue::Text(s), CellValue::Empty) => {
            s.is_empty()
        }
        _ => false,
    }
}

fn compare_values(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => x.partial_cmp(y),
        (CellValue::Text(x), CellValue::Text(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::compile;
    use super::super::parser::parse_formula;
    use super::super::test_index::{index_with, indices_with};
    use super::super::types::CellValue;
    use super::*;
    use crate::model::Uid;

    fn cell(row: u32, col: u32) -> CellId {
        CellId::bound(
            0,
            Uid {
                peer: 1,
                counter: row,
            },
            Uid {
                peer: 2,
                counter: col,
            },
        )
    }

    /// Compile `src` for the cell at `(0, fcol)` over a 10x10 sheet, evaluate it
    /// against `cache` (plain cell values keyed by their bound `CellId`), and
    /// return the result text. Exercises `PushCell`/`EvalRange` resolution.
    fn eval_with_cells(src: &str, fcol: u32, cache: HashMap<CellId, CellValue>) -> String {
        let indices = indices_with(10, 10);
        let bc = compile(&parse_formula(src).unwrap(), &indices);
        let mut bcc = BytecodeCache::new();
        bcc.insert(cell(0, fcol), bc);
        let mut ctx = EvalContext::new(&bcc, &cache, &indices);
        ctx.eval_cell(cell(0, fcol)).to_display()
    }

    #[test]
    fn push_cell_reads_bound_value() {
        let mut cache = HashMap::new();
        cache.insert(cell(0, 0), CellValue::Number(7.0)); // A1 = 7
        assert_eq!(eval_with_cells("A1*2", 1, cache), "14");
    }

    #[test]
    fn push_cell_virtual_beyond_used_range_reads_empty() {
        // Column Z (25) is past the 10 materialized columns -> virtual -> Empty,
        // so the sum is just the literal addend.
        assert_eq!(eval_with_cells("Z1+5", 1, HashMap::new()), "5");
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
        idx.apply_delete_row(0); // removes row_uid(0)
        let mut indices = super::super::compile::SheetIndices::new();
        indices.insert(0, idx);

        let cache = HashMap::new();
        let mut ctx = EvalContext::new(&bcc, &cache, &indices);
        assert!(matches!(
            ctx.eval_cell(cell(0, 1)),
            CellValue::Error(FormulaError::BrokenRef)
        ));
    }

    #[test]
    fn eval_range_sums_bound_cells_and_ignores_virtual_tail() {
        // A1=1, A2=2, A3=3; SUM(A1:A100) spans far past the used range but the
        // virtual tail reads Empty, so the result is 6.
        let mut cache = HashMap::new();
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
                assert_eq!((r.rows, r.cols), (1, 1));
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
        let cache = HashMap::new();
        let mut bc_cache_mut = BytecodeCache::new();
        bc_cache_mut.insert(cell(0, 0), bc);
        let bc_ref = bc_cache_mut.get(&cell(0, 0)).unwrap();
        let mut ctx = EvalContext::new(&bc_cache_mut, &cache, &indices);
        // SAFETY-free: execute borrows bc_ref immutably; ctx borrows the same
        // cache immutably. Clone the bytecode to avoid the double borrow.
        let owned = bc_ref.clone();
        ctx.execute(&owned)
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
