#!/usr/bin/env python3
"""What `spec_shape.py` reads, and what it refuses.

Nine checks guard every spec of the corpus, and nothing guarded them. The
argument is the one `test_prose.py` states: a silent gate and a satisfied gate
read the same from outside, so a check that stopped matching reports success
until a reader finds the breach by hand.

Each check is frozen here twice -- what it must refuse, and what it must let
through. The second half is the one that earns the file. Three markers reading
a verb and guessing its subject (`landed`, `used to`, `was fixed`) sat in
`FEATURE_MARKERS` refusing "the codec used to encode the frame", and no test
said what the check was supposed to accept, so nothing contradicted them.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_shape  # noqa: E402  -- the module under test


def doc(*lines: str) -> str:
    return "\n".join(lines) + "\n"


def reasons(issues) -> list[str]:
    """The reason of each defect, dropping the line number a test never pins."""
    return [reason for _, reason in issues]


def kinds(issues) -> list[str]:
    """The kind of each defect, which is what a header or an answer yields."""
    return [kind for kind, _ in issues]


# A contract row in the seven columns the template declares. `CONTRACT_ROW`
# matches on the ID cell alone -- this script only needs to know the table is
# there -- but the `Statement` checks read the third cell, so the shape counts.
ROW = "| `MB-001` | `INV` | The bus MUST drop the frame. | derived | none | none | verified |"


class Classify(unittest.TestCase):
    """The partition every share and every budget is built on."""

    def test_a_table_row_is_structure(self):
        shape = spec_shape.classify(doc(ROW))
        self.assertEqual(0, shape.prose_lines)
        self.assertEqual(1, shape.structure_lines)

    def test_a_wrapped_list_item_is_one_item_not_a_wall(self):
        """The first version read a continuation as prose, so a tight list
        measured as a wall on the meter while reading fine on the page."""
        shape = spec_shape.classify(doc("* the caret follows the gesture,", "  and stops at the margin"))
        self.assertEqual(0, shape.prose_lines)
        self.assertEqual(2, shape.structure_lines)

    def test_a_blank_line_ends_the_list(self):
        shape = spec_shape.classify(doc("* the caret follows", "", "the margin holds it"))
        self.assertEqual(1, shape.prose_lines)
        self.assertEqual(1, shape.structure_lines)

    def test_a_fence_counts_whole_markers_included(self):
        shape = spec_shape.classify(doc("```rust", "let caret = 0;", "```"))
        self.assertEqual(3, shape.fenced_lines)
        self.assertEqual(0, shape.prose_lines)

    def test_a_heading_is_navigation_and_is_skipped(self):
        shape = spec_shape.classify(doc("## 1. THE BUS", "", "The bus drops the frame."))
        self.assertEqual(1, shape.prose_lines)

    def test_a_separator_separates_and_emphasis_does_not(self):
        """`|a|b|` is two words; `**bold**word` stays one."""
        self.assertEqual(2, spec_shape.count_words("|a|b|"))
        self.assertEqual(1, spec_shape.count_words("**bold**word"))


class ProseShare(unittest.TestCase):
    def test_the_share_counts_words_not_lines(self):
        """One dense table row against one short prose line: on lines the
        document is half prose, on words it is far less."""
        text = doc(ROW, "It drops.")
        self.assertLess(spec_shape.prose_share(text), 25.0)

    def test_a_document_with_no_content_is_not_a_division_by_zero(self):
        self.assertEqual(0.0, spec_shape.prose_share(doc("## 1. EMPTY")))


class LongLines(unittest.TestCase):
    def test_a_wrapped_paragraph_line_is_not_long(self):
        self.assertEqual(0, spec_shape.long_lines(doc("x" * spec_shape.LONG_LINE_COLS)))

    def test_one_column_over_is_long(self):
        self.assertEqual(1, spec_shape.long_lines(doc("x" * (spec_shape.LONG_LINE_COLS + 1))))

    def test_a_table_row_cannot_be_wrapped_so_it_is_skipped(self):
        self.assertEqual(0, spec_shape.long_lines(doc("| " + "x" * 200 + " |")))

    def test_a_fenced_line_cannot_be_wrapped_so_it_is_skipped(self):
        self.assertEqual(0, spec_shape.long_lines(doc("```", "x" * 200, "```")))


class FeatureMarkers(unittest.TestCase):
    def test_a_checkbox_is_the_roadmap(self):
        self.assertIn("checkbox", reasons(spec_shape.feature_issues(doc("* [ ] wire the caret")))[0])

    def test_a_milestone_word_is_refused(self):
        self.assertIn("milestone", reasons(spec_shape.feature_issues(doc("Shipped for MVP.")))[0])

    def test_a_phase_is_refused(self):
        self.assertIn("phase", reasons(spec_shape.feature_issues(doc("Phase 2 adds the caret.")))[0])

    def test_a_table_row_may_quote_a_marker(self):
        """The contract grammar names the Story an `unimplemented` row is bound
        to, and a Story cell says `MVP` legitimately."""
        self.assertEqual([], list(spec_shape.feature_issues(doc("| `MB-001` | ... | MVP |"))))

    def test_a_past_tense_about_the_system_is_not_history(self):
        """The regression this file exists for. Each of the three sentences was
        refused as production history by a marker that read a verb and guessed
        the subject."""
        for line in (
            "The codec used to encode the frame is the one the schema names.",
            "The row landed in the table the reader opens first.",
            "A pass was fixed to the viewport, and the caret followed it.",
        ):
            with self.subTest(line=line):
                self.assertEqual([], list(spec_shape.feature_issues(doc(line))))


class Header(unittest.TestCase):
    def test_a_document_with_no_implementation_field_is_refused(self):
        self.assertEqual(["header"], kinds(spec_shape.header_issues(doc("**Type:** feature"))))

    def test_a_fourth_implementation_value_is_refused(self):
        text = doc("**Type:** feature", "**Implementation:** shipped")
        self.assertIn("expected implemented", "".join(d for _, d in spec_shape.header_issues(text)))

    def test_a_trailing_period_is_a_fourth_spelling(self):
        """A word boundary let `implemented.` through, which is the `Kind`
        column's story again: three values read, four spellings carried."""
        text = doc("**Type:** feature", "**Implementation:** implemented.")
        self.assertEqual(["header"], kinds(spec_shape.header_issues(text)))

    def test_a_clause_after_the_value_is_allowed(self):
        text = doc("**Type:** feature", "**Implementation:** partial -- the file round trip is not built")
        self.assertEqual([], list(spec_shape.header_issues(text)))

    def test_the_field_is_read_in_the_first_eight_lines_only(self):
        text = doc("**Type:** feature", *([""] * 8), "**Implementation:** implemented")
        self.assertEqual(["header"], kinds(spec_shape.header_issues(text)))


