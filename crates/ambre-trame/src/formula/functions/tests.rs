// ambre-trame/src/formula/functions/tests.rs
// Unit tests for the builtin set (descendant module of `functions`).

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
fn countif_numeric_criterion_over_non_numeric_cells() {
    // A cell that does not coerce to a number can never satisfy an ordering
    // criterion. But `<>5` must count it: "not equal to 5" is true of any text.
    let r = range(3, 1, vec![num(5.0), txt("apple"), num(7.0)]);
    assert_eq!(countif(&[r.clone(), txt("<>5")]).to_display(), "2");
    assert_eq!(countif(&[r.clone(), txt(">5")]).to_display(), "1");
    assert_eq!(countif(&[r, txt(">=5")]).to_display(), "2");
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
    approx(variance(std::slice::from_ref(&r)), 2.5);
    approx(stdev(std::slice::from_ref(&r)), 2.5f64.sqrt());
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

#[test]
fn total_propagates_an_error_it_cannot_read() {
    let with_ref = || {
        range(
            3,
            1,
            vec![
                CellValue::Number(1.0),
                CellValue::Error(FormulaError::BrokenRef),
                CellValue::Number(2.0),
            ],
        )
    };
    let totals = [
        "SUM", "AVERAGE", "MIN", "MAX", "PRODUCT", "SUMSQ", "MEDIAN", "STDEV", "VAR", "MODE",
    ];
    for name in totals {
        assert_eq!(
            call_builtin(name, &[with_ref()], FnCtx::at_clock(0.0)).to_display(),
            "#REF!",
            "{name}"
        );
    }
    // COUNT counts the numbers it finds and skips an error, as the spreadsheets do.
    assert_eq!(
        call_builtin("COUNT", &[with_ref()], FnCtx::at_clock(0.0)).to_display(),
        "2"
    );
}

/// The names the `call_builtin` dispatch answers, read from its `match` arms.
fn dispatched_names() -> std::collections::BTreeSet<String> {
    let src = include_str!("mod.rs");
    let body = src
        .split_once("pub fn call_builtin(")
        .and_then(|(_, rest)| rest.split_once("        _ =>"))
        .map(|(body, _)| body)
        .expect("call_builtin has a fallback arm");
    body.lines()
        .filter_map(|line| line.trim_start().strip_prefix('"').map(|_| line))
        .filter_map(|line| line.split_once("=>").map(|(pattern, _)| pattern))
        .flat_map(|pattern| pattern.split('|'))
        .map(|name| name.trim().trim_matches('"').to_string())
        .collect()
}

/// The names the catalogue of `ambre-spec-trame-function-library.md` section 3 lists.
fn catalogued_names() -> std::collections::BTreeSet<String> {
    let spec =
        include_str!("../../../../../specs/2-Trame/features/ambre-spec-trame-function-library.md");
    let table = spec
        .split_once("**The catalogue**")
        .and_then(|(_, rest)| rest.split_once("**How a call behaves:**"))
        .map(|(table, _)| table)
        .expect("section 3 carries the catalogue");
    table
        .lines()
        .filter_map(|row| row.trim_end_matches('|').rsplit('|').next())
        .flat_map(|cell| cell.split('`'))
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn catalogue_lists_every_dispatched_name() {
    let dispatched = dispatched_names();
    let catalogued = catalogued_names();
    assert!(
        dispatched.len() > 100,
        "the dispatch parse found {}",
        dispatched.len()
    );
    assert_eq!(
        dispatched.difference(&catalogued).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "dispatched but not catalogued"
    );
    assert_eq!(
        catalogued.difference(&dispatched).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "catalogued but not dispatched"
    );
}

#[test]
fn power_without_a_real_root_is_a_num_error() {
    assert_eq!(
        call_builtin("POWER", &[num(-1.0), num(0.5)], FnCtx::at_clock(0.0)).to_display(),
        "#NUM!"
    );
    assert_eq!(
        call_builtin("POWER", &[num(-8.0), num(3.0)], FnCtx::at_clock(0.0)).to_display(),
        "-512"
    );
}
