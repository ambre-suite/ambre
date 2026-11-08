//! Two-level dependency graph for spreadsheet formulas.
//!
//! Level 1 (`cell_dag`): cell-to-cell dependencies with static cycle detection
//! via Kosaraju SCC. Level 2 (`range_index`): range dependencies registered per
//! sheet and scanned linearly. Each formula's range is tested for containment
//! of the changed cell through the current [`SheetIndex`] (MVP).
//!
//! Both levels are keyed by the uid-anchored [`CellId`], so that dependencies
//! survive structural reorder. A `=A1` edge points at the row/column `Uid`, not
//! a grid position.
//!
//! # Design rationale
//! If `=SUM(A1:A100)` created 100 edges in `petgraph`, an entire-column formula
//! would explode memory. Two-level resolution avoids this: ranges are stored
//! separately and checked for containment at dirty-collection time.

use crate::hash::{HashMap, HashSet};
use ambre_core::index::Pos;

use petgraph::Direction;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::EdgeRef;

use super::types::{AxisId, CellId, EngineSheet, RangeId};
use crate::model::SheetIndex;

// -----------------------------------------------------------------------------
// CycleError
// -----------------------------------------------------------------------------

/// Returned by [`DependencyGraph::update`] when adding a formula would create a
/// cycle in the cell-to-cell dependency graph.
#[derive(Debug, Clone)]
pub struct CycleError(Vec<CellId>);

impl CycleError {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: Vec<CellId>) -> Self {
        Self(value)
    }

    /// Returns the wrapped value.
    pub fn into_inner(self) -> Vec<CellId> {
        self.0
    }
}

// -----------------------------------------------------------------------------
// DependencyGraph
// -----------------------------------------------------------------------------

pub struct DependencyGraph {
    /// Cell-to-cell DAG. A directed edge `A -> B` means "B depends on A"
    /// (A is a dependency of B). Topological order visits A before B;
    /// dirty propagation follows `A -> B` from a changed cell A to its
    /// dependents.
    pub cell_dag: StableGraph<CellId, ()>,
    /// Map from `CellId` to its node index in `cell_dag`.
    pub cell_to_node: HashMap<CellId, petgraph::graph::NodeIndex>,

