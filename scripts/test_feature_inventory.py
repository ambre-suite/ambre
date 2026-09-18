#!/usr/bin/env python3
"""What `feature_inventory.py` derives, and what it refuses.

This script writes a file nobody reviews line by line: a reader trusts the
product surface because a generator produced it. So a defect here is invisible
twice -- the derived state is wrong, and the file it lands in looks generated
and therefore right.

One such defect has already shipped. The script named `unverified`, a status the
template no longer has, and `read()` raised a `KeyError` the day an inventory
was regenerated; the same name inside a `Counter` elsewhere returned 0 instead
and under-reported the shipped surface by 826 rows. Both statements now read the
vocabulary from one place, and the tests below hold that they still agree.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contracts  # noqa: E402  -- where `LOOSE` is stated
import feature_inventory as inventory  # noqa: E402  -- the module under test
import template_vocabulary  # noqa: E402  -- where the template's sets are read


def row(status: str, ident: str = "FR-001") -> str:
    """One contract row, in the seven columns the template declares."""
    return (f"| `{ident}` | `INV` | The frame MUST land. | derived"
            f" | none | none | {status} |")


def doc(*lines: str) -> str:
    return "\n".join(lines) + "\n"


class Vocabulary(unittest.TestCase):
    """One statement of the statuses, read by every gate that needs them."""

    def test_the_unbuilt_status_is_one_the_template_declares(self):
        """The defect that shipped: this file named a status the template had
        dropped, and nothing said so until an inventory was regenerated."""
        self.assertIn(inventory.UNBUILT, template_vocabulary.STATUSES)

    def test_loose_is_read_from_contracts_never_typed_again(self):
        self.assertIs(inventory.LOOSE, contracts.LOOSE)

    def test_every_loose_status_is_one_the_template_declares(self):
        for status in inventory.LOOSE:
            with self.subTest(status=status):
                self.assertIn(status, template_vocabulary.STATUSES)

    def test_the_unbuilt_status_is_not_loose(self):
        """`loose` counts **built** rows that nothing green holds."""
        self.assertNotIn(inventory.UNBUILT, inventory.LOOSE)

    def test_the_states_are_the_headers_own_vocabulary(self):
        """Two readings of one fact, one by hand and one derived, so they can
        disagree -- which they cannot do in two vocabularies."""
        self.assertIs(inventory.STATES, template_vocabulary.IMPLEMENTATION_VALUES)


class Counts(unittest.TestCase):
    def test_a_row_is_counted_under_its_status(self):
        text = doc(row("verified"), row("failing", "FR-002"), row("verified", "FR-003"))
        tally = inventory.counts(text)
        self.assertEqual(2, tally["verified"])
        self.assertEqual(1, tally["failing"])

    def test_every_status_of_the_template_has_a_slot(self):
        """A missing key is what turned a status rename into a `KeyError` in
        one file and a silent zero in another."""
        self.assertEqual(set(template_vocabulary.STATUSES), set(inventory.counts("")))

    def test_a_status_the_template_does_not_declare_is_not_counted(self):
        self.assertEqual(0, sum(inventory.counts(doc(row("unverified"))).values()))

    def test_a_line_that_is_not_a_contract_row_is_not_counted(self):
        self.assertEqual(0, sum(inventory.counts(doc("| Feature | Scope | verified |")).values()))


class Scope(unittest.TestCase):
    """The document's own summary, and nothing written here."""

    def test_the_scope_is_the_first_sentence_of_the_summary(self):
        text = doc("# TITLE", "", "> Frames the page. It also paints the margin.", "")
        self.assertEqual("Frames the page.", inventory.scope(text))

    def test_a_wrapped_summary_is_one_summary(self):
        text = doc("# TITLE", "", "> Frames the page", "> and paints the margin.", "")
        self.assertEqual("Frames the page and paints the margin.", inventory.scope(text))

    def test_a_document_with_no_summary_has_no_scope(self):
        self.assertEqual("", inventory.scope(doc("# TITLE", "", "The frame lands.")))


