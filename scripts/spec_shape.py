#!/usr/bin/env python3
"""Fail when a spec drifts from the shape the corpus commits to.

`scripts/spec_code.py` guards what a spec may quote. This guards how a spec is
built: how much of it is undifferentiated prose, whether a feature document
carries production history, whether the schema document stays a schema, and
whether the persisted format still matches the code.

One check per pillar that a script can hold (`ambre-spec-template.md`
section 1.D):

  * prose share  -- a reader must be able to stop after the tables. A migrated
                    document is capped at 25 % of its content **words**. Words
                    for the same reason the ceiling below counts them, and the
                    cap moved from 30 to 25 with the unit: a table row is
                    word-dense, so a line-based share overstated prose by about
                    six points and 30-in-words would have held nothing.
  * prose words  -- every unmigrated spec carries a ceiling on its **count** of
                    prose words, so it can improve and never regress. Not the
                    share: a share is a ratio, so deleting a table makes a
                    document that got shorter look worse, and the migration's
                    whole job is deleting tables. Measured 2026-08-21: moving the
                    twenty per-spec backlogs out cut 168 prose lines and 1 264
                    lines overall, and turned the share check red on fifteen
                    specs. And words, not lines: a metric has to be invariant to
                    the improvement it asks for. Wrapping one 613-character
                    paragraph in `ambre-spec-ui.md` turned 1 prose line into 6
                    and took the count from 67 to 104 -- the line ceiling made
                    the wrap a regression, which is the ratio bug again, one
                    metric later.
  * long lines   -- **the prose meter is blind to line length, and a reader is
                    not.** A bullet is structure to the classifier whatever its
                    length, so the template measured 29.4 %, under the cap,
                    while its section 0 was three bullets of 741, 650 and 530
                    characters. Wrapping them changed no word and no metric, and
                    it was the biggest difference in how the file reads. Held
                    like the prose count: a ceiling per unmigrated spec, zero
                    for a migrated one. A table row and a fenced line are not
                    counted, because neither can be wrapped at all -- a
                    too-wide table is fixed by a shorter cell, which no script
                    can hold.
  * feature      -- a feature document states what is, never the steps that
                    built it: no checkbox, no milestone word, no past-tense
                    production marker.
  * schedule     -- **no spec carries a backlog**, feature document or not. One
                    checkbox list, or one roadmap heading, and the spec is a
                    second schedule. Twenty of them were what made "Phase 1"
                    name four different things and let four specs call a
                    shipped behaviour unbuilt (`ambre-spec-testing.md
                    section 7`, findings 12 and 14). The ban lived in
                    `ambre-spec-template.md` alone until 2026-08-21, which is
                    to say it was a wish.
  * one home     -- **no spec restates a fact another spec owns.** Four shapes
                    are refused. A `QUALITY` section and a "Definition of Done":
                    nineteen documents held the same tiers, seams and belts with
                    the names changed, and two contradicted their own crate. A
                    `PERFORMANCE` section: eighteen documents budgeted the same
                    operations at different figures, one against its own
                    sibling. A `SECURITY & CONFIDENTIALITY` section: thirteen
                    audited a boundary they do not own, restating `ZK-001..008`.
                    The homes are `ambre-spec-testing.md`, the roadmap,
                    `ambre-spec-budgets.md` and `ambre-spec-zero-knowledge.md`;
                    the document that owns a fact is exempt from its marker.
                    Matched on the
                    title, because the numbering ran from 3 to 15 and some
                    sections carried no number at all.
  * answers      -- **a table states rules; it asks nothing.** The `QUALITY`,
                    `PERFORMANCE` and `SECURITY` sections held few facts, and
                    deleting them removed the one thing they did well: a writer
                    met three headings and had to answer them, and an empty
                    section was visible. The question comes back as three
                    bullets under the contract table -- `Budget`, `Boundary`
                    and `Verification` -- at three lines instead of three
                    pages. Each must be present and non-empty. **Whether the
                    answer is true, no script holds**, exactly like the
                    `Implementation` field: forgetting becomes impossible, and
                    a false answer becomes visible in review.
  * header       -- the lines a reader gets first: `Type` and
                    `Implementation`, and `Implementation` must be one of the
                    three values `ambre-spec-template.md section 1.A` declares.
                    A fourth value appeared within two documents of the field
                    being written down, which is what an unchecked vocabulary
                    does -- and the deleted `Status` field, which no script
                    ever checked, ended on four values and three meanings.
  * schema       -- the schema document keeps its budgets: prose words for the
                    whole file, words for one container note.
  * containers   -- asymmetric on purpose: a container declared in the spec with
                    no code is normal under spec-driven development; a container
                    built in the code and absent from the table is a lie.

Run: `just spec-shape` (report), `--check` (fail), `--update` (rewrite the
ceilings after a deliberate change). `--update` **lowers only**: a ceiling that
rises is a ceiling that does nothing, and the discipline was a sentence in this
docstring until 2026-08-24, which is to say the command blessed a regression by
being run twice. A spec that legitimately grows needs `--raise` and a reviewed
diff.
"""

