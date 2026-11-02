//! Deferred feature payloads (Phase 2+): typed value types carried by the
//! deferred `TrameCommand` variants (fill/sort keys, conditional
//! formatting and data-validation predicates). Travel by move, no `serde`.

// ----------------------------------------------------------------------------- Deferred
// feature payloads (Phase 2+)
//
// Typed value types carried by the deferred `TrameCommand` variants above. They live in
// `ambre-bus` (re-exported by `ambre-trame`) and travel by move inside `AppCommand::Trame`
// -- no `serde`. Ranges are `CellRect`s in **grid coordinates**, because the
// engine-level `RangeDep` (`ambre-trame`) is not visible from the bus. The Logic thread
// resolves these positions to stable ids when each feature's handler lands.
// -----------------------------------------------------------------------------
use super::{CellNumber, CellRect, Col, DateSerial, SourceForm};
use crate::app::trame::CellText;
use ambre_core::id::{Id, Sheet};
use ambre_core::index::Count;
use ambre_core::{Bounded, Bounds, Refusal, Rgb, Tally};

/// How [`TrameCommand::FillRange`] fills `dest \ src` (`trame-autofill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    /// The Logic thread picks `Series` when a fill line's source values form an
    /// arithmetic numeric progression, `Copy` otherwise. The fill handle default.
    #[default]
    Auto,
    /// Tile the source values verbatim across the fill region (relative formula
    /// references reindex by the tile offset).
    Copy,
    /// Extrapolate a numeric arithmetic progression from the source values; a
    /// line whose source is not numeric falls back to `Copy`.
    Series,
}

/// One key of a multi-column sort ([`TrameCommand::SortRangeMulti`],
/// `trame-sort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// Column to sort on, as an offset within the range (`0` = `col_start`).
    pub col: Count<Col>,
    pub order: SortOrder,
}

/// Which way a sort orders the rows on one key (`trame-sort`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// The smallest value first.
    #[default]
    Ascending,
    /// The largest value first.
    Descending,
}

/// Whether the first row of a range is a header (`trame-sort`). A sort leaves
/// it in place, and a CSV import reads it as column names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderRow {
    /// Every row is data.
    #[default]
    Absent,
    /// The first row names the columns.
    Present,
}

/// A share of a whole, in percent, from 0 to 100 inclusive, kept as given
/// with no rounding (`TRAME-239`, `TRAME-240`). A top-N rule ranks the cells it covers by
/// it (`trame-cell-styles`). A number below 0, above 100, or NaN is refused.
pub enum Percent {}

impl Bounds for Percent {
    type Repr = f32;
    const MIN: f32 = 0.0;
    const MAX: f32 = 100.0;
    const NAME: &'static str = "Percent";
    const REFUSAL: Refusal = Refusal::new("", " is not a percentage between 0 and 100");
}

/// A conditional-formatting rule predicate (`trame-cell-styles`).
/// `String` operands are either a literal value or a cell reference, resolved at
/// evaluation. MVP+ kinds (value/text/formula/presets) land in Phase 2; the
/// gradient / data-bar / top-bottom kinds are Phase 3.
#[derive(Clone)]
pub enum CondFormatKind {
    GreaterThan(String),
    LessThan(String),
    /// Between two operands, `low` and `high`.
    Between {
        low: String,
        high: String,
    },
    Equal(String),
    Contains(String),
    StartsWith(String),
    /// Custom formula, for example `"=A1>AVERAGE(A:A)"` (relative refs resolved from
    /// the target cell), as the user typed it. The Logic thread parses it.
    Formula(CellText<SourceForm>),
    IsEmpty,
    IsNotEmpty,
    IsError,
    ColorScale2 {
        min_color: Rgb,
        max_color: Rgb,
    },
    ColorScale3 {
        min_color: Rgb,
        mid_color: Rgb,
        max_color: Rgb,
    },
    DataBar {
        color: Rgb,
    },
    TopN(Tally<Ranks>),
    BottomN(Tally<Ranks>),
    TopNPercent(Bounded<Percent>),
}

