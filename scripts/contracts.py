#!/usr/bin/env python3
"""Generate the contract ledger from the spec tables, and check their rules.

`ambre-contract-ledger.md` is the roll-up of every per-spec contract table. A
hand-maintained roll-up is the same failure mode as a hand-maintained test list:
it is believed, and it is wrong. So the file is written by this script and by
nothing else, like each application's feature inventory. Nobody reads it; CI
reads it.

The rules it enforces, beyond generating the file:

  * a status must be one of the seven declared values;
  * **a rule half held is `partial`, never `untested`.** The status names a
    mechanism and a `Gap` opening on `partial -- `, so the ledger tells a row
    whose wiring is loose from a row nothing tests;
  * a `partial`, `failing`, `untested` or `unimplemented` row must name the Story that closes
    its gap ("lands with");
  * **that Story must resolve to an open row of `ambre-roadmap.md`.** The template
    says "that story is not `[x]` until the row's status has moved". Reading the
    substring "lands with" and never the roadmap leaves a story free to be
    ticked, renamed or moved under another epic while every row aimed at it
    points at nothing. The name is the join key, hence
    three failures: a name no row carries, a name two rows carry, and a name on a
    row that is `[x]` or that closed into `ambre-roadmap-journal.md`. The epic in
    the citation is checked too when it is there, so a moved story cannot leave a
    stale key behind;
  * **every mechanism a built row cites must resolve to a `fn` in the workspace.**
    The corpus demands it -- "a test name that resolves to a `fn` behind a Cargo
    feature no job enables is the same claim as a name that resolves to nothing".
    A batch cleaned by hand is free to come back, and a `verified` row citing a
    `fn` that never existed reads exactly like one citing a test that runs. An
    `unimplemented` row is exempt: its `Mechanism` cell reads `not built`, and
    the Story it lands with is in `Gap`;
  * **every audit finding a row cites must exist, and every finding must be
    cited.** The template makes the citation the join between a finding and the
    rule that closes it -- "the finding stops drifting from the code in silence".
    Both ends rot unheld: a row cites `audit #3`, which is not a finding id, and
    a finding a remediation wave closed carries no rule, so nothing goes red the
    day the fix is reverted. A finding with no rule to state is declared in
    `NO_RULE` below, with its reason;
  * **the grammar of a row's own cells**, which a review holds and nothing else
    does. A kind written bare where the template writes it in backticks splits
    one column into two spellings, and a check comparing stripped text passes on
    both. A `Statement` with no RFC 2119 keyword reads as description and binds
    nothing. A `Sources` cell citing a form the template does not declare --
    `pillar "Local-First"` for `README pillar "Local-First"` -- is a citation
    nobody can resolve. Those three are errors here. A `Statement` carrying two
    rules is the fourth, below: the line between a bundle the kind licenses and a
    conjunction that hides a second rule is drawn by the kind, not by a count;
  * **a `Statement` states one rule.** `FAIL` binds "the refusal, the notice,
    the rollback" and `FRAME` names everything an operation must not touch, so
    both legitimately carry a list; the other six kinds do not. A second MUST
    after an `and` is two rules under one status, and the half nothing tests
    breaks in green;
  * **a rule each application holds names a test in each one.** A row labelling
    its tests `Prose`, `Trame` or `Toile` sits in a document declaring
    `**Applications:**`, and is `verified` only once every declared application
    has its test. One test of the shared crate proved `KINETIC-002` while Toile
    broke it;
  * an id is never reused, and a prefix has no gap in its numbering;
  * **every gap that can close names its Story.** A `partial`, `failing`, `untested` or
    `unimplemented` row lands with a Story of the roadmap, and that Story must
    resolve to one open row of it. A per-document ceiling on `unverified` sums
    three states nobody can compare: a rule the code breaks today reads as a
    missing test, and the debt sits in a JSON file under `scripts/` rather than
    in the one place a schedule lives. A row whose
    gap cannot close carries `qa-manual` or `human-review`, which owe no Story:
    they are decisions, not debt;
  * **a gap that cannot close names its obstacle.** "Cannot" is a fact about
    the tooling on one day. The `Gap` of a `qa-manual` or `human-review` row
    names one obstacle of the testing document, and an obstacle a Story lifts
    fails every row still citing it, so the requalification is listed for the
    reader rather than remembered. An obstacle no row cites is refused, and so
    is a lifted one whose Story has shipped: the table holds only live reasons;
  * **the counts under a table are the table's own.** The `Verification` answer
    opens on counts `--write` derives from the table and writes over the head of
    the bullet, keeping the prose after it. A count by hand drifts: a document
    claiming one `verified` row fewer than it carries claims it in prose nobody
    recounts. Without `--write`, a count off the table is refused as stale. The
    `human-review` table of `CONTRIBUTING.md` is written from the rows the same
    way. The presence of the label stays `spec_shape.py`,
    which holds the three answers together.

What this still cannot see: a row citing a test that runs, passes, and does not
test the rule. That needs a probe or `cargo-mutants`, not a parser.

Run: `just contracts` (fail when the file on disk is not what this would
write), `just contracts-write` (rewrite it). The check compares the whole text,
so a spec edited without a regeneration turns CI red instead of leaving a stale
roll-up behind.
"""

import json
import re
import sys
import textwrap
from collections import Counter, defaultdict
from pathlib import Path
from typing import NamedTuple

# Every closed vocabulary this file checks against is the template's, read from
# it. What each one means is stated there, beside the table it is read from, and
# is not repeated here (`ambre-spec-template.md section 1`).
from template_vocabulary import (  # noqa: F401  -- re-exported to the other gates
    BARE_GAP_OPENERS,
    BARE_TOKENS,
    CARRIED_GAP_OPENERS,
    GAP,
    GAP_OPENERS,
    KIND,
    KINDS,
    LOCATED_TOKENS,
    MECHANISM,
    MECHANISM_TOKENS,
    NUMBERS,
    OPERATION_KINDS,
    SOURCES,
    STATE_KINDS,
    STATEMENT,
    STATUSES,
)

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"
CRATES = ROOT / "crates"
TEMPLATE = "ambre-spec-template.md"  # carries example rows, not real contracts
LEDGER = "1-Foundations/ambre-contract-ledger.md"  # generated here, nowhere else
ROADMAP = "ambre-roadmap.md"  # the one schedule: every Story is a row of it
JOURNAL = "ambre-roadmap-journal.md"  # closed epics, cut there verbatim
AUDIT = "audits/security-audit.md"  # numbered findings, cited by the rows that close them
LOOSE = ("failing", "untested", "human-review", "qa-manual")  # built, nothing goes red
# The three that are debt: each owes an open Story of the roadmap.
OWES_STORY = ("partial", "failing", "untested", "unimplemented")
APPS = ("1-Foundations", "2-Trame", "3-Toile", "4-Prose", "5-Murmure")

# An id minted once, then withdrawn because the rule it carried left the
# application for a shared document. The id is **never reused** -- that is what
# this register buys -- and the gap check below reads it so a withdrawal is
# declared here rather than passing as a numbering hole. Each entry names the
# shared row that carries the rule today, so the trail from an old citation is
# one lookup. Nothing else may be entered: a row whose rule is still this
# application's is narrowed or split, never withdrawn -- unless another row of
# the same document already states it, which is the only other entry this
# register takes.
RETIRED = {
    # Withdrawn into the shared row that states the same rule word for word,
    # found by `twin_defects`: each application held a copy of it.
    "TRAME-026": "SCHEMA-018",
    "TOILE-028": "SCHEMA-018",
    "PROSE-029": "SCHEMA-018",
    "TRAME-225": "SCHEMA-019",
    "TOILE-258": "SCHEMA-019",
    "PROSE-103": "SCHEMA-019",
    "TRAME-UI-003": "UI-THREAD-002",
    "TOILE-UI-311": "UI-THREAD-002",
    "PROSE-UI-003": "UI-THREAD-002",
    "TRAME-152": "UNDO-010",
    "TOILE-095": "UNDO-010",
    "TRAME-121": "UNDO-011",
    "TOILE-096": "UNDO-011",
    "TRAME-UI-060": "UI-THREAD-044",
    "TOILE-216": "SYNC-101",
    "PROSE-049": "SYNC-101",
    "PROSE-020": "PRESENCE-003",
    "PROSE-021": "PRESENCE-004",
    "PROSE-UI-011": "PRESENCE-001, PRESENCE-003, PRESENCE-005",
    "PROSE-UI-012": "UI-THREAD-001",
    "TRAME-UI-016": "PRESENCE-006",
    "TRAME-UI-017": "PRESENCE-005",
    "TRAME-UI-018": "PRESENCE-004",
    "TOILE-UI-072": "PRESENCE-006",
    "TRAME-UI-066": "UI-THREAD-009",
    "TRAME-122": "UI-THREAD-016",
    "TOILE-077": "UI-THREAD-016",
    # Withdrawn as a duplicate: each stated, as a precondition, a guard the
    # operation checks and refuses, which its `FAIL` row already stated. A
    # checked guard is one row. This is the second reason the register admits.
    "TRAME-129": "TRAME-019",
    "TRAME-134": "TRAME-137",
    "TRAME-139": "TRAME-143",
    "TRAME-018": "UNDO-001",
    "TRAME-116": "UNDO-002",
    "TRAME-117": "UNDO-005",
    "TRAME-118": "UNDO-003",
    "TRAME-119": "UNDO-004",
    "TRAME-120": "UNDO-006",
    "TOILE-015": "UNDO-001",
    "TOILE-072": "UNDO-005",
    "TOILE-073": "UNDO-002",
    "TOILE-074": "UNDO-003",
    "TOILE-075": "UNDO-006",
    "TOILE-076": "UNDO-004",
    "PROSE-022": "UNDO-001",
    "PROSE-FEAT-012": "UNDO-001",
    "PROSE-FEAT-032": "UNDO-002",
    "TRAME-IO-020": "CANVAS-028",
    "TRAME-IO-024": "PDF-004",
    "TRAME-UI-022": "CANVAS-028",
    "TRAME-UI-023": "PDF-006",
    "TOILE-IO-032": "PDF-002",
    "TOILE-IO-033": "PDF-005",
    "PROSE-IO-015": "PDF-001",
    "PROSE-IO-030": "PDF-005",
    "TRAME-UI-075": "FIND-002",
    "TRAME-UI-076": "FIND-003",
    "TRAME-UI-081": "FIND-007",
    "TRAME-UI-082": "FIND-006",
    "TRAME-UI-084": "FIND-001",
    "TOILE-019": "FIND-001",
    "TOILE-078": "FIND-002",
    "TOILE-081": "FIND-010",
    "PROSE-FEAT-015": "FIND-006",
    "PROSE-FEAT-016": "FIND-001",
    "PROSE-FEAT-037": "FIND-004",
    # Withdrawn to `ambre-spec-find.md`: each restated a rule the shared document
    # states for every application, which is the second reason this register
    # admits.
    "TRAME-UI-137": "FIND-008",
    "TOILE-141": "FIND-010",
    "TOILE-146": "FIND-005",
    "TOILE-147": "FIND-003",
    "TOILE-148": "FIND-003",
    "PROSE-FEAT-147": "FIND-001",
    "PROSE-FEAT-149": "UI-THREAD-020",
    "PROSE-FEAT-154": "FIND-007",
    "TOILE-IO-005": "PDF-004",
    "TRAME-IO-032": "PDF-001",
    "PROSE-IO-034": "PDF-006",
    "PROSE-IO-036": "CANVAS-028",
    "PROSE-102": "SCHEMA-017",
    "TRAME-224": "SCHEMA-017",
    "TOILE-257": "SCHEMA-017",
}

