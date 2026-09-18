#!/usr/bin/env python3
"""The project vocabulary, read from `specs/ambre-glossary.md`.

`scripts/template_vocabulary.py` states the rule this file obeys: a script that
declares a vocabulary again states it twice, and the second statement drifts
without a sound. The glossary is the one place a term, its gloss and the
synonyms it forbids are written. Nothing below is typed out here that the
glossary does not say first.

**A forbidden word may carry one frozen compound.** `deny (not default-deny)`
refuses the verb and leaves the CORS policy alone, because no regular expression
tells the two apart and a check that guesses is a check nobody trusts. The
exception is matched literally, before the word itself.

**An empty parse is a gate that accepts everything**, so a table this cannot
read fails at import rather than narrowing to what it found.
"""

from __future__ import annotations

import re
from pathlib import Path

GLOSSARY = Path(__file__).resolve().parent.parent / "specs" / "ambre-glossary.md"

# A row of any glossary table: term, gloss, forbidden, reference. The header and
# the alignment row carry no backtick in the reference cell, so both fall out.
ROW = re.compile(
    r"^\|\s*(?P<term>[^|]+?)\s*\|\s*(?P<gloss>[^|]*?)\s*\|"
    r"\s*(?P<forbidden>[^|]*?)\s*\|\s*`(?P<ref>[^`]+)`\s*\|\s*$",
    re.M,
)
# `deny (not default-deny)`: the word, and the one compound that stays legal.
FORBIDDEN = re.compile(r"^(?P<word>[a-z][a-z -]*?)\s*(?:\(not (?P<except>[^)]+)\))?$", re.I)


class Entry:
    """One glossary row, parsed."""

    def __init__(self, term: str, gloss: str, forbidden: str, ref: str) -> None:
        self.term = term
        self.gloss = gloss
        self.ref = ref
        self.forbidden: list[tuple[str, str | None]] = []
        for raw in (f.strip() for f in forbidden.split(",")):
            if not raw:
                continue
            match = FORBIDDEN.match(raw)
            if not match:
                raise ValueError(f"glossary: {term!r}: cannot read forbidden {raw!r}")
            self.forbidden.append((match["word"].strip(), match["except"]))


def entries() -> list[Entry]:
    """Every row of the glossary, in file order."""
    out = [
        Entry(m["term"], m["gloss"], m["forbidden"], m["ref"])
        for m in ROW.finditer(GLOSSARY.read_text())
    ]
    if not out:
        raise ValueError("glossary: no row parsed -- the table shape changed")
    return out
