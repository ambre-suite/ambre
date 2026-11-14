//! The boundaries that `crate_boundaries.py` read by regular expression,
//! read with types. Each test plants a breach that a scan of the text let
//! through, and the driver refuses it. The breach is an alias, a re-export,
//! an inferred type, a macro or a value under a neutral name. A control
//! beside it stays free (`ARCH-007`, `ARCH-010`, `ARCH-014`, `ARCH-072`,
//! `BUS-004`, `BUS-010`, `BUS-035`, `FIXTURE-004`, `FIXTURE-032`, `SRV-001`,
//! `SRV-170`, `UI-THREAD-020`, `UI-THREAD-051`, `ZK-012`, `ZK-030`, `ZK-031`).

use crate::harness::{Build, Workspace, of};

/// The boundary refusals in `records`.
fn refused(records: &[String]) -> Vec<String> {
    of(records, "refuse")
        .into_iter()
        .filter_map(|l| l.strip_prefix("boundary\t").map(str::to_owned))
        .collect()
}

/// Asserts that `refusals` start, in order, with each of `starts`.
fn assert_refused(refusals: &[String], starts: &[&str]) {
    assert_eq!(refusals.len(), starts.len(), "{refusals:#?}");
    for (refusal, start) in refusals.iter().zip(starts) {
        assert!(refusal.starts_with(start), "{start}\n{refusals:#?}");
    }
}

/// A stand-in for the CRDT engine, with the names the boundaries read.
const LORO: &str = "pub struct LoroDoc;\n\
    pub struct LoroMap;\n\
    pub struct TreeID;\n\
    impl LoroDoc { pub fn get_map(&self, _: &str) -> LoroMap { LoroMap } }\n";

/// A core crate that hides what the boundaries read behind names of its own:
/// an alias, a re-export and a macro.
const CORE: &str = "pub type Doc = loro::LoroDoc;\n\
    pub use std::fs::read as load;\n\
    #[macro_export]\n\
    macro_rules! stamp { () => { ::std::time::Instant::now() } }\n\
    #[macro_export]\n\
    macro_rules! embed { () => { include_bytes!(\"seed.bin\") } }\n\
    pub struct Seq<K>(u64, core::marker::PhantomData<K>);\n\
    impl<K> Seq<K> {\n\
        pub const fn nth(n: u64) -> Self { Seq(n, core::marker::PhantomData) }\n\
        pub const fn get(&self) -> u64 { self.0 }\n\
    }\n\
    pub struct Confidential<T>(T);\n\
    pub struct Passphrase;\n\
    pub type Key = Confidential<Passphrase>;\n\
    pub struct LoroPayload(pub Vec<u8>);\n";

/// The workspace with the CRDT stand-in and the core crate.
fn core(name: &str) -> Workspace {
    let ws = Workspace::new(name);
    ws.file("crates/loro/src/lib.rs", LORO);
    ws.file("crates/ambre-core/src/lib.rs", CORE);
    ws.file("crates/ambre-core/src/seed.bin", "seed");
    ws.compile("loro", "loro", &[], Build::Shipped);
    ws.compile("ambre_core", "ambre-core", &["loro"], Build::Shipped);
    ws
}

// The switchboard reaches the browser through a re-export of the core crate
// and reads the clock through one of its macros. Neither line spells a
// browser crate nor `Instant::now` (`ARCH-072`).
#[test]
fn the_switchboard_reaches_no_browser_or_clock_through_a_re_export_or_a_macro() {
    let ws = core("switchboard");
    ws.file("crates/web-sys/src/lib.rs", "pub struct Window;\n");
    ws.compile("web_sys", "web-sys", &[], Build::Shipped);
    ws.file(
        "crates/helpers/src/lib.rs",
        "pub use web_sys::Window as Host;\n",
    );
    ws.compile("helpers", "helpers", &["web_sys"], Build::Shipped);
    let body = "pub fn decide(_: &helpers::Host) {}\n\
        pub fn at() -> std::time::Duration { ambre_core::stamp!().elapsed() }\n\
        pub fn given(now: u64) -> u64 { now }\n";
    ws.file(
        "crates/ambre-app/src/lib.rs",
        &format!("pub mod switchboard;\n{body}"),
    );
    ws.file("crates/ambre-app/src/switchboard.rs", body);
    let externs = ["web_sys", "helpers", "ambre_core"];
    let app = ws.compile("ambre_app", "ambre-app", &externs, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/switchboard.rs:1: the switchboard's decisions name the browser",
            "crates/ambre-app/src/switchboard.rs:2: the switchboard reads a clock",
        ],
    );
    // The tests of the switchboard set a clock of their own.
    let tests = ws.compile("ambre_app", "ambre-app", &externs, Build::Test);
    assert_eq!(refused(&tests), Vec::<String>::new());
}