# A finding that states no rule, and why. It is the `RETIRED` register applied to
# the audit: the coverage check below reads it, so a finding lands here by
# decision instead of passing as a hole. Nothing else belongs here -- a finding
# about behaviour is a row in the spec of the component it constrains.
NO_RULE = {
    "C3": "false positive: the comparison it reports is already constant-time",
    "M8": "a module declared and unreachable, which states no behaviour to hold",
}

# What a Sources cell writes to cite a finding: ``audit `H9` ``, or a list of
# them. The ids inside are read back and checked, so ``audit `#3` `` fails as a
# malformed citation rather than resolving to nothing in silence.
AUDIT_CITE = re.compile(r"audit\s+(`[^`]+`(?:\s*,\s*`[^`]+`)*)")
FINDING = re.compile(r"^[CHMN]\d{1,2}$")
# Where the audit declares one: a section heading, a row of a status table, or a
# story of a remediation wave. A finding named in running prose is a mention.
FINDING_HEAD = re.compile(r"^###\s+(.*)$")
FINDING_ROW = re.compile(r"^\|\s*`?([CHMN]\d{1,2})`?\s*\|")
FINDING_STORY = re.compile(r"^-\s*\[[ x]\]\s*\*\*([CHMN]\d{1,2})\*\*")

ROW = re.compile(
    r"^\|\s*`(?P<id>[A-Z][A-Z0-9-]*-\d{3})`\s*\|"
    r"(?P<rest>.*)\|\s*(?P<status>[a-z-]+)\s*\|\s*$"
)

# The header of a contract table. A table keyed by `ID` that declares `Kind` is
# a contract table, and it MUST declare `Gap` too: the mechanism and what the
# mechanism leaves open are two columns, and a header that merges them back into
# one is refused (`ambre-spec-template.md section 1.B`).
TABLE_HEAD = re.compile(r"^\|\s*ID\s*\|")
# `**Frame:** `UI-THREAD-044`` on the line that closes an operation table: the
# operation leaves alone what a shared `FRAME` row states, so it cites that row
# instead of copying its statement under a second id.
FRAME_LINE = re.compile(r"^\*\*Frame:\*\*\s*(?P<ids>.+)$")
FRAME_ID = re.compile(r"`([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*-\d+)`")


def header_cells(header: str) -> list[str]:
    """The column names one table header declares, in order.

    The names, not the template's order: pinning a column to its place makes a
    reordered header opt the document out of every check below in silence, which
    is the failure mode this whole file exists to refuse.
    """
    return [c.strip() for c in header.strip().strip("|").split("|")]


def row_cells(columns: list[str] | None, rest: str) -> dict[str, str]:
    """One row's cells, keyed by the name its own header declares.

    `rest` starts after the `ID` cell, so a header index shifts by one. A cell
    read by its position is what a new column breaks in silence: "the last one
    before the status" names a different cell the day a column is inserted
    there, and no check sees the shift (`ambre-spec-template.md section 1.B`).
    """
    if not columns:
        return {}
    cells = [c.strip() for c in rest.split("|")]
    return {
        name: cells[at - 1] for at, name in enumerate(columns) if 0 < at <= len(cells)
    }


# `STATE_KINDS` and `OPERATION_KINDS` come from the template's `Binds` column:
# a state kind holds at every instant, an operation kind at one point of one
# operation's run. The two families never share a table, which is what makes a
# missing row visible, and `table_defects` below is where that is refused.
#
# Both families carry a list by right: a `FAIL` binds the refusal, the notice and
# the rollback as one outcome, and a `FRAME` names everything one operation must
# not touch. The conjunction measure below leaves them alone.
BUNDLING_KINDS = ("FAIL", "FRAME")

# How the template writes a `Kind` cell: the word, in backticks, and nothing
# else. Comparing the word alone lets a row drop the backticks and pass -- a
# vocabulary check that forgives the spelling checks half a column.
KIND_CELL = re.compile(r"^`([A-Z]+)`$")

# What a `Statement` must carry to bind anything: one of the RFC 2119 keywords
# the template declares. A row without one describes, and a description cannot
# be falsified, so it cannot carry a status.
RFC2119 = re.compile(r"\bMUST(?:\s+NOT)?\b|\bSHOULD(?:\s+NOT)?\b|\bMAY\b")

# A second RFC 2119 keyword, behind an `and`, in a kind that does not license a
# list: "MUST paint above the content, and MUST follow the gesture" is two rules
# under one status, so the half nothing tests breaks in green. Refused: the
# corpus is split, and the row that comes back is split before it lands.
CONJUNCTION = re.compile(
    r"\b(?:MUST(?:\s+NOT)?|SHOULD(?:\s+NOT)?|MAY)\b[^.;:]*?"
    r",?\s+and\s+(?:it\s+)?(?:MUST|SHOULD|MAY)\b"
)

# The same shape, with a subject between the `and` and the second keyword: "MUST
# be dropped, and the others MUST be told" is the rule above with four words in
# it, and the rule above reads none of them. A report and not a refusal: whether
# a match is two obligations or one rule stated twice is a reading, and a gate
# that guesses it refuses rows that are right.
SPLIT_CANDIDATE = re.compile(
    r"\b(?:MUST(?:\s+NOT)?|SHOULD(?:\s+NOT)?|MAY)\b[^.;:]*?"
    r",?\s+and\s+[^.;:]{0,40}?\b(?:MUST|SHOULD|MAY)\b"
)

# The rule id a `Sources` cell cites, on its own or beside the document that
# declares it. An id is unique in the corpus, so one resolves to one row without
# a search, which is what the ledger's index exists for. A prefix carries its own
# dashes -- `PROSE-FEAT-030` -- so the number is the last segment and nothing
# else.
SOURCE_ID = re.compile(r"`([A-Z][A-Z0-9]*(?:-[A-Z]+)*-\d{1,4})`")

# The pieces a source is built from. A document is named or pathed, and a short
# name is a form of its own that `spec_refs.py` resolves against the corpus, so
# `architecture section 2` is a citation and not prose. A section number may
# carry a letter, which `spec_refs.py` reads the number of and no more.
_ID = r"[A-Z][A-Z0-9]*(?:-[A-Z]+)*-\d{1,4}"
_FINDING = r"[A-Z]\d{1,3}"
_NAME = r"[\w][\w./-]*"
_SECTION = r"\d{1,2}(?:\.[A-Z])?"
_IDS = rf"`{_ID}`(?:,\s+`{_ID}`)*"
_FINDINGS = rf"`{_FINDING}`(?:,\s+`{_FINDING}`)*"
# What a source may carry past its target: a parenthesis, a comma clause, or
# both. The parenthesis holds the rule ids a document citation points at; the
# clause holds the prose that locates the rule inside the section.
_TAIL = r"(?:\s*\((?P<paren>[^)]*)\))?(?:,\s+(?P<note>.+))?"
# A comma clause that is nothing but ids is the citation itself, not prose about
# it: `` `ambre-spec-zero-knowledge.md`, `ZK-002` `` is how a source names the
# rule it points at, and reading that clause as a note loses the ids in it.
CITED_CLAUSE = re.compile(rf"^{_IDS}$")

# The grammar of one source, and the whole of it. Six forms, each naming its
# kind, its target and, where it has one, its locator: what a reader opens, and
# where in it they land. What this replaces was five `search()` patterns over
# the cell taken whole, and a cell passed when **any** of its sources matched
# **any** pattern anywhere inside it. Three things hid in that gap. A bare
# `` `ZK-001` `` matched none of the five and rode 95 times on the section
# citation beside it, so the form the corpus writes most after a section was
# never declared. `` `ambre-spec-ui.md` (design tokens) `` passed on the
# document and the parenthesis was never read. And `architecture section 2.D`,
# a citation of another document, was typed as a section of this one, because
# the pattern that matched it looks for `section` and not for what precedes it.
#
# Order matters: `derived` and `audit` are keywords, so they are tried before
# the forms that would read them as a document name.
SOURCE_GRAMMAR = (
    ("derived", re.compile(rf"^derived{_TAIL}$")),
    ("pillar", re.compile(rf'^README pillar "(?P<target>[^"]+)"{_TAIL}$')),
    ("audit", re.compile(rf"^audit\s+(?P<target>{_FINDINGS}){_TAIL}$")),
    ("rule", re.compile(rf"^(?P<target>{_IDS}){_TAIL}$")),
    (
        "doc",
        re.compile(
            rf"^`(?P<target>{_NAME})(?:\s+section\s+(?P<loc>{_SECTION}))?`"
            rf"(?:\s+section\s+(?P<loc2>{_SECTION}))?{_TAIL}$"
        ),
    ),
    ("doc", re.compile(rf"^(?P<target>{_NAME})\s+section\s+(?P<loc>{_SECTION}){_TAIL}$")),
    ("own", re.compile(rf"^(?:this spec\s+)?section\s+(?P<loc>{_SECTION}){_TAIL}$")),
)

