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

use super::cache::BytecodeCache;
use super::compile::SheetIndices;
use super::dag::DependencyGraph;
use super::text::StoredFormula;
use super::types::{CellId, CellValue, FormulaCtx};
use super::vm::EvalContext;
use ambre_bus::DateSerial;

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

    /// Drop all formula state. Used before a full reseed.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Notify the engine that `cell`'s raw text changed.
    pub fn on_cell_changed(
        &mut self,
        cell: CellId,
        raw: StoredFormula,
        indices: &SheetIndices,
        ctx: &dyn FormulaCtx,
    ) {
        // The same text compiles to the same bytecode and computes the same
        // value: nothing downstream changes either (`TRAME-212`).
        if self.bytecode.compiled_from(cell, &raw) {
            return;
        }
        self.bytecode.invalidate(cell);
        let downstream = self.deps.collect_dirty(cell, indices);
        self.dirty.insert(cell);
        self.dirty.extend(downstream);

        // Plain (non-formula) cells never go through the VM, so their value must
        // be published here for formula references to resolve. It is read
        // before the cache takes the text.
        let plain =
            (!raw.is_formula()).then(|| (!raw.is_empty()).then(|| literal_value(raw.as_str())));
        let rejected = self
            .bytecode
            .recompile_if_stale(cell, raw, indices, &mut self.deps, ctx);

        if let Some(literal) = plain {
            let mut cache = self.formula_cache.borrow_mut();
            match literal {
                Some(value) => cache.insert(cell, value),
                None => cache.remove(&cell),
            };
        } else if let Some(err @ CellValue::Error(_)) = rejected {
            // A rejected formula (cycle, parse error) has no bytecode, so the VM
            // pass will not overwrite the cache. Publish the error here, else the
            // cell keeps displaying the result of its previous text.
            self.formula_cache.borrow_mut().insert(cell, err);
        }
    }

    /// Mark every formula cell and every cached plain cell dirty. Used after a
    /// structural reorder (sort) that keeps the uid-anchored bytecode intact. The
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
            let mut ctx = EvalContext::new(&self.bytecode, &cache_ref, indices, now_serial());

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

        // Whatever the topological walk did not visit has no node in `cell_dag`:
        // a plain literal nobody references. Its cached value was published by
        // `on_cell_changed` and needs no recomputation, so just drop it from the
        // dirty set. (Re-reading and re-inserting it, as this loop once did, was
        // a no-op that read like a fixup.)
        self.dirty.clear();
    }

    /// Get the display text for a cell, or `None` if it has no cached result.
    pub fn evaluated_text(&self, cell: CellId) -> Option<String> {
        self.formula_cache
            .borrow()
            .get(&cell)
            .map(|v| v.to_display())
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
}

