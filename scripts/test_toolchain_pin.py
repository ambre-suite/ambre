#!/usr/bin/env python3
"""What `toolchain_pin.py` refuses, and what it lets through.

Each test plants one file through the pure functions and asserts the verdict.
Listing the tracked files is left to the run on the tree.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import toolchain_pin  # noqa: E402  -- the module under test

PIN = '[toolchain]\nchannel = "nightly-2026-03-29"\ncomponents = ["rust-src"]\n'


def refused(path: str, text: str) -> list[str]:
    return toolchain_pin.file_breaches(path, text)


class TheRootPin(unittest.TestCase):
    def test_a_dated_nightly_passes(self) -> None:
        self.assertEqual(toolchain_pin.pin_breaches(PIN), [])

    def test_a_missing_file_fails(self) -> None:
        self.assertIn("missing", toolchain_pin.pin_breaches(None)[0])

    def test_a_floating_nightly_fails(self) -> None:
        self.assertTrue(toolchain_pin.pin_breaches('channel = "nightly"\n'))

    def test_stable_fails(self) -> None:
        self.assertTrue(toolchain_pin.pin_breaches('channel = "stable"\n'))

    def test_a_date_that_does_not_exist_fails(self) -> None:
        self.assertIn("not a date", toolchain_pin.pin_breaches('channel = "nightly-2026-02-30"\n')[0])


class ASecondToolchainFile(unittest.TestCase):
    def test_a_nested_workspace_file_fails(self) -> None:
        self.assertTrue(refused("fuzz/rust-toolchain.toml", PIN))

    def test_the_legacy_name_fails(self) -> None:
        self.assertTrue(refused("spikes/a/rust-toolchain", "nightly\n"))

    def test_the_root_file_passes(self) -> None:
        self.assertEqual(refused("rust-toolchain.toml", PIN), [])


class ANamedToolchain(unittest.TestCase):
    def test_a_channel_argument_in_a_recipe_fails(self) -> None:
        self.assertTrue(refused("justfile", "fuzz:\n    cargo +nightly fuzz run x\n"))

    def test_a_version_argument_in_a_script_fails(self) -> None:
        self.assertTrue(refused("run_tests.sh", "cargo +1.80.0 test\n"))

    def test_the_environment_override_fails(self) -> None:
        self.assertTrue(refused("scripts/x.py", 'env["RUSTUP_TOOLCHAIN"] = "stable"\n'))

    def test_a_toolchain_flag_fails(self) -> None:
        self.assertTrue(refused(".githooks/pre-push", "rustup component add clippy --toolchain beta\n"))

    def test_a_rustup_install_fails(self) -> None:
        self.assertTrue(refused(".github/workflows/ci.yml", "      - run: rustup toolchain install stable\n"))

    def test_a_rust_image_fails(self) -> None:
        self.assertTrue(refused("docker-compose.yml", "    image: rust:1.80-slim\n"))

    def test_a_non_test_rust_file_fails(self) -> None:
        self.assertTrue(refused("crates/a/src/lib.rs", 'Command::new("cargo").arg("+stable");\n'))

    def test_a_rust_test_file_may_quote_one(self) -> None:
        self.assertEqual(refused("crates/a/src/depinfo_tests.rs", '"cargo +stable clippy"\n'), [])
        self.assertEqual(refused("crates/a/tests/belt.rs", '"cargo +stable clippy"\n'), [])

    def test_a_rust_test_file_that_selects_a_toolchain_fails(self) -> None:
        self.assertTrue(refused("crates/a/src/x_tests.rs", 'cmd.env("RUSTUP_TOOLCHAIN", "beta");\n'))
        self.assertTrue(refused("crates/a/tests/belt.rs", 'Command::new("cargo").arg("+stable");\n'))

    def test_a_component_add_passes(self) -> None:
        self.assertEqual(refused(".github/workflows/ci.yml", "        run: rustup component add clippy\n"), [])

    def test_a_plus_inside_a_word_passes(self) -> None:
        self.assertEqual(refused("scripts/x.py", "a-stable+stable_sort = 1\n"), [])


class AWorkflow(unittest.TestCase):
    def test_a_toolchain_input_fails(self) -> None:
        text = "      - uses: some/action@v1\n        with:\n          toolchain: stable\n"
        self.assertTrue(refused(".github/workflows/ci.yml", text))

    def test_a_toolchain_action_fails(self) -> None:
        self.assertTrue(refused(".github/workflows/ci.yml", "      - uses: dtolnay/rust-toolchain@nightly\n"))

    def test_a_toolchain_key_outside_a_workflow_is_not_a_workflow_input(self) -> None:
        self.assertEqual(refused("Trunk.toml", "toolchain = 1\n"), [])


class AManifest(unittest.TestCase):
    def test_a_rust_version_fails(self) -> None:
        self.assertTrue(refused("crates/a/Cargo.toml", '[package]\nrust-version = "1.80"\n'))

    def test_an_inherited_rust_version_fails(self) -> None:
        self.assertTrue(refused("fuzz/Cargo.toml", "[package]\nrust-version.workspace = true\n"))


class AMarkdownFile(unittest.TestCase):
    def test_a_shell_fence_fails(self) -> None:
        self.assertTrue(refused("README.md", "Run:\n\n```bash\ncargo +stable build\n```\n"))

    def test_prose_and_other_fences_pass(self) -> None:
        text = "A rule refuses `+nightly` and `RUSTUP_TOOLCHAIN`.\n\n```text\ncargo +stable\n```\n"
        self.assertEqual(refused("specs/a.md", text), [])

    def test_a_line_after_a_shell_fence_closes_passes(self) -> None:
        self.assertEqual(refused("a.md", "```sh\nls\n```\ncargo +stable\n"), [])


class Reach(unittest.TestCase):
    def test_the_gate_reads_the_files_that_run_a_build(self) -> None:
        for path in ["justfile", ".github/workflows/ci.yml", "fuzz/Cargo.toml", ".cargo/config.toml",
                     "run_tests.sh", "scripts/contracts.py", "crates/a/src/lib.rs", "README.md"]:
            with self.subTest(path=path):
                self.assertTrue(toolchain_pin.is_read(path))

    def test_the_gate_reads_neither_itself_nor_binary_data(self) -> None:
        for path in ["scripts/toolchain_pin.py", "scripts/test_toolchain_pin.py", "fixtures/a.ambre"]:
            with self.subTest(path=path):
                self.assertFalse(toolchain_pin.is_read(path))


if __name__ == "__main__":
    unittest.main()
