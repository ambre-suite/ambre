//! Built-in spreadsheet functions.
//!
//! Each function receives a slice of already-evaluated [`CellValue`] arguments
//! and returns a single result. Functions that expect numeric arguments flatten
//! `Range` values transparently (so `SUM(A1:A10)` works with the range as a
//! single argument containing 10 values).

use super::types::{CellValue, FormulaError};

// -----------------------------------------------------------------------------
// Dispatch
// -----------------------------------------------------------------------------

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
        "STDEV" => stdev(args, true),
        "VAR" => variance(args, true),
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

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

/// Flatten all arguments into a single numeric iterator. Range values are
/// unpacked; non-numeric values are silently skipped.
fn flatten_numeric(args: &[CellValue]) -> Vec<f64> {
    let mut nums = Vec::new();
    for arg in args {
        flatten_numeric_inner(arg, &mut nums);
    }
    nums
}

fn flatten_numeric_inner(value: &CellValue, out: &mut Vec<f64>) {
    match value {
        CellValue::Number(n) => out.push(*n),
        CellValue::Range(r) => {
            for v in &r.values {
                flatten_numeric_inner(v, out);
            }
        }
        CellValue::Boolean(true) => out.push(1.0),
        CellValue::Boolean(false) => out.push(0.0),
        CellValue::Text(s) => {
            if let Ok(n) = s.parse::<f64>() {
                out.push(n);
            }
        }
        _ => {}
    }
}

/// Flatten all arguments into a flat list of values (for COUNTA etc.).
fn flatten_all(args: &[CellValue]) -> Vec<CellValue> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            CellValue::Range(r) => out.extend(r.values.iter().cloned()),
            other => out.push(other.clone()),
        }
    }
    out
}

