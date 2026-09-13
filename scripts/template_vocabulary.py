#!/usr/bin/env python3
"""The closed vocabularies of a contract row, read from the template.

`ambre-spec-template.md` states every one of them in a table a reader is meant
to consult. A script that declares them again states them twice, and the second
statement drifts without a sound: 519 rows wrote a bare `Kind` cell for months
because the gate and the template disagreed about the spelling, and each of the
two was right about itself. Nothing below is typed out here that the template
does not say first.

**The shape is the contract.** Where a heading counts the members of its table
-- "The five status values", "The eight kinds" -- the count is read with the
table and a mismatch fails here. That is the whole guard: a parse that stops
matching returns an empty set, and an empty closed set is a gate that accepts
everything. Failing at import is loud; narrowing to whatever the parse found is
not.
"""

import re
from pathlib import Path

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


def _lines() -> list[str]:
    return TEMPLATE.read_text().splitlines()


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
    the vocabulary itself. A blank line before the table is prose the anchor
    leads with; the first one after it closes the table.
    """
    rows: list[list[str]] = []
    for line in _lines()[_anchored(anchor) + 1 :]:
        stripped = line.strip()
        if not stripped.startswith("|"):
            if rows:
                break
            continue
        cells = [c.strip() for c in stripped.strip("|").split("|")]
        if set("".join(cells)) <= set("-: "):
            continue
        rows.append(cells)
    if len(rows) < 2:
        raise TemplateDrift(f"no table under {anchor!r} in {TEMPLATE.name}")
    return rows[1:]


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
    for line in _lines():
        if re.match(r"^\|\s*ID\s*\|", line):
            return tuple(c.strip() for c in line.strip().strip("|").split("|"))
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


def _mechanism_tokens() -> tuple[str, ...]:
    """Every token a `Verification` cell may be written in.

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
ID, KIND, STATEMENT, SOURCES, VERIFICATION, GAP, STATUS = COLUMNS

STATUSES = tuple(
    _token(cells[0], "statuses") for cells in _counted("**The five status values.**")
)
STATE_KINDS, OPERATION_KINDS = _kinds()
KINDS = STATE_KINDS + OPERATION_KINDS

IMPLEMENTATION_VALUES = tuple(
    _token(cells[0], "implementation")
    for cells in _table_after("`Implementation` is about **the code it describes**")
)

VERIFICATION_TOKENS = _mechanism_tokens()
# The tokens a reader cannot grep for, and the two that name no mechanism at
# all. Both sets are the template's own sentences, which say why each is what it
# is; neither is a subset a script may widen.
LOCATED_TOKENS = _sentence_tokens("**Seven tokens must carry one**")
BARE_TOKENS = _sentence_tokens("**Two tokens must not**")

GAP_OPENERS = tuple(
    _token(cells[0], "gap openers") for cells in _counted("**The six gap openers.**")
)
# What tells the one opener carrying no prose from the five that carry it: the
# template writes the others with the dash and the space the argument follows,
# and `none` is an answer whole (`ambre-spec-template.md section 1.B`).
BARE_GAP_OPENERS = tuple(o for o in GAP_OPENERS if not o.endswith(" "))
CARRIED_GAP_OPENERS = tuple(o for o in GAP_OPENERS if o.endswith(" "))


if __name__ == "__main__":
    for name, value in sorted(vars().copy().items()):
        if name.isupper() and isinstance(value, tuple):
            print(f"{name:24} {', '.join(repr(v) for v in value)}")
