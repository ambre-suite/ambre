//! Shared filter definitions and the manual cell-formatting vocabulary
//! (`trame-shared-filter`, `trame-cell-styles`): filter kinds, alignment, number
//! formats, borders, and the resolved [`CellStyle`] / sparse [`StylePatch`].

use ambre_core::{FontFamily, FontSize, Rgb};
pub use ambre_core::{Slant, Strikethrough, Underline, Weight, WordBreak};
use serde::{Deserialize, Serialize};

// -----------------------------------------------------------------------------
// Shared filter -- definition (UI -> Logic)
// -----------------------------------------------------------------------------

/// How a shared filter decides whether a row's value in the filtered column is
/// kept (visible) or hidden (`trame-shared-filter`). MVP covers
/// the "by values" and "by condition" modes; colour filters are Phase 2.
///
/// `Serialize` / `Deserialize` back the one whole register the Logic thread
/// persists (`TRAME-FEAT-025`). Column and kind encode together, so two
/// concurrent filters race once, never field by field.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum FilterKind {
    /// Keep only rows whose filtered-column display text is one of these values
    /// (Google Sheets "filter by values": the listed values are the *checked*,
    /// visible ones).
    Values(Vec<String>),
    /// Keep only rows whose filtered-column value satisfies the condition.
    Condition {
        op: FilterOp,
        /// Comparison operand (parsed as a number when both sides are numeric,
        /// compared as text otherwise). Ignored by the unary operators
        /// ([`IsEmpty`](FilterOp::IsEmpty) / [`IsNotEmpty`](FilterOp::IsNotEmpty)).
        value: String,
    },
}

// A kept value and an operand are cell text, and print their length alone.
impl std::fmt::Debug for FilterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::RedactedChars;
        match self {
            Self::Values(values) => f
                .debug_tuple("Values")
                .field(
                    &values
                        .iter()
                        .map(|text| RedactedChars::of(text))
                        .collect::<Vec<_>>(),
                )
                .finish(),
            Self::Condition { op, value } => f
                .debug_struct("Condition")
                .field("op", op)
                .field("value", &RedactedChars::of(value))
                .finish(),
        }
    }
}

/// Condition operators for a [`FilterKind::Condition`]
/// (`trame-shared-filter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    NotContains,
    StartsWith,
    IsEmpty,
    IsNotEmpty,
}

// ----------------------------------------------------------------------------- Manual cell
// formatting (`trame-cell-styles`)
//
// The toolbar's typed style vocabulary. `CellStyle` is the resolved formatting of a single
// cell, the one the toolbar reflects for the active cell. `StylePatch` is the sparse delta
// a control emits over the selection (`SetCellStyle`). Both travel by move inside
// `AppCommand::Trame` / `AppEvent::Trame` -- no `serde`, like the rest of the data plane.
// Colours are packed `0xRRGGBB`.
// -----------------------------------------------------------------------------

/// Horizontal text alignment of a cell (`trame-cell-styles`). [`Auto`](Self::Auto)
/// is the spreadsheet default: numbers align right, everything else left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HAlign {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

/// Vertical text alignment of a cell (`trame-cell-styles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// A cell number-format family (`trame-number-formats`). The decimal places
/// rides alongside in [`CellStyle::decimals`]; [`Custom`](Self::Custom) carries
/// its own pattern string (applied verbatim by the Logic thread).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CellFormat {
    /// No reformatting: the raw value / formula result is shown as-is.
    #[default]
    General,
    Number,
    Currency,
    Percent,
    Scientific,
    Fraction,
    /// Date (ISO `YYYY-MM-DD`) -- the numeric value is a day serial (MVP stub
    /// keeps the raw text when it is not numeric).
    Date,
    /// Time (`HH:MM:SS`).
    Time,
    /// Force textual interpretation (no numeric coercion).
    Text,
    Custom(String),
}

// A custom pattern is the author's, and prints its length alone.
impl std::fmt::Debug for CellFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::General => "General",
            Self::Number => "Number",
            Self::Currency => "Currency",
            Self::Percent => "Percent",
            Self::Scientific => "Scientific",
            Self::Fraction => "Fraction",
            Self::Date => "Date",
            Self::Time => "Time",
            Self::Text => "Text",
            Self::Custom(pattern) => {
                return f
                    .debug_tuple("Custom")
                    .field(&ambre_core::RedactedChars::of(pattern))
                    .finish();
            }
        })
    }
}

/// A count of decimal places a number format renders, from zero to
/// [`Decimals::MAX`]. An `f64` holds no more significant digits than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Decimals(u8);

impl Decimals {
    /// No decimal places: a whole number.
    pub const ZERO: Self = Self(0);
    /// Two decimal places, the default of the numeric families.
    pub const TWO: Self = Self(2);
    /// The most decimal places a cell may carry.
    pub const MAX: Self = Self(15);

