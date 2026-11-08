#!/usr/bin/env python3
"""Fail when English drifts from `ai/ambre-writing-style.md`.

That file states the rules and names the half a script can hold. This is that
half, and every metric below is a hard error and an exact count.

  * long      -- a sentence over 25 words.
  * latin     -- a Latin abbreviation, where the rule asks for words.
  * para      -- a paragraph over 6 sentences.

A rule enters this script when the measure is exact, never because a regular
expression reaches it: a detector that guesses produces a number, not a rule.
"Active voice, named actor" fails that test. `be` plus a participle names a
state as often as a passive, and a closed list of participles reads about thirty
of the two hundred English carries.

Five more rules are a hard error: the four empty openers the style file names,
the banned terms of its own table, and a code point outside 32 to 126, letters
of another script apart.

**What no script holds.** A metaphor, an ambiguous pronoun, one word used for
two meanings, the passive voice, one negative per sentence, and a comment that
states what the code does instead of why. Those stay a human read of the diff.
The script narrows the read; it does not replace it.

**Why a breach fails, and never warns.** The limits frame a writer that runs
long, an agent first. A failure costs a rerun; a warning is read once and then
ignored, and the corpus grows back. A story to demote them to warnings was
withdrawn for that reason.

**Scope.** The comments of `crates/**/*.rs`, and the prose of the corpus.

Usage:
    python3 scripts/prose.py    # fails on a breach, with no flag
"""

from __future__ import annotations

import re
import sys
import unicodedata
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import glossary  # noqa: E402  -- the vocabulary, read from the corpus

ROOT = Path(__file__).resolve().parent.parent

# The Rust the workspace owns. `target/` holds generated crates.
RUST_ROOTS = (ROOT / "crates",)
# The English the corpus owns, beyond the specs' own shape gate.
MARKDOWN_ROOTS = (ROOT / "specs", ROOT / "ai")

# One sentence, one idea: 25 words for descriptive text (20 for a rule, which no
# script can tell apart from the rest).
SENTENCE_WORDS_MAX = 25
# Six sentences maximum per paragraph.
PARAGRAPH_SENTENCES_MAX = 6

# "Delete empty openers", named one by one in the style file. No addition: a
# banned-word list nobody agreed to is how a style gate loses its authority.
OPENERS = re.compile(
    r"\b(it should be noted that|in order to|basically|obviously)\b", re.I
)

# "One word, one meaning", read from `specs/ambre-glossary.md` rather than typed
# again here. The glossary is where a term, its gloss and the synonyms it
# forbids are written, and a second copy in this file would drift from it in
# silence (`scripts/template_vocabulary.py` states the rule and the incident
# behind it).
#
# A hyphen binds as tightly as a letter, so `\b` is too loose: it read the
# tool `cargo-deny` as the verb "deny" fourteen times. The list forbids a verb,
# never a syllable of a name.
TERMS = tuple(
    (re.compile(rf"(?<![\w-]){re.escape(word)}(s|ed|ing)?(?![\w-])", re.I), entry.term, exception)
    for entry in glossary.entries()
    for word, exception in entry.forbidden
)

# A gloss names an idea; it does not specify one. These are what turns a gloss
# into a second specification, free to drift from the document that defines the
# term: a figure, a normative keyword, and length.
GLOSS_WORDS_MAX = 12
GLOSS_FIGURE = re.compile(r"\d")
GLOSS_NORMATIVE = re.compile(r"\b(MUST|SHALL|SHOULD|MAY|REQUIRED|OPTIONAL)\b")

# "A figure and its unit are separated by a space": 250 ms, not 250ms. A hard
# error at zero, over a closed list of the units the workspace writes. A digit
# welded to a unit reads as one token, which is what a literal is, and `clean`
# has already dropped the code spans where `250ms` is exactly that.
SI_UNIT = re.compile(r"(?<![\w.])\d+(ms|us|ns|s|Hz|KiB|MiB|GiB|B)\b")

# "No Latin abbreviation: write 'for example', 'that is'."
LATIN = re.compile(r"(?<![\w.])(e\.g\.|i\.e\.|etc\.|cf\.|viz\.)", re.I)

# A code span, a Markdown link target and a URL are not English. Removing them
# before the measure is what keeps `self.doc.commit()` from ending a sentence and
# a path from counting as five words.
CODE_SPAN = re.compile(r"`[^`]*`")
LINK = re.compile(r"\[([^\]]*)\]\([^)]*\)")
URL = re.compile(r"https?://\S+")
# An ASCII diagram or a table is structure, not prose.
DIAGRAM = re.compile(r"^\s*[|+`\\/*-]{2,}")

