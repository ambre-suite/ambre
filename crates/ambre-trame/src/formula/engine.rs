//! Formula engine -- orchestrates parsing, compilation, dependency tracking,
//! and evaluation for all formula cells in a Trame document.
//!
//! Owns the `DependencyGraph` (petgraph), `BytecodeCache`, formula result cache,
//! and the `dirty` set. `compute_all_dirty()` is the single entry point for
//! recomputation: topological sort on the cell DAG -> recompile stale bytecode
//! -> evaluate via the stack VM. All state is keyed by the uid-anchored
//! [`CellId`]; the host passes the current per-sheet indices on every call so
//! references resolve against live positions.

use crate::formula::text::CellText;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use petgraph::visit::Topo;

use super::cache::BytecodeCache;
use super::dag::DependencyGraph;
use super::text::StoredForm;
use super::types::{CellId, CellValue, EngineSheet, FormulaCtx};
use super::vm::EvalContext;
use crate::model::SheetIndex;
use ambre_core::index::Pos;
use ambre_core::{Instant, SheetDateTime};

/// How [`FormulaEngine::apply_structural`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carry {
    /// The engine holds what a full reseed would.
    Done,
    /// A formula compiled again met a cycle, and only a reseed can say which
    /// cell of it is refused.
    CycleMet,
}

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
    /// Every cell whose text or value the engine took or computed since the
    /// caller last cleared the set. The caller names them to the view
    /// (`TRAME-284`).
    pub changed: HashSet<CellId>,
}

