#!/usr/bin/env python3
"""The corpus parsed once: one record per contract row, and the edges between them.

`ambre-contract-ledger.md` is a reading view. It answers "how much is verified
today" and it answers nothing else: the rows are rolled up per document and per
status, and the statement, the sources and the mechanism are gone by the time it
is written. Anything that needs a row -- a reader, a review, a context compiler
-- re-parses every document carrying a table to get it back.

So the parse is emitted, twice.

**The index** is the row as written, taken apart: its id, its document, its kind,
its statement, its sources parsed by the grammar of `contracts.py`, its mechanism,
its gap and its status. Nothing is derived and nothing is summed. A consumer reads
one file instead of walking the corpus, and two consumers cannot disagree about
what a row says.

**The graph** is the join the corpus already asserts and nowhere states: a row
cites a rule, a document, a section, a pillar or an audit finding; a built row
names the test that holds it; a row with a gap that can close names the Story it
lands with. Every edge here is resolved by a gate already -- `contracts.py` for
the rule, the finding, the mechanism and the Story, `spec_refs.py` for the
document and the section -- except the pillar, which nothing resolved until now.
The graph is where that join is a fact rather than a scan.

**Neither file is a source of truth, and neither carries a coordinate.** The
Markdown is authoritative, exactly as it is for the ledger, and a stale artefact
fails rather than being read. No record carries a line number: a line is the one
part of a citation an edit above it breaks, which is what
`ambre-spec-template.md` section 1.G refuses for `crates/`. A document and an id
locate a row, and the id is unique corpus-wide.

**A row cites no container.** The plan this file closes asked for that edge too.
Template section 2 is what refuses it: the Contract "names no container, no type
and no function", so a container is cited by a `3. MODEL` section and belongs to
a document, never to a row. `spec_shape.py` holds the container table against the
code, and an edge from a row would have no tail to attach to.

Run: `just index` (fail when the files on disk differ), `just index-write`.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contracts  # noqa: E402  -- the row grammar, read once and never copied
import corpus  # noqa: E402  -- the one reader of the Markdown
import spec_refs  # noqa: E402  -- the document and section namespaces
from template_vocabulary import (  # noqa: E402
    GAP,
    KIND,
    MECHANISM,
    SOURCES,
    STATEMENT,
)

INDEX = contracts.SPECS / "1-Foundations" / "ambre-contract-index.json"
GRAPH = contracts.SPECS / "1-Foundations" / "ambre-contract-graph.json"

# A pillar of `README.md`, declared in the one table that carries them. The bold
# run is the name a `Sources` cell quotes verbatim.
PILLAR_ROW = re.compile(r"^\|\s*\*\*([^*]+)\*\*\s*\|")

# A finding id inside an `audit` source's target, which may name several.
FINDING = re.compile(r"`([A-Z]\d{1,3})`")


def read_rows():
    """Yield `(document, id, status, cells)` for every contract row of the corpus.

    The rows are the corpus's, read by the parser every gate shares, so the
    index holds the rows the contract gate checks: a table is opened by a header
    declaring `ID` and `Kind`, and a blank line inside it closes nothing.
    """
    for path in corpus.markdown(contracts.SPECS):
        if path.name == contracts.TEMPLATE:
            continue  # example rows, not contracts
        rel = str(path.relative_to(contracts.SPECS))
        for row in corpus.read(path).contract_rows:
            if row.table.contract:
                yield rel, row.id, row.status, row.cells


def record(document: str, rid: str, status: str, cells: dict[str, str]) -> dict:
    """One row, as written, with its `Sources` cell taken apart.

    The kind is unwrapped from its backticks because the backticks are a rule
    about the Markdown, held by `contracts.py`, and not a part of the value.
    """
    kind = contracts.KIND_CELL.match(cells.get(KIND, ""))
    parsed = [
        contracts.parse_source(source)
        for source in cells.get(SOURCES, "").split(";")
    ]
    return {
        "id": rid,
        "document": document,
        "kind": kind.group(1) if kind else "",
        "statement": cells.get(STATEMENT, ""),
        "sources": [
            {
                "kind": source.kind,
                "target": source.target,
                "locator": source.locator,
                "rules": list(source.rules),
                "note": source.note,
            }
            for source in parsed
            if source is not None
        ],
        "mechanism": cells.get(MECHANISM, ""),
        "gap": cells.get(GAP, ""),
        "status": status,
    }


def index() -> list[dict]:
    """Every contract row of the corpus, in the order the corpus carries them."""
    return [record(*row) for row in read_rows()]


def edges(records: list[dict]) -> list[dict]:
    """Every edge the rows assert, one per citation.

    A `derived` source yields none, which is what `derived` means: the row claims
    no source but the design. Every other source yields exactly one edge for what
    it points at, plus one per rule id it carries, because a document citation
    naming two ids is two joins and not one.
    """
    found = []
    for row in records:
        rid, document = row["id"], row["document"]
        for source in row["sources"]:
            kind, target, locator = source["kind"], source["target"], source["locator"]
            if kind == "own":
                found.append({"from": rid, "edge": "cites-section",
                              "to": document, "section": locator})
            elif kind == "doc":
                edge = {"from": rid, "edge": "cites-document", "to": target}
                if locator:
                    edge["section"] = locator
                found.append(edge)
            elif kind == "pillar":
                found.append({"from": rid, "edge": "cites-pillar", "to": target})
            elif kind == "audit":
                found += [{"from": rid, "edge": "cites-finding", "to": name}
                          for name in FINDING.findall(target)]
            found += [{"from": rid, "edge": "cites-rule", "to": cited}
                      for cited in source["rules"]]
        found += [{"from": rid, "edge": "verified-by", "to": token}
                  for token in contracts.CITED.findall(row["mechanism"])]
        story = contracts.cited_story(row["gap"])
        if story is not None:
            found.append({"from": rid, "edge": "lands-with",
                          "to": story[0].strip('`"').strip()})
    return found


def declared_pillars() -> set[str]:
    """The pillars `README.md` declares, by name.

    The one source form nothing resolved. A `Sources` cell quotes the name
    verbatim -- `README pillar "Zero-Knowledge"` -- and the README table is where
    that name is minted, so a renamed pillar leaves every citation of it pointing
    at a row that no longer exists.
    """
    readme = corpus.read(contracts.ROOT / "README.md")
    return {match.group(1).strip() for table in readme.tables for row in table.rows
            if (match := PILLAR_ROW.match(row.text))}


def edge_defects(records: list[dict], found: list[dict]) -> list[str]:
    """Every edge whose target does not exist, one line each.

    Six of the seven kinds are resolved by a gate already, and this resolves them
    a second time on purpose: the graph is read by a consumer that never runs
    those gates, and an edge it cannot follow is worse than no graph at all.
    """
    declared = {row["id"] for row in records}
    documents = spec_refs.documents()
    shorts = spec_refs.short_names(documents)
    sections = {name: spec_refs.sections(path) for name, path in documents.items()}
    pillars = declared_pillars()
    findings = contracts.declared_findings()
    schedule = contracts.read_schedule(contracts.SPECS / contracts.ROADMAP)
    per_crate, pooled = contracts.declared_fns()

    def named(target: str) -> str | None:
        """The file name one document citation claims, path or short name."""
        name = Path(target).name
        if name in documents:
            return name
        return shorts.get(target)

    defects = []
    for edge in found:
        kind, target = edge["edge"], edge["to"]
        where = f"{edge['from']} -- {kind} -> {target}"
        if kind == "cites-rule" and target not in declared:
            defects.append(f"{where}: no row declares that id")
        elif kind == "cites-pillar" and target not in pillars:
            defects.append(f"{where}: `README.md` declares no such pillar")
        elif kind == "cites-finding" and target not in findings:
            defects.append(f"{where}: the audit declares no such finding")
        elif kind == "lands-with" and contracts.story_key(target) not in schedule:
            defects.append(f"{where}: `ambre-roadmap.md` carries no such Story")
        elif kind == "verified-by":
            defects += [f"{where}: {why}" for why in
                        contracts.unresolved_citations(f"`{target}`", per_crate, pooled)]
        elif kind in ("cites-document", "cites-section"):
            name = named(target)
            if name is None:
                defects.append(f"{where}: no document carries that name")
                continue
            number = edge.get("section", "").split(".")[0]
            if number and sections.get(name) and number not in sections[name]:
                defects.append(f"{where}: that document has no section {number}")
    return defects


def render(payload: list[dict]) -> str:
    """One record per line, inside one JSON array.

    Valid JSON that a reader diffs by row. An indented dump spreads one row over
    fifteen lines, so a one-word edit to a statement reads as a block move, and a
    single-line dump makes every edit the whole file.
    """
    body = ",\n".join("  " + json.dumps(item, sort_keys=True) for item in payload)
    return f"[\n{body}\n]\n"


def main() -> int:
    records = index()
    found = edges(records)
    defects = edge_defects(records, found)
    for defect in defects:
        print(f"ERROR {defect}")
    artefacts = ((INDEX, render(records)), (GRAPH, render(found)))
    if "--write" in sys.argv:
        for path, text in artefacts:
            path.write_text(text)
            print(f"wrote {path.relative_to(contracts.ROOT)}")
        return 1 if defects else 0
    stale = [path for path, text in artefacts
             if not path.exists() or path.read_text() != text]
    for path in stale:
        print(f"ERROR {path.relative_to(contracts.SPECS)}: stale")
    print(f"\n{len(records)} rows, {len(found)} edges, "
          f"{len(defects)} edges resolving to nothing")
    if stale:
        print(
            "\nERROR: a generated artefact on disk is not what the corpus says "
            "today (ambre-spec-template.md section 1.G). Run `just index-write`.",
            file=sys.stderr,
        )
        return 1
    if defects:
        print(
            "\nERROR: an edge of the graph resolves to nothing "
            "(ambre-spec-template.md section 1.G). Point the citation at what "
            "holds the rule today.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
