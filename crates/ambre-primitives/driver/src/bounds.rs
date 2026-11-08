//! The crate boundaries read with types (`ambre-spec-architecture.md`
//! section 2).
//!
//! A boundary names what a part of the workspace must not reach: a browser
//! API, a crate, a variant of an application's command. The driver reads each
//! path, method call and type-relative path once it resolves. So an alias, a
//! re-export, a glob import and the code a macro writes name the item they
//! reach, and the boundary sees it. The text of the line does not matter.
//!
//! | Check | Rows |
//! | :--- | :--- |
//! | a network call outside the Sync worker, the relay, the SFU and the belt | `ARCH-005` |
//! | a worker spawned outside the launcher's spawner | `ARCH-001`, `ARCH-011` |
//! | a worker that two tabs would share | `ARCH-011` |
//! | browser storage named by the Sync worker | `SYNC-054`, `SYNC-060` |
//! | the PDF back end named outside a `src/logic/` directory | `CANVAS-028` |
//! | the display density read outside `density.rs` | `CANVAS-031`, `CANVAS-043` |
//! | an application's command, event or command id named by the launcher | `ARCH-018` |
//! | an application message named by the Sync worker | `BUS-021` |
//! | a command or event enum of `ambre-bus` other than the envelopes | `BUS-032`, `ARCH-070` |
//! | a type declared by the worker, the relay or the protocol | `ARCH-006` |
//!
//! A refusal names the file and the line of the call site, each macro call
//! followed back to the source. The judge prints it as it prints the other
//! refusals the code alone settles.

use rustc_hir as hir;
use rustc_hir::def::{CtorOf, DefKind};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::Span;

use crate::Gate;
use crate::checks::shown_paths;
use crate::place;

/// What a resolved path reaches, as a boundary reads it.
pub struct Reached {
    /// The crate of the item, as the compiler names it: `web_sys`.
    pub krate: String,
    /// The name of the item. A crate root takes the crate name.
    pub name: String,
    /// The type, trait or enum the item belongs to: the self type of the
    /// impl of a method, the enum of a variant.
    pub owner: Option<String>,
    /// The name of each module on the path of the item.
    pub modules: Vec<String>,
    pub kind: DefKind,
}

impl Reached {
    /// Whether the item or its owner carries `name`.
    fn named(&self, name: &str) -> bool {
        self.name == name || self.owner.as_deref() == Some(name)
    }

    /// Whether the item or its owner carries a name for which `test` holds.
    fn named_by(&self, test: impl Fn(&str) -> bool) -> bool {
        test(&self.name) || self.owner.as_deref().is_some_and(test)
    }

    /// Whether the item is a variant, or the constructor of one.
    fn variant(&self) -> bool {
        matches!(
            self.kind,
            DefKind::Variant | DefKind::Ctor(CtorOf::Variant, _)
        )
    }
}

/// Whether test code is read too. A rule over a file reads all of it; a rule
/// over the shipped code reads what a test build adds nothing to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reads {
    AllCode,
    ShippedCode,
}

/// One boundary: what must not be reached, and where.
struct Bound {
    /// The rows the boundary holds, as a refusal cites them.
    rows: &'static str,
    /// What the refusal says the line does.
    what: &'static str,
    reads: Reads,
    /// Whether a file, from the workspace root, sits on the side that must
    /// not reach.
    closed: fn(&str) -> bool,
    /// Whether an item is one the closed side must not reach.
    reaches: fn(&Reached) -> bool,
}

/// The crates a network call may leave from. The relay and the SFU are
/// servers, and the belt is a test harness that ships nothing.
const NETWORK_CRATES: &[&str] = &[
    "crates/ambre-sync/",
    "crates/ambre-server/",
    "crates/ambre-sfu/",
    "crates/ambre-belt/",
];