import json
import re
import sys
from collections import namedtuple
from pathlib import Path

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
CEILINGS = Path(__file__).resolve().parent / "spec_shape_ceilings.json"

# Not specs: an audit, and the roadmap journal (a record of what shipped, held to
# the roadmap's one-row rule, not to a prose budget).
NON_NORMATIVE = {
    "security-audit.md",
    "ambre-roadmap-journal.md",
}

# A generated aggregation: no author, so no prose budget and no `Implementation`
# header. Its shape is held by the script that writes it -- `feature_inventory.py`
# and `contracts.py` each fail when the file on disk differs from what they would
# write today -- which is a stricter guard than any of the rules here, not an
# exemption from them.
GENERATED = re.compile(r"-(inventory|ledger)\.md$")

# A Foundations document rewritten to the migrated shape: held by the prose cap,
# not by a ceiling. A ceiling only lets an unmigrated spec improve; a document
# that has already been rewritten answers the real question instead -- may a
# reader stop after the tables.
MIGRATED_DOCS = {
    "1-Foundations/ambre-spec-testing.md",
    "1-Foundations/ambre-spec-presence.md",
    "1-Foundations/ambre-spec-docfile.md",
    "1-Foundations/ambre-spec-fixtures.md",
    "1-Foundations/ambre-spec-frame.md",
    "1-Foundations/ambre-spec-render-assets.md",
    "1-Foundations/ambre-spec-drawing-surface.md",
    "1-Foundations/ambre-spec-text-editor.md",
    "1-Foundations/ambre-spec-high-density.md",
    "1-Foundations/ambre-spec-chrome.md",
    "1-Foundations/ambre-spec-text-input.md",
    "1-Foundations/ambre-spec-accessibility.md",
    "1-Foundations/ambre-spec-adaptive-layout.md",
    "1-Foundations/ambre-spec-launcher.md",
    "1-Foundations/ambre-spec-key-verification.md",
    "1-Foundations/ambre-spec-ui.md",
    "1-Foundations/ambre-spec-clipboard.md",
    "1-Foundations/ambre-spec-zoom.md",
    "1-Foundations/ambre-spec-undo.md",
    "1-Foundations/ambre-spec-pdf-export.md",
    "1-Foundations/ambre-spec-find.md",
    "1-Foundations/ambre-spec-commands.md",
    "1-Foundations/ambre-spec-design-tokens.md",
    "1-Foundations/ambre-spec-sync.md",
    "1-Foundations/ambre-spec-sealed-envelope.md",
    "1-Foundations/ambre-spec-delta-order.md",
    "1-Foundations/ambre-spec-membership.md",
    "1-Foundations/ambre-spec-key-rotation.md",
    "1-Foundations/ambre-spec-media.md",
    "1-Foundations/ambre-spec-server.md",
    "1-Foundations/ambre-spec-session.md",
    "1-Foundations/ambre-spec-sequencer.md",
    "1-Foundations/ambre-spec-broadcast.md",
    "1-Foundations/ambre-spec-object-store.md",
    "1-Foundations/ambre-spec-organizations.md",
    "1-Foundations/ambre-spec-architecture.md",
    "1-Foundations/ambre-spec-message-bus.md",
    "1-Foundations/ambre-spec-zero-knowledge.md",
    "1-Foundations/ambre-spec-schema-version.md",
    "1-Foundations/ambre-spec-budgets.md",
}

# The one register the corpus keeps outside both templates: every budget of the
# workspace, one row each. It is `ambre-spec-template.md section 4` material --
# the class the roadmap and the generated inventories belong to -- so it carries
# no `Implementation` field and declares no contract table. It is still held to
# the prose cap and to the long-line rule, because a register a reader cannot
# scan is a register nobody cites.
BUDGETS = "1-Foundations/ambre-spec-budgets.md"

# The template declares the pillars, so it is held to the one it can be held to:
# the prose cap. It was exempt until 2026-08-21 and stood at 34.5 %, which is
# what an exemption written by the author of the rule is worth.
TEMPLATE = "ambre-spec-template.md"

# Per cent of content **words**. 25, not the 30 the line-based share used: a
# table row is word-dense, so lines overstated prose by about six points and the
# migrated corpus peaks at 23.6 %. 30 in words would be a cap that holds
# nothing; 25 reproduces the severity 30-on-lines had.
MIGRATED_PROSE_MAX = 25.0

