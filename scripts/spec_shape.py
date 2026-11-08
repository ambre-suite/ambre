#!/usr/bin/env python3
"""Fail when a spec drifts from the shape the corpus commits to.

`scripts/spec_code.py` guards what a spec may quote. This guards how a spec is
built: how much of it is undifferentiated prose, whether a feature document
carries production history, whether the schema document stays a schema, and
whether the persisted format still matches the code.

One check per pillar that a script can hold (`ambre-spec-template.md`
section 1.G):

  * prose share  -- a reader must be able to stop after the tables, so a
                    document is capped at 25 % of its content **words**. Words
                    and not lines: a table row is word-dense, so a line-based
                    share overstates prose by about six points.
  * long lines   -- **the prose meter is blind to line length, and a reader is
                    not.** A bullet is structure to the classifier whatever its
                    length, so a document of walls measures under the cap. Held
                    to zero for every spec. A table row and a fenced line are
                    not counted, because neither can be wrapped at all -- a
                    too-wide table is fixed by a shorter cell, which no script
                    can hold.
  * feature      -- a feature document states what is, never the steps that
                    built it: no checkbox, no milestone word, no past-tense
                    production marker.
  * schedule     -- **no spec carries a backlog**, feature document or not. One
                    checkbox list, or one roadmap heading, and the spec is a
                    second schedule. Two of them make "Phase 1" name two things.
  * one home     -- **no spec restates a fact another spec owns.** Four shapes
                    are refused: a `QUALITY` section, a "Definition of Done", a
                    `PERFORMANCE` section and a `SECURITY & CONFIDENTIALITY`
                    section. The homes are `ambre-spec-testing.md`, the roadmap,
                    `ambre-spec-budgets.md` and `ambre-spec-zero-knowledge.md`;
                    the document that owns a fact is exempt from its marker.
                    Matched on the title, because such a section is numbered
                    anything or nothing at all.
  * answers      -- **a table states rules; it asks nothing.** Three bullets
                    under the contract table ask -- `Budget`, `Boundary` and
                    `Verification` -- and each must be present and non-empty.
                    **Whether the answer is true, no script holds**, exactly
                    like the `Implementation` field: forgetting becomes
                    impossible, and a false answer becomes visible in review.
  * header       -- the lines a reader gets first: `Type` and
                    `Implementation`, and `Implementation` must be one of the
                    three values `ambre-spec-template.md section 1.A` declares.
                    An unchecked vocabulary grows a fourth value within two
                    documents of being written down.
  * schema       -- the schema document keeps its budgets: prose words for the
                    whole file, words for one container note.
  * containers   -- asymmetric on purpose: a container declared in the spec with
                    no code is normal under spec-driven development; a container
                    built in the code and absent from the table is a lie.

Every metric is a hard cap, never a per-file ceiling: a breach is red.

**The prose word count is a column, not a check.** The report prints it beside
the share so a reader sees a document growing, and no threshold reads it: a
count is not comparable between a two-page spec and a register, which is what
the share is for. The schema document is the one place a raw count decides
anything, and there it is a budget the template states.

Run: `just spec-shape`. It prints the report, and fails on any defect.
"""

import re
import sys
from collections import namedtuple
from pathlib import Path

# Both vocabularies are the template's, read from it, and the other gates read
# them from the same place (`scripts/template_vocabulary.py`).
from template_vocabulary import ANSWER_LABELS, IMPLEMENTATION_VALUES


# What `classify` returns: the same partition counted twice, in lines and in
# words. Named because a six-tuple indexed by position is how a caller reads
# `[3]` and gets the wrong unit.
Shape = namedtuple(
    "Shape",
    "prose_lines structure_lines fenced_lines prose_words structure_words fenced_words",
)

# Markup that is not a word, in two classes because they must not be treated
# alike. A cell separator **separates**, so it becomes a space: without that,
# `|a|b|` counts as one word and a dense table reads as almost empty. Emphasis
# and code ticks are *inside* a word, so they are deleted: turning them into
# spaces would split `**bold**word` in two and inflate every prose count in the
# corpus by about 5 % -- a change of definition wearing the mask of a regression.
WORD_SEPARATOR = re.compile(r"\|")
WORD_NOISE = re.compile(r"[`*>#]")

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"

