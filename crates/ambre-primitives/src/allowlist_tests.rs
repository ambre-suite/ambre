use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{Allowlist, Category, HEADER};
use crate::scan::{SiteKey, Tree, scan};

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

/// One crate `demo` with one `src/lib.rs`, the tree a refutation reads.
struct OneFile(String);

impl Tree for OneFile {
    fn read(&self, path: &Path) -> Option<String> {
        if path == Path::new("/w/crates/demo/src/lib.rs") {
            Some(self.0.clone())
        } else {
            (path == Path::new("/w/crates/demo/Cargo.toml")).then(String::new)
        }
    }

    fn files(&self, dir: &Path) -> Vec<PathBuf> {
        ["/w/crates/demo/Cargo.toml", "/w/crates/demo/src/lib.rs"]
            .map(PathBuf::from)
            .into_iter()
            .filter(|path| path.starts_with(dir))
            .collect()
    }
}

#[test]
fn a_line_whose_site_refutes_its_category_is_reported() {
    const FIELD: &str = "demo | src/lib.rs | crate | Rect.w | field | u32";
    const FROM: &str = "demo | src/lib.rs | crate | <Rect as From<u32>>::from(w) | param | u32";
    const LOCAL: &str = "demo | src/lib.rs | crate | Rect.h | field | u32";
    let scan = scan(
        &OneFile(
            "pub struct Rect { w: u32, h: u32 }\n\
             impl From<u32> for Rect { fn from(w: u32) -> Self { x() } }"
                .to_owned(),
        ),
        Path::new("/w/crates"),
    );
    let list = Allowlist::parse(&file(&[
        (FIELD, "newtype-inner"),
        (FROM, "trait"),
        (LOCAL, "local"),
    ]))
    .expect("a valid file");
    assert!(list.compare(&scan.sites).is_clean());
    let refuted = list.refuted(&scan);
    assert_eq!(
        refuted
            .iter()
            .map(|line| (line.key.clone(), line.category))
            .collect::<Vec<_>>(),
        [(key(FIELD), Category::NewtypeInner)]
    );
}
