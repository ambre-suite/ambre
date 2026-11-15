#!/usr/bin/env python3
"""The write cost per delta of the relay, held under its ceiling.

The relay writes three rows of work into Postgres per accepted delta: the
insert of its index row, the move to a batch object, and the delete by the
trim. Each write puts bytes in the write-ahead log (WAL), and the WAL bytes
per delta set how many writers a database node holds
(`ambre-spec-budgets.md section 3`).

**A count of bytes, not a time.** A time on a shared runner moves by tens of
percent, and the disk sets it. The short run of
`ambre_server::hot_tail::write_cost::write_cost_short_run` writes the same
statements on every machine instead: unpaced, on one connection, compaction
every 500 deltas, autovacuum off. It reads the WAL each step wrote through
`pg_walinspect`, which no disk changes. It prints one line, a JSON object
after `WRITE_COST `, and this script holds that line.

**What still moves.** A run moves by a few bytes per delta, 0.4 % to 1.4 %,
for a cause not known yet. `MARGIN` covers that spread, measured on the day
the ceiling was set. The random key of each row of `updates` was not the
cause: the spread stayed the same once the key went.

**A ceiling moves down.** The stories on `updates` each lower the WAL per
delta, and each one lowers `CEILING` to its new figure plus `MARGIN`, in the
same change. A raise needs a story row that names the new figure and its
cause, as the budget row states.

**It runs alone.** The short run reads the WAL of the whole cluster, so the
script runs it as the one test of its process, and never beside the db tier.

Run: `just write-cost`, services up. `--runs N` repeats the run and holds the
dearest, to measure a change: the stories that lower the cost read it there.
"""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The most WAL the steps may write per delta, in bytes: the figure of the
# budget row, which `test_write_cost.py` checks the row names.
CEILING = 2_280

# The room a ceiling leaves above the dearest run it was set from, as a
# fraction. Ten runs of 2026-11-14 read 2 632 B to 2 643 B per delta, a spread
# of 0.41 %. The CI `test-db` job read 2 676 B the same day, 1.2 % over the
# dearest local run, for a cause not known yet. On 2026-11-15, with `updates`
# keyed by document and number and two snapshots kept, five runs read
# 2 197 B to 2 228 B, and the CI job of two snapshots kept read inside the
# local spread. The ceiling is that dearest run plus the margin. A new index
# on `updates` costs more than the margin.
MARGIN = 0.02

TEST = "hot_tail::write_cost::write_cost_short_run"
PREFIX = "WRITE_COST "

# The build of the db tier, so the step reuses what `just test-db` compiled.
COMMAND = [
    "cargo",
    "test",
    "-p",
    "ambre-server",
    "--features",
    "db,test-helpers",
    "--bin",
    "ambre-server",
    "--",
    "--ignored",
    "--exact",
    TEST,
    "--nocapture",
]


@dataclass(frozen=True)
class Figures:
    """What one short run measured."""

    deltas: int
    wal_bytes_per_delta: float
    fpi_bytes_per_delta: float
    steps: dict[str, float]


def parse(output: str) -> Figures | None:
    """The figures of the last summary line of `output`, or None when it has none.

    A line that is no JSON object of the four fields is no summary.
    """
    figures = None
    for line in output.splitlines():
        if not line.startswith(PREFIX):
            continue
        try:
            raw = json.loads(line[len(PREFIX) :])
            figures = Figures(
                deltas=int(raw["deltas"]),
                wal_bytes_per_delta=float(raw["wal_bytes_per_delta"]),
                fpi_bytes_per_delta=float(raw["fpi_bytes_per_delta"]),
                steps={str(k): float(v) for k, v in raw["steps"].items()},
            )
        except (ValueError, KeyError, TypeError, AttributeError):
            continue
    return figures


def verdict(figures: Figures, ceiling: int) -> str | None:
    """Why `figures` fail `ceiling`, or None when they hold it.

    A run of no delta, or of no WAL, measured nothing: it fails like an overrun.
    """
    if figures.deltas <= 0 or figures.wal_bytes_per_delta <= 0:
        return f"the run measured nothing: {figures.deltas} deltas, {figures.wal_bytes_per_delta:.0f} B/delta"
    if figures.wal_bytes_per_delta > ceiling:
        return f"{figures.wal_bytes_per_delta:.0f} B of WAL per delta, over the ceiling of {ceiling} B"
    return None


def lowered(figure: float) -> int:
    """The ceiling a story sets from its dearest run: `figure` plus `MARGIN`,
    rounded up to ten bytes."""
    return math.ceil(figure * (1 + MARGIN) / 10) * 10


def run_once() -> Figures | None:
    """Runs the short run once, prints its report, and returns its figures."""
    done = subprocess.run(COMMAND, cwd=ROOT, capture_output=True, text=True, check=False)
    sys.stdout.write(done.stdout)
    sys.stderr.write(done.stderr)
    if done.returncode != 0:
        return None
    return parse(done.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--runs", type=int, default=1, help="repeat the run and hold the dearest")
    args = parser.parse_args(argv)

    measured: list[Figures] = []
    for i in range(max(args.runs, 1)):
        figures = run_once()
        if figures is None:
            print(f"write-cost: run {i + 1} failed or printed no `{PREFIX.strip()}` line", file=sys.stderr)
            return 1
        measured.append(figures)

    print()
    for i, f in enumerate(measured):
        steps = ", ".join(f"{k} {v:.0f}" for k, v in sorted(f.steps.items()))
        print(f"run {i + 1}: {f.wal_bytes_per_delta:.1f} B/delta, of which {f.fpi_bytes_per_delta:.1f} B of full-page images ({steps})")
    dearest = max(measured, key=lambda f: f.wal_bytes_per_delta)
    if len(measured) > 1:
        low = min(f.wal_bytes_per_delta for f in measured)
        high = dearest.wal_bytes_per_delta
        print(f"spread: {low:.1f} to {high:.1f} B/delta, {(high - low) / low * 100:.2f} %")
    failure = verdict(dearest, CEILING)
    if failure:
        print(f"write-cost: FAIL, {failure}", file=sys.stderr)
        return 1
    print(f"write-cost: {dearest.wal_bytes_per_delta:.0f} B/delta, ceiling {CEILING} B")
    if lowered(dearest.wal_bytes_per_delta) < CEILING:
        print(f"write-cost: the ceiling may fall to {lowered(dearest.wal_bytes_per_delta)} B, the dearest run plus the margin")
    return 0


if __name__ == "__main__":
    sys.exit(main())