// A message names the document through an alias another crate declares, so
// its text spells no `Loro` (`UI-THREAD-020`). A view names an engine state
// through a glob import and the document through the same alias. A tree
// node's identifier passes (`UI-THREAD-020`, `UI-THREAD-051`).
#[test]
fn a_crdt_handle_under_an_alias_or_a_glob_is_refused_in_a_message_or_a_view() {
    let ws = core("crdt");
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod message;\npub mod ui;\npub mod logic { pub struct TrameState(loro::LoroDoc); }\n",
    );
    ws.file(
        "crates/ambre-trame/src/message.rs",
        "pub struct Snapshot(pub ambre_core::Doc);\npub struct Delta(pub ambre_core::LoroPayload);\n",
    );
    ws.file(
        "crates/ambre-trame/src/ui/mod.rs",
        "use crate::logic::*;\n\
         pub fn show(_: &TrameState, _: &ambre_core::Doc) {}\n\
         pub fn node(_: loro::TreeID) {}\n",
    );
    let trame = ws.compile(
        "ambre_trame",
        "ambre-trame",
        &["loro", "ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&trame),
        &[
            "crates/ambre-trame/src/message.rs:1: a message names a CRDT handle",
            "crates/ambre-trame/src/ui/mod.rs:2: the UI thread holds the document",
        ],
    );
}

