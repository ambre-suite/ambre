//! Clipboard payloads (`trame-clipboard`): the serialized cells
//! of a copied/cut range, moved to the Logic thread inside
//! [`TrameCommand::PasteRange`](super::TrameCommand).

use super::CellStyle;
use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Clipboard (`trame-clipboard`)
//
// The serialized payload of a copied/cut range. Built by the UI from its
// viewport cache, and written onto the OS clipboard (postcard, alongside a
// tab-separated text fallback for cross-app interop). It is read back on paste,
// and moved to the Logic thread inside [`TrameCommand::PasteRange`]. It derives
// `serde` (unlike the rest of the data plane) precisely so the UI can postcard
// it onto the clipboard.
// -----------------------------------------------------------------------------

/// A copied/cut rectangular range: its source origin (top-left absolute grid
/// position, for relative-formula reindexing). It also carries its non-empty
/// cells, each keyed by an offset from that origin.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPayload {
    /// Source range top-left absolute row (0-based). The paste reindex shift is
    /// `target_row - src_row`.
    pub src_row: u32,
    /// Source range top-left absolute column (0-based).
    pub src_col: u32,
    /// Source range height in rows (so a cut can clear the whole rectangle and
    /// the renderer can draw marching-ants over it).
    pub rows: u32,
    /// Source range width in columns.
    pub cols: u32,
    /// The range's non-empty cells. Blank cells are omitted; the paste leaves
    /// the corresponding destination cells untouched (Excel/Sheets behaviour).
    pub cells: Vec<ClipboardCell>,
}

impl ClipboardPayload {
    /// Lay the payload out as tab-separated rows (`\t` between columns, `\n`
    /// between rows), the lingua-franca a spreadsheet writes to the OS clipboard
    /// for cross-application paste. Cells are placed by their `(dr, dc)` offset;
    /// gaps (blank source cells, omitted from `cells`) render as empty fields. A
    /// formula cell contributes its raw text (for example `=A1*2`).
    pub fn to_tsv(&self) -> String {
        let mut grid =
            vec![vec![String::new(); self.cols.max(1) as usize]; self.rows.max(1) as usize];
        for cell in &self.cells {
            if let Some(row) = grid.get_mut(cell.dr as usize)
                && let Some(slot) = row.get_mut(cell.dc as usize)
            {
                *slot = cell.raw.clone();
            }
        }
        grid.into_iter()
            .map(|r| r.join("\t"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One cell of a [`ClipboardPayload`], positioned by its offset from the source
/// origin. It carries its raw text (formula source for a formula cell) plus its
/// full formatting.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardCell {
    /// Row offset from [`ClipboardPayload::src_row`].
    pub dr: u32,
    /// Column offset from [`ClipboardPayload::src_col`].
    pub dc: u32,
    /// Raw cell text (for example `=SUM(A1:B1)` for a formula), in display form. A
    /// formula's relative references are reindexed by the Logic thread on paste.
    pub raw: String,
    pub bold: bool,
    pub italic: bool,
    /// The cell's resolved rich formatting, written verbatim at the destination.
    pub style: CellStyle,
}
