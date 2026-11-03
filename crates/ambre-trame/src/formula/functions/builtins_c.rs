// ambre-trame/src/formula/functions/builtins_c.rs
// Spreadsheet builtin implementations dispatched by `call_builtin`. Cross-refs
// resolve through the module glob mesh.

use super::*;
use crate::formula::types::Arg;
use ambre_core::index::{Count, Pos};

/// STDEV / VAR over the sample (denominator `n - 1`).
pub(crate) fn variance(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    if nums.len() < 2 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
    let ss: f64 = nums.iter().map(|n| (n - mean).powi(2)).sum();
    CellValue::Number(ss / (nums.len() - 1) as f64)
}

pub(crate) fn stdev(args: &[CellValue]) -> CellValue {
    match variance(args) {
        CellValue::Number(v) => CellValue::Number(v.sqrt()),
        other => other,
    }
}

/// LARGE / SMALL(array..., k) -- the k-th largest / smallest numeric value.
pub(crate) fn large_small(args: &[CellValue], rank: Rank) -> CellValue {
    let Some((last, values)) = args.split_last().filter(|(_, values)| !values.is_empty()) else {
        return CellValue::Error(FormulaError::NaError);
    };
    let k = match last.as_number() {
        Ok(n) if n >= 1.0 => n as usize,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let mut nums = match flatten_total(values) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    if k > nums.len() {
        return CellValue::Error(FormulaError::NumError);
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = match rank {
        Rank::Largest => nums.len() - k,
        Rank::Smallest => k - 1,
    };
    match nums.get(pick) {
        Some(&n) => CellValue::Number(n),
        None => CellValue::Error(FormulaError::NumError),
    }
}

/// RANK(number, ref, [order]) -- 1-based rank; order 0/omitted = descending.
pub(crate) fn rank(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let num = match arg(args, Pos::<Arg>::new(0)).as_number() {
        Ok(n) => n,
        Err(e) => return CellValue::Error(e),
    };
    let data: Vec<f64> = range_cells(arg(args, Pos::<Arg>::new(1)))
        .filter_map(|v| v.as_number().ok())
        .collect();
    if !data.iter().any(|n| nearly_equal(*n, num)) {
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
pub(crate) fn mode(args: &[CellValue]) -> CellValue {
    let nums = match flatten_total(args) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    let Some(&first) = nums.first() else {
        return CellValue::Error(FormulaError::NaError);
    };
    let mut best = first;
    let mut best_count = 0usize;
    for (i, &a) in nums.iter().enumerate() {
        let c = nums.iter().skip(i).filter(|&&b| nearly_equal(b, a)).count();
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

/// When the payments of an annuity fall, from its type argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayTiming {
    /// Type 0, the default: at the end of each period.
    End,
    /// Any other type: at the start of each period.
    Start,
}

impl PayTiming {
    /// The timing the type argument at `i` names. An absent or zero type is
    /// the end, and any other number the start: a type of 2 is no third
    /// timing.
    pub(crate) fn of_arg(args: &[CellValue], i: Pos<Arg>) -> Self {
        if opt_num(args, i, 0.0) == 0.0 {
            Self::End
        } else {
            Self::Start
        }
    }

    /// What one payment grows by before it counts: one period of interest
    /// at the start, nothing at the end.
    pub(crate) fn growth(self, rate: f64) -> f64 {
        match self {
            Self::End => 1.0,
            Self::Start => 1.0 + rate,
        }
    }
}

/// The terms of an annuity the formulas share. Named fields, so the rate and
/// the count of periods cannot trade places.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Annuity {
    pub(crate) rate: f64,
    pub(crate) periods: f64,
    pub(crate) timing: PayTiming,
}

impl Annuity {
    /// `((1+r)^n - 1) / r` times the growth of one payment: what the stream
    /// of payments is worth at the last period.
    pub(crate) fn factor(self) -> f64 {
        ((1.0 + self.rate).powf(self.periods) - 1.0) / self.rate * self.timing.growth(self.rate)
    }
}

/// Optional positional argument as a number, with a default.
pub(crate) fn opt_num(args: &[CellValue], i: Pos<Arg>, default: f64) -> f64 {
    i.to_index()
        .and_then(|i| args.get(i))
        .and_then(|v| v.as_number().ok())
        .unwrap_or(default)
}

/// PMT(rate, nper, pv, [fv], [type]).
pub(crate) fn pmt(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pv) = (
        opt_num(args, Pos::<Arg>::new(0), 0.0),
        opt_num(args, Pos::<Arg>::new(1), 0.0),
        opt_num(args, Pos::<Arg>::new(2), 0.0),
    );
    let fv = opt_num(args, Pos::<Arg>::new(3), 0.0);
    let timing = PayTiming::of_arg(args, Pos::<Arg>::new(4));
    if nper == 0.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let annuity = Annuity {
        rate,
        periods: nper,
        timing,
    };
    let value = if rate == 0.0 {
        -(pv + fv) / nper
    } else {
        -(pv * (1.0 + rate).powf(nper) + fv) / annuity.factor()
    };
    CellValue::Number(value)
}

/// FV(rate, nper, pmt, [pv], [type]).
pub(crate) fn fv(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pmt_v) = (
        opt_num(args, Pos::<Arg>::new(0), 0.0),
        opt_num(args, Pos::<Arg>::new(1), 0.0),
        opt_num(args, Pos::<Arg>::new(2), 0.0),
    );
    let pv = opt_num(args, Pos::<Arg>::new(3), 0.0);
    let annuity = Annuity {
        rate,
        periods: nper,
        timing: PayTiming::of_arg(args, Pos::<Arg>::new(4)),
    };
    let value = if rate == 0.0 {
        -(pv + pmt_v * nper)
    } else {
        -(pv * (1.0 + rate).powf(nper) + pmt_v * annuity.factor())
    };
    CellValue::Number(value)
}

/// PV(rate, nper, pmt, [fv], [type]).
pub(crate) fn pv(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, nper, pmt_v) = (
        opt_num(args, Pos::<Arg>::new(0), 0.0),
        opt_num(args, Pos::<Arg>::new(1), 0.0),
        opt_num(args, Pos::<Arg>::new(2), 0.0),
    );
    let fv = opt_num(args, Pos::<Arg>::new(3), 0.0);
    let annuity = Annuity {
        rate,
        periods: nper,
        timing: PayTiming::of_arg(args, Pos::<Arg>::new(4)),
    };
    let value = if rate == 0.0 {
        -(fv + pmt_v * nper)
    } else {
        -(fv + pmt_v * annuity.factor()) / (1.0 + rate).powf(nper)
    };
    CellValue::Number(value)
}

/// NPER(rate, pmt, pv, [fv], [type]).
pub(crate) fn nper(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (rate, pmt_v, pv) = (
        opt_num(args, Pos::<Arg>::new(0), 0.0),
        opt_num(args, Pos::<Arg>::new(1), 0.0),
        opt_num(args, Pos::<Arg>::new(2), 0.0),
    );
    let fv = opt_num(args, Pos::<Arg>::new(3), 0.0);
    let timing = PayTiming::of_arg(args, Pos::<Arg>::new(4));
    if rate == 0.0 {
        if pmt_v == 0.0 {
            return CellValue::Error(FormulaError::DivisionByZero);
        }
        return CellValue::Number(-(pv + fv) / pmt_v);
    }
    let adj = pmt_v * timing.growth(rate);
    let num = adj - fv * rate;
    let den = adj + pv * rate;
    if den == 0.0 || num / den <= 0.0 {
        return CellValue::Error(FormulaError::NumError);
    }
    CellValue::Number((num / den).ln() / (1.0 + rate).ln())
}

/// RATE(nper, pmt, pv, [fv], [type], [guess]) -- Newton-Raphson on the annuity
/// balance equation. Returns `#NUM!` if it fails to converge.
pub(crate) fn rate(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (nper, pmt_v, pv) = (
        opt_num(args, Pos::<Arg>::new(0), 0.0),
        opt_num(args, Pos::<Arg>::new(1), 0.0),
        opt_num(args, Pos::<Arg>::new(2), 0.0),
    );
    let fv = opt_num(args, Pos::<Arg>::new(3), 0.0);
    let timing = PayTiming::of_arg(args, Pos::<Arg>::new(4));
    let mut r = opt_num(args, Pos::<Arg>::new(5), 0.1);
    // f(r) = pv*(1+r)^n + pmt*growth(r)*((1+r)^n-1)/r + fv
    let f = |r: f64| -> f64 {
        if r == 0.0 {
            pv + pmt_v * nper + fv
        } else {
            let annuity = Annuity {
                rate: r,
                periods: nper,
                timing,
            };
            pv * (1.0 + r).powf(nper) + pmt_v * annuity.factor() + fv
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
pub(crate) fn npv(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let rate = match arg(args, Pos::<Arg>::new(0)).as_number() {
        Ok(r) => r,
        Err(e) => return CellValue::Error(e),
    };
    if rate == -1.0 {
        return CellValue::Error(FormulaError::DivisionByZero);
    }
    let flows = match flatten_total(rest_from(args, Pos::<Arg>::new(1))) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
    let total = flows
        .iter()
        .enumerate()
        .map(|(i, v)| v / (1.0 + rate).powi(i as i32 + 1))
        .sum();
    CellValue::Number(total)
}

/// IRR(values, [guess]) -- Newton-Raphson on the NPV equation. Period-0 cash
/// flow is undiscounted; `#NUM!` if it fails to converge.
pub(crate) fn irr(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    let flows = match flatten_total(one(args, Pos::<Arg>::new(0))) {
        Ok(nums) => nums,
        Err(err) => return err,
    };
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

pub(crate) fn is_text(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Text(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

pub(crate) fn is_error(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Error(_))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

pub(crate) fn is_na(args: &[CellValue]) -> CellValue {
    match args.first() {
        Some(v) => CellValue::Boolean(matches!(v, CellValue::Error(FormulaError::NaError))),
        None => CellValue::Error(FormulaError::NaError),
    }
}

pub(crate) fn is_even(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Boolean((n.trunc() as i64).rem_euclid(2) == 0),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

pub(crate) fn is_odd(args: &[CellValue]) -> CellValue {
    match first_num(args) {
        Some(n) => CellValue::Boolean((n.trunc() as i64).rem_euclid(2) == 1),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// IFNA(value, fallback) -- `fallback` only when `value` is the `#N/A` error.
pub(crate) fn ifna(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, Pos::<Arg>::new(0)) {
        CellValue::Error(FormulaError::NaError) => arg(args, Pos::<Arg>::new(1)).clone(),
        other => other.clone(),
    }
}

/// IFS(cond1, val1, cond2, val2, ...) -- first truthy condition wins.
pub(crate) fn ifs(args: &[CellValue]) -> CellValue {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return CellValue::Error(FormulaError::NaError);
    }
    for pair in args.chunks_exact(2) {
        if let [cond, value] = pair
            && is_truthy(cond)
        {
            return value.clone();
        }
    }
    CellValue::Error(FormulaError::NaError)
}

/// SWITCH(expr, val1, res1, [val2, res2, ...], [default]).
pub(crate) fn switch_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let subject = arg(args, Pos::<Arg>::new(0));
    let rest = rest_from(args, Pos::<Arg>::new(1));
    let pairs = rest.chunks_exact(2);
    // A trailing unpaired argument is the default.
    let default = pairs.remainder().first().cloned();
    for pair in pairs {
        if let [value, result] = pair
            && values_equal_ci(subject, value)
        {
            return result.clone();
        }
    }
    default.unwrap_or(CellValue::Error(FormulaError::NaError))
}

/// HLOOKUP(lookup, table, row_index, [range_lookup]) -- horizontal VLOOKUP.
pub(crate) fn hlookup(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let table = match arg(args, Pos::<Arg>::new(1)) {
        CellValue::Range(r) => r,
        _ => return CellValue::Error(FormulaError::NaError),
    };
    let row_index = match arg(args, Pos::<Arg>::new(2)).as_number() {
        Ok(n) if n >= 1.0 => Pos::<Row>::from_usize(n as usize - 1),
        _ => return CellValue::Error(FormulaError::NaError),
    };
    if row_index >= table.rows.as_end() {
        return CellValue::Error(FormulaError::NaError);
    }
    let approximate = args.get(3).map(is_truthy).unwrap_or(true);
    if approximate
        && !keys_ascending(
            table
                .col_offsets()
                .into_iter()
                .filter_map(|col| table.get(Pos::<Row>::ZERO, col)),
        )
    {
        return CellValue::Error(FormulaError::NaError);
    }
    let mut best: Option<Pos<Col>> = None;
    for col in table.col_offsets() {
        let key = match table.get(Pos::<Row>::ZERO, col) {
            Some(CellValue::Empty) if approximate => continue,
            Some(k) => k,
            None => continue,
        };
        if approximate {
            if compare_cells(key, arg(args, Pos::<Arg>::new(0)))
                .map(|o| o.is_le())
                .unwrap_or(false)
            {
                best = Some(col);
            } else {
                break;
            }
        } else if values_equal_ci(key, arg(args, Pos::<Arg>::new(0))) {
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
pub(crate) fn choose(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, Pos::<Arg>::new(0)).as_number() {
        Ok(n) if n >= 1.0 && (n as usize) < args.len() => {
            arg(args, Pos::<Arg>::from_usize(n as usize)).clone()
        }
        Ok(_) => CellValue::Error(FormulaError::NumError),
        Err(e) => CellValue::Error(e),
    }
}

/// Dimensions of the first argument: a `Range` yields `(rows, cols)`, a scalar
/// behaves like a 1x1 range.
pub(crate) fn range_dims(args: &[CellValue]) -> Option<(Count<Row>, Count<Col>)> {
    match args.first()? {
        CellValue::Range(r) => Some((r.rows, r.cols)),
        _ => Some((Count::<Row>::ONE, Count::<Col>::ONE)),
    }
}

/// ROW()/COLUMN() -- only the no-argument form is supported (the evaluating
/// cell's 1-based number, `None` when unknown). The reference form (`ROW(A1)`)
/// needs reference-preserving compilation and is rejected with `#N/A` for now.
pub(crate) fn row_col(args: &[CellValue], current: Option<f64>) -> CellValue {
    match current {
        Some(number) if args.is_empty() => CellValue::Number(number),
        _ => CellValue::Error(FormulaError::NaError),
    }
}

/// SEARCH(find_text, within_text, [start]) -- case-insensitive `FIND`.
pub(crate) fn search_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    // One folded char per char, so a position in the folded text is the
    // position in the text the call was given. A whole-string lowercase
    // grows a capital I with a dot into two chars.
    let fold = |value: &CellValue| -> String {
        value
            .to_display()
            .chars()
            .map(crate::formula::text::fold_char_simple)
            .collect()
    };
    let lowered = vec![
        CellValue::Text(fold(arg(args, Pos::<Arg>::new(0)))),
        CellValue::Text(fold(arg(args, Pos::<Arg>::new(1)))),
    ];
    let mut call: Vec<CellValue> = lowered;
    if let Some(start) = args.get(2) {
        call.push(start.clone());
    }
    find_fn(&call)
}

/// REPLACE(old_text, start_num, num_chars, new_text) -- positional replacement.
pub(crate) fn replace_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 4 {
        return CellValue::Error(FormulaError::NaError);
    }
    let chars: Vec<char> = arg(args, Pos::<Arg>::new(0)).to_display().chars().collect();
    let start = match arg(args, Pos::<Arg>::new(1)).as_number() {
        Ok(n) if n >= 1.0 => n as usize - 1,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let count = match arg(args, Pos::<Arg>::new(2)).as_number() {
        Ok(n) if n >= 0.0 => n as usize,
        _ => return CellValue::Error(FormulaError::NumError),
    };
    let new = arg(args, Pos::<Arg>::new(3)).to_display();
    let start = start.min(chars.len());
    let end = (start + count).min(chars.len());
    let mut out: String = chars.iter().take(start).collect();
    out.push_str(&new);
    out.extend(chars.iter().skip(end));
    CellValue::Text(out)
}

/// EXACT(text1, text2) -- case-sensitive equality.
pub(crate) fn exact(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    CellValue::Boolean(
        arg(args, Pos::<Arg>::new(0)).to_display() == arg(args, Pos::<Arg>::new(1)).to_display(),
    )
}

/// TEXTJOIN(delimiter, ignore_empty, text1, ...) -- join with a separator.
pub(crate) fn textjoin(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let delim = arg(args, Pos::<Arg>::new(0)).to_display();
    let ignore_empty = is_truthy(arg(args, Pos::<Arg>::new(1)));
    let parts: Vec<String> = flatten_all(rest_from(args, Pos::<Arg>::new(2)))
        .iter()
        .filter(|v| !(ignore_empty && matches!(v, CellValue::Empty)))
        .map(|v| v.to_display())
        .filter(|s| !(ignore_empty && s.is_empty()))
        .collect();
    CellValue::Text(parts.join(&delim))
}

/// CHAR(number) -- the character for a Unicode code point (1..=0x10FFFF).
pub(crate) fn char_fn(args: &[CellValue]) -> CellValue {
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
pub(crate) fn code_fn(args: &[CellValue]) -> CellValue {
    if args.is_empty() {
        return CellValue::Error(FormulaError::NaError);
    }
    match arg(args, Pos::<Arg>::new(0)).to_display().chars().next() {
        Some(c) => CellValue::Number(c as u32 as f64),
        None => CellValue::Error(FormulaError::TypeError),
    }
}

/// TIME(hour, minute, second) -- fraction of a day (0..1).
pub(crate) fn time_fn(args: &[CellValue]) -> CellValue {
    if args.len() < 3 {
        return CellValue::Error(FormulaError::NaError);
    }
    let secs = match (
        arg(args, Pos::<Arg>::new(0)).as_number(),
        arg(args, Pos::<Arg>::new(1)).as_number(),
        arg(args, Pos::<Arg>::new(2)).as_number(),
    ) {
        (Ok(h), Ok(m), Ok(s)) => h * 3600.0 + m * 60.0 + s,
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    // Excel wraps to a 24h fraction.
    CellValue::Number((secs / 86_400.0).rem_euclid(1.0))
}

/// The part of a time of day HOUR, MINUTE and SECOND read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimePart {
    Hour,
    Minute,
    Second,
}

/// HOUR/MINUTE/SECOND -- `which` selects the part.
pub(crate) fn time_part(args: &[CellValue], which: TimePart) -> CellValue {
    let serial = match first_num(args) {
        Some(n) => n,
        None => return CellValue::Error(FormulaError::TypeError),
    };
    let frac = serial - serial.floor();
    let total = (frac * 86_400.0).round() as i64;
    let value = match which {
        TimePart::Hour => (total / 3600) % 24,
        TimePart::Minute => (total / 60) % 60,
        TimePart::Second => total % 60,
    };
    CellValue::Number(value as f64)
}

/// WEEKDAY(serial, [type]) -- type 1 (default) Sun=1..Sat=7; 2 Mon=1..Sun=7;
/// 3 Mon=0..Sun=6.
pub(crate) fn weekday(args: &[CellValue]) -> CellValue {
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

/// Number of days in the month of `date`.
pub(crate) fn days_in_month(date: DateParts) -> TimeSpan<Day> {
    let first = DateParts {
        day: DayOfMonth::new(1),
        ..date
    };
    first.next_month_start().serial() - first.serial()
}

/// The last day of the month of `date`.
fn last_day(date: DateParts) -> DayOfMonth {
    DayOfMonth::last_of(days_in_month(date))
}

/// Shift the month of `date` by `months`, normalizing into a valid month. The
/// day is kept as it stands.
pub(crate) fn shift_month(date: DateParts, months: MonthCount) -> DateParts {
    DateParts::of_months(
        date.year,
        MonthCount::new(i64::from(date.month.get()) - 1).plus(months),
        date.day,
    )
}

/// EDATE(start, months) -- same day-of-month `months` away (clamped to the
/// target month length).
pub(crate) fn edate(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (serial, months) = match (
        arg(args, Pos::<Arg>::new(0)).as_number(),
        arg(args, Pos::<Arg>::new(1)).as_number(),
    ) {
        (Ok(s), Ok(m)) => (s.floor() as i64, m as i64),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    let shifted = shift_month(
        DateParts::of_day(Instant::<SheetDay>::new(serial)),
        MonthCount::new(months),
    );
    let day = shifted.day.min(last_day(shifted));
    CellValue::Number(DateParts { day, ..shifted }.serial().get() as f64)
}

/// EOMONTH(start, months) -- last day of the month `months` away.
pub(crate) fn eomonth(args: &[CellValue]) -> CellValue {
    if args.len() < 2 {
        return CellValue::Error(FormulaError::NaError);
    }
    let (serial, months) = match (
        arg(args, Pos::<Arg>::new(0)).as_number(),
        arg(args, Pos::<Arg>::new(1)).as_number(),
    ) {
        (Ok(s), Ok(m)) => (s.floor() as i64, m as i64),
        _ => return CellValue::Error(FormulaError::TypeError),
    };
    let shifted = shift_month(
        DateParts::of_day(Instant::<SheetDay>::new(serial)),
        MonthCount::new(months),
    );
    let day = last_day(shifted);
    CellValue::Number(DateParts { day, ..shifted }.serial().get() as f64)
}
