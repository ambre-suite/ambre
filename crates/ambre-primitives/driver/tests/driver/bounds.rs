//! The crate boundaries, read with types. Each test plants a breach that a
//! scan of the text lets through: an alias, a re-export or an item a macro
//! writes. The driver refuses it (`ARCH-001`, `ARCH-005`, `ARCH-006`,
//! `ARCH-011`, `ARCH-018`, `BUS-021`, `BUS-032`, `CANVAS-028`, `CANVAS-031`,
//! `SYNC-054`).

use crate::harness::{Build, Workspace, of};

/// The boundary refusals in `records`.
fn refused(records: &[String]) -> Vec<String> {
    of(records, "refuse")
        .into_iter()
        .filter_map(|l| l.strip_prefix("boundary\t").map(str::to_owned))
        .collect()
}

/// A stand-in for the browser bindings, with the names the boundaries read.
const WEB_SYS: &str = "pub struct Window;\n\
    impl Window {\n\
        pub fn fetch_with_str(&self, _: &str) {}\n\
        pub fn device_pixel_ratio(&self) -> f64 { 1.0 }\n\
        pub fn local_storage(&self) -> Option<Storage> { None }\n\
    }\n\
    pub struct Storage;\n\
    pub struct WebSocket;\n\
    impl WebSocket { pub fn new(_: &str) -> Self { WebSocket } }\n\
    pub struct Worker;\n\
    impl Worker { pub fn new(_: &str) -> Self { Worker } }\n\
    pub struct SharedWorker;\n";

/// A foreign crate that re-exports browser types under names of its own, as
/// a helper crate of the ecosystem does.
const HELPERS: &str = "pub use web_sys::SharedWorker as Shared;\n\
    pub use web_sys::Storage as Store;\n";

/// The workspace with the browser bindings, the helper crate and a core crate
/// that re-exports the socket and opens it from a macro.
fn browser(name: &str) -> Workspace {
    let ws = Workspace::new(name);
    ws.file("crates/web-sys/src/lib.rs", WEB_SYS);
    ws.file("crates/helpers/src/lib.rs", HELPERS);
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "pub use web_sys::WebSocket as Pipe;\n\
         #[macro_export]\n\
         macro_rules! open { ($u:expr) => { $crate::Pipe::new($u) } }\n",
    );
    ws.compile("web_sys", "web-sys", &[], Build::Shipped);
    ws.compile("helpers", "helpers", &["web_sys"], Build::Shipped);
    ws
}

// The launcher opens the socket through an alias and through a macro of
// another crate. Its text names neither `WebSocket::new` nor a fetch, so a
// scan lets both through. The worker opens freely (`ARCH-005`).
#[test]
fn a_network_call_through_an_alias_or_a_macro_is_refused_outside_the_worker() {
    let ws = browser("network");
    let core = ws.compile("ambre_core", "ambre-core", &["web_sys"], Build::Shipped);
    assert_eq!(refused(&core), Vec::<String>::new());
    let body = "use web_sys::WebSocket as Socket;\n\
        pub fn alias() { let _ = Socket::new(\"wss://relay\"); }\n\
        pub fn hidden() { let _ = ambre_core::open!(\"wss://relay\"); }\n";
    ws.file("crates/ambre-app/src/lib.rs", body);
    let app = ws.compile(
        "ambre_app",
        "ambre-app",
        &["web_sys", "ambre_core"],
        Build::Shipped,
    );
    let app = refused(&app);
    assert_eq!(app.len(), 2, "{app:#?}");
    assert!(
        app[0].starts_with("crates/ambre-app/src/lib.rs:2: a network call"),
        "{app:#?}"
    );
    assert!(
        app[1].starts_with("crates/ambre-app/src/lib.rs:3: a network call"),
        "{app:#?}"
    );
    // A test build reads the test code as well.
    let tests = ws.compile(
        "ambre_app",
        "ambre-app",
        &["web_sys", "ambre_core"],
        Build::Test,
    );
    assert_eq!(refused(&tests).len(), 2);
    ws.file("crates/ambre-sync/src/lib.rs", body);
    let sync = ws.compile(
        "ambre_sync",
        "ambre-sync",
        &["web_sys", "ambre_core"],
        Build::Shipped,
    );
    assert_eq!(refused(&sync), Vec::<String>::new());
}