SOURCE_KINDS = tuple(dict.fromkeys(kind for kind, _ in SOURCE_GRAMMAR))


class Source(NamedTuple):
    """One entry of a `Sources` cell, taken apart.

    `target` is what a reader opens -- a document name, a pillar, a finding
    number -- and is empty for the two forms that name nothing outside the row:
    `own`, which locates a section of the citing document, and `derived`, which
    claims no source but the design. `rules` are the contract ids the source
    cites, whether it cites them bare or in the parenthesis after a document.
    """

    kind: str
    target: str
    locator: str
    rules: tuple[str, ...]
    note: str


def parse_source(source: str) -> Source | None:
    """One source read into its kind, its target and its locator, or `None`.

    `None` is a source no form of the grammar accepts, which is the whole of
    the refusal: a citation a reader cannot follow, and a compiler cannot
    resolve, whatever it means to the person who wrote it.
    """
    for kind, form in SOURCE_GRAMMAR:
        match = form.match(source.strip())
        if match is None:
            continue
        found = match.groupdict()
        target = found.get("target") or ""
        note = found.get("note") or ""
        rules = SOURCE_ID.findall(found.get("paren") or "")
        if kind == "rule":
            rules = SOURCE_ID.findall(target)
        elif CITED_CLAUSE.match(note):
            rules += SOURCE_ID.findall(note)
            note = ""
        return Source(
            kind=kind,
            target=target,
            locator=found.get("loc") or found.get("loc2") or "",
            rules=tuple(rules),
            note=note,
        )
    return None


def sources_cited(sources: str) -> list[tuple[str, list[str]]]:
    """One entry per id a `Sources` cell cites, with the documents beside it.

    The documents are the ones of that source alone. An empty list means the
    source cites the id bare, which resolves through the ledger index and is
    legitimate. The grammar is what splits them now: an id the note mentions in
    passing is prose, and pairing an id with the document of the source beside
    it reads five true citations as lies.
    """
    cited = []
    for source in sources.split(";"):
        parsed = parse_source(source)
        if parsed is None:
            continue  # the grammar refusal names it; one defect per source
        docs = [parsed.target] if parsed.kind == "doc" else []
        cited += [(rid, docs) for rid in parsed.rules]
    return cited


def grammar_defects(where: str, sources: str) -> list[str]:
    """Whether every source of one cell parses, source by source.

    The cell is a `;`-separated list and each entry is a citation on its own,
    so each one is read on its own. A cell that passed because the entry beside
    it matched is the looseness this replaced.
    """
    defects = []
    for source in sources.split(";"):
        if parse_source(source) is None:
            defects.append(
                f"{where}: `Sources` reads {source.strip()[:60]!r}, which no "
                f"form of the grammar accepts -- a source names its kind, its "
                f"target and, where it has one, its section: `this spec "
                f"section 3`, `` `ambre-spec-ui.md section 2` ``, `` `ZK-001` "
                f"``, ``audit `H9` ``, `README pillar \"E2EE\"`, or `derived`"
            )
    return defects


def source_defects(where: str, sources: str, declared: dict[str, str]) -> list[str]:
    """Whether every rule a `Sources` cell cites resolves to a row that exists.

    Two questions, one pass. The id must be declared somewhere in the corpus: a
    row deleted or renumbered leaves its citations pointing at nothing, and the
    join to the rule that produced the obligation breaks in silence. When the
    source names a document too, that document must be the one declaring the id,
    because a citation naming the wrong file sends a reader to a row that is not
    there.
    """
    defects = []
    for rid, docs in sources_cited(sources):
        home = declared.get(rid)
        if home is None:
            defects.append(
                f"{where}: `Sources` cites `{rid}`, and no document declares it "
                f"-- an id is a join key, so a citation of one that is gone "
                f"points at nothing and nothing said so"
            )
        elif docs and not any(
            home == doc or home.rsplit("/", 1)[-1] == doc.rsplit("/", 1)[-1]
            for doc in docs
        ):
            defects.append(
                f"{where}: `Sources` cites `{rid}` beside "
                f"{', '.join(f'`{doc}`' for doc in docs)}, and `{rid}` is "
                f"declared by `{home}` -- the document named is where the reader "
                f"goes, so it is the one that holds the row"
            )
    return defects


# `MECHANISM_TOKENS` is the template's mechanism table, and every token of it
# names a rung of the ladder `1-Foundations/ambre-spec-testing.md section 2`
# owns. The cell is read whole: one token off that list fails the row, whatever
# the prose around it says, because a cell a gate reads half of is a cell
# nothing holds. `BARE_TOKENS` name no mechanism, so no clause can locate one;
# `LOCATED_TOKENS` name a shape a reader cannot grep for, so each one must.
#
# The belt: a backticked Rust path, or the `::name` continuation of the path
# before it. An application name may precede it where one rule is held once per
# application -- `Toile `ambre_toile::...``.
MECHANISM_PATH = re.compile(
    r"^(?:(?:Trame|Toile|Prose|chrome)\s+)?"
    r"`(?:::)?[a-z_][A-Za-z0-9_]*(?:::[A-Za-z0-9_]+)*`$"
)

# The header field a shared document declares the applications it binds with:
# `**Applications:** Prose, Trame, Toile`. A row labelling its tests per
# application is a rule each of them must hold, so it names one test for each
# (`ambre-spec-template.md section 1.B`). One test of the shared crate proves
# the shared code, never that an application calls it: `KINETIC-002` read
# `verified` while Toile broke it.
APPLICATIONS = re.compile(r"^\*\*Applications:\*\*\s+(?P<apps>.+?)\s*$")
APPLICATION_NAMES = ("Prose", "Trame", "Toile")
APPLICATION_LABEL = re.compile(r"(?:^|;)\s*(Prose|Trame|Toile)\s+`")


def declared_applications(line: str) -> tuple[list[str], list[str]] | None:
    """The applications a header line declares, and the names it gets wrong."""
    m = APPLICATIONS.match(line)
    if not m:
        return None
    names = [name.strip() for name in m["apps"].split(",")]
    return names, [name for name in names if name not in APPLICATION_NAMES]


def per_application_defects(
    where: str, mechanism: str, status: str, apps: list[str] | None
) -> list[str]:
    """Whether a row held once per application names a test in each one.

    A row with no label is the shared code's own rule, and passes. A labelled
    row may be `verified` only once every declared application has its test:
    the ones missing are the half nothing holds.
    """
    labels = set(APPLICATION_LABEL.findall(mechanism))
    if not labels:
        return []
    if apps is None:
        return [
            f"{where}: the `Mechanism` names a test per application, and the "
            f"document declares no `**Applications:**` field to hold it to"
        ]
    stray = sorted(labels - set(apps))
    if stray:
        return [
            f"{where}: the `Mechanism` labels {', '.join(stray)}, which the "
            f"`**Applications:**` field does not declare"
        ]
    missing = [app for app in apps if app not in labels]
    if status == "verified" and missing:
        return [
            f"{where}: `verified` with no test in {', '.join(missing)} -- a rule "
            f"each application must hold is proven in each, so the row is "
            f"`partial` until every declared application names its test"
        ]
    return []


# `GAP_OPENERS` is the closed set a `Gap` cell opens on. The template writes the
# five carrying an argument with the dash and the space that argument follows,
# and `none` bare, so the two families are read off the spelling rather than
# named again here. That prose is the one place in the row where an argument
# belongs.


def mechanism_defects(where: str, cell: str) -> list[str]:
    """Whether a `Mechanism` cell is a mechanism, and nothing else.

    A total read. The cell is a `;`-separated list, and every item of it must be
    a token or a path. Prose fails here even when the prose is true: what it
    says belongs in `Gap`, where a reader and a script can both find it.
    """
    if not cell:
        return [f"{where}: the `Mechanism` cell is empty -- write `none`, which claims nothing"]
    defects = []
    for item in cell.split(";"):
        item = item.strip()
        token, _, clause = item.partition(" -- ")
        if MECHANISM_PATH.match(token):
            continue
        if token not in MECHANISM_TOKENS:
            defects.append(
                f"{where}: `Mechanism` carries {item[:60]!r}, which is no mechanism -- "
                f"the cell is a `;`-separated list of the closed set, and an argument "
                f"about what it does not hold belongs in `Gap`"
            )
        elif clause and token in BARE_TOKENS:
            defects.append(
                f"{where}: `Mechanism` reads {item[:60]!r} -- {token!r} names no "
                f"mechanism for a clause to locate, so what follows it is an "
                f"argument, and an argument belongs in `Gap`"
            )
        elif token in LOCATED_TOKENS and not clause:
            defects.append(
                f"{where}: `Mechanism` is {token!r} on its own -- a reader cannot "
                f"find what it names, so write `{token} -- ` and then the construction "
                f"that holds the rule"
            )
    return defects


