// ambre-trame/src/logic/clipboard.rs Clipboard copy and paste (`trame-clipboard`).
//
// A grid copy or cut reaches the Logic thread. The UI sends `CopyRange`, and
// `copy_range` reads the whole selection from the document, never a viewport. Only an in-cell text selection is copied on the UI thread alone.
// A paste also runs here: reindexing a formula's relative references and
// applying styles both need the document model the UI does not hold. A cut
// clears its source later, through the UI's `SetCellsBatch` after the paste.
// Submodule of `logic`, so it reaches the parent's private items through
// `use super::*`.

use super::*;
use crate::formula::shift::shift_formula;
use crate::formula::text::CellText;
use ambre_core::index::Len;

impl TrameState {
    /// Paste `payload` with its top-left landing on `target`
    /// (`trame-clipboard`). Each pasted cell lands `(dr, dc)` from it.
    ///
    /// A formula's relative references shift by the same `(target - src)` offset,
    /// since every reference is relative and AMBRE has no `$` syntax. The carried
    /// [`CellStyle`] (plus bold/italic) is written verbatim.
    ///
    /// The whole paste is a single undo group, so one paste = one undo item. Blank
    /// source cells are omitted from the payload and leave their destination
    /// untouched (Excel/Sheets behaviour). Returns the standard write outcome (a
    /// `DataReset` so the UI re-fetches the pasted region, plus the CRDT delta).
    pub(super) fn paste_range(
        &mut self,
        target: CellPos,
        payload: ClipboardPayload,
    ) -> TrameOutcome {
        if payload.cells.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // The reindex offset, constant across every pasted formula.
        let dr = target.row - payload.src_row;
        let dc = target.col - payload.src_col;
        let land = |cell: &ClipboardCell| CellPos::new(target.row + cell.dr, target.col + cell.dc);

        for cell in &payload.cells {
            if !cell.raw.is_empty() {
                let _ = self.materialize(land(cell));
            }
        }

        for cell in &payload.cells {
            let at = land(cell);
            // A formula's relative refs reindex by the paste offset; a literal is
            // copied verbatim. The shifted text is in display form -- `write_cell`
            // re-canonicalizes it, so cross-sheet qualifiers survive
            // (`trame-sheets`).
            let raw = CellText::<SourceForm>::new(cell.raw.clone());
            let text = if raw.is_formula() {
                shift_formula(&raw, dr, dc)
            } else {
                raw
            };
            // The value and the style both need the cell's lines. A cell short
            // of them is refused once, and the refusal is reported, as a fill
            // reports it. A cell with no line is blank and unstyled already, so
            // an empty unstyled paste leaves nothing to refuse (`TRAME-243`).
            if let Err(refused) = self.materialize(at) {
                let styled = cell.bold == Weight::Bold
                    || cell.italic == Slant::Italic
                    || cell.style != CellStyle::default();
                if !text.as_str().is_empty() || styled {
                    self.write_refusals.push(WriteRefusal::from(refused));
                }
                continue;
            }
            self.write_cell(at, &text);
            self.apply_pasted_style(at, cell);
        }

        self.finish_cell_writes()
    }

    /// Paste plain tab-separated `text` at the tracked active cell
    /// (`trame-clipboard`), each field a literal cell. Source origin = the active
    /// cell, so the reindex offset is zero (external text is never reindexed).
    /// Backs the toolbar/menu Paste button, which reads the OS clipboard
    /// asynchronously and holds no cursor.
    pub(super) fn paste_text(&mut self, text: &str) -> TrameOutcome {
        let origin = self.active;
        let normalized = text.replace('\r', "");
        let mut cells = Vec::new();
        let mut rows = Len::<Row>::ZERO;
        let mut cols = Len::<Col>::ZERO;
        for (dr, line) in normalized.trim_end_matches('\n').split('\n').enumerate() {
            let dr = Len::<Row>::of(dr);
            rows = rows.max(dr + Len::<Row>::ONE);
            for (dc, field) in line.split('\t').enumerate() {
                let dc = Len::<Col>::of(dc);
                cols = cols.max(dc + Len::<Col>::ONE);
                if field.is_empty() {
                    continue;
                }
                cells.push(ClipboardCell {
                    dr,
                    dc,
                    raw: field.to_owned(),
                    ..Default::default()
                });
            }
        }
        if cells.is_empty() {
            return TrameOutcome::display_only(vec![]);
        }
        // src == target -> zero shift, so any "=..." in external text stays as-is.
        let payload = ClipboardPayload {
            src_row: origin.row,
            src_col: origin.col,
            rows,
            cols,
            cells,
        };
        self.paste_range(origin, payload)
    }

