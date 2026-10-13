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
        "fn keys() { register(trame_chrome::COMMANDS); }\n",
    )
    write(root, cb.DENSITY, "fn read() -> f64 { window.device_pixel_ratio() }\n")
    write(root, f"{cb.RENDERER}/mod.rs", "fn quad(c: Color) -> [f32; 4] { c.components }\n")
    write(
        root,
        "crates/ambre-sync/Cargo.toml",
        '[dependencies]\ngloo-net = { workspace = true }\n\n[lints]\nworkspace = true\n',
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

    def test_a_missing_client_manifest_is_an_error_not_a_pass(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            (root / cb.CLIENT).unlink()
            errors = self.run_checks(root)
        self.assertTrue(any("gone from the tree" in e for e in errors), errors)


if __name__ == "__main__":
    unittest.main()
