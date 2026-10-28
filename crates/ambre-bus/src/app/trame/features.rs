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
use super::{CellRect, ColCount, DateSerial, FormulaSource, SheetId};
use ambre_core::Rgb;

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
    pub col: ColCount,
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

/// A number refused as a percentage: outside `0..=100`, or not a number.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{0} is not a percentage between 0 and 100")]
pub struct InvalidPercent(pub f32);

/// A share of a whole, in percent, from 0 to 100 inclusive, kept as given
/// with no rounding (`TRAME-239`, `TRAME-240`). A top-N rule ranks the cells it covers by
/// it (`trame-cell-styles`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Percent(f32);

impl Percent {
    /// The percentage `value` is.
    ///
    /// # Errors
    ///
    /// [`InvalidPercent`] when `value` is below 0, above 100, or NaN.
    pub fn new(value: f32) -> Result<Self, InvalidPercent> {
        if (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidPercent(value))
        }
    }

    /// The percentage, from 0 to 100.
    pub const fn value(self) -> f32 {
        self.0
    }
}

/// A conditional-formatting rule predicate (`trame-cell-styles`).
/// `String` operands are either a literal value or a cell reference, resolved at
/// evaluation. MVP+ kinds (value/text/formula/presets) land in Phase 2; the
/// gradient / data-bar / top-bottom kinds are Phase 3.
#[derive(Debug, Clone)]
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
    Formula(FormulaSource),
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
    TopN(RankCount),
    BottomN(RankCount),
    TopNPercent(Percent),
}

ambre_core::define_tally! {
    /// How many of the top or bottom values a rule marks.
    pub struct RankCount;
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

/// A pair of number limits refused: one is not finite, or `min` exceeds `max`.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("{min} to {max} is not a range of finite numbers, lowest first")]
pub struct InvalidNumberBounds {
    pub min: f64,
    pub max: f64,
}

/// The two limits of a number range: both finite, `min` at most `max`
/// (`trame-cell-editing`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberBounds {
    min: f64,
    max: f64,
}

impl NumberBounds {
    /// The range from `min` to `max`.
    ///
    /// # Errors
    ///
    /// [`InvalidNumberBounds`] when either limit is NaN or infinite, or
    /// when `min` exceeds `max`.
    pub fn new(min: f64, max: f64) -> Result<Self, InvalidNumberBounds> {
        if min.is_finite() && max.is_finite() && min <= max {
            Ok(Self { min, max })
        } else {
            Err(InvalidNumberBounds { min, max })
        }
    }

    /// The lower limit.
    pub const fn min(self) -> f64 {
        self.min
    }

    /// The upper limit.
    pub const fn max(self) -> f64 {
        self.max
    }
}

/// A data-validation predicate (`trame-cell-editing`). `List` /
/// `NumberRange` / `TextLength` are Phase 2; `ListRange` / `DateRange` / `Regex`
/// are Phase 3.
#[derive(Debug, Clone)]
pub enum ValidationKind {
    /// Dropdown over a fixed value set.
    List(Vec<String>),
    /// Dropdown over the values of a range on a sheet. The view resolves the
    /// A1 reference once, at input (the engine-level `RangeDep` is not
    /// visible from the bus).
    ListRange {
        sheet: SheetId,
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
        min: Option<ambre_core::CharCount>,
        max: Option<ambre_core::CharCount>,
    },
    Regex(String),
    /// Custom formula that must return `TRUE`, as the user typed it. The Logic
    /// thread parses it.
    Formula(FormulaSource),
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
    use super::{NumberBounds, Percent};

    /// A number range keeps its two finite limits, lowest first, and
    /// refuses a reversed pair or a limit that is not finite.
    #[test]
    fn a_number_range_runs_between_two_finite_limits_lowest_first() {
        let bounds = NumberBounds::new(-1.5, 2.0).expect("ordered finite limits");
        assert_eq!((bounds.min(), bounds.max()), (-1.5, 2.0));
        assert!(NumberBounds::new(3.0, 3.0).is_ok(), "one value is a range");
        for (min, max) in [
            (2.0, 1.0),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (f64::NEG_INFINITY, 0.0),
        ] {
            assert!(
                NumberBounds::new(min, max).is_err(),
                "{min} to {max} is refused"
            );
        }
    }

    /// `TRAME-239`: the range is closed. `TRAME-240`: a value inside it is
    /// kept as given, with no rounding.
    #[test]
    fn a_percentage_runs_from_zero_to_a_hundred() {
        for value in [0.0, 12.5, 33.333_33, 100.0] {
            assert_eq!(Percent::new(value).map(Percent::value), Ok(value));
        }
        for value in [-0.5, 100.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(Percent::new(value).is_err(), "{value} is refused");
        }
    }
}
