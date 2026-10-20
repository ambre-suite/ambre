#!/usr/bin/env python3
"""The download of the Main instance, against its ceiling and its budget.

The budget is `ambre-spec-budgets.md section 5`: the Main instance under 2 MB
gzip. The build breaks it today, so the check holds a **ceiling** at today's
figure instead. A change that grows the download fails; a change that shrinks
it lowers `CEILING` in the same commit. The ceiling is not the budget. The
`The Main instance within its download budget` Story (`ambre-roadmap.md`) is
done when the ceiling sits under the budget.

The figure is the gzip of the wasm Trunk ships, at the highest level, as a
server compresses a static file once. The JavaScript glue is left out: a few
kilobytes beside megabytes.

Run: `just size`, which builds the release first. Needs `trunk`.
"""

from __future__ import annotations

import gzip
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The Main instance under 2 MB gzip (`ambre-spec-budgets.md section 5`).
BUDGET = 2_000_000

# Today's figure, rounded up to the next 10 KB. Lowered by any change that
# shrinks the download, and never raised to fit one that grows it.
CEILING = 4_270_000

# The wasm of the Main instance, as `trunk build --release` writes it.
MAIN = ROOT / "dist" / "ambre-app_bg.wasm"


def gzipped(data: bytes) -> int:
    """The bytes `data` weighs on the wire."""
    return len(gzip.compress(data, compresslevel=9, mtime=0))


def verdict(size: int, ceiling: int, budget: int) -> tuple[bool, str]:
    """Whether `size` holds `ceiling`, and the line that says so."""
    line = f"Main instance: {size:,} bytes gzip, ceiling {ceiling:,}, budget {budget:,}"
    if size > ceiling:
        return False, f"{line}: {size - ceiling:,} over the ceiling"
    if size > budget:
        return True, f"{line}: {size - budget:,} over the budget"
    return True, f"{line}: within the budget"


def main() -> int:
    if not MAIN.is_file():
        print(f"{MAIN} is missing: run `trunk build --release` first", file=sys.stderr)
        return 1
    ok, line = verdict(gzipped(MAIN.read_bytes()), CEILING, BUDGET)
    print(line)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
