//! The `fixed-hash-key` feature of `ambre-core` reaches the instruction
//! benches alone (`ARCH-089`).
//!
//! The feature gives every process foldhash's fixed key, which any peer reads
//! from the foldhash source. A shipped build that carried it would let a peer
//! choose keys that collide. A feature turns on through a manifest or a cargo
//! command line, so this test reads both. No table of any workspace manifest
//! may name it, and of the files that hold the command lines of the tree, only
//! `scripts/instructions.py` may. The wasm32 half is a `compile_error!` in
//! `src/hash.rs`.

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::path::{Path, PathBuf};

/// The feature no shipped build enables.
const FEATURE: &str = "fixed-hash-key";

/// The one file whose command line may name the feature: the bench build of
/// the instruction counts.
const BENCH_SCRIPT: &str = "scripts/instructions.py";

/// The root of the workspace, two levels above this crate.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The text of `path`.
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// The root manifest, then the manifest of every workspace member.
fn manifests() -> Vec<PathBuf> {
    let root = workspace_root();
    let root_path = root.join("Cargo.toml");
    let parsed: toml::Table = read(&root_path)
        .parse()
        .unwrap_or_else(|err| panic!("parse {}: {err}", root_path.display()));
    let members: Vec<PathBuf> = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .expect("the root manifest lists its members")
        .iter()
        .filter_map(toml::Value::as_str)
        .map(|member| root.join(member).join("Cargo.toml"))
        .collect();
    assert!(
        members.len() > 1,
        "the walk must read the members, not only the root"
    );
    let mut all = vec![root_path];
    all.extend(members);
    all
}

/// Every place of one manifest that names the feature, as a line number and
/// its text. The declaration of the feature in `ambre-core`, `fixed-hash-key
/// = []`, is no place: it turns nothing on.
fn manifest_offences(text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = line.split('#').next().unwrap_or_default();
            code.contains(FEATURE) && code.trim() != format!("{FEATURE} = []")
        })
        .map(|(number, line)| format!("{}: {}", number + 1, line.trim()))
        .collect()
}

/// The files that hold a cargo command line of the tree. They are the
/// recipes, the CI, the shell wrappers, the build configuration and the scripts.
fn command_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        root.join("justfile"),
        root.join(".cargo/config.toml"),
        root.join("rust-toolchain.toml"),
    ];
    for dir in [".github/workflows", "scripts", "."] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        files.extend(
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| matches!(e, "yml" | "yaml" | "py" | "sh" | "mjs"))
                }),
        );
    }
    files.extend(
        manifests()
            .iter()
            .filter_map(|m| m.parent().map(|dir| dir.join("Trunk.toml"))),
    );
    files.retain(|p| p.is_file());
    files
}

/// Whether `rel` may name the feature. The bench script may, and so may the
/// tests of the scripts, which hold a breach to check that a gate goes red.
fn may_name(rel: &str) -> bool {
    rel == BENCH_SCRIPT || rel.starts_with("scripts/test_")
}

#[test]
fn the_fixed_hash_key_is_named_by_the_instruction_benches_alone() {
    let root = workspace_root();
    let mut refused = Vec::new();
    let mut declared = 0_usize;
    for path in manifests() {
        let text = read(&path);
        declared += text
            .lines()
            .filter(|line| line.trim() == format!("{FEATURE} = []"))
            .count();
        refused.extend(
            manifest_offences(&text)
                .into_iter()
                .map(|place| format!("{}:{place}", path.display())),
        );
    }
    let mut bench_names_it = false;
    for path in command_files(&root) {
        let rel = path
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().trim_start_matches("./").to_owned())
            .unwrap_or_default();
        if !read(&path).contains(FEATURE) {
            continue;
        }
        if rel == BENCH_SCRIPT {
            bench_names_it = true;
        } else if !may_name(&rel) {
            refused.push(format!("{rel}: names `{FEATURE}` on a command line"));
        }
    }
    assert!(
        refused.is_empty(),
        "the `{FEATURE}` feature of ambre-core can reach a shipped build: {refused:#?}"
    );
    // The walk saw the declaration and the one command line that names the
    // feature, so a walk that reads nothing cannot pass.
    assert_eq!(declared, 1, "ambre-core declares `{FEATURE}` once");
    assert!(bench_names_it, "`{BENCH_SCRIPT}` turns `{FEATURE}` on");
}

#[test]
fn a_manifest_that_turns_the_feature_on_is_refused() {
    let manifest = r#"
        [features]
        fixed-hash-key = []
        bench = ["ambre-core/fixed-hash-key"]
        default = ["fixed-hash-key"] # every build
        [dependencies]
        ambre-core = { path = "../ambre-core", features = ["fixed-hash-key"] }
        # fixed-hash-key in a comment turns nothing on
        [dev-dependencies]
        core = { package = "ambre-core", path = "../ambre-core", features = ["fixed-hash-key"] }
    "#;
    assert_eq!(
        manifest_offences(manifest),
        [
            r#"4: bench = ["ambre-core/fixed-hash-key"]"#,
            r#"5: default = ["fixed-hash-key"] # every build"#,
            r#"7: ambre-core = { path = "../ambre-core", features = ["fixed-hash-key"] }"#,
            r#"10: core = { package = "ambre-core", path = "../ambre-core", features = ["fixed-hash-key"] }"#,
        ]
    );
}
