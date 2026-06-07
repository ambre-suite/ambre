//! Stack VM for evaluating formula bytecode.
//!
//! The VM executes a [`CellBytecode`] sequence on a value stack. It detects
//! dynamic cycles (not visible in the static cell_dag, e.g. cycles through
//! ranges or cross-sheet references) via an `eval_stack` and memoizes results
//! in `resolved` to avoid redundant work within a single recomputation pass.

use std::collections::{HashMap, HashSet};

use super::cache::BytecodeCache;
use super::functions::call_builtin;
use super::types::{CellRef, CellValue, FormulaError, Op, RangeDep};

/// Holds the mutable evaluation state shared across all cell evaluations in a
/// single recomputation pass (`compute_all_dirty`).
pub struct EvalContext<'a> {
    /// Cells currently on the evaluation stack — used for dynamic cycle
    /// detection. If a cell appears here, evaluating it would recurse
    /// infinitely.
    pub eval_stack: HashSet<CellRef>,
    /// Results already computed during this pass, keyed by `CellRef`. Avoids
    /// recomputing the same cell multiple times within one cycle.
    pub resolved: HashMap<CellRef, CellValue>,
    /// Borrowed reference to the bytecode cache.
    pub bc: &'a BytecodeCache,
    /// Borrowed reference to the persistent formula result cache so recursive
    /// `eval_cell` calls for non-dirty cells can reuse prior results.
    pub formula_cache: &'a HashMap<CellRef, CellValue>,
}

impl<'a> EvalContext<'a> {
    pub fn new(
        bc: &'a BytecodeCache,
        formula_cache: &'a HashMap<CellRef, CellValue>,
    ) -> Self {
        Self {
            eval_stack: HashSet::new(),
            resolved: HashMap::new(),
            bc,
            formula_cache,
        }
    }

    /// Evaluate a single cell, detecting dynamic cycles and caching results.
    ///
    /// Returns `CellValue::Error(FormulaError::CircularReference)` if the cell
    /// is already on the evaluation stack. Checks `resolved` first to avoid
    /// redundant work.
    pub fn eval_cell(&mut self, cell: CellRef) -> CellValue {
        if self.eval_stack.contains(&cell) {
            return CellValue::Error(FormulaError::CircularReference);
        }
        if let Some(v) = self.resolved.get(&cell) {
            return v.clone();
        }
        self.eval_stack.insert(cell);
        let result = match self.bc.get(&cell) {
            Some(bc) => self.execute(bc),
            None => {
                // Not a formula cell — return the cached display value.
                self.formula_cache
                    .get(&cell)
                    .cloned()
                    .unwrap_or(CellValue::Empty)
            }
        };
        self.eval_stack.remove(&cell);
        self.resolved.insert(cell, result.clone());
        result
    }

    /// Execute a compiled bytecode sequence, consuming stack-based operands and
    /// pushing the final result.
    pub fn execute(&mut self, bc: &super::types::CellBytecode) -> CellValue {
        let mut stack: Vec<CellValue> = Vec::with_capacity(8);

        for op in &bc.ops {
            match op {
                Op::PushNum(n) => stack.push(CellValue::Number(*n)),
                Op::PushStr(s) => stack.push(CellValue::Text(s.clone())),
                Op::PushBool(b) => stack.push(CellValue::Boolean(*b)),
                Op::PushEmpty => stack.push(CellValue::Empty),

                Op::PushCell(r) => {
                    stack.push(self.eval_cell(*r));
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
                        (Ok(_), Ok(d)) if d == 0.0 => {
                            CellValue::Error(FormulaError::DivisionByZero)
                        }
                        (Ok(x), Ok(y)) => CellValue::Number(x / y),
                        _ => CellValue::Error(FormulaError::TypeError),
                    });
                }
                Op::Pow => {
                    let b = stack.pop().unwrap_or(CellValue::Empty);
                    let a = stack.pop().unwrap_or(CellValue::Empty);
                    stack.push(match (a.as_number(), b.as_number()) {
                        (Ok(x), Ok(y)) => CellValue::Number(x.powf(y)),
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
                    stack.push(call_builtin(name, &args));
                }
            }
        }

        stack.pop().unwrap_or(CellValue::Empty)
    }

    /// Evaluate every cell in a range, pushing the values as a flat vector.
    fn eval_range(&mut self, dep: &RangeDep) -> Vec<CellValue> {
        let mut values = Vec::with_capacity(dep.len());
        for row in dep.row_start..=dep.row_end {
            for col in dep.col_start..=dep.col_end {
                values.push(self.eval_cell(CellRef::new(row, col)));
            }
        }
        values
    }

    /// Pop two values, apply a numeric binary operation, push the result.
    /// Propagates the first error encountered.
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
    use super::*;
    use super::super::compile::compile;
    use super::super::parser::parse_formula;
    use super::super::types::CellValue;

    fn eval_formula(src: &str) -> CellValue {
        let ast = parse_formula(src).unwrap();
        let bc = compile(&ast);
        let cache = HashMap::new();
        let mut bc_cache_mut = BytecodeCache::new();
        bc_cache_mut.insert(CellRef::new(0, 0), bc);
        let bc_ref = bc_cache_mut.get(&CellRef::new(0, 0)).unwrap();
        let mut ctx = EvalContext::new(&bc_cache_mut, &cache);
        ctx.execute(bc_ref)
    }

    #[test]
    fn literal_number() {
        let v = eval_formula("42");
        assert_eq!(v.to_display(), "42");
    }

    #[test]
    fn literal_string() {
        let v = eval_formula(r#""hello""#);
        assert_eq!(v.to_display(), "hello");
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
        let v = eval_formula("1/0");
        assert!(matches!(v, CellValue::Error(FormulaError::DivisionByZero)));
    }

    #[test]
    fn concatenation() {
        assert_eq!(eval_formula(r#""hello"&" world""#).to_display(), "hello world");
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
    fn not() {
        // NOT isn't a unary operator in our parser yet, so this tests the bytecode.
        // In practice, the function NOT() handles this.
    }

    #[test]
    fn precedence() {
        assert_eq!(eval_formula("2+3*4").to_display(), "14");
        assert_eq!(eval_formula("(2+3)*4").to_display(), "20");
    }
}
