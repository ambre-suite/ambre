// ambre-trame/src/logic/fixtures.rs Trame's half of the fixture loader surface
// (`ambre-spec-fixtures.md`).
//
// `shape_of` and `canonical_dump` are declared in `ambre-logic` over `DocState`.
// Their per-app bodies live here, next to the schema they read, for the same
// reason that puts generators next to their engine. A schema change and the code
// that decodes it then move in one commit. `ambre-logic` only dispatches.
//
// Fixture code (I4): the whole module sits behind the non-default `fixtures`
// feature and is unreachable from any client build. The determinism seam
// (`TrameState::with_peer`) is *not* here -- it is engine API, ungated, and
// specified with the rest of the fixture surface in `ambre-spec-fixtures.md`.
//
// Part of the `TrameState` impl, split out by concern like `sync` and `docfile`.
// A generator aborts on a corpus it cannot build, so the panic lints of
// `ARCH-028` do not hold here.

#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "a fixture generator aborts on a corpus it cannot build"
)]

use std::collections::BTreeMap;

use ambre_core::{
    DocType, FixtureGen, FixtureShape, ShapeBody, fixture_peer_count, seeded_document_id,
};

use super::*;

impl TrameState {
    /// The coarse shape of this workbook, as the manifest records it.
    ///
    /// Counted from the CRDT rather than declared by the generator. This checks
    /// the manifest against a recomputed shape; a generator declaring its own
    /// would assert only that it agrees with itself.
    ///
    /// `formulas` counts *stored* formula sources (a leading `=`), not the
    /// engine's bytecode cache. The cache is a derived, local structure, and the
    /// manifest describes the document.
    pub fn fixture_shape(&self) -> FixtureShape {
        let mut cells = 0u32;
        let mut formulas = 0u32;
        let sheet_ids = self.fixture_sheet_ids();
        for sheet in &sheet_ids {
            if let LoroValue::Map(map) = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, sheet).as_str())
                .get_value()
            {
                for value in map.values() {
                    cells = cells.saturating_add(1);
                    if matches!(value, LoroValue::String(s) if s.starts_with('=')) {
                        formulas = formulas.saturating_add(1);
                    }
                }
            }
        }
        FixtureShape {
            peers: self.fixture_peer_count(),
            body: ShapeBody::Trame {
                cells,
                sheets: u16::try_from(sheet_ids.len()).unwrap_or(u16::MAX),
                formulas,
            },
        }
    }

    /// The canonical semantic form of this workbook: every user-visible value, in
    /// a deterministic order, as text.
    ///
    /// **Not** the CRDT: two workbooks agreeing on content still differ in op
    /// history, peer ids and commit timestamps. No byte or state comparison can
    /// therefore express "same document by a different route". This is what the
    /// frozen belt compares, and text means a failure prints a readable diff
    /// rather than a length mismatch.
    ///
    /// **Formula cells carry both representations** -- the stored source and the
    /// evaluated result. Dumping only the source would let an evaluation
    /// regression pass unnoticed; dumping only the result would hide a change in
    /// what is persisted. The source is dumped at its positions: the stored
    /// form names rows and columns by their ids, which two routes mint apart
    /// (`TRAME-228`).
    pub fn canonical_dump(&self) -> String {
        let mut out = String::from("trame\n");
        let lines = self.sheet_lines();
        for sheet in self.fixture_sheet_ids() {
            let name = self
                .sheet_id_to_name
                .get(&sheet)
                .map_or_else(|| sheet.to_string(), SheetName::to_string);
            out.push_str(&format!("sheet {sheet} {name}\n"));
            // One index per sheet: `self.index` tracks only the active sheet, and
            // a dump quantifies over all of them.
            let index = SheetIndex::build(
                &self
                    .doc
                    .get_movable_list(SheetContainer::of(SheetRoot::Rows, &sheet).as_str()),
                &self
                    .doc
                    .get_movable_list(SheetContainer::of(SheetRoot::Cols, &sheet).as_str()),
            );
            let sheet_u16 = self.engine_sheet_by_id.get(&sheet).copied();
            // Keyed by grid position, not by stored key. The stored key carries
            // peer ids and mint counters, exactly what two documents "agreeing by
            // a different route" are allowed to differ in. `BTreeMap` sorts the
            // rows/columns for free, making the order deterministic.
            let mut by_pos: BTreeMap<CellPos, String> = BTreeMap::new();
            if let LoroValue::Map(map) = self
                .doc
                .get_map(SheetContainer::of(SheetRoot::Cells, &sheet).as_str())
                .get_value()
            {
                for (key, value) in map.iter() {
                    let LoroValue::String(raw) = value else {
                        continue;
                    };
                    let raw = raw.as_str();
                    let Some((rl, cl)) = parse_cell_key(key) else {
                        continue;
                    };
                    // A cell whose row or column was deleted has no position and
                    // no user-visible existence; it is not part of the content.
                    let (Some(row), Some(col)) = (index.row_pos(&rl), index.col_pos(&cl)) else {
                        continue;
                    };
                    let label = format!(
                        "{}{}",
                        crate::columns::col_to_alpha(col),
                        row.get().saturating_add(1)
                    );
                    let line = match sheet_u16 {
                        // Both representations for a formula cell, one for a
                        // literal. `#N/A` marks a formula the engine has not
                        // evaluated -- a real difference, not one to paper over.
                        Some(idx) if raw.starts_with('=') => {
                            let cell = CellId::bound(idx, rl, cl);
                            let text = self
                                .engine
                                .evaluated_text(cell)
                                .unwrap_or_else(|| "<unevaluated>".to_owned());
                            let stored = StoredFormula::new(raw.to_owned());
                            let raw = self.formula_source(&stored, &sheet, &lines);
                            let raw = raw.as_str();
                            format!("  {label} raw={raw} text={text}\n")
                        }
                        _ => format!("  {label} {raw}\n"),
                    };
                    by_pos.insert(CellPos::new(row, col), line);
                }
            }
            for line in by_pos.values() {
                out.push_str(line);
            }
        }
        out
    }

    /// Sheet ids in a deterministic order: the tab order first, then any sheet
    /// the order list does not mention, sorted by id.
    ///
    /// Not `sheet_order_ids()`: that one is a *view* concern and pins the active
    /// sheet first. The same workbook would then dump differently depending on
    /// which tab happened to be open.
    fn fixture_sheet_ids(&self) -> Vec<SheetId> {
        let mut listed: Vec<SheetId> = match self
            .doc
            .get_movable_list(Root::SheetOrder.as_str())
            .get_value()
        {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(SheetId(s.to_string())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let mut rest: Vec<SheetId> = match self.doc.get_map(Root::Sheets.as_str()).get_value() {
            LoroValue::Map(sheets) => sheets
                .keys()
                .filter(|id| !listed.iter().any(|l| l.as_str() == id.as_str()))
                .cloned()
                .map(SheetId)
                .collect(),
            _ => Vec::new(),
        };
        rest.sort();
        listed.extend(rest);
        // A workbook always has at least the implicit default sheet, even before
        // anything registers it in the `sheets` map.
        if listed.is_empty() {
            listed.push(default_sheet());
        }
        listed
    }

    /// Number of distinct peers whose ops are present (1 for a solo document).
    /// `oplog_vv()` derefs to a map keyed by `PeerID`; the codebase already calls
    /// it on both engines' sync paths. The count drops the loader's own pinned
    /// peer, so a restored solo document still reports one author.
    fn fixture_peer_count(&self) -> u8 {
        fixture_peer_count(
            self.doc.oplog_vv().keys().copied().map(ambre_core::PeerId),
            super::scaffold_identity(),
        )
    }
}

// -----------------------------------------------------------------------------
// The correctness generators
// -----------------------------------------------------------------------------

/// Which workbook a generator builds.
///
/// One closed enum plus a `match`, rather than one type per document: the
/// registry holds trait objects, so the alternative is four near-identical impl
/// blocks. Here the compiler checks that every variant declares a name, a seed,
/// a version and a body.
///
/// Each variant is a *correctness* corpus. It rebuilds inside the
/// `just fixtures` budget, and it takes the shape of one feature a test would
/// exercise. The scale corpora belong to `FIXTURES P2` and stay uncommitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrameFixture {
    /// A dense literal grid with a header row, one merge and one inserted row.
    /// The plain workbook, and the one whose structural operations shifted the
    /// stable ids away from their grid positions.
    GridLiterals,
    /// A 40-deep formula chain over a literal column, plus three aggregate
    /// cells. The document a recomputation test starts from, and the one whose
    /// canonical dump carries both cell representations (`FIXTURE-017`).
    FormulaDag,
    /// Three sheets, a tab order that is not the creation order, and a named
    /// range a formula reads.
    MultiSheet,
    /// The post-convergence workbook: two pinned peers edit disjoint cells and
    /// exchange snapshots both ways. Its manifest reports two peers, which is
    /// what no single-peer generator can produce.
    TwoPeerMerge,
}

