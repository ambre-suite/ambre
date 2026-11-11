//! The code rules `crate_boundaries.py` still read by regular expression,
//! read with types. Each test plants a breach that a scan of the text let
//! through. The breach is a re-export, an alias, a `use` rename, a macro or
//! a literal `concat!` builds. The driver refuses it, and a control beside
//! it stays free.

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

/// The facts of `name` in `records`: the value and the line.
fn facts(records: &[String], name: &str) -> Vec<String> {
    of(records, "fact")
        .into_iter()
        .filter_map(|l| l.strip_prefix(&format!("{name}\t")).map(str::to_owned))
        .collect()
}

/// A stand-in for the browser bindings, with the names the rules read.
const WEB_SYS: &str = "pub struct HtmlCanvasElement;\n\
    pub struct Storage;\n\
    pub struct Document;\n\
    impl Document { pub fn create_element(&self, _: &str) {} }\n\
    pub struct MouseEvent;\n\
    impl MouseEvent {\n\
        pub fn offset_x(&self) -> i32 { 0 }\n\
        pub fn client_x(&self) -> i32 { 0 }\n\
    }\n";

/// A helper crate of the workspace that hides foreign names behind names
/// of its own. Its macros write a forbidden name at the call site.
const HELPERS: &str = "pub use web_sys::HtmlCanvasElement as Surface;\n\
    pub use std::sync::Condvar as Latch;\n\
    pub use csv::Reader as Table;\n\
    pub type Store = web_sys::Storage;\n\
    pub use wgpu::ComputePass as Pass;\n\
    #[macro_export]\n\
    macro_rules! at { ($e:expr) => { $e.offset_x() } }\n\
    #[macro_export]\n\
    macro_rules! page { ($e:expr) => { $e.client_x() } }\n\
    #[macro_export]\n\
    macro_rules! face { () => { include_bytes!(concat!(\"face.\", \"ttf\")) } }\n";

/// The workspace with the foreign stand-ins and the helper crate.
fn helpers(name: &str) -> Workspace {
    let ws = Workspace::new(name);
    ws.file("crates/web-sys/src/lib.rs", WEB_SYS);
    ws.file("crates/csv/src/lib.rs", "pub struct Reader;\n");
    ws.file("crates/wgpu/src/lib.rs", "pub struct ComputePass;\n");
    ws.file("crates/ambre-helpers/src/lib.rs", HELPERS);
    ws.file("crates/ambre-canvas/src/face.ttf", "font");
    ws.compile("web_sys", "web-sys", &[], Build::Shipped);
    ws.compile("csv", "csv", &[], Build::Shipped);
    ws.compile("wgpu", "wgpu", &[], Build::Shipped);
    ws.compile(
        "ambre_helpers",
        "ambre-helpers",
        &["web_sys", "csv", "wgpu"],
        Build::Shipped,
    );
    ws
}

/// The externs of a crate that reads the helpers.
const HELPED: &[&str] = &["web_sys", "csv", "wgpu", "ambre_helpers"];

