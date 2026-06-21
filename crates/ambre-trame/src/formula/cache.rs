//! Bytecode cache -- incremental recompilation.
//!
//! Holds compiled [`CellBytecode`] keyed by [`CellId`] and a `stale` set of
//! cells whose raw text has changed and need recompilation on the next
//! evaluation pass. Recompilation resolves positional references against the
//! current per-sheet indices (so `=A1` binds to the row/column `Uid`).

use std::collections::{HashMap, HashSet};

use super::compile::{SheetIndices, compile};
use super::dag::DependencyGraph;
use super::parser::{parse_formula, resolve_refs};
use super::types::{CellBytecode, CellId, CellValue, FormulaCtx, FormulaError};

/// Caches compiled bytecode and tracks stale cells for incremental compilation.
pub struct BytecodeCache {
    /// Compiled bytecode for each formula cell.
    entries: HashMap<CellId, CellBytecode>,
    /// Cells whose raw text has changed since their last compilation.
    stale: HashSet<CellId>,
}

impl BytecodeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            stale: HashSet::new(),
        }
    }

    /// Mark a cell as needing recompilation (its raw text changed).
    pub fn invalidate(&mut self, cell: CellId) {
        self.stale.insert(cell);
    }

    /// Look up the bytecode for a cell, returning `None` if it's not a formula.
    pub fn get(&self, cell: &CellId) -> Option<&CellBytecode> {
        self.entries.get(cell)
    }

    /// Iterate every formula cell that currently has compiled bytecode. Used to
    /// mark the whole sheet dirty after a structural reorder that preserves the
    /// uid-anchored bytecode (sort).
    pub fn cells(&self) -> impl Iterator<Item = &CellId> {
        self.entries.keys()
    }

    /// Insert bytecode for a cell (used in tests).
    #[cfg(test)]
    pub fn insert(&mut self, cell: CellId, bc: CellBytecode) {
        self.entries.insert(cell, bc);
    }

    /// Recompile `cell` if its raw text changed (`stale`).
    ///
    /// Returns `Some(error_value)` when the formula is rejected (cycle / parse
    /// error), `None` when it compiled cleanly, was already up-to-date, or is no
    /// longer a formula.
    pub fn recompile_if_stale(
        &mut self,
        cell: CellId,
        raw: &str,
        indices: &SheetIndices,
        deps: &mut DependencyGraph,
        ctx: &dyn FormulaCtx,
    ) -> Option<CellValue> {
        if !self.stale.remove(&cell) {
            return None;
        }

        if !raw.starts_with('=') {
            // No longer a formula: remove bytecode and graph edges.
            self.entries.remove(&cell);
            deps.remove(cell);
            return Some(CellValue::Empty);
        }

        let formula_text = &raw[1..]; // Strip leading '='.
        // Parse (context-free) then resolve sheet-qualified refs and named
        // ranges against the host context; unqualified refs land on the
        // formula's own sheet (`cell.sheet`). Positional `A1` is bound to a
        // row/column `Uid` by `compile` against `indices`.
        let parsed = parse_formula(formula_text).and_then(|ast| resolve_refs(ast, cell.sheet, ctx));
        match parsed {
            Ok(ast) => {
                let bc = compile(&ast, indices);
                match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
                    Ok(()) => {
                        self.entries.insert(cell, bc);
                        None // No error -- caller evaluates normally.
                    }
                    Err(_cycle_err) => {
                        self.entries.remove(&cell);
                        Some(CellValue::Error(FormulaError::CircularReference))
                    }
                }
            }
            Err(msg) => {
                self.entries.remove(&cell);
                Some(CellValue::Error(FormulaError::ParseError(msg)))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for BytecodeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_index::indices_with;
    use super::super::types::{AxisId, SingleSheetCtx};
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

    #[test]
    fn stale_cell_recompiled() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        cache.invalidate(c);
        let err = cache.recompile_if_stale(c, "=42", &idx, &mut deps, &SingleSheetCtx);
        assert!(err.is_none());
        assert!(cache.get(&c).is_some());
        assert!(!cache.stale.contains(&c));
    }

    #[test]
    fn non_formula_clears_cache() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        cache.invalidate(c);
        cache.recompile_if_stale(c, "=42", &idx, &mut deps, &SingleSheetCtx);
        assert!(cache.get(&c).is_some());

        cache.invalidate(c);
        cache.recompile_if_stale(c, "hello", &idx, &mut deps, &SingleSheetCtx);
        assert!(cache.get(&c).is_none());
    }

    #[test]
    fn parse_error_returns_error_value() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        cache.invalidate(c);
        let err = cache.recompile_if_stale(c, "=SUM(", &idx, &mut deps, &SingleSheetCtx);
        assert!(matches!(
            err,
            Some(CellValue::Error(FormulaError::ParseError(_)))
        ));
    }

    #[test]
    fn not_stale_no_recompile() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        let err = cache.recompile_if_stale(c, "=42", &idx, &mut deps, &SingleSheetCtx);
        assert!(err.is_none());
        assert!(cache.get(&c).is_none());
    }

    #[test]
    fn formula_binds_ref_to_uid() {
        // `=A1` on a 10x10 sheet binds its dependency to the bound row/col uid.
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 1);

        cache.invalidate(c);
        cache.recompile_if_stale(c, "=A1", &idx, &mut deps, &SingleSheetCtx);
        let bc = cache.get(&c).unwrap();
        assert!(matches!(
            bc.cell_deps[0],
            CellId {
                row: AxisId::Bound(_),
                col: AxisId::Bound(_),
                ..
            }
        ));
    }
}