impl FormulaEngine {
    pub fn new() -> Self {
        Self {
            deps: DependencyGraph::new(),
            bytecode: BytecodeCache::new(),
            formula_cache: RefCell::new(HashMap::new()),
            dirty: HashSet::new(),
            changed: HashSet::new(),
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
        raw: CellText<StoredForm>,
        indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
        ctx: &dyn FormulaCtx,
    ) {
        // The same text compiles to the same bytecode and computes the same
        // value: nothing downstream changes either (`TRAME-212`).
        if self.bytecode.compiled_from(cell, &raw) {
            return;
        }
        self.changed.insert(cell);
        self.bytecode.invalidate(cell);
        let downstream = self.deps.collect_dirty(cell, indices);
        self.dirty.insert(cell);
        self.dirty.extend(downstream);

        // Plain (non-formula) cells never go through the VM, so their value must
        // be published here for formula references to resolve. It is read
        // before the cache takes the text.
        let plain = (!raw.is_formula()).then(|| (!raw.is_empty()).then(|| literal_value(&raw)));
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

    /// The formulas a structural edit must re-read, given the cells it removed.
    ///
    /// Two kinds qualify. A formula that reads a removed cell names a line that
    /// is gone, so its text and its value change. A formula that reads
    /// positions may move with any edit ([`BytecodeCache::positional`]). Any
    /// other formula reads its cells by their lines, which the edit leaves in
    /// place, so neither its text nor its value changes. The search costs the
    /// edges out of the removed cells and the positional formulas, never the
    /// whole graph (`TRAME-280`).
    pub fn structural_reach(&self, removed: &HashSet<CellId>) -> Vec<CellId> {
        let mut reach: HashSet<CellId> = HashSet::new();
        for cell in removed {
            let Some(&node) = self.deps.cell_to_node.get(cell) else {
                continue;
            };
            reach.extend(
                self.deps
                    .cell_dag
                    .neighbors_directed(node, petgraph::Direction::Outgoing)
                    .map(|n| self.deps.cell_dag[n]),
            );
        }
        reach.extend(self.bytecode.positional().copied());
        reach.retain(|cell| !removed.contains(cell));
        reach.into_iter().collect()
    }

    /// Carry the engine across one structural edit, without a reseed.
    ///
    /// `removed` are the cells of the lines the edit removed, and `reread` the
    /// formulas [`structural_reach`](Self::structural_reach) named, each with
    /// its text after the edit. Each removed cell is cleared, each formula of
    /// `reread` compiles again from its text, and the dirty cells recompute.
    ///
    /// Returns [`Carry::CycleMet`] when a formula was refused for a cycle. A
    /// full reseed then decides which cell of the cycle is refused, by its
    /// feed order, and the caller runs one.
    pub fn apply_structural(
        &mut self,
        removed: &HashSet<CellId>,
        reread: &[(CellId, CellText<StoredForm>)],
        indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
        ctx: &dyn FormulaCtx,
    ) -> Carry {
        self.apply_remote(removed, reread, &[], indices, ctx)
    }

    /// Carry the engine across a remote batch, without a reseed
    /// (`TRAME-282`).
    ///
    /// `removed` and `reread` are what a structural edit of the batch leaves,
    /// as for [`apply_structural`](Self::apply_structural). `written` are the
    /// cells the batch wrote, each with the text it now shows, which no
    /// formula of `reread` repeats. A written cell compiles from its text as
    /// a local write does, so a text it already compiled from costs nothing
    /// (`TRAME-212`).
    ///
    /// Returns [`Carry::CycleMet`] when a formula was refused for a cycle, as
    /// [`apply_structural`](Self::apply_structural) does.
    pub fn apply_remote(
        &mut self,
        removed: &HashSet<CellId>,
        reread: &[(CellId, CellText<StoredForm>)],
        written: &[(CellId, CellText<StoredForm>)],
        indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
        ctx: &dyn FormulaCtx,
    ) -> Carry {
        for &cell in removed {
            if self.knows(cell) {
                self.on_cell_changed(cell, CellText::new(String::new()), indices, ctx);
            }
        }
        for (cell, raw) in reread {
            self.bytecode.forget_source(*cell);
            self.on_cell_changed(*cell, raw.clone(), indices, ctx);
        }
        for (cell, raw) in written {
            self.on_cell_changed(*cell, raw.clone(), indices, ctx);
        }
        if self.bytecode.has_cyclic() {
            self.dirty.clear();
            return Carry::CycleMet;
        }
        // A removed cell kept its node while a formula still read it. Each of
        // those formulas compiled again above and reads it no more.
        for &cell in removed {
            self.deps.drop_if_isolated(cell);
        }
        self.compute_dirty_closure(indices);
        Carry::Done
    }

    /// Whether the engine holds anything for `cell`: a value, bytecode or a
    /// node of the graph.
    fn knows(&self, cell: CellId) -> bool {
        self.formula_cache.borrow().contains_key(&cell)
            || self.bytecode.get(&cell).is_some()
            || self.deps.cell_to_node.contains_key(&cell)
    }

    /// Whether a formula is refused for a static cycle. The graph keeps no
    /// edge of such a formula, so only a full reseed can say whether a later
    /// edit frees it.
    pub fn has_cyclic(&self) -> bool {
        self.bytecode.has_cyclic()
    }

    /// Recompute the `dirty` set in topological order of the graph that the
    /// dirty cells induce, never walking the rest of the graph.
    ///
    /// The evaluation pass reads each cell it needs on demand and keeps what
    /// it computed, so the order changes no value of an acyclic graph. The
    /// order is still topological, so a cell reads each dependency computed
    /// once.
    fn compute_dirty_closure(&mut self, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) {
        let dag = &self.deps.cell_dag;
        let mut nodes: Vec<petgraph::graph::NodeIndex> = self
            .dirty
            .iter()
            .filter_map(|cell| self.deps.cell_to_node.get(cell))
            .copied()
            .collect();
        nodes.sort_unstable();
        let inside: HashSet<petgraph::graph::NodeIndex> = nodes.iter().copied().collect();
        let mut waiting: HashMap<petgraph::graph::NodeIndex, usize> = nodes
            .iter()
            .map(|&n| {
                let deps = dag
                    .neighbors_directed(n, petgraph::Direction::Incoming)
                    .filter(|d| inside.contains(d))
                    .count();
                (n, deps)
            })
            .collect();
        let mut ready: Vec<_> = nodes
            .iter()
            .rev()
            .copied()
            .filter(|n| waiting.get(n) == Some(&0))
            .collect();

        let mut results: Vec<(CellId, CellValue)> = Vec::new();
        {
            let cache_ref = self.formula_cache.borrow();
            let mut ctx = EvalContext::new(&self.bytecode, &cache_ref, indices, now_serial());
            while let Some(node) = ready.pop() {
                let cell = dag[node];
                if self.bytecode.get(&cell).is_some() {
                    results.push((cell, ctx.eval_cell(cell)));
                }
                for next in dag.neighbors_directed(node, petgraph::Direction::Outgoing) {
                    if let Some(left) = waiting.get_mut(&next) {
                        *left = left.saturating_sub(1);
                        if *left == 0 {
                            ready.push(next);
                        }
                    }
                }
            }
        }

        let mut cache = self.formula_cache.borrow_mut();
        for (cell, value) in results {
            self.changed.insert(cell);
            cache.insert(cell, value);
        }
        self.dirty.clear();
    }

    /// Recompute every cell in the `dirty` set in topological order.
    pub fn compute_all_dirty(&mut self, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) {
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
                self.changed.insert(cell);
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
fn now_serial() -> Instant<SheetDateTime> {
    #[cfg(target_arch = "wasm32")]
    {
        // js_sys::Date::now() returns milliseconds since the Unix epoch.
        Instant::<SheetDateTime>::from_unix_days(js_sys::Date::now() / 86_400_000.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        Instant::<SheetDateTime>::from_unix_days(secs / 86_400.0)
    }
}

/// Parse a plain cell's raw text into a literal [`CellValue`]. Text that
/// spells no finite number stays text (`TRAME-245`).
fn literal_value(raw: &CellText<StoredForm>) -> CellValue {
    match super::types::finite_number(raw.as_str()) {
        Some(n) => CellValue::Number(n),
        None => CellValue::Text(raw.as_str().to_owned()),
    }
}

impl Default for FormulaEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_index::{index_with, indices_with, stored};
    use super::super::types::SingleSheetCtx;
    use super::*;
    use crate::Row;
    use crate::formula::ACTIVE_SHEET;
    use crate::model::Uid;
    use ambre_core::index::Pos;

    /// Bound cell at grid `(row, col)` on sheet 0.
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
        moved.apply_move_row(Pos::<Row>::new(0), Pos::<Row>::new(1));
        let mut indices = HashMap::new();
        indices.insert(ACTIVE_SHEET, moved);
        engine.mark_all_dirty();
        engine.compute_all_dirty(&indices);
        assert_eq!(
            engine.evaluated_text(cell(0, 1)).unwrap(),
            "10",
            "ref follows the data, not the position"
        );
    }

    /// A loop closed through a range shows `#CIRCULAR!` on each of its cells
    /// and on each cell downstream of it, whatever order the pass takes. The
    /// range also holds `#DIV/0!`, which a pass entering from outside the loop
    /// meets first (`TRAME-281`).
    #[test]
    fn a_loop_through_a_range_shows_circular_in_every_order() {
        let idx = indices_with(10, 10);
        let mut engine = FormulaEngine::new();
        for (at, text) in [
            (cell(0, 1), "=1/0"),
            (cell(1, 0), "=SUM(B1:B3)"),
            (cell(1, 1), "=A2*2"),
            (cell(0, 3), "=SUM(B2:B3)"),
            (cell(0, 2), "=B2+1"),
        ] {
            engine.on_cell_changed(at, stored(text), &idx, &SingleSheetCtx);
        }
        let looped = [cell(1, 0), cell(1, 1), cell(0, 3), cell(0, 2)];
        let orders = [
            looped.to_vec(),
            looped.iter().rev().copied().collect::<Vec<_>>(),
        ];
        for order in orders {
            let cache = engine.formula_cache.borrow();
            let mut ctx = EvalContext::new(
                &engine.bytecode,
                &cache,
                &idx,
                Instant::<SheetDateTime>::from_unix_days(0.0),
            );
            for &at in &order {
                ctx.eval_cell(at);
            }
            for at in looped {
                let shown = ctx.eval_cell(at).to_display();
                assert_eq!(shown, "#CIRCULAR!", "{at:?} in the order {order:?}");
            }
        }
        // A cell the loop does not reach keeps its own error.
        let cache = engine.formula_cache.borrow();
        let mut ctx = EvalContext::new(
            &engine.bytecode,
            &cache,
            &idx,
            Instant::<SheetDateTime>::from_unix_days(0.0),
        );
        assert_eq!(ctx.eval_cell(cell(0, 1)).to_display(), "#DIV/0!");
    }
}