// Each name reaches the UI thread through a re-export, an alias or a macro.
// No line of the chrome spells it. The surface itself may name the canvas (`UI-THREAD-058`, `UI-THREAD-002`, `TRAME-IO-001`,
// `UI-THREAD-004`, `UI-THREAD-005`, `UI-THREAD-001`).
#[test]
fn a_ui_thread_name_under_a_re_export_an_alias_or_a_macro_is_refused() {
    let ws = helpers("ui-thread");
    ws.file(
        "crates/ambre-chrome/src/lib.rs",
        "pub fn paint(_: &ambre_helpers::Surface) {}\n\
         pub fn hold() -> ambre_helpers::Latch { ambre_helpers::Latch::new() }\n\
         pub fn read(_: ambre_helpers::Table) {}\n\
         pub fn keep(_: ambre_helpers::Store) {}\n\
         pub fn pass(_: ambre_helpers::Pass) {}\n\
         pub fn point(e: &web_sys::MouseEvent) -> i32 { ambre_helpers::at!(e) }\n\
         pub fn build(d: &web_sys::Document) { d.create_element(\"canvas\") }\n",
    );
    let chrome = ws.compile("ambre_chrome", "ambre-chrome", HELPED, Build::Shipped);
    assert_refused(
        &refused(&chrome),
        &[
            "crates/ambre-chrome/src/lib.rs:1: a page element draws",
            "crates/ambre-chrome/src/lib.rs:2: the UI thread waits",
            "crates/ambre-chrome/src/lib.rs:3: the UI thread converts a file",
            "crates/ambre-chrome/src/lib.rs:4: a persistence path the rule does not name",
            "crates/ambre-chrome/src/lib.rs:5: outside the shared graphics subset",
            "crates/ambre-chrome/src/lib.rs:6: a canvas point read off the host",
            "crates/ambre-chrome/src/lib.rs:7: a page element draws",
        ],
    );
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        "pub fn paint(_: &ambre_helpers::Surface) {}\n",
    );
    let canvas = ws.compile("ambre_canvas", "ambre-canvas", HELPED, Build::Shipped);
    assert_eq!(refused(&canvas), Vec::<String>::new());
}

// An interface file reads a page point through a macro and names the canvas
// through `concat!`, so neither half is spelled. Each half alone is free
// (`UI-THREAD-001`).
#[test]
fn a_page_point_beside_the_canvas_built_by_concat_is_refused_in_an_interface() {
    let ws = helpers("page-point");
    ws.file(
        "crates/ambre-chrome/src/lib.rs",
        "pub mod both;\npub mod half;\n",
    );
    ws.file(
        "crates/ambre-chrome/src/both.rs",
        "pub fn at(e: &web_sys::MouseEvent) -> (i32, &'static str) {\n\
             (ambre_helpers::page!(e), concat!(\"ambre-\", \"canvas\"))\n\
         }\n",
    );
    ws.file(
        "crates/ambre-chrome/src/half.rs",
        "pub fn at(e: &web_sys::MouseEvent) -> i32 { ambre_helpers::page!(e) }\n",
    );
    let chrome = ws.compile("ambre_chrome", "ambre-chrome", HELPED, Build::Shipped);
    assert_refused(
        &refused(&chrome),
        &["crates/ambre-chrome/src/both.rs: an interface component turns a page point"],
    );
}

// A macro of another crate includes a font whose name `concat!` builds, so
// no line spells a font file. The compile loaded it (`CANVAS-018`).
#[test]
fn a_font_a_foreign_macro_includes_is_refused() {
    let ws = helpers("font");
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        "pub static FACE: &[u8] = ambre_helpers::face!();\n",
    );
    let canvas = ws.compile("ambre_canvas", "ambre-canvas", HELPED, Build::Shipped);
    assert_refused(
        &refused(&canvas),
        &["crates/ambre-canvas/src/face.ttf: a font embedded in the binary of ambre-canvas"],
    );
}

// The renderer names a palette colour through a `use` rename, so its line
// spells no `Color::`. A colour the view hands over stays free
// (`CANVAS-060`).
#[test]
fn a_colour_under_a_rename_is_refused_in_the_renderer() {
    let ws = Workspace::new("colour");
    ws.file(
        "crates/peniko/src/lib.rs",
        "pub struct AlphaColor;\n\
         impl AlphaColor { pub const WHITE: AlphaColor = AlphaColor; }\n\
         pub type Color = AlphaColor;\n",
    );
    ws.compile("peniko", "peniko", &[], Build::Shipped);
    ws.file(
        "crates/ambre-canvas/src/lib.rs",
        "pub mod render;\n\
         pub fn backdrop() -> peniko::Color { peniko::Color::WHITE }\n",
    );
    ws.file(
        "crates/ambre-canvas/src/render/mod.rs",
        "use peniko::Color as Paint;\n\
         pub fn clear() -> Paint { Paint::WHITE }\n\
         pub fn given(c: Paint) -> Paint { c }\n",
    );
    let canvas = ws.compile("ambre_canvas", "ambre-canvas", &["peniko"], Build::Shipped);
    assert_refused(
        &refused(&canvas),
        &["crates/ambre-canvas/src/render/mod.rs:2: the renderer holds a colour of its own"],
    );
}

