"""Each check of `crate_boundaries.py`, against a tree that breaches it.

The green run proves nothing on its own: a check that cannot fail holds no
rule. Each case here moves one line of the real tree and asserts the breach is
named.
"""

import unittest
from pathlib import Path

import crate_boundaries as cb


class Tree:
    """The module pointed at a scratch tree, restored on exit."""

    def __init__(self, root: Path):
        self.root = root
        self.was = cb.ROOT

    def __enter__(self):
        cb.ROOT = self.root
        return self

    def __exit__(self, *_):
        cb.ROOT = self.was


def write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def read_back(root: Path, rel: str) -> str:
    path = root / rel
    return path.read_text() if path.exists() else ""


# What each relay writes off an event: one arm per line shape the gate reads.
TRAME_ABSORB = (
    "fn absorb(&self, te: &TrameEvent) {\n"
    "    match te {\n"
    "        TrameEvent::SheetList { sheets, active } => {\n"
    "            self.sheets.set(sheets.clone());\n"
    "            self.active_sheet.set(active.clone());\n"
    "        }\n"
    "        TrameEvent::FilterApplied { .. } | TrameEvent::FilterCleared => self.filter_popover.set(None),\n"
    "        _ => {}\n"
    "    }\n"
    "}\n"
)
TOILE_ABSORB = (
    "fn absorb(&self, te: &ToileEvent) {\n"
    "    match te {\n"
    "        ToileEvent::FindResults(fr) => self\n"
    "            .find_result\n"
    "            .set(Some(fr.clone())),\n"
    "        _ => {}\n"
    "    }\n"
    "}\n"
)

# One walk per application, each reaching only what `WALK_REACH` lists.
WALKS = {
    "crates/ambre-prose/src/view/walk.rs": (
        "use crate::view::cache::{RunCache, RunKey};\nuse crate::view::{Highlight, HighlightKind};\n"
        "impl DocumentWalk for PageWalk<'_> {}\n"
    ),
    "crates/ambre-trame/src/render/walk.rs": (
        "use super::*;\nuse crate::{TOTAL_COLS, TOTAL_ROWS};\nimpl<'t> DocumentWalk for SheetWalk<'t> {}\n"
    ),
    "crates/ambre-toile/src/ui/walk.rs": (
        "use crate::logic::animate::AnimatedElement;\nuse super::label::{LABEL_FONT_SIZE, label_rect};\n"
        "impl DocumentWalk for BoardWalk<'_> {}\n"
    ),
}


# A route DTO whose identifiers reach JSON as their text, one adapter spread
# over lines as rustfmt writes it, and a row the database reads.
WIRE_ROUTE = (
    "#[derive(Deserialize)]\n"
    "pub(crate) struct RevokeRequest {\n"
    '    #[serde(with = "ambre_core::serde_id::user_id")]\n'
    "    pub user_id: UserId,\n"
    "    #[serde(\n"
    "        default,\n"
    '        with = "ambre_core::serde_id::user_id::vec"\n'
    "    )]\n"
    "    pub members: Vec<UserId>,\n"
    "    pub link: String,\n"
    "}\n"
)
WIRE_ROW = (
    "#[derive(sqlx::FromRow)]\n"
    "pub struct UserRow {\n"
    '    #[sqlx(try_from = "PgId")]\n'
    "    pub id: UserId,\n"
    "    pub oidc_subject: Option<String>,\n"
    "}\n"
)


def scratch(tmp: str) -> Path:
    """A minimal tree that passes every check."""
    root = Path(tmp)
    write(root, cb.CLIENT, '[dependencies]\nambre-logic = { path = "../ambre-logic" }\n')
    write(root, cb.CHROME, '[dependencies]\nambre-commands = { path = "../ambre-commands" }\n')
    for rel in cb.FIXTURE_MODULES:
        write(root, rel, "impl S {\n    fn read(&self) { self.doc.get_map(X); }\n}\n\nfn build_a(s: &mut S) {\n    s.handle_command(C);\n}\n")
    write(
        root,
        f"{cb.LAUNCHER}/lib.rs",
        "// Fed by `TrameEvent::SheetList` (`ARCH-018`).\n"
        "fn relay(ev: &UiEvent) { trame_relay.absorb(ev); }\n"
        "fn keys() { register(trame_chrome::COMMANDS); }\n"
        "fn boot() { spawn(move || ambre_logic::run(bell)); }\n"
        "fn pre() { start.drain(&logic_to_sync_rx, post); }\n"
        "fn hook() { std::panic::set_hook(Box::new(|i| { ambre_canvas::signal_fatal(&m); })); }\n"
        "fn queues() {\n    let (ui_tx, ui_rx) =\n        conduit::to_worker::<LogicCommand>(CAP, &bell);\n"
        "    let shell = outbox::ShellOutbox::new(ui_tx);\n}\n"
        "fn turn() {\n    relay_tx.flush();\n    if relay_tx.take_refusal() { toasts.error(M); }\n}\n",
    )
    write(
        root,
        cb.SHELL_OUTBOX,
        "pub(crate) struct ShellOutbox { to_logic: Sender<LogicCommand> }\n"
        "fn port() { Port::embedded(self.to_logic.clone()) }\n",
    )
    write(root, cb.FRAME_HOST, "fn go() { drive_frame(&raf_c, &mut pre_frame, render); }\n")
    write(root, cb.DENSITY, "fn read() -> f64 { window.device_pixel_ratio() }\n")
    write(root, f"{cb.RENDERER}/mod.rs", "fn quad(c: Color) -> [f32; 4] { c.components }\n")
    write(
        root,
        "crates/ambre-sync/Cargo.toml",
        '[lib]\ncrate-type = ["cdylib", "rlib"]\n\n[dependencies]\ngloo-net = { workspace = true }\n'
        'ambre-bus = { path = "../ambre-bus" }\n\n[lints]\nworkspace = true\n',
    )
    write(
        root,
        cb.WORKSPACE,
        '[workspace]\nmembers = [\n    "crates/ambre-sync",\n]\n\n'
        '[workspace.lints.clippy]\nunwrap_used = "deny"\n',
    )
    write(root, f"{cb.WORKER}/http.rs", "fn put() { gloo_net::http::Request::put(url); }\n")
    write(root, cb.NETWORK_EXCEPTIONS[0], "fn font() { window.fetch_with_str(url); }\n")
    write(
        root,
        cb.LAZY_CREATE,
        "    pub(crate) fn trigger_lazy_create(&mut self) {\n        let b = Req { name_enc: None, };\n    }\n",
    )
    write(
        root,
        cb.STYLESHEET,
        ".label {\n    display: none;\n}\n\n@media (min-width: 600px) {\n    .label {\n"
        "        display: inline;\n    }\n}\n",
    )
    write(root, cb.PIPE, 'fn create() {\n    for (k, v) in [("tabindex", "0")] {}\n}\n')
    write(root, f"{cb.RELAY}/ws.rs", 'fn put() {\n    tracing::info!(seq, len = payload.len(), "update {doc_id}");\n}\n')
    write(root, cb.KINETIC_MODEL, "pub const DECAY_MS: f64 = 325.0;\n")
    write(root, cb.AXIS_MODULE, "fn list() -> SheetRoot { SheetRoot::Rows }\n")
    write(root, cb.DRAW_TRAIT, "pub trait DrawSurface {\n    fn fill_rect(&mut self, rect: Rect, color: Color);\n}\n")
    write(root, cb.COMPOSED_NAME, 'const UNTITLED: &str = "Untitled";\n')
    for rel, setter in zip(
        cb.SCROLL_SETTERS,
        (
            "    pub fn scroll_by(&mut self, step: ScrollStep) {\n        let moved = self.scroll_y + step.y();\n    }\n",
            "    pub fn pan_by_screen(&mut self, step: ScrollStep) {\n        self.center += step.x();\n    }\n",
            "    fn accumulate(&mut self, step: ScrollStep) {\n        self.accum_y += step.y();\n    }\n"
            "    fn apply(&mut self, grid: &mut G) {\n        grid.scroll_by(1, 0);\n    }\n",
            "    pub fn scroll_by(&mut self, step: ScrollStep) {\n        let moved = self.scroll_y - step.y();\n    }\n",
        ),
        strict=True,
    ):
        write(root, rel, f"impl V {{\n{setter}}}\n")
    for rel, holder in cb.SIGNAL_HOLDERS.items():
        path = root / rel
        held = path.read_text() if path.exists() else ""
        body = f"struct {holder} {{\n    pub sel_stats: RwSignal<Option<chrome::SelStats>>,\n    pub zoom: RwSignal<Scale<Doc, View>>,\n}}\n"
        write(root, rel, held + body)
    write(root, cb.EVENT_RELAYS["Trame"], read_back(root, cb.EVENT_RELAYS["Trame"]) + TRAME_ABSORB)
    write(root, cb.EVENT_RELAYS["Toile"], read_back(root, cb.EVENT_RELAYS["Toile"]) + TOILE_ABSORB)
    write(
        root,
        cb.PAGE,
        '<link data-trunk rel="rust" data-wasm-opt="0" />\n'
        '<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker"\n      data-loader-shim="true" />\n',
    )
    write(
        root,
        cb.WORKER_SPAWNER,
        "fn spawn() {\n    web_sys::Worker::new(URL);\n"
        "    let e = Closure::new(|_| { ambre_canvas::signal_fatal(r); });\n"
        "    worker.set_onerror(Some(e));\n}\n",
    )
    write(root, cb.LOGIC_MANIFEST, '[dependencies]\nambre-bus = { path = "../ambre-bus" }\n\n[dev-dependencies]\nwgpu = "1"\n')
    write(root, "crates/ambre-bus/Cargo.toml", '[dependencies]\nambre-core = { path = "../ambre-core" }\n')
    write(root, cb.CANVAS_MANIFEST, '[dependencies]\nambre-draw = { path = "../ambre-draw" }\n')
    write(root, f"{cb.PROTOCOL}/lib.rs", "pub struct Update {\n    pub seq: u64,\n}\n")
    write(root, "crates/ambre-trame/src/logic/mod.rs", "fn h(&mut self) -> Outcome { Outcome::default() }\n")
    write(root, cb.CARGO_CONFIG, '[unstable]\nbuild-std = ["std", "panic_abort"]\n')
    write(root, f"{cb.BUS}/lib.rs", "pub struct SheetId(pub String);\npub enum C {\n    Go { sheet_id: SheetId },\n}\n")
    for messages in cb.APP_MESSAGES:
        write(root, f"{messages}/mod.rs", "pub enum E {\n    Gone { sheet_id: SheetId },\n}\n")
    write(root, f"{cb.WIRE_DIRS[0]}/share.rs", WIRE_ROUTE)
    write(root, f"{cb.WIRE_DIRS[1]}/users.rs", WIRE_ROW)
    for rel, text in WALKS.items():
        write(root, rel, text)
    return root


