#!/usr/bin/env python3
"""What `spec_answers.py` reads, and what it refuses.

Two caps, and both are counts a reader can redo by hand: the words the three
answers spend, and the lines a document spends writing its own history. The
first is arithmetic and the risk is in the split -- which lines belong to an
answer. The second is a marker list, and a marker list is where this suite has
already been wrong: three patterns read a verb without its subject and refused
prose about the system.

The tests below hold both halves. The accepting half is the load-bearing one.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_answers  # noqa: E402  -- the module under test
import template_vocabulary  # noqa: E402  -- the labels it reads from the template


def doc(*lines: str) -> str:
    return "\n".join(lines) + "\n"


def reasons(text: str) -> list[str]:
    return [reason for _, reason in spec_answers.history_lines(text)]


class AnswerWords(unittest.TestCase):
    """The words the three bullets spend, which is one paragraph to a reader."""

    def test_the_three_bullets_are_counted_together(self):
        text = doc(
            "* **Budget.** one frame",
            "* **Boundary.** none here",
            "* **Verification.** two rows",
        )
        self.assertEqual(12, spec_answers.answer_words(text))

    def test_a_wrapped_answer_is_the_same_answer(self):
        """The template wraps a long answer under an indent, and the second
        line is the same bullet."""
        text = doc("* **Budget.** one frame", "  per gesture, never two")
        self.assertEqual(8, spec_answers.answer_words(text))

    def test_a_blank_line_ends_the_answer(self):
        """An answer that needs a second paragraph has already lost, so the
        paragraph after it is not charged to it."""
        text = doc("* **Budget.** one frame", "", "The bus drops the rest.")
        self.assertEqual(4, spec_answers.answer_words(text))

    def test_prose_outside_the_bullets_costs_nothing(self):
        text = doc("The bus drops the frame it cannot place.", "", "## 2. NEXT")
        self.assertEqual(0, spec_answers.answer_words(text))

    def test_a_bullet_that_is_not_one_of_the_three_is_not_an_answer(self):
        self.assertEqual(0, spec_answers.answer_words(doc("* **Scope.** the bus alone")))

    def test_the_three_are_the_templates_own(self):
        """A second list here is a list that drifts: the table states them."""
        for label in template_vocabulary.ANSWER_LABELS:
            with self.subTest(label=label):
                self.assertTrue(spec_answers.ANSWER.match(f"* **{label}.** none"))

    def test_the_cap_is_a_refusal_not_a_target(self):
        """`ANSWER_WORDS_TARGET` is what the template states and what the gate
        refuses, one figure for the whole corpus."""
        self.assertEqual(spec_answers.ANSWER_WORDS_TARGET, spec_answers.CAPS["answer_words"])
        self.assertEqual(0, spec_answers.CAPS["history"])


class History(unittest.TestCase):
    """A line writing the history of the document itself is a commit message."""

    def test_a_pass_that_did_something_to_this_document_is_refused(self):
        self.assertIn("commit message", reasons(doc("This pass added four rows."))[0])

    def test_a_row_that_came_apart_is_refused(self):
        self.assertIn("split", reasons(doc("The row came apart into two."))[0])

    def test_a_row_that_was_carrying_a_half_is_refused(self):
        self.assertIn("carried", reasons(doc("The row was carrying both halves."))[0])

    def test_a_row_that_is_new_here_is_refused(self):
        self.assertIn("git says", reasons(doc("Two rows are new."))[0])

    def test_a_pass_that_runs_is_the_system(self):
        """Each pattern names a verb as well as a subject, so `the pass added`
        is caught and `the settling pass runs` is not."""
        self.assertEqual([], reasons(doc("The settling pass runs before the frame lands.")))

    def test_a_past_tense_about_the_system_is_not_history(self):
        """The regression this file exists for: `landed`, `used to` and
        `was fixed` read a verb and guessed its subject."""
        for line in (
            "The codec used to encode the frame is the one the schema names.",
            "The row landed in the table the reader opens first.",
            "A pass was fixed to the viewport, and the caret followed it.",
        ):
            with self.subTest(line=line):
                self.assertEqual([], reasons(doc(line)))

    def test_a_target_that_no_longer_exists_is_the_system(self):
        """A wide net over `no longer` matched 80 lines of which a handful were
        history, which is why no marker reads it."""
        self.assertEqual([], reasons(doc("A target that no longer exists is dropped.")))

    def test_a_table_cell_states_a_gap_which_is_what_a_row_is_for(self):
        self.assertEqual([], reasons(doc("| `MB-001` | unimplemented | this pass added it | `E9` |")))

    def test_a_fenced_block_is_code_not_prose(self):
        self.assertEqual([], reasons(doc("```", "// this pass added the caret", "```")))

    def test_the_document_stating_the_rule_has_to_spell_the_markers_out(self):
        """Exempt from `history` alone: the template's answers are measured
        like any other's."""
        self.assertIn("ambre-spec-template.md", spec_answers.HISTORY_EXEMPT)


if __name__ == "__main__":
    unittest.main()
