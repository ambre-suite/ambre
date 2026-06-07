//! Two-level dependency graph for spreadsheet formulas.
//!
//! Level 1 (`cell_dag`): cell-to-cell dependencies with static cycle detection
//! via Kosaraju SCC. Level 2 (`range_deps` / `range_index`): range dependencies
//! where each cell in the range is a dependency, stored compactly and scanned
//! linearly (MVP).
//!
//! # Design rationale
//! If `=SUM(A1:A100)` created 100 edges in `petgraph`, an entire-column formula
//! would explode memory. Two-level resolution avoids this: ranges are stored
//! separately and checked for containment at dirty-collection time.

use std::collections::{HashMap, HashSet};

use petgraph::stable_graph::StableGraph;
use petgraph::visit::{Dfs, EdgeRef};
use petgraph::Direction;

use super::types::{CellRef, RangeDep};

// -----------------------------------------------------------------------------
// CycleError
// -----------------------------------------------------------------------------

/// Returned by [`DependencyGraph::update`] when adding a formula would create a
/// cycle in the cell-to-cell dependency graph.
#[derive(Debug, Clone)]
pub struct CycleError(pub Vec<CellRef>);

// -----------------------------------------------------------------------------
// DependencyGraph
// -----------------------------------------------------------------------------

pub struct DependencyGraph {
    /// Cell-to-cell DAG. A directed edge `A → B` means "B depends on A"
    /// (A is a dependency of B). Topological order visits A before B;
    /// dirty propagation follows `A → B` from a changed cell A to its
    /// dependents.
    pub cell_dag: StableGraph<CellRef, ()>,
    /// Map from `CellRef` to its node index in `cell_dag`.
    pub cell_to_node: HashMap<CellRef, petgraph::graph::NodeIndex>,

    /// Range dependencies for each formula cell.
    pub range_deps: HashMap<CellRef, Vec<RangeDep>>,
    /// Reverse index: `CellRef` in this sheet → list of `(formula_cell, RangeDep)`
    /// whose range contains this cell. MVP linear scan; Phase 2: 2D interval tree.
    pub range_index: HashMap<CellRef, Vec<(CellRef, RangeDep)>>,
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
    fn get_or_create_node(&mut self, cell: CellRef) -> petgraph::graph::NodeIndex {
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
        formula_cell: CellRef,
        new_cell_deps: Vec<CellRef>,
        new_range_deps: Vec<RangeDep>,
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

        // Add new cell edges: dep → formula_cell (dependency points to the
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

        // Update range dependencies.
        // Clear previous range entries for this formula across all entries.
        for entries in self.range_index.values_mut() {
            entries.retain(|(fc, _)| *fc != formula_cell);
        }
        self.range_deps.remove(&formula_cell);

        if !new_range_deps.is_empty() {
            // Register each cell in each range as depending on formula_cell.
            for dep in &new_range_deps {
                for row in dep.row_start..=dep.row_end {
                    for col in dep.col_start..=dep.col_end {
                        let cell = CellRef::new(row, col);
                        self.range_index
                            .entry(cell)
                            .or_default()
                            .push((formula_cell, dep.clone()));
                    }
                }
            }
            self.range_deps.insert(formula_cell, new_range_deps);
        }

        Ok(())
    }

