// ambre-trame/src/formula/functions/builtins_a.rs
// Spreadsheet builtin implementations dispatched by `call_builtin`. Cross-refs
// resolve through the module glob mesh.

use super::*;

/// The argument at `i`, or an empty cell when the call passed fewer. Every
/// builtin checks its arity first, so the fallback only guards a slip there.
/// A wrong arity check answers a value instead of killing the Logic thread.
pub(crate) fn arg(args: &[CellValue], i: usize) -> &CellValue {
    static EMPTY: CellValue = CellValue::Empty;
    args.get(i).unwrap_or(&EMPTY)
}

/// The argument at `i` as a one-element slice, or an empty one when absent.
pub(crate) fn one(args: &[CellValue], i: usize) -> &[CellValue] {
    args.get(i..=i).unwrap_or_default()
}

/// The arguments from `i` on, or an empty slice when the call passed fewer.
pub(crate) fn rest_from(args: &[CellValue], i: usize) -> &[CellValue] {
    args.get(i..).unwrap_or_default()
}

/// Flatten all arguments into a single numeric iterator. Range values are
/// unpacked; non-numeric values are silently skipped.
pub(crate) fn flatten_numeric(args: &[CellValue]) -> Vec<f64> {
    let mut nums = Vec::new();
    for arg in args {
        flatten_numeric_inner(arg, &mut nums);
    }
    nums
}

