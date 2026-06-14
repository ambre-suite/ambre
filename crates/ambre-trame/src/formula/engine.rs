//! Formula engine -- orchestrates parsing, compilation, dependency tracking,
//! and evaluation for all formula cells in a Trame document.
//!
//! Owns the `DependencyGraph` (petgraph), `BytecodeCache`, formula result cache,
//! and the `dirty` set. `compute_all_dirty()` is the single entry point for
//! recomputation: topological sort on the cell DAG -> recompile stale bytecode
//! -> evaluate via the stack VM. All state is keyed by the uid-anchored
//! [`CellId`]; the host passes the current per-sheet indices on every call so
//! references resolve against live positions.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use petgraph::visit::Topo;

use crate::model::SheetIndex;

use super::cache::BytecodeCache;
use super::compile::SheetIndices;
use super::dag::DependencyGraph;
use super::types::{CellId, CellValue, FormulaCtx};
use super::vm::EvalContext;

/// Central orchestrator for the Trame formula engine.
pub struct FormulaEngine {
    /// Two-level dependency graph (cell-to-cell + ranges).
    pub deps: DependencyGraph,
    /// Compiled bytecode cache with incremental recompilation.
    pub bytecode: BytecodeCache,
    /// Persisted evaluation results keyed by `CellId`.
    pub formula_cache: RefCell<HashMap<CellId, CellValue>>,
    /// Cells that need re-evaluation (directly edited or downstream of an edit).
    pub dirty: HashSet<CellId>,
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

    /// Discard all formula state. Used before a full reseed.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Notify the engine that `cell`'s raw text changed.
    pub fn on_cell_changed(
        &mut self,
        cell: CellId,
        raw: &str,
        indices: &SheetIndices,
        ctx: &dyn FormulaCtx,
    ) {
        self.bytecode.invalidate(cell);
        let empty = SheetIndex::default();
        let idx = indices.get(&cell.sheet).unwrap_or(&empty);
        let downstream = self.deps.collect_dirty(cell, idx);
        self.dirty.insert(cell);
        self.dirty.extend(downstream);

        let _ = self
            .bytecode
            .recompile_if_stale(cell, raw, indices, &mut self.deps, ctx);

        // Plain (non-formula) cells never go through the VM, so their value must
        // be published here for formula references to resolve.
        if !raw.starts_with('=') {
            let mut cache = self.formula_cache.borrow_mut();
            if raw.is_empty() {
                cache.remove(&cell);
            } else {
                cache.insert(cell, literal_value(raw));
            }
        }
    }

    /// Mark every formula cell and every cached plain cell dirty. Used after a
    /// structural reorder (sort) that keeps the uid-anchored bytecode intact: the
    /// references resolve to fresh positions, so a recompute makes results follow
    /// the moved data without re-parsing any cell text.
    pub fn mark_all_dirty(&mut self) {
        let cells: Vec<CellId> = self
            .bytecode
            .cells()
            .copied()
            .chain(self.formula_cache.borrow().keys().copied())
            .collect();
        self.dirty.extend(cells);
    }

    /// Recompute every cell in the `dirty` set in topological order.
    pub fn compute_all_dirty(&mut self, indices: &SheetIndices) {
        if self.dirty.is_empty() {
            return;
        }

        let mut results: Vec<(CellId, CellValue)> = Vec::new();

        {
            let cache_ref = self.formula_cache.borrow();
            let mut ctx = EvalContext::new(&self.bytecode, &cache_ref, indices)
                .with_now(super::functions::now_serial());

            let mut topo = Topo::new(&self.deps.cell_dag);
            while let Some(node) = topo.next(&self.deps.cell_dag) {
                let cell = self.deps.cell_dag[node];
                if !self.dirty.remove(&cell) {
                    continue;
                }
                let value = ctx.eval_cell(cell);
                results.push((cell, value));
            }
        }

        let mut cache = self.formula_cache.borrow_mut();
        for (cell, value) in results {
            if self.bytecode.get(&cell).is_some() {
                cache.insert(cell, value);
            }
        }

        for cell in self.dirty.drain() {
            if let Some(value) = cache.get(&cell).cloned() {
                cache.insert(cell, value);
            }
        }
    }

