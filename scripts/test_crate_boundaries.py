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
        "fn pre() { start.drain(&logic_to_sync_rx, post); }\n",
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
    write(
        root,
        cb.PAGE,
        '<link data-trunk rel="rust" data-wasm-opt="0" />\n'
        '<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker"\n      data-loader-shim="true" />\n',
    )
    write(root, cb.WORKER_SPAWNER, "fn spawn() { web_sys::Worker::new(URL); }\n")
    write(root, cb.LOGIC_MANIFEST, '[dependencies]\nambre-bus = { path = "../ambre-bus" }\n\n[dev-dependencies]\nwgpu = "1"\n')
    write(root, "crates/ambre-bus/Cargo.toml", '[dependencies]\nambre-core = { path = "../ambre-core" }\n')
    write(root, f"{cb.PROTOCOL}/lib.rs", "pub struct Update {\n    pub seq: u64,\n}\n")
    return root


class CrateBoundaries(unittest.TestCase):
    def run_checks(self, root: Path) -> list[str]:
        with Tree(root):
            errors: list[str] = []
            cb.check_client(errors)
            cb.check_generators(errors)
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
            cb.check_instances(errors)
            cb.check_thread_reach(errors)
            cb.check_worker_messages(errors)
            cb.check_engine_and_relay(errors)
            return errors

    def breach(self, rel: str, text: str) -> list[str]:
        """The errors of the passing tree with one file written into it."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, rel, text)
            return self.run_checks(root)

    def test_a_view_holding_the_document_is_refused(self):
        for line in (
            "fn draw(d: &loro::LoroDoc) {}\n",
            "fn draw(s: &ProseState) {}\n",
            "fn load(b: &[u8]) { let m = ExportMode::Snapshot; }\n",
        ):
            errors = self.breach("crates/ambre-toile/src/ui/view.rs", line)
            self.assertTrue(any("holds the document" in e for e in errors), (line, errors))

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

    def test_a_storage_path_the_rule_does_not_name_is_refused(self):
        line = "fn keep() { window.local_storage(); }\n"
        errors = self.breach("crates/ambre-chrome/src/theme.rs", line)
        self.assertTrue(any("a persistence path the rule does not name" in e for e in errors), errors)
        self.assertEqual(self.breach(cb.STORAGE_OWNERS[2], line), [])

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
        good = "view! { <button aria-pressed=move || aria_bool(on.get()) /> }\n"
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
            "#[derive(Serialize)]\nstruct Body {\n    secret: Option<SecretBytes>,\n}\n",
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
            "fn unlock(passphrase: SecretBytes) {\n"
            "    tracing::warn!(\"wrong passphrase\");\n}\n"
            "#[derive(Deserialize)]\nstruct Reply {\n    passphrase_hint: String,\n}\n"
            "#[derive(Serialize)]\nstruct Body {\n    salt: Vec<u8>,\n}\n"
            "fn later(passphrase: SecretBytes) {}\n"
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

    def test_a_decay_outside_the_kinetic_model_is_refused(self):
        for line in ("const FRICTION: f64 = 0.95;\n", "fn step(velocity_x: f64) {}\n"):
            errors = self.breach("crates/ambre-toile/src/ui/pan.rs", line)
            self.assertTrue(any("kinetic model outside" in e for e in errors), (line, errors))

    def test_the_kinetic_model_named_from_a_view_stays_free(self):
        text = "// A flick decays through `Kinetic`.\nfn fling(k: &mut Kinetic) { k.step(dt); }\n"
        self.assertEqual(self.breach("crates/ambre-toile/src/ui/pan.rs", text), [])

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

    def test_a_redeclared_wire_type_is_refused(self):
        errors = self.breach("crates/ambre-server/src/ws.rs", "pub struct Update {\n    seq: u64,\n}\n")
        self.assertTrue(any("redeclares the wire type `Update`" in e for e in errors), errors)

    def test_a_worker_id_or_an_app_message_is_refused(self):
        for line in ("fn f() { let id = RequestId::next(); }\n", "fn f(c: TrameCommand) {}\n"):
            errors = self.breach(f"{cb.WORKER}/extra.rs", line)
            self.assertTrue(any("allocates an id or names an app message" in e for e in errors), (line, errors))

    def test_a_worker_test_allocating_an_id_stays_free(self):
        text = "fn f() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() { RequestId::next(); }\n}\n"
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


if __name__ == "__main__":
    unittest.main()
