use super::*;

/// One dependency of a fixture package: the package, its kind, its features.
type Dep<'a> = (&'a str, Option<&'a str>, &'a [&'a str]);

/// A fixture package: its name, its dependencies, its features, and whether
/// it builds a cdylib.
struct Fixture<'a> {
    name: &'a str,
    deps: &'a [Dep<'a>],
    features: &'a [(&'a str, &'a [&'a str])],
    cdylib: bool,
}

/// A package with the dependencies `deps` and nothing else.
fn pkg<'a>(name: &'a str, deps: &'a [Dep<'a>]) -> Fixture<'a> {
    Fixture {
        name,
        deps,
        features: &[],
        cdylib: name == "ambre-sync",
    }
}

/// The members every boundary reads, each with no dependency.
const MEMBERS: &[&str] = &[
    "ambre-core",
    "ambre-app",
    "ambre-chrome",
    "ambre-sync",
    "ambre-logic",
    "ambre-canvas",
    "ambre-sfu",
];

/// The metadata cargo would print for the members of [`MEMBERS`] and the
/// packages `extra`, a package of `extra` replacing the member of its name.
/// A name that starts with `ambre-` is a member. Each declared dependency on
/// a package of the fixture is a resolved edge.
fn metadata(extra: &[Fixture<'_>]) -> String {
    let mut all: Vec<&Fixture<'_>> = extra.iter().collect();
    let bare: Vec<Fixture<'_>> = MEMBERS
        .iter()
        .filter(|m| !extra.iter().any(|e| e.name == **m))
        .map(|m| pkg(m, &[]))
        .collect();
    all.extend(bare.iter());
    let id = |name: &str| format!("id-{name}");
    let packages: Vec<Value> = all
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": id(p.name),
                "name": p.name,
                "manifest_path": format!("/ws/crates/{}/Cargo.toml", p.name),
                "dependencies": p.deps.iter().map(|(name, kind, features)| serde_json::json!({
                    "name": name, "kind": kind, "features": features,
                })).collect::<Vec<_>>(),
                "features": p.features.iter().map(|(k, v)| ((*k).to_owned(), serde_json::json!(v)))
                    .collect::<serde_json::Map<_, _>>(),
                "targets": [{"crate_types": if p.cdylib { vec!["cdylib", "rlib"] } else { vec!["lib"] }}],
            })
        })
        .collect();
    let nodes: Vec<Value> = all
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": id(p.name),
                "deps": p.deps.iter()
                    .filter(|(name, _, _)| all.iter().any(|q| q.name == *name))
                    .map(|(name, kind, _)| serde_json::json!({
                        "pkg": id(name), "dep_kinds": [{"kind": kind}],
                    })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let members: Vec<String> = all
        .iter()
        .filter(|p| p.name.starts_with("ambre-"))
        .map(|p| id(p.name))
        .collect();
    serde_json::json!({
        "packages": packages,
        "workspace_members": members,
        "workspace_root": "/ws",
        "resolve": {"nodes": nodes},
    })
    .to_string()
}

/// A manifest that inherits the lint table, for each member, and a root
/// manifest that declares it.
fn manifest(path: &str) -> Option<String> {
    Some(if path == "/ws/Cargo.toml" {
        "[workspace.lints.rust]\nunsafe_code = \"forbid\"\n".to_owned()
    } else {
        "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n".to_owned()
    })
}

