//! Every feature the workspace manifests declare, held to the features the
//! compiles of the lint turn on (`ARCH-033`).
//!
//! The gate reads the files of the compiles it runs, the lines of `lint` and
//! `lint-wasm`. A feature no line turns on gates code that no compile reads,
//! so the gate reads none of it and clippy lints none of it. The fuzz entry
//! points of `ambre-sync` sat there until the orphan check refused their
//! file.
//!
//! The check reads what cargo turned on, never a model of it. cargo names the
//! features of each unit in its artifact message, and `depinfo::collect`
//! gathers them for the shipped builds of each package. So a feature counts
//! when a line names it, and when a feature or a dev-dependency the lines
//! reach turns it on. A weak `dep?/feature` counts only when another edge
//! turns the dependency on, and a test build alone counts for nothing.
//!
//! Each declared feature is turned on by a compile, or one line of the
//! exclusions file names it with its reason. A line that names no declared
//! feature fails. So does a line that names a feature a compile turns on:
//! the file only shrinks on its own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::allowlist::LineNo;
use crate::scan::{NESTED_WORKSPACES, Tree};

/// The exclusions file, relative to the workspace root.
pub(crate) const EXCLUSIONS: &str = "specs/1-Foundations/ambre-features.tsv";

/// The header line, the first line of the file that is not a comment.
const HEADER: &str = "feature\treason";

/// Each refusal of the feature check. `turned_on` maps each canonical
/// manifest to the features cargo turned on for a shipped build of its
/// package. `tree` reads the manifests under `root`, and the exclusions
/// file.
///
/// A manifest that does not parse as TOML is `depinfo::levers`'s to refuse,
/// so this check skips it. A manifest under a nested workspace builds apart,
/// and is left out.
pub(crate) fn check(
    tree: &impl Tree,
    root: &Path,
    turned_on: &BTreeMap<PathBuf, BTreeSet<String>>,
) -> Vec<String> {
    let none = BTreeSet::new();
    let declared = declared(tree, root);
    let mut refusals = Vec::new();
    let text = tree.read(&root.join(EXCLUSIONS));
    let excluded = match text.as_deref().map(parse) {
        Some(Ok(excluded)) => excluded,
        Some(Err(mut errors)) => {
            refusals.append(&mut errors);
            BTreeMap::new()
        }
        None => {
            refusals.push(format!(
                "{EXCLUSIONS}: the check cannot read it, so it cannot show which features no \
                 compile turns on by decision, and fails closed"
            ));
            BTreeMap::new()
        }
    };
    let mut known = BTreeSet::new();
    for (manifest, (package, features)) in &declared {
        let on = turned_on.get(manifest).unwrap_or(&none);
        for feature in features {
            let key = format!("{package}/{feature}");
            match (on.contains(feature), excluded.get(&key)) {
                (true, Some(line)) => refusals.push(format!(
                    "{EXCLUSIONS}:{line}: a compile of the lint turns on {key}, so the line \
                     excludes nothing. Remove it"
                )),
                (false, None) => refusals.push(format!(
                    "{key}: no compile of the lint turns the feature on, so the gate reads none \
                     of the code it gates and clippy lints none of it (ARCH-033). Name it on \
                     the host line of `lint` in the justfile, or add a line to {EXCLUSIONS} \
                     with the reason"
                )),
                (true, None) | (false, Some(_)) => {}
            }
            known.insert(key);
        }
    }
    for (key, line) in &excluded {
        if !known.contains(key) {
            refusals.push(format!(
                "{EXCLUSIONS}:{line}: no manifest of the workspace declares {key}, so the line \
                 is stale. Remove it"
            ));
        }
    }
    refusals
}

/// The package name and the declared features of each manifest under
/// `root`, by its canonical path. A manifest with no `[package]` name, one
/// that does not parse, and one under a nested workspace are left out.
fn declared(tree: &impl Tree, root: &Path) -> BTreeMap<PathBuf, (String, BTreeSet<String>)> {
    let nested: Vec<PathBuf> = NESTED_WORKSPACES.iter().map(|dir| root.join(dir)).collect();
    tree.files(root)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|path| !nested.iter().any(|dir| path.starts_with(dir)))
        .filter_map(|path| {
            let table = tree.read(&path)?.parse::<toml::Table>().ok()?;
            let package = table
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(toml::Value::as_str)?
                .to_owned();
            let features = table
                .get("features")
                .and_then(toml::Value::as_table)
                .map(|features| features.keys().cloned().collect())
                .unwrap_or_default();
            Some((tree.canonical(&path), (package, features)))
        })
        .collect()
}

/// The lines of the exclusions file by key, with their number. The format
/// breaks on five defects:
///
/// * a missing header;
/// * a line of other than two fields;
/// * a key that is not `package/feature`;
/// * an empty reason;
/// * a key that two lines carry.
fn parse(text: &str) -> Result<BTreeMap<String, LineNo>, Vec<String>> {
    let mut listed = BTreeMap::new();
    let mut problems = Vec::new();
    let mut header = false;
    for (i, line) in text.lines().enumerate() {
        let number = LineNo::new(i + 1);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !header {
            if line != HEADER {
                problems.push(format!(
                    "{EXCLUSIONS}:{number}: the header must read {HEADER:?}"
                ));
            }
            header = true;
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [key, reason] = fields.as_slice() else {
            problems.push(format!(
                "{EXCLUSIONS}:{number}: {} fields, want 2: feature, reason",
                fields.len()
            ));
            continue;
        };
        let parts: Vec<&str> = key.split('/').collect();
        if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
            problems.push(format!(
                "{EXCLUSIONS}:{number}: the feature must read package/feature, as the lint \
                 line names one"
            ));
            continue;
        }
        if reason.trim().is_empty() {
            problems.push(format!(
                "{EXCLUSIONS}:{number}: the reason is empty -- say why no compile of the lint \
                 turns the feature on"
            ));
            continue;
        }
        if listed.insert((*key).to_owned(), number).is_some() {
            problems.push(format!(
                "{EXCLUSIONS}:{number}: a second line for the feature {key}"
            ));
        }
    }
    if !header {
        problems.push(format!(
            "{EXCLUSIONS}: the file has no header line {HEADER:?}"
        ));
    }
    if problems.is_empty() {
        Ok(listed)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
