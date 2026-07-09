//! Built-in spreadsheet functions.
//!
//! Each function receives a slice of already-evaluated [`CellValue`] arguments
//! and returns a single result. Functions that expect numeric arguments flatten
//! `Range` values transparently (so `SUM(A1:A10)` works with the range as a
//! single argument containing 10 values).

use super::types::{CellValue, FormulaError, nearly_equal};

// -----------------------------------------------------------------------------
// Dispatch
// -----------------------------------------------------------------------------

// The ~120 builtins are split across sibling modules; the dispatcher and shared
// helpers/types stay here. All are re-globbed for transparent cross-reference.
mod builtins_a;
mod builtins_b;
mod builtins_c;

use builtins_a::*;
use builtins_b::*;
use builtins_c::*;

/// Ambient evaluation context threaded into every built-in call.
///
/// `now_serial` is the recomputation clock (shared so `TODAY()`/`NOW()` are
/// stable within a pass); `cell_row`/`cell_col` are the 1-based grid position of
/// the cell being evaluated, consumed by the no-argument `ROW()`/`COLUMN()`.
/// A position of `0` means "unknown".
#[derive(Clone, Copy)]
pub struct FnCtx {
    pub now_serial: f64,
    pub cell_row: u32,
    pub cell_col: u32,
}

impl FnCtx {
    /// Context with only the clock set (helper for tests / non-positional calls).
    #[cfg(test)]
    fn at_clock(now_serial: f64) -> Self {
        Self {
            now_serial,
            cell_row: 0,
            cell_col: 0,
        }
    }
}

/// Call the named built-in function with pre-evaluated arguments.
pub fn call_builtin(name: &str, args: &[CellValue], ctx: FnCtx) -> CellValue {
    let now_serial = ctx.now_serial;
    match name {
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
        "ROUNDUP" => round_dir(args, true),
        "ROUNDDOWN" => round_dir(args, false),
        "CEILING" => ceil_floor(args, true),
        "FLOOR" => ceil_floor(args, false),
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
        "LARGE" => large_small(args, true),
        "SMALL" => large_small(args, false),
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
        "ROWS" => CellValue::Number(range_dims(args).map(|(r, _)| r).unwrap_or(0) as f64),
        "COLUMNS" => CellValue::Number(range_dims(args).map(|(_, c)| c).unwrap_or(0) as f64),
        "ROW" => row_col(args, ctx.cell_row),
        "COLUMN" => row_col(args, ctx.cell_col),

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
        "TODAY" => CellValue::Number(now_serial.floor()),
        "NOW" => CellValue::Number(now_serial),
        "DATE" => date(args),
        "YEAR" => year(args),
        "MONTH" => month(args),
        "DAY" => day(args),
        "TIME" => time_fn(args),
        "HOUR" => time_part(args, 0),
        "MINUTE" => time_part(args, 1),
        "SECOND" => time_part(args, 2),
        "WEEKDAY" => weekday(args),
        "EDATE" => edate(args),
        "EOMONTH" => eomonth(args),

        _ => CellValue::Error(FormulaError::NaError),
    }
}

/// A parsed SUMIF/COUNTIF criterion: a comparison operator plus an operand.
enum Criterion {
    Num(std::cmp::Ordering, f64, bool), // (op, operand, negated for `<>`)
    Text(std::cmp::Ordering, String, bool),
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
            ">" => (Greater, false),
            "<" => (Less, false),
            ">=" => (Less, true),    // not Less
            "<=" => (Greater, true), // not Greater
            "<>" => (Equal, true),
            _ => (Equal, false),
        };
        match rest.parse::<f64>() {
            Ok(n) => Criterion::Num(ord, n, negated),
            Err(_) => Criterion::Text(ord, rest.to_string(), negated),
        }
    }

    fn eq_value(value: &CellValue) -> Self {
        match value.as_number() {
            Ok(n) => Criterion::Num(std::cmp::Ordering::Equal, n, false),
            Err(_) => Criterion::Text(std::cmp::Ordering::Equal, value.to_display(), false),
        }
    }

    fn matches(&self, value: &CellValue) -> bool {
        match self {
            Criterion::Num(ord, operand, negated) => match value.as_number() {
                Ok(n) => {
                    let cmp = n.partial_cmp(operand);
                    let hit = cmp == Some(*ord);
                    if *negated { !hit } else { hit }
                }
                // A cell that does not coerce to a number cannot be ordered
                // against one, so `>5` and `>=5` reject it. `<>5` is the sole
                // exception: "different from 5" holds of any non-number.
                Err(_) => *negated && *ord == std::cmp::Ordering::Equal,
            },
            Criterion::Text(ord, operand, negated) => {
                let hit = value
                    .to_display()
                    .to_lowercase()
                    .cmp(&operand.to_lowercase())
                    == *ord;
                if *negated { !hit } else { hit }
            }
        }
    }
}

/// Current wall-clock time as an Excel serial number. The integer part is the
/// day; the fractional part is the time of day.
pub fn now_serial() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        // js_sys::Date::now() returns milliseconds since the Unix epoch.
        js_sys::Date::now() / 86_400_000.0 + 25_569.0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        secs / 86_400.0 + 25_569.0
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