/// The drawing crate, with the kinetic model and a re-export of its decay.
const DRAW: &str = "pub mod kinetic;\n\
    pub use kinetic::Decay as Fade;\n\
    pub use kinetic::ScrollStep as Step;\n\
    pub struct Rect;\n\
    pub struct ViewState;\n";

/// The kinetic model of the drawing crate.
const KINETIC: &str = "pub struct Decay;\n\
    pub struct ScrollStep(pub f64);\n\
    impl ScrollStep { pub fn dragged(a: f64, b: f64) -> Self { ScrollStep(a - b) } }\n";

/// The workspace with the drawing crate and a core crate whose macro moves
/// an offset by a distance.
fn draw(name: &str) -> Workspace {
    let ws = Workspace::new(name);
    ws.file("crates/ambre-draw/src/lib.rs", DRAW);
    ws.file("crates/ambre-draw/src/kinetic.rs", KINETIC);
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "#[macro_export]\n\
         macro_rules! nudge { ($v:expr, $d:expr) => { $v.scroll_y += $d } }\n",
    );
    ws.compile("ambre_draw", "ambre-draw", &[], Build::Shipped);
    ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    ws
}

// A view names the decay through the re-export, builds a drag step through
// another, and moves its offset through a macro. None of the three lines
// spells what the scan read. A setter that takes a step of its own name is
// no step of the model (`KINETIC-012`, `KINETIC-001`).
#[test]
fn a_kinetic_name_a_drag_step_and_an_offset_under_a_re_export_or_a_macro_are_refused() {
    let ws = draw("kinetic");
    ws.file(
        "crates/ambre-prose/src/lib.rs",
        "pub mod view;\n\
         pub fn fade() -> ambre_draw::Fade { ambre_draw::Fade }\n\
         pub fn drag() -> ambre_draw::Step { ambre_draw::Step::dragged(1.0, 0.0) }\n",
    );
    ws.file(
        "crates/ambre-prose/src/view/mod.rs",
        "pub struct ScrollStep(pub f64);\n\
         pub struct View { scroll_y: f64 }\n\
         impl View {\n\
             pub fn scroll_by(&mut self, step: ScrollStep) { self.scroll_y += step.0; }\n\
             pub fn jump(&mut self, d: f64) { ambre_core::nudge!(self, d); }\n\
         }\n",
    );
    let externs = ["ambre_draw", "ambre_core"];
    let prose = ws.compile("ambre_prose", "ambre-prose", &externs, Build::Shipped);
    assert_refused(
        &refused(&prose),
        &[
            "crates/ambre-prose/src/lib.rs:2: a kinetic model outside `kinetic.rs`: `Decay`",
            "crates/ambre-prose/src/lib.rs:3: a drag step built away",
            "crates/ambre-prose/src/view/mod.rs: the scroll setter `scroll_by` takes no model step",
            "crates/ambre-prose/src/view/mod.rs:5: an offset moved by a distance outside its setter",
        ],
    );
}

// The setter takes the model's step under a `use` rename, and moves the
// offset in its own body: both are the model's (`KINETIC-001`).
#[test]
fn a_setter_that_takes_the_model_step_under_a_rename_moves_its_offset() {
    let ws = draw("setter");
    ws.file("crates/ambre-prose/src/lib.rs", "pub mod view;\n");
    ws.file(
        "crates/ambre-prose/src/view/mod.rs",
        "use ambre_draw::Step as Move;\n\
         pub struct View { scroll_y: f64 }\n\
         impl View {\n\
             pub fn scroll_by(&mut self, step: Move) { self.scroll_y += step.0; }\n\
         }\n",
    );
    let externs = ["ambre_draw", "ambre_core"];
    let prose = ws.compile("ambre_prose", "ambre-prose", &externs, Build::Shipped);
    assert_eq!(refused(&prose), Vec::<String>::new());
}

