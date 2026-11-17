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

**One operation, each call apart.** A budget may hold each slice of a job
whose work a slice cuts unevenly, such as a page of a PDF export: some slices
shape, one draws. Such an operation names `each`, a function it calls right
after each call of `symbol`. `callgrind` then dumps its count at the entry of
both, and the dump at the entry of `each` holds one call alone. The count is
the dearest call. `--toggle-collect` stops every dump, so this mode counts the
whole run and keeps the dumps it needs.

**A ceiling is a budget converted, not a measure recorded.** It is the row's
target times `PER_MS`, the instructions of the native build that fit in one
millisecond of the phone the budgets are written against. The measure sits
below it, and the gap is the margin a change may spend. A ceiling raised to fit
a slower build is a budget missed.

**A count of zero proves nothing**: it means the function was renamed or
inlined, and the operation was never counted. It fails like an overrun.

**A count repeats exactly.** The hash tables of the applications hash under
a key drawn per process, and a random key moves the probes of each lookup, so
it moves the count from run to run. The bench build therefore turns on the
`fixed-hash-key` feature of `ambre-core`, and only this build does
(`ARCH-089`). Two runs then count the same total, and a ceiling that is no
budget sits 0.5 % above the count (`ambre-spec-testing.md section 2`). A
bench also pins the Loro peer of each document. Two counts still move from a
dependency's own hasher, and both hold a budget far above them.

**The counts run side by side.** Each operation sets its document up under
`callgrind` too, about 125 times slower than natively: one Trame operation
takes about six minutes, for a count of a few milliseconds. So the script builds
each bench binary first, then runs one `callgrind` per processor. A count does
not depend on the load of the machine, and the report keeps the order of
`CEILINGS`.

**The counts split across runners.** `--shard k/n` keeps the operations
whose index in `CEILINGS` leaves `k` when divided by `n`, so the `n` shards
together count each operation once. The Trame operations, the dearest, sit
next to each other in `CEILINGS`, so each shard takes its share of them. The
CI `instructions` job runs one shard per runner, as the `mutants` job does.

