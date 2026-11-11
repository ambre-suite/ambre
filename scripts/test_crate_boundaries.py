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


def scratch(tmp: str) -> Path:
    """A minimal tree that passes every check."""
    root = Path(tmp)
    write(root, "crates/ambre-app/Cargo.toml", '[dependencies]\nambre-logic = { path = "../ambre-logic" }\n')
    write(root, cb.CHROME, '[dependencies]\nambre-commands = { path = "../ambre-commands" }\n')
    write(
        root,
        "crates/ambre-app/src/lib.rs",
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
        "crates/ambre-app/src/outbox.rs",
        "pub(crate) struct ShellOutbox { hold: StoredValue<Hold, LocalStorage> }\n"
        "fn new(to_logic: Sender<LogicCommand>) -> Self { Self::with_lane(Lane::new(to_logic)) }\n",
    )
    write(root, cb.DENSITY, "fn read() -> f64 { window.device_pixel_ratio() }\n")
    write(
        root,
        "crates/ambre-sync/Cargo.toml",
        '[lib]\ncrate-type = ["cdylib", "rlib"]\n\n[dependencies]\ngloo-net = { workspace = true }\n'
        'ambre-bus = { path = "../ambre-bus" }\n\n[lints]\nworkspace = true\n',
    )
    write(
        root,
        "Cargo.toml",
        '[workspace]\nmembers = [\n    "crates/ambre-sync",\n]\n\n'
        '[workspace.lints.clippy]\nunwrap_used = "deny"\n',
    )
    write(root, "crates/ambre-sync/src/http.rs", "fn put() { gloo_net::http::Request::put(url); }\n")
    write(root, "crates/ambre-canvas/src/host.rs", "fn font() { window.fetch_with_str(url); }\n")
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
    write(root, "crates/ambre-canvas/src/render/quad.wgsl", "@vertex\nfn vs() {}\n")
    write(root, cb.PIPE, 'fn create() {\n    for (k, v) in [("tabindex", "0")] {}\n}\n')
    write(root, cb.COMPOSED_NAME, 'const UNTITLED: &str = "Untitled";\n')
    write(
        root,
        cb.PAGE,
        '<link data-trunk rel="rust" data-wasm-opt="0" />\n'
        '<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker"\n      data-loader-shim="true" />\n'
        '<link data-trunk rel="rust" href="../ambre-call/Cargo.toml" data-type="worker"\n      data-loader-shim="true" />\n',
    )
    write(
        root,
        "crates/ambre-app/src/platform.rs",
        "fn spawn() {\n    web_sys::Worker::new(URL);\n"
        "    let e = Closure::new(|_| { ambre_canvas::signal_fatal(r); });\n"
        "    worker.set_onerror(Some(e));\n}\n",
    )
    write(root, "crates/ambre-bus/Cargo.toml", '[dependencies]\nambre-core = { path = "../ambre-core" }\n')
    write(root, "crates/ambre-trame/src/logic/mod.rs", "fn h(&mut self) -> Outcome { Outcome::default() }\n")
    return root


class CrateBoundaries(unittest.TestCase):
    def run_checks(self, root: Path) -> list[str]:
        with Tree(root):
            errors: list[str] = []
            cb.check_gpu_subset(errors)
            cb.check_widths(errors)
            cb.check_modals_and_drags(errors)
            cb.check_key_verification(errors)
            cb.check_accessibility(errors)
            cb.check_composed_names(errors)
            cb.check_instances(errors)
            return errors

    def breach(self, rel: str, text: str) -> list[str]:
        """The errors of the passing tree with one file written into it."""
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            write(root, rel, text)
            return self.run_checks(root)

    def test_a_module_declared_for_the_tests_stays_free(self):
        import tempfile

        engine = "fn s() { let w = window.inner_width(); }\n"
        with tempfile.TemporaryDirectory() as tmp:
            root = scratch(tmp)
            view = '#[cfg(all(test, not(target_arch = "wasm32")))]\nmod view_tests;\nmod input;\n'
            write(root, "crates/ambre-trame/src/ui/view/mod.rs", view)
            write(root, "crates/ambre-trame/src/ui/view/view_tests.rs", engine)
            write(root, "crates/ambre-trame/src/ui/view/input.rs", engine)
            write(root, "crates/ambre-trame/src/ui/grid/mod.rs", "#[cfg(test)]\nmod tests;\n")
            write(root, "crates/ambre-trame/src/ui/grid/tests/mod.rs", "mod viewport;\n")
            write(root, "crates/ambre-trame/src/ui/grid/tests/viewport.rs", engine)
            errors = [e for e in self.run_checks(root) if "reads the window" in e]
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("ui/view/input.rs", errors[0])

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

    def test_the_real_tree_is_clean(self):
        self.assertEqual(self.run_checks(cb.ROOT), [])

    def test_the_scratch_tree_is_clean(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.run_checks(scratch(tmp)), [])

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

    def test_a_test_attribute_on_a_statement_does_not_end_the_read(self):
        text = 'fn f() {\n    #[cfg(test)]\n    let x = 1;\n}\nfn g() -> &str { "Untitled" }\n'
        errors = self.breach("crates/ambre-toile/src/ui/view.rs", text)
        self.assertTrue(any("composed outside" in e for e in errors), errors)

    def test_a_fourth_artefact_on_the_page_is_refused(self):
        page = (
            '<link data-trunk rel="rust" />\n<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker" />\n'
            '<link data-trunk rel="rust" href="../ambre-call/Cargo.toml" data-type="worker" />\n'
        )
        self.assertEqual([e for e in self.breach(cb.PAGE, page) if "wasm artefacts" in e], [])
        two = '<link data-trunk rel="rust" />\n<link data-trunk rel="rust" href="../ambre-sync/Cargo.toml" data-type="worker" />\n'
        for text in (page + '<link data-trunk rel="rust" href="../macro/Cargo.toml" />\n', two, '<link data-trunk rel="rust" />\n'):
            errors = self.breach(cb.PAGE, text)
            self.assertTrue(any("wasm artefacts" in e for e in errors), (text, errors))

    def test_an_unsandboxed_frame_is_refused_and_a_test_frame_stays_free(self):
        frame = 'fn f() { doc.create_element("iframe"); }\n'
        errors = self.breach("crates/ambre-app/src/host/extra.rs", frame)
        self.assertTrue(any("never sandboxes it" in e for e in errors), errors)
        free = 'use wasm_bindgen_test::wasm_bindgen_test;\n' + frame
        self.assertEqual(self.breach("crates/ambre-app/src/shell/narrow.rs", free), [])
        sandboxed = frame + 'fn g() { f.set_attribute("sandbox", S); }\n'
        self.assertEqual(self.breach("crates/ambre-app/src/host/extra.rs", sandboxed), [])

