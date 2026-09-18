#!/usr/bin/env python3
"""What `prose.py` reads, and what it refuses.

The gate guards every Rust comment and every document of the corpus, and
nothing guarded the gate. Three defects shipped in it, found by accident while
fixing a fourth, and the worst of the three produced no symptom at all: a table
row left the result entirely, so the hard rules never read it and 23 forbidden
synonyms sat in cells for as long as the check existed. A silent gate and a
satisfied gate read the same from outside, which is the case for these tests.

Each of the three is frozen below under the behaviour it proves, the way
`ai/ambre-testing.md` asks. The rest hold the reader itself: a fence, a
heading, a bullet and a blank line each decide where a paragraph ends, and a
budget is only worth what the split under it is worth.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import prose  # noqa: E402  -- the module under test


def rust(*lines: str) -> list[tuple[int, str, bool, bool]]:
    return prose.rust_comments("\n".join(lines) + "\n")


def markdown(*lines: str) -> list[tuple[int, str, bool, bool]]:
    return prose.markdown_prose("\n".join(lines) + "\n")


def split(lines: list[tuple[int, str, bool, bool]]) -> list[list[str]]:
    """One list of sentences per paragraph, which is what the budgets count."""
    return [prose.sentences(text) for _, text in prose.paragraphs(lines)]


TABLE = (
    "| Flag | Meaning |",
    "| :--- | :------ |",
    "| `-u` | how many simulated users the run spawns, one thousand by default |",
    "| `-r` | how fast they arrive, counted in users per second, fifty of them |",
)


class TableRowIsStructure(unittest.TestCase):
    """A table is a shape, and joining its cells invents a sentence.

    `markdown_prose` skipped a table row and `rust_comments` did not, which the
    docstring of the second forbids in writing: a doc comment is Markdown, so
    the two readers must agree. A four-row table in a `//!` block read as one
    sentence of about forty words, which no rewrite can shorten because nobody
    wrote it.
    """

    def test_a_table_in_a_doc_comment_is_no_sentence(self):
        lines = rust("//! Prose above.", "//!", *(f"//! {row}" for row in TABLE))
        self.assertEqual(split(lines), [["Prose above."]])

    def test_a_table_in_a_document_is_no_sentence(self):
        self.assertEqual(split(markdown("Prose above.", "", *TABLE)), [["Prose above."]])

    def test_a_table_ends_the_paragraph_above_it(self):
        lines = rust("//! Prose above.", *(f"//! {row}" for row in TABLE), "//! Prose below.")
        self.assertEqual(split(lines), [["Prose above."], ["Prose below."]])

    def test_the_two_readers_agree_on_a_table(self):
        document = split(markdown("Prose above.", "", *TABLE, "", "Prose below."))
        comment = rust("//! Prose above.", "//!", *(f"//! {r} " for r in TABLE), "//!", "//! Prose below.")
        self.assertEqual(document, split(comment))


class HardRulesReadATableCell(unittest.TestCase):
    """The defect that showed nothing: a skipped row is an unread row.

    Most of the corpus is written in tables, so a rule the cells escaped was a
    rule holding a minority of what it claimed. The row now leaves the
    paragraph and stays in the result, as every other boundary line does.
    """

    def test_a_forbidden_term_in_a_cell_is_a_breach(self):
        lines = markdown("| Case | What it does |", "| stale | the answer is discarded by version |")
        self.assertTrue(prose.term_breaches(lines))

    def test_a_forbidden_term_in_a_cell_of_a_doc_comment_is_a_breach(self):
        lines = rust("//! | Case | What it does |", "//! | stale | the answer is discarded |")
        self.assertTrue(prose.term_breaches(lines))


class AHyphenBindsAsTightlyAsALetter(unittest.TestCase):
    """`cargo-deny` is a tool, and the list forbids a verb.

    The word boundary around a forbidden term treated the hyphen as a break, so
    the name of the tool read as the verb it contains, fourteen times over, in
    the `Verification` column of the contract tables.
    """

    def test_a_hyphenated_name_is_not_the_verb_it_contains(self):
        self.assertEqual(prose.term_breaches([(1, "checked by cargo-deny in CI", False, True)]), [])

    def test_the_bare_verb_is_still_a_breach(self):
        self.assertTrue(prose.term_breaches([(1, "it exists to deny it", False, True)]))


class ParagraphBoundaries(unittest.TestCase):
    """Where one paragraph ends, since every budget is counted per paragraph.

    A budget of six sentences is worth exactly what the split under it is
    worth. Merging two paragraphs spends one budget on two ideas, and an empty
    `///` line once made a whole rustdoc block a single paragraph.
    """

    def test_an_empty_comment_line_ends_a_paragraph(self):
        self.assertEqual(split(rust("/// One.", "///", "/// Two.")), [["One."], ["Two."]])

    def test_a_bullet_opens_a_paragraph(self):
        self.assertEqual(split(rust("/// Lead.", "/// * One.", "/// * Two.")),
                         [["Lead."], ["One."], ["Two."]])

    def test_a_heading_is_no_sentence_of_the_text_under_it(self):
        self.assertEqual(split(markdown("# Title", "", "The text.")), [["The text."]])

    def test_a_blank_line_ends_a_paragraph(self):
        self.assertEqual(split(markdown("One.", "", "Two.")), [["One."], ["Two."]])

    def test_a_fenced_block_carries_no_english(self):
        lines = rust("/// Prose.", "/// ```", "/// let x = 1; // a comment nobody measures", "/// ```")
        self.assertEqual(split(lines), [["Prose."]])


class CountedMetrics(unittest.TestCase):
    """The three counts, each at its exact boundary.

    A budget is a number, so the test that matters is the one either side of
    it: 25 words pass and 26 do not, 6 sentences pass and 7 do not.
    """

    def test_a_sentence_of_the_budget_passes_and_one_word_over_fails(self):
        at = "word " * (prose.SENTENCE_WORDS_MAX - 1) + "end."
        self.assertEqual(prose.measure(at, rust(f"/// {at}"))[0]["long"], 0)
        over = "word " * prose.SENTENCE_WORDS_MAX + "end."
        self.assertEqual(prose.measure(over, rust(f"/// {over}"))[0]["long"], 1)

    def test_a_paragraph_of_the_budget_passes_and_one_sentence_over_fails(self):
        at = " ".join(f"Sentence {i}." for i in range(prose.PARAGRAPH_SENTENCES_MAX))
        self.assertEqual(prose.measure(at, rust(f"/// {at}"))[0]["para"], 0)
        over = " ".join(f"Sentence {i}." for i in range(prose.PARAGRAPH_SENTENCES_MAX + 1))
        self.assertEqual(prose.measure(over, rust(f"/// {over}"))[0]["para"], 1)

    def test_a_latin_abbreviation_is_a_breach(self):
        text = "/// The rule holds, e.g. for a merge."
        self.assertEqual(prose.measure(text, rust(text))[0]["latin"], 1)


class CodeIsNotEnglish(unittest.TestCase):
    """What `clean` drops before a word is counted or matched.

    A code span is a name, not prose: measuring its words counts the author's
    identifiers against a sentence budget, and matching them against the
    glossary reads a field name as a forbidden synonym.
    """

    def test_a_code_span_counts_as_one_word(self):
        self.assertEqual(len(prose.clean("`a::very::long::path` ends it.").split()), 3)

    def test_a_forbidden_term_inside_a_code_span_is_no_breach(self):
        self.assertEqual(prose.term_breaches([(1, "the field `discarded_at` holds it", False, True)]), [])

    def test_a_comment_inside_a_string_literal_is_code(self):
        self.assertEqual(rust('let url = "https://example.test/a";'), [])


if __name__ == "__main__":
    unittest.main()
