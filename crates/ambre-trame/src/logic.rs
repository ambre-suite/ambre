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

use std::collections::HashSet;

use loro::cursor::Side;
use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

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

// -----------------------------------------------------------------------------
// TrameState
// -----------------------------------------------------------------------------

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
}

impl TrameState {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        // Register the implicit default sheet's name (idempotent map insert, so
        // a peer that also "creates" s0 converges to the same entry).
        let _ = doc.get_map(SHEETS_MAP).insert(DEFAULT_SHEET, "Sheet1");
        doc.commit();
        Self {
            active: (0, 0),
            active_sheet: DEFAULT_SHEET.to_owned(),
            bold: HashSet::new(),
            italic: HashSet::new(),
            doc,
            index: SheetIndex::default(),
            last_exported_vv: VersionVector::default(),
            engine: FormulaEngine::new(),
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
    /// cell, otherwise the edited value from the CRDT if present, otherwise the
    /// generated coordinate label.
    fn cell_text(&self, row: u32, col: u32) -> String {
        let cell = CellRef::new(row, col);
        // Formula cells: return cached evaluation result.
        if let Some(text) = self.engine.evaluated_text(cell) {
            return text;
        }
        // Plain-text cells: resolve the grid position to its stable id pair and
        // read the CRDT value. A virtual (never-written) cell, or one cleared
        // back to empty, shows its generated coordinate label.
        if let Some((rl, cl)) = self.index.resolve(row as usize, col as usize) {
            let map = self.doc.get_map(cells_map_name(&self.active_sheet));
            if let Some(LoroValue::String(s)) = map.get(&cell_key(rl, cl)).map(|v| v.get_deep_value()) {
                return s.to_string();
            }
        }
        format!("{}{}", col_to_alpha(col), row + 1)
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
        let rows = self.doc.get_movable_list(rows_list_name(&self.active_sheet));
        while self.index.row_len() <= row {
            rows.push(LoroValue::Null).ok()?;
            let id = rows.get_cursor(rows.len() - 1, Side::Middle).and_then(|c| c.id)?;
            self.index.push_row(id);
        }
        let cols = self.doc.get_movable_list(cols_list_name(&self.active_sheet));
        while self.index.col_len() <= col {
            cols.push(LoroValue::Null).ok()?;
            let id = cols.get_cursor(cols.len() - 1, Side::Middle).and_then(|c| c.id)?;
            self.index.push_col(id);
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
                        cells.push(DisplayCell {
                            row,
                            col,
                            text: self.cell_text(row, col),
                            bold: self.bold.contains(&(row, col)),
                            italic: self.italic.contains(&(row, col)),
                        });
                    }
                }

                TrameOutcome::display_only(vec![TrameEvent::ViewportData { version, row_start, col_start, cells }])
            }
        }
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
        // Reseed the formula engine from the resulting cell values, mapping each
        // stable key pair back to its grid position via the rebuilt index.
        if let LoroValue::Map(cells) = cells_map.get_value() {
            for (key, value) in cells.iter() {
                let (Some((rl, cl)), LoroValue::String(text)) = (parse_cell_key(key), value) else {
                    continue;
                };
                if let (Some(row), Some(col)) = (self.index.row_pos(&rl), self.index.col_pos(&cl)) {
                    self.engine.on_cell_changed(CellRef::new(row as u32, col as u32), text.as_str());
                }
            }
        }
        self.engine.compute_all_dirty();
        // Rebuild bold/italic from the merged `format_{sheet}` map.
        self.reseed_formatting();
        // Refresh the toolbar / context-menu toggles for the active cell: after a
        // reload its formatting is only known once the `format` map is restored.
        vec![
            TrameEvent::DataReset,
            TrameEvent::BoldState { enabled: self.bold.contains(&self.active) },
            TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) },
        ]
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
    /// label; clearing it falls back to the coordinate label.
    #[test]
    fn edited_text_overrides_generated_label() {
        let mut state = TrameState::new();

        // A1 starts as its generated label.
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 1, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "A1");

        // Edit A1, then read it back: the stored text wins.
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: "hello".to_owned(),
        });
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 2, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "hello");

        // Clearing it restores the generated label.
        state.handle_command(TrameCommand::SetCellText {
            row: 0, col: 0, text: String::new(),
        });
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 3, row_start: 0, row_end: 0, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "A1");
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
        // An untouched far cell still shows its generated label (virtual space).
        let evts = restored.handle_command(TrameCommand::ScrollViewport {
            version: 2, row_start: 2000, row_end: 2000, col_start: 0, col_end: 0,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts.display[0] else { panic!() };
        assert_eq!(cells[0].text, "A2001");
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