SENTENCE_SPLIT = re.compile(r"(?<=[.!?])[\s]+")
# A Markdown bullet or numbered item, with its indent.
BULLET = re.compile(r"^\s*(?:[*\-+]|\d+\.)\s+")
# A block quote marker. The line under it is prose; the marker is not, and left
# in place it lands mid-sentence once the lines are joined.
QUOTE = re.compile(r"^\s*>\s?")
# A line that is one bold run and nothing else. The spec template writes a
# section heading that way, and a heading is not a sentence of the paragraph
# below it.
BOLD_HEADING = re.compile(r"^\*\*[^*]+\*\*$")
# A horizontal rule: a block separator, never a sentence.
RULE = re.compile(r"^\s*(?:-{3,}|\*{3,}|_{3,})\s*$")


def printable_breaches(text: str) -> list[tuple[int, str]]:
    """Every line carrying a code point outside 32..126, letters apart.

    The whole file, not its comments alone: the rule covers prose, code and file
    names, and a `-` in a string is what reaches the user's screen.

    A letter of another script is the one exception, and it is a class, not a
    list of files. An em dash, a non-breaking hyphen, an arrow, an ellipsis and
    a `<=` all have an ASCII spelling, so writing them outside ASCII is a
    choice. A han ideograph has none: where one appears it is the text Ambre
    renders, and `ambre-canvas` selects a font by script, so the character is
    the subject of the test and not a way of writing English. Unicode says
    which is which, which is what makes the exception exact.
    """
    out = []
    for lineno, line in enumerate(text.splitlines(), 1):
        bad = {
            ch
            for ch in line
            if not (32 <= ord(ch) <= 126)
            and ch != "\t"
            and not unicodedata.category(ch).startswith("L")
        }
        if bad:
            shown = ", ".join(f"U+{ord(ch):04X}" for ch in sorted(bad))
            out.append((lineno, f"non-printable character: {shown}"))
    return out


def rust_comments(text: str) -> list[tuple[int, str, bool, bool]]:
    """The comment text of a Rust file, one entry per line, fences dropped.

    A fenced block inside a doc comment is code or a diagram, so it carries no
    English to measure. The third field marks a line that opens a paragraph; a
    Rust comment opens one by following a line of code, which the line numbers
    already say. The fourth says the line is prose at all.

    A table row is structure, not a sentence, and it is the one shape that reads
    as both. A doc comment holding a five-row table joins its cells into one
    long "sentence", which no rewrite can shorten because nobody wrote it. It
    stays in the result for the hard rules and leaves the paragraph.

    An empty comment line and a Markdown heading both drop out, the way
    `markdown_prose` drops a blank line and a `#` heading. A doc comment is
    Markdown, so the two readers must agree. Keeping an empty `///` would make a
    whole rustdoc block one paragraph, so a section added to a six-sentence
    block would breach the budget whatever it said. Dropping it also ends a
    paragraph at a `// ---` rule.
    """
    out: list[tuple[int, str, bool, bool]] = []
    fenced = False
    for lineno, line in enumerate(text.splitlines(), 1):
        body = line_comment(line)
        if body is None:
            continue
        if body.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or not body.strip():
            continue
        if body.lstrip().startswith("#"):
            continue
        if body.lstrip().startswith("|"):
            out.append((lineno, body, True, False))
            continue
        opens = bool(BULLET.match(body))
        out.append((lineno, BULLET.sub("", body), opens, True))
    return out


def line_comment(line: str) -> str | None:
    """The comment on one Rust line, or `None`.

    Walks the line so a `//` inside a string literal (`"https://..."`) stays
    code. A block comment is out of scope: the workspace writes none.
    """
    in_string = False
    escaped = False
    prev = ""
    for i, ch in enumerate(line):
        if escaped:
            escaped = False
        elif ch == "\\" and in_string:
            escaped = True
        elif ch == '"':
            in_string = not in_string
        elif ch == "/" and prev == "/" and not in_string:
            return line[i + 1 :].lstrip("/!").strip()
        prev = ch
    return None