# Not a spec: the roadmap journal, a record of what shipped, held to the
# roadmap's one-row rule rather than to a prose budget. `specs/audits/` is out
# for the same reason, and by directory: the next audit needs no script edit.
NON_NORMATIVE = {"ambre-roadmap-journal.md"}

# A generated aggregation: no author, so no prose budget and no `Implementation`
# header. Its shape is held by the script that writes it -- `feature_inventory.py`
# and `contracts.py` each fail when the file on disk differs from what they would
# write today -- which is a stricter guard than any of the rules here, not an
# exemption from them.
GENERATED = re.compile(r"-(inventory|ledger)\.md$")

# The one register the corpus keeps outside both templates: every budget of the
# workspace, one row each. It is `ambre-spec-template.md section 4` material --
# the class the roadmap and the generated inventories belong to -- so it carries
# no `Implementation` field and declares no contract table. It is still held to
# the prose cap and to the long-line rule, because a register a reader cannot
# scan is a register nobody cites.
BUDGETS = "1-Foundations/ambre-spec-budgets.md"

# The second register, and the same class. One row per term of art: a gloss that
# names the idea, the synonyms the term forbids, and the document that defines
# it. The gloss is deliberately too short to specify anything, because a
# definition living in two files drifts in one of them -- the rule
# `ambre-spec-template.md section 0` holds over the code, and prose earns it for
# the same reason. `scripts/glossary.py` reads the table, and `prose.py` holds
# the forbidden column.
TERM_INDEX = "ambre-glossary.md"

# The template declares the pillars, so it is held to the one it can be held
# to: the prose cap. An exemption written by the author of a rule is worth
# nothing.
TEMPLATE = "ambre-spec-template.md"

# Per cent of content **words**, not lines: a table row is word-dense, so a
# line-based share overstates prose by several points. The cap is in words, and
# the figure is set to the severity a reader feels.
PROSE_MAX = 25.0

# A line a reader meets as a wall. 110 leaves a tolerance over the corpus habit
# of wrapping at 100, which nothing in `ai/ambre-writing-style.md` writes down.
LONG_LINE_COLS = 110
LONG_LINES_MAX = 0

SCHEMA_PROSE_WORDS_MAX = 600
SCHEMA_NOTE_WORDS_MAX = 40

# The schedule lives in one file. These are the two shapes a backlog takes when
# it grows back inside a spec: a ticked list, and a section to hold it.
SCHEDULE_MARKERS = [
    (re.compile(r"^\s*[-*+]\s*\[[ x~]\]"), "a checkbox list: the schedule is `ambre-roadmap.md`"),
    (
        re.compile(r"^#{2,}\s.*\b(ROADMAP|INTEGRATION PLAN|BACKLOG|PHASES)\b", re.I),
        "a roadmap section: the schedule is `ambre-roadmap.md`",
    ),
]

# The two files that *are* the schedule.
SCHEDULE_DOCS = {"ambre-roadmap.md", "ambre-roadmap-journal.md"}

# A shared fact lives in one file. These are the shapes each one takes when it
# grows back inside a spec: a section to hold it, and the one claim a quality
# section always ends on. Matched on the **title**, never on a number -- such a
# section is numbered anything or nothing at all, so a number-based ban catches
# almost none. The third element is the set of documents that **own** the fact
# and therefore carry the section legitimately.
METHOD_MARKERS = [
    (
        re.compile(r"^#{2,}\s.*\bQUALITY\b", re.I),
        "a quality section: the method is `ambre-spec-testing.md`",
        set(),
    ),
    (
        re.compile(r"\*\*Definition of Done\b", re.I),
        "a per-component done condition: the rule is one, in `ambre-roadmap.md`",
        set(),
    ),
    (
        re.compile(r"^#{2,}\s.*\bPERFORMANCE\b", re.I),
        "a performance section: a budget is a row of `ambre-spec-budgets.md`",
        # The register is the one exemption: it carries the budgets of the whole
        # workspace, so a heading naming performance is its subject, not a
        # section a feature document grew on its own.
        {BUDGETS},
    ),
    (
        re.compile(r"^#{2,}\s.*\bSECURITY\s*&\s*CONFIDENTIALITY\b", re.I),
        "a boundary audit: the boundary is `ambre-spec-zero-knowledge.md`,"
        " refined by a `Boundary` bullet in every document that reaches it;"
        " a rule about this document's own data is a row",
        # `ambre-spec-server.md` was the second owner until `F6` split it into six
        # documents, each answering the boundary in its own `Boundary` bullet
        # instead of auditing it in a section. One owner is left, which is the
        # state the rule always described.
        {"1-Foundations/ambre-spec-zero-knowledge.md"},
    ),
]

