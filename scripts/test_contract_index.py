#!/usr/bin/env python3
"""What `contract_index.py` emits, and what it refuses.

This script re-reads the corpus with a second reader, which is the defect it is
most likely to carry: a walk that drifts from the gate's reads different rows
from the ones every check holds, and both files look right. So the first test
below counts the rows against the ledger, which is written by the gate itself.

The rest hold the two decisions the artefacts rest on. No record carries a line
number, because a line is the coordinate an edit above it breaks. And a
`derived` source yields no edge, because a row that claims no source asserts no
join.

The last class holds where the two files go: under a path git ignores, built
only by `--write`, so no commit can carry them again.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import io
import json
import re
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import contract_index  # noqa: E402  -- the module under test
import contracts  # noqa: E402  -- whose row count the index must match


# The one sentence of the ledger that counts the corpus whole.
LEDGER_TOTAL = re.compile(r"^\*\*(\d+) rows:")


def rows(*cells: str) -> list[dict]:
    """One record per `Sources` cell, everything else held fixed."""
    return [
        contract_index.record(
            "1-Foundations/ambre-spec-frame.md",
            f"FRAME-{at:03}",
            "verified",
            {"Kind": "`INV`", "Statement": "The frame MUST land.", "Sources": cell,
             "Mechanism": "", "Gap": "none"},
        )
        for at, cell in enumerate(cells, 1)
    ]


class RowCount(unittest.TestCase):
    """The second reader must read what the first one does."""

    def test_the_index_counts_the_rows_the_ledger_counts(self):
        """The ledger is written by `contracts.py` from its own walk. A count
        that disagrees means one of the two readers is closing a table where
        the other is not, and the rows between them are held by nothing."""
        ledger = (contracts.SPECS / "1-Foundations" / "ambre-contract-ledger.md")
        totals = [int(m.group(1)) for line in ledger.read_text().splitlines()
                  if (m := LEDGER_TOTAL.match(line))]
        self.assertEqual(1, len(totals), "the ledger states one total")
        self.assertEqual(totals[0], len(contract_index.index()))

    def test_the_template_is_not_indexed(self):
        """Its example rows are not contracts, and indexing them would bind the
        corpus to ids no document declares."""
        documents = {row["document"] for row in contract_index.index()}
        self.assertNotIn(contracts.TEMPLATE, documents)


class Records(unittest.TestCase):
    """One row, as written, with its sources taken apart."""

    def test_a_record_carries_no_line_number(self):
        """A line is the one part of a citation an edit above it breaks, which
        is what the template refuses for `crates/`. A document and an id locate
        a row, and an id is unique corpus-wide."""
        self.assertNotIn("line", rows("this spec section 3")[0])

    def test_the_kind_is_unwrapped(self):
        self.assertEqual("INV", rows("derived")[0]["kind"])

    def test_the_sources_are_parsed(self):
        found = rows("`ambre-spec-ui.md section 2` (`UI-THREAD-001`)")[0]["sources"]
        self.assertEqual(
            [{"kind": "doc", "target": "ambre-spec-ui.md", "locator": "2",
              "rules": ["UI-THREAD-001"], "note": ""}],
            found,
        )


class Edges(unittest.TestCase):
    """The join the corpus asserts, one edge per citation."""

    def edges(self, *cells: str) -> list[dict]:
        return contract_index.edges(rows(*cells))

    def test_a_section_of_the_citing_document_points_at_it(self):
        self.assertEqual(
            [{"from": "FRAME-001", "edge": "cites-section", "section": "3",
              "to": "1-Foundations/ambre-spec-frame.md"}],
            self.edges("this spec section 3"),
        )

    def test_a_document_citation_carries_its_section(self):
        found = self.edges("`ambre-spec-ui.md section 2`")
        self.assertEqual("2", found[0]["section"])

    def test_a_document_citation_without_a_section_carries_none(self):
        """An edge whose `section` is absent and one whose `section` is empty
        are two shapes for one fact, and a consumer would have to read both."""
        self.assertNotIn("section", self.edges("`ambre-spec-ui.md`")[0])

    def test_a_rule_citation_is_its_own_edge(self):
        found = self.edges("`ambre-spec-ui.md`, `UI-THREAD-001`")
        self.assertIn(
            {"from": "FRAME-001", "edge": "cites-rule", "to": "UI-THREAD-001"}, found
        )

    def test_two_ids_under_one_document_are_two_edges(self):
        found = self.edges("`ambre-spec-ui.md` (`UI-THREAD-001`, `UI-THREAD-050`)")
        self.assertEqual(2, len([e for e in found if e["edge"] == "cites-rule"]))

    def test_an_audit_source_naming_two_findings_is_two_edges(self):
        found = self.edges("audit `H9`, `H12`")
        self.assertEqual(["H9", "H12"], [e["to"] for e in found])

    def test_a_derived_source_asserts_nothing(self):
        self.assertEqual([], self.edges("derived"))

    def test_a_pillar_is_an_edge(self):
        self.assertEqual(
            [{"from": "FRAME-001", "edge": "cites-pillar", "to": "Zero-Knowledge"}],
            self.edges('README pillar "Zero-Knowledge"'),
        )

    def test_every_pillar_the_corpus_cites_is_declared(self):
        """The one source form nothing resolved before this file."""
        declared = contract_index.declared_pillars()
        for name in ("Zero-Knowledge", "E2EE", "Security"):
            with self.subTest(pillar=name):
                self.assertIn(name, declared)


class Resolution(unittest.TestCase):
    """An edge a consumer cannot follow is worse than no graph at all."""

    def defects(self, *cells: str) -> list[str]:
        records = rows(*cells)
        return contract_index.edge_defects(records, contract_index.edges(records))

    def test_a_rule_no_row_declares_fails(self):
        self.assertEqual(1, len(self.defects("`ZK-999`")))

    def test_a_document_no_file_carries_fails(self):
        self.assertEqual(1, len(self.defects("`ambre-spec-gone.md`")))

    def test_a_section_the_document_lacks_fails(self):
        self.assertEqual(1, len(self.defects("`ambre-spec-ui.md section 44`")))

    def test_a_pillar_the_readme_lacks_fails(self):
        self.assertEqual(1, len(self.defects('README pillar "Local-First"')))

    def test_a_finding_the_audit_lacks_fails(self):
        self.assertEqual(1, len(self.defects("audit `H99`")))

    def test_a_short_name_resolves(self):
        """`architecture section 2.D` is 20 sources of the corpus, and the
        letter is not checked, exactly as `spec_refs.py` reads it."""
        self.assertEqual([], self.defects("architecture section 2.D"))

    def test_a_path_resolves_to_the_bare_name(self):
        self.assertEqual([], self.defects("`1-Foundations/ambre-spec-ui.md section 2`"))


class Render(unittest.TestCase):
    """One record per line, inside one JSON array."""

    def test_the_output_is_valid_json(self):
        self.assertEqual([{"a": 1}], json.loads(contract_index.render([{"a": 1}])))

    def test_one_record_is_one_line(self):
        """A row edited by one word must diff as one line. An indented dump
        spreads it over fifteen, and a single-line dump makes every edit the
        whole file."""
        text = contract_index.render([{"a": 1}, {"b": 2}])
        self.assertEqual(4, len(text.splitlines()))

    def test_the_keys_are_sorted(self):
        """The dict order is the code's; a reordered literal would rewrite the
        whole artefact and diff as a corpus change."""
        self.assertEqual('[\n  {"a": 1, "b": 2}\n]\n',
                         contract_index.render([{"b": 2, "a": 1}]))



class BuiltOnDemand(unittest.TestCase):
    """The two files are build outputs: ignored by git, and written by `--write` only."""

    def test_both_files_land_in_a_path_git_ignores(self):
        for path in (contract_index.INDEX, contract_index.GRAPH):
            with self.subTest(file=path.name):
                run = subprocess.run(["git", "check-ignore", "-q", "--no-index", str(path)],
                                     cwd=contracts.ROOT)
                self.assertEqual(0, run.returncode, f"{path} is not ignored")

    def test_no_file_of_the_index_is_tracked(self):
        tracked = subprocess.run(["git", "ls-files", "*ambre-contract-index.json",
                                  "*ambre-contract-graph.json"], cwd=contracts.ROOT,
                                 capture_output=True, text=True, check=True).stdout
        self.assertEqual("", tracked)

    def test_write_builds_both_files_from_the_corpus(self):
        records, found = contract_index.build()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "nested"
            index, graph = out / "index.json", out / "graph.json"
            with mock.patch.object(contract_index, "INDEX", index), \
                    mock.patch.object(contract_index, "GRAPH", graph), \
                    mock.patch.object(sys, "argv", ["contract_index.py", "--write"]), \
                    redirect_stdout(io.StringIO()):
                self.assertEqual(0, contract_index.main())
            self.assertEqual(records, json.loads(index.read_text()))
            self.assertEqual(found, json.loads(graph.read_text()))


if __name__ == "__main__":
    unittest.main()
