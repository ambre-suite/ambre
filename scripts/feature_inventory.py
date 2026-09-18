#!/usr/bin/env python3
"""Generate the feature inventory of each application, and fail when it is stale.

An inventory answers a question no other file answers: **what does the product do
today**. The roadmap answers what is left to do, and a feature built by three
stories across two epics appears nowhere whole in it. The journal answers whether
a story shipped, under which epic. Neither reads as a product surface.

`ambre-spec-template.md section 4` gives the shape: name, scope, and a state
derived from the status of the feature's contract rows. It carries no design
content, so it is generated. The schema document is the opposite case and stays
hand-written: it holds decisions.

Everything in the file is read from the feature documents themselves. The scope
is the document's own summary. The state is derived from its contract rows:

  * `implemented`  -- no row is `unimplemented`
  * `partial`      -- some rows are built, some name a Story that has not landed
  * `design-only`  -- no row is built

That is the vocabulary of the `Implementation` header field on purpose. Two
readings of one fact, one written by hand and one derived, so they can disagree.

Two rules are held here, and no other script can hold them:

  * **A feature document carries a contract table.** `ambre-spec-template.md
    section 1.B` demands one from both templates. `contracts.py` counts the rows
    it finds; it cannot see a document that declares none.
  * **The header may not claim more than the rows support.** A document that
    reads `implemented` while a row says `not built` is the lie the field exists
    to stop. Asymmetric, like the container check in `spec_shape.py`: claiming
    less is legitimate, because a surface may ship before it has a row.

Run: `just inventory` (fail when the file on disk is not what this would write),
`just inventory-write` (rewrite it). The check regenerates in memory and compares
the whole text, so a spec edited without a regeneration turns CI red on the next
push rather than leaving a stale product surface behind.
"""

import re
import sys
from pathlib import Path

# What "loose" means is a rule about statuses, and `contracts.py` is where the
# gates state it.
from contracts import LOOSE

# The template states both closed sets this file reads, and `contracts.py` reads
# them from the same place (`scripts/template_vocabulary.py`).
from template_vocabulary import IMPLEMENTATION_VALUES, STATUSES

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"

# A row is built unless it is `unimplemented`. What "loose" means is stated in
# `contracts.py` and read from there, never typed again: the day `unverified`
# became three values, that file was corrected and this one kept naming a status
# the template no longer has, which is the second statement drifting on its own.
UNBUILT = "unimplemented"

# `STATES` is the template's `Implementation` vocabulary, and its order is the
# reading order of the file: what the application does, then what it half does,
# then what it only describes. Alphabetical inside a state, so a diff of the
# generated file shows the feature that moved and nothing else.
STATES = IMPLEMENTATION_VALUES

# Same row grammar as `contracts.py`: an id, then cells, then the status last.
ROW = re.compile(r"^\|\s*`(?P<id>[A-Z][A-Z0-9-]*-\d{3})`\s*\|.*\|\s*(?P<status>[a-z-]+)\s*\|\s*$")
IMPLEMENTATION = re.compile(r"^\*\*Implementation:\*\* ([a-z-]+)")


def scope(text: str) -> str:
    """The first sentence of the document's own summary.

    `ambre-spec-template.md section 1.A` puts two or three lines in user terms
    right under the header, and it is the one place a feature says what it is
    without a container or a type. Nothing is written here that a document does
    not already say about itself.
    """
    lines = []
    for line in text.splitlines()[:20]:
        if line.startswith(">"):
            lines.append(line[1:].strip())
        elif lines:
            break
    summary = " ".join(lines)
    return re.split(r"(?<=\.)\s", summary)[0] if summary else ""


def counts(text: str) -> dict[str, int]:
    """One count per status, over every contract row of the document."""
    tally = dict.fromkeys(STATUSES, 0)
    for line in text.splitlines():
        match = ROW.match(line)
        if match and match["status"] in tally:
            tally[match["status"]] += 1
    return tally