    /// Build the [`ClipboardPayload`] for the tracked selection from the document
    /// (`trame-clipboard`), replied via [`TrameEvent::Clipboard`].
    ///
    /// This is the authoritative, Logic-side counterpart of the old UI
    /// viewport-cache builder (`ui/grid/geometry.rs`). The whole selection is read
    /// from the CRDT, never culled to a viewport, so a copy after `Ctrl+A` or a
    /// scroll-extend is complete. Blank cells are omitted, and they leave their
    /// paste destination untouched.
    ///
    /// The per-cell [`CellStyle`] is taken verbatim from the tracked style
    /// register. Number format and decimals, which the viewport did not surface,
    /// therefore survive the round-trip. Non-mutating: emits a display-only
    /// outcome with the single clipboard event.
    pub(super) fn copy_range(&self) -> TrameOutcome {
        // The whole selection: the bounding box is the payload shape, and every
        // deduplicated cell fills it (`TRAME-UI-129`). Gaps in a disjoint
        // selection stay blank, which paste leaves untouched.
        let bounds = self.selection_bounds();
        let first = bounds.top_left;
        let mut cells = Vec::new();
        for pos in self.selected_cells() {
            let raw = self.cell_source(pos);
            if raw.is_empty() {
                continue;
            }
            let raw = raw.into_string();
            cells.push(ClipboardCell {
                dr: pos.row.len_from(first.row),
                dc: pos.col.len_from(first.col),
                raw,
                bold: Weight::from_bool(self.bold.contains(&pos)),
                italic: Slant::from_bool(self.italic.contains(&pos)),
                style: self.styles.get(&pos).cloned().unwrap_or_default(),
            });
        }
        let payload = ClipboardPayload {
            src_row: first.row,
            src_col: first.col,
            rows: bounds.row_count(),
            cols: bounds.col_count(),
            cells,
        };
        TrameOutcome::display_only(vec![TrameEvent::Clipboard(payload)])
    }

    /// Write a pasted cell's formatting into the single `cellstyle_{sheet}`
    /// register, mirroring the per-cell tail of
    /// [`set_cell_style`](Self::set_cell_style). Bold/italic go into their own
    /// in-memory sets, the rest into the cell's `CellStyle`, and
    /// [`write_cell_style`](Self::write_cell_style) folds the whole thing into one
    /// record.
    ///
    /// Like [`write_cell`](Self::write_cell) it does not commit. The whole paste is
    /// committed once by [`finish_cell_writes`](Self::finish_cell_writes), so a
    /// styled paste is a single undo item. Borders are taken verbatim from the
    /// source cell: no selection-level preset resolution, since the source already
    /// carries resolved per-edge borders.
    pub(super) fn apply_pasted_style(&mut self, pos: CellPos, cell: &ClipboardCell) {
        if cell.bold == Weight::Bold {
            self.bold.insert(pos);
        } else {
            self.bold.remove(&pos);
        }
        if cell.italic == Slant::Italic {
            self.italic.insert(pos);
        } else {
            self.italic.remove(&pos);
        }
        // A default style is not tracked (absence == default) so the map never
        // accumulates dead keys.
        if cell.style == CellStyle::default() {
            self.styles.remove(&pos);
        } else {
            self.styles.insert(pos, cell.style.clone());
        }
        let _ = self.write_cell_style(pos);
    }
}
