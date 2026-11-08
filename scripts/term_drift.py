#!/usr/bin/env python3
"""Report the competing term families of the corpus, for a human to judge.

**This script decides nothing, and that is the design.** Whether `drop` and
`discard` name one idea cannot be read off a count: it is read off two
sentences. `scripts/prose.py` holds the words the glossary already forbids,
exactly and with no judgement. This one finds the words it does not yet hold,
and hands them over with the evidence.

The measurement it repeats found `drop` against `discard` by putting two
paragraphs side by side, a sentence apart, in `ambre-spec-toile-engine.md` and
`ambre-spec-prose-engine.md`. It also found three families that look like drift
and are not: `seal` / `freeze` / `commit` name three ideas, and so do `budget` /
`cap` / `ceiling`. A count alone would have banned six correct words.

**Read the contexts, never the totals.** A family whose members are spread
evenly is usually three ideas. A family with one dominant term and a handful of
stragglers is usually drift, and the stragglers are usually right about
something the dominant term misses. Neither reading is safe without `--context`.

Usage:
    python3 scripts/term_drift.py                    # the families of the glossary
    python3 scripts/term_drift.py drop discard evict # one family, ad hoc
    python3 scripts/term_drift.py --context drop discard
"""

from __future__ import annotations

import collections
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import glossary  # noqa: E402  -- the vocabulary, read from the corpus
import prose  # noqa: E402  -- the prose readers and filters, over `corpus.py`

# A term and its ordinary inflections. Wide on purpose: a miss here is a synonym
# the report never shows, and the cost of a false hit is one line a reader skips.
INFLECTIONS = r"(s|es|ed|d|ing)?"
CONTEXT_CHARS = 70


def families() -> list[list[str]]:
    """Every glossary row that already names a forbidden word, as a family.

    The glossary is the record of what drift was found before, so re-running it
    is the cheapest regression: a word that came back shows up first.
    """
    out = []
    for entry in glossary.entries():
        if entry.forbidden:
            out.append([entry.term] + [word for word, _ in entry.forbidden])
    return out


def measure(terms: list[str]) -> tuple[collections.Counter, dict[str, list[str]]]:
    """Per-term counts across the corpus, and one context line per hit."""
    counts: collections.Counter = collections.Counter()
    contexts: dict[str, list[str]] = collections.defaultdict(list)
    patterns = {t: re.compile(rf"\b{re.escape(t)}{INFLECTIONS}\b", re.I) for t in terms}
    for path in prose.rust_files() + prose.markdown_files():
        rel = str(path.relative_to(prose.ROOT))
        if path.suffix == ".rs":
            lines = prose.rust_comments(path.read_text())
        else:
            lines = prose.markdown_prose(prose.corpus.read(path))
        for lineno, para in prose.paragraphs(lines):
            body = prose.clean(para)
            for term, pattern in patterns.items():
                for match in pattern.finditer(body):
                    counts[term] += 1
                    start = max(0, match.start() - CONTEXT_CHARS)
                    end = min(len(body), match.end() + CONTEXT_CHARS)
                    contexts[term].append(f"{rel}:{lineno}  ...{body[start:end]}...")
    return counts, contexts


def report(terms: list[str], with_context: bool) -> None:
    counts, contexts = measure(terms)
    total = sum(counts.values())
    print(f"\n=== {' / '.join(terms)}   total {total}")
    for term in terms:
        print(f"  {term:16} {counts[term]:5}")
    if with_context:
        for term in terms:
            for line in contexts[term]:
                print(f"    [{term}] {line}")


def main(argv: list[str]) -> int:
    with_context = "--context" in argv
    terms = [a for a in argv if not a.startswith("--")]
    if terms:
        report(terms, with_context)
        return 0
    for family in families():
        report(family, with_context)
    print("\nA count decides nothing. Read the contexts before a word is forbidden.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
