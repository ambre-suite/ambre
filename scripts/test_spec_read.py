#!/usr/bin/env python3
"""What `spec_read.py` prints, and what it refuses.

A reading view is read and never checked, so nothing in CI notices when it
stops printing a row: the output still looks like a contract, one rule shorter.
The cases below are the four shapes the corpus has -- a feature document's
subsections, a schema document's plain sections, a wrapped header field, and a
document with no contract at all -- and each is frozen here because a reader
cannot tell a dropped row from a document that never declared it.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import spec_read  # noqa: E402  -- the module under test

HEAD = "| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |"
RULE = "|---|---|---|---|---|---|---|"


def row(rid: str, kind: str, statement: str) -> str:
    return f"| `{rid}` | `{kind}` | {statement} | this spec | none | none | verified |"


class Opening(unittest.TestCase):
    def test_the_title_the_fields_and_the_summary_are_kept(self):
        text = "\n".join(
            [
                "# AMBRE -- FEATURE: A THING",
                "",
                "**Type:** Horizontal (framework)",
                "**Applications:** Prose, Trame",
                "",
                "> What the thing does.",
                "",
                "A paragraph the tables restate.",
                "",
                "---",
            ]
        )
        self.assertEqual(
            spec_read.opening(text),
            [
                "# AMBRE -- FEATURE: A THING",
                "**Type:** Horizontal (framework)",
                "**Applications:** Prose, Trame",
                "> What the thing does.",
            ],
        )

    def test_a_wrapped_field_keeps_its_second_line(self):
        text = "\n".join(
            [
                "# AMBRE -- FEATURE: A THING",
                "**Type:** Horizontal (framework)",
                "**Applications:** Prose, Trame, Toile and every application that",
                "embeds the shared toolkit.",
                "",
                "> What the thing would do.",
                "",
                "---",
            ]
        )
        self.assertIn("embeds the shared toolkit.", spec_read.opening(text))

    def test_nothing_after_the_first_rule_is_kept(self):
        text = "\n".join(["# A", "", "---", "", "> a quote under the rule"])
        self.assertEqual(spec_read.opening(text), ["# A"])


class ReadingView(unittest.TestCase):
    def test_a_row_keeps_three_cells_of_seven(self):
        text = "\n".join(
            ["# A", "", "### 1.1 The state", "", HEAD, RULE, row("A-001", "INV", "It MUST hold.")]
        )
        self.assertIn("| `A-001` | `INV` | It MUST hold. |", spec_read.reading_view(text))

    def test_each_subsection_heads_its_own_table(self):
        text = "\n".join(
            [
                "# A",
                "",
                "### 1.1 The state",
                "",
                HEAD,
                RULE,
                row("A-001", "INV", "It MUST hold."),
                "",
                "### 1.2 Do the thing",
                "",
                HEAD,
                RULE,
                row("A-002", "POST", "It MUST land."),
            ]
        )
        view = spec_read.reading_view(text)
        self.assertEqual(view.count("| ID | Kind | Statement |"), 2)
        self.assertIn("### 1.2 Do the thing", view)

    def test_a_schema_section_heads_its_table_too(self):
        """A schema document declares its rows under `##`, with no subsection."""
        text = "\n".join(["# A", "", "## 1. INVARIANTS", "", HEAD, RULE, row("A-001", "INV", "It MUST hold.")])
        view = spec_read.reading_view(text)
        self.assertIn("## 1. INVARIANTS", view)
        self.assertIn("| ID | Kind | Statement |", view)

    def test_a_table_that_is_not_a_contract_is_not_read(self):
        text = "\n".join(
            [
                "# A",
                "",
                "## 4. SURFACE",
                "",
                "| Trigger | Behaviour |",
                "|---|---|",
                "| a click | it happens (`A-001`) |",
            ]
        )
        self.assertNotIn("| ID | Kind | Statement |", spec_read.reading_view(text))


class Resolve(unittest.TestCase):
    def test_a_bare_name_resolves_under_specs(self):
        self.assertEqual(
            spec_read.resolve("ambre-spec-locale.md").name, "ambre-spec-locale.md"
        )

    def test_a_name_no_document_carries_is_refused(self):
        with self.assertRaises(FileNotFoundError):
            spec_read.resolve("ambre-spec-nothing-here.md")


if __name__ == "__main__":
    unittest.main()
