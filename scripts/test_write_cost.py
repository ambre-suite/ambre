#!/usr/bin/env python3
"""What `write_cost.py` reads, and what it refuses.

The check reads one line the Rust short run prints, and holds its figure under
a ceiling the budget register states. The reader and the verdict are tested on
text alone. So is the link between the ceiling and its budget row: a ceiling
that moves without its row, or a row that moves without its ceiling, fails.
Running the short run itself is left to `just write-cost`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import contextlib
import io
import re
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import write_cost  # noqa: E402  -- the module under test

LINE = (
    'WRITE_COST {"deltas":4000,"fpi_bytes_per_delta":508.5,'
    '"steps":{"delete":307.5,"insert":1507.2,"move":817.7,"snapshot":4.3},'
    '"wal_bytes_per_delta":2636.7}'
)


def figures(wal: float, deltas: int = 4000) -> write_cost.Figures:
    return write_cost.Figures(deltas=deltas, wal_bytes_per_delta=wal, fpi_bytes_per_delta=0.0, steps={})


class Parse(unittest.TestCase):
    def test_the_summary_line_among_the_report_is_read(self) -> None:
        got = write_cost.parse(f"warm-up: 23000 deltas\n{LINE}\ntest ... ok\n")
        self.assertIsNotNone(got)
        assert got is not None
        self.assertEqual(got.deltas, 4000)
        self.assertAlmostEqual(got.wal_bytes_per_delta, 2636.7)
        self.assertAlmostEqual(got.steps["insert"], 1507.2)

    def test_an_output_without_the_line_reads_as_nothing(self) -> None:
        self.assertIsNone(write_cost.parse("test result: ok. 1 passed\n"))

    def test_a_line_that_is_no_object_of_the_fields_is_no_summary(self) -> None:
        self.assertIsNone(write_cost.parse('WRITE_COST {"deltas":4000}\n'))
        self.assertIsNone(write_cost.parse("WRITE_COST not json\n"))
        self.assertIsNone(write_cost.parse("WRITE_COST [1, 2]\n"))

    def test_the_last_summary_line_wins(self) -> None:
        later = LINE.replace("2636.7", "2700.0")
        got = write_cost.parse(f"{LINE}\n{later}\n")
        assert got is not None
        self.assertAlmostEqual(got.wal_bytes_per_delta, 2700.0)


class Verdict(unittest.TestCase):
    def test_a_figure_under_the_ceiling_holds(self) -> None:
        self.assertIsNone(write_cost.verdict(figures(2600.0), 2700))

    def test_a_figure_at_the_ceiling_holds(self) -> None:
        self.assertIsNone(write_cost.verdict(figures(2700.0), 2700))

    def test_a_figure_over_the_ceiling_fails(self) -> None:
        failure = write_cost.verdict(figures(2700.5), 2700)
        self.assertIsNotNone(failure)
        self.assertIn("over the ceiling", failure or "")

    def test_a_run_that_measured_nothing_fails(self) -> None:
        self.assertIsNotNone(write_cost.verdict(figures(0.0), 2700))
        self.assertIsNotNone(write_cost.verdict(figures(2600.0, deltas=0), 2700))


class Lowered(unittest.TestCase):
    def test_the_ceiling_of_a_figure_is_the_margin_above_it_rounded_up(self) -> None:
        with mock.patch.object(write_cost, "MARGIN", 0.02):
            self.assertEqual(write_cost.lowered(2643.1), 2700)
            self.assertEqual(write_cost.lowered(2000.0), 2040)


class Main(unittest.TestCase):
    def run_main(self, runs: list[write_cost.Figures | None], argv: list[str]) -> int:
        with (
            mock.patch.object(write_cost, "run_once", side_effect=runs),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            return write_cost.main(argv)

    def test_the_dearest_of_several_runs_is_held(self) -> None:
        under = figures(write_cost.CEILING - 10.0)
        over = figures(write_cost.CEILING + 1.0)
        self.assertEqual(self.run_main([under, under], ["--runs", "2"]), 0)
        self.assertEqual(self.run_main([under, over], ["--runs", "2"]), 1)

    def test_a_run_without_figures_fails(self) -> None:
        self.assertEqual(self.run_main([None], []), 1)

    def test_the_command_runs_the_short_run_alone(self) -> None:
        # The run reads the WAL of the whole cluster: one exact test, no other.
        self.assertIn("--exact", write_cost.COMMAND)
        self.assertIn("--ignored", write_cost.COMMAND)
        self.assertIn(write_cost.TEST, write_cost.COMMAND)


class BudgetRow(unittest.TestCase):
    """The ceiling and the row of the budget register name one figure."""

    BUDGETS = write_cost.ROOT / "specs/1-Foundations/ambre-spec-budgets.md"

    def row(self) -> str:
        rows = [line for line in self.BUDGETS.read_text().splitlines() if line.startswith("| Delta index write cost")]
        self.assertEqual(len(rows), 1, "one row of the budget register holds the write cost per delta")
        return rows[0]

    def test_the_row_states_the_ceiling(self) -> None:
        cells = self.row().split("|")
        target = cells[2]
        figure = re.sub(r"(?<=\d)[ ,_](?=\d{3}\b)", "", target)
        self.assertIn(f"{write_cost.CEILING} B", figure, "the Target cell names the ceiling of `write_cost.py`")

    def test_the_row_names_the_mechanism(self) -> None:
        row = self.row()
        self.assertIn("write_cost_short_run", row)
        self.assertIn("just write-cost", row)

    def test_the_rust_test_prints_the_prefix_the_script_reads(self) -> None:
        source = (write_cost.ROOT / "crates/ambre-server/src/hot_tail/write_cost.rs").read_text()
        self.assertIn(f'const SUMMARY_PREFIX: &str = "{write_cost.PREFIX}";', source)
        self.assertIn(f"async fn {write_cost.TEST.rsplit('::', 1)[1]}(", source)


if __name__ == "__main__":
    unittest.main()