/// Trame's half of the shipping registry (`ambre-logic`'s `REGISTRY`).
///
/// Declared next to the generators rather than spelled out in `ambre-logic`.
/// A generator added here reaches the corpus without a second edit one crate
/// down -- the edit a contributor forgets.
pub const TRAME_FIXTURES: &[&dyn FixtureGen] = &[
    &TrameFixture::GridLiterals,
    &TrameFixture::FormulaDag,
    &TrameFixture::MultiSheet,
    &TrameFixture::TwoPeerMerge,
];

impl FixtureGen for TrameFixture {
    fn name(&self) -> &'static str {
        match self {
            Self::GridLiterals => "trame/grid_literals",
            Self::FormulaDag => "trame/formula_dag",
            Self::MultiSheet => "trame/multi_sheet",
            Self::TwoPeerMerge => "trame/two_peer_merge",
        }
    }

    fn doc_type(&self) -> DocType {
        DocType::Trame
    }

    fn seed(&self) -> u64 {
        // Distinct per generator, so no two artifacts share a `document_id` and
        // a copied file is visible in the diff rather than plausible.
        match self {
            Self::GridLiterals => 0x5EED_7A1E_0000_0001,
            Self::FormulaDag => 0x5EED_7A1E_0000_0002,
            Self::MultiSheet => 0x5EED_7A1E_0000_0003,
            Self::TwoPeerMerge => 0x5EED_7A1E_0000_0004,
        }
    }

    fn generator_version(&self) -> u32 {
        1
    }

    fn build(&self) -> ambre_core::PlaintextContainer {
        let seed = self.seed();
        let mut state = TrameState::with_peer(ambre_core::PeerId(seed));
        match self {
            Self::GridLiterals => build_grid_literals(&mut state),
            Self::FormulaDag => build_formula_dag(&mut state),
            Self::MultiSheet => build_multi_sheet(&mut state),
            // The only body that needs a second state, and therefore the only
            // one that does not fit the shared `state` above.
            Self::TwoPeerMerge => build_two_peer_merge(&mut state, seed),
        }
        state
            .export_ambre_plaintext(seeded_document_id(seed))
            .expect("the document reads")
    }
}

