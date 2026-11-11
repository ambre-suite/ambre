#!/usr/bin/env python3
"""What `wasm_size.py` passes, and what it fails.

The verdict is tested on figures alone: a raw figure over its ceiling fails,
and so does a brotli figure over its budget, whatever the ceilings. Building
the release is left to `just size`.

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

    def test_figures_under_the_ceilings_but_over_the_budget_fail(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 2_400_000, "Sync": 100_000}, CEILINGS, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("500,000 over the budget", lines[-1])

    def test_figures_within_the_budget_pass(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 1_700_000, "Sync": 200_000}, CEILINGS, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("within the budget", lines[-1])


class FirstDownload(unittest.TestCase):
    def test_a_module_counts_against_its_ceiling_alone(self) -> None:
        ceilings = {**CEILINGS, "PDF module": 500_000}
        sizes = {"Main": 1_700_000, "Sync": 200_000, "PDF module": 400_000}
        ok, lines = wasm_size.verdict(sizes, ceilings, 2_000_000, {"PDF module": 600_000})
        self.assertTrue(ok)
        self.assertIn("First download: 1,900,000", lines[-1])

    def test_a_module_over_its_ceiling_fails(self) -> None:
        ceilings = {**CEILINGS, "PDF module": 500_000}
        sizes = {"Main": 1, "Sync": 1, "PDF module": 500_001}
        ok, lines = wasm_size.verdict(sizes, ceilings, 2_000_000, {"PDF module": 600_000})
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", lines[2])


class ModuleBudget(unittest.TestCase):
    def test_a_module_over_its_budget_fails_whatever_its_ceiling(self) -> None:
        ceilings = {**CEILINGS, "Markup module": 900_000}
        sizes = {"Main": 1, "Sync": 1, "Markup module": 300_001}
        ok, lines = wasm_size.verdict(sizes, ceilings, 2_000_000, {"Markup module": 300_000})
        self.assertFalse(ok)
        self.assertIn("1 over the budget", lines[2])

    def test_each_module_with_a_budget_holds_a_raw_ceiling(self) -> None:
        # A ceiling holds raw bytes and a budget brotli bytes, so the two do
        # not compare: the brotli figure meets the budget in `verdict`.
        for name in wasm_size.MODULE_BUDGETS:
            self.assertGreater(wasm_size.CEILINGS[name], 0, name)

    def test_each_family_the_build_writes_is_measured(self) -> None:
        import converter_build

        globs = [glob for name, glob in wasm_size.GLOBS.items() if name in wasm_size.MODULE_BUDGETS]
        for family in converter_build.FAMILIES:
            self.assertIn(f"ambre-convert-{family}-*.wasm.br", globs)


class TwoUnits(unittest.TestCase):
    def test_the_ceiling_holds_the_raw_figure_and_not_the_brotli_one(self) -> None:
        sizes = {"Main": 3_000_001, "Sync": 1}
        compressed = {"Main": 1_000_000, "Sync": 1}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {}, compressed)
        self.assertFalse(ok)
        self.assertIn("Main: 3,000,001 bytes raw, ceiling 3,000,000; 1,000,000 brotli", lines[0])

    def test_a_raw_figure_over_the_budget_passes_when_its_brotli_holds(self) -> None:
        sizes = {"Main": 2_900_000, "Sync": 290_000}
        compressed = {"Main": 700_000, "Sync": 90_000}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 1_000_000, {}, compressed)
        self.assertTrue(ok)
        self.assertIn("First download: 790,000 bytes brotli", lines[-1])

    def test_the_first_download_budget_holds_the_brotli_figures(self) -> None:
        sizes = {"Main": 1, "Sync": 1}
        compressed = {"Main": 1_500_000, "Sync": 600_000}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {}, compressed)
        self.assertFalse(ok)
        self.assertIn("100,000 over the budget", lines[-1])

    def test_a_module_budget_holds_the_brotli_figure(self) -> None:
        ceilings = {**CEILINGS, "PDF module": 2_000_000}
        sizes = {"Main": 1, "Sync": 1, "PDF module": 1_500_000}
        compressed = {"Main": 1, "Sync": 1, "PDF module": 600_001}
        ok, lines = wasm_size.verdict(sizes, ceilings, 2_000_000, {"PDF module": 600_000}, compressed)
        self.assertFalse(ok)
        self.assertIn("1 over the budget", lines[2])


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

    def test_the_raw_wasm_beside_its_brotli_copy_is_found(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            dist = Path(tmp)
            (dist / "ambre-app-0123abcd_bg.wasm.br").write_bytes(b"x")
            (dist / "ambre-app-0123abcd_bg.wasm").write_bytes(b"xy")
            for name, glob in wasm_size.GLOBS.items():
                self.assertTrue(wasm_size.raw_glob(glob).endswith(".wasm"), name)
            found = wasm_size.find(dist, wasm_size.raw_glob(wasm_size.GLOBS["Main"]))
            self.assertEqual(found.name, "ambre-app-0123abcd_bg.wasm")

    def test_a_missing_copy_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(FileNotFoundError):
            wasm_size.find(Path(tmp), wasm_size.GLOBS["Sync"])


class Ceilings(unittest.TestCase):
    def test_every_instance_has_a_ceiling(self) -> None:
        self.assertEqual(wasm_size.CEILINGS.keys(), wasm_size.GLOBS.keys())
        self.assertTrue(all(ceiling > 0 for ceiling in wasm_size.CEILINGS.values()))


if __name__ == "__main__":
    unittest.main()
