#!/usr/bin/env python3
"""The wasm artefacts, against their raw ceilings and their brotli budgets.

The budget is `ambre-spec-budgets.md section 5`: the first download, Main plus
Sync, under 4 MB brotli, one wasm for the whole suite. The check fails past it.
It also holds a tighter **ceiling** per artefact, on its raw wasm bytes. A
change that grows an artefact past its ceiling fails; a change that shrinks
one lowers its ceiling in the same commit.

A ceiling holds raw bytes because brotli at quality 11 is noisy: two builds of
one source differ by about 850 bytes of brotli and 63 raw bytes on Main (the
noise band of `ambre-spec-budgets.md section 5`). The raw figure is the one a
change controls. The brotli figures stay in the output, for information.

A converter module is fetched on the first conversion of its family, not at
load, so it counts against its own ceiling and its own budget, and never
against the first download (`ambre-spec-architecture.md section 2`, the
converter modules). Each module's budget is its row of
`ambre-spec-budgets.md section 5`, in brotli: room for the formats the roadmap
announces in that family.

The brotli figure is the size of the `.br` copy the post-build hook writes
beside each wasm (`scripts/trunk_post_build.py`), which a static server sends
as it is. The JavaScript glue is left out: a few kilobytes beside megabytes.

Run: `just size`, which builds the release first. Needs `trunk` and `brotli`.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The first download, Main plus Sync, under 4 MB brotli: one wasm for the
# whole suite (`ambre-spec-budgets.md section 5`). It rises only as a ceiling
# does (`ARCH-095`).
BUDGET = 4_000_000

# The raw wasm bytes of each artefact. Each ceiling is the measured figure
# rounded up to the next multiple of 10,000, or to the one after when that
# leaves under 1,000 bytes of room: the raw noise band is 63 bytes. Lowered by
# any change that shrinks an artefact. Raised only when `ambre-spec-budgets.md
# section 5` names the crate that grew and why, and a story row names the new
# figure and the cut tried first: `scripts/ceiling_raise.py` refuses any other
# raise (`ARCH-095`).
CEILINGS = {
    "Main": 10_410_000,
    "Sync": 1_310_000,
    "Tables module": 140_000,
    "PDF module": 1_830_000,
    "Markup module": 870_000,
    "Images module": 830_000,
    "Office module": 1_480_000,
    # The call worker, spawned on the first call (`ambre-spec-call-media.md`).
    "Call worker": 110_000,
}

# The brotli budget of each converter module, its row of `ambre-spec-budgets.md
# section 5`. It rises only as a ceiling does (`ARCH-095`).
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
    """Whether every artefact holds its ceiling and its budget, and the first download the budget.

    `sizes` holds the raw wasm bytes, which the ceilings hold. `compressed`
    holds the brotli bytes, which the budgets hold; a test that omits it reads
    the raw figures as brotli too. Returns the verdict and the lines that say so.
    """
    module_budgets = MODULE_BUDGETS if module_budgets is None else module_budgets
    compressed = sizes if compressed is None else compressed
    ok = True
    lines = []
    for name, size in sizes.items():
        br = compressed[name]
        line = f"{name}: {size:,} bytes raw, ceiling {ceilings[name]:,}; {br:,} brotli"
        if name in module_budgets:
            line += f", budget {module_budgets[name]:,}"
        if size > ceilings[name]:
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


def main() -> int:
    dist = ROOT / "dist"
    try:
        compressed = {name: find(dist, pattern).stat().st_size for name, pattern in GLOBS.items()}
        sizes = {name: find(dist, raw_glob(pattern)).stat().st_size for name, pattern in GLOBS.items()}
    except FileNotFoundError as err:
        print(f"{err}: run `trunk build --release` first", file=sys.stderr)
        return 1
    ok, lines = verdict(sizes, CEILINGS, BUDGET, compressed=compressed)
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