    /// Range dependencies for each formula cell.
    pub range_deps: HashMap<CellId, Vec<RangeId>>,
    /// Reverse index: sheet ([`Pos<EngineSheet>`]) -> list of `(formula_cell, RangeId)`
    /// whose range lives on that sheet. MVP linear scan; Phase 2: 2D interval tree.
    pub range_index: HashMap<Pos<EngineSheet>, Vec<(CellId, RangeId)>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            cell_dag: StableGraph::new(),
            cell_to_node: HashMap::default(),
            range_deps: HashMap::default(),
            range_index: HashMap::default(),
        }
    }

    /// Ensure a node exists for `cell` in `cell_dag`, returning its index.
    fn get_or_create_node(&mut self, cell: CellId) -> petgraph::graph::NodeIndex {
        *self
            .cell_to_node
            .entry(cell)
            .or_insert_with(|| self.cell_dag.add_node(cell))
    }

    /// Update the dependencies for `formula_cell`.
    ///
    /// Removes all previous outgoing edges (both cell and range), adds new
    /// cell edges, and registers new range deps. If cell-level edges create a
    /// cycle, the operation is rolled back and a [`CycleError`] is returned
    /// listing the offending cells.
    ///
    /// # Errors
    ///
    /// [`CycleError`] when the new cell edges close a cycle, listing the cells
    /// that carry it. The dependencies are rolled back first, so a refused
    /// formula depends on nothing.
    pub fn update(
        &mut self,
        formula_cell: CellId,
        new_cell_deps: Vec<CellId>,
        new_range_deps: Vec<RangeId>,
    ) -> Result<(), CycleError> {
        let node = self.get_or_create_node(formula_cell);

        // Drop the previous dependencies, cell and range alike. Both are cleared
        // up front, so that a rejected formula (cycle) is left depending on
        // nothing. It does not keep range entries its bytecode no longer has.
        self.clear_incoming(node);
        self.clear_range_deps(formula_cell);

        // A formula reading its own cell is the shortest cycle there is, a
        // self-loop in `cell_dag`. It is refused before any edge is added. The
        // formula then depends on nothing, as after the rollback of a longer cycle.
        if new_cell_deps.contains(&formula_cell) {
            return Err(CycleError(vec![formula_cell]));
        }

        // Add new cell edges: dep -> formula_cell (dependency points to the
        // formula that depends on it).
        for dep in &new_cell_deps {
            let dep_node = self.get_or_create_node(*dep);
            self.cell_dag.add_edge(dep_node, node, ());
        }

        // Static cycle detection, before the rollback, so the culprits can be
        // named. The graph held no cycle before this update. A new cycle
        // therefore runs through `node`, and exists only when `node` already
        // reaches one of its new dependencies. The search reads the
        // descendants of `node` alone, never the whole graph.
        if let Some(culprits) = self.cycle_through(node) {
            self.clear_incoming(node);
            return Err(CycleError(culprits));
        }

        if !new_range_deps.is_empty() {
            for dep in &new_range_deps {
                self.range_index
                    .entry(dep.sheet)
                    .or_default()
                    .push((formula_cell, dep.clone()));
            }
            self.range_deps.insert(formula_cell, new_range_deps);
        }

        Ok(())
    }

    /// Drop the dependencies *of* `cell` -- the cells and ranges it reads --
    /// when it stops being a formula (cleared, or replaced by a literal).
    ///
    /// The node itself survives as long as something still points out of it. An
    /// edge `cell -> dependent` is owned by the *dependent*'s formula. Removing
    /// the node would silently unhook it, freezing that dependent on its last
    /// computed value. Only a node left with no edge at all is dropped.
    pub fn clear_deps(&mut self, cell: CellId) {
        if let Some(&node) = self.cell_to_node.get(&cell) {
            self.clear_incoming(node);
            if self.cell_dag.edges(node).next().is_none() {
                self.cell_dag.remove_node(node);
                self.cell_to_node.remove(&cell);
            }
        }
        self.clear_range_deps(cell);
    }

    /// Drop the node of `cell` once no edge touches it. A cell of a deleted
    /// line keeps its node while a formula still reads it. Once no formula
    /// reads it, the node goes: a full reseed never creates it.
    pub fn drop_if_isolated(&mut self, cell: CellId) {
        let Some(&node) = self.cell_to_node.get(&cell) else {
            return;
        };
        let touched = self
            .cell_dag
            .edges_directed(node, Direction::Incoming)
            .next()
            .is_some()
            || self.cell_dag.edges(node).next().is_some();
        if !touched {
            self.cell_dag.remove_node(node);
            self.cell_to_node.remove(&cell);
        }
    }

    /// Remove every edge pointing *into* `node`, that is, the dependencies its own
    /// formula declared. Outgoing edges (its dependents) are untouched.
    fn clear_incoming(&mut self, node: petgraph::graph::NodeIndex) {
        let incoming: Vec<_> = self
            .cell_dag
            .edges_directed(node, Direction::Incoming)
            .map(|e| e.id())
            .collect();
        for e in incoming {
            self.cell_dag.remove_edge(e);
        }
    }

    /// Unregister every range `cell` reads, on every sheet.
    fn clear_range_deps(&mut self, cell: CellId) {
        self.range_deps.remove(&cell);
        for entries in self.range_index.values_mut() {
            entries.retain(|(fc, _)| *fc != cell);
        }
    }

    /// Collect all cells that must be marked dirty after `changed` is modified.
    ///
    /// Returns the transitive closure of downstream dependents across *both*
    /// levels. A single sweep does not suffice. A range dependent found at
    /// level 2 may itself sit inside another formula's range (`B1=SUM(A1:A3)`,
    /// `C1=SUM(B1:B3)`), a link the `cell_dag` does not carry. So newly dirtied
    /// cells are queued and re-expanded until the set reaches a fixpoint.
    ///
    /// Containment is tested per sheet: `indices` carries every sheet's index.
    /// A dependent reached through a cross-sheet edge lives on its own sheet,
    /// whose uids the edited cell's index knows nothing about.
    pub fn collect_dirty(
        &self,
        changed: CellId,
        indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
    ) -> HashSet<CellId> {
        let mut dirty = HashSet::default();
        // A reference typed past the used range kept its position, and the
        // line it named exists now. The edit reaches it under that position
        // too, else the formula keeps the value it read as empty (`TRAME-233`).
        let mut queue = vec![changed];
        queue.extend(position_aliases(changed, indices));
        let mut seen: HashSet<CellId> = queue.iter().copied().collect();

        while let Some(cur) = queue.pop() {
            // Level 1: the cells that read `cur` directly. The queue walks
            // their own readers in turn, so the closure is complete. A `Dfs`
            // per cell would clear a bit set over the whole graph each time:
            // about 2 500 bytes per cell on a sheet of 10k formulas.
            if let Some(&node) = self.cell_to_node.get(&cur) {
                for n in self.cell_dag.neighbors(node) {
                    let Some(&cell) = self.cell_dag.node_weight(n) else {
                        continue;
                    };
                    if cell == cur {
                        continue;
                    }
                    dirty.insert(cell);
                    if seen.insert(cell) {
                        queue.push(cell);
                    }
                }
            }

            // Level 2: formulas on the same sheet whose range contains `cur`.
            // A range is keyed by row/column `Uid`s, so containment needs the
            // cell's bound uids; a virtual cell cannot be inside a range.
            if let (Some(row), Some(col)) = (cur.row_uid(), cur.col_uid())
                && let Some(entries) = self.range_index.get(&cur.sheet)
                && let Some(index) = indices.get(&cur.sheet)
            {
                for (formula_cell, dep) in entries {
                    if *formula_cell == cur || !dep.contains(row, col, index) {
                        continue;
                    }
                    dirty.insert(*formula_cell);
                    if seen.insert(*formula_cell) {
                        queue.push(*formula_cell);
                    }
                }
            }
        }

        // `changed` is dirtied by its own caller; a formula sitting inside the
        // range it reads must not re-enter the set through level 2.
        dirty.remove(&changed);
        dirty
    }

    /// The cells of the cycle through `node`, or `None` when `node` is on no
    /// cycle.
    ///
    /// The graph was acyclic before the edges into `node` were added. The
    /// only strongly connected component of more than one cell is therefore
    /// the one that holds `node`. Its cells are the descendants of `node` that reach `node`
    /// back. Both searches stay inside the descendants of `node`: a formula
    /// that nothing reads costs one step.
    fn cycle_through(&self, node: petgraph::graph::NodeIndex) -> Option<Vec<CellId>> {
        // A set of the nodes met, not a `Dfs`: a `Dfs` clears a bit set over
        // the whole graph, which costs more than the walk when few cells read
        // `node`.
        let mut below: HashSet<_> = std::iter::once(node).collect();
        let mut stack = vec![node];
        while let Some(cur) = stack.pop() {
            for n in self.cell_dag.neighbors(cur) {
                if below.insert(n) {
                    stack.push(n);
                }
            }
        }
        // A dependency of `node` that is also below it closes the loop.
        let closes = self
            .cell_dag
            .neighbors_directed(node, Direction::Incoming)
            .any(|dep| below.contains(&dep));
        if !closes {
            return None;
        }
        // Walk back from `node` along the incoming edges, inside the
        // descendants: each cell met is on a path from `node` back to `node`.
        let mut on_cycle = vec![node];
        let mut seen: HashSet<_> = on_cycle.iter().copied().collect();
        let mut queue = vec![node];
        while let Some(cur) = queue.pop() {
            for dep in self.cell_dag.neighbors_directed(cur, Direction::Incoming) {
                if below.contains(&dep) && seen.insert(dep) {
                    on_cycle.push(dep);
                    queue.push(dep);
                }
            }
        }
        Some(on_cycle.into_iter().map(|n| self.cell_dag[n]).collect())
    }

    /// Number of nodes in the cell-level DAG. Only the tests read it.
    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        self.cell_to_node.len()
    }
}