// A primitive takes a parameter whose type an alias names `Rect`, so the
// listed word hides a screen state. A bare number not named as a width
// fails too (`CANVAS-056`).
#[test]
fn a_drawing_primitive_that_takes_screen_state_under_an_alias_is_refused() {
    let ws = Workspace::new("primitive");
    ws.file(
        "crates/ambre-draw/src/lib.rs",
        "pub struct Rect;\n\
         pub struct ViewState;\n\
         mod screen { pub type Rect = super::ViewState; }\n\
         pub trait DrawSurface {\n\
             fn fill_rect(&mut self, rect: Rect);\n\
             fn fill_view(&mut self, rect: screen::Rect);\n\
             fn scaled(&mut self, density: f64);\n\
             fn stroke(&mut self, rect: Rect, width: f64);\n\
         }\n",
    );
    let draw = ws.compile("ambre_draw", "ambre-draw", &[], Build::Shipped);
    assert_refused(
        &refused(&draw),
        &[
            "crates/ambre-draw/src/lib.rs:6: `fill_view` takes `rect: ViewState`",
            "crates/ambre-draw/src/lib.rs:7: `scaled` takes `density: f64`",
        ],
    );
}

// A walk reaches a camera through a re-export of a listed module, so the
// path it writes names `crate::render`. The camera is declared in the
// interface module. A name of the screen's state fails under any path
// (`CANVAS-074`).
#[test]
fn a_walk_that_reaches_screen_state_through_a_listed_re_export_is_refused() {
    let ws = Workspace::new("walk");
    ws.file(
        "crates/ambre-draw/src/lib.rs",
        "pub trait DocumentWalk { fn walk(&self); }\n",
    );
    ws.compile("ambre_draw", "ambre-draw", &[], Build::Shipped);
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod render;\n\
         pub mod ui { pub struct Camera; pub struct GridUiState; }\n\
         pub const TOTAL_ROWS: u32 = 1;\n",
    );
    ws.file(
        "crates/ambre-trame/src/render/mod.rs",
        "pub mod walk;\n\
         pub use crate::ui::Camera as Zoom;\n\
         pub struct Layout;\n",
    );
    ws.file(
        "crates/ambre-trame/src/render/walk.rs",
        "use ambre_draw::DocumentWalk;\n\
         pub struct SheetWalk;\n\
         impl DocumentWalk for SheetWalk {\n\
             fn walk(&self) {\n\
                 let _ = crate::render::Layout;\n\
                 let _ = crate::TOTAL_ROWS;\n\
                 let _ = crate::render::Zoom;\n\
                 let _: Option<crate::ui::GridUiState> = None;\n\
             }\n\
         }\n",
    );
    let trame = ws.compile(
        "ambre_trame",
        "ambre-trame",
        &["ambre_draw"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&trame),
        &[
            "crates/ambre-trame/src/render/walk.rs:7: the walk reaches `crate::ui`",
            "crates/ambre-trame/src/render/walk.rs:8: the walk names screen state",
        ],
    );
}

