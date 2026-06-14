//! Formula engine — orchestrates parsing, compilation, dependency tracking,
//! and evaluation for all formula cells in a Trame document.
//!
//! Owns the `DependencyGraph` (petgraph), `BytecodeCache`, formula result cache,
//! and the `dirty` set. `compute_all_dirty()` is the single entry point for
//! recomputation: topological sort on the cell DAG → recompile stale bytecode
//! → evaluate via the stack VM.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use petgraph::visit::Topo;

use super::cache::BytecodeCache;
use super::dag::DependencyGraph;
use super::types::{CellRef, CellValue, FormulaCtx};
use super::vm::EvalContext;

/// Central orchestrator for the Trame formula engine.
///
/// All mutable state for formula computation lives here, separate from the
/// CRDT (`LoroDoc`) and the UI-facing `TrameState`. Each `TrameState` holds
/// one `FormulaEngine`.
pub struct FormulaEngine {
    /// Two-level dependency graph (cell-to-cell + ranges).
    pub deps: DependencyGraph,
    /// Compiled bytecode cache with incremental recompilation.
    pub bytecode: BytecodeCache,
    /// Persisted evaluation results keyed by `CellRef`. The UI reads from
    /// here for display; recomputation updates entries for dirty cells.
    /// Uses `RefCell` for interior mutability so `EvalContext` can hold an
    /// immutable borrow while `compute_all_dirty` inserts results.
    pub formula_cache: RefCell<HashMap<CellRef, CellValue>>,
    /// Cells that need re-evaluation (directly edited or downstream of an edit).
    pub dirty: HashSet<CellRef>,
}

impl FormulaEngine {
    pub fn new() -> Self {
        Self {
            deps: DependencyGraph::new(),
            bytecode: BytecodeCache::new(),
            formula_cache: RefCell::new(HashMap::new()),
            dirty: HashSet::new(),
        }
    }

    /// Discard all formula state (dependency graph, bytecode, results, dirty
    /// set). Used before a full reseed after a structural op (insert/delete
    /// row/column) shifts every cell's grid position: the engine is positional,
    /// so stale entries at old positions must be cleared before re-feeding cells
    /// at their new positions.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Notify the engine that `cell`'s raw text changed.
    ///
    /// Invalidates its bytecode, marks it stale, collects all downstream
    /// dependents, and extends the `dirty` set.
    pub fn on_cell_changed(&mut self, cell: CellRef, raw: &str, ctx: &dyn FormulaCtx) {
        self.bytecode.invalidate(cell);
        let downstream = self.deps.collect_dirty(cell);
        self.dirty.insert(cell);
        self.dirty.extend(downstream);

        // Recompile immediately to detect parse/cycle errors early; the
        // result (error or success) is stored in formula_cache by
        // compute_all_dirty. `ctx` resolves sheet-qualified refs / named ranges.
        let _ = self.bytecode.recompile_if_stale(cell, raw, &mut self.deps, ctx);

        // Plain (non-formula) cells never go through the VM, so their value
        // must be published here for formula references (e.g. SUM(A1:B1)) to
        // resolve. Formula cells get their value from `compute_all_dirty`.
        if !raw.starts_with('=') {
            let mut cache = self.formula_cache.borrow_mut();
            if raw.is_empty() {
                cache.remove(&cell);
            } else {
                cache.insert(cell, literal_value(raw));
            }
        }
    }

