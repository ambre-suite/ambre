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
//! | the PDF back end named outside a `src/logic/` directory and the converter module | `CANVAS-028` |
//! | the display density read outside `density.rs` | `CANVAS-031`, `CANVAS-043` |
//! | an application's command, event or command id named by the launcher | `ARCH-018` |
//! | an application message named by the Sync worker | `BUS-021` |
//! | a command or event enum of `ambre-bus` other than the envelopes | `BUS-032`, `ARCH-070` |
//! | a type declared by the worker, the relay or the protocol | `ARCH-006` |
//! | a browser API, the interface toolkit or a clock in the switchboard's decisions | `ARCH-072` |
//! | a CRDT path or type in an application's message vocabulary, which reaches the UI thread | `UI-THREAD-020` |
//! | a CRDT type, an export mode or an engine state in UI-thread code | `UI-THREAD-020`, `UI-THREAD-051` |
//! | a queue or a send in an engine's command handlers | `BUS-010` |
//! | a port or a lane over the Logic queue outside the launcher's outbox | `BUS-035`, `BUS-037` |
//! | a file, the host or an embedded file read by a generator module | `FIXTURE-032` |
//! | a browser storage name outside the persistence paths of the client | `UI-THREAD-004` |
//! | a compute pass, a compute pipeline, a storage buffer or a storage texture | `UI-THREAD-005` |
//! | a canvas or a drawing context outside the surface | `UI-THREAD-058` |
//! | a wait on the UI thread | `UI-THREAD-002` |
//! | a file converter on the UI thread | `ambre-spec-trame-import-export.md` section 3 |
//! | a canvas point read off the host, a content hit-test in an interface | `UI-THREAD-001` |
//! | a colour of the paint crates in the renderer | `CANVAS-060` |
//! | a drag step built away from the board, the grid panned outside its accumulator | `KINETIC-001` |
//!
//! The checks that read a type, a body or a fact of the whole unit are
//! `wiring.rs`, `names.rs`, `views.rs` and `wire.rs`. A new path rule is one
//! more row of [`BOUNDS`]. A rule over a name, a literal or an item shape is
//! one more method of those modules. The hooks of `main.rs` call it.
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
    pub fn named_by(&self, test: impl Fn(&str) -> bool) -> bool {
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
///
/// The call client of the UI thread holds the peer connection of a call, which carries
/// sealed chunks alone (`ARCH-005`, `CALL-001`).
const NETWORK_EXCEPTIONS: &[&str] = &[
    "crates/ambre-canvas/src/host.rs",
    "crates/ambre-app/src/platform.rs",
    "crates/ambre-app/src/call.rs",
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

/// The file where the switchboard decides (`ARCH-072`).
const SWITCHBOARD: &str = "crates/ambre-app/src/switchboard.rs";

/// The crates of the browser and of the interface toolkit, by prefix: a
/// binding, a signal, a component, a helper (`ARCH-072`).
const BROWSER_CRATES: &[&str] = &[
    "web_sys",
    "js_sys",
    "wasm_bindgen",
    "leptos",
    "reactive_graph",
    "reactive_stores",
    "tachys",
    "gloo",
];

/// Whether an item reads a clock. A `now` of any crate reads one, and so does
/// a clock type of the standard library or of a crate in its place
/// (`ARCH-072`).
fn clock(r: &Reached) -> bool {
    r.name == "now"
        || (["std", "core"].contains(&r.krate.as_str())
            && r.named_by(|n| n == "SystemTime" || n == "Instant"))
        || ["web_time", "chrono", "time"].contains(&r.krate.as_str())
}

/// The message vocabulary of each application. The views read it, so a CRDT
/// handle there reaches the UI thread (`UI-THREAD-020`).
const MESSAGES: &[&str] = &[
    "crates/ambre-trame/src/message",
    "crates/ambre-toile/src/message",
    "crates/ambre-prose/src/message",
    "crates/ambre-murmure/src/message",
];

/// The code that runs on the UI thread: the page's own crates, and the view,
/// render and chrome modules of each application (`UI-THREAD-020`).
pub const UI_THREAD: &[&str] = &[
    "crates/ambre-app/src/",
    "crates/ambre-chrome/src/",
    "crates/ambre-canvas/src/",
    "crates/ambre-trame/src/ui/",
    "crates/ambre-trame/src/render/",
    "crates/ambre-trame/src/chrome/",
    "crates/ambre-toile/src/ui/",
    "crates/ambre-toile/src/chrome/",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view/",
    "crates/ambre-prose/src/chrome/",
];

/// The engine states, which hold the document (`UI-THREAD-020`).
const ENGINE_STATES: &[&str] = &["TrameState", "ToileState", "ProseState", "MurmureState"];

/// Whether an item belongs to the CRDT engine. The crate splits into
/// `loro`, `loro_internal` and `loro_common`, and a re-export names the item
/// where it is declared.
pub fn crdt(r: &Reached) -> bool {
    r.krate.starts_with("loro")
}

/// Whether a file sits in the `src/logic` directory of a crate: an engine's
/// command handlers (`BUS-010`).
fn engine_logic(f: &str) -> bool {
    f.strip_prefix("crates/")
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, rest)| rest.starts_with("src/logic/"))
}

