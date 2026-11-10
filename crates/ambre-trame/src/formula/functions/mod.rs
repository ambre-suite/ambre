//! Built-in spreadsheet functions.
//!
//! Each function receives a slice of already-evaluated [`CellValue`] arguments
//! and returns a single result. Functions that expect numeric arguments flatten
//! `Range` values transparently (so `SUM(A1:A10)` works with the range as a
//! single argument containing 10 values).

use ambre_bus::{Col, Row};
use ambre_core::Validated;
use ambre_core::index::Pos;

use super::types::{CellValue, FormulaError, FunctionName, finite_number, nearly_equal};
use crate::message::{DateParts, DayOfMonth};
use ambre_core::Year;
use ambre_core::{Day, Instant, Months, SheetDateTime, SheetDay, TimeSpan};

// -----------------------------------------------------------------------------
// Dispatch
// -----------------------------------------------------------------------------

// The ~120 builtins are split across sibling modules; the dispatcher and shared
// helpers/types stay here. All are re-globbed for transparent cross-reference.
mod builtins_a;
mod builtins_b;
mod builtins_c;

/// The exact total `SUM` takes, for the status band (`TRAME-FEAT-029`).
pub(crate) use builtins_a::exact_sum;
use builtins_a::*;
use builtins_b::*;
use builtins_c::*;

/// Ambient evaluation context threaded into every built-in call.
///
/// `now_serial` is the recomputation clock, shared so `TODAY()`/`NOW()` are
/// stable within a pass. `cell_row`/`cell_col` are the grid position of the
/// cell being evaluated, consumed by the no-argument `ROW()`/`COLUMN()`.
#[derive(Clone, Copy)]
pub struct FnCtx {
    pub now_serial: Instant<SheetDateTime>,
    /// The row of the evaluating cell, `None` when it is unknown.
    pub cell_row: Option<Pos<Row>>,
    /// The column of the evaluating cell, `None` when it is unknown.
    pub cell_col: Option<Pos<Col>>,
}

impl FnCtx {
    /// Context with only the clock set (helper for tests / non-positional calls).
    #[cfg(test)]
    fn at_clock(now_serial: Instant<SheetDateTime>) -> Self {
        Self {
            now_serial,
            cell_row: None,
            cell_col: None,
        }
    }
}