    /// Recompute every cell in the `dirty` set in topological order.
    ///
    /// Cells are evaluated via the stack VM. Results are written to
    /// `formula_cache`. This function is synchronous and runs inside the
    /// Logic thread event handler.
    pub fn compute_all_dirty(&mut self) {
        if self.dirty.is_empty() {
            return;
        }

        // Collect evaluated results in a temporary buffer so we can drop the
        // immutable borrow on formula_cache before writing back.
        let mut results: Vec<(CellRef, CellValue)> = Vec::new();

        {
            let cache_ref = self.formula_cache.borrow();
            let mut ctx =
                EvalContext::new(&self.bytecode, &cache_ref).with_now(super::functions::now_serial());

            let mut topo = Topo::new(&self.deps.cell_dag);
            while let Some(node) = topo.next(&self.deps.cell_dag) {
                let cell = self.deps.cell_dag[node];
                if !self.dirty.remove(&cell) {
                    continue;
                }
                let value = ctx.eval_cell(cell);
                results.push((cell, value));
            }
        } // cache_ref and ctx dropped here.

        // Flush results back to formula_cache. Only cache cells that have
        // valid bytecode; cells whose formula was rejected (cycle, parse error)
        // have no bytecode and should not appear in the cache.
        let mut cache = self.formula_cache.borrow_mut();
        for (cell, value) in results {
            if self.bytecode.get(&cell).is_some() {
                cache.insert(cell, value);
            }
        }

        // Any remaining dirty cells not in the DAG.
        for cell in self.dirty.drain() {
            if let Some(value) = cache.get(&cell).cloned() {
                cache.insert(cell, value);
            }
        }
    }

    /// Get the display text for a cell: its evaluated formula result if it
    /// has a cached entry, otherwise `None` (caller falls back to raw text).
    pub fn evaluated_text(&self, cell: CellRef) -> Option<String> {
        self.formula_cache.borrow().get(&cell).map(|v| v.to_display())
    }

    /// Whether `cell` has a formula (as opposed to plain text).
    pub fn is_formula_cell(&self, cell: CellRef) -> bool {
        self.bytecode.get(&cell).is_some()
    }

    /// Number of cached formula cells.
    pub fn formula_count(&self) -> usize {
        self.bytecode.len()
    }
}

/// Parse a plain cell's raw text into a literal [`CellValue`]: a number when it
/// parses as one, otherwise text. Empty strings are handled by the caller.
fn literal_value(raw: &str) -> CellValue {
    match raw.parse::<f64>() {
        Ok(n) => CellValue::Number(n),
        Err(_) => CellValue::Text(raw.to_string()),
    }
}