// A worker spawned through an alias outside the spawner, and a shared worker
// reached through a foreign re-export (`ARCH-001`, `ARCH-011`).
#[test]
fn a_worker_spawned_through_an_alias_or_shared_through_a_re_export_is_refused() {
    let ws = browser("workers");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod platform;\n\
         type Spawn = web_sys::Worker;\n\
         pub fn spawn() { let _ = Spawn::new(\"sync.js\"); }\n\
         pub fn tab(_: helpers::Shared) {}\n",
    );
    ws.file(
        "crates/ambre-app/src/platform.rs",
        "pub fn spawn() { let _ = web_sys::Worker::new(\"sync.js\"); }\n",
    );
    let app = ws.compile(
        "ambre_app",
        "ambre-app",
        &["web_sys", "helpers"],
        Build::Shipped,
    );
    let app = refused(&app);
    assert_eq!(app.len(), 2, "{app:#?}");
    assert!(
        app[0].starts_with("crates/ambre-app/src/lib.rs:3: a worker spawned"),
        "{app:#?}"
    );
    assert!(
        app[1].starts_with("crates/ambre-app/src/lib.rs:4: a worker that two tabs"),
        "{app:#?}"
    );
}

// The worker reaches browser storage through a name a helper crate gives it
// (`SYNC-054`, `SYNC-060`).
#[test]
fn browser_storage_under_a_foreign_name_is_refused_in_the_worker() {
    let ws = browser("storage");
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "pub fn keep(_: Option<helpers::Store>) {}\n",
    );
    let sync = ws.compile(
        "ambre_sync",
        "ambre-sync",
        &["web_sys", "helpers"],
        Build::Shipped,
    );
    let sync = refused(&sync);
    assert_eq!(sync.len(), 1, "{sync:#?}");
    assert!(
        sync[0].starts_with("crates/ambre-sync/src/lib.rs:1: the Sync worker reaches browser"),
        "{sync:#?}"
    );
}

// The PDF back end reached through a foreign re-export outside a Logic
// directory, and the same call from one (`CANVAS-028`).
#[test]
fn the_pdf_back_end_under_a_foreign_name_is_refused_outside_the_logic_thread() {
    let ws = Workspace::new("pdf");
    ws.file("crates/krilla/src/lib.rs", "pub struct Document;\n");
    ws.file(
        "crates/printer/src/lib.rs",
        "pub use krilla::Document as Page;\n",
    );
    ws.compile("krilla", "krilla", &[], Build::Shipped);
    ws.compile("printer", "printer", &["krilla"], Build::Shipped);
    ws.file(
        "crates/ambre-toile/src/lib.rs",
        "pub mod logic;\npub fn print(_: printer::Page) {}\n",
    );
    ws.file(
        "crates/ambre-toile/src/logic/mod.rs",
        "pub fn print(_: printer::Page) {}\n",
    );
    let toile = ws.compile("ambre_toile", "ambre-toile", &["printer"], Build::Shipped);
    let toile = refused(&toile);
    assert_eq!(toile.len(), 1, "{toile:#?}");
    assert!(
        toile[0].starts_with("crates/ambre-toile/src/lib.rs:2: the PDF back end"),
        "{toile:#?}"
    );
}

// The density read by its method and by its JavaScript name outside
// `density.rs`, and inside it (`CANVAS-031`, `CANVAS-043`).
#[test]
fn the_density_read_by_method_or_by_name_is_refused_outside_its_file() {
    let ws = browser("density");
    let read = "pub fn ratio(w: &web_sys::Window) -> f64 { w.device_pixel_ratio() }\n\
        pub fn by_name() -> &'static str { \"devicePixelRatio\" }\n";
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        &format!("pub mod density;\n{read}"),
    );
    ws.file("crates/ambre-canvas/src/density.rs", read);
    let canvas = ws.compile("ambre_canvas", "ambre-canvas", &["web_sys"], Build::Shipped);
    let canvas = refused(&canvas);
    assert_eq!(canvas.len(), 2, "{canvas:#?}");
    assert!(
        canvas
            .iter()
            .all(|r| r.starts_with("crates/ambre-canvas/src/lib.rs:")),
        "{canvas:#?}"
    );
}