    /// The count `places`, or `None` past [`Self::MAX`].
    pub const fn new(places: u8) -> Option<Self> {
        if places <= Self::MAX.0 {
            Some(Self(places))
        } else {
            None
        }
    }

    /// The number of places.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The number of places as a format precision.
    pub const fn precision(self) -> usize {
        self.0 as usize
    }

    /// The count moved by `step`, held between zero and [`Self::MAX`].
    #[must_use]
    pub fn stepped(self, step: DecimalsStep) -> Self {
        Self(self.0.saturating_add_signed(step.0).min(Self::MAX.0))
    }
}

impl std::fmt::Display for Decimals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The error of a decoded decimal count past [`Decimals::MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a cell carries at most 15 decimal places, not {0}")]
pub struct InvalidDecimals(u8);

impl InvalidDecimals {
    /// Wraps a raw value. The constructor checks nothing.
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw value.
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Decimals {
    type Error = InvalidDecimals;

    fn try_from(places: u8) -> Result<Self, Self::Error> {
        Self::new(places).ok_or(InvalidDecimals(places))
    }
}

impl From<Decimals> for u8 {
    fn from(decimals: Decimals) -> Self {
        decimals.0
    }
}

/// A signed number of decimal places to add to a count: what one press of
/// the Increase or Decrease decimals control asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecimalsStep(i8);

impl DecimalsStep {
    /// One place more.
    pub const MORE: Self = Self(1);
    /// One place fewer.
    pub const FEWER: Self = Self(-1);

    /// A step of `places`, negative for fewer.
    pub const fn new(places: i8) -> Self {
        Self(places)
    }
}

impl CellFormat {
    /// Whether this format honours a decimal-places count (that is, the
    /// Increase/Decrease-decimals controls are meaningful). Only the numeric
    /// families render with [`CellStyle::decimals`]; `General`, `Fraction`,
    /// `Date`, `Time`, `Text` and `Custom` ignore it (see `format_number`).
    pub fn has_decimals(&self) -> bool {
        self.default_decimals().is_some()
    }

    /// The decimal count the family renders when a cell carries none, or `None`
    /// for a family that renders no decimals.
    pub fn default_decimals(&self) -> Option<Decimals> {
        match self {
            CellFormat::Number | CellFormat::Currency | CellFormat::Scientific => {
                Some(Decimals::TWO)
            }
            CellFormat::Percent => Some(Decimals::ZERO),
            CellFormat::General
            | CellFormat::Fraction
            | CellFormat::Date
            | CellFormat::Time
            | CellFormat::Text
            | CellFormat::Custom(_) => None,
        }
    }
}

/// A border preset naming a set of edges within the selection
/// (`trame-cell-styles`). Resolved per cell (merge-aware) by the Logic thread
/// into the concrete edges of [`Borders`] the chosen stroke is applied to. Every preset
/// except [`BorderPreset::None`] is additive: it sets only its own edges and
/// leaves the others untouched; `None` clears all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderPreset {
    #[default]
    None,
    /// Border around the outside of the selection rectangle only.
    Outline,
    /// Border on every cell edge in the selection.
    All,
    /// All interior edges of the selection (between adjacent selected cells).
    Inner,
    /// Interior horizontal edges only (between stacked rows).
    InnerHorizontal,
    /// Interior vertical edges only (between side-by-side columns).
    InnerVertical,
    Top,
    Bottom,
    Left,
    Right,
}

/// The stroke style of a cell border, conflating line pattern and weight into a
/// single choice (`trame-cell-styles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BorderStyle {
    /// A solid line border.
    #[default]
    Solid,
    /// A dashed line border.
    Dashed,
    /// A dotted line border.
    Dotted,
    /// A thicker line border.
    Medium,
    /// A very thick line border.
    Thick,
    /// A double line border.
    Double,
}

/// A single cell edge's border: its stroke style and colour. The edge's
/// presence is carried by the enclosing `Option<CellBorder>` in [`Borders`]
/// (`None` = no border on that edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CellBorder {
    pub style: BorderStyle,
    /// Border colour, packed `0xRRGGBB`. `None` = the default grid-line colour.
    pub color: Option<Rgb>,
}

/// The four independent edges of a cell border (`trame-cell-styles`). Each edge
/// carries its own stroke style and colour, so a cell can mix, for example, a thick
/// red top with a dotted blue left. `None` on an edge means no border there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Borders {
    pub top: Option<CellBorder>,
    pub right: Option<CellBorder>,
    pub bottom: Option<CellBorder>,
    pub left: Option<CellBorder>,
}

