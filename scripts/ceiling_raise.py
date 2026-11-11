#!/usr/bin/env python3
"""A size ceiling or budget rises only through a story row that names the new figure.

`ARCH-095` states the rule, and `ambre-spec-budgets.md section 5` states why:
the Main ceiling rose twice in the merge of a feature story, so the growth
reached `main` with no cut tried and no row to read it in.

The gate reads three literals of `scripts/wasm_size.py`: `CEILINGS`, the raw
bytes per artefact, `MODULE_BUDGETS`, the brotli budget per module, and
`BUDGET`, the brotli budget of the first download. It reads them at the base
revision, `origin/main` by default, or its parent on a push of `main` itself,
as `schema_freeze.py` does, and in the work tree. For each figure that went
up, a story row of `specs/ambre-roadmap.md` or of the journal must name the
new figure and the cut tried first, that is the word `cut`. The figure may be
written `2,750,000`, `2 750 000`, `2_750_000` or `2750000`.

* A figure that went down or held passes: lowering is always free.
* A name absent at the base that `ALIASES` maps from an old name is a rename:
  its figure is compared with the base figure of the old name.
* A name absent at the base and in no alias is a new artefact. It passes when
  a story row names the artefact and its figure.
* A name in an alias that the base holds under neither name fails: a rename
  needs a figure to compare with.
* A table the base lacks is checked from the next base on.
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

# Each literal the gate holds, and the word an error names its figures with.
# `BUDGET` is one figure, read as a table of one entry under `FIRST_DOWNLOAD`.
TABLES = {"CEILINGS": "ceiling", "MODULE_BUDGETS": "budget", "BUDGET": "budget"}
FIRST_DOWNLOAD = "First download"

# A renamed artefact: the old name, then the new one. The new name is compared
# with the base figure of the old name, so a rename cannot hide a raise.
ALIASES = {"OOXML module": "Office module"}

# The plan files, where the story of a raise lives, and the shape of a story row.
PLANS = ("specs/ambre-roadmap.md", "specs/ambre-roadmap-journal.md")
STORY_ROW = re.compile(r"^\|\s*`\[[ x]\]`")

# A figure in a row: digits grouped by three under one separator, or bare digits.
# A period that ends a sentence ends the figure; a period before a digit makes
# it a decimal, which names no figure.
FIGURE = re.compile(r"(?<![\d.])\d{1,3}(?:([ ,_])\d{3}(?:\1\d{3})*)?(?!\d|\.\d)|(?<![\d.])\d+(?!\d|\.\d)")
CUT = re.compile(r"\bcut\b", re.IGNORECASE)


def literal(text: str | None, name: str) -> object | None:
    """The literal value bound to `name` at the top of `text`, or None when there is none.

    The value is read as a literal, so a figure computed at run time is no figure.
    """
    if text is None:
        return None
    for node in ast.parse(text).body:
        target = node.targets[0] if isinstance(node, ast.Assign) else getattr(node, "target", None)
        if isinstance(target, ast.Name) and target.id == name and node.value is not None:
            try:
                return ast.literal_eval(node.value)
            except ValueError:
                return None
    return None


def table(text: str | None, name: str) -> dict[str, int] | None:
    """One limit table of one copy of `wasm_size.py`, or None when it holds none.

    `BUDGET` is one integer, and reads as `{FIRST_DOWNLOAD: value}`.
    """
    value = literal(text, name)
    if isinstance(value, int) and not isinstance(value, bool):
        return {FIRST_DOWNLOAD: value}
    if isinstance(value, dict) and all(
        isinstance(k, str) and isinstance(v, int) and not isinstance(v, bool) for k, v in value.items()
    ):
        return value
    return None


def ceilings(text: str | None) -> dict[str, int] | None:
    """The `CEILINGS` table of one copy of `wasm_size.py`, or None when it holds none."""
    return table(text, TABLE)


def figures(line: str) -> set[int]:
    """Each figure one line names, separators dropped."""
    return {int(re.sub(r"[ ,_]", "", m.group(0))) for m in FIGURE.finditer(line)}


def story_rows(plans: list[str]):
    """Each story row of the plan files."""
    for text in plans:
        for line in text.splitlines():
            if STORY_ROW.match(line):
                yield line


def named(figure: int, plans: list[str]) -> bool:
    """Whether a story row names `figure` and the cut tried first."""
    return any(CUT.search(line) and figure in figures(line) for line in story_rows(plans))


def introduced(artefact: str, figure: int, plans: list[str]) -> bool:
    """Whether a story row names a new artefact and its figure."""
    return any(
        artefact.lower() in line.lower() and figure in figures(line) for line in story_rows(plans)
    )


def aliased(name: str) -> bool:
    """Whether `name` stands in the alias table, as an old name or a new one."""
    return name in ALIASES or name in ALIASES.values()


def raise_breaches(
    base: dict[str, int] | None,
    head: dict[str, int],
    plans: list[str],
    word: str = "ceiling",
) -> list[str]:
    """Each figure of one table that went up since the base with no story row that names it.

    `plans` holds the text of the roadmap and the journal at the head. `word`
    names the figures of the table in an error: `ceiling` or `budget`.
    """
    if base is None:
        return []
    out = []
    for name, now in head.items():
        was, old = base.get(name), name
        if was is None:
            old = next((o for o, n in ALIASES.items() if n == name and o in base), name)
            was = base.get(old)
        if was is None:
            if aliased(name):
                out.append(
                    f"{SIZE}: the {name} {word}, {now:,}, is a renamed artefact the base holds "
                    "under neither name of `ALIASES`. Fix the alias, or name the artefact "
                    "apart from it (`ARCH-095`)"
                )
            elif not introduced(name, now, plans):
                out.append(
                    f"{SIZE}: the {name} {word}, {now:,}, is new since the base, and no story "
                    f"row of `{PLANS[0]}` or `{PLANS[1]}` names {name} and {now:,}. Write a "
                    "story row that names the new artefact and its figure (`ARCH-095`)"
                )
            continue
        if now <= was or named(now, plans):
            continue
        since = name if old == name else f"{old}, renamed {name},"
        out.append(
            f"{SIZE}: the {since} {word} rose from {was:,} to {now:,}, and no story row of "
            f"`{PLANS[0]}` or `{PLANS[1]}` names {now:,} and a cut. Write a story row that "
            f"names the new figure, {now:,}, and the cut tried first, or lower the {word} "
            "(`ARCH-095`)"
        )
    return out


def breaches(base_text: str | None, head_text: str | None, plans: list[str]) -> list[str]:
    """Each raise of every table of `TABLES` between two copies of `wasm_size.py`.

    A table the head lacks is an error: the gate holds nothing without it.
    """
    out = []
    for name, word in TABLES.items():
        head = table(head_text, name)
        if head is None:
            out.append(f"ERROR: {SIZE} holds no literal `{name}`")
            continue
        out.extend(raise_breaches(table(base_text, name), head, plans, word))
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
    head = current()
    errors = breaches(at(base, SIZE), head, plans())
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"ceiling raise: {len(errors)} figure(s) raised with no story row", file=sys.stderr)
        return 1
    count = sum(len(table(head, name) or {}) for name in TABLES)
    print(f"ceiling raise: {count} ceilings and budgets, none raised without a story row")
    return 0

if __name__ == "__main__":
    sys.exit(main())
