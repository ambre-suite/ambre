#!/usr/bin/env python3
"""Everything one contract row needs, and nothing else: the closure of an id.

Building one rule means reading the rule, the design that produced it, the rows
it must not break, and the mechanism that would catch it if it did. Today that
is a read of the whole document, plus every document it cites, and a reader
cannot tell which paragraph mattered. `ambre-spec-trame-cell-merges.md` is 5 700
words; one of its rows needs a few hundred of them.

So the closure is compiled, from the two artefacts of `contract_index.py`. The
index says where the row is and what it says; the graph says what else it
reaches. Only then is any Markdown opened, and only the documents those two
named. Nothing here re-walks the corpus.

**Seven blocks, one per question a builder has**, in the order they are asked.

1. What must be built -- the row, seven columns.
2. What it must not change -- the frame conditions of its table.
3. The operation it belongs to -- the other rows of that table.
4. What design governs it -- each section it cites, verbatim.
5. What other rules it depends on -- each row it cites, and each pillar.
6. How it is verified -- the mechanism, and what the mechanism leaves open.
7. What is missing -- the gap, and the story it lands with.

**The closure carries the sections the row cites, and no others.** It never
guesses a design section from the document's shape. A row whose design is not
reachable from its `Sources` is a row citing the wrong thing, and this file
reports it rather than reading around it. That is what makes the closure
falsifiable: what it misses is a defect with an address.

**It prints the code pointers and never the code.** A path into `crates/` is
what to open next, and inlining a file here would put a second copy of the
implementation in front of a reader who is about to change it.

It writes to standard output and never to a file, for the reason `spec_read.py`
gives: a closure checked into the tree is a copy of every rule, free to go
stale. The measurement goes to standard error, so the closure stays pipeable.

Run: `just spec-context <ID>`.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from functools import lru_cache
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contract_index  # noqa: E402  -- the two artefacts, and where they live
import contracts  # noqa: E402  -- the corpus has one contract parser
import spec_refs  # noqa: E402  -- the document and section namespaces
from template_vocabulary import KIND  # noqa: E402  -- the column name, as declared

# The kind whose whole job is to say what an operation leaves alone. It is read
# before the siblings, because a builder who reads a `POST` and stops has been
# told what to write and not what to preserve.
FRAME = "FRAME"

# A pillar row of `README.md`, quoted verbatim into the closure. An audit
# finding is located by `contracts.py`, which reads the three shapes declaring
# one; a second reader here would quote two of the three.
PILLAR_LINE = re.compile(r"^\|\s*\*\*([^*]+)\*\*\s*\|")

# What the closure counts as a word, for the measurement alone: a run of
# non-space. It is the measure `prose.py` uses, so the two figures compare.
WORD = re.compile(r"\S+")

# What opens a block of the closure. A cited section is quoted verbatim, wrapping
# `## 3. MODEL` and its subsections into the output, so a plain `## 4.` spine
# would be four headings deep in a document that already has one. `spec_refs.py`
# reads a section heading as `## <digit>`, so a word after `##` can never be a
# section of the corpus, and the spine stays unmistakable at a glance and to a
# tool. The quotation is never demoted: a heading rewritten on the way out is a
# closure that disagrees with the document it quotes.
BLOCK = "## CONTEXT"


class Missing(Exception):
    """An id, a document or a section the closure was sent to and cannot reach."""


@lru_cache(maxsize=None)
def corpus() -> tuple[dict[str, Path], dict[str, str]]:
    """The document namespace, read once.

    `--all` compiles 2 485 closures, and each one resolves a citation. Walking
    `specs/` per closure spends the walk this file exists to avoid.
    """
    known = spec_refs.documents()
    return known, spec_refs.short_names(known)


@lru_cache(maxsize=None)
def lines_of(path: Path) -> tuple[str, ...]:
    """One document's lines, read once per run."""
    return tuple(path.read_text().splitlines())


@lru_cache(maxsize=None)
def findings() -> dict[str, int]:
    """Where the audit declares each finding, read once."""
    return contracts.declared_findings()


@lru_cache(maxsize=None)
def schedule() -> dict[str, tuple[str, bool, int, str]]:
    """The roadmap, read once."""
    return contracts.read_schedule(contracts.SPECS / contracts.ROADMAP)


def load() -> tuple[dict[str, dict], list[dict]]:
    """The index by id, and the graph whole.

    A stale artefact is a wrong closure, silently: the row would be compiled
    from what the corpus said at the last write. `just index` holds the two
    files, and this refuses to run when it has not been run.
    """
    for path in (contract_index.INDEX, contract_index.GRAPH):
        if not path.exists():
            raise Missing(f"`{path.name}` is not generated. Run `just index-write`")
    records = json.loads(contract_index.INDEX.read_text())
    return {row["id"]: row for row in records}, json.loads(
        contract_index.GRAPH.read_text()
    )


def document(target: str, shorts: dict[str, str], known: dict[str, Path]) -> Path:
    """The file one citation names, given a path, a file name or a short name."""
    name = Path(target).name
    resolved = known.get(name) or known.get(shorts.get(target, ""))
    if resolved is None:
        raise Missing(f"no document named `{target}`")
    return resolved


