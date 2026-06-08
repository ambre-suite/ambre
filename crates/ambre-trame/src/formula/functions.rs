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
pub fn call_builtin(name: &str, args: &[CellValue]) -> CellValue {
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

        // Logic
        "IF" => if_fn(args),
        "AND" => and(args),
        "OR" => or(args),
        "NOT" => not(args),

        // Text
        "CONCAT" | "CONCATENATE" => concat_fn(args),
        "LEFT" => left(args),
        "RIGHT" => right(args),
        "MID" => mid(args),
        "UPPER" => upper(args),
        "LOWER" => lower(args),
        "TRIM" => trim(args),
        "LEN" => len(args),

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
        CellValue::Range(vals) => {
            for v in vals {
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
            CellValue::Range(vals) => out.extend(vals.iter().cloned()),
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
        CellValue::Range(vals) => !vals.is_empty(),
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

// -----------------------------------------------------------------------------
// Helper
// -----------------------------------------------------------------------------

/// Flatten a value (including nested ranges) into a string builder.
fn push_display(value: &CellValue, out: &mut String) {
    match value {
        CellValue::Range(vals) => {
            for v in vals {
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

    #[test]
    fn math_functions() {
        assert_eq!(abs(&[CellValue::Number(-5.0)]).to_display(), "5");
        assert_eq!(round(&[CellValue::Number(std::f64::consts::PI), CellValue::Number(2.0)]).to_display(), "3.14");
        assert_eq!(sqrt(&[CellValue::Number(16.0)]).to_display(), "4");
        assert_eq!(power(&[CellValue::Number(2.0), CellValue::Number(3.0)]).to_display(), "8");
    }
}