class Derived(unittest.TestCase):
    """The state the rows say, read from a document on disk."""

    def read(self, *rows: str, declared: str = "implemented") -> dict:
        directory = tempfile.mkdtemp()
        path = Path(directory) / "ambre-spec-frame.md"
        path.write_text(doc(
            "# AMBRE -- FRAME",
            "",
            "**Type:** Feature",
            f"**Implementation:** {declared}",
            "",
            "> Frames the page.",
            "",
            *rows,
        ))
        self.addCleanup(shutil.rmtree, directory)
        return inventory.read(path)

    def test_no_unbuilt_row_reads_implemented(self):
        self.assertEqual("implemented", self.read(row("verified"), row("failing", "FR-002"))["derived"])

    def test_one_unbuilt_row_among_built_ones_reads_partial(self):
        self.assertEqual("partial", self.read(row("verified"), row("unimplemented", "FR-002"))["derived"])

    def test_nothing_built_reads_design_only(self):
        self.assertEqual("design-only", self.read(row("unimplemented"))["derived"])

    def test_the_cite_name_drops_the_fixed_prefix(self):
        self.assertEqual("frame", self.read(row("verified"))["cite"])

    def test_the_declared_state_is_read_from_the_header(self):
        """Two readings of one fact, kept apart so they can disagree."""
        found = self.read(row("unimplemented"), declared="implemented")
        self.assertEqual("implemented", found["declared"])
        self.assertEqual("design-only", found["derived"])

    def test_a_loose_row_is_built_and_counted_loose(self):
        found = self.read(row("untested"))
        self.assertEqual(1, found["built"])
        self.assertEqual(1, found["loose"])

    def test_a_verified_row_is_built_and_not_loose(self):
        found = self.read(row("verified"))
        self.assertEqual(1, found["built"])
        self.assertEqual(0, found["loose"])


class Defects(unittest.TestCase):
    """What the rows say and the header does not."""

    def base(self, declared: str, rows: int, built: int) -> dict:
        return {"cite": "frame", "scope": "", "declared": declared,
                "derived": "", "rows": rows, "built": built, "loose": 0}

    def test_a_feature_document_with_no_contract_table_is_refused(self):
        """`contracts.py` counts the rows it finds; it cannot see a document
        that declares none."""
        found = inventory.defects("trame", [self.base("partial", 0, 0)])
        self.assertIn("no contract table", found[0])

    def test_a_header_claiming_more_than_the_rows_support_is_the_lie(self):
        found = inventory.defects("trame", [self.base("implemented", 4, 3)])
        self.assertIn("header says `implemented`", found[0])

    def test_a_header_claiming_less_is_legitimate(self):
        """Asymmetric on purpose: a surface may ship before it has a row."""
        self.assertEqual([], inventory.defects("trame", [self.base("partial", 4, 4)]))

    def test_a_design_only_header_over_built_rows_is_refused(self):
        found = inventory.defects("trame", [self.base("design-only", 4, 2)])
        self.assertIn("header says `design-only`", found[0])


class Render(unittest.TestCase):
    """The generated file, whose text lives in one place."""

    def entry(self, derived: str, cite: str = "frame", rows: int = 2, loose: int = 0) -> dict:
        return {"cite": cite, "scope": "Frames the page.", "declared": derived,
                "derived": derived, "rows": rows, "built": rows, "loose": loose}

    def test_the_table_is_ordered_by_state_then_by_name(self):
        """Alphabetical inside a state, so a diff shows the feature that moved
        and nothing else."""
        text = inventory.render("trame", [
            self.entry("design-only", "zoom"),
            self.entry("implemented", "undo"),
            self.entry("implemented", "find"),
        ])
        order = [line.split("`")[1] for line in text.splitlines() if line.startswith("| `")]
        self.assertEqual(["find", "undo", "zoom"], order)

    def test_the_loose_legend_names_the_statuses_the_count_covers(self):
        text = inventory.render("trame", [self.entry("implemented")])
        for status in inventory.LOOSE:
            with self.subTest(status=status):
                self.assertIn(f"`{status}`", text)

    def test_a_feature_with_no_loose_row_leaves_the_cell_empty(self):
        text = inventory.render("trame", [self.entry("implemented", loose=0)])
        self.assertTrue(any(line.endswith("| 2 |  |") for line in text.splitlines()))

    def test_the_totals_are_the_sum_of_the_rows(self):
        text = inventory.render("trame", [
            self.entry("implemented", "find", rows=3, loose=1),
            self.entry("partial", "undo", rows=2, loose=2),
        ])
        self.assertIn("5 contract rows, 3 loose", text)


if __name__ == "__main__":
    unittest.main()
