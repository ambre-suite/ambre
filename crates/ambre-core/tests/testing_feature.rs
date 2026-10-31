//! The `testing` feature of `ambre-core` reaches test builds alone.
//!
//! Cargo enables the features of a `[dev-dependencies]` entry only when it
//! builds a test, a bench or an example. So the feature stays out of every
//! shipped build when no other table of any workspace manifest names it.
//! This test reads every manifest of the workspace and refuses such a table.

#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a test asserts by panicking (ARCH-028 holds outside the tests)"
)]

use std::path::{Path, PathBuf};

/// The feature a shipped build never enables.
const FEATURE: &str = "testing";
/// The crate that declares it.
const CRATE: &str = "ambre-core";

/// The root of the workspace, two levels above this crate.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One manifest, parsed.
fn manifest(path: &Path) -> toml::Table {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    text.parse::<toml::Table>()
        .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

/// The root manifest, then the manifest of every workspace member.
fn manifests() -> Vec<(PathBuf, toml::Table)> {
    let root = workspace_root();
    let root_path = root.join("Cargo.toml");
    let root_manifest = manifest(&root_path);
    let members: Vec<PathBuf> = root_manifest
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
    let mut all = vec![(root_path, root_manifest)];
    all.extend(members.into_iter().map(|path| {
        let parsed = manifest(&path);
        (path, parsed)
    }));
    all
}

/// Whether a table path is a dev-dependency table: `dev-dependencies`, or
/// `target.<cfg>.dev-dependencies`.
fn is_dev_table(path: &[String]) -> bool {
    path.iter().any(|segment| segment == "dev-dependencies")
}

/// Whether a dependency entry, keyed `key`, names `ambre-core`.
fn names_core(key: &str, entry: &toml::Value) -> bool {
    key == CRATE || entry.get("package").and_then(toml::Value::as_str) == Some(CRATE)
}

/// Whether a dependency entry turns the feature on.
fn enables_feature(entry: &toml::Value) -> bool {
    entry
        .get("features")
        .and_then(toml::Value::as_array)
        .is_some_and(|features| features.iter().any(|f| f.as_str() == Some(FEATURE)))
}

/// Whether a feature list forwards to the feature of `ambre-core`.
fn forwards_feature(list: &toml::Value) -> bool {
    let forwards = [format!("{CRATE}/{FEATURE}"), format!("{CRATE}?/{FEATURE}")];
    list.as_array().is_some_and(|items| {
        items
            .iter()
            .filter_map(toml::Value::as_str)
            .any(|item| forwards.iter().any(|forward| forward == item))
    })
}

/// Every place of one table tree that enables the feature outside a
/// dev-dependency table, as a dotted path.
fn offences(path: &mut Vec<String>, table: &toml::Table, out: &mut Vec<String>) {
    let in_features = path.last().is_some_and(|last| last == "features");
    let in_dependencies = path
        .last()
        .is_some_and(|last| last.ends_with("dependencies"));
    for (key, value) in table {
        if in_features && forwards_feature(value) {
            out.push(format!("{}.{key}", path.join(".")));
        }
        if in_dependencies
            && !is_dev_table(path)
            && names_core(key, value)
            && enables_feature(value)
        {
            out.push(format!("{}.{key}", path.join(".")));
        }
        if let Some(inner) = value.as_table() {
            path.push(key.clone());
            offences(path, inner, out);
            path.pop();
        }
    }
}

#[test]
fn the_testing_feature_is_named_in_dev_dependencies_alone() {
    // Resolver 1 unifies the features of a dev-dependency into the build
    // that ships, so the dev table alone keeps nothing out.
    let root = manifest(&workspace_root().join("Cargo.toml"));
    let resolver = root
        .get("workspace")
        .and_then(|workspace| workspace.get("resolver"))
        .and_then(toml::Value::as_str);
    assert!(
        matches!(resolver, Some(version) if version != "1"),
        "the workspace must name resolver 2 or later, found {resolver:?}"
    );
    let mut refused = Vec::new();
    let mut dev_uses = 0_usize;
    for (path, table) in manifests() {
        let mut found = Vec::new();
        offences(&mut Vec::new(), &table, &mut found);
        refused.extend(
            found
                .into_iter()
                .map(|place| format!("{}: {place}", path.display())),
        );
        let dev = table
            .get("dev-dependencies")
            .and_then(toml::Value::as_table);
        if dev.is_some_and(|dev| {
            dev.iter()
                .any(|(key, entry)| names_core(key, entry) && enables_feature(entry))
        }) {
            dev_uses += 1;
        }
    }
    // `ambre-core` declares the feature; its own default set must not hold it.
    let core = manifest(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let default_holds = core
        .get("features")
        .and_then(|features| features.get("default"))
        .and_then(toml::Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(FEATURE)));
    assert!(!default_holds, "the `testing` feature is a default feature");
    assert!(
        refused.is_empty(),
        "the `testing` feature of ambre-core reaches a shipped build: {refused:#?}"
    );
    // The walk saw the tables that do name it, so a parse that reads nothing
    // cannot pass.
    assert!(
        dev_uses > 0,
        "no dev-dependency names the `testing` feature"
    );
}

#[test]
fn a_forward_or_a_normal_dependency_is_refused() {
    let shipped: toml::Table = r#"
        [features]
        helpers = ["ambre-core/testing"]
        [dependencies]
        ambre-core = { path = "../ambre-core", features = ["testing"] }
        [target.'cfg(unix)'.dependencies]
        core-again = { package = "ambre-core", path = "../ambre-core", features = ["testing"] }
        [dev-dependencies]
        ambre-core = { path = "../ambre-core", features = ["testing"] }
    "#
    .parse()
    .expect("a manifest");
    let mut found = Vec::new();
    offences(&mut Vec::new(), &shipped, &mut found);
    found.sort();
    assert_eq!(
        found,
        [
            "dependencies.ambre-core",
            "features.helpers",
            "target.cfg(unix).dependencies.core-again",
        ]
    );
}