/// The launcher's one writer on the UI to Logic queue (`BUS-035`).
pub const SHELL_OUTBOX: &str = "crates/ambre-app/src/outbox.rs";

/// The corpus driver, which reads and writes the corpus by design, so the
/// one `fixtures.rs` that holds no generator (`FIXTURE-032`).
const FIXTURE_DRIVER: &str = "crates/ambre-logic/src/fixtures.rs";

/// Whether a file holds a generator: each `fixtures.rs` of a crate's source,
/// the corpus driver aside. Read off the path, so a generator a new
/// application adds is read without anyone naming it here (`FIXTURE-032`).
pub fn generator_module(f: &str) -> bool {
    f.starts_with("crates/")
        && f.contains("/src/")
        && f.ends_with("/fixtures.rs")
        && f != FIXTURE_DRIVER
}

/// The modules of the standard library that read a file or the host
/// (`FIXTURE-032`).
const HOST_MODULES: &[&str] = &["fs", "env", "process", "net"];

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

/// The surface itself: the drawing crate, the host that boots it, and the
/// shell cell that holds the element (`UI-THREAD-058`).
const DRAWING_OWNERS: &[&str] = &[
    "crates/ambre-canvas/src/",
    "crates/ambre-app/src/host/",
    "crates/ambre-chrome/src/app_shell.rs",
];

/// The two files that listen on the canvas (`UI-THREAD-001`).
pub const POINTER_OWNERS: &[&str] = &[
    "crates/ambre-app/src/host/",
    "crates/ambre-app/src/platform.rs",
];

/// The interface components, which never hit-test content: the shared
/// toolkit, each application's chrome, and the launcher outside the host
/// (`UI-THREAD-001`).
pub const INTERFACE: &[&str] = &[
    "crates/ambre-chrome/src/",
    "crates/ambre-trame/src/chrome/",
    "crates/ambre-toile/src/chrome/",
    "crates/ambre-prose/src/chrome/",
    "crates/ambre-app/src/",
];

/// The names that resolve a point to content: the shared trait, each view's
/// own resolver, and the camera inverse a resolver starts from
/// (`UI-THREAD-001`).
const HIT_NAMES: &[&str] = &[
    "hit_test",
    "hit",
    "cell_at",
    "drag_cell_at",
    "element_at",
    "hit_topmost",
    "place_at",
    "marquee_hits",
    "exact_hit",
    "screen_to_doc",
    "screen_to_world",
];

/// The crates that are no client instance: the Sync worker, the relay, the
/// SFU, the browser belt and the primitive gate (`UI-THREAD-004`).
const NON_CLIENT_CRATES: &[&str] = &[
    "crates/ambre-sync/",
    "crates/ambre-server/",
    "crates/ambre-sfu/",
    "crates/ambre-belt/",
    "crates/ambre-primitives/",
];

/// The persistence paths `UI-THREAD-004` names: the local store, and the
/// sign-in's own state for one tab and for the development identity.
const STORAGE_OWNERS: &[&str] = &[
    "crates/ambre-logic/src/persistence.rs",
    "crates/ambre-app/src/signin.rs",
    "crates/ambre-app/src/platform.rs",
];

/// The renderer, which holds no colour of its own (`CANVAS-060`).
const RENDERER: &str = "crates/ambre-canvas/src/render/";

/// The one shared kinetic model (`KINETIC-001`, `KINETIC-012`).
pub const KINETIC_MODEL: &str = "crates/ambre-draw/src/kinetic.rs";

/// The board's pointer handler, which alone builds a drag step
/// (`KINETIC-001`).
const DRAG_CALLER: &str = "crates/ambre-toile/src/ui/view.rs";

/// The accumulator of the Trame grid, which alone pans it (`KINETIC-001`).
pub const GRID_SCROLL: &str = "crates/ambre-trame/src/ui/view/scroll.rs";

/// Whether a file runs on the UI thread.
pub fn ui_thread(f: &str) -> bool {
    UI_THREAD.iter().any(|u| f.starts_with(u))
}