/// Require exactly `n` arguments; return an error if not met.
#[allow(dead_code)]
fn check_argc(args: &[CellValue], expected: usize, _name: &str) -> Result<(), CellValue> {
    if args.len() != expected {
        return Err(CellValue::Error(FormulaError::NaError));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Math functions
// -----------------------------------------------------------------------------

fn sum(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    if nums.is_empty() {
        return CellValue::Number(0.0);
    }
    CellValue::Number(nums.iter().sum())
}

fn average(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    if nums.is_empty() {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    CellValue::Number(nums.iter().sum::<f64>() / nums.len() as f64)
}

fn min(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    nums.iter()
        .cloned()
        .fold(f64::NAN, f64::min)
        .let_ok_or(CellValue::Number(0.0))
}

fn max(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    nums.iter()
        .cloned()
        .fold(f64::NAN, f64::max)
        .let_ok_or(CellValue::Number(0.0))
}

fn count(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    CellValue::Number(nums.len() as f64)
}

fn counta(args: &[CellValue]) -> CellValue {
    let flat = flatten_all(args);
    let non_empty = flat
        .iter()
        .filter(|v| !matches!(v, CellValue::Empty))
        .count();
    CellValue::Number(non_empty as f64)
}

fn abs(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match flatten_numeric(&args[..1]).first() {
        Some(n) => CellValue::Number(n.abs()),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

fn round(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let num = match flatten_numeric(&args[..1]).first() {
        Some(n) => *n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let digits = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(0.0) as i32
    } else {
        0
    };
    let factor = 10f64.powi(digits);
    CellValue::Number((num * factor).round() / factor)
}

fn sqrt(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match flatten_numeric(&args[..1]).first() {
        Some(&n) if n >= 0.0 => CellValue::Number(n.sqrt()),
        Some(_) => CellValue::Error(FormulaError::NumError),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

fn power(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let base = match flatten_numeric(&args[..1]).first() {
        Some(n) => *n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let exp = match flatten_numeric(&args[1..2]).first() {
        Some(n) => *n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    CellValue::Number(base.powf(exp))
}

/// Read the first argument as a single number, or `None` if absent/non-numeric.
fn first_num(args: &[CellValue]) -> Option<f64> {
    flatten_numeric(&args[..args.len().min(1)]).first().copied()
}

/// INT(n) -- round down to the nearest integer (toward negative infinity).
fn int_fn(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Number(n.floor()),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// MOD(number, divisor) -- remainder with the sign of the divisor (Excel style).
fn mod_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (a, b) = match (first_num(&args[..1]), first_num(&args[1..2])) {
        (Some(a), Some(b)) => (a, b),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    if b == 0.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    CellValue::Number(a - b * (a / b).floor())
}

/// ROUNDUP / ROUNDDOWN(number, [digits]) -- round away from / toward zero.
fn round_dir(args: &[CellValue], up: bool) -> CellValue {
    let num = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let digits = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(0.0) as i32
    } else {
        0
    };
    let factor = 10f64.powi(digits);
    let scaled = num * factor;
    let rounded = if up {
        scaled.abs().ceil().copysign(scaled)
    } else {
        scaled.trunc()
    };
    CellValue::Number(rounded / factor)
}

/// CEILING / FLOOR(number, [significance]) -- round to a multiple of `significance`.
fn ceil_floor(args: &[CellValue], up: bool) -> CellValue {
    let num = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let sig = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(1.0)
    } else {
        1.0
    };
    if sig == 0.0 {
        return CellValue::Number(0.0);
    }
    let ratio = num / sig;
    let stepped = if up { ratio.ceil() } else { ratio.floor() };
    CellValue::Number(stepped * sig)
}

/// PRODUCT(...) -- product of all numeric arguments (empty input -> 0).
fn product(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    if nums.is_empty() {
        return CellValue::Number(0.0);
    }
    CellValue::Number(nums.iter().product())
}

/// SUMPRODUCT(range1, range2, ...) -- sum of the element-wise products of the
/// arguments, paired by position. Non-numeric cells contribute 0.
fn sumproduct(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let columns: Vec<Vec<f64>> = args
        .iter()
        .map(|a| {
            range_cells(a)
                .map(|v| v.as_number().unwrap_or(0.0))
                .collect()
        })
        .collect();
    let len = columns.iter().map(Vec::len).min().unwrap_or(0);
    let total = (0..len)
        .map(|i| columns.iter().map(|c| c[i]).product::<f64>())
        .sum();
    CellValue::Number(total)
}

/// MEDIAN(...) -- middle value of the numeric arguments (mean of the two
/// central values for an even count).
fn median(args: &[CellValue]) -> CellValue {
    let mut nums = flatten_numeric(args);
    if nums.is_empty() {
        return CellValue::Error(FormulaError::NumError);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = nums.len() / 2;
    let m = if nums.len().is_multiple_of(2) {
        (nums[mid - 1] + nums[mid]) / 2.0
    } else {
        nums[mid]
    };
    CellValue::Number(m)
}

/// COUNTBLANK(range) -- count empty cells in the argument(s).
fn countblank(args: &[CellValue]) -> CellValue {
    let n = flatten_all(args)
        .iter()
        .filter(|v| matches!(v, CellValue::Empty))
        .count();
    CellValue::Number(n as f64)
}

// -----------------------------------------------------------------------------
// Logic functions
// -----------------------------------------------------------------------------

fn if_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let condition = is_truthy(&args[0]);
    if condition {
        args[1].clone()
    } else if args.len() >= 3 {
        args[2].clone()
    } else {
        CellValue::Boolean(false)
    }
}

fn and(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flat = flatten_all(args);
    CellValue::Boolean(flat.iter().all(is_truthy))
}

fn or(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flat = flatten_all(args);
    CellValue::Boolean(flat.iter().any(is_truthy))
}

fn not(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Boolean(!is_truthy(&args[0]))
}

/// XOR(...) -- TRUE when an odd number of arguments are truthy.
fn xor(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let truthy = flatten_all(args).iter().filter(|v| is_truthy(v)).count();
    CellValue::Boolean(truthy % 2 == 1)
}

/// ISBLANK(value) -- TRUE when the argument is an empty cell.
fn is_blank(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Empty)),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// ISNUMBER(value) -- TRUE when the argument is a number.
fn is_number(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Number(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

fn is_truthy(value: &CellValue) -> bool {
    match value {
        CellValue::Number(n) => *n != 0.0,
        CellValue::Text(s) => !s.is_empty(),
        CellValue::Boolean(b) => *b,
        CellValue::Empty => false,
        CellValue::Error(_) => false,
        CellValue::Range(r) => !r.values.is_empty(),
    }
}

// -----------------------------------------------------------------------------
// Text functions
// -----------------------------------------------------------------------------

fn concat_fn(args: &[CellValue]) -> CellValue {
    let mut result = String::new();
    for arg in args {
        push_display(arg, &mut result);
    }
    CellValue::Text(result)
}

fn left(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = args[0].to_display();
    let n = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(1.0) as usize
    } else {
        1
    };
    CellValue::Text(s.chars().take(n).collect())
}

fn right(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = args[0].to_display();
    let n = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(1.0) as usize
    } else {
        1
    };
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    CellValue::Text(chars[start..].iter().collect())
}

fn mid(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = args[0].to_display();
    let start = flatten_numeric(&args[1..2])
        .first()
        .copied()
        .unwrap_or(1.0)
        .max(1.0) as usize
        - 1;
    let n = flatten_numeric(&args[2..3])
        .first()
        .copied()
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let chars: Vec<char> = s.chars().collect();
    let slice: String = chars.iter().skip(start).take(n).collect();
    CellValue::Text(slice)
}

fn upper(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Text(args[0].to_display().to_uppercase())
}

fn lower(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Text(args[0].to_display().to_lowercase())
}

fn trim(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = args[0].to_display();
    CellValue::Text(s.trim().to_string())
}

fn len(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Number(args[0].to_display().chars().count() as f64)
}

/// PROPER(text) -- capitalize the first letter of each word, lowercase the rest.
fn proper(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let mut out = String::new();
    let mut at_word_start = true;
    for c in args[0].to_display().chars() {
        if c.is_alphabetic() {
            if at_word_start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    CellValue::Text(out)
}

/// SUBSTITUTE(text, old, new, [instance]) -- replace occurrences of `old`.
///
/// Without `instance`, every occurrence is replaced; with it, only the Nth
/// (1-based) occurrence is. An empty `old` returns the text unchanged.
fn substitute(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let text = args[0].to_display();
    let old = args[1].to_display();
    let new = args[2].to_display();
    if old.is_empty() {
        return CellValue::Text(text);
    }
    let instance = args
        .get(3)
        .and_then(|v| v.as_number().ok())
        .map(|n| n as usize);
    let result = match instance {
        None => text.replace(&old, &new),
        Some(target) if target >= 1 => {
            let mut out = String::new();
            let mut rest = text.as_str();
            let mut seen = 0;
            while let Some(pos) = rest.find(&old) {
                seen += 1;
                out.push_str(&rest[..pos]);
                if seen == target {
                    out.push_str(&new);
                } else {
                    out.push_str(&old);
                }
                rest = &rest[pos + old.len()..];
            }
            out.push_str(rest);
            out
        }
        Some(_) => text,
    };
    CellValue::Text(result)
}

/// REPT(text, count) -- repeat `text` `count` times.
fn rept(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let n = match args[1].as_number() {
        Ok(n) if n >= 0.0 => n as usize,
        Ok(_) => return CellValue::Error(FormulaError::NumError),
        Err(e) => return CellValue::Error(e),
    };
    CellValue::Text(args[0].to_display().repeat(n))
}

/// FIND(find_text, within_text, [start]) -- 1-based, case-sensitive position of
/// `find_text` in `within_text`. Returns `#VALUE!` (TypeError) when not found.
fn find_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let needle = args[0].to_display();
    let haystack: Vec<char> = args[1].to_display().chars().collect();
    let start = args
        .get(2)
        .and_then(|v| v.as_number().ok())
        .map(|n| (n as usize).max(1) - 1)
        .unwrap_or(0);
    if start > haystack.len() {
        return CellValue::Error(FormulaError::TypeError);
    }
    // Char-aware search from `start`: rebuild the searchable suffix as a string.
    let suffix: String = haystack[start..].iter().collect();
    match suffix.find(&needle) {
        // `find` yields a byte offset; convert it to a char offset.
        Some(byte_pos) => {
            let char_off = suffix[..byte_pos].chars().count();
            CellValue::Number((start + char_off + 1) as f64)
        }
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// VALUE(text) -- coerce a textual number into a numeric value.
fn value_fn(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match args[0].as_number() {
        Ok(n) => CellValue::Number(n),
        Err(_) => CellValue::Error(FormulaError::TypeError),
    }
}

/// TEXT(value, format) -- minimal numeric formatting.
///
/// Supports the common Excel patterns: a run of `0`/`#` with an optional `.`
/// decimal section controls the number of decimals, and a leading `,` (in the
/// integer section) turns on the thousands separator. Non-numeric values are
/// returned via their display form. Full Excel format-code parsing is out of
/// scope.
fn text(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let fmt = args[1].to_display();
    let num = match args[0].as_number() {
        Ok(n) => n,
        Err(_) => return CellValue::Text(args[0].to_display()),
    };

    let decimals = match fmt.split_once('.') {
        Some((_, frac)) => frac.chars().filter(|c| *c == '0' || *c == '#').count(),
        None => 0,
    };
    let thousands = fmt.split('.').next().unwrap_or("").contains(',');

    let mut out = format!("{:.*}", decimals, num);
    if thousands {
        out = group_thousands(&out);
    }
    CellValue::Text(out)
}

/// Insert `,` every three digits in the integer part of a (possibly signed,
/// possibly fractional) numeric string.
fn group_thousands(s: &str) -> String {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::new();
    let bytes = int_part.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*b as char);
    }
    match frac_part {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

// -----------------------------------------------------------------------------
// Conditional aggregation (SUMIF / COUNTIF)
// -----------------------------------------------------------------------------

/// COUNTIF(range, criterion) -- count cells in `range` matching `criterion`.
fn countif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(&args[1]);
    let n = range_cells(&args[0]).filter(|v| crit.matches(v)).count();
    CellValue::Number(n as f64)
}

/// SUMIF(range, criterion, [sum_range]) -- sum cells matching `criterion`.
///
/// When `sum_range` is omitted, the criterion range is summed; otherwise the
/// two ranges are paired by position (Excel anchors `sum_range` at its top-left
/// and walks it in parallel with `range`).
fn sumif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(&args[1]);
    let test: Vec<CellValue> = range_cells(&args[0]).cloned().collect();
    let sum_src: Vec<CellValue> = match args.get(2) {
        Some(r) => range_cells(r).cloned().collect(),
        None => test.clone(),
    };
    let mut total = 0.0;
    for (i, v) in test.iter().enumerate() {
        if crit.matches(v) {
            if let Some(CellValue::Number(n)) = sum_src.get(i) {
                total += n;
            } else if let Some(s) = sum_src.get(i)
                && let Ok(n) = s.as_number()
            {
                total += n;
            }
        }
    }
    CellValue::Number(total)
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
                Err(_) => false,
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

// -----------------------------------------------------------------------------
// Logic (cont.)
// -----------------------------------------------------------------------------

/// IFERROR(value, fallback) -- return `fallback` when `value` is an error.
fn iferror(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match &args[0] {
        CellValue::Error(_) => args[1].clone(),
        other => other.clone(),
    }
}

// -----------------------------------------------------------------------------
// Lookup (VLOOKUP / INDEX / MATCH)
// -----------------------------------------------------------------------------

/// VLOOKUP(lookup, table, col_index, [range_lookup]).
///
/// Searches the first column of `table` and returns the value in column
/// `col_index` (1-based) of the matching row. `range_lookup` defaults to TRUE
/// (approximate match on an ascending first column); FALSE forces an exact
/// match.
fn vlookup(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let table = match &args[1] {
        CellValue::Range(r) => r,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    let col_index = match args[2].as_number() {
        Ok(n) if n >= 1.0 => n as usize - 1,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    if col_index >= table.cols {
        return CellValue::Error(FormulaError::NaError);
    }
    let approximate = match args.get(3) {
        Some(v) => is_truthy(v),
        None => true,
    };

    let mut best: Option<usize> = None;
    for row in 0..table.rows {
        let key = match table.get(row, 0) {
            Some(k) => k,
            None => continue,
        };
        if approximate {
            // Largest key <= lookup (first column assumed ascending).
            if compare_cells(key, &args[0])
                .map(|o| o.is_le())
                .unwrap_or(false)
            {
                best = Some(row);
            } else {
                break;
            }
        } else if values_equal_ci(key, &args[0]) {
            best = Some(row);
            break;
        }
    }
    match best.and_then(|row| table.get(row, col_index)) {
        Some(v) => v.clone(),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// INDEX(range, row_num, [col_num]) -- 1-based cell lookup inside `range`.
///
/// For a single-row or single-column range, `col_num` may be omitted and
/// `row_num` is interpreted along the populated axis.
fn index(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let range = match &args[0] {
        CellValue::Range(r) => r,
        // A scalar behaves like a 1x1 range.
        other => {
            return if to_index(&args[1]) == Some(1) {
                other.clone()
            } else {
                CellValue::Error(FormulaError::NaError)
            };
        }
    };
    let n1 = match to_index(&args[1]) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::NaError),
    };
    let (row, col) = match args.get(2).and_then(to_index) {
        Some(c) => (n1, c),
        None if range.rows == 1 => (1, n1),
        None if range.cols == 1 => (n1, 1),
        None => (n1, 1),
    };
    match range.get(row - 1, col - 1) {
        Some(v) => v.clone(),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// MATCH(lookup, range, [match_type]) -- 1-based position in a 1D `range`.
///
/// `match_type`: 0 = exact; 1 (default) = largest value <= lookup (ascending);
/// -1 = smallest value >= lookup (descending).
fn match_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let cells: Vec<&CellValue> = range_cells(&args[1]).collect();
    let match_type = args
        .get(2)
        .and_then(|v| v.as_number().ok())
        .map(|n| n as i64)
        .unwrap_or(1);

    if match_type == 0 {
        for (i, c) in cells.iter().enumerate() {
            if values_equal_ci(c, &args[0]) {
                return CellValue::Number((i + 1) as f64);
            }
        }
        return CellValue::Error(FormulaError::NaError);
    }

    let mut best: Option<usize> = None;
    for (i, c) in cells.iter().enumerate() {
        let ord = match compare_cells(c, &args[0]) {
            Some(o) => o,
            None => continue,
        };
        let acceptable = if match_type >= 1 {
            ord.is_le() // value <= lookup, list ascending
        } else {
            ord.is_ge() // value >= lookup, list descending
        };
        if acceptable {
            best = Some(i);
        } else {
            break;
        }
    }
    match best {
        Some(i) => CellValue::Number((i + 1) as f64),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// Convert a value to a 1-based positive index, if possible.
fn to_index(value: &CellValue) -> Option<usize> {
    match value.as_number() {
        Ok(n) if n >= 1.0 => Some(n as usize),
        _ => None,
    }
}

/// Iterate the cells of an argument: a `Range` yields its elements in order, a
/// scalar yields itself once.
fn range_cells(value: &CellValue) -> Box<dyn Iterator<Item = &CellValue> + '_> {
    match value {
        CellValue::Range(r) => Box::new(r.values.iter()),
        other => Box::new(std::iter::once(other)),
    }
}

/// Case-insensitive equality used by lookups (numbers compared numerically).
fn values_equal_ci(a: &CellValue, b: &CellValue) -> bool {
    match (a.as_number(), b.as_number()) {
        (Ok(x), Ok(y)) => (x - y).abs() < f64::EPSILON,
        _ => a.to_display().eq_ignore_ascii_case(&b.to_display()),
    }
}

/// Ordering between two cells: numeric when both coerce to numbers, otherwise a
/// case-insensitive lexical comparison.
fn compare_cells(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (a.as_number(), b.as_number()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y),
        _ => Some(
            a.to_display()
                .to_lowercase()
                .cmp(&b.to_display().to_lowercase()),
        ),
    }
}

// -----------------------------------------------------------------------------
// Date / time (Excel serial numbers, epoch 1899-12-30)
// -----------------------------------------------------------------------------

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

/// DATE(year, month, day) -> serial number. Two-digit-style years (0..1899)
/// are offset by 1900, matching Excel.
fn date(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (y, m, d) = match (
        args[0].as_number(),
        args[1].as_number(),
        args[2].as_number(),
    ) {
        (Ok(y), Ok(m), Ok(d)) => (y as i64, m as i64, d as i64),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    let year = if (0..1900).contains(&y) { y + 1900 } else { y };
    // Normalize month overflow/underflow into the year (Excel allows it).
    let year = year + (m - 1).div_euclid(12);
    let month = (m - 1).rem_euclid(12) + 1;
    let serial = serial_from_ymd(year, month, d);
    CellValue::Number(serial as f64)
}

fn year(args: &[CellValue]) -> CellValue {
    date_part(args, |(y, _, _)| y)
}
fn month(args: &[CellValue]) -> CellValue {
    date_part(args, |(_, m, _)| m)
}
fn day(args: &[CellValue]) -> CellValue {
    date_part(args, |(_, _, d)| d)
}

fn date_part(args: &[CellValue], pick: fn((i64, i64, i64)) -> i64) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match args[0].as_number() {
        Ok(serial) => {
            let ymd = ymd_from_serial(serial.floor() as i64);
            CellValue::Number(pick(ymd) as f64)
        }
        Err(_) => CellValue::Error(FormulaError::TypeError),
    }
}

/// Days from the Excel epoch (1899-12-30) for a proleptic-Gregorian date.
/// Uses Howard Hinnant's `days_from_civil` algorithm against the Unix epoch.
fn serial_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    let yy = y - i64::from(m <= 2);
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let unix_days = era * 146_097 + doe - 719_468;
    unix_days + 25_569
}

/// Inverse of [`serial_from_ymd`]: `(year, month, day)` from an Excel serial.
fn ymd_from_serial(serial: i64) -> (i64, i64, i64) {
    let z = serial - 25_569 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// -----------------------------------------------------------------------------
// Helper
// -----------------------------------------------------------------------------

/// Flatten a value (including nested ranges) into a string builder.
fn push_display(value: &CellValue, out: &mut String) {
    match value {
        CellValue::Range(r) => {
            for v in &r.values {
                push_display(v, out);
            }
        }
        other => out.push_str(&other.to_display()),
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

// -----------------------------------------------------------------------------
// Math (cont.)
// -----------------------------------------------------------------------------

/// TRUNC(number, [digits]) -- truncate toward zero at `digits` decimals.
fn trunc(args: &[CellValue]) -> CellValue {
    let num = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let digits = if args.len() >= 2 {
        flatten_numeric(&args[1..2]).first().copied().unwrap_or(0.0) as i32
    } else {
        0
    };
    let factor = 10f64.powi(digits);
    CellValue::Number((num * factor).trunc() / factor)
}

/// SIGN(number) -- -1, 0 or 1.
fn sign(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Number(if n > 0.0 {
            1.0
        } else if n < 0.0 {
            -1.0
        } else {
            0.0
        }),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// SUMSQ(...) -- sum of the squares of all numeric arguments.
fn sumsq(args: &[CellValue]) -> CellValue {
    CellValue::Number(flatten_numeric(args).iter().map(|n| n * n).sum())
}

/// Apply a total `f64 -> f64` function to the single numeric argument.
fn unary_math(args: &[CellValue], f: fn(f64) -> f64) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Number(f(n)),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// Apply a logarithm to the single argument, rejecting non-positive inputs.
fn log_checked(args: &[CellValue], f: fn(f64) -> f64) -> CellValue {
    match first_num(args) {
        Some(n) if n > 0.0 => CellValue::Number(f(n)),
        Some(_) => CellValue::Error(FormulaError::NumError),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// LOG(number, [base=10]).
fn log_fn(args: &[CellValue]) -> CellValue {
    let num = match first_num(args) {
        Some(n) if n > 0.0 => n,
        Some(_) => return CellValue::Error(FormulaError::NumError),
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let base = if args.len() >= 2 {
        flatten_numeric(&args[1..2])
            .first()
            .copied()
            .unwrap_or(10.0)
    } else {
        10.0
    };
    if base <= 0.0 || base == 1.0 {
        return CellValue::Error(FormulaError::NumError);
    }
    CellValue::Number(num.log(base))
}

// -----------------------------------------------------------------------------
// Statistics
// -----------------------------------------------------------------------------

/// Build the `(values, criterion)` pairs of a `*IFS`-style argument tail.
/// Returns `None` when the tail is empty or has an odd length.
fn criteria_pairs(args: &[CellValue]) -> Option<Vec<(Vec<CellValue>, Criterion)>> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return None;
    }
    Some(
        args.chunks_exact(2)
            .map(|c| {
                (
                    range_cells(&c[0]).cloned().collect(),
                    Criterion::parse(&c[1]),
                )
            })
            .collect(),
    )
}

/// Positions where every criterion in `pairs` matches (ranges paired by index).
fn matching_indices(pairs: &[(Vec<CellValue>, Criterion)]) -> Vec<usize> {
    let len = pairs.iter().map(|(v, _)| v.len()).min().unwrap_or(0);
    (0..len)
        .filter(|&i| pairs.iter().all(|(vals, crit)| crit.matches(&vals[i])))
        .collect()
}

/// Sum the numeric values of `src` at the given indices.
fn sum_at(src: &[CellValue], idx: &[usize]) -> f64 {
    idx.iter()
        .filter_map(|&i| src.get(i).and_then(|v| v.as_number().ok()))
        .sum()
}

/// AVERAGEIF(range, criterion, [average_range]).
fn averageif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(&args[1]);
    let test: Vec<CellValue> = range_cells(&args[0]).cloned().collect();
    let avg_src: Vec<CellValue> = match args.get(2) {
        Some(r) => range_cells(r).cloned().collect(),
        None => test.clone(),
    };
    let idx: Vec<usize> = (0..test.len())
        .filter(|&i| crit.matches(&test[i]))
        .collect();
    average_at(&avg_src, &idx)
}

/// AVERAGEIFS(average_range, criteria_range1, criteria1, ...).
fn averageifs(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let avg_src: Vec<CellValue> = range_cells(&args[0]).cloned().collect();
    let Some(pairs) = criteria_pairs(&args[1..]) else {
        return CellValue::Error(FormulaError::NaError);
    };
    average_at(&avg_src, &matching_indices(&pairs))
}

/// Average the numeric values of `src` at the given indices (`#DIV/0!` if none).
fn average_at(src: &[CellValue], idx: &[usize]) -> CellValue {
    let nums: Vec<f64> = idx
        .iter()
        .filter_map(|&i| src.get(i).and_then(|v| v.as_number().ok()))
        .collect();
    if nums.is_empty() {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    CellValue::Number(nums.iter().sum::<f64>() / nums.len() as f64)
}

/// SUMIFS(sum_range, criteria_range1, criteria1, ...).
fn sumifs(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let sum_src: Vec<CellValue> = range_cells(&args[0]).cloned().collect();
    let Some(pairs) = criteria_pairs(&args[1..]) else {
        return CellValue::Error(FormulaError::NaError);
    };
    CellValue::Number(sum_at(&sum_src, &matching_indices(&pairs)))
}

/// COUNTIFS(criteria_range1, criteria1, ...).
fn countifs(args: &[CellValue]) -> CellValue {
    let Some(pairs) = criteria_pairs(args) else {
        return CellValue::Error(FormulaError::NaError);
    };
    CellValue::Number(matching_indices(&pairs).len() as f64)
}

/// STDEV / VAR over the sample (denominator `n - 1`).
fn variance(args: &[CellValue], _sample: bool) -> CellValue {
    let nums = flatten_numeric(args);
    if nums.len() < 2 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
    let ss: f64 = nums.iter().map(|n| (n - mean).powi(2)).sum();
    CellValue::Number(ss / (nums.len() - 1) as f64)
}

fn stdev(args: &[CellValue], sample: bool) -> CellValue {
    match variance(args, sample) {
        CellValue::Number(v) => CellValue::Number(v.sqrt()),
        other => other,
    }
}

/// LARGE / SMALL(array..., k) -- the k-th largest / smallest numeric value.
fn large_small(args: &[CellValue], largest: bool) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let k = match args[args.len() - 1].as_number() {
        Ok(n) if n >= 1.0 => n as usize,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let mut nums = flatten_numeric(&args[..args.len() - 1]);
    if k > nums.len() {
        return CellValue::Error(FormulaError::NumError);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = if largest { nums.len() - k } else { k - 1 };
    CellValue::Number(nums[pick])
}

/// RANK(number, ref, [order]) -- 1-based rank; order 0/omitted = descending.
fn rank(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let num = match args[0].as_number() {
        Ok(n) => n,
        Err(e) => return CellValue::Error(e),
    };
    let data: Vec<f64> = range_cells(&args[1])
        .filter_map(|v| v.as_number().ok())
        .collect();
    if !data.iter().any(|n| (n - num).abs() < f64::EPSILON) {
        return CellValue::Error(FormulaError::NaError);
    }
    let descending = !args.get(2).map(is_truthy).unwrap_or(false);
    let better = data
        .iter()
        .filter(|&&n| if descending { n > num } else { n < num })
        .count();
    CellValue::Number((better + 1) as f64)
}

/// MODE(...) -- the most frequently occurring numeric value (first on ties).
fn mode(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    if nums.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let mut best = nums[0];
    let mut best_count = 0usize;
    for (i, &a) in nums.iter().enumerate() {
        let c = nums[i..]
            .iter()
            .filter(|&&b| (b - a).abs() < f64::EPSILON)
            .count();
        if c > best_count {
            best_count = c;
            best = a;
        }
    }
    if best_count < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Number(best)
}

// -----------------------------------------------------------------------------
// Financial (annuities; cash-flow sign convention follows Excel)
// -----------------------------------------------------------------------------

/// `(1+r)^n * pmt-coefficient` shared by the annuity formulas.
fn annuity_factor(rate: f64, nper: f64, paytype: f64) -> f64 {
    ((1.0 + rate).powf(nper) - 1.0) / rate * (1.0 + rate * paytype)
}

/// Optional positional argument as a number, with a default.
fn opt_num(args: &[CellValue], i: usize, default: f64) -> f64 {
    args.get(i)
        .and_then(|v| v.as_number().ok())
        .unwrap_or(default)
}

/// PMT(rate, nper, pv, [fv], [type]).
fn pmt(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pv) = (
        opt_num(args, 0, 0.0),
        opt_num(args, 1, 0.0),
        opt_num(args, 2, 0.0),
    );
    let fv = opt_num(args, 3, 0.0);
    let paytype = opt_num(args, 4, 0.0);
    if nper == 0.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let value = if rate == 0.0 {
        -(pv + fv) / nper
    } else {
        -(pv * (1.0 + rate).powf(nper) + fv) / annuity_factor(rate, nper, paytype)
    };
    CellValue::Number(value)
}

/// FV(rate, nper, pmt, [pv], [type]).
fn fv(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pmt_v) = (
        opt_num(args, 0, 0.0),
        opt_num(args, 1, 0.0),
        opt_num(args, 2, 0.0),
    );
    let pv = opt_num(args, 3, 0.0);
    let paytype = opt_num(args, 4, 0.0);
    let value = if rate == 0.0 {
        -(pv + pmt_v * nper)
    } else {
        -(pv * (1.0 + rate).powf(nper) + pmt_v * annuity_factor(rate, nper, paytype))
    };
    CellValue::Number(value)
}

/// PV(rate, nper, pmt, [fv], [type]).
fn pv(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pmt_v) = (
        opt_num(args, 0, 0.0),
        opt_num(args, 1, 0.0),
        opt_num(args, 2, 0.0),
    );
    let fv = opt_num(args, 3, 0.0);
    let paytype = opt_num(args, 4, 0.0);
    let value = if rate == 0.0 {
        -(fv + pmt_v * nper)
    } else {
        -(fv + pmt_v * annuity_factor(rate, nper, paytype)) / (1.0 + rate).powf(nper)
    };
    CellValue::Number(value)
}

/// NPER(rate, pmt, pv, [fv], [type]).
fn nper(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, pmt_v, pv) = (
        opt_num(args, 0, 0.0),
        opt_num(args, 1, 0.0),
        opt_num(args, 2, 0.0),
    );
    let fv = opt_num(args, 3, 0.0);
    let paytype = opt_num(args, 4, 0.0);
    if rate == 0.0 {
        if pmt_v == 0.0 {
            return CellValue::Error(FormulaError::DivisionByZero);
        }
        return CellValue::Number(-(pv + fv) / pmt_v);
    }
    let adj = pmt_v * (1.0 + rate * paytype);
    let num = adj - fv * rate;
    let den = adj + pv * rate;
    if den == 0.0 || num / den <= 0.0 {
        return CellValue::Error(FormulaError::NumError);
    }
    CellValue::Number((num / den).ln() / (1.0 + rate).ln())
}

/// RATE(nper, pmt, pv, [fv], [type], [guess]) -- Newton-Raphson on the annuity
/// balance equation. Returns `#NUM!` if it fails to converge.
fn rate(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (nper, pmt_v, pv) = (
        opt_num(args, 0, 0.0),
        opt_num(args, 1, 0.0),
        opt_num(args, 2, 0.0),
    );
    let fv = opt_num(args, 3, 0.0);
    let paytype = opt_num(args, 4, 0.0);
    let mut r = opt_num(args, 5, 0.1);
    // f(r) = pv*(1+r)^n + pmt*(1+r*type)*((1+r)^n-1)/r + fv
    let f = |r: f64| -> f64 {
        if r == 0.0 {
            pv + pmt_v * nper + fv
        } else {
            pv * (1.0 + r).powf(nper) + pmt_v * annuity_factor(r, nper, paytype) + fv
        }
    };
    for _ in 0..100 {
        let y = f(r);
        if y.abs() < 1e-9 {
            return CellValue::Number(r);
        }
        // Numerical derivative.
        let h = 1e-6;
        let dy = (f(r + h) - f(r - h)) / (2.0 * h);
        if dy.abs() < 1e-12 {
            break;
        }
        let next = r - y / dy;
        if !next.is_finite() {
            break;
        }
        r = next;
    }
    CellValue::Error(FormulaError::NumError)
}

/// NPV(rate, value1, value2, ...) -- values discounted from period 1.
fn npv(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let rate = match args[0].as_number() {
        Ok(r) => r,
        Err(e) => return CellValue::Error(e),
    };
    if rate == -1.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let flows = flatten_numeric(&args[1..]);
    let total = flows
        .iter()
        .enumerate()
        .map(|(i, v)| v / (1.0 + rate).powi(i as i32 + 1))
        .sum();
    CellValue::Number(total)
}

/// IRR(values, [guess]) -- Newton-Raphson on the NPV equation. Period-0 cash
/// flow is undiscounted; `#NUM!` if it fails to converge.
fn irr(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flows = flatten_numeric(&args[..1]);
    if flows.len() < 2 {
        return CellValue::Error(FormulaError::NumError);
    }
    let mut r = args.get(1).and_then(|v| v.as_number().ok()).unwrap_or(0.1);
    let npv_at = |r: f64| -> f64 {
        flows
            .iter()
            .enumerate()
            .map(|(i, v)| v / (1.0 + r).powi(i as i32))
            .sum()
    };
    for _ in 0..100 {
        let y = npv_at(r);
        if y.abs() < 1e-9 {
            return CellValue::Number(r);
        }
        let h = 1e-6;
        let dy = (npv_at(r + h) - npv_at(r - h)) / (2.0 * h);
        if dy.abs() < 1e-12 {
            break;
        }
        let next = r - y / dy;
        if !next.is_finite() || next <= -1.0 {
            break;
        }
        r = next;
    }
    CellValue::Error(FormulaError::NumError)
}

// -----------------------------------------------------------------------------
// Logic / information (cont.)
// -----------------------------------------------------------------------------

fn is_text(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Text(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

fn is_error(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Error(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

fn is_na(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Error(FormulaError::NaError))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

fn is_even(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Boolean((n.trunc() as i64).rem_euclid(2) == 0),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

fn is_odd(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Boolean((n.trunc() as i64).rem_euclid(2) == 1),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// IFNA(value, fallback) -- `fallback` only when `value` is the `#N/A` error.
fn ifna(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match &args[0] {
        CellValue::Error(FormulaError::NaError) => args[1].clone(),
        other => other.clone(),
    }
}

/// IFS(cond1, val1, cond2, val2, ...) -- first truthy condition wins.
fn ifs(args: &[CellValue]) -> CellValue {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return CellValue::Error(FormulaError::NaError);
    }
    for pair in args.chunks_exact(2) {
        if is_truthy(&pair[0]) {
            return pair[1].clone();
        }
    }
    CellValue::Error(FormulaError::NaError)
}

/// SWITCH(expr, val1, res1, [val2, res2, ...], [default]).
fn switch_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let subject = &args[0];
    let rest = &args[1..];
    let mut i = 0;
    while i + 1 < rest.len() {
        if values_equal_ci(subject, &rest[i]) {
            return rest[i + 1].clone();
        }
        i += 2;
    }
    // A trailing unpaired argument is the default.
    if !rest.len().is_multiple_of(2) {
        return rest[rest.len() - 1].clone();
    }
    CellValue::Error(FormulaError::NaError)
}

// -----------------------------------------------------------------------------
// Lookup / reference (cont.)
// -----------------------------------------------------------------------------

/// HLOOKUP(lookup, table, row_index, [range_lookup]) -- horizontal VLOOKUP.
fn hlookup(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let table = match &args[1] {
        CellValue::Range(r) => r,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    let row_index = match args[2].as_number() {
        Ok(n) if n >= 1.0 => n as usize - 1,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    if row_index >= table.rows {
        return CellValue::Error(FormulaError::NaError);
    }
    let approximate = args.get(3).map(is_truthy).unwrap_or(true);
    let mut best: Option<usize> = None;
    for col in 0..table.cols {
        let key = match table.get(0, col) {
            Some(k) => k,
            None => continue,
        };
        if approximate {
            if compare_cells(key, &args[0])
                .map(|o| o.is_le())
                .unwrap_or(false)
            {
                best = Some(col);
            } else {
                break;
            }
        } else if values_equal_ci(key, &args[0]) {
            best = Some(col);
            break;
        }
    }
    match best.and_then(|col| table.get(row_index, col)) {
        Some(v) => v.clone(),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// CHOOSE(index, value1, value2, ...) -- 1-based selection.
fn choose(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match args[0].as_number() {
        Ok(n) if n >= 1.0 && (n as usize) < args.len() => args[n as usize].clone(),
        Ok(_) => CellValue::Error(FormulaError::NumError),
        Err(e) => CellValue::Error(e),
    }
}

/// Dimensions of the first argument: a `Range` yields `(rows, cols)`, a scalar
/// behaves like a 1x1 range.
fn range_dims(args: &[CellValue]) -> Option<(usize, usize)> {
    match args.first()? {
        CellValue::Range(r) => Some((r.rows, r.cols)),
        _ => Some((1, 1)),
    }
}

/// ROW()/COLUMN() -- only the no-argument form is supported (the evaluating
/// cell's 1-based position). The reference form (`ROW(A1)`) needs
/// reference-preserving compilation and is rejected with `#N/A` for now.
fn row_col(args: &[CellValue], current: u32) -> CellValue {
    if !args.is_empty() || current == 0 {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Number(current as f64)
}

// -----------------------------------------------------------------------------
// Text (cont.)
// -----------------------------------------------------------------------------

/// SEARCH(find_text, within_text, [start]) -- case-insensitive `FIND`.
fn search_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let lowered = vec![
        CellValue::Text(args[0].to_display().to_lowercase()),
        CellValue::Text(args[1].to_display().to_lowercase()),
    ];
    let mut call: Vec<CellValue> = lowered;
    if let Some(start) = args.get(2) {
        call.push(start.clone());
    }
    find_fn(&call)
}

/// REPLACE(old_text, start_num, num_chars, new_text) -- positional replacement.
fn replace_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 4 {
        return CellValue::Error(FormulaError::NaError);
    }
    let chars: Vec<char> = args[0].to_display().chars().collect();
    let start = match args[1].as_number() {
        Ok(n) if n >= 1.0 => n as usize - 1,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let count = match args[2].as_number() {
        Ok(n) if n >= 0.0 => n as usize,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let new = args[3].to_display();
    let start = start.min(chars.len());
    let end = (start + count).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.push_str(&new);
    out.extend(&chars[end..]);
    CellValue::Text(out)
}

/// EXACT(text1, text2) -- case-sensitive equality.
fn exact(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Boolean(args[0].to_display() == args[1].to_display())
}

/// TEXTJOIN(delimiter, ignore_empty, text1, ...) -- join with a separator.
fn textjoin(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let delim = args[0].to_display();
    let ignore_empty = is_truthy(&args[1]);
    let parts: Vec<String> = flatten_all(&args[2..])
        .iter()
        .filter(|v| !(ignore_empty && matches!(v, CellValue::Empty)))
        .map(|v| v.to_display())
        .filter(|s| !(ignore_empty && s.is_empty()))
        .collect();
    CellValue::Text(parts.join(&delim))
}

/// CHAR(number) -- the character for a Unicode code point (1..=0x10FFFF).
fn char_fn(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) if n >= 1.0 => match char::from_u32(n as u32) {
            Some(c) => CellValue::Text(c.to_string()),
            None => CellValue::Error(FormulaError::NumError),
        },
        Some(_) => CellValue::Error(FormulaError::NumError),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// CODE(text) -- the code point of the first character.
fn code_fn(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match args[0].to_display().chars().next() {
        Some(c) => CellValue::Number(c as u32 as f64),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

// -----------------------------------------------------------------------------
// Date / time (cont.)
// -----------------------------------------------------------------------------

/// TIME(hour, minute, second) -- fraction of a day (0..1).
fn time_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let secs = match (
        args[0].as_number(),
        args[1].as_number(),
        args[2].as_number(),
    ) {
        (Ok(h), Ok(m), Ok(s)) => h * 3600.0 + m * 60.0 + s,
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    // Excel wraps to a 24h fraction.
    CellValue::Number((secs / 86_400.0).rem_euclid(1.0))
}

/// HOUR/MINUTE/SECOND -- `which` selects 0=hour, 1=minute, 2=second.
fn time_part(args: &[CellValue], which: u8) -> CellValue {
    let serial = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let frac = serial - serial.floor();
    let total = (frac * 86_400.0).round() as i64;
    let value = match which {
        0 => (total / 3600) % 24,
        1 => (total / 60) % 60,
        _ => total % 60,
    };
    CellValue::Number(value as f64)
}

/// WEEKDAY(serial, [type]) -- type 1 (default) Sun=1..Sat=7; 2 Mon=1..Sun=7;
/// 3 Mon=0..Sun=6.
fn weekday(args: &[CellValue]) -> CellValue {
    let serial = match first_num(args) {
        Some(n) => n.floor() as i64,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    // Excel serial 0 (1899-12-30) is a Saturday.
    let sat0 = serial.rem_euclid(7); // 0=Sat,1=Sun,..6=Fri
    let wtype = args
        .get(1)
        .and_then(|v| v.as_number().ok())
        .map(|n| n as i64)
        .unwrap_or(1);
    let value = match wtype {
        2 => {
            // Mon=1..Sun=7
            let mon0 = (sat0 + 5).rem_euclid(7); // 0=Mon..6=Sun
            mon0 + 1
        }
        3 => (sat0 + 5).rem_euclid(7), // Mon=0..Sun=6
        _ => {
            // type 1: Sun=1..Sat=7
            if sat0 == 0 { 7 } else { sat0 }
        }
    };
    CellValue::Number(value as f64)
}

/// Number of days in month `m` of year `y`.
fn days_in_month(y: i64, m: i64) -> i64 {
    serial_from_ymd(y, m + 1, 1) - serial_from_ymd(y, m, 1)
}

/// Shift `(year, month)` by `months`, normalizing into a valid month.
fn shift_month(y: i64, m: i64, months: i64) -> (i64, i64) {
    let total = (m - 1) + months;
    (y + total.div_euclid(12), total.rem_euclid(12) + 1)
}

/// EDATE(start, months) -- same day-of-month `months` away (clamped to the
/// target month length).
fn edate(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (serial, months) = match (args[0].as_number(), args[1].as_number()) {
        (Ok(s), Ok(m)) => (s.floor() as i64, m as i64),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    let (y, m, d) = ymd_from_serial(serial);
    let (ny, nm) = shift_month(y, m, months);
    let nd = d.min(days_in_month(ny, nm));
    CellValue::Number(serial_from_ymd(ny, nm, nd) as f64)
}

/// EOMONTH(start, months) -- last day of the month `months` away.
fn eomonth(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (serial, months) = match (args[0].as_number(), args[1].as_number()) {
        (Ok(s), Ok(m)) => (s.floor() as i64, m as i64),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    let (y, m, _) = ymd_from_serial(serial);
    let (ny, nm) = shift_month(y, m, months);
    CellValue::Number(serial_from_ymd(ny, nm, days_in_month(ny, nm)) as f64)
}

#[cfg(test)]
mod tests {
    use super::super::types::RangeValue;
    use super::*;

    #[test]
    fn sum_basic() {
        assert_eq!(
            sum(&[
                CellValue::Number(1.0),
                CellValue::Number(2.0),
                CellValue::Number(3.0),
            ])
            .to_display(),
            "6"
        );
    }

    #[test]
    fn sum_empty() {
        assert_eq!(sum(&[]).to_display(), "0");
    }

    #[test]
    fn average_basic() {
        assert_eq!(
            average(&[CellValue::Number(10.0), CellValue::Number(20.0)]).to_display(),
            "15"
        );
    }

    #[test]
    fn min_max_basic() {
        assert_eq!(
            min(&[
                CellValue::Number(5.0),
                CellValue::Number(3.0),
                CellValue::Number(9.0),
            ])
            .to_display(),
            "3"
        );
        assert_eq!(
            max(&[
                CellValue::Number(5.0),
                CellValue::Number(3.0),
                CellValue::Number(9.0),
            ])
            .to_display(),
            "9"
        );
    }

    #[test]
    fn count_functions() {
        assert_eq!(
            count(&[
                CellValue::Number(1.0),
                CellValue::Text("x".into()),
                CellValue::Number(2.0),
                CellValue::Empty,
            ])
            .to_display(),
            "2" // only 1 and 2 are numeric
        );
        assert_eq!(
            counta(&[
                CellValue::Number(1.0),
                CellValue::Text("x".into()),
                CellValue::Empty,
            ])
            .to_display(),
            "2" // 1 and "x" are non-empty
        );
    }

    #[test]
    fn if_simple() {
        assert_eq!(
            if_fn(&[
                CellValue::Boolean(true),
                CellValue::Text("yes".into()),
                CellValue::Text("no".into()),
            ])
            .to_display(),
            "yes"
        );
        assert_eq!(
            if_fn(&[
                CellValue::Boolean(false),
                CellValue::Text("yes".into()),
                CellValue::Text("no".into()),
            ])
            .to_display(),
            "no"
        );
    }

    #[test]
    fn text_functions() {
        assert_eq!(
            concat_fn(&[
                CellValue::Text("hello".into()),
                CellValue::Text(" ".into()),
                CellValue::Text("world".into()),
            ])
            .to_display(),
            "hello world"
        );

        assert_eq!(left(&[CellValue::Text("hello".into())]).to_display(), "h");
        assert_eq!(
            left(&[CellValue::Text("hello".into()), CellValue::Number(3.0)]).to_display(),
            "hel"
        );

        assert_eq!(right(&[CellValue::Text("hello".into())]).to_display(), "o");
        assert_eq!(
            mid(&[
                CellValue::Text("hello".into()),
                CellValue::Number(2.0),
                CellValue::Number(3.0),
            ])
            .to_display(),
            "ell"
        );

        assert_eq!(
            upper(&[CellValue::Text("hello".into())]).to_display(),
            "HELLO"
        );
        assert_eq!(
            lower(&[CellValue::Text("HELLO".into())]).to_display(),
            "hello"
        );
        assert_eq!(trim(&[CellValue::Text("  hi  ".into())]).to_display(), "hi");
        assert_eq!(len(&[CellValue::Text("hello".into())]).to_display(), "5");
    }

    // -- Phase 2 helpers -------------------------------------------------------

    fn num(n: f64) -> CellValue {
        CellValue::Number(n)
    }
    fn txt(s: &str) -> CellValue {
        CellValue::Text(s.into())
    }
    fn range(rows: usize, cols: usize, vals: Vec<CellValue>) -> CellValue {
        CellValue::Range(RangeValue::new(rows, cols, vals))
    }

    #[test]
    fn iferror_passthrough_and_fallback() {
        assert_eq!(iferror(&[num(5.0), txt("fb")]).to_display(), "5");
        assert_eq!(
            iferror(&[CellValue::Error(FormulaError::DivisionByZero), txt("fb")]).to_display(),
            "fb"
        );
    }

    #[test]
    fn countif_and_sumif() {
        // range 1..=5 in a single column.
        let r = range(5, 1, (1..=5).map(|n| num(n as f64)).collect());
        assert_eq!(countif(&[r.clone(), txt(">2")]).to_display(), "3");
        assert_eq!(countif(&[r.clone(), num(4.0)]).to_display(), "1");
        // Sum of values > 2 -> 3+4+5 = 12.
        assert_eq!(sumif(&[r.clone(), txt(">2")]).to_display(), "12");
        // Paired sum_range: weights doubled.
        let w = range(5, 1, (1..=5).map(|n| num((n * 10) as f64)).collect());
        assert_eq!(sumif(&[r, txt(">=4"), w]).to_display(), "90"); // 40+50
    }

    #[test]
    fn countif_text() {
        let r = range(3, 1, vec![txt("apple"), txt("Banana"), txt("apple")]);
        assert_eq!(countif(&[r.clone(), txt("apple")]).to_display(), "2");
        assert_eq!(countif(&[r, txt("<>apple")]).to_display(), "1");
    }

    #[test]
    fn vlookup_exact_and_approx() {
        // table: key | label
        let table = range(
            3,
            2,
            vec![
                num(1.0),
                txt("one"),
                num(3.0),
                txt("three"),
                num(5.0),
                txt("five"),
            ],
        );
        // Exact match.
        assert_eq!(
            vlookup(&[num(3.0), table.clone(), num(2.0), CellValue::Boolean(false)]).to_display(),
            "three"
        );
        // Exact miss.
        assert!(matches!(
            vlookup(&[num(4.0), table.clone(), num(2.0), CellValue::Boolean(false)]),
            CellValue::Error(FormulaError::NaError)
        ));
        // Approximate: 4 falls back to key 3.
        assert_eq!(
            vlookup(&[num(4.0), table, num(2.0), CellValue::Boolean(true)]).to_display(),
            "three"
        );
    }

    #[test]
    fn index_and_match() {
        let row = range(1, 3, vec![txt("a"), txt("b"), txt("c")]);
        assert_eq!(index(&[row.clone(), num(2.0)]).to_display(), "b");
        let col = range(3, 1, vec![num(10.0), num(20.0), num(30.0)]);
        assert_eq!(index(&[col.clone(), num(3.0)]).to_display(), "30");
        let grid = range(2, 2, vec![num(1.0), num(2.0), num(3.0), num(4.0)]);
        assert_eq!(index(&[grid, num(2.0), num(2.0)]).to_display(), "4");

        // MATCH exact and approximate.
        assert_eq!(
            match_fn(&[num(20.0), col.clone(), num(0.0)]).to_display(),
            "2"
        );
        assert_eq!(match_fn(&[num(25.0), col, num(1.0)]).to_display(), "2");
    }

    #[test]
    fn text_formatting() {
        assert_eq!(text(&[num(12.12), txt("0.00")]).to_display(), "12.12");
        assert_eq!(text(&[num(1234.5), txt("#,##0.0")]).to_display(), "1,234.5");
        assert_eq!(text(&[num(42.0), txt("0")]).to_display(), "42");
    }

    #[test]
    fn date_roundtrip() {
        // 2024-06-13 -> known serial, then decomposed back.
        let serial = match date(&[num(2024.0), num(6.0), num(13.0)]) {
            CellValue::Number(n) => n,
            other => panic!("expected number, got {other:?}"),
        };
        assert_eq!(year(&[num(serial)]).to_display(), "2024");
        assert_eq!(month(&[num(serial)]).to_display(), "6");
        assert_eq!(day(&[num(serial)]).to_display(), "13");
        // 1900-01-01 is Excel serial 2 with the 1899-12-30 epoch.
        assert_eq!(date(&[num(1900.0), num(1.0), num(1.0)]).to_display(), "2");
    }

    #[test]
    fn date_month_overflow() {
        // Month 13 rolls into the next year.
        let s1 = date(&[num(2024.0), num(13.0), num(1.0)]).to_display();
        let s2 = date(&[num(2025.0), num(1.0), num(1.0)]).to_display();
        assert_eq!(s1, s2);
    }

    #[test]
    fn math_functions() {
        assert_eq!(abs(&[CellValue::Number(-5.0)]).to_display(), "5");
        assert_eq!(
            round(&[
                CellValue::Number(std::f64::consts::PI),
                CellValue::Number(2.0)
            ])
            .to_display(),
            "3.14"
        );
        assert_eq!(sqrt(&[CellValue::Number(16.0)]).to_display(), "4");
        assert_eq!(
            power(&[CellValue::Number(2.0), CellValue::Number(3.0)]).to_display(),
            "8"
        );
    }

    #[test]
    fn extra_math_functions() {
        assert_eq!(int_fn(&[num(-2.5)]).to_display(), "-3");
        assert_eq!(mod_fn(&[num(7.0), num(3.0)]).to_display(), "1");
        // Excel MOD takes the sign of the divisor.
        assert_eq!(mod_fn(&[num(-1.0), num(3.0)]).to_display(), "2");
        assert!(matches!(
            mod_fn(&[num(1.0), num(0.0)]),
            CellValue::Error(FormulaError::DivisionByZero)
        ));
        assert_eq!(
            round_dir(&[num(2.001), num(2.0)], true).to_display(),
            "2.01"
        );
        assert_eq!(round_dir(&[num(2.009), num(2.0)], false).to_display(), "2");
        assert_eq!(
            round_dir(&[num(-2.001), num(2.0)], true).to_display(),
            "-2.01"
        );
        assert_eq!(ceil_floor(&[num(4.3), num(1.0)], true).to_display(), "5");
        assert_eq!(ceil_floor(&[num(4.7), num(1.0)], false).to_display(), "4");
        assert_eq!(product(&[num(2.0), num(3.0), num(4.0)]).to_display(), "24");
        assert_eq!(median(&[num(3.0), num(1.0), num(2.0)]).to_display(), "2");
        assert_eq!(
            median(&[num(1.0), num(2.0), num(3.0), num(4.0)]).to_display(),
            "2.5"
        );
    }

    #[test]
    fn sumproduct_and_countblank() {
        let a = range(3, 1, vec![num(1.0), num(2.0), num(3.0)]);
        let b = range(3, 1, vec![num(4.0), num(5.0), num(6.0)]);
        // 1*4 + 2*5 + 3*6 = 32.
        assert_eq!(sumproduct(&[a, b]).to_display(), "32");
        let r = range(
            4,
            1,
            vec![num(1.0), CellValue::Empty, txt("x"), CellValue::Empty],
        );
        assert_eq!(countblank(&[r]).to_display(), "2");
    }

    #[test]
    fn extra_logic_functions() {
        assert_eq!(
            xor(&[CellValue::Boolean(true), CellValue::Boolean(false)]).to_display(),
            "true"
        );
        assert_eq!(
            xor(&[CellValue::Boolean(true), CellValue::Boolean(true)]).to_display(),
            "false"
        );
        assert_eq!(is_blank(&[CellValue::Empty]).to_display(), "true");
        assert_eq!(is_blank(&[num(0.0)]).to_display(), "false");
        assert_eq!(is_number(&[num(1.0)]).to_display(), "true");
        assert_eq!(is_number(&[txt("1")]).to_display(), "false");
    }

    #[test]
    fn extra_text_functions() {
        assert_eq!(proper(&[txt("hELLO wORLD")]).to_display(), "Hello World");
        assert_eq!(
            substitute(&[txt("a-b-c"), txt("-"), txt("+")]).to_display(),
            "a+b+c"
        );
        assert_eq!(
            substitute(&[txt("a-b-c"), txt("-"), txt("+"), num(2.0)]).to_display(),
            "a-b+c"
        );
        assert_eq!(rept(&[txt("ab"), num(3.0)]).to_display(), "ababab");
        assert_eq!(find_fn(&[txt("c"), txt("abcabc")]).to_display(), "3");
        assert_eq!(
            find_fn(&[txt("c"), txt("abcabc"), num(4.0)]).to_display(),
            "6"
        );
        assert!(matches!(
            find_fn(&[txt("z"), txt("abc")]),
            CellValue::Error(FormulaError::TypeError)
        ));
        assert_eq!(value_fn(&[txt("42")]).to_display(), "42");
    }

    fn approx(v: CellValue, expected: f64) {
        match v {
            CellValue::Number(n) => {
                assert!((n - expected).abs() < 1e-4, "expected {expected}, got {n}")
            }
            other => panic!("expected number, got {other:?}"),
        }
    }

    #[test]
    fn more_math_functions() {
        assert_eq!(trunc(&[num(8.9)]).to_display(), "8");
        assert_eq!(trunc(&[num(-8.9)]).to_display(), "-8");
        assert_eq!(trunc(&[num(1.14159), num(2.0)]).to_display(), "1.14");
        assert_eq!(sign(&[num(-3.0)]).to_display(), "-1");
        assert_eq!(sign(&[num(0.0)]).to_display(), "0");
        assert_eq!(sumsq(&[num(3.0), num(4.0)]).to_display(), "25");
        approx(unary_math(&[num(0.0)], f64::exp), 1.0);
        approx(log_checked(&[num(std::f64::consts::E)], |n| n.ln()), 1.0);
        approx(log_fn(&[num(8.0), num(2.0)]), 3.0);
        assert!(matches!(
            log_checked(&[num(-1.0)], |n| n.ln()),
            CellValue::Error(FormulaError::NumError)
        ));
        approx(
            unary_math(&[num(180.0)], f64::to_radians),
            std::f64::consts::PI,
        );
    }

    #[test]
    fn stats_functions() {
        let r = range(5, 1, (1..=5).map(|n| num(n as f64)).collect());
        approx(variance(std::slice::from_ref(&r), true), 2.5);
        approx(stdev(std::slice::from_ref(&r), true), 2.5f64.sqrt());
        assert_eq!(large_small(&[r.clone(), num(2.0)], true).to_display(), "4");
        assert_eq!(large_small(&[r.clone(), num(2.0)], false).to_display(), "2");
        // RANK of 4 within 1..5 descending = 2.
        assert_eq!(rank(&[num(4.0), r.clone()]).to_display(), "2");
        assert_eq!(
            rank(&[num(4.0), r, CellValue::Boolean(true)]).to_display(),
            "4"
        );
        let m = range(5, 1, vec![num(2.0), num(3.0), num(3.0), num(5.0), num(3.0)]);
        assert_eq!(mode(&[m]).to_display(), "3");
    }

    #[test]
    fn conditional_aggregation() {
        // category | amount
        let cat = range(4, 1, vec![txt("a"), txt("b"), txt("a"), txt("a")]);
        let amt = range(4, 1, vec![num(10.0), num(20.0), num(30.0), num(40.0)]);
        assert_eq!(
            sumifs(&[amt.clone(), cat.clone(), txt("a")]).to_display(),
            "80"
        );
        assert_eq!(countifs(&[cat.clone(), txt("a")]).to_display(), "3");
        assert_eq!(
            averageif(&[cat.clone(), txt("a"), amt.clone()]).to_display(),
            "26.666666666666668"
        );
        // Multi-criteria: category a AND amount > 15 -> 30 + 40 = 70.
        assert_eq!(
            sumifs(&[amt.clone(), cat.clone(), txt("a"), amt, txt(">15")]).to_display(),
            "70"
        );
    }

    #[test]
    fn financial_functions() {
        // Loan: 5%/yr over 10 periods, PV = 1000, FV = 0, end-of-period.
        approx(pmt(&[num(0.05), num(10.0), num(1000.0)]), -129.504_57);
        // FV of that payment stream should bring the balance to ~0.
        approx(
            fv(&[num(0.05), num(10.0), num(-129.504_57), num(1000.0)]),
            0.0,
        );
        // PV inverse.
        approx(pv(&[num(0.05), num(10.0), num(-129.504_57)]), 1000.0);
        // NPER inverse (~10 periods).
        approx(nper(&[num(0.05), num(-129.504_57), num(1000.0)]), 10.0);
        // RATE recovers 5%.
        approx(rate(&[num(10.0), num(-129.504_57), num(1000.0)]), 0.05);
        // NPV / IRR.
        approx(npv(&[num(0.1), num(100.0), num(200.0)]), 256.198_3);
        approx(
            irr(&[range(3, 1, vec![num(-100.0), num(60.0), num(60.0)])]),
            0.130_662,
        );
    }

    #[test]
    fn logic_info_functions() {
        assert_eq!(is_text(&[txt("x")]).to_display(), "true");
        assert_eq!(
            is_error(&[CellValue::Error(FormulaError::NaError)]).to_display(),
            "true"
        );
        assert_eq!(
            is_na(&[CellValue::Error(FormulaError::NaError)]).to_display(),
            "true"
        );
        assert_eq!(
            is_na(&[CellValue::Error(FormulaError::DivisionByZero)]).to_display(),
            "false"
        );
        assert_eq!(is_even(&[num(4.0)]).to_display(), "true");
        assert_eq!(is_odd(&[num(3.0)]).to_display(), "true");
        assert_eq!(
            ifna(&[CellValue::Error(FormulaError::NaError), txt("fb")]).to_display(),
            "fb"
        );
        assert_eq!(
            ifs(&[
                CellValue::Boolean(false),
                txt("a"),
                CellValue::Boolean(true),
                txt("b")
            ])
            .to_display(),
            "b"
        );
        assert_eq!(
            switch_fn(&[num(2.0), num(1.0), txt("one"), num(2.0), txt("two")]).to_display(),
            "two"
        );
        assert_eq!(
            switch_fn(&[num(9.0), num(1.0), txt("one"), txt("default")]).to_display(),
            "default"
        );
    }

    #[test]
    fn lookup_reference_functions() {
        // HLOOKUP: keys in the first row, return the 2nd row.
        let table = range(
            2,
            3,
            vec![num(1.0), num(3.0), num(5.0), txt("a"), txt("b"), txt("c")],
        );
        assert_eq!(
            hlookup(&[num(3.0), table.clone(), num(2.0), CellValue::Boolean(false)]).to_display(),
            "b"
        );
        assert_eq!(
            choose(&[num(2.0), txt("x"), txt("y"), txt("z")]).to_display(),
            "y"
        );
        let r = range(3, 4, vec![CellValue::Empty; 12]);
        assert_eq!(
            call_builtin("ROWS", std::slice::from_ref(&r), FnCtx::at_clock(0.0)).to_display(),
            "3"
        );
        assert_eq!(
            call_builtin("COLUMNS", &[r], FnCtx::at_clock(0.0)).to_display(),
            "4"
        );
        // ROW()/COLUMN() with the evaluating cell at (row 7, col 2).
        let ctx = FnCtx {
            now_serial: 0.0,
            cell_row: 7,
            cell_col: 2,
        };
        assert_eq!(call_builtin("ROW", &[], ctx).to_display(), "7");
        assert_eq!(call_builtin("COLUMN", &[], ctx).to_display(), "2");
        // The reference form is deferred -> #N/A.
        assert!(matches!(
            row_col(&[num(1.0)], 7),
            CellValue::Error(FormulaError::NaError)
        ));
    }

    #[test]
    fn more_text_functions() {
        assert_eq!(search_fn(&[txt("C"), txt("abcabc")]).to_display(), "3");
        assert_eq!(
            replace_fn(&[txt("abcdef"), num(2.0), num(3.0), txt("XY")]).to_display(),
            "aXYef"
        );
        assert_eq!(exact(&[txt("abc"), txt("abc")]).to_display(), "true");
        assert_eq!(exact(&[txt("abc"), txt("Abc")]).to_display(), "false");
        assert_eq!(
            textjoin(&[
                txt("-"),
                CellValue::Boolean(true),
                txt("a"),
                CellValue::Empty,
                txt("b")
            ])
            .to_display(),
            "a-b"
        );
        assert_eq!(char_fn(&[num(65.0)]).to_display(), "A");
        assert_eq!(code_fn(&[txt("A")]).to_display(), "65");
    }

    #[test]
    fn datetime_functions() {
        // 06:00:00 -> a quarter of a day.
        approx(time_fn(&[num(6.0), num(0.0), num(0.0)]), 0.25);
        let t = match time_fn(&[num(13.0), num(30.0), num(15.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(time_part(&[num(t)], 0).to_display(), "13");
        assert_eq!(time_part(&[num(t)], 1).to_display(), "30");
        assert_eq!(time_part(&[num(t)], 2).to_display(), "15");
        // 2024-06-13 was a Thursday -> type 1 gives 5 (Sun=1).
        let serial = match date(&[num(2024.0), num(6.0), num(13.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(weekday(&[num(serial)]).to_display(), "5");
        assert_eq!(weekday(&[num(serial), num(2.0)]).to_display(), "4"); // Mon=1
        // EDATE +1 month -> 2024-07-13; EOMONTH +0 -> 2024-06-30.
        let edate_s = match edate(&[num(serial), num(1.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(month(&[num(edate_s)]).to_display(), "7");
        assert_eq!(day(&[num(edate_s)]).to_display(), "13");
        let eom = match eomonth(&[num(serial), num(0.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(day(&[num(eom)]).to_display(), "30");
        // EDATE day clamping: Jan 31 + 1 month -> Feb 29 (2024 leap year).
        let jan31 = match date(&[num(2024.0), num(1.0), num(31.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        let feb = match edate(&[num(jan31), num(1.0)]) {
            CellValue::Number(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(month(&[num(feb)]).to_display(), "2");
        assert_eq!(day(&[num(feb)]).to_display(), "29");
    }
}
