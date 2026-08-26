#!/usr/bin/env python3
"""Fail when a citation names a document, or a section of one, that does not exist.

The corpus cites a document by its name, never by a path or a number: a comment
writes `` `trame-clipboard` `` and a spec writes `` `ambre-spec-sync.md` ``. A
name is stable, greppable and readable, which is why `ambre-spec-template.md`
section 1.D makes it a pillar. Nothing resolved it. Splitting one spec into
fifteen feature documents left every stale citation silent, and a reader follows
a name to a document that no longer holds the rule, or to no document at all.
`prose-engine` named no document for as long as three Prose specs have existed.

Two namespaces, one resolver.

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
has one, its **invariant id** -- the two namespaces an ID is minted once and never
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

This resolves two namespaces and checks nothing else. It does not grade a
citation, rewrite it into a rule id or a code path, or read what the target says.

`specs/reviews/` is out of the corpus, exactly as in `spec_code.py`. A dated
record is out of the *coordinate* namespace: `security-audit.md` and its kin state
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
# document still resolves to nothing, so `` `crates/ambre-prose/src/lib.rs` ``
# stays what it was: not a citation.
NAME = re.compile(r"`(?:[a-z0-9][a-z0-9-]*/)*([a-z0-9][a-z0-9.-]*)(?=[`\s])")

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
# not resolved; their document names are, except for the names below.
RECORDS = ("security-audit.md", "ambre-roadmap-journal.md")

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


def documents() -> dict[str, Path]:
    """Every citable document, by file name: the corpus, plus the rules.

    `specs/` is the corpus. `ai/` holds the rules a spec legitimately cites --
    `ambre-spec-template.md` names the writing style -- so those files are
    resolved as targets. They are not `sources()`: nothing checks a citation
    they carry.
    """
    known = {path.name: path for path in SPECS.rglob("*.md")}
    known.update({path.name: path for path in RULES.glob("*.md")})
    return known


def short_names(known: dict[str, Path]) -> dict[str, str]:
    """Short name -> file name, kept only where the short name is unambiguous.

    `ambre-spec-trame-sort.md` is cited `trame-sort`, `ambre-spec-ui.md` is cited
    `ui`. A short name claimed by two documents resolves to neither: it is dropped
    here, so a citation using it is reported as a dead name rather than silently
    aimed at one of the two.
    """
    claims: dict[str, set[str]] = {}
    for name in known:
        if name.startswith("ambre-spec-") and name.endswith(".md"):
            claims.setdefault(name[len("ambre-spec-"):-len(".md")], set()).add(name)
    return {short: next(iter(files)) for short, files in claims.items()
            if len(files) == 1}


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

    for number, line in enumerate(text.splitlines(), 1):
        tokens = []
        starts = [match.start() for match in COORD.finditer(line)]
        for match in NAME.finditer(line):
            near = any(match.end() <= at <= match.end() + GLUE_MAX
                       for at in starts)
            # The coordinate is inside this candidate's own span: the span opened
            # at the backtick and no backtick closed it before the coordinate.
            span = any(match.end() <= at <= match.end() + 1 for at in starts)
            hit = resolve(match.group(1), shorts, near, span)
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
    known = documents()
    shorts = short_names(known)
    known_sections = {name: sections(path) for name, path in known.items()}

    dead_names: dict[str, list[str]] = {}
    dead_coords: dict[str, list[str]] = {}
    names = coords = 0
    for path in sources():
        rel = path.relative_to(ROOT)
        own = path.name if path.suffix == ".md" else None
        for kind, lineno, target, value, _ in citations(path.read_text(), own,
                                                        shorts):
            if kind == "name":
                names += 1
                if value not in known:
                    # A record may name a document the corpus deleted, and only
                    # such a document: see `RETIRED`.
                    if path.name in RECORDS and (
                        value in RETIRED or value in RETIRED_SHORTS
                    ):
                        continue
                    dead_names.setdefault(target, []).append(f"{rel}:{lineno}")
                continue
            # The coordinate namespace is the corpus. A `.rs` file cites a name and
            # an invariant id, both minted once; a section number is the one part
            # that a renumbering breaks, so the code does not carry it.
            if path.suffix != ".md" or path.name in RECORDS:
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
    print(f"\n{names} document citations, {coords} section coordinates, "
          f"{len(known)} documents, "
          f"{len(dead_names)} dead names, {len(dead_coords)} dead coordinates")
    if check and (dead_names or dead_coords):
        print(
            "\nERROR: a citation names a document, or a section of one, that does "
            "not exist (ambre-spec-template.md section 1.D). Point the citation at "
            "the document and the section that hold the rule today.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