# A line a reader meets as a wall. 110 leaves a tolerance over the corpus habit
# of wrapping at 100, so an ordinary paragraph does not trip the check; nothing
# in `ai/ambre-writing-style.md` writes the 100 down, which is how three bullets
# of 741, 650 and 530 characters reached the template.
LONG_LINE_COLS = 110
MIGRATED_LONG_MAX = 0

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

# The applications the migration has not reached. Each still carries the old
# skeleton, and deleting a section before the app is measured against the
# templates would be a blind delete. **This set only ever shrinks**, and it is
# empty when Toile and Prose have migrated. **It is empty**: every application
# has, and the four files it named are deleted.
METHOD_EXEMPT: set[str] = set()

# A shared fact lives in one file. These are the shapes each one takes when it
# grows back inside a spec: a section to hold it, and the one claim a quality
# section always ended on. Matched on the **title**, never on a number -- the
# instances were numbered 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15 or nothing at all
# depending on the file, so a number-based ban would have caught almost none.
# The third element is the set of documents that **own** the fact and therefore
# carry the section legitimately; `METHOD_EXEMPT` is added to every one of them.
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
        # `ambre-spec-architecture.md` owned the register until `F7` moved it out.
        # The register carries the budgets of the whole workspace, most of which
        # measure no part of the stack, and holding it inside a feature document
        # is what kept that document outside both templates.
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

# The three answers a contract table owes, directly under it. Matched as a
# **bullet**, so naming a label inside a table cell is not an answer --
# `ambre-spec-template.md` names all three to define them.
ANSWERS = {
    label: re.compile(rf"^\*\s+\*\*{label}\.\*\*(?P<body>.*)$")
    for label in ("Budget", "Boundary", "Verification")
}

# A numbered section heading, `## 11 bis.` and `## 4bis.` included. Kept here
# rather than imported from `spec_refs.py`: that script reads a coordinate to
# resolve it, this one reads a heading to count it, and one shared regex would tie
# the two rules together for no gain.
SECTION_HEADING = re.compile(r"^##\s+(\d+)\s*(bis)?\s*[.:]")

# Documents that describe no code, so they carry no `Implementation` field.
NO_CODE_DOCS = SCHEDULE_DOCS | {TEMPLATE, BUDGETS}

# The header field, and the only three values it takes
# (`ambre-spec-template.md section 1.A`). One clause may follow, after a `--`,
# naming what is missing: "partial -- the file round trip is not built".
HEADER_FIELDS = ("Type", "Implementation")
IMPLEMENTATION = re.compile(r"^\*\*Implementation:\*\* (implemented|partial|design-only)\b")