// An operand the author typed prints its length alone.
impl std::fmt::Debug for CondFormatKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::RedactedChars;
        let operand = |name: &str, f: &mut std::fmt::Formatter<'_>, text: &str| {
            f.debug_tuple(name).field(&RedactedChars::of(text)).finish()
        };
        match self {
            Self::GreaterThan(text) => operand("GreaterThan", f, text),
            Self::LessThan(text) => operand("LessThan", f, text),
            Self::Between { low, high } => f
                .debug_struct("Between")
                .field("low", &RedactedChars::of(low))
                .field("high", &RedactedChars::of(high))
                .finish(),
            Self::Equal(text) => operand("Equal", f, text),
            Self::Contains(text) => operand("Contains", f, text),
            Self::StartsWith(text) => operand("StartsWith", f, text),
            Self::Formula(source) => f.debug_tuple("Formula").field(source).finish(),
            Self::IsEmpty => f.write_str("IsEmpty"),
            Self::IsNotEmpty => f.write_str("IsNotEmpty"),
            Self::IsError => f.write_str("IsError"),
            Self::ColorScale2 {
                min_color,
                max_color,
            } => f
                .debug_struct("ColorScale2")
                .field("min_color", min_color)
                .field("max_color", max_color)
                .finish(),
            Self::ColorScale3 {
                min_color,
                mid_color,
                max_color,
            } => f
                .debug_struct("ColorScale3")
                .field("min_color", min_color)
                .field("mid_color", mid_color)
                .field("max_color", max_color)
                .finish(),
            Self::DataBar { color } => f.debug_struct("DataBar").field("color", color).finish(),
            Self::TopN(count) => f.debug_tuple("TopN").field(count).finish(),
            Self::BottomN(count) => f.debug_tuple("BottomN").field(count).finish(),
            Self::TopNPercent(share) => f.debug_tuple("TopNPercent").field(share).finish(),
        }
    }
}

/// How many of the top or bottom values a rule marks.
pub enum Ranks {}

impl ambre_core::TallyKind for Ranks {
    const NAME: &'static str = "RankCount";
}

/// Style applied when a conditional-formatting rule matches
/// (`trame-cell-styles`). Layered on
/// top of the base style; absent fields leave the base intact. Colours are
/// packed `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondStyle {
    pub fg: Option<Rgb>,
    pub bg: Rgb,
    pub bold: super::Weight,
}

/// Whether a number range admits its two bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RangeBounds {
    /// A value equal to `min` or `max` is refused.
    #[default]
    Exclusive,
    /// A value equal to `min` or `max` is admitted.
    Inclusive,
}

/// A pair of number limits refused: `min` exceeds `max`. A limit that is not
/// finite is no [`CellNumber`], so it is refused before.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{min} to {max} is not a range lowest first")]
pub struct InvalidNumberBounds {
    pub min: CellNumber,
    pub max: CellNumber,
}

/// The two limits of a number range: two cell numbers, `min` at most `max`
/// (`trame-cell-editing`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberBounds {
    min: CellNumber,
    max: CellNumber,
}

impl NumberBounds {
    /// The range from `min` to `max`.
    ///
    /// # Errors
    ///
    /// [`InvalidNumberBounds`] when `min` exceeds `max`.
    pub fn new(min: CellNumber, max: CellNumber) -> Result<Self, InvalidNumberBounds> {
        if min <= max {
            Ok(Self { min, max })
        } else {
            Err(InvalidNumberBounds { min, max })
        }
    }

    /// The lower limit.
    pub const fn min(self) -> CellNumber {
        self.min
    }

    /// The upper limit.
    pub const fn max(self) -> CellNumber {
        self.max
    }
}

/// A data-validation predicate (`trame-cell-editing`). `List` /
/// `NumberRange` / `TextLength` are Phase 2; `ListRange` / `DateRange` / `Regex`
/// are Phase 3.
#[derive(Clone)]
pub enum ValidationKind {
    /// Dropdown over a fixed value set.
    List(Vec<String>),
    /// Dropdown over the values of a range on a sheet. The view resolves the
    /// A1 reference once, at input (the engine-level `RangeDep` is not
    /// visible from the bus).
    ListRange {
        sheet: Id<Sheet>,
        range: CellRect,
    },
    /// Numbers between two limits, each end admitted or refused by `bounds`.
    NumberRange {
        limits: NumberBounds,
        bounds: RangeBounds,
    },
    NumberInteger,
    /// Dates between two bounds, each as a serial; the view parses the
    /// ISO text once, at input.
    DateRange {
        min: Option<DateSerial>,
        max: Option<DateSerial>,
    },
    TextLength {
        min: Option<Count<ambre_core::Char>>,
        max: Option<Count<ambre_core::Char>>,
    },
    Regex(String),
    /// Custom formula that must return `TRUE`, as the user typed it. The Logic
    /// thread parses it.
    Formula(CellText<SourceForm>),
}

