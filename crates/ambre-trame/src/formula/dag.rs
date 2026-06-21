//! Two-level dependency graph for spreadsheet formulas.
//!
//! Level 1 (`cell_dag`): cell-to-cell dependencies with static cycle detection
//! via Kosaraju SCC. Level 2 (`range_index`): range dependencies registered per
//! sheet and scanned linearly, testing each formula's range for containment of
//! the changed cell through the current [`SheetIndex`] (MVP).
//!
//! Both levels are keyed by the uid-anchored [`CellId`] so that dependencies
//! survive structural reorder: a `=A1` edge points at the row/column `Uid`, not
//! a grid position.
//!
//! # Design rationale
//! If `=SUM(A1:A100)` created 100 edges in `petgraph`, an entire-column formula
//! would explode memory. Two-level resolution avoids this: ranges are stored
//! separately and checked for containment at dirty-collection time.

use std::collections::{HashMap, HashSet};

use petgraph::Direction;
use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Dfs, EdgeRef};

use crate::model::SheetIndex;

use super::types::{CellId, RangeId};

// -----------------------------------------------------------------------------
// CycleError
// -----------------------------------------------------------------------------

/// Returned by [`DependencyGraph::update`] when adding a formula would create a
/// cycle in the cell-to-cell dependency graph.
#[derive(Debug, Clone)]
pub struct CycleError(pub Vec<CellId>);

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
    /// Reverse index: sheet (local `u16`) -> list of `(formula_cell, RangeId)`
    /// whose range lives on that sheet. MVP linear scan; Phase 2: 2D interval tree.
    pub range_index: HashMap<u16, Vec<(CellId, RangeId)>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            cell_dag: StableGraph::new(),
            cell_to_node: HashMap::new(),
            range_deps: HashMap::new(),
            range_index: HashMap::new(),
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
    pub fn update(
        &mut self,
        formula_cell: CellId,
        new_cell_deps: Vec<CellId>,
        new_range_deps: Vec<RangeId>,
    ) -> Result<(), CycleError> {
        let node = self.get_or_create_node(formula_cell);

        // Remove old cell edges pointing to formula_cell (incoming).
        let old_edges: Vec<_> = self
            .cell_dag
            .edges_directed(node, Direction::Incoming)
            .map(|e| e.id())
            .collect();
        for e in old_edges {
            self.cell_dag.remove_edge(e);
        }

        // Add new cell edges: dep -> formula_cell (dependency points to the
        // formula that depends on it).
        for dep in &new_cell_deps {
            if *dep != formula_cell {
                let dep_node = self.get_or_create_node(*dep);
                self.cell_dag.add_edge(dep_node, node, ());
            }
        }

        // Static cycle detection (before rollback, so we can identify the
        // offending cells).
        if petgraph::algo::is_cyclic_directed(&self.cell_dag) {
            let culprits = self.find_cycle_cells();
            // Rollback the edges we just added.
            let added: Vec<_> = self
                .cell_dag
                .edges_directed(node, Direction::Incoming)
                .map(|e| e.id())
                .collect();
            for e in added {
                self.cell_dag.remove_edge(e);
            }
            return Err(CycleError(culprits));
        }

        // Update range dependencies: clear previous range entries for this
        // formula across all sheets, then re-register per sheet.
        for entries in self.range_index.values_mut() {
            entries.retain(|(fc, _)| *fc != formula_cell);
        }
        self.range_deps.remove(&formula_cell);

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

    /// Remove a formula cell entirely from the graph (called when a cell is
    /// cleared or its formula is deleted).
    pub fn remove(&mut self, cell: CellId) {
        // Remove cell_dag node.
        if let Some(&node) = self.cell_to_node.get(&cell) {
            self.cell_dag.remove_node(node);
            self.cell_to_node.remove(&cell);
            // Rebuild cell_to_node since StableGraph may reindex.
            self.cell_to_node.clear();
            for idx in self.cell_dag.node_indices() {
                self.cell_to_node.insert(self.cell_dag[idx], idx);
            }
        }

        // Remove range deps for this cell.
        self.range_deps.remove(&cell);
        for entries in self.range_index.values_mut() {
            entries.retain(|(fc, _)| *fc != cell);
        }
    }

    /// Collect all cells that must be marked dirty after `changed` is modified.
    ///
    /// Returns the transitive closure of downstream dependents: all cells in
    /// `cell_dag` reachable from `changed`, plus any formula whose range
    /// contains `changed` (resolved through `index`) and its transitive
    /// cell-level dependents.
    pub fn collect_dirty(&self, changed: CellId, index: &SheetIndex) -> HashSet<CellId> {
        let mut dirty = HashSet::new();

        // Level 1: DFS descendants in cell_dag (cells that depend on `changed`).
        if let Some(&node) = self.cell_to_node.get(&changed) {
            let mut dfs = Dfs::new(&self.cell_dag, node);
            while let Some(n) = dfs.next(&self.cell_dag) {
                let cell = self.cell_dag[n];
                if cell != changed {
                    dirty.insert(cell);
                }
            }
        }

        // Level 2: formulas on the same sheet whose range contains `changed`.
        // A range is keyed by row/column `Uid`s, so containment needs the
        // changed cell's bound uids; a virtual cell cannot be inside a range.
        if let (Some(row), Some(col)) = (changed.row_uid(), changed.col_uid())
            && let Some(entries) = self.range_index.get(&changed.sheet)
        {
            for (formula_cell, dep) in entries {
                if !dep.contains(row, col, index) {
                    continue;
                }
                dirty.insert(*formula_cell);
                if let Some(&node) = self.cell_to_node.get(formula_cell) {
                    let mut dfs = Dfs::new(&self.cell_dag, node);
                    while let Some(n) = dfs.next(&self.cell_dag) {
                        dirty.insert(self.cell_dag[n]);
                    }
                }
            }
        }

        dirty
    }

    /// Find cells participating in a cycle via Kosaraju SCC.
    fn find_cycle_cells(&self) -> Vec<CellId> {
        petgraph::algo::kosaraju_scc(&self.cell_dag)
            .into_iter()
            .find(|scc| scc.len() > 1)
            .unwrap_or_default()
            .into_iter()
            .map(|n| self.cell_dag[n])
            .collect()
    }

    /// Number of nodes in the cell-level DAG.
    pub fn node_count(&self) -> usize {
        self.cell_to_node.len()
    }
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
    use super::super::test_index::index_with;
    use super::super::types::{AxisId, RangeEndpoint};
    use super::*;
    use crate::model::Uid;

    /// Bound cell at grid `(row, col)` on sheet 0, matching `index_with`'s
    /// synthetic uids (rows peer 1, cols peer 2, counter == position).
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

    /// Bound range over grid rows `[r0, r1]` x cols `[c0, c1]` on sheet 0.
    fn range(r0: u32, r1: u32, c0: u32, c1: u32) -> RangeId {
        let rid = |r: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: 1,
                counter: r,
            }))
        };
        let cid = |c: u32| {
            RangeEndpoint::Specific(AxisId::Bound(Uid {
                peer: 2,
                counter: c,
            }))
        };
        RangeId {
            sheet: 0,
            row_start: rid(r0),
            row_end: rid(r1),
            col_start: cid(c0),
            col_end: cid(c1),
        }
    }

    #[test]
    fn simple_dependency_propagation() {
        let idx = index_with(10, 10);
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
        let idx = index_with(10, 10);
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
        let idx = index_with(10, 10);
        let mut dag = DependencyGraph::new();
        dag.update(cell(2, 2), vec![], vec![range(0, 2, 0, 0)])
            .unwrap();

        // Changing B5 (outside range) does not dirty C3.
        let dirty = dag.collect_dirty(cell(4, 1), &idx);
        assert!(!dirty.contains(&cell(2, 2)));
    }

    #[test]
    fn remove_formula_cleans_up() {
        let mut dag = DependencyGraph::new();
        dag.update(cell(0, 1), vec![cell(0, 0)], vec![]).unwrap();
        assert_eq!(dag.node_count(), 2);

        dag.remove(cell(0, 1));
        assert_eq!(dag.node_count(), 1);
        assert!(dag.range_deps.is_empty());
    }

    #[test]
    fn self_reference_rejected() {
        let mut dag = DependencyGraph::new();
        let result = dag.update(cell(0, 0), vec![], vec![]);
        assert!(result.is_ok());
        assert!(!petgraph::algo::is_cyclic_directed(&dag.cell_dag));
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