# A declared contract row, matched on its ID cell alone. `contracts.py` parses
# the whole row because it counts a status; this script only needs to know the
# table is there, so it reads the one cell that says so.
CONTRACT_ROW = re.compile(r"^\|\s*`[A-Z][A-Z0-9-]*-\d{3}`\s*\|")

# A code span inside a cell is a token, not English: a row quoting `may` names
# the word rather than using it.
CODE_SPAN = re.compile(r"`[^`]*`")

# An RFC 2119 keyword written in lower case, read inside the `Statement` cell of
# a contract row and nowhere else. The scope is what makes the measure exact:
# descriptive prose uses `may` correctly, and the same word inside a rule is
# either the modality, which is spelled in upper case, or a description, which
# belongs to a verb that is not a keyword at all (`can`, `needs`). A code span is
# dropped first, so a row quoting `may` as a token is not a breach.
RFC2119_LOWER = re.compile(
    r"(?<![\w`-])(must not|shall not|should not|may not|must|shall|should|may)(?![\w`-])"
)
# The `Statement (RFC 2119)` column, third in every contract table.
STATEMENT_CELL = 3

# The three answers a contract table owes, directly under it, read from the
# template that names them. Matched as a **bullet**, so naming a label inside a
# table cell is not an answer -- `ambre-spec-template.md` names all three to
# define them.
ANSWERS = {
    label: re.compile(rf"^\*\s+\*\*{label}\.\*\*(?P<body>.*)$")
    for label in ANSWER_LABELS
}

# A numbered section heading, `## 11 bis.` and `## 4bis.` included. This is the
# one script that reads the Latin ordinal, and it reads it to refuse it: nothing
# else in the workspace knows the word, so a heading this lets through is a
# coordinate `spec_refs.py` cannot resolve and does not report either.
SECTION_HEADING = re.compile(r"^##\s+(\d+)\s*(bis)?\s*[.:]")

# Documents that describe no code, so they carry no `Implementation` field.
NO_CODE_DOCS = SCHEDULE_DOCS | {TEMPLATE, BUDGETS, TERM_INDEX}

# The header field, and the only values it takes, read from the table that
# states them (`ambre-spec-template.md section 1.A`). One clause may follow,
# after a ` -- `, naming what is missing: "partial -- the file round trip is
# not built".
#
# The value ends at the space or at the line, never at a word boundary: a
# boundary lets `implemented.` through, and a gate reading one value where the
# corpus writes two spellings of it holds nothing. Anything after the value is a
# clause, so it announces itself with the dash the template writes.
HEADER_FIELDS = ("Type", "Implementation")

# The title, read off the first heading. It is what a search result and a diff
# hang the document on, so two documents sharing one make a reader open both.
# A title is unique in the corpus (`ambre-spec-template.md section 1.A`): the
# obvious names -- `UNDO AND REDO`, `PDF EXPORT`, `IMPORT AND EXPORT` -- are the
# ones several documents reach for. The search is bounded to the opening lines,
# so a `#` inside a later fence is never read as a heading.
TITLE = re.compile(r"^#\s+(\S.*?)\s*$", re.M)
IMPLEMENTATION = re.compile(
    r"^\*\*Implementation:\*\* ("
    + "|".join(re.escape(v) for v in IMPLEMENTATION_VALUES)
    + r")( -- \S.*)?$"
)

# A feature document describes the current state. These betray the steps.
#
# Each marker names a form, never a sense. `landed`, `used to` and `was fixed`
# do not qualify: they read a verb and guess what it is about, so "the codec
# used to encode the frame" reads as production history. A detector that guesses
# produces a number, not a rule, which is why the passive meter is absent too.
# What a past tense is really saying stays a reading, and a reading is review.
FEATURE_MARKERS = [
    (re.compile(r"^\s*[-*]\s*\[[ x]\]"), "a checkbox: the steps live in the roadmap"),
    (re.compile(r"\bMVP\b"), "a milestone word: a milestone tags a story, never a feature"),
    (re.compile(r"\bPhase\s+\d"), "a phase: the schedule lives in the roadmap"),
]

