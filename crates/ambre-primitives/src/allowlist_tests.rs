use std::collections::BTreeSet;

use super::{Allowlist, Category, Defect, HEADER};
use crate::records::{Records, SiteKey};

const SITE: &str = "demo | src/lib.rs | crate | Thing.id | field | u32";
const OTHER: &str = "demo | src/lib.rs | crate | get(index) | param | usize";

fn key(text: &str) -> SiteKey {
    SiteKey::parse(text).expect("a six-field key")
}

fn sites(keys: &[&str]) -> BTreeSet<SiteKey> {
    keys.iter().map(|text| key(text)).collect()
}

/// A file with a comment, the header and one line per `(key, category)`.
fn file(lines: &[(&str, &str)]) -> String {
    let mut text = format!("# A hand-edited file.\n{HEADER}\n");
    for (key, category) in lines {
        text.push_str(&format!("{key}\t{category}\tthe reason\n"));
    }
    text
}

#[test]
fn a_site_no_line_names_fails_the_gate() {
    let list = Allowlist::parse(&file(&[(SITE, "local")])).expect("a valid file");
    let verdict = list.compare(&sites(&[SITE, OTHER]));
    assert_eq!(verdict.missing, [key(OTHER)]);
    assert!(!verdict.is_clean());
}

#[test]
fn a_line_that_names_no_site_is_stale() {
    let list = Allowlist::parse(&file(&[(SITE, "math"), (OTHER, "math")])).expect("a valid file");
    let verdict = list.compare(&sites(&[SITE]));
    assert_eq!(
        verdict
            .stale
            .iter()
            .map(|(_, key)| key.clone())
            .collect::<Vec<_>>(),
        [key(OTHER)]
    );
    assert!(!verdict.is_clean());
}

#[test]
fn a_file_naming_every_site_once_passes() {
    let list =
        Allowlist::parse(&file(&[(SITE, "free-text"), (OTHER, "math")])).expect("a valid file");
    assert!(list.compare(&sites(&[SITE, OTHER])).is_clean());
}

#[test]
fn a_category_outside_the_closed_set_is_refused() {
    let errors = Allowlist::parse(&file(&[(SITE, "fine")])).expect_err("an unknown category");
    assert_eq!(errors.len(), 1);
    assert!(errors.iter().all(|error| error.contains("closed set")));
}

#[test]
fn every_category_reads_back_as_itself() {
    for category in Category::ALL {
        assert_eq!(Category::parse(category.as_str()), Some(category));
    }
}

#[test]
fn a_malformed_line_is_refused() {
    let empty_note = format!("{HEADER}\n{SITE}\tlocal\t \n");
    let short_key = format!("{HEADER}\ndemo | src/lib.rs\tlocal\twhy\n");
    let twice = file(&[(SITE, "local"), (SITE, "math")]);
    let no_header = format!("{SITE}\tlocal\twhy\n");
    for text in [empty_note, short_key, twice, no_header] {
        assert!(Allowlist::parse(&text).is_err(), "accepted {text:?}");
    }
}

#[test]
fn a_prune_removes_the_stale_lines_and_nothing_else() {
    let text = file(&[(SITE, "local"), (OTHER, "math")]);
    let list = Allowlist::parse(&text).expect("a valid file");
    let kept = sites(&[SITE]);
    let pruned = list.pruned(&kept);
    assert_eq!(pruned, file(&[(SITE, "local")]));
    let again = Allowlist::parse(&pruned).expect("a valid file");
    assert!(again.compare(&kept).is_clean());
}

#[test]
fn a_prune_adds_no_line_for_a_new_site() {
    let list = Allowlist::parse(&file(&[(SITE, "local")])).expect("a valid file");
    let pruned = list.pruned(&sites(&[SITE, OTHER]));
    assert!(!pruned.contains(OTHER));
}