def table_of(path: Path, rid: str) -> tuple[str, list[str], list[tuple[str, str]]]:
    """The heading, the header and every row of the table declaring `rid`.

    A table is the unit, not the document: the state kinds share one, and each
    operation has its own. Reading a postcondition without the frame condition
    beside it is how an operation gets built to spec and breaks its neighbour.
    """
    heading, opened = "", ""
    block: list[str] = []
    reading = False
    for line in lines_of(path) + ("",):
        if contracts.TABLE_HEAD.match(line):
            columns = contracts.header_cells(line)
            reading = KIND in columns
            opened, block = heading, [line] if reading else []
            continue
        row = contracts.ROW.match(line) if reading else None
        if row is not None:
            block.append(line)
            continue
        if reading and line.lstrip().startswith("|"):
            block.append(line)  # the separator under the header
            continue
        # The table is closed. It is the one the id lives in, or it is not.
        ids = [found for found in (contracts.ROW.match(kept) for kept in block)
               if found]
        if any(found["id"] == rid for found in ids):
            return opened, block, [(found["id"], found["rest"]) for found in ids]
        reading, block = False, []
        if line.startswith(("## ", "### ")):
            heading = line.strip()
    raise Missing(f"`{rid}` is declared by no table of `{path.name}`")


def section(path: Path, number: str) -> list[str]:
    """The body of `## <number>.`, its heading included, up to the next `##`."""
    kept: list[str] = []
    for line in lines_of(path):
        match = spec_refs.HEADING.match(line)
        if match:
            found = spec_refs.coordinate(match.group(1), match.group(2))
            if kept:
                break
            if found != number:
                continue
            kept.append(line)
        elif kept:
            kept.append(line)
    if not kept:
        raise Missing(f"`{path.name}` has no section {number}")
    return [line for line in kept if not line.startswith("---")]


def pillar(name: str) -> str:
    """The `README.md` row declaring one pillar, verbatim."""
    for line in (contracts.ROOT / "README.md").read_text().splitlines():
        match = PILLAR_LINE.match(line)
        if match and match.group(1).strip() == name:
            return line
    raise Missing(f"`README.md` declares no pillar `{name}`")


def finding(name: str) -> list[str]:
    """One audit finding, quoted from where the audit declares it.

    Three shapes declare a finding, and `contracts.py` reads all three, so the
    line comes from there rather than from a second reader. A heading opens a
    section and the section is quoted whole; a status row or a remediation story
    is one line, and one line is what a closed finding still has to say.
    """
    declared = findings()
    if name not in declared:
        raise Missing(f"no audit declares the finding `{name}`")
    lines = lines_of(contracts.SPECS / contracts.AUDIT)
    start = declared[name] - 1
    if not lines[start].startswith("### "):
        return [lines[start]]
    end = next((at for at in range(start + 1, len(lines))
                if lines[at].startswith(("## ", "### "))), len(lines))
    return list(lines[start:end])


def story(gap: str) -> list[str]:
    """The roadmap row a gap lands with, verbatim, with its epic."""
    cited = contracts.cited_story(gap)
    if cited is None:
        return []
    name = cited[0].strip('`"').strip()
    path = contracts.SPECS / contracts.ROADMAP
    found = schedule().get(contracts.story_key(name))
    if found is None:
        raise Missing(f"`{contracts.ROADMAP}` carries no story named `{name}`")
    epic, _, lineno, _ = found
    return [f"Epic {epic}:", lines_of(path)[lineno - 1]]


def pointers(blocks: list[str]) -> list[str]:
    """Every `crates/` path the closure already carries, once each, in order.

    The paths are collected from the closure and never from the document, so a
    pointer arrives only with the section that justified reading it.
    """
    found: list[str] = []
    for _, cited, path, _ in spec_refs.code_citations("\n".join(blocks)):
        del cited
        if path not in found:
            found.append(path)
    return found


