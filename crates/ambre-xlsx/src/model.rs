//! The workbook the codec reads a file into and writes a file from.
//!
//! The model holds what Trame can hold, in Trame's own vocabulary. The font,
//! alignment and colour types are those of `ambre-core`. The number format
//! and the border mirror the cell style of `ambre-trame` one to one.
//! The reader brings every construct of the file to this model or reports
//! it. So the map from the model to the document drops nothing, and the
//! report the reader returns is the whole of what the import lost.

use ambre_core::index::Pos;
use ambre_core::{
    Bounded, Bounds, Doc, FontFamily, FontSize, Px, Refusal, Rgb, Rule, Slant, Strikethrough,
    Underline, Validated, Weight, WordBreak,
};
use serde::{Deserialize, Serialize};

use crate::cellref::{CellRange, CellRef, Col, Row};
use crate::formula::{AmbreFormula, Formula};

/// A whole workbook: its sheets in tab order, and the styles their cells
/// name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    /// The distinct styles of the workbook. A cell names one by its place in
    /// this list, and the first entry is the default style.
    pub styles: Vec<CellStyle>,
}

impl Workbook {
    /// The style `id` names, or the default style when the list holds no
    /// such entry.
    #[must_use]
    pub fn style(&self, id: StyleId) -> CellStyle {
        id.slot()
            .to_index()
            .and_then(|i| self.styles.get(i))
            .cloned()
            .unwrap_or_default()
    }
}

/// The place of a style in [`Workbook::styles`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct StyleId(Pos<StyleSlot>);

/// The axis of the style list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleSlot {}

impl ambre_core::index::Axis for StyleSlot {
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("StyleSlot");
}

impl StyleId {
    /// The default style, the first of the list.
    pub const DEFAULT: Self = Self(Pos::new(0));

    /// The style at `slot` of the list.
    #[must_use]
    pub const fn at(slot: Pos<StyleSlot>) -> Self {
        Self(slot)
    }

    /// The place of the style in the list.
    #[must_use]
    pub const fn slot(self) -> Pos<StyleSlot> {
        self.0
    }
}

/// One sheet: its name, whether its tab shows, its cells and its layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub name: Validated<SheetName>,
    pub visibility: Visibility,
    /// The cells that hold a value, a formula or a style, in row-major order.
    pub cells: Vec<Cell>,
    /// The merged ranges. Two of them never overlap.
    pub merges: Vec<CellRange>,
    /// The columns whose width differs from the default, each once.
    pub column_widths: Vec<ColumnWidth>,
    /// The rows whose height differs from the default, each once.
    pub row_heights: Vec<RowHeight>,
}

impl Sheet {
    /// An empty, visible sheet named `name`.
    #[must_use]
    pub fn named(name: Validated<SheetName>) -> Self {
        Self {
            name,
            visibility: Visibility::Visible,
            cells: Vec::new(),
            merges: Vec::new(),
            column_widths: Vec::new(),
            row_heights: Vec::new(),
        }
    }
}

/// The rule of a sheet name, which Excel holds on every name it reads. A name
/// holds 1 to 31 characters and none of `\ / ? * [ ] :`. No apostrophe
/// opens or closes it.
pub enum SheetName {}

impl Rule for SheetName {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        let chars = text.chars().count();
        if chars == 0 || chars > 31 {
            return Err("a sheet name holds 1 to 31 characters");
        }
        if text.contains(['\\', '/', '?', '*', '[', ']', ':']) {
            return Err("a sheet name holds none of \\ / ? * [ ] :");
        }
        if text.starts_with('\'') || text.ends_with('\'') {
            return Err("a sheet name neither starts nor ends with an apostrophe");
        }
        Ok(text)
    }
}

/// Whether a sheet's tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Visibility {
    #[default]
    Visible,
    /// The tab is hidden, and the user can show it again.
    Hidden,
}

/// One cell: where it is, what it holds, and its style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub at: CellRef,
    pub content: Content,
    pub style: StyleId,
}

/// What a cell holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Content {
    /// A value the user typed.
    Value(Value),
    /// A formula, and the value its last computation left in the file.
    Formula { formula: Formula, cached: Value },
    /// A formula in Trame's dialect alone, and the value Trame computed for
    /// it. The writer derives its Excel text. The reader never yields one:
    /// it lets the Main instance hand a formula over without linking the
    /// translator, which the converter module holds.
    TrameFormula {
        formula: Validated<AmbreFormula>,
        cached: Value,
    },
}

/// A value of a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum Value {
    /// No value: a cell that only carries a style.
    #[default]
    Empty,
    /// A number. A date is its serial number in the 1900 date system.
    Number(f64),
    /// A text, as the user reads it.
    Text(String),
    Boolean(bool),
    Error(ErrorValue),
}

