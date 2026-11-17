#!/usr/bin/env python3
"""What `wasm_size.py` passes, and what it fails.

The verdict is tested on figures alone: a brotli figure over its budget
fails, and so does the raw figure of Main or Sync over its ceiling. No
other raw figure fails, and no change against main does. Building the release is left
to `just size`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import wasm_size  # noqa: E402  -- the module under test

CEILINGS = {"Main": 3_000_000, "Sync": 1_000_000}


class Verdict(unittest.TestCase):
    def test_a_figure_over_its_ceiling_fails(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 3_000_001, "Sync": 1}, CEILINGS, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", lines[0])

    def test_sync_over_its_raw_ceiling_fails_under_the_budget(self) -> None:
        # Sync holds a raw ceiling of its own (`ARCH-096`): its brotli stays
        # far under the first download budget, and the build still fails.
        sizes = {"Main": 1, "Sync": 1_000_001}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {}, {"Main": 1, "Sync": 1})
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", lines[1])

    def test_an_artefact_no_ceiling_names_fails_nothing_on_its_raw_bytes(self) -> None:
        # The ceilings of the call worker and the modules became a report
        # (`ARCH-096`): a raw figure far over its old ceiling passes.
        sizes = {"Main": 1, "Sync": 1, "Call worker": 9_000_000}
        compressed = {"Main": 1, "Sync": 1, "Call worker": 1}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {}, compressed)
        self.assertTrue(ok)
        self.assertEqual("Call worker: 9,000,000 bytes raw; 1 brotli", lines[2])

    def test_figures_under_the_ceilings_but_over_the_budget_fail(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 2_400_000, "Sync": 100_000}, CEILINGS, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("500,000 over the budget", lines[-1])

    def test_figures_within_the_budget_pass(self) -> None:
        ok, lines = wasm_size.verdict({"Main": 1_700_000, "Sync": 200_000}, CEILINGS, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("within the budget", lines[-1])


class FirstDownload(unittest.TestCase):
    def test_a_module_stays_out_of_the_first_download(self) -> None:
        sizes = {"Main": 1_700_000, "Sync": 200_000, "PDF module": 400_000}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {"PDF module": 600_000})
        self.assertTrue(ok)
        self.assertIn("First download: 1,900,000", lines[-1])

    def test_a_module_far_over_its_old_raw_ceiling_passes_under_its_budget(self) -> None:
        sizes = {"Main": 1, "Sync": 1, "PDF module": 5_000_000}
        compressed = {"Main": 1, "Sync": 1, "PDF module": 599_999}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {"PDF module": 600_000}, compressed)
        self.assertTrue(ok)
        self.assertNotIn("over", lines[2])


class ModuleBudget(unittest.TestCase):
    def test_a_module_over_its_budget_fails(self) -> None:
        sizes = {"Main": 1, "Sync": 1, "Markup module": 300_001}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {"Markup module": 300_000})
        self.assertFalse(ok)
        self.assertIn("1 over the budget", lines[2])

    def test_every_lazy_artefact_holds_a_budget(self) -> None:
        lazy = wasm_size.GLOBS.keys() - set(wasm_size.FIRST_DOWNLOAD)
        self.assertEqual(lazy, wasm_size.MODULE_BUDGETS.keys())
        self.assertTrue(all(budget > 0 for budget in wasm_size.MODULE_BUDGETS.values()))

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
        sizes = {"Main": 1, "Sync": 1, "PDF module": 1_500_000}
        compressed = {"Main": 1, "Sync": 1, "PDF module": 600_001}
        ok, lines = wasm_size.verdict(sizes, CEILINGS, 2_000_000, {"PDF module": 600_000}, compressed)
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
    def test_the_first_download_alone_holds_raw_ceilings(self) -> None:
        self.assertEqual(wasm_size.CEILINGS.keys(), set(wasm_size.FIRST_DOWNLOAD))
        self.assertTrue(all(figure > 0 for figure in wasm_size.CEILINGS.values()))


HEAD = wasm_size.Measure("a" * 40, {"Main": 1_000, "Sync": 500}, {"Main": 300, "Sync": 100})


class Delta(unittest.TestCase):
    def test_each_artefact_reads_its_change_against_main(self) -> None:
        base = wasm_size.Measure("b" * 40, {"Main": 1_200, "Sync": 500}, {"Main": 290, "Sync": 100})
        rows = wasm_size.delta(HEAD, base)
        self.assertIn("| Main | 1,000 | -200 | 300 | +10 |", rows)
        self.assertIn("| Sync | 500 | 0 | 100 | 0 |", rows)

    def test_an_artefact_main_lacks_reads_new(self) -> None:
        base = wasm_size.Measure("b" * 40, {"Main": 1_000}, {"Main": 300})
        self.assertIn("| Sync | 500 | new | 100 | new |", wasm_size.delta(HEAD, base))

    def test_no_measure_of_main_still_prints_the_figures(self) -> None:
        lines = wasm_size.report(HEAD, None)
        self.assertIn("No measure of `main`", lines[0])
        self.assertIn("| Main | 1,000 | -- | 300 | -- |", lines)

    def test_the_report_names_the_commit_it_reads_against(self) -> None:
        base = wasm_size.Measure("c" * 40, {}, {})
        self.assertIn("`cccccccccccc`", wasm_size.report(HEAD, base)[0])

    def test_a_measure_round_trips_through_its_record(self) -> None:
        self.assertEqual(wasm_size.Measure.from_json(HEAD.to_json()), HEAD)

    def test_a_malformed_record_is_no_measure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "main.json"
            path.write_text('{"commit": "x", "raw": {"Main": "big"}, "brotli": {}}')
            base, why = wasm_size.read_base(path)
            self.assertIsNone(base)
            self.assertIn("is not one", why)
            base, why = wasm_size.read_base(Path(tmp) / "absent.json")
            self.assertIsNone(base)
            self.assertIn("no measure of main", why)


class Workflow(unittest.TestCase):
    """The `size` job of the CI workflow, read as text."""

    def job(self) -> list[str]:
        lines = (wasm_size.ROOT / ".github/workflows/ci.yml").read_text().splitlines()
        start = lines.index("  size:")
        # The job ends at the next key of two spaces, a comment included.
        end = next(i for i in range(start + 1, len(lines)) if lines[i][:3].strip() and lines[i][2] != " ")
        return lines[start:end]

    def test_the_size_job_runs_on_every_push(self) -> None:
        # A job-level `if:` sits at four spaces; a step-level one sits deeper.
        self.assertFalse([line for line in self.job() if line.startswith("    if:")])

    def test_the_size_job_reads_and_records_the_measure_of_main(self) -> None:
        run = next(line for line in self.job() if "just size" in line)
        self.assertIn("--baseline", run)
        self.assertIn("--record", run)

    def test_a_run_of_main_saves_its_measure_under_its_commit(self) -> None:
        job = "\n".join(self.job())
        save = job[job.index("actions/cache/save@"):]
        self.assertIn("key: wasm-size-${{ github.sha }}", save)
        self.assertIn("env.CACHE_WRITE == 'true'", job[job.rindex("if:", 0, job.index("actions/cache/save@")):])


if __name__ == "__main__":
    unittest.main()
