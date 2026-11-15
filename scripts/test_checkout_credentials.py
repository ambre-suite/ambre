#!/usr/bin/env python3
"""What `checkout_credentials.py` refuses, and what it lets through.

Each test plants one workflow through `breaches` and asserts the verdict.
Reading the files of `.github/` is left to the run on the tree.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import checkout_credentials  # noqa: E402  -- the module under test

PIN = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"


def workflow(step: str) -> str:
    return (
        "jobs:\n"
        "  lint:\n"
        "    runs-on: ubuntu-latest\n"
        "    steps:\n"
        f"{step}"
        "      - name: Install just\n"
        "        uses: taiki-e/install-action@v2\n"
        "        with:\n"
        "          persist-credentials: true\n"
        "      - run: just lint\n"
    )


def verdict(step: str) -> list[str]:
    errors, _ = checkout_credentials.breaches({"ci.yml": workflow(step)})
    return errors


class CheckoutCredentials(unittest.TestCase):
    def test_false_passes(self) -> None:
        step = (f"      - uses: {PIN}\n"
                "        with:\n"
                "          fetch-depth: 0\n"
                "          persist-credentials: false\n")
        self.assertEqual(verdict(step), [])

    def test_quoted_false_passes(self) -> None:
        step = f"      - uses: {PIN}\n        with:\n          persist-credentials: 'false'\n"
        self.assertEqual(verdict(step), [])

    def test_a_checkout_with_no_value_fails(self) -> None:
        errors = verdict(f"      - uses: {PIN}\n")
        self.assertEqual(len(errors), 1)
        self.assertIn("ci.yml:5:", errors[0])
        self.assertIn("sets no `persist-credentials`", errors[0])

    def test_a_value_in_the_next_step_does_not_count(self) -> None:
        # The install step below the checkout sets the key: it belongs to that step.
        errors = verdict(f"      - uses: {PIN}\n        with:\n          fetch-depth: 0\n")
        self.assertEqual(len(errors), 1)

    def test_a_named_checkout_with_uses_below_its_name_is_read(self) -> None:
        errors = verdict(f"      - name: Checkout\n        uses: {PIN}\n")
        self.assertEqual(len(errors), 1)
        self.assertIn("ci.yml:6:", errors[0])

    def test_a_commented_value_does_not_count(self) -> None:
        step = f"      - uses: {PIN}\n        with:\n          # persist-credentials: false\n"
        self.assertEqual(len(verdict(step)), 1)

    def test_an_unpinned_checkout_is_read(self) -> None:
        self.assertEqual(len(verdict("      - uses: actions/checkout\n")), 1)

    def test_an_expression_fails(self) -> None:
        step = (f"      - uses: {PIN}\n        with:\n"
                "          persist-credentials: ${{ inputs.push }}\n")
        errors = verdict(step)
        self.assertEqual(len(errors), 1)
        self.assertIn("neither `false` nor `true`", errors[0])

    def test_a_flow_mapping_fails(self) -> None:
        step = f"      - uses: {PIN}\n        with: {{ persist-credentials: false }}\n"
        self.assertEqual(len(verdict(step)), 1)

    def test_true_without_a_push_name_fails(self) -> None:
        step = (f"      - name: Checkout\n        uses: {PIN}\n        with:\n"
                "          persist-credentials: true\n")
        errors = verdict(step)
        self.assertEqual(len(errors), 1)
        self.assertIn("Checkout to push: <reason>", errors[0])

    def test_true_with_no_name_fails(self) -> None:
        step = f"      - uses: {PIN}\n        with:\n          persist-credentials: true\n"
        self.assertEqual(len(verdict(step)), 1)

    def test_true_with_an_empty_reason_fails(self) -> None:
        step = (f"      - name: 'Checkout to push: '\n        uses: {PIN}\n        with:\n"
                "          persist-credentials: true\n")
        self.assertEqual(len(verdict(step)), 1)

    def test_true_with_a_push_name_passes(self) -> None:
        step = (f"      - name: 'Checkout to push: the release job tags main'\n"
                f"        uses: {PIN}\n        with:\n"
                "          persist-credentials: true\n")
        self.assertEqual(verdict(step), [])

    def test_no_checkout_read_fails(self) -> None:
        errors, count = checkout_credentials.breaches({"ci.yml": "jobs: {}\n"})
        self.assertEqual(count, 0)
        self.assertEqual(len(errors), 1)
        self.assertIn("no `actions/checkout` step read", errors[0])

    def test_each_file_is_read(self) -> None:
        good = f"      - uses: {PIN}\n        with:\n          persist-credentials: false\n"
        errors, count = checkout_credentials.breaches(
            {"a.yml": workflow(good), "b.yml": workflow(f"      - uses: {PIN}\n")}
        )
        self.assertEqual(count, 2)
        self.assertEqual(len(errors), 1)
        self.assertIn("b.yml:5:", errors[0])


if __name__ == "__main__":
    unittest.main()