// The launcher names a variant through an alias of the enum, and a command id
// through a renamed crate (`ARCH-018`).
#[test]
fn an_application_variant_or_command_id_under_an_alias_is_refused_in_the_launcher() {
    let ws = Workspace::new("launcher");
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub enum TrameCommand { Insert, Clear(u8) }\n\
         pub mod chrome { pub const BOLD: u8 = 1; pub const COMMANDS: &[u8] = &[BOLD]; }\n",
    );
    ws.compile("ambre_trame", "ambre-trame", &[], Build::Shipped);
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "use ambre_trame::TrameCommand as T;\n\
         use ambre_trame as grid;\n\
         pub fn send() -> T { T::Insert }\n\
         pub fn clear() -> T { T::Clear(grid::chrome::BOLD) }\n\
         pub fn ids() -> &'static [u8] { grid::chrome::COMMANDS }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", &["ambre_trame"], Build::Shipped);
    let app = refused(&app);
    assert_eq!(app.len(), 2, "{app:#?}");
    assert!(
        app[0].starts_with("crates/ambre-app/src/lib.rs:3: the launcher names application logic"),
        "{app:#?}"
    );
    assert!(
        app[1].starts_with("crates/ambre-app/src/lib.rs:4: the launcher names application logic"),
        "{app:#?}"
    );
}

// The worker reaches the application module of the bus through a re-export
// that never spells it (`BUS-021`). The bus declares an application's enum
// through a macro (`BUS-032`).
#[test]
fn the_bus_and_the_worker_hold_no_application_message_behind_a_macro_or_a_re_export() {
    let ws = Workspace::new("bus");
    ws.file(
        "crates/ambre-bus/src/lib.rs",
        "pub mod app { pub struct Payload; pub mod murmure { pub struct ChannelName; } }\n\
         pub use app::Payload as Opaque;\n\
         pub enum SyncCommand { Push }\n\
         macro_rules! message { ($name:ident) => { pub enum $name { Insert } } }\n\
         message!(TrameCommand);\n",
    );
    let bus = ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    let bus = refused(&bus);
    assert_eq!(bus.len(), 1, "{bus:#?}");
    assert!(
        bus[0].starts_with("crates/ambre-bus/src/lib.rs:5: the bus declares an application's"),
        "{bus:#?}"
    );
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "pub fn relay(_: ambre_bus::Opaque, _: ambre_bus::SyncCommand) {}\n\
         pub fn list(_: ambre_bus::app::murmure::ChannelName) {}\n",
    );
    let sync = ws.compile("ambre_sync", "ambre-sync", &["ambre_bus"], Build::Shipped);
    let sync = refused(&sync);
    assert_eq!(sync.len(), 1, "{sync:#?}");
    assert!(
        sync[0].starts_with("crates/ambre-sync/src/lib.rs:1: the worker names an application"),
        "{sync:#?}"
    );
}

// The driver records a type of the worker whatever its visibility, and a
// type a macro writes too. The judge holds each against the wire types
// (`ARCH-006`).
#[test]
fn every_type_the_worker_declares_is_recorded_a_macro_written_one_included() {
    let ws = Workspace::new("wire");
    ws.file(
        "crates/ambre-protocol/src/lib.rs",
        "pub struct Hello;\nstruct Inner;\n",
    );
    let protocol = ws.compile("ambre_protocol", "ambre-protocol", &[], Build::Shipped);
    let decls = of(&protocol, "decl");
    assert!(
        decls.contains(
            &"ambre-protocol\tHello\tpublic\tcrates/ambre-protocol/src/lib.rs:1".to_owned()
        ),
        "{decls:#?}"
    );
    assert!(
        decls.contains(
            &"ambre-protocol\tInner\tprivate\tcrates/ambre-protocol/src/lib.rs:2".to_owned()
        ),
        "{decls:#?}"
    );
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "macro_rules! wire { ($name:ident) => { struct $name; } }\nwire!(Hello);\n",
    );
    let sync = ws.compile("ambre_sync", "ambre-sync", &[], Build::Shipped);
    assert_eq!(
        of(&sync, "decl"),
        vec!["ambre-sync\tHello\tprivate\tcrates/ambre-sync/src/lib.rs:2".to_owned()]
    );
}