class Answers(unittest.TestCase):
    def test_a_document_with_no_contract_row_owes_nothing(self):
        self.assertEqual([], list(spec_shape.answer_issues(doc("The bus drops the frame."))))

    def test_a_missing_bullet_is_refused(self):
        text = doc(ROW, "", "* **Budget.** one frame", "* **Boundary.** none, the relay never reads it")
        self.assertEqual(1, len(list(spec_shape.answer_issues(text))))

    def test_an_empty_bullet_is_refused(self):
        text = doc(
            ROW,
            "",
            "* **Budget.** one frame",
            "* **Boundary.** none, the relay never reads it",
            "* **Verification.**",
        )
        self.assertIn("empty", "".join(d for _, d in spec_shape.answer_issues(text)))

    def test_a_label_inside_a_table_cell_is_not_an_answer(self):
        """The template names all three labels to define them, in cells."""
        text = doc(ROW, "| rule | **Budget.** what it costs |")
        self.assertEqual(3, len(list(spec_shape.answer_issues(text))))

    def test_three_non_empty_bullets_pass(self):
        text = doc(
            ROW,
            "",
            "* **Budget.** one frame per gesture",
            "* **Boundary.** none, the relay never reads it",
            "* **Verification.** 1 verified, 0 failing",
        )
        self.assertEqual([], list(spec_shape.answer_issues(text)))


class LowerCaseKeyword(unittest.TestCase):
    def test_a_lower_case_modality_in_a_rule_is_refused(self):
        row = "| `MB-001` | `INV` | The bus may drop the frame. | derived | none | none | verified |"
        self.assertIn('"may"', reasons(spec_shape.keyword_issues(doc(row)))[0])

    def test_a_code_span_quotes_the_word_rather_than_using_it(self):
        row = "| `MB-001` | `INV` | A `may` is not a modality here. | derived | none | none | verified |"
        self.assertEqual([], list(spec_shape.keyword_issues(doc(row))))

    def test_prose_outside_a_row_uses_may_correctly(self):
        """The scope is what makes the measure exact: descriptive prose is not
        read at all."""
        self.assertEqual([], list(spec_shape.keyword_issues(doc("A reader may stop after the tables."))))

    def test_the_keyword_is_read_in_the_statement_cell_only(self):
        """A `Gap` cell writes English about what nothing holds, and `may`
        there is the word, not a modality."""
        row = "| `MB-001` | `INV` | The bus MUST drop it. | derived | none | a reader may miss it | verified |"
        self.assertEqual([], list(spec_shape.keyword_issues(doc(row))))


class Schedule(unittest.TestCase):
    def test_a_checkbox_list_is_a_schedule(self):
        self.assertIn("checkbox", reasons(spec_shape.schedule_issues(doc("- [x] the caret lands")))[0])

    def test_a_roadmap_heading_is_a_schedule(self):
        self.assertIn("roadmap", reasons(spec_shape.schedule_issues(doc("## 3. BACKLOG")))[0])

    def test_a_table_cell_may_quote_the_syntax_it_maps(self):
        """Prose's Markdown mapping names the GFM checkbox in a cell, and the
        roadmap's own rule is quoted in a Definition-of-Done row."""
        self.assertEqual([], list(spec_shape.schedule_issues(doc("| `- [ ]` | a task item |"))))