/// The two reads `UI-THREAD-004` names: the same-origin font fetch and the
/// pre-session sign-in configuration.
const NETWORK_EXCEPTIONS: &[&str] = &[
    "crates/ambre-canvas/src/host.rs",
    "crates/ambre-app/src/platform.rs",
];

/// The crates that open a connection, whole.
const TRANSPORT_CRATES: &[&str] = &[
    "gloo_net",
    "reqwest",
    "tungstenite",
    "tokio_tungstenite",
    "ewebsock",
    "ws_stream_wasm",
];

/// The browser types that open a connection, whole.
const NETWORK_TYPES: &[&str] = &["XmlHttpRequest", "EventSource", "RtcPeerConnection"];

/// The crates that reach the browser's storage, whole.
const STORAGE_CRATES: &[&str] = &["gloo_storage", "indexed_db_futures", "rexie", "idb"];

/// The names that reach the browser's storage: web storage, the cache, the
/// storage manager and the private file system.
const STORAGE_NAMES: &[&str] = &[
    "local_storage",
    "session_storage",
    "Storage",
    "indexed_db",
    "caches",
    "CacheStorage",
    "StorageManager",
    "get_directory",
];

/// The envelopes and the service family `ambre-bus` declares (`BUS-032`).
pub const BUS_ENVELOPES: &[&str] = &[
    "LogicCommand",
    "UiEvent",
    "SyncCommand",
    "LogicEvent",
    "RestCommand",
    "RestEvent",
];

/// The applications whose messages and command ids stay in their crate.
const APPLICATIONS: &[&str] = &["Trame", "Toile", "Prose", "Murmure"];

/// The file that may read the display density.
const DENSITY: &str = "crates/ambre-canvas/src/density.rs";

/// The JavaScript property of the density, read by name through `Reflect`.
pub const DENSITY_PROPERTY: &str = "devicePixelRatio";

/// The file that spawns the Sync worker.
const WORKER_SPAWNER: &str = "crates/ambre-app/src/platform.rs";

/// Whether `name` is the command or the event enum of an application, or of
/// the application family when `family` is set.
fn app_message(name: &str, family: bool) -> bool {
    ["Command", "Event"].iter().any(|kind| {
        APPLICATIONS
            .iter()
            .chain(family.then_some(&"App"))
            .any(|app| name == format!("{app}{kind}"))
    })
}

