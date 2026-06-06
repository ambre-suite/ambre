// ambre-trame/src/logic.rs
// ============================================================================
// Trame (spreadsheet) per-document model -- Logic thread side.
//
// `TrameState` owns everything the Logic thread needs for one open Trame
// document (CRDT / formula engine, once wired). The Logic dispatcher
// (`ambre-logic`) holds it inside `DocState::Trame` and calls the handlers
// below with **typed** `TrameCommand`s, getting back typed `TrameEvent`s --
// no trait object, no postcard, no `Vec<u8>` on this path.
//
// The shared IPC types (TrameCommand, TrameEvent, DisplayCell) live in
// `ambre-bus`; the grid dimensions shared with the UI live in the crate root
// (`crate::TOTAL_ROWS` / `crate::TOTAL_COLS`).
//
// Cell content (hardcoded for now): standard spreadsheet reference notation,
// e.g. column 0 row 0 -> "A1", column 26 row 99 -> "AA100".
// ============================================================================

use std::collections::HashSet;

use loro::{ExportMode, LoroDoc, LoroValue, VersionVector};

use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

/// Name of the Loro map holding edited cell text, keyed by `"row:col"`.
const CELLS_MAP: &str = "cells";

/// Loro map key for a cell. Stable, collision-free (decimal, single separator).
fn cell_key(row: u32, col: u32) -> String {
    format!("{row}:{col}")
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
}

impl TrameState {
    pub fn new() -> Self {
        Self {
            active: (0, 0),
            bold: HashSet::new(),
            italic: HashSet::new(),
            doc: LoroDoc::new(),
            last_exported_vv: VersionVector::default(),
        }
    }

    /// Display text for a cell: the edited value from the CRDT if present,
    /// otherwise the generated coordinate label.
    fn cell_text(&self, row: u32, col: u32) -> String {
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
                // Commit the edit into the CRDT. Empty text clears the cell back
                // to its generated label. The UI already updated its cache
                // optimistically, so no display event is echoed.
                let map = self.doc.get_map(CELLS_MAP);
                let key = cell_key(row, col);
                if text.is_empty() {
                    let _ = map.delete(&key);
                } else {
                    let _ = map.insert(&key, text);
                }
                self.doc.commit();
                let update = self.flush_delta();
                TrameOutcome { display: vec![], update, ephemeral: None }
            }
            TrameCommand::ToggleBold => {
                let enabled = toggle(&mut self.bold, self.active);
                TrameOutcome::display_only(vec![
                    TrameEvent::BoldState { enabled },
                    TrameEvent::CellFormat {
                        row: self.active.0,
                        col: self.active.1,
                        bold: enabled,
                        italic: self.italic.contains(&self.active),
                    },
                ])
            }
            TrameCommand::ToggleItalic => {
                let enabled = toggle(&mut self.italic, self.active);
                TrameOutcome::display_only(vec![
                    TrameEvent::ItalicState { enabled },
                    TrameEvent::CellFormat {
                        row: self.active.0,
                        col: self.active.1,
                        bold: self.bold.contains(&self.active),
                        italic: enabled,
                    },
                ])
            }
            TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end } => {
                // Clamp to grid bounds.
                let row_end = row_end.min(TOTAL_ROWS.saturating_sub(1));
                let col_end = col_end.min(TOTAL_COLS.saturating_sub(1));

                if row_start > row_end || col_start > col_end {
                    return TrameOutcome::display_only(vec![]);
                }

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

    /// Export any new CRDT ops since `last_exported_vv` and bump the tracking
    /// cursor. Returns `Some` binary delta if there were new ops.
    fn flush_delta(&mut self) -> Option<Vec<u8>> {
        let current = self.doc.oplog_vv();
        if current == self.last_exported_vv {
            return None;
        }
        let delta = self.doc.export(ExportMode::updates(&self.last_exported_vv)).ok();
        self.last_exported_vv = current;
        delta
    }

    /// Apply a remote CRDT delta / snapshot and return display events.
    ///
    /// `payload` is the raw (decrypted) Loro binary delta forwarded by the Sync
    /// thread. Stubbed until CRDT editing is wired.
    pub fn apply_remote(&mut self, _payload: &[u8]) -> Vec<TrameEvent> {
        // TODO: import the Loro delta and recompute the affected viewport.
        vec![]
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
