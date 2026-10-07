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
    write(root, "crates/ambre-sync/Cargo.toml", '[dependencies]\ngloo-net = { workspace = true }\n')
    write(root, f"{cb.WORKER}/http.rs", "fn put() { gloo_net::http::Request::put(url); }\n")
    write(root, cb.NETWORK_EXCEPTIONS[0], "fn font() { window.fetch_with_str(url); }\n")
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
            return errors

    def test_the_real_tree_is_clean(self):
        self.assertEqual(self.run_checks(cb.ROOT), [])

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