# Per-application: the schema document, and the source -- a file or a directory
# -- that names every container the code actually writes. Trame names them all in
# one module; Toile declares its constants in `state/mod.rs` and opens each
# container from the sibling module that owns it, so the whole directory is the
# source. Prose has no container yet: the crate is a `lib.rs`, so the check reads
# it and finds nothing, which is the asymmetry working -- a declared container
# with no code is normal, and this application is that case whole.
SCHEMA_DOCS = {
    "2-Trame/ambre-spec-trame-schema.md": ROOT / "crates/ambre-trame/src/logic/schema.rs",
    "3-Toile/ambre-spec-toile-schema.md": ROOT / "crates/ambre-toile/src/logic/state",
    "4-Prose/ambre-spec-prose-schema.md": ROOT / "crates/ambre-prose/src",
    "5-Murmure/ambre-spec-murmure-schema.md": ROOT / "crates/ambre-murmure/src",
}

# `format!("cells_{sheet}")` and `const SHEETS_MAP: &str = "sheets";`
CONTAINER_FMT = re.compile(r'format!\("([a-z_]+)_\{[a-z_]*\}"\)')
CONTAINER_CONST = re.compile(r'(?:pub(?:\([^)]*\))?\s+)?const\s+\w*(?:MAP|ORDER|LIST)\w*:\s*&str\s*=\s*"([a-z_]+)"')

# The second way a crate names a container: a bare `const ELEMENTS: &str =
# "elements";` reached through the Loro accessor that opens it. The name alone
# proves nothing -- `const MATERIALIZE_ORIGIN: &str = "mat";` is a commit tag --
# so a constant counts only where a call opens a container under it. A key held
# in a variable (an element id, a content hash) is not a container name and is
# not matched.
CONST_DEF = re.compile(
    r'(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Z][A-Z0-9_]*):\s*&str\s*=\s*"([a-z_]+)"')
CONTAINER_CALL = re.compile(
    r'(?:get_map|get_movable_list|get_list|get_text|insert_container)\(\s*&?([A-Z][A-Z0-9_]*)\b')

# The third way: a closed set of root names, declared with `crdt_names!` as
# `enum Root: ContainerName { Header = "header", ... }`. Every variant is a
# root container the crate opens, so each spelling has to be in the table. A
# `ContainerFamily` variant names one container per id, as `cells_{sheet}`
# does, and its spelling is the leaf the table declares.
CONTAINER_ENUM = re.compile(
    r"enum\s+\w+\s*:\s*Container(?:Name|Family)\s*\{(.*?)^\s*\}", re.S | re.M)
CONTAINER_SPELLING = re.compile(r'=\s*"([a-z_]+)"')

LIST = re.compile(r"^(?:[-*+]\s|\d+\.\s)")


def count_words(stripped: str) -> int:
    """Words in one line, markup removed.

    The same helper for prose, a table row and a fenced line, so the three are
    comparable -- which is the whole point of a share.
    """
    return len(WORD_NOISE.sub("", WORD_SEPARATOR.sub(" ", stripped)).split())


def classify(text: str) -> "Shape":
    """Lines and words, per class.

    Structure is everything a reader scans rather than reads: a table row, a
    list item **and its wrapped continuation lines**, which are the same item.
    Counting a continuation as prose makes a tight bullet list read as a wall
    on the meter while it reads fine on the page. A heading and a rule are
    neither prose nor structure: they are
    navigation, and they are skipped. A fenced block counts whole, fence
    markers included: it is neither prose to read nor a table to scan.

    Both units come out of one pass because both are used, and the reason each
    one is used is in the module docstring: the **share** and the **ceiling**
    are word counts, wrap-invariant; the line counts are what the report shows
    a reader.
    """
    lines = [0, 0, 0]
    words = [0, 0, 0]
    PROSE, STRUCTURE, FENCED = 0, 1, 2
    in_fence = in_list = False

    def add(kind: int, stripped: str) -> None:
        lines[kind] += 1
        words[kind] += count_words(stripped)

    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            add(FENCED, stripped)
            continue
        if in_fence:
            add(FENCED, stripped)
            continue
        if not stripped:
            in_list = False
            continue
        if stripped.startswith("#") or set(stripped) <= set("-=*") and len(stripped) >= 3:
            in_list = False
            continue
        if stripped.startswith("|"):
            in_list = False
            add(STRUCTURE, stripped)
            continue
        if LIST.match(stripped):
            in_list = True
            add(STRUCTURE, stripped)
            continue
        if in_list:
            add(STRUCTURE, stripped)
            continue
        add(PROSE, stripped)
    return Shape(*lines, *words)


