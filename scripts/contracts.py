#!/usr/bin/env python3
"""Generate the contract ledger from the spec tables, and check their rules.

`ambre-contract-ledger.md` is the roll-up of every per-spec contract table. A
hand-maintained roll-up is the same failure mode as a hand-maintained test list:
it is believed, and it is wrong (it was, by one, for a week). So the file is
written by this script and by nothing else, like each application's feature
inventory. Nobody reads it; CI reads it.

The rules it enforces, beyond generating the file:

  * a status must be one of the five declared values;
  * an `unimplemented` row must name the Story that will build it ("lands with");
  * **that Story must resolve to an open row of `ambre-roadmap.md`.** The template
    says "that story is not `[x]` until the row's status has moved", and nothing
    held it: this script looked for the substring "lands with" and never read the
    roadmap. So a story could be ticked, renamed or moved under another epic and
    373 rows would keep pointing at nothing. The name is the join key, hence
    three failures: a name no row carries, a name two rows carry, and a name on a
    row that is `[x]` or that closed into `ambre-roadmap-journal.md`. The epic in
    the citation is checked too when it is there, so a moved story cannot leave a
    stale key behind;
  * **every mechanism a built row cites must resolve to a `fn` in the workspace.**
    The corpus demands it -- "a test name that resolves to a `fn` behind a Cargo
    feature no job enables is the same claim as a name that resolves to nothing"
    -- and nothing held it. The corpus once cited 82 test names of which 23
    resolved; that was cleaned by hand, so it was free to come back. It had:
    `TRAME-UI-011` cited `::editor_preview_display`, which never existed, under
    status `verified`. An `unimplemented` row is exempt: its Verification cell
    names a Story, not a test;
  * **every audit finding a row cites must exist, and every finding must be
    cited.** The template makes the citation the join between a finding and the
    rule that closes it -- "the finding stops drifting from the code in silence"
    -- and nothing held either end. Both rotted: `SRV-029` cited `audit #3`,
    which is not a finding id, and eight findings that a remediation wave closed
    had no rule at all, so nothing would go red the day the fix was reverted. A
    finding with no rule to state is declared in `NO_RULE` below, with its
    reason;
  * **the grammar of a row's own cells**, which a review held and nothing else
    did. Four drifts reached the corpus that way and each was found by reading,
    long after it landed: 519 rows across sixteen documents wrote a bare `INV`
    where the template writes `` `INV` `` -- this script stripped the backticks
    before comparing, so the vocabulary check passed and the corpus split into
    two spellings of one column; six rows stated no RFC 2119 keyword at all, so
    they read as description and bound nothing; twelve `Sources` cells cited a
    form the template does not declare, `pillar "Local-First"` for `README
    pillar "Local-First"`, and a citation nobody can resolve is a citation that
    rots; and a `Statement` carrying two rules is two rules with one status, so
    half of it can break in green. The first three are errors here. The fourth
    is a ceiling, below: the line between a bundle the kind licenses and a
    conjunction that hides a second rule is a reading, not a parse;
  * **a `Statement` stating two rules may not grow.** `FAIL` binds "the refusal,
    the notice, the rollback" and `FRAME` names everything an operation must not
    touch, so both legitimately carry a list; the other six kinds do not, and 65
    of their rows state a second MUST after an `and`. Splitting all of them is a
    reading of each; letting a sixty-sixth arrive unremarked is how the first 65
    got here. So each document carries a count in `statement_ceilings.json`, on
    the same terms as the `unverified` ceiling: it may only fall, and a growth is
    `--ceilings --raise`, one reviewed line of diff;
  * an id is never reused, and a prefix has no gap in its numbering;
  * **a document's `unverified` count may only fall.** An unbuilt row is bound to
    an open Story and cannot be forgotten; a shipped row nothing would fail on is
    bound to nothing. It was counted here, rolled into a total of 355, and adding
    one more cost an author nothing: regenerate, and the ledger reports the
    larger number in green. So each document carries a ceiling in
    `contract_ceilings.json`, and a document absent from that file has a ceiling
    of zero. Growth is still allowed -- `--ceilings --raise` -- as one reviewed
    line of diff instead of silence. `human-review` and `qa-manual` carry no
    ceiling: they are decisions, and a ceiling over them would never reach zero;
  * **the counts under a table are the table's own.** The `Verification` answer
    is written by hand, beside a table that is not, so the two drift: nine
    documents were wrong the day this check landed, and `ambre-spec-membership.md`
    claimed one `verified` row fewer than it carries. A status the answer leaves
    out claims zero, so an omission is compared like a figure. The presence of
    the label stays `spec_shape.py`, which holds the three answers together.

What this still cannot see: a row citing a test that runs, passes, and does not
test the rule. That is `TRAME-FEAT-004`'s defect, and it needs a probe or
`cargo-mutants`, not a parser.

Run: `just contracts` (fail when the file on disk is not what this would
write), `just contracts-write` (rewrite it). The check compares the whole text,
so a spec edited without a regeneration turns CI red instead of leaving a stale
roll-up behind. `just contracts-ceilings` rewrites the ceilings, lowering only.
"""

