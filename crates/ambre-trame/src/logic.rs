// ambre-trame/src/logic.rs
// ============================================================================
// Trame (spreadsheet) per-document model -- Logic thread side.
//
// `TrameState` owns everything the Logic thread needs for one open Trame
// document: CRDT, formula engine, formatting. The Logic dispatcher
// (`ambre-logic`) holds it inside `DocState::Trame` and calls the handlers
// below with **typed** `TrameCommand`s, getting back typed `TrameEvent`s --
// no trait object, no postcard, no `Vec<u8>` on this path.
//
// The shared IPC types (TrameCommand, TrameEvent, DisplayCell) live in
// `ambre-bus`; the grid dimensions shared with the UI live in the crate root
// (`crate::TOTAL_ROWS` / `crate::TOTAL_COLS`).
// ============================================================================

use std::collections::{HashMap, HashSet};

use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use crate::formula::{render, AxisId, CellId, FormulaCtx, FormulaEngine, NameTarget, SheetIndices};
use crate::formula::types::CellValue;
use crate::formula::types::Expr;
use crate::formula::parser::{parse_formula, resolve_refs};
use crate::model::{SheetIndex, Uid, cell_key, parse_cell_key};
use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

/// Stable id of the implicit default sheet created by [`TrameState::new`].
///
/// A single constant (not a per-instance generated id) so that two fresh
/// `TrameState`s -- the document creator and a peer/reload restoring its
/// snapshot -- address the *same* per-sheet containers (`cells_s0`, `rows_s0`,
/// ...) and merge cleanly. Explicit additional sheets (a later step) get unique
/// ids and populate `sheet_order`; with one sheet, `sheet_order` is left empty
/// to avoid a duplicate `push` on merge (see `ambre-spec-trame-logic.md` §2).
const DEFAULT_SHEET: &str = "s0";

/// Loro map holding sheet metadata: `{sheet_id -> name}`. Step 1 stores the
/// name flat; the `{name: LoroText, hidden, tab_color}` sub-map of §2.B lands
/// with the sheet-tabs UI.
const SHEETS_MAP: &str = "sheets";

/// `LoroMovableList` holding the tab-strip order as a sequence of `sheet_id`s
/// (§2.B/§2.D). The implicit [`DEFAULT_SHEET`] is *not* eagerly pushed here (two
/// fresh peers would each push it and converge to a duplicate); it is treated as
/// the implicit first tab by [`TrameState::sheet_order_ids`]. Explicit sheets
/// are appended on [`TrameCommand::CreateSheet`].
const SHEET_ORDER: &str = "sheet_order";

/// Loro map holding workbook named ranges: `{name -> reference}` where the
/// reference is an A1-syntax string, optionally sheet-qualified (`Sheet2!A1:B2`).
/// LWW: the last writer of a given name wins. Resolved positionally at reseed
/// time into [`TrameState::named`] (`ambre-spec-trame-logic.md` §8, Phase 2).
const NAMED_RANGES_MAP: &str = "named_ranges";

/// Bit flags packed into the per-sheet `format_{sheet}` map's i64 value.
const FMT_BOLD: i64 = 1 << 0;
const FMT_ITALIC: i64 = 1 << 1;

/// Maximum number of rows/columns a single write may materialize past the
/// current used-range high-water mark. Bounds the pathological "lone value far
/// down the grid" case that would otherwise create ~100k placeholder elements
/// (see `ambre-spec-trame-logic.md` §2.A bis). A write beyond it is dropped.
const MATERIALIZE_CAP: usize = 10_000;

/// Commit origin tagging the implicit row/column scaffolding that a cell edit
/// forces into existence (see [`TrameState::materialize`]). The `UndoManager` is
/// told to exclude this prefix, so that scaffolding never lands on the undo
/// stack -- only the user-intended value change does. Explicit structural ops
/// (`InsertRow`, etc.) use the default origin and stay undoable.
const MATERIALIZE_ORIGIN: &str = "mat";

/// Per-sheet container names (`cells_{sheet}`, etc.). Keyed by sheet id so a
/// multi-sheet document keeps each sheet's cells/rows/columns/formatting
/// isolated (§2.B).
fn cells_map_name(sheet: &str) -> String {
    format!("cells_{sheet}")
}
fn format_map_name(sheet: &str) -> String {
    format!("format_{sheet}")
}
fn rows_list_name(sheet: &str) -> String {
    format!("rows_{sheet}")
}
fn cols_list_name(sheet: &str) -> String {
    format!("cols_{sheet}")
}
/// Per-sheet merge map (`merges_{sheet}`): a LWW `LoroMap` keyed by the master
/// (top-left) cell's stable id pair, with the bottom-right corner's id pair as
/// value (`ambre-spec-trame-logic.md` §6). Keyed by stable ids so a merge
/// follows its cells across inserts/deletes.
fn merges_map_name(sheet: &str) -> String {
    format!("merges_{sheet}")
}

/// Which axis a structural op targets. Lets [`TrameState::insert_axis`] /
/// [`delete_axis`](TrameState::delete_axis) share one body across rows/columns.
#[derive(Clone, Copy)]
enum Axis {
    Row,
    Col,
}

// -----------------------------------------------------------------------------
// FormulaCtx bridge
// -----------------------------------------------------------------------------

/// Resolution context handed to the formula engine: maps a sheet *display name*
/// to its local engine index and a defined name to its target. Borrows the
/// derived caches on [`TrameState`] so it can be built at a call site without
/// borrowing the whole state (the engine, a sibling field, is mutated in the
/// same call).
struct TrameCtx<'a> {
    sheet_name_to_u16: &'a HashMap<String, u16>,
    sheet_id_to_u16: &'a HashMap<String, u16>,
    named: &'a HashMap<String, NameTarget>,
}

impl FormulaCtx for TrameCtx<'_> {
    fn resolve_sheet(&self, name: &str) -> Option<u16> {
        self.sheet_name_to_u16.get(name).copied()
    }
    fn resolve_sheet_id(&self, id: &str) -> Option<u16> {
        self.sheet_id_to_u16.get(id).copied()
    }
    fn resolve_name(&self, name: &str) -> Option<NameTarget> {
        self.named.get(name).cloned()
    }
}

// -----------------------------------------------------------------------------
// TrameOutcome -- typed editing result
// -----------------------------------------------------------------------------

/// Outcome of processing a [`TrameCommand`]: display events for the UI Thread,
/// plus optional CRDT and ephemeral payloads to forward to the Sync thread.
///
/// The Logic dispatcher sends `display` to the UI, `update` / `ephemeral` to
/// Sync as [`SyncCommand::SendUpdate`] / [`SyncCommand::SendEphemeral`].  This
/// is the *lossless* path required by the architecture: no app content is
/// silently dropped.
/// A resolvable merge rectangle: master key, inclusive top-left `(row, col)`,
/// and inclusive bottom-right `(row, col)` in grid positions.
type MergeRect = (String, (u32, u32), (u32, u32));

pub struct TrameOutcome {
    /// Display events destined for the UI Thread.
    pub display: Vec<TrameEvent>,
    /// Binary CRDT delta (Loro `ExportMode::Updates`) to encrypt and broadcast.
    pub update: Option<Vec<u8>>,
    /// Ephemeral (presence) payload to encrypt and broadcast.
    pub ephemeral: Option<Vec<u8>>,
}

impl TrameOutcome {
    /// Convenience: display events only, no sync payloads.
    pub fn display_only(events: Vec<TrameEvent>) -> Self {
        Self { display: events, update: None, ephemeral: None }
    }
}

// -----------------------------------------------------------------------------
// TrameState
// -----------------------------------------------------------------------------

/// Per-document state for a Trame (spreadsheet) document.
///
/// One instance per open document, owned by the Logic thread's `DocState`
/// registry keyed by `document_id`.
pub struct TrameState {
    /// Active cell (selection), kept in sync by the UI via
    /// [`TrameCommand::SetActiveCell`]. Formatting toggles target this cell.
    active: (u32, u32),
    /// Id of the sheet the UI is currently looking at. Step 1 always holds the
    /// implicit [`DEFAULT_SHEET`]; multi-sheet switching arrives with the tabs.
    active_sheet: String,
    /// Cells carrying a bold run on the active sheet. Keyed by grid `(row, col)`;
    /// absence means regular weight. Mirrors the persisted `format_{sheet}` map
    /// (the CRDT source of truth) for fast lookup; reseeded on import.
    bold: HashSet<(u32, u32)>,
    /// Cells carrying an italic run on the active sheet.
    italic: HashSet<(u32, u32)>,
    /// The whole workbook CRDT: multi-sheet schema (§2.B). Cells are keyed by
    /// stable `(row_lid, col_lid)` pairs -- see [`crate::model`].
    doc: LoroDoc,
    /// Position <-> stable-id cache for the **active sheet**'s rows/columns.
    /// Rebuilt by [`Self::rebuild_index`] after any import; mutated in place by
    /// the write path as it materializes rows/columns.
    index: SheetIndex,
    /// Last CRDT version vector exported to Sync. Used to produce incremental
    /// deltas via [`ExportMode::Updates`].
    last_exported_vv: VersionVector,
    /// Formula engine: dependency graph, bytecode cache, dirty recomputation.
    /// Addresses cells positionally; [`Self::index`] bridges to the CRDT ids.
    engine: FormulaEngine,
    /// Active-sheet merges, master grid position -> bottom-right grid position.
    /// Derived cache of `merges_{sheet}`, rebuilt by [`Self::rebuild_merges`]
    /// after any import or structural op (§6).
    merges: HashMap<(u32, u32), (u32, u32)>,
    /// Grid positions covered by a merge but not the master (rendered blank,
    /// swallowed by the master cell). Derived alongside [`Self::merges`].
    covered: HashSet<(u32, u32)>,
    /// Stable cell keys of covered cells that still hold a non-empty value -- a
    /// concurrent-edit conflict (§6). Masked: excluded from the formula engine
    /// and not displayed, but never destroyed; reappear on unmerge.
    masked_cells: HashSet<String>,
    /// CRDT sheet id per local engine index (`sheets_by_idx[i]` has index `i`).
    /// The **active sheet is always index 0**, so single-sheet code paths keep
    /// using [`CellRef::new`] (sheet 0); other sheets take 1.. in sorted-id
    /// order. Rebuilt by [`Self::rebuild_sheet_maps`].
    sheets_by_idx: Vec<String>,
    /// Sheet *display name* -> local engine index, for resolving `Sheet2!A1`
    /// references in formulas. Derived alongside [`Self::sheets_by_idx`].
    sheet_name_to_u16: HashMap<String, u16>,
    /// Stable `sheet_id` -> local engine index, for resolving the canonical
    /// `#s0!A1` form persisted in the CRDT `v` text (rename-stable, §2.D).
    /// Derived alongside [`Self::sheets_by_idx`].
    sheet_id_to_u16: HashMap<String, u16>,
    /// Stable `sheet_id` -> current display name, for reconstructing formula-bar
    /// text from the canonical id form. Derived alongside [`Self::sheets_by_idx`].
    sheet_id_to_name: HashMap<String, String>,
    /// Resolved named ranges: defined name -> target cell/range. Rebuilt from the
    /// `named_ranges` CRDT map by [`Self::rebuild_named`] on every reseed.
    named: HashMap<String, NameTarget>,
    /// Local undo/redo history (§8). Subscribes to [`Self::doc`] and records one
    /// item per commit; each `TrameCommand` handler commits exactly once, so one
    /// command maps to one undo step. Scope is the open-document session: it only
    /// knows this peer's commits, not imported remote ops. `undo`/`redo` mutate
    /// it, hence it is created after the constructor's initial commit so that the
    /// implicit-sheet registration is never undoable.
    undo_manager: UndoManager,
    /// Per-session monotonic counter for minting explicit `sheet_id`s. Combined
    /// with the document `PeerID` so two peers that concurrently add a sheet mint
    /// distinct ids that both survive (§2.D). Not persisted: ids, once minted,
    /// live in the CRDT; the counter only needs to be unique within this session.
    sheet_counter: u32,
    /// Per-session monotonic counter for minting row/column [`Uid`]s. Same
    /// peer-scoped convergence argument as `sheet_counter` (§2.A): two peers that
    /// concurrently materialize/insert a line mint distinct uids that both
    /// survive. Not persisted -- once minted, a uid lives in the CRDT as the list
    /// element's value.
    uid_counter: u32,
}

impl TrameState {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        // Register the implicit default sheet's name (idempotent map insert, so
        // a peer that also "creates" s0 converges to the same entry).
        let _ = doc.get_map(SHEETS_MAP).insert(DEFAULT_SHEET, "Sheet1");
        doc.commit();
        // Create the undo manager AFTER the initial commit: the implicit-sheet
        // registration is document scaffolding, not a user edit, so it must not
        // sit on the undo stack.
        let mut undo_manager = UndoManager::new(&doc);
        // Keep implicit row/column materialization off the undo stack (see
        // [`MATERIALIZE_ORIGIN`]); only user-intended edits are undoable.
        undo_manager.add_exclude_origin_prefix(MATERIALIZE_ORIGIN);
        let mut state = Self {
            active: (0, 0),
            active_sheet: DEFAULT_SHEET.to_owned(),
            bold: HashSet::new(),
            italic: HashSet::new(),
            doc,
            index: SheetIndex::default(),
            last_exported_vv: VersionVector::default(),
            engine: FormulaEngine::new(),
            merges: HashMap::new(),
            covered: HashSet::new(),
            masked_cells: HashSet::new(),
            sheets_by_idx: Vec::new(),
            sheet_name_to_u16: HashMap::new(),
            sheet_id_to_u16: HashMap::new(),
            sheet_id_to_name: HashMap::new(),
            named: HashMap::new(),
            undo_manager,
            sheet_counter: 0,
            uid_counter: 0,
        };
        // Index the implicit sheet so `=Sheet1!A1` resolves before any reseed.
        state.rebuild_sheet_maps();
        state
    }
}