/// An error value of the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorValue {
    Null,
    DivisionByZero,
    Value,
    Ref,
    Name,
    Num,
    NotAvailable,
    GettingData,
    Spill,
    Calc,
}

impl ErrorValue {
    /// Every error value, in the order of their codes.
    pub const ALL: [Self; 10] = [
        Self::Null,
        Self::DivisionByZero,
        Self::Value,
        Self::Ref,
        Self::Name,
        Self::Num,
        Self::NotAvailable,
        Self::GettingData,
        Self::Spill,
        Self::Calc,
    ];

    /// The text a file and a formula spell the error with.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Null => "#NULL!",
            Self::DivisionByZero => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Num => "#NUM!",
            Self::NotAvailable => "#N/A",
            Self::GettingData => "#GETTING_DATA",
            Self::Spill => "#SPILL!",
            Self::Calc => "#CALC!",
        }
    }

    /// The error `code` spells, case ignored, as Excel ignores it.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|e| e.code().eq_ignore_ascii_case(code))
    }
}

/// The resolved style of one cell, in the terms of a Trame cell style.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CellStyle {
    pub weight: Weight,
    pub slant: Slant,
    pub underline: Underline,
    pub strikethrough: Strikethrough,
    pub h_align: HAlign,
    pub v_align: VAlign,
    /// `Anywhere` wraps the text inside the cell; `Whole` lets it overflow.
    pub wrap: WordBreak,
    /// The text colour. `None` is the theme's text colour.
    pub fg: Option<Rgb>,
    /// The fill colour. `None` is no fill.
    pub bg: Option<Rgb>,
    /// The font family. `None` is the workbook's default font.
    pub font: Option<FontFamily>,
    /// The font size. `None` is the workbook's default size.
    pub size: Option<FontSize>,
    pub format: NumberFormat,
    pub borders: Borders,
}

/// The horizontal alignment of a cell's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HAlign {
    /// Text to the left, numbers to the right.
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

/// The vertical alignment of a cell's text. The default is Trame's, the
/// middle: an Excel cell that names none sits at the bottom, and the reader
/// writes that down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// The number format of a cell: a family, or a pattern of the format.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NumberFormat {
    /// The value as it is.
    #[default]
    General,
    /// Fixed decimals, no grouping.
    Number(Bounded<Decimals>),
    /// A currency amount, with grouping.
    Currency(Bounded<Decimals>),
    Percent(Bounded<Decimals>),
    Scientific(Bounded<Decimals>),
    Fraction,
    /// The serial as a calendar date.
    Date,
    /// The serial as a time of day.
    Time,
    /// The value as text, never a number.
    Text,
    /// A pattern no family names, kept as the file wrote it.
    Custom(Validated<FormatCode>),
}

/// The decimal places of a number format: 0 to 15, Trame's cap.
pub enum Decimals {}

impl Bounds for Decimals {
    type Repr = u8;
    const MIN: u8 = 0;
    const MAX: u8 = 15;
    const NAME: ambre_core::StaticKey<ambre_core::DebugNames> =
        ambre_core::StaticKey::new("XlsxDecimals");
    const REFUSAL: Refusal = Refusal::new("a number format carries at most 15 decimals, not ", "");
}

/// The rule of a number format pattern: 1 to 255 characters, the cap of
/// the format.
pub enum FormatCode {}

impl Rule for FormatCode {
    type Error = &'static str;

    fn admit(text: String) -> Result<String, Self::Error> {
        let chars = text.chars().count();
        if chars == 0 || chars > 255 {
            return Err("a number format holds 1 to 255 characters");
        }
        Ok(text)
    }
}

/// The four edges of a cell's border. `None` on an edge is no border there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Borders {
    pub top: Option<CellBorder>,
    pub right: Option<CellBorder>,
    pub bottom: Option<CellBorder>,
    pub left: Option<CellBorder>,
}

/// One edge of a border: its stroke and its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CellBorder {
    pub style: BorderStyle,
    /// `None` is the default line colour.
    pub color: Option<Rgb>,
}

/// The stroke of a border edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BorderStyle {
    /// A thin solid line.
    #[default]
    Solid,
    Dashed,
    Dotted,
    /// A medium solid line.
    Medium,
    /// A thick solid line.
    Thick,
    Double,
}

/// The width of a run of columns.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColumnWidth {
    pub first: Pos<Col>,
    pub last: Pos<Col>,
    /// The width on screen, at 96 dots per inch.
    pub width: Px<Doc>,
}

/// The height of one row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RowHeight {
    pub row: Pos<Row>,
    /// The height on screen, at 96 dots per inch.
    pub height: Px<Doc>,
}