    /// Remove a formula cell entirely from the graph (called when a cell is
    /// cleared or its formula is deleted).
    pub fn remove(&mut self, cell: CellRef) {
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
    /// contains `changed` and its transitive cell-level dependents.
    pub fn collect_dirty(&self, changed: CellRef) -> HashSet<CellRef> {
        let mut dirty = HashSet::new();

        // Level 1: DFS descendants in cell_dag (cells that depend on `changed`).
        // Dfs starts at `changed` and follows outgoing edges; the starting node
        // is included in the traversal but should not be counted as dirty.
        if let Some(&node) = self.cell_to_node.get(&changed) {
            let mut dfs = Dfs::new(&self.cell_dag, node);
            while let Some(n) = dfs.next(&self.cell_dag) {
                let cell = self.cell_dag[n];
                if cell != changed {
                    dirty.insert(cell);
                }
            }
        }

        // Level 2: formulas whose range contains `changed`.
        if let Some(entries) = self.range_index.get(&changed) {
            for (formula_cell, _dep) in entries {
                dirty.insert(*formula_cell);
                // Propagate from each affected formula.
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
    fn find_cycle_cells(&self) -> Vec<CellRef> {
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
    use super::*;

    #[test]
    fn simple_dependency_propagation() {
        let mut dag = DependencyGraph::new();

        // B1 depends on A1.
        dag.update(CellRef::new(1, 1), vec![CellRef::new(0, 0)], vec![])
            .unwrap();
        // C1 depends on B1.
        dag.update(CellRef::new(2, 1), vec![CellRef::new(1, 1)], vec![])
            .unwrap();

        // Changing A1 should dirty B1 and C1.
        let dirty = dag.collect_dirty(CellRef::new(0, 0));
        assert!(dirty.contains(&CellRef::new(1, 1)));
        assert!(dirty.contains(&CellRef::new(2, 1)));
        assert!(!dirty.contains(&CellRef::new(0, 0))); // source not included
    }

    #[test]
    fn static_cycle_detected() {
        let mut dag = DependencyGraph::new();

        // A1 depends on B1.
        dag.update(CellRef::new(0, 0), vec![CellRef::new(0, 1)], vec![])
            .unwrap();
        // B1 depends on A1 → cycle.
        let result = dag.update(CellRef::new(0, 1), vec![CellRef::new(0, 0)], vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn range_dirty_collection() {
        let mut dag = DependencyGraph::new();

        // C1 = SUM(A1:A3)
        let range = RangeDep::new(0, 2, 0, 0);
        dag.update(CellRef::new(2, 2), vec![], vec![range.clone()])
            .unwrap();

        // Changing A2 (inside range) dirties C1.
        let dirty = dag.collect_dirty(CellRef::new(1, 0));
        assert!(dirty.contains(&CellRef::new(2, 2)));
    }

    #[test]
    fn range_outside_no_dirty() {
        let mut dag = DependencyGraph::new();

        let range = RangeDep::new(0, 2, 0, 0);
        dag.update(CellRef::new(2, 2), vec![], vec![range])
            .unwrap();

        // Changing B5 (outside range) does not dirty C1.
        let dirty = dag.collect_dirty(CellRef::new(4, 1));
        assert!(!dirty.contains(&CellRef::new(2, 2)));
    }

    #[test]
    fn remove_formula_cleans_up() {
        let mut dag = DependencyGraph::new();

        dag.update(CellRef::new(0, 1), vec![CellRef::new(0, 0)], vec![])
            .unwrap();
        assert_eq!(dag.node_count(), 2);

        dag.remove(CellRef::new(0, 1));
        assert_eq!(dag.node_count(), 1);
        assert!(dag.range_deps.is_empty());
    }

    #[test]
    fn self_reference_rejected() {
        let mut dag = DependencyGraph::new();

        // Self-reference: edge A1 → A1 is not added (guarded in update),
        // but the graph is still acyclic.
        let result = dag.update(CellRef::new(0, 0), vec![], vec![]);
        assert!(result.is_ok());
        assert!(!petgraph::algo::is_cyclic_directed(&dag.cell_dag));
    }

    #[test]
    fn cycle_detection_kosaraju_returns_culprits() {
        let mut dag = DependencyGraph::new();

        // A1 → B1 → C1 → A1
        dag.update(CellRef::new(0, 0), vec![CellRef::new(0, 1)], vec![])
            .unwrap();
        dag.update(CellRef::new(0, 1), vec![CellRef::new(0, 2)], vec![])
            .unwrap();
        let err = dag
            .update(CellRef::new(0, 2), vec![CellRef::new(0, 0)], vec![])
            .unwrap_err();
        let cycle = err.0;
        assert_eq!(cycle.len(), 3);
    }
}
