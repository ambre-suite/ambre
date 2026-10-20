#!/usr/bin/env python3
"""What `wasm_size.py` passes, and what it fails.

The verdict is tested on figures alone: an overrun of a ceiling fails, and a
first download between the budget and the ceilings passes while it says how far
over the budget it sits. Building the release is left to `just size`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import wasm_size  # noqa: E402  -- the module under test

CEILINGS = {"Main": 3_000_000, "Sync": 300_000}


class Verdict(unittest.TestCase):
    def test_a_figure_over_its_ceiling_fails(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 3_000_001, "Sync": 1}, CEILINGS, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", lines[0])

    def test_the_sync_ceiling_is_held_on_its_own(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 1, "Sync": 300_001}, CEILINGS, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", lines[1])

    def test_figures_under_the_ceilings_but_over_the_budget_pass_and_say_so(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 2_400_000, "Sync": 100_000}, CEILINGS, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("500,000 over the budget", lines[-1])

    def test_figures_within_the_budget_pass(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 1_700_000, "Sync": 200_000}, CEILINGS, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("within the budget", lines[-1])


class Find(unittest.TestCase):
    def test_one_hashed_file_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "ambre-app-0123abcd_bg.wasm.br").write_bytes(b"x")
            (dist / "ambre-app-0123abcd_bg.wasm").write_bytes(b"x")
            found = wasm_size.find(dist, wasm_size.GLOBS["Main"])
            self.assertEqual(found.name, "ambre-app-0123abcd_bg.wasm.br")

    def test_two_builds_in_one_dist_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "ambre-app-0123abcd_bg.wasm.br").write_bytes(b"x")
            (dist / "ambre-app-4567ef01_bg.wasm.br").write_bytes(b"x")
            with self.assertRaises(FileNotFoundError):
                wasm_size.find(dist, wasm_size.GLOBS["Main"])

    def test_a_missing_copy_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(FileNotFoundError):
            wasm_size.find(Path(tmp), wasm_size.GLOBS["Sync"])


class Ceilings(unittest.TestCase):
    def test_every_instance_has_a_ceiling(self) -> None:
        self.assertEqual(wasm_size.CEILINGS.keys(), wasm_size.GLOBS.keys())
        self.assertTrue(all(ceiling > 0 for ceiling in wasm_size.CEILINGS.values()))


if __name__ == "__main__":
    unittest.main()
