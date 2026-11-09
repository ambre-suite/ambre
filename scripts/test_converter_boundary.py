#!/usr/bin/env python3
"""What `converter_boundary.py` refuses, on graphs written by hand.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import converter_boundary as gate  # noqa: E402  -- the module under test


class Breaches(unittest.TestCase):
    def test_a_converter_in_an_instance_is_refused_by_name(self) -> None:
        found = gate.breaches({"Main": ["ambre-app", "ambre-convert", "krilla"], "Sync": ["ambre-sync"]})
        self.assertEqual(len(found), 1)
        self.assertIn("Main", found[0])
        self.assertIn("krilla", found[0])

    def test_a_reader_of_each_family_is_refused(self) -> None:
        for reader in ("csv", "krilla", "pulldown-cmark", "html5ever", "usvg", "image-webp"):
            found = gate.breaches({"Main": ["ambre-app", reader]})
            self.assertEqual(len(found), 1, reader)

    def test_the_loader_alone_passes(self) -> None:
        self.assertEqual(gate.breaches({"Main": ["ambre-app", "ambre-convert"], "Sync": []}), [])

    def test_a_name_that_only_starts_like_a_converter_passes(self) -> None:
        self.assertEqual(gate.breaches({"Main": ["csv-core", "krilla-svg"]}), [])


class FamilyBreaches(unittest.TestCase):
    def test_a_module_that_links_another_familys_converter_is_refused(self) -> None:
        found = gate.family_breaches({"tables": ["ambre-convert", "csv", "krilla"]})
        self.assertEqual(len(found), 1)
        self.assertIn("tables", found[0])
        self.assertIn("pdf", found[0])

    def test_a_module_of_its_own_family_alone_passes(self) -> None:
        modules = {family: ["ambre-convert", *readers] for family, readers in gate.FAMILY_CONVERTERS.items()}
        self.assertEqual(gate.family_breaches(modules), [])

    def test_every_family_the_build_writes_is_read(self) -> None:
        import converter_build

        self.assertEqual(tuple(gate.FAMILY_CONVERTERS), converter_build.FAMILIES)


if __name__ == "__main__":
    unittest.main()
