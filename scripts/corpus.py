#!/usr/bin/env python3
"""The Markdown corpus, parsed once: the one reader every gate shares.

Ten gates read `specs/`, and each one used to walk the lines on its own. Two of
them read one table two ways: one gate closed a contract table on a blank line
and the next did not, so the second half of an operation was indexed against
no header in one reader and checked in the other. A fence was a line starting
with three backticks for one gate, the same line once indented for another, and
nothing at all for a third, which counted a row quoted inside a fence.

So the shape of a document is decided here, once, and every gate reads the
result. A gate keeps its rules and its messages. It never decides again what a
table, a fence, a section or a paragraph is.

**Line kinds.** Every line has exactly one kind, read in this order:

| Kind | The line |
| :--- | :--- |
| `fence` | opens or closes a fence: three backticks, indent allowed |
| `code` | sits inside a fence |
| `blank` | carries nothing but white space |
| `heading` | opens on `#` |
| `rule` | is three or more of `-`, `=`, `*` and nothing else |
| `table` | opens on `|` |
| `text` | anything else: prose, a list item, a quote, a field |

**A table** is a run of `table` lines. Its first line is the header, and the
line under it is the separator when it carries only dashes and colons. A blank
line does not close a table: a run that follows a table across blank lines and
opens on no header of its own carries on the table above. An author who breaks
a long operation over one blank line still writes one operation. A line opening
on a contract id is a row wherever it sits, so a table missing its header still
declares the row, and the contract gate refuses the missing header.

**A contract table** is a table whose header opens on `ID` and declares the
`Kind` column the template names. A contract row is a row of any table that
opens on an id and closes on a status, so the gate can refuse a row under a
header that is not a contract header. Its cells are read by the names its own
header declares, never by position.

**A section** opens on `## <number>.` and runs to the next numbered section.
**A field** is a `text` line opening on `**Name:**`. **A labelled bullet** is a
`* **Label.**` line, with the indented lines that carry it on. **A paragraph**
is a run of prose lines, opened by a blank line, a list item, the edge of a
quote, a bold heading or a rule.

A document is parsed once per text, whatever the number of gates that read it
in one run, and a file is read again at each call so a writer never reads its
own stale output.
"""

from __future__ import annotations

import re
from bisect import bisect_left
from dataclasses import dataclass, field
from functools import cached_property, lru_cache
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"

FENCE_MARK = "```"

BLANK = "blank"
FENCE = "fence"
CODE = "code"
TABLE = "table"
HEADING = "heading"
RULE = "rule"
TEXT = "text"

# A list item, read on the stripped line: what makes a line and its wrapped
# continuations structure rather than prose.
LIST_ITEM = re.compile(r"^(?:[-*+]\s|\d+\.\s)")
# The same marker with its indent, as the prose reader strips it from a line.
BULLET = re.compile(r"^\s*(?:[*\-+]|\d+\.)\s+")
# A block quote marker. The line under it is prose; the marker is not.
QUOTE = re.compile(r"^\s*>\s?")
# A line that is one bold run and nothing else: the template writes a heading so.
BOLD_HEADING = re.compile(r"^\*\*[^*]+\*\*$")
# A horizontal rule, as the prose reader reads one inside a line's body.
RULE_BODY = re.compile(r"^\s*(?:-{3,}|\*{3,}|_{3,})\s*$")
# An ATX heading. A `#` line that is no heading -- `#[derive]` -- has level 0.
HEADING_LINE = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
# A numbered section heading, `## 4.` or `## 4:`.
SECTION = re.compile(r"^##\s+(\d+)\s*[.:]")
# A header field: `**Type:** Vertical (feature)`.
FIELD = re.compile(r"^\*\*(?P<name>[A-Z][A-Za-z ]*):\*\*\s*(?P<value>.*?)\s*$")
# A labelled bullet: `* **Verification.** 7 `verified`.`
LABELLED = re.compile(r"^\*\s+\*\*(?P<label>[^*]+?)\.\*\*(?P<body>.*)$")
# A contract row: an id cell, the cells, a status cell last.
CONTRACT_ROW = re.compile(
    r"^\|\s*`(?P<id>[A-Z][A-Z0-9-]*-\d{3})`\s*\|"
    r"(?P<rest>.*)\|\s*(?P<status>[a-z-]+)\s*\|\s*$"
)
# The separator under a table header: dashes, colons, spaces and pipes only.
SEPARATOR_CHARS = set("-: ")


class Line(NamedTuple):
    """One line of a document, numbered from 1, with its kind."""

    no: int
    text: str
    kind: str
    # A `text` line inside a list item: the item, or its wrapped continuation.
    listed: bool = False