import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"
CRATES = ROOT / "crates"
TEMPLATE = "ambre-spec-template.md"  # carries example rows, not real contracts
LEDGER = "1-Foundations/ambre-contract-ledger.md"  # generated here, nowhere else
ROADMAP = "ambre-roadmap.md"  # the one schedule: every Story is a row of it
JOURNAL = "ambre-roadmap-journal.md"  # closed epics, cut there verbatim
AUDIT = "security-audit.md"  # numbered findings, cited by the rows that close them
CEILINGS = Path(__file__).resolve().parent / "contract_ceilings.json"
STATEMENT_CEILINGS = Path(__file__).resolve().parent / "statement_ceilings.json"
STATUSES = ("verified", "unverified", "human-review", "qa-manual", "unimplemented")
LOOSE = ("unverified", "human-review", "qa-manual")  # built, and no test would fail
APPS = ("1-Foundations", "2-Trame", "3-Toile", "4-Prose")

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
    # Withdrawn by the migration that gave `ambre-spec-find.md` its operation
    # tables: each restated a rule the shared document now states for every
    # application, which is the second reason this register admits.
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

# The header of a contract table, and whether it declares the `Kind` column.
# A document that declares it is held to the vocabulary and to the completeness
# rule below; one that does not is a table the column has not
# reached, so the corpus migrates a document at a time
# (`ambre-spec-template.md section 1.B`).
TABLE_HEAD = re.compile(r"^\|\s*ID\s*\|")


def header_cells(header: str) -> list[str]:
    """The column names one table header declares, in order.

    The names, not the template's order: pinning a column to its place makes a
    reordered header opt the document out of every check below in silence, which
    is the failure mode this whole file exists to refuse.
    """
    return [c.strip() for c in header.strip().strip("|").split("|")]


# What a rule binds. A state kind holds at every instant; an operation kind
# holds at one point of one operation's run, and the four of them are what
# design by contract adds. The two families never share a table: one table per
# operation is what makes a missing row visible.
STATE_KINDS = ("INV", "CONC", "OBS", "IO")
OPERATION_KINDS = ("PRE", "POST", "FRAME", "FAIL")
KINDS = STATE_KINDS + OPERATION_KINDS
# Both families carry a list by right: a `FAIL` binds the refusal, the notice and
# the rollback as one outcome, and a `FRAME` names everything one operation must
# not touch. The conjunction measure below leaves them alone.
BUNDLING_KINDS = ("FAIL", "FRAME")

# How the template writes a `Kind` cell: the word, in backticks, and nothing
# else. Comparing the word alone is what let 519 rows drop the backticks and
# pass -- a vocabulary check that forgives the spelling checks half a column.
KIND_CELL = re.compile(r"^`([A-Z]+)`$")

# What a `Statement` must carry to bind anything: one of the RFC 2119 keywords
# the template declares. A row without one describes, and a description cannot
# be falsified, so it cannot carry a status.
RFC2119 = re.compile(r"\bMUST(?:\s+NOT)?\b|\bSHOULD(?:\s+NOT)?\b|\bMAY\b")

# A second RFC 2119 keyword, behind an `and`, in a kind that does not license a
# list: "MUST paint above the content, and MUST follow the gesture" is two rules
# under one status, so the half nothing tests breaks in green. Counted, not
# refused: the split is a reading of each row (see the ceiling above).
CONJUNCTION = re.compile(
    r"\b(?:MUST(?:\s+NOT)?|SHOULD(?:\s+NOT)?|MAY)\b[^.;:]*?"
    r",?\s+and\s+(?:it\s+)?(?:MUST|SHOULD|MAY)\b"
)

