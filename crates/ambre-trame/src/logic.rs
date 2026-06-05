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

use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameEvent};

// -----------------------------------------------------------------------------
// TrameState
// -----------------------------------------------------------------------------

/// Per-document state for a Trame (spreadsheet) document.
///
/// One instance per open document, owned by the Logic thread's `DocState`
/// registry keyed by `document_id`. Empty for now (cell content is generated
/// from coordinates); it will hold the `LoroDoc` and formula engine once CRDT
/// editing is wired.
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
}

impl TrameState {
    pub fn new() -> Self {
        Self {
            active: (0, 0),
            bold: HashSet::new(),
            italic: HashSet::new(),
        }
    }

    /// Handle a UI command and return display events for the UI Thread.
    ///
    /// Must complete in < 50 ms (P2); scroll requests in < 10 ms (P1).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> Vec<TrameEvent> {
        match cmd {
            TrameCommand::SetActiveCell { row, col } => {
                self.active = (row, col);
                // Report the new active cell's formatting so the chrome toggle
                // buttons reflect the selection.
                vec![
                    TrameEvent::BoldState { enabled: self.bold.contains(&self.active) },
                    TrameEvent::ItalicState { enabled: self.italic.contains(&self.active) },
                ]
            }
            TrameCommand::ToggleBold => {
                let enabled = toggle(&mut self.bold, self.active);
                vec![
                    TrameEvent::BoldState { enabled },
                    TrameEvent::CellFormat {
                        row: self.active.0,
                        col: self.active.1,
                        bold: enabled,
                        italic: self.italic.contains(&self.active),
                    },
                ]
            }
            TrameCommand::ToggleItalic => {
                let enabled = toggle(&mut self.italic, self.active);
                vec![
                    TrameEvent::ItalicState { enabled },
                    TrameEvent::CellFormat {
                        row: self.active.0,
                        col: self.active.1,
                        bold: self.bold.contains(&self.active),
                        italic: enabled,
                    },
                ]
            }
            TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end } => {
                // Clamp to grid bounds.
                let row_end = row_end.min(TOTAL_ROWS.saturating_sub(1));
                let col_end = col_end.min(TOTAL_COLS.saturating_sub(1));

                if row_start > row_end || col_start > col_end {
                    return vec![];
                }

                let mut cells = Vec::with_capacity(
                    ((row_end - row_start + 1) * (col_end - col_start + 1)) as usize,
                );

                for row in row_start..=row_end {
                    for col in col_start..=col_end {
                        cells.push(DisplayCell {
                            row,
                            col,
                            text: format!("{}{}", col_to_alpha(col), row + 1),
                            bold: self.bold.contains(&(row, col)),
                            italic: self.italic.contains(&(row, col)),
                        });
                    }
                }

                vec![TrameEvent::ViewportData { version, row_start, col_start, cells }]
            }
        }
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
    ///
    /// Must be a valid `LoroDoc::export_snapshot()` payload. Stubbed (empty)
    /// until CRDT editing is wired.
    pub fn export_snapshot(&self) -> Vec<u8> {
        vec![]
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
        assert!(matches!(evts[0], TrameEvent::BoldState { enabled: true }));
        assert!(matches!(
            evts[1],
            TrameEvent::CellFormat { row: 0, col: 0, bold: true, italic: false }
        ));

        // Move the selection to B2 (1,1): the active cell is not bold, so the
        // button reports false -- A1 keeps its bold.
        let evts = state.handle_command(TrameCommand::SetActiveCell { row: 1, col: 1 });
        assert!(matches!(evts[0], TrameEvent::BoldState { enabled: false }));

        // A viewport covering both cells: only A1 carries the bold run.
        let evts = state.handle_command(TrameCommand::ScrollViewport {
            version: 1,
            row_start: 0,
            row_end: 1,
            col_start: 0,
            col_end: 1,
        });
        let TrameEvent::ViewportData { cells, .. } = &evts[0] else {
            panic!("expected ViewportData");
        };
        let a1 = cells.iter().find(|c| c.row == 0 && c.col == 0).unwrap();
        let b2 = cells.iter().find(|c| c.row == 1 && c.col == 1).unwrap();
        assert!(a1.bold, "A1 must stay bold");
        assert!(!b2.bold, "B2 must not become bold");
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