/// 20 rows by 8 columns of literals, then one merge and one inserted row.
///
/// The structural operations are the point. They move the stable row ids away
/// from their grid positions. A fixture from this artifact then exercises the
/// index, not a grid whose positions and ids still agree.
fn build_grid_literals(state: &mut TrameState) {
    let header = ["region", "q1", "q2", "q3", "q4", "target", "owner", "note"];
    let at = |row: u32, col: u32| CellPos::new(Row::new(row), Col::new(col));
    let mut cells: Vec<(CellPos, String)> = header
        .iter()
        .zip(0..)
        .map(|(text, col)| (at(0, col), (*text).to_owned()))
        .collect();
    cells.extend((1..20).flat_map(|row| {
        (0..8).map(move |col| {
            let text = match col {
                0 => format!("region {row}"),
                6 => format!("owner {}", row % 4),
                7 => String::new(),
                _ => (row * 100 + col).to_string(),
            };
            (at(row, col), text)
        })
    }));
    state.handle_command(TrameCommand::SetCellsBatch { cells });
    // A merged title band over the header, then a row inserted under it.
    state.handle_command(TrameCommand::MergeCells {
        rect: crate::CellRect::spanning(
            crate::CellPos::new(Row::ZERO, Col::new(1)),
            crate::CellPos::new(Row::ZERO, Col::new(4)),
        ),
    });
    state.handle_command(TrameCommand::InsertRow { at: Row::new(1) });
}

