#!/usr/bin/env python3
"""What `schema_freeze.py` reads, and what it refuses.

The check rests on two decisions, and both are tested here. A section is cut
at its `== vN` line, so the comment header above the first one stays free to be
reworded while every body below is held. And the revision it compares against
is resolved, not assumed: a branch sitting on the base reads the parent commit,
so a section edited by a commit landing straight on the base branch is caught
like any other.

The section reader is tested on text alone. The rest runs against a real
repository built in a temporary directory, because the defect worth catching is
in what `git show` hands back, not in the comparison.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import schema_freeze  # noqa: E402  -- the module under test


HEADER = "# a comment header\n# a second line\n"
V1 = "== v1\nmeta/schema_version: i64\nmeta: map\n"
V2 = "== v2\nmeta/schema_version: i64\nmeta: map\nrows/*: string\n"


def git(cwd: Path, *args: str) -> str:
    """One git command, failing loudly."""
    done = subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
    )
    return done.stdout


class SectionsTest(unittest.TestCase):
    """The reader that cuts a shape file into its frozen sections."""

    def test_the_comment_header_is_no_section(self):
        self.assertEqual(sorted(schema_freeze.sections(HEADER + V1)), ["== v1"])

    def test_a_body_stops_at_the_next_marker(self):
        cut = schema_freeze.sections(HEADER + V1 + V2)
        self.assertEqual(sorted(cut), ["== v1", "== v2"])
        self.assertIn("meta: map", cut["== v1"])
        self.assertNotIn("rows/*: string", cut["== v1"])

    def test_a_file_with_no_marker_yields_nothing(self):
        self.assertEqual(schema_freeze.sections(HEADER), {})


class RepositoryTest(unittest.TestCase):
    """The check against a base revision, on a repository built for it."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.shapes = self.root / "crates" / "ambre-x" / "schema" / "shapes.txt"
        self.shapes.parent.mkdir(parents=True)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.email", "t@example.com")
        git(self.root, "config", "user.name", "t")
        self.shapes.write_text(HEADER + V1)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "freeze v1")
        # The module reads one repository, named by a module constant.
        self.was = schema_freeze.ROOT
        schema_freeze.ROOT = self.root
        self.addCleanup(self.restore)

    def restore(self):
        schema_freeze.ROOT = self.was
        self.tmp.cleanup()

    def run_check(self, base: str) -> int:
        argv = sys.argv
        sys.argv = ["schema_freeze.py", "--check", "--base", base]
        try:
            return schema_freeze.main()
        finally:
            sys.argv = argv

    def commit(self, text: str) -> None:
        self.shapes.write_text(text)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "edit")

    def test_a_section_appended_below_is_free(self):
        self.commit(HEADER + V1 + V2)
        self.assertEqual(self.run_check("HEAD~1"), 0)

    def test_a_published_section_edited_is_refused(self):
        self.commit(HEADER + V1.replace("i64", "f64"))
        self.assertEqual(self.run_check("HEAD~1"), 1)

    def test_a_published_section_dropped_is_refused(self):
        self.commit(HEADER + V2)
        self.assertEqual(self.run_check("HEAD~1"), 1)

    def test_the_file_deleted_is_refused(self):
        self.shapes.unlink()
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "delete")
        self.assertEqual(self.run_check("HEAD~1"), 1)

    def test_the_comment_header_reworded_is_free(self):
        self.commit("# another header\n" + V1)
        self.assertEqual(self.run_check("HEAD~1"), 0)

    def test_a_base_the_branch_sits_on_reads_the_parent(self):
        # `main` is `HEAD`, so a diff against it is empty and the edit below
        # would pass unseen unless the parent is read instead.
        self.commit(HEADER + V1.replace("i64", "f64"))
        self.assertEqual(self.run_check("main"), 1)

    def test_a_base_that_cannot_be_read_is_refused(self):
        self.assertEqual(self.run_check("nope/nope"), 1)


if __name__ == "__main__":
    unittest.main()
