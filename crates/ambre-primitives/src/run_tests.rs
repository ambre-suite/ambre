use std::path::Path;

use super::{compiles, unit_stem};

/// A `justfile` with the two lint recipes.
fn justfile(lint: &str, wasm: &str) -> String {
    format!("lint:\n{lint}\n\nlint-wasm:\n{wasm}\n\nother:\n    cargo test\n")
}

#[test]
fn the_lint_lines_read_as_cargo_check_with_their_flags() {
    let text = justfile(
        "    cargo fmt --all --check\n    cargo clippy --workspace --all-targets --features a/b -- -D warnings",
        "    # a comment\n    RUSTFLAGS=\"-D warnings\" cargo clippy --target wasm32-unknown-unknown --lib",
    );
    let lines = compiles(&text).expect("two compile lines");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].rustflags, None);
    assert_eq!(
        lines[0].args,
        ["--workspace", "--all-targets", "--features", "a/b"]
    );
    assert_eq!(lines[1].rustflags.as_deref(), Some("-D warnings"));
    assert_eq!(
        lines[1].args,
        ["--target", "wasm32-unknown-unknown", "--lib"]
    );
}

#[test]
fn a_line_the_gate_cannot_run_as_written_fails_closed() {
    for (line, why) in [
        (
            "    cargo clippy --config x=1",
            "changes what the gate reads",
        ),
        (
            "    cargo clippy -Zbuild-std",
            "changes what the gate reads",
        ),
        ("    cargo clippy $FLAGS", "a character the shell"),
        ("    cargo build", "no `cargo clippy` or `cargo fmt` line"),
        (
            "    RUSTFLAGS=\"$X\" cargo clippy",
            "asks the shell for an expansion",
        ),
    ] {
        let errors = compiles(&justfile(line, "    cargo clippy --lib")).expect_err(line);
        assert!(errors.iter().any(|e| e.contains(why)), "{line}: {errors:?}");
    }
}

#[test]
fn a_recipe_with_no_compile_line_or_a_parameter_fails_closed() {
    let errors = compiles(&justfile("    cargo fmt --all", "    cargo clippy --lib")).unwrap_err();
    assert!(
        errors.iter().any(|e| e.contains("holds no compile line")),
        "{errors:?}"
    );
    let errors = compiles("lint x:\n    cargo clippy\nlint-wasm:\n    cargo clippy\n").unwrap_err();
    assert!(
        errors.iter().any(|e| e.contains("takes a parameter")),
        "{errors:?}"
    );
}

// The lines of the workspace's own `justfile` parse: the gate runs them.
#[test]
fn the_lint_lines_of_the_workspace_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let text = std::fs::read_to_string(root.join(super::JUSTFILE)).expect("the justfile");
    let lines = compiles(&text).expect("the lint lines parse");
    assert!(lines.len() >= 2);
}

#[test]
fn a_build_script_is_read_under_the_record_the_driver_writes_for_it() {
    assert_eq!(
        unit_stem(Path::new(
            "/t/debug/build/ambre-sync-d96ed25e6a97cc7a/build-script-build"
        ))
        .as_deref(),
        Some("build_script_build-d96ed25e6a97cc7a")
    );
    assert_eq!(
        unit_stem(Path::new("/t/debug/deps/libambre_core-0123abcd.rlib")).as_deref(),
        Some("ambre_core-0123abcd")
    );
}