// A Murmure view, painter or page element takes an engine state. No list
// names it: the gate knows it by the document it holds, at any depth. A
// call that returns one is read through its signature. The view modules are
// read off the path, so a view the crate adds is read too.
// A pure helper of the Logic module, which holds no document, passes, and so
// does the Logic module itself (`UI-THREAD-020`, `UI-THREAD-051`).
#[test]
fn a_murmure_view_that_takes_an_engine_state_is_refused() {
    let ws = core("murmure-views");
    ws.file(
        "crates/ambre-murmure/src/lib.rs",
        "pub mod logic;\npub mod ui;\npub mod chrome;\n\
         pub use logic::channel::ChannelState;\n",
    );
    ws.file(
        "crates/ambre-murmure/src/logic/mod.rs",
        "pub mod channel {\n\
             struct Held { doc: loro::LoroDoc }\n\
             pub struct ChannelState { meta: Option<Held> }\n\
             impl ChannelState { pub fn unread(&self) -> u32 { self.meta.is_some().into() } }\n\
         }\n\
         pub mod space {\n\
             pub struct SpaceState { docs: Vec<(u8, loro::LoroDoc)> }\n\
             pub fn open() -> SpaceState { SpaceState { docs: Vec::new() } }\n\
         }\n\
         pub mod style { pub enum Mark { Bold } pub fn linkable(_: &str) -> bool { true } }\n\
         pub fn hold(_: &channel::ChannelState) {}\n",
    );
    ws.file(
        "crates/ambre-murmure/src/ui/mod.rs",
        "pub mod paint;\n\
         pub fn show(_: &crate::ChannelState) {}\n\
         pub fn mark(_: crate::logic::style::Mark) -> bool { crate::logic::style::linkable(\"\") }\n",
    );
    ws.file(
        "crates/ambre-murmure/src/ui/paint.rs",
        "pub fn badge() -> u32 { crate::logic::space::open(); 0 }\n\
         pub fn count(c: &crate::logic::channel::ChannelState) -> u32 { c.unread() }\n",
    );
    ws.file(
        "crates/ambre-murmure/src/chrome.rs",
        "use crate::logic::space::*;\npub fn panel(_: SpaceState) {}\n",
    );
    let murmure = ws.compile(
        "ambre_murmure",
        "ambre-murmure",
        &["loro", "ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&murmure),
        &[
            "crates/ambre-murmure/src/chrome.rs:2: the UI thread holds the document",
            "crates/ambre-murmure/src/ui/mod.rs:2: the UI thread holds the document",
            "crates/ambre-murmure/src/ui/paint.rs:1: the UI thread holds the document",
            "crates/ambre-murmure/src/ui/paint.rs:2: the UI thread holds the document",
        ],
    );
}

// An engine's handler holds the queue's sender under a name a core crate
// gives it. The dispatcher, outside `src/logic`, may (`BUS-010`).
#[test]
fn a_queue_under_a_neutral_name_is_refused_in_a_command_handler() {
    let ws = Workspace::new("handlers");
    ws.file(
        "crates/ambre-bus/src/lib.rs",
        "pub mod conduit { pub struct Sender<T>(pub T); }\n",
    );
    ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "pub use ambre_bus::conduit::Sender as Pipe;\n",
    );
    ws.compile("ambre_core", "ambre-core", &["ambre_bus"], Build::Shipped);
    let body = "pub fn handle(_: &ambre_core::Pipe<u8>) {}\n";
    ws.file(
        "crates/ambre-toile/src/lib.rs",
        &format!("pub mod logic;\n{body}"),
    );
    ws.file("crates/ambre-toile/src/logic/mod.rs", body);
    let toile = ws.compile(
        "ambre_toile",
        "ambre-toile",
        &["ambre_bus", "ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&toile),
        &["crates/ambre-toile/src/logic/mod.rs:1: a command handler sends on a queue"],
    );
}

/// The bus stand-in: the queue, the port, the lane and the service family.
const BUS: &str = "pub enum LogicCommand { Go }\n\
    pub enum SyncCommand { Push }\n\
    pub enum Request {}\n\
    pub type ToLogic = conduit::Sender<LogicCommand>;\n\
    pub mod conduit {\n\
        pub struct Sender<T>(pub T);\n\
        impl<T> Sender<T> { pub fn push(&self) {} }\n\
        pub struct Receiver<T>(pub T);\n\
        pub fn to_worker<T: Default>() -> (Sender<T>, Receiver<T>) { (Sender(T::default()), Receiver(T::default())) }\n\
    }\n\
    pub struct Port;\n\
    impl Port { pub fn embedded<T>(_: conduit::Sender<T>) -> Self { Port } }\n\
    impl Default for LogicCommand { fn default() -> Self { LogicCommand::Go } }\n\
    impl Default for SyncCommand { fn default() -> Self { SyncCommand::Push } }\n\
    pub fn next_request() -> u64 { 0 }\n";

// A launcher file reaches the raw sender of the Logic queue through an
// alias the bus declares, so its text spells no `Sender<LogicCommand>`. It
// builds a port through an alias of the port. The outbox may do both, and
// the relay flushes it (`BUS-035`, `BUS-036`).
#[test]
fn the_raw_logic_sender_under_an_alias_is_refused_past_the_outbox() {
    let ws = Workspace::new("outbox");
    ws.file("crates/ambre-bus/src/lib.rs", BUS);
    ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod outbox;\npub mod view;\n\
         use ambre_bus::{LogicCommand, conduit};\n\
         pub fn boot() {\n\
             let (tx, _rx) = conduit::to_worker::<LogicCommand>();\n\
             let out = outbox::ShellOutbox::new(tx);\n\
             out.flush();\n\
             let _ = out.take_refusal();\n\
         }\n",
    );
    ws.file(
        "crates/ambre-app/src/outbox.rs",
        "pub struct ShellOutbox(ambre_bus::ToLogic);\n\
         impl ShellOutbox {\n\
             pub fn new(tx: ambre_bus::ToLogic) -> Self { Self(tx) }\n\
             pub fn flush(&self) { self.0.push() }\n\
             pub fn take_refusal(&self) -> bool { false }\n\
             pub fn port(self) -> ambre_bus::Port { ambre_bus::Port::embedded(self.0) }\n\
         }\n",
    );
    ws.file(
        "crates/ambre-app/src/view.rs",
        "type Door = ambre_bus::Port;\n\
         pub fn send(tx: ambre_bus::ToLogic) -> Door { tx.push(); Door::embedded(tx) }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", &["ambre_bus"], Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/view.rs:2: the launcher writes past its outbox",
            "crates/ambre-app/src/view.rs:2: the raw UI to Logic sender is used past its outbox",
        ],
    );
    // A test of the launcher may hold the raw sender.
    let tests = ws.compile("ambre_app", "ambre-app", &["ambre_bus"], Build::Test);
    assert_eq!(refused(&tests), Vec::<String>::new());
    let facts = of(&app, "fact");
    for fact in [
        "ui-queue",
        "outbox-move",
        "outbox-flush",
        "outbox-take_refusal",
    ] {
        assert!(
            facts
                .iter()
                .any(|f| f.starts_with(&format!("{fact}\t-\tcrates/ambre-app/src/lib.rs:"))),
            "{fact}: {facts:#?}"
        );
    }
}