@dataclass(frozen=True)
class Heading:
    no: int
    level: int
    title: str
    text: str


@dataclass(frozen=True)
class Section:
    """`## <number>.` and every line up to the next numbered section."""

    number: str
    start: int
    end: int


@dataclass(frozen=True)
class Fence:
    """A fenced block: its two markers, its tag, and the lines between them."""

    start: int
    end: int | None  # `None`: the fence never closes
    tag: str
    body: tuple[Line, ...]


@dataclass(frozen=True)
class Field:
    no: int
    name: str
    value: str
    text: str


@dataclass(frozen=True)
class Bullet:
    """A `* **Label.**` bullet, its own wrapped lines, and the whole item.

    `lines` is the bullet's own text: its line and the indented lines that carry
    it on, up to a nested bullet. `item` adds every indented line below, nested
    bullets included, since an item owns what it indents. A writer rewrapping
    the bullet rewraps `lines`; a reader counting what the item spends counts
    `item`.
    """

    no: int
    end: int
    label: str
    body: str
    lines: tuple[str, ...]
    item: tuple[str, ...] = ()

    @property
    def joined(self) -> str:
        """The body and its continuations, as one line of text."""
        return " ".join([self.body.lstrip()] + [line.strip() for line in self.lines[1:]])


def fence_tag(line: str) -> str:
    """The base language token of a fence open line (`` ```rust,ignore `` -> `rust`)."""
    info = line.strip()[len(FENCE_MARK):].strip()
    return re.split(r"[,\s]", info)[0] if info else ""


def cells_of(line: str) -> tuple[str, ...]:
    """The cells of one table line, stripped, the outer pipes dropped."""
    return tuple(c.strip() for c in line.strip().strip("|").split("|"))


def is_separator(line: str) -> bool:
    joined = "".join(cells_of(line))
    return "-" in joined and set(joined) <= SEPARATOR_CHARS


class TableRow(NamedTuple):
    no: int
    text: str
    cells: tuple[str, ...]


@dataclass(eq=False)
class Table:
    """A table: its header, its body rows, and the line that closes it."""

    start: int
    end: int
    header: tuple[str, ...]
    lines: tuple[Line, ...]
    rows: tuple[TableRow, ...]
    closer: Line | None = None

    @property
    def contract(self) -> bool:
        """Whether the header opens on `ID` and declares the `Kind` column."""
        from template_vocabulary import ID, KIND  # the template parses through here

        return self.header[:1] == (ID,) and KIND in self.header


@dataclass(frozen=True, eq=False)
class ContractRow:
    no: int
    id: str
    status: str
    rest: str
    text: str
    table: Table
    # The cells by the names the header declares; empty under a header that is
    # not a contract header, so a gate refusing that header still sees the row.
    cells: dict = field(default_factory=dict)


def row_cells(columns: tuple[str, ...] | list[str] | None, rest: str) -> dict[str, str]:
    """One contract row's cells, keyed by the name its own header declares.

    `rest` starts after the `ID` cell, so a header index shifts by one. A cell
    read by its position is what a new column breaks in silence.
    """
    if not columns:
        return {}
    cells = [c.strip() for c in rest.split("|")]
    return {name: cells[at - 1] for at, name in enumerate(columns) if 0 < at <= len(cells)}


def paragraphs(lines) -> list[tuple[int, str]]:
    """Group `(line, body, opens, is_prose)` entries into paragraphs.

    A gap in the line numbers, an entry that opens one and an entry that is not
    prose each close the paragraph above. A table row is not a sentence, so it
    joins none. The prose reader of a Rust comment builds the same entries, so
    both kinds of file are cut into paragraphs by this one rule.
    """
    out: list[tuple[int, str]] = []
    start: int | None = None
    buffer: list[str] = []
    previous: int | None = None
    for lineno, text, opens, is_prose in lines:
        if not is_prose:
            if start is not None:
                out.append((start, " ".join(buffer)))
                start, buffer = None, []
            previous = lineno
            continue
        if start is not None and (opens or lineno != previous + 1):
            out.append((start, " ".join(buffer)))
            start, buffer = None, []
        if start is None:
            start = lineno
        buffer.append(text)
        previous = lineno
    if start is not None:
        out.append((start, " ".join(buffer)))
    return out


def indented(line: str) -> bool:
    """Whether a line is indented and not blank: it belongs to the item above."""
    return line[:1] in (" ", "\t") and bool(line.strip())


def continues_bullet(line: str) -> bool:
    """Whether a line carries on the bullet above it: indented, not blank, and
    not a nested bullet of its own."""
    return indented(line) and not line.strip().startswith(("* ", "- "))