/// The ids a reference may hold for the bound cell `cell`: each axis kept as
/// the position it sits at today. Empty for a cell with a virtual axis, or one
/// whose lines its sheet index does not hold.
fn position_aliases(cell: CellId, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) -> Vec<CellId> {
    let (Some(row), Some(col)) = (cell.row_uid(), cell.col_uid()) else {
        return Vec::new();
    };
    let Some(index) = indices.get(&cell.sheet) else {
        return Vec::new();
    };
    let Some(at) = index.pos_of(&row, &col) else {
        return Vec::new();
    };
    let (row_at, col_at) = (AxisId::Virtual(at.row), AxisId::Virtual(at.col));
    vec![
        CellId::new(cell.sheet, AxisId::Bound(row), col_at),
        CellId::new(cell.sheet, row_at, AxisId::Bound(col)),
        CellId::new(cell.sheet, row_at, col_at),
    ]
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::test_index::indices_with;
    use super::super::types::{ACTIVE_SHEET, AxisId, RangeEndpoint};
    use super::*;
    use crate::model::SheetIndex;
    use crate::model::Uid;

    /// Bound cell at grid `(row, col)` on sheet 0, matching `index_with`'s
    /// synthetic uids (rows peer 1, cols peer 2, counter == position).
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

    /// Bound range over grid rows `[r0, r1]` x cols `[c0, c1]` on sheet 0.
    fn range(r0: u32, r1: u32, c0: u32, c1: u32) -> RangeId {
        let rid = |r: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: ambre_core::PeerId::new(1),
                counter: ambre_core::IdCounter::new(r),
            }))
        };
        let cid = |c: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: ambre_core::PeerId::new(2),
                counter: ambre_core::IdCounter::new(c),
            }))
        };
        RangeId {
            sheet: ACTIVE_SHEET,
            row_start: rid(r0),
            row_end: rid(r1),
            col_start: cid(c0),
            col_end: cid(c1),
        }
    }

    #[test]
    fn simple_dependency_propagation() {
        let idx = indices_with(10, 10);
        let mut dag = DependencyGraph::new();

        // B1 depends on A1.
        dag.update(cell(0, 1), vec![cell(0, 0)], vec![]).unwrap();
        // C1 depends on B1.
        dag.update(cell(0, 2), vec![cell(0, 1)], vec![]).unwrap();

        let dirty = dag.collect_dirty(cell(0, 0), &idx);
        assert!(dirty.contains(&cell(0, 1)));
        assert!(dirty.contains(&cell(0, 2)));
        assert!(!dirty.contains(&cell(0, 0)));
    }

    #[test]
    fn static_cycle_detected() {
        let mut dag = DependencyGraph::new();
        dag.update(cell(0, 0), vec![cell(0, 1)], vec![]).unwrap();
        let result = dag.update(cell(0, 1), vec![cell(0, 0)], vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn range_dirty_collection() {
        let idx = indices_with(10, 10);
        let mut dag = DependencyGraph::new();

        // C3 = SUM(A1:A3)
        dag.update(cell(2, 2), vec![], vec![range(0, 2, 0, 0)])
            .unwrap();

        // Changing A2 (inside range) dirties C3.
        let dirty = dag.collect_dirty(cell(1, 0), &idx);
        assert!(dirty.contains(&cell(2, 2)));
    }

    #[test]
    fn range_outside_no_dirty() {
        let idx = indices_with(10, 10);
        let mut dag = DependencyGraph::new();
        dag.update(cell(2, 2), vec![], vec![range(0, 2, 0, 0)])
            .unwrap();

        // Changing B5 (outside range) does not dirty C3.
        let dirty = dag.collect_dirty(cell(4, 1), &idx);
        assert!(!dirty.contains(&cell(2, 2)));
    }

    #[test]
    fn clear_deps_drops_the_formula_and_its_now_orphan_node() {
        let mut dag = DependencyGraph::new();
        dag.update(cell(0, 1), vec![cell(0, 0)], vec![]).unwrap();
        assert_eq!(dag.node_count(), 2);

        // B1 stops being a formula: it reads nothing and nothing reads it.
        dag.clear_deps(cell(0, 1));
        assert_eq!(dag.node_count(), 1);
        assert!(dag.range_deps.is_empty());
    }

    #[test]
    fn clear_deps_keeps_the_node_that_dependents_read() {
        let idx = indices_with(10, 10);
        let mut dag = DependencyGraph::new();
        // B1 = A1, then A1 turns into a plain literal.
        dag.update(cell(0, 1), vec![cell(0, 0)], vec![]).unwrap();
        dag.clear_deps(cell(0, 0));

        assert_eq!(dag.node_count(), 2, "A1's node still carries A1 -> B1");
        assert!(dag.collect_dirty(cell(0, 0), &idx).contains(&cell(0, 1)));
    }

    #[test]
    fn collect_dirty_reaches_a_fixpoint_over_chained_ranges() {
        let idx = indices_with(10, 10);
        let mut dag = DependencyGraph::new();
        // B1 = SUM(A1:A3), C1 = SUM(B1:B3): the B1 -> C1 link exists only at
        // the range level, so one sweep would stop at B1.
        dag.update(cell(0, 1), vec![], vec![range(0, 2, 0, 0)])
            .unwrap();
        dag.update(cell(0, 2), vec![], vec![range(0, 2, 1, 1)])
            .unwrap();

        let dirty = dag.collect_dirty(cell(0, 0), &idx);
        assert!(dirty.contains(&cell(0, 1)));
        assert!(dirty.contains(&cell(0, 2)));
    }

    #[test]
    fn rejected_cycle_leaves_no_stale_range_deps() {
        let mut dag = DependencyGraph::new();
        dag.update(cell(0, 0), vec![cell(0, 1)], vec![]).unwrap();
        // B1 = SUM(C1:C3) first, then B1 = A1 which closes a cycle.
        dag.update(cell(0, 1), vec![], vec![range(0, 2, 2, 2)])
            .unwrap();
        assert!(dag.update(cell(0, 1), vec![cell(0, 0)], vec![]).is_err());

        assert!(dag.range_deps.is_empty());
        assert!(dag.range_index.values().all(|e| e.is_empty()));
    }

    /// A `SheetIndex` for `sheet`, with per-sheet peers so that no uid is shared
    /// between two sheets. As in a live document, where each sheet owns its own
    /// Loro row/column lists.
    fn sheet_index(sheet: u16, rows: usize, cols: usize) -> SheetIndex {
        let mut idx = SheetIndex::default();
        for i in 0..rows {
            idx.push_row(Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 1),
                counter: ambre_core::IdCounter::new(i as u32),
            });
        }
        for i in 0..cols {
            idx.push_col(Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 2),
                counter: ambre_core::IdCounter::new(i as u32),
            });
        }
        idx
    }

    /// Bound cell at grid `(row, col)` on `sheet`, matching `sheet_index`'s uids.
    fn cell_on(sheet: u32, row: u32, col: u32) -> CellId {
        CellId::bound(
            Pos::<EngineSheet>::new(sheet),
            Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 1),
                counter: ambre_core::IdCounter::new(row),
            },
            Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 2),
                counter: ambre_core::IdCounter::new(col),
            },
        )
    }

    /// Bound range on `sheet`, matching `sheet_index`'s uids.
    fn range_on(sheet: u32, r0: u32, r1: u32, c0: u32, c1: u32) -> RangeId {
        let rid = |r: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 1),
                counter: ambre_core::IdCounter::new(r),
            }))
        };
        let cid = |c: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: ambre_core::PeerId::new(sheet as u64 * 10 + 2),
                counter: ambre_core::IdCounter::new(c),
            }))
        };
        RangeId {
            sheet: Pos::<EngineSheet>::new(sheet),
            row_start: rid(r0),
            row_end: rid(r1),
            col_start: cid(c0),
            col_end: cid(c1),
        }
    }

    #[test]
    fn collect_dirty_tests_containment_against_each_cell_own_sheet() {
        // Sheet1!B1 = Sheet0!A1 (a level-1 cross-sheet edge), and
        // Sheet1!C1 = SUM(Sheet1!B1:B3) (a level-2 range on sheet 1).
        // Editing Sheet0!A1 must reach C1: once the fixpoint walks past B1 it has
        // to test B1's containment through *sheet 1's* index. Sheet 0's index
        // knows none of sheet 1's uids, so reusing it silently drops the link.
        let mut indices = HashMap::default();
        indices.insert(Pos::<EngineSheet>::new(0), sheet_index(0, 10, 10));
        indices.insert(Pos::<EngineSheet>::new(1), sheet_index(1, 10, 10));

        let mut dag = DependencyGraph::new();
        dag.update(cell_on(1, 0, 1), vec![cell_on(0, 0, 0)], vec![])
            .unwrap();
        dag.update(cell_on(1, 0, 2), vec![], vec![range_on(1, 0, 2, 1, 1)])
            .unwrap();

        let dirty = dag.collect_dirty(cell_on(0, 0, 0), &indices);
        assert!(dirty.contains(&cell_on(1, 0, 1)), "direct cross-sheet edge");
        assert!(
            dirty.contains(&cell_on(1, 0, 2)),
            "range dependent living on the other sheet"
        );
    }

    #[test]
    fn self_reference_rejected() {
        let mut dag = DependencyGraph::new();
        let err = dag
            .update(cell(0, 0), vec![cell(0, 1), cell(0, 0)], vec![])
            .unwrap_err();
        assert_eq!(err.0, vec![cell(0, 0)]);
        // The refused formula depends on nothing, so no edge survives.
        assert_eq!(dag.cell_dag.edge_count(), 0);
    }

    #[test]
    fn cycle_detection_kosaraju_returns_culprits() {
        let mut dag = DependencyGraph::new();
        dag.update(cell(0, 0), vec![cell(0, 1)], vec![]).unwrap();
        dag.update(cell(0, 1), vec![cell(0, 2)], vec![]).unwrap();
        let err = dag
            .update(cell(0, 2), vec![cell(0, 0)], vec![])
            .unwrap_err();
        assert_eq!(err.0.len(), 3);
    }
}
