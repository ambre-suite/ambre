#!/usr/bin/env python3
"""Print one spec's contract as a reading view: the id, the kind, the rule.

A contract table is seven columns wide, and four of them answer a question a
reader asking "what does this document bind" is not asking. `Sources`,
`Mechanism`, `Gap` and `Status` are what a review and a gate read: where the
rule comes from, which mechanism holds it, what the mechanism leaves open, and
where that stands today. They are the reason the table is trustworthy, and they
are also why a row wraps over four screen widths.

So the columns stay, and the reading view drops them here rather than in the
file. What comes out is the document's opening, then every contract row under
its own subsection, reduced to `ID | Kind | Statement`. Nothing is computed:
this reads the rows `contracts.py` reads, through `corpus.py`, so a view that
disagreed with the ledger would be a bug in one file and not a second
interpretation of the corpus.

It writes to standard output and never to a file. There is no `--check` and no
generated artefact: a reading view checked into the tree is a second copy of
every rule, free to go stale, and the corpus already refuses that shape for
facts it generates (`ambre-spec-template.md section 1.G`).

Run: `just spec-read <file>`, where `<file>` is a path or a bare file name.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import corpus  # noqa: E402  -- the corpus has one parser
from template_vocabulary import KIND, STATEMENT  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"

# The opening a reader needs before the rules: the title, the header fields,
# and the summary. Everything after the first `---` is prose the tables restate.
HEADER_FIELDS = ("**Type:**", "**Applications:**")


def resolve(name: str) -> Path:
    """The document `name` points at, given a path or a bare file name.

    A bare name is what a citation carries, so it is what a reader has in hand:
    `ambre-spec-locale.md` rather than the three directories above it.
    """
    direct = Path(name)
    if direct.is_file():
        return direct
    candidates = sorted(SPECS.rglob(Path(name).name))
    if not candidates:
        raise FileNotFoundError(f"no document named `{name}` under `specs/`")
    if len(candidates) > 1:
        found = ", ".join(str(c.relative_to(ROOT)) for c in candidates)
        raise FileNotFoundError(f"`{name}` names {len(candidates)}: {found}")
    return candidates[0]


def opening(text: str) -> list[str]:
    """The title, the header fields and the summary, in the order they are in.

    A field wraps, and its second line opens with none of the markers above, so
    the wrap is kept by what precedes it: an `Applications` list cut at the
    margin still names every application.
    """
    kept: list[str] = []
    wrapping = False
    for line in (line.text for line in corpus.parse(text).lines):
        if line.startswith("---"):
            break
        opens = line.startswith("# ") or line.startswith(">") or line.startswith(HEADER_FIELDS)
        if opens or (wrapping and line.strip()):
            kept.append(line)
        wrapping = line.startswith(HEADER_FIELDS) or (wrapping and bool(line.strip()))
    return kept


def reading_view(text: str) -> list[str]:
    """Every contract row of `text`, under the subsection heading that holds it.

    A row is printed where it was declared: the subsections are what group an
    operation's four kinds, and a flat list of the ids would lose the one piece
    of structure the reduction cannot restate. The heading is the most recent of
    either depth: a feature document groups its rows under `###` subsections,
    and a schema document declares them under a `##` section with no subsection.
    """
    doc = corpus.parse(text)
    lines = opening(text)
    printed_heading = None

    for row in doc.contract_rows:
        above = doc.heading_above(row.no)
        heading = above.text.strip() if above else ""
        if heading != printed_heading:
            lines += ["", heading, "", "| ID | Kind | Statement |", "|---|---|---|"]
            printed_heading = heading
        lines.append(
            f"| `{row.id}` | {row.cells.get(KIND, '')} |"
            f" {row.cells.get(STATEMENT, '')} |"
        )

    return lines


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("document", help="a path, or a bare file name under `specs/`")
    args = parser.parse_args()

    try:
        path = resolve(args.document)
    except FileNotFoundError as refusal:
        print(f"ERROR {refusal}", file=sys.stderr)
        return 1

    view = reading_view(path.read_text(encoding="utf-8"))
    if not any(line.startswith("| `") for line in view):
        print(f"ERROR `{path.name}` declares no contract row", file=sys.stderr)
        return 1

    print("\n".join(view))
    return 0


if __name__ == "__main__":
    sys.exit(main())
