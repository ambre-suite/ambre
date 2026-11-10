//! The crate boundaries a manifest holds, read from `cargo metadata`, and the
//! one wire definition, read from the declarations the driver records
//! (`ambre-spec-architecture.md` section 2).
//!
//! cargo resolves the graph. A renamed dependency, a target table and a
//! member a glob lists read as cargo reads them. An edge through a crate of
//! the ecosystem counts too. The check never parses a manifest line.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | the client turns on no fixture feature, and no crate makes it a default | `FIXTURE-007` |
//! | the shared chrome reaches no application crate | `SCHEMA-002` |
//! | the Sync worker builds an artefact of its own, and no crate depends on it | `ARCH-001`, `SYNC-056` |
//! | the Logic runtime declares no render runtime and no toolkit | `ARCH-003` |
//! | the Sync worker reaches no rendering and no application | `SYNC-096`, `ARCH-070` |
//! | the renderer reaches no protocol between the threads | `ARCH-071` |
//! | a transport crate is declared by the network crates alone | `ARCH-005` |
//! | every member inherits the workspace lint table | `ARCH-032` |
//! | the SFU links no C cryptography library | `CALL-013` |
//! | no crate depends on the SFU, which alone holds the TURN secret | `CALL-014` |
//! | the worker and the relay redeclare no wire type | `ARCH-006` |
//! | the launcher starts the Logic runtime once | `ARCH-010` |
//! | a frame loop runs a relay that drains the Logic to Sync queue | `ARCH-007` |
//! | a panic and a failed worker each latch the fatal screen | `ARCH-014` |
//! | the raw Logic sender moves into the outbox, which the relay flushes | `BUS-035`, `BUS-036` |

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::records::Records;

/// The feature that carries the fixture machinery (`FIXTURE-007`).
const FIXTURES: &str = "fixtures";

/// The crate built into the wasm artefact of the launcher.
const CLIENT: &str = "ambre-app";

/// The crates whose type a shared control could emit (`SCHEMA-002`).
const APP_CRATES: &[&str] = &[
    "ambre-bus",
    "ambre-trame",
    "ambre-toile",
    "ambre-prose",
    "ambre-murmure",
    "ambre-logic",
];

/// The render runtime and the interface toolkit (`ARCH-003`).
const RENDER_OR_TOOLKIT: &[&str] = &[
    "ambre-canvas",
    "ambre-chrome",
    "ambre-app",
    "wgpu",
    "vello",
    "leptos",
];

/// What the Sync worker reaches by no edge: the rendering, and every crate
/// that declares or drives an application (`SYNC-096`, `ARCH-070`).
const SYNC_UNREACHABLE: &[&str] = &[
    "ambre-canvas",
    "ambre-chrome",
    "ambre-app",
    "wgpu",
    "vello",
    "leptos",
    "ambre-draw",
    "ambre-crdt",
    "ambre-trame",
    "ambre-toile",
    "ambre-prose",
    "ambre-murmure",
    "ambre-logic",
];

/// The crates a network call may leave from (`ARCH-005`).
const NETWORK_CRATES: &[&str] = &["ambre-sync", "ambre-server", "ambre-sfu", "ambre-belt"];

/// The crates that open a connection (`ARCH-005`).
const TRANSPORT_CRATES: &[&str] = &[
    "gloo-net",
    "reqwest",
    "tungstenite",
    "tokio-tungstenite",
    "ewebsock",
    "ws_stream_wasm",
];

/// The C cryptography libraries a Rust crate can link (`CALL-013`).
const C_CRYPTO: &[&str] = &[
    "aws-lc-sys",
    "aws-lc-fips-sys",
    "ring",
    "openssl-sys",
    "boring-sys",
];

/// The crates whose types the driver records, with the one that defines the
/// wire (`ARCH-006`).
const WIRE: &str = "ambre-protocol";
const WIRE_READERS: &[&str] = &["ambre-sync", "ambre-server"];

/// Which edges a walk follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edges {
    /// What the crate links: its normal dependencies.
    Linked,
    /// Every edge cargo resolved: normal, build and dev.
    All,
}

/// One dependency a manifest declares, as cargo reads it.
struct Declared {
    /// The package it names, a rename undone.
    name: String,
    /// `None` for a normal dependency, `dev` or `build`.
    kind: Option<String>,
    features: Vec<String>,
}