class SharedFacts(unittest.TestCase):
    def test_a_quality_section_is_the_method_growing_back(self):
        issues = list(spec_shape.method_issues(doc("## 4. QUALITY"), "1-Foundations/ambre-spec-frame.md"))
        self.assertIn("quality section", issues[0][1])

    def test_the_owner_carries_the_fact_legitimately(self):
        """A marker is a defect everywhere except in the document that owns it:
        the budget register is the one place a performance section belongs."""
        text = doc("## 2. PERFORMANCE")
        self.assertNotEqual([], list(spec_shape.method_issues(text, "1-Foundations/ambre-spec-frame.md")))
        self.assertEqual([], list(spec_shape.method_issues(text, spec_shape.BUDGETS)))

    def test_the_marker_is_read_on_the_title_never_on_a_number(self):
        """The instances were numbered 3 to 15 or not at all, so a number-based
        ban would have caught almost none."""
        issues = list(spec_shape.method_issues(doc("### QUALITY GATES"), "1-Foundations/ambre-spec-frame.md"))
        self.assertEqual(1, len(issues))

    def test_naming_the_ban_is_not_breaking_it(self):
        text = doc("| a quality section | **Definition of Done** | banned |")
        self.assertEqual([], list(spec_shape.method_issues(text, "1-Foundations/ambre-spec-frame.md")))


class Numbering(unittest.TestCase):
    def test_a_sequence_from_one_passes(self):
        text = doc("## 1. ONE", "## 2. TWO", "## 3. THREE")
        self.assertEqual([], list(spec_shape.numbering_issues(text)))

    def test_a_sequence_from_zero_passes(self):
        text = doc("## 0. ZERO", "## 1. ONE")
        self.assertEqual([], list(spec_shape.numbering_issues(text)))

    def test_a_hole_is_what_a_deletion_leaves(self):
        text = doc("## 1. ONE", "## 2. TWO", "## 6. SIX")
        self.assertIn("section 6 follows section 2", reasons(spec_shape.numbering_issues(text))[0])

    def test_bis_is_not_a_number(self):
        text = doc("## 1. ONE", "## 1 bis. INSERTED", "## 2. TWO")
        self.assertIn("`bis`", reasons(spec_shape.numbering_issues(text))[0])

    def test_a_first_section_that_is_neither_zero_nor_one_is_refused(self):
        self.assertIn("the first section is 4", reasons(spec_shape.numbering_issues(doc("## 4. FOUR")))[0])


class Schema(unittest.TestCase):
    """The schema document keeps two budgets and one comparison with the code."""

    def source(self, code: str) -> Path:
        handle = tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False)
        handle.write(code)
        handle.close()
        path = Path(handle.name)
        self.addCleanup(path.unlink)
        return path

    def test_a_container_built_in_the_code_and_absent_from_the_table_is_a_lie(self):
        issues = list(spec_shape.schema_issues(doc("## 1. CONTAINERS"), self.source(
            'const SHEETS_MAP: &str = "sheets";'
        )))
        self.assertEqual(["container-missing"], kinds(issues))

    def test_a_declared_container_with_no_code_is_normal(self):
        """Asymmetric on purpose: that is spec-driven development working."""
        text = doc("## 1. CONTAINERS", "| `sheets` | a map |")
        self.assertEqual([], list(spec_shape.schema_issues(text, self.source(""))))

    def test_a_nested_container_is_compared_on_its_leaf(self):
        text = doc("## 1. CONTAINERS", "| `elements/{id}/children` | a list |")
        code = 'const CHILDREN: &str = "children";\nlet c = doc.get_list(CHILDREN);'
        self.assertEqual([], list(spec_shape.schema_issues(text, self.source(code))))

    def test_a_constant_no_call_opens_is_not_a_container(self):
        """`const MATERIALIZE_ORIGIN: &str = "mat";` is a commit tag."""
        code = 'const MATERIALIZE_ORIGIN: &str = "mat";'
        self.assertEqual([], list(spec_shape.schema_issues(doc("## 1. CONTAINERS"), self.source(code))))

    def test_the_prose_budget_is_a_hard_cap(self):
        text = doc("## 1. CONTAINERS", "word " * (spec_shape.SCHEMA_PROSE_WORDS_MAX + 1))
        self.assertIn("prose-budget", kinds(spec_shape.schema_issues(text, self.source(""))))

    def test_a_container_note_over_budget_is_refused(self):
        text = doc(
            "## 2. CONTAINER NOTES",
            "",
            "* **sheets.** " + "word " * (spec_shape.SCHEMA_NOTE_WORDS_MAX + 1),
        )
        self.assertIn("note-budget", kinds(spec_shape.schema_issues(text, self.source(""))))

    def test_a_missing_source_is_reported_rather_than_crashed_on(self):
        issues = list(spec_shape.schema_issues(doc("## 1. CONTAINERS"), Path("/nonexistent/schema.rs")))
        self.assertEqual(["source-missing"], kinds(issues))


if __name__ == "__main__":
    unittest.main()
