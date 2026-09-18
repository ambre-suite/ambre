#!/usr/bin/env python3
"""Fail when a citation names a document, or a section of one, that does not exist.

The corpus cites a document by its name, never by a path or a number: a comment
writes `` `trame-clipboard` `` and a spec writes `` `ambre-spec-sync.md` ``. A
name is stable, greppable and readable, which is why `ambre-spec-template.md`
section 1.D makes it a pillar. Nothing resolved it. Splitting one spec into
fifteen feature documents left every stale citation silent, and a reader follows
a name to a document that no longer holds the rule, or to no document at all.
`prose-engine` named no document for as long as three Prose specs have existed.

Three namespaces, one resolver.

**The document name.** Five forms are recognised, and two of them are why this
file was rewritten. The name and the coordinate sit inside one backtick span more
often than not, and the pattern that required a closing backtick after the name
saw none of them: 423 citations were outside the resolver while it reported
success. The fifth is the same failure a second time -- a name pattern stopping at
`/` read a prefixed sibling as no citation at all, 311 of them (finding 68).

  `trame-sort`                          -> specs/**/ambre-spec-trame-sort.md
  `ambre-spec-docfile.md`               -> specs/**/ambre-spec-docfile.md
  `architecture section 5`              -> a short name, when it is unambiguous
  `ambre-spec-trame-logic.md sec. 9`    -> the name inside the coordinate's span
  `features/ambre-spec-toile-links.md`  -> the prefix is dropped, the name resolves

A short name is accepted only when it resolves to exactly **one** document. That
is what makes `` `architecture section 5` `` legitimate and `` `ui section 5bis` ``
in a Toile document a defect: `ui` is `ambre-spec-ui.md` corpus-wide, so a reader
following it lands in Foundations, not in a Toile document.

**The section coordinate.** A citation that carries `section N` must name a
section the target document has. The number is checked, never the letter that may
follow it: `section 5.B` is verified as `5`. Renumbering a document silently
breaks every coordinate aimed at it, and the corpus had accumulated coordinates
pointing at sections deleted three commits earlier.

`§N` is the same coordinate, and it was the spelling `crates/` used. Both are
read, so neither can hide from the other.

**The coordinate namespace is the corpus. The code is out of it, deliberately.**
A section number is the one part of a citation that rots: renumbering ten
documents re-pointed 108 coordinates, and the code held 906 more that named no
document at all. A code comment cites the **document name** and, where the rule
has one, its **contract id** -- the two namespaces an ID is minted once and never
renumbered in. Nothing in `crates/` needs a number to be followed: a name picks
one document out of 57, and a grep finds the rule inside it. So a coordinate
written in a `.rs` file is neither resolved nor reported here; what is held there
is the name.

Which document a coordinate belongs to is decided by adjacency: the name it
touches, the coordinate it chains from (`section 6/section 7`), or, failing both,
the document doing the citing. A coordinate that resolves to no document at all is
reported too -- `` Trame `logic section 11 bis` `` names no file, so no reader can
follow it either.

Adjacency crosses a line break. A citation wrapped between `` `architecture` ``
and `section 5.F` is one citation to a reader and was two to the resolver, which
fell back on the citing document and found a section 5 that existed and said
something else. The glue may therefore carry the end of one line, the newline,
and the comment or quote marker that opens the next.

**The code path.** A document names a file of the workspace --
`` `crates/ambre-sync/src/pipeline.rs` `` -- and 329 such paths sit in the corpus
and in the code. Nothing resolved one: this file used to declare a `crates/` path
"not a citation", because it is not a *document*, and a rename left the reader
walking to a file that no longer exists.

The split above holds here unchanged. **The path is a name, and the line range is
a coordinate.** So a path is resolved everywhere, a record included, and a range
is refused everywhere but in a record: `rate_limit.rs:131-197` is exact on the day
an audit is written and wrong at the next edit of the file, which is what
`ambre-spec-template.md` section 1.D means by `crates/` carrying no section
number. The range is cut before the path is resolved, so a finding keeps both and
only the half that rots is reported. A path carrying `*` is a shape rather than a
file, and it resolves when it matches at least one.

This resolves three namespaces and checks nothing else. It does not grade a
citation, rewrite it into a rule id, or read what the target says.

`specs/reviews/` is out of the corpus, exactly as in `spec_code.py`. A dated
record is out of the *coordinate* namespace: `specs/audits/` and its kin state
what the corpus held the day they were written, and a finding that quotes a
since-deleted heading is doing its job. Their document names are still resolved,
less the names the corpus has deliberately deleted -- `RETIRED`, which is that
list and nothing wider.

Run: `just spec-refs` (list), `just spec-refs --check` (fail on any dead name).
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"
CRATES = ROOT / "crates"
RULES = ROOT / "ai"

# The three applications. A document of an application is named
# `ambre-spec-<app>-<rest>.md`; a citation may drop the fixed halves.
APPS = ("trame", "toile", "prose")

# A backticked name candidate. It ends at a backtick (`` `trame-logic` ``) or at a
# space, which is the form that carries the coordinate in the same span
# (`` `trame-logic section 9` ``). The name class excludes `_`, so an identifier
# (`name_enc`) is not a name.
#
# A directory prefix is consumed and dropped: the corpus writes a sibling document
# as `` `features/ambre-spec-toile-links.md` `` and a rules file as
# `` `ai/ambre-writing-style.md` ``, and a name pattern stopping at `/` reads
# neither as a citation at all -- 311 of them, outside the rule that a citation
# names a document that exists (finding 68). A path whose last component is not a
# document still resolves to nothing here, so `` `crates/ambre-prose/src/lib.rs` ``
# is no document citation; `CODE_PATH` is what resolves it, in its own namespace.
NAME = re.compile(r"`((?:[a-z0-9][a-z0-9-]*/)*)([a-z0-9][a-z0-9.-]*)(?=[`\s])")

# A prefix that names a crate marks a Cargo feature, never a document path. The
# corpus writes a feature as `` `ambre-logic/toile-pdf` ``, and the name pattern
# above drops the prefix and reads `toile-pdf` as a Toile document, which no
# directory holds. A crate name is the one prefix that cannot introduce a
# document, so the candidate behind it is not a citation at all.
CRATE_NAMES = {path.name for path in (ROOT / "crates").iterdir() if path.is_dir()}

# `this spec`, `this document`: the citing file is the target.
SELF = re.compile(r"\bthis (?:spec|document)\b", re.IGNORECASE)

# The Latin ordinal an unmigrated document appends to a section it inserted
# rather than renumbering: `5bis`, `5 ter`, `5quinquies`. Toile and Prose still
# carry them, so the resolver reads them; `spec_shape.py` is what refuses a new
# one in a migrated document.
ORDINAL = r"bis|ter|quater|quinquies|sexies"

# A section coordinate. `sec.5bis`, `section 11 bis` and `sections 1` are all in
# the corpus, and `§2.A` is the spelling `crates/` used. The sub-section that may
# follow (`5.B`, `11 bis.F`, `2.A bis`) is matched but left outside the two
# groups: it is consumed so that the next coordinate in a chain
# (`section 2.D / section 12`) still reads as adjacent, and it is not captured
# because only the number is checked and renumbered.
#
# `§` carries no word boundary of its own -- it sits against `(`, `/` or a space
# -- so the boundary belongs to the `sec` branch alone.
COORD = re.compile(
    rf"(?:\bsec(?:tions?|\.)?\s*|§\s*)"
    rf"(\d+)\s*({ORDINAL})?(?:\.[A-Za-z0-9]+(?:\s*(?:{ORDINAL}))?)?\b",
    re.IGNORECASE)

# What may sit between a name and the coordinate it owns. Short by design: a
# coordinate at the far end of a sentence belongs to the citing document, not to
# whatever document that sentence happened to mention first.
GLUE = re.compile(r"[`'\"(),;:\s]*(?:see|in|of|per|at|and|the)?[`'\"(),;:\s]*")
GLUE_MAX = 10

# `section 6/section 7`, `section 9-section 12`, `section 5 and section 6`: the
# second coordinate inherits the first one's document. The backticks are part of
# the glue, not of the chain: `` `doc section 1.C` / `section 1.D` `` is how a
# chain is written once the citation carries a name.
CHAIN = re.compile(r"[`'\"]*\s*(?:[/,;-]|and|to|or)\s*[`'\"]*")

# What opens the continuation line of a wrapped citation, and counts for nothing
# in the glue: a doc comment, a comment, a blockquote, a list bullet.
MARKER = re.compile(r"^\s*(?://[/!]?|[*>|-]+|#+)?\s*")

# A heading that opens a numbered section, `## 11 bis.` and `## 5quinquies.`
# included. It must read every ordinal the coordinate reads, or a citation of a
# section that exists is reported dead.
HEADING = re.compile(rf"^##\s+(\d+)\s*({ORDINAL})?\s*[.:]", re.IGNORECASE)

# Dated records. Their coordinates are quotations of a past state, so they are
# not resolved; their document names are, except for the names below. Everything
# under `specs/audits/` is one, by directory rather than by name, so the next
# audit is a record the day it lands.
RECORDS = ("ambre-roadmap-journal.md",)
RECORD_DIRS = ("audits/",)


def is_record(path: Path) -> bool:
    rel = str(path.relative_to(SPECS)) if SPECS in path.parents else path.name
    return path.name in RECORDS or any(d in rel for d in RECORD_DIRS)


# A code path, always backticked and always rooted at `crates/`: the corpus names
# a file of the workspace and never a path relative to the document. The span may
# carry more than the path -- a line range, then prose about it -- so it is taken
# whole here and cut below.
CODE_PATH = re.compile(r"`(crates/[^`]*)`")

# The line range a path may carry: `rate_limit.rs:131-197`, `sync.rs:210,237`.
# The colon before a digit is what marks it, and nothing else in a path carries
# one. This is the coordinate half of a code citation, and the half that rots.
LINE_RANGE = re.compile(r":\s*\d")


def code_citations(text: str):
    """Yield `(line, cited, path, ranged)` for each code path one file carries.

    `path` is the file alone, with the range and the prose after it cut off, so
    the name half resolves whatever the coordinate half says. `ranged` is whether
    the citation carried a range at all, which is the caller's to refuse.
    """
    for lineno, line in enumerate(text.splitlines(), 1):
        for cited in CODE_PATH.findall(line):
            head = cited.split(" ")[0]
            found = LINE_RANGE.search(head)
            path = (head[: found.start()] if found else head).rstrip("/")
            yield lineno, cited, path, found is not None


def code_resolves(path: str) -> bool:
    """Whether a code path names something the workspace holds.

    A path carrying `*` is a shape and not a file -- `crates/**/*.rs` is how the
    roadmap names every Rust file there is -- so it resolves when it matches at
    least one path, and a shape matching nothing is as dead as a dead name.
    """
    if "*" in path:
        return any(ROOT.glob(path))
    return (ROOT / path).exists()


# Documents the corpus deleted. A record naming one is quoting the corpus of the
# day it was written, exactly as it quotes a since-deleted heading: finding 65 is
# *about* what `trame-ui` said, so rewriting it to name `trame-engine` would make
# the record claim something it never said. Reporting it instead asks the author
# to falsify the archive, which is the one thing an archive may not do.
#
# Narrow on purpose. It buys the exemption for these names alone, inside a record
# alone: every other dead name in a record is still reported, which is the class
# `prose-engine` belonged to. **The set only grows by a deletion**, one line per
# document, so it doubles as the register of what the corpus removed and when.
RETIRED = {
    # Step 31: the four Trame specs, emptied by the feature migration and
    # replaced by `features/ambre-spec-trame-engine.md`.
    "ambre-spec-trame-logic.md",
    "ambre-spec-trame-ui.md",
    "ambre-spec-trame-features.md",
    "ambre-spec-trame-interop.md",
    # Step T9: the four Toile specs, emptied by the same migration and replaced
    # by `features/ambre-spec-toile-engine.md`.
    "ambre-spec-toile-logic.md",
    "ambre-spec-toile-ui.md",
    "ambre-spec-toile-features.md",
    "ambre-spec-toile-interop.md",
    # Step P9: the four Prose specs, emptied by the same migration and replaced
    # by `features/ambre-spec-prose-engine.md`.
    "ambre-spec-prose-logic.md",
    "ambre-spec-prose-ui.md",
    "ambre-spec-prose-features.md",
    "ambre-spec-prose-interop.md",
    # Step F2: the content rendering spec, cut into the five feature documents
    # of the rendering layer -- `ambre-spec-frame.md`,
    # `ambre-spec-render-assets.md`, `ambre-spec-drawing-surface.md`,
    # `ambre-spec-text-editor.md` and `ambre-spec-high-density.md`.
    "ambre-spec-canvas.md",
}

# The same set, as `resolve` claims a short name once the file is gone: the short
# form no longer sits in `shorts`, so the candidate falls through to `in_span` and
# claims `<short>.md`. A record cites either spelling -- finding 62 wrote `canvas
# section 5.B` -- so the exemption covers both or it covers the archive by half.
RETIRED_SHORTS = {name[len("ambre-spec-"):] for name in RETIRED}


def coordinate(number: str, ordinal: str | None) -> str:
    """One spelling for a coordinate, whatever the citation or heading wrote.

    `5bis`, `5 BIS` and `## 5 bis.` are one coordinate; so are `§5quinquies` and
    `section 5 quinquies`.
    """
    return f"{number} {ordinal.lower()}" if ordinal else number


def corpus_files() -> list[Path]:
    """Every file `documents` reads, before the file name becomes the key.

    `specs/` is the corpus. `ai/` holds the rules a spec legitimately cites --
    `ambre-spec-template.md` names the writing style -- so those files are
    resolved as targets. They are not `sources()`: nothing checks a citation
    they carry.
    """
    return sorted(SPECS.rglob("*.md")) + sorted(RULES.glob("*.md"))


def documents() -> dict[str, Path]:
    """Every citable document, by file name.

    The key is the file name, not the path, because that is what a citation
    carries: `ambre-spec-sync.md`, or `sync section 3`. A path that appears is
    dropped before resolution. So two documents of the same name are a defect
    of the corpus, reported by `duplicates`, and never a case to resolve.
    """
    return {path.name: path for path in corpus_files()}


def duplicates(paths: list[Path]) -> dict[str, list[Path]]:
    """File names that two documents share, with the documents that share them.

    `documents` keys by file name, so the second file of a name silently
    replaces the first, and the survivor depends on the walk order. Citations
    then reach the wrong document, and their coordinates are verified against
    the wrong table of contents.

    A writer cannot lift such an ambiguity anyway: `sync section 3` has no room
    to say which `sync`. The namespace citations use is flat, so the corpus is
    held flat rather than the resolver taught to carry two.
    """
    seen: dict[str, list[Path]] = {}
    for path in paths:
        seen.setdefault(path.name, []).append(path)
    return {name: found for name, found in seen.items() if len(found) > 1}


def short_names(known: dict[str, Path]) -> dict[str, str]:
    """Short name -> file name, for every document carrying the fixed prefix.

    `ambre-spec-trame-sort.md` is cited `trame-sort`, `ambre-spec-ui.md` is
    cited `ui`. A short name is unique because the file name it is cut from is:
    `duplicates` is what holds that, and it holds it over the whole corpus
    rather than over the documents that happen to carry the prefix.
    """
    shorts = {}
    for name in known:
        if name.startswith("ambre-spec-") and name.endswith(".md"):
            shorts[name[len("ambre-spec-"):-len(".md")]] = name
    return shorts


def sections(path: Path) -> set[str]:
    """The coordinates one document actually has, from its `## N.` headings."""
    found = set()
    for line in path.read_text().splitlines():
        match = HEADING.match(line)
        if match:
            found.add(coordinate(match.group(1), match.group(2)))
    return found


def sources():
    """Yield every file that may carry a citation: the code and the corpus."""
    yield from sorted(CRATES.rglob("*.rs"))
    for path in sorted(SPECS.rglob("*.md")):
        # A past review is an archive, not corpus content. Same exclusion as
        # `spec_code.py`: it cites what the corpus held on the day it was written.
        if "reviews/" not in str(path.relative_to(SPECS)):
            yield path


def resolve(candidate: str, shorts: dict[str, str], coordinated: bool,
            in_span: bool) -> tuple[str, str] | None:
    """Map one name candidate to (cited name, the file name it claims).

    Returns `None` when the candidate is not a document citation at all. Four
    shapes are one: the `ambre-spec-` prefix, a hyphenated bare `.md` name -- the
    roadmap, the audits, the generated ledger and each inventory carry no prefix --
    an application prefix, and a short name.

    The `ambre-spec-` prefix is required, not `ambre-`: every crate is named
    `ambre-<something>` and backticked in half the corpus, and `ambre-bus` is not a
    document. Every corpus document is hyphenated, while a fixture cited by file
    name is not (`reference.md`).

    A short name counts only when a coordinate follows it (`coordinated`).
    `` `sync section 3` `` is a citation; a bare `` `sync` `` is the word.

    `in_span` says the coordinate sits inside this candidate's own backtick span,
    which makes the candidate a document name **by position**: whatever
    `` `logic section 6` `` is, it is being read as a document, so it must resolve
    or be reported. `` `ambre-chrome` (section 1bis) `` is the other case -- a crate
    name, and a coordinate that belongs to the citing document.
    """
    stem = candidate[:-len(".md")] if candidate.endswith(".md") else candidate
    if stem.startswith("ambre-spec-"):
        return candidate, f"{stem}.md"
    if candidate.endswith(".md") and "-" in stem:
        return candidate, candidate
    if stem.startswith(APPS) and "-" in stem:
        return candidate, f"ambre-spec-{stem}.md"
    if coordinated and stem in shorts:
        return candidate, shorts[stem]
    # A transposition reads as a name and resolves to nothing: `spec-ambre-sync`
    # and `spec-ambre-architecture.md` both sat in the corpus unseen. Any candidate
    # carrying both halves of the prefix must resolve.
    if "ambre" in stem and "spec" in stem:
        return candidate, f"{stem}.md"
    if in_span:
        return candidate, f"{stem}.md"
    return None


def citations(text: str, own: str | None, shorts: dict[str, str]):
    """Yield every citation in one file, name and coordinate alike.

    Two kinds of tuple, discriminated by the first field:
      ("name", line, cited name, claimed file, span)
      ("coord", line, claimed file or None, coordinate, span)

    The span of a coordinate covers the number and its ordinal only, never the
    word `section` and never the letter of a sub-section, so a renumbering pass
    can rewrite it in place.

    Adjacency survives one line break, and one only: a citation the writer wrapped
    is still one citation, but a document named a paragraph earlier owns nothing.
    A line carrying no token at all ends the chain.
    """
    # Carried across lines so a wrapped citation stays one citation.
    previous = None   # (kind, target)
    trailing = None   # text between the last token and its end of line

    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        tokens = []
        starts = [match.start() for match in COORD.finditer(line)]
        # A coordinate opening the next line is adjacent to a name closing this
        # one, and a short name needs that to be a citation at all: `` `sync` ``
        # resolves only when a coordinate follows it. Without this, a wrapped
        # `` `architecture` `` / `section 5.F` produced no name token, so the
        # coordinate fell back on the citing document and was verified against a
        # section 5 that existed there and said something else -- the exact
        # fall-back the wrap rule was written against, surviving inside it for
        # every short name.
        following = MARKER.sub("", lines[number], count=1) if number < len(lines) else ""
        wrapped = COORD.match(following) is not None
        for match in NAME.finditer(line):
            near = any(match.end() <= at <= match.end() + GLUE_MAX
                       for at in starts) or (
                wrapped and len(line) - match.end() <= GLUE_MAX
            )
            # The coordinate is inside this candidate's own span: the span opened
            # at the backtick and no backtick closed it before the coordinate.
            span = any(match.end() <= at <= match.end() + 1 for at in starts)
            if match.group(1).rstrip("/").rsplit("/", 1)[-1] in CRATE_NAMES:
                continue
            hit = resolve(match.group(2), shorts, near, span)
            # A backticked word that is not a document citation is not an anchor
            # either: `` `ambre-chrome` (section 1bis) `` names a crate, and the
            # coordinate belongs to the citing document.
            if hit:
                tokens.append((match.start(), match.end(), "name", hit[1], hit))
        for match in SELF.finditer(line):
            tokens.append((match.start(), match.end(), "name", own, None))
        for match in COORD.finditer(line):
            inner = (match.start(1), match.end(2) if match.group(2) else match.end(1))
            tokens.append((match.start(), match.end(), "coord",
                           coordinate(match.group(1), match.group(2)), inner))
        tokens.sort()
        if not tokens:
            previous, trailing = None, None
            continue

        cursor = None  # end of the previous token, on this line
        for start, end, kind, value, extra in tokens:
            if cursor is not None:
                glue = line[cursor:start]
            elif trailing is not None:
                glue = f"{trailing} {MARKER.sub('', line[:start], count=1)}"
            else:
                glue = None
            cursor = end

            if kind == "name":
                if extra:
                    yield "name", number, extra[0], extra[1], (start, end)
                previous = ("name", value)
                continue

            target = own
            if previous and glue is not None and len(glue) <= GLUE_MAX:
                if previous[0] == "name" and GLUE.fullmatch(glue):
                    target = previous[1]
                elif previous[0] == "coord" and CHAIN.fullmatch(glue):
                    target = previous[1]
            yield "coord", number, target, value, extra
            previous = ("coord", target)

        trailing = line[cursor:]


def main() -> int:
    check = "--check" in sys.argv
    files = corpus_files()
    shared = duplicates(files)
    known = documents()
    shorts = short_names(known)
    known_sections = {name: sections(path) for name, path in known.items()}

    dead_names: dict[str, list[str]] = {}
    dead_coords: dict[str, list[str]] = {}
    dead_paths: dict[str, list[str]] = {}
    ranged_paths: dict[str, list[str]] = {}
    names = coords = paths = 0
    for path in sources():
        rel = path.relative_to(ROOT)
        text = path.read_text()
        record = is_record(path)
        for lineno, cited, target, ranged in code_citations(text):
            paths += 1
            if not code_resolves(target):
                dead_paths.setdefault(target, []).append(f"{rel}:{lineno}")
            # The range is the coordinate of a code file. A record quotes the
            # workspace of the day it was written, ranges included.
            elif ranged and not record:
                ranged_paths.setdefault(cited, []).append(f"{rel}:{lineno}")
        own = path.name if path.suffix == ".md" else None
        for kind, lineno, target, value, _ in citations(text, own,
                                                        shorts):
            if kind == "name":
                names += 1
                if value not in known:
                    # A record may name a document the corpus deleted, and only
                    # such a document: see `RETIRED`.
                    if record and (
                        value in RETIRED or value in RETIRED_SHORTS
                    ):
                        continue
                    dead_names.setdefault(target, []).append(f"{rel}:{lineno}")
                continue
            # The coordinate namespace is the corpus. A `.rs` file cites a name and
            # a contract id, both minted once; a section number is the one part
            # that a renumbering breaks, so the code does not carry it.
            if path.suffix != ".md" or record:
                continue
            coords += 1
            if target is None:
                dead_coords.setdefault(f"? section {value}", []).append(
                    f"{rel}:{lineno}")
            elif target in known_sections and known_sections[target]:
                if value not in known_sections[target]:
                    dead_coords.setdefault(f"{target} section {value}", []).append(
                        f"{rel}:{lineno}")

    def report(dead: dict[str, list[str]], what: str) -> None:
        for name in sorted(dead):
            sites = dead[name]
            print(f"\n`{name}` -- {what}, {len(sites)} citations")
            for site in sites[:8]:
                print(f"  {site}")
            if len(sites) > 8:
                print(f"  ... and {len(sites) - 8} more")

    report(dead_names, "no document")
    report(dead_coords, "no such section")
    report(dead_paths, "no such file")
    report(ranged_paths, "a line range outside a record")
    for name in sorted(shared):
        print(f"\n`{name}` -- one file name, {len(shared[name])} documents")
        for path in shared[name]:
            print(f"  {path.relative_to(ROOT)}")
    print(f"\n{names} document citations, {coords} section coordinates, "
          f"{paths} code paths, {len(known)} documents, "
          f"{len(dead_names)} dead names, {len(dead_coords)} dead coordinates, "
          f"{len(dead_paths)} dead paths, {len(ranged_paths)} line ranges, "
          f"{len(shared)} shared file names")
    if check and shared:
        print(
            "\nERROR: two documents carry one file name, and a citation carries "
            "the file name alone (ambre-spec-template.md section 1.D). One of "
            "them resolves and the other is unreachable, so rename one.",
            file=sys.stderr,
        )
        return 1
    if check and (dead_names or dead_coords):
        print(
            "\nERROR: a citation names a document, or a section of one, that does "
            "not exist (ambre-spec-template.md section 1.D). Point the citation at "
            "the document and the section that hold the rule today.",
            file=sys.stderr,
        )
        return 1
    if check and dead_paths:
        print(
            "\nERROR: a citation names a file of `crates/` that does not exist "
            "(ambre-spec-template.md section 1.D). Point it at the file that "
            "holds the code today, or cut it.",
            file=sys.stderr,
        )
        return 1
    if check and ranged_paths:
        print(
            "\nERROR: a code path carries a line range, and `crates/` carries no "
            "coordinate (ambre-spec-template.md section 1.D). A range is exact "
            "the day it is written and wrong at the next edit: name the file, and "
            "the symbol where the citation needs one.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