def gap_defects(where: str, cell: str, mechanism: str, status: str) -> list[str]:
    """Whether a `Gap` cell states what the mechanism leaves open.

    The opener is read whole; what follows it is prose and is read by nobody.
    The two rules across the columns are here because neither can be asked of
    one cell: they compare `Gap` against `Mechanism` and against `Status`.
    """
    defects = []
    bare = cell in BARE_GAP_OPENERS
    if not bare and not any(cell.startswith(o) for o in CARRIED_GAP_OPENERS):
        defects.append(
            f"{where}: `Gap` opens on {cell[:60]!r} -- write "
            f"{', '.join(repr(o) for o in BARE_GAP_OPENERS)}, or one of "
            f"{', '.join(repr(o.strip()) for o in CARRIED_GAP_OPENERS)} "
            f"and then the argument"
        )
        return defects
    if mechanism in BARE_TOKENS and bare:
        defects.append(
            f"{where}: `Mechanism` is {mechanism!r} and `Gap` is {cell!r} -- a row "
            f"with no mechanism and no gap claims a coverage it does not have"
        )
    held = mechanism.split(";")[0].partition(" -- ")[0].strip() not in BARE_TOKENS
    half = cell.startswith("partial -- ")
    if status == "partial" and not (half and held):
        defects.append(
            f"{where}: status is `partial` -- the row names the mechanism that "
            f"holds half the rule, and `Gap` opens on 'partial -- ' to say which "
            f"half is left"
        )
    if status == "untested" and half:
        defects.append(
            f"{where}: `untested` beside a `Gap` opening on 'partial -- ' -- a rule "
            f"half held is `partial`, and one nothing holds opens on 'unheld -- '"
        )
    if status == "verified" and not bare:
        defects.append(
            f"{where}: status is `verified` beside a `Gap` that is not `none` -- a "
            f"status saying something turns red, next to a cell saying which half "
            f"is loose, is two answers to one question"
        )
    return defects


def row_defects(where: str, kind_cell: str, statement: str, sources: str) -> list[str]:
    """Whether one row's own cells obey the grammar of `ambre-spec-template.md`.

    Three cells, three rules, and each held a drift this script could not see
    (the docstring above names them). The conjunction is not here: it is
    measured, not refused.
    """
    defects = []
    match = KIND_CELL.match(kind_cell)
    if not match:
        defects.append(
            f"{where}: the `Kind` cell is {kind_cell!r} -- a kind is written in "
            f"backticks, `INV`, so one column has one spelling"
        )
    elif match.group(1) not in KINDS:
        defects.append(
            f"{where}: kind `{match.group(1)}` is none of the eight "
            f"({', '.join(KINDS)})"
        )
    if not RFC2119.search(statement):
        defects.append(
            f"{where}: the `Statement` carries no RFC 2119 keyword -- a rule "
            f"reads MUST, MUST NOT, SHOULD, SHOULD NOT or MAY, and a sentence "
            f"without one describes instead of binding"
        )
    defects += grammar_defects(where, sources)
    return defects


# A backticked Rust path in a `Mechanism` cell: `crate::module::name`, or the
# `::name` shorthand for "same module as the path just before it". A single
# segment is prose, and a CamelCase last segment is a type or an enum variant,
# not something `fn` declares.
CITED = re.compile(
    r"`(::[A-Za-z_]\w*|[A-Za-z_]\w*(?:::[A-Za-z_]\w*)+)`"
)
FN = re.compile(r"\bfn\s+([a-z_]\w*)")

# The schedule side. An epic heading is numbered in the roadmap and bare in the
# journal; a story is the first cell of a row, behind its checkbox.
EPIC_HEAD = re.compile(r"^##\s+(?:\d+\.\s+)?(?P<key>[A-Z][A-Z0-9-]*)\b")
STORY_ROW = re.compile(r"^\|\s*`\[(?P<done>[ x])\]`\s*(?P<name>[^|]+?)\s*\|")
# What a `Gap` cell says after "lands with": the story, then an optional
# parenthesis naming the epic, then whatever note the cell adds. No story name
# carries a parenthesis or a sentence break, so the name ends at the first of
# either.
NOTE = re.compile(r";|\.\s|, and \*\*")
HINT = re.compile(r"\(([^)]*)\)")
EPIC_IN_HINT = re.compile(r"\b(post-V1|POST-V1|E[0-9]|E[A-Z])\b")


def story_key(name: str) -> str:
    """The join key of a story name: what two spellings of it have in common.

    A citation delimits the name with quotes or backticks and the roadmap does
    not, so both sides are stripped of them, folded to lower case and collapsed on
    whitespace. Everything else is compared exactly: the name is a join key, and a
    key that forgives a word is a key that joins the wrong row.
    """
    return re.sub(r"\s+", " ", name.replace("`", "").replace('"', "")).strip(" .").lower()


def story_keys(name: str) -> list[str]:
    """The key of a story, plus its phase code when the name carries one.

    `ARCH P8 -- persisted schema versioning` is cited 13 times as `ARCH P8`:
    section 1 of the roadmap makes the phase code a citation key of its own. So a
    name shaped `<code> -- <words>` registers under both, and the alias is held to
    the same uniqueness as the name.
    """
    keys = [story_key(name)]
    if " -- " in name:
        keys.append(story_key(name.split(" -- ")[0]))
    return keys


def read_schedule(path: Path) -> dict[str, tuple[str, bool, int, str]]:
    """Every story of one schedule file, per key: epic, ticked, line, name."""
    index: dict[str, tuple[str, bool, int, str]] = {}
    epic = "?"
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        head = EPIC_HEAD.match(line)
        if head:
            epic = head["key"]
            continue
        row = STORY_ROW.match(line)
        if row:
            for key in story_keys(row["name"]):
                index[key] = (epic, row["done"] == "x", lineno, row["name"].strip())
    return index


# The release scopes, in the order the work reaches them. `--` names none and
# is not on the line.
MILESTONE_ORDER = ("MVP", "V1", "post-V1")


def read_milestones(path: Path) -> dict[str, tuple[str, bool]]:
    """The milestone of every story of the roadmap, per key, and whether it is
    ticked.

    The `Milestone` cell is the row's second cell. Section 8 carries no such
    column, and a story there is `post-V1` by where it sits.
    """
    milestones: dict[str, tuple[str, bool]] = {}
    epic = "?"
    for line in path.read_text().splitlines():
        head = EPIC_HEAD.match(line)
        if head:
            epic = head["key"]
            continue
        row = STORY_ROW.match(line)
        if not row:
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        milestone = "post-V1" if epic.upper() == "POST-V1" else cells[1]
        for key in story_keys(row["name"]):
            milestones[key] = (milestone, row["done"] == "x")
    return milestones


def current_milestone(milestones: dict[str, tuple[str, bool]]) -> str | None:
    """The milestone in progress: the earliest one with a story still open."""
    open_ones = {m for m, done in milestones.values() if not done}
    return next((m for m in MILESTONE_ORDER if m in open_ones), None)


def milestone_defects(
    where: str,
    status: str,
    cell: str,
    milestones: dict[str, tuple[str, bool]],
    current: str | None,
) -> list[str]:
    """Whether a `failing` row lands with a story of the milestone in progress.

    A `failing` row is code that breaks its rule today, so the defect was found
    in the milestone in progress, and it is fixed there. A later milestone
    carries only what is not built, and `--` carries no release at all.
    """
    if status != "failing" or current is None:
        return []
    cite = cited_story(cell)
    if cite is None:
        return []  # `story_defects` already reports the missing citation
    name = cite[0].strip('`"')
    entry = milestones.get(story_key(name))
    if entry is None or entry[0] == current:
        return []  # an unknown story is `story_defects`' to report
    return [
        f"{where}: `failing` and lands with {name!r}, a {entry[0]} story -- a "
        f"defect is fixed in the milestone that finds it, and {current} is in "
        f"progress"
    ]


# The statuses that claim a gap cannot close, and the obstacle each names. The
# table is the testing document's, not the template's: an obstacle is a fact
# about the tooling, and the method owns the tooling. A `Lifted by` cell names
# the Story bringing the mechanism, or `--`.
CANNOT_CLOSE = ("human-review", "qa-manual")
TESTING = "1-Foundations/ambre-spec-testing.md"
OBSTACLES_AT = re.compile(r"\*\*The (?P<count>[a-z]+) obstacles\.\*\*")
# `partial -- ` when a mechanism holds part of the rule and the obstacle is why
# the rest stays a reviewer's.
OBSTACLE_GAP = re.compile(r"^(?:unheld|partial) -- `(?P<name>[a-z][a-z-]*)`(?: -- |$)")
OBSTACLE_ROW = re.compile(r"^\|\s*`(?P<name>[a-z][a-z-]*)`\s*\|")


def read_obstacles(text: str) -> tuple[dict[str, str | None], list[str]]:
    """Each obstacle the testing document declares, and the Story lifting it.

    The heading counts the table, as a vocabulary heading of the template does:
    a parse that stops matching returns nothing, and an empty set would let a
    row cite any word. So a count off the table is an error, never a narrowing.
    """
    lines = text.splitlines()
    at = [i for i, line in enumerate(lines) if OBSTACLES_AT.search(line)]
    if len(at) != 1:
        return {}, [f"{TESTING}: {len(at)} lines carry '**The <n> obstacles.**', want 1"]
    word = OBSTACLES_AT.search(lines[at[0]])["count"]
    obstacles: dict[str, str | None] = {}
    started = False
    for line in lines[at[0] + 1 :]:
        if not line.startswith("|"):
            if started:
                break
            continue
        started = True
        row = OBSTACLE_ROW.match(line)
        if not row:
            continue  # the header and the separator
        lifted = line.strip().strip("|").split("|")[-1].strip()
        cite = CITED_STORY.search(lifted)
        obstacles[row["name"]] = cite["name"].strip('`"') if cite else None
    if NUMBERS.get(word) != len(obstacles):
        return obstacles, [
            f"{TESTING}: the obstacles heading counts {word}, its table carries "
            f"{len(obstacles)}"
        ]
    return obstacles, []