def markdown_prose(text: str) -> list[tuple[int, str, bool, bool]]:
    """The prose lines of a Markdown file: no fence, no table, no heading.

    The third field says the line opens a paragraph. A bullet opens one, and so
    does every other block boundary: a bold heading, a horizontal rule, and each
    edge of a block quote. Merging a list into one paragraph counted its items
    as sentences, which is the metric measuring the layout instead of the
    writing, and a heading glued to the text below it costs that text a sentence
    of its budget.

    A boundary line stays in the result rather than leaving it, because the hard
    rules read every line. A heading that carries a forbidden synonym is still a
    breach; it is simply not a sentence of the paragraph under it. A table row
    is the same, and a corpus written in tables would otherwise go unread.
    """
    out: list[tuple[int, str, bool, bool]] = []
    fenced = False
    quoted = False
    for lineno, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or not line.strip():
            continue
        if line.lstrip().startswith("#"):
            continue
        if line.lstrip().startswith("|"):
            out.append((lineno, line.strip(), True, False))
            continue
        was_quoted, quoted = quoted, bool(QUOTE.match(line))
        body = BULLET.sub("", QUOTE.sub("", line)).strip()
        if not body:
            continue
        opens = bool(BULLET.match(line)) or quoted != was_quoted
        opens = opens or bool(BOLD_HEADING.match(body)) or bool(RULE.match(body))
        out.append((lineno, body, opens, True))
    return out


def clean(text: str) -> str:
    """Drop what is not English: code spans, link targets, URLs, emphasis."""
    text = CODE_SPAN.sub("CODE", text)
    text = LINK.sub(r"\1", text)
    text = URL.sub("URL", text)
    return text.replace("**", "").replace("__", "")


def opener_breaches(lines: list[tuple[int, str, bool, bool]]) -> list[tuple[int, str]]:
    """The one hard check: an empty opener the style file names."""
    out = []
    for lineno, raw, _, _ in lines:
        for match in OPENERS.finditer(clean(raw)):
            out.append((lineno, f'empty opener: "{match.group(0)}"'))
    return out


def unit_breaches(lines: list[tuple[int, str, bool, bool]]) -> list[tuple[int, str]]:
    """A figure welded to its unit, outside a code span.

    A hard error and not a ceiling: the count is zero and stays there.
    """
    out = []
    for lineno, raw, _, _ in lines:
        for match in SI_UNIT.finditer(clean(raw)):
            spaced = f"{match.group(0)[: -len(match.group(1))]} {match.group(1)}"
            out.append((lineno, f'"{match.group(0)}": a unit takes a space -- "{spaced}"'))
    return out


def term_breaches(lines: list[tuple[int, str, bool, bool]]) -> list[tuple[int, str]]:
    """A forbidden synonym of the glossary, with the term that replaces it.

    A hard error and not a ceiling: a synonym enters the glossary once its uses
    are rewritten, so the count starts at zero and stays there.

    A frozen compound is removed before the word is matched, so
    `deny (not default-deny)` refuses the verb and leaves the policy alone.
    """
    out = []
    for lineno, raw, _, _ in lines:
        text = clean(raw)
        for pattern, instead, exception in TERMS:
            body = re.sub(re.escape(exception), "", text, flags=re.I) if exception else text
            for match in pattern.finditer(body):
                out.append((lineno, f'"{match.group(0)}": the term is "{instead}"'))
    return out


def glossary_breaches() -> list[str]:
    """What the glossary owes as a register, checked once per run.

    A duplicate term is two definitions of one idea, which is the drift the
    register exists to end. A reference that does not hold its own term sends
    the reader to the wrong document, and reads as a reference like any other.
    """
    out = []
    seen: dict[str, str] = {}
    for entry in glossary.entries():
        if entry.term in seen:
            out.append(f"glossary: {entry.term!r} has two rows")
        seen[entry.term] = entry.ref
        words = len(entry.gloss.split())
        if not entry.gloss:
            out.append(f"glossary: {entry.term!r} has no definition")
        elif words > GLOSS_WORDS_MAX:
            out.append(
                f"glossary: {entry.term!r}: {words} words, budget {GLOSS_WORDS_MAX}"
                " -- a gloss names an idea, the reference defines it"
            )
        if GLOSS_FIGURE.search(entry.gloss):
            out.append(f"glossary: {entry.term!r}: a figure belongs to the reference")
        if GLOSS_NORMATIVE.search(entry.gloss):
            out.append(f"glossary: {entry.term!r}: a normative keyword belongs to the reference")
        path = ROOT / "specs" / entry.ref if not entry.ref.startswith("ai/") else ROOT / entry.ref
        if not path.exists():
            out.append(f"glossary: {entry.term!r}: reference {entry.ref} does not exist")
        elif not re.search(re.escape(entry.term), path.read_text(), re.I):
            out.append(f"glossary: {entry.term!r} is absent from {entry.ref}")
    return out


