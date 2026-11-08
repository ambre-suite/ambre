use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{EXCLUSIONS, check};

/// The header and comment lines of an exclusions file with no line.
const EMPTY: &str = "# comment\nfeature\treason\n";

/// A workspace written to a fresh directory, removed on drop.
struct Files(PathBuf);

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A workspace of the crate `demo`, whose manifest declares `features`,
/// with the exclusions file `exclusions`, or none. Each call takes its own
/// directory, so the tests run in parallel.
fn workspace(features: &str, exclusions: Option<&str>) -> Files {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("ambre-features-{}-{n}", std::process::id()));
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a written file");
    };
    write("Cargo.toml", "[workspace]\nmembers = [\"crates/demo\"]\n");
    write(
        "crates/demo/Cargo.toml",
        &format!("[package]\nname = \"demo\"\n\n[features]\n{features}"),
    );
    if let Some(text) = exclusions {
        write(EXCLUSIONS, text);
    }
    Files(std::fs::canonicalize(&root).expect("a root"))
}

/// The features of `demo` that the compiles turned on.
fn turned_on(tree: &Files, features: &[&str]) -> BTreeMap<PathBuf, BTreeSet<String>> {
    BTreeMap::from([(
        tree.0.join("crates/demo/Cargo.toml"),
        features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect(),
    )])
}

#[test]
fn a_feature_every_compile_leaves_off_is_refused() {
    let tree = workspace("a = []\nb = []\n", Some(EMPTY));
    let refusals = check(&tree.0, &turned_on(&tree, &["a"]));
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(
        refusal.starts_with("demo/b: no compile of the lint turns the feature on"),
        "{refusal}"
    );
}

#[test]
fn a_package_no_compile_builds_has_each_feature_refused() {
    let tree = workspace("a = []\n", Some(EMPTY));
    let refusals = check(&tree.0, &BTreeMap::new());
    assert_eq!(refusals.len(), 1, "{refusals:?}");
}

#[test]
fn a_feature_a_compile_turns_on_passes_whatever_turned_it_on() {
    // cargo reports `b` on when the line names only `a = ["b"]`: the check
    // reads what cargo turned on, never the line.
    let tree = workspace("a = [\"b\"]\nb = []\n", Some(EMPTY));
    assert_eq!(
        check(&tree.0, &turned_on(&tree, &["a", "b"])),
        Vec::<String>::new()
    );
}

#[test]
fn an_excluded_feature_passes_with_its_reason() {
    let tree = workspace(
        "a = []\n",
        Some(&format!("{EMPTY}demo/a\tgates only a test attribute\n")),
    );
    assert_eq!(check(&tree.0, &turned_on(&tree, &[])), Vec::<String>::new());
}

#[test]
fn an_exclusion_that_names_no_declared_feature_is_stale() {
    let tree = workspace(
        "a = []\n",
        Some(&format!("{EMPTY}demo/gone\treason\nother/a\treason\n")),
    );
    let refusals = check(&tree.0, &turned_on(&tree, &["a"]));
    assert_eq!(refusals.len(), 2, "{refusals:?}");
    assert!(
        refusals
            .iter()
            .all(|refusal| refusal.contains("no manifest of the workspace declares")),
        "{refusals:?}"
    );
    assert!(
        refusals[0].starts_with(&format!("{EXCLUSIONS}:3:")),
        "{refusals:?}"
    );
}

#[test]
fn an_exclusion_of_a_feature_a_compile_turns_on_is_stale() {
    let tree = workspace("a = []\n", Some(&format!("{EMPTY}demo/a\treason\n")));
    let refusals = check(&tree.0, &turned_on(&tree, &["a"]));
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(
        refusal.contains("a compile of the lint turns on demo/a"),
        "{refusal}"
    );
}

#[test]
fn a_missing_or_malformed_exclusions_file_fails_closed() {
    let missing = check(&workspace("", None).0, &BTreeMap::new());
    assert!(
        missing
            .iter()
            .any(|refusal| refusal.contains("cannot read it")),
        "{missing:?}"
    );
    for (text, defect) in [
        ("demo/a\treason\n", "the header must read"),
        ("", "no header line"),
        (&format!("{EMPTY}demo/a\n") as &str, "1 fields, want 2"),
        (&format!("{EMPTY}a\treason\n"), "must read package/feature"),
        (
            &format!("{EMPTY}demo/x/a\treason\n"),
            "must read package/feature",
        ),
        (&format!("{EMPTY}demo/a\t \n"), "the reason is empty"),
        (
            &format!("{EMPTY}demo/a\treason\ndemo/a\treason\n"),
            "a second line",
        ),
    ] {
        let refusals = check(&workspace("", Some(text)).0, &BTreeMap::new());
        assert!(
            refusals.iter().any(|refusal| refusal.contains(defect)),
            "{text:?}: {refusals:?}"
        );
    }
}

#[test]
fn a_manifest_of_a_nested_workspace_or_with_no_package_declares_nothing() {
    let tree = workspace("", Some(EMPTY));
    for (path, text) in [
        (
            "fuzz/Cargo.toml",
            "[package]\nname = \"fuzz\"\n\n[features]\nf = []\n",
        ),
        ("crates/virtual/Cargo.toml", "[features]\nf = []\n"),
    ] {
        let path = tree.0.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a written file");
    }
    assert_eq!(check(&tree.0, &BTreeMap::new()), Vec::<String>::new());
}

#[test]
fn the_exclusions_of_the_workspace_parse_and_name_declared_features() {
    // The exclusions file of the workspace parses, and each line names a
    // feature a manifest declares. Whether a compile turns it on is the
    // gate's run to show, since it needs the lint's compiles.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let refusals = check(&root, &BTreeMap::new());
    assert!(
        refusals
            .iter()
            .all(|refusal| refusal.contains("no compile of the lint turns the feature on")),
        "{refusals:?}"
    );
}
