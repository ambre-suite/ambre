//! The chrome rules `crate_boundaries.py` read by regular expression, read
//! with types. Each test plants a breach that a scan of the line let through.
//! The breach is a `use` rename, a re-export, a macro, a literal `concat!`
//! builds or a body a closure holds. The driver refuses it, and a control
//! beside it stays free.

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

/// A stand-in for the browser bindings, with the names the rules read.
const WEB_SYS: &str = "pub struct Window;\n\
    impl Window {\n\
        pub fn inner_width(&self) -> f64 { 0.0 }\n\
        pub fn screen(&self) -> Screen { Screen }\n\
        pub fn match_media(&self, _: &str) {}\n\
    }\n\
    pub struct Screen;\n\
    pub struct Element;\n\
    impl Element { pub fn set_attribute(&self, _: &str, _: &str) {} }\n\
    pub struct Document;\n\
    impl Document { pub fn create_element(&self, _: &str) -> Element { Element } }\n";

/// A stand-in for the toolkit: the element functions `view!` writes for a
/// tag, and the attribute methods it writes for an attribute.
const TACHYS: &str = "pub mod html {\n\
        pub struct InertElement;\n\
        impl InertElement { pub fn new(_: &'static str) -> InertElement { InertElement } }\n\
        pub mod element {\n\
        pub struct El;\n\
        impl El {\n\
            pub fn r#type(self, _: &str) -> El { El }\n\
            pub fn sandbox(self, _: &str) -> El { El }\n\
            pub fn role(self, _: &str) -> El { El }\n\
            pub fn on<F>(self, _: &str, _: F) -> El { El }\n\
            pub fn aria_pressed<V>(self, _: V) -> El { El }\n\
        }\n\
        pub fn dialog() -> El { El }\n\
        pub fn iframe() -> El { El }\n\
        pub fn input() -> El { El }\n\
        pub fn ul() -> El { El }\n\
        pub fn button() -> El { El }\n\
    } }\n";

/// A stand-in for the bus: the command that carries a name, and the
/// invitation.
const BUS: &str = "pub enum RestCommand { CreateDocument { name: String }, InviteToDocument }\n";

/// A helper crate of the workspace that hides the names behind its own. Its
/// macros write a forbidden name at the call site.
const HELPERS: &str = "pub use web_sys::Window as Frame;\n\
    pub use ambre_bus::RestCommand as Rest;\n\
    #[macro_export]\n\
    macro_rules! modal { () => { tachys::html::element::dialog() } }\n\
    #[macro_export]\n\
    macro_rules! menu { () => { tachys::html::element::ul().role(concat!(\"me\", \"nu\")) } }\n\
    #[macro_export]\n\
    macro_rules! slider { () => { tachys::html::element::input().r#type(concat!(\"ra\", \"nge\")) } }\n";

/// The workspace with the stand-ins, the helper crate and the debouncer.
fn helpers(name: &str) -> Workspace {
    let ws = Workspace::new(name);
    ws.file("crates/web-sys/src/lib.rs", WEB_SYS);
    ws.file("crates/tachys/src/lib.rs", TACHYS);
    ws.file("crates/ambre-bus/src/lib.rs", BUS);
    ws.file("crates/ambre-helpers/src/lib.rs", HELPERS);
    ws.file(
        "crates/ambre-chrome/src/lib.rs",
        "pub struct Debouncer;\nimpl Debouncer { pub fn call(&self) {} }\n\
         pub fn on_menu_keydown() {}\n",
    );
    ws.compile("web_sys", "web-sys", &[], Build::Shipped);
    ws.compile("tachys", "tachys", &[], Build::Shipped);
    ws.compile("ambre_bus", "ambre-bus", &[], Build::Shipped);
    ws.compile(
        "ambre_helpers",
        "ambre-helpers",
        &["web_sys", "tachys", "ambre_bus"],
        Build::Shipped,
    );
    ws.compile("ambre_chrome", "ambre-chrome", &[], Build::Shipped);
    ws
}

/// The externs of a crate that reads the helpers.
const HELPED: &[&str] = &[
    "web_sys",
    "tachys",
    "ambre_bus",
    "ambre_helpers",
    "ambre_chrome",
];

