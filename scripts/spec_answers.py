#!/usr/bin/env python3
"""Fail when the three answers under a contract table turn into an essay.

`scripts/spec_shape.py` holds whether the `Budget`, `Boundary` and
`Verification` bullets exist and are non-empty. `scripts/contracts.py` holds
whether the counts the `Verification` answer opens on are the table's own. This
holds the one thing neither reads: **how much prose those three answers spend**,
and whether that prose is about the system or about the document.

Two metrics, both a hard cap:

  * answer words -- the words of the three bullets, taken together. The three
                    bullets are one paragraph to a reader and to
                    `scripts/prose.py`, which already caps them at six
                    sentences. Six sentences under the 25-word sentence cap is
                    150 words, so 150 is the target the template states, and
                    the gate refuses a document over it. The corpus measured a
                    median of 199 and a maximum of 463 when this script was
                    written, 99 documents of 124 over the target: a ratchet
                    carried them down, and the cap holds them there on its own.
  * history      -- a line writing the history of the document itself: which
                    conjunction came apart, what this pass added, which row was
                    carrying which half. That is a commit message. It ages
                    without a reader, no gate can tell whether it is still true,
                    and it was the bulk of what the `Verification` answers spent
                    their words on. Measured over the whole document, not just
                    the answers, because the shape spreads.

**Why the markers are narrow.** `no longer` and `is new` read as history and are
overwhelmingly the system: a target that no longer exists, an address that is
new. A wide net over those two matched 80 lines of which a handful were history.
Each pattern below names a verb as well as a subject, so `the pass added` is
caught and `the settling pass runs` is not. `landed`, `used to` and `was fixed`
were here and are gone: a lone verb is not a subject, so "the codec used to
encode the frame" was refused as production history. A marker this script
misses is a review act, like a `Budget` answer that is false.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"

# The target the template states, and the cap this script holds.
ANSWER_WORDS_TARGET = 150

# A journal is history by definition: it says so in its own first line, and it
# carries no contract table. `specs/audits/` is the same, by directory.
NON_NORMATIVE = {"ambre-roadmap-journal.md"}

# The document that states the rule has to spell the markers out to state it.
# Exempt from `history` alone: its answers are measured like any other's.
HISTORY_EXEMPT = {"ambre-spec-template.md"}
GENERATED = re.compile(r"-(inventory|ledger)\.md$")

ANSWER = re.compile(r"^\*\s+\*\*(?:Budget|Boundary|Verification)\.\*\*")
CONTRACT_ROW = re.compile(r"^\|\s*`[A-Z][A-Z0-9-]*-\d+`\s*\|")

# The document writing about itself. Each pattern pairs a subject that could be
# the system with a verb that could not.
HISTORY_MARKERS = [
    (re.compile(r"\bcame apart\b", re.I), "how a row was split is a commit message"),
    (re.compile(r"\bcomes apart\b", re.I), "how a row was split is a commit message"),
    (re.compile(r"\bth(?:e|is) pass\b(?=[^.]*\b(?:added|adds|split|splits|found|finds|took|takes|exposed|opened|gave|made)\b)", re.I),
     "what a pass did to this document is a commit message"),
    (re.compile(r"\bbefore this pass\b", re.I), "what a pass did to this document is a commit message"),
    (re.compile(r"\b(?:was|were) carrying\b", re.I), "which row carried which half is a commit message"),
    (re.compile(r"\brows?\s+(?:is|are)\s+new\b", re.I), "a row that is new here is a row: git says when it arrived"),
    (re.compile(r"\bconditions?\s+(?:is|are)\s+new\b", re.I), "a row that is new here is a row: git says when it arrived"),
]

METRICS = (
    ("answer_words", "words of answer"),
    ("history", "lines of document history"),
)

# What every document holds. A ratchet once softened the two while the corpus
# migrated; the corpus is under both, so the cap stands alone.
CAPS = {"answer_words": ANSWER_WORDS_TARGET, "history": 0}


def measured_files() -> list[tuple[str, str]]:
    """`(rel, text)` for every normative document under `specs/`."""
    out = []
    for path in sorted(SPECS.rglob("*.md")):
        rel = str(path.relative_to(SPECS))
        if "reviews/" in rel or "Todo/" in rel or "audits/" in rel:
            continue
        if path.name in NON_NORMATIVE or GENERATED.search(path.name):
            continue
        out.append((rel, path.read_text()))
    return out


def answer_words(text: str) -> int:
    """The words the three answers spend, bullet and continuation together.

    A continuation is an indented non-empty line under an answer bullet, which
    is how the template wraps one. A blank line ends the run: an answer that
    needs a second paragraph has already lost.
    """
    total = 0
    inside = False
    for line in text.splitlines():
        if ANSWER.match(line):
            inside = True
        elif inside and not (line.startswith("  ") and line.strip()):
            inside = False
        if inside:
            total += len(line.split())
    return total


def history_lines(text: str):
    """Yield `(line_no, reason)` for every line writing the document's history."""
    fenced = False
    for i, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        # A table cell states a gap, which is what the row is for, and the
        # contract grammar puts the Story of an `unimplemented` row there.
        if fenced or line.lstrip().startswith("|"):
            continue
        for pattern, reason in HISTORY_MARKERS:
            if pattern.search(line):
                yield i, f"{reason}: {line.strip()[:70]}"
                break


def main() -> int:
    check = "--check" in sys.argv
    fresh: dict[str, dict[str, int]] = {}
    errors: list[str] = []
    detail: dict[str, list[str]] = {}

    for rel, text in measured_files():
        lines = text.splitlines()
        if not any(CONTRACT_ROW.match(line) for line in lines):
            # No table, no answers owed. History is still refused, so the
            # document is measured on that metric alone.
            words = 0
        else:
            words = answer_words(text)
        found = [] if rel in HISTORY_EXEMPT else list(history_lines(text))
        detail[rel] = [f"{rel}:{n}: {reason}" for n, reason in found]
        fresh[rel] = {"answer_words": words, "history": len(found)}

        for key, label in METRICS:
            if fresh[rel][key] > CAPS[key]:
                errors.append(
                    f"{rel}: {fresh[rel][key]} {label}, cap {CAPS[key]}"
                )

    over = sum(1 for m in fresh.values() if m["answer_words"] > ANSWER_WORDS_TARGET)
    history = sum(m["history"] for m in fresh.values())
    for rel in sorted(fresh):
        flag = "!" if fresh[rel]["answer_words"] > ANSWER_WORDS_TARGET else " "
        print(
            f"{flag} {fresh[rel]['answer_words']:>4} answer words"
            f"  {fresh[rel]['history']:>3} history  {rel}"
        )
    for rel in sorted(detail):
        for line in detail[rel]:
            print(f"  {line}")
    print(
        f"\n{over} documents over the {ANSWER_WORDS_TARGET}-word target,"
        f" {history} lines of document history"
    )
    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    print(f"{len(errors)} answer defects")
    return 1 if (check and errors) else 0


if __name__ == "__main__":
    sys.exit(main())