def lifting_defects(
    obstacles: dict[str, str | None],
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether each `Lifted by` Story is a row of the roadmap or of the journal.

    The loose-citation check skips a backticked phrase no schedule carries,
    since most are not story names. Here the cell is a story name by
    declaration, so a rename would otherwise re-open the obstacle in silence.
    """
    return [
        f"{TESTING}: the obstacle `{name}` is lifted by {story!r}, which no row of "
        f"{ROADMAP} or {JOURNAL} carries"
        for name, story in obstacles.items()
        if story and story_key(story) not in schedule and story_key(story) not in closed
    ]


def obstacle_defects(
    where: str, status: str, gap: str, obstacles: dict[str, str | None]
) -> list[str]:
    """Whether a row claiming its gap cannot close names a standing obstacle.

    `qa-manual` and `human-review` owe no Story because nothing could hold the
    rule. That is true of the tooling on one day. Naming the obstacle is what
    lets the day a Story lifts it fail every row that still leans on it.
    """
    if status not in CANNOT_CLOSE:
        return []
    found = OBSTACLE_GAP.match(gap)
    if not found:
        return [
            f"{where}: `{status}` claims a gap that cannot close -- `Gap` opens on "
            f"'unheld -- `<obstacle>` -- ', or 'partial -- ' when a mechanism holds "
            f"part of it, with one obstacle of {TESTING} section 3"
        ]
    name = found["name"]
    if name not in obstacles:
        return [
            f"{where}: names the obstacle `{name}`, which {TESTING} section 3 does "
            f"not declare -- a new obstacle is a row of that table first"
        ]
    story = obstacles[name]
    if story:
        return [
            f"{where}: `{status}` on the obstacle `{name}`, which the {story!r} Story "
            f"lifts -- the gap can close, so the row is debt: `untested` or "
            f"`partial`, landing with a Story"
        ]
    return []


def leaned_on(status: str, gap: str) -> str | None:
    """The obstacle a row claiming its gap cannot close names, if any."""
    found = OBSTACLE_GAP.match(gap) if status in CANNOT_CLOSE else None
    return found["name"] if found else None


def orphan_obstacles(
    obstacles: dict[str, str | None],
    cited: set[str],
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether each obstacle of the table is still a live reason.

    A standing obstacle no row cites pre-approves a `qa-manual` no reader has
    weighed: the next row would lean on it without arguing it. A lifted one stays
    while its Story is scheduled, since it lists the requalification under way,
    and goes once the Story ships, when no row may cite it.
    """
    errors: list[str] = []
    for name, story in obstacles.items():
        if story is None and name not in cited:
            errors.append(
                f"{TESTING}: the obstacle `{name}` is cited by no `qa-manual` or "
                f"`human-review` row -- an obstacle is declared the day a row needs it"
            )
        elif story and story_key(story) not in schedule and story_key(story) in closed:
            errors.append(
                f"{TESTING}: the obstacle `{name}` is lifted by {story!r}, which has "
                f"shipped -- the reason is gone, so its row goes"
            )
    return errors


def duplicate_stories(path: Path) -> list[str]:
    """Each story name or phase code that two rows of one schedule carry.

    `read_schedule` keeps the last row per key, so a collision is invisible to a
    citation: it resolves, silently, to whichever row comes last. That is the
    defect this reports -- "PDF export" named a Prose story and a Trame one.
    """
    seen: dict[str, int] = {}
    errors: list[str] = []
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        row = STORY_ROW.match(line)
        if not row:
            continue
        for key in story_keys(row["name"]):
            if key in seen:
                errors.append(
                    f"{path.name}:{lineno}: two stories share the name or code "
                    f"{key!r} (the other is line {seen[key]}) -- a story name is a "
                    f"join key, so an `unimplemented` row cannot name one of them"
                )
            seen[key] = lineno
    return errors


def cited_story(cell: str) -> tuple[str, str | None] | None:
    """The story a `Gap` cell lands with, and the epic it claims."""
    if "lands with" not in cell:
        return None
    text = NOTE.split(cell.split("lands with", 1)[1])[0].strip()
    epic = None
    hint = HINT.search(text)
    if hint:
        text = text[: hint.start()].strip()
        found = EPIC_IN_HINT.search(hint.group(1))
        epic = found.group(1) if found else None
    text = re.sub(r"\s+Story$", "", text).strip()
    text = re.sub(r'^the\s+(?=[`"])', "", text).strip()
    return text, epic


def story_defects(
    where: str,
    status: str,
    cell: str,
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether one row that owes a Story lands with an open story of the roadmap."""
    cite = cited_story(cell)
    if cite is None:
        return [
            f"{where}: `{status}` without a Story -- the `Gap` cell must read "
            f"'... lands with the <Story> Story (`{ROADMAP}` <epic>)'"
        ]
    name, epic = cite
    name = name.strip('`"')  # the delimiters a citation puts around the name
    entry = schedule.get(story_key(name))
    if entry is None:
        gone = closed.get(story_key(name))
        if gone:
            return [
                f"{where}: lands with {name!r}, which closed under epic `{gone[0]}` "
                f"in {JOURNAL} -- a `{status}` row cannot wait on work that shipped"
            ]
        return [
            f"{where}: lands with {name!r}, and no row of {ROADMAP} carries that "
            f"name or phase code"
        ]
    story_epic, done, lineno, real = entry
    defects = []
    if done:
        defects.append(
            f"{where}: lands with {real!r}, which {ROADMAP}:{lineno} ticks `[x]` "
            f"-- a story stays open until every row bound to it is `verified`"
        )
    if not epic:
        defects.append(
            f"{where}: lands with {real!r} and names no epic -- write "
            f"(`{ROADMAP}` {story_epic}), so a move of the story fails here"
        )
    elif epic.upper() != story_epic.upper():
        defects.append(
            f"{where}: cites epic `{epic}`, and {real!r} lives under `{story_epic}`"
            f" ({ROADMAP}:{lineno})"
        )
    return defects


# A story cited anywhere else: a shipped row naming the Story that repairs it, or
# a section 5 bullet naming the one that will build the thing. The `lands with`
# form is the same citation with the parenthesis optional, and is read above.
CITED_STORY = re.compile(
    r"(?P<name>`[^`]+`|\"[^\"]+\")\s*(?:Story\s*)?"
    r"\(`ambre-roadmap\.md`\s*(?P<epic>[^),]*)"
)
# The same citation with the parenthesis left off. It reads as a citation and
# resolves to nothing: no gate holds a name it cannot join, so the story behind
# it is free to be renamed, or to close into the journal while the sentence
# still says "the roadmap's". The name is on the line the word `Story` is on, so
# a citation wrapped across two lines is this defect too, and is rewrapped rather
# than excused.
BARE_STORY = re.compile(r"(?P<name>`[^`]+`|\"[^\"]+\")\s*[Ss]tory\b")


def loose_citation_defects(
    line: str,
    where: str,
    handled: set[str],
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether every other story this line cites resolves, and to the epic named.

    A row that ships and fails its own rule names the Story that repairs it, and
    that citation rots exactly like an `unimplemented` one: `One clipboard host`
    was cited under `E6` by three `unverified` rows while the story moved to `E7`.
    """
    defects = []
    resolved = {story_key(m["name"]) for m in CITED_STORY.finditer(line)} | handled
    for m in BARE_STORY.finditer(line):
        name = m["name"].strip('`"')
        if story_key(name) in resolved:
            continue
        if story_key(name) in schedule or story_key(name) in closed:
            defects.append(
                f"{where}: names the Story {name!r} and does not cite it -- a name "
                f"the gate cannot resolve rots in silence, so write it "
                f"{name} Story (`{ROADMAP}` <epic>)"
            )
    for m in CITED_STORY.finditer(line):
        name = m["name"].strip('`"')
        if story_key(name) in handled:
            continue
        entry = schedule.get(story_key(name))
        if entry is None:
            if story_key(name) in closed:
                gone = closed[story_key(name)]
                defects.append(
                    f"{where}: cites the Story {name!r} as {ROADMAP}, and it closed"
                    f" under epic `{gone[0]}` in {JOURNAL}"
                )
            continue  # a backticked phrase that is not a story name
        story_epic, done, lineno, real = entry
        if done:
            defects.append(
                f"{where}: cites the Story {real!r}, which {ROADMAP}:{lineno} ticks"
                f" `[x]` -- the work it waits on has shipped"
            )
        elif not m["epic"].strip():
            # Ten citations named the roadmap and no epic, so the check below had
            # nothing to compare and the move that renames an epic stayed green.
            defects.append(
                f"{where}: cites {real!r} with no epic -- write "
                f"(`{ROADMAP}` {story_epic}), so a move of the story fails here"
            )
        elif m["epic"].strip().upper() != story_epic.upper():
            defects.append(
                f"{where}: cites {real!r} under epic `{m['epic'].strip()}`, and it "
                f"lives under `{story_epic}` ({ROADMAP}:{lineno})"
            )
    return defects


def declared_fns() -> tuple[dict[str, set[str]], set[str]]:
    """Every `fn` name in the workspace, per crate and pooled.

    Names, not paths: resolving a module path to a file would mean reimplementing
    Rust's module system in a regex, and the claim being checked is only that the
    mechanism exists. A cell naming the wrong module but the right function is a
    citation defect this does not catch, and a far smaller one than a name that
    resolves to nothing.
    """
    per_crate: dict[str, set[str]] = {}
    for source in sorted(CRATES.glob("*/*/**/*.rs")):
        crate = source.relative_to(CRATES).parts[0].replace("-", "_")
        per_crate.setdefault(crate, set()).update(FN.findall(source.read_text()))
    pooled = set().union(*per_crate.values()) if per_crate else set()
    return per_crate, pooled


def unresolved_citations(cell: str, per_crate: dict[str, set[str]], pooled: set[str]):
    """Yield each cited mechanism in `cell` that no `fn` declares."""
    for token in CITED.findall(cell):
        segments = token.lstrip(":").split("::")
        name = segments[-1]
        if name[:1].isupper():
            continue
        crate = segments[0] if segments[0].startswith("ambre_") else None
        pool = per_crate.get(crate, set()) if crate else pooled
        if name not in pool:
            where = f" in `{crate}`" if crate else " anywhere in the workspace"
            yield f"cites `{token}`, and no `fn {name}` exists{where}"


def declared_findings() -> dict[str, int]:
    """Every finding the audit declares, per id, with the line declaring it.

    Three shapes declare one: the heading of its section, a row of a status
    table, and a story of a remediation wave. Only the part of a heading before
    the dash is read, so `### H4 / H5 -- ...` declares two findings and the title
    beside them declares none.
    """
    found: dict[str, int] = {}
    for lineno, line in enumerate((SPECS / AUDIT).read_text().splitlines(), 1):
        ids: list[str] = []
        head = FINDING_HEAD.match(line)
        if head:
            title = head.group(1).split(" -- ")[0]
            ids = [t for t in re.split(r"[^A-Za-z0-9]+", title) if FINDING.match(t)]
        row = FINDING_ROW.match(line)
        if row:
            ids = [row.group(1)]
        story = FINDING_STORY.match(line)
        if story:
            ids = [story.group(1)]
        for fid in ids:
            found.setdefault(fid, lineno)
    return found


def cited_findings(line: str) -> list[str]:
    """Every token one row hands the word `audit`, whether or not it is an id."""
    tokens: list[str] = []
    for m in AUDIT_CITE.finditer(line):
        tokens += re.findall(r"`([^`]+)`", m.group(1))
    return tokens


def audit_coverage_defects(findings: dict[str, int], cited: set[str]) -> list[str]:
    """Whether every finding is held by a rule, or declared as holding none."""
    defects = []
    for fid, lineno in sorted(findings.items()):
        if fid in cited:
            if fid in NO_RULE:
                defects.append(
                    f"{AUDIT}:{lineno}: finding `{fid}` is cited by a contract "
                    f"row and declared in `NO_RULE` -- one or the other"
                )
            continue
        if fid in NO_RULE:
            continue
        defects.append(
            f"{AUDIT}:{lineno}: finding `{fid}` is cited by no contract row -- "
            f"state the rule that closes it in the spec of the component it "
            f"constrains, or declare it in `NO_RULE` with its reason"
        )
    for fid in sorted(NO_RULE):
        if fid not in findings:
            defects.append(
                f"`NO_RULE` carries `{fid}`, and {AUDIT} declares no such finding"
            )
    return defects


# The `Verification` answer under a contract table, and the counts it opens
# on: `7 `verified`, 1 `untested`, 1 `unimplemented`.` Both separators the
# corpus uses are read, the comma and the final "and".
ANSWER = re.compile(r"^\*\s+\*\*Verification\.\*\*\s+(?P<text>.*)$")
COUNT = re.compile(r"(?P<n>\d+)\s+`(?P<status>[a-z-]+)`")
COUNT_SEP = re.compile(r",\s+(?:and\s+)?")


def continues_bullet(line: str) -> bool:
    """Whether a line carries on the bullet above it: indented, not blank, and
    not a nested bullet of its own."""
    stripped = line.strip()
    return line[:1] in (" ", "\t") and bool(stripped) and not stripped.startswith(("* ", "- "))


def stated_counts(text: str) -> dict[str, int]:
    """The counts one `Verification` answer opens on, empty when it opens on none.

    Only the head of the bullet is read. What follows the counts names the gaps
    in prose and carries figures of its own -- "two of the four are rules the
    code answers with the opposite" -- and those figures are a review act, not a
    parse.
    """
    counts: dict[str, int] = {}
    pos = 0
    while (m := COUNT.match(text, pos)) is not None:
        counts[m["status"]] = int(m["n"])
        pos = m.end()
        sep = COUNT_SEP.match(text, pos)
        if sep is None:
            break
        pos = sep.end()
    return counts


def table_defects(
    spec: str, table: list[tuple[int, str, str]], frame: list[str] | None = None
) -> list[str]:
    """Whether one table that declares `Kind` obeys the grammar of section 1.C.

    Two rules about the table as a whole; the cells of one row are
    `row_defects`. A table mixing a state kind with an operation kind
    is the layout collapsing back into one list, which is what hid the frame
    conditions. And the completeness rule is the reason for the column: a `POST`
    with no `FRAME` names what an operation writes and never what it leaves
    alone.

    What no rule here holds is the line between `PRE` and `FAIL`, because it is
    semantic: a `PRE` binds an obligation nothing checks, a `FAIL` states a
    refusal the operation performs. Writing both for one condition states one
    guard twice, and doubles one test into two `verified` rows. The template
    carries the test that separates them; a reader applies it.

    `frame` holds the ids a `**Frame:**` line under the table cites. A cited
    shared `FRAME` answers the completeness rule as a local one does; `main`
    checks, once the corpus is read, that each id is a `FRAME` row.
    """
    defects = []
    # An unknown kind is reported once, by `row_defects`, which reads the cell
    # whole. Here it is dropped: a typo must not also read as a family clash.
    kinds = {kind for _, _, kind in table if kind in KINDS}
    if not kinds:
        return defects
    operation = kinds & set(OPERATION_KINDS)
    state = kinds & set(STATE_KINDS)
    lineno = table[0][0]
    if operation and state:
        defects.append(
            f"{spec}:{lineno}: one table holds the state kinds "
            f"{sorted(state)} and the operation kinds {sorted(operation)} -- an "
            f"operation carries a table of its own, under its own heading"
        )
    if frame is not None and "POST" not in kinds:
        defects.append(
            f"{spec}:{lineno}: a `**Frame:**` line closes a table that declares "
            f"no `POST` -- the line answers an operation's frame, and this table "
            f"states none"
        )
    if "POST" in kinds and "FRAME" not in kinds and not frame:
        defects.append(
            f"{spec}:{lineno}: this operation declares a `POST` and no `FRAME` "
            f"-- state what it MUST NOT touch, `untested` when nothing "
            f"isolates it"
        )
    return defects


def render_counts(counts: Counter) -> str:
    """The count run a `Verification` answer opens on, in the order of `STATUSES`."""
    return ", ".join(f"{counts[s]} `{s}`" for s in STATUSES if counts[s])


def rewrite_answer(path: Path, lineno: int, counts: Counter) -> bool:
    """Writes the table's counts over the head of its `Verification` bullet.

    The counts are derived, so a writer never states them: the run the bullet
    opens on is replaced, the prose after it is kept, and the bullet is rewrapped
    to the column limit. Answers whether the file changed.
    """
    lines = path.read_text().split("\n")
    start = lineno - 1
    end = start + 1
    while end < len(lines) and continues_bullet(lines[end]):
        end += 1
    head = ANSWER.match(lines[start])
    text = " ".join([head["text"]] + [l.strip() for l in lines[start + 1:end]])
    pos, stated = 0, False
    while (m := COUNT.match(text, pos)) is not None:
        stated, pos = True, m.end()
        sep = COUNT_SEP.match(text, pos)
        if sep is None or COUNT.match(text, sep.end()) is None:
            break
        pos = sep.end()
    rest = text[pos:] if stated else f". {text}"
    body = f"* **Verification.** {render_counts(counts)}{rest}"
    wrapped = textwrap.wrap(
        body, width=100, subsequent_indent="  ",
        break_long_words=False, break_on_hyphens=False,
    )
    if wrapped == lines[start:end]:
        return False
    lines[start:end] = wrapped
    path.write_text("\n".join(lines))
    return True


# The list of `human-review` rows in `CONTRIBUTING.md`, written from the rows
# between two markers, as the ledger is: a list kept by hand drifts.
CONTRIBUTING = ROOT / "CONTRIBUTING.md"
REVIEW_OPEN = "<!-- human-review rows: generated by `just contracts-write`, never edit -->"
REVIEW_CLOSE = "<!-- end of the human-review rows -->"


def render_review_table(reviewed: list[tuple[str, str, str]]) -> str:
    """The table of `human-review` rows, one line each, sorted by spec then id."""
    lines = ["| Row | What the reviewer holds | Spec |", "| :--- | :--- | :--- |"]
    for rid, statement, spec in sorted(reviewed, key=lambda r: (r[2], r[0])):
        lines.append(f"| `{rid}` | {statement} | `{spec}` |")
    return "\n".join(lines)


def review_table_defects(reviewed: list[tuple[str, str, str]]) -> list[str]:
    """Whether `CONTRIBUTING.md` lists the `human-review` rows the corpus holds."""
    text = CONTRIBUTING.read_text()
    head, sep, tail = text.partition(REVIEW_OPEN + "\n")
    _, sep2, after = tail.partition(REVIEW_CLOSE)
    if not sep or not sep2:
        return [f"CONTRIBUTING.md: the `human-review` table has lost its markers"]
    fresh = f"{head}{REVIEW_OPEN}\n{render_review_table(reviewed)}\n{REVIEW_CLOSE}{after}"
    if fresh == text:
        return []
    if "--write" in sys.argv:
        CONTRIBUTING.write_text(fresh)
        return []
    return ["CONTRIBUTING.md: the `human-review` table is stale -- run `just contracts-write`"]


def answer_defects(spec: str, answer: tuple[int, str] | None, counts: Counter) -> list[str]:
    """Whether the answer's counts are the counts of the table above it."""
    if answer is None:
        return []  # a missing label is `spec_shape.py`: one rule, one home
    lineno, text = answer
    where = f"{spec}:{lineno}"
    stated = stated_counts(text)
    if not stated:
        return [
            f"{where}: the `Verification` answer states no count -- it opens on "
            f"the count per status, then names the rows that are the gaps"
        ]
    defects = []
    for status in sorted(set(stated) | set(counts)):
        if status not in STATUSES:
            defects.append(
                f"{where}: the `Verification` answer counts {status!r}, which is "
                f"no status of the seven"
            )
        elif stated.get(status, 0) != counts.get(status, 0):
            defects.append(
                f"{where}: the `Verification` answer says {stated.get(status, 0)} "
                f"`{status}` and the table holds {counts.get(status, 0)} -- the "
                f"counts are derived: run `just contracts-write`"
            )
    return defects


def leaning_report(leans: dict[str, list[str]]) -> list[str]:
    """The tests that back more than one `verified` row, worst first.

    Not a defect: a test carrying five assertions legitimately backs five rows.
    It is the number the ledger cannot show. Splitting one row into five raises
    `verified` by four without a new assertion, so a migration that only
    relabels reads exactly like one that adds coverage. This prints the
    difference and lets a reader judge it.
    """
    # One row citing a test twice in its cell is one row, not two.
    shared = {fn: sorted(set(rows)) for fn, rows in leans.items() if len(set(rows)) > 1}
    if not shared:
        return []
    lines = [
        "",
        f"one test, several rows: {len(shared)} tests back "
        f"{sum(len(r) for r in shared.values())} `verified` rows",
    ]
    for fn, rows in sorted(shared.items(), key=lambda kv: (-len(kv[1]), kv[0]))[:10]:
        lines.append(f"  {len(rows):>2}  {fn}  {', '.join(rows)}")
    return lines


def split_report(candidates: list[tuple[str, str]]) -> list[str]:
    """The rows a second keyword reaches that the refusal above does not.

    Not a defect, and for one reason: the measure is exact and the reading is
    not. "MUST mark it resolved, and resolving it again MUST change nothing" is
    two obligations. "MUST each get their own thread, and neither MUST lose
    theirs" is one convergence rule stated from both ends. A script cannot tell
    them apart, so it prints the list and a reader decides each one.
    """
    if not candidates:
        return []
    lines = ["", f"two keywords, one status: {len(candidates)} rows to read"]
    for where, statement in candidates:
        lines.append(f"  {where}  {statement[:96]}")
    return lines


def app_of(spec: str) -> str:
    """The application a spec belongs to, from the first component of its path."""
    head = spec.split("/", 1)[0]
    return head if head in APPS else "other"


def id_ranges(ids: list[str]) -> str:
    """The ids of one document, as runs: `TRAME-017,121,123..124,151..154`.

    A document declares ids from one prefix family and rarely in one block, so
    the list is folded into runs per prefix. Whole it would be a ledger of
    thousands of lines, which is a file nobody opens; folded it is one line per
    document, and a reader resolves `TRAME-088` by the run it falls in.
    """
    runs_by_prefix: dict[str, list[int]] = defaultdict(list)
    for rid in ids:
        prefix, number = rid.rsplit("-", 1)
        runs_by_prefix[prefix].append(int(number))
    cells = []
    for prefix, numbers in sorted(runs_by_prefix.items()):
        runs: list[tuple[int, int]] = []
        for number in sorted(set(numbers)):
            if runs and number == runs[-1][1] + 1:
                runs[-1] = (runs[-1][0], number)
            else:
                runs.append((number, number))
        spans = ",".join(f"{a:03d}" if a == b else f"{a:03d}..{b:03d}" for a, b in runs)
        cells.append(f"{prefix}-{spans}")
    return "; ".join(cells)


def render_ledger(per_spec: dict[str, Counter], total: Counter, declared: dict[str, str]) -> str:
    """The ledger file, whole. This is the only place its text is written."""
    shipped = sum(total[s] for s in STATUSES if s != "unimplemented")
    loose = sum(total[s] for s in LOOSE)
    by_app: dict[str, Counter] = {app: Counter() for app in APPS}
    for spec, counts in per_spec.items():
        by_app.setdefault(app_of(spec), Counter()).update(counts)

    lines = [
        "# AMBRE -- CONTRACT LEDGER",
        "",
        "**Type:** Inventory (generated)",
        "",
        "> The status of every contract row in the corpus, rolled up. The rows themselves live in",
        "> the spec of the component they constrain, and this file reports their state, never intent.",
        "> Generated by `scripts/contracts.py` from the tables themselves, so it carries no fact of",
        "> its own.",
        "",
        "**Do not edit this file.** Edit the contract row and run `just contracts-write`.",
        "",
        "* **Shipped** counts a row whose code exists. **Unbuilt** counts `unimplemented`, and the",
        "  split is the whole point: one number is a debt, the other is a plan.",
        "* **Loose** counts a shipped row that nothing would fail on: `failing`, `untested`,",
        "  `human-review`, `qa-manual`. That number is the coverage debt of the product.",
        "* **Partial** counts a shipped row a mechanism holds in part: `partial`.",
        "* **Every gap that can close names its Story.** A `partial`, `failing`, `untested` or",
        "  `unimplemented` row lands with an open Story of `ambre-roadmap.md`, and",
        "  `contracts.py` fails when that Story does not resolve. The roadmap is the debt",
        "  register. `human-review` and `qa-manual` owe none: they are decisions, not debt.",
        "* How each layer is tested is `ambre-spec-testing.md`.",
        "",
        f"**{sum(total.values())} rows: {shipped} shipped, of which {total['partial']} partial "
        f"and {loose} loose. "
        f"{total['unimplemented']} unbuilt.**",
        "",
        "| Status | " + " | ".join(a.split("-", 1)[-1] for a in APPS) + " | Total |",
        "| :--- | " + " | ".join(["---:"] * (len(APPS) + 1)) + " |",
    ]
    for status in STATUSES:
        cells = " | ".join(str(by_app[app][status] or "") for app in APPS)
        lines.append(f"| `{status}` | {cells} | {total[status]} |")

    lines += [
        "",
        "## THE SHIPPED SIDE, PER DOCUMENT",
        "",
        "> A document with no shipped row is omitted. `Loose` is the column to read.",
        "",
        "| Document | Rows | Shipped | `verified` | Partial | Loose | Unbuilt |",
        "| :--- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for spec, counts in sorted(per_spec.items()):
        built = sum(counts[s] for s in STATUSES if s != "unimplemented")
        if not built:
            continue
        slack = sum(counts[s] for s in LOOSE)
        lines.append(
            f"| `{Path(spec).name}` | {sum(counts.values())} | {built} | "
            f"{counts['verified']} | {counts['partial'] or ''} | {slack or ''} | "
            f"{counts['unimplemented'] or ''} |"
        )

    ids_by_spec: dict[str, list[str]] = defaultdict(list)
    for rid, spec in declared.items():
        ids_by_spec[Path(spec).name].append(rid)
    lines += [
        "",
        "## WHICH DOCUMENT DECLARES AN ID",
        "",
        "> A citation carries an id and never a path, and the corpus mints an id once. This is the",
        "> one place the two are joined: find the run the number falls in, and the row names the",
        "> file. Every document is here, including the ones with nothing shipped.",
        "",
        "| Document | Ids |",
        "| :--- | :--- |",
    ]
    for name, ids in sorted(ids_by_spec.items()):
        lines.append(f"| `{name}` | {id_ranges(ids)} |")
    lines.append("")
    return "\n".join(lines)


def twin_defects(rows: list[tuple[str, str, str, str]]) -> list[str]:
    """Every row stating a rule another row states word for word.

    `rows` holds (where, id, statement, sources). A rule copied into a second
    spec drifts: the copy is edited, the original is not, and two ids claim one
    rule with two statuses. A copy that cites its twin is a deliberate instance
    of it, which a reader can follow; a copy that cites nothing is the drift
    waiting. Only the exact statement is compared, so a near copy passes:
    similarity would flag rules that share a verb and nothing else.
    """
    by_text: dict[str, list[tuple[str, str, str]]] = defaultdict(list)
    for where, rid, statement, sources in rows:
        text = " ".join(statement.split()).lower()
        if text:
            by_text[text].append((where, rid, sources))
    defects = []
    for twins in by_text.values():
        if len(twins) < 2:
            continue
        ids = {rid for _, rid, _ in twins}
        for where, rid, _ in twins:
            others = ", ".join(sorted(ids - {rid}))
            defects.append(
                f"{where}: states word for word the rule of {others} -- keep one "
                f"row, retire the other, and cite the kept `FRAME` on a "
                f"`**Frame:**` line"
            )
    return defects


def main() -> int:
    per_spec: dict[str, Counter] = {}
    reviewed: list[tuple[str, str, str]] = []
    seen: dict[str, str] = {}
    errors: list[str] = []
    per_crate, pooled = declared_fns()
    # Which `verified` rows lean on the same test, for the report below.
    leans: dict[str, list[str]] = defaultdict(list)
    schedule = read_schedule(SPECS / ROADMAP)
    closed = read_schedule(SPECS / JOURNAL)
    milestones = read_milestones(SPECS / ROADMAP)
    in_progress = current_milestone(milestones)
    errors += duplicate_stories(SPECS / ROADMAP)
    obstacles, found = read_obstacles((SPECS / TESTING).read_text())
    errors += found + lifting_defects(obstacles, schedule, closed)
    findings = declared_findings()
    cited_audit: set[str] = set()
    cited_obstacles: set[str] = set()
    # The `Sources` cells, held until every id is read. A citation may name a row
    # declared further down the corpus, so resolving one during the pass would
    # answer on the half read so far.
    cited_sources: list[tuple[str, str]] = []
    # Every row's statement, for `twin_defects` once the corpus is read.
    statements: list[tuple[str, str, str, str]] = []
    # Each id's kind, and the `**Frame:**` citations checked against it at the end.
    kind_of: dict[str, str] = {}
    frame_refs: list[tuple[str, str]] = []
    # The rows `split_report` prints: a second keyword the refusal does not reach.
    split_candidates: list[tuple[str, str]] = []

    for path in sorted(SPECS.rglob("*.md")):
        if path.name == TEMPLATE:
            continue
        counts: Counter = Counter()
        answer: tuple[int, str] | None = None
        in_answer = False
        rel = str(path.relative_to(SPECS))
        # The table being read, and whether its header declares `Kind`. A table
        # ends where the pipes do, so a heading, a bullet or a blank line
        # closes it and the rows it collected are checked as one operation.
        table: list[tuple[int, str, str]] = []
        columns: list[str] | None = None
        apps: list[str] | None = None
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            stripped = line.lstrip()
            declared = declared_applications(line)
            if declared:
                apps, unknown = declared
                for name in unknown:
                    errors.append(
                        f"{rel}:{lineno}: `**Applications:**` names {name!r} -- "
                        f"the field lists {', '.join(APPLICATION_NAMES)}"
                    )
            head = TABLE_HEAD.match(line)
            # A blank line does not close a table: an author who breaks a long
            # operation table over one still writes one operation, and treating
            # the halves as two tables checked each half against nothing.
            framed = FRAME_LINE.match(stripped)
            frame = FRAME_ID.findall(framed["ids"]) if framed else None
            if framed:
                for ref in frame:
                    frame_refs.append((f"{rel}:{lineno}", ref))
                if not frame:
                    errors.append(f"{rel}:{lineno}: a `**Frame:**` line cites no id")
                elif columns is None or not table:
                    errors.append(
                        f"{rel}:{lineno}: a `**Frame:**` line that closes no "
                        f"contract table -- it goes on the line under the "
                        f"operation it frames"
                    )
            if head or (stripped and not stripped.startswith("|")):
                if columns is not None and table:
                    errors += table_defects(rel, table, frame)
                table = []
                columns = header_cells(line) if head else None
                if columns is not None and KIND not in columns:
                    columns = None  # a table keyed by `ID` that is not a contract
            m = ROW.match(line)
            # The row's cells by name. `Gap` is where an `unimplemented` row
            # names its Story, so it is the cell the citation check reads. A row
            # under a header that declares neither `Kind` nor `Gap` is refused
            # below; its last cell is read here so the citation check still runs.
            cell = row_cells(columns, m["rest"]) if m else {}
            binding = cell.get(GAP, "")
            if m and not binding:
                binding = m["rest"].rsplit("|", 1)[-1]
            # a story the row's own binding already answers for, checked below
            bound: set[str] = set()
            if m and m["status"] == "unimplemented":
                cite = cited_story(binding)
                if cite:
                    bound.add(story_key(cite[0].strip('`"')))
            errors += loose_citation_defects(
                line, f"{path.relative_to(SPECS)}:{lineno}", bound, schedule, closed
            )
            # A bullet wraps onto indented lines, and a count cut at the end of
            # the first line must still reach the parse: the continuation is
            # joined to the text until the bullet ends.
            bullet = ANSWER.match(line)
            if bullet:
                answer = (lineno, bullet["text"])
                in_answer = True
            elif in_answer and answer and continues_bullet(line):
                answer = (answer[0], f"{answer[1]} {stripped}")
            else:
                in_answer = False
            if not m:
                continue
            rid, status, rest = m["id"], m["status"], m["rest"]
            where = f"{path.relative_to(SPECS)}:{lineno} {rid}"
            if columns is None:
                errors.append(
                    f"{where}: a contract row under a header that declares no "
                    f"`Kind` column -- the column is what holds the row to the "
                    f"eight kinds, so a table without it checks nothing"
                )
            else:
                kind_cell = cell.get(KIND, "")
                statement = cell.get(STATEMENT, "")
                errors += row_defects(where, kind_cell, statement, cell.get(SOURCES, ""))
                cited_sources.append((where, cell.get(SOURCES, "")))
                statements.append((where, rid, statement, cell.get(SOURCES, "")))
                # Both grammars need the two cells apart. A header that declares
                # no `Gap` states the mechanism and the argument about it in one
                # cell, which is the shape neither grammar can read.
                if GAP not in (columns or []):
                    errors.append(
                        f"{where}: a contract row under a header that declares no "
                        f"`Gap` column -- the mechanism and what it leaves open "
                        f"are two cells, and one cell holding both is what the "
                        f"seventh column exists to take apart"
                    )
                else:
                    mechanism = cell.get(MECHANISM, "")
                    errors += mechanism_defects(where, mechanism)
                    errors += gap_defects(where, cell.get(GAP, ""), mechanism, status)
                    errors += per_application_defects(where, mechanism, status, apps)
                kind = kind_cell.strip("`")
                kind_of[rid] = kind
                table.append((lineno, rid, kind))
                if (
                    kind not in BUNDLING_KINDS
                    and not CONJUNCTION.search(statement)
                    and SPLIT_CANDIDATE.search(statement)
                ):
                    split_candidates.append((where, statement))
                if kind not in BUNDLING_KINDS and CONJUNCTION.search(statement):
                    errors.append(
                        f"{where}: the `Statement` states a second rule after an "
                        f"`and`. Two rules under one status means the half "
                        f"nothing tests breaks in green: split the row, giving "
                        f"the second rule an id and a status of its own"
                    )

            if status not in STATUSES:
                errors.append(f"{where}: unknown status {status!r}")
                continue
            if rid in seen:
                errors.append(f"{where}: duplicate id, already in {seen[rid]}")
            seen[rid] = str(path.relative_to(SPECS))

            for token in cited_findings(line):
                if not FINDING.match(token):
                    errors.append(
                        f"{where}: cites audit `{token}`, which is not a finding "
                        f"id -- a finding is cited by its number, `H9` or `N4`"
                    )
                elif token not in findings:
                    errors.append(
                        f"{where}: cites audit `{token}`, and {AUDIT} declares no "
                        f"finding of that number"
                    )
                else:
                    cited_audit.add(token)

            # The mechanism cell by name, never by position. `binding` is the cell
            # the story citation lives in, which the split moved into `Gap`.
            held_by = cell.get(MECHANISM, rest.rsplit("|", 1)[-1])
            if status in OWES_STORY:
                errors += story_defects(where, status, binding, schedule, closed)
                errors += milestone_defects(where, status, binding, milestones, in_progress)
            errors += obstacle_defects(where, status, binding, obstacles)
            if named := leaned_on(status, binding):
                cited_obstacles.add(named)
            if status != "unimplemented" and "not built" in held_by:
                errors.append(f"{where}: says 'not built' but status is {status!r}")
            if status != "unimplemented":
                # A `Gap` names a test too -- "the half that is held is `::name`" --
                # so both cells are read for a path the workspace does not carry.
                cited_in = f"{held_by} | {cell.get(GAP, '')}"
                for defect in unresolved_citations(cited_in, per_crate, pooled):
                    errors.append(f"{where}: {defect}")
            if status == "verified":
                for token in CITED.findall(held_by):
                    name = token.lstrip(":").split("::")[-1]
                    if not name[:1].isupper():
                        leans[name].append(rid)

            counts[status] += 1
            if status == "human-review":
                reviewed.append((rid, cell.get(STATEMENT, "").strip(), path.name))
        if columns is not None and table:
            errors += table_defects(rel, table)
        if counts:
            per_spec[str(path.relative_to(SPECS))] = counts
            found = answer_defects(str(path.relative_to(SPECS)), answer, counts)
            if found and answer and "--write" in sys.argv:
                rewrite_answer(path, answer[0], counts)
                found = []
            errors += found

    errors += review_table_defects(reviewed)
    errors += audit_coverage_defects(findings, cited_audit)
    errors += orphan_obstacles(obstacles, cited_obstacles, schedule, closed)
    for where, sources in cited_sources:
        errors += source_defects(where, sources, seen)
    errors += twin_defects(statements)
    for where, ref in frame_refs:
        if kind_of.get(ref) != "FRAME":
            what = f"a `{kind_of[ref]}` row" if ref in kind_of else "no row"
            errors.append(
                f"{where}: `**Frame:**` cites `{ref}`, which is {what} -- the "
                f"line cites a `FRAME` row"
            )

    # gaps in a prefix's numbering: an id that was renumbered or dropped
    by_prefix: dict[str, list[int]] = defaultdict(list)
    for rid in seen:
        prefix, num = rid.rsplit("-", 1)
        by_prefix[prefix].append(int(num))
    retired: dict[str, set[int]] = defaultdict(set)
    for rid in RETIRED:
        if rid in seen:
            errors.append(
                f"{rid}: retired in `contracts.py` and still declared by a spec"
                " -- a withdrawn id is never re-minted"
            )
        prefix, num = rid.rsplit("-", 1)
        retired[prefix].add(int(num))
    for prefix, nums in sorted(by_prefix.items()):
        nums.sort()
        missing = sorted(
            set(range(1, nums[-1] + 1)) - set(nums) - retired.get(prefix, set())
        )
        if missing:
            errors.append(
                f"{prefix}: gap in numbering, missing {missing} -- ids are a join "
                f"key and are never renumbered"
            )

    total: Counter = Counter()
    width = max(len(s) for s in per_spec)
    print(f"{'spec':<{width}}  " + "  ".join(f"{s:>13}" for s in STATUSES) + "   total")
    for spec, counts in per_spec.items():
        total += counts
        row = "  ".join(f"{counts[s] or '':>13}" for s in STATUSES)
        print(f"{spec:<{width}}  {row}   {sum(counts.values()):>5}")
    row = "  ".join(f"{total[s]:>13}" for s in STATUSES)
    print(f"{'TOTAL':<{width}}  {row}   {sum(total.values()):>5}")

    # Shipped is every row but the unbuilt one, and a `Counter` answers 0 for a
    # status nobody has: name the set through `LOOSE`, never one value at a time.
    shipped = total["verified"] + sum(total[s] for s in LOOSE)
    print(
        f"\nshipped surface: {shipped} contract rows ("
        + ", ".join(f"{total[s]} {s}" for s in LOOSE)
        + ")"
    )
    print(f"specified, not built: {total['unimplemented']}")
    for line in leaning_report(leans):
        print(line)
    for line in split_report(split_candidates):
        print(line)

    target = SPECS / LEDGER
    text = render_ledger(per_spec, total, seen)
    if "--write" in sys.argv:
        target.write_text(text)
        print(f"\nwritten {target.relative_to(ROOT)}: {sum(total.values())} rows")
    else:
        current = target.read_text() if target.exists() else None
        if current is None:
            errors.append(f"{LEDGER}: missing -- run `just contracts-write`")
        elif current != text:
            errors.append(
                f"{LEDGER}: stale -- a contract row changed and the ledger did not. "
                f"Run `just contracts-write`"
            )

    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
