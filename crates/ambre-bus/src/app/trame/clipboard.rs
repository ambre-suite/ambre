//! Clipboard payloads (`trame-clipboard`): the serialized cells
//! of a copied/cut range, moved to the Logic thread inside
//! [`TrameCommand::PasteRange`](super::TrameCommand).

use super::{
    CellStyle, Col, ColCount, ColSpan, ColVec, Row, RowCount, RowSpan, RowVec, Slant, Weight,
};
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
    pub src_row: Row,
    /// Source range top-left absolute column (0-based).
    pub src_col: Col,
    /// Source range height in rows (so a cut can clear the whole rectangle and
    /// the renderer can draw marching-ants over it).
    pub rows: RowCount,
    /// Source range width in columns.
    pub cols: ColCount,
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
        let width = ColCount::new(self.cols.get().max(1));
        let height = RowCount::new(self.rows.get().max(1));
        let mut grid: RowVec<ColVec<String>> = RowSpan::from_count(Row::ZERO, height)
            .into_iter()
            .map(|_| {
                ColSpan::from_count(Col::ZERO, width)
                    .into_iter()
                    .map(|_| String::new())
                    .collect()
            })
            .collect();
        for cell in &self.cells {
            if let Some(slot) = grid
                .get_mut(cell.dr.as_end())
                .and_then(|row| row.get_mut(cell.dc.as_end()))
            {
                *slot = cell.raw.clone();
            }
        }
        grid.iter()
            .map(|r| r.as_slice().join("\t"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One cell of a [`ClipboardPayload`], positioned by its offset from the source
/// origin. It carries its raw text (formula source for a formula cell) plus its
/// full formatting.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardCell {
    /// Row offset from [`ClipboardPayload::src_row`].
    pub dr: RowCount,
    /// Column offset from [`ClipboardPayload::src_col`].
    pub dc: ColCount,
    /// Raw cell text (for example `=SUM(A1:B1)` for a formula), in display form. A
    /// formula's relative references are reindexed by the Logic thread on paste.
    pub raw: String,
    pub bold: Weight,
    pub italic: Slant,
    /// The cell's resolved rich formatting, written verbatim at the destination.
    pub style: CellStyle,
}

// The raw text is the author's: a log carries its length alone.
impl std::fmt::Debug for ClipboardCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardCell")
            .field("dr", &self.dr)
            .field("dc", &self.dc)
            .field("raw", &ambre_core::RedactedChars::of(&self.raw))
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("style", &self.style)
            .finish()
    }
}
