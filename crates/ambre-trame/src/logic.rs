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
#[derive(Default)]
pub struct TrameState {}

impl TrameState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle a UI command and return display events for the UI Thread.
    ///
    /// Must complete in < 50 ms (P2); scroll requests in < 10 ms (P1).
    pub fn handle_command(&mut self, cmd: TrameCommand) -> Vec<TrameEvent> {
        match cmd {
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
    use super::col_to_alpha;

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