Run: `just instructions`, or `just instructions 1/4` for one shard. Needs
`valgrind` on the path.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
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
    # The function the operation calls right after each call of `symbol`.
    # Set, the count is the dearest call, not the sum.
    each: str | None = None
    # Further features the bench builds with, beside the fixed key, for an
    # operation whose code a feature gates.
    features: tuple[str, ...] = ()


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
    # Each page of the export of a deck of 50 slides, counted apart: what an
    # edit of the board waits behind while the export runs (`TOILE-IO-099`).
    # The pages are recorded under the `pdf` feature alone.
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="deck_page",
        symbol="ambre_budget_toile_deck_page",
        row="Toile PDF export, one slice: the recording of one slide (< 4 ms)",
        ceiling=4 * PER_MS,
        each="ambre_pass_end_toile_deck_page",
        features=("ambre-toile/pdf",),
    ),
    # The pass of the request of that export, whole: the slides, the
    # pictures and the version read off the board, then the first page
    # framed, derived and recorded. It derives one page, not the deck (`TOILE-IO-099`).
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="deck_request",
        symbol="ambre_budget_toile_deck_request",
        row="Toile PDF export, one slice: the pass of the request (< 4 ms)",
        ceiling=4 * PER_MS,
        features=("ambre-toile/pdf",),
    ),
    # The same pass for a deck of 200 slides. The request reads no element to
    # list its pictures, and frames each page in the step that records it.
    # The ceiling is the count reached on 2026-11-17 under the fixed key,
    # plus 0.5 % rounded up to the next 10 000: 1 714 580, against 1 516 899
    # for 50 slides. A request that reads each element again fails here. It
    # only moves down (`TOILE-IO-099`).
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="deck_request_large",
        symbol="ambre_budget_toile_deck_request",
        row="Toile PDF export, one slice: the pass of the request, 200 slides (< 4 ms, the ceiling reached)",
        ceiling=1_730_000,
        features=("ambre-toile/pdf",),
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
    # The undo of that delete, and its redo, each held whole to the same
    # budget: a history step of a structural edit is one more structural
    # edit. The delete records its own inverse, so neither step goes through
    # the checkout of Loro's undo manager (`TRAME-286`).
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete_undo",
        symbol="ambre_budget_trame_line_delete_undo",
        row="Trame whole line delete, 10k formula cells, its whole undo (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete_redo",
        symbol="ambre_budget_trame_line_delete_redo",
        row="Trame whole line delete, 10k formula cells, its whole redo (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    # The undo and the redo of a row insert, held to the same budget: every
    # edit records its own inverse (`TRAME-291`).
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_insert_undo",
        symbol="ambre_budget_trame_line_insert_undo",
        row="Trame whole line delete, 10k formula cells, the whole undo of a row insert (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_insert_redo",
        symbol="ambre_budget_trame_line_insert_redo",
        row="Trame whole line delete, 10k formula cells, the whole redo of a row insert (< 10 ms)",
        ceiling=10 * PER_MS,
    ),
    # The delete of a block of five hundred rows, its undo and its redo. The
    # 16 ms of the row is 12 800 000, and each count sits below it since the
    # delete hides its lines (`TRAME-297`, `TRAME-299`). Each ceiling is the
    # count reached on 2026-11-12 under the fixed key, plus 0.5 % rounded up to
    # the next 10 000, so a regression fails before it spends the margin. It
    # only moves down (`TRAME-296`, `TRAME-298`).
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="many_line_delete",
        symbol="ambre_budget_trame_block_remove",
        row="Trame delete of a block of 500 rows, 10k formula cells (< 16 ms, the ceiling reached)",
        ceiling=9_160_000,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="many_line_delete_undo",
        symbol="ambre_budget_trame_block_undo",
        row="Trame delete of a block of 500 rows, its whole undo (< 16 ms, the ceiling reached)",
        ceiling=10_390_000,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="many_line_delete_redo",
        symbol="ambre_budget_trame_block_redo",
        row="Trame delete of a block of 500 rows, its whole redo (< 16 ms, the ceiling reached)",
        ceiling=6_540_000,
    ),
    # A line delete while a block of 500 rows stays hidden: the axis view
    # steps over each hidden slot, so it holds the line delete row
    # (`TRAME-308`).
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="line_delete_after_block",
        symbol="ambre_budget_trame_line_delete_after_block",
        row="Trame whole line delete, 10k formula cells, while 500 rows stay hidden (< 10 ms)",
        ceiling=8_000_000,
    ),
    # One slice of the sweep of 500 rows an ended session hid, held to the
    # structural edit row: it runs between two messages of the Logic thread
    # (`TRAME-307`).
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="many_line_sweep_slice",
        symbol="ambre_budget_trame_sweep_slice",
        row="Trame structural edit, 10k formula cells, one slice of a sweep (< 10 ms)",
        ceiling=8_000_000,
    ),
    # The undo and the redo of one cell write, held to the command row of a
    # cell write: no undo manager checks out the cell map any more.
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="cell_write_undo",
        symbol="ambre_budget_trame_cell_write_undo",
        row="Trame SetCellText (P2), 10k formula cells, the whole undo of one cell write (< 5 ms)",
        ceiling=5 * PER_MS,
    ),
    Ceiling(
        crate="ambre-trame",
        bench="instructions",
        operation="cell_write_redo",
        symbol="ambre_budget_trame_cell_write_redo",
        row="Trame SetCellText (P2), 10k formula cells, the whole redo of one cell write (< 5 ms)",
        ceiling=5 * PER_MS,
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
    # The undo and the redo of one move on a board of 2 000 stickies, each
    # from its command to its outcome. An undo refreshes only the caches of
    # what it wrote (`TOILE-292`), so both hold the < 1 ms of the row
    # (`TOILE-291`): 419 225 and 415 173 on 2026-11-10, on foldhash.
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="move_undo",
        symbol="ambre_budget_toile_move_undo",
        row="Toile transform command, the whole undo of a move on 2 000 elements (< 1 ms)",
        ceiling=1 * PER_MS,
    ),
    Ceiling(
        crate="ambre-toile",
        bench="instructions",
        operation="move_redo",
        symbol="ambre_budget_toile_move_redo",
        row="Toile transform command, the whole redo of a move on 2 000 elements (< 1 ms)",
        ceiling=1 * PER_MS,
    ),
    # The undo and the redo of one typed key in a document of about a hundred
    # pages, each from its command to its outcome (`PROSE-135`).
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="key_undo",
        symbol="ambre_budget_prose_key_undo",
        row="Prose local text insert, the whole undo of a key on 100 pages (< 2 ms)",
        ceiling=2 * PER_MS,
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="key_redo",
        symbol="ambre_budget_prose_key_redo",
        row="Prose local text insert, the whole redo of a key on 100 pages (< 2 ms)",
        ceiling=2 * PER_MS,
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="paragraph_shape",
        symbol="ambre_budget_prose_paragraph_shape",
        row="Text layout (Parley, dirty paragraph), a key moving every break of 500 characters (< 2 ms)",
        ceiling=2 * PER_MS,
    ),
    # One slice of a running Prose import, and one of a running export: what
    # a key typed in another document waits behind (`PROSE-IO-099`).
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="import_slice",
        symbol="ambre_budget_prose_import_slice",
        row="Prose import or export, one slice: the write of one import slice (< 4 ms)",
        ceiling=4 * PER_MS,
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="export_slice",
        symbol="ambre_budget_prose_export_slice",
        row="Prose import or export, one slice: the walk of one export slice (< 4 ms)",
        ceiling=4 * PER_MS,
    ),
    # Each slice of the drawing of a near page and of a far page of a Prose
    # PDF export (`PROSE-IO-103`): the slices that shape runs, and the one
    # that draws the page.
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="print_slice",
        symbol="ambre_budget_prose_print_slice",
        row="Prose PDF export, one slice: the shaping of runs or the drawing of one page (< 4 ms)",
        ceiling=4 * PER_MS,
        each="ambre_pass_end_prose_print",
    ),
    # The same slices with pictures on the first pages, and over pages dense
    # with text and pictures: a style change every few words and a picture
    # after every third block. Each holds the same row, and each ceiling is
    # the count reached on 2026-11-14 under the fixed key, plus 0.5 % rounded
    # up to the next 10 000, so a regression fails before it spends the
    # margin. It only moves down (`PROSE-IO-103`).
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="print_slice_picture",
        symbol="ambre_budget_prose_picture_print",
        row="Prose PDF export, one slice, pictures on the first pages (< 4 ms, the ceiling reached)",
        ceiling=2_390_000,
        each="ambre_pass_end_prose_picture",
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="print_slice_dense",
        symbol="ambre_budget_prose_dense_print",
        row="Prose PDF export, one slice, a page dense with text and pictures (< 4 ms, the ceiling reached)",
        ceiling=2_850_000,
        each="ambre_pass_end_prose_dense",
    ),
    # The Logic thread's half of a JPEG picture that arrives: the header, and
    # the decode handed to the converter thread (`PROSE-FEAT-330`).
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="picture_arrive",
        symbol="ambre_budget_prose_picture_arrive",
        row="Prose picture decode, one picture: the Logic thread's half of a JPEG (< 4 ms)",
        ceiling=4 * PER_MS,
    ),
    # The same half for a PNG and a GIF, whose decode leaves for the
    # converter thread too (`PROSE-FEAT-332`).
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="picture_arrive_png",
        symbol="ambre_budget_prose_png_arrive",
        row="Prose picture decode, one picture: the Logic thread's half of a PNG (< 4 ms)",
        ceiling=4 * PER_MS,
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="picture_arrive_gif",
        symbol="ambre_budget_prose_gif_arrive",
        row="Prose picture decode, one picture: the Logic thread's half of a GIF (< 4 ms)",
        ceiling=4 * PER_MS,
    ),
    # The dearest module call of each converter family on the largest input
    # it admits, held to the answer deadline of the family divided by its
    # margin of 3 (`ARCH-127`, `ambre-spec-budgets.md section 8`). Each run
    # takes one to two minutes under `callgrind`.
    Ceiling(
        crate="ambre-convert",
        bench="instructions",
        operation="images_largest",
        symbol="ambre_deadline_images",
        row="Answer deadline of the images family, a JPEG at the source bound (45 s / 3)",
        ceiling=45_000 * PER_MS // 3,
        features=("ambre-convert/images", "ambre-convert/office", "ambre-convert/markup", "ambre-convert/tables"),
    ),
    Ceiling(
        crate="ambre-convert",
        bench="instructions",
        operation="sheet_write_largest",
        symbol="ambre_deadline_sheet",
        row="Answer deadline of the Office family, a sheet at the part bound (40 s / 3)",
        ceiling=40_000 * PER_MS // 3,
        features=("ambre-convert/images", "ambre-convert/office", "ambre-convert/markup", "ambre-convert/tables"),
    ),
    Ceiling(
        crate="ambre-convert",
        bench="instructions",
        operation="html_largest",
        symbol="ambre_deadline_markup",
        row="Answer deadline of the markup family, an HTML file at the import bound (30 s / 3)",
        ceiling=30_000 * PER_MS // 3,
        features=("ambre-convert/images", "ambre-convert/office", "ambre-convert/markup", "ambre-convert/tables"),
    ),
    Ceiling(
        crate="ambre-convert",
        bench="instructions",
        operation="csv_read_largest",
        symbol="ambre_deadline_tables",
        row="Answer deadline of the tables family, a CSV file at the import bound (45 s / 3)",
        ceiling=45_000 * PER_MS // 3,
        features=("ambre-convert/images", "ambre-convert/office", "ambre-convert/markup", "ambre-convert/tables"),
    ),
    Ceiling(
        crate="ambre-prose",
        bench="instructions",
        operation="pdf_write_largest",
        symbol="ambre_pdf::document::write_document",
        row="Answer deadline of the PDF family, the pictures an export admits (35 s / 3)",
        ceiling=35_000 * PER_MS // 3,
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


# The feature that swaps the key of every hash table for a fixed one, so a
# count repeats. A shipped build never names it (`ARCH-089`).
FIXED_KEY = "ambre-core/fixed-hash-key"


def build_command(c: Ceiling) -> list[str]:
    """The cargo command that builds the bench of `c`, under the fixed key."""
    return [
        "cargo",
        "bench",
        "-p",
        c.crate,
        "--bench",
        c.bench,
        "--features",
        ",".join((FIXED_KEY, *c.features)),
        "--no-run",
        "--message-format=json",
    ]


def binary_key(c: Ceiling) -> tuple[str, str, tuple[str, ...]]:
    """The build an operation runs in: two operations of one bench share it
    only when they build with the same features."""
    return (c.crate, c.bench, c.features)


def build(c: Ceiling) -> str:
    """Build the bench of `c` and return its binary."""
    out = subprocess.run(
        build_command(c),
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


def dumps(c: Ceiling) -> list[str]:
    """One `--dump-before` per function that opens or closes a call of `c`."""
    return [f"--dump-before=*{s}*" for s in (c.symbol, c.each) if s is not None]


def dearest(profiles: list[str], each: str) -> int:
    """The largest total among the profiles a dump at the entry of `each`
    wrote, or 0 when none did. Each such profile holds what ran since the dump
    at the entry of the counted function: one call."""
    closing = [
        p
        for p in profiles
        if any(
            line.startswith("desc: Trigger: --dump-before=") and each in line
            for line in p.splitlines()
        )
    ]
    return max((total(p) for p in closing), default=0)


def count(exe: str, c: Ceiling) -> int:
    """Run one operation under `callgrind` and return what it counted."""
    if c.each is not None:
        return count_each(exe, c, c.each)
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


def count_each(exe: str, c: Ceiling, each: str) -> int:
    """Run one operation under `callgrind`, one dump per call boundary, and
    return the dearest call."""
    with tempfile.TemporaryDirectory() as tmp:
        profile = Path(tmp) / "callgrind.out"
        subprocess.run(
            [
                "valgrind",
                "--tool=callgrind",
                *dumps(c),
                f"--callgrind-out-file={profile}",
                exe,
                c.operation,
            ],
            check=True,
            capture_output=True,
        )
        return dearest([p.read_text() for p in Path(tmp).glob("callgrind.out*")], each)


def shard(ceilings: list[Ceiling], spec: str) -> list[Ceiling]:
    """The operations of shard `k/n`: every `n`-th one, from index `k`.

    Raises `ValueError` on a spec that is not `k/n` with `0 <= k < n`."""
    k, slash, n = spec.partition("/")
    if not slash or not k.isdigit() or not n.isdigit() or not 0 <= int(k) < int(n):
        raise ValueError(f"a shard reads `k/n` with 0 <= k < n, not `{spec}`")
    return [c for i, c in enumerate(ceilings) if i % int(n) == int(k)]


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    spec = "0/1"
    if args[:1] == ["--shard"] and len(args) == 2:
        spec = args[1]
    elif args:
        print("usage: instructions.py [--shard k/n]", file=sys.stderr)
        return 2
    try:
        ceilings = shard(CEILINGS, spec)
    except ValueError as why:
        print(why, file=sys.stderr)
        return 2
    failures = []
    binaries: dict[tuple[str, str, tuple[str, ...]], str] = {}
    for c in ceilings:
        if binary_key(c) not in binaries:
            binaries[binary_key(c)] = build(c)
    # One `callgrind` per processor: each runs one process on one thread.
    with ThreadPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        counts = list(pool.map(lambda c: count(binaries[binary_key(c)], c), ceilings))
    for c, n in zip(ceilings, counts):
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
