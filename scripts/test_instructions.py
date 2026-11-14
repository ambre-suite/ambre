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

import contextlib
import io
import json
import sys
import time
import unittest
from pathlib import Path
from unittest import mock

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


class BuildCommand(unittest.TestCase):
    def test_every_bench_builds_under_the_fixed_key(self) -> None:
        for c in instructions.CEILINGS:
            command = instructions.build_command(c)
            self.assertIn("--features", command)
            self.assertEqual(
                command[command.index("--features") + 1], "ambre-core/fixed-hash-key"
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
        symbols = [
            s
            for c in instructions.CEILINGS
            for s in (c.symbol, *c.also, *((c.each,) if c.each else ()))
        ]
        self.assertEqual(len(symbols), len(set(symbols)))
        for s in symbols:
            self.assertFalse(
                any(s != o and s in o for o in symbols),
                f"`*{s}*` also matches another counted function",
            )


class Toggles(unittest.TestCase):
    def test_each_counted_function_is_a_toggle(self) -> None:
        two = instructions.Ceiling(
            crate="c",
            bench="b",
            operation="op",
            symbol="walk",
            row="a row",
            ceiling=1000,
            also=("step",),
        )
        self.assertEqual(
            instructions.toggles(two),
            ["--toggle-collect=*walk*", "--toggle-collect=*step*"],
        )
        self.assertEqual(instructions.toggles(CEILING), ["--toggle-collect=*sym*"])


def dump(trigger: str, count: int) -> str:
    """One `callgrind` profile, as a dump at the entry of `trigger` writes it."""
    return f"version: 1\ndesc: Trigger: --dump-before={trigger}\nevents: Ir\ntotals: {count}\n"


class Each(unittest.TestCase):
    EACH = instructions.Ceiling(
        crate="c",
        bench="b",
        operation="op",
        symbol="slice",
        row="a row",
        ceiling=1000,
        each="slice_end",
    )

    def test_each_call_is_opened_and_closed_by_a_dump(self) -> None:
        self.assertEqual(
            instructions.dumps(self.EACH),
            ["--dump-before=*slice*", "--dump-before=*slice_end*"],
        )

    def test_the_dearest_call_counts_and_the_gaps_between_do_not(self) -> None:
        profiles = [
            dump("bench::slice", 900_000),
            dump("bench::slice_end", 40),
            dump("bench::slice", 5),
            dump("bench::slice_end", 70),
            "version: 1\ndesc: Trigger: Program termination\ntotals: 800000\n",
        ]
        self.assertEqual(instructions.dearest(profiles, "slice_end"), 70)

    def test_no_closing_dump_counts_zero(self) -> None:
        self.assertEqual(instructions.dearest([dump("bench::slice", 9)], "slice_end"), 0)

    def test_the_report_keeps_the_order_of_the_ceilings(self) -> None:
        first = instructions.Ceiling("crate", "bench", "slow", "symbol", "row", 10)
        second = instructions.Ceiling("crate", "bench", "fast", "symbol", "row", 10)

        def count(exe: str, c: instructions.Ceiling) -> int:
            # The first count ends last, as a slow operation does.
            if c.operation == "slow":
                time.sleep(0.2)
            return 5

        out = io.StringIO()
        with (
            mock.patch.object(instructions, "CEILINGS", [first, second]),
            mock.patch.object(instructions, "build", lambda c: "exe"),
            mock.patch.object(instructions, "count", count),
            contextlib.redirect_stdout(out),
        ):
            self.assertEqual(instructions.main([]), 0)
        names = [line.split(":")[0] for line in out.getvalue().splitlines()]
        self.assertEqual(names, ["slow", "fast"])


def ceilings(count: int) -> list[instructions.Ceiling]:
    """`count` operations, named by their index."""
    return [instructions.Ceiling("c", "b", str(i), "sym", "row", 10) for i in range(count)]


class Shard(unittest.TestCase):
    def test_the_shards_count_each_operation_once(self) -> None:
        all_ops = ceilings(26)
        seen = [c.operation for k in range(4) for c in instructions.shard(all_ops, f"{k}/4")]
        self.assertEqual(sorted(seen, key=int), [c.operation for c in all_ops])

    def test_a_shard_takes_every_nth_operation(self) -> None:
        names = [c.operation for c in instructions.shard(ceilings(10), "1/4")]
        self.assertEqual(names, ["1", "5", "9"])

    def test_one_shard_is_the_whole_list(self) -> None:
        self.assertEqual(instructions.shard(ceilings(3), "0/1"), ceilings(3))

    def test_a_shard_past_the_count_is_refused(self) -> None:
        for spec in ("4/4", "1", "a/4", "0/0", "-1/4"):
            with self.subTest(spec=spec), self.assertRaises(ValueError):
                instructions.shard(ceilings(3), spec)

    def test_a_shard_counts_its_operations_alone(self) -> None:
        counted = []

        def count(exe: str, c: instructions.Ceiling) -> int:
            counted.append(c.operation)
            return 5

        with (
            mock.patch.object(instructions, "CEILINGS", ceilings(5)),
            mock.patch.object(instructions, "build", lambda c: "exe"),
            mock.patch.object(instructions, "count", count),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(instructions.main(["--shard", "1/2"]), 0)
        self.assertEqual(sorted(counted), ["1", "3"])

    def test_an_unknown_argument_is_refused(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(instructions.main(["--shard", "2/2"]), 2)
            self.assertEqual(instructions.main(["1/2"]), 2)


if __name__ == "__main__":
    unittest.main()