// A generator module reads a file through a re-export of the core crate and
// embeds one through its macro: neither line spells `std::fs` nor
// `include_bytes!`. The tests of the module may read (`FIXTURE-032`).
#[test]
fn a_generator_reading_a_file_through_a_re_export_or_a_macro_is_refused() {
    let ws = core("generator-reads");
    ws.file("crates/ambre-toile/src/seed.bin", "seed");
    ws.file(
        "crates/ambre-toile/src/lib.rs",
        "pub mod fixtures;\npub fn load() { let _ = ambre_core::load(\"x\"); }\n",
    );
    ws.file(
        "crates/ambre-toile/src/fixtures.rs",
        "pub fn seed() -> usize { ambre_core::load(\"real.ambre\").map_or(0, |b| b.len()) }\n\
         pub fn embedded() -> usize { ambre_core::embed!().len() }\n\
         pub fn host() -> Option<&'static str> { option_env!(\"HOME\") }\n\
         #[cfg(test)]\n\
         mod tests { #[test] fn reads() { let _ = std::fs::read(\"x\"); } }\n",
    );
    let externs = ["ambre_core"];
    // The corpus driver reads and writes the corpus by design.
    ws.file("crates/ambre-logic/src/lib.rs", "pub mod fixtures;\n");
    ws.file(
        "crates/ambre-logic/src/fixtures.rs",
        "pub fn load() -> usize { ambre_core::load(\"corpus\").map_or(0, |b| b.len()) }\n",
    );
    let logic = ws.compile("ambre_logic", "ambre-logic", &externs, Build::Shipped);
    assert_eq!(refused(&logic), Vec::<String>::new());
    let toile = ws.compile("ambre_toile", "ambre-toile", &externs, Build::Shipped);
    assert_refused(
        &refused(&toile),
        &[
            "crates/ambre-toile/src/fixtures.rs:1: a generator reads besides its seed",
            "crates/ambre-toile/src/fixtures.rs:2: a generator reads besides its seed: `include_bytes!`",
            "crates/ambre-toile/src/fixtures.rs:3: a generator reads besides its seed: `option_env!`",
        ],
    );
    let tests = ws.compile("ambre_toile", "ambre-toile", &externs, Build::Test);
    assert_eq!(refused(&tests), Vec::<String>::new());
}

// A generator reaches the document through a helper of the state, whose name
// is no container, and through the `FixtureGen` impl. The reading half, in
// an inherent impl, names the document freely (`FIXTURE-004`).
#[test]
fn a_generator_reaching_the_document_through_a_helper_is_refused() {
    let ws = core("generators");
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod logic;\n\
         pub struct TrameState { doc: loro::LoroDoc }\n\
         impl TrameState {\n\
             pub fn document(&self) -> &loro::LoroDoc { &self.doc }\n\
             pub fn command(&mut self) {}\n\
         }\n\
         pub trait FixtureGen { fn build(&self, state: &mut TrameState); }\n",
    );
    ws.file("crates/ambre-trame/src/logic/mod.rs", "pub mod fixtures;\n");
    ws.file(
        "crates/ambre-trame/src/logic/fixtures.rs",
        "use crate::{FixtureGen, TrameState};\n\
         pub struct Grid;\n\
         impl FixtureGen for Grid {\n\
             fn build(&self, state: &mut TrameState) { let _ = state.document().get_map(\"cells\"); }\n\
         }\n\
         fn by_command(state: &mut TrameState) { state.command(); }\n\
         fn by_helper(state: &TrameState) { let _ = state.document(); }\n\
         impl TrameState { pub fn dump(&self) -> &loro::LoroDoc { self.document() } }\n",
    );
    let trame = ws.compile("ambre_trame", "ambre-trame", &["loro"], Build::Shipped);
    let trame = refused(&trame);
    assert_refused(
        &trame,
        &[
            "crates/ambre-trame/src/logic/fixtures.rs:4: a generator reaches the CRDT",
            "crates/ambre-trame/src/logic/fixtures.rs:7: a generator reaches the CRDT: `LoroDoc`",
        ],
    );
}

