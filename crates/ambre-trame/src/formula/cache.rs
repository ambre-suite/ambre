//! Bytecode cache — incremental recompilation.
//!
//! Holds compiled [`CellBytecode`] keyed by [`CellRef`] and a `stale` set of
//! cells whose raw text has changed and need recompilation on the next
//! evaluation pass.

use std::collections::{HashMap, HashSet};

use super::compile::compile;
use super::dag::DependencyGraph;
use super::parser::parse_formula;
use super::types::{CellBytecode, CellRef, CellValue, FormulaError};

/// Caches compiled bytecode and tracks stale cells for incremental compilation.
pub struct BytecodeCache {
    /// Compiled bytecode for each formula cell.
    entries: HashMap<CellRef, CellBytecode>,
    /// Cells whose raw text has changed since their last compilation.
    stale: HashSet<CellRef>,
}

impl BytecodeCache {
    pub fn new() -> Self {
        Self { entries: HashMap::new(), stale: HashSet::new() }
    }

    /// Mark a cell as needing recompilation (its raw text changed).
    pub fn invalidate(&mut self, cell: CellRef) {
        self.stale.insert(cell);
    }

    /// Look up the bytecode for a cell, returning `None` if it's not a formula.
    pub fn get(&self, cell: &CellRef) -> Option<&CellBytecode> {
        self.entries.get(cell)
    }

    /// Insert bytecode for a cell (used in tests).
    #[cfg(test)]
    pub fn insert(&mut self, cell: CellRef, bc: CellBytecode) {
        self.entries.insert(cell, bc);
    }

    /// Recompile `cell` if its raw text changed (`stale`).
    ///
    /// Returns `true` if recompilation happened (caller should mark surrounding
    /// viewport dirty), `false` if the cell was already up-to-date or not a
    /// formula.
    pub fn recompile_if_stale(
        &mut self,
        cell: CellRef,
        raw: &str,
        deps: &mut DependencyGraph,
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
        match parse_formula(formula_text) {
            Ok(ast) => {
                let bc = compile(&ast);
                match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
                    Ok(()) => {
                        self.entries.insert(cell, bc);
                        None // No error — caller evaluates normally.
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
    use super::*;

    #[test]
    fn stale_cell_recompiled() {
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let cell = CellRef::new(0, 0);

        cache.invalidate(cell);
        let err = cache.recompile_if_stale(cell, "=42", &mut deps);
        assert!(err.is_none());
        assert!(cache.get(&cell).is_some());
        assert!(!cache.stale.contains(&cell));
    }

    #[test]
    fn non_formula_clears_cache() {
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let cell = CellRef::new(0, 0);

        // First, compile as formula.
        cache.invalidate(cell);
        cache.recompile_if_stale(cell, "=42", &mut deps);
        assert!(cache.get(&cell).is_some());

        // Now, set to plain text.
        cache.invalidate(cell);
        cache.recompile_if_stale(cell, "hello", &mut deps);
        assert!(cache.get(&cell).is_none());
    }

    #[test]
    fn parse_error_returns_error_value() {
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let cell = CellRef::new(0, 0);

        cache.invalidate(cell);
        let err = cache.recompile_if_stale(cell, "=SUM(", &mut deps);
        assert!(matches!(err, Some(CellValue::Error(FormulaError::ParseError(_)))));
    }

    #[test]
    fn not_stale_no_recompile() {
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let cell = CellRef::new(0, 0);

        let err = cache.recompile_if_stale(cell, "=42", &mut deps);
        assert!(err.is_none());
        assert!(cache.get(&cell).is_none()); // Not compiled because not stale.
    }
}