def closure(rid: str, records: dict[str, dict], graph: list[dict]) -> list[str]:
    """The seven blocks, in the order a builder asks for them."""
    row = records.get(rid)
    if row is None:
        raise Missing(f"no contract row declares the id `{rid}`")
    known, shorts = corpus()
    own = contracts.SPECS / row["document"]
    heading, table, siblings = table_of(own, rid)
    mine = [edge for edge in graph if edge["from"] == rid]

    out = [f"# CONTEXT FOR `{rid}`", "", f"From `{row['document']}`, under {heading}.", ""]

    out += [f"{BLOCK} 1 -- WHAT MUST BE BUILT", ""]
    out += [f"* **Kind:** `{row['kind']}`", f"* **Status:** {row['status']}"]
    out += [f"* **Statement:** {row['statement']}", ""]

    frames = [(rid2, rest) for rid2, rest in siblings
              if records.get(rid2, {}).get("kind") == FRAME and rid2 != rid]
    out += [f"{BLOCK} 2 -- WHAT IT MUST NOT CHANGE", ""]
    out += [f"* `{rid2}` -- {records[rid2]['statement']}" for rid2, _ in frames] or [
        "No frame condition sits beside it. Its table binds state, or the "
        "operation declares none."
    ]
    out.append("")

    out += [f"{BLOCK} 3 -- THE OPERATION IT BELONGS TO", ""] + table + [""]

    out += [f"{BLOCK} 4 -- WHAT DESIGN GOVERNS IT", ""]
    cited = [edge for edge in mine
             if edge["edge"] in ("cites-section", "cites-document")]
    if not cited:
        kinds = {source["kind"] for source in row["sources"]}
        out += ["Every source reads `derived`: the row claims no design but the "
                "one it follows from." if kinds == {"derived"} else
                "The row names sources and cites no section, so no design text "
                "is reachable from it. That is a defect of the row.", ""]
    for edge in cited:
        path = own if edge["edge"] == "cites-section" else document(
            edge["to"], shorts, known
        )
        number = edge.get("section", "")
        if not number:
            out += [f"**`{path.name}`, cited whole, with no section:**", ""]
            continue
        out += [f"**`{path.name}` section {number}:**", ""]
        out += section(path, number.split(".")[0]) + [""]

    out += [f"{BLOCK} 5 -- WHAT OTHER RULES IT DEPENDS ON", ""]
    depends = [edge["to"] for edge in mine if edge["edge"] == "cites-rule"]
    out += [f"* `{name}` (`{records[name]['kind']}`, {records[name]['status']}) -- "
            f"{records[name]['statement']}" for name in depends if name in records]
    for edge in mine:
        if edge["edge"] == "cites-pillar":
            out.append(f"* README pillar: {pillar(edge['to'])}")
    for edge in mine:
        if edge["edge"] == "cites-finding":
            out += ["", f"**Audit finding `{edge['to']}`:**", ""] + finding(edge["to"])
    if not depends and not any(
        edge["edge"] in ("cites-pillar", "cites-finding") for edge in mine
    ):
        out.append("None. The row stands on its own design alone.")
    out.append("")

    out += [f"{BLOCK} 6 -- HOW IT IS VERIFIED", "",
            f"{row['verification'] or 'none'}", ""]

    out += [f"{BLOCK} 7 -- WHAT IS MISSING", "", f"{row['gap'] or 'none'}", ""]
    out += story(row["gap"])
    out.append("")

    found = pointers(out)
    out += [f"{BLOCK} -- WHERE TO GO NEXT", ""]
    out += [f"* `{path}`" for path in found] or [
        "No section of the closure points into `crates/`."
    ]
    return out


def words(lines: list[str]) -> int:
    return len(WORD.findall("\n".join(lines)))


def measure(rid: str, out: list[str], records: dict[str, dict],
            graph: list[dict]) -> str:
    """The closure against a full read of every document it draws from."""
    known, shorts = corpus()
    touched = {contracts.SPECS / records[rid]["document"]}
    for edge in graph:
        if edge["from"] == rid and edge["edge"] == "cites-document":
            try:
                touched.add(document(edge["to"], shorts, known))
            except Missing:
                continue
    whole = sum(words(list(lines_of(path))) for path in touched)
    here = words(out)
    share = round(100 * here / whole) if whole else 0
    read = "document" if len(touched) == 1 else "documents"
    return (f"{here} words, against {whole} in the {len(touched)} {read} it "
            f"draws from: {share} percent")


def survey(records: dict[str, dict], graph: list[dict]) -> int:
    """Compile every closure of the corpus, and report what they cost.

    The compiler has to pay for itself, and one closure cannot say whether it
    does: a row citing one section looks cheap, and a row citing four documents
    is the case that decides. So the figure is the whole corpus, and a closure
    that fails to compile is counted rather than hidden.
    """
    shares, refused = [], []
    for rid in records:
        try:
            out = closure(rid, records, graph)
        except Missing as refusal:
            refused.append(f"{rid}: {refusal}")
            continue
        shares.append(int(measure(rid, out, records, graph).split(": ")[1].split()[0]))
    shares.sort()
    for line in refused:
        print(f"ERROR {line}")
    print(f"\n{len(shares)} closures compiled, {len(refused)} refused.")
    print(f"Share of a full read: {shares[0]} percent at the least, "
          f"{shares[len(shares) // 2]} at the median, {shares[-1]} at the most.")
    return 1 if refused else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("id", nargs="?", help="a contract id, such as `TRAME-125`")
    parser.add_argument("--all", action="store_true",
                        help="compile every closure and report the cost alone")
    args = parser.parse_args()

    try:
        records, graph = load()
        if args.all:
            return survey(records, graph)
        if args.id is None:
            raise Missing("no id given, and `--all` not asked for")
        rid = args.id.strip("`")
        out = closure(rid, records, graph)
    except Missing as refusal:
        print(f"ERROR {refusal}", file=sys.stderr)
        return 1

    print("\n".join(out))
    print(measure(rid, out, records, graph), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