def read(path: Path) -> dict:
    """Name, scope, declared state and row counts for one feature document."""
    text = path.read_text()
    tally = counts(text)
    total = sum(tally.values())
    built = total - tally[UNBUILT]
    # The header field, from the first eight lines: the same window
    # `spec_shape.py` reads, which is where the contract puts it.
    header = next((l for l in text.splitlines()[:8] if l.startswith("**Implementation:**")), "")
    match = IMPLEMENTATION.match(header)
    return {
        "cite": path.stem.replace("ambre-spec-", ""),
        "scope": scope(text),
        "declared": match[1] if match else None,
        "derived": "design-only" if built == 0 else ("implemented" if tally[UNBUILT] == 0 else "partial"),
        "rows": total,
        "built": built,
        "loose": sum(tally[s] for s in LOOSE),
    }


def defects(app: str, features: list[dict]) -> list[str]:
    """Yield what the rows say and the header does not."""
    out = []
    for f in features:
        where = f"{app}: `{f['cite']}`"
        if f["rows"] == 0:
            out.append(f"{where}: no contract table -- both templates carry one (template section 1.B)")
            continue
        if f["declared"] == "implemented" and f["rows"] != f["built"]:
            out.append(
                f"{where}: header says `implemented`, {f['rows'] - f['built']} rows are "
                f"`unimplemented` -- the field describes the code, not the intention"
            )
        if f["declared"] == "design-only" and f["built"]:
            out.append(f"{where}: header says `design-only`, {f['built']} rows are built")
    return out


def render(app: str, features: list[dict]) -> str:
    """The inventory file, whole. This is the only place its text is written."""
    tally = {state: sum(1 for f in features if f["derived"] == state) for state in STATES}
    lines = [
        f"# AMBRE -- {app.upper()} FEATURE INVENTORY",
        "",
        "**Type:** Inventory (generated)",
        "",
        "> What the application does today, one row per feature document. The roadmap holds what is",
        "> left to do; this holds what is. Generated by `scripts/feature_inventory.py` from the",
        "> documents themselves, so it carries no fact of its own.",
        "",
        "**Do not edit this file.** Edit the feature document and run `just inventory-write`.",
        "",
        "* **State** is derived from the contract rows, in the vocabulary of the `Implementation`",
        "  header field. It is `implemented` when no row is unbuilt, `design-only` when none is",
        "  built. The table is ordered by it, then by name.",
        "* **Loose** counts built rows that nothing green holds: "
        + ", ".join(f"`{s}`" for s in LOOSE)
        + ".",
        "",
        "| Feature | Scope | State | Rows | Loose |",
        "| :--- | :--- | :--- | ---: | ---: |",
    ]
    for f in sorted(features, key=lambda f: (STATES.index(f["derived"]), f["cite"])):
        lines.append(
            f"| `{f['cite']}` | {f['scope']} | {f['derived']} | {f['rows']} | {f['loose'] or ''} |"
        )
    rows = sum(f["rows"] for f in features)
    loose = sum(f["loose"] for f in features)
    lines += [
        "",
        f"**{len(features)} features: {tally['implemented']} implemented, {tally['partial']} partial, "
        f"{tally['design-only']} design-only. {rows} contract rows, {loose} loose.**",
        "",
    ]
    return "\n".join(lines)


def applications():
    """Every application that has feature documents: its directory and its slug."""
    for directory in sorted(SPECS.glob("*/features")):
        app = directory.parent
        yield app, app.name.split("-", 1)[-1].lower()


def main() -> int:
    check = "--check" in sys.argv
    write = "--write" in sys.argv
    errors: list[str] = []

    for app_dir, slug in applications():
        features = [read(p) for p in sorted((app_dir / "features").glob("*.md"))]
        if not features:
            continue
        errors += defects(slug, features)
        target = app_dir / f"ambre-{slug}-inventory.md"
        text = render(slug, features)
        rel = target.relative_to(ROOT)

        if write:
            target.write_text(text)
            print(f"written {rel}: {len(features)} features")
            continue

        current = target.read_text() if target.exists() else None
        if current is None:
            errors.append(f"{rel}: missing -- run `just inventory-write`")
        elif current != text:
            errors.append(
                f"{rel}: stale -- a feature document changed and the inventory did not. "
                f"Run `just inventory-write` and commit the result"
            )
        print(f"{rel}: {len(features)} features, "
              f"{sum(f['rows'] for f in features)} rows, "
              f"{sum(f['loose'] for f in features)} loose")

    if write:
        return 0
    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    print(f"\n{len(errors)} inventory defects")
    return 1 if (check and errors) else 0


if __name__ == "__main__":
    sys.exit(main())