class CrateBoundaries(unittest.TestCase):
    def run_checks(self, root: Path) -> list[str]:
        with Tree(root):
            errors: list[str] = []
            cb.check_client(errors)
            cb.check_generators(errors)
            cb.check_generator_reads(errors)
            cb.check_chrome(errors)
            cb.check_launcher(errors)
            cb.check_density(errors)
            cb.check_network(errors)
            cb.check_storage(errors)
            cb.check_passphrase(errors)
            cb.check_lints(errors)
            cb.check_pdf(errors)
            cb.check_fonts(errors)
            cb.check_renderer_colors(errors)
            cb.check_ui_thread(errors)
            cb.check_client_storage(errors)
            cb.check_gpu_subset(errors)
            cb.check_widths(errors)
            cb.check_modals_and_drags(errors)
            cb.check_key_verification(errors)
            cb.check_accessibility(errors)
            cb.check_payload_logs(errors)
            cb.check_kinetic_copy(errors)
            cb.check_axis_lists(errors)
            cb.check_primitives(errors)
            cb.check_scroll_sources(errors)
            cb.check_composed_names(errors)
            cb.check_one_copy(errors)
            cb.check_one_event_copy(errors)
            cb.check_signal_types(errors)
            cb.check_bus_ids(errors)
            cb.check_message_crdt(errors)
            cb.check_bus_vocabulary(errors)
            cb.check_wire_ids(errors)
            cb.check_typed_answers(errors)
            cb.check_bench_frame_rate(errors)
            cb.check_instances(errors)
            cb.check_thread_reach(errors)
            cb.check_renderer_reach(errors)
            cb.check_worker_messages(errors)
            cb.check_engine_and_relay(errors)
            cb.check_handlers(errors)
            cb.check_launcher_outbox(errors)
            cb.check_failure_latches(errors)
            cb.check_exit_writes(errors)
            cb.check_walks(errors)
            return errors

    def breach(self, rel: str, text: str) -> list[str]:
        """The errors of the passing tree with one file written into it."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, rel, text)
            return self.run_checks(root)

    def test_a_family_state_is_told_apart_by_its_marker(self):
        web = cb.state_type("Validated<WebAddress>")
        shown = cb.state_type("RwSignal<Option<ambre_core::Validated<ambre_core::DisplayName>>>")
        self.assertEqual(web, "Validated<WebAddress>")
        self.assertNotEqual(web, cb.state_type("Option<Validated<DisplayName>>"))
        self.assertEqual(cb.state_type("Option<ambre_core::Validated<ambre_core::DisplayName>>"), "Validated<DisplayName>")
        self.assertIsNone(cb.state_type("Id<User>"))
        self.assertEqual(shown, "RwSignal<Option<Validated<DisplayName>>>")

    def test_a_view_holding_the_document_is_refused(self):
        for line in (
            "fn draw(d: &loro::LoroDoc) {}\n",
            "fn draw(s: &ProseState) {}\n",
            "fn load(b: &[u8]) { let m = ExportMode::Snapshot; }\n",
        ):
            for rel in ("crates/ambre-toile/src/ui/view.rs", "crates/ambre-trame/src/ui/grid.rs"):
                errors = self.breach(rel, line)
                self.assertTrue(any("holds the document" in e for e in errors), (rel, line, errors))

    def test_a_view_converting_a_file_is_refused(self):
        line = "fn read(b: &[u8]) { let r = csv::ReaderBuilder::new(); }\n"
        errors = self.breach("crates/ambre-trame/src/ui/menu.rs", line)
        self.assertTrue(any("converts a file" in e for e in errors), errors)

    def test_a_tree_address_and_a_test_engine_stay_free(self):
        text = "fn hit(id: loro::TreeID) {}\n\n#[cfg(test)]\nmod tests {\n    fn s() -> ProseState {}\n}\n"
        self.assertEqual(self.breach("crates/ambre-prose/src/view/hit.rs", text), [])

    def test_a_module_declared_for_the_tests_stays_free(self):
        import tempfile

        engine = "fn s() -> TrameState { TrameState::new() }\n"
        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            view = '#[cfg(all(test, not(target_arch = "wasm32")))]\nmod view_tests;\nmod input;\n'
            write(root, "crates/ambre-trame/src/ui/view/mod.rs", view)
            write(root, "crates/ambre-trame/src/ui/view/view_tests.rs", engine)
            write(root, "crates/ambre-trame/src/ui/view/input.rs", engine)
            write(root, "crates/ambre-trame/src/ui/grid/mod.rs", "#[cfg(test)]\nmod tests;\n")
            write(root, "crates/ambre-trame/src/ui/grid/tests/mod.rs", "mod viewport;\n")
            write(root, "crates/ambre-trame/src/ui/grid/tests/viewport.rs", engine)
            errors = [e for e in self.run_checks(root) if "holds the document" in e]
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("ui/view/input.rs", errors[0])

    def test_a_view_that_waits_is_refused(self):
        for line in ("fn next(w: &Waiter) { w.wait(); }\n", "fn run() { futures::executor::block_on(f); }\n"):
            errors = self.breach("crates/ambre-trame/src/ui/view.rs", line)
            self.assertTrue(any("the UI thread waits" in e for e in errors), (line, errors))

    def test_a_component_that_draws_is_refused_and_the_surface_is_not(self):
        line = "fn chart(c: &HtmlCanvasElement) { c.get_context(\"2d\"); }\n"
        errors = self.breach("crates/ambre-chrome/src/chart.rs", line)
        self.assertTrue(any("a page element draws" in e for e in errors), errors)
        self.assertEqual(self.breach("crates/ambre-canvas/src/host.rs", line), [])

    def test_a_canvas_point_read_off_the_host_is_refused_and_the_host_is_not(self):
        line = "fn drop(e: &DragEvent) { let x = e.offset_x(); }\n"
        errors = self.breach("crates/ambre-toile/src/chrome/handler.rs", line)
        self.assertTrue(any("a canvas point read off the host" in e for e in errors), errors)
        for owner in ("crates/ambre-app/src/host/pointer.rs", cb.POINTER_OWNERS[1]):
            errors = self.breach(owner, line)
            self.assertFalse(any("a canvas point read off the host" in e for e in errors), (owner, errors))

    def test_an_interface_component_that_hit_tests_is_refused_and_a_view_is_not(self):
        line = "fn menu(v: &ToileView) { v.hit_test(x, y); }\n"
        for rel in ("crates/ambre-toile/src/chrome/menu.rs", "crates/ambre-app/src/shell/app_root.rs"):
            errors = self.breach(rel, line)
            self.assertTrue(any("an interface component hit-tests content" in e for e in errors), (rel, errors))
        for rel in ("crates/ambre-toile/src/ui/view.rs", "crates/ambre-app/src/host/pointer.rs"):
            self.assertEqual(self.breach(rel, line), [], rel)

    def test_a_page_point_beside_the_canvas_in_an_interface_file_is_refused(self):
        text = 'fn at(e: &MouseEvent) {\n    let c = doc.get_element_by_id("ambre-canvas");\n    e.client_x();\n}\n'
        errors = self.breach("crates/ambre-trame/src/chrome/handler.rs", text)
        self.assertTrue(any("turns a page point into a canvas point" in e for e in errors), errors)
        alone = "fn at(e: &MouseEvent) { e.client_x(); }\n"
        self.assertEqual(self.breach("crates/ambre-trame/src/chrome/sheet_tabs.rs", alone), [])
        self.assertEqual(self.breach("crates/ambre-trame/src/ui/view.rs", text), [])

    def test_a_storage_path_the_rule_does_not_name_is_refused(self):
        line = "fn keep() { window.local_storage(); }\n"
        errors = self.breach("crates/ambre-chrome/src/theme.rs", line)
        self.assertTrue(any("a persistence path the rule does not name" in e for e in errors), errors)
        owner = cb.STORAGE_OWNERS[2]
        self.assertFalse(any(e.startswith(owner) and "persistence" in e for e in self.breach(owner, line)))

    def test_a_compute_shader_is_refused(self):
        errors = self.breach("crates/ambre-canvas/src/render/blur.wgsl", "@compute @workgroup_size(8)\nfn main() {}\n")
        self.assertTrue(any("outside the shared graphics subset" in e for e in errors), errors)

    def test_a_fourth_width_is_refused(self):
        errors = self.breach(cb.STYLESHEET, "@media (max-width: 600px) {\n    .p {\n        top: 0;\n    }\n}\n")
        self.assertTrue(any("a width the chrome does not declare" in e for e in errors), errors)

    def test_a_width_query_that_hides_is_refused(self):
        text = "@media (max-width: 599px) {\n    .bar {\n        display: none;\n    }\n}\n"
        errors = self.breach(cb.STYLESHEET, text)
        self.assertTrue(any("a width query hides" in e for e in errors), errors)

    def test_a_width_spelled_outside_the_stylesheet_is_refused(self):
        errors = self.breach("crates/ambre-chrome/src/sidebar.rs", "fn narrow() { w.match_media(\"(max-width: 599px)\"); }\n")
        self.assertTrue(any("a width spelled outside the stylesheet" in e for e in errors), errors)

    def test_a_content_view_reading_the_window_is_refused(self):
        errors = self.breach("crates/ambre-trame/src/ui/view.rs", "fn fit() { let w = window.inner_width(); }\n")
        self.assertTrue(any("a content view reads the window" in e for e in errors), errors)

    def test_a_modal_outside_the_shared_dialog_is_refused(self):
        errors = self.breach("crates/ambre-app/src/about.rs", 'fn v() { view! { <dialog open=true /> } }\n')
        self.assertTrue(any("a modal outside the shared dialog" in e for e in errors), errors)
        errors = self.breach(cb.MODAL_OWNER, 'fn v() { view! { <dialog open=true /> } }\n')
        self.assertFalse(any("a modal outside" in e for e in errors), errors)

    def test_a_drag_control_without_a_debouncer_is_refused(self):
        line = 'fn v() { view! { <input type="range" on:input=commit /> } }\n'
        errors = self.breach("crates/ambre-toile/src/chrome/style.rs", line)
        self.assertTrue(any("a drag control that is not debounced" in e for e in errors), errors)
        held = "use ambre_chrome::Debouncer;\n" + line
        errors = self.breach("crates/ambre-toile/src/chrome/style.rs", held)
        self.assertFalse(any("not debounced" in e for e in errors), errors)

    def test_a_page_creating_a_document_is_refused(self):
        line = "fn new() { send(RestCommand::CreateDocument { doc_type, name }); }\n"
        errors = self.breach("crates/ambre-app/src/landing.rs", line)
        self.assertTrue(any("the UI thread creates a document" in e for e in errors), errors)

    def test_an_open_time_create_carrying_a_name_is_refused(self):
        text = "    pub(crate) fn trigger_lazy_create(&mut self) {\n        let b = Req { name_enc: Some(n), };\n    }\n"
        errors = self.breach(cb.LAZY_CREATE, text)
        self.assertTrue(any("does not send `name_enc: None`" in e for e in errors), errors)

    def test_a_chrome_crate_declaring_the_crypto_crate_is_refused(self):
        text = '[dependencies]\nambre-crypto = { path = "../ambre-crypto" }\n'
        errors = self.breach("crates/ambre-chrome/Cargo.toml", text)
        self.assertTrue(any("crypto crate outside the worker" in e for e in errors), errors)

    def test_the_logic_crate_may_declare_the_crypto_crate_for_its_fixtures_only(self):
        optional = '[dependencies]\nambre-crypto = { path = "../ambre-crypto", optional = true }\n'
        self.assertEqual(self.breach(cb.CRYPTO_FIXTURES, optional), [])
        always = '[dependencies]\nambre-crypto = { path = "../ambre-crypto" }\n'
        errors = self.breach(cb.CRYPTO_FIXTURES, always)
        self.assertTrue(any("crypto crate outside the worker" in e for e in errors), errors)

    def test_an_invitation_built_outside_the_trust_decision_is_refused(self):
        text = "fn f() { let c = RestCommand::InviteToDocument { role }; }\n"
        errors = self.breach("crates/ambre-app/src/shell/app_root.rs", text)
        self.assertTrue(any("past the trust decision" in e for e in errors), errors)
        self.assertEqual(self.breach(cb.INVITE_OWNER, text), [])

    def test_a_two_state_attribute_that_drops_false_is_refused(self):
        bad = 'view! { <button aria-pressed=move || on.get().then_some("true") /> }\n'
        errors = self.breach("crates/ambre-chrome/src/format.rs", bad)
        self.assertTrue(any("drops its false state" in e for e in errors), errors)
        good = "view! { <button aria-pressed=move || Pressed::from_bool(on.get()).aria() /> }\n"
        self.assertEqual(self.breach("crates/ambre-chrome/src/format.rs", good), [])

    def test_a_panel_claiming_a_modal_role_is_refused(self):
        text = 'view! { <div class="sort" role="dialog" aria-label="Sort" /> }\n'
        errors = self.breach("crates/ambre-trame/src/chrome/sort_dialog.rs", text)
        self.assertTrue(any("claiming a modal role" in e for e in errors), errors)
        self.assertEqual(self.breach(cb.MODAL_OWNER, 'view! { <section role="dialog" /> }\n'), [])

    def test_a_menu_without_keys_is_refused(self):
        bare = 'view! { <ul role="menu"><li role="menuitem" /></ul> }\n'
        errors = self.breach("crates/ambre-app/src/landing.rs", bare)
        self.assertTrue(any("a menu with no keyboard" in e for e in errors), errors)
        driven = bare + "fn key(ev: &E, m: &M) { on_menu_keydown(ev, m, || ()); }\n"
        self.assertEqual(self.breach("crates/ambre-app/src/landing.rs", driven), [])

    def test_a_hidden_or_unordered_pipe_is_refused(self):
        for text, said in (
            ('fn create() {\n    for (k, v) in [("tabindex", "-1")] {}\n}\n', "leave the tab order"),
            ('fn create() {\n    t.set_attribute("aria-hidden", "true");\n    ("tabindex", "0");\n}\n', "hidden"),
        ):
            errors = self.breach(cb.PIPE, text)
            self.assertTrue(any(said in e for e in errors), (text, errors))
        commented = '// NOT `aria-hidden`: the pipe is the entry point.\nfn create() { ("tabindex", "0"); }\n'
        self.assertEqual(self.breach(cb.PIPE, commented), [])

    def test_a_renderer_clearing_to_its_own_colour_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, f"{cb.RENDERER}/renderer.rs", "fn pass() { Clear(wgpu::Color::WHITE); }\n")
            errors = self.run_checks(root)
        self.assertTrue(any("a colour of its own: `Color::WHITE`" in e for e in errors), errors)

    def test_a_colour_in_the_renderer_tests_stays_free(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                f"{cb.RENDERER}/surface.rs",
                "fn draw(c: Color) {}\n\n#[cfg(test)]\nmod tests {\n    const RED: Color = Color::from_rgb8(200, 0, 0);\n}\n",
            )
            errors = self.run_checks(root)
        self.assertEqual(errors, [])

    def test_a_pdf_call_outside_the_logic_thread_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-toile/src/ui/export.rs", "fn save() { let s = KrillaSurface::new(); }\n")
            errors = self.run_checks(root)
        self.assertTrue(any("outside the Logic thread: `KrillaSurface`" in e for e in errors), errors)

    def test_a_pdf_call_under_the_logic_directory_stays_free(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-toile/src/logic/pdf.rs", "fn save() { krilla::Document::new(); }\n")
            errors = self.run_checks(root)
        self.assertEqual(errors, [])

    def test_an_embedded_font_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-canvas/src/fonts.rs", 'static F: &[u8] = include_bytes!("NotoSans.ttf");\n')
            errors = self.run_checks(root)
        self.assertTrue(any("a font embedded in the binary" in e for e in errors), errors)

    def test_an_identifier_held_as_text_on_the_wire_is_refused(self):
        for rel, untyped in (
            (f"{cb.WIRE_DIRS[0]}/share.rs", "pub user_id: String,"),
            (f"{cb.WIRE_DIRS[1]}/users.rs", "pub id: uuid::Uuid,"),
            (cb.WIRE_FILES[0], "pub document_ids: Option<Vec<String>>,"),
        ):
            with self.subTest(rel=rel):
                body = f"#[derive(sqlx::FromRow)]\npub struct Row {{\n    {untyped}\n}}\n"
                errors = self.breach(rel, body)
                self.assertTrue(any("carries an identifier as" in e for e in errors), errors)

    def test_an_identifier_written_as_bytes_on_the_wire_is_refused(self):
        bare = WIRE_ROUTE.replace('    #[serde(with = "ambre_core::serde_id::user_id")]\n', "")
        errors = self.breach(f"{cb.WIRE_DIRS[0]}/share.rs", bare)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("the `user_id` field writes its identifier as bytes", errors[0])

    def test_a_closed_set_held_as_text_on_the_wire_is_refused(self):
        for rel, untyped in (
            (f"{cb.WIRE_DIRS[1]}/documents.rs", "pub doc_type: String,"),
            (cb.WIRE_FILES[0], "pub role: String,"),
            (cb.WIRE_FILES[0], "pub status: Option<String>,"),
            (f"{cb.WIRE_DIRS[0]}/channels.rs", "pub kind: &'static str,"),
            (cb.WIRE_FILES[1], "pub error: &'static str,"),
            (cb.WIRE_FILES[0], "error: String,"),
        ):
            with self.subTest(rel=rel, field=untyped):
                body = f"#[derive(Deserialize)]\npub struct Row {{\n    {untyped}\n}}\n"
                errors = self.breach(rel, body)
                self.assertTrue(any("carries a closed set as" in e for e in errors), errors)

    def test_a_closed_set_held_as_its_enum_stays_free(self):
        body = (
            "#[derive(Deserialize)]\npub struct Row {\n"
            "    pub doc_type: ambre_core::DocType,\n"
            "    pub status: Option<ambre_core::SnapshotStatus>,\n"
            "    pub display_name: String,\n}\n"
        )
        self.assertEqual(self.breach(cb.WIRE_FILES[0], body), [])

    def test_an_answer_built_with_json_is_refused(self):
        """Holds `SRV-181`: a route, the refusal and a middleware each answer
        with a type."""
        body = 'fn answer() -> Json<Value> {\n    Json(serde_json::json!({ "status": "revoked" }))\n}\n'
        for rel in (
            f"{cb.ANSWER_SOURCES[0]}/share.rs",
            cb.ANSWER_SOURCES[1],
            f"{cb.ANSWER_SOURCES[2]}/rate_limit.rs",
        ):
            with self.subTest(rel=rel):
                errors = self.breach(rel, body)
                self.assertEqual(errors, [f"{rel}:2: an answer built with `json!`; give it a type"])

    def test_json_in_the_tests_of_a_route_stays_free(self):
        body = (
            "fn answer() -> Json<Done> { Done::json(WriteStatus::Revoked) }\n\n"
            "#[cfg(test)]\nmod tests {\n    fn body() { let _ = serde_json::json!({}); }\n}\n"
        )
        self.assertEqual(self.breach(f"{cb.ANSWER_SOURCES[0]}/share.rs", body), [])

    def test_a_row_the_database_reads_needs_no_text_adapter(self):
        errors = self.breach(f"{cb.WIRE_DIRS[1]}/users.rs", WIRE_ROW.replace("UserId", "OrgId"))
        self.assertEqual(errors, [])

    def test_the_real_tree_is_clean(self):
        self.assertEqual(self.run_checks(cb.ROOT), [])

    def test_the_scratch_tree_is_clean(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.run_checks(scratch(tmp)), [])

    def test_a_member_that_does_not_inherit_the_lints_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-sync/Cargo.toml", '[dependencies]\ngloo-net = { workspace = true }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("does not inherit the workspace lints" in e for e in errors), errors)

    def test_a_root_without_a_lint_table_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.WORKSPACE, '[workspace]\nmembers = [\n    "crates/ambre-sync",\n]\n')
            errors = self.run_checks(root)
        self.assertTrue(any("no `[workspace.lints]` table" in e for e in errors), errors)

    def test_a_client_enabling_the_fixture_feature_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.CLIENT, '[dependencies]\nambre-logic = { path = "..", features = ["fixtures"] }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("enables `fixtures`" in e for e in errors), errors)

    def test_a_default_feature_set_folding_in_fixtures_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-logic/Cargo.toml", '[features]\ndefault = ["fixtures"]\n')
            errors = self.run_checks(root)
        self.assertTrue(any("`default` feature set" in e for e in errors), errors)

    def test_a_generator_writing_to_the_crdt_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                cb.FIXTURE_MODULES[0],
                "fn build_a(s: &mut S) {\n    s.doc.get_map(CELLS).insert(k, v);\n}\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("reaches the CRDT" in e for e in errors), errors)

    def test_the_reading_half_of_a_fixture_module_stays_free(self):
        """`fixture_shape` counts the CRDT it was handed. Only free functions are read."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            errors = self.run_checks(root)
        self.assertEqual([e for e in errors if "reaches the CRDT" in e], [])

    def test_a_generator_reading_a_file_is_refused(self):
        for line in (
            'let b = include_bytes!("board.ambre");',
            'let t = std::fs::read_to_string(p);',
            "let f = File::open(p);",
            'let s = std::env::var("SEED");',
        ):
            errors = self.breach(cb.FIXTURE_MODULES[0], f"fn build_a(s: &mut S) {{\n    {line}\n}}\n")
            self.assertTrue(any("reads besides its seed" in e for e in errors), (line, errors))

    def test_a_generator_module_no_list_names_is_read(self):
        """A new application's generator is found by its file name, not by a list."""
        errors = self.breach("crates/ambre-murmure/src/fixtures.rs", 'const B: &[u8] = include_bytes!("x");\n')
        self.assertTrue(any("ambre-murmure" in e and "reads besides its seed" in e for e in errors), errors)

    def test_the_driver_and_a_test_module_may_read_files(self):
        errors = self.breach(cb.FIXTURE_DRIVER, "fn load(p: &Path) { std::fs::read(p); }\n")
        self.assertEqual([e for e in errors if "reads besides its seed" in e], [])
        errors = self.breach(
            cb.FIXTURE_MODULES[0],
            "fn build_a(s: &mut S) {\n    s.handle_command(C);\n}\n\n#[cfg(test)]\nmod tests {\n"
            "    fn t() { std::fs::read(p); }\n}\n",
        )
        self.assertEqual([e for e in errors if "reads besides its seed" in e], [])

    def test_a_tree_with_no_generator_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            for rel in cb.FIXTURE_MODULES:
                (root / rel).unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("no generator module found" in e for e in errors), errors)

    def test_a_chrome_that_depends_on_an_app_crate_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.CHROME, '[dependencies]\nambre-bus = { path = "../ambre-bus" }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("depends on `ambre-bus`" in e for e in errors), errors)

    def test_a_launcher_building_an_application_command_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                f"{cb.LAUNCHER}/shell/app_root.rs",
                "fn bold() { send(AppCommand::Trame(TrameCommand::SetCellStyle { patch })); }\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("`TrameCommand::SetCellStyle`" in e for e in errors), errors)

    def test_a_launcher_matching_an_application_event_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                f"{cb.LAUNCHER}/lib.rs",
                "fn relay() { if let AppEvent::Toile(ToileEvent::FindResults(fr)) = ev {} }\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("`ToileEvent::FindResults`" in e for e in errors), errors)

    def test_a_launcher_deciding_what_a_command_id_does_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                f"{cb.LAUNCHER}/platform.rs",
                "fn run(id: CommandId) { if id == ambre_prose::commands::SAVE_FILE {} }\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("`ambre_prose::commands::SAVE_FILE`" in e for e in errors), errors)

    def test_a_view_reading_the_density_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                "crates/ambre-trame/src/ui/view/lifecycle.rs",
                "fn new() { let dpr = window.device_pixel_ratio(); }\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("lifecycle.rs:1: reads the display density" in e for e in errors), errors)

    def test_a_density_read_named_in_a_comment_is_not_a_breach(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-toile/src/ui/view.rs", "// No devicePixelRatio here (`CANVAS-043`).\n")
            errors = self.run_checks(root)
        self.assertEqual(errors, [])

    def test_a_density_file_that_reads_nothing_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, cb.DENSITY, "fn read() -> f64 { 1.0 }\n")
            errors = self.run_checks(root)
        self.assertTrue(any("reads no density" in e for e in errors), errors)

    def test_a_network_call_outside_the_worker_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(
                root,
                "crates/ambre-logic/src/dispatch.rs",
                "fn leak() { let ws = web_sys::WebSocket::new(url); }\n",
            )
            errors = self.run_checks(root)
        self.assertTrue(any("dispatch.rs:1: a network call outside" in e for e in errors), errors)

    def test_a_transport_crate_outside_the_worker_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, "crates/ambre-trame/Cargo.toml", '[dependencies]\nreqwest = { workspace = true }\n')
            errors = self.run_checks(root)
        self.assertTrue(any("declares the transport crate `reqwest`" in e for e in errors), errors)

    def test_the_worker_and_the_stated_reads_stay_free(self):
        """The worker's transport and the font fetch are the rule, not a breach."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            errors = self.run_checks(root)
        self.assertEqual(errors, [])

    def test_a_worker_writing_to_browser_storage_is_refused(self):
        import tempfile

        for line in (
            "fn keep(t: &str) { window.local_storage().set_item(\"token\", t); }\n",
            "fn keep(t: &str) { document.set_cookie(t); }\n",
            "fn keep(b: &[u8]) { scope.indexed_db(); }\n",
        ):
            with tempfile.TemporaryDirectory() as tmp:
                root = scratch(tmp)
                write(root, f"{cb.WORKER}/auth.rs", line)
                errors = self.run_checks(root)
            self.assertTrue(any("reaches browser storage" in e for e in errors), (line, errors))

    def test_a_request_carrying_a_passphrase_is_refused(self):
        import tempfile

        for text in (
            "#[derive(Serialize)]\nstruct Body {\n    pub passphrase: String,\n}\n",
            "#[derive(Debug, Serialize)]\n#[serde(rename_all = \"camelCase\")]\n"
            "pub(crate) struct Body {\n    user_password: Vec<u8>,\n}\n",
            "#[derive(Serialize)]\nstruct Body {\n    #[serde(rename = \"passphrase\")]\n    p: String,\n}\n",
            "fn send() { let b = serde_json::json!({ \"passphrase\": p }); }\n",
            "#[derive(Serialize)]\nstruct Body {\n    secret: Option<Confidential<Plaintext>>,\n}\n",
        ):
            with tempfile.TemporaryDirectory() as tmp:
                root = scratch(tmp)
                write(root, f"{cb.WORKER}/auth.rs", text)
                errors = self.run_checks(root)
            self.assertTrue(any("names a passphrase" in e for e in errors), (text, errors))

    def test_a_passphrase_outside_a_request_body_stays_free(self):
        """A handler parameter, a deserialized reply and a log line send nothing."""
        import tempfile

        text = (
            "fn unlock(passphrase: Confidential<Passphrase>) {\n"
            "    tracing::warn!(\"wrong passphrase\");\n}\n"
            "#[derive(Deserialize)]\nstruct Reply {\n    passphrase_hint: String,\n}\n"
            "#[derive(Serialize)]\nstruct Body {\n    salt: Vec<u8>,\n}\n"
            "fn later(passphrase: Confidential<Passphrase>) {}\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, f"{cb.WORKER}/auth.rs", text)
            errors = self.run_checks(root)
        self.assertEqual(errors, [])

    def test_a_relay_log_naming_a_payload_is_refused(self):
        for text in (
            'fn put() {\n    tracing::info!("stored {payload:?}");\n}\n',
            "fn put() {\n    tracing::debug!(\n        doc = %id,\n        body = ?request_body,\n    );\n}\n",
            "#[instrument(fields(blob))]\nfn put(blob: Vec<u8>) {}\n",
        ):
            errors = self.breach(f"{cb.RELAY}/blob.rs", text)
            self.assertTrue(any("names a payload" in e for e in errors), (text, errors))

    def test_a_payload_length_a_message_word_and_a_test_stay_free(self):
        text = (
            'fn put(payload: Vec<u8>) {\n    tracing::warn!(payload_len, "payload too large");\n'
            "    tracing::info!(n = payload.len());\n}\n"
            '#[cfg(test)]\nmod tests {\n    fn t() { println!("{payload:?}"); }\n}\n'
        )
        self.assertEqual(self.breach(f"{cb.RELAY}/blob.rs", text), [])

    def test_a_file_that_logs_and_reads_a_payloads_bytes_is_refused(self):
        log = 'fn put() {\n    tracing::info!(seq, "stored");\n}\n'
        for read in (
            "fn r() { let x = frame.expose_bytes(); }\n",
            "fn keep(c: &ambre_protocol::messages::Ciphertext) {}\n",
            "fn keep(c: &ambre_core::Payload<ambre_protocol::messages::Ciphertext>) {}\n",
        ):
            text = log + read
            errors = self.breach(f"{cb.RELAY}/store.rs", text)
            self.assertTrue(any("reads a payload's bytes" in e for e in errors), (text, errors))

    def test_a_file_that_reads_bytes_without_logging_stays_free(self):
        text = (
            "fn keep(c: &Ciphertext, frame: &StoredPayload) {\n    put(&c.0, frame.expose_bytes());\n}\n"
            '#[cfg(test)]\nmod tests {\n    fn t() { tracing::info!("x"); }\n}\n'
        )
        self.assertEqual(self.breach(f"{cb.RELAY}/store.rs", text), [])
        logs = 'fn put(frame: &StoredPayload) {\n    tracing::info!(len = frame.len(), "stored");\n}\n'
        self.assertEqual(self.breach(f"{cb.RELAY}/handler.rs", logs), [])

    def test_a_handler_that_sends_on_a_queue_is_refused(self):
        for line in (
            "struct S { tx: Sender<UiEvent> }\n",
            "fn h(s: &mut S) { s.out.try_send(e); }\n",
            "fn h(s: &mut S) { s.send_to_sync(c); }\n",
        ):
            for rel in ("crates/ambre-trame/src/logic/state.rs", "crates/ambre-new/src/logic/mod.rs"):
                errors = self.breach(rel, line)
                self.assertTrue(any("sends on a queue" in e for e in errors), (rel, line, errors))

    def test_a_handler_that_returns_an_outcome_stays_free(self):
        text = "fn h(&mut self, c: Command) -> Outcome { Outcome::default() }\n"
        self.assertEqual(self.breach("crates/ambre-toile/src/logic/state.rs", text), [])

    def test_a_launcher_writing_past_its_outbox_is_refused(self):
        for line in (
            "struct W { to_logic: ambre_bus::conduit::Sender<LogicCommand> }\n",
            "fn view() { let p = Port::embedded(tx); }\n",
            "fn rig() { let p = ambre_bus::Port::direct(tx); }\n",
        ):
            errors = self.breach(f"{cb.LAUNCHER}/landing.rs", line)
            self.assertTrue(any("writes past its outbox" in e for e in errors), (line, errors))

    def test_a_raw_sender_kept_past_the_outbox_is_refused(self):
        text = (
            "fn queues() {\n    let (ui_tx, ui_rx) =\n        conduit::to_worker::<LogicCommand>(CAP, &bell);\n"
            "    let relay = ui_tx.clone();\n    let shell = outbox::ShellOutbox::new(ui_tx);\n}\n"
        )
        errors = self.breach(cb.APP_LIB, text)
        self.assertTrue(any("used past its outbox" in e for e in errors), errors)
        kept = "fn queues() {\n    let (ui_tx, ui_rx) =\n        conduit::to_worker::<LogicCommand>(CAP, &bell);\n}\n"
        errors = self.breach(cb.APP_LIB, kept)
        self.assertTrue(any("never moves into the outbox" in e for e in errors), errors)

    def test_a_relay_that_neither_flushes_nor_tells_is_refused(self):
        text = (
            "fn queues() {\n    let (ui_tx, ui_rx) =\n        conduit::to_worker::<LogicCommand>(CAP, &bell);\n"
            "    let shell = outbox::ShellOutbox::new(ui_tx);\n}\n"
            "fn turn() { sync_inbox.borrow_mut().flush(); }\n"
        )
        errors = self.breach(cb.APP_LIB, text)
        self.assertTrue(any("does not flush the launcher's outbox" in e for e in errors), errors)
        self.assertTrue(any("no refusal of the launcher's outbox" in e for e in errors), errors)

    def test_a_port_built_in_a_test_or_the_outbox_stays_free(self):
        text = "fn f() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() { Port::direct(tx); }\n}\n"
        self.assertEqual(self.breach(f"{cb.LAUNCHER}/host/prose.rs", text), [])

    def test_a_panic_hook_that_latches_nothing_is_refused(self):
        text = "fn hook() { std::panic::set_hook(Box::new(|i| { tracing::error!(\"{i}\"); })); }\n"
        errors = self.breach(cb.APP_LIB, text)
        self.assertTrue(any("panic hook does not latch" in e for e in errors), errors)

    def test_a_worker_error_that_latches_nothing_is_refused(self):
        text = "fn spawn() {\n    let e = Closure::new(|_| { tracing::error!(\"x\"); });\n    worker.set_onerror(Some(e));\n}\n"
        errors = self.breach(cb.WORKER_SPAWNER, text)
        self.assertTrue(any("error event does not latch" in e for e in errors), errors)

    def test_a_build_that_unwinds_on_a_panic_is_refused(self):
        for text in ('[unstable]\nbuild-std = ["std", "panic_unwind"]\n', "[unstable]\n"):
            errors = self.breach(cb.CARGO_CONFIG, text)
            self.assertTrue(any("does not abort on a panic" in e for e in errors), (text, errors))

    def test_a_destructor_in_a_thread_that_writes_is_refused(self):
        text = "impl Drop for Flush {\n    fn drop(&mut self) { self.save(); }\n}\n"
        for rel in ("crates/ambre-logic/src/flush.rs", "crates/ambre-toile/src/logic/flush.rs"):
            errors = self.breach(rel, text)
            self.assertTrue(any("destructor on its way out" in e for e in errors), (rel, errors))
        self.assertEqual(self.breach("crates/ambre-canvas/src/flush.rs", text), [])

    def test_a_decay_outside_the_kinetic_model_is_refused(self):
        for line in ("const FRICTION: f64 = 0.95;\n", "fn step(velocity_x: f64) {}\n"):
            errors = self.breach("crates/ambre-toile/src/ui/pan.rs", line)
            self.assertTrue(any("kinetic model outside" in e for e in errors), (line, errors))

    def test_an_axis_list_reached_outside_the_axis_module_is_refused(self):
        for line in (
            "fn rows(s: &S) -> SheetContainer { SheetContainer::of(SheetRoot::Rows, &s.sheet) }\n",
            "fn cols(d: &LoroDoc) { d.get_movable_list(\"cols_s0\"); }\n",
            "const GEN: &str = \"line_gen_\";\n",
            "fn back(d: &LoroDoc) { d.get_movable_list(name); }\n",
        ):
            errors = self.breach("crates/ambre-trame/src/logic/sort.rs", line)
            self.assertTrue(any("axis list reached outside" in e for e in errors), (line, errors))

    def test_the_axis_names_declared_once_and_the_tab_strip_stay_free(self):
        for rel, text in (
            (cb.AXIS_REGISTRY, "enum SheetRoot {\n    Rows = \"rows\",\n    Cols = \"cols\",\n}\n"),
            ("crates/ambre-trame/src/logic/sheets.rs", "fn tabs(d: &LoroDoc) { d.get_movable_list(Root::SheetOrder.as_str()); }\n"),
            ("crates/ambre-trame/src/logic/sort.rs", "fn s() {}\n\n#[cfg(test)]\nmod tests {\n    fn r() { SheetRoot::Rows; }\n}\n"),
        ):
            self.assertEqual(self.breach(rel, text), [], rel)

    def test_an_axis_module_naming_no_list_holds_nothing(self):
        errors = self.breach(cb.AXIS_MODULE, "fn nothing() {}\n")
        self.assertTrue(any("holds nothing" in e for e in errors), errors)

    def test_the_kinetic_model_named_from_a_view_stays_free(self):
        text = "// A flick decays through `Kinetic`.\nfn fling(k: &mut Kinetic) { k.step(dt); }\n"
        self.assertEqual(self.breach("crates/ambre-toile/src/ui/pan.rs", text), [])

    def test_a_primitive_taking_screen_state_is_refused(self):
        for signature, said in (
            ("fn fill_rect(&mut self, rect: Rect, color: Color, density: f32);", "neither geometry nor paint"),
            ("fn stroke_rect(&mut self, rect: Rect, frame: &FrameStats);", "neither geometry nor paint"),
            ("fn stroke_line(&mut self, line: Line, zoom: f64);", "bare number `zoom`"),
        ):
            errors = self.breach(cb.DRAW_TRAIT, f"pub trait DrawSurface {{\n    {signature}\n}}\n")
            self.assertTrue(any(said in e for e in errors), (signature, errors))

    def test_a_primitive_split_over_lines_with_a_glyph_run_stays_free(self):
        text = (
            "pub trait DrawSurface {\n"
            "    // A density read here would be the back end's (`CANVAS-056`).\n"
            "    fn draw_glyphs(\n        &mut self,\n        run: &GlyphRun<'_, ParleyBrush>,\n"
            "        brush: &Brush,\n        transform: Affine,\n    );\n"
            "    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {}\n"
            "    fn stroke_line(&mut self, line: Line, color: Color, width: Px<Surface>);\n"
            "}\n\nfn density(scale: f32) {}\n"
        )
        self.assertEqual(self.breach(cb.DRAW_TRAIT, text), [])

    def test_a_drawing_crate_without_the_trait_is_an_error_not_a_pass(self):
        errors = self.breach(cb.DRAW_TRAIT, "pub struct Rect;\n")
        self.assertTrue(any("holds nothing" in e for e in errors), errors)

    def test_a_scroll_moved_by_a_bare_distance_is_refused(self):
        for rel, text, said in (
            ("crates/ambre-prose/src/view/caret.rs", "fn f(v: &mut V) { v.scroll_y += 12.0; }\n", "outside its setter"),
            ("crates/ambre-toile/src/ui/view.rs", "fn f(c: &mut C) { c.center = c.center - d; }\n", "outside its setter"),
            ("crates/ambre-trame/src/ui/view/input.rs", "fn f(g: &mut G) { g.scroll_by(3, 0); }\n", "outside its accumulator"),
            ("crates/ambre-prose/src/view/fling.rs", "fn f() { ScrollStep::dragged((0.0, d), (0.0, 0.0)); }\n", "drag step"),
            (cb.GRID_SCROLL, "fn accumulate(&mut self, dy: f32) {}\n", "takes no model step"),
            (
                "crates/ambre-prose/src/view/mod.rs",
                "pub fn scroll_by(&mut self, dy: f32) {\n    self.scroll_y += dy;\n}\n",
                "takes no model step",
            ),
        ):
            errors = self.breach(rel, text)
            self.assertTrue(any(said in e for e in errors), (rel, errors))

    def test_a_scroll_set_to_a_target_or_moved_in_a_test_stays_free(self):
        text = (
            "    pub fn scroll_by(&mut self, step: ScrollStep) {\n        self.scroll_y = self.scroll_y + step.y();\n    }\n"
            "    // The offset is set to the caret, never moved by a distance.\n"
            "    fn to_caret(&mut self) { self.scroll_y = self.caret_top; }\n"
            "\n#[cfg(test)]\nmod tests {\n    fn t(v: &mut V) { v.scroll_y += 5.0; }\n}\n"
        )
        self.assertEqual(self.breach("crates/ambre-prose/src/view/mod.rs", text), [])

    def test_a_test_module_declared_under_a_second_attribute_is_test_code(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            lib = '#[cfg(test)]\n#[cfg(not(target_arch = "wasm32"))]\nmod input_tests;\n'
            write(root, "crates/ambre-prose/src/lib.rs", lib)
            write(root, "crates/ambre-prose/src/input_tests.rs", "fn t(v: &mut V) { v.scroll_y += 5.0; }\n")
            self.assertEqual(self.run_checks(root), [])

    def test_a_name_composed_outside_its_type_is_refused(self):
        for line in ('let t = format!("Untitled - {label}");\n', 'let n = name.unwrap_or("Untitled");\n'):
            errors = self.breach("crates/ambre-app/src/landing.rs", line)
            self.assertTrue(any("composed outside" in e for e in errors), (line, errors))

    def test_a_frame_placeholder_or_a_comment_stays_free(self):
        text = '// An unnamed card shows "Untitled".\nlet l = "Untitled frame";\n'
        self.assertEqual(self.breach("crates/ambre-toile/src/chrome/choosers.rs", text), [])

    def test_a_composed_name_file_spelling_no_placeholder_is_an_error_not_a_pass(self):
        errors = self.breach(cb.COMPOSED_NAME, "pub struct ComposedName(String);\n")
        self.assertTrue(any("holds nothing" in e for e in errors), errors)

    def test_a_view_field_holding_a_signals_state_is_refused(self):
        for field in ("stats: SelStats,", "pub(crate) last: Option<crate::chrome::SelStats>,", "seen: Vec<Rc<SelStats>>,"):
            text = f"pub struct BoardView {{\n    {field}\n}}\n"
            errors = self.breach("crates/ambre-toile/src/ui/view.rs", text)
            self.assertTrue(any("`sel_stats` signal already holds" in e for e in errors), (field, errors))

    def test_a_view_holding_the_signal_or_a_primitive_stays_free(self):
        text = "pub struct BoardView {\n    stats: RwSignal<Option<SelStats>>,\n    zoom: f64,\n    id: ElementId,\n}\n"
        self.assertEqual(self.breach("crates/ambre-toile/src/ui/view.rs", text), [])
        tests = "pub struct Plain;\n#[cfg(test)]\nmod tests {\n    struct Rig {\n        stats: SelStats,\n    }\n}\n"
        self.assertEqual(self.breach("crates/ambre-trame/src/ui/grid.rs", tests), [])

    def test_an_accepted_view_field_stays_free(self):
        text = "pub struct ToolMachine {\n    tool: SelStats,\n}\n"
        self.assertEqual(self.breach("crates/ambre-toile/src/ui/tool.rs", text), [])

    def test_a_signal_holder_holding_no_signal_is_an_error_not_a_pass(self):
        errors = self.breach("crates/ambre-toile/src/chrome/relay.rs", "pub struct ToileRelay {}\n")
        self.assertTrue(any("holds no signal" in e for e in errors), errors)

    def test_a_signal_carrying_a_bare_primitive_is_refused(self):
        for payload in ("f64", "Option<String>", "Vec<usize>", "(f64, f64)", "[String; 5]", "Option<Rc<bool>>"):
            text = f"pub struct ToileRelay {{\n    pub mode: RwSignal<{payload}>,\n}}\n"
            errors = self.breach("crates/ambre-toile/src/chrome/relay.rs", text)
            self.assertTrue(any("`mode` signal carries a bare" in e for e in errors), (payload, errors))

    def test_a_signal_carrying_a_type_of_its_own_or_accepted_text_stays_free(self):
        text = (
            "pub struct ToileRelay {\n"
            "    pub present: RwSignal<PresentMode>,\n"
            "    pub following: RwSignal<Option<ambre_bus::UserId>>,\n"
            "    pub presenter_notes: RwSignal<String>,\n"
            "}\n"
        )
        self.assertEqual(self.breach("crates/ambre-toile/src/chrome/relay.rs", text + TOILE_ABSORB), [])

    def test_a_bus_identifier_of_a_primitive_type_is_refused(self):
        for text in (
            "pub enum C {\n    Rename { sheet_id: String, name: String },\n}\n",
            "pub enum C {\n    Remove {\n        sheet_id: SheetId,\n        id: String,\n    },\n}\n",
            "pub struct S {\n    pub target_sheet_id: Option<String>,\n}\n",
            "pub struct S {\n    pub row_ids: Vec<u64>,\n}\n",
        ):
            for rel in (f"{cb.BUS}/app/grid.rs", f"{cb.APP_MESSAGES[0]}/command.rs"):
                errors = self.breach(rel, text)
                self.assertTrue(any("carries an identifier as a bare" in e for e in errors), (rel, text, errors))

    def test_a_bus_identifier_of_its_own_type_a_parameter_or_a_test_stays_free(self):
        text = (
            "pub enum C {\n    Go { sheet_id: SheetId, map: HashMap<String, u32> },\n    U { user_id: ambre_core::UserId },\n}\n"
            "pub fn go(sheet_id: &str) {}\n"
            "#[cfg(test)]\nmod tests {\n    struct T {\n        id: u32,\n    }\n}\n"
        )
        self.assertEqual(self.breach(f"{cb.APP_MESSAGES[0]}/command.rs", text), [])

    def test_a_bus_crate_gone_from_the_tree_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.BUS / "lib.rs").unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("identifier boundary reads nothing" in e for e in errors), errors)

    def test_an_application_message_declared_in_the_bus_is_refused(self):
        text = "pub enum TrameCommand {\n    Undo,\n}\n"
        errors = self.breach(f"{cb.BUS}/app/trame.rs", text)
        self.assertTrue(any("declares an application's message" in e for e in errors), errors)

    def test_an_envelope_or_a_test_enum_in_the_bus_stays_free(self):
        text = (
            "pub enum LogicCommand<A> {\n    App(A),\n}\npub enum RestEvent {\n    Done,\n}\n"
            "#[cfg(test)]\nmod tests {\n    pub enum ProbeCommand {\n        Go,\n    }\n}\n"
        )
        self.assertEqual(self.breach(f"{cb.BUS}/logic.rs", text), [])

    def test_a_message_naming_a_crdt_handle_is_refused(self):
        for text in (
            "pub enum E {\n    Doc { doc: loro::LoroDoc },\n}\n",
            "pub struct S {\n    pub tree: LoroTree,\n}\n",
        ):
            errors = self.breach(f"{cb.APP_MESSAGES[-1]}/mod.rs", text)
            self.assertTrue(any("names a CRDT handle" in e for e in errors), (text, errors))

    def test_a_message_carrying_payload_bytes_or_naming_loro_in_a_comment_stays_free(self):
        text = (
            "//! No `loro::LoroDoc` handle here.\n"
            "pub enum E {\n    Updated { payload: ambre_core::LoroPayload },\n}\n"
            "#[cfg(test)]\nmod tests {\n    fn t(_: loro::LoroDoc) {}\n}\n"
        )
        self.assertEqual(self.breach(f"{cb.APP_MESSAGES[-1]}/mod.rs", text), [])

    def test_an_application_vocabulary_gone_from_the_tree_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.APP_MESSAGES[0] / "mod.rs").unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("its identifiers go unread" in e for e in errors), errors)

    def test_a_view_storing_what_the_relay_writes_off_the_event_is_refused(self):
        cases = (
            (
                "crates/ambre-trame/src/ui/view/frame.rs",
                "TrameEvent::SheetList { active, .. } => {\n    self.state.active_sheet = active;\n}\n",
                "`TrameEvent::SheetList` sets `self.state.active_sheet` here, and",
            ),
            # The relay arm names two events; `.set(` sits lines below the
            # receiver.
            (
                "crates/ambre-trame/src/ui/view/frame.rs",
                "TrameEvent::FilterCleared => {\n    self.state.filtered_col = None;\n}\n",
                "writes the `filter_popover` signal off it",
            ),
            (
                "crates/ambre-toile/src/ui/view.rs",
                "ToileEvent::FindResults(fr) => {\n    self.find_hits += fr.hits;\n    self.needs_full = true;\n}\n",
                "writes the `find_result` signal off it",
            ),
        )
        for rel, arm, needle in cases:
            text = f"impl V {{\n    fn dispatch(&mut self, te: E) {{\n        match te {{\n{arm}        }}\n    }}\n}}\n"
            errors = self.breach(rel, text)
            if "filter_popover" in needle:
                self.assertEqual(errors, [], "the popover pair is accepted")
                continue
            self.assertTrue(any(needle in e for e in errors), (arm, errors))

    def test_a_view_arm_keeping_nothing_or_under_test_stays_free(self):
        for text in (
            # A repaint flag holds nothing the event carries.
            "fn d(&mut self) {\n    match te {\n        TrameEvent::SheetList { .. } => {\n"
            "            self.needs_full = true;\n        }\n    }\n}\n",
            # A comparison is no store; an event the relay ignores is free.
            "fn d(&mut self) {\n    match te {\n        TrameEvent::SheetList { active, .. } => {\n"
            "            let same = self.state.active_sheet == active;\n        }\n"
            "        TrameEvent::DataReset => {\n            self.state.cache = None;\n        }\n    }\n}\n",
            "pub struct Plain;\n#[cfg(test)]\nmod tests {\n    fn d(&mut self) {\n        match te {\n"
            "            TrameEvent::SheetList { active, .. } => {\n                self.state.active_sheet = active;\n"
            "            }\n        }\n    }\n}\n",
        ):
            self.assertEqual(self.breach("crates/ambre-trame/src/ui/view/frame.rs", text), [], text)

    def test_a_relay_with_no_event_arm_is_an_error_not_a_pass(self):
        text = "struct ToileRelay {\n    pub sel_stats: RwSignal<Option<chrome::SelStats>>,\n}\n"
        errors = self.breach(cb.EVENT_RELAYS["Toile"], text)
        self.assertTrue(any("no arm on a `ToileEvent`" in e for e in errors), errors)

    def test_a_test_attribute_on_a_statement_does_not_end_the_read(self):
        text = "fn f() {\n    #[cfg(test)]\n    let x = 1;\n}\npub struct BoardView {\n    stats: SelStats,\n}\n"
        errors = self.breach("crates/ambre-toile/src/ui/view.rs", text)
        self.assertTrue(any("`sel_stats` signal already holds" in e for e in errors), errors)

    def test_a_fixture_benchmark_reporting_a_frame_rate_is_refused(self):
        for line in ('c.bench_function("fixture_board_fps", |b| {});\n', "let frame_rate = 60.0 / t; // fixture\n"):
            text = "use ambre_core::fixture::load;\n" + line
            errors = self.breach("crates/ambre-toile/benches/board.rs", text)
            self.assertTrue(any("reports a frame rate" in e for e in errors), (line, errors))

    def test_a_frame_rate_outside_a_fixture_benchmark_stays_free(self):
        text = "use ambre_core::fixture::load;\n// No figure here is a frame rate.\nfn main() {}\n"
        self.assertEqual(self.breach("crates/ambre-toile/benches/board.rs", text), [])
        self.assertEqual(self.breach("crates/ambre-canvas/benches/frame.rs", 'c.bench_function("fps", |b| {});\n'), [])

    def test_a_third_artefact_on_the_page_is_refused(self):
        page = '<link data-trunk rel="rust" />\n<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker" />\n'
        for text in (page + '<link data-trunk rel="rust" href="../macro/Cargo.toml" />\n', '<link data-trunk rel="rust" />\n'):
            errors = self.breach(cb.PAGE, text)
            self.assertTrue(any("wasm artefacts" in e for e in errors), (text, errors))

    def test_a_crate_depending_on_the_worker_is_refused(self):
        errors = self.breach("crates/ambre-app/Cargo.toml", '[dependencies]\nambre-sync = { path = "../ambre-sync" }\n')
        self.assertTrue(any("merges it into a second instance" in e for e in errors), errors)

    def test_a_shared_worker_or_a_second_spawn_is_refused(self):
        for line, words in (
            ("fn s() { web_sys::SharedWorker::new(URL); }\n", "two tabs would share"),
            ("fn s() { nav.service_worker().register(URL); }\n", "two tabs would share"),
            ("fn s() { web_sys::Worker::new(URL); }\n", "spawned outside the launcher"),
        ):
            errors = self.breach("crates/ambre-app/src/shell/extra.rs", line)
            self.assertTrue(any(words in e for e in errors), (line, errors))

    def test_an_unsandboxed_frame_is_refused_and_a_test_frame_stays_free(self):
        frame = 'fn f() { doc.create_element("iframe"); }\n'
        errors = self.breach("crates/ambre-app/src/host/extra.rs", frame)
        self.assertTrue(any("never sandboxes it" in e for e in errors), errors)
        free = 'use wasm_bindgen_test::wasm_bindgen_test;\n' + frame
        self.assertEqual(self.breach("crates/ambre-app/src/shell/narrow.rs", free), [])
        sandboxed = frame + 'fn g() { f.set_attribute("sandbox", S); }\n'
        self.assertEqual(self.breach("crates/ambre-app/src/host/extra.rs", sandboxed), [])

    def test_the_logic_runtime_declaring_the_toolkit_is_refused(self):
        errors = self.breach(cb.LOGIC_MANIFEST, '[dependencies]\nleptos = { workspace = true }\n')
        self.assertTrue(any("declares `leptos`" in e for e in errors), errors)

    def test_the_worker_reaching_the_canvas_through_a_crate_is_refused(self):
        text = '[dependencies]\nambre-core = { path = "../ambre-core" }\nambre-canvas = { path = "../ambre-canvas" }\n'
        errors = self.breach("crates/ambre-bus/Cargo.toml", text)
        self.assertTrue(any("reaches `ambre-canvas`" in e for e in errors), errors)

    def test_the_renderer_declaring_the_bus_is_refused(self):
        for table in ("dependencies", "dev-dependencies", "target.'cfg(target_arch = \"wasm32\")'.dependencies"):
            text = f'[dependencies]\nambre-draw = {{ path = "../ambre-draw" }}\n\n[{table}]\nambre-bus = {{ path = "../ambre-bus" }}\n'
            errors = self.breach(cb.CANVAS_MANIFEST, text)
            self.assertTrue(any("the renderer declares `ambre-bus`" in e for e in errors), (table, errors))

    def test_the_renderer_reaching_the_bus_through_a_crate_is_refused(self):
        text = '[dependencies]\nambre-core = { path = "../ambre-core" }\nambre-bus = { path = "../ambre-bus" }\n'
        errors = self.breach("crates/ambre-draw/Cargo.toml", text)
        self.assertTrue(any("the renderer reaches `ambre-bus` through `ambre-draw`" in e for e in errors), errors)

    def test_a_missing_renderer_manifest_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.CANVAS_MANIFEST).unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("the renderer reach is unread" in e for e in errors), errors)

    def test_a_redeclared_wire_type_is_refused(self):
        errors = self.breach("crates/ambre-server/src/ws.rs", "pub struct Update {\n    seq: u64,\n}\n")
        self.assertTrue(any("redeclares the wire type `Update`" in e for e in errors), errors)

    def test_a_worker_id_or_an_app_message_is_refused(self):
        for line in (
            "fn f() { let id = ambre_bus::next_request(); }\n",
            "fn f(c: TrameCommand) {}\n",
            "fn f() { let id = Seq::<Request>::ZERO.next(); }\n",
            "fn f() { let id = Seq::new(1); }\n",
            "fn f() { let id = ambre_core::Seq::ZERO.next(); }\n",
            "static ISSUER: SharedIssuer<ambre_bus::Request> = SharedIssuer::new(());\n",
            "fn f() { let issuer = Issuer::<Request>::new(()); }\n",
        ):
            errors = self.breach(f"{cb.WORKER}/extra.rs", line)
            self.assertTrue(any("allocates an id or names an app message" in e for e in errors), (line, errors))

    def test_a_worker_test_allocating_an_id_stays_free(self):
        text = "fn f() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() { next_request(); }\n}\n"
        self.assertEqual(self.breach(f"{cb.WORKER}/extra.rs", text), [])

    def test_a_second_engine_start_is_refused(self):
        errors = self.breach(f"{cb.LAUNCHER}/again.rs", "fn again() { ambre_logic::run(bell); }\n")
        self.assertTrue(any("starts the Logic runtime 2 times" in e for e in errors), errors)

    def test_a_frame_that_draws_without_the_relay_is_refused(self):
        errors = self.breach(cb.FRAME_HOST, "fn go() { drive_frame(&raf_c, || {}, render); }\n")
        self.assertTrue(any("does not run `pre_frame`" in e for e in errors), errors)

    def test_a_relay_that_drains_no_sync_queue_is_refused(self):
        errors = self.breach(cb.APP_LIB, "fn boot() { ambre_logic::run(bell); }\n")
        self.assertTrue(any("does not drain the Logic to Sync queue" in e for e in errors), errors)

    def test_a_missing_client_manifest_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.CLIENT).unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("gone from the tree" in e for e in errors), errors)

    def test_a_walk_reaching_a_screen_module_is_refused(self):
        rel = "crates/ambre-prose/src/view/walk.rs"
        for line, module in (
            ("use crate::view::caret::CaretPath;\n", "crate::view::caret"),
            ("use super::{cache::RunKey, column::Column};\n", "crate::view::column"),
            ("fn f() { crate::ui::state::touch(); }\n", "crate::ui::state"),
        ):
            errors = self.breach(rel, WALKS[rel] + line)
            self.assertTrue(any(f"reaches `{module}`" in e for e in errors), (line, errors))

    def test_a_walk_naming_screen_state_is_refused(self):
        rel = "crates/ambre-toile/src/ui/walk.rs"
        for line in ("struct V<'v> { camera: &'v ViewportTransform }\n", "fn f(s: &ToileUiState) {}\n"):
            errors = self.breach(rel, WALKS[rel] + line)
            self.assertTrue(any("names screen state" in e for e in errors), (line, errors))

    def test_a_walk_test_module_stays_free(self):
        rel = "crates/ambre-trame/src/render/walk.rs"
        text = WALKS[rel] + "#[cfg(test)]\nmod tests {\n    use crate::ui::TrameUiState;\n}\n"
        self.assertEqual(self.breach(rel, text), [])

    def test_an_application_without_a_walk_is_an_error_not_a_pass(self):
        rel = "crates/ambre-trame/src/render/walk.rs"
        errors = self.breach(rel, "use super::*;\n")
        self.assertTrue(any("walk boundary holds nothing" in e for e in errors), errors)


if __name__ == "__main__":
    unittest.main()