impl Default for TrameState {
    fn default() -> Self {
        Self::new()
    }
}

impl TrameState {
    /// Display text for a cell: the formula evaluation result if it's a formula
    /// cell, otherwise the edited value from the CRDT if present, otherwise
    /// empty (a virtual or cleared cell renders blank).
    fn cell_text(&self, row: u32, col: u32) -> String {
        let cell = self.active_cell_id(row, col);
        // Formula cells: return cached evaluation result. Plain cells also have
        // a cached literal value now (so formula references resolve), but we
        // render their raw CRDT string verbatim to avoid normalizing user input.
        if self.engine.is_formula_cell(cell)
            && let Some(text) = self.engine.evaluated_text(cell)
        {
            return text;
        }
        // Plain-text cells: resolve the grid position to its stable id pair and
        // read the CRDT value. A virtual (never-written) cell, or one cleared
        // back to empty, has no stored value and renders blank.
        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
            let map = self.doc.get_map(cells_map_name(&self.active_sheet));
            if let Some(LoroValue::String(s)) = map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value()) {
                return s.to_string();
            }
        }
        String::new()
    }

    /// Raw cell text as stored in the CRDT (the formula source for a formula
    /// cell, the literal otherwise). Empty for virtual/cleared cells.
    fn cell_raw(&self, row: u32, col: u32) -> String {
        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
            let map = self.doc.get_map(cells_map_name(&self.active_sheet));
            if let Some(LoroValue::String(s)) = map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value()) {
                return s.to_string();
            }
        }
        String::new()
    }

    /// Rewrite user-entered formula text into its canonical CRDT form: each
    /// cross-sheet qualifier is bound to the stable `sheet_id`
    /// (`Sheet2!A1` -> `#s0!A1`), so a later rename/reorder never invalidates the
    /// reference (§2.D). Plain text, formulas without a sheet qualifier, and
    /// formulas that fail to parse are stored verbatim (the user's exact input).
    fn canonicalize_input(&self, text: &str) -> String {
        let Some(body) = text.strip_prefix('=') else {
            return text.to_string();
        };
        // A cross-sheet reference always carries a `!`; nothing else to rewrite.
        if !body.contains('!') {
            return text.to_string();
        }
        let name_to_id = |name: &str| {
            self.sheet_name_to_u16
                .get(name)
                .and_then(|idx| self.sheets_by_idx.get(*idx as usize))
                .cloned()
        };
        match parse_formula(body) {
            Ok(ast) => format!("={}", render::render(&render::canonicalize(ast, &name_to_id))),
            Err(_) => text.to_string(),
        }
    }

    /// Reconstruct the formula-bar text for a stored CRDT value: canonical `#id`
    /// qualifiers are rendered through the current `sheet_id -> name` map
    /// (`#s0!A1` -> `Data!A1`), so a rename is reflected immediately with no CRDT
    /// mutation. A deleted target stays in `#id` form, signalling the dangling
    /// reference. Plain text and formulas without an id qualifier pass through.
    fn formula_display(&self, stored: &str) -> String {
        let Some(body) = stored.strip_prefix('=') else {
            return stored.to_string();
        };
        // The canonical id qualifier is the only `#` we persist in `v`.
        if !body.contains('#') {
            return stored.to_string();
        }
        let id_to_name = |id: &str| self.sheet_id_to_name.get(id).cloned();
        match parse_formula(body) {
            Ok(ast) => format!("={}", render::render(&render::to_display_form(ast, &id_to_name))),
            Err(_) => stored.to_string(),
        }
    }

    /// Ensure grid `(row, col)` is materialized (its row and column exist in the
    /// `rows_/cols_` lists) and return the `(row_lid, col_lid)` pair to key the
    /// cell with. Extends the dense used range by pushing placeholder elements,
    /// capturing each new element's stable id. Returns `None` -- dropping the
    /// write -- if the jump past the high-water mark exceeds [`MATERIALIZE_CAP`].
    /// Mint a fresh, peer-scoped row/column [`Uid`] (§2.A). The pair
    /// `(document PeerID, per-session counter)` is unique across peers, so
    /// concurrent materialize/insert ops never collide.
    fn mint_uid(&mut self) -> Uid {
        let uid = Uid { peer: self.doc.peer_id(), counter: self.uid_counter };
        self.uid_counter += 1;
        uid
    }

    fn materialize(&mut self, row: u32, col: u32) -> Option<(Uid, Uid)> {
        let (row, col) = (row as usize, col as usize);
        if row >= self.index.row_len() && row - self.index.row_len() >= MATERIALIZE_CAP
            || col >= self.index.col_len() && col - self.index.col_len() >= MATERIALIZE_CAP
        {
            tracing::warn!(
                "[Logic] materialize ({row},{col}) exceeds cap past hwm \
                 (rows {}, cols {}); write dropped",
                self.index.row_len(),
                self.index.col_len(),
            );
            return None;
        }
        let mut grew = false;
        while self.index.row_len() <= row {
            let uid = self.mint_uid();
            let rows = self.doc.get_movable_list(rows_list_name(&self.active_sheet));
            rows.push(uid.to_key_part().as_str()).ok()?;
            self.index.push_row(uid);
            grew = true;
        }
        while self.index.col_len() <= col {
            let uid = self.mint_uid();
            let cols = self.doc.get_movable_list(cols_list_name(&self.active_sheet));
            cols.push(uid.to_key_part().as_str()).ok()?;
            self.index.push_col(uid);
            grew = true;
        }
        // Commit the implicit scaffolding under the excluded origin so it stays
        // OUT of the undo stack (see [`MATERIALIZE_ORIGIN`]). A cell edit's intent
        // is the value, not the row/column placeholders it forced into existence;
        // keeping those off the stack means undo reverts only the value and, on
        // redo, the cell's key still resolves to the same (never-removed) list
        // element ids -- without this split, Loro would recreate those elements
        // with fresh ids on redo and orphan the cell. A no-op when nothing grew.
        if grew {
            self.doc.set_next_commit_origin(MATERIALIZE_ORIGIN);
            self.doc.commit();
        }
        self.index.resolve(row, col)
    }

    /// Rebuild [`Self::index`] from the active sheet's `rows_/cols_` lists.
    /// Called after any import, where peers' (or our own persisted) structural
    /// ops may have added rows/columns the cache does not yet know about.
    fn rebuild_index(&mut self) {
        let rows = self.doc.get_movable_list(rows_list_name(&self.active_sheet));
        let cols = self.doc.get_movable_list(cols_list_name(&self.active_sheet));
        self.index = SheetIndex::build(&rows, &cols);
    }

    /// Build the per-sheet position<->uid index map the formula engine needs to
    /// resolve references. The active sheet is local index 0 (reusing the
    /// maintained [`Self::index`]); other sheets are rebuilt on demand. Local
    /// indices match [`Self::sheets_by_idx`] (active pinned first).
    fn engine_indices(&self) -> SheetIndices {
        let mut m = SheetIndices::new();
        for (idx, id) in self.sheets_by_idx.iter().enumerate() {
            let index = if idx == 0 { self.index.clone() } else { self.build_sheet_index(id) };
            m.insert(idx as u16, index);
        }
        m
    }

    /// Engine [`CellId`] for grid `(row, col)` on the active sheet (local index
    /// 0): each axis binds to its row/column `Uid` when materialized, else stays
    /// a virtual grid position.
    fn active_cell_id(&self, row: u32, col: u32) -> CellId {
        let r = match self.index.row_id(row as usize) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(row),
        };
        let c = match self.index.col_id(col as usize) {
            Some(u) => AxisId::Bound(u),
            None => AxisId::Virtual(col),
        };
        CellId { sheet: 0, row: r, col: c }
    }

    /// Handle a UI command and return a typed outcome.
    ///
    /// Must complete in < 50 ms (P2); scroll requests in < 10 ms (P1).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome {
        match cmd {
            TrameCommand::SetActiveCell { row, col } => {
                self.active = (row, col);
                // Report the new active cell's formatting so the chrome toggle
                // buttons reflect the selection.
                TrameOutcome::display_only(vec![
                    TrameEvent::BoldState { enabled: self.bold.contains(&self.active) },
                    TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) },
                ])
            }
            TrameCommand::SetCellText { row, col, text } => {
                tracing::info!(
                    "[Logic] SetCellText row={} col={} text='{:.80}'",
                    row, col, text,
                );
                if text.is_empty() {
                    // Clearing: only touch the CRDT if the cell was materialized.
                    if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
                        let _ = self.doc.get_map(cells_map_name(&self.active_sheet)).delete(&cell_key(rl, cl));
                    }
                    let cell = self.active_cell_id(row, col);
                    let indices = self.engine_indices();
                    let ctx = TrameCtx {
                        sheet_name_to_u16: &self.sheet_name_to_u16,
                        sheet_id_to_u16: &self.sheet_id_to_u16,
                        named: &self.named,
                    };
                    self.engine.on_cell_changed(cell, "", &indices, &ctx);
                } else {
                    let Some((rl, cl)) = self.materialize(row, col) else {
                        // Beyond the materialization cap: drop the write.
                        return TrameOutcome::display_only(vec![]);
                    };
                    // Persist the canonical (`sheet_id`-bound) form so cross-sheet
                    // references survive rename/reorder (§2.D).
                    let stored = self.canonicalize_input(&text);
                    let _ = self.doc.get_map(cells_map_name(&self.active_sheet)).insert(&cell_key(rl, cl), stored.as_str());
                    let cell = CellId::bound(0, rl, cl);
                    let indices = self.engine_indices();
                    let ctx = TrameCtx {
                        sheet_name_to_u16: &self.sheet_name_to_u16,
                        sheet_id_to_u16: &self.sheet_id_to_u16,
                        named: &self.named,
                    };
                    self.engine.on_cell_changed(cell, &stored, &indices, &ctx);
                }
                self.doc.commit();
                let indices = self.engine_indices();
                self.engine.compute_all_dirty(&indices);
                let update = self.flush_delta();
                // Ask the UI to re-fetch its viewport so the edit (and any
                // formula it feeds) renders its computed result immediately,
                // without waiting for the network echo. The optimistic UI-side
                // update covers plain literals; formulas need this round-trip.
                TrameOutcome { display: vec![TrameEvent::DataReset], update, ephemeral: None }
            }
            TrameCommand::ToggleBold => {
                let (row, col) = self.active;
                let enabled = toggle(&mut self.bold, self.active);
                let update = self.persist_format(row, col);
                TrameOutcome {
                    display: vec![
                        TrameEvent::BoldState { enabled },
                        TrameEvent::CellFormat {
                            row,
                            col,
                            bold: enabled,
                            italic: self.italic.contains(&self.active),
                        },
                    ],
                    update,
                    ephemeral: None,
                }
            }
            TrameCommand::ToggleItalic => {
                let (row, col) = self.active;
                let enabled = toggle(&mut self.italic, self.active);
                let update = self.persist_format(row, col);
                TrameOutcome {
                    display: vec![
                        TrameEvent::ItalicState { enabled },
                        TrameEvent::CellFormat {
                            row,
                            col,
                            bold: self.bold.contains(&self.active),
                            italic: enabled,
                        },
                    ],
                    update,
                    ephemeral: None,
                }
            }
            TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end } => {
                // Clamp to grid bounds.
                let row_end = row_end.min(TOTAL_ROWS.saturating_sub(1));
                let col_end = col_end.min(TOTAL_COLS.saturating_sub(1));

                if row_start > row_end || col_start > col_end {
                    return TrameOutcome::display_only(vec![]);
                }

                // Synchronous recomputation of any dirty cells before serving
                // the viewport. Ensures the UI displays up-to-date formula results.
                let indices = self.engine_indices();
                self.engine.compute_all_dirty(&indices);

                let mut cells = Vec::with_capacity(
                    ((row_end - row_start + 1) * (col_end - col_start + 1)) as usize,
                );

                for row in row_start..=row_end {
                    for col in col_start..=col_end {
                        // A merge master carries its span; a covered cell renders
                        // blank (swallowed by the master). Both flag `is_merged`.
                        let merge_span = self
                            .merges
                            .get(&(row, col))
                            .map(|&(end_row, end_col)| (end_col - col + 1, end_row - row + 1));
                        let is_covered = self.covered.contains(&(row, col));
                        cells.push(DisplayCell {
                            row,
                            col,
                            text: if is_covered { String::new() } else { self.cell_text(row, col) },
                            raw: if is_covered {
                                String::new()
                            } else {
                                // Reconstruct cross-sheet qualifiers from the
                                // stable id to the current display name (§2.D).
                                self.formula_display(&self.cell_raw(row, col))
                            },
                            bold: self.bold.contains(&(row, col)),
                            italic: self.italic.contains(&(row, col)),
                            is_merged: merge_span.is_some() || is_covered,
                            merge_span,
                        });
                    }
                }

                TrameOutcome::display_only(vec![TrameEvent::ViewportData { version, row_start, col_start, cells }])
            }
            TrameCommand::InsertRow { at } => self.insert_axis(at, Axis::Row),
            TrameCommand::DeleteRow { at } => self.delete_axis(at, Axis::Row),
            TrameCommand::InsertColumn { at } => self.insert_axis(at, Axis::Col),
            TrameCommand::DeleteColumn { at } => self.delete_axis(at, Axis::Col),
            TrameCommand::MergeCells { row, col, row_end, col_end } => {
                self.merge_cells(row, col, row_end, col_end)
            }
            TrameCommand::UnmergeCells { row, col } => self.unmerge_cells(row, col),
            TrameCommand::Undo => self.apply_history(true),
            TrameCommand::Redo => self.apply_history(false),
            TrameCommand::DefineName { name, reference } => self.define_name(&name, &reference),
            TrameCommand::DeleteName { name } => self.delete_name(&name),
            TrameCommand::ActivateSheet { sheet_id } => self.activate_sheet(&sheet_id),
            TrameCommand::CreateSheet { name } => self.create_sheet(&name),
            TrameCommand::RenameSheet { sheet_id, name } => self.rename_sheet(&sheet_id, &name),
            TrameCommand::MoveSheet { sheet_id, to_index } => self.move_sheet(&sheet_id, to_index),
            TrameCommand::DeleteSheet { sheet_id } => self.delete_sheet(&sheet_id),
            TrameCommand::SortRange {
                sheet_id,
                row_start,
                row_end,
                col_start,
                col_end,
                sort_col,
                ascending,
                has_header,
            } => self.sort_range(
                &sheet_id, row_start, row_end, col_start, col_end, sort_col, ascending, has_header,
            ),
        }
    }

    /// Apply one undo (`undo = true`) or redo step from Loro's `UndoManager`,
    /// then rebuild every derived cache from the resulting CRDT state and
    /// broadcast the inverse ops it produced (§8).
    ///
    /// `undo()` / `redo()` commit the inverse of a past local commit as fresh
    /// ops, so the rebuild path is identical to a structural op or import: the
    /// index, merge cache, formula engine and formatting are all reseeded from
    /// the new document state. Returns the same `DataReset` + active-cell state
    /// the UI needs to repaint, plus an [`UndoState`] reporting the new stack
    /// availability. When the requested stack is empty the call is a no-op and
    /// only the (unchanged) availability is reported.
    ///
    /// [`UndoState`]: ambre_bus::TrameEvent::UndoState
    fn apply_history(&mut self, undo: bool) -> TrameOutcome {
        let changed = if undo { self.undo_manager.undo() } else { self.undo_manager.redo() }.unwrap_or(false);

        if !changed {
            return TrameOutcome::display_only(vec![self.undo_state_event()]);
        }

        // The inverse ops are already committed by undo()/redo(); reseed the
        // derived caches and broadcast the delta exactly as `after_structural`.
        let mut outcome = self.after_structural();
        outcome.display.push(TrameEvent::BoldState { enabled: self.bold.contains(&self.active) });
        outcome.display.push(TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) });
        outcome.display.push(self.undo_state_event());
        outcome
    }

    /// Current undo/redo availability, for the chrome's Undo/Redo affordances.
    fn undo_state_event(&self) -> TrameEvent {
        TrameEvent::UndoState {
            can_undo: self.undo_manager.can_undo(),
            can_redo: self.undo_manager.can_redo(),
        }
    }

    /// Merge the range with corners `(r0,c0)`-`(r1,c1)` (given in any order) into
    /// one cell. Rejected locally if a covered cell is non-empty (§6); otherwise
    /// records the master->extent entry in `merges_{sheet}` keyed by stable ids.
    fn merge_cells(&mut self, r0: u32, c0: u32, r1: u32, c1: u32) -> TrameOutcome {
        let (top, bottom) = (r0.min(r1), r0.max(r1));
        let (left, right) = (c0.min(c1), c0.max(c1));
        // A single cell is not a merge.
        if top == bottom && left == right {
            return TrameOutcome::display_only(vec![]);
        }
        // Client-side rule: every covered cell (excluding the master) must be
        // empty locally, matching Excel/Sheets. Cannot prevent a later
        // concurrent conflict -- that path masks instead (rebuild_merges).
        for row in top..=bottom {
            for col in left..=right {
                if (row, col) == (top, left) {
                    continue;
                }
                if self.cell_has_raw(row, col) {
                    return TrameOutcome::display_only(vec![TrameEvent::MergeRejected {
                        reason: format!(
                            "cell {}{} is not empty",
                            col_to_alpha(col),
                            row + 1
                        ),
                    }]);
                }
            }
        }
        // Materialize the bottom-right corner so the whole rectangle has ids.
        if self.materialize(bottom, right).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let (Some(master), Some(extent)) = (
            self.index.resolve(top as usize, left as usize),
            self.index.resolve(bottom as usize, right as usize),
        ) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self.doc.get_map(merges_map_name(&self.active_sheet)).insert(
            &cell_key(master.0, master.1),
            cell_key(extent.0, extent.1).as_str(),
        );
        self.doc.commit();
        self.after_merge_change()
    }

    /// Split the merge whose rectangle contains `(row, col)`, restoring covered
    /// cells (and any masked value). A no-op when the cell is not in a merge.
    fn unmerge_cells(&mut self, row: u32, col: u32) -> TrameOutcome {
        let master = self.merges.iter().find_map(|(&(top, left), &(bottom, right))| {
            (row >= top && row <= bottom && col >= left && col <= right).then_some((top, left))
        });
        let Some((top, left)) = master else {
            return TrameOutcome::display_only(vec![]);
        };
        let Some((rl, cl)) = self.index.resolve(top as usize, left as usize) else {
            return TrameOutcome::display_only(vec![]);
        };
        let _ = self.doc.get_map(merges_map_name(&self.active_sheet)).delete(&cell_key(rl, cl));
        self.doc.commit();
        self.after_merge_change()
    }

    /// Shared tail of merge/unmerge: rebuild the derived merge cache (which also
    /// re-evaluates masking), reseed the formula engine so unmasked cells rejoin
    /// the DAG, flush the delta and ask the UI to re-fetch. Conflict events from
    /// the rebuild ride along with the `DataReset`.
    fn after_merge_change(&mut self) -> TrameOutcome {
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        display.push(TrameEvent::DataReset);
        let update = self.flush_delta();
        TrameOutcome { display, update, ephemeral: None }
    }

    /// Whether grid `(row, col)` holds a non-empty string value in the CRDT.
    /// Resolves the position to its stable id pair; a virtual cell has no value.
    fn cell_has_raw(&self, row: u32, col: u32) -> bool {
        self.index
            .resolve(row as usize, col as usize)
            .and_then(|(rl, cl)| {
                self.doc
                    .get_map(cells_map_name(&self.active_sheet))
                    .get(&cell_key(rl, cl))
                    .map(|v| v.get_deep_value())
            })
            .is_some_and(|v| matches!(v, LoroValue::String(s) if !s.is_empty()))
    }

    /// Rebuild the derived merge caches ([`Self::merges`], [`Self::covered`],
    /// [`Self::masked_cells`]) from the active sheet's `merges_{sheet}` map.
    /// Requires [`Self::index`] to be current (keys are stable id pairs mapped
    /// back to grid positions). Returns one [`TrameEvent::MergeConflictDetected`]
    /// per masked cell.
    ///
    /// Overlapping merges are resolved deterministically: rectangles are placed
    /// in ascending master-key order and any that intersects an already-placed
    /// one is skipped. The order is identical on every peer (it derives only
    /// from the stable keys), so all clients converge on the same layout without
    /// mutating the CRDT (no corrective op, no conflict loop -- §6).
    fn rebuild_merges(&mut self) -> Vec<TrameEvent> {
        self.merges.clear();
        self.covered.clear();
        self.masked_cells.clear();
        let LoroValue::Map(entries) =
            self.doc.get_map(merges_map_name(&self.active_sheet)).get_value()
        else {
            return Vec::new();
        };
        // Collect resolvable rectangles tagged with their master key for a
        // deterministic, peer-independent overlap precedence.
        let mut rects: Vec<MergeRect> = entries
            .iter()
            .filter_map(|(master_key, value)| {
                let LoroValue::String(extent_key) = value else { return None };
                let (mr, mc) = parse_cell_key(master_key)?;
                let (er, ec) = parse_cell_key(extent_key)?;
                let top = self.index.row_pos(&mr)? as u32;
                let left = self.index.col_pos(&mc)? as u32;
                let bottom = self.index.row_pos(&er)? as u32;
                let right = self.index.col_pos(&ec)? as u32;
                Some((master_key.to_string(), (top.min(bottom), left.min(right)), (top.max(bottom), right.max(left))))
            })
            .collect();
        rects.sort_by(|a, b| a.0.cmp(&b.0));

        let mut events = Vec::new();
        let mut occupied: HashSet<(u32, u32)> = HashSet::new();
        for (_key, (top, left), (bottom, right)) in rects {
            let overlaps = (top..=bottom)
                .flat_map(|r| (left..=right).map(move |c| (r, c)))
                .any(|cell| occupied.contains(&cell));
            if overlaps {
                continue;
            }
            self.merges.insert((top, left), (bottom, right));
            for row in top..=bottom {
                for col in left..=right {
                    occupied.insert((row, col));
                    if (row, col) == (top, left) {
                        continue;
                    }
                    self.covered.insert((row, col));
                    // Mask a covered cell that still holds a value (concurrent
                    // conflict): keep the data, exclude it from the engine.
                    if self.cell_has_raw(row, col) {
                        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
                            self.masked_cells.insert(cell_key(rl, cl));
                        }
                        events.push(TrameEvent::MergeConflictDetected {
                            master_row: top,
                            master_col: left,
                            covered_row: row,
                            covered_col: col,
                        });
                    }
                }
            }
        }
        events
    }

    /// Insert a blank row/column at grid position `at`, shifting later elements
    /// down/right. One `LoroMovableList` op; cells keyed by stable id pairs
    /// follow their element automatically. A no-op past the materialized used
    /// range (those rows/columns are already virtual blanks, nothing to shift).
    fn insert_axis(&mut self, at: u32, axis: Axis) -> TrameOutcome {
        let at = at as usize;
        let used = match axis {
            Axis::Row => self.index.row_len(),
            Axis::Col => self.index.col_len(),
        };
        if at > used {
            return TrameOutcome::display_only(vec![]);
        }
        let list_name = match axis {
            Axis::Row => rows_list_name(&self.active_sheet),
            Axis::Col => cols_list_name(&self.active_sheet),
        };
        // The new line carries a freshly minted, move-invariant uid value (§2.A).
        let uid = self.mint_uid();
        if self
            .doc
            .get_movable_list(list_name)
            .insert(at, uid.to_key_part().as_str())
            .is_err()
        {
            return TrameOutcome::display_only(vec![]);
        }
        self.doc.commit();
        self.after_structural()
    }

    /// Delete the row/column at grid position `at`, shifting later elements
    /// up/left and purging the removed element's cells and formatting. Any
    /// formula whose referenced cell was below the deletion now reads the
    /// shifted (or emptied) position -- references are not rewritten
    /// (`ambre-spec-trame-logic.md` §12). Full id-based `#REF!` on delete is a
    /// separate broken-references story. A no-op in virtual space.
    fn delete_axis(&mut self, at: u32, axis: Axis) -> TrameOutcome {
        let at = at as usize;
        let removed = match axis {
            Axis::Row => self.index.row_id(at),
            Axis::Col => self.index.col_id(at),
        };
        let Some(removed) = removed else {
            return TrameOutcome::display_only(vec![]); // virtual: nothing materialized
        };
        let list_name = match axis {
            Axis::Row => rows_list_name(&self.active_sheet),
            Axis::Col => cols_list_name(&self.active_sheet),
        };
        if self.doc.get_movable_list(list_name).delete(at, 1).is_err() {
            return TrameOutcome::display_only(vec![]);
        }
        self.purge_axis(removed, axis);
        self.doc.commit();
        self.after_structural()
    }

    /// Drop every cell and formatting entry whose key references the removed
    /// row/column id, so a deleted line leaves no orphan entries behind.
    fn purge_axis(&mut self, removed: Uid, axis: Axis) {
        for name in [cells_map_name(&self.active_sheet), format_map_name(&self.active_sheet)] {
            let map = self.doc.get_map(name);
            let LoroValue::Map(entries) = map.get_value() else {
                continue;
            };
            let stale: Vec<String> = entries
                .keys()
                .filter(|k| {
                    parse_cell_key(k).is_some_and(|(rl, cl)| match axis {
                        Axis::Row => rl == removed,
                        Axis::Col => cl == removed,
                    })
                })
                .cloned()
                .collect();
            for key in stale {
                let _ = map.delete(&key);
            }
        }
    }

    /// Shared tail of every structural op: rebuild the position <-> id cache
    /// (positions shifted), reseed the positional formula engine and the
    /// formatting sets from the new layout, then flush the CRDT delta and ask
    /// the UI to re-fetch its viewport.
    fn after_structural(&mut self) -> TrameOutcome {
        self.rebuild_index();
        // Merges resolve through the freshly rebuilt index; do this before the
        // engine reseed so masked cells are excluded from the DAG.
        let mut display = self.rebuild_merges();
        self.reseed_engine();
        self.reseed_formatting();
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        TrameOutcome { display, update, ephemeral: None }
    }

    /// Reset the positional formula engine and re-feed it from the active
    /// sheet's cells at their current grid positions (resolved through the
    /// freshly rebuilt [`Self::index`]). Required after any structural op or
    /// import, where a cell's grid position may have changed under stable keys.
    fn reseed_engine(&mut self) {
        self.engine.reset();
        // Sheet maps and named ranges must be current before feeding cells, so
        // that `Sheet2!A1` and named references resolve while compiling.
        self.rebuild_sheet_maps();
        self.rebuild_named();

        // Feed every sheet's cells under a uid-anchored [`CellId`], tagged with
        // the sheet's local index. The active sheet is index 0 (see
        // `sheets_by_idx`), so its cells address the same keys the single-sheet
        // read paths use. Only the active sheet tracks merge masking. Each cell's
        // stable key already carries its `(row_uid, col_uid)`, so feeding is a
        // direct `CellId::bound`; a cell whose row/column uid is absent from the
        // sheet index (a deleted line) is skipped.
        let indices = self.engine_indices();
        let sheets: Vec<(String, u16)> = self
            .sheets_by_idx
            .iter()
            .enumerate()
            .map(|(idx, id)| (id.clone(), idx as u16))
            .collect();
        for (sheet_id, idx) in sheets {
            let Some(index) = indices.get(&idx) else { continue };
            let LoroValue::Map(cells) = self.doc.get_map(cells_map_name(&sheet_id)).get_value() else {
                continue;
            };
            for (key, value) in cells.iter() {
                // Masked cells (covered by a merge, conflicting value) stay out of
                // the DAG on the active sheet; other sheets have no masking.
                if idx == 0 && self.masked_cells.contains(key) {
                    continue;
                }
                let (Some((rl, cl)), LoroValue::String(text)) = (parse_cell_key(key), value) else {
                    continue;
                };
                if index.contains_row(&rl) && index.contains_col(&cl) {
                    let ctx = TrameCtx {
                        sheet_name_to_u16: &self.sheet_name_to_u16,
                        sheet_id_to_u16: &self.sheet_id_to_u16,
                        named: &self.named,
                    };
                    self.engine.on_cell_changed(
                        CellId::bound(idx, rl, cl),
                        text.as_str(),
                        &indices,
                        &ctx,
                    );
                }
            }
        }
        self.engine.compute_all_dirty(&indices);
    }

    /// Rebuild [`Self::sheets_by_idx`] and [`Self::sheet_name_to_u16`] from the
    /// `sheets` CRDT map. The active sheet is pinned to local index 0; remaining
    /// sheets follow in sorted-id order. The mapping is local-only (computed
    /// values never leave this client), so the exact ordering need not match
    /// peers -- only stability within this session matters.
    fn rebuild_sheet_maps(&mut self) {
        // Collect {id -> name} from the CRDT.
        let mut entries: Vec<(String, String)> = Vec::new();
        if let LoroValue::Map(sheets) = self.doc.get_map(SHEETS_MAP).get_value() {
            for (id, name) in sheets.iter() {
                let name = match name {
                    LoroValue::String(s) => s.to_string(),
                    _ => id.clone(),
                };
                entries.push((id.clone(), name));
            }
        }
        // Active sheet first (index 0), then the rest by id for determinism.
        entries.sort_by(|a, b| {
            let a_active = a.0 == self.active_sheet;
            let b_active = b.0 == self.active_sheet;
            b_active.cmp(&a_active).then_with(|| a.0.cmp(&b.0))
        });

        self.sheets_by_idx.clear();
        self.sheet_name_to_u16.clear();
        self.sheet_id_to_u16.clear();
        self.sheet_id_to_name.clear();
        for (idx, (id, name)) in entries.into_iter().enumerate() {
            self.sheet_name_to_u16.insert(name.clone(), idx as u16);
            self.sheet_id_to_u16.insert(id.clone(), idx as u16);
            self.sheet_id_to_name.insert(id.clone(), name);
            self.sheets_by_idx.push(id);
        }
        // A document always has at least the active sheet at index 0.
        if self.sheets_by_idx.is_empty() {
            self.sheets_by_idx.push(self.active_sheet.clone());
        }
    }

    /// Rebuild [`Self::named`] from the `named_ranges` CRDT map: parse each stored
    /// A1-syntax reference into a concrete cell/range target, resolving any
    /// sheet qualifier against [`Self::sheet_name_to_u16`]. A reference that
    /// fails to parse is dropped (the formula using it surfaces `#PARSE!`).
    fn rebuild_named(&mut self) {
        self.named.clear();
        let LoroValue::Map(names) = self.doc.get_map(NAMED_RANGES_MAP).get_value() else {
            return;
        };
        // Names cannot reference other names: resolve sheet qualifiers only.
        let no_names: HashMap<String, NameTarget> = HashMap::new();
        for (name, reference) in names.iter() {
            let LoroValue::String(reference) = reference else {
                continue;
            };
            let ctx = TrameCtx {
                sheet_name_to_u16: &self.sheet_name_to_u16,
                sheet_id_to_u16: &self.sheet_id_to_u16,
                named: &no_names,
            };
            // Unqualified references in a name definition default to the active
            // sheet (index 0).
            let target = parse_formula(reference)
                .and_then(|ast| resolve_refs(ast, 0, &ctx))
                .ok()
                .and_then(expr_to_name_target);
            if let Some(target) = target {
                self.named.insert(name.clone(), target);
            }
        }
    }

    /// Build a fresh [`SheetIndex`] for an arbitrary sheet id (the cross-sheet
    /// reseed needs each sheet's position<->id map, not just the active one).
    fn build_sheet_index(&self, sheet_id: &str) -> SheetIndex {
        let rows = self.doc.get_movable_list(rows_list_name(sheet_id));
        let cols = self.doc.get_movable_list(cols_list_name(sheet_id));
        SheetIndex::build(&rows, &cols)
    }

    /// Define or redefine a named range, persist it to the CRDT, and reseed so
    /// formulas using the name pick it up immediately.
    fn define_name(&mut self, name: &str, reference: &str) -> TrameOutcome {
        let _ = self.doc.get_map(NAMED_RANGES_MAP).insert(name, reference);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome { display: vec![TrameEvent::DataReset], update, ephemeral: None }
    }

    /// Remove a named range and reseed. A no-op if the name is undefined.
    fn delete_name(&mut self, name: &str) -> TrameOutcome {
        let map = self.doc.get_map(NAMED_RANGES_MAP);
        if map.get(name).is_none() {
            return TrameOutcome::display_only(vec![]);
        }
        let _ = map.delete(name);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome { display: vec![TrameEvent::DataReset], update, ephemeral: None }
    }

    // -------------------------------------------------------------------------
    // Sheet lifecycle (create / rename / reorder / delete / activate) -- §2.D
    // -------------------------------------------------------------------------

    /// Every sheet id registered in the `sheets` CRDT map (unordered).
    fn all_sheet_ids(&self) -> Vec<String> {
        match self.doc.get_map(SHEETS_MAP).get_value() {
            LoroValue::Map(sheets) => sheets.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// Canonical tab-strip order, deterministic on every peer: the implicit
    /// default sheet first, then any sheet not yet present in `sheet_order`
    /// (sorted by id -- covers a create whose order op has not merged), then the
    /// explicit `sheet_order` entries that still correspond to a live sheet.
    fn sheet_order_ids(&self) -> Vec<String> {
        let listed: Vec<String> = match self.doc.get_movable_list(SHEET_ORDER).get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let in_list: HashSet<&String> = listed.iter().collect();
        let mut unlisted: Vec<String> =
            self.all_sheet_ids().into_iter().filter(|id| !in_list.contains(id)).collect();
        // Default sheet first, the rest by id for a stable, peer-independent order.
        unlisted.sort_by(|a, b| {
            let a_def = a == DEFAULT_SHEET;
            let b_def = b == DEFAULT_SHEET;
            b_def.cmp(&a_def).then_with(|| a.cmp(b))
        });
        let existing: HashSet<String> = self.all_sheet_ids().into_iter().collect();
        drop(in_list);
        unlisted
            .into_iter()
            .chain(listed.into_iter().filter(|id| existing.contains(id)))
            .collect()
    }

    /// Build the [`TrameEvent::SheetList`] the tab strip consumes, in canonical
    /// order with the active sheet flagged.
    fn sheet_list_event(&self) -> TrameEvent {
        let map = self.doc.get_map(SHEETS_MAP);
        let sheets = self
            .sheet_order_ids()
            .into_iter()
            .map(|id| {
                let name = match map.get(&id).map(|v| v.get_deep_value()) {
                    Some(LoroValue::String(s)) => s.to_string(),
                    _ => id.clone(),
                };
                (id, name)
            })
            .collect();
        TrameEvent::SheetList { sheets, active: self.active_sheet.clone() }
    }

    /// Rebuild every active-sheet derived cache (index, merges, formula engine,
    /// formatting) from the current CRDT state. Used when the active sheet
    /// changes (switch or delete fallback) -- no CRDT mutation, no delta.
    /// Returns merge-conflict events from the rebuild.
    fn reseed_active_sheet(&mut self) -> Vec<TrameEvent> {
        self.rebuild_index();
        let conflicts = self.rebuild_merges();
        self.reseed_engine();
        self.reseed_formatting();
        conflicts
    }

    /// Switch the active sheet and re-derive its caches (§2.D). A no-op if the id
    /// is unknown or already active.
    fn activate_sheet(&mut self, sheet_id: &str) -> TrameOutcome {
        if sheet_id == self.active_sheet || !self.all_sheet_ids().iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        self.active_sheet = sheet_id.to_owned();
        let mut display = self.reseed_active_sheet();
        display.push(TrameEvent::DataReset);
        display.push(self.sheet_list_event());
        display.push(TrameEvent::BoldState { enabled: self.bold.contains(&self.active) });
        display.push(TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) });
        TrameOutcome::display_only(display)
    }

    /// Add a sheet with a freshly minted peer-scoped id (§2.D). The new sheet is
    /// empty; its containers are created lazily on first write. Does not switch
    /// the active sheet (the UI sends [`TrameCommand::ActivateSheet`] for that).
    fn create_sheet(&mut self, name: &str) -> TrameOutcome {
        let id = format!("{:016x}_{:08x}", self.doc.peer_id(), self.sheet_counter);
        self.sheet_counter += 1;
        let _ = self.doc.get_map(SHEETS_MAP).insert(&id, name);
        let _ = self.doc.get_movable_list(SHEET_ORDER).push(id.as_str());
        self.doc.commit();
        // Refresh the name -> index map so the new sheet is addressable.
        self.rebuild_sheet_maps();
        let update = self.flush_delta();
        TrameOutcome { display: vec![self.sheet_list_event()], update, ephemeral: None }
    }

    /// Rename a sheet's display label (LWW on the name). The stable id is
    /// untouched. The name -> index map changes, so reseed to recompile any
    /// cross-sheet reference (the display-name binding is the §2.D MVP gap).
    /// A no-op if the id is unknown.
    fn rename_sheet(&mut self, sheet_id: &str, name: &str) -> TrameOutcome {
        if !self.all_sheet_ids().iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        let _ = self.doc.get_map(SHEETS_MAP).insert(sheet_id, name);
        self.doc.commit();
        self.reseed_engine();
        let update = self.flush_delta();
        TrameOutcome {
            display: vec![self.sheet_list_event(), TrameEvent::DataReset],
            update,
            ephemeral: None,
        }
    }

    /// Move a sheet to `to_index` in the tab strip (§2.D). Pure presentation, no
    /// recompute. Materializes the canonical order into `sheet_order` first (the
    /// implicit default and unmerged entries are otherwise absent) so positions
    /// are well-defined, then issues one move op. A no-op if the id is unknown or
    /// already at the target position.
    fn move_sheet(&mut self, sheet_id: &str, to_index: u32) -> TrameOutcome {
        let order = self.sheet_order_ids();
        let Some(from) = order.iter().position(|id| id == sheet_id) else {
            return TrameOutcome::display_only(vec![]);
        };
        let to = (to_index as usize).min(order.len().saturating_sub(1));
        if from == to {
            return TrameOutcome::display_only(vec![]);
        }
        let list = self.doc.get_movable_list(SHEET_ORDER);
        // Rewrite the list to the canonical order if it diverges, so `mov`
        // operates on the positions the tab strip actually shows.
        let current: Vec<String> = match list.get_value() {
            LoroValue::List(items) => items
                .iter()
                .filter_map(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if current != order {
            let _ = list.clear();
            for id in &order {
                let _ = list.push(id.as_str());
            }
        }
        let _ = list.mov(from, to);
        self.doc.commit();
        let update = self.flush_delta();
        TrameOutcome { display: vec![self.sheet_list_event()], update, ephemeral: None }
    }

    /// Delete a sheet and its per-sheet containers (§2.D). Rejected locally if it
    /// is the last remaining sheet. If the active sheet is deleted, the active
    /// sheet falls back to the previous tab (else the first remaining). The sheet
    /// set changed, so a full reseed remaps engine indices and lets cross-sheet
    /// formulas targeting the gone id surface `#REF!`.
    fn delete_sheet(&mut self, sheet_id: &str) -> TrameOutcome {
        let ids = self.all_sheet_ids();
        if !ids.iter().any(|id| id == sheet_id) {
            return TrameOutcome::display_only(vec![]);
        }
        if ids.len() <= 1 {
            return TrameOutcome::display_only(vec![TrameEvent::Error {
                message: "a workbook must keep at least one sheet".to_owned(),
            }]);
        }
        // Pick the fallback active sheet from the pre-delete order.
        let order = self.sheet_order_ids();
        let list = self.doc.get_movable_list(SHEET_ORDER);
        if let LoroValue::List(items) = list.get_value()
            && let Some(pos) = items
                .iter()
                .position(|v| matches!(v, LoroValue::String(s) if s.as_str() == sheet_id))
        {
            let _ = list.delete(pos, 1);
        }
        let _ = self.doc.get_map(SHEETS_MAP).delete(sheet_id);
        self.drop_sheet_containers(sheet_id);
        self.doc.commit();

        if self.active_sheet == sheet_id {
            let pos = order.iter().position(|id| id == sheet_id).unwrap_or(0);
            self.active_sheet = if pos > 0 {
                order[pos - 1].clone()
            } else {
                order
                    .iter()
                    .find(|id| id.as_str() != sheet_id)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_SHEET.to_owned())
            };
        }

        let mut display = self.reseed_active_sheet();
        display.push(self.sheet_list_event());
        display.push(TrameEvent::DataReset);
        let update = self.flush_delta();
        TrameOutcome { display, update, ephemeral: None }
    }

    /// Drop every cell, formatting and merge entry of `sheet_id` and clear its
    /// row/column lists, so a deleted sheet leaves no orphan containers behind.
    fn drop_sheet_containers(&mut self, sheet_id: &str) {
        for name in [
            cells_map_name(sheet_id),
            format_map_name(sheet_id),
            merges_map_name(sheet_id),
        ] {
            let map = self.doc.get_map(name);
            if let LoroValue::Map(entries) = map.get_value() {
                let keys: Vec<String> = entries.keys().cloned().collect();
                for key in keys {
                    let _ = map.delete(&key);
                }
            }
        }
        for name in [rows_list_name(sheet_id), cols_list_name(sheet_id)] {
            let _ = self.doc.get_movable_list(name).clear();
        }
    }

    /// Single-column sort of a row range (`ambre-spec-trame-features.md` §3).
    ///
    /// Reorders whole rows of the active sheet by the typed value in column
    /// `col_start + sort_col`. Cells of a row outside `[col_start..=col_end]`
    /// travel with it (Google Sheets "sort range").
    ///
    /// **Realized as a pure row-order permutation of `rows_{sheet}`**: each
    /// sortable row is keyed by its own move-invariant `Uid` (the list element
    /// *value*, not a Loro slot id), and the desired order is replayed with
    /// `LoroMovableList::mov`. Cells keyed by `(row_uid, col_uid)` travel with
    /// their row for free -- no cell content is read or rewritten (§2.A, §12).
    ///
    /// References then **follow the data**: the formula engine's bytecode is
    /// uid-anchored, so the sort preserves it (no reseed/re-parse) and merely
    /// recomputes -- every `A1` resolves to its row/column's new position, the
    /// section 12 guarantee. The `mov` ops produce a CRDT delta like any edit;
    /// LWW on the row order resolves concurrent sorts deterministically.
    ///
    /// Sorting only ever targets the **active sheet** (the UI sends an
    /// [`ActivateSheet`](TrameCommand::ActivateSheet) before any range op): the
    /// formula cache the sort keys read from is pinned to index 0. A command for
    /// any other sheet, or an empty/virtual range, is a no-op.
    #[allow(clippy::too_many_arguments)]
    fn sort_range(
        &mut self,
        sheet_id: &str,
        row_start: u32,
        row_end: u32,
        col_start: u32,
        col_end: u32,
        sort_col: u32,
        ascending: bool,
        has_header: bool,
    ) -> TrameOutcome {
        // Only the active sheet has its values positioned in the engine cache.
        if sheet_id != self.active_sheet {
            return TrameOutcome::display_only(vec![]);
        }
        // Clamp the range to the materialized rows; virtual rows carry no cell
        // data to reorder and no value to sort on.
        let last = self.index.row_len().saturating_sub(1) as u32;
        let row_end = row_end.min(last);
        if col_start + sort_col > col_end || row_start > row_end {
            return TrameOutcome::display_only(vec![]);
        }
        let sort_col_abs = col_start + sort_col;
        // First sortable position: the header row (if any) stays pinned in place.
        let sort_start = if has_header { row_start + 1 } else { row_start };
        if sort_start >= row_end {
            // Zero or one sortable row: nothing to reorder.
            return TrameOutcome::display_only(vec![]);
        }

        // Key each sortable row by its uid and the evaluated value of its sort
        // column. `evaluated_value` returns the formula result or parsed literal;
        // an empty cell sorts as `Empty` (last). Whole rows travel by uid, so no
        // cell contents are read or rewritten -- the sort is a pure permutation.
        let mut keyed: Vec<(Uid, CellValue)> = (sort_start..=row_end)
            .filter_map(|pos| {
                let row_uid = self.index.row_id(pos as usize)?;
                let value = self
                    .engine
                    .evaluated_value(self.active_cell_id(pos, sort_col_abs))
                    .unwrap_or(CellValue::Empty);
                Some((row_uid, value))
            })
            .collect();

        // Current uid order of the sortable region, to detect an already-sorted
        // no-op after the stable sort below.
        let before: Vec<Uid> = keyed.iter().map(|(uid, _)| *uid).collect();
        keyed.sort_by(|(_, a), (_, b)| {
            let ord = compare_cell_values(a, b);
            if ascending { ord } else { ord.reverse() }
        });
        let target: Vec<Uid> = keyed.into_iter().map(|(uid, _)| uid).collect();
        if before == target {
            // Stable sort left the order untouched: acknowledge, no CRDT delta.
            return TrameOutcome::display_only(vec![TrameEvent::SortComplete]);
        }

        // Realize the permutation as `mov` ops on `rows_{sheet}`: cells keyed by
        // the move-invariant uid follow for free (§2.A, §12). Move each desired
        // uid into place left-to-right over the sortable region; `cur` mirrors the
        // list's evolving order so the `mov` indices stay correct.
        let list = self.doc.get_movable_list(rows_list_name(&self.active_sheet));
        let mut cur: Vec<Uid> = self.index.row_ids().to_vec();
        let base = sort_start as usize;
        for (offset, want) in target.into_iter().enumerate() {
            let dst = base + offset;
            if cur[dst] == want {
                continue;
            }
            // The wanted uid is always at or after `dst` (earlier slots are fixed).
            let src = cur[dst..].iter().position(|u| *u == want).map(|p| p + dst);
            let Some(src) = src else { continue };
            let _ = list.mov(src, dst);
            let u = cur.remove(src);
            cur.insert(dst, u);
        }
        self.doc.commit();

        // Refresh the position<->uid cache and the position-keyed derived caches
        // (merges, formatting) for the new row order. Crucially, do NOT reseed
        // (re-parse) the formula engine: the compiled bytecode is uid-anchored,
        // so preserving it and merely recomputing lets every reference resolve to
        // its row/column's NEW position -- references follow the moved data, the
        // section 12 guarantee. A full reseed would re-bind each `A1` against the
        // post-sort index and collapse the engine back to positional semantics.
        self.rebuild_index();
        let mut display = self.rebuild_merges();
        self.reseed_formatting();
        let indices = self.engine_indices();
        self.engine.mark_all_dirty();
        self.engine.compute_all_dirty(&indices);
        let update = self.flush_delta();
        display.push(TrameEvent::DataReset);
        display.push(TrameEvent::SortComplete);
        TrameOutcome { display, update, ephemeral: None }
    }

    /// Mirror a cell's current bold/italic state into the CRDT `format` map and
    /// return the incremental delta to broadcast (and persist). A cell with no
    /// formatting is removed from the map so it does not accumulate dead entries.
    fn persist_format(&mut self, row: u32, col: u32) -> Option<Vec<u8>> {
        let mut mask = 0i64;
        if self.bold.contains(&(row, col)) {
            mask |= FMT_BOLD;
        }
        if self.italic.contains(&(row, col)) {
            mask |= FMT_ITALIC;
        }
        // Formatting a cell pins an identity to it, so it must be materialized
        // just like an edited value. Beyond the cap there is nothing to persist.
        let (rl, cl) = self.materialize(row, col)?;
        let map = self.doc.get_map(format_map_name(&self.active_sheet));
        let key = cell_key(rl, cl);
        if mask == 0 {
            let _ = map.delete(&key);
        } else {
            let _ = map.insert(&key, mask);
        }
        self.doc.commit();
        self.flush_delta()
    }

    /// Rebuild the in-memory bold/italic sets from the active sheet's
    /// `format_{sheet}` map. Called after any import so a reload or a peer's
    /// formatting edit is reflected. Requires [`Self::index`] to be current
    /// (the keys are stable id pairs, mapped back to grid positions).
    fn reseed_formatting(&mut self) {
        self.bold.clear();
        self.italic.clear();
        if let LoroValue::Map(fmts) = self.doc.get_map(format_map_name(&self.active_sheet)).get_value() {
            for (key, value) in fmts.iter() {
                let (Some((rl, cl)), LoroValue::I64(mask)) = (parse_cell_key(key), value) else {
                    continue;
                };
                let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl)) else {
                    continue; // references a deleted row/column
                };
                let cell = (row as u32, col as u32);
                if mask & FMT_BOLD != 0 {
                    self.bold.insert(cell);
                }
                if mask & FMT_ITALIC != 0 {
                    self.italic.insert(cell);
                }
            }
        }
    }

    /// Export any new CRDT ops since `last_exported_vv` and bump the tracking
    /// cursor. Returns `Some` binary delta if there were new ops.
    fn flush_delta(&mut self) -> Option<Vec<u8>> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        let delta = self.doc.export(ExportMode::updates(&self.last_exported_vv)).ok();
        self.last_exported_vv = current;
        if let Some(ref d) = delta {
            tracing::info!("[Logic] flush_delta → {} bytes of CRDT ops", d.len());
        }
        delta
    }

    /// Apply a remote CRDT delta and return display events.
    ///
    /// `payload` is the raw (decrypted) Loro binary update forwarded by the Sync
    /// thread (a peer's edit, or our own history replayed on open). Loro's
    /// `import` ingests both incremental updates and full snapshots, so the same
    /// reseed path as [`import_snapshot`](Self::import_snapshot) applies: merge
    /// the ops, reseed the formula engine, and tell the UI to re-fetch.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent> {
        self.import_and_reseed(payload, "apply_remote")
    }

    /// Apply a remote ephemeral (presence) payload and return presence events.
    ///
    /// Stubbed until collaborator presence is wired.
    pub fn apply_ephemeral(&mut self, _payload: &[u8]) -> Vec<TrameEvent> {
        vec![]
    }

    /// Export the full CRDT state for snapshot compaction.
    pub fn export_snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap_or_default()
    }

    /// Import a full CRDT snapshot (local restore from IndexedDB at startup).
    ///
    /// Imports the persisted Loro state into the live document, advances the
    /// export cursor so the next edit still produces a correct incremental
    /// delta, and reseeds the formula engine from the restored cells (so formula
    /// cells evaluate instead of showing their literal source). Returns no
    /// display events: the next viewport request re-renders the restored grid.
    pub fn import_snapshot(&mut self, payload: &[u8]) -> Vec<TrameEvent> {
        self.import_and_reseed(payload, "import_snapshot")
    }

    /// Resolve a sheet name to its stable sheet id (e.g. `"Sheet1"` -> `"s0"`).
    /// Groundwork for cross-sheet formula references (`Sheet2!A1`). Linear scan
    /// over the `sheets` map -- there are only a handful of sheets.
    pub fn sheet_name_to_id(&self, name: &str) -> Option<String> {
        match self.doc.get_map(SHEETS_MAP).get_value() {
            LoroValue::Map(sheets) => sheets.iter().find_map(|(id, value)| {
                matches!(value, LoroValue::String(n) if n.as_str() == name).then(|| id.to_string())
            }),
            _ => None,
        }
    }

    /// Shared import path for both a local snapshot restore and a remote delta:
    /// merge the Loro bytes, advance the export cursor past the imported ops (so
    /// they are not re-broadcast), reseed the formula engine from the resulting
    /// cell values, and emit a `DataReset` so the UI re-fetches its viewport
    /// (the original request may predate this import and show an empty grid).
    fn import_and_reseed(&mut self, payload: &[u8], origin: &str) -> Vec<TrameEvent> {
        if payload.is_empty() {
            return Vec::new();
        }
        if let Err(e) = self.doc.import(payload) {
            tracing::warn!("[Logic] {origin} import failed: {e:?}");
            return Vec::new();
        }
        // Rebuild the position <-> id cache first: the merged doc may carry rows
        // or columns this cache does not yet know about, and both the engine
        // reseed and the formatting reseed below resolve cell keys through it.
        self.rebuild_index();
        let cells_map = self.doc.get_map(cells_map_name(&self.active_sheet));
        let cell_count = match cells_map.get_value() {
            LoroValue::Map(cells) => cells.len(),
            _ => 0,
        };
        tracing::info!("[Logic] {origin} imported {} bytes, {cell_count} cell(s) now present", payload.len());
        // Advance the cursor to include the imported ops: we did not author them
        // (or already persisted them), so the next `flush_delta` must not export
        // them back. Local edits are flushed each frame before remote applies, so
        // there are no pending local-only ops to swallow here.
        self.last_exported_vv = self.doc.oplog_vv();
        // Rebuild the merge cache (and re-evaluate masking) before the engine
        // reseed so masked cells are excluded; conflict events ride along below.
        let conflicts = self.rebuild_merges();
        // Reseed the formula engine from the resulting cell values, mapping each
        // stable key pair back to its grid position via the rebuilt index. A
        // merge may have moved cells under their stable keys, so reset first.
        self.reseed_engine();
        // Rebuild bold/italic from the merged `format_{sheet}` map.
        self.reseed_formatting();
        // Refresh the toolbar / context-menu toggles for the active cell: after a
        // reload its formatting is only known once the `format` map is restored.
        let mut events = vec![
            TrameEvent::DataReset,
            self.sheet_list_event(),
            TrameEvent::BoldState { enabled: self.bold.contains(&self.active) },
            TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) },
        ];
        events.extend(conflicts);
        events
    }
}

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Convert a resolved name-definition expression into a [`NameTarget`]. Only a
/// bare cell or range is a valid named-range body; anything else (an arithmetic
/// expression, a function call) is rejected.
fn expr_to_name_target(expr: Expr) -> Option<NameTarget> {
    match expr {
        Expr::CellRef(r) => Some(NameTarget::Cell(r)),
        Expr::Range(r) => Some(NameTarget::Range(r)),
        _ => None,
    }
}