// The worker builds a request id with the family left to inference, which
// the scan could not see. Reading the number of one it was handed is no
// allocation (`BUS-004`).
#[test]
fn a_request_id_built_with_an_inferred_family_is_refused_in_the_worker() {
    let ws = core("requests");
    ws.file("crates/ambre-bus/src/lib.rs", BUS);
    ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "use ambre_core::Seq;\nuse ambre_bus::Request;\n\
         pub fn mint() -> Seq<Request> { Seq::nth(3) }\n\
         pub fn echo(id: &Seq<Request>) -> u64 { id.get() }\n\
         pub fn other() -> Seq<u8> { Seq::nth(3) }\n\
         pub fn issue() -> u64 { ambre_bus::next_request() }\n",
    );
    let sync = ws.compile(
        "ambre_sync",
        "ambre-sync",
        &["ambre_core", "ambre_bus"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&sync),
        &[
            "crates/ambre-sync/src/lib.rs:3: the worker allocates a request id: `Seq::nth`",
            "crates/ambre-sync/src/lib.rs:6: the worker allocates a request id: `next_request`",
        ],
    );
    let tests = ws.compile(
        "ambre_sync",
        "ambre-sync",
        &["ambre_core", "ambre_bus"],
        Build::Test,
    );
    assert_eq!(refused(&tests), Vec::<String>::new());
}

// A serialized request of the worker holds the passphrase under an alias the
// core crate declares, so its text spells no `Confidential`. A key a macro
// writes names a password (`ZK-012`).
#[test]
fn a_passphrase_under_an_alias_or_a_macro_key_is_refused_in_a_request() {
    let ws = core("passphrase");
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        "pub trait Serialize {}\n\
         macro_rules! key { () => { \"user_password\" } }\n\
         pub struct Login { pub name: String, pub key: ambre_core::Key }\n\
         #[automatically_derived]\n\
         impl Serialize for Login {}\n\
         pub struct Local { pub key: ambre_core::Key }\n\
         pub fn field() -> &'static str { key!() }\n",
    );
    let sync = ws.compile("ambre_sync", "ambre-sync", &["ambre_core"], Build::Shipped);
    assert_refused(
        &refused(&sync),
        &[
            "crates/ambre-sync/src/lib.rs:3: a serialized field holds a secret: `key: Confidential`",
            "crates/ambre-sync/src/lib.rs:7: a request key names a passphrase: `user_password`",
        ],
    );
}

/// A stand-in for the log crate: a macro that records a value as `debug`.
const TRACING: &str = "pub mod field { pub fn debug<T: core::fmt::Debug>(t: T) -> T { t } }\n\
    #[macro_export]\n\
    macro_rules! info { ($e:expr) => { let _ = $crate::field::debug(&$e); } }\n";

// The relay logs a payload under a neutral name, as a field and as a
// formatted argument. Its length passes (`SRV-170`). A file that takes a
// ciphertext whole logs nothing (`SRV-001`).
#[test]
fn a_payload_under_a_neutral_name_is_refused_in_a_log_line() {
    let ws = Workspace::new("relay-logs");
    ws.file("crates/tracing/src/lib.rs", TRACING);
    ws.compile("tracing", "tracing", &[], Build::Shipped);
    ws.file(
        "crates/ambre-server/src/lib.rs",
        "pub mod seal;\npub mod quiet;\n\
         pub fn store(frame: Vec<u8>) {\n\
             let x = frame.clone();\n\
             tracing::info!(x);\n\
             tracing::info!(x.len());\n\
             eprintln!(\"{x:?}\");\n\
         }\n",
    );
    ws.file(
        "crates/ambre-server/src/seal.rs",
        "pub struct Ciphertext;\n\
         pub fn open(c: &Ciphertext) -> usize { eprintln!(\"open\"); let _ = c; 0 }\n",
    );
    ws.file(
        "crates/ambre-server/src/quiet.rs",
        "pub fn open(c: &crate::seal::Ciphertext) -> usize { let _ = c; 0 }\n",
    );
    let server = ws.compile("ambre_server", "ambre-server", &["tracing"], Build::Shipped);
    assert_refused(
        &refused(&server),
        &[
            "crates/ambre-server/src/lib.rs:5: a log line or span names a payload",
            "crates/ambre-server/src/lib.rs:7: a log line or span names a payload",
            "crates/ambre-server/src/seal.rs:2: a file that logs reads a payload's bytes",
        ],
    );
    let tests = ws.compile("ambre_server", "ambre-server", &["tracing"], Build::Test);
    assert_eq!(refused(&tests), Vec::<String>::new());
}