/// Call the named built-in function with pre-evaluated arguments.
///
/// A result that is not a finite number answers `#NUM!`, as it does for an
/// operator (`TRAME-244`). One guard here holds every function.
/// It covers a total past the largest number, a product, an exponential, and
/// a function handed a number that is not finite.
pub fn call_builtin(name: &Validated<FunctionName>, args: &[CellValue], ctx: FnCtx) -> CellValue {
    match dispatch(name, args, ctx) {
        Some(CellValue::Number(n)) if !n.is_finite() => CellValue::Error(FormulaError::NumError),
        Some(value) => value,
        // A name no built-in carries reads `#N/A`.
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// Whether the engine carries a built-in named `name`. An import sorts a
/// formula that calls any other function into the verbatim state
/// (`TRAME-IO-010`). The probe calls the function on no argument: every
/// built-in is pure, so the call has no effect.
pub fn is_builtin(name: &Validated<FunctionName>) -> bool {
    let probe = FnCtx {
        now_serial: Instant::<SheetDateTime>::new(0.0),
        cell_row: None,
        cell_col: None,
    };
    dispatch(name, &[], probe).is_some()
}

/// The function `name` names, applied to `args`. `None` for a name no
/// built-in carries.
fn dispatch(name: &Validated<FunctionName>, args: &[CellValue], ctx: FnCtx) -> Option<CellValue> {
    let now_serial = ctx.now_serial;
    Some(match name.as_str() {
        // Math
        "SUM" => sum(args),
        "AVERAGE" => average(args),
        "MIN" => min(args),
        "MAX" => max(args),
        "COUNT" => count(args),
        "COUNTA" => counta(args),
        "ABS" => abs(args),
        "ROUND" => round(args),
        "SQRT" => sqrt(args),
        "POWER" => power(args),
        "SUMIF" => sumif(args),
        "COUNTIF" => countif(args),
        "INT" => int_fn(args),
        "MOD" => mod_fn(args),
        "ROUNDUP" => round_dir(args, Rounding::Up),
        "ROUNDDOWN" => round_dir(args, Rounding::Down),
        "CEILING" => ceil_floor(args, Rounding::Up),
        "FLOOR" => ceil_floor(args, Rounding::Down),
        "PRODUCT" => product(args),
        "SUMPRODUCT" => sumproduct(args),
        "MEDIAN" => median(args),
        "COUNTBLANK" => countblank(args),
        "TRUNC" => trunc(args),
        "SIGN" => sign(args),
        "SUMSQ" => sumsq(args),
        "EXP" => unary_math(args, f64::exp),
        "LN" => log_checked(args, |n| n.ln()),
        "LOG10" => log_checked(args, |n| n.log10()),
        "LOG" => log_fn(args),
        "PI" => CellValue::Number(std::f64::consts::PI),
        "SIN" => unary_math(args, f64::sin),
        "COS" => unary_math(args, f64::cos),
        "TAN" => unary_math(args, f64::tan),
        "RADIANS" => unary_math(args, f64::to_radians),
        "DEGREES" => unary_math(args, f64::to_degrees),

        // Statistics
        "AVERAGEIF" => averageif(args),
        "AVERAGEIFS" => averageifs(args),
        "SUMIFS" => sumifs(args),
        "COUNTIFS" => countifs(args),
        "STDEV" => stdev(args),
        "VAR" => variance(args),
        "LARGE" => large_small(args, Rank::Largest),
        "SMALL" => large_small(args, Rank::Smallest),
        "RANK" => rank(args),
        "MODE" => mode(args),

        // Financial
        "PMT" => pmt(args),
        "FV" => fv(args),
        "PV" => pv(args),
        "RATE" => rate(args),
        "NPER" => nper(args),
        "NPV" => npv(args),
        "IRR" => irr(args),

        // Logic
        "IF" => if_fn(args),
        "IFERROR" => iferror(args),
        "AND" => and(args),
        "OR" => or(args),
        "NOT" => not(args),
        "XOR" => xor(args),
        "ISBLANK" => is_blank(args),
        "ISNUMBER" => is_number(args),
        "ISTEXT" => is_text(args),
        "ISERROR" => is_error(args),
        "ISNA" => is_na(args),
        "ISEVEN" => is_even(args),
        "ISODD" => is_odd(args),
        "IFNA" => ifna(args),
        "IFS" => ifs(args),
        "SWITCH" => switch_fn(args),
        "TRUE" => CellValue::Boolean(true),
        "FALSE" => CellValue::Boolean(false),

        // Lookup
        "VLOOKUP" => vlookup(args),
        "HLOOKUP" => hlookup(args),
        "INDEX" => index(args),
        "MATCH" => match_fn(args),
        "CHOOSE" => choose(args),
        "ROWS" => CellValue::Number(range_dims(args).map_or(0.0, |(r, _)| f64::from(r.get()))),
        "COLUMNS" => CellValue::Number(range_dims(args).map_or(0.0, |(_, c)| f64::from(c.get()))),
        "ROW" => row_col(args, ctx.cell_row.map(|r| f64::from(r.get()) + 1.0)),
        "COLUMN" => row_col(args, ctx.cell_col.map(|c| f64::from(c.get()) + 1.0)),

        // Text
        "CONCAT" | "CONCATENATE" => concat_fn(args),
        "TEXT" => text(args),
        "LEFT" => left(args),
        "RIGHT" => right(args),
        "MID" => mid(args),
        "UPPER" => upper(args),
        "LOWER" => lower(args),
        "TRIM" => trim(args),
        "LEN" => len(args),
        "PROPER" => proper(args),
        "SUBSTITUTE" => substitute(args),
        "REPT" => rept(args),
        "FIND" => find_fn(args),
        "SEARCH" => search_fn(args),
        "REPLACE" => replace_fn(args),
        "EXACT" => exact(args),
        "TEXTJOIN" => textjoin(args),
        "CHAR" => char_fn(args),
        "CODE" => code_fn(args),
        "VALUE" => value_fn(args),

        // Date / time (serial-number based, epoch 1899-12-30)
        "TODAY" => CellValue::Number(now_serial.get().floor()),
        "NOW" => CellValue::Number(now_serial.get()),
        "DATE" => date(args),
        "YEAR" => year(args),
        "MONTH" => month(args),
        "DAY" => day(args),
        "TIME" => time_fn(args),
        "HOUR" => time_part(args, TimePart::Hour),
        "MINUTE" => time_part(args, TimePart::Minute),
        "SECOND" => time_part(args, TimePart::Second),
        "WEEKDAY" => weekday(args),
        "EDATE" => edate(args),
        "EOMONTH" => eomonth(args),

        _ => return None,
    })
}

/// Which way a rounding function moves a value it does not keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rounding {
    /// Away from zero, or to the next multiple above.
    Up,
    /// Toward zero, or to the next multiple below.
    Down,
}

/// Which end of the sorted values LARGE or SMALL counts its `k` from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rank {
    Largest,
    Smallest,
}

