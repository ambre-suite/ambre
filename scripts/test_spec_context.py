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


def a_story(rows: int) -> tuple[str, list[str]]:
    """One story of the roadmap landing at least `rows` rows, and those rows.

    A story name written into this file is a name a build ticks, and a ticked
    story is a test that fails for the one reason a test must never fail. So the
    corpus picks it, in roadmap order, which keeps the choice the same between
    two runs of an unchanged tree.
    """
    records, graph = spec_context.load()
    for line in sorted({found[2] for found in spec_context.schedule().values()}):
        ids = spec_context.landing(line, records, graph)
        if len(ids) >= rows:
            name = next(found[3] for found in spec_context.schedule().values()
                        if found[2] == line)
            return name, ids
    raise AssertionError(f"no story of the roadmap lands {rows} rows")


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


class Stories(unittest.TestCase):
    """A builder is given a name, and the closure starts at an id."""

    def setUp(self):
        self.records, self.graph = spec_context.load()
        self.name, self.ids = a_story(2)
        self.out = spec_context.for_story(self.name, self.records, self.graph)
        self.text = "\n".join(self.out)

    def test_the_roadmap_row_is_carried_verbatim(self):
        """The story says what the work is. A closure that summarized it would
        put a second wording of the row beside the one the roadmap holds."""
        epic, _, lineno, _ = spec_context.scheduled(self.name)
        row = spec_context.lines_of(
            contracts.SPECS / contracts.ROADMAP
        )[lineno - 1]
        self.assertIn(row, self.out)
        self.assertIn(f"Epic {epic}", self.text)

    def test_every_row_landing_with_it_is_compiled(self):
        for rid in self.ids:
            self.assertIn(f"# CONTEXT FOR `{rid}`", self.out)

    def test_a_row_landing_with_another_story_is_left_out(self):
        other = next(rid for rid in self.records if rid not in self.ids)
        self.assertNotIn(f"# CONTEXT FOR `{other}`", self.out)

    def test_the_phase_code_reaches_the_row_its_name_reaches(self):
        """`ARCH P2` is a citation key of its own, and two keys that compiled
        two different closures would make the roadmap ambiguous."""
        coded = [found for found in spec_context.schedule().values()
                 if " -- " in found[3]]
        if not coded:
            self.skipTest("no story of the roadmap carries a phase code")
        epic, _, lineno, name = coded[0]
        del epic
        self.assertEqual(lineno, spec_context.scheduled(name.split(" -- ")[0])[2])

    def test_a_name_no_row_carries_is_refused(self):
        with self.assertRaises(spec_context.Missing):
            spec_context.for_story("no story is named this", self.records,
                                   self.graph)

    def test_a_story_landing_no_row_is_an_answer_and_not_a_refusal(self):
        """39 open stories land no row: they write rules the corpus does not
        carry yet. An empty output would read as a broken tool."""
        empty = [found for found in spec_context.schedule().values()
                 if not spec_context.landing(found[2], self.records, self.graph)]
        if not empty:
            self.skipTest("every story of the roadmap lands a row")
        out = spec_context.for_story(empty[0][3], self.records, self.graph)
        self.assertIn("No contract row lands with this story.", "\n".join(out))

    def test_a_block_quoted_once_is_not_quoted_again(self):
        """Rows of one story share a table. Quoting it per closure is the full
        read the compiler exists to replace, once per row."""
        self.assertIn("printed under `", self.text)
        alone = spec_context.closure(self.ids[0], self.records, self.graph)
        self.assertLess(spec_context.words(self.out),
                        spec_context.words(alone) * len(self.ids))

    def test_a_dropped_block_names_the_id_that_carries_it(self):
        """What is absent has an address in the same output, or the reader
        cannot tell a skipped block from a missing one."""
        for line in self.text.splitlines():
            if "printed under `" in line:
                named = line.split("printed under `")[1].split("`")[0]
                self.assertIn(named, self.ids)

    def test_a_single_id_still_quotes_everything_it_cites(self):
        """The register is passed in, so the dedup cannot reach the one-id run."""
        self.assertNotIn("printed under `", "\n".join(compiled(OPERATION)))


class Cost(unittest.TestCase):
    """The compiler has to pay for itself, over the corpus and not one row."""

    def test_the_closure_is_a_fraction_of_a_full_read(self):
        records, graph = spec_context.load()
        out = compiled(OPERATION)
        share = int(
            spec_context.measure(OPERATION, out, records, graph).split(": ")[1].split()[0]
        )
        self.assertLess(share, 100)

    def test_a_grouped_closure_that_costs_more_says_so(self):
        """A story of 67 rows costs more than its documents do. The compiler is
        what has to say it: a reader who finds out has already paid."""
        records, graph = spec_context.load()
        lines = sorted({found[2] for found in spec_context.schedule().values()})
        worst = ""
        for line in lines:
            ids = spec_context.landing(line, records, graph)
            if not ids:
                continue
            name = next(found[3] for found in spec_context.schedule().values()
                        if found[2] == line)
            out = spec_context.for_story(name, records, graph)
            cost = spec_context.measure_story(ids, out, records, graph)
            share = int(cost.split(": ")[1].split()[0])
            if share >= 100:
                worst = cost
                break
        if not worst:
            self.skipTest("every story is cheaper than a full read today")
        self.assertIn("cheaper read whole", worst)

    def test_every_story_of_the_roadmap_compiles(self):
        """The `--all` of the story mode. A story that raises is a name the
        roadmap carries and this file cannot follow."""
        records, graph = spec_context.load()
        refused = []
        for lineno in sorted({found[2] for found in
                              spec_context.schedule().values()}):
            name = next(found[3] for found in spec_context.schedule().values()
                        if found[2] == lineno)
            try:
                spec_context.for_story(name, records, graph)
            except spec_context.Missing as why:
                refused.append(f"{name}: {why}")
        self.assertEqual([], refused)

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