const BOUNDS: &[Bound] = &[
    Bound {
        rows: "ARCH-005",
        what: "a network call outside the Sync worker",
        reads: Reads::AllCode,
        closed: |f| {
            f.starts_with("crates/")
                && !NETWORK_CRATES.iter().any(|c| f.starts_with(c))
                && !NETWORK_EXCEPTIONS.contains(&f)
        },
        reaches: |r| {
            TRANSPORT_CRATES.contains(&r.krate.as_str())
                || NETWORK_TYPES.iter().any(|t| r.named(t))
                || (r.owner.as_deref() == Some("WebSocket") && r.name.starts_with("new"))
                || r.name.starts_with("fetch_with_")
                || r.name == "send_beacon"
        },
    },
    Bound {
        rows: "ARCH-001, ARCH-011",
        what: "a worker spawned outside the launcher's spawner",
        reads: Reads::ShippedCode,
        closed: |f| {
            f.starts_with("crates/")
                && f != WORKER_SPAWNER
                && !f.starts_with("crates/ambre-server/")
        },
        reaches: |r| r.owner.as_deref() == Some("Worker") && r.name.starts_with("new"),
    },
    Bound {
        rows: "ARCH-011",
        what: "a worker that two tabs would share",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/"),
        reaches: |r| {
            r.named_by(|n| n.starts_with("SharedWorker") || n.starts_with("ServiceWorker"))
                || r.name == "service_worker"
        },
    },
    Bound {
        rows: "SYNC-054, SYNC-060",
        what: "the Sync worker reaches browser storage",
        reads: Reads::AllCode,
        closed: |f| f.starts_with("crates/ambre-sync/"),
        reaches: |r| {
            STORAGE_CRATES.contains(&r.krate.as_str())
                || STORAGE_NAMES.iter().any(|n| r.named(n))
                || r.named_by(|n| n.starts_with("Idb") || n.to_lowercase().contains("cookie"))
        },
    },
    Bound {
        rows: "CANVAS-028",
        what: "the PDF back end outside the Logic thread",
        reads: Reads::AllCode,
        closed: |f| {
            f.starts_with("crates/")
                && !f.starts_with("crates/ambre-pdf/")
                && !f.contains("/src/logic/")
        },
        reaches: |r| {
            ["krilla", "ambre_pdf"].contains(&r.krate.as_str()) || r.named("KrillaSurface")
        },
    },
    Bound {
        rows: "CANVAS-031, CANVAS-043",
        what: "reads the display density outside density.rs",
        reads: Reads::AllCode,
        closed: |f| f.starts_with("crates/") && f != DENSITY,
        reaches: |r| r.name == "device_pixel_ratio",
    },
    Bound {
        rows: "ARCH-018",
        what: "the launcher names application logic",
        // A test of the launcher holds two applications side by side, which
        // is integration. `murmure_regions.rs` checks that a Murmure style
        // takes the chord of Prose for its mark.
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/ambre-app/src/"),
        reaches: |r| {
            let message = r.variant() && r.owner.as_deref().is_some_and(|o| app_message(o, false));
            let app_crate = APPLICATIONS
                .iter()
                .any(|app| r.krate == format!("ambre_{}", app.to_lowercase()));
            let command_id = app_crate
                && matches!(r.kind, DefKind::Const { .. })
                && r.name != "COMMANDS"
                && r.modules.iter().any(|m| m == "commands" || m == "chrome");
            message || command_id
        },
    },
    Bound {
        rows: "BUS-021",
        what: "the worker names an application message",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/ambre-sync/"),
        reaches: |r| {
            // The `murmure` module of `ambre_bus::app` holds the channel types
            // of the service plane, which the worker reads by design.
            // A module on the way is no item: the item at its end is read.
            (r.krate == "ambre_bus"
                && !matches!(r.kind, DefKind::Mod)
                && r.modules.iter().any(|m| m == "app")
                && !r.modules.iter().any(|m| m == "murmure"))
                || r.named_by(|n| n == "LogicCommand" || n == "UiEvent" || app_message(n, true))
        },
    },
];

/// What `def` reaches, as a boundary reads it.
pub fn reached(tcx: TyCtxt<'_>, def: DefId) -> Reached {
    let krate = tcx.crate_name(def.krate).to_string();
    let kind = tcx.def_kind(def);
    let name = tcx
        .opt_item_name(def)
        .map_or_else(|| krate.clone(), |n| n.to_string());
    let owner = owner(tcx, def, kind);
    let modules = tcx
        .def_path(def)
        .data
        .iter()
        .filter_map(|d| match d.data {
            rustc_hir::definitions::DefPathData::TypeNs(sym) => Some(sym.to_string()),
            _ => None,
        })
        .collect();
    Reached {
        krate,
        name,
        owner,
        modules,
        kind,
    }
}

/// The type, trait or enum an item belongs to, by name.
fn owner(tcx: TyCtxt<'_>, def: DefId, kind: DefKind) -> Option<String> {
    let parent = tcx.opt_parent(def)?;
    match kind {
        DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy => {
            match tcx.def_kind(parent) {
                DefKind::Impl { .. } => match tcx.type_of(parent).instantiate_identity().kind() {
                    ty::Adt(adt, _) => Some(tcx.item_name(adt.did()).to_string()),
                    ty::Foreign(foreign) => Some(tcx.item_name(*foreign).to_string()),
                    _ => None,
                },
                _ => tcx.opt_item_name(parent).map(|n| n.to_string()),
            }
        }
        DefKind::Variant => tcx.opt_item_name(parent).map(|n| n.to_string()),
        DefKind::Ctor(CtorOf::Variant, _) => tcx
            .opt_parent(parent)
            .and_then(|e| tcx.opt_item_name(e))
            .map(|n| n.to_string()),
        DefKind::Ctor(CtorOf::Struct, _) => tcx.opt_item_name(parent).map(|n| n.to_string()),
        _ => None,
    }
}

