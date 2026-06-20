// ambre-trame/src/logic/clipboard.rs
// Clipboard paste (`ambre-spec-trame-ui.md` sec.5.E).
//
// Copy/cut are pure UI-thread operations (they read the viewport cache and write
// the OS clipboard); only paste reaches the Logic thread, because reindexing a
// formula's relative references and applying styles both need the document model
// the UI does not hold (sec.16). Submodule of `logic`, so it reaches the private
// fields/helpers of the parent through `use super::*`.

use super::*;
use crate::formula::shift::shift_formula;

impl TrameState {
    /// Paste `payload` with its top-left landing on `(target_row, target_col)`
    /// (`-ui.md` sec.5.E). Each pasted cell lands at `(target_row + dr,
    /// target_col + dc)`; a formula's relative references shift by the same
    /// `(target - src)` offset (every reference is relative -- AMBRE has no `$`
    /// syntax), and the carried [`CellStyle`] (plus bold/italic) is written
    /// verbatim. The whole paste is a single undo group, so one paste = one undo
    /// item. Blank source cells are omitted from the payload and leave their
    /// destination untouched (Excel/Sheets behaviour). Returns the standard write
    /// outcome (a `DataReset` so the UI re-fetches the pasted region, plus the
    /// CRDT delta).
    pub(super) fn paste_range(
        &mut self,
        target_row: u32,
        target_col: u32,
        payload: ClipboardPayload,
    ) -> TrameOutcome {
        if payload.cells.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // The reindex offset, constant across every pasted formula.
        let dr = target_row as i64 - payload.src_row as i64;
        let dc = target_col as i64 - payload.src_col as i64;

        // One undo group for the whole paste (sealed by the next command's
        // fallback `group_end`). Pre-materialize every written cell first so the
        // value writes collapse into a single trailing commit -> one undo item
        // (same reasoning as `SetCellsBatch`, see `commands.rs`).
        let _ = self.undo_manager.group_start();
        for cell in &payload.cells {
            if !cell.raw.is_empty() {
                let _ = self.materialize(target_row + cell.dr, target_col + cell.dc);
            }
        }

        for cell in &payload.cells {
            let row = target_row + cell.dr;
            let col = target_col + cell.dc;
            // A formula's relative refs reindex by the paste offset; a literal is
            // copied verbatim. The shifted text is in display form -- `write_cell`
            // re-canonicalizes it, so cross-sheet qualifiers survive (§2.D).
            let text = if cell.raw.starts_with('=') {
                shift_formula(&cell.raw, dr, dc)
            } else {
                cell.raw.clone()
            };
            self.write_cell(row, col, &text);
            self.apply_pasted_style(row, col, cell);
        }

        self.finish_cell_writes()
    }

    /// Paste plain tab-separated `text` at the tracked active cell (`-ui.md`
    /// sec.5.E), each field a literal cell. Source origin = the active cell, so
    /// the reindex offset is zero (external text is never formula-reindexed).
    /// Backs the toolbar/menu Paste button, which reads the OS clipboard
    /// asynchronously and holds no cursor.
    pub(super) fn paste_text(&mut self, text: &str) -> TrameOutcome {
        let (row, col) = self.active;
        let normalized = text.replace('\r', "");
        let mut cells = Vec::new();
        let mut rows = 0u32;
        let mut cols = 0u32;
        for (dr, line) in normalized.trim_end_matches('\n').split('\n').enumerate() {
            rows = rows.max(dr as u32 + 1);
            for (dc, field) in line.split('\t').enumerate() {
                cols = cols.max(dc as u32 + 1);
                if field.is_empty() {
                    continue;
                }
                cells.push(ClipboardCell {
                    dr: dr as u32,
                    dc: dc as u32,
                    raw: field.to_owned(),
                    ..Default::default()
                });
            }
        }
        if cells.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // src == target -> zero shift, so any "=..." in external text stays as-is.
        let payload = ClipboardPayload { src_row: row, src_col: col, rows, cols, cells };
        self.paste_range(row, col, payload)
    }

    /// Write a pasted cell's formatting into the style containers, mirroring the
    /// per-cell tail of [`set_cell_style`](Self::set_cell_style): bold/italic live
    /// in their own sets plus the format map, the rest in the rich-style map.
    /// Borders are taken verbatim from the source cell (no selection-level preset
    /// resolution -- the source already carries resolved per-edge borders).
    fn apply_pasted_style(&mut self, row: u32, col: u32, cell: &ClipboardCell) {
        let pos = (row, col);
        if cell.bold {
            self.bold.insert(pos);
        } else {
            self.bold.remove(&pos);
        }
        if cell.italic {
            self.italic.insert(pos);
        } else {
            self.italic.remove(&pos);
        }
        let _ = self.persist_format(row, col);

        // A default style removes the entry so the map never accumulates dead keys.
        if cell.style == CellStyle::default() {
            self.styles.remove(&pos);
        } else {
            self.styles.insert(pos, cell.style.clone());
        }
        let _ = self.persist_style(row, col, &cell.style);
    }
}