// A Trame file reaches an axis list through a `use` rename of the root enum,
// and spells a container name through `concat!`. The registry's own macro
// and the tab strip stay free (`TRAME-275`).
#[test]
fn an_axis_list_under_a_rename_or_a_concat_is_refused_outside_the_axis_module() {
    let ws = Workspace::new("axis");
    ws.file(
        "crates/loro/src/lib.rs",
        "pub struct LoroDoc;\n\
         impl LoroDoc { pub fn get_movable_list(&self, _: &str) {} }\n",
    );
    ws.compile("loro", "loro", &[], Build::Shipped);
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod logic;\npub mod model;\npub mod sheets;\n",
    );
    ws.file("crates/ambre-trame/src/logic/mod.rs", "pub mod schema;\n");
    ws.file(
        "crates/ambre-trame/src/logic/schema.rs",
        "macro_rules! roots {\n\
             ($($v:ident = $s:literal,)*) => {\n\
                 pub enum SheetRoot { $($v,)* }\n\
                 impl SheetRoot { pub fn as_str(&self) -> &'static str { match self { $(Self::$v => $s,)* } } }\n\
             };\n\
         }\n\
         roots! {\n\
             Rows = \"rows\",\n\
             SheetOrder = \"sheet_order\",\n\
         }\n",
    );
    ws.file("crates/ambre-trame/src/model.rs", "pub mod axis;\n");
    ws.file(
        "crates/ambre-trame/src/model/axis.rs",
        "pub fn rows(d: &loro::LoroDoc) { d.get_movable_list(crate::logic::schema::SheetRoot::Rows.as_str()) }\n",
    );
    ws.file(
        "crates/ambre-trame/src/sheets.rs",
        "use crate::logic::schema::SheetRoot as R;\n\
         pub fn root() -> R { R::Rows }\n\
         pub const NAMED: &str = concat!(\"ro\", \"ws\");\n\
         pub fn label() -> &'static str { \"rows\" }\n\
         pub fn tabs(d: &loro::LoroDoc) { d.get_movable_list(R::SheetOrder.as_str()) }\n\
         pub fn lines(d: &loro::LoroDoc) { d.get_movable_list(NAMED) }\n",
    );
    let trame = ws.compile("ambre_trame", "ambre-trame", &["loro"], Build::Shipped);
    let axis = "an axis list reached outside `model/axis.rs`";
    assert_refused(
        &refused(&trame),
        &[
            &format!("crates/ambre-trame/src/sheets.rs:2: {axis}: `Rows`"),
            &format!("crates/ambre-trame/src/sheets.rs:3: {axis}: \"rows\""),
            &format!("crates/ambre-trame/src/sheets.rs:6: {axis}: `get_movable_list`"),
        ],
    );
}

// An identifier field of a message holds a `String` under an alias, so its
// type reads as a type of its own (`BUS-029`).
#[test]
fn an_identifier_field_of_a_message_under_an_alias_is_refused() {
    let ws = Workspace::new("bus-ids");
    ws.file(
        "crates/ambre-bus/src/lib.rs",
        "pub type SheetId = String;\n\
         pub struct Sheet(pub u32);\n\
         pub enum Message { Rename { sheet_id: SheetId }, Show { sheet_id: Sheet } }\n",
    );
    let bus = ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    assert_refused(
        &refused(&bus),
        &[
            "crates/ambre-bus/src/lib.rs:3: the `sheet_id` field carries an identifier as a bare `String`",
        ],
    );
}

// A row of the relay holds a closed set under an alias. It writes a core
// identifier `Id<Snapshot>` with no text adapter. It builds an answer with
// `json!` called by another name. The scan read a `DocumentId` word, a
// `String` word and a `json!` call alone (`SRV-179`, `SRV-180`, `SRV-181`).
#[test]
fn a_wire_field_under_an_alias_and_a_renamed_json_are_refused() {
    let ws = Workspace::new("wire");
    ws.file("crates/serde/src/lib.rs", "pub trait Serialize {}\n");
    ws.file(
        "crates/serde_json/src/lib.rs",
        "#[macro_export]\nmacro_rules! json { ($($t:tt)*) => { 0u8 } }\n",
    );
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "pub struct Id<T>(core::marker::PhantomData<T>);\npub struct Snapshot;\n",
    );
    // A derive that writes nothing stands in for serde's, so its helper
    // attribute compiles.
    ws.file(
        "crates/serde_derive/src/lib.rs",
        "#[proc_macro_derive(Serialize, attributes(serde))]\n\
         pub fn derive(_: proc_macro::TokenStream) -> proc_macro::TokenStream {\n\
             proc_macro::TokenStream::new()\n\
         }\n",
    );
    ws.compile("serde_derive", "serde_derive", &[], Build::Macro);
    for (name, dir) in [
        ("serde", "serde"),
        ("serde_json", "serde_json"),
        ("ambre_core", "ambre-core"),
    ] {
        ws.compile(name, dir, &[], Build::Shipped);
    }
    ws.file("crates/ambre-server/src/lib.rs", "pub mod routes;\n");
    ws.file(
        "crates/ambre-server/src/routes/mod.rs",
        "use ambre_core::{Id, Snapshot};\n\
         use serde_json::json as body;\n\
         pub type Word = String;\n\
         #[derive(serde_derive::Serialize)]\n\
         pub struct Row {\n\
             pub snapshot_id: Id<Snapshot>,\n\
             pub role: Word,\n\
             #[serde(with = \"ambre_core::serde_id::text\")]\n\
             pub owner_id: Id<Snapshot>,\n\
         }\n\
         #[automatically_derived]\n\
         impl serde::Serialize for Row {}\n\
         pub fn answer() -> u8 { body!({}) }\n",
    );
    let externs = ["serde", "serde_derive", "serde_json", "ambre_core"];
    let server = ws.compile("ambre_server", "ambre-server", &externs, Build::Shipped);
    assert_refused(
        &refused(&server),
        &[
            "crates/ambre-server/src/routes/mod.rs:13: an answer built with `json!`",
            "crates/ambre-server/src/routes/mod.rs:6: the `snapshot_id` field writes its identifier as bytes",
            "crates/ambre-server/src/routes/mod.rs:7: the `role` field carries a closed set as `String`",
        ],
    );
}