/// One package of the graph.
struct Package {
    name: String,
    manifest: String,
    declared: Vec<Declared>,
    features: BTreeMap<String, Vec<String>>,
    /// The crate types of each target.
    crate_types: Vec<String>,
}

/// The graph cargo resolved, read from its metadata.
pub(crate) struct Graph {
    packages: BTreeMap<String, Package>,
    /// The ids of the workspace members.
    members: Vec<String>,
    /// The root manifest of the workspace.
    root: String,
    /// Each resolved edge: the target id and whether it is linked.
    edges: BTreeMap<String, Vec<(String, bool)>>,
}

impl Graph {
    /// The graph of `cargo metadata --format-version 1`.
    ///
    /// # Errors
    ///
    /// A message when the text is no metadata the check can read.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| format!("cargo metadata is no JSON: {e}"))?;
        let strs = |v: &Value| -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut packages = BTreeMap::new();
        for p in at(&value, "packages")
            .as_array()
            .ok_or("cargo metadata lists no package")?
        {
            let id = at(p, "id")
                .as_str()
                .ok_or("a package with no id")?
                .to_owned();
            let declared = at(p, "dependencies")
                .as_array()
                .map(|deps| {
                    deps.iter()
                        .map(|d| Declared {
                            name: at(d, "name").as_str().unwrap_or_default().to_owned(),
                            kind: at(d, "kind").as_str().map(str::to_owned),
                            features: strs(at(d, "features")),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let features = at(p, "features")
                .as_object()
                .map(|f| f.iter().map(|(k, v)| (k.clone(), strs(v))).collect())
                .unwrap_or_default();
            let crate_types = at(p, "targets")
                .as_array()
                .map(|ts| ts.iter().flat_map(|t| strs(at(t, "crate_types"))).collect())
                .unwrap_or_default();
            packages.insert(
                id,
                Package {
                    name: at(p, "name").as_str().unwrap_or_default().to_owned(),
                    manifest: at(p, "manifest_path")
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    declared,
                    features,
                    crate_types,
                },
            );
        }
        let members = strs(at(&value, "workspace_members"));
        let mut edges = BTreeMap::new();
        for node in at(at(&value, "resolve"), "nodes")
            .as_array()
            .ok_or("cargo metadata holds no resolved graph")?
        {
            let id = at(node, "id").as_str().unwrap_or_default().to_owned();
            let out = at(node, "deps")
                .as_array()
                .map(|deps| {
                    deps.iter()
                        .map(|d| {
                            let linked = at(d, "dep_kinds")
                                .as_array()
                                .is_some_and(|ks| ks.iter().any(|k| at(k, "kind").is_null()));
                            (at(d, "pkg").as_str().unwrap_or_default().to_owned(), linked)
                        })
                        .collect()
                })
                .unwrap_or_default();
            edges.insert(id, out);
        }
        let root = Path::new(at(&value, "workspace_root").as_str().unwrap_or_default())
            .join("Cargo.toml")
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            packages,
            members,
            root,
            edges,
        })
    }

    /// The name of the package `id`.
    fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.packages.get(id).map_or(id, |p| p.name.as_str())
    }

