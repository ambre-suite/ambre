#!/usr/bin/env python3
"""The download of the wasm artefacts, against their ceilings and the budget.

The budget is `ambre-spec-budgets.md section 5`: the first download, Main plus
Sync, under 4 MB brotli, one wasm for the whole suite. The check fails past it,
and it also holds a tighter **ceiling** per instance at today's figure. A change
that grows either download fails; a change that shrinks one lowers its ceiling
in the same commit.

A converter module is fetched on the first conversion of its family, not at
load, so it counts against its own ceiling and its own budget, and never
against the first download (`ambre-spec-architecture.md section 2`, the
converter modules). Each module's budget is its row of
`ambre-spec-budgets.md section 5`: room for the formats the roadmap announces
in that family. Its ceiling sits at today's figure, as an instance's does.

The figure is the size of the `.br` copy the post-build hook writes beside each
wasm (`scripts/trunk_post_build.py`), which a static server sends as it is. The
JavaScript glue is left out: a few kilobytes beside megabytes.

Run: `just size`, which builds the release first. Needs `trunk` and `brotli`.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The first download, Main plus Sync, under 4 MB brotli: one wasm for the
# whole suite (`ambre-spec-budgets.md section 5`).
BUDGET = 4_000_000

# Today's figures, rounded up to the next 10 KB. Lowered by any change that
# shrinks a download. Raised only when `ambre-spec-budgets.md section 5` names
# the crate that grew and why.
CEILINGS = {
    "Main": 2_740_000,
    "Sync": 380_000,
    "Tables module": 50_000,
    "PDF module": 420_000,
    "Markup module": 250_000,
    "Images module": 230_000,
    "OOXML module": 340_000,
    # The call worker, spawned on the first call (`ambre-spec-call-media.md`).
    "Call worker": 40_000,
}

# The budget of each converter module, its row of `ambre-spec-budgets.md
# section 5`. A ceiling may rise up to it, and no further, without a new row.
MODULE_BUDGETS = {
    "Tables module": 100_000,
    "PDF module": 600_000,
    "Markup module": 300_000,
    "Images module": 300_000,
    "OOXML module": 900_000,
    # The call worker waits for a call as a module waits for a conversion, so it
    # holds a budget of its own and stays out of the first download.
    "Call worker": 100_000,
}

# The artefacts the page loads before its first interaction: the first
# download the budget holds. The converter modules wait for a conversion.
FIRST_DOWNLOAD = ("Main", "Sync")

# The brotli copy of each artefact in `dist/`, hashed by `filehash` or named
# by its digest (`scripts/trunk_post_build.py`).
GLOBS = {
    "Main": "ambre-app*_bg.wasm.br",
    "Sync": "ambre-sync*_bg.wasm.br",
    "Tables module": "ambre-convert-tables-*.wasm.br",
    "PDF module": "ambre-convert-pdf-*.wasm.br",
    "Markup module": "ambre-convert-markup-*.wasm.br",
    "Images module": "ambre-convert-images-*.wasm.br",
    "OOXML module": "ambre-convert-ooxml-*.wasm.br",
    "Call worker": "ambre-call*_bg.wasm.br",
}


def find(dist: Path, pattern: str) -> Path:
    """The one file of `dist` that `pattern` names."""
    found = sorted(dist.glob(pattern))
    if len(found) != 1:
        raise FileNotFoundError(f"{len(found)} files match {pattern} in {dist}, not one")
    return found[0]


def verdict(
    sizes: dict[str, int],
    ceilings: dict[str, int],
    budget: int,
    module_budgets: dict[str, int] | None = None,
) -> tuple[bool, list[str]]:
    """Whether every size holds its ceiling and its budget, and the first download the budget.

    Returns the verdict and the lines that say so.
    """
    module_budgets = MODULE_BUDGETS if module_budgets is None else module_budgets
    ok = True
    lines = []
    for name, size in sizes.items():
        line = f"{name}: {size:,} bytes brotli, ceiling {ceilings[name]:,}"
        if name in module_budgets:
            line += f", budget {module_budgets[name]:,}"
        if size > ceilings[name]:
            ok = False
            line += f": {size - ceilings[name]:,} over the ceiling"
        if name in module_budgets and size > module_budgets[name]:
            ok = False
            line += f": {size - module_budgets[name]:,} over the budget"
        lines.append(line)
    total = sum(size for name, size in sizes.items() if name in FIRST_DOWNLOAD)
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
        sizes = {name: find(dist, pattern).stat().st_size for name, pattern in GLOBS.items()}
    except FileNotFoundError as err:
        print(f"{err}: run `trunk build --release` first", file=sys.stderr)
        return 1
    ok, lines = verdict(sizes, CEILINGS, BUDGET)
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
