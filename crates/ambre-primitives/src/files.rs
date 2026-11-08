//! The file-level checks of the gate, which read the tree rather than a type
//! (`ARCH-033`, `ARCH-046`).
//!
//! * A `.rs` file under the workspace root fails when no compile of the gate
//!   reads it, test builds included. No file sits in the tree that no build
//!   checks. The check leaves out the `.git/` and `target/` of the root, the
//!   vendored crates and the nested workspaces of [`NESTED_WORKSPACES`]. Any
//!   other manifest that declares a workspace of its own fails, so a nested
//!   workspace is named here or refused.
//! * A manifest that renames the `zeroize` crate fails (`ARCH-046`). The
//!   driver reads the crate by its name, so a rename hides nothing from it;
//!   the rule keeps the name the reader greps for.
//!
//! The listing follows no link to a directory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The nested workspaces under the root, each built apart: the fuzz targets,
/// which `cargo fuzz` builds on its own, and the rustc driver of the gate.
pub const NESTED_WORKSPACES: &[&str] = &["fuzz", "crates/ambre-primitives/driver"];

/// The vendored crates, patched third-party code built as a dependency.
pub const VENDORED: &[&str] = &["vendor"];

/// The directories of the root no listing reads.
const SKIPPED: &[&str] = &[".git", "target"];

/// Every file under `dir`, links to a directory left out.
pub fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let skipped = entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIPPED.contains(&name));
                if !skipped {
                    pending.push(path);
                }
            } else if kind.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Whether `path` sits under one of `dirs` of `root`.
fn under(root: &Path, path: &Path, dirs: &[&str]) -> bool {
    dirs.iter().any(|dir| path.starts_with(root.join(dir)))
}

/// Each refusal of the file checks. `files` holds every file a compile read,
/// absolute, each `..` undone.
pub fn check(root: &Path, files: &BTreeSet<PathBuf>) -> Vec<String> {
    let mut refusals = Vec::new();
    let all = listing(root);
    for path in &all {
        let shown = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let skipped = under(root, path, NESTED_WORKSPACES) || under(root, path, VENDORED);
        if path.extension().is_some_and(|ext| ext == "rs") && !skipped && !files.contains(path) {
            refusals.push(format!(
                "{shown}: no compile of the lint reads this file, so no gate checks it (ARCH-033). \
                 Declare it in a module of a crate, or remove it"
            ));
        }
        if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            refusals.extend(manifest(root, path, &shown));
        }
    }
    for dir in NESTED_WORKSPACES {
        let declares = std::fs::read_to_string(root.join(dir).join("Cargo.toml"))
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .is_some_and(|table| table.contains_key("workspace"));
        if !declares {
            refusals.push(format!(
                "{dir}: named as a nested workspace, and its manifest declares none. Remove it \
                 from NESTED_WORKSPACES"
            ));
        }
    }
    refusals
}

/// The refusals of one manifest: a workspace of its own outside the root and
/// the nested workspaces, and a rename of the `zeroize` crate.
fn manifest(root: &Path, path: &Path, shown: &str) -> Vec<String> {
    let mut refusals = Vec::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return refusals;
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        refusals.push(format!("{shown}: does not parse as TOML"));
        return refusals;
    };
    let nested = NESTED_WORKSPACES
        .iter()
        .any(|dir| path == root.join(dir).join("Cargo.toml"));
    if table.contains_key("workspace") && path != root.join("Cargo.toml") && !nested {
        refusals.push(format!(
            "{shown}: declares a workspace of its own, which no compile of the lint reads. Name \
             it in NESTED_WORKSPACES with its reason, or remove the key"
        ));
    }
    let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut tables: Vec<&toml::Table> = sections
        .iter()
        .filter_map(|section| table.get(*section).and_then(toml::Value::as_table))
        .collect();
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target in targets.values().filter_map(toml::Value::as_table) {
            tables.extend(
                sections
                    .iter()
                    .filter_map(|section| target.get(*section).and_then(toml::Value::as_table)),
            );
        }
    }
    for deps in tables {
        for (name, value) in deps {
            let package = value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(name);
            if package == "zeroize" && name != "zeroize" {
                refusals.push(format!(
                    "{shown}: renames the zeroize crate as {name}. Name it zeroize, the name the \
                     Secret module alone writes (ARCH-046)"
                ));
            }
        }
    }
    refusals
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
