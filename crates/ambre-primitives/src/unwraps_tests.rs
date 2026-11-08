use super::{HEADER, Outcome, Row, SiteCount, Strictness, judge, parse_allowlist};
use crate::allowlist::LineNo;

/// The key of two sites, `Name::as_str -> store` in `save`.
const KEY: &str = "demo | src/lib.rs | crate::save | Name::as_str -> store";

/// The allowlist text over the header, one line per row.
fn allowlist(rows: &[&str]) -> String {
    let mut text = format!("# comment\n{HEADER}\n");
    for row in rows {
        text.push_str(row);
        text.push('\n');
    }
    text
}

#[test]
fn the_allowlist_names_each_defect_of_its_lines() {
    let text = "key\treason\n\
        a\t0\tzero\n\
        b\tx\tnot a number\n\
        c\t1\t \n\
        d\n\
        e\t1\tfine\n\
        e\t2\ttwice\n";
    let errors = parse_allowlist(text).unwrap_err();
    assert_eq!(
        errors,
        [
            format!("line 1: the header is {HEADER:?}"),
            "line 2: the count is a whole number above 0".to_owned(),
            "line 3: the count is a whole number above 0".to_owned(),
            "line 4: the reason is empty".to_owned(),
            "line 5: a key, a tab, a count, a tab, then the reason".to_owned(),
            "line 7: the key is named twice".to_owned(),
        ]
    );
    assert_eq!(
        parse_allowlist("# only a comment\n").unwrap_err(),
        [format!("the header {HEADER:?} is missing")]
    );
    let rows = parse_allowlist(&allowlist(&["k\t2\tthe reason"])).unwrap();
    assert_eq!(
        rows.get("k"),
        Some(&Row {
            line: LineNo::new(3),
            count: SiteCount::new(2)
        })
    );
}

#[test]
fn report_mode_passes_on_refusals_and_strict_mode_fails() {
    let empty = allowlist(&[]);
    let lenient = judge([KEY, KEY], Some(&empty), Strictness::Report).unwrap();
    assert_eq!(lenient.outcome, Outcome::Pass);
    assert_eq!(lenient.unlisted, SiteCount::new(1));
    assert_eq!(
        lenient.problems,
        [format!(
            "no line names the key, the report finds 2\n  {KEY}"
        )]
    );
    let strict = judge([KEY, KEY], Some(&empty), Strictness::Strict).unwrap();
    assert_eq!(strict.outcome, Outcome::Fail);
}

#[test]
fn a_missing_or_malformed_allowlist_fails_both_modes() {
    for strictness in [Strictness::Report, Strictness::Strict] {
        assert!(judge([KEY], None, strictness).is_err());
        assert!(judge([KEY], Some("key\treason\n"), strictness).is_err());
    }
}

#[test]
fn strict_mode_holds_each_line_to_its_count() {
    let exact = allowlist(&[&format!("{KEY}\t2\tthe reason")]);
    let passed = judge([KEY, KEY], Some(&exact), Strictness::Strict).unwrap();
    assert_eq!(passed.outcome, Outcome::Pass);
    assert!(passed.problems.is_empty());

    let short = allowlist(&[&format!("{KEY}\t1\tthe reason")]);
    let over = judge([KEY, KEY], Some(&short), Strictness::Strict).unwrap();
    assert_eq!(over.outcome, Outcome::Fail);
    assert_eq!(over.over, SiteCount::new(1));

    let long = allowlist(&[&format!("{KEY}\t3\tthe reason")]);
    let stale = judge([KEY, KEY], Some(&long), Strictness::Strict).unwrap();
    assert_eq!(stale.outcome, Outcome::Fail);
    assert_eq!(stale.stale, SiteCount::new(1));

    let gone = allowlist(&[&format!("{KEY}\t2\tthe reason"), "no such key\t1\tgone"]);
    let judged = judge([KEY, KEY], Some(&gone), Strictness::Strict).unwrap();
    assert_eq!(judged.outcome, Outcome::Fail);
    assert_eq!(judged.stale, SiteCount::new(1));
    assert_eq!(
        judged.problems,
        ["line 4 allows 1 sites, the report finds 0\n  no such key"]
    );
    let lenient = judge([KEY, KEY], Some(&gone), Strictness::Report).unwrap();
    assert_eq!(lenient.outcome, Outcome::Pass);
}
