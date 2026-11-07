#!/usr/bin/env python3
"""The download of the two wasm instances, against their ceilings and the budget.

The budget is `ambre-spec-budgets.md section 5`: the first download, Main plus
Sync, under 2 MB brotli. The build breaks it today, so the check holds a
**ceiling** per instance at today's figure instead. A change that grows either
download fails; a change that shrinks one lowers its ceiling in the same
commit. A ceiling is not the budget. The `The Main instance within its download
budget` Story (`ambre-roadmap.md`) is done when the first download sits under
the budget.

The figure is the size of the `.br` copy the post-build hook writes beside each
wasm (`scripts/trunk_post_build.py`), which a static server sends as it is. The
JavaScript glue is left out: a few kilobytes beside megabytes.

Run: `just size`, which builds the release first. Needs `trunk` and `brotli`.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The first download, Main plus Sync, under 2 MB brotli
# (`ambre-spec-budgets.md section 5`).
BUDGET = 2_000_000

# Today's figures, rounded up to the next 10 KB. Lowered by any change that
# shrinks a download. Raised only when `ambre-spec-budgets.md section 5` names
# the crate that grew and why.
CEILINGS = {
    "Main": 3_130_000,
    "Sync": 360_000,
}

# The brotli copy of each instance in `dist/`, hashed by `filehash`.
GLOBS = {
    "Main": "ambre-app*_bg.wasm.br",
    "Sync": "ambre-sync*_bg.wasm.br",
}


def find(dist: Path, pattern: str) -> Path:
    """The one file of `dist` that `pattern` names."""
    found = sorted(dist.glob(pattern))
    if len(found) != 1:
        raise FileNotFoundError(f"{len(found)} files match {pattern} in {dist}, not one")
    return found[0]


def verdict(sizes: dict[str, int], ceilings: dict[str, int], budget: int) -> tuple[bool, list[str]]:
    """Whether every size holds its ceiling, and the lines that say so."""
    ok = True
    lines = []
    for name, size in sizes.items():
        line = f"{name} instance: {size:,} bytes brotli, ceiling {ceilings[name]:,}"
        if size > ceilings[name]:
            ok = False
            line += f": {size - ceilings[name]:,} over the ceiling"
        lines.append(line)
    total = sum(sizes.values())
    line = f"First download: {total:,} bytes brotli, budget {budget:,}"
    if total > budget:
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
