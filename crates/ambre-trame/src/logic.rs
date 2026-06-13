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

use loro::cursor::Side;
use loro::{ExportMode, LoroDoc, LoroValue, UndoManager, VersionVector};

use crate::formula::{CellRef, FormulaEngine};
use crate::model::{Lid, SheetIndex, cell_key, parse_cell_key};
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
    /// Local undo/redo history (§8). Subscribes to [`Self::doc`] and records one
    /// item per commit; each `TrameCommand` handler commits exactly once, so one
    /// command maps to one undo step. Scope is the open-document session: it only
    /// knows this peer's commits, not imported remote ops. `undo`/`redo` mutate
    /// it, hence it is created after the constructor's initial commit so that the
    /// implicit-sheet registration is never undoable.
    undo_manager: UndoManager,
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
        Self {
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
            undo_manager,
        }
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
        let cell = CellRef::new(row, col);
        // Formula cells: return cached evaluation result.
        if let Some(text) = self.engine.evaluated_text(cell) {
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

    /// Ensure grid `(row, col)` is materialized (its row and column exist in the
    /// `rows_/cols_` lists) and return the `(row_lid, col_lid)` pair to key the
    /// cell with. Extends the dense used range by pushing placeholder elements,
    /// capturing each new element's stable id. Returns `None` -- dropping the
    /// write -- if the jump past the high-water mark exceeds [`MATERIALIZE_CAP`].
    fn materialize(&mut self, row: u32, col: u32) -> Option<(Lid, Lid)> {
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
        let rows = self.doc.get_movable_list(rows_list_name(&self.active_sheet));
        while self.index.row_len() <= row {
            rows.push(LoroValue::Null).ok()?;
            let id = rows.get_cursor(rows.len() - 1, Side::Middle).and_then(|c| c.id)?;
            self.index.push_row(id);
            grew = true;
        }
        let cols = self.doc.get_movable_list(cols_list_name(&self.active_sheet));
        while self.index.col_len() <= col {
            cols.push(LoroValue::Null).ok()?;
            let id = cols.get_cursor(cols.len() - 1, Side::Middle).and_then(|c| c.id)?;
            self.index.push_col(id);
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
                    self.engine.on_cell_changed(CellRef::new(row, col), "");
                } else {
                    let Some((rl, cl)) = self.materialize(row, col) else {
                        // Beyond the materialization cap: drop the write.
                        return TrameOutcome::display_only(vec![]);
                    };
                    let _ = self.doc.get_map(cells_map_name(&self.active_sheet)).insert(&cell_key(rl, cl), text.as_str());
                    self.engine.on_cell_changed(CellRef::new(row, col), &text);
                }
                self.doc.commit();
                self.engine.compute_all_dirty();
                let update = self.flush_delta();
                TrameOutcome { display: vec![], update, ephemeral: None }
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
                self.engine.compute_all_dirty();

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
        if self.doc.get_movable_list(list_name).insert(at, LoroValue::Null).is_err() {
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
    fn purge_axis(&mut self, removed: Lid, axis: Axis) {
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
        if let LoroValue::Map(cells) = self.doc.get_map(cells_map_name(&self.active_sheet)).get_value() {
            for (key, value) in cells.iter() {
                // Masked cells (covered by a merge, conflicting value) stay out of
                // the DAG: they are neither displayed nor computed while masked.
                if self.masked_cells.contains(key) {
                    continue;
                }
                let (Some((rl, cl)), LoroValue::String(text)) = (parse_cell_key(key), value) else {
                    continue;
                };
                if let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl)) {
                    self.engine.on_cell_changed(CellRef::new(row as u32, col as u32), text.as_str());
                }
            }
        }
        self.engine.compute_all_dirty();
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
        // A1 is a formula literal so the engine ingests its value (plain numeric
        // literals feeding formulas are a separate engine concern, out of scope).
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
        other.push(loro::LoroValue::Null).unwrap();
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