/// Whether an item reaches the browser's storage (`SYNC-054`,
/// `UI-THREAD-004`). Storage is a storage crate, web storage, the cache, the
/// storage manager, the private file system, a database or a cookie.
fn browser_storage(r: &Reached) -> bool {
    // A module of the workspace or a type of the toolchain may share a name
    // with the browser's storage. The `caches` of a view is no cache storage.
    // The slot of a `thread_local!` is no web storage.
    !r.krate.starts_with("ambre_")
        && !["std", "core", "alloc"].contains(&r.krate.as_str())
        && (STORAGE_CRATES.contains(&r.krate.as_str())
            || STORAGE_NAMES.iter().any(|n| r.named(n))
            || r.named_by(|n| n.starts_with("Idb") || n.to_lowercase().contains("cookie")))
}

/// Whether an item is a colour of the paint crates (`CANVAS-060`). That is
/// a named constant of a colour type or of the palette, or a colour built
/// from its components.
fn colour(r: &Reached) -> bool {
    if !["color", "peniko"].contains(&r.krate.as_str()) {
        return false;
    }
    let constant = |n: &str| {
        n.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && n.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    };
    let colour_type = r.owner.as_deref().is_some_and(|o| {
        [
            "Color",
            "AlphaColor",
            "OpaqueColor",
            "PremulColor",
            "DynamicColor",
        ]
        .contains(&o)
    });
    let built = ["new", "from_rgb", "from_rgb8", "from_rgba8"].contains(&r.name.as_str());
    (colour_type && (constant(&r.name) || built))
        || (matches!(r.kind, DefKind::Const) && r.modules.iter().any(|m| m == "palette"))
}

/// Whether an item leaves the subset both graphics back ends share: a
/// compute pass or pipeline, a storage buffer or a storage texture
/// (`UI-THREAD-005`).
fn compute(r: &Reached) -> bool {
    r.krate.starts_with("wgpu")
        && (r.named_by(|n| {
            n.starts_with("ComputePass")
                || n.starts_with("ComputePipeline")
                || n.starts_with("StorageTexture")
        }) || r.name == "create_compute_pipeline"
            || (r.owner.as_deref() == Some("BufferUsages") && r.name == "STORAGE")
            || (r.owner.as_deref() == Some("BufferBindingType") && r.name == "Storage"))
}

/// Whether an item waits (`UI-THREAD-002`). That is the conduit's parking
/// handle or its parking send, a parked thread, a futex, a condition, or an
/// executor run to completion.
fn blocking(r: &Reached) -> bool {
    ["Waiter", "WorkerSender", "Condvar"]
        .iter()
        .any(|n| r.named(n))
        || (matches!(r.kind, DefKind::AssocFn) && r.name == "wait")
        || (r.owner.as_deref() == Some("Atomics") && r.name.starts_with("wait"))
        || (["std", "core"].contains(&r.krate.as_str())
            && r.modules.iter().any(|m| m == "thread")
            && r.name.starts_with("park"))
        || r.name == "block_on"
}

