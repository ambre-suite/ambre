#!/usr/bin/env python3
"""One fixture document, and every gate reading it the same way.

`corpus.py` decides once what a table, a fence, a section and a bullet are, and
every gate reads its result. Before, each gate walked the lines on its own, and
two of them read one contract table two ways: one closed it on a blank line and
the other did not. The fixture below carries every shape two readers used to
disagree on: a contract table broken by a blank line, a `**Frame:**` line under
it, a row and a section heading quoted inside a fence, and a nested bullet under
an answer. Each gate is then asked what it reads there, and the answers have to
agree with the corpus and with each other.

The second half of this file holds the move: no gate reads a Markdown file of
the corpus on its own again.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import contract_index  # noqa: E402
import contracts  # noqa: E402
import corpus  # noqa: E402  -- the module under test
import feature_inventory  # noqa: E402
import prose  # noqa: E402
import spec_answers  # noqa: E402
import spec_code  # noqa: E402
import spec_context  # noqa: E402
import spec_read  # noqa: E402
import spec_refs  # noqa: E402
import spec_shape  # noqa: E402

FENCE = "`" * 3

FIXTURE = "\n".join([
    "# AMBRE -- FEATURE: FIXTURE",
    "",
    "**Type:** Vertical (feature)",
    "**Applications:** Prose, Trame",
    "",
    "> A document every gate reads the same way.",
    "",
    "---",
    "",
    "## 1. RULES",
    "",
    "### The operation",
    "",
    "| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |",
    "| :--- | :--- | :--- | :--- | :--- | :--- | :--- |",
    "| `FIX-001` | `POST` | The fixture MUST hold. | derived | `crate::tests::holds` | none | verified |",
    "",
    "| `FIX-002` | `FRAME` | The fixture may change nothing else. | derived | not built | "
    "lands with `A story` Story (`ambre-roadmap.md` E9) | unimplemented |",
    "",
    "**Frame:** `FIX-002`",
    "",
    f"{FENCE}text",
    "| `FIX-009` | `POST` | A row quoted in a fence. | derived | none | none | verified |",
    "## 9. A HEADING QUOTED IN A FENCE",
    FENCE,
    "",
    "* **Budget.** none.",
    "* **Boundary.** none.",
    "* **Verification.** 1 `verified`, 1 `unimplemented`.",
    "  * A nested bullet the answer owns.",
    "",
    "## 2. WHAT IS LEFT",
    "",
    "A closing paragraph.",
    "",
])

ROWS = ["FIX-001", "FIX-002"]
NAME = "ambre-spec-fixture.md"


class Fixture(unittest.TestCase):
    """What the corpus reads in the fixture, and what each gate reads there."""

    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.specs = Path(tmp.name)
        self.path = self.specs / NAME
        self.path.write_text(FIXTURE)
        self.doc = corpus.parse(FIXTURE)

    def test_the_corpus_reads_one_table_across_the_blank_line(self) -> None:
        tables = self.doc.tables
        self.assertEqual(1, len(tables))
        self.assertTrue(tables[0].contract)
        self.assertEqual(ROWS, [row.id for row in self.doc.contract_rows])

    def test_the_corpus_reads_no_row_and_no_section_inside_a_fence(self) -> None:
        self.assertEqual(["1", "2"], [s.number for s in self.doc.sections])
        self.assertEqual(1, len(self.doc.fences))
        self.assertEqual("text", self.doc.fences[0].tag)

    def test_the_frame_line_closes_the_table(self) -> None:
        self.assertTrue(contracts.FRAME_LINE.match(self.doc.tables[0].closer.text))

    def test_the_contract_index_reads_the_rows_the_corpus_reads(self) -> None:
        with mock.patch.object(contracts, "SPECS", self.specs):
            found = [rid for _, rid, _, _ in contract_index.read_rows()]
        self.assertEqual(ROWS, found)

    def test_the_second_half_is_read_under_the_header(self) -> None:
        """The header names the cells of the row past the blank line too."""
        records = [contract_index.record(NAME, r.id, r.status, r.cells)
                   for r in self.doc.contract_rows]
        self.assertEqual(["POST", "FRAME"], [r["kind"] for r in records])
        self.assertIn("A story", records[1]["gap"])

    def test_the_inventory_counts_the_rows_the_corpus_reads(self) -> None:
        tally = feature_inventory.counts(FIXTURE)
        self.assertEqual(len(ROWS), sum(tally.values()))
        self.assertEqual(1, tally["unimplemented"])

    def test_the_reading_view_prints_the_rows_the_corpus_reads(self) -> None:
        view = spec_read.reading_view(FIXTURE)
        printed = [rid for rid in ROWS + ["FIX-009"] if any(f"`{rid}`" in line for line in view)]
        self.assertEqual(ROWS, printed)

    def test_the_closure_quotes_the_table_the_corpus_reads(self) -> None:
        heading, block, siblings = spec_context.table_of(self.path, "FIX-002")
        self.assertEqual("### The operation", heading)
        self.assertEqual(ROWS, [rid for rid, _ in siblings])
        self.assertIn("**Frame:** `FIX-002`", block)

    def test_the_shape_gate_reads_the_second_half_as_a_rule(self) -> None:
        found = list(spec_shape.keyword_issues(FIXTURE))
        self.assertEqual([18], [line for line, _ in found])
        self.assertEqual([], list(spec_shape.answer_issues(FIXTURE)))

    def test_the_shape_gate_numbers_no_fenced_heading(self) -> None:
        self.assertEqual([], list(spec_shape.numbering_issues(FIXTURE)))
        self.assertEqual({"1", "2"}, spec_refs.sections(self.path))

    def test_the_closure_cuts_a_section_where_the_corpus_does(self) -> None:
        kept = spec_context.section(self.path, "1")
        self.assertEqual("## 1. RULES", kept[0])
        self.assertNotIn("## 2. WHAT IS LEFT", kept)
        self.assertIn("## 9. A HEADING QUOTED IN A FENCE", kept)

    def test_every_reader_counts_the_fence_as_one_block(self) -> None:
        fenced = spec_shape.classify(FIXTURE).fenced_lines
        self.assertEqual(4, fenced)
        self.assertEqual([], list(spec_code.fence_issues(FIXTURE)))
        prose_lines = {line for line, _, _, _ in prose.markdown_prose(FIXTURE)}
        self.assertFalse(prose_lines & {23, 24, 25, 26})

    def test_the_prose_reader_marks_the_table_as_structure(self) -> None:
        table = {line for line, _, _, is_prose in prose.markdown_prose(FIXTURE) if not is_prose}
        self.assertEqual({14, 15, 16, 18}, table)

    def test_the_answer_owns_its_nested_bullet(self) -> None:
        """The word budget counts the nested bullet; the count parse stops before it."""
        verification = [b for b in self.doc.bullets if b.label == contracts.VERIFICATION]
        self.assertEqual(1, len(verification))
        self.assertEqual(1, len(verification[0].lines))
        self.assertEqual(2, len(verification[0].item))
        self.assertEqual({"verified": 1, "unimplemented": 1},
                         contracts.stated_counts(verification[0].joined))
        self.assertEqual(3 + 3 + 6 + 7, spec_answers.answer_words(FIXTURE))

    def test_one_text_is_parsed_once(self) -> None:
        self.assertIs(corpus.parse(FIXTURE), corpus.read(self.path))
        self.assertIs(corpus.as_document(FIXTURE), corpus.as_document(self.doc))


# The gates that read the spec corpus. `prose.py` still reads the Markdown of a
# Rust doc comment on its own: a comment is not a document of the corpus.
GATES = (
    "contract_index", "contracts", "feature_inventory", "glossary", "spec_answers",
    "spec_code", "spec_context", "spec_read", "spec_refs", "spec_shape",
    "template_vocabulary", "term_drift",
)
# What a gate reading the Markdown on its own writes: a fence test, a table
# test, its own walk of a Markdown tree.
OWN_PARSE = (f'"{FENCE}"', 'startswith("|")', "startswith('|')", 'rglob("*.md")')


class NoGateParsesOnItsOwn(unittest.TestCase):
    def test_every_gate_reads_through_the_corpus(self) -> None:
        for name in GATES:
            with self.subTest(gate=name):
                source = (HERE / f"{name}.py").read_text()
                self.assertIn("import corpus", source.replace("prose.corpus", "import corpus"))
                for marker in OWN_PARSE:
                    self.assertNotIn(marker, source, f"{name}.py parses on its own")


if __name__ == "__main__":
    unittest.main()
