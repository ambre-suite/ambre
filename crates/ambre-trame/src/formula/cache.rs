//! Bytecode cache -- incremental recompilation.
//!
//! Holds compiled [`CellBytecode`] keyed by [`CellId`] and a `stale` set of
//! cells whose raw text has changed and need recompilation on the next
//! evaluation pass. Recompilation resolves positional references against the
//! current per-sheet indices (so `=A1` binds to the row/column `Uid`).

use crate::formula::text::CellText;
use crate::hash::{HashMap, HashSet};

use super::compile::compile;
use super::dag::DependencyGraph;
use super::identity::{NamedCell, lines_it_names, unbind};
use super::parser::{parse_formula, resolve_refs};
use super::text::StoredForm;
use super::types::EngineSheet;
use super::types::{CellBytecode, CellId, CellValue, FormulaCtx, FormulaError, SheetQualifier};
use crate::model::SheetIndex;
use ambre_core::index::Pos;

/// Caches compiled bytecode and tracks stale cells for incremental compilation.
pub struct BytecodeCache {
    /// Compiled bytecode for each formula cell, with the lines it names.
    entries: HashMap<CellId, Compiled>,
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
    /// one that reads positions ([`lines_it_names`] is `None`).
    positional: HashSet<CellId>,
    /// The formulas refused for a static cycle. A refused formula keeps no
    /// edge, so the graph cannot say what it reads.
    cyclic: HashSet<CellId>,
    /// The formulas that read a cell by a virtual axis: a position past the
    /// used range. A write that adds the line at that position binds it on
    /// the next compile, so such a formula compiles again (`TRAME-289`).
    unbound: HashSet<CellId>,
    /// The bytecode and the source of each formula a local line removal
    /// cleared, for the undo that writes the same text back (`TRAME-300`).
    parked: HashMap<CellId, Parked>,
}

/// The bytecode of one formula, and the cells its text names by their lines.
struct Compiled {
    bytecode: CellBytecode,
    /// Each cell a formula that follows its lines names, in the order of its
    /// text. Empty for a formula that reads positions. A parked bytecode is
    /// reused only while each one stays placed (`TRAME-300`).
    lines: Box<[NamedCell]>,
}

/// A formula a local line removal cleared: the bytecode it compiled to and
/// the text it compiled from.
struct Parked {
    source: CellText<StoredForm>,
    compiled: Compiled,
}