def long_lines(text: str) -> int:
    """Count the lines a reader meets as a wall: over `LONG_LINE_COLS` columns.

    Everything that wraps is counted -- prose, a heading, a list item and its
    continuations, a blockquote line. Two things are skipped because neither can
    be wrapped: a fenced block, whole, and a table row, which *is* the line in
    GFM. That is also why a corpus-wide reflow pass was struck: 962 of the
    3 658 long lines it would have had to read could not be wrapped at all.
    """
    count = 0
    in_fence = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence or stripped.startswith("|"):
            continue
        if len(line) > LONG_LINE_COLS:
            count += 1
    return count


def prose_share(text: str) -> float:
    """Prose as a share of every content **word**.

    Words, not lines, for the reason the ceiling counts words: a share built on
    lines rises when a paragraph is wrapped, so it reports the fix the line
    ceiling asks for as a regression. Words also weigh a table honestly -- a
    wrapped table row is one line and forty words, a prose line is one line and
    fifteen, so a line-based share overstated prose by about six points.
    """
    shape = classify(text)
    total = shape.prose_words + shape.structure_words + shape.fenced_words
    return 100.0 * shape.prose_words / total if total else 0.0


def feature_issues(text: str):
    """Yield (line_no, reason) for every production marker in a feature document."""
    for i, line in enumerate(text.splitlines(), 1):
        # A table row carries the contract grammar, which names the Story an
        # `unimplemented` row is bound to. That is required, not history.
        if line.lstrip().startswith("|"):
            continue
        for pattern, reason in FEATURE_MARKERS:
            if pattern.search(line):
                yield i, f"{reason}: {line.strip()[:70]}"
                break


def title_of(text: str) -> str | None:
    """The document's title, or `None` when it opens without a heading."""
    match = TITLE.search("\n".join(text.splitlines()[:8]))
    return match.group(1) if match else None


def header_issues(text: str):
    """Yield (kind, detail) for a header a reader cannot place in four lines.

    `Type` says what kind of document this is, `Implementation` how much of the
    code it describes exists. A document that omits the second lets a 7-line
    stub read like a shipped subsystem, which is the most expensive lie the
    corpus can tell.
    """
    head = text.splitlines()[:8]
    # Each field opens its own line. A substring test let one document spell the
    # two on one line, where a reader scanning the left margin finds one field
    # and a gate anchored on the line finds none.
    for field in HEADER_FIELDS:
        if not any(line.startswith(f"**{field}:**") for line in head):
            yield "header", (
                f"no `{field}` opening a line of the first 8"
                " -- a missing field, or two fields sharing a line"
            )
    for line in text.splitlines()[:8]:
        if line.startswith("**Implementation:**") and not IMPLEMENTATION.match(line):
            yield "header", (
                f"`Implementation` is `{line[19:].strip()[:40]}` -- "
                "expected implemented | partial | design-only"
            )


def answer_issues(text: str):
    """Yield (kind, detail) for an answer a document with a contract table owes.

    A document with no row owes nothing: the three bullets answer **for a
    table**, and a document that declares none has no cost, no boundary and no
    coverage to report.
    """
    lines = text.splitlines()
    if not any(CONTRACT_ROW.match(line) for line in lines):
        return
    for label, pattern in ANSWERS.items():
        bodies = [m.group("body").strip() for line in lines if (m := pattern.match(line))]
        if not bodies:
            yield "answer", f"no `**{label}.**` bullet under the contract table"
        elif not any(bodies):
            yield "answer", f"`**{label}.**` is empty -- `none`, and why, is an answer"


def keyword_issues(text: str):
    """Yield (line_no, reason) for a lower-case RFC 2119 keyword in a rule.

    A hard error at zero. The corpus held thirteen when the rule was written,
    and each one was read rather than cased: a keyword that describes an actor
    is a description and takes a plain verb, and a negative permission takes the
    `MUST NOT` the RFC writes it with, not a lower-case `may` under a negation.
    """
    for i, line in enumerate(text.splitlines(), 1):
        if not CONTRACT_ROW.match(line):
            continue
        cells = line.split("|")
        if len(cells) <= STATEMENT_CELL:
            continue
        for match in RFC2119_LOWER.finditer(CODE_SPAN.sub("CODE", cells[STATEMENT_CELL])):
            word = match.group(0)
            yield i, (
                f'"{word}" in a rule: the modality is "{word.upper()}", '
                "and a description takes a verb that is not a keyword"
            )


