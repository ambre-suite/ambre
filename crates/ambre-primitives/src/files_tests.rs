use std::collections::BTreeSet;
use std::path::PathBuf;

use super::check;

/// A tree written to a fresh directory, removed on drop.
struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A tree of `files`, each a path and its text.
fn tree(files: &[(&str, &str)]) -> Tree {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("ambre-files-{}-{n}", std::process::id()));
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("a written file");
    }
    Tree(std::fs::canonicalize(&root).expect("a root"))
}

/// The nested workspaces the gate names, each declaring one.
const NESTED: [(&str, &str); 2] = [
    ("fuzz/Cargo.toml", "[workspace]\n"),
    (
        "crates/ambre-primitives/driver/Cargo.toml",
        "[package]\nname = \"d\"\n[workspace]\n",
    ),
];

#[test]
fn a_rust_file_no_compile_reads_is_refused() {
    let mut files = NESTED.to_vec();
    files.extend([
        ("Cargo.toml", "[workspace]\n"),
        ("crates/demo/src/lib.rs", ""),
        ("crates/demo/src/orphan.rs", ""),
        ("fuzz/fuzz_targets/a.rs", ""),
        ("vendor/x/src/lib.rs", ""),
        ("target/debug/build/out.rs", ""),
        ("crates/ambre-primitives/driver/src/main.rs", ""),
    ]);
    let tree = tree(&files);
    let read: BTreeSet<PathBuf> = [tree.0.join("crates/demo/src/lib.rs")].into();
    let refusals = check(&tree.0, &read);
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(
        refusal.starts_with("crates/demo/src/orphan.rs: no compile of the lint reads this file"),
        "{refusal}"
    );
}

#[test]
fn a_stray_workspace_and_a_named_one_that_declares_none_are_refused() {
    let tree = tree(&[
        ("Cargo.toml", "[workspace]\n"),
        (
            "crates/stray/Cargo.toml",
            "[package]\nname = \"s\"\n[workspace]\n",
        ),
        ("fuzz/Cargo.toml", "[package]\nname = \"f\"\n"),
        NESTED[1],
    ]);
    let refusals = check(&tree.0, &BTreeSet::new());
    assert_eq!(refusals.len(), 2, "{refusals:?}");
    assert!(
        refusals
            .iter()
            .any(|r| r.starts_with("crates/stray/Cargo.toml: declares a workspace"))
    );
    assert!(
        refusals
            .iter()
            .any(|r| r.starts_with("fuzz: named as a nested workspace"))
    );
}

// The driver reads the crate by its name, so the rename only hides it from a
// reader; the rule keeps it refused (`ARCH-046`).
#[test]
fn a_renamed_zeroize_crate_is_refused() {
    let mut files = NESTED.to_vec();
    files.extend([
        ("Cargo.toml", "[workspace]\n"),
        (
            "crates/demo/Cargo.toml",
            "[package]\nname = \"demo\"\n[dependencies]\nwipe = { package = \"zeroize\", version = \"1\" }\nzeroize = \"1\"\n",
        ),
    ]);
    let tree = tree(&files);
    let refusals = check(&tree.0, &BTreeSet::new());
    let [refusal] = refusals.as_slice() else {
        panic!("one refusal expected: {refusals:?}");
    };
    assert!(
        refusal.contains("renames the zeroize crate as wipe"),
        "{refusal}"
    );
}
