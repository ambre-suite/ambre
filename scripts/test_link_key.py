#!/usr/bin/env python3
"""What `link_key.py` refuses, and what it lets through.

Each test plants one breach into the client build of the tree, or hands the
check one value, and asserts the fault, so a gate that reads nothing fails
here.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent))

import link_key as lk  # noqa: E402  -- the module under test

DOCKERFILE = lk.DOCKERFILE.read_text()
RELEASE = yaml.safe_load(lk.RELEASE.read_text())
DEV = lk.dev_key(lk.PROTOCOL.read_text())
KEY = "11" * 32


def release() -> dict:
    return copy.deepcopy(RELEASE)


class Check(unittest.TestCase):
    """`ZK-057`: a release refuses no key, a malformed key and the dev key."""

    def test_the_dev_key_is_the_one_of_the_public_dev_seed(self) -> None:
        self.assertTrue(DEV.startswith("927308f5") and len(DEV) == 64)

    def test_a_release_refuses_no_key_a_malformed_key_and_the_dev_key(self) -> None:
        for value in (None, "", "zz" * 32, KEY[2:], DEV, DEV.upper()):
            with self.subTest(value=value):
                self.assertIsNotNone(lk.refusal(value, DEV))

    def test_a_release_pins_any_other_key(self) -> None:
        self.assertIsNone(lk.refusal(KEY, DEV))
        self.assertIsNone(lk.refusal(KEY.upper(), DEV))


class Lint(unittest.TestCase):
    """`ZK-057`: each release build of the tree runs the check first."""

    def test_the_tree_passes(self) -> None:
        self.assertEqual(lk.check_dockerfile(DOCKERFILE), [])
        self.assertEqual(lk.check_release(RELEASE), [])

    def test_a_web_build_without_the_argument_fails(self) -> None:
        planted = DOCKERFILE.replace(f"ARG {lk.VARIABLE}=", "ARG UNRELATED=")
        self.assertNotEqual(planted, DOCKERFILE)
        self.assertTrue(any("no ARG" in f for f in lk.check_dockerfile(planted)))

    def test_a_release_build_without_the_check_fails(self) -> None:
        planted = DOCKERFILE.replace(f"python3 {lk.CHECK}; \\", "true; \\")
        self.assertNotEqual(planted, DOCKERFILE)
        self.assertTrue(any("before" in f for f in lk.check_dockerfile(planted)))

    def test_a_digest_without_the_check_fails(self) -> None:
        planted = release()
        job = planted["jobs"]["client-digest"]
        job["steps"] = [s for s in job["steps"] if lk.CHECK not in str(s.get("run", ""))]
        self.assertTrue(any("before" in f for f in lk.check_release(planted)))

    def test_a_digest_under_a_key_of_another_source_fails(self) -> None:
        for source in (None, "${{ secrets.THREAD_LINK_KEY }}", KEY):
            with self.subTest(source=source):
                planted = release()
                env = planted["jobs"]["client-digest"].setdefault("env", {})
                env.pop(lk.VARIABLE, None)
                if source is not None:
                    env[lk.VARIABLE] = source
                self.assertTrue(any("is not" in f for f in lk.check_release(planted)))

    def test_an_image_build_without_the_argument_fails(self) -> None:
        planted = release()
        for step in planted["jobs"]["images"]["steps"]:
            if str(step.get("uses", "")).startswith("docker/build-push-action@"):
                step["with"].pop("build-args", None)
        self.assertTrue(any("passes no" in f for f in lk.check_release(planted)))


if __name__ == "__main__":
    unittest.main()