// The launcher names the invitation and the named create through a
// re-export, so no line spells `InviteToDocument` or `CreateDocument`. The
// file of the trust decision may invite, and a test may name both
// (`CHROME-050`, `CHROME-037`).
#[test]
fn an_invitation_or_a_named_create_under_a_re_export_is_refused_on_the_ui_thread() {
    let ws = helpers("invite");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod share;\npub mod tofu;\n",
    );
    ws.file(
        "crates/ambre-app/src/share.rs",
        "use ambre_helpers::Rest as R;\n\
         pub fn invite() -> R { R::InviteToDocument }\n\
         pub fn create() -> R { R::CreateDocument { name: String::new() } }\n",
    );
    ws.file(
        "crates/ambre-app/src/tofu.rs",
        "pub fn invite() -> ambre_helpers::Rest { ambre_helpers::Rest::InviteToDocument }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/share.rs:2: an invitation built past the trust decision",
            "crates/ambre-app/src/share.rs:3: the UI thread creates a named document",
        ],
    );
    let test = ws.compile("ambre_app", "ambre-app", HELPED, Build::Test);
    assert_eq!(refused(&test), Vec::<String>::new());
}

// A content view reads the window through a re-export, so its line spells
// `Frame`, never `Window`. The chrome of the same crate may read it
// (`CHROME-034`).
#[test]
fn a_window_read_under_a_re_export_is_refused_in_a_content_view() {
    let ws = helpers("window");
    ws.file(
        "crates/ambre-trame/src/lib.rs",
        "pub mod chrome;\npub mod ui;\n",
    );
    ws.file(
        "crates/ambre-trame/src/ui/mod.rs",
        "pub fn width(w: &ambre_helpers::Frame) -> f64 { w.inner_width() }\n\
         pub fn screen(w: &ambre_helpers::Frame) { let _ = w.screen(); }\n",
    );
    ws.file(
        "crates/ambre-trame/src/chrome/mod.rs",
        "pub fn width(w: &ambre_helpers::Frame) -> f64 { w.inner_width() }\n",
    );
    let trame = ws.compile("ambre_trame", "ambre-trame", HELPED, Build::Shipped);
    assert_refused(
        &refused(&trame),
        &[
            "crates/ambre-trame/src/ui/mod.rs:1: a content view reads the window",
            "crates/ambre-trame/src/ui/mod.rs:2: a content view reads the window",
        ],
    );
}

// A media read built by `concat!` holds a width no line spells, through a
// re-export of the window. A query that names a dimension is refused, test
// code included, and so is one the driver cannot read as a literal. The
// colour scheme and the motion setting name no dimension, and the density
// module stays free (`CHROME-056`).
#[test]
fn a_media_read_that_names_a_dimension_is_refused_in_test_code_too() {
    let ws = helpers("media");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod narrow;\npub mod settings;\n",
    );
    ws.file(
        "crates/ambre-app/src/narrow.rs",
        "pub fn narrow(w: &ambre_helpers::Frame) { w.match_media(concat!(\"(max-\", \"width: 599px)\")); }\n\
         pub fn wide(w: &ambre_helpers::Frame) { w.match_media(concat!(\"screen and (min-\", \"width: 600px)\")); }\n\
         pub fn short(w: &ambre_helpers::Frame) { w.match_media(\"(max-height: 400px)\"); }\n\
         pub fn square(w: &ambre_helpers::Frame) { w.match_media(\"(max-aspect-ratio: 1/1)\"); }\n\
         pub fn ranged(w: &ambre_helpers::Frame) { w.match_media(\"(400px <= device-width)\"); }\n\
         pub fn built(w: &ambre_helpers::Frame, q: &str) { w.match_media(q); }\n",
    );
    ws.file(
        "crates/ambre-app/src/settings.rs",
        "pub fn still(w: &ambre_helpers::Frame) { w.match_media(\"(prefers-reduced-motion: reduce)\"); }\n\
         pub fn dark(w: &ambre_helpers::Frame) { w.match_media(&\"(prefers-color-scheme: dark)\"); }\n",
    );
    for build in [Build::Shipped, Build::Test] {
        let app = ws.compile("ambre_app", "ambre-app", HELPED, build);
        assert_refused(
            &refused(&app),
            &[
                "crates/ambre-app/src/narrow.rs:1: a width threshold read outside the stylesheet",
                "crates/ambre-app/src/narrow.rs:2: a width threshold read outside the stylesheet",
                "crates/ambre-app/src/narrow.rs:3: a width threshold read outside the stylesheet",
                "crates/ambre-app/src/narrow.rs:4: a width threshold read outside the stylesheet",
                "crates/ambre-app/src/narrow.rs:5: a width threshold read outside the stylesheet",
                "crates/ambre-app/src/narrow.rs:6: a width threshold read outside the stylesheet",
            ],
        );
    }
    ws.file("crates/ambre-canvas/src/lib.rs", "pub mod density;\n");
    ws.file(
        "crates/ambre-canvas/src/density.rs",
        "pub fn dense(w: &ambre_helpers::Frame) { w.match_media(\"(resolution: 2dppx)\"); }\n",
    );
    let canvas = ws.compile("ambre_canvas", "ambre-canvas", HELPED, Build::Shipped);
    assert_eq!(refused(&canvas), Vec::<String>::new());
}