#[test]
fn the_lines_are_counted_per_category() {
    let list = Allowlist::parse(&file(&[(SITE, "local"), (OTHER, "local")])).expect("a valid file");
    let verdict = list.compare(&sites(&[SITE, OTHER]));
    assert_eq!(verdict.counts.get(&Category::Local), Some(&2));
    assert_eq!(verdict.counts.get(&Category::Math), None);
}

#[test]
fn a_debt_line_is_refused() {
    let errors = Allowlist::parse(&file(&[(SITE, "debt")])).expect_err("debt left the set");
    assert_eq!(errors.len(), 1);
    assert!(errors.iter().all(|error| error.contains("closed set")));
}

/// One site record, as the driver writes it.
fn site(
    key: &str,
    privacy: &str,
    shape: &str,
    inner: &str,
    trait_claim: &str,
    slot: &str,
) -> String {
    format!(
        "site\t{key}\t{privacy}\t{shape}\tmember\t{inner}\t{trait_claim}\t{slot}\tsource\tsrc/lib.rs:1\n"
    )
}

/// The refusals of `lines` against the records `text`, as key and defect.
fn refusals(text: &str, lines: &[(&str, &str)]) -> Vec<(SiteKey, Defect)> {
    let mut records = Records::default();
    records.add(text);
    assert!(records.errors.is_empty(), "{:?}", records.errors);
    let list = Allowlist::parse(&file(lines)).expect("a valid file");
    assert!(list.compare(&records.keys()).is_clean());
    list.refuted(&records)
        .into_iter()
        .map(|refuted| (refuted.key, refuted.defect))
        .collect()
}

// `From<u32>` is a foreign trait, but the impl chose the `u32`: the driver
// writes why the claim fails, and the judge refuses the line (`ARCH-036`).
#[test]
fn a_line_whose_site_refutes_its_category_is_reported() {
    const FIELD: &str = "demo | src/lib.rs | crate | Rect.w | field | u32";
    const FROM: &str = "demo | src/lib.rs | crate | <Rect as From<u32>>::from(w) | param | u32";
    const LOCAL: &str = "demo | src/lib.rs | crate | Rect.h | field | u32";
    let text = [
        site(
            FIELD,
            "private",
            "other",
            "a field of a struct of 2 fields",
            "x",
            "-",
        ),
        site(FROM, "trait-impl", "other", "x", "the impl writes u32", "-"),
        site(LOCAL, "private", "other", "x", "x", "-"),
    ]
    .concat();
    let found = refusals(
        &text,
        &[(FIELD, "newtype-inner"), (FROM, "trait"), (LOCAL, "local")],
    );
    assert_eq!(
        found,
        [(key(FIELD), Defect::Shape), (key(FROM), Defect::Shape)]
    );
}

// An associated type holds `newtype-inner` when a generic newtype holds it as
// its one field, a fact of another record (`ARCH-036`).
#[test]
fn an_associated_type_holds_when_a_generic_newtype_holds_it() {
    const HELD: &str = "demo | src/lib.rs | crate | <Doc as Space>::Repr | assoc | f32";
    const LOOSE: &str = "demo | src/lib.rs | crate | <Doc as Other>::Repr | assoc | f32";
    let text = [
        site(
            HELD,
            "trait-impl",
            "other",
            "repr:demo::Space::Repr",
            "x",
            "-",
        ),
        site(
            LOOSE,
            "trait-impl",
            "other",
            "repr:demo::Other::Repr",
            "x",
            "-",
        ),
        "repr\tdemo::Space::Repr\n".to_owned(),
    ]
    .concat();
    let found = refusals(&text, &[(HELD, "newtype-inner"), (LOOSE, "newtype-inner")]);
    assert_eq!(found, [(key(LOOSE), Defect::Shape)]);
}

