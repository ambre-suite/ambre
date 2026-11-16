#!/usr/bin/env python3
"""The wasm artefacts, against the product budgets, and their change against main.

The gate fails on three figures alone, the budgets of `ambre-spec-budgets.md
section 5` (`ARCH-096`):

* the first download, Main plus Sync, under `BUDGET` in brotli: one wasm for
  the whole suite;
* each lazy artefact, a converter module or the call worker, under its own
  brotli figure of `MODULE_BUDGETS`;
* the raw wasm of Main under its ceiling of `CEILINGS`, set at least 2 % above
  its measure: the net against growth on the first download.

No other figure fails. A brotli budget holds a wide margin, so the noise of
brotli at quality 11, about 850 bytes between two builds of one source on
Main, cannot move its verdict. The Main ceiling holds raw bytes, which move
by about 63 bytes between two such builds.

Every other figure is a report (`ARCH-115`): the raw and brotli bytes of each
artefact, and their change against a measure of `main` that `--baseline`
names. `--record` writes the measure of this build, so a run of `main` leaves
the measure the next branch reads. CI keeps it in its cache.

A converter module is fetched on the first conversion of its family, not at
load, so it never counts against the first download (`ambre-spec-architecture.md
section 2`, the converter modules).

The brotli figure is the size of the `.br` copy the post-build hook writes
beside each wasm (`scripts/trunk_post_build.py`), which a static server sends
as it is. The JavaScript glue is left out: a few kilobytes beside megabytes.

Run: `just size`, which builds the release first. Needs `trunk` and `brotli`.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The first download, Main plus Sync, under 4 MB brotli: one wasm for the
# whole suite (`ambre-spec-budgets.md section 5`). It rises only through a
# story row (`ARCH-095`).
BUDGET = 4_000_000

# The raw wasm bytes of Main, the one raw ceiling that fails a build. Set at
# least 2 % above the measure of `main`, rounded up to 10,000 bytes. It rises
# only through a story row that names the new figure and the cut tried first
# (`ARCH-095`), and the row sets it 2 % above the new measure again.
CEILINGS = {
    "Main": 10_660_000,
}

# The brotli budget of each lazy artefact, its row of `ambre-spec-budgets.md
# section 5`. It rises only through a story row (`ARCH-095`).
MODULE_BUDGETS = {
    "Tables module": 100_000,
    "PDF module": 600_000,
    "Markup module": 300_000,
    "Images module": 300_000,
    "Office module": 900_000,
    # The call worker waits for a call as a module waits for a conversion, so it
    # holds a budget of its own and stays out of the first download.
    "Call worker": 100_000,
}

# The artefacts the page loads before its first interaction: the first
# download the budget holds. The converter modules wait for a conversion.
FIRST_DOWNLOAD = ("Main", "Sync")

# The brotli copy of each artefact in `dist/`, hashed by `filehash` or named
# by its digest (`scripts/trunk_post_build.py`). The raw wasm sits beside it,
# under the same name without `.br` (`raw_glob`).
GLOBS = {
    "Main": "ambre-app*_bg.wasm.br",
    "Sync": "ambre-sync*_bg.wasm.br",
    "Tables module": "ambre-convert-tables-*.wasm.br",
    "PDF module": "ambre-convert-pdf-*.wasm.br",
    "Markup module": "ambre-convert-markup-*.wasm.br",
    "Images module": "ambre-convert-images-*.wasm.br",
    "Office module": "ambre-convert-office-*.wasm.br",
    "Call worker": "ambre-call*_bg.wasm.br",
}

# The brotli noise of Main between two builds of one source
# (`ambre-spec-budgets.md section 5`, the noise band). A brotli change under
# it says nothing, and the report says so.
BROTLI_NOISE = 1_000


@dataclass(frozen=True)
class Measure:
    """The raw and brotli bytes of each artefact of one build, and the commit built."""

    commit: str
    raw: dict[str, int]
    brotli: dict[str, int]

    def to_json(self) -> str:
        return json.dumps({"commit": self.commit, "raw": self.raw, "brotli": self.brotli}, indent=2)

    @staticmethod
    def from_json(text: str) -> Measure:
        """A measure `to_json` wrote. Raises `ValueError` on any other text."""
        data = json.loads(text)
        if not isinstance(data, dict):
            raise ValueError("a measure is a JSON object")
        commit, raw, brotli = data.get("commit"), data.get("raw"), data.get("brotli")
        if not isinstance(commit, str) or not all(
            isinstance(table, dict)
            and all(isinstance(k, str) and type(v) is int for k, v in table.items())
            for table in (raw, brotli)
        ):
            raise ValueError("a measure holds a commit, then raw and brotli bytes per artefact")
        return Measure(commit, raw, brotli)


def find(dist: Path, pattern: str) -> Path:
    """The one file of `dist` that `pattern` names."""
    found = sorted(dist.glob(pattern))
    if len(found) != 1:
        raise FileNotFoundError(f"{len(found)} files match {pattern} in {dist}, not one")
    return found[0]


def raw_glob(pattern: str) -> str:
    """The glob of the raw wasm beside the brotli copy that `pattern` names."""
    return pattern.removesuffix(".br")


def verdict(
    sizes: dict[str, int],
    ceilings: dict[str, int],
    budget: int,
    module_budgets: dict[str, int] | None = None,
    compressed: dict[str, int] | None = None,
) -> tuple[bool, list[str]]:
    """Whether the budgets hold, and the lines that say so.

    `sizes` holds the raw wasm bytes, which `ceilings` holds for the artefacts
    it names. `compressed` holds the brotli bytes, which the budgets hold; a
    test that omits it reads the raw figures as brotli too. An artefact that no
    ceiling and no budget names fails nothing.
    """
    module_budgets = MODULE_BUDGETS if module_budgets is None else module_budgets
    compressed = sizes if compressed is None else compressed
    ok = True
    lines = []
    for name, size in sizes.items():
        br = compressed[name]
        line = f"{name}: {size:,} bytes raw"
        if name in ceilings:
            line += f", ceiling {ceilings[name]:,}"
        line += f"; {br:,} brotli"
        if name in module_budgets:
            line += f", budget {module_budgets[name]:,}"
        if name in ceilings and size > ceilings[name]:
            ok = False
            line += f": {size - ceilings[name]:,} over the ceiling"
        if name in module_budgets and br > module_budgets[name]:
            ok = False
            line += f": {br - module_budgets[name]:,} over the budget"
        lines.append(line)
    total = sum(br for name, br in compressed.items() if name in FIRST_DOWNLOAD)
    line = f"First download: {total:,} bytes brotli, budget {budget:,}"
    if total > budget:
        ok = False
        line += f": {total - budget:,} over the budget"
    else:
        line += ": within the budget"
    lines.append(line)
    return ok, lines


def signed(change: int) -> str:
    """A change in bytes with its sign: `+12`, `-3`, `0`."""
    return f"{change:+,}" if change else "0"


def delta(head: Measure, base: Measure | None) -> list[str]:
    """One Markdown row per artefact: its figures and their change against `base`.

    A change fails nothing (`ARCH-115`). An artefact the base lacks reads `new`.
    """
    rows = [
        "| Artefact | Raw | Raw change | Brotli | Brotli change |",
        "| :--- | ---: | ---: | ---: | ---: |",
    ]
    for name, raw in head.raw.items():
        br = head.brotli[name]
        if base is None or name not in base.raw or name not in base.brotli:
            changes = ("--", "--") if base is None else ("new", "new")
        else:
            changes = (signed(raw - base.raw[name]), signed(br - base.brotli[name]))
        rows.append(f"| {name} | {raw:,} | {changes[0]} | {br:,} | {changes[1]} |")
    return rows


def report(head: Measure, base: Measure | None) -> list[str]:
    """The delta table under a line that names the base it reads against."""
    if base is None:
        title = "No measure of `main` to compare with: the changes read `--`."
    else:
        title = f"Change against the measure of `main` at `{base.commit[:12]}`."
    note = (
        f"Brotli moves by up to {BROTLI_NOISE:,} bytes between two builds of one source; "
        "a smaller brotli change is noise. No change fails the build."
    )
    return [title, "", *delta(head, base), "", note]


def commit() -> str:
    """The commit of the work tree, or `unknown` outside a git checkout."""
    done = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True)
    return done.stdout.strip() if done.returncode == 0 else "unknown"


def read_base(path: Path | None) -> tuple[Measure | None, str | None]:
    """The measure at `path`, or None with the reason it reads none."""
    if path is None:
        return None, None
    if not path.is_file():
        return None, f"no measure of main at {path}"
    try:
        return Measure.from_json(path.read_text(encoding="utf-8")), None
    except ValueError as err:
        return None, f"the measure at {path} is not one: {err}"


def summary(lines: list[str]) -> None:
    """Append `lines` to the step summary of a GitHub Actions run, when there is one."""
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as out:
            out.write("\n".join(["## Wasm size", "", *lines, ""]) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline", type=Path, help="a measure of main that --record wrote, read for the change"
    )
    parser.add_argument("--record", type=Path, help="write the measure of this build there")
    args = parser.parse_args()
    dist = ROOT / "dist"
    try:
        compressed = {name: find(dist, pattern).stat().st_size for name, pattern in GLOBS.items()}
        sizes = {name: find(dist, raw_glob(pattern)).stat().st_size for name, pattern in GLOBS.items()}
    except FileNotFoundError as err:
        print(f"{err}: run `trunk build --release` first", file=sys.stderr)
        return 1
    base, why = read_base(args.baseline)
    if why:
        print(f"note: {why}", file=sys.stderr)
    head = Measure(commit(), sizes, compressed)
    ok, lines = verdict(sizes, CEILINGS, BUDGET, compressed=compressed)
    table = report(head, base)
    print("\n".join([*lines, "", *table]))
    summary([*(f"* {line}" for line in lines), "", *table])
    if args.record:
        args.record.parent.mkdir(parents=True, exist_ok=True)
        args.record.write_text(head.to_json() + "\n", encoding="utf-8")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