// A helper macro writes the `<dialog>` element, and `concat!` builds the tag
// a `create_element` takes, so no line spells `<dialog`. The shared dialog
// may render it (`CHROME-011`).
#[test]
fn a_modal_a_macro_writes_or_concat_spells_is_refused_outside_the_shared_dialog() {
    let ws = helpers("modal");
    ws.file("crates/ambre-app/src/lib.rs", "pub mod palette;\n");
    ws.file(
        "crates/ambre-app/src/palette.rs",
        "pub fn open() { let _ = ambre_helpers::modal!(); }\n\
         pub fn build(d: &web_sys::Document) { d.create_element(concat!(\"dia\", \"log\")); }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/palette.rs:1: a modal outside the shared dialog",
            "crates/ambre-app/src/palette.rs:2: a modal outside the shared dialog",
        ],
    );
    ws.file("crates/ambre-chrome/src/lib.rs", "pub mod dialog;\n");
    ws.file(
        "crates/ambre-chrome/src/dialog.rs",
        "pub fn open() { let _ = ambre_helpers::modal!(); }\n",
    );
    let chrome = ws.compile(
        "ambre_chrome",
        "ambre-chrome",
        &["web_sys", "tachys", "ambre_bus", "ambre_helpers"],
        Build::Shipped,
    );
    assert_eq!(refused(&chrome), Vec::<String>::new());
}

// A helper macro writes a range input whose type `concat!` builds, so no line
// spells `type="range"`. A control whose own handler calls the debouncer,
// named through a `use` rename, is free (`CHROME-015`).
#[test]
fn a_drag_control_a_macro_writes_is_refused_unless_its_handler_debounces() {
    let ws = helpers("drag");
    ws.file("crates/ambre-toile/src/lib.rs", "pub mod chrome;\n");
    ws.file(
        "crates/ambre-toile/src/chrome/mod.rs",
        "pub mod coalesced;\npub mod style;\n",
    );
    ws.file(
        "crates/ambre-toile/src/chrome/style.rs",
        "pub fn opacity() { let _ = ambre_helpers::slider!(); }\n",
    );
    ws.file(
        "crates/ambre-toile/src/chrome/coalesced.rs",
        "use ambre_chrome::Debouncer as Burst;\n\
         pub fn opacity(b: &Burst) { let _ = ambre_helpers::slider!().on(\"input\", move || b.call()); }\n",
    );
    let toile = ws.compile("ambre_toile", "ambre-toile", HELPED, Build::Shipped);
    assert_refused(
        &refused(&toile),
        &["crates/ambre-toile/src/chrome/style.rs:1: a drag control that is not debounced"],
    );
}