/// Order two cell values for sorting (`ambre-spec-trame-features.md` §3). Within
/// a type the natural order applies; across types the precedence is
/// `Number < Text < Boolean < Error < Empty`, so blanks sink to the bottom and
/// errors group just above them, matching the spec. `Range` only ever appears on
/// the VM stack, never in the cache, so it is treated as `Empty` defensively.
fn compare_cell_values(a: &CellValue, b: &CellValue) -> std::cmp::Ordering {
    use std::cmp::Ordering::Equal;
    // Type rank: lower sorts first.
    fn rank(v: &CellValue) -> u8 {
        match v {
            CellValue::Number(_) => 0,
            CellValue::Text(_) => 1,
            CellValue::Boolean(_) => 2,
            CellValue::Error(_) => 3,
            CellValue::Empty | CellValue::Range(_) => 4,
        }
    }
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (CellValue::Text(x), CellValue::Text(y)) => x.cmp(y),
        (CellValue::Boolean(x), CellValue::Boolean(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Toggle `cell`'s membership in `set`, returning whether it is now present.
fn toggle(set: &mut HashSet<(u32, u32)>, cell: (u32, u32)) -> bool {
    if set.remove(&cell) {
        false
    } else {
        set.insert(cell);
        true
    }
}

/// Convert a 0-based column index to a spreadsheet column label (A, B, ..., Z,
/// AA, AB, ...).
///
/// col=0 -> "A", col=25 -> "Z", col=26 -> "AA", col=701 -> "ZZ".
fn col_to_alpha(mut col: u32) -> String {
    let mut result = String::new();
    loop {
        let rem = (col % 26) as u8;
        result.insert(0, (b'A' + rem) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{col_to_alpha, TrameState};
    use crate::model::Uid;
    use crate::{TrameCommand, TrameEvent};

    /// Bold a cell, then move the selection elsewhere: the formatting must stay
    /// pinned to the cell it was applied to, not follow the cursor. This is the
    /// regression the per-cell model fixes.
    #[test]
    fn bold_stays_on_the_cell_not_the_selection() {
        let mut state = TrameState::new();

        // Select A1 (0,0) and bold it.
        state.handle_command(TrameCommand::SetActiveCell { row: 0, col: 0 });
        let evts = state.handle_command(TrameCommand::ToggleBold);
        assert!(matches!(evts.display[0], TrameEvent::BoldState { enabled: true }));
        assert!(matches!(
            evts.display[1],
            TrameEvent::CellFormat { row: 0, col: 0, bold: true, italic: false }
        ));

        // Move the selection to B2 (1,1): the active cell is not bold, so the
        // button reports false -- A1 keeps its bold.
        let evts = state.handle_command(TrameCommand::SetActiveCell { row: 1, col: 1 });
        assert!(matches!(evts.display[0], TrameEvent::BoldState { enabled: false }));

        // A viewport covering both cells: only A1 carries the bold run.
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 1,
            row_start: 0,
            row_end: 1,
            col_start: 0,
            col_end: 1,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
            panic!("expected ViewportData");
        };
        let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        let b2 = cells.iter().find(|c| c.row == 1 && c.col == 1).unwrap();
        assert!(a1.bold, "A1 must stay bold");
        assert!(!b2.bold, "B2 must not become bold");
    }

    /// Bold and italic survive a snapshot round-trip (the F5 reload path):
    /// formatting must be persisted in the CRDT, not only in memory.
    #[test]
    fn formatting_survives_snapshot_roundtrip() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetActiveCell { row: 0, col: 0 });
        state.handle_command(TrameCommand::ToggleBold);
        state.handle_command(TrameCommand::ToggleItalic);

        // Simulate a reload: fresh state restored from the persisted snapshot.
        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);

        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 1,
            row_start: 0,
            row_end: 0,
            col_start: 0,
            col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else {
            panic!("expected ViewportData");
        };
        let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        assert!(a1.bold, "A1 must stay bold after reload");
        assert!(a1.italic, "A1 must stay italic after reload");
    }

    /// An edited cell stores its text in the CRDT and overrides the generated
    /// label; clearing it falls back to blank.
    #[test]
    fn edited_text_overrides_generated_label() {
        let mut state = TrameState::new();

        // A1 starts blank (virtual cell).
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "");

        // Edit A1, then read it back: the stored text wins.
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: "hello".to_owned(),
        });
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 2, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "hello");

        // Clearing it restores the blank cell.
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: String::new(),
        });
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 3, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "");
    }

    /// A value written far down the grid is keyed by its row/column stable ids,
    /// survives a snapshot round-trip, and resolves at the same grid position in
    /// a fresh state -- proving the LID-keyed persistence + `SheetIndex` rebuild.
    #[test]
    fn far_cell_survives_snapshot_roundtrip() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText {
            row: 1000, col: 3, text: "deep".to_owned(),
        });

        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);

        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 1000, row_end: 1000, col_start: 3, col_end: 3,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "deep");
        // An untouched far cell is blank (virtual space).
        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 2, row_start: 2000, row_end: 2000, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "");
    }

    /// A formula referencing another cell still evaluates after the value and the
    /// formula are stored under stable id keys and reloaded.
    #[test]
    fn formula_evaluates_through_stable_keys() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "=21".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "=A1*2".to_owned() });

        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);

        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 0, row_end: 0, col_start: 0, col_end: 1,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        let b1 = cells.iter().find(|c| c.row == 0 && c.col == 1).unwrap();
        assert_eq!(b1.text, "42", "B1 = A1*2 must evaluate after reload");
    }

    /// A defined name resolves inside a formula, recomputes when its target
    /// changes, survives a snapshot round-trip, and breaks when deleted.
    #[test]
    fn named_range_defines_resolves_and_deletes() {
        let mut state = TrameState::new();
        // A1=1, A2=2, A3=3.
        for r in 0..3 {
            state.handle_command(TrameCommand::SetCellText {
                row: r, col: 0, text: format!("{}", r + 1),
            });
        }
        // Define TOTAL = A1:A3, then C1 = SUM(TOTAL).
        state.handle_command(TrameCommand::DefineName {
            name: "TOTAL".to_owned(),
            reference: "A1:A3".to_owned(),
        });
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 2, text: "=SUM(TOTAL)".to_owned(),
        });

        let read_c1 = |state: &mut TrameState, version: u64| {
            let evts = state.handle_command(TrameCommand::ScrollViewport {
                version, row_start: 0, row_end: 0, col_start: 2, col_end: 2,
            });
            let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
            cells[0].text.clone()
        };

        assert_eq!(read_c1(&mut state, 1), "6", "SUM(TOTAL) = 1+2+3");

        // Editing a target cell recomputes the name's consumer.
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: "10".to_owned(),
        });
        assert_eq!(read_c1(&mut state, 2), "15", "SUM(TOTAL) tracks A1 edit");

        // The name survives a snapshot round-trip.
        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);
        assert_eq!(read_c1(&mut restored, 3), "15", "name persists across reload");

        // Deleting the name stops it resolving: its consumer no longer computes
        // the sum (the engine drops the now-unresolvable formula, so the cell
        // falls back to its raw text rather than the stale 15).
        restored.handle_command(TrameCommand::DeleteName { name: "TOTAL".to_owned() });
        let after = read_c1(&mut restored, 4);
        assert_ne!(after, "15", "deleted name must no longer resolve");
        assert_eq!(after, "=SUM(TOTAL)", "unresolved formula falls back to raw text");
    }

    /// End-to-end of the reported bug: plain numeric literals typed into A1/B1
    /// must feed `=SUM(A1:B1)` (display "3"), and the formula cell's `raw` must
    /// carry the formula source (so the in-cell editor reopens on the formula,
    /// not its result).
    #[test]
    fn sum_over_plain_literals_and_formula_raw_in_viewport() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "1".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "2".to_owned() });
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 2, text: "=SUM(A1:B1)".to_owned(),
        });

        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 0, row_end: 0, col_start: 0, col_end: 2,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };

        let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        assert_eq!(a1.text, "1");
        assert_eq!(a1.raw, "1", "plain cell raw == display");

        let c1 = cells.iter().find(|c| c.row == 0 && c.col == 2).unwrap();
        assert_eq!(c1.text, "3", "SUM must see plain literals A1=1, B1=2");
        assert_eq!(c1.raw, "=SUM(A1:B1)", "editor must reopen on the formula");
    }

    /// The same scenario after a reseed (the path a remote echo / snapshot
    /// restore takes): plain literals must still feed the formula so the
    /// refreshed viewport shows "3", not the optimistic formula text.
    #[test]
    fn sum_over_plain_literals_survives_reseed() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "1".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "2".to_owned() });
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 2, text: "=SUM(A1:B1)".to_owned(),
        });

        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);

        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 0, row_end: 0, col_start: 0, col_end: 2,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        let c1 = cells.iter().find(|c| c.row == 0 && c.col == 2).unwrap();
        assert_eq!(c1.text, "3", "SUM must re-evaluate after reseed");
        assert_eq!(c1.raw, "=SUM(A1:B1)");
    }

    /// Two sheets keep their cells isolated (distinct `cells_{sheet}` containers)
    /// and `sheet_name_to_id` resolves the default sheet's name.
    #[test]
    fn multi_sheet_isolation_and_name_lookup() {
        let mut state = TrameState::new();
        assert_eq!(state.sheet_name_to_id("Sheet1").as_deref(), Some("s0"));
        assert_eq!(state.sheet_name_to_id("Nope"), None);

        // Write to the default sheet, then to a second sheet's containers
        // directly (the add-sheet command lands in a later step).
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "on_s0".to_owned() });
        let _ = state.doc.get_map(super::SHEETS_MAP).insert("s1", "Sheet2");
        let other = state.doc.get_movable_list(super::rows_list_name("s1"));
        other.push(Uid { peer: 1, counter: 0 }.to_key_part().as_str()).unwrap();
        state.doc.commit();

        // `cells_s1` is empty; `cells_s0` holds the value -- containers are isolated.
        assert!(matches!(
            state.doc.get_map(super::cells_map_name("s0")).get_value(),
            loro::LoroValue::Map(ref m) if m.len() == 1
        ));
        assert!(matches!(
            state.doc.get_map(super::cells_map_name("s1")).get_value(),
            loro::LoroValue::Map(ref m) if m.is_empty()
        ));
        assert_eq!(state.sheet_name_to_id("Sheet2").as_deref(), Some("s1"));
    }

    /// Read the text at a single grid cell through a viewport request.
    fn read_cell(state: &mut TrameState, row: u32, col: u32) -> String {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 999, row_start: row, row_end: row, col_start: col, col_end: col,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        cells[0].text.clone()
    }

    /// Read the formula-bar `raw` text at a single grid cell through a viewport
    /// request (the reconstructed display form, not the stored canonical form).
    fn read_raw(state: &mut TrameState, row: u32, col: u32) -> String {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 999, row_start: row, row_end: row, col_start: col, col_end: col,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        cells[0].raw.clone()
    }

    /// A cross-sheet formula is stored bound to the stable `sheet_id` (`#id`
    /// form), so a later rename keeps it computing and its formula-bar text
    /// follows the new name; a reorder is inert; deleting the target sheet turns
    /// it into `#REF!` (spec section 2.D / section 12).
    #[test]
    fn cross_sheet_reference_survives_rename_and_breaks_on_delete() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet { name: "Sheet2".to_owned() });
        let s2 = second_sheet_id(&state);

        // Sheet2!A1 = 5.
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: s2.clone() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "5".to_owned() });

        // Sheet1!B1 = Sheet2!A1 * 2.
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: "s0".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "=Sheet2!A1*2".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 1), "10");
        // Stored canonical form is id-bound; the formula bar shows the name.
        assert_eq!(state.cell_raw(0, 1), format!("=#{s2}!A1*2"), "v is sheet_id-bound");
        assert_eq!(read_raw(&mut state, 0, 1), "=Sheet2!A1*2", "bar shows display name");

        // Rename the target: the reference keeps computing and the bar follows.
        state.handle_command(TrameCommand::RenameSheet { sheet_id: s2.clone(), name: "Data".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 1), "10", "rename preserves the reference");
        assert_eq!(read_raw(&mut state, 0, 1), "=Data!A1*2", "bar reflects the new name");

        // Reorder: pure presentation, value unchanged.
        state.handle_command(TrameCommand::MoveSheet { sheet_id: s2.clone(), to_index: 0 });
        assert_eq!(read_cell(&mut state, 0, 1), "10", "reorder is inert");

        // Delete the target sheet: the cross-sheet reference becomes #REF!.
        state.handle_command(TrameCommand::DeleteSheet { sheet_id: s2 });
        assert_eq!(read_cell(&mut state, 0, 1), "#REF!", "deleted target -> #REF!");
    }

    /// Undo reverts the last local edit; redo reapplies it. The `UndoManager`
    /// records one item per command (one commit), so a single undo restores the
    /// cell to its pre-edit (empty) label and a single redo brings the value back.
    #[test]
    fn undo_then_redo_round_trips_a_cell_edit() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "hello".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 0), "hello");

        let evts = state.handle_command(TrameCommand::Undo);
        assert!(matches!(evts.display[0], TrameEvent::DataReset));
        // Undo produces inverse ops to broadcast, and reports the new stacks.
        assert!(evts.update.is_some(), "undo broadcasts its inverse ops");
        assert!(matches!(
            evts.display.last(),
            Some(TrameEvent::UndoState { can_undo: false, can_redo: true })
        ));
        assert_eq!(read_cell(&mut state, 0, 0), "", "value gone, cell blank");

        let evts = state.handle_command(TrameCommand::Redo);
        assert!(matches!(
            evts.display.last(),
            Some(TrameEvent::UndoState { can_undo: true, can_redo: false })
        ));
        assert_eq!(read_cell(&mut state, 0, 0), "hello", "redo restores the value");
    }

    /// Undo with an empty stack is a no-op: no `DataReset`, no broadcast, just the
    /// (unchanged) availability so the chrome can disable the button.
    #[test]
    fn undo_with_empty_stack_is_a_noop() {
        let mut state = TrameState::new();
        let evts = state.handle_command(TrameCommand::Undo);
        assert!(evts.update.is_none());
        assert_eq!(evts.display.len(), 1);
        assert!(matches!(
            evts.display[0],
            TrameEvent::UndoState { can_undo: false, can_redo: false }
        ));
    }

    /// Inserting a row shifts the cells below it down by one position; cells are
    /// keyed by stable ids so their content follows the shift.
    #[test]
    fn insert_row_shifts_cells_down() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "top".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "mid".to_owned() });

        let evts = state.handle_command(TrameCommand::InsertRow { at: 1 });
        assert!(matches!(evts.display[0], TrameEvent::DataReset));

        assert_eq!(read_cell(&mut state, 0, 0), "top", "row above stays put");
        assert_eq!(read_cell(&mut state, 1, 0), "", "new blank row is empty");
        assert_eq!(read_cell(&mut state, 2, 0), "mid", "old row 1 shifted to row 2");
    }

    /// Deleting a row shifts the rows below it up and drops the deleted row's
    /// cell content (no orphan entry resurfaces).
    #[test]
    fn delete_row_shifts_cells_up_and_purges() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "keep".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "gone".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 2, col: 0, text: "last".to_owned() });

        state.handle_command(TrameCommand::DeleteRow { at: 1 });

        assert_eq!(read_cell(&mut state, 0, 0), "keep");
        assert_eq!(read_cell(&mut state, 1, 0), "last", "row 2 shifted up to row 1");
        // The purged content is gone for good, even after a snapshot round-trip.
        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);
        assert_eq!(read_cell(&mut restored, 0, 0), "keep");
        assert_eq!(read_cell(&mut restored, 1, 0), "last");
    }

    /// Column insert/delete mirror the row behavior on the horizontal axis.
    #[test]
    fn insert_and_delete_column_shift_cells() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "a".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "b".to_owned() });

        state.handle_command(TrameCommand::InsertColumn { at: 1 });
        assert_eq!(read_cell(&mut state, 0, 0), "a");
        assert_eq!(read_cell(&mut state, 0, 1), "", "inserted blank column");
        assert_eq!(read_cell(&mut state, 0, 2), "b", "old col 1 shifted right");

        state.handle_command(TrameCommand::DeleteColumn { at: 1 });
        assert_eq!(read_cell(&mut state, 0, 0), "a");
        assert_eq!(read_cell(&mut state, 0, 1), "b", "shift back after delete");
    }

    /// References are positional and never rewritten (§12): after deleting the
    /// row that held a formula's source, the formula reads whatever now sits at
    /// that grid position -- here an emptied cell. (Full id-based `#REF!` on
    /// delete is a separate broken-references story.)
    #[test]
    fn formula_reference_is_positional_after_delete() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "=7".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "=A2".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 1), "7", "B1 reads A2 before delete");

        // Delete row 1 (the source A2). B1 stays at row 0; A2 is now empty, so
        // the positional reference reads an empty cell.
        state.handle_command(TrameCommand::DeleteRow { at: 1 });
        assert_eq!(read_cell(&mut state, 0, 1), "", "positional ref now empty");
    }

    /// Inserting past the materialized used range is a no-op: virtual rows are
    /// already blank, there is nothing to shift, and no CRDT delta is produced.
    #[test]
    fn insert_beyond_used_range_is_noop() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "x".to_owned() });
        let evts = state.handle_command(TrameCommand::InsertRow { at: 50 });
        assert!(evts.display.is_empty(), "no DataReset");
        assert!(evts.update.is_none(), "no CRDT delta");
        assert_eq!(read_cell(&mut state, 0, 0), "x");
    }

    /// Read the full DisplayCell at a single grid position.
    fn read_display(state: &mut TrameState, row: u32, col: u32) -> crate::DisplayCell {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 5000, row_start: row, row_end: row, col_start: col, col_end: col,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        cells[0].clone()
    }

    /// Merging a range marks the top-left master with its span and renders the
    /// covered cells blank; the merge survives a snapshot round-trip.
    #[test]
    fn merge_marks_master_span_and_blanks_covered() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "title".to_owned() });

        // Merge A1:B2 (2 cols x 2 rows), master = A1.
        let evts = state.handle_command(TrameCommand::MergeCells {
            row: 0, col: 0, row_end: 1, col_end: 1,
        });
        assert!(evts.display.iter().any(|e| matches!(e, TrameEvent::DataReset)));

        let master = read_display(&mut state, 0, 0);
        assert_eq!(master.merge_span, Some((2, 2)), "master spans 2x2");
        assert!(master.is_merged);
        assert_eq!(master.text, "title");

        let covered = read_display(&mut state, 1, 1);
        assert!(covered.is_merged, "covered cell flags merged");
        assert_eq!(covered.merge_span, None, "covered cell has no span");
        assert_eq!(covered.text, "", "covered cell renders blank");

        // Round-trip: the merge is persisted in the CRDT.
        let snapshot = state.export_snapshot();
        let mut restored = TrameState::new();
        restored.import_snapshot(&snapshot);
        assert_eq!(read_display(&mut restored, 0, 0).merge_span, Some((2, 2)));
    }

    /// A merge is rejected locally when a covered cell is non-empty, and no CRDT
    /// delta is produced (Excel/Sheets behavior, §6).
    #[test]
    fn merge_rejected_when_covered_cell_non_empty() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "busy".to_owned() });

        let evts = state.handle_command(TrameCommand::MergeCells {
            row: 0, col: 0, row_end: 0, col_end: 1,
        });
        assert!(
            matches!(&evts.display[0], TrameEvent::MergeRejected { .. }),
            "non-empty covered cell rejects the merge"
        );
        assert!(evts.update.is_none(), "no CRDT mutation on reject");
        // The cells stay independent.
        assert_eq!(read_display(&mut state, 0, 0).merge_span, None);
    }

    /// Unmerging restores the covered cells (span and blanking cleared).
    #[test]
    fn unmerge_restores_covered_cells() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::MergeCells {
            row: 0, col: 0, row_end: 0, col_end: 2,
        });
        assert_eq!(read_display(&mut state, 0, 0).merge_span, Some((3, 1)));

        // Unmerge by pointing at any cell inside the range.
        state.handle_command(TrameCommand::UnmergeCells { row: 0, col: 2 });
        assert_eq!(read_display(&mut state, 0, 0).merge_span, None);
        assert!(!read_display(&mut state, 0, 1).is_merged, "covered cell freed");
    }

    /// A concurrent edit that lands a value in a covered cell is masked, not
    /// destroyed: the value is hidden while the merge is active and reappears on
    /// unmerge (§6, data is never destroyed).
    #[test]
    fn concurrent_value_under_merge_is_masked_then_restored() {
        // Shared base so both peers address rows 0/1 by the *same* stable ids
        // (concurrent dense-list inserts would otherwise create distinct rows).
        // Seeding then clearing A2 materializes rows 0 and 1 without leaving a
        // value behind.
        let mut base = TrameState::new();
        base.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "seed".to_owned() });
        base.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: String::new() });
        let base_snap = base.export_snapshot();

        let mut a = TrameState::new();
        a.import_snapshot(&base_snap);
        let mut b = TrameState::new();
        b.import_snapshot(&base_snap);

        // Peer A merges A1:A2 (both empty on its side).
        a.handle_command(TrameCommand::MergeCells { row: 0, col: 0, row_end: 1, col_end: 0 });
        let merge_delta = a.export_snapshot();

        // Peer B concurrently writes A2, then receives A's merge.
        b.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "concurrent".to_owned() });
        let conflicts = b.apply_remote(&merge_delta);
        assert!(
            conflicts.iter().any(|e| matches!(e, TrameEvent::MergeConflictDetected { .. })),
            "the covered non-empty cell is reported as a conflict"
        );
        // While merged, A2 is masked (blank) but the data still exists.
        assert_eq!(read_display(&mut b, 1, 0).text, "");

        // Unmerge restores the masked value.
        b.handle_command(TrameCommand::UnmergeCells { row: 0, col: 0 });
        assert_eq!(read_display(&mut b, 1, 0).text, "concurrent", "masked value reappears");
    }

    /// A merge follows its cells across a structural insert: inserting a row
    /// above the merge shifts the master span down with it.
    #[test]
    fn merge_follows_structural_insert() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::SetCellText { row: 1, col: 0, text: "x".to_owned() });
        state.handle_command(TrameCommand::MergeCells { row: 1, col: 0, row_end: 1, col_end: 1 });
        assert_eq!(read_display(&mut state, 1, 0).merge_span, Some((2, 1)));

        // Insert a row at the top: the merge shifts from row 1 to row 2.
        state.handle_command(TrameCommand::InsertRow { at: 0 });
        assert_eq!(read_display(&mut state, 1, 0).merge_span, None, "old position freed");
        assert_eq!(read_display(&mut state, 2, 0).merge_span, Some((2, 1)), "merge shifted down");
    }

    /// An edit emits a CRDT delta through the `CommandSink`: absorbing the
    /// outcome captures exactly one `SyncCommand::SendUpdate`, the seam the
    /// production controller drives the Sync thread through.
    #[test]
    fn edit_emits_send_update_through_sink() {
        use crate::test_support::MockSyncHandle;
        use ambre_bus::{DocumentId, SyncCommand};

        let mut state = TrameState::new();
        let sync = MockSyncHandle::new();
        let doc = DocumentId::new();

        let outcome = state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: "hi".to_owned(),
        });
        sync.absorb(doc, &outcome);

        let sent = sync.drain();
        assert_eq!(sent.len(), 1, "one SendUpdate per mutating command");
        assert!(matches!(
            &sent[0],
            SyncCommand::SendUpdate { document_id, payload }
                if *document_id == doc && !payload.is_empty()
        ));
    }

    /// Full two-peer collaboration loop driven through `MockSyncHandle`: peer A
    /// edits, the mock captures and relays the delta to peer B, and B converges
    /// on A's value -- a controller test with no network and no hand-shuffled
    /// snapshots.
    #[test]
    fn two_peers_converge_via_mock_sync() {
        use crate::test_support::MockSyncHandle;
        use ambre_bus::DocumentId;

        let doc = DocumentId::new();
        let mut a = TrameState::new();
        let mut b = TrameState::new();
        let sync = MockSyncHandle::new();

        // A writes A1; the mock absorbs the emitted delta.
        let outcome = a.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: "shared".to_owned(),
        });
        sync.absorb(doc, &outcome);

        // B has not seen the edit yet.
        assert_eq!(read_cell(&mut b, 0, 0), "");

        // Relay A's pending updates to B; B converges.
        let events = sync.deliver_updates_to(&mut b);
        assert!(events.iter().any(|e| matches!(e, TrameEvent::DataReset)));
        assert!(sync.is_empty(), "buffer drained after delivery");
        assert_eq!(read_cell(&mut b, 0, 0), "shared", "B converges on A's value");
    }

    /// Two peers sharing a cross-sheet formula converge when one renames the
    /// target sheet: the reference is bound to the stable `sheet_id` (same id on
    /// both peers via sync), so both keep computing and reconstruct identical
    /// formula-bar text (spec section 2.D).
    #[test]
    fn cross_sheet_reference_converges_across_peers_on_rename() {
        let mut a = TrameState::new();

        // A builds the workbook: Sheet2!A1 = 5, Sheet1!B1 = Sheet2!A1 * 2.
        a.handle_command(TrameCommand::CreateSheet { name: "Sheet2".to_owned() });
        let s2 = second_sheet_id(&a);
        a.handle_command(TrameCommand::ActivateSheet { sheet_id: s2.clone() });
        a.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "5".to_owned() });
        a.handle_command(TrameCommand::ActivateSheet { sheet_id: "s0".to_owned() });
        a.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "=Sheet2!A1*2".to_owned() });

        // B joins by loading A's snapshot (the real "open shared doc" path, so
        // both peers share the same op history) and converges on the value.
        let mut b = TrameState::new();
        b.import_snapshot(&a.export_snapshot());
        assert_eq!(read_cell(&mut b, 0, 1), "10", "B resolves the shared sheet_id");
        assert_eq!(read_raw(&mut b, 0, 1), "=Sheet2!A1*2");

        // B renames the target sheet; A merges B's state (a snapshot carries
        // B's full history, including the implicit-s0 op each peer mints in
        // `new()`, which an incremental delta would leave as a dangling dep).
        b.handle_command(TrameCommand::RenameSheet { sheet_id: s2.clone(), name: "Data".to_owned() });
        a.apply_remote(&b.export_snapshot());

        // Both peers keep computing and reconstruct the same new label.
        assert_eq!(read_cell(&mut a, 0, 1), "10");
        assert_eq!(read_cell(&mut b, 0, 1), "10");
        assert_eq!(read_raw(&mut a, 0, 1), "=Data!A1*2");
        assert_eq!(read_raw(&mut b, 0, 1), "=Data!A1*2");
    }

    /// The id of the first non-default sheet in the current tab order.
    fn second_sheet_id(state: &TrameState) -> String {
        state.sheet_order_ids().into_iter().find(|id| id != super::DEFAULT_SHEET).expect("a second sheet")
    }

    /// Creating a sheet mints a stable id, lists it after the default in the tab
    /// strip, and keeps its cells isolated from the default sheet.
    #[test]
    fn create_sheet_lists_and_isolates() {
        let mut state = TrameState::new();
        let evts = state.handle_command(TrameCommand::CreateSheet { name: "Data".to_owned() });
        let TrameEvent::SheetList { sheets, active } = &evts.display[0] else { panic!("expected SheetList") };
        assert_eq!(active, "s0", "active sheet unchanged by create");
        assert_eq!(sheets.len(), 2);
        assert_eq!(sheets[0], ("s0".to_owned(), "Sheet1".to_owned()), "default tab first");
        assert_eq!(sheets[1].1, "Data");

        let s1 = sheets[1].0.clone();
        // Write on the default sheet, switch to the new one: it is blank.
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "on_s0".to_owned() });
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: s1.clone() });
        assert_eq!(read_cell(&mut state, 0, 0), "", "new sheet starts empty");
        // Write here, switch back: each sheet keeps its own value.
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "on_s1".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 0), "on_s1");
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: "s0".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 0), "on_s0");
    }

    /// A cross-sheet formula on the active sheet evaluates a cell living on
    /// another sheet -- the end-to-end path unlocked by the lifecycle commands.
    #[test]
    fn cross_sheet_formula_evaluates_end_to_end() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet { name: "Sheet2".to_owned() });
        let s2 = second_sheet_id(&state);

        // Put 5 into Sheet2!A1.
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: s2 });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "5".to_owned() });

        // Back on Sheet1, B1 references it.
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: "s0".to_owned() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 1, text: "=Sheet2!A1*2".to_owned() });
        assert_eq!(read_cell(&mut state, 0, 1), "10", "B1 = Sheet2!A1 * 2");
    }

    /// Renaming changes only the display label; the stable id (and the cells
    /// keyed under it) are untouched.
    #[test]
    fn rename_sheet_keeps_id_and_cells() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet { name: "Data".to_owned() });
        let s1 = second_sheet_id(&state);
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: s1.clone() });
        state.handle_command(TrameCommand::SetCellText { row: 0, col: 0, text: "v".to_owned() });

        let evts = state.handle_command(TrameCommand::RenameSheet { sheet_id: s1.clone(), name: "Renamed".to_owned() });
        let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else { panic!() };
        assert!(sheets.iter().any(|(id, name)| id == &s1 && name == "Renamed"));
        // The value survives the rename (same id, same container).
        assert_eq!(read_cell(&mut state, 0, 0), "v");
    }

    /// Moving a sheet reorders the tab strip without touching its content.
    #[test]
    fn move_sheet_reorders_tabs() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet { name: "B".to_owned() });
        let b = second_sheet_id(&state);
        // Order is [s0, B]; move B to the front.
        let evts = state.handle_command(TrameCommand::MoveSheet { sheet_id: b.clone(), to_index: 0 });
        let TrameEvent::SheetList { sheets, .. } = &evts.display[0] else { panic!() };
        assert_eq!(sheets[0].0, b, "B moved to the front");
        assert_eq!(sheets[1].0, "s0");
    }

    /// Deleting the last remaining sheet is rejected; the workbook keeps >= 1.
    #[test]
    fn delete_last_sheet_rejected() {
        let mut state = TrameState::new();
        let evts = state.handle_command(TrameCommand::DeleteSheet { sheet_id: "s0".to_owned() });
        assert!(matches!(evts.display[0], TrameEvent::Error { .. }));
        assert!(evts.update.is_none());
    }

    /// Deleting the active sheet falls back to a neighbour and refreshes the
    /// tab strip; the deleted sheet is gone from the list.
    #[test]
    fn delete_active_sheet_falls_back() {
        let mut state = TrameState::new();
        state.handle_command(TrameCommand::CreateSheet { name: "B".to_owned() });
        let b = second_sheet_id(&state);
        state.handle_command(TrameCommand::ActivateSheet { sheet_id: b.clone() });

        let evts = state.handle_command(TrameCommand::DeleteSheet { sheet_id: b.clone() });
        let list = evts.display.iter().find_map(|e| match e {
            TrameEvent::SheetList { sheets, active } => Some((sheets.clone(), active.clone())),
            _ => None,
        }).expect("SheetList emitted");
        assert_eq!(list.1, "s0", "active fell back to the default sheet");
        assert!(!list.0.iter().any(|(id, _)| id == &b), "deleted sheet gone");
    }

    /// Read column `col` over rows `[0..=row_end]` as display strings.
    fn read_col(state: &mut TrameState, col: u32, row_end: u32, version: u64) -> Vec<String> {
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version, row_start: 0, row_end, col_start: 0, col_end: col,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        (0..=row_end)
            .map(|r| {
                cells
                    .iter()
                    .find(|c| c.row == r && c.col == col)
                    .map(|c| c.text.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Ascending sort reorders whole rows by the key column; cells in other
    /// columns of the same row travel with it (stable-id keyed).
    #[test]
    fn sort_range_ascending_moves_whole_rows() {
        let mut state = TrameState::new();
        // A: 3,1,2  B: "c","a","b" (paired with A so we can check rows move whole)
        for (r, (a, b)) in [("3", "c"), ("1", "a"), ("2", "b")].iter().enumerate() {
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 0, text: a.to_string() });
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 1, text: b.to_string() });
        }
        let evts = state.handle_command(TrameCommand::SortRange {
            sheet_id: "s0".to_owned(),
            row_start: 0, row_end: 2, col_start: 0, col_end: 1,
            sort_col: 0, ascending: true, has_header: false,
        });
        assert!(evts.display.iter().any(|e| matches!(e, TrameEvent::SortComplete)));
        assert!(evts.update.is_some(), "a real reorder broadcasts a delta");
        assert_eq!(read_col(&mut state, 0, 2, 1), vec!["1", "2", "3"]);
        assert_eq!(read_col(&mut state, 1, 2, 2), vec!["a", "b", "c"], "row B follows A");
    }

    /// Descending sort and the `has_header` pin: the first row stays put.
    #[test]
    fn sort_range_descending_keeps_header() {
        let mut state = TrameState::new();
        for (r, a) in ["Header", "1", "3", "2"].iter().enumerate() {
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 0, text: a.to_string() });
        }
        state.handle_command(TrameCommand::SortRange {
            sheet_id: "s0".to_owned(),
            row_start: 0, row_end: 3, col_start: 0, col_end: 0,
            sort_col: 0, ascending: false, has_header: true,
        });
        assert_eq!(read_col(&mut state, 0, 3, 1), vec!["Header", "3", "2", "1"]);
    }

    /// Formula references follow the data through a sort (the section 12
    /// guarantee). Each `=A{n}*10` binds to its row's `Uid` at edit time; the sort
    /// reorders rows by `mov` and the uid-anchored bytecode is preserved (no
    /// reseed/re-parse), so every formula keeps reading the *same physical* A cell
    /// it referenced -- now at a new grid position.
    #[test]
    fn sort_range_formulas_follow_data() {
        let mut state = TrameState::new();
        for (r, a) in ["2", "1"].iter().enumerate() {
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 0, text: a.to_string() });
            // B = A of the same row, times 10 (row0 -> =A1*10 -> 20, row1 -> =A2*10 -> 10).
            let cell = format!("=A{}*10", r + 1);
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 1, text: cell });
        }
        state.handle_command(TrameCommand::SortRange {
            sheet_id: "s0".to_owned(),
            row_start: 0, row_end: 1, col_start: 0, col_end: 1,
            sort_col: 0, ascending: true, has_header: false,
        });
        // Data sorts ascending: A = [1, 2].
        assert_eq!(read_col(&mut state, 0, 1, 1), vec!["1", "2"]);
        // References follow the data: the `=A2*10` cell (bound to the row whose
        // A was 1) is now at pos0 and still reads 1 -> 10; the `=A1*10` cell
        // (bound to the row whose A was 2) is now at pos1 and reads 2 -> 20.
        assert_eq!(read_col(&mut state, 1, 1, 2), vec!["10", "20"]);
    }

    /// A range is a positional *block*, not a set of specific rows: sorting the
    /// rows inside `SUM(A1:A3)` reorders the same three values, so the block (and
    /// its total) is unchanged -- the Excel/Sheets behavior, and the only
    /// well-defined one (uid-bound endpoints would scatter and invert the range).
    /// Contrast `sort_range_formulas_follow_data`, where single-cell refs *do*
    /// follow their row.
    #[test]
    fn sort_range_sum_block_is_stable_under_internal_sort() {
        let mut state = TrameState::new();
        for (r, a) in ["3", "1", "2"].iter().enumerate() {
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 0, text: a.to_string() });
        }
        // A4 lives below the sorted block and sums the whole block, plus a
        // sub-block A1:A2 whose endpoints would invert under a naive uid binding.
        state.handle_command(TrameCommand::SetCellText { row: 3, col: 0, text: "=SUM(A1:A3)".into() });
        state.handle_command(TrameCommand::SetCellText { row: 4, col: 0, text: "=SUM(A1:A2)".into() });
        assert_eq!(read_col(&mut state, 0, 4, 1)[3], "6");
        assert_eq!(read_col(&mut state, 0, 4, 1)[4], "4"); // 3 + 1

        state.handle_command(TrameCommand::SortRange {
            sheet_id: "s0".to_owned(),
            row_start: 0, row_end: 2, col_start: 0, col_end: 0,
            sort_col: 0, ascending: true, has_header: false,
        });
        // Data sorts to [1, 2, 3]; the SUM cells below are untouched.
        assert_eq!(read_col(&mut state, 0, 2, 2), vec!["1", "2", "3"]);
        // Whole block A1:A3 still sums all three -> 6 (no #REF!, no collapse).
        assert_eq!(read_col(&mut state, 0, 4, 3)[3], "6");
        // Sub-block A1:A2 is positions 0..1 -> 1 + 2 = 3.
        assert_eq!(read_col(&mut state, 0, 4, 3)[4], "3");
    }

    /// A sort targeting a non-active sheet, or an already-sorted range, makes no
    /// CRDT mutation.
    #[test]
    fn sort_range_noops() {
        let mut state = TrameState::new();
        for (r, a) in ["1", "2"].iter().enumerate() {
            state.handle_command(TrameCommand::SetCellText { row: r as u32, col: 0, text: a.to_string() });
        }
        // Wrong sheet: ignored.
        let other = state.handle_command(TrameCommand::SortRange {
            sheet_id: "s9".to_owned(),
            row_start: 0, row_end: 1, col_start: 0, col_end: 0,
            sort_col: 0, ascending: true, has_header: false,
        });
        assert!(other.update.is_none());
        assert!(other.display.is_empty());
        // Already ascending: acknowledged but no delta.
        let sorted = state.handle_command(TrameCommand::SortRange {
            sheet_id: "s0".to_owned(),
            row_start: 0, row_end: 1, col_start: 0, col_end: 0,
            sort_col: 0, ascending: true, has_header: false,
        });
        assert!(sorted.update.is_none(), "no reorder, no broadcast");
        assert!(sorted.display.iter().any(|e| matches!(e, TrameEvent::SortComplete)));
    }

    #[test]
    fn col_alpha_basics() {
        assert_eq!(col_to_alpha(0), "A");
        assert_eq!(col_to_alpha(25), "Z");
        assert_eq!(col_to_alpha(26), "AA");
        assert_eq!(col_to_alpha(51), "AZ");
        assert_eq!(col_to_alpha(52), "BA");
        assert_eq!(col_to_alpha(701), "ZZ");
    }
}