    /// Get the display text for a cell, or `None` if it has no cached result.
    pub fn evaluated_text(&self, cell: CellId) -> Option<String> {
        self.formula_cache.borrow().get(&cell).map(|v| v.to_display())
    }

    /// Get the evaluated [`CellValue`] for a cell (formula result or parsed
    /// literal), or `None` for a virtual/empty cell.
    pub fn evaluated_value(&self, cell: CellId) -> Option<CellValue> {
        self.formula_cache.borrow().get(&cell).cloned()
    }

    /// Whether `cell` has a formula (as opposed to plain text).
    pub fn is_formula_cell(&self, cell: CellId) -> bool {
        self.bytecode.get(&cell).is_some()
    }

    /// Number of cached formula cells.
    pub fn formula_count(&self) -> usize {
        self.bytecode.len()
    }
}

/// Parse a plain cell's raw text into a literal [`CellValue`].
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
    use super::super::compile::SheetIndices;
    use super::super::test_index::{index_with, indices_with};
    use super::super::types::SingleSheetCtx;
    use crate::model::Uid;

    /// Bound cell at grid `(row, col)` on sheet 0.
    fn cell(row: u32, col: u32) -> CellId {
        CellId::bound(0, Uid { peer: 1, counter: row }, Uid { peer: 2, counter: col })
    }

    #[test]
    fn literal_formula_evaluated() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        let c = cell(0, 0);

        engine.on_cell_changed(c, "=42", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(c).unwrap(), "42");
        assert!(engine.is_formula_cell(c));
    }

    #[test]
    fn arithmetic_chain() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), "=10", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), "=A1*2", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "20");
    }

    #[test]
    fn cascade_after_edit() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), "=5", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), "=A1+3", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "8");

        engine.dirty.clear();
        engine.on_cell_changed(cell(0, 0), "=10", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "10");
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "13");
    }

    #[test]
    fn division_by_zero_displayed() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        let c = cell(0, 0);

        engine.on_cell_changed(c, "=1/0", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(c).unwrap(), "#DIV/0!");
    }

    #[test]
    fn sum_range() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        for i in 0..3 {
            engine.on_cell_changed(cell(i, 0), &format!("={}", i + 1), &idx, &SingleSheetCtx);
        }
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 2), "=SUM(A1:A3)", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "6");
    }

    #[test]
    fn sum_over_plain_literal_cells() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), "1", &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), "2", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 2), "=SUM(A1:B1)", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "3");
    }

    #[test]
    fn static_cycle_rejected() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), "=B1", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), "=A1", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert!(engine.evaluated_text(cell(0, 1)).is_none());
    }

    #[test]
    fn function_if() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), "=IF(1>0, \"yes\", \"no\")", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "yes");
    }

    #[test]
    fn ref_follows_data_through_index_remap() {
        // A1 = 10 (row uid 0), A2 = 20 (row uid 1), B1 = "=A1" binds to row uid 0.
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        engine.on_cell_changed(cell(0, 0), "=10", &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(1, 0), "=20", &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), "=A1", &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "10");

        // Swap rows 0 and 1 (a sort-style permutation) and recompute WITHOUT
        // re-parsing: the uid-anchored bytecode is preserved. B1 is bound to row
        // uid 0, which is now at position 1, so it keeps reading that row's A
        // value (10) -- it follows the data, it is NOT the positional 20 that a
        // re-parse against the new order would produce.
        let mut moved = index_with(10, 10);
        moved.apply_move_row(0, 1);
        let mut indices = SheetIndices::new();
        indices.insert(0, moved);
        engine.mark_all_dirty();
        engine.compute_all_dirty(&indices);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "10", "ref follows the data, not the position");
    }
}