// A macro of the core crate writes a destructor into the Logic thread, so
// no line there spells `impl Drop` (`ARCH-015`).
#[test]
fn a_destructor_a_macro_writes_into_the_logic_thread_is_refused() {
    let ws = Workspace::new("destructor");
    ws.file(
        "crates/ambre-core/src/lib.rs",
        "#[macro_export]\n\
         macro_rules! guard { ($t:ident) => { impl Drop for $t { fn drop(&mut self) {} } } }\n",
    );
    ws.compile("ambre_core", "ambre-core", &[], Build::Shipped);
    ws.file(
        "crates/ambre-logic/src/lib.rs",
        "pub struct Writer;\nambre_core::guard!(Writer);\n",
    );
    let logic = ws.compile(
        "ambre_logic",
        "ambre-logic",
        &["ambre_core"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&logic),
        &["crates/ambre-logic/src/lib.rs:2: a thread that writes a document runs a destructor"],
    );
}

// A benchmark loads a fixture and names a frame rate, each through a
// re-export whose name says neither. A benchmark that loads no fixture may
// name one (`FIXTURE-024`).
#[test]
fn a_frame_rate_a_fixture_benchmark_names_under_a_re_export_is_refused() {
    let ws = Workspace::new("bench");
    ws.file(
        "crates/ambre-logic/src/lib.rs",
        "pub mod fixtures { pub fn load() {} }\n\
         pub struct FramesPerSecond(pub u32);\n\
         pub use fixtures::load as open;\n\
         pub use FramesPerSecond as Pace;\n",
    );
    ws.compile("ambre_logic", "ambre-logic", &[], Build::Shipped);
    ws.file(
        "crates/ambre-logic/benches/load.rs",
        "pub fn run() -> ambre_logic::Pace { ambre_logic::open(); ambre_logic::Pace(1) }\n",
    );
    let bench = ws.compile_root(
        "load",
        "ambre-logic",
        "benches/load.rs",
        &["ambre_logic"],
        Build::Test,
    );
    let found = refused(&bench);
    assert!(!found.is_empty(), "{found:#?}");
    assert!(
        found.iter().all(|r| r.starts_with(
            "crates/ambre-logic/benches/load.rs:1: a fixture benchmark reports a frame rate"
        )),
        "{found:#?}"
    );
    ws.file(
        "crates/ambre-logic/benches/paint.rs",
        "pub fn run() -> ambre_logic::Pace { ambre_logic::Pace(1) }\n",
    );
    let paint = ws.compile_root(
        "paint",
        "ambre-logic",
        "benches/paint.rs",
        &["ambre_logic"],
        Build::Test,
    );
    assert_eq!(refused(&paint), Vec::<String>::new());
}

/// A stand-in for the interface toolkit's signal.
const LEPTOS: &str = "pub struct RwSignal<T>(pub T);\n";