/// Whether an item draws from a page element: a canvas, a drawing context
/// on one, or the `<canvas>` element of the interface toolkit
/// (`UI-THREAD-058`).
fn drawing(r: &Reached) -> bool {
    (r.krate == "web_sys"
        && (r.named_by(|n| {
            n.starts_with("HtmlCanvasElement")
                || n.starts_with("CanvasRenderingContext2d")
                || n.starts_with("OffscreenCanvas")
                || n.starts_with("WebGl")
        }) || r.name.starts_with("get_context")))
        || (["tachys", "leptos"].contains(&r.krate.as_str())
            && matches!(r.kind, DefKind::Fn)
            && r.name == "canvas")
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
        reaches: browser_storage,
    },
    Bound {
        rows: "UI-THREAD-004",
        what: "a persistence path the rule does not name",
        reads: Reads::ShippedCode,
        closed: |f| {
            f.starts_with("crates/")
                && !NON_CLIENT_CRATES.iter().any(|c| f.starts_with(c))
                && !STORAGE_OWNERS.contains(&f)
        },
        reaches: browser_storage,
    },
    Bound {
        rows: "UI-THREAD-005",
        what: "outside the shared graphics subset",
        reads: Reads::AllCode,
        closed: |f| f.starts_with("crates/"),
        reaches: compute,
    },
    Bound {
        rows: "UI-THREAD-058",
        what: "a page element draws",
        reads: Reads::ShippedCode,
        closed: |f| ui_thread(f) && !DRAWING_OWNERS.iter().any(|o| f.starts_with(o)),
        reaches: drawing,
    },
    Bound {
        rows: "UI-THREAD-002",
        what: "the UI thread waits",
        reads: Reads::ShippedCode,
        closed: ui_thread,
        reaches: blocking,
    },
    Bound {
        rows: "ambre-spec-trame-import-export.md section 3",
        what: "the UI thread converts a file",
        reads: Reads::ShippedCode,
        closed: ui_thread,
        reaches: |r| r.krate == "csv",
    },
    Bound {
        rows: "UI-THREAD-001",
        what: "a canvas point read off the host",
        reads: Reads::ShippedCode,
        closed: |f| ui_thread(f) && !POINTER_OWNERS.iter().any(|o| f.starts_with(o)),
        reaches: |r| {
            r.krate == "web_sys"
                && ["offset_x", "offset_y", "layer_x", "layer_y"].contains(&r.name.as_str())
        },
    },
    Bound {
        rows: "UI-THREAD-001",
        what: "an interface component hit-tests content",
        reads: Reads::ShippedCode,
        closed: |f| {
            ui_thread(f)
                && INTERFACE.iter().any(|i| f.starts_with(i))
                && !POINTER_OWNERS
                    .first()
                    .is_some_and(|host| f.starts_with(host))
        },
        reaches: |r| {
            (matches!(r.kind, DefKind::Fn | DefKind::AssocFn)
                && HIT_NAMES.contains(&r.name.as_str()))
                || r.named("HitTest")
        },
    },
    Bound {
        rows: "CANVAS-060",
        what: "the renderer holds a colour of its own",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with(RENDERER),
        reaches: colour,
    },
    Bound {
        rows: "KINETIC-001",
        what: "a drag step built away from the board's pointer",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/") && f != KINETIC_MODEL && f != DRAG_CALLER,
        reaches: |r| r.owner.as_deref() == Some("ScrollStep") && r.name == "dragged",
    },
    Bound {
        rows: "KINETIC-001",
        what: "the grid panned outside its accumulator",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/ambre-trame/") && f != GRID_SCROLL,
        reaches: |r| matches!(r.kind, DefKind::AssocFn) && r.name == "scroll_by",
    },
    Bound {
        rows: "CANVAS-028",
        what: "the PDF back end outside the Logic thread",
        reads: Reads::AllCode,
        closed: |f| {
            // The converter module runs on the Logic thread too: the Logic
            // thread instantiates it and calls it synchronously.
            f.starts_with("crates/")
                && !f.starts_with("crates/ambre-pdf/")
                && !f.starts_with("crates/ambre-convert/")
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
                && matches!(r.kind, DefKind::Const)
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
    Bound {
        rows: "ARCH-072",
        what: "the switchboard's decisions name the browser",
        reads: Reads::ShippedCode,
        closed: |f| f == SWITCHBOARD,
        reaches: |r| BROWSER_CRATES.iter().any(|c| r.krate.starts_with(c)),
    },
    Bound {
        rows: "ARCH-072",
        what: "the switchboard reads a clock, not the instant it is given",
        reads: Reads::ShippedCode,
        closed: |f| f == SWITCHBOARD,
        reaches: clock,
    },
    Bound {
        rows: "UI-THREAD-020",
        what: "a message names a CRDT handle",
        reads: Reads::ShippedCode,
        closed: |f| MESSAGES.iter().any(|m| f.starts_with(m)),
        reaches: crdt,
    },
    Bound {
        rows: "UI-THREAD-020, UI-THREAD-051",
        what: "the UI thread holds the document",
        reads: Reads::ShippedCode,
        closed: |f| UI_THREAD.iter().any(|u| f.starts_with(u)),
        // A tree node's identifier is an address, not the document. A module
        // on the way is no item: the item at its end is read.
        reaches: |r| {
            (crdt(r) && !matches!(r.kind, DefKind::Mod) && !r.named("TreeID"))
                || ENGINE_STATES.iter().any(|s| r.named(s))
        },
    },
    Bound {
        rows: "BUS-010",
        what: "a command handler sends on a queue",
        reads: Reads::ShippedCode,
        closed: engine_logic,
        reaches: |r| {
            (r.krate == "ambre_bus" && r.modules.iter().any(|m| m == "conduit"))
                || ["Sender", "WorkerSender", "Outbox"]
                    .iter()
                    .any(|n| r.named(n))
                || r.name == "try_send"
                || r.name.starts_with("send_to_")
        },
    },
    Bound {
        rows: "BUS-035, BUS-037",
        what: "the launcher writes past its outbox",
        reads: Reads::ShippedCode,
        closed: |f| f.starts_with("crates/ambre-app/src/") && f != SHELL_OUTBOX,
        reaches: |r| {
            r.krate == "ambre_bus"
                && ((r.owner.as_deref() == Some("Port")
                    && ["embedded", "direct"].contains(&r.name.as_str()))
                    || (r.owner.as_deref() == Some("Lane")
                        && ["new", "with_capacity"].contains(&r.name.as_str())))
        },
    },
    Bound {
        rows: "FIXTURE-032",
        what: "a generator reads besides its seed",
        reads: Reads::ShippedCode,
        closed: generator_module,
        reaches: |r| {
            ["std", "core", "alloc"].contains(&r.krate.as_str())
                && r.modules.iter().any(|m| HOST_MODULES.contains(&m.as_str()))
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
        DefKind::AssocFn | DefKind::AssocConst | DefKind::AssocTy => match tcx.def_kind(parent) {
            DefKind::Impl { .. } => match tcx
                .type_of(parent)
                .instantiate_identity()
                .skip_normalization()
                .kind()
            {
                ty::Adt(adt, _) => Some(tcx.item_name(adt.did()).to_string()),
                ty::Foreign(foreign) => Some(tcx.item_name(*foreign).to_string()),
                _ => None,
            },
            _ => tcx.opt_item_name(parent).map(|n| n.to_string()),
        },
        DefKind::Variant => tcx.opt_item_name(parent).map(|n| n.to_string()),
        DefKind::Ctor(CtorOf::Variant, _) => tcx
            .opt_parent(parent)
            .and_then(|e| tcx.opt_item_name(e))
            .map(|n| n.to_string()),
        DefKind::Ctor(CtorOf::Struct, _) => tcx.opt_item_name(parent).map(|n| n.to_string()),
        _ => None,
    }
}

/// The built-in macros that read a file or the build host into a literal.
const HOST_MACROS: &[&str] = &["include_bytes", "include_str", "env", "option_env"];

/// The built-in macro of [`HOST_MACROS`] that wrote `span`, if one did,
/// followed through every macro call between it and the source.
fn host_macro(span: Span) -> Option<String> {
    let mut span = span;
    while span.from_expansion() {
        let data = span.ctxt().outer_expn_data();
        // A macro carries the path its call writes: `std::include_bytes`.
        if let rustc_span::ExpnKind::Macro(_, name) = data.kind
            && HOST_MACROS.contains(&crate::wiring::last_segment(name.as_str()))
        {
            return Some(crate::wiring::last_segment(name.as_str()).to_owned());
        }
        span = data.call_site;
    }
    None
}

impl Gate {
    /// Refuses the name at `span` when it reaches what a boundary closes to
    /// the file that writes it. `def` is what the name resolves to.
    pub fn bound_check(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
        // An alias resolves to itself, so the items its target names are read
        // too: an alias declared in another crate hides nothing.
        if matches!(tcx.def_kind(def), DefKind::TyAlias) {
            let target = tcx.type_of(def).instantiate_identity().skip_normalization();
            let named: Vec<DefId> = target
                .walk()
                .filter_map(|arg| arg.as_type())
                .filter_map(|ty| match ty.kind() {
                    ty::Adt(adt, _) => Some(adt.did()),
                    ty::Foreign(foreign) => Some(*foreign),
                    _ => None,
                })
                .collect();
            for target in named {
                self.reached_check(tcx, target, span);
            }
        }
        self.reached_check(tcx, def, span);
    }

    /// Refuses the item `def` named at `span` when a boundary closes it to
    /// the file.
    fn reached_check(&mut self, tcx: TyCtxt<'_>, def: DefId, span: Span) {
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

    /// Refuses a literal that a generator module embeds or reads off the build
    /// host. `include_bytes!`, `include_str!`, `env!` and `option_env!` write
    /// one, whoever calls the macro (`FIXTURE-032`).
    pub fn embedded_literal(&mut self, tcx: TyCtxt<'_>, span: Span) {
        let Some(name) = host_macro(span) else {
            return;
        };
        let at = place::position(tcx, &self.unit, span);
        let files = shown_paths(&self.unit.cwd, &at.path);
        if self.test || !files.iter().any(|f| generator_module(f)) {
            return;
        }
        if self.bounded.insert(("FIXTURE-032", at.at())) {
            self.refuse(
                "boundary",
                format!(
                    "{}: a generator reads besides its seed: `{name}!` (FIXTURE-032)",
                    at.at()
                ),
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
                | hir::ItemKind::Trait { .. }
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
