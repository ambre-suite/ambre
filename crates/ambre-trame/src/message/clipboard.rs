//! Clipboard payloads (`trame-clipboard`): the serialized cells
//! of a copied/cut range, moved to the Logic thread inside
//! [`TrameCommand::PasteRange`](super::TrameCommand).

use super::{CellStyle, Col, Row, Slant, Weight};
use ambre_core::index::{IdxVec, Len, Pos, Span};
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
    pub src_row: Pos<Row>,
    /// Source range top-left absolute column (0-based).
    pub src_col: Pos<Col>,
    /// Source range height in rows (so a cut can clear the whole rectangle and
    /// the renderer can draw marching-ants over it).
    pub rows: Len<Row>,
    /// Source range width in columns.
    pub cols: Len<Col>,
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
        self.grid()
            .iter()
            .map(|r| r.as_slice().join("\t"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Lay the payload out as one HTML table, the same grid as [`Self::to_tsv`]
    /// with each cell escaped. A spreadsheet or a word processor reads a table
    /// as cells. Where the browser refuses the `web ` types, the host carries
    /// the ownership token in this HTML. A same-tab paste then keeps rung 2 on
    /// every browser (`TRAME-UI-186`).
    pub fn to_html(&self) -> String {
        let mut html = String::from("<table>");
        for row in self.grid().iter() {
            html.push_str("<tr>");
            for raw in row.iter() {
                html.push_str("<td>");
                escape_html(raw, &mut html);
                html.push_str("</td>");
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        html
    }

    /// The payload as a dense grid of raw texts. A zero extent clamps to one
    /// cell, and a cell whose offset falls outside the extent is dropped.
    fn grid(&self) -> IdxVec<Row, IdxVec<Col, String>> {
        let width = Len::<Col>::new(self.cols.get().max(1));
        let height = Len::<Row>::new(self.rows.get().max(1));
        let mut grid: IdxVec<Row, IdxVec<Col, String>> =
            Span::<Row>::from_len(Pos::<Row>::ZERO, height)
                .into_iter()
                .map(|_| {
                    Span::<Col>::from_len(Pos::<Col>::ZERO, width)
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
                slot.clone_from(&cell.raw);
            }
        }
        grid
    }
}

/// Append `text` to `out`, each of the five HTML special characters escaped.
/// A cell holding markup then lands as text in the editor that reads the table.
fn escape_html(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

/// One cell of a [`ClipboardPayload`], positioned by its offset from the source
/// origin. It carries its raw text (formula source for a formula cell) plus its
/// full formatting.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipboardCell {
    /// Row offset from [`ClipboardPayload::src_row`].
    pub dr: Len<Row>,
    /// Column offset from [`ClipboardPayload::src_col`].
    pub dc: Len<Col>,
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
