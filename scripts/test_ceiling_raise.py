#!/usr/bin/env python3
"""What `ceiling_raise.py` refuses, and what it lets through.

Each test plants two copies of a limit table, or of the three tables, and
the plan files through the pure functions, and asserts the verdict. Reading the base revision is
left to the run on the tree, except where the base is missing.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import io
import sys
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ceiling_raise  # noqa: E402  -- the module under test

BASE = {"Main": 2_740_000, "Sync": 380_000}
ROW = "| `[x]` Main grows for the call bar | MVP | the cut of `tracing` took 2 KB; the ceiling of Main rises to {} |\n"


def raised(head: dict[str, int], *plans: str) -> list[str]:
    return ceiling_raise.raise_breaches(BASE, head, list(plans) or [""])


class TheTable(unittest.TestCase):
    def test_the_literal_table_is_read(self) -> None:
        text = 'X = 1\nCEILINGS = {\n    "Main": 2_740_000,\n    # a comment\n    "Sync": 380_000,\n}\n'
        self.assertEqual(ceiling_raise.ceilings(text), BASE)

    def test_an_annotated_table_is_read(self) -> None:
        self.assertEqual(ceiling_raise.ceilings('CEILINGS: dict[str, int] = {"Main": 1}\n'), {"Main": 1})

    def test_a_computed_table_is_no_table(self) -> None:
        self.assertIsNone(ceiling_raise.ceilings("CEILINGS = dict(Main=1)\n"))

    def test_a_missing_file_is_no_table(self) -> None:
        self.assertIsNone(ceiling_raise.ceilings(None))

    def test_the_tree_holds_a_table(self) -> None:
        self.assertIn("Main", ceiling_raise.ceilings(ceiling_raise.current()) or {})


class ARaise(unittest.TestCase):
    def test_a_raise_with_no_row_fails_and_says_what_to_write(self) -> None:
        errors = raised({"Main": 2_750_000, "Sync": 380_000}, "# ROADMAP\n")
        self.assertEqual(len(errors), 1)
        self.assertIn("rose from 2,740,000 to 2,750,000", errors[0])
        self.assertIn("names the new figure, 2,750,000, and the cut tried first", errors[0])

    def test_a_raise_named_by_prose_alone_fails(self) -> None:
        self.assertTrue(raised({"Main": 2_750_000}, "The cut failed, so Main rises to 2,750,000.\n"))

    def test_a_row_naming_another_figure_fails(self) -> None:
        self.assertTrue(raised({"Main": 2_750_000}, ROW.format("2,760,000")))

    def test_a_row_naming_the_figure_and_no_cut_fails(self) -> None:
        row = "| `[x]` Main grows | MVP | the ceiling of Main rises to 2,750,000 |\n"
        self.assertTrue(raised({"Main": 2_750_000}, row))

    def test_a_row_naming_the_figure_and_the_cut_passes(self) -> None:
        for written in ["2,750,000", "2 750 000", "2_750_000", "2750000"]:
            with self.subTest(figure=written):
                self.assertEqual(raised({"Main": 2_750_000}, "", ROW.format(written)), [])

    def test_an_open_row_counts_as_a_story_row(self) -> None:
        self.assertEqual(raised({"Main": 2_750_000}, ROW.replace("[x]", "[ ]").format("2,750,000")), [])

    def test_a_figure_that_ends_a_sentence_is_named(self) -> None:
        row = "| `[x]` Main grows | MVP | no cut. The ceiling of Main rises to 2,750,000. Done |\n"
        self.assertEqual(raised({"Main": 2_750_000}, row), [])

    def test_a_decimal_names_no_figure(self) -> None:
        self.assertTrue(raised({"Sync": 390_000}, ROW.format("390,000.5")))

    def test_a_figure_inside_a_longer_number_is_not_named(self) -> None:
        self.assertTrue(raised({"Sync": 390_000}, ROW.format("1,390,000")))

    def test_each_raised_ceiling_needs_its_own_figure(self) -> None:
        errors = raised({"Main": 2_750_000, "Sync": 390_000}, ROW.format("2,750,000"))
        self.assertEqual(len(errors), 1)
        self.assertIn("Sync", errors[0])


class FreeMoves(unittest.TestCase):
    def test_a_lowered_ceiling_passes(self) -> None:
        self.assertEqual(raised({"Main": 2_700_000, "Sync": 370_000}), [])

    def test_an_unmoved_ceiling_passes(self) -> None:
        self.assertEqual(raised(dict(BASE)), [])

    def test_a_new_artefact_named_by_a_row_with_its_figure_passes(self) -> None:
        row = "| `[ ]` The Fonts module | MVP | a new artefact, the Fonts module, under 60,000 |\n"
        self.assertEqual(raised({**BASE, "Fonts module": 60_000}, row), [])

    def test_a_base_with_no_table_raises_nothing(self) -> None:
        self.assertEqual(ceiling_raise.raise_breaches(None, {"Main": 9_999_999}, [""]), [])


class ANewName(unittest.TestCase):
    def test_a_new_artefact_no_row_names_fails(self) -> None:
        errors = raised({**BASE, "Fonts module": 60_000}, ROW.format("60,000"))
        self.assertEqual(len(errors), 1)
        self.assertIn("names Fonts module and 60,000", errors[0])

    def test_a_row_naming_the_artefact_with_another_figure_fails(self) -> None:
        row = "| `[ ]` The Fonts module | MVP | a new artefact, the Fonts module, under 50,000 |\n"
        self.assertTrue(raised({**BASE, "Fonts module": 60_000}, row))

    def test_a_rename_is_compared_with_the_base_figure_of_the_old_name(self) -> None:
        base = {**BASE, "OOXML module": 340_000}
        head = {**BASE, "Office module": 400_000}
        errors = ceiling_raise.raise_breaches(base, head, ["| `[ ]` The Office module | MVP | 400,000 |\n"])
        self.assertEqual(len(errors), 1)
        self.assertIn("OOXML module, renamed Office module, ceiling rose from 340,000 to 400,000", errors[0])

    def test_a_rename_that_holds_its_figure_passes(self) -> None:
        base = {**BASE, "OOXML module": 340_000}
        self.assertEqual(ceiling_raise.raise_breaches(base, {**BASE, "Office module": 330_000}, [""]), [])

    def test_a_renamed_raise_named_with_a_cut_passes(self) -> None:
        base = {**BASE, "OOXML module": 340_000}
        head = {**BASE, "Office module": 400_000}
        self.assertEqual(ceiling_raise.raise_breaches(base, head, [ROW.format("400,000")]), [])

    def test_an_aliased_name_the_base_holds_under_neither_name_fails(self) -> None:
        row = "| `[ ]` The Office module | MVP | a new artefact, the Office module, under 400,000 |\n"
        errors = raised({**BASE, "Office module": 400_000}, row)
        self.assertEqual(len(errors), 1)
        self.assertIn("renamed artefact the base holds under neither name", errors[0])


class TheBudgets(unittest.TestCase):
    SIZE = (
        'BUDGET = {budget}\nCEILINGS = {{"Main": 2_740_000}}\n'
        'MODULE_BUDGETS = {{"PDF module": {module}}}\n'
    )

    def text(self, budget: int = 4_000_000, module: int = 600_000) -> str:
        return self.SIZE.format(budget=budget, module=module)

    def test_the_first_download_budget_is_read_as_one_entry(self) -> None:
        self.assertEqual(ceiling_raise.table(self.text(), "BUDGET"), {"First download": 4_000_000})

    def test_a_raised_first_download_budget_with_no_row_fails(self) -> None:
        errors = ceiling_raise.breaches(self.text(), self.text(budget=4_500_000), [""])
        self.assertEqual(len(errors), 1)
        self.assertIn("First download budget rose from 4,000,000 to 4,500,000", errors[0])

    def test_a_raised_module_budget_with_no_row_fails(self) -> None:
        errors = ceiling_raise.breaches(self.text(), self.text(module=700_000), [""])
        self.assertEqual(len(errors), 1)
        self.assertIn("PDF module budget rose from 600,000 to 700,000", errors[0])

    def test_a_raised_budget_named_with_a_cut_passes(self) -> None:
        plans = [ROW.format("700,000")]
        self.assertEqual(ceiling_raise.breaches(self.text(), self.text(module=700_000), plans), [])

    def test_a_lowered_budget_passes(self) -> None:
        self.assertEqual(ceiling_raise.breaches(self.text(), self.text(3_000_000, 500_000), [""]), [])

    def test_a_head_without_a_budget_table_fails(self) -> None:
        errors = ceiling_raise.breaches(self.text(), 'CEILINGS = {"Main": 1}\nBUDGET = 1\n', [""])
        self.assertEqual(errors, ["ERROR: scripts/wasm_size.py holds no literal `MODULE_BUDGETS`"])

    def test_the_tree_holds_the_three_tables(self) -> None:
        for name in ceiling_raise.TABLES:
            self.assertTrue(ceiling_raise.table(ceiling_raise.current(), name), name)


class TheRecordedRaises(unittest.TestCase):
    def test_the_brotli_raises_made_inside_feature_commits_are_named_by_a_row(self) -> None:
        # The OpenDocument stories raised the Office ceiling, and the call-devices
        # story the Main ceiling, before the gate reached `main`.
        base = {"Main": 2_740_000, "Office module": 340_000}
        head = {"Main": 2_755_000, "Office module": 400_000}
        self.assertEqual(ceiling_raise.raise_breaches(base, head, ceiling_raise.plans()), [])

    def test_the_unit_change_to_raw_bytes_is_named_by_a_row(self) -> None:
        base = {"Main": 2_755_000, "Sync": 380_000, "Office module": 400_000, "Call worker": 40_000}
        head = ceiling_raise.ceilings(ceiling_raise.current()) or {}
        self.assertEqual(ceiling_raise.raise_breaches(base, head, ceiling_raise.plans()), [])


class AMissingBase(unittest.TestCase):
    def test_a_base_that_is_not_fetched_fails_and_names_the_fetch(self) -> None:
        err = io.StringIO()
        with mock.patch.object(sys, "argv", ["ceiling_raise.py"]), redirect_stderr(err), mock.patch.object(
            ceiling_raise, "resolve", side_effect=LookupError("origin/main")
        ):
            code = ceiling_raise.main()
        self.assertEqual(code, 1)
        self.assertIn("base origin/main not found: run git fetch origin main", err.getvalue())


if __name__ == "__main__":
    unittest.main()
