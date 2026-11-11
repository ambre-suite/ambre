#!/usr/bin/env python3
"""A size ceiling rises only through a story row that names the new figure.

`ARCH-095` states the rule, and `ambre-spec-budgets.md section 5` states why:
the Main ceiling rose twice in the merge of a feature story, so the growth
reached `main` with no cut tried and no row to read it in.

The gate reads `CEILINGS` of `scripts/wasm_size.py` at the base revision,
`origin/main` by default, or its parent on a push of `main` itself, as
`schema_freeze.py` does. It reads the same table in the work tree. For each
ceiling that went up, a story row of `specs/ambre-roadmap.md` or of the journal
must name the new figure and the cut tried first, that is the word `cut`. The
figure may be written `2,750,000`, `2 750 000`, `2_750_000` or `2750000`.

* A ceiling that went down or held passes: lowering is always free.
* A ceiling absent at the base passes: a new artefact has no figure to cut
  from, and the budget row of its module bounds it (`wasm_size.py`).
* A base the gate cannot read is an error, never a pass.

Run: `just ceiling-raise`. It writes nothing.
"""

from __future__ import annotations

import argparse
import ast
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from schema_freeze import base_not_found, resolve  # noqa: E402  -- one definition of the base revision

ROOT = Path(__file__).resolve().parent.parent
SIZE = "scripts/wasm_size.py"
TABLE = "CEILINGS"

# The plan files, where the story of a raise lives, and the shape of a story row.
PLANS = ("specs/ambre-roadmap.md", "specs/ambre-roadmap-journal.md")
STORY_ROW = re.compile(r"^\|\s*`\[[ x]\]`")

# A figure in a row: digits grouped by three under one separator, or bare digits.
FIGURE = re.compile(r"(?<![\d.])\d{1,3}(?:([ ,_])\d{3}(?:\1\d{3})*)?(?![\d.])|(?<![\d.])\d+(?![\d.])")
CUT = re.compile(r"\bcut\b", re.IGNORECASE)


def ceilings(text: str | None) -> dict[str, int] | None:
    """The `CEILINGS` table of one copy of `wasm_size.py`, or None when it holds none.

    The table is read as a literal, so a ceiling computed at run time is no ceiling.
    """
    if text is None:
        return None
    for node in ast.parse(text).body:
        target = node.targets[0] if isinstance(node, ast.Assign) else getattr(node, "target", None)
        if isinstance(target, ast.Name) and target.id == TABLE and node.value is not None:
            try:
                table = ast.literal_eval(node.value)
            except ValueError:
                return None
            if isinstance(table, dict) and all(
                isinstance(k, str) and isinstance(v, int) for k, v in table.items()
            ):
                return table
            return None
    return None


def figures(line: str) -> set[int]:
    """Each figure one line names, separators dropped."""
    return {int(re.sub(r"[ ,_]", "", m.group(0))) for m in FIGURE.finditer(line)}


def named(figure: int, plans: list[str]) -> bool:
    """Whether a story row names `figure` and the cut tried first."""
    for text in plans:
        for line in text.splitlines():
            if STORY_ROW.match(line) and CUT.search(line) and figure in figures(line):
                return True
    return False


def raise_breaches(base: dict[str, int] | None, head: dict[str, int], plans: list[str]) -> list[str]:
    """Each ceiling that went up since the base with no story row that names it.

    `plans` holds the text of the roadmap and the journal at the head.
    """
    out = []
    for name, now in head.items():
        was = (base or {}).get(name)
        if was is None or now <= was or named(now, plans):
            continue
        out.append(
            f"{SIZE}: the {name} ceiling rose from {was:,} to {now:,}, and no story row of "
            f"`{PLANS[0]}` or `{PLANS[1]}` names {now:,} and a cut. Write a story row that "
            f"names the new figure, {now:,}, and the cut tried first, or lower the ceiling "
            "(`ARCH-095`)"
        )
    return out


def at(revision: str, path: str) -> str | None:
    """One file's text at a revision, or None when the revision lacks it."""
    done = subprocess.run(
        ["git", "show", f"{revision}:{path}"], cwd=ROOT, capture_output=True, text=True
    )
    return done.stdout if done.returncode == 0 else None


def plans() -> list[str]:
    """The text of each plan file in the work tree."""
    return [(ROOT / p).read_text(encoding="utf-8") for p in PLANS if (ROOT / p).is_file()]


def current() -> str | None:
    """The text of `wasm_size.py` in the work tree."""
    path = ROOT / SIZE
    return path.read_text(encoding="utf-8") if path.is_file() else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        default="origin/main",
        help="the revision a raise is read against (default: origin/main)",
    )
    args = parser.parse_args()
    try:
        base = resolve(args.base)
    except LookupError as missing:
        print(
            f"ERROR: {base_not_found(missing.args[0])}. No ceiling is checked until "
            "the base is read; CI sets `fetch-depth` on the checkout step.",
            file=sys.stderr,
        )
        return 1
    head = ceilings(current())
    if head is None:
        print(f"ERROR: {SIZE} holds no literal `{TABLE}` table", file=sys.stderr)
        return 1
    errors = raise_breaches(ceilings(at(base, SIZE)), head, plans())
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"ceiling raise: {len(errors)} ceiling(s) raised with no story row", file=sys.stderr)
        return 1
    print(f"ceiling raise: {len(head)} ceilings, none raised without a story row")
    return 0


if __name__ == "__main__":
    sys.exit(main())
