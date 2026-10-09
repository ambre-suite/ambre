// ambre-trame/src/formula/functions/builtins_b.rs
// Spreadsheet builtin implementations dispatched by `call_builtin`. Cross-refs
// resolve through the module glob mesh.

use super::*;

/// PROPER(text) -- capitalize the first letter of each word, lowercase the rest.
pub(crate) fn proper(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let mut out = String::new();
    let mut at_word_start = true;
    for c in arg(args, 0).to_display().chars() {
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
pub(crate) fn substitute(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let text = arg(args, 0).to_display();
    let old = arg(args, 1).to_display();
    let new = arg(args, 2).to_display();
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
pub(crate) fn rept(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let n = match arg(args, 1).as_number() {
        Ok(n) if n >= 0.0 => n as usize,
        Ok(_) => return CellValue::Error(FormulaError::NumError),
        Err(e) => return CellValue::Error(e),
    };
    CellValue::Text(arg(args, 0).to_display().repeat(n))
}

/// FIND(find_text, within_text, [start]) -- 1-based, case-sensitive position of
/// `find_text` in `within_text`. Returns `#VALUE!` (TypeError) when not found.
pub(crate) fn find_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let needle = arg(args, 0).to_display();
    let haystack: Vec<char> = arg(args, 1).to_display().chars().collect();
    let start = args
        .get(2)
        .and_then(|v| v.as_number().ok())
        .map(|n| (n as usize).max(1) - 1)
        .unwrap_or(0);
    // Char-aware search from `start`: rebuild the searchable suffix as a string.
    let Some(tail) = haystack.get(start..) else {
        return CellValue::Error(FormulaError::TypeError);
    };
    let suffix: String = tail.iter().collect();
    match suffix.find(&needle) {
        // `find` yields a byte offset; convert it to a char offset.
        Some(byte_pos) => {
            let char_off = suffix.get(..byte_pos).unwrap_or_default().chars().count();
            CellValue::Number((start + char_off + 1) as f64)
        }
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// VALUE(text) -- coerce a textual number into a numeric value.
pub(crate) fn value_fn(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, 0).as_number() {
        Ok(n) => CellValue::Number(n),
        Err(_) => CellValue::Error(FormulaError::TypeError),
    }
}

/// TEXT(value, format) -- minimal numeric formatting.
///
/// Supports the common Excel patterns. A run of `0`/`#` with an optional `.`
/// decimal section controls the number of decimals. A leading `,` in the
/// integer section turns on the thousands separator. Non-numeric values are
/// returned via their display form. Full Excel format-code parsing is out of
/// scope.
pub(crate) fn text(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let fmt = arg(args, 1).to_display();
    let num = match arg(args, 0).as_number() {
        Ok(n) => n,
        Err(_) => return CellValue::Text(arg(args, 0).to_display()),
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
pub(crate) fn group_thousands(s: &str) -> String {
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

/// COUNTIF(range, criterion) -- count cells in `range` matching `criterion`.
pub(crate) fn countif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(arg(args, 1));
    let n = range_cells(arg(args, 0))
        .filter(|v| crit.matches(v))
        .count();
    CellValue::Number(n as f64)
}

/// SUMIF(range, criterion, [sum_range]) -- sum cells matching `criterion`.
///
/// When `sum_range` is omitted, the criterion range is summed. Otherwise the
/// two ranges are paired by position: Excel anchors `sum_range` at its top-left
/// and walks it in parallel with `range`.
pub(crate) fn sumif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(arg(args, 1));
    let test: Vec<CellValue> = range_cells(arg(args, 0)).cloned().collect();
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

/// IFERROR(value, fallback) -- return `fallback` when `value` is an error.
pub(crate) fn iferror(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, 0) {
        CellValue::Error(_) => arg(args, 1).clone(),
        other => other.clone(),
    }
}

/// VLOOKUP(lookup, table, col_index, [range_lookup]).
///
/// Searches the first column of `table` and returns the value in column
/// `col_index` (1-based) of the matching row. `range_lookup` defaults to TRUE
/// (approximate match on an ascending first column); FALSE forces an exact
/// match.
pub(crate) fn vlookup(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let table = match arg(args, 1) {
        CellValue::Range(r) => r,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    let col_index = match arg(args, 2).as_number() {
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
            if compare_cells(key, arg(args, 0))
                .map(|o| o.is_le())
                .unwrap_or(false)
            {
                best = Some(row);
            } else {
                break;
            }
        } else if values_equal_ci(key, arg(args, 0)) {
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
pub(crate) fn index(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let range = match arg(args, 0) {
        CellValue::Range(r) => r,
        // A scalar behaves like a 1x1 range.
        other => {
            return if to_index(arg(args, 1)) == Some(1) {
                other.clone()
            } else {
                CellValue::Error(FormulaError::NaError)
            };
        }
    };
    let n1 = match to_index(arg(args, 1)) {
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
pub(crate) fn match_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let cells: Vec<&CellValue> = range_cells(arg(args, 1)).collect();
    let match_type = args
        .get(2)
        .and_then(|v| v.as_number().ok())
        .map(|n| n as i64)
        .unwrap_or(1);

    if match_type == 0 {
        for (i, c) in cells.iter().enumerate() {
            if values_equal_ci(c, arg(args, 0)) {
                return CellValue::Number((i + 1) as f64);
            }
        }
        return CellValue::Error(FormulaError::NaError);
    }

    let mut best: Option<usize> = None;
    for (i, c) in cells.iter().enumerate() {
        let ord = match compare_cells(c, arg(args, 0)) {
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
pub(crate) fn to_index(value: &CellValue) -> Option<usize> {
    match value.as_number() {
        Ok(n) if n >= 1.0 => Some(n as usize),
        _ => None,
    }
}

/// Iterate the cells of an argument: a `Range` yields its elements in order, a
/// scalar yields itself once.
pub(crate) fn range_cells(value: &CellValue) -> Box<dyn Iterator<Item = &CellValue> + '_> {
    match value {
        CellValue::Range(r) => Box::new(r.values.iter()),
        other => Box::new(std::iter::once(other)),
    }
}

/// Case-insensitive equality used by lookups (numbers compared numerically).
pub(crate) fn values_equal_ci(a: &CellValue, b: &CellValue) -> bool {
    match (a.as_number(), b.as_number()) {
        (Ok(x), Ok(y)) => nearly_equal(x, y),
        _ => a.to_display().eq_ignore_ascii_case(&b.to_display()),
    }
}

/// Ordering between two cells: numeric when both coerce to numbers, otherwise a
/// case-insensitive lexical comparison.
pub(crate) fn compare_cells(a: &CellValue, b: &CellValue) -> Option<std::cmp::Ordering> {
    match (a.as_number(), b.as_number()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y),
        _ => Some(
            a.to_display()
                .to_lowercase()
                .cmp(&b.to_display().to_lowercase()),
        ),
    }
}

/// DATE(year, month, day) -> serial number. Two-digit-style years (0..1899)
/// are offset by 1900, matching Excel.
pub(crate) fn date(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (y, m, d) = match (
        arg(args, 0).as_number(),
        arg(args, 1).as_number(),
        arg(args, 2).as_number(),
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

pub(crate) fn year(args: &[CellValue]) -> CellValue {
    date_part(args, |(y, _, _)| y)
}

pub(crate) fn month(args: &[CellValue]) -> CellValue {
    date_part(args, |(_, m, _)| m)
}

pub(crate) fn day(args: &[CellValue]) -> CellValue {
    date_part(args, |(_, _, d)| d)
}

pub(crate) fn date_part(args: &[CellValue], pick: fn((i64, i64, i64)) -> i64) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, 0).as_number() {
        Ok(serial) => {
            let ymd = ymd_from_serial(serial.floor() as i64);
            CellValue::Number(pick(ymd) as f64)
        }
        Err(_) => CellValue::Error(FormulaError::TypeError),
    }
}

/// Days from the Excel epoch (1899-12-30) for a proleptic-Gregorian date.
/// Uses Howard Hinnant's `days_from_civil` algorithm against the Unix epoch.
pub(crate) fn serial_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    let yy = y - i64::from(m <= 2);
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let unix_days = era * 146_097 + doe - 719_468;
    unix_days + 25_569
}

/// Inverse of [`serial_from_ymd`]: `(year, month, day)` from an Excel serial.
pub(crate) fn ymd_from_serial(serial: i64) -> (i64, i64, i64) {
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

/// Flatten a value (including nested ranges) into a string builder.
pub(crate) fn push_display(value: &CellValue, out: &mut String) {
    match value {
        CellValue::Range(r) => {
            for v in &r.values {
                push_display(v, out);
            }
        }
        other => out.push_str(&other.to_display()),
    }
}

/// TRUNC(number, [digits]) -- truncate toward zero at `digits` decimals.
pub(crate) fn trunc(args: &[CellValue]) -> CellValue {
    let num = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let digits = if args.len() >= 2 {
        flatten_numeric(one(args, 1))
            .first()
            .copied()
            .unwrap_or(0.0) as i32
    } else {
        0
    };
    let factor = 10f64.powi(digits);
    CellValue::Number((num * factor).trunc() / factor)
}

/// SIGN(number) -- -1, 0 or 1.
pub(crate) fn sign(args: &[CellValue]) -> CellValue {
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
pub(crate) fn sumsq(args: &[CellValue]) -> CellValue {
    match flatten_total(args) {
        Ok(nums) => CellValue::Number(nums.iter().map(|n| n * n).sum()),
        Err(err) => err,
    }
}

/// Apply a total `f64 -> f64` function to the single numeric argument.
pub(crate) fn unary_math(args: &[CellValue], f: fn(f64) -> f64) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Number(f(n)),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// Apply a logarithm to the single argument, rejecting non-positive inputs.
pub(crate) fn log_checked(args: &[CellValue], f: fn(f64) -> f64) -> CellValue {
    match first_num(args) {
        Some(n) if n > 0.0 => CellValue::Number(f(n)),
        Some(_) => CellValue::Error(FormulaError::NumError),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// LOG(number, [base=10]).
pub(crate) fn log_fn(args: &[CellValue]) -> CellValue {
    let num = match first_num(args) {
        Some(n) if n > 0.0 => n,
        Some(_) => return CellValue::Error(FormulaError::NumError),
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let base = if args.len() >= 2 {
        flatten_numeric(one(args, 1))
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

/// Build the `(values, criterion)` pairs of a `*IFS`-style argument tail.
/// Returns `None` when the tail is empty or has an odd length.
pub(crate) fn criteria_pairs(args: &[CellValue]) -> Option<Vec<(Vec<CellValue>, Criterion)>> {
    if args.is_empty() || !args.len().is_multiple_of(2) {
        return None;
    }
    Some(
        args.chunks_exact(2)
            .map(|c| {
                (
                    range_cells(arg(c, 0)).cloned().collect(),
                    Criterion::parse(arg(c, 1)),
                )
            })
            .collect(),
    )
}

/// Positions where every criterion in `pairs` matches (ranges paired by index).
pub(crate) fn matching_indices(pairs: &[(Vec<CellValue>, Criterion)]) -> Vec<usize> {
    let len = pairs.iter().map(|(v, _)| v.len()).min().unwrap_or(0);
    (0..len)
        .filter(|&i| pairs.iter().all(|(vals, crit)| crit.matches(arg(vals, i))))
        .collect()
}

/// Sum the numeric values of `src` at the given indices.
pub(crate) fn sum_at(src: &[CellValue], idx: &[usize]) -> f64 {
    idx.iter()
        .filter_map(|&i| src.get(i).and_then(|v| v.as_number().ok()))
        .sum()
}

/// AVERAGEIF(range, criterion, [average_range]).
pub(crate) fn averageif(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let crit = Criterion::parse(arg(args, 1));
    let test: Vec<CellValue> = range_cells(arg(args, 0)).cloned().collect();
    let avg_src: Vec<CellValue> = match args.get(2) {
        Some(r) => range_cells(r).cloned().collect(),
        None => test.clone(),
    };
    let idx: Vec<usize> = test
        .iter()
        .enumerate()
        .filter(|(_, v)| crit.matches(v))
        .map(|(i, _)| i)
        .collect();
    average_at(&avg_src, &idx)
}

/// AVERAGEIFS(average_range, criteria_range1, criteria1, ...).
pub(crate) fn averageifs(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let avg_src: Vec<CellValue> = range_cells(arg(args, 0)).cloned().collect();
    let Some(pairs) = criteria_pairs(rest_from(args, 1)) else {
        return CellValue::Error(FormulaError::NaError);
    };
    average_at(&avg_src, &matching_indices(&pairs))
}

/// Average the numeric values of `src` at the given indices (`#DIV/0!` if none).
pub(crate) fn average_at(src: &[CellValue], idx: &[usize]) -> CellValue {
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
pub(crate) fn sumifs(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let sum_src: Vec<CellValue> = range_cells(arg(args, 0)).cloned().collect();
    let Some(pairs) = criteria_pairs(rest_from(args, 1)) else {
        return CellValue::Error(FormulaError::NaError);
    };
    CellValue::Number(sum_at(&sum_src, &matching_indices(&pairs)))
}

/// COUNTIFS(criteria_range1, criteria1, ...).
pub(crate) fn countifs(args: &[CellValue]) -> CellValue {
    let Some(pairs) = criteria_pairs(args) else {
        return CellValue::Error(FormulaError::NaError);
    };
    CellValue::Number(matching_indices(&pairs).len() as f64)
}