def paragraphs(lines: list[tuple[int, str, bool, bool]]) -> list[tuple[int, str]]:
    """Group consecutive prose lines into paragraphs, keyed by first line.

    A blank line, a fence and a bullet each open a new one. A line the reader
    marked as structure closes the paragraph above it and joins none: a table
    row is not a sentence, and joining a column of cells into one made a
    31-word "sentence" nobody wrote.
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


def sentences(text: str) -> list[str]:
    """Split on a sentence end, once the code and the links are gone."""
    text = clean(text).strip()
    if not text or DIAGRAM.match(text):
        return []
    return [s for s in SENTENCE_SPLIT.split(text) if s.strip()]


def measure(
    text: str, lines: list[tuple[int, str]]
) -> tuple[dict[str, int], list[tuple[int, str, str]]]:
    """The three counts of one file, and one `(line, metric, message)` per breach."""
    counts = {"latin": 0, "long": 0, "para": 0}
    detail: list[tuple[int, str, str]] = []
    for lineno, raw, _, _ in lines:
        for match in LATIN.finditer(clean(raw)):
            counts["latin"] += 1
            detail.append(
                (
                    lineno,
                    "latin",
                    f'the Latin abbreviation "{match.group(0)}" reads as jargon: '
                    "spell it in English",
                )
            )
    for lineno, text in paragraphs(lines):
        parts = sentences(text)
        if len(parts) > PARAGRAPH_SENTENCES_MAX:
            counts["para"] += 1
            detail.append(
                (
                    lineno,
                    "para",
                    f"{len(parts)} sentences in one paragraph, budget "
                    f"{PARAGRAPH_SENTENCES_MAX}: a paragraph holds one idea, so "
                    "break it where the idea changes",
                )
            )
        for part in parts:
            words = len(part.split())
            if words > SENTENCE_WORDS_MAX:
                counts["long"] += 1
                detail.append(
                    (lineno, "long", f"{words} words in one sentence: {part[:60]}...")
                )
    return counts, detail


# The three counted metrics, each a hard error: a breach is red, never a budget.
HARD = ("long", "latin", "para")

# A document that states a rule has to spell what the rule forbids. The style
# file quotes the four openers it bans, and it and the glossary both carry the
# banned terms of the table. The exemptions, named here rather than left as a
# mystery.
OPENER_EXEMPT = {"ai/ambre-writing-style.md"}
TERM_EXEMPT = {
    "ai/ambre-writing-style.md",
    "specs/ambre-glossary.md",
}


# `specs/reviews/` and `specs/Todo/` are local working notes, excluded from the
# repository. A gate cannot hold a file no commit carries, so they are out,
# for the same reason `spec_shape.py` skips them. `specs/audits/` is tracked and
# still out: an audit quotes what it found, in the words of whoever wrote the
# code, and a style gate rewriting the evidence is a gate rewriting the finding.
SKIPPED_DIRS = ("/reviews/", "/Todo/", "/audits/")


def tracked(path: Path) -> bool:
    return not any(part in f"/{path.relative_to(ROOT)}" for part in SKIPPED_DIRS)


def rust_files() -> list[Path]:
    return sorted(p for root in RUST_ROOTS for p in root.rglob("*.rs") if tracked(p))


def markdown_files() -> list[Path]:
    found = [p for root in MARKDOWN_ROOTS for p in root.rglob("*.md") if tracked(p)]
    # A glob, never a list of names: a named list measures the documents someone
    # remembered, and a document added later escapes the gate in silence.
    found += sorted(ROOT.glob("*.md"))
    return sorted(found)


def main() -> int:
    errors: list[str] = list(glossary_breaches())
    rust = rust_files()
    documents = markdown_files()

    for path in rust + documents:
        rel = str(path.relative_to(ROOT))
        text = path.read_text()
        lines = rust_comments(text) if path.suffix == ".rs" else markdown_prose(text)
        if rel not in OPENER_EXEMPT:
            for lineno, reason in opener_breaches(lines):
                errors.append(f"{rel}:{lineno}: {reason}")
        for lineno, reason in printable_breaches(text):
            errors.append(f"{rel}:{lineno}: {reason}")
        for lineno, reason in unit_breaches(lines):
            errors.append(f"{rel}:{lineno}: {reason}")
        if rel not in TERM_EXEMPT:
            for lineno, reason in term_breaches(lines):
                errors.append(f"{rel}:{lineno}: {reason}")
        _, detail = measure(text, lines)
        for lineno, key, reason in detail:
            if key in HARD:
                errors.append(f"{rel}:{lineno}: {reason}")

    print(f"{len(rust)} Rust files, {len(documents)} documents")
    print(f"{len(errors)} defects")
    for error in errors:
        print(f"ERROR {error}")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