impl BytecodeCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::default(),
            stale: HashSet::default(),
            sources: HashMap::default(),
            compiles: 0,
            positional: HashSet::default(),
            cyclic: HashSet::default(),
            unbound: HashSet::default(),
            parked: HashMap::default(),
        }
    }

    /// Park the bytecode and the source of `cell`, which a local line removal
    /// is about to clear (`TRAME-300`). Some bytecode depends on more than
    /// the lines it names, and the cache parks none of it. That covers a
    /// formula that reads positions or a virtual axis, and one refused for a
    /// cycle.
    pub fn park(&mut self, cell: CellId) {
        if self.positional.contains(&cell)
            || self.unbound.contains(&cell)
            || self.cyclic.contains(&cell)
        {
            return;
        }
        let (Some(compiled), Some(source)) =
            (self.entries.remove(&cell), self.sources.remove(&cell))
        else {
            return;
        };
        self.parked.insert(cell, Parked { source, compiled });
    }

    /// Drop every parked formula: no step of the history can write one back.
    pub fn clear_parked(&mut self) {
        self.parked = HashMap::default();
    }

    /// Whether a formula is parked.
    pub fn has_parked(&self) -> bool {
        !self.parked.is_empty()
    }

    /// The heap bytes the parked formulas hold, keys and table slots aside,
    /// for the test that states what a parked block costs.
    #[cfg(test)]
    pub fn parked_bytes(&self) -> usize {
        self.parked
            .values()
            .map(|p| {
                let bc = &p.compiled.bytecode;
                p.source.as_str().len()
                    + bc.ops.capacity() * std::mem::size_of::<super::types::Op>()
                    + bc.cell_deps.capacity() * std::mem::size_of::<CellId>()
                    + bc.range_deps.capacity() * std::mem::size_of::<super::types::RangeId>()
                    + std::mem::size_of_val(&*p.compiled.lines)
                    + std::mem::size_of::<Parked>()
                    + std::mem::size_of::<CellId>()
            })
            .sum()
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

    /// The formulas that read a cell by a virtual axis (`TRAME-289`).
    pub fn unbound(&self) -> impl Iterator<Item = &CellId> {
        self.unbound.iter()
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
        self.entries.get(cell).map(|compiled| &compiled.bytecode)
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
        self.entries.insert(
            cell,
            Compiled {
                bytecode: bc,
                lines: Box::default(),
            },
        );
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
        self.unbound.remove(&cell);

        let Some(formula_text) = raw.formula_body() else {
            // No longer a formula: drop its bytecode and the dependencies it
            // declared. The graph node stays if other formulas read this cell.
            self.entries.remove(&cell);
            self.sources.remove(&cell);
            deps.clear_deps(cell);
            return Some(CellValue::Empty);
        };
        // A parked formula written back with the text it compiled from, over
        // the lines it named, compiles to the bytecode it holds (`TRAME-300`).
        if let Some(parked) = self.parked.remove(&cell)
            && parked.source == raw
            && parked.compiled.lines_back(indices)
        {
            return self.install(cell, raw, parked.compiled, deps);
        }
        // Parse (context-free), then give each stored line its position in
        // today's layout (`TRAME-228`). Then resolve sheet-qualified refs and
        // named ranges against the host context; unqualified refs land on the
        // formula's own sheet (`cell.sheet`). Positional `A1` is bound to a
        // row/column `Uid` by `compile` against `indices`.
        let sheet_of = |q: Option<&SheetQualifier>| {
            let sheet = match q {
                None => Some(cell.sheet),
                Some(SheetQualifier::Id(id)) => ctx.resolve_sheet_id(id),
                Some(SheetQualifier::Name(name)) => ctx.resolve_sheet(name),
            }?;
            Some((sheet, indices.get(&sheet)?))
        };
        let index_of = |q: Option<&SheetQualifier>| sheet_of(q).map(|(_, index)| index);
        let mut lines = None;
        let parsed = parse_formula(formula_text).and_then(|ast| {
            lines = lines_it_names(&ast, &sheet_of);
            resolve_refs(unbind(ast, &index_of), cell.sheet, ctx)
        });
        self.compiles += 1;
        self.sources.remove(&cell);
        match parsed {
            Ok(ast) => {
                let follows = lines.is_some();
                let compiled = Compiled {
                    bytecode: compile(&ast, indices),
                    lines: lines.unwrap_or_default().into_boxed_slice(),
                };
                let rejected = self.install(cell, raw, compiled, deps);
                if !follows && rejected.is_none() {
                    self.positional.insert(cell);
                }
                rejected
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

    /// Take `compiled`, compiled from `raw` or parked from it, as the
    /// bytecode of `cell`, and its dependencies into the graph. Returns the
    /// error value of a formula refused for a cycle.
    fn install(
        &mut self,
        cell: CellId,
        raw: CellText<StoredForm>,
        compiled: Compiled,
        deps: &mut DependencyGraph,
    ) -> Option<CellValue> {
        let bc = &compiled.bytecode;
        match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
            Ok(()) => {
                if bc.cell_deps.iter().any(CellId::reads_a_virtual_axis) {
                    self.unbound.insert(cell);
                }
                self.entries.insert(cell, compiled);
                self.sources.insert(cell, raw);
                None // No error -- caller evaluates normally.
            }
            Err(_cycle_err) => {
                self.entries.remove(&cell);
                self.cyclic.insert(cell);
                Some(CellValue::Error(FormulaError::CircularReference))
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Compiled {
    /// Whether every cell the formula names is placed in `indices` on the
    /// elements its text names. A formula that reads positions names none,
    /// and is never parked. A compile of the parked source then binds each
    /// reference to the uid the bytecode holds, so the two agree
    /// (`TRAME-300`).
    fn lines_back(&self, indices: &HashMap<Pos<EngineSheet>, SheetIndex>) -> bool {
        self.lines.iter().all(|cell| {
            indices
                .get(&cell.sheet)
                .is_some_and(|index| cell.placed_in(index))
        })
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
