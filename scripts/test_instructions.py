#!/usr/bin/env python3
"""What `instructions.py` reads, and what it refuses.

The check rests on reading two outputs it does not control: the JSON messages
of `cargo bench --no-run`, and a `callgrind` profile. Both readers are tested
on text alone. So is the verdict, which must fail an overrun and must fail a
count of zero, the sign of a function renamed or inlined. Running `valgrind`
itself is left to `just instructions`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import instructions  # noqa: E402  -- the module under test


def artifact(name: str, kind: str, exe: str | None) -> str:
    """One `compiler-artifact` message, as cargo prints it."""
    return json.dumps(
        {
            "reason": "compiler-artifact",
            "target": {"name": name, "kind": [kind]},
            "executable": exe,
        }
    )


CEILING = instructions.Ceiling(
    crate="c",
    bench="b",
    operation="op",
    symbol="sym",
    row="a row",
    ceiling=1000,
)


class Executable(unittest.TestCase):
    def test_the_bench_binary_is_found_among_the_artifacts(self) -> None:
        messages = "\n".join(
            [
                artifact("ambre_toile", "lib", None),
                "not json at all",
                artifact("board_cost", "bench", "/t/board_cost-1"),
                artifact("instructions", "bench", "/t/instructions-2"),
                json.dumps({"reason": "build-finished", "success": True}),
            ]
        )
        self.assertEqual(
            instructions.executable(messages, "instructions"), "/t/instructions-2"
        )

    def test_a_library_of_the_same_name_is_not_the_bench(self) -> None:
        messages = artifact("instructions", "lib", "/t/libinstructions")
        self.assertIsNone(instructions.executable(messages, "instructions"))


class Total(unittest.TestCase):
    def test_totals_is_read(self) -> None:
        profile = "version: 1\nevents: Ir\nfn=main\n12 34\nsummary: 999\ntotals: 686451\n"
        self.assertEqual(instructions.total(profile), 686451)

    def test_summary_is_read_when_totals_is_missing(self) -> None:
        self.assertEqual(instructions.total("events: Ir\nsummary: 42\n"), 42)

    def test_a_profile_with_no_total_reads_zero(self) -> None:
        self.assertEqual(instructions.total("events: Ir\n"), 0)


class Verdict(unittest.TestCase):
    def test_a_count_under_the_ceiling_holds(self) -> None:
        self.assertIsNone(instructions.verdict(CEILING, 999))
        self.assertIsNone(instructions.verdict(CEILING, 1000))

    def test_a_count_over_the_ceiling_fails_and_names_the_row(self) -> None:
        why = instructions.verdict(CEILING, 1001)
        self.assertIsNotNone(why)
        self.assertIn("a row", why or "")

    def test_a_count_of_zero_fails(self) -> None:
        why = instructions.verdict(CEILING, 0)
        self.assertIsNotNone(why)
        self.assertIn("sym", why or "")


class Ceilings(unittest.TestCase):
    def test_each_operation_is_counted_in_a_function_of_its_own(self) -> None:
        symbols = [c.symbol for c in instructions.CEILINGS]
        self.assertEqual(len(symbols), len(set(symbols)))
        for c in instructions.CEILINGS:
            self.assertFalse(
                any(c.symbol != o and c.symbol in o for o in symbols),
                f"`*{c.symbol}*` also matches another operation's function",
            )


if __name__ == "__main__":
    unittest.main()