/// A 40-deep dependency chain plus three aggregates.
///
/// Deep rather than wide. A recomputation order defect shows up in a chain, and
/// one wrong link changes every cell below it in the dump.
fn build_formula_dag(state: &mut TrameState) {
    const DEPTH: u32 = 40;
    let at = |row: u32, col: u32| CellPos::new(Row::new(row), Col::new(col));
    let mut cells: Vec<(CellPos, String)> = (0..DEPTH)
        .map(|row| (at(row, 0), (row + 1).to_string()))
        .collect();
    // B1 seeds the chain; every later B reads the A beside it and the B above.
    cells.push((at(0, 1), "=A1".to_owned()));
    cells.extend((1..DEPTH).map(|row| (at(row, 1), format!("=A{}+B{}", row + 1, row))));
    cells.push((at(0, 2), format!("=SUM(A1:A{DEPTH})")));
    cells.push((at(1, 2), format!("=AVERAGE(A1:A{DEPTH})")));
    cells.push((at(2, 2), format!("=IF(C1>100,B{DEPTH},C2)")));
    state.handle_command(TrameCommand::SetCellsBatch { cells });
}

/// Three sheets, a tab order that is not the creation order, and a named range.
///
/// A formula on the defining sheet reads the named range. A cross-sheet
/// reference binds to the display name, a known gap (`trame-sheets`). The
/// fixture would record a parse error rather than a feature.
fn build_multi_sheet(state: &mut TrameState) {
    let at = |row: u32| CellPos::new(Row::new(row), Col::ZERO);
    state.handle_command(TrameCommand::SetCellsBatch {
        cells: (0..6)
            .map(|row| (at(row), format!("summary {row}")))
            .collect(),
    });
    for name in ["Data", "Notes"] {
        let outcome = state.handle_command(TrameCommand::CreateSheet {
            name: SheetName::new(name).expect("a fixed name"),
        });
        let sheet_id = new_sheet_id(&outcome, name);
        state.handle_command(TrameCommand::ActivateSheet {
            sheet_id: sheet_id.clone(),
        });
        let cells = (0..8)
            .map(|row| {
                let text = match name {
                    "Data" => (row * 7 + 1).to_string(),
                    _ => format!("note {row}"),
                };
                (at(row), text)
            })
            .collect();
        state.handle_command(TrameCommand::SetCellsBatch { cells });
        if name == "Data" {
            state.handle_command(TrameCommand::DefineName {
                name: crate::DefinedName::new("readings").expect("a fixed name"),
                // Qualified by display name: the definition is stored under the
                // sheet's id, so it survives a rename (`TRAME-067`).
                reference: "Data!A1:A8".to_owned(),
            });
            state.handle_command(TrameCommand::SetCellText {
                row: Row::ZERO,
                col: Col::new(2),
                text: "=SUM(readings)".to_owned(),
            });
            // The tab order stops matching the creation order, so a dump that
            // walked the `sheets` map instead of the order list is visible.
            state.handle_command(TrameCommand::MoveSheet {
                sheet_id,
                to_index: SheetTabPos::ZERO,
            });
        }
    }
}

/// Two pinned peers edit disjoint cells and exchange snapshots both ways.
///
/// The artifact leaves from `state`; the second peer lives here and ends with
/// the call. The generator replays both directions, so the exported history
/// carries both peers and the manifest reports two (`FIXTURE-028`).
///
/// The two peers materialize their rows and columns independently, so the merged
/// grid concatenates both axes (20 rows, 6 columns) instead of overlapping them.
/// That is what merging two documents born apart looks like, and the corpus wants
/// it on record.
fn build_two_peer_merge(state: &mut TrameState, seed: u64) {
    let mut other = TrameState::with_peer(ambre_core::PeerId(seed ^ 0xFFFF_FFFF));
    state.handle_command(TrameCommand::SetCellsBatch {
        cells: (0..10)
            .map(|row| {
                (
                    CellPos::new(Row::new(row), Col::ZERO),
                    format!("local {row}"),
                )
            })
            .collect(),
    });
    other.handle_command(TrameCommand::SetCellsBatch {
        cells: (0..10)
            .map(|row| {
                (
                    CellPos::new(Row::new(row), Col::new(3)),
                    format!("remote {row}"),
                )
            })
            .collect(),
    });
    other
        .import_snapshot(&state.export_snapshot())
        .expect("the snapshot imports");
    state
        .import_snapshot(&other.export_snapshot())
        .expect("the snapshot imports");
    // One edit after convergence, so the artifact holds ops that follow a merge
    // rather than only ops that precede one. The formula reads one cell from each
    // peer, so a reload that lost either side's ops changes the result.
    state.handle_command(TrameCommand::SetCellText {
        row: Row::ZERO,
        col: Col::new(5),
        text: "=A1&E11".to_owned(),
    });
}