/// A stand-in for the log crate with a loud and a quiet macro, each of which
/// records a value as `display`.
const LEVELS: &str = "pub mod field { pub fn display<T: core::fmt::Display>(t: T) -> T { t } }\n\
    #[macro_export]\n\
    macro_rules! info { ($e:expr) => { let _ = $crate::field::display(&$e); } }\n\
    #[macro_export]\n\
    macro_rules! debug { ($e:expr) => { let _ = $crate::field::display(&$e); } }\n";

/// A stand-in for the core identifier, with a document marker.
const IDS: &str = "pub struct Document;\n\
    pub struct Id<E>(u128, core::marker::PhantomData<E>);\n\
    impl<E> core::fmt::Display for Id<E> {\n\
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, \"{}\", self.0) }\n\
    }\n\
    pub struct RoomPseudonym;\n\
    pub struct FixedBytes<R>([u8; 16], core::marker::PhantomData<R>);\n\
    impl<R> core::fmt::Display for FixedBytes<R> {\n\
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, \"{:?}\", self.0) }\n\
    }\n";

// The relay logs a document identifier at the info level, bare and inside
// an error. It logs a client address at the debug level. Each is refused.
// The identifier at the debug level passes, as a count does. The SFU logs a
// room pseudonym at the info level (`ZK-030`, `ZK-031`).
#[test]
fn an_identifier_at_the_info_level_and_an_address_at_any_level_are_refused_in_a_log_line() {
    let ws = Workspace::new("naming-logs");
    ws.file("crates/tracing/src/lib.rs", LEVELS);
    ws.compile("tracing", "tracing", &[], Build::Shipped);
    ws.file("crates/ambre-core/src/lib.rs", IDS);
    ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    ws.file(
        "crates/ambre-server/src/lib.rs",
        "use ambre_core::{Document, Id};\n\
         pub struct Failed { pub doc: Id<Document> }\n\
         impl core::fmt::Display for Failed {\n\
             fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, \"x\") }\n\
         }\n\
         pub fn store(doc: Id<Document>, e: Failed, peer: std::net::SocketAddr, n: usize) {\n\
             tracing::info!(doc);\n\
             tracing::info!(e);\n\
             tracing::debug!(peer);\n\
             tracing::debug!(doc);\n\
             tracing::info!(n);\n\
         }\n",
    );
    let server = ws.compile(
        "ambre_server",
        "ambre-server",
        &["tracing", "ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&server),
        &[
            "crates/ambre-server/src/lib.rs:7: a log line at the info level or above names an identifier",
            "crates/ambre-server/src/lib.rs:8: a log line at the info level or above names an identifier",
            "crates/ambre-server/src/lib.rs:9: a log line names a network address",
        ],
    );
    ws.file(
        "crates/ambre-sfu/src/lib.rs",
        "pub fn seat(room: ambre_core::FixedBytes<ambre_core::RoomPseudonym>) {\n\
             tracing::info!(room);\n\
         }\n",
    );
    let sfu = ws.compile(
        "ambre_sfu",
        "ambre-sfu",
        &["tracing", "ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&sfu),
        &[
            "crates/ambre-sfu/src/lib.rs:2: a log line at the info level or above names an identifier",
        ],
    );
    let tests = ws.compile(
        "ambre_server",
        "ambre-server",
        &["tracing", "ambre_core"],
        Build::Test,
    );
    assert_eq!(refused(&tests), Vec::<String>::new());
}

// The launcher starts the Logic runtime twice, once through an alias the
// scan never counted (`ARCH-010`). A panic hook that never latches is
// refused, and a worker error that latches through a binding passes
// (`ARCH-014`).
#[test]
fn every_start_and_every_failure_path_of_the_launcher_is_recorded() {
    let ws = Workspace::new("launcher-wiring");
    ws.file("crates/ambre-logic/src/lib.rs", "pub fn run() {}\n");
    ws.compile("ambre_logic", "ambre-logic", &[], Build::Shipped);
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        "pub fn signal_fatal(_: &str) {}\n",
    );
    ws.compile("ambre_canvas", "ambre-canvas", &[], Build::Shipped);
    ws.file(
        "crates/web-sys/src/lib.rs",
        "pub struct Worker;\nimpl Worker { pub fn set_onerror(&self, _: Option<&dyn Fn()>) {} }\n",
    );
    ws.compile("web_sys", "web-sys", &[], Build::Shipped);
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "use ambre_logic::run as engine;\n\
         pub fn boot(worker: &web_sys::Worker) {\n\
             ambre_logic::run();\n\
             engine();\n\
             std::panic::set_hook(Box::new(|_| {}));\n\
             let onerror = || ambre_canvas::signal_fatal(\"the Sync worker failed\");\n\
             worker.set_onerror(Some(&onerror));\n\
         }\n",
    );
    let externs = ["ambre_logic", "ambre_canvas", "web_sys"];
    let app = ws.compile("ambre_app", "ambre-app", &externs, Build::Shipped);
    assert_refused(
        &refused(&app),
        &["crates/ambre-app/src/lib.rs:5: a panic-hook waits on a queue"],
    );
    let facts = of(&app, "fact");
    assert_eq!(
        facts,
        vec![
            "logic-start\t-\tcrates/ambre-app/src/lib.rs:3".to_owned(),
            "logic-start\t-\tcrates/ambre-app/src/lib.rs:4".to_owned(),
            "panic-hook\topen\tcrates/ambre-app/src/lib.rs:5".to_owned(),
            "worker-error\tlatched\tcrates/ambre-app/src/lib.rs:7".to_owned(),
        ]
    );
}