// The scan read `create_element("iframe")` on one line. A frame `view!`
// writes, or one whose tag `concat!` builds, passed it. A frame whose own
// call or binding sets the sandbox is free (`ARCH-001`).
#[test]
fn a_frame_the_toolkit_writes_or_concat_spells_is_refused_unless_it_is_sandboxed() {
    let ws = helpers("frame");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod embed;\npub mod sandboxed;\n",
    );
    ws.file(
        "crates/ambre-app/src/embed.rs",
        "pub fn element() { let _ = tachys::html::element::iframe(); }\n\
         pub fn built(d: &web_sys::Document) { d.create_element(concat!(\"i\", \"frame\")); }\n",
    );
    ws.file(
        "crates/ambre-app/src/sandboxed.rs",
        "pub fn built(d: &web_sys::Document) {\n\
             let frame = d.create_element(\"iframe\");\n\
             frame.set_attribute(\"sandbox\", \"allow-scripts\");\n\
         }\n\
         pub fn element() { let _ = tachys::html::element::iframe().sandbox(\"\"); }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/embed.rs:1: creates a frame and never sandboxes it",
            "crates/ambre-app/src/embed.rs:2: creates a frame and never sandboxes it",
        ],
    );
}

/// The Sync worker's create body, and the open-time create whose body a
/// closure builds.
fn worker(name: &str, value: &str) -> Vec<String> {
    let ws = Workspace::new(name);
    ws.file(
        "crates/ambre-sync/src/lib.rs",
        &format!(
            "pub struct CreateDocumentRequest {{ pub name_enc: Option<String> }}\n\
             pub struct Worker {{ pub name: Option<String> }}\n\
             impl Worker {{\n\
                 pub fn trigger_lazy_create(&self) -> CreateDocumentRequest {{\n\
                     let name = self.name.clone();\n\
                     let make = move || CreateDocumentRequest {{ name_enc: {value} }};\n\
                     make()\n\
                 }}\n\
             }}\n"
        ),
    );
    ws.compile("ambre_sync", "ambre-sync", &[], Build::Shipped)
}

// The scan read the text `name_enc: None` in the function. A body that a
// closure builds from a captured name passed it when a comment beside it
// spelled the field. The driver reads the expression the field takes
// (`CHROME-037`).
#[test]
fn an_open_time_create_that_sends_a_name_is_refused_and_an_unnamed_one_is_a_fact() {
    let named = worker("lazy-named", "name.clone() /* name_enc: None */");
    assert_refused(
        &refused(&named),
        &["crates/ambre-sync/src/lib.rs:6: the open-time create does not send `name_enc: None`"],
    );
    assert_eq!(of(&named, "fact"), Vec::<String>::new());
    let unnamed = worker("lazy-unnamed", "None");
    assert_eq!(refused(&unnamed), Vec::<String>::new());
    assert_eq!(
        of(&unnamed, "fact"),
        vec!["lazy-create-unnamed\tNone\tcrates/ambre-sync/src/lib.rs:6".to_owned()]
    );
}

// A helper closure turns `false` into `None`, so the attribute vanishes, and
// the line spells no `then_some` beside `aria-pressed`. A state that writes
// both values stays free (`CHROME-066`).
#[test]
fn a_two_state_attribute_whose_false_state_drops_it_is_refused() {
    let ws = helpers("two-state");
    ws.file(
        "crates/ambre-chrome/src/lib.rs",
        "pub fn toggle(on: bool) {\n\
             let _ = tachys::html::element::button().aria_pressed(move || on.then_some(\"true\"));\n\
             let _ = tachys::html::element::button().aria_pressed(move || if on { \"true\" } else { \"false\" });\n\
         }\n",
    );
    let chrome = ws.compile(
        "ambre_chrome",
        "ambre-chrome",
        &["web_sys", "tachys", "ambre_bus", "ambre_helpers"],
        Build::Shipped,
    );
    assert_refused(
        &refused(&chrome),
        &["crates/ambre-chrome/src/lib.rs:2: a two-state attribute that drops its false state"],
    );
}