// A list entry and a pattern the author typed print their length alone.
impl std::fmt::Debug for ValidationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ambre_core::RedactedChars;
        match self {
            Self::List(values) => f
                .debug_tuple("List")
                .field(
                    &values
                        .iter()
                        .map(|text| RedactedChars::of(text))
                        .collect::<Vec<_>>(),
                )
                .finish(),
            Self::ListRange { sheet, range } => f
                .debug_struct("ListRange")
                .field("sheet", sheet)
                .field("range", range)
                .finish(),
            Self::NumberRange { limits, bounds } => f
                .debug_struct("NumberRange")
                .field("limits", limits)
                .field("bounds", bounds)
                .finish(),
            Self::NumberInteger => f.write_str("NumberInteger"),
            Self::DateRange { min, max } => f
                .debug_struct("DateRange")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::TextLength { min, max } => f
                .debug_struct("TextLength")
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Regex(pattern) => f
                .debug_tuple("Regex")
                .field(&RedactedChars::of(pattern))
                .finish(),
            Self::Formula(source) => f.debug_tuple("Formula").field(source).finish(),
        }
    }
}

/// What happens when a [`ValidationKind`] rejects an input (`trame-cell-editing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationOnInvalid {
    /// Block the local mutation -> [`TrameEvent::ValidationError`](super::TrameEvent::ValidationError).
    Reject,
    /// Allow the mutation but warn -> [`TrameEvent::ValidationWarning`](super::TrameEvent::ValidationWarning).
    Warn,
    /// Visual indicator only.
    Info,
}

#[cfg(test)]
mod tests {
    use super::{CellNumber, InvalidNumberBounds, NumberBounds, Percent};
    use ambre_core::Bounded;

    /// The range from `min` to `max`, each limit refused first when it is
    /// no cell number.
    fn range(min: f64, max: f64) -> Option<Result<NumberBounds, InvalidNumberBounds>> {
        let min = CellNumber::new(min).ok()?;
        let max = CellNumber::new(max).ok()?;
        Some(NumberBounds::new(min, max))
    }

    /// A number range keeps its two finite limits, lowest first. A reversed
    /// pair is refused by the range, a limit that is not finite by the cell
    /// number it must be first.
    #[test]
    fn a_number_range_runs_between_two_finite_limits_lowest_first() {
        let bounds = range(-1.5, 2.0)
            .expect("finite limits")
            .expect("ordered limits");
        assert_eq!((bounds.min().get(), bounds.max().get()), (-1.5, 2.0));
        assert!(
            matches!(range(3.0, 3.0), Some(Ok(_))),
            "one value is a range"
        );
        let reversed = range(2.0, 1.0).expect("finite limits");
        assert_eq!(
            reversed.map_err(|e| (e.min.get(), e.max.get())),
            Err((2.0, 1.0))
        );
        for (min, max) in [
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (f64::NEG_INFINITY, 0.0),
        ] {
            assert!(range(min, max).is_none(), "{min} to {max} is refused");
        }
    }

    /// `TRAME-239`: the range is closed. `TRAME-240`: a value inside it is
    /// kept as given, with no rounding.
    #[test]
    fn a_percentage_runs_from_zero_to_a_hundred() {
        for value in [0.0, 12.5, 33.333_33, 100.0] {
            assert_eq!(
                Bounded::<Percent>::try_new(value).map(Bounded::get),
                Ok(value)
            );
        }
        for value in [-0.5, 100.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                Bounded::<Percent>::try_new(value).is_err(),
                "{value} is refused"
            );
        }
    }
}