pub(crate) fn flatten_numeric_inner(value: &CellValue, out: &mut Vec<f64>) {
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

/// Flatten the arguments of a total, as `flatten_numeric` does. The first error
/// met, in a range or as a scalar, is returned instead of the numbers. A total
/// that skipped it would pass a partial sum off as the sum of the range
/// (`TRAME-056`).
pub(crate) fn flatten_total(args: &[CellValue]) -> Result<Vec<f64>, CellValue> {
    fn first_error(value: &CellValue) -> Option<&CellValue> {
        match value {
            CellValue::Error(_) => Some(value),
            CellValue::Range(r) => r.values.iter().find_map(first_error),
            _ => None,
        }
    }
    match args.iter().find_map(first_error) {
        Some(err) => Err(err.clone()),
        None => Ok(flatten_numeric(args)),
    }
}

/// Flatten all arguments into a flat list of values, for the counting functions
/// such as COUNTA.
pub(crate) fn flatten_all(args: &[CellValue]) -> Vec<CellValue> {
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
pub(crate) fn check_argc(
    args: &[CellValue],
    expected: usize,
    _name: &str,
) -> Result<(), CellValue> {
    if args.len() != expected {
        return Err(CellValue::Error(FormulaError::NaError));
    }
    Ok(())
}

/// Terms at or above this magnitude could overflow a partial sum, so the exact
/// path stops there. Far above any value a sheet holds (about 1e301).
const EXACT_SUM_LIMIT: f64 = 1.0e301;

/// The sum of `nums`, rounded once from the exact total, so no order of the
/// terms gives another answer (`TRAME-199`).
///
/// Shewchuk's partials, finished as CPython's `math.fsum` does. A term that
/// is not finite, or at or above [`EXACT_SUM_LIMIT`], takes the plain fold.
pub(crate) fn exact_sum(nums: impl IntoIterator<Item = f64>) -> f64 {
    let nums: Vec<f64> = nums.into_iter().collect();
    if nums
        .iter()
        .any(|n| !n.is_finite() || n.abs() >= EXACT_SUM_LIMIT)
    {
        return nums.iter().sum();
    }
    // Non-overlapping partials, in increasing magnitude, whose exact sum is
    // the exact sum of the terms read so far.
    let mut partials: Vec<f64> = Vec::new();
    let mut next: Vec<f64> = Vec::new();
    for mut x in nums {
        next.clear();
        for &partial in &partials {
            let mut y = partial;
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                next.push(lo);
            }
            x = hi;
        }
        next.push(x);
        std::mem::swap(&mut partials, &mut next);
    }
    // Add the partials from the largest down, and stop at the first rounding.
    let Some(mut hi) = partials.pop() else {
        return 0.0;
    };
    let mut lo = 0.0;
    while let Some(y) = partials.pop() {
        let x = hi;
        hi = x + y;
        lo = y - (hi - x);
        if lo != 0.0 {
            break;
        }
    }
    // Round half to even across the partials the loop left. When `lo` and
    // the next partial share a sign, the true total sits past the halfway.
    if let Some(&next) = partials.last()
        && ((lo < 0.0 && next < 0.0) || (lo > 0.0 && next > 0.0))
    {
        let y = lo * 2.0;
        let x = hi + y;
        if y == x - hi {
            hi = x;
        }
    }
    hi
}

pub(crate) fn sum(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    CellValue::Number(exact_sum(nums))
}

pub(crate) fn average(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    if nums.is_empty() {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let count = nums.len() as f64;
    CellValue::Number(exact_sum(nums) / count)
}

pub(crate) fn min(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    nums.iter()
        .cloned()
        .fold(f64::NAN, f64::min)
        .let_ok_or(CellValue::Number(0.0))
}

pub(crate) fn max(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    nums.iter()
        .cloned()
        .fold(f64::NAN, f64::max)
        .let_ok_or(CellValue::Number(0.0))
}

pub(crate) fn count(args: &[CellValue]) -> CellValue {
    let nums = flatten_numeric(args);
    CellValue::Number(nums.len() as f64)
}

pub(crate) fn counta(args: &[CellValue]) -> CellValue {
    let flat = flatten_all(args);
    let non_empty = flat
        .iter()
        .filter(|v| !matches!(v, CellValue::Empty))
        .count();
    CellValue::Number(non_empty as f64)
}

pub(crate) fn abs(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match flatten_numeric(one(args, 0)).first() {
        Some(n) => CellValue::Number(n.abs()),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

pub(crate) fn round(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let num = match flatten_numeric(one(args, 0)).first() {
        Some(n) => *n,
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
    CellValue::Number((num * factor).round() / factor)
}

pub(crate) fn sqrt(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match flatten_numeric(one(args, 0)).first() {
        Some(&n) if n >= 0.0 => CellValue::Number(n.sqrt()),
        Some(_) => CellValue::Error(FormulaError::NumError),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

pub(crate) fn power(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let base = match flatten_numeric(one(args, 0)).first() {
        Some(n) => *n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let exp = match flatten_numeric(one(args, 1)).first() {
        Some(n) => *n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    // A negative base under a fractional exponent has no real root: `powf`
    // answers NaN, which is not a value a cell may hold (`TRAME-181`).
    match base.powf(exp) {
        n if n.is_finite() => CellValue::Number(n),
        _ => CellValue::Error(FormulaError::NumError),
    }
}

/// Read the first argument as a single number, or `None` if absent/non-numeric.
pub(crate) fn first_num(args: &[CellValue]) -> Option<f64> {
    flatten_numeric(one(args, 0)).first().copied()
}

/// INT(n) -- round down to the nearest integer (toward negative infinity).
pub(crate) fn int_fn(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Number(n.floor()),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// MOD(number, divisor) -- remainder with the sign of the divisor (Excel style).
pub(crate) fn mod_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (a, b) = match (first_num(one(args, 0)), first_num(one(args, 1))) {
        (Some(a), Some(b)) => (a, b),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    if b == 0.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    CellValue::Number(a - b * (a / b).floor())
}

/// ROUNDUP / ROUNDDOWN(number, [digits]) -- round away from / toward zero.
pub(crate) fn round_dir(args: &[CellValue], up: bool) -> CellValue {
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
    let scaled = num * factor;
    let rounded = if up {
        scaled.abs().ceil().copysign(scaled)
    } else {
        scaled.trunc()
    };
    CellValue::Number(rounded / factor)
}

/// CEILING / FLOOR(number, [significance]) -- round to a multiple of `significance`.
pub(crate) fn ceil_floor(args: &[CellValue], up: bool) -> CellValue {
    let num = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let sig = if args.len() >= 2 {
        flatten_numeric(one(args, 1))
            .first()
            .copied()
            .unwrap_or(1.0)
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
pub(crate) fn product(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    if nums.is_empty() {
        return CellValue::Number(0.0);
    }
    CellValue::Number(nums.iter().product())
}

/// SUMPRODUCT(range1, range2, ...) -- sum of the element-wise products of the
/// arguments, paired by position. Non-numeric cells contribute 0.
pub(crate) fn sumproduct(args: &[CellValue]) -> CellValue {
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
        .map(|i| columns.iter().filter_map(|c| c.get(i)).product::<f64>())
        .sum();
    CellValue::Number(total)
}

/// MEDIAN(...) -- middle value of the numeric arguments (mean of the two
/// central values for an even count).
pub(crate) fn median(args: &[CellValue]) -> CellValue {
    let mut nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    if nums.is_empty() {
        return CellValue::Error(FormulaError::NumError);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = nums.len() / 2;
    let m = match (nums.get(mid.wrapping_sub(1)), nums.get(mid)) {
        (Some(low), Some(high)) if nums.len().is_multiple_of(2) => (low + high) / 2.0,
        (_, Some(&middle)) => middle,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    CellValue::Number(m)
}

/// COUNTBLANK(range) -- count empty cells in the argument(s).
pub(crate) fn countblank(args: &[CellValue]) -> CellValue {
    let n = flatten_all(args)
        .iter()
        .filter(|v| matches!(v, CellValue::Empty))
        .count();
    CellValue::Number(n as f64)
}

pub(crate) fn if_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let condition = is_truthy(arg(args, 0));
    if condition {
        arg(args, 1).clone()
    } else if args.len() >= 3 {
        arg(args, 2).clone()
    } else {
        CellValue::Boolean(false)
    }
}

pub(crate) fn and(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flat = flatten_all(args);
    CellValue::Boolean(flat.iter().all(is_truthy))
}

pub(crate) fn or(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flat = flatten_all(args);
    CellValue::Boolean(flat.iter().any(is_truthy))
}

pub(crate) fn not(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Boolean(!is_truthy(arg(args, 0)))
}

/// XOR(...) -- TRUE when an odd number of arguments are truthy.
pub(crate) fn xor(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let truthy = flatten_all(args).iter().filter(|v| is_truthy(v)).count();
    CellValue::Boolean(truthy % 2 == 1)
}

/// ISBLANK(value) -- TRUE when the argument is an empty cell.
pub(crate) fn is_blank(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Empty)),
        None => CellValue::Error(FormulaError::NaError),
    }
}

/// ISNUMBER(value) -- TRUE when the argument is a number.
pub(crate) fn is_number(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Number(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

pub(crate) fn is_truthy(value: &CellValue) -> bool {
    match value {
        CellValue::Number(n) => *n != 0.0,
        CellValue::Text(s) => !s.is_empty(),
        CellValue::Boolean(b) => *b,
        CellValue::Empty => false,
        CellValue::Error(_) => false,
        CellValue::Range(r) => !r.values.is_empty(),
    }
}

pub(crate) fn concat_fn(args: &[CellValue]) -> CellValue {
    let mut result = String::new();
    for arg in args {
        push_display(arg, &mut result);
    }
    CellValue::Text(result)
}

/// Resolve the optional character-count argument shared by `LEFT`/`RIGHT`.
/// Returns the count as `usize`, or `#NUM!` for a negative value. A bare `as
/// usize` cast wraps a negative `f64` to a gigantic length. Excel returns an
/// error here rather than silently taking the whole string.
fn str_count_arg(args: &[CellValue], default: usize) -> Result<usize, FormulaError> {
    if args.len() < 2 {
        return Ok(default);
    }
    let n = flatten_numeric(one(args, 1))
        .first()
        .copied()
        .unwrap_or(default as f64);
    if n < 0.0 || !n.is_finite() {
        return Err(FormulaError::NumError);
    }
    Ok(n as usize)
}

pub(crate) fn left(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = arg(args, 0).to_display();
    let n = match str_count_arg(args, 1) {
        Ok(n) => n,
        Err(e) => return CellValue::Error(e),
    };
    CellValue::Text(s.chars().take(n).collect())
}

pub(crate) fn right(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = arg(args, 0).to_display();
    let n = match str_count_arg(args, 1) {
        Ok(n) => n,
        Err(e) => return CellValue::Error(e),
    };
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    CellValue::Text(chars.iter().skip(start).collect())
}

pub(crate) fn mid(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = arg(args, 0).to_display();
    let start = flatten_numeric(one(args, 1))
        .first()
        .copied()
        .unwrap_or(1.0)
        .max(1.0) as usize
        - 1;
    let n = flatten_numeric(one(args, 2))
        .first()
        .copied()
        .unwrap_or(0.0)
        .max(0.0) as usize;
    let chars: Vec<char> = s.chars().collect();
    let slice: String = chars.iter().skip(start).take(n).collect();
    CellValue::Text(slice)
}

pub(crate) fn upper(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Text(arg(args, 0).to_display().to_uppercase())
}

pub(crate) fn lower(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Text(arg(args, 0).to_display().to_lowercase())
}

pub(crate) fn trim(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let s = arg(args, 0).to_display();
    CellValue::Text(s.trim().to_string())
}

pub(crate) fn len(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Number(arg(args, 0).to_display().chars().count() as f64)
}