impl Gate {
    /// Refuses the name at `span` when it reaches what a boundary closes to
    /// the file that writes it. `def` is what the name resolves to.
    pub fn bound_check(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
        let reached = reached(tcx, def);
        let open: Vec<&Bound> = BOUNDS
            .iter()
            .filter(|b| (!self.test || b.reads == Reads::AllCode) && (b.reaches)(&reached))
            .collect();
        if open.is_empty() {
            return;
        }
        let at = place::position(tcx, &self.unit, span);
        let files = shown_paths(&self.unit.cwd, &at.path);
        for bound in open {
            if !files.iter().any(|f| (bound.closed)(f)) {
                continue;
            }
            // One refusal per rule and line: a path names its type and its
            // method on one line.
            if !self.bounded.insert((bound.rows, at.at())) {
                continue;
            }
            let path = tcx.def_path_str(def);
            self.refuse(
                "boundary",
                format!("{}: {}: `{path}` ({})", at.at(), bound.what, bound.rows),
            );
        }
    }

    /// Refuses the JavaScript name of the density outside `density.rs`: a
    /// `Reflect` read spells it as a string (`CANVAS-031`).
    pub fn density_literal(&mut self, tcx: TyCtxt<'_>, text: &str, span: Span) {
        if text != DENSITY_PROPERTY {
            return;
        }
        let at = place::position(tcx, &self.unit, span);
        let files = shown_paths(&self.unit.cwd, &at.path);
        if files
            .iter()
            .any(|f| f.starts_with("crates/") && f != DENSITY)
        {
            self.refuse(
                "boundary",
                format!(
                    "{}: reads the display density outside density.rs: \"{DENSITY_PROPERTY}\" \
                     (CANVAS-031, CANVAS-043)",
                    at.at()
                ),
            );
        }
    }

    /// Records a type, an alias or a trait of the worker, the relay or the
    /// protocol (`ARCH-006`). Refuses a message enum of `ambre-bus` other
    /// than the envelopes (`BUS-032`).
    pub fn declaration(&mut self, tcx: TyCtxt<'_>, item: &hir::Item<'_>, def: LocalDefId) {
        let Some(ident) = item.kind.ident() else {
            return;
        };
        let declares = matches!(
            item.kind,
            hir::ItemKind::Struct(..)
                | hir::ItemKind::Enum(..)
                | hir::ItemKind::Union(..)
                | hir::ItemKind::TyAlias(..)
                | hir::ItemKind::Trait(..)
        );
        if !declares {
            return;
        }
        let name = ident.as_str();
        let at = place::position(tcx, &self.unit, ident.span);
        if self.unit.krate == "ambre-bus"
            && matches!(item.kind, hir::ItemKind::Enum(..))
            && (name.ends_with("Command") || name.ends_with("Event"))
            && !BUS_ENVELOPES.contains(&name)
        {
            self.refuse(
                "boundary",
                format!(
                    "{}: the bus declares an application's message `{name}`: it belongs to the \
                     application's crate (BUS-032, ARCH-070)",
                    at.at()
                ),
            );
        }
        if ["ambre-protocol", "ambre-sync", "ambre-server"].contains(&self.unit.krate.as_str()) {
            let public = if tcx.visibility(def.to_def_id()).is_public() {
                "public"
            } else {
                "private"
            };
            let krate = self.unit.krate.clone();
            self.record(&["decl", &krate, name, public, &at.at()]);
        }
    }
}