// A helper macro renders the menu with a role `concat!` builds, so no line
// spells `role="menu"`. A menu whose own handler calls a key handler of the
// shared menu, named through a `use` rename, is free (`CHROME-026`).
#[test]
fn a_menu_a_macro_renders_is_refused_unless_its_handler_reaches_a_menu_key() {
    let ws = helpers("menu");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod keyed;\npub mod mute;\n",
    );
    ws.file(
        "crates/ambre-app/src/mute.rs",
        "pub fn types() { let _ = ambre_helpers::menu!(); }\n",
    );
    ws.file(
        "crates/ambre-app/src/keyed.rs",
        "use ambre_chrome::on_menu_keydown as keys;\n\
         pub fn types() { let _ = ambre_helpers::menu!().on(\"keydown\", move || keys()); }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &["crates/ambre-app/src/mute.rs:1: a menu with no keyboard"],
    );
}

// `view!` writes a static subtree as one HTML string an inert element takes,
// so no attribute call carries the type, the tag or the role. The driver reads
// the string where that call takes it (`CHROME-011`, `CHROME-015`,
// `CHROME-026`, `ARCH-001`).
#[test]
fn a_static_subtree_the_toolkit_writes_as_html_is_read_where_the_inert_element_takes_it() {
    let ws = helpers("inert");
    ws.file(
        "crates/ambre-app/src/lib.rs",
        "pub mod still;\npub mod sandboxed;\n",
    );
    ws.file(
        "crates/ambre-app/src/still.rs",
        "pub fn page() { let _ = tachys::html::InertElement::new(concat!(\
             \"<div><input type=\\\"range\\\"><dialog></dialog>\", \
             \"<ul role=\\\"menu\\\"></ul><iframe src=\\\"x\\\"></iframe></div>\")); }\n",
    );
    ws.file(
        "crates/ambre-app/src/sandboxed.rs",
        "pub fn page() { let _ = tachys::html::InertElement::new(\
             \"<iframe sandbox=\\\"\\\" src=\\\"x\\\"></iframe>\"); }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/still.rs:1: a drag control that is not debounced",
            "crates/ambre-app/src/still.rs:1: a menu with no keyboard",
            "crates/ambre-app/src/still.rs:1: a modal outside the shared dialog",
            "crates/ambre-app/src/still.rs:1: creates a frame and never sandboxes it",
        ],
    );
}

// The script read each guard on the file: one debouncer, one key handler or
// one sandbox anywhere freed every control of the file. Here each guarded
// control sits beside a second one with no guard of its own, and the second
// is refused. A static subtree that sandboxes one frame of two is refused too
// (`CHROME-015`, `CHROME-026`, `ARCH-001`).
#[test]
fn a_second_control_in_a_file_rides_no_guard_of_the_first() {
    let ws = helpers("two-controls");
    ws.file("crates/ambre-app/src/lib.rs", "pub mod panel;\n");
    ws.file(
        "crates/ambre-app/src/panel.rs",
        "use ambre_chrome::{Debouncer, on_menu_keydown};\n\
         pub fn panel(b: &Debouncer, d: &web_sys::Document) {\n\
             let _ = ambre_helpers::slider!().on(\"input\", move || b.call());\n\
             let _ = ambre_helpers::slider!();\n\
             let _ = ambre_helpers::menu!().on(\"keydown\", on_menu_keydown);\n\
             let _ = ambre_helpers::menu!();\n\
             let frame = d.create_element(\"iframe\");\n\
             frame.set_attribute(\"sandbox\", \"\");\n\
             let _ = tachys::html::element::iframe();\n\
             let bare = d.create_element(\"iframe\");\n\
             let _ = tachys::html::InertElement::new(\"<iframe sandbox=\\\"\\\"></iframe><iframe src=\\\"x\\\"></iframe>\");\n\
             let _ = bare;\n\
         }\n",
    );
    let app = ws.compile("ambre_app", "ambre-app", HELPED, Build::Shipped);
    assert_refused(
        &refused(&app),
        &[
            "crates/ambre-app/src/panel.rs:10: creates a frame and never sandboxes it",
            "crates/ambre-app/src/panel.rs:11: creates a frame and never sandboxes it",
            "crates/ambre-app/src/panel.rs:4: a drag control that is not debounced",
            "crates/ambre-app/src/panel.rs:6: a menu with no keyboard",
            "crates/ambre-app/src/panel.rs:9: creates a frame and never sandboxes it",
        ],
    );
}