class Document:
    """One Markdown text, parsed. Every structure is read off `lines`."""

    def __init__(self, text: str) -> None:
        self.text = text
        self.lines: tuple[Line, ...] = tuple(self._classify(text))

    @staticmethod
    def _classify(text: str):
        in_fence = in_list = False
        for no, raw in enumerate(text.splitlines(), 1):
            stripped = raw.strip()
            listed = False
            if stripped.startswith(FENCE_MARK):
                in_fence = not in_fence
                kind = FENCE
            elif in_fence:
                kind = CODE
            elif not stripped:
                kind, in_list = BLANK, False
            elif stripped.startswith("#"):
                kind, in_list = HEADING, False
            elif len(stripped) >= 3 and set(stripped) <= set("-=*"):
                kind, in_list = RULE, False
            elif stripped.startswith("|"):
                kind, in_list = TABLE, False
            else:
                kind = TEXT
                if LIST_ITEM.match(stripped):
                    in_list = True
                listed = in_list
            yield Line(no, raw, kind, listed)

    def line(self, no: int) -> Line:
        return self.lines[no - 1]

    @cached_property
    def headings(self) -> tuple[Heading, ...]:
        out = []
        for line in self.lines:
            if line.kind != HEADING:
                continue
            found = HEADING_LINE.match(line.text.strip())
            if found:
                out.append(Heading(line.no, len(found.group(1)), found.group(2), line.text))
            else:
                out.append(Heading(line.no, 0, "", line.text))
        return tuple(out)

    def heading_above(self, no: int, levels: tuple[int, ...] = (2, 3)) -> Heading | None:
        """The last heading of one of `levels` above line `no`."""
        found = None
        for heading in self.headings:
            if heading.no >= no:
                break
            if heading.level in levels:
                found = heading
        return found

    @cached_property
    def title(self) -> Heading | None:
        """The first level-1 heading."""
        return next((h for h in self.headings if h.level == 1), None)

    @cached_property
    def sections(self) -> tuple[Section, ...]:
        opened = [(h.no, m.group(1)) for h in self.headings
                  if h.level == 2 and (m := SECTION.match(h.text))]
        ends = [no - 1 for no, _ in opened[1:]] + [len(self.lines)]
        return tuple(Section(number, start, end)
                     for (start, number), end in zip(opened, ends))

    def section(self, number: str) -> Section | None:
        return next((s for s in self.sections if s.number == number), None)

    @cached_property
    def fences(self) -> tuple[Fence, ...]:
        out = []
        start = None
        tag = ""
        body: list[Line] = []
        for line in self.lines:
            if line.kind == FENCE:
                if start is None:
                    start, tag, body = line.no, fence_tag(line.text), []
                else:
                    out.append(Fence(start, line.no, tag, tuple(body)))
                    start = None
            elif start is not None:
                body.append(line)
        if start is not None:
            out.append(Fence(start, None, tag, tuple(body)))
        return tuple(out)

    @cached_property
    def tables(self) -> tuple[Table, ...]:
        runs: list[list[Line]] = []
        # Whether each run follows a table across blank lines alone.
        follows: list[bool] = []
        previous = last = None  # the line above, and the last line not blank
        for line in self.lines:
            if line.kind == TABLE:
                if previous is not None and previous.kind == TABLE:
                    runs[-1].append(line)
                else:
                    runs.append([line])
                    follows.append(last is not None and last.kind == TABLE)
            previous = line
            if line.kind != BLANK:
                last = line
        tables: list[Table] = []
        for run, after_table in zip(runs, follows):
            # `ID` is the contract header, which opens a table even with no
            # separator under it: the contract gate has always read it so.
            opens_header = not CONTRACT_ROW.match(run[0].text) and (
                not after_table
                or (len(run) > 1 and is_separator(run[1].text))
                or cells_of(run[0].text)[:1] == ("ID",)
            )
            if not opens_header and after_table and tables:
                table = tables[-1]
                table.lines += tuple(run)
                table.end = run[-1].no
                table.rows += tuple(TableRow(l.no, l.text, cells_of(l.text)) for l in run)
                continue
            header: tuple[str, ...] = ()
            body = run
            if opens_header:
                header, body = cells_of(run[0].text), run[1:]
                if body and is_separator(body[0].text):
                    body = body[1:]
            tables.append(Table(
                start=run[0].no, end=run[-1].no, header=header, lines=tuple(run),
                rows=tuple(TableRow(l.no, l.text, cells_of(l.text)) for l in body),
            ))
        for table in tables:
            table.closer = next(
                (l for l in self.lines[table.end:] if l.kind != BLANK), None
            )
        return tuple(tables)

    def table_at(self, no: int) -> Table | None:
        """The table carrying line `no`."""
        starts = [t.start for t in self.tables]
        at = bisect_left(starts, no + 1) - 1
        if at >= 0 and self.tables[at].start <= no <= self.tables[at].end:
            return self.tables[at]
        return None

    def table_after(self, no: int) -> Table | None:
        """The first table opening below line `no`."""
        return next((t for t in self.tables if t.start > no), None)

    @cached_property
    def contract_rows(self) -> tuple[ContractRow, ...]:
        """Every contract row, in line order, with the table that holds it.

        Read apart from `tables`, because the template's own vocabulary is read
        through `tables` and names the `Kind` column this needs.
        """
        out = []
        for table in self.tables:
            columns = table.header if table.contract else None
            out += [
                ContractRow(r.no, m["id"], m["status"], m["rest"], r.text, table,
                            row_cells(columns, m["rest"]))
                for r in table.rows if (m := CONTRACT_ROW.match(r.text))
            ]
        return tuple(out)

    def rows_of(self, table: Table) -> tuple[ContractRow, ...]:
        """The contract rows one table holds."""
        return tuple(r for r in self.contract_rows if r.table is table)

    @cached_property
    def fields(self) -> tuple[Field, ...]:
        out = []
        for line in self.lines:
            if line.kind == TEXT and (m := FIELD.match(line.text.strip())):
                out.append(Field(line.no, m["name"], m["value"], line.text))
        return tuple(out)

    def field(self, name: str, within: int | None = None) -> Field | None:
        """The first field of that name, in the first `within` lines when given."""
        return next((f for f in self.fields
                     if f.name == name and (within is None or f.no <= within)), None)

    @cached_property
    def bullets(self) -> tuple[Bullet, ...]:
        out = []
        lines = self.lines
        for at, line in enumerate(lines):
            if line.kind != TEXT or not (m := LABELLED.match(line.text)):
                continue
            end = at + 1
            while end < len(lines) and continues_bullet(lines[end].text):
                end += 1
            last = end
            while last < len(lines) and indented(lines[last].text):
                last += 1
            out.append(Bullet(line.no, lines[end - 1].no, m["label"], m["body"],
                              tuple(l.text for l in lines[at:end]),
                              tuple(l.text for l in lines[at:last])))
        return tuple(out)

    def bullet_at(self, no: int) -> Bullet | None:
        return next((b for b in self.bullets if b.no == no), None)

    @cached_property
    def quotes(self) -> tuple[tuple[Line, ...], ...]:
        """Each run of block quote lines."""
        out: list[list[Line]] = []
        previous = None
        for line in self.lines:
            if line.kind == TEXT and line.text.startswith(">"):
                if previous is not None and previous.no == line.no - 1 and out:
                    out[-1].append(line)
                else:
                    out.append([line])
                previous = line
        return tuple(tuple(run) for run in out)

    @cached_property
    def prose(self) -> tuple[tuple[int, str, bool, bool], ...]:
        """`(line, body, opens, is_prose)` for every line a prose reader reads.

        A fence, a blank line and a heading drop out. A table row stays, marked
        as structure: the hard rules read it, and no sentence joins it. The body
        of a prose line has its quote and list markers removed.
        """
        out = []
        quoted = False
        for line in self.lines:
            if line.kind in (FENCE, CODE, BLANK, HEADING):
                continue
            if line.kind == TABLE:
                out.append((line.no, line.text.strip(), True, False))
                continue
            was_quoted, quoted = quoted, bool(QUOTE.match(line.text))
            body = BULLET.sub("", QUOTE.sub("", line.text)).strip()
            if not body:
                continue
            opens = bool(BULLET.match(line.text)) or quoted != was_quoted
            opens = opens or bool(BOLD_HEADING.match(body)) or bool(RULE_BODY.match(body))
            out.append((line.no, body, opens, True))
        return tuple(out)

    @cached_property
    def paragraphs(self) -> list[tuple[int, str]]:
        return paragraphs(self.prose)


@lru_cache(maxsize=None)
def parse(text: str) -> Document:
    """The parsed form of one text, built once per distinct text."""
    return Document(text)


def read(path: Path) -> Document:
    """One file, parsed. The text is read at each call, the parse is cached."""
    return parse(path.read_text(encoding="utf-8"))


def as_document(source: Document | str) -> Document:
    """A document, from a parsed one or from its text."""
    return source if isinstance(source, Document) else parse(source)


def markdown(root: Path) -> list[Path]:
    """Every Markdown file under `root`, in path order."""
    return sorted(root.rglob("*.md"))