// A shell signal carries a count under an alias, so its type reads as a type
// of its own. Each payload state and each view field state is a fact, its
// alias resolved, for the judge to join (`UI-THREAD-060`, `UI-THREAD-059`).
#[test]
fn a_signal_payload_under_an_alias_is_refused_and_each_state_is_a_fact() {
    let ws = Workspace::new("signals");
    ws.file("crates/leptos/src/lib.rs", LEPTOS);
    ws.compile("leptos", "leptos", &[], Build::Shipped);
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod chrome;\npub mod ui;\n\
         pub struct SortDialog;\n\
         pub type Count = usize;\n\
         pub type Dialog = SortDialog;\n",
    );
    ws.file("crates/ambre-trame/src/chrome/mod.rs", "pub mod relay;\n");
    ws.file(
        "crates/ambre-trame/src/chrome/relay.rs",
        "use leptos::RwSignal;\n\
         pub struct TrameRelay {\n\
             pub found: RwSignal<crate::Count>,\n\
             pub sort_dialog: RwSignal<Option<crate::SortDialog>>,\n\
         }\n",
    );
    ws.file(
        "crates/ambre-trame/src/ui/mod.rs",
        "pub struct TrameView { pub dialog: crate::Dialog, pub rows: u32 }\n",
    );
    let trame = ws.compile("ambre_trame", "ambre-trame", &["leptos"], Build::Shipped);
    assert_refused(
        &refused(&trame),
        &["crates/ambre-trame/src/chrome/relay.rs:3: the `found` signal carries a bare `usize`"],
    );
    assert_eq!(
        facts(&trame, "signal-state"),
        ["sort_dialog ambre_trame::SortDialog\tcrates/ambre-trame/src/chrome/relay.rs:4"]
    );
    assert_eq!(
        facts(&trame, "view-state"),
        ["TrameView.dialog ambre_trame::SortDialog\tcrates/ambre-trame/src/ui/mod.rs:1"]
    );
}

// A view arm matches an event through a `use` rename of its enum, and stores
// a field. The relay writes a signal off the same event. Each is a fact for
// the judge to join (`UI-THREAD-003`).
#[test]
fn an_event_arm_under_a_renamed_enum_is_read_in_a_view_and_in_a_relay() {
    let ws = Workspace::new("arms");
    ws.file("crates/leptos/src/lib.rs", LEPTOS);
    ws.compile("leptos", "leptos", &[], Build::Shipped);
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod chrome;\npub mod ui;\n\
         pub enum TrameEvent { SheetList(u32), Repaint }\n",
    );
    ws.file("crates/ambre-trame/src/chrome/mod.rs", "pub mod relay;\n");
    ws.file(
        "crates/ambre-trame/src/chrome/relay.rs",
        "use crate::TrameEvent;\n\
         pub struct Sheets(pub u32);\n\
         impl Sheets { pub fn set(&self, _: u32) {} }\n\
         pub struct TrameRelay { sheets: Sheets }\n\
         impl TrameRelay {\n\
             pub fn on(&self, e: TrameEvent) {\n\
                 match e {\n\
                     TrameEvent::SheetList(n) => self.sheets.set(n),\n\
                     TrameEvent::Repaint => {}\n\
                 }\n\
             }\n\
         }\n",
    );
    ws.file(
        "crates/ambre-trame/src/ui/mod.rs",
        "use crate::TrameEvent as E;\n\
         pub struct TrameView { active: u32, needs_full: bool }\n\
         impl TrameView {\n\
             pub fn on(&mut self, e: E) {\n\
                 match e {\n\
                     E::SheetList(n) => self.active = n,\n\
                     E::Repaint => self.needs_full = true,\n\
                 }\n\
             }\n\
         }\n",
    );
    let trame = ws.compile("ambre_trame", "ambre-trame", &["leptos"], Build::Shipped);
    assert_eq!(
        facts(&trame, "relay-writes"),
        ["TrameEvent::SheetList sheets\tcrates/ambre-trame/src/chrome/relay.rs:8"]
    );
    assert_eq!(
        facts(&trame, "view-stores"),
        ["TrameEvent::SheetList active\tcrates/ambre-trame/src/ui/mod.rs:6"]
    );
}