def schedule_issues(text: str):
    """Yield (line_no, reason) for every backlog shape in a spec."""
    for i, line in enumerate(text.splitlines(), 1):
        # A table cell may quote a checkbox: the roadmap's own rule is quoted in
        # a Definition-of-Done row, and Prose's Markdown mapping names the GFM
        # syntax. Neither is a backlog.
        if line.lstrip().startswith("|"):
            continue
        for pattern, reason in SCHEDULE_MARKERS:
            if pattern.search(line):
                yield i, f"{reason}: {line.strip()[:70]}"
                break


def method_issues(text: str, rel: str):
    """Yield (line_no, reason) for every shared fact restated in a spec.

    `rel` is the document's path relative to `SPECS`, because a marker is a
    defect everywhere except in the one document that owns the fact.
    """
    for i, line in enumerate(text.splitlines(), 1):
        # A table cell may name the ban: `ambre-spec-template.md` section 1.F
        # lists what no document carries, and `ambre-spec-testing.md` section 10
        # records why the section was dropped. Naming a rule is not breaking it.
        if line.lstrip().startswith("|"):
            continue
        for pattern, reason, owners in METHOD_MARKERS:
            if rel in owners:
                continue
            if pattern.search(line):
                yield i, f"{reason}: {line.strip()[:70]}"
                break


def numbering_issues(text: str):
    """Yield (line_no, reason) for a section number that breaks the sequence.

    A hole is what a deletion leaves and what an insertion refuses to pay for:
    `1, 2, 6` after three sections go, `4bis` when one arrives. Nothing breaks,
    which is the problem -- a reader counts and finds three sections missing, and
    a coordinate citing a hole reads as a citation like any other. The rule is the
    cheapest one a document can carry: numbers run from 0 or 1 upward, one step at
    a time, and `bis` is not a number.
    """
    seen, expected = [], None
    for i, line in enumerate(text.splitlines(), 1):
        match = SECTION_HEADING.match(line)
        if not match:
            continue
        if match.group(2):
            yield i, f"`bis` is not a section number: {line.strip()[:60]}"
            continue
        number = int(match.group(1))
        if expected is None:
            expected = number if number in (0, 1) else 1
        if number != expected:
            yield i, (f"section {number} follows section {seen[-1]}"
                      if seen else f"the first section is {number}, not 0 or 1")
        seen.append(number)
        expected = number + 1


def declared_containers(text: str) -> set[str]:
    """The container names a schema's table declares, as their leaves.

    Only the container table counts. A mention in the hierarchy or in a note is
    not a declaration, and letting one satisfy the check is how the table drifts
    from the code while every name still appears somewhere in the file.

    The table is the one whose header names a `Container` column, and only that
    column is read. Murmure puts a `Document` column first, so the position of
    the column varies. A document name in the other cells declares nothing.

    A nested container is declared by its path (`elements/{id}/children`); the
    leaf is what the code names, so the leaf is what is compared. A cell holds
    one backticked name and nothing else.
    """
    declared = set()
    column = None
    for line in text.splitlines():
        if not line.startswith("|"):
            column = None
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if column is None:
            column = cells.index("Container") if "Container" in cells else -1
            continue
        if column < 0 or column >= len(cells):
            continue
        match = re.fullmatch(r"`[a-z_{}/]*?([a-z_]+)(?:_\{sheet\})?`", cells[column])
        if match:
            declared.add(match.group(1))
    return declared


