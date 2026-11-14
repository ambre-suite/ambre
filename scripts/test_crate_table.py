#!/usr/bin/env python3
"""What `crate_table.py` refuses, on manifests and tables written by hand, and that the tree passes.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import crate_table as gate  # noqa: E402  -- the module under test


def dep(name: str, kind: str | None = None, optional: bool = False, target: str | None = None) -> dict:
    return {"name": name, "kind": kind, "optional": optional, "target": target}


def meta(packages: dict[str, list[dict]]) -> dict:
    """A `cargo metadata` answer, each package a workspace member."""
    listed = [{"id": f"{name} 0.1.0", "name": name, "dependencies": deps} for name, deps in packages.items()]
    return {"packages": listed, "workspace_members": [p["id"] for p in listed]}


# A workspace of four members, one edge of each shape.
GRAPH = gate.graph(meta({
    "ambre-core": [dep("serde")],
    "ambre-crypto": [dep("ambre-core"), dep("ambre-core", kind="dev")],
    "ambre-logic": [
        dep("ambre-core"),
        dep("ambre-crypto", optional=True),
        dep("ambre-chrome", target='cfg(target_arch = "wasm32")'),
    ],
    "ambre-chrome": [dep("ambre-core", kind="dev")],
}))

TABLE = """\
| Crate | Depends on | Why the line is there |
|---|---|---|
| `ambre-core` | nothing of ours | ids |
| `ambre-crypto` | `ambre-core` | keys |
| `ambre-logic` | `ambre-core`, `ambre-crypto` (optional), `ambre-chrome` (wasm only) | the runtime, not `ambre-app` |
| `ambre-chrome` | the page framework | toolkit |
"""

README = """\
| Crate | Purpose |
|-------|---------|
| `ambre-core` | ids |
| `ambre-crypto` | keys |
| `ambre-logic` | runtime |
| `ambre-chrome` | toolkit |
"""


def architecture(text: str) -> list[str]:
    return gate.check_architecture(gate.table(text, gate.ARCHITECTURE_HEADER), GRAPH)


def readme(text: str) -> list[str]:
    return gate.check_readme(gate.table(text, gate.README_HEADER), GRAPH)


class Graph(unittest.TestCase):
    def test_a_dev_dependency_and_a_third_party_crate_are_no_edge(self) -> None:
        self.assertEqual(GRAPH["ambre-core"], {})
        self.assertEqual(GRAPH["ambre-chrome"], {})
        self.assertEqual(GRAPH["ambre-crypto"], {"ambre-core": ()})

    def test_an_edge_on_both_targets_carries_no_target_qualifier(self) -> None:
        found = gate.qualifier([
            dep("ambre-convert"),
            dep("ambre-convert", target='cfg(not(target_arch = "wasm32"))'),
        ])
        self.assertEqual(found, ())

    def test_each_qualifier_is_read_off_the_manifest(self) -> None:
        self.assertEqual(gate.qualifier([dep("a", target='cfg(not(target_arch = "wasm32"))')]), ("host only",))
        self.assertEqual(gate.qualifier([dep("a", kind="build", optional=True)]), ("build only", "optional"))
        self.assertEqual(gate.qualifier([dep("a", optional=True), dep("a")]), ())

    def test_an_unplaced_cfg_stops_the_gate(self) -> None:
        with self.assertRaises(RuntimeError):
            gate.qualifier([dep("a", target='cfg(unix)')])


class Architecture(unittest.TestCase):
    def test_the_table_of_the_graph_passes(self) -> None:
        self.assertEqual(architecture(TABLE), [])

    def test_an_edge_cargo_does_not_resolve_is_refused(self) -> None:
        wrong = TABLE.replace("| `ambre-chrome` | the page framework |", "| `ambre-chrome` | `ambre-crypto` |")
        found = architecture(wrong)
        self.assertEqual(len(found), 1)
        self.assertIn("row `ambre-chrome`", found[0])
        self.assertIn("`ambre-crypto`", found[0])

    def test_an_omitted_edge_is_refused(self) -> None:
        found = architecture(TABLE.replace("| `ambre-crypto` | `ambre-core` |", "| `ambre-crypto` | nothing |"))
        self.assertEqual(len(found), 1)
        self.assertIn("omits `ambre-core`", found[0])

    def test_a_missing_or_wrong_qualifier_is_refused(self) -> None:
        for before, after in (
            ("`ambre-chrome` (wasm only)", "`ambre-chrome`"),
            ("`ambre-crypto` (optional)", "`ambre-crypto`"),
            ("`ambre-chrome` (wasm only)", "`ambre-chrome` (host only)"),
            ("`ambre-crypto` (optional)", "`ambre-crypto` (feature)"),
        ):
            with self.subTest(after=after):
                self.assertEqual(len(architecture(TABLE.replace(before, after))), 1)

    def test_qualifiers_out_of_order_are_refused(self) -> None:
        wrong = TABLE.replace("`ambre-crypto` (optional)", "`ambre-crypto` (optional, wasm only)")
        self.assertIn("out of the order", architecture(wrong)[0])

    def test_a_member_with_no_row_is_refused(self) -> None:
        found = architecture(TABLE.replace("| `ambre-core` | nothing of ours | ids |\n", ""))
        self.assertEqual(found, ["architecture table: the workspace member `ambre-core` has no row"])

    def test_a_row_for_no_member_and_a_second_row_are_refused(self) -> None:
        extra = TABLE + "| `ambre-ghost` | `ambre-core` | none |\n| `ambre-core` | nothing | again |\n"
        found = architecture(extra)
        self.assertEqual(len(found), 2)
        self.assertIn("no workspace member", found[0])
        self.assertIn("second row", found[1])

    def test_a_group_row_is_refused(self) -> None:
        found = architecture(TABLE.replace("| `ambre-chrome` |", "| each toolkit crate |"))
        self.assertIn("names no single crate", found[0])

    def test_a_table_the_gate_cannot_find_stops_it(self) -> None:
        with self.assertRaises(RuntimeError):
            gate.table("no table here\n", gate.ARCHITECTURE_HEADER)


class Readme(unittest.TestCase):
    def test_the_map_of_the_members_passes(self) -> None:
        self.assertEqual(readme(README), [])

    def test_a_missing_and_an_extra_crate_are_refused(self) -> None:
        wrong = README.replace("| `ambre-logic` | runtime |", "| `ambre-ghost` | runtime |")
        found = readme(wrong)
        self.assertEqual(len(found), 2)
        self.assertIn("`ambre-ghost` names no workspace member", found[0])
        self.assertIn("`ambre-logic` has no row", found[1])


class Tree(unittest.TestCase):
    """The two tables of the tree, against the manifests of the tree."""

    def test_the_tables_of_the_tree_pass(self) -> None:
        resolved = gate.graph(gate.metadata())
        text = gate.ARCHITECTURE.read_text()
        self.assertEqual(gate.check_architecture(gate.table(text, gate.ARCHITECTURE_HEADER), resolved), [])
        self.assertEqual(gate.check_readme(gate.table(gate.README.read_text(), gate.README_HEADER), resolved), [])


if __name__ == "__main__":
    unittest.main()
