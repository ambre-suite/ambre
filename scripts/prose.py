#!/usr/bin/env python3
"""Fail when English drifts from `ai/ambre-writing-style.md`.

The style file closed on its own admission: "No script checks this file, unlike
`just contracts` and `just spec-code`. It is a review rule, and review rules
drift." This is the half of it a script can hold.

**A ceiling, not a hard error.** The corpus predates the check by a thousand
breaches, so a hard error would be red on the first run and turn into a flag
everyone passes. Each file carries a per-metric ceiling that may only fall, the
mechanism `contracts.py` and `spec_shape.py` already use. A file the table does
not name has a ceiling of zero, which is the whole point: a new file, and a new
comment in an old one, starts clean.

  * ascii     -- a code point outside 32 to 126.
  * latin     -- a Latin abbreviation, where the rule asks for words.
  * long      -- a sentence over 25 words.
  * para      -- a paragraph over 6 sentences.
  * passive   -- `is` / `are` / `was` / `were` plus a participle.

One rule stays a hard error, because the corpus breaches it nowhere: the four
empty openers the style file names.

**What no script holds.** A metaphor, an ambiguous pronoun, one word used for
two meanings, and a comment that states what the code does instead of why. Those
stay a human read of the diff. The script narrows the read; it does not replace
it.

**Scope.** The comments of `crates/**/*.rs`, and the prose of the corpus. A
comment is where the workspace had no check at all; a spec had `spec_shape.py`
for its shape and nothing for its sentences.

Usage:
    python3 scripts/prose.py --check              # CI: fail on a breach
    python3 scripts/prose.py --ceilings           # record the counts, lower only
    python3 scripts/prose.py --ceilings --raise   # a deliberate growth, reviewed
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CEILINGS = Path(__file__).resolve().parent / "prose_ceilings.json"

# The Rust the workspace owns. `target/` holds generated crates.
RUST_ROOTS = (ROOT / "crates",)
# The English the corpus owns, beyond the specs' own shape gate.
MARKDOWN_ROOTS = (ROOT / "specs", ROOT / "ai")
MARKDOWN_FILES = (ROOT / "README.md", ROOT / "AGENTS.md", ROOT / "CONTRIBUTING.md")

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

# "No Latin abbreviation: write 'for example', 'that is'."
LATIN = re.compile(r"(?<![\w.])(e\.g\.|i\.e\.|etc\.|cf\.|viz\.)", re.I)

# The passive the style file bans: "the frame is sealed" for "the Sync worker
# seals the frame". A participle after `be` catches the regular form and the
# irregulars the corpus actually uses. An adjective ("is tired") reads as a false
# positive, which is why this one is a ceiling and not an error.
PARTICIPLES = (
    r"\w+ed|born|built|drawn|given|held|kept|known|lost|made|meant|put|read|"
    r"seen|sent|set|shown|taken|told|written|thrown|drawn|torn|worn"
)
PASSIVE = re.compile(rf"\b(is|are|was|were|be|been|being)\s+(?:\w+ly\s+)?({PARTICIPLES})\b")

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


def printable_breaches(text: str) -> list[tuple[int, str]]:
    """Every line carrying a code point outside 32..126.

    The whole file, not its comments alone: the rule covers prose, code and file
    names, and a `-` in a string is what reaches the user's screen.
    """
    out = []
    for lineno, line in enumerate(text.splitlines(), 1):
        bad = {ch for ch in line if not (32 <= ord(ch) <= 126) and ch != "\t"}
        if bad:
            shown = ", ".join(f"U+{ord(ch):04X}" for ch in sorted(bad))
            out.append((lineno, f"non-printable character: {shown}"))
    return out


def rust_comments(text: str) -> list[tuple[int, str, bool]]:
    """The comment text of a Rust file, one entry per line, fences dropped.

    A fenced block inside a doc comment is code or a diagram, so it carries no
    English to measure. The third field marks a line that opens a paragraph; a
    Rust comment opens one by following a line of code, which the line numbers
    already say.
    """
    out: list[tuple[int, str, bool]] = []
    fenced = False
    for lineno, line in enumerate(text.splitlines(), 1):
        body = line_comment(line)
        if body is None:
            continue
        if body.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced:
            continue
        out.append((lineno, body, body.lstrip().startswith(("*", "-"))))
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


def markdown_prose(text: str) -> list[tuple[int, str, bool]]:
    """The prose lines of a Markdown file: no fence, no table, no heading.

    A bullet opens a paragraph of its own. Merging a list into one paragraph
    counted its items as sentences, which is the metric measuring the layout
    instead of the writing.
    """
    out: list[tuple[int, str, bool]] = []
    fenced = False
    for lineno, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or not line.strip():
            continue
        if line.lstrip().startswith(("|", "#")):
            continue
        bullet = bool(BULLET.match(line))
        out.append((lineno, BULLET.sub("", line).strip(), bullet))
    return out


def clean(text: str) -> str:
    """Drop what is not English: code spans, link targets, URLs, emphasis."""
    text = CODE_SPAN.sub("CODE", text)
    text = LINK.sub(r"\1", text)
    text = URL.sub("URL", text)
    return text.replace("**", "").replace("__", "")


def opener_breaches(lines: list[tuple[int, str, bool]]) -> list[tuple[int, str]]:
    """The one hard check: an empty opener the style file names."""
    out = []
    for lineno, raw, _ in lines:
        for match in OPENERS.finditer(clean(raw)):
            out.append((lineno, f'empty opener: "{match.group(0)}"'))
    return out


def paragraphs(lines: list[tuple[int, str, bool]]) -> list[tuple[int, str]]:
    """Group consecutive prose lines into paragraphs, keyed by first line.

    A blank line, a fence and a bullet each open a new one.
    """
    out: list[tuple[int, str]] = []
    start: int | None = None
    buffer: list[str] = []
    previous: int | None = None
    for lineno, text, opens in lines:
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


def measure(text: str, lines: list[tuple[int, str]]) -> tuple[dict[str, int], list[tuple[int, str]]]:
    """The five ratcheted counts of one file, and one message per breach."""
    counts = {"ascii": 0, "latin": 0, "long": 0, "para": 0, "passive": 0}
    detail: list[tuple[int, str]] = []
    for lineno, reason in printable_breaches(text):
        counts["ascii"] += 1
        detail.append((lineno, reason))
    for lineno, raw, _ in lines:
        for match in LATIN.finditer(clean(raw)):
            counts["latin"] += 1
            detail.append((lineno, f'Latin abbreviation: "{match.group(0)}"'))
    for lineno, text in paragraphs(lines):
        parts = sentences(text)
        if len(parts) > PARAGRAPH_SENTENCES_MAX:
            counts["para"] += 1
            detail.append((lineno, f"{len(parts)} sentences, budget {PARAGRAPH_SENTENCES_MAX}"))
        for part in parts:
            words = len(part.split())
            if words > SENTENCE_WORDS_MAX:
                counts["long"] += 1
                detail.append((lineno, f"{words} words in one sentence: {part[:60]}..."))
            for match in PASSIVE.finditer(part):
                counts["passive"] += 1
                detail.append((lineno, f'passive: "{match.group(0)}"'))
    return counts, detail


METRICS = (
    ("ascii", "non-printable characters"),
    ("latin", "Latin abbreviations"),
    ("long", f"sentences over {SENTENCE_WORDS_MAX} words"),
    ("para", f"paragraphs over {PARAGRAPH_SENTENCES_MAX} sentences"),
    ("passive", "passive verbs"),
)

# The style file quotes the four openers it bans, so it breaches its own rule by
# stating it. The one exemption, named here rather than left as a mystery.
OPENER_EXEMPT = {"ai/ambre-writing-style.md"}


# `specs/reviews/` and `specs/Todo/` are local working notes, excluded from the
# repository. A ceiling cannot hold a file no commit carries, so they are out,
# for the same reason `spec_shape.py` skips them.
UNTRACKED_DIRS = ("/reviews/", "/Todo/")


def tracked(path: Path) -> bool:
    return not any(part in f"/{path.relative_to(ROOT)}" for part in UNTRACKED_DIRS)


def rust_files() -> list[Path]:
    return sorted(p for root in RUST_ROOTS for p in root.rglob("*.rs") if tracked(p))


def markdown_files() -> list[Path]:
    found = [p for root in MARKDOWN_ROOTS for p in root.rglob("*.md") if tracked(p)]
    found += [p for p in MARKDOWN_FILES if p.exists()]
    return sorted(found)


def load_ceilings() -> dict[str, dict[str, int]]:
    return json.loads(CEILINGS.read_text()) if CEILINGS.exists() else {}


def main() -> int:
    check = "--check" in sys.argv
    write = "--ceilings" in sys.argv
    allow_raise = "--raise" in sys.argv
    ceilings = load_ceilings()
    fresh: dict[str, dict[str, int]] = {}
    errors: list[str] = []
    verbose = "--verbose" in sys.argv

    for path in rust_files() + markdown_files():
        rel = str(path.relative_to(ROOT))
        text = path.read_text()
        lines = rust_comments(text) if path.suffix == ".rs" else markdown_prose(text)
        if rel not in OPENER_EXEMPT:
            for lineno, reason in opener_breaches(lines):
                errors.append(f"{rel}:{lineno}: {reason}")
        counts, detail = measure(text, lines)
        if any(counts.values()):
            fresh[rel] = counts
        recorded = ceilings.get(rel) or {}
        for key, label in METRICS:
            ceiling = recorded.get(key, 0)
            if counts[key] > ceiling:
                errors.append(
                    f"{rel}: {counts[key]} {label}, ceiling {ceiling} -- it may only fall"
                )
        if verbose and detail:
            for lineno, reason in detail:
                print(f"  {rel}:{lineno}: {reason}")

    if write:
        merged = dict(ceilings)
        for rel, counts in fresh.items():
            recorded = merged.get(rel) or {}
            # A ceiling only falls. `--raise` is the reviewed exception, so a
            # growth is recorded in a diff instead of happening in silence.
            merged[rel] = {
                key: (counts[key] if allow_raise else min(counts[key], recorded.get(key, counts[key])))
                for key, _ in METRICS
            }
        # A file with no breach left carries no entry: the ceiling is zero.
        merged = {k: v for k, v in merged.items() if any(v.values()) and (ROOT / k).exists()}
        CEILINGS.write_text(json.dumps(dict(sorted(merged.items())), indent=2) + "\n")
        print(f"{len(merged)} files carry a prose ceiling")
        return 0

    total = sum(sum(c.values()) for c in fresh.values())
    print(f"{len(rust_files())} Rust files, {len(markdown_files())} documents")
    print(f"{total} ratcheted breaches under ceiling, {len(errors)} defects")
    for error in errors:
        print(f"ERROR {error}")
    return 1 if (check and errors) else 0


if __name__ == "__main__":
    raise SystemExit(main())