def schema_issues(text: str, source: Path):
    """Yield (kind, detail) for every schema-document defect."""
    words = classify(text).prose_words
    if words > SCHEMA_PROSE_WORDS_MAX:
        yield "prose-budget", f"{words} prose words, budget {SCHEMA_PROSE_WORDS_MAX}"

    # Container notes: the bullet list under the notes heading, one bullet each.
    section = re.split(r"^## \d+\. CONTAINER NOTES\s*$", text, flags=re.M)
    if len(section) > 1:
        body = re.split(r"^## ", section[1], flags=re.M)[0]
        for bullet in re.findall(r"^\* \*\*(.+?)\*\*\.?\s*(.*?)(?=^\* \*\*|\Z)", body, flags=re.M | re.S):
            name, note = bullet[0].rstrip("."), bullet[1]
            count = len(re.sub(r"[`*]", "", note).split())
            if count > SCHEMA_NOTE_WORDS_MAX:
                yield "note-budget", f"{name}: {count} words, budget {SCHEMA_NOTE_WORDS_MAX}"

    if not source.exists():
        yield "source-missing", str(source)
        return
    code = "\n".join(p.read_text() for p in sorted(source.rglob("*.rs"))) \
        if source.is_dir() else source.read_text()
    built = set(CONTAINER_FMT.findall(code)) | set(CONTAINER_CONST.findall(code))
    names = dict(CONST_DEF.findall(code))
    built |= {names[c] for c in CONTAINER_CALL.findall(code) if c in names}
    built |= {name for block in CONTAINER_ENUM.findall(code)
              for name in CONTAINER_SPELLING.findall(block)}
    declared = declared_containers(text)
    for name in sorted(built - declared):
        where = source.name if source.is_file() else f"{source.parent.name}/{source.name}"
        yield "container-missing", f"`{name}` is built in {where} and absent from the table"


# How a breach of the line cap reads. The prose word count has no entry here:
# it is printed and nothing refuses it, and a tuple pairing a gate with a
# column made the report look like two rules.
LONG_LINES = f"lines over {LONG_LINE_COLS} columns"


def main() -> int:
    errors: list[str] = []
    titles: dict[str, list[str]] = {}

    for path in sorted(SPECS.rglob("*.md")):
        rel = str(path.relative_to(SPECS))
        # `Todo/` holds scoping notes for applications with no crate and no
        # contract table: not specs under the corpus's own grammar, and their
        # effort tables are the shape a scoping note legitimately takes.
        if (
            "reviews/" in rel
            or "Todo/" in rel
            or "audits/" in rel
            or path.name in NON_NORMATIVE
            or GENERATED.search(path.name)
        ):
            continue
        text = path.read_text()
        title = title_of(text)
        if title is None:
            errors.append(f"{rel}: header: no title in the first 8 lines")
        else:
            titles.setdefault(title, []).append(rel)
        share = prose_share(text)
        measured = {"prose_words": classify(text).prose_words, "long": long_lines(text)}

        # Every spec, whatever its class.
        if share > PROSE_MAX:
            errors.append(f"{rel}: prose {share:.1f} %, cap {PROSE_MAX:.0f} %")
        if measured["long"] > LONG_LINES_MAX:
            errors.append(
                f"{rel}: {measured['long']} {LONG_LINES},"
                f" cap {LONG_LINES_MAX} -- a spec wraps"
            )

        if "/features/" in f"/{rel}":
            for lineno, reason in feature_issues(text):
                errors.append(f"{rel}:{lineno}: {reason}")

        for lineno, reason in keyword_issues(text):
            errors.append(f"{rel}:{lineno}: {reason}")

        if rel not in SCHEDULE_DOCS:
            for lineno, reason in schedule_issues(text):
                errors.append(f"{rel}:{lineno}: {reason}")

        for lineno, reason in method_issues(text, rel):
            errors.append(f"{rel}:{lineno}: {reason}")
        for lineno, reason in numbering_issues(text):
            errors.append(f"{rel}:{lineno}: {reason}")

        for kind, detail in answer_issues(text):
            errors.append(f"{rel}: {kind}: {detail}")

        if rel not in NO_CODE_DOCS:
            for kind, detail in header_issues(text):
                errors.append(f"{rel}: {kind}: {detail}")

        if rel in SCHEMA_DOCS:
            for kind, detail in schema_issues(text, SCHEMA_DOCS[rel]):
                errors.append(f"{rel}: {kind}: {detail}")

        flag = "F" if "/features/" in f"/{rel}" else ("S" if rel in SCHEMA_DOCS else ("T" if rel == TEMPLATE else " "))
        print(
            f"{flag} {share:>5.1f} %  {measured['prose_words']:>5} prose words"
            f"  {measured['long']:>4} long  {rel}"
        )

    for title, shared in sorted(titles.items()):
        if len(shared) > 1:
            errors.append(f"`{title}` is the title of {len(shared)}: {', '.join(shared)}")

    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    print(f"\n{len(errors)} shape defects")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