/// The refusals of the manifest boundaries over `extra`.
fn refusals(extra: &[Fixture<'_>]) -> Vec<String> {
    let graph = Graph::parse(&metadata(extra)).expect("fixture metadata");
    manifests(&graph, &manifest)
}

/// The refusals that cite `row`.
fn citing(refusals: &[String], row: &str) -> Vec<String> {
    refusals
        .iter()
        .filter(|r| r.contains(row))
        .cloned()
        .collect()
}

#[test]
fn a_clean_graph_passes_every_manifest_boundary() {
    assert_eq!(refusals(&[]), Vec::<String>::new());
}

// The client turns the fixture feature on through a crate it links, and
// through a feature of its own. Neither line sits in a dependency table of
// the client (`FIXTURE-007`).
#[test]
fn the_fixture_feature_reached_through_a_linked_crate_or_a_client_feature_is_refused() {
    let found = refusals(&[
        pkg("ambre-app", &[("ambre-commands", None, &[])]),
        pkg("ambre-commands", &[("ambre-logic", None, &["fixtures"])]),
    ]);
    assert_eq!(citing(&found, "FIXTURE-007").len(), 1, "{found:#?}");
    let found = refusals(&[Fixture {
        name: "ambre-app",
        deps: &[("ambre-logic", None, &[])],
        features: &[("debug", &["ambre-logic/fixtures"])],
        cdylib: false,
    }]);
    assert_eq!(citing(&found, "FIXTURE-007").len(), 1, "{found:#?}");
    let found = refusals(&[Fixture {
        name: "ambre-logic",
        deps: &[],
        features: &[("default", &["ambre-core?/fixtures"])],
        cdylib: false,
    }]);
    assert_eq!(citing(&found, "FIXTURE-007").len(), 1, "{found:#?}");
}

// The chrome reaches an application crate through a crate of its own
// (`SCHEMA-002`).
#[test]
fn the_chrome_reaching_an_application_through_another_crate_is_refused() {
    let found = refusals(&[
        pkg("ambre-chrome", &[("ambre-commands", None, &[])]),
        pkg("ambre-commands", &[("ambre-trame", None, &[])]),
        pkg("ambre-trame", &[]),
    ]);
    assert_eq!(citing(&found, "SCHEMA-002").len(), 1, "{found:#?}");
}

// A dependency on the worker under a rename, from a test table, and a worker
// that builds no cdylib (`ARCH-001`, `SYNC-056`).
#[test]
fn a_dependency_on_the_worker_and_a_worker_with_no_artefact_are_refused() {
    // `cargo metadata` names the package a renamed dependency takes.
    let found = refusals(&[pkg("ambre-app", &[("ambre-sync", Some("dev"), &[])])]);
    assert_eq!(citing(&found, "ARCH-001").len(), 1, "{found:#?}");
    let found = refusals(&[Fixture {
        name: "ambre-sync",
        deps: &[],
        features: &[],
        cdylib: false,
    }]);
    assert_eq!(citing(&found, "SYNC-056").len(), 1, "{found:#?}");
}

// The Logic runtime declares the toolkit (`ARCH-003`). The worker reaches the
// render stack through a crate of the ecosystem, which no walk of the path
// dependencies reads (`SYNC-096`, `ARCH-070`).
#[test]
fn a_thread_reaching_the_rendering_through_any_crate_is_refused() {
    let found = refusals(&[
        pkg("ambre-logic", &[("leptos", None, &[])]),
        pkg("leptos", &[]),
    ]);
    assert_eq!(citing(&found, "ARCH-003").len(), 1, "{found:#?}");
    let found = refusals(&[
        pkg("ambre-sync", &[("gpu-helper", None, &[])]),
        pkg("gpu-helper", &[("wgpu", None, &[])]),
        pkg("wgpu", &[]),
    ]);
    let sync = citing(&found, "SYNC-096");
    assert_eq!(sync.len(), 1, "{found:#?}");
    assert!(sync[0].contains("wgpu through gpu-helper"), "{sync:#?}");
    // A test table of the worker links nothing into it.
    let found = refusals(&[
        pkg("ambre-sync", &[("ambre-trame", Some("dev"), &[])]),
        pkg("ambre-trame", &[]),
    ]);
    assert_eq!(citing(&found, "ARCH-070"), Vec::<String>::new());
}

// The renderer reaches the bus from a test table of its own, and through a
// crate it links (`ARCH-071`).
#[test]
fn the_renderer_reaching_the_bus_is_refused() {
    let found = refusals(&[
        pkg(
            "ambre-canvas",
            &[("ambre-bus", Some("dev"), &[]), ("ambre-draw", None, &[])],
        ),
        pkg("ambre-draw", &[("ambre-bus", None, &[])]),
        pkg("ambre-bus", &[]),
    ]);
    assert_eq!(citing(&found, "ARCH-071").len(), 2, "{found:#?}");
}

// A transport crate declared by a client crate, a build table included, and
// by the worker (`ARCH-005`).
#[test]
fn a_transport_crate_is_refused_outside_the_network_crates() {
    let found = refusals(&[
        pkg("ambre-app", &[("reqwest", Some("build"), &[])]),
        pkg("ambre-sync", &[("gloo-net", None, &[])]),
    ]);
    let network = citing(&found, "ARCH-005");
    assert_eq!(network.len(), 1, "{found:#?}");
    assert!(network[0].contains("ambre-app declares the transport crate reqwest"));
}

// The SFU reaches a C library through a test table and an edge of the
// ecosystem (`CALL-013`).
#[test]
fn a_c_cryptography_library_under_the_sfu_is_refused() {
    let found = refusals(&[
        pkg("ambre-sfu", &[("tls", Some("dev"), &[])]),
        pkg("tls", &[("ring", None, &[])]),
        pkg("ring", &[]),
    ]);
    assert_eq!(citing(&found, "CALL-013").len(), 1, "{found:#?}");
}

// A member that does not inherit the table is refused, and the dotted form of
// the key is read as cargo reads it (`ARCH-032`).
#[test]
fn a_member_that_does_not_inherit_the_lint_table_is_refused() {
    let graph = Graph::parse(&metadata(&[])).expect("fixture metadata");
    let read = |path: &str| -> Option<String> {
        if path.ends_with("ambre-sfu/Cargo.toml") {
            Some("[package]\nname = \"ambre-sfu\"\n".to_owned())
        } else if path.ends_with("ambre-core/Cargo.toml") {
            Some("lints.workspace = true\n".to_owned())
        } else {
            manifest(path)
        }
    };
    let found = manifests(&graph, &read);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(
        found[0].contains("ambre-sfu does not inherit"),
        "{found:#?}"
    );
    let found = manifests(&graph, &|path: &str| {
        (path != "/ws/Cargo.toml").then(|| manifest(path)).flatten()
    });
    assert_eq!(citing(&found, "[workspace.lints]").len(), 1, "{found:#?}");
}

// A member the boundaries read is gone: they fail closed.
#[test]
fn a_member_gone_fails_closed() {
    let mut json: Value = serde_json::from_str(&metadata(&[])).expect("fixture metadata");
    json["workspace_members"] = serde_json::json!(["id-ambre-core"]);
    let graph = Graph::parse(&json.to_string()).expect("fixture metadata");
    let found = manifests(&graph, &manifest);
    assert!(
        found.iter().any(|r| r.starts_with("ambre-sync: no member")),
        "{found:#?}"
    );
}

/// The records of the declarations `decls`.
fn declared(decls: &[(&str, &str, &str, &str)]) -> Records {
    let mut records = Records::default();
    for (krate, name, public, at) in decls {
        records.add(&format!("decl\t{krate}\t{name}\t{public}\t{at}"));
    }
    records
}

// The worker redeclares a wire type, private or not, and is refused. A
// private type of the protocol is no wire type. A protocol with no wire type
// fails closed (`ARCH-006`).
#[test]
fn a_wire_type_redeclared_by_the_worker_or_the_relay_is_refused() {
    let records = declared(&[
        ("ambre-protocol", "Hello", "public", "p.rs:1"),
        ("ambre-protocol", "Inner", "private", "p.rs:2"),
        ("ambre-sync", "Hello", "private", "s.rs:4"),
        ("ambre-server", "Inner", "public", "r.rs:9"),
        ("ambre-app", "Hello", "public", "a.rs:1"),
    ]);
    assert_eq!(
        wire(&records),
        vec![
            "s.rs:4: ambre-sync redeclares the wire type Hello: take it from ambre-protocol (ARCH-006)"
        ]
    );
    assert_eq!(wire(&declared(&[])).len(), 1);
}