/// Current wall-clock time as an Excel serial number. The integer part is the
/// day; the fractional part is the time of day.
///
/// Private to the engine: a recomputation reads it once and hands it to every
/// call, so no function can read the clock for itself (`TRAME-194`).
fn now_serial() -> DateSerial {
    #[cfg(target_arch = "wasm32")]
    {
        // js_sys::Date::now() returns milliseconds since the Unix epoch.
        DateSerial::from_unix_days(js_sys::Date::now() / 86_400_000.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        DateSerial::from_unix_days(secs / 86_400.0)
    }
}

/// Parse a plain cell's raw text into a literal [`CellValue`]. Text that
/// spells no finite number stays text (`TRAME-245`).
fn literal_value(raw: &str) -> CellValue {
    match super::types::finite_number(raw) {
        Some(n) => CellValue::Number(n),
        None => CellValue::Text(raw.to_string()),
    }
}

impl Default for FormulaEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::SheetIndices;
    use super::super::test_index::{index_with, indices_with, stored};
    use super::super::types::SingleSheetCtx;
    use super::*;
    use crate::Row;
    use crate::formula::types::EngineSheet;
    use crate::model::Uid;

    /// Bound cell at grid `(row, col)` on sheet 0.
    fn cell(row: u32, col: u32) -> CellId {
        CellId::bound(
            EngineSheet::ACTIVE,
            Uid {
                peer: ambre_core::PeerId(1),
                counter: ambre_core::IdCounter::new(row),
            },
            Uid {
                peer: ambre_core::PeerId(2),
                counter: ambre_core::IdCounter::new(col),
            },
        )
    }

    #[test]
    fn literal_formula_evaluated() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        let c = cell(0, 0);

        engine.on_cell_changed(c, stored("=42"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(c).unwrap(), "42");
        assert!(engine.is_formula_cell(c));
    }

    #[test]
    fn arithmetic_chain() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=10"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), stored("=A1*2"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "20");
    }

    #[test]
    fn cascade_after_edit() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=5"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), stored("=A1+3"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "8");

        engine.dirty.clear();
        engine.on_cell_changed(cell(0, 0), stored("=10"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "10");
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "13");
    }

    #[test]
    fn division_by_zero_displayed() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        let c = cell(0, 0);

        engine.on_cell_changed(c, stored("=1/0"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(c).unwrap(), "#DIV/0!");
    }

    #[test]
    fn sum_range() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        for i in 0..3 {
            engine.on_cell_changed(
                cell(i, 0),
                stored(&format!("={}", i + 1)),
                &idx,
                &SingleSheetCtx,
            );
        }
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 2), stored("=SUM(A1:A3)"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "6");
    }

    #[test]
    fn sum_over_plain_literal_cells() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("1"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), stored("2"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 2), stored("=SUM(A1:B1)"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "3");
    }

    #[test]
    fn static_cycle_rejected() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=B1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        engine.dirty.clear();

        engine.on_cell_changed(cell(0, 1), stored("=A1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "#CIRCULAR!");
    }

    #[test]
    fn self_reference_rejected() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=A1+1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "#CIRCULAR!");
    }

    #[test]
    fn retyping_the_same_text_does_not_recompile() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        engine.on_cell_changed(cell(0, 0), stored("=1+1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.bytecode.compiles(), 1);

        engine.on_cell_changed(cell(0, 0), stored("=1+1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(
            engine.bytecode.compiles(),
            1,
            "the same text is not compiled again"
        );
        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "2");

        engine.on_cell_changed(cell(0, 0), stored("=1+2"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.bytecode.compiles(), 2, "a new text is");
        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "3");
    }

    #[test]
    fn plain_cell_keeps_dependents_across_repeated_edits() {
        // A plain literal is not a formula, so it carries no bytecode -- but a
        // formula referencing it owns an edge out of its node. Rewriting the
        // literal must not tear that edge down, or the dependent silently
        // freezes on its last computed value.
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("1"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), stored("=A1+0"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "1");

        for (n, expected) in [("2", "2"), ("3", "3"), ("4", "4")] {
            engine.on_cell_changed(cell(0, 0), stored(n), &idx, &SingleSheetCtx);
            engine.compute_all_dirty(&idx);
            assert_eq!(
                engine.evaluated_text(cell(0, 1)).unwrap(),
                expected,
                "A1 = {n}"
            );
        }
    }

    #[test]
    fn range_dependency_propagates_through_a_chain() {
        // Range deps live outside the cell DAG, so dirty collection must reach a
        // fixpoint over them: C1 depends on B1 only through `B1:B3`.
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("1"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), stored("=SUM(A1:A3)"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 2), stored("=SUM(B1:B3)"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "1");

        engine.on_cell_changed(cell(0, 0), stored("10"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "10", "direct");
        assert_eq!(engine.evaluated_text(cell(0, 2)).unwrap(), "10", "chained");
    }

    #[test]
    fn cycle_replaces_the_stale_value_with_an_error() {
        // B1 holds a good value, then A1 is edited into a cycle with it. The
        // rejected formula must surface `#CIRCULAR!`, not keep displaying the
        // result its previous text produced.
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=1"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), stored("=A1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "1");

        engine.on_cell_changed(cell(0, 0), stored("=B1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "#CIRCULAR!");
    }

    #[test]
    fn parse_error_replaces_the_stale_value() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(cell(0, 0), stored("=1+1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "2");

        engine.on_cell_changed(cell(0, 0), stored("=SUM("), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert!(
            engine
                .evaluated_text(cell(0, 0))
                .unwrap()
                .starts_with("#PARSE!"),
            "a broken formula must not keep its previous result"
        );
    }

    #[test]
    fn function_if() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();

        engine.on_cell_changed(
            cell(0, 0),
            stored("=IF(1>0, \"yes\", \"no\")"),
            &idx,
            &SingleSheetCtx,
        );
        engine.compute_all_dirty(&idx);

        assert_eq!(engine.evaluated_text(cell(0, 0)).unwrap(), "yes");
    }

    #[test]
    fn ref_follows_data_through_index_remap() {
        // A1 = 10 (row uid 0), A2 = 20 (row uid 1), B1 = "=A1" binds to row uid 0.
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        engine.on_cell_changed(cell(0, 0), stored("=10"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(1, 0), stored("=20"), &idx, &SingleSheetCtx);
        engine.on_cell_changed(cell(0, 1), stored("=A1"), &idx, &SingleSheetCtx);
        engine.compute_all_dirty(&idx);
        assert_eq!(engine.evaluated_text(cell(0, 1)).unwrap(), "10");

        // Swap rows 0 and 1 (a sort-style permutation) and recompute WITHOUT
        // re-parsing: the uid-anchored bytecode is preserved. B1 is bound to row
        // uid 0, which is now at position 1. So it keeps reading that row's A
        // value (10): it follows the data. It is NOT the positional 20 that a
        // re-parse against the new order would produce.
        let mut moved = index_with(10, 10);
        moved.apply_move_row(Row::new(0), Row::new(1));
        let mut indices = SheetIndices::new();
        indices.insert(EngineSheet::ACTIVE, moved);
        engine.mark_all_dirty();
        engine.compute_all_dirty(&indices);
        assert_eq!(
            engine.evaluated_text(cell(0, 1)).unwrap(),
            "10",
            "ref follows the data, not the position"
        );
    }
}