/// The resolved manual formatting of a single cell, the value the toolbar
/// reflects (`trame-cell-styles`). Bold/italic are tracked by their dedicated
/// toggle commands and reported separately ([`TrameEvent::StyleApplied`]); the
/// underline run lives here with the rest of the patchable attributes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CellStyle {
    pub underline: Underline,
    /// Strikethrough run drawn through the glyphs (`trame-cell-styles`).
    pub strikethrough: Strikethrough,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: WordBreak,
    /// Text colour, packed `0xRRGGBB`. `None` = theme default (adapts to the
    /// light/dark theme).
    pub fg: Option<Rgb>,
    /// Fill colour, packed `0xRRGGBB`. `None` = transparent (shows the themed
    /// sheet background).
    pub bg: Option<Rgb>,
    /// Font family. `None` = the grid default font.
    pub font: Option<FontFamily>,
    /// Font size. `None` = the grid default size. Stored as whole points, the
    /// steps the size picker offers. A stored size past
    /// [`CELL_FONT_SIZE_MAX`] reads as it (`CANVAS-088`).
    #[serde(with = "capped_whole_points")]
    pub size: Option<FontSize>,
    pub format: CellFormat,
    /// Decimal places for the active number format. `None` = the format default.
    pub decimals: Option<Decimals>,
    /// Per-edge border (style + colour each), resolved per cell by the Logic
    /// thread from the selection-level [`StylePatch::border`] edit.
    pub borders: Borders,
}

/// The smallest font size a style command may set on a cell.
pub const CELL_FONT_SIZE_MIN: FontSize = FontSize::from_whole_points(1);

/// The largest font size a cell carries: 409 points, the cap of the
/// spreadsheet formats (`CANVAS-088`). A style command past it is refused.
pub const CELL_FONT_SIZE_MAX: FontSize = FontSize::from_whole_points(409);

/// The whole-point layout of [`CellStyle::size`], read back at most
/// [`CELL_FONT_SIZE_MAX`]. The bytes stay those of `ambre_core::font::whole_points`.
mod capped_whole_points {
    use ambre_core::FontSize;
    use ambre_core::font::whole_points;
    use serde::{Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        size: &Option<FontSize>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        whole_points::serialize(size, out)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        input: D,
    ) -> Result<Option<FontSize>, D::Error> {
        Ok(whole_points::deserialize(input)?.map(|size| size.at_most(super::CELL_FONT_SIZE_MAX)))
    }
}

/// A sparse formatting delta emitted by a toolbar control over the current
/// selection ([`TrameCommand::SetCellStyle`], `trame-cell-styles`). Every `None`
/// field leaves the existing cell value intact; a `Some` overwrites it. The
/// nested `Option<Option<_>>` colour/font/size fields distinguish "leave alone"
/// (`None`) from "clear" (`Some(None)`) and "set" (`Some(Some(v))`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StylePatch {
    pub bold: Option<Weight>,
    pub italic: Option<Slant>,
    pub underline: Option<Underline>,
    pub strikethrough: Option<Strikethrough>,
    pub h_align: Option<HAlign>,
    pub v_align: Option<VAlign>,
    pub wrap: Option<WordBreak>,
    pub fg: Option<Option<Rgb>>,
    pub bg: Option<Option<Rgb>>,
    pub font: Option<Option<FontFamily>>,
    pub size: Option<Option<FontSize>>,
    pub format: Option<CellFormat>,
    pub decimals: Option<Option<Decimals>>,
    /// Add this many places to the count each cell already carries, or its
    /// family's default when it carries none (`TRAME-105`). Applied after
    /// `format` and `decimals`, per cell, so a mixed selection steps each count
    /// from its own value. A family that renders no decimals is left alone.
    pub decimals_step: Option<DecimalsStep>,
    pub border: Option<BorderEdit>,
}

/// A border edit emitted by the toolbar's borders popover. It applies the chosen stroke
/// `style` and `color` to the edges named by `preset`, or clears every edge for
/// [`BorderPreset::None`]. Resolved per cell, merge-aware, by the Logic thread
/// (`trame-cell-styles`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderEdit {
    pub preset: BorderPreset,
    pub style: BorderStyle,
    /// Border colour, packed `0xRRGGBB`. `None` = the default grid-line colour.
    pub color: Option<Rgb>,
}

#[cfg(test)]
mod tests {
    use super::{CELL_FONT_SIZE_MAX, CellStyle, FontSize};

    /// A cell size a peer stored past the cap reads as the cap, and a size
    /// in range keeps its bytes (`CANVAS-088`).
    #[test]
    fn a_cell_size_past_the_cap_reads_as_the_cap() {
        let sized = |points| CellStyle {
            size: Some(FontSize::from_whole_points(points)),
            ..CellStyle::default()
        };
        let huge = postcard::to_allocvec(&sized(4000)).expect("encodes");
        let read = postcard::from_bytes::<CellStyle>(&huge).expect("decodes");
        assert_eq!(read.size, Some(CELL_FONT_SIZE_MAX));
        assert_eq!(CELL_FONT_SIZE_MAX.points(), 409.0);
        let twelve = postcard::to_allocvec(&sized(12)).expect("encodes");
        let read = postcard::from_bytes::<CellStyle>(&twelve).expect("decodes");
        assert_eq!(postcard::to_allocvec(&read).expect("encodes"), twelve);
    }
}
