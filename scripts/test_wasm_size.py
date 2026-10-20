#!/usr/bin/env python3
"""What `wasm_size.py` passes, and what it fails.

The verdict is tested on figures alone: an overrun of the ceiling fails, and a
figure between the budget and the ceiling passes while it says how far over the
budget it sits. Building the release is left to `just size`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import wasm_size  # noqa: E402  -- the module under test


class Verdict(unittest.TestCase):
    def test_a_figure_over_the_ceiling_fails(self) -> None:
        ok, line = wasm_size.verdict(3_000_001, 3_000_000, 2_000_000)
        self.assertFalse(ok)
        self.assertIn("1 over the ceiling", line)

    def test_a_figure_under_the_ceiling_but_over_the_budget_passes_and_says_so(self) -> None:
        ok, line = wasm_size.verdict(2_500_000, 3_000_000, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("500,000 over the budget", line)

    def test_a_figure_within_the_budget_passes(self) -> None:
        ok, line = wasm_size.verdict(1_900_000, 1_950_000, 2_000_000)
        self.assertTrue(ok)
        self.assertIn("within the budget", line)


class Gzipped(unittest.TestCase):
    def test_the_same_bytes_weigh_the_same(self) -> None:
        data = bytes(range(256)) * 64
        self.assertEqual(wasm_size.gzipped(data), wasm_size.gzipped(data))
        self.assertLess(wasm_size.gzipped(data), len(data))


class Ceiling(unittest.TestCase):
    def test_the_ceiling_is_set(self) -> None:
        self.assertGreater(wasm_size.CEILING, 0)


if __name__ == "__main__":
    unittest.main()