/// The id of the sheet a `CreateSheet` just minted, read off the refreshed
/// `SheetList`.
///
/// Panics when the event is absent or names no such sheet. A generator drives
/// commands it chose itself, so a missing reply is a defect here rather than
/// a runtime condition (`FixtureGen::build`).
fn new_sheet_id(outcome: &TrameOutcome, name: &str) -> SheetId {
    outcome
        .display
        .iter()
        .find_map(|event| match event {
            TrameEvent::SheetList { sheets, .. } => sheets
                .iter()
                .find(|tab| tab.name.as_str() == name)
                .map(|tab| tab.id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("CreateSheet must report the sheet it minted: {name}"))
}

// -----------------------------------------------------------------------------
// The scale generators
// -----------------------------------------------------------------------------

/// Which workbook a scale generator builds (`FIXTURE-022`).
///
/// A second enum beside [`TrameFixture`], not two more variants of it. The two
/// registries go to two directories. Git carries a correctness artifact and
/// never a scale one. One registry per destination therefore keeps
/// `just fixtures` from writing a multi-megabyte file into the working copy.
///
/// * Every variant takes a figure `ambre-spec-budgets.md` already states. This
///   module invents no size and repeats no target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrameBenchFixture {
    /// 100 000 literal cells over 12 500 rows. The document size and cache
    /// budgets name that figure, and the snapshot import and index rebuild
    /// benches read the same artifact.
    DenseGrid100k,
    /// 100 000 formula cells over 12 500 rows, on two literal source columns.
    /// The DAG and cache memory budget counts formula cells, so the figure it
    /// names has to be the formula count and not the cell count.
    FormulaGrid100k,
    /// 10 000 formula cells in five cascading columns over 2 000 rows. The
    /// recompute budget names 10 000 cells; five columns make the walk wide
    /// rather than deep, which is the shape a recompute meets.
    FormulaDag10k,
    /// 10 000 cells over 1 250 rows by 8 columns. The CSV and xlsx export
    /// budgets name that figure.
    ExportSheet10k,
}

/// Trame's half of the scale registry (`ambre-logic`'s `BENCH_REGISTRY`).
///
/// Declared next to the generators, like [`TRAME_FIXTURES`]. A generator added
/// here reaches the corpus with no second edit one crate down.
pub const TRAME_BENCH_FIXTURES: &[&dyn FixtureGen] = &[
    &TrameBenchFixture::DenseGrid100k,
    &TrameBenchFixture::FormulaGrid100k,
    &TrameBenchFixture::FormulaDag10k,
    &TrameBenchFixture::ExportSheet10k,
];

impl FixtureGen for TrameBenchFixture {
    fn name(&self) -> &'static str {
        match self {
            Self::DenseGrid100k => "trame/bench_dense_grid_100k",
            Self::FormulaGrid100k => "trame/bench_formula_grid_100k",
            Self::FormulaDag10k => "trame/bench_formula_dag_10k",
            Self::ExportSheet10k => "trame/bench_export_sheet_10k",
        }
    }

    fn doc_type(&self) -> DocType {
        DocType::Trame
    }

    fn seed(&self) -> u64 {
        // A range of its own, disjoint from the correctness seeds above, so no
        // scale artifact shares a `document_id` with a committed one.
        match self {
            Self::DenseGrid100k => 0x5EED_B0C4_0000_0001,
            Self::FormulaGrid100k => 0x5EED_B0C4_0000_0002,
            Self::FormulaDag10k => 0x5EED_B0C4_0000_0003,
            Self::ExportSheet10k => 0x5EED_B0C4_0000_0004,
        }
    }

    fn generator_version(&self) -> u32 {
        1
    }

    fn build(&self) -> ambre_core::PlaintextContainer {
        let seed = self.seed();
        let mut state = TrameState::with_peer(ambre_core::PeerId(seed));
        match self {
            Self::DenseGrid100k => build_literal_grid(&mut state, 12_500, 8),
            Self::FormulaGrid100k => build_formula_grid(&mut state),
            Self::FormulaDag10k => build_formula_cascade(&mut state),
            Self::ExportSheet10k => build_literal_grid(&mut state, 1_250, 8),
        }
        state
            .export_ambre_plaintext(seeded_document_id(seed))
            .expect("the document reads")
    }
}

