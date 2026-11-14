use super::*;

/// The metadata cargo would print for `packages`: each a name, its declared
/// dependencies as `(name, kind, optional)`, and its features.
type Dep<'a> = (&'a str, Option<&'a str>, bool);
type Pkg<'a> = (&'a str, &'a [Dep<'a>], &'a [(&'a str, &'a [&'a str])]);

fn graph(packages: &[Pkg<'_>]) -> Graph {
    let id = |name: &str| format!("id-{name}");
    let listed: Vec<serde_json::Value> = packages
        .iter()
        .map(|(name, deps, features)| {
            serde_json::json!({
                "id": id(name),
                "name": name,
                "manifest_path": format!("/ws/crates/{name}/Cargo.toml"),
                "dependencies": deps.iter().map(|(d, kind, optional)| serde_json::json!({
                    "name": d, "kind": kind, "features": [], "optional": optional,
                })).collect::<Vec<_>>(),
                "features": features.iter()
                    .map(|(k, v)| ((*k).to_owned(), serde_json::json!(v)))
                    .collect::<serde_json::Map<_, _>>(),
                "targets": [{"crate_types": ["lib"]}],
            })
        })
        .collect();
    let nodes: Vec<serde_json::Value> = packages
        .iter()
        .map(|(name, _, _)| serde_json::json!({"id": id(name), "deps": []}))
        .collect();
    let text = serde_json::json!({
        "packages": listed,
        "workspace_members": packages.iter().map(|(n, _, _)| id(n)).collect::<Vec<_>>(),
        "workspace_root": "/ws",
        "resolve": {"nodes": nodes},
    })
    .to_string();
    Graph::parse(&text).expect("fixture metadata")
}

/// The workspace as it stands: the worker and the call worker declare the
/// crypto crate, and the Logic crate declares it for its fixtures alone.
const HOLDERS: &[Pkg<'static>] = &[
    ("ambre-crypto", &[], &[]),
    ("ambre-sync", &[(CRYPTO, None, false)], &[]),
    ("ambre-call", &[(CRYPTO, None, false)], &[]),
    (
        "ambre-logic",
        &[(CRYPTO, None, true)],
        &[("fixtures", &["dep:ambre-crypto"])],
    ),
];

#[test]
fn the_holders_and_the_fixture_only_declaration_pass() {
    assert_eq!(manifests(&graph(HOLDERS)), Vec::<String>::new());
}

// The scan read a line that starts `ambre-crypto =`. A renamed dependency
// (`hash = { package = "ambre-crypto" }`) passed it; cargo names the package.
// A test table of the chrome passed it too when its line was indented.
#[test]
fn the_crypto_crate_declared_by_the_chrome_or_the_launcher_is_refused() {
    let mut packages = HOLDERS.to_vec();
    packages.push(("ambre-chrome", &[(CRYPTO, Some("dev"), false)], &[]));
    packages.push(("ambre-app", &[(CRYPTO, None, false)], &[]));
    let refusals = manifests(&graph(&packages));
    assert_eq!(refusals.len(), 2, "{refusals:#?}");
    assert!(refusals.iter().all(|r| r.contains("CHROME-049")));
    assert!(refusals.iter().any(|r| r.contains("ambre-chrome declares")));
    assert!(refusals.iter().any(|r| r.contains("ambre-app declares")));
}

// The scan let the Logic crate through on `optional = true` alone. A second
// feature that turns the crate on would put it in a build the client takes.
#[test]
fn the_logic_crate_beyond_its_fixtures_is_refused() {
    let packages: &[Pkg<'_>] = &[
        ("ambre-sync", &[(CRYPTO, None, false)], &[]),
        (
            "ambre-logic",
            &[(CRYPTO, None, true), (CRYPTO, Some("dev"), false)],
            &[
                ("fixtures", &["dep:ambre-crypto"]),
                ("hashing", &["ambre-crypto?/std"]),
            ],
        ),
    ];
    let refusals = manifests(&graph(packages));
    assert_eq!(refusals.len(), 2, "{refusals:#?}");
    assert!(refusals.iter().any(|r| r.contains("ambre-logic declares")));
    assert!(refusals.iter().any(|r| r.contains("`hashing`")));
}

#[test]
fn an_open_time_create_no_unit_records_fails_closed() {
    let mut records = Records::default();
    assert_eq!(facts(&records).len(), 1);
    records.add(&format!(
        "unit\tambre-sync\tambre_sync\tbuild\n\
         fact\t{LAZY_CREATE}\tNone\tcrates/ambre-sync/src/updates.rs:1470\n"
    ));
    assert_eq!(facts(&records), Vec::<String>::new());
}
