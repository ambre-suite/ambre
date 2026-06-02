// ambre-trame/src/logic.rs
// ============================================================================
// Trame (spreadsheet) AppController -- Logic thread side.
//
// Behind the `logic` feature so that crates wiring only the UI side
// (ambre-app) don't pull the CRDT / formula-engine dependency tree.
//
// The shared IPC types (TrameCommand, TrameViewUpdate, DisplayCell) live in
// `crate::events` and are always compiled; the grid dimensions shared with the
// UI live in the crate root (`crate::TOTAL_ROWS` / `crate::TOTAL_COLS`).
//
// Cell content (hardcoded for now): standard spreadsheet reference notation,
// e.g. column 0 row 0 -> "A1", column 26 row 99 -> "AA100".
// ============================================================================

use ambre_bus::{AppController, CommandSink, DocType, LogicEvent};

use crate::{DisplayCell, TOTAL_COLS, TOTAL_ROWS, TrameCommand, TrameViewUpdate};

// -----------------------------------------------------------------------------
// TrameController
// -----------------------------------------------------------------------------

/// Controller for Trame (spreadsheet) documents.
///
/// Instantiated by the Logic thread's controller registry, keyed by
/// `document_id`.
pub struct TrameController {
    /// Sink to the Sync thread. Will be used when CRDT edits are implemented.
    #[allow(dead_code)]
    sync_tx: Box<dyn CommandSink>,
}

impl TrameController {
    pub fn new(sync_tx: Box<dyn CommandSink>) -> Self {
        Self { sync_tx }
    }
}

impl AppController for TrameController {
    fn doc_type(&self) -> DocType {
        DocType::Trame
    }

    fn handle_ui_command(&mut self, cmd: Vec<u8>) -> Vec<Vec<u8>> {
        let cmd = match postcard::from_bytes::<TrameCommand>(&cmd) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("TrameController: deserialize TrameCommand: {e}");
                return vec![];
            }
        };

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

                let update = TrameViewUpdate::ViewportData {
                    version,
                    row_start,
                    col_start,
                    cells,
                };

                match postcard::to_allocvec(&update) {
                    Ok(bytes) => vec![bytes],
                    Err(e) => {
                        tracing::error!("TrameController: serialize TrameViewUpdate: {e}");
                        vec![]
                    }
                }
            }
        }
    }

    fn on_logic_event(&mut self, _event: LogicEvent) -> Vec<Vec<u8>> {
        vec![]
    }

    fn export_snapshot(&self) -> Vec<u8> {
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