/// `rows` by `cols` of literal cells, written as one batch.
///
/// One batch rather than one command per cell. A command derives the viewport,
/// so a cell-at-a-time build costs the square of the cell count and produces the
/// same document.
fn build_literal_grid(state: &mut TrameState, rows: u32, cols: u32) {
    let cells: Vec<(CellPos, String)> = (0..rows)
        .flat_map(|row| {
            (0..cols).map(move |col| {
                // A mix of text and numbers. An all-numeric grid compresses far
                // better than a real sheet, and the size budget reads the
                // compressed artifact.
                let text = match col {
                    0 => format!("row {row}"),
                    7 => format!("owner {}", row % 37),
                    _ => (row * 31 + col * 7).to_string(),
                };
                (CellPos::new(Row::new(row), Col::new(col)), text)
            })
        })
        .collect();
    state.handle_command(TrameCommand::SetCellsBatch { cells });
}

/// 100 000 formulas over 12 500 rows, on two literal source columns.
///
/// Eight formula columns per row, so the formula count is the figure the memory
/// budget names. Counting the literal sources into it instead would report a DAG
/// a fifth smaller than the one the budget is about.
///
/// The eight differ in shape: an addition, a product, a reference chain, a
/// conditional and a range aggregate. A cache sized on one expression kind
/// measures that kind rather than a workbook.
fn build_formula_grid(state: &mut TrameState) {
    const ROWS: u32 = 12_500;
    let at = |row: u32, col: u32| CellPos::new(Row::new(row), Col::new(col));
    let mut cells: Vec<(CellPos, String)> = Vec::with_capacity(125_000);
    for row in 0..ROWS {
        let line = row + 1;
        cells.push((at(row, 0), (row * 13 + 1).to_string()));
        cells.push((at(row, 1), (row * 13 + 2).to_string()));
        cells.push((at(row, 2), format!("=A{line}+B{line}")));
        cells.push((at(row, 3), format!("=C{line}*2")));
        cells.push((at(row, 4), format!("=C{line}+D{line}")));
        cells.push((at(row, 5), format!("=IF(E{line}>100,E{line},D{line})")));
        cells.push((at(row, 6), format!("=SUM(A{line}:B{line})")));
        cells.push((at(row, 7), format!("=AVERAGE(C{line}:F{line})")));
        cells.push((at(row, 8), format!("=G{line}+H{line}")));
        cells.push((at(row, 9), format!("=I{line}-A{line}")));
    }
    state.handle_command(TrameCommand::SetCellsBatch { cells });
}

/// 10 000 formulas in five cascading columns over 2 000 rows.
///
/// Wide rather than deep. A 10 000-long chain measures recursion depth; five
/// columns over 2 000 rows measure the topological walk, which is what the
/// recompute budget is about.
fn build_formula_cascade(state: &mut TrameState) {
    const ROWS: u32 = 2_000;
    const CASCADE: u32 = 5;
    let at = |row: u32, col: u32| CellPos::new(Row::new(row), Col::new(col));
    let mut cells: Vec<(CellPos, String)> = Vec::with_capacity(12_000);
    for row in 0..ROWS {
        let line = row + 1;
        cells.push((at(row, 0), (row + 1).to_string()));
        // Every later column reads the one beside it and the row above, so a
        // wrong walk order changes the whole cascade rather than one cell.
        for col in 1..=CASCADE {
            let previous = crate::columns::col_to_alpha(Col::new(col - 1));
            let text = if row == 0 {
                format!("={previous}{line}")
            } else {
                let here = crate::columns::col_to_alpha(Col::new(col));
                format!("={previous}{line}+{here}{row}")
            };
            cells.push((at(row, col), text));
        }
    }
    state.handle_command(TrameCommand::SetCellsBatch { cells });
}