    /// The workspace member named `name`.
    fn member(&self, name: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|id| self.name(id) == name)
            .map(String::as_str)
    }

    /// Every package `from` reaches over `edges`, `from` included.
    fn closure(&self, from: &[&str], edges: Edges) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<String> = from.iter().map(|id| (*id).to_owned()).collect();
        while let Some(id) = todo.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            for (to, linked) in self.edges.get(&id).into_iter().flatten() {
                if *linked || edges == Edges::All {
                    todo.push(to.clone());
                }
            }
        }
        seen
    }

    /// The direct dependencies of `id` cargo resolved, linked or not.
    fn direct(&self, id: &str) -> Vec<&str> {
        self.edges
            .get(id)
            .into_iter()
            .flatten()
            .map(|(to, _)| to.as_str())
            .collect()
    }

    /// Each linked edge from a package of `closure` to a package named in
    /// `names`, as `(from, to)`.
    fn reached<'a>(
        &'a self,
        closure: &'a BTreeSet<String>,
        names: &[&str],
    ) -> Vec<(&'a str, &'a str)> {
        let mut found = Vec::new();
        for id in closure {
            for (to, linked) in self.edges.get(id).into_iter().flatten() {
                if *linked && names.contains(&self.name(to)) {
                    found.push((self.name(id), self.name(to)));
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// The field `key` of a JSON object, or null.
fn at<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// Runs `cargo metadata` at `root`.
///
/// # Errors
///
/// A message when cargo does not run or fails.
pub(crate) fn metadata(root: &Path) -> Result<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--locked"])
        .output()
        .map_err(|e| format!("cargo metadata does not run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata fails, so the crate boundaries read nothing: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether a feature entry turns on the fixture feature: `fixtures`,
/// `dep/fixtures` or `dep?/fixtures`.
fn fixture_entry(entry: &str) -> bool {
    entry.rsplit('/').next() == Some(FIXTURES)
}

/// Each refusal of the manifest boundaries, over the graph and the manifests
/// each package names. `lints` reads a manifest as TOML: the metadata of
/// cargo carries no lint table.
pub(crate) fn manifests(graph: &Graph, read: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut refusals = Vec::new();
    let mut need = |name: &str| -> Option<String> {
        let id = graph.member(name);
        if id.is_none() {
            refusals.push(format!(
                "{name}: no member of the workspace, so the boundaries that read it hold \
                 nothing, and fail closed"
            ));
        }
        id.map(str::to_owned)
    };
    let client = need(CLIENT);
    let chrome = need("ambre-chrome");
    let sync = need("ambre-sync");
    let call = need("ambre-call");
    let logic = need("ambre-logic");
    let canvas = need("ambre-canvas");
    let sfu = need("ambre-sfu");

    // FIXTURE-007: no default holds the fixture feature, and no edge the
    // client links, nor a test table of the client, turns it on.
    for id in &graph.members {
        let Some(p) = graph.packages.get(id) else {
            continue;
        };
        if p.features
            .get("default")
            .is_some_and(|d| d.iter().any(|e| fixture_entry(e)))
        {
            refusals.push(format!(
                "{}: `{FIXTURES}` is in the default feature set of {}, so every build that \
                 takes the crate takes the fixture machinery (FIXTURE-007)",
                p.manifest, p.name
            ));
        }
    }
    if let Some(client) = &client {
        let mut packages: Vec<(&Package, bool)> = graph
            .closure(&[client], Edges::Linked)
            .iter()
            .filter(|id| graph.members.contains(id))
            .filter_map(|id| graph.packages.get(id).map(|p| (p, id == client)))
            .collect();
        packages.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        for (p, own) in packages {
            for d in &p.declared {
                let linked = d.kind.is_none() || d.kind.as_deref() == Some("build");
                if (linked || own) && d.features.iter().any(|f| f == FIXTURES) {
                    refusals.push(format!(
                        "{}: {} turns on `{FIXTURES}` of {} on an edge the client reaches \
                         (FIXTURE-007)",
                        p.manifest, p.name, d.name
                    ));
                }
            }
            if own {
                for (feature, entries) in &p.features {
                    if entries.iter().any(|e| fixture_entry(e)) {
                        refusals.push(format!(
                            "{}: the client feature `{feature}` turns on `{FIXTURES}` \
                             (FIXTURE-007)",
                            p.manifest
                        ));
                    }
                }
            }
        }
    }

    // SCHEMA-002: the shared chrome reaches no crate an application's style
    // type could come from, by a test table or by a linked edge.
    if let Some(chrome) = &chrome {
        let mut from: Vec<&str> = graph.direct(chrome);
        from.push(chrome);
        let closure = graph.closure(&from, Edges::Linked);
        for id in &closure {
            if APP_CRATES.contains(&graph.name(id)) {
                refusals.push(format!(
                    "ambre-chrome reaches {}, so a shared control could emit an application's \
                     type (SCHEMA-002)",
                    graph.name(id)
                ));
            }
        }
    }

    // ARCH-001, SYNC-056: each worker is an artefact of its own, and no crate
    // merges it into another. The call worker is the third instance, which a call
    // spawns (CALL-004).
    for (worker, (named, rows)) in [&sync, &call].into_iter().zip([
        ("the Sync worker", "ARCH-001, SYNC-056"),
        ("the call worker", "ARCH-001, CALL-004"),
    ]) {
        let Some(p) = worker.as_ref().and_then(|id| graph.packages.get(id)) else {
            continue;
        };
        let name = p.name.clone();
        if !p.crate_types.iter().any(|t| t == "cdylib") {
            refusals.push(format!(
                "{}: {name} builds no cdylib, so {named} is no artefact of its own ({rows})",
                p.manifest
            ));
        }
        for id in &graph.members {
            if let Some(q) = graph.packages.get(id)
                && q.name != name
                && q.declared.iter().any(|d| d.name == name)
            {
                refusals.push(format!(
                    "{}: {} depends on {named}, which merges it into another instance ({rows})",
                    q.manifest, q.name
                ));
            }
        }
    }

    // ARCH-003: the Logic runtime declares no render runtime and no toolkit.
    if let Some(p) = logic.as_ref().and_then(|id| graph.packages.get(id)) {
        for d in &p.declared {
            if d.kind.as_deref() != Some("dev") && RENDER_OR_TOOLKIT.contains(&d.name.as_str()) {
                refusals.push(format!(
                    "{}: the Logic runtime declares {} (ARCH-003)",
                    p.manifest, d.name
                ));
            }
        }
    }

    // SYNC-096, ARCH-070: nothing the worker links reaches the rendering or
    // an application, whichever crate declares the edge.
    if let Some(sync) = &sync {
        let closure = graph.closure(&[sync], Edges::Linked);
        for (from, to) in graph.reached(&closure, SYNC_UNREACHABLE) {
            refusals.push(format!(
                "the Sync worker reaches {to} through {from} (SYNC-096, ARCH-070)"
            ));
        }
    }

    // ARCH-071: the renderer reaches no protocol between the threads, by a
    // test table of its own or by a linked edge.
    if let Some(canvas) = &canvas {
        for to in graph.direct(canvas) {
            if graph.name(to) == "ambre-bus" {
                refusals.push("ambre-canvas declares ambre-bus (ARCH-071)".to_owned());
            }
        }
        let closure = graph.closure(&graph.direct(canvas), Edges::Linked);
        for (from, to) in graph.reached(&closure, &["ambre-bus"]) {
            refusals.push(format!(
                "the renderer reaches {to} through {from} (ARCH-071)"
            ));
        }
    }

    // ARCH-005: a transport crate is declared by the network crates alone.
    for id in &graph.members {
        let Some(p) = graph.packages.get(id) else {
            continue;
        };
        if NETWORK_CRATES.contains(&p.name.as_str()) {
            continue;
        }
        for d in &p.declared {
            if TRANSPORT_CRATES.contains(&d.name.as_str()) {
                refusals.push(format!(
                    "{}: {} declares the transport crate {} (ARCH-005)",
                    p.manifest, p.name, d.name
                ));
            }
        }
    }

    // CALL-013: no edge cargo resolved from the SFU reaches a C library.
    if let Some(sfu) = &sfu {
        for id in graph.closure(&[sfu], Edges::All) {
            if C_CRYPTO.contains(&graph.name(&id)) {
                refusals.push(format!(
                    "ambre-sfu reaches {}, a C cryptography library (CALL-013)",
                    graph.name(&id)
                ));
            }
        }
    }

    // CALL-014: the SFU holds the secret that mints the TURN credentials, in its own module.
    // No crate depends on it, so the relay can name neither that secret nor its mint.
    for id in &graph.members {
        if let Some(p) = graph.packages.get(id)
            && p.name != "ambre-sfu"
            && p.declared.iter().any(|d| d.name == "ambre-sfu")
        {
            refusals.push(format!(
                "{}: {} depends on the SFU, which holds the TURN secret (CALL-014)",
                p.manifest, p.name
            ));
        }
    }

    // ARCH-032: the root carries the lint table, and every member inherits it.
    let table = |text: &str| text.parse::<toml::Table>().ok();
    let root_lints = read(&graph.root)
        .and_then(|t| table(&t))
        .is_some_and(|t| t.get("workspace").and_then(|w| w.get("lints")).is_some());
    if !root_lints {
        refusals.push(
            "Cargo.toml: no [workspace.lints] table, so a crate has nothing to inherit (ARCH-032)"
                .to_owned(),
        );
    }
    for id in &graph.members {
        let Some(p) = graph.packages.get(id) else {
            continue;
        };
        let inherits = read(&p.manifest).and_then(|t| table(&t)).is_some_and(|t| {
            t.get("lints")
                .and_then(|l| l.get("workspace"))
                .and_then(toml::Value::as_bool)
                == Some(true)
        });
        if !inherits {
            refusals.push(format!(
                "{}: {} does not inherit the workspace lints: `[lints]` `workspace = true` \
                 (ARCH-032)",
                p.manifest, p.name
            ));
        }
    }
    refusals
}

/// Each type, alias or trait the worker or the relay declares under the name
/// of a public type of the protocol crate (`ARCH-006`). The driver records
/// every declaration of the three crates, a private one and one a macro
/// writes included.
pub(crate) fn wire(records: &Records) -> Vec<String> {
    let defined: BTreeSet<&str> = records
        .decls
        .iter()
        .filter(|(krate, _, public, _)| krate == WIRE && public == "public")
        .map(|(_, name, _, _)| name.as_str())
        .collect();
    let mut refusals = Vec::new();
    if defined.is_empty() {
        refusals.push(format!(
            "{WIRE}: the driver records no public type of it, so the one wire definition holds \
             nothing, and fails closed (ARCH-006)"
        ));
    }
    for (krate, name, _, at) in &records.decls {
        if WIRE_READERS.contains(&krate.as_str()) && defined.contains(name.as_str()) {
            refusals.push(format!(
                "{at}: {krate} redeclares the wire type {name}: take it from {WIRE} (ARCH-006)"
            ));
        }
    }
    refusals
}

/// The file of the launcher whose relay flushes the outbox (`BUS-036`).
const LAUNCHER_LIB: &str = "crates/ambre-app/src/lib.rs";

/// A file each boundary of the driver closes, and the rows it holds. A
/// boundary over a file that moved reads nothing, so a file no compile reads
/// fails closed.
const ANCHORS: &[(&str, &str)] = &[
    ("crates/ambre-app/src/switchboard.rs", "ARCH-072"),
    ("crates/ambre-app/src/outbox.rs", "BUS-035"),
    (
        "crates/ambre-trame/src/logic/fixtures.rs",
        "FIXTURE-004, FIXTURE-032",
    ),
    (
        "crates/ambre-toile/src/logic/state/fixtures.rs",
        "FIXTURE-004, FIXTURE-032",
    ),
    ("crates/ambre-trame/src/logic/mod.rs", "BUS-010"),
    ("crates/ambre-sync/src/lib.rs", "BUS-004, ZK-012"),
    ("crates/ambre-server/src/main.rs", "SRV-170, SRV-001"),
];

/// What the launcher's wiring must hold, counted over the facts of every
/// unit (`ARCH-007`, `ARCH-010`, `ARCH-014`, `BUS-035`, `BUS-036`). The
/// driver refuses a wrong fact where it reads it. A fact no unit records is
/// refused here, so a rule never holds by reading nothing.
pub(crate) fn wiring(records: &Records) -> Vec<String> {
    let sites = |name: &str| -> Vec<&str> {
        records
            .facts
            .iter()
            .filter(|(n, _, _)| n == name)
            .map(|(_, _, at)| at.as_str())
            .collect()
    };
    let mut refusals: Vec<String> = ANCHORS
        .iter()
        .filter(|(anchor, _)| !records.files.iter().any(|f| f.ends_with(anchor)))
        .map(|(anchor, rows)| {
            format!("{anchor}: no compile reads it, so {rows} reads nothing, and fails closed")
        })
        .collect();
    let starts = sites("logic-start");
    if starts.len() != 1 {
        refusals.push(format!(
            "the launcher starts the Logic runtime {} times, not once: {starts:?} (ARCH-010)",
            starts.len()
        ));
    }
    let required = [
        (
            "frame-relay",
            None,
            "no frame loop runs a relay that drains the Logic to Sync queue (ARCH-007)",
        ),
        (
            "panic-hook",
            None,
            "the launcher installs no panic hook that latches the fatal screen (ARCH-014)",
        ),
        (
            "worker-error",
            None,
            "the Sync worker's error event latches no fatal screen (ARCH-014)",
        ),
        (
            "ui-queue",
            None,
            "the launcher creates no UI to Logic queue, so the outbox rule reads nothing (BUS-035)",
        ),
        (
            "outbox-move",
            None,
            "the raw UI to Logic sender never moves into the outbox (BUS-035)",
        ),
        (
            "outbox-flush",
            Some(LAUNCHER_LIB),
            "the relay does not flush the launcher's outbox (BUS-036)",
        ),
        (
            "outbox-take_refusal",
            Some(LAUNCHER_LIB),
            "the relay tells the user of no refusal of the launcher's outbox (BUS-036)",
        ),
    ];
    for (name, file, refusal) in required {
        let found = sites(name)
            .iter()
            .any(|at| file.is_none_or(|f| at.starts_with(&format!("{f}:"))));
        if !found {
            refusals.push(refusal.to_owned());
        }
    }
    refusals
}

#[cfg(test)]
#[path = "boundaries_tests.rs"]
mod tests;