# A feature document describes the current state. These betray the steps.
FEATURE_MARKERS = [
    (re.compile(r"^\s*[-*]\s*\[[ x]\]"), "a checkbox: the steps live in the roadmap"),
    (re.compile(r"\bMVP\b"), "a milestone word: a milestone tags a story, never a feature"),
    (re.compile(r"\bPhase\s+\d"), "a phase: the schedule lives in the roadmap"),
    (re.compile(r"\blanded\b", re.I), "production history: git holds it"),
    (re.compile(r"\bused to\b", re.I), "production history: git holds it"),
    (re.compile(r"\bwas fixed\b", re.I), "production history: git holds it"),
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
    Counting a continuation as prose was the first version's bug -- it made a
    tight bullet list read as a wall on the meter while reading fine on the
    page. A heading and a rule are neither prose nor structure: they are
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


def header_issues(text: str):
    """Yield (kind, detail) for a header a reader cannot place in four lines.

    `Type` says what kind of document this is, `Implementation` how much of the
    code it describes exists. A document that omits the second lets a 7-line
    stub read like a shipped subsystem, which is the most expensive lie the
    corpus can tell.
    """
    head = "\n".join(text.splitlines()[:8])
    for field in HEADER_FIELDS:
        if f"**{field}:**" not in head:
            yield "header", f"no `{field}` in the first 8 lines"
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
        # A table cell may name the ban: `ambre-spec-template.md` section 1.C
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
    the corpus reached `1, 2, 6` in four documents, `1, 2, 3, 6, 7, 8` in one, and
    `4bis` / `5bis` / `13bis` in the spreadsheet's UI. Nothing broke, which is the
    problem -- a reader counts and finds three sections missing, and a coordinate
    citing a hole reads as a citation like any other. The rule is the cheapest one
    a document can carry: numbers run from 0 or 1 upward, one step at a time, and
    `bis` is not a number.
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
    # Only the container table counts. A mention in the hierarchy or in a note is
    # not a declaration, and letting one satisfy the check is how the table drifts
    # from the code while every name still appears somewhere in the file.
    #
    # A nested container is declared by its path (`elements/{id}/children`); the
    # leaf is what the code names, so the leaf is what is compared.
    declared = set()
    for row in re.findall(r"^\| `[a-z_{}/]*?([a-z_]+)(?:_\{sheet\})?` \|", text, flags=re.M):
        declared.add(row)
    for name in sorted(built - declared):
        where = source.name if source.is_file() else f"{source.parent.name}/{source.name}"
        yield "container-missing", f"`{name}` is built in {where} and absent from the table"


def load_ceilings() -> dict:
    """`{spec: {"prose": n, "long": m}}` -- one ceiling per metric, per spec."""
    return json.loads(CEILINGS.read_text()) if CEILINGS.exists() else {}


# The two ratcheted counts, and how a breach reads. Reflowing a paragraph lowers
# the second and must not move the first, which is why the first counts words.
METRICS = (
    ("prose_words", "prose words"),
    ("long", f"lines over {LONG_LINE_COLS} columns"),
)


def main() -> int:
    check = "--check" in sys.argv
    update = "--update" in sys.argv
    allow_raise = "--raise" in sys.argv
    ceilings = load_ceilings()
    fresh: dict[str, dict[str, int]] = {}
    held_by_cap: set[str] = set()
    errors: list[str] = []

    for path in sorted(SPECS.rglob("*.md")):
        rel = str(path.relative_to(SPECS))
        # `Todo/` holds scoping notes for applications with no crate and no
        # contract table: not specs under the corpus's own grammar, and their
        # effort tables are the shape a scoping note legitimately takes.
        if (
            "reviews/" in rel
            or "Todo/" in rel
            or path.name in NON_NORMATIVE
            or GENERATED.search(path.name)
        ):
            continue
        text = path.read_text()
        share = prose_share(text)
        measured = {"prose_words": classify(text).prose_words, "long": long_lines(text)}
        migrated = (
            "/features/" in f"/{rel}"
            or rel in SCHEMA_DOCS
            or rel in MIGRATED_DOCS
            or rel == TEMPLATE
        )
        fresh[rel] = measured

        if migrated:
            held_by_cap.add(rel)
            if share > MIGRATED_PROSE_MAX:
                errors.append(f"{rel}: prose {share:.1f} %, cap {MIGRATED_PROSE_MAX:.0f} %")
            if measured["long"] > MIGRATED_LONG_MAX:
                errors.append(
                    f"{rel}: {measured['long']} {METRICS[1][1]},"
                    f" cap {MIGRATED_LONG_MAX} -- a migrated document wraps"
                )
        else:
            recorded = ceilings.get(rel) or {}
            for key, label in METRICS:
                ceiling = recorded.get(key)
                if ceiling is None:
                    errors.append(f"{rel}: no {key} ceiling recorded -- run --update")
                elif measured[key] > ceiling:
                    errors.append(
                        f"{rel}: {measured[key]} {label}, ceiling {ceiling}"
                        " -- it may only fall"
                    )

        if "/features/" in f"/{rel}":
            for lineno, reason in feature_issues(text):
                errors.append(f"{rel}:{lineno}: {reason}")

        if rel not in SCHEDULE_DOCS:
            for lineno, reason in schedule_issues(text):
                errors.append(f"{rel}:{lineno}: {reason}")

        if rel not in METHOD_EXEMPT:
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

    if update:
        # A migrated document is held by the caps, not by a ceiling. Recording
        # one is dead weight, and worse: it is a number `--update` may raise,
        # which is how `ambre-spec-testing.md` carried a ceiling that grew from
        # 35 to 49 while the document it holds was capped all along.
        keep = {k: v for k, v in fresh.items() if k not in held_by_cap}
        out: dict[str, dict[str, int]] = {}
        refused: list[str] = []
        for rel, measured in keep.items():
            recorded = ceilings.get(rel) or {}
            row = {}
            for key, _ in METRICS:
                ceiling = recorded.get(key)
                if ceiling is not None and measured[key] > ceiling:
                    refused.append(f"{rel}: {key} {ceiling} -> {measured[key]}")
                    row[key] = measured[key] if allow_raise else ceiling
                else:
                    row[key] = measured[key]
            out[rel] = row
        CEILINGS.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")
        print(f"\nceilings rewritten: {len(out)} specs")
        for line in refused:
            verb = "RAISED" if allow_raise else "KEPT"
            print(f"{verb} {line}", file=sys.stderr)
        if refused and not allow_raise:
            print(
                f"{len(refused)} ceilings would have risen and were kept:"
                " a ceiling only falls. Use `--raise` for a deliberate growth.",
                file=sys.stderr,
            )
        return 0

    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    print(f"\n{len(errors)} shape defects")
    return 1 if (check and errors) else 0


if __name__ == "__main__":
    sys.exit(main())
