#!/usr/bin/env python3
"""The closed vocabularies of a contract row, read from the template.

`ambre-spec-template.md` states every one of them in a table a reader is meant
to consult. A script that declares them again states them twice, and the second
statement drifts without a sound: a gate and the template disagreeing about a
spelling are each right about themselves, and every row between them is wrong.
Nothing below is typed out here that the template does not say first.

**The shape is the contract.** Where a heading counts the members of its table
-- "The six status values", "The eight kinds" -- the count is read with the
table and a mismatch fails here. That is the whole guard: a parse that stops
matching returns an empty set, and an empty closed set is a gate that accepts
everything. Failing at import is loud; narrowing to whatever the parse found is
not.
"""

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import corpus  # noqa: E402  -- the one reader of the Markdown

TEMPLATE = Path(__file__).resolve().parent.parent / "specs" / "ambre-spec-template.md"

# The counts a heading states in words. Only what the template writes today: a
# seventh value arriving in a table whose heading says "six" is the drift this
# file exists to catch, so an unknown word fails rather than defaulting.
NUMBERS = {"two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7, "eight": 8}

BACKTICKED = re.compile(r"`([^`]+)`")


class TemplateDrift(RuntimeError):
    """The template no longer states a vocabulary where this file reads it.

    Raised at import, never caught: a gate that ran on half a vocabulary would
    pass the corpus and mean nothing.
    """


def _document() -> corpus.Document:
    return corpus.read(TEMPLATE)


def _lines() -> list[str]:
    return [line.text for line in _document().lines]


def _anchored(anchor: str) -> int:
    """The index of the one line carrying `anchor`."""
    lines = _lines()
    at = [i for i, line in enumerate(lines) if anchor in line]
    if len(at) != 1:
        raise TemplateDrift(
            f"{TEMPLATE.name} carries {len(at)} lines matching {anchor!r}, want 1"
        )
    return at[0]


def _table_after(anchor: str) -> list[list[str]]:
    """The body rows of the first table under the line carrying `anchor`.

    The header row and the `|---|` separator are dropped, so what comes back is
    the vocabulary itself.
    """
    table = _document().table_after(_anchored(anchor) + 1)
    rows = [list(row.cells) for row in table.rows] if table else []
    if not table or not rows:
        raise TemplateDrift(f"no table under {anchor!r} in {TEMPLATE.name}")
    return rows


def _counted(anchor: str) -> list[list[str]]:
    """The rows of a table whose heading says how many there are."""
    word = anchor.strip("*. ").split()[1].lower()
    want = NUMBERS.get(word)
    if want is None:
        raise TemplateDrift(f"{anchor!r} states no count this file can read")
    rows = _table_after(anchor)
    if len(rows) != want:
        raise TemplateDrift(f"{anchor!r} counts {want}, its table carries {len(rows)}")
    return rows


def _token(cell: str, anchor: str) -> str:
    """The one backticked word a vocabulary table's first column carries."""
    found = BACKTICKED.findall(cell)
    if len(found) != 1:
        raise TemplateDrift(f"{anchor!r}: {cell[:40]!r} is not one backticked value")
    return found[0]


def _sentence_tokens(anchor: str) -> tuple[str, ...]:
    """The backticked tokens of the sentence opening on `anchor`.

    The sentence, not the paragraph: both sentences this reads are followed by
    another naming a token they exclude, and reading the paragraph whole would
    collect it. The count the bold phrase opens on is held against what is
    found, on the same terms as a counted table.
    """
    lines = _lines()
    at = _anchored(anchor)
    paragraph = " ".join(
        line.strip() for line in lines[at:] if line.strip()
    ).split(". ")[0]
    found = tuple(BACKTICKED.findall(paragraph[paragraph.index(anchor) :]))
    word = anchor.strip("*").split()[0].lower()
    want = NUMBERS.get(word)
    if want is None:
        raise TemplateDrift(f"{anchor!r} states no count this file can read")
    if len(found) != want:
        raise TemplateDrift(f"{anchor!r} counts {want}, its sentence names {len(found)}")
    return found


def _columns() -> tuple[str, ...]:
    """The column names of a contract table, in the order the template writes.

    Read by position afterwards, which every reader of a *document* is forbidden
    to do -- a reordered header there opts the document out of every check in
    silence. Here it is the declaration itself, so its order is the one thing
    that cannot be wrong, and unpacking it is what fails the day a column is
    added without a reader for it.
    """
    for table in _document().tables:
        if table.header[:1] == ("ID",):
            return table.header
    raise TemplateDrift(f"{TEMPLATE.name} declares no contract table header")


def _kinds() -> tuple[tuple[str, ...], tuple[str, ...]]:
    """The eight kinds, split by what they bind: the state, or one operation.

    The `Binds` column is what says which family a kind is in, and the two
    families never share a table. Reading the split from the same row that
    states it keeps the rule and the vocabulary one statement.
    """
    anchor = "**The eight kinds.**"
    state, operation = [], []
    for cells in _counted(anchor):
        kind = _token(cells[0], anchor)
        if cells[1].startswith("the state"):
            state.append(kind)
        elif cells[1].startswith("one operation"):
            operation.append(kind)
        else:
            raise TemplateDrift(f"{anchor}: {kind} binds {cells[1][:30]!r}, neither family")
    return tuple(state), tuple(operation)


def _fence_tags() -> tuple[str, ...]:
    """Every tag a fence may carry, the untagged fence included.

    One row names the artefact languages together, so the cell is read for every
    backticked value and not for one. The untagged fence is the tag no cell can
    spell: the row allowing it says `none` in words, and the empty string joins
    the set here rather than in the gate that reads it.
    """
    anchor = "**Fences.**"
    tags: list[str] = []
    for cells in _table_after(anchor):
        tags += [t for t in BACKTICKED.findall(cells[0]) if t.strip()]
        if cells[0].lower().startswith("none"):
            tags.append("")
    if "" not in tags:
        raise TemplateDrift(f"{anchor}: no row allows the untagged fence")
    return tuple(tags)


def _mechanism_tokens() -> tuple[str, ...]:
    """Every token a `Mechanism` cell may be written in.

    The table's first two rows are the belt -- a test path and its continuation
    -- and a path is not a token: it locates itself, and `MECHANISM_PATH` in
    `contracts.py` is what reads it. One row names six upper rungs at once, so
    the cell is read for every backticked value and not for one. A value that is
    blank is the `` `` `` a path row wraps itself in, and it names nothing.
    """
    anchor = "**The mechanism tokens.**"
    tokens: list[str] = []
    for cells in _table_after(anchor):
        found = BACKTICKED.findall(cells[0])
        tokens += [t for t in found if t.strip() and "::" not in t]
    if not tokens:
        raise TemplateDrift(f"{anchor}: no token in the table")
    return tuple(tokens)


COLUMNS = _columns()
ID, KIND, STATEMENT, SOURCES, MECHANISM, GAP, STATUS = COLUMNS

STATUSES = tuple(
    _token(cells[0], "statuses") for cells in _counted("**The six status values.**")
)
STATE_KINDS, OPERATION_KINDS = _kinds()
KINDS = STATE_KINDS + OPERATION_KINDS

IMPLEMENTATION_VALUES = tuple(
    _token(cells[0], "implementation")
    for cells in _table_after("`Implementation` is about **the code it describes**")
)

FENCE_TAGS = _fence_tags()

MECHANISM_TOKENS = _mechanism_tokens()
# The tokens a reader cannot grep for, and the two that name no mechanism at
# all. Both sets are the template's own sentences, which say why each is what it
# is; neither is a subset a script may widen.
LOCATED_TOKENS = _sentence_tokens("**Eight tokens must carry one**")
BARE_TOKENS = _sentence_tokens("**Two tokens must not**")

# The three questions a contract table does not answer, in the order the
# template asks them. Two gates read them: one holds every document to all
# three, the other holds the three to a word budget.
ANSWER_LABELS = tuple(
    _token(cells[0], "answers") for cells in _counted("**The three answers.**")
)

GAP_OPENERS = tuple(
    _token(cells[0], "gap openers") for cells in _counted("**The five gap openers.**")
)
# What tells the one opener carrying no prose from the four that carry it: the
# template writes the others with the dash and the space the argument follows,
# and `none` is an answer whole (`ambre-spec-template.md section 1.B`).
BARE_GAP_OPENERS = tuple(o for o in GAP_OPENERS if not o.endswith(" "))
CARRIED_GAP_OPENERS = tuple(o for o in GAP_OPENERS if o.endswith(" "))

# What each field of the field table announces a count of. A field is here when
# its rule says "one of ...": the map resolves that announcement to the set the
# gates actually enforce, so the two cannot disagree. `Sources` counts in place,
# because the six forms are written in its own cell and nowhere else -- in that
# cell the bold run marks a form and marks nothing else.
FIELD_COUNTS = {
    "Kind": lambda cell: len(KINDS),
    "Sources": lambda cell: len(BOLD_RUN.findall(cell)),
    "Gap": lambda cell: len(GAP_OPENERS),
    "Status": lambda cell: len(STATUSES),
}

# "one of the eight below", "in one of five forms". The number is a word, and
# the phrase is the one a field uses to point at a closed set.
ANNOUNCED_COUNT = re.compile(r"\bone of (?:the )?([a-z]+)\b")
BOLD_RUN = re.compile(r"\*\*(.+?)\*\*")


def _field_counts() -> None:
    """Whether each field's announced count matches the set it points at.

    The field table is what a contributor reads before the vocabulary tables
    sixty lines below it, and it states their sizes a second time. A second
    statement drifts: splitting `unverified` into three statuses grew the table
    to six and left the `Status` field announcing five, and no gate saw it
    because no gate reads prose. This one does, on the two terms the rest of
    this file already works on -- the count is a word, and the set is the one
    the gates enforce.

    A field announcing a count `FIELD_COUNTS` does not name fails rather than
    passing unread: an announcement nothing resolves is the next drift, waiting.
    """
    anchor = "**The seven fields.**"
    for cells in _counted(anchor):
        field, rule = _token(cells[0], anchor), cells[1]
        found = ANNOUNCED_COUNT.search(rule)
        if found is None:
            if field in FIELD_COUNTS:
                raise TemplateDrift(
                    f"{anchor}: the `{field}` field announces no count, and "
                    f"`FIELD_COUNTS` holds one for it. Restore the announcement, "
                    f"or drop the entry"
                )
            continue
        word = found.group(1)
        want = NUMBERS.get(word)
        if want is None:
            raise TemplateDrift(
                f"{anchor}: the `{field}` field says 'one of {word}', and that "
                f"is no number this file reads"
            )
        if field not in FIELD_COUNTS:
            raise TemplateDrift(
                f"{anchor}: the `{field}` field announces {want}, and nothing "
                f"says what it counts. Add it to `FIELD_COUNTS`"
            )
        have = FIELD_COUNTS[field](rule)
        if have != want:
            raise TemplateDrift(
                f"{anchor}: the `{field}` field announces {want}, and what it "
                f"points at carries {have}"
            )


_field_counts()


if __name__ == "__main__":
    for name, value in sorted(vars().copy().items()):
        if name.isupper() and isinstance(value, tuple):
            print(f"{name:24} {', '.join(repr(v) for v in value)}")
