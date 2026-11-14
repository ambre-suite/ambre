#!/usr/bin/env python3
"""What `ci_cache.py` keeps of `target/`, and what it dates.

`prune` must keep each dependency and each crate of `std`, which `rust-cache`
would delete, and must drop the workspace's own artifacts at any depth. `pin`
must date the whole tree it is given. Both run on a temporary tree.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci_cache  # noqa: E402  -- the module under test

HASH = "0123456789abcdef"

METADATA = {
    "packages": [
        {
            "name": "ambre-core",
            "targets": [{"name": "ambre_core"}, {"name": "model_props"}],
        }
    ]
}


def touch(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("")


class Prune(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.target = Path(self.tmp.name)
        self.names = ci_cache.workspace_names(METADATA)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def tree(self, profile: str) -> Path:
        base = self.target / profile
        touch(base / "deps" / f"libstd-{HASH}.rlib")
        touch(base / "deps" / f"libcore-{HASH}.rmeta")
        touch(base / "deps" / f"libserde-{HASH}.rlib")
        touch(base / "deps" / f"libambre_core-{HASH}.rlib")
        touch(base / "deps" / f"ambre_core-{HASH}.d")
        touch(base / "deps" / f"model_props-{HASH}")
        touch(base / ".fingerprint" / f"std-{HASH}" / "lib-std")
        touch(base / ".fingerprint" / f"ambre-core-{HASH}" / "lib-ambre_core")
        touch(base / "build" / f"serde-{HASH}" / "output")
        touch(base / "build" / f"ambre-core-{HASH}" / "output")
        touch(base / "incremental" / "x" / "y")
        touch(base / "ambre-server")
        return base

    def test_std_and_the_dependencies_stay(self) -> None:
        base = self.tree("debug")
        ci_cache.prune(self.target, self.names)
        for kept in (
            f"deps/libstd-{HASH}.rlib",
            f"deps/libcore-{HASH}.rmeta",
            f"deps/libserde-{HASH}.rlib",
            f".fingerprint/std-{HASH}",
            f"build/serde-{HASH}",
        ):
            with self.subTest(kept=kept):
                self.assertTrue((base / kept).exists())

    def test_the_workspace_artifacts_go(self) -> None:
        base = self.tree("debug")
        ci_cache.prune(self.target, self.names)
        for gone in (
            f"deps/libambre_core-{HASH}.rlib",
            f"deps/ambre_core-{HASH}.d",
            f"deps/model_props-{HASH}",
            f".fingerprint/ambre-core-{HASH}",
            f"build/ambre-core-{HASH}",
            "incremental",
            "ambre-server",
        ):
            with self.subTest(gone=gone):
                self.assertFalse((base / gone).exists())

    def test_a_target_triple_is_pruned_as_the_host(self) -> None:
        base = self.tree("wasm32-unknown-unknown/debug")
        ci_cache.prune(self.target, self.names)
        self.assertTrue((base / "deps" / f"libstd-{HASH}.rlib").exists())
        self.assertFalse((base / "deps" / f"libambre_core-{HASH}.rlib").exists())

    def test_the_build_directory_layout_is_read_by_package(self) -> None:
        base = self.target / "debug"
        touch(base / "build" / "std" / HASH / "out" / f"libstd-{HASH}.rlib")
        touch(base / "build" / "serde" / HASH / "fingerprint" / "lib-serde")
        touch(base / "build" / "ambre-core" / HASH / "out" / f"model_props-{HASH}")
        ci_cache.prune(self.target, self.names)
        self.assertTrue((base / "build" / "std" / HASH).exists())
        self.assertTrue((base / "build" / "serde" / HASH).exists())
        self.assertFalse((base / "build" / "ambre-core").exists())

    def test_loose_files_go_and_the_tag_stays(self) -> None:
        touch(self.target / "CACHEDIR.TAG")
        touch(self.target / "specs" / "ambre-contract-index.json")
        ci_cache.prune(self.target, self.names)
        self.assertTrue((self.target / "CACHEDIR.TAG").exists())
        self.assertFalse((self.target / "specs" / "ambre-contract-index.json").exists())

    def test_a_name_without_a_hash_is_read_whole(self) -> None:
        self.assertEqual(ci_cache.crate_of("ambre-core"), "ambre-core")
        self.assertEqual(ci_cache.crate_of(f"libambre_core-{HASH}.rlib"), "libambre_core")

    def test_a_dependency_named_like_a_prefix_stays(self) -> None:
        base = self.target / "debug"
        touch(base / "deps" / f"libambre-{HASH}.rlib")
        ci_cache.prune(self.target, self.names)
        self.assertTrue((base / "deps" / f"libambre-{HASH}.rlib").exists())


class Pin(unittest.TestCase):
    def test_every_entry_is_dated(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tree = Path(tmp)
            touch(tree / "core" / "src" / "lib.rs")
            touch(tree / "std" / "Cargo.toml")
            count = ci_cache.pin(tree, 1000)
            self.assertEqual(count, 5)
            for path in tree.rglob("*"):
                with self.subTest(path=path):
                    self.assertEqual(int(os.stat(path).st_mtime), 1000)

    def test_a_missing_tree_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(SystemExit):
            ci_cache.pin(Path(tmp) / "absent")


if __name__ == "__main__":
    unittest.main()
