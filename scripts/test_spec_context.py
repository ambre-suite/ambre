#!/usr/bin/env python3
"""What the closure of one contract id carries, and what it leaves out.

A compiler that reads too much is the full read it replaces, and a compiler that
reads too little is worse than one: a builder trusts it and ships against half a
contract. Both failures are silent, so both are held here by a row of the corpus
rather than by a fixture.

The strongest test is the last: every id of the corpus compiles. A closure that
raises is a citation the corpus writes and this file cannot follow, and it is
found on the first run rather than the day someone asks for that row.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contracts  # noqa: E402  -- where the corpus lives
import spec_context  # noqa: E402  -- the module under test


# One document of the corpus, and rows of it that sit in two different tables.
# `TRAME-130` is a postcondition of `1.2 Merge cells`; `TRAME-125` is a state
# row, three subsections above it, in the same file.
MERGES = "2-Trame/features/ambre-spec-trame-cell-merges.md"
OPERATION, STATE = "TRAME-130", "TRAME-125"


def compiled(rid: str) -> list[str]:
    records, graph = spec_context.load()
    return spec_context.closure(rid, records, graph)


class Tables(unittest.TestCase):
    """The unit is the table, never the document."""

    def setUp(self):
        self.path = contracts.SPECS / MERGES

    def test_the_table_carries_the_rows_after_the_target(self):
        """A closure that stopped at the target row would hand a builder the
        rows above it and hide the ones below, which is half a contract."""
        _, _, siblings = spec_context.table_of(self.path, OPERATION)
        ids = [rid for rid, _ in siblings]
        self.assertEqual(OPERATION, ids[0])
        self.assertGreater(len(ids), 1)

    def test_the_table_stops_at_its_own_boundary(self):
        """The state table and the operation tables are separate contracts."""
        _, _, siblings = spec_context.table_of(self.path, OPERATION)
        self.assertNotIn(STATE, [rid for rid, _ in siblings])

    def test_the_heading_is_the_one_above_the_table(self):
        heading, _, _ = spec_context.table_of(self.path, OPERATION)
        self.assertTrue(heading.startswith("### 1.2"))

    def test_an_id_the_document_does_not_declare_is_refused(self):
        with self.assertRaises(spec_context.Missing):
            spec_context.table_of(self.path, "TRAME-999")


class Sections(unittest.TestCase):
    """A cited section, quoted whole and never past its end."""

    def setUp(self):
        self.path = contracts.SPECS / MERGES

    def test_the_body_opens_on_its_heading(self):
        body = spec_context.section(self.path, "3")
        self.assertTrue(body[0].startswith("## 3."))

    def test_the_body_stops_before_the_next_section(self):
        body = spec_context.section(self.path, "3")
        self.assertEqual(1, len([line for line in body if line.startswith("## ")]))

    def test_a_section_the_document_lacks_is_refused(self):
        with self.assertRaises(spec_context.Missing):
            spec_context.section(self.path, "44")


class Closure(unittest.TestCase):
    """The seven blocks, in the order a builder asks for them."""

    def setUp(self):
        self.out = compiled(OPERATION)
        self.text = "\n".join(self.out)

    def spine(self) -> list[str]:
        return [line for line in self.out if line.startswith(spec_context.BLOCK)]

    def test_the_seven_questions_are_answered_in_order(self):
        asked = [line.split(" -- ")[0].split()[-1] for line in self.spine()]
        self.assertEqual(["1", "2", "3", "4", "5", "6", "7", "CONTEXT"], asked)

    def test_a_quoted_heading_cannot_be_read_as_a_block(self):
        """A cited section is quoted verbatim, `## 3. MODEL` included. A spine
        numbered the same way would give the closure two structures, and a
        reader no way to tell which heading is the compiler talking."""
        self.assertIn("## 3. MODEL", self.text)
        self.assertEqual(8, len(self.spine()))

    def test_the_statement_is_the_first_thing_carried(self):
        records, _ = spec_context.load()
        self.assertIn(records[OPERATION]["statement"], self.text)

    def test_an_unrelated_row_of_the_same_document_is_left_out(self):
        """The done-when of this phase. A closure carrying the document's other
        tables is the full read it replaces, one indirection later."""
        self.assertNotIn(STATE, self.text)

    def test_the_frame_conditions_come_before_the_siblings(self):
        spine = self.spine()
        self.assertLess(spine.index(f"{spec_context.BLOCK} 2 -- WHAT IT MUST NOT CHANGE"),
                        spine.index(f"{spec_context.BLOCK} 3 -- THE OPERATION IT BELONGS TO"))

    def test_the_gap_and_its_story_are_carried(self):
        records, graph = spec_context.load()
        unbuilt = next(rid for rid, row in records.items()
                       if row["status"] == "unimplemented")
        text = "\n".join(spec_context.closure(unbuilt, records, graph))
        self.assertIn("Epic ", text)

    def test_an_id_no_row_declares_is_refused(self):
        with self.assertRaises(spec_context.Missing):
            compiled("NOPE-001")


class Pointers(unittest.TestCase):
    """What to open next, never the code itself."""

    def test_a_path_is_carried_once(self):
        found = spec_context.pointers(
            ["`crates/a/src/b.rs` and `crates/a/src/b.rs` again"]
        )
        self.assertEqual(["crates/a/src/b.rs"], found)

    def test_a_line_range_is_cut_from_the_path(self):
        """P2 refuses a range in the corpus, and a record quoting one would
        hand a builder a coordinate the file may no longer have."""
        self.assertEqual(["crates/a/src/b.rs"],
                         spec_context.pointers(["`crates/a/src/b.rs:12-40`"]))

    def test_no_rust_source_is_inlined(self):
        out = compiled(OPERATION)
        at = out.index(f"{spec_context.BLOCK} -- WHERE TO GO NEXT")
        self.assertNotIn("fn ", "\n".join(out[at:]))


class Cost(unittest.TestCase):
    """The compiler has to pay for itself, over the corpus and not one row."""

    def test_the_closure_is_a_fraction_of_a_full_read(self):
        records, graph = spec_context.load()
        out = compiled(OPERATION)
        share = int(
            spec_context.measure(OPERATION, out, records, graph).split(": ")[1].split()[0]
        )
        self.assertLess(share, 100)

    def test_every_id_of_the_corpus_compiles(self):
        """The one test that reads the whole corpus. A closure that raises is a
        citation the corpus writes and this file cannot follow."""
        records, graph = spec_context.load()
        refused = []
        for rid in records:
            try:
                spec_context.closure(rid, records, graph)
            except spec_context.Missing as why:
                refused.append(f"{rid}: {why}")
        self.assertEqual([], refused)


if __name__ == "__main__":
    unittest.main()
