#!/usr/bin/env python3
"""The budgets held by a count of instructions, each against its ceiling.

A budget in time measured on a shared CI runner moves by tens of percent
between two runs, so a threshold on it fails a build on noise. `callgrind`
counts the instructions an operation retires instead, and that count moves only
when the code does (`ambre-spec-testing.md section 2`).

**One operation, one function.** Each bench binary named in `CEILINGS` runs an
operation inside a function that is never inlined and whose name is unique. The
binary takes the operation's name as its argument and runs only that one, and
`--toggle-collect` counts only inside its function. The setup of the operation
therefore costs nothing. An operation whose budget covers two phases of one
command names a function for each, `symbol` and `also`: the count is their sum,
and what runs between them falls outside it.

**A ceiling is a budget converted, not a measure recorded.** It is the row's
target times `PER_MS`, the instructions of the native build that fit in one
millisecond of the phone the budgets are written against. The measure sits
below it, and the gap is the margin a change may spend. A ceiling raised to fit
a slower build is a budget missed.

**A count of zero proves nothing**: it means the function was renamed or
inlined, and the operation was never counted. It fails like an overrun.

Run: `just instructions`. Needs `valgrind` on the path.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Instructions of the native x86-64 build per millisecond of the phone the
# budgets target. A desktop core retires about 5.8 per nanosecond on the Toile
# body commit, and `ambre-spec-budgets.md` takes a phone to be about 7 times
# slower: 830 000, rounded down.
PER_MS = 800_000


@dataclass(frozen=True)
class Ceiling:
    """One operation, the budget row it holds, and the most it may cost."""

    crate: str
    bench: str
    operation: str
    # The function `callgrind` counts in, matched as `*symbol*`.
    symbol: str
    row: str
    ceiling: int
    # Further functions counted with `symbol`, each matched the same way. None
    # of them may call another, or the toggle turns the count off inside it.
    also: tuple[str, ...] = ()


CEILINGS = [
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="body_commit",
        symbol="ambre_budget_toile_body_commit",
        row="Toile body commit, a 4 KB body merged from its base (< 1 ms)",
        ceiling=1 * PER_MS,
    ),
    # Not a budget: the most a close costs once both diffs run to their cap.
    # It holds the cap where it is, so a raised cap fails here and not on a
    # phone.
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="body_commit_at_cap",
        symbol="ambre_bound_toile_body_commit",
        row="Toile body commit, both diffs at their cap (the bound, not a budget)",
        ceiling=6_500_000,
    ),
    # The walk of the busiest frame. Tessellation and the GPU submit fall
    # outside the count, so the walk is held to the frame's typical total.
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="board_paint",
        symbol="ambre_budget_toile_board_paint",
        row="Toile board paint, 2 000 visible elements, its content walk (< 5 ms of the 16 ms)",
        ceiling=5 * PER_MS,
    ),
    # The walk and the engine step of one structural edit, the two phases its
    # budget row covers. The purge, the commit and the caches of positions,
    # merges, styles and sizes fall outside the count.
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="structural_edit",
        symbol="reanchor_formulas",
        row="Trame structural edit, 10k formula cells, its walk and its engine step (< 10 ms)",
        ceiling=10 * PER_MS,
        also=("carry_engine",),
    ),
    # One row delete from its command to its outcome: the purge, the reads of
    # the lists, the walk, the commit, the engine step and every cache.
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete",
        symbol="ambre_budget_trame_structural_edit",
        row="Trame whole line delete, 10k formula cells, one DeleteRow (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    # The undo of that delete, and its redo, each held to the same budget: a
    # history step of a structural edit is one more structural edit. Only the
    # tail after the step of Loro's undo manager is counted: that step alone
    # costs about 300 million (`ambre-spec-budgets.md`, the whole line delete
    # row), and no code of this workspace runs inside it.
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete_undo",
        symbol="after_undo",
        row="Trame whole line delete, 10k formula cells, the tail of its undo (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete_redo",
        symbol="after_redo",
        row="Trame whole line delete, 10k formula cells, the tail of its redo (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    # One remote batch from its import to its events: every cache, the
    # engine step and the layout derived from the list diffs.
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="remote_batch",
        symbol="ambre_budget_trame_remote_batch",
        row="Trame remote batch, 10k formula cells, a cell write and a row delete (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="paragraph_shape",
        symbol="ambre_budget_prose_paragraph_shape",
        row="Text layout (Parley, dirty paragraph), a key moving every break of 500 characters (< 2 ms)",
        ceiling=2 * PER_MS,
    ),
]


def executable(messages: str, bench: str) -> str | None:
    """The bench binary cargo built, read from its JSON messages."""
    for line in messages.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = msg.get("target", {})
        if (
            msg.get("reason") == "compiler-artifact"
            and msg.get("executable")
            and target.get("name") == bench
            and "bench" in target.get("kind", [])
        ):
            return msg["executable"]
    return None


def total(profile: str) -> int:
    """The instructions a `callgrind` profile counted, or 0 when it holds no
    total. `totals:` is the sum of every event line; `summary:` is the older
    spelling of the same figure."""
    for prefix in ("totals:", "summary:"):
        for line in profile.splitlines():
            if line.startswith(prefix):
                fields = line.split()
                if len(fields) > 1 and fields[1].isdigit():
                    return int(fields[1])
    return 0


def verdict(c: Ceiling, count: int) -> str | None:
    """Why `count` fails `c`, or `None` when it holds."""
    if count == 0:
        return f"{c.operation}: nothing counted in `{c.symbol}`; renamed or inlined?"
    if count > c.ceiling:
        return f"{c.operation}: {count:,} instructions, over the ceiling of {c.ceiling:,} ({c.row})"
    return None


def build(c: Ceiling) -> str:
    """Build the bench of `c` and return its binary."""
    out = subprocess.run(
        [
            "cargo",
            "bench",
            "-p",
            c.crate,
            "--bench",
            c.bench,
            "--no-run",
            "--message-format=json",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    exe = executable(out.stdout, c.bench)
    if exe is None:
        raise SystemExit(f"cargo built no binary for bench `{c.bench}` of `{c.crate}`")
    return exe


def toggles(c: Ceiling) -> list[str]:
    """One `--toggle-collect` per function `c` counts in."""
    return [f"--toggle-collect=*{s}*" for s in (c.symbol, *c.also)]


def count(exe: str, c: Ceiling) -> int:
    """Run one operation under `callgrind` and return what it counted."""
    with tempfile.TemporaryDirectory() as tmp:
        profile = Path(tmp) / "callgrind.out"
        subprocess.run(
            [
                "valgrind",
                "--tool=callgrind",
                *toggles(c),
                f"--callgrind-out-file={profile}",
                exe,
                c.operation,
            ],
            check=True,
            capture_output=True,
        )
        return total(profile.read_text())


def main() -> int:
    failures = []
    binaries: dict[tuple[str, str], str] = {}
    for c in CEILINGS:
        key = (c.crate, c.bench)
        if key not in binaries:
            binaries[key] = build(c)
        n = count(binaries[key], c)
        share = n * 100 // c.ceiling
        print(f"{c.operation}: {n:,} of {c.ceiling:,} instructions ({share}%)")
        why = verdict(c, n)
        if why is not None:
            failures.append(why)
    for why in failures:
        print(f"FAIL {why}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