# The forms a `Sources` cell may take, and the whole of them. A form off this
# list is a citation nobody can resolve: `pillar "Local-First"` names a pillar
# no reader can grep for, and `audit #3` names no finding. The document form
# accepts a path, so a citation of `1-Foundations/ambre-spec-message-bus.md` is
# read as the citation it is rather than flagged as prose.
SOURCE_FORMS = (
    re.compile(r"\bsection\s+\d"),  # this spec's own section, or a sibling's
    re.compile(r'\bREADME pillar\s+"'),  # a pillar of the README, by its name
    re.compile(r"\baudit\s+`"),  # a `security-audit.md` finding, by number
    re.compile(r"^derived\b"),  # nothing but the design itself
    re.compile(r"`[\w./-]+\.md`"),  # a named document, with the rule it cites
)


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
    if not any(form.search(sources) for form in SOURCE_FORMS):
        defects.append(
            f"{where}: `Sources` reads {sources[:60]!r}, which is none of the "
            f"declared forms -- a section of this spec or a sibling, a "
            f'`README pillar "..."`, an ``audit `H9` `` finding, a named '
            f"`document.md` with the rule it cites, or `derived`"
        )
    return defects


# A backticked Rust path in a Verification cell: `crate::module::name`, or the
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
# What a Verification cell says after "lands with": the story, then an optional
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
    """The story a Verification cell lands with, and the epic it claims."""
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
    cell: str,
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether one `unimplemented` row lands with an open story of the roadmap."""
    cite = cited_story(cell)
    if cite is None:
        return [
            f"{where}: `unimplemented` without a Story -- the Verification cell "
            f"must read 'not built -- lands with <Story>'"
        ]
    name, epic = cite
    name = name.strip('`"')  # the delimiters a citation puts around the name
    entry = schedule.get(story_key(name))
    if entry is None:
        gone = closed.get(story_key(name))
        if gone:
            return [
                f"{where}: lands with {name!r}, which closed under epic `{gone[0]}` "
                f"in {JOURNAL} -- an unbuilt row cannot wait on work that shipped"
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
            f"-- a story stays open until every row it builds has left "
            f"`unimplemented`"
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
# resolves to nothing: 41 of them named a story no gate then held, and one named
# a story that had closed into the journal while the sentence still said
# "the roadmap's". The name is on the line the word `Story` is on, so a citation
# wrapped across two lines is this defect too, and is rewrapped rather than
# excused.
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
# on: `7 `verified`, 1 `unverified`, 1 `unimplemented`.` Both separators the
# corpus uses are read, the comma and the final "and".
ANSWER = re.compile(r"^\*\s+\*\*Verification\.\*\*\s+(?P<text>.*)$")
COUNT = re.compile(r"(?P<n>\d+)\s+`(?P<status>[a-z-]+)`")
COUNT_SEP = re.compile(r",\s+(?:and\s+)?")


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


def table_defects(spec: str, table: list[tuple[int, str, str]]) -> list[str]:
    """Whether one table that declares `Kind` obeys the grammar of section 1.B.

    Two rules about the table as a whole; the cells of one row are
    `row_defects`. A table mixing a state kind with an operation kind
    is the layout collapsing back into one list, which is what hid the frame
    conditions. And the completeness rule is the reason for the column: a `POST`
    with no `FRAME` names what an operation writes and never what it leaves
    alone.

    What no rule here holds is the line between `PRE` and `FAIL`, because it is
    semantic: a `PRE` binds an obligation nothing checks, a `FAIL` states a
    refusal the operation performs. Writing both for one condition states one
    guard twice, which four rows of `trame-cell-merges` did, and doubles one
    test into two `verified` rows. The template carries the test that separates
    them; a reader applies it.
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
    if "POST" in kinds and "FRAME" not in kinds:
        defects.append(
            f"{spec}:{lineno}: this operation declares a `POST` and no `FRAME` "
            f"-- state what it MUST NOT touch, `unverified` when nothing "
            f"isolates it"
        )
    return defects


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
                f"no status of the five"
            )
        elif stated.get(status, 0) != counts.get(status, 0):
            defects.append(
                f"{where}: the `Verification` answer says {stated.get(status, 0)} "
                f"`{status}` and the table holds {counts.get(status, 0)}"
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


def statement_ceiling_defects(
    per_spec: dict[str, int], ceilings: dict[str, int]
) -> list[str]:
    """Whether any document states two rules in one row more often than it may."""
    defects = []
    for spec, measured in sorted(per_spec.items()):
        ceiling = ceilings.get(spec, 0)
        if measured > ceiling:
            defects.append(
                f"{spec}: {measured} row{'s' if measured > 1 else ''} state a "
                f"second rule after an `and`, "
                f"ceiling {ceiling} -- it may only fall. Split the row, giving the"
                " second rule an id and a status of its own, or record the debt"
                " with `just contracts-ceilings`"
            )
    return defects


def load_ceilings(path: Path) -> dict[str, int]:
    """`{spec: n}` -- what a document is still allowed to carry, per document.

    A spec the file does not name has a ceiling of zero, so a document that
    arrives loose declares it in the same commit rather than inheriting silence.
    """
    return json.loads(path.read_text()) if path.exists() else {}


def ceiling_defects(per_spec: dict[str, Counter], ceilings: dict[str, int]) -> list[str]:
    """Whether any document carries more `unverified` rows than it may."""
    defects = []
    for spec, counts in sorted(per_spec.items()):
        ceiling = ceilings.get(spec, 0)
        if counts["unverified"] > ceiling:
            defects.append(
                f"{spec}: {counts['unverified']} `unverified` rows, ceiling {ceiling}"
                " -- it may only fall. Write the mechanism, or record the debt with"
                " `just contracts-ceilings`"
            )
    return defects


def write_ceilings(
    path: Path,
    what: str,
    per_spec: dict[str, int],
    ceilings: dict[str, int],
    allow_raise: bool,
) -> int:
    """Rewrite one ceiling file from what the corpus holds today, lowering only.

    A ceiling that rises on its own is a ceiling that holds nothing, which is why
    a growth needs `--raise` and shows up as a line of diff. A document back to
    zero leaves the file: an entry of zero and no entry are the same ceiling.
    """
    out: dict[str, int] = {}
    refused: list[str] = []
    for spec, measured in sorted(per_spec.items()):
        if not measured:
            continue
        recorded = ceilings.get(spec, 0)
        if measured > recorded:
            refused.append(f"{spec}: {recorded} -> {measured}")
            if allow_raise:
                out[spec] = measured
            elif recorded:
                out[spec] = recorded
        else:
            out[spec] = measured
    path.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
    print(f"{path.name}: {len(out)} specs, {sum(out.values())} {what}")
    for line in refused:
        print(f"{'RAISED' if allow_raise else 'KEPT'} {line}", file=sys.stderr)
    if refused and not allow_raise:
        print(
            f"{len(refused)} ceilings would have risen in {path.name} and were kept:"
            " a ceiling only falls. Use `--ceilings --raise` for a deliberate growth.",
            file=sys.stderr,
        )
    return 0


def app_of(spec: str) -> str:
    """The application a spec belongs to, from the first component of its path."""
    head = spec.split("/", 1)[0]
    return head if head in APPS else "other"


def render_ledger(per_spec: dict[str, Counter], total: Counter) -> str:
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
        "* **Loose** counts a shipped row that nothing would fail on: `unverified`, `human-review`,",
        "  `qa-manual`. That number is the coverage debt of the product.",
        "* **`unverified` may only fall.** Each document's count is a ceiling in",
        "  `scripts/contract_ceilings.json`, and `contracts.py` fails when one grows. A",
        "  deliberate growth is `just contracts-ceilings-raise`, one line of reviewed diff.",
        "  `human-review` and `qa-manual` carry no ceiling: they are decisions, not debt.",
        "* How each layer is tested is `ambre-spec-testing.md`.",
        "",
        f"**{sum(total.values())} rows: {shipped} shipped, of which {loose} loose. "
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
        "| Document | Rows | Shipped | `verified` | Loose | Unbuilt |",
        "| :--- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for spec, counts in sorted(per_spec.items()):
        built = sum(counts[s] for s in STATUSES if s != "unimplemented")
        if not built:
            continue
        slack = sum(counts[s] for s in LOOSE)
        lines.append(
            f"| `{Path(spec).name}` | {sum(counts.values())} | {built} | "
            f"{counts['verified']} | {slack or ''} | {counts['unimplemented'] or ''} |"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    per_spec: dict[str, Counter] = {}
    # Rows stating a second rule after an `and`, per document, for the ceiling.
    conjunctions: Counter = Counter()
    seen: dict[str, str] = {}
    errors: list[str] = []
    per_crate, pooled = declared_fns()
    # Which `verified` rows lean on the same test, for the report below.
    leans: dict[str, list[str]] = defaultdict(list)
    schedule = read_schedule(SPECS / ROADMAP)
    closed = read_schedule(SPECS / JOURNAL)
    errors += duplicate_stories(SPECS / ROADMAP)
    findings = declared_findings()
    cited_audit: set[str] = set()

    for path in sorted(SPECS.rglob("*.md")):
        if path.name == TEMPLATE:
            continue
        counts: Counter = Counter()
        answer: tuple[int, str] | None = None
        rel = str(path.relative_to(SPECS))
        # The table being read, and whether its header declares `Kind`. A table
        # ends where the pipes do, so a heading, a bullet or a blank line
        # closes it and the rows it collected are checked as one operation.
        table: list[tuple[int, str, str]] = []
        columns: list[str] | None = None
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            stripped = line.lstrip()
            head = TABLE_HEAD.match(line)
            # A blank line does not close a table: an author who breaks a long
            # operation table over one still writes one operation, and treating
            # the halves as two tables checked each half against nothing.
            if head or (stripped and not stripped.startswith("|")):
                if columns is not None and table:
                    errors += table_defects(rel, table)
                table = []
                columns = header_cells(line) if head else None
                if columns is not None and "Kind" not in columns:
                    columns = None  # a table keyed by `ID` that is not a contract
            m = ROW.match(line)
            # a story the row's own binding already answers for, checked below
            bound: set[str] = set()
            if m and m["status"] == "unimplemented":
                cite = cited_story(m["rest"].rsplit("|", 1)[-1])
                if cite:
                    bound.add(story_key(cite[0].strip('`"')))
            errors += loose_citation_defects(
                line, f"{path.relative_to(SPECS)}:{lineno}", bound, schedule, closed
            )
            bullet = ANSWER.match(line)
            if bullet:
                answer = (lineno, bullet["text"])
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
                # `rest` starts after the ID cell, so a header index shifts by one.
                cells = [c.strip() for c in rest.split("|")]
                cell = {
                    name: cells[at - 1]
                    for at, name in enumerate(columns)
                    if 0 < at <= len(cells)
                }
                kind_cell = cell.get("Kind", "")
                statement = cell.get("Statement (RFC 2119)", "")
                errors += row_defects(where, kind_cell, statement, cell.get("Sources", ""))
                kind = kind_cell.strip("`")
                table.append((lineno, rid, kind))
                if kind not in BUNDLING_KINDS and CONJUNCTION.search(statement):
                    conjunctions[rel] += 1

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

            verification = rest.rsplit("|", 1)[-1]
            if status == "unimplemented":
                errors += story_defects(where, verification, schedule, closed)
            if status != "unimplemented" and "not built" in verification:
                errors.append(f"{where}: says 'not built' but status is {status!r}")
            if status != "unimplemented":
                for defect in unresolved_citations(verification, per_crate, pooled):
                    errors.append(f"{where}: {defect}")
            if status == "verified":
                for token in CITED.findall(verification):
                    name = token.lstrip(":").split("::")[-1]
                    if not name[:1].isupper():
                        leans[name].append(rid)

            counts[status] += 1
        if columns is not None and table:
            errors += table_defects(rel, table)
        if counts:
            per_spec[str(path.relative_to(SPECS))] = counts
            errors += answer_defects(str(path.relative_to(SPECS)), answer, counts)

    if "--ceilings" in sys.argv:
        allow_raise = "--raise" in sys.argv
        loose = {spec: counts["unverified"] for spec, counts in per_spec.items()}
        return write_ceilings(
            CEILINGS, "unverified rows", loose, load_ceilings(CEILINGS), allow_raise
        ) or write_ceilings(
            STATEMENT_CEILINGS,
            "rows stating two rules",
            dict(conjunctions),
            load_ceilings(STATEMENT_CEILINGS),
            allow_raise,
        )
    errors += ceiling_defects(per_spec, load_ceilings(CEILINGS))
    errors += statement_ceiling_defects(
        dict(conjunctions), load_ceilings(STATEMENT_CEILINGS)
    )

    errors += audit_coverage_defects(findings, cited_audit)

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

    debt = total["verified"] + total["unverified"] + total["human-review"] + total["qa-manual"]
    print(
        f"\nshipped surface: {debt} contract rows "
        f"({total['unverified']} unverified, {total['human-review']} human-review)"
    )
    print(f"specified, not built: {total['unimplemented']}")
    for line in leaning_report(leans):
        print(line)

    target = SPECS / LEDGER
    text = render_ledger(per_spec, total)
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