/// Whether a criterion holds where its comparison does, or where it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polarity {
    Match,
    /// The `<>`, `>=` and `<=` operators: the comparison negated.
    NotMatch,
}

/// A parsed SUMIF/COUNTIF criterion: a comparison operator plus an operand.
enum Criterion {
    Num(std::cmp::Ordering, f64, Polarity),
    Text(std::cmp::Ordering, String, Polarity),
}

impl Criterion {
    fn parse(value: &CellValue) -> Self {
        // A bare number/boolean criterion means strict equality.
        let raw = match value {
            CellValue::Text(s) => s.clone(),
            other => return Self::eq_value(other),
        };
        let (op_str, rest) = if let Some(r) = raw.strip_prefix(">=") {
            (">=", r)
        } else if let Some(r) = raw.strip_prefix("<=") {
            ("<=", r)
        } else if let Some(r) = raw.strip_prefix("<>") {
            ("<>", r)
        } else if let Some(r) = raw.strip_prefix('>') {
            (">", r)
        } else if let Some(r) = raw.strip_prefix('<') {
            ("<", r)
        } else if let Some(r) = raw.strip_prefix('=') {
            ("=", r)
        } else {
            ("=", raw.as_str())
        };
        use std::cmp::Ordering::*;
        let (ord, negated) = match op_str {
            ">" => (Greater, Polarity::Match),
            "<" => (Less, Polarity::Match),
            ">=" => (Less, Polarity::NotMatch),    // not Less
            "<=" => (Greater, Polarity::NotMatch), // not Greater
            "<>" => (Equal, Polarity::NotMatch),
            _ => (Equal, Polarity::Match),
        };
        match finite_number(rest) {
            Some(n) => Criterion::Num(ord, n, negated),
            None => Criterion::Text(ord, rest.to_string(), negated),
        }
    }

    fn eq_value(value: &CellValue) -> Self {
        match value.as_number() {
            Ok(n) => Criterion::Num(std::cmp::Ordering::Equal, n, Polarity::Match),
            Err(_) => Criterion::Text(
                std::cmp::Ordering::Equal,
                value.to_display(),
                Polarity::Match,
            ),
        }
    }

    fn matches(&self, value: &CellValue) -> bool {
        match self {
            Criterion::Num(ord, operand, negated) => match value.as_number() {
                Ok(n) => {
                    let hit = n.partial_cmp(operand) == Some(*ord);
                    match negated {
                        Polarity::Match => hit,
                        Polarity::NotMatch => !hit,
                    }
                }
                // A cell that does not coerce to a number cannot be ordered
                // against one, so `>5` and `>=5` reject it. `<>5` is the sole
                // exception: "different from 5" holds of any non-number.
                Err(_) => *negated == Polarity::NotMatch && *ord == std::cmp::Ordering::Equal,
            },
            Criterion::Text(ord, operand, negated) => {
                let hit = crate::formula::text::fold_cmp(&value.to_display(), operand) == *ord;
                match negated {
                    Polarity::Match => hit,
                    Polarity::NotMatch => !hit,
                }
            }
        }
    }
}

/// Trait extension to turn f64::NAN into a default on folds.
trait NanDefault {
    fn let_ok_or(self, default: CellValue) -> CellValue;
}

impl NanDefault for f64 {
    fn let_ok_or(self, default: CellValue) -> CellValue {
        if self.is_nan() {
            default
        } else {
            CellValue::Number(self)
        }
    }
}

#[cfg(test)]
mod tests;
