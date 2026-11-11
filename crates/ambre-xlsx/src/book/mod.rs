//! The workbook in the form Trame stores it: what the Office module answers
//! an import with, and what Trame hands it for an export.
//!
//! The Main instance decodes this form and nothing of the model above it.
//! The map between the model and Trame's terms runs in the converter module.
//! It maps the cell style, the text of a value, the name of a sheet and the
//! width of a column. Main links the serde code of this form alone
//! (`2-Trame/features/ambre-spec-trame-import-export.md section 3`).

use ambre_core::{
    Bounded, FontFamily, FontSize, Payload, PayloadKind, Rgb, Slant, Strikethrough, Underline,
    Validated, Weight, WordBreak,
};
use serde::{Deserialize, Serialize};

use crate::cellref::{CellRange, CellRef};
use crate::formula::{AmbreFormula, ExcelFormula};
use crate::model::{
    Borders, ColumnWidth, Decimals, HAlign, RowHeight, StyleId, VAlign, Value, Visibility,
};

/// A workbook in Trame's terms: its sheets in tab order, and the styles
/// their cells name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Book {
    pub sheets: Vec<BookSheet>,
    /// The distinct styles, each the bytes of a [`TrameStyle`]: the bytes
    /// Trame stores in a cell style register. A cell names one by its place,
    /// and the first entry is the default style.
    pub styles: Vec<Payload<StyleBytes>>,
}

/// The role of a style as Trame stores it: the postcard bytes of a
/// [`TrameStyle`]. `Payload<StyleBytes>`.
pub enum StyleBytes {}

impl PayloadKind for StyleBytes {
    const NAME: &'static str = "StyleBytes";
}

/// One sheet: its name as Trame shows it, whether its tab shows, and what it
/// holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookSheet {
    /// The name as Trame shows it. The export brings it to Excel's rule.
    pub name: String,
    pub visibility: Visibility,
    pub cells: Vec<BookCell>,
    pub merges: Vec<CellRange>,
    /// One entry per column, its first and last the same column.
    pub column_widths: Vec<ColumnWidth>,
    pub row_heights: Vec<RowHeight>,
}

/// One cell: where it is, what it holds, and the place of its style in
/// [`Book::styles`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookCell {
    pub at: CellRef,
    pub content: BookContent,
    pub style: StyleId,
}

/// What a cell holds, in Trame's terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BookContent {
    /// The raw text Trame stores for a value: a number written in full, a
    /// text, `TRUE` or `FALSE`. Empty for a cell that carries a style alone.
    Literal(String),
    /// A formula. An import carries both texts the codec has, the Trame one
    /// when the translation reached it. An export carries one: the Trame
    /// text of a formula Trame runs, or the Excel text Trame kept verbatim.
    Formula {
        trame: Option<Validated<AmbreFormula>>,
        excel: Option<Validated<ExcelFormula>>,
        cached: Value,
    },
}

/// The layout twin of Trame's cell style record: field for field, variant
/// for variant, so its postcard bytes are the bytes Trame stores. A test of
/// `ambre-trame` holds the two layouts to one.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrameStyle {
    pub bold: Weight,
    pub italic: Slant,
    pub style: TrameCellStyle,
}

/// The layout twin of Trame's `CellStyle`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrameCellStyle {
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: WordBreak,
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub font: Option<FontFamily>,
    #[serde(with = "ambre_core::font::whole_points")]
    pub size: Option<FontSize>,
    pub format: TrameFormat,
    pub decimals: Option<Bounded<Decimals>>,
    pub borders: Borders,
}

/// The layout twin of Trame's `CellFormat`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TrameFormat {
    #[default]
    General,
    Number,
    Currency,
    Percent,
    Scientific,
    Fraction,
    Date,
    Time,
    Text,
    Custom(String),
}

#[cfg(feature = "codec")]
mod map;
#[cfg(feature = "codec")]
pub use map::{book_of, workbook_of};