#[test]
fn a_bool_field_of_a_foreign_schema_holds_under_foreign_api() {
    const VERIFIED: &str = "demo | src/lib.rs | crate | Claims.email_verified | field | bool";
    const ADMIN: &str = "demo | src/lib.rs | crate | Claims.admin | field | bool";
    let text = [
        site(VERIFIED, "serialised", "bool", "x", "x", "-"),
        site(ADMIN, "serialised", "bool", "x", "x", "-"),
    ]
    .concat();
    let found = refusals(&text, &[(VERIFIED, "foreign-api"), (ADMIN, "generic")]);
    assert_eq!(found, [(key(ADMIN), Defect::BoolField)]);
}

#[test]
fn a_privacy_refusal_holds_whatever_the_category() {
    const FLAG: &str = "demo | src/lib.rs | crate | Toggle.on | field | bool";
    const SET: &str = "demo | src/lib.rs | crate | set(on) | param | bool";
    const COUNT: &str = "demo | src/lib.rs | crate | count(n) | param | u32";
    let text = [
        site(FLAG, "public", "bool", "x", "x", "-"),
        site(SET, "public", "bool", "x", "x", "demo::set#0"),
        site(COUNT, "public", "other", "x", "x", "demo::count#0"),
        "lit\tdemo::set#0\tsrc/lib.rs:4\n".to_owned(),
    ]
    .concat();
    let found = refusals(
        &text,
        &[(FLAG, "local"), (SET, "generic"), (COUNT, "local")],
    );
    assert_eq!(
        found,
        [
            (key(FLAG), Defect::LocalSite),
            (key(FLAG), Defect::BoolField),
            (key(SET), Defect::BoolLiteral),
            (key(COUNT), Defect::LocalSite),
        ]
    );
}

#[test]
fn a_trait_line_takes_no_literal_refusal() {
    const FOREIGN: &str =
        "demo | src/lib.rs | crate | <Px as gloo::Handler>::call(on) | param | bool";
    const OWN: &str = "demo | src/lib.rs | crate | Px::call(on) | param | bool";
    let text = [
        site(
            FOREIGN,
            "trait-impl",
            "bool",
            "x",
            "ok",
            "demo::{impl#0}::call#0",
        ),
        site(OWN, "public", "bool", "x", "x", "demo::{impl#1}::call#0"),
        "lit\tdemo::{impl#0}::call#0\tsrc/lib.rs:4\n".to_owned(),
        "lit\tdemo::{impl#1}::call#0\tsrc/lib.rs:5\n".to_owned(),
    ]
    .concat();
    let found = refusals(&text, &[(FOREIGN, "trait"), (OWN, "generic")]);
    assert_eq!(found, [(key(OWN), Defect::BoolLiteral)]);
}

// A literal reaches the parameter its call resolves to, and no other site:
// a private site takes none (`ARCH-040`).
#[test]
fn a_literal_reaches_its_resolved_parameter_alone() {
    const PUBLIC: &str = "demo | src/lib.rs | crate | a::set(on) | param | bool";
    const PRIVATE: &str = "demo | src/lib.rs | crate | b::set(on) | param | bool";
    const WRAPPED: &str = "demo | src/lib.rs | crate | c::set(on) | param | Option<bool>";
    let text = [
        site(PUBLIC, "public", "bool", "x", "x", "demo::a::set#0"),
        site(PRIVATE, "private", "bool", "x", "x", "demo::b::set#0"),
        site(
            WRAPPED,
            "restricted",
            "holds-bool",
            "x",
            "x",
            "demo::c::set#0",
        ),
        "lit\tdemo::b::set#0\tsrc/lib.rs:4\n".to_owned(),
        "lit\tdemo::c::set#0\tsrc/lib.rs:5\n".to_owned(),
    ]
    .concat();
    let found = refusals(
        &text,
        &[
            (PUBLIC, "generic"),
            (PRIVATE, "generic"),
            (WRAPPED, "generic"),
        ],
    );
    assert_eq!(found, [(key(WRAPPED), Defect::BoolLiteral)]);
}