// The frame loop runs the relay its function takes, and two call sites pass
// one. The first drains the Logic to Sync queue. The second, in another
// file, does not, and a scan of `lib.rs` for one drain let it through
// (`ARCH-007`).
#[test]
fn a_frame_relay_that_does_not_drain_the_sync_queue_is_refused() {
    let ws = Workspace::new("frame-relay");
    ws.file("crates/ambre-bus/src/lib.rs", BUS);
    ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        "pub fn drive_frame(_: &u8, mut relay: impl FnMut(), mut render: impl FnMut()) { relay(); render(); }\n",
    );
    ws.compile("ambre_canvas", "ambre-canvas", &[], Build::Shipped);
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod host;\npub mod other;\n\
         use ambre_bus::{SyncCommand, conduit};\n\
         pub struct Start;\n\
         impl Start { pub fn drain<T>(&self, _: &conduit::Receiver<T>, _: impl FnMut(T)) {} }\n\
         pub fn boot() {\n\
             let (_tx, rx) = conduit::to_worker::<SyncCommand>();\n\
             let start = Start;\n\
             let pre_frame = move || start.drain(&rx, |_| {});\n\
             host::run_with(pre_frame);\n\
         }\n",
    );
    ws.file(
        "crates/ambre-app/src/host.rs",
        "pub fn run_with<P: FnMut()>(pre_frame: P) {\n\
             let mut relay = pre_frame;\n\
             ambre_canvas::drive_frame(&0, || relay(), || {});\n\
         }\n",
    );
    ws.file(
        "crates/ambre-app/src/other.rs",
        "pub fn boot() { crate::host::run_with(|| {}); }\n\
         pub fn draw() { ambre_canvas::drive_frame(&0, || {}, || {}); }\n",
    );
    let externs = ["ambre_bus", "ambre_canvas"];
    let app = ws.compile("ambre_app", "ambre-app", &externs, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/other.rs:1: the relay does not drain the Logic to Sync queue",
            "crates/ambre-app/src/other.rs:2: the frame loop runs no relay the launcher passes in",
        ],
    );
    assert_eq!(
        of(&app, "fact"),
        vec!["frame-relay\tdrains\tcrates/ambre-app/src/lib.rs:10".to_owned()]
    );
}
