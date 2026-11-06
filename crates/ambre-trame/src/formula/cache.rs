//! Bytecode cache -- incremental recompilation.
//!
//! Holds compiled [`CellBytecode`] keyed by [`CellId`] and a `stale` set of
//! cells whose raw text has changed and need recompilation on the next
//! evaluation pass. Recompilation resolves positional references against the
//! current per-sheet indices (so `=A1` binds to the row/column `Uid`).

use crate::formula::text::CellText;
use std::collections::{HashMap, HashSet};

use super::compile::compile;
use super::dag::DependencyGraph;
use super::identity::{follows_its_lines, unbind};
use super::parser::{parse_formula, resolve_refs};
use super::text::StoredForm;
use super::types::EngineSheet;
use super::types::{CellBytecode, CellId, CellValue, FormulaCtx, FormulaError, SheetQualifier};
use crate::model::SheetIndex;
use ambre_core::index::Pos;

/// Caches compiled bytecode and tracks stale cells for incremental compilation.
pub struct BytecodeCache {
    /// Compiled bytecode for each formula cell.
    entries: HashMap<CellId, CellBytecode>,
    /// Cells whose raw text has changed since their last compilation.
    stale: HashSet<CellId>,
    /// The raw text each entry of `entries` was compiled from, so a cell
    /// retyped with the same text is recognised and not recompiled
    /// (`TRAME-212`).
    sources: HashMap<CellId, CellText<StoredForm>>,
    /// Formulas compiled since construction, for the tests that hold
    /// `TRAME-212`.
    compiles: u64,
    /// The formulas a structural edit re-reads whatever lines it edits: each
    /// one that reads positions ([`follows_its_lines`] is false).
    positional: HashSet<CellId>,
    /// The formulas refused for a static cycle. A refused formula keeps no
    /// edge, so the graph cannot say what it reads.
    cyclic: HashSet<CellId>,
}

impl BytecodeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            stale: HashSet::new(),
            sources: HashMap::new(),
            compiles: 0,
            positional: HashSet::new(),
            cyclic: HashSet::new(),
        }
    }

    /// Forget the text `cell` was compiled from, so the next write of that
    /// same text compiles it again. A structural edit uses it on a formula
    /// whose text it kept but whose positions it moved.
    pub fn forget_source(&mut self, cell: CellId) {
        self.sources.remove(&cell);
    }

    /// The formulas that read positions, which a structural edit re-reads.
    pub fn positional(&self) -> impl Iterator<Item = &CellId> {
        self.positional.iter()
    }

    /// Whether a formula is refused for a static cycle.
    pub fn has_cyclic(&self) -> bool {
        !self.cyclic.is_empty()
    }

    /// Mark a cell as needing recompilation (its raw text changed).
    pub fn invalidate(&mut self, cell: CellId) {
        self.stale.insert(cell);
    }

    /// Whether `cell` holds bytecode compiled from exactly `raw`: retyping it
    /// changes nothing, so the edit path skips it (`TRAME-212`).
    pub fn compiled_from(&self, cell: CellId, raw: &CellText<StoredForm>) -> bool {
        self.sources.get(&cell).is_some_and(|source| source == raw)
    }

    /// Formulas compiled since construction.
    #[cfg(test)]
    pub fn compiles(&self) -> u64 {
        self.compiles
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
        raw: CellText<StoredForm>,
        indices: &HashMap<Pos<EngineSheet>, SheetIndex>,
        deps: &mut DependencyGraph,
        ctx: &dyn FormulaCtx,
    ) -> Option<CellValue> {
        if !self.stale.remove(&cell) {
            return None;
        }
        self.positional.remove(&cell);
        self.cyclic.remove(&cell);

        let Some(formula_text) = raw.formula_body() else {
            // No longer a formula: drop its bytecode and the dependencies it
            // declared. The graph node stays if other formulas read this cell.
            self.entries.remove(&cell);
            self.sources.remove(&cell);
            deps.clear_deps(cell);
            return Some(CellValue::Empty);
        };
        // Parse (context-free), then give each stored line its position in
        // today's layout (`TRAME-228`). Then resolve sheet-qualified refs and
        // named ranges against the host context; unqualified refs land on the
        // formula's own sheet (`cell.sheet`). Positional `A1` is bound to a
        // row/column `Uid` by `compile` against `indices`.
        let index_of = |q: Option<&SheetQualifier>| {
            let sheet = match q {
                None => Some(cell.sheet),
                Some(SheetQualifier::Id(id)) => ctx.resolve_sheet_id(id),
                Some(SheetQualifier::Name(name)) => ctx.resolve_sheet(name),
            };
            indices.get(&sheet?)
        };
        let mut follows = true;
        let parsed = parse_formula(formula_text).and_then(|ast| {
            follows = follows_its_lines(&ast, &index_of);
            resolve_refs(unbind(ast, &index_of), cell.sheet, ctx)
        });
        self.compiles += 1;
        self.sources.remove(&cell);
        match parsed {
            Ok(ast) => {
                let bc = compile(&ast, indices);
                match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
                    Ok(()) => {
                        self.entries.insert(cell, bc);
                        self.sources.insert(cell, raw);
                        if !follows {
                            self.positional.insert(cell);
                        }
                        None // No error -- caller evaluates normally.
                    }
                    Err(_cycle_err) => {
                        self.entries.remove(&cell);
                        self.cyclic.insert(cell);
                        Some(CellValue::Error(FormulaError::CircularReference))
                    }
                }
            }
            Err(msg) => {
                // A formula that no longer parses reads nothing: drop its old
                // dependencies too, or it would keep being recomputed from them.
                self.entries.remove(&cell);
                deps.clear_deps(cell);
                Some(CellValue::Error(FormulaError::ParseError(msg)))
            }
        }
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
    use super::super::test_index::{indices_with, stored};
    use super::super::types::{ACTIVE_SHEET, AxisId, SingleSheetCtx};
    use super::*;
    use crate::model::Uid;

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
    fn stale_cell_recompiled() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        cache.invalidate(c);
        let err = cache.recompile_if_stale(c, stored("=42"), &idx, &mut deps, &SingleSheetCtx);
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
        cache.recompile_if_stale(c, stored("=42"), &idx, &mut deps, &SingleSheetCtx);
        assert!(cache.get(&c).is_some());

        cache.invalidate(c);
        cache.recompile_if_stale(c, stored("hello"), &idx, &mut deps, &SingleSheetCtx);
        assert!(cache.get(&c).is_none());
    }

    #[test]
    fn parse_error_returns_error_value() {
        let idx = indices_with(10, 10);
        let mut cache = BytecodeCache::new();
        let mut deps = DependencyGraph::new();
        let c = cell(0, 0);

        cache.invalidate(c);
        let err = cache.recompile_if_stale(c, stored("=SUM("), &idx, &mut deps, &SingleSheetCtx);
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

        let err = cache.recompile_if_stale(c, stored("=42"), &idx, &mut deps, &SingleSheetCtx);
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
        cache.recompile_if_stale(c, stored("=A1"), &idx, &mut deps, &SingleSheetCtx);
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
