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

/// Call the named built-in function with pre-evaluated arguments.
///
/// `now_serial` is the evaluation clock (Excel serial number) shared across the
/// recomputation pass; it is only consumed by `TODAY`/`NOW`.
pub fn call_builtin(name: &str, args: &[CellValue], now_serial: f64) -> CellValue {
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

        // Logic
        "IF" => if_fn(args),
        "IFERROR" => iferror(args),
        "AND" => and(args),
        "OR" => or(args),
        "NOT" => not(args),

        // Lookup
        "VLOOKUP" => vlookup(args),
        "INDEX" => index(args),
        "MATCH" => match_fn(args),

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

        // Date / time (serial-number based, epoch 1899-12-30)
        "TODAY" => CellValue::Number(now_serial.floor()),
        "NOW" => CellValue::Number(now_serial),
        "DATE" => date(args),
        "YEAR" => year(args),
        "MONTH" => month(args),
        "DAY" => day(args),

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
    let non_empty = flat.iter().filter(|v| !matches!(v, CellValue::Empty)).count();
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
        flatten_numeric(&args[1..2])
            .first()
            .copied()
            .unwrap_or(0.0) as i32
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
        flatten_numeric(&args[1..2])
            .first()
            .copied()
            .unwrap_or(1.0) as usize
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
        flatten_numeric(&args[1..2])
            .first()
            .copied()
            .unwrap_or(1.0) as usize
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
            ">=" => (Less, true),  // not Less
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
                    if *negated {
                        !hit
                    } else {
                        hit
                    }
                }
                Err(_) => false,
            },
            Criterion::Text(ord, operand, negated) => {
                let hit = value
                    .to_display()
                    .to_lowercase()
                    .cmp(&operand.to_lowercase())
                    == *ord;
                if *negated {
                    !hit
                } else {
                    hit
                }
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
            if compare_cells(key, &args[0]).map(|o| o.is_le()).unwrap_or(false) {
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
        other => return if to_index(&args[1]) == Some(1) { other.clone() } else { CellValue::Error(FormulaError::NaError) },
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
        if self.is_nan() { default } else { CellValue::Number(self) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::RangeValue;

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
            average(&[CellValue::Number(10.0), CellValue::Number(20.0)])
                .to_display(),
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
        assert_eq!(concat_fn(&[
            CellValue::Text("hello".into()),
            CellValue::Text(" ".into()),
            CellValue::Text("world".into()),
        ]).to_display(), "hello world");

        assert_eq!(left(&[CellValue::Text("hello".into())]).to_display(), "h");
        assert_eq!(left(&[CellValue::Text("hello".into()), CellValue::Number(3.0)]).to_display(), "hel");

        assert_eq!(right(&[CellValue::Text("hello".into())]).to_display(), "o");
        assert_eq!(mid(&[
            CellValue::Text("hello".into()),
            CellValue::Number(2.0),
            CellValue::Number(3.0),
        ]).to_display(), "ell");

        assert_eq!(upper(&[CellValue::Text("hello".into())]).to_display(), "HELLO");
        assert_eq!(lower(&[CellValue::Text("HELLO".into())]).to_display(), "hello");
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
                num(1.0), txt("one"),
                num(3.0), txt("three"),
                num(5.0), txt("five"),
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
        assert_eq!(match_fn(&[num(20.0), col.clone(), num(0.0)]).to_display(), "2");
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
        assert_eq!(round(&[CellValue::Number(std::f64::consts::PI), CellValue::Number(2.0)]).to_display(), "3.14");
        assert_eq!(sqrt(&[CellValue::Number(16.0)]).to_display(), "4");
        assert_eq!(power(&[CellValue::Number(2.0), CellValue::Number(3.0)]).to_display(), "8");
    }
}
