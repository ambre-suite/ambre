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

use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

use crate::formula::{CellRef, FormulaEngine};
use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

/// Name of the Loro map holding edited cell text, keyed by `"row:col"`.
const CELLS_MAP: &str = "cells";

/// Name of the Loro map holding per-cell formatting bitmasks, keyed by
/// `"row:col"`. Source of truth for bold/italic so they survive a reload and
/// replicate to collaborators, exactly like edited cell text.
const FORMAT_MAP: &str = "format";

/// Bit flags packed into the `format` map's i64 value.
const FMT_BOLD: i64 = 1 << 0;
const FMT_ITALIC: i64 = 1 << 1;

/// Loro map key for a cell. Stable, collision-free (decimal, single separator).
fn cell_key(row: u32, col: u32) -> String {
    format!("{row}:{col}")
}

/// Parse a `"row:col"` cell key back into coordinates. Inverse of [`cell_key`].
fn parse_cell_key(key: &str) -> Option<(u32, u32)> {
    let (row, col) = key.split_once(':')?;
    Some((row.parse().ok()?, col.parse().ok()?))
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
    /// Cells carrying a bold run. Keyed by `(row, col)`; absence means regular
    /// weight. A `HashSet` (not a flag on the cell) because cell content is
    /// generated on the fly -- formatting is the only per-cell state we persist
    /// until the CRDT model lands.
    bold: HashSet<(u32, u32)>,
    /// Cells carrying an italic run.
    italic: HashSet<(u32, u32)>,
    /// CRDT source of truth for **edited** cell text: a Loro map keyed by
    /// `"row:col"` -> plaintext. A cell absent from the map shows its generated
    /// coordinate label (A1, B2, ...). This is the first wiring of the document
    /// model; bold/italic stay in the sets above until they move to Loro runs.
    doc: LoroDoc,
    /// Last CRDT version vector exported to Sync. Used to produce incremental
    /// deltas via [`ExportMode::Updates`].
    last_exported_vv: VersionVector,
    /// Formula engine: dependency graph, bytecode cache, dirty recomputation.
    engine: FormulaEngine,
}

impl TrameState {
    pub fn new() -> Self {
        Self {
            active: (0, 0),
            bold: HashSet::new(),
            italic: HashSet::new(),
            doc: LoroDoc::new(),
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
        // Plain-text cells: return CRDT value or generated label.
        let map = self.doc.get_map(CELLS_MAP);
        match map.get(&cell_key(row, col)).map(|v| v.get_deep_value()) {
            Some(LoroValue::String(s)) => s.to_string(),
            _ => format!("{}{}", col_to_alpha(col), row + 1),
        }
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
                let map = self.doc.get_map(CELLS_MAP);
                let key = cell_key(row, col);
                if text.is_empty() {
                    let _ = map.delete(&key);
                    self.engine.on_cell_changed(CellRef::new(row, col), "");
                } else {
                    let _ = map.insert(&key, text.as_str());
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
        let map = self.doc.get_map(FORMAT_MAP);
        let key = cell_key(row, col);
        if mask == 0 {
            let _ = map.delete(&key);
        } else {
            let _ = map.insert(&key, mask);
        }
        self.doc.commit();
        self.flush_delta()
    }

    /// Rebuild the in-memory bold/italic sets from the CRDT `format` map. Called
    /// after any import so a reload or a peer's formatting edit is reflected.
    fn reseed_formatting(&mut self) {
        self.bold.clear();
        self.italic.clear();
        if let LoroValue::Map(fmts) = self.doc.get_map(FORMAT_MAP).get_value() {
            for (key, value) in fmts.iter() {
                if let (Some((row, col)), LoroValue::I64(mask)) = (parse_cell_key(key), value) {
                    if mask & FMT_BOLD != 0 {
                        self.bold.insert((row, col));
                    }
                    if mask & FMT_ITALIC != 0 {
                        self.italic.insert((row, col));
                    }
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
        let cell_count = match self.doc.get_map(CELLS_MAP).get_value() {
            LoroValue::Map(cells) => cells.len(),
            _ => 0,
        };
        tracing::info!("[Logic] {origin} imported {} bytes, {cell_count} cell(s) now present", payload.len());
        // Advance the cursor to include the imported ops: we did not author them
        // (or already persisted them), so the next `flush_delta` must not export
        // them back. Local edits are flushed each frame before remote applies, so
        // there are no pending local-only ops to swallow here.
        self.last_exported_vv = self.doc.oplog_vv();
        // Reseed the formula engine from the resulting cell values.
        if let LoroValue::Map(cells) = self.doc.get_map(CELLS_MAP).get_value() {
            for (key, value) in cells.iter() {
                if let (Some((row, col)), LoroValue::String(text)) = (parse_cell_key(key), value) {
                    self.engine.on_cell_changed(CellRef::new(row, col), text.as_str());
                }
            }
        }
        self.engine.compute_all_dirty();
        // Rebuild bold/italic from the merged `format` map (reload or peer edit).
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