impl Default for FormulaEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::SingleSheetCtx;

    #[test]
    fn literal_formula_evaluated() {
        let mut engine = FormulaEngine::new();
        let cell = CellRef::new(0, 0);

        engine.on_cell_changed(cell, "=42", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(cell).unwrap(), "42");
        assert!(engine.is_formula_cell(cell));
    }

    #[test]
    fn arithmetic_chain() {
        let mut engine = FormulaEngine::new();

        // A1 = 10
        engine.on_cell_changed(CellRef::new(0, 0), "=10", &SingleSheetCtx);
        engine.compute_all_dirty();
        engine.dirty.clear();

        // B1 = A1 * 2
        engine.on_cell_changed(CellRef::new(0, 1), "=A1*2", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 1)).unwrap(), "20");
    }

    #[test]
    fn cascade_after_edit() {
        let mut engine = FormulaEngine::new();

        // A1 = 5
        engine.on_cell_changed(CellRef::new(0, 0), "=5", &SingleSheetCtx);
        engine.compute_all_dirty();
        engine.dirty.clear();

        // B1 = A1 + 3
        engine.on_cell_changed(CellRef::new(0, 1), "=A1+3", &SingleSheetCtx);
        engine.compute_all_dirty();
        assert_eq!(engine.evaluated_text(CellRef::new(0, 1)).unwrap(), "8");

        // Edit A1 to 10: B1 should cascade.
        engine.dirty.clear();
        engine.on_cell_changed(CellRef::new(0, 0), "=10", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 0)).unwrap(), "10");
        assert_eq!(engine.evaluated_text(CellRef::new(0, 1)).unwrap(), "13");
    }

    #[test]
    fn division_by_zero_displayed() {
        let mut engine = FormulaEngine::new();
        let cell = CellRef::new(0, 0);

        engine.on_cell_changed(cell, "=1/0", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(cell).unwrap(), "#DIV/0!");
    }

    #[test]
    fn sum_range() {
        let mut engine = FormulaEngine::new();

        // A1 = 1, A2 = 2, A3 = 3
        for i in 0..3 {
            let cell = CellRef::new(i, 0);
            engine.on_cell_changed(cell, &format!("={}", i + 1), &SingleSheetCtx);
        }
        engine.compute_all_dirty();
        engine.dirty.clear();

        // C1 = SUM(A1:A3)
        engine.on_cell_changed(CellRef::new(0, 2), "=SUM(A1:A3)", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 2)).unwrap(), "6");
    }

    #[test]
    fn sum_over_plain_literal_cells() {
        let mut engine = FormulaEngine::new();

        // A1 = 1, B1 = 2 entered as plain literals (no leading '=').
        engine.on_cell_changed(CellRef::new(0, 0), "1", &SingleSheetCtx);
        engine.on_cell_changed(CellRef::new(0, 1), "2", &SingleSheetCtx);
        engine.compute_all_dirty();
        engine.dirty.clear();

        // C1 = SUM(A1:B1) must see the plain literals, not Empty.
        engine.on_cell_changed(CellRef::new(0, 2), "=SUM(A1:B1)", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 2)).unwrap(), "3");
    }

    #[test]
    fn static_cycle_rejected() {
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(CellRef::new(0, 0), "=B1", &SingleSheetCtx);
        engine.compute_all_dirty();
        engine.dirty.clear();

        engine.on_cell_changed(CellRef::new(0, 1), "=A1", &SingleSheetCtx);
        engine.compute_all_dirty();

        // B1's parse/compile should have failed (cycle), so no cached result.
        assert!(engine.evaluated_text(CellRef::new(0, 1)).is_none());
    }

    #[test]
    fn function_if() {
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(CellRef::new(0, 0), "=IF(1>0, \"yes\", \"no\")", &SingleSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 0)).unwrap(), "yes");
    }

    /// Context mapping `Sheet2` to local index 1 (other names/sheets absent).
    struct TwoSheetCtx;
    impl FormulaCtx for TwoSheetCtx {
        fn resolve_sheet(&self, name: &str) -> Option<u16> {
            (name == "Sheet2").then_some(1)
        }
        fn resolve_sheet_id(&self, id: &str) -> Option<u16> {
            (id == "s1").then_some(1)
        }
        fn resolve_name(&self, _: &str) -> Option<super::super::types::NameTarget> {
            None
        }
    }

    #[test]
    fn cross_sheet_reference_evaluates() {
        let mut engine = FormulaEngine::new();
        // Sheet2!A1 = 7 (sheet index 1).
        engine.on_cell_changed(CellRef::with_sheet(1, 0, 0), "7", &TwoSheetCtx);
        // Sheet1!A1 = Sheet2!A1 + 1 (default sheet 0).
        engine.on_cell_changed(CellRef::new(0, 0), "=Sheet2!A1+1", &TwoSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 0)).unwrap(), "8");
    }

    #[test]
    fn cross_sheet_range_sum() {
        let mut engine = FormulaEngine::new();
        // Sheet2!A1=1, A2=2, A3=3.
        for r in 0..3 {
            engine.on_cell_changed(
                CellRef::with_sheet(1, r, 0),
                &format!("{}", r + 1),
                &TwoSheetCtx,
            );
        }
        engine.on_cell_changed(CellRef::new(0, 0), "=SUM(Sheet2!A1:A3)", &TwoSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 0)).unwrap(), "6");
    }

    #[test]
    fn same_position_different_sheets_are_distinct() {
        let mut engine = FormulaEngine::new();
        engine.on_cell_changed(CellRef::new(0, 0), "10", &TwoSheetCtx);
        engine.on_cell_changed(CellRef::with_sheet(1, 0, 0), "99", &TwoSheetCtx);
        engine.compute_all_dirty();

        assert_eq!(engine.evaluated_text(CellRef::new(0, 0)).unwrap(), "10");
        assert_eq!(engine.evaluated_text(CellRef::with_sheet(1, 0, 0)).unwrap(), "99");
    }
}
