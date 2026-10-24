#!/usr/bin/env python3
"""What `schema_freeze.py` reads, and what it refuses.

The check rests on two decisions, and both are tested here. A section is cut
at its `== vN` line, so the comment header above the first one stays free to be
reworded while every body below is held. And the revision it compares against
is resolved, not assumed: a branch sitting on the base reads the parent commit,
so a section edited by a commit landing straight on the base branch is caught
like any other.

A section binds only from the first release tag on. Each repository test runs
in both modes, so an edit refused once a release ships passes before it, and a
freeze parted from its belt is refused in both. A shallow clone with no tag is
refused, since it cannot tell a release from none.

The pairing of a freeze with its belt is tested on a tree alone, since no
revision comes into it. The section reader is tested on text alone. The rest runs against a real
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


def belt(root: Path, crate: str, version: int = 1, missing: str | None = None) -> Path:
    """A crate's source side of the pairing: the stamp, and the belt that reads
    each frozen file. `missing` names a belt left out."""
    src = root / "crates" / crate / "src"
    src.mkdir(parents=True, exist_ok=True)
    body = (
        "pub const SCHEMA_VERSION: SchemaVersion = "
        f"SchemaVersion::new({version});\n"
    )
    for name in schema_freeze.FREEZES.values():
        if name != missing:
            body += f"fn {name}() {{}}\n"
    path = src / "schema.rs"
    path.write_text(body)
    return path


def shape(root: Path, crate: str, text: str = HEADER + V1, name: str = "shapes.txt") -> Path:
    """A crate's frozen file, the shape file unless `name` says otherwise."""
    path = root / "crates" / crate / "schema" / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def frozen(root: Path, crate: str, missing: str | None = None) -> None:
    """Every frozen file of a crate, each holding `v1`. `missing` names a file
    left out."""
    for name in schema_freeze.FREEZES:
        if name != missing:
            shape(root, crate, name=name)


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
    """The check against a base revision, once a release tag exists.

    `BOUND` is the exit code of an edited or a dropped section: 1 once a
    release binds the sections, 0 before. The subclass below reruns every test
    with no tag, so the coherence tests, whose outcome is fixed, hold in both
    modes.
    """

    # The tag a test repository carries, or `None` for a pre-release one.
    TAG: str | None = "v1.0.0"
    BOUND = 1

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.shapes = self.root / "crates" / "ambre-x" / "schema" / "shapes.txt"
        self.shapes.parent.mkdir(parents=True)
        belt(self.root, "ambre-x", version=1)
        frozen(self.root, "ambre-x", missing="shapes.txt")
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "user.email", "t@example.com")
        git(self.root, "config", "user.name", "t")
        self.shapes.write_text(HEADER + V1)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "freeze v1")
        if self.TAG is not None:
            git(self.root, "tag", self.TAG)
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

    # The sections, held from the first release on.

    def test_a_published_section_edited_binds_once_released(self):
        self.commit(HEADER + V1.replace("i64", "f64"))
        self.assertEqual(self.run_check("HEAD~1"), self.BOUND)

    def test_a_published_section_dropped_binds_once_released(self):
        # The compiled version moves to `v2` with every file, so the pairing
        # holds and only the dropped `v1` can refuse.
        belt(self.root, "ambre-x", version=2)
        for name in schema_freeze.FREEZES:
            shape(self.root, "ambre-x", HEADER + V2, name=name)
        self.commit(HEADER + V2)
        self.assertEqual(self.run_check("HEAD~1"), self.BOUND)

    def test_a_published_scaffold_section_edited_binds_once_released(self):
        shape(self.root, "ambre-x", HEADER + V1.replace("i64", "f64"), name="scaffold.txt")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "edit the scaffold")
        self.assertEqual(self.run_check("HEAD~1"), self.BOUND)

    def test_a_base_the_branch_sits_on_reads_the_parent_once_released(self):
        # `main` is `HEAD`, so a diff against it is empty and the edit below
        # would pass unseen unless the parent is read instead.
        self.commit(HEADER + V1.replace("i64", "f64"))
        self.assertEqual(self.run_check("main"), self.BOUND)

    def test_a_base_that_cannot_be_read_binds_once_released(self):
        # Before a release no base is read, so a missing one costs nothing.
        self.assertEqual(self.run_check("nope/nope"), self.BOUND)

    def test_only_a_tag_of_the_release_pattern_releases(self):
        git(self.root, "tag", "nightly")
        expected = [] if self.TAG is None else [self.TAG]
        self.assertEqual(schema_freeze.released(), expected)

    # Free in both modes.

    def test_a_section_appended_below_is_free(self):
        self.commit(HEADER + V1 + V2)
        self.assertEqual(self.run_check("HEAD~1"), 0)

    def test_a_section_appended_after_a_blank_line_is_free(self):
        # The blank line is the separator every shape file writes between two
        # sections; it closes the body above and does not edit it.
        self.commit(HEADER + V1 + "\n" + V2)
        self.assertEqual(self.run_check("HEAD~1"), 0)

    def test_the_comment_header_reworded_is_free(self):
        self.commit("# another header\n" + V1)
        self.assertEqual(self.run_check("HEAD~1"), 0)

    # The coherence of the tree, refused in both modes.

    def test_the_file_deleted_is_refused(self):
        # The crate still stamps `v1`, so a deleted file parts the freeze from
        # its stamp whatever the mode.
        self.shapes.unlink()
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "delete")
        self.assertEqual(self.run_check("HEAD~1"), 1)

    def test_a_freeze_with_no_belt_is_refused(self):
        belt(self.root, "ambre-x", version=1, missing=schema_freeze.BELT)
        self.assertEqual(self.run_check("HEAD"), 1)

    def test_a_bump_with_no_section_to_compare_against_is_refused(self):
        belt(self.root, "ambre-x", version=2)
        self.assertEqual(self.run_check("HEAD"), 1)


class PrereleaseTest(RepositoryTest):
    """The same repository before its first release: no section binds yet."""

    TAG = None
    BOUND = 0


class ShallowTest(unittest.TestCase):
    """The release read from a shallow clone, which may carry no tag at all.

    A shallow clone with no tag cannot tell a release from none, so it is
    refused. The same clone holding a release tag binds the sections. A full
    clone with no tag is pre-release, as it reads.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.origin = Path(self.tmp.name) / "origin"
        self.origin.mkdir()
        belt(self.origin, "ambre-x", version=1)
        frozen(self.origin, "ambre-x")
        git(self.origin, "init", "-q", "-b", "main")
        git(self.origin, "config", "user.email", "t@example.com")
        git(self.origin, "config", "user.name", "t")
        git(self.origin, "add", "-A")
        git(self.origin, "commit", "-qm", "freeze v1")
        self.was = schema_freeze.ROOT
        self.addCleanup(self.restore)

    def restore(self):
        schema_freeze.ROOT = self.was
        self.tmp.cleanup()

    def edit(self, text: str) -> None:
        """One commit on the origin that rewrites its shape file."""
        shape(self.origin, "ambre-x", text)
        git(self.origin, "add", "-A")
        git(self.origin, "commit", "-qm", "edit")

    def clone(self, depth: int | None) -> None:
        """A clone of the origin, shallow when `depth` is set, and read."""
        root = Path(self.tmp.name) / "clone"
        args = ["clone", "-q", "--no-tags"]
        if depth is not None:
            args += [f"--depth={depth}"]
        git(Path(self.tmp.name), *args, self.origin.as_uri(), str(root))
        schema_freeze.ROOT = root

    def run_check(self) -> int:
        argv = sys.argv
        sys.argv = ["schema_freeze.py", "--check", "--base", "HEAD~1"]
        try:
            return schema_freeze.main()
        finally:
            sys.argv = argv

    def test_a_shallow_clone_without_tags_is_refused(self):
        # The appended section is free in both modes, so only the missing
        # tags can refuse.
        self.edit(HEADER + V1 + V2)
        self.clone(depth=2)
        self.assertTrue(schema_freeze.shallow())
        self.assertEqual(self.run_check(), 1)

    def test_a_shallow_clone_with_a_release_tag_binds(self):
        self.edit(HEADER + V1.replace("i64", "f64"))
        self.clone(depth=2)
        git(schema_freeze.ROOT, "tag", "v1.0.0")
        self.assertTrue(schema_freeze.shallow())
        self.assertEqual(self.run_check(), 1)

    def test_a_shallow_clone_with_a_release_tag_lets_an_append_pass(self):
        # The twin of the test above: the clone runs, so the refusal there is
        # the edited section, not the shallow clone.
        self.edit(HEADER + V1 + V2)
        self.clone(depth=2)
        git(schema_freeze.ROOT, "tag", "v1.0.0")
        self.assertEqual(self.run_check(), 0)

    def test_a_full_clone_without_tags_runs_pre_release(self):
        # The edited section would bind once released: exit 0 proves the
        # pre-release mode ran.
        self.edit(HEADER + V1.replace("i64", "f64"))
        self.clone(depth=None)
        self.assertFalse(schema_freeze.shallow())
        self.assertEqual(self.run_check(), 0)


class BeltTest(unittest.TestCase):
    """The pairing of a frozen shape with the test that compares a build to it.

    `SCHEMA-009` names three test paths, one per application. A fourth
    application joins none of them by existing, which is the whole reason this
    reads the tree rather than trusting the list.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.was = schema_freeze.ROOT
        schema_freeze.ROOT = self.root
        self.addCleanup(self.restore)

    def restore(self):
        schema_freeze.ROOT = self.was
        self.tmp.cleanup()

    def findings(self) -> list[str]:
        errors: list[str] = []
        schema_freeze.check_belts(errors)
        return errors

    def test_a_crate_that_freezes_and_compares_is_clean(self):
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=1)
        self.assertEqual(self.findings(), [])

    def test_the_real_tree_is_clean(self):
        schema_freeze.ROOT = self.was
        self.assertEqual(self.findings(), [])

    def test_a_freeze_with_no_belt_is_refused(self):
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=1, missing=schema_freeze.BELT)
        self.assertEqual(len(self.findings()), 1)

    def test_a_frozen_scaffold_with_no_belt_is_refused(self):
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=1, missing=schema_freeze.SCAFFOLD_BELT)
        self.assertEqual(len(self.findings()), 1)

    def test_frozen_migrations_with_no_belt_are_refused(self):
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=1, missing=schema_freeze.MIGRATION_BELT)
        self.assertEqual(len(self.findings()), 1)

    def test_a_stamped_crate_with_no_frozen_shape_is_refused(self):
        frozen(self.root, "ambre-x", missing="shapes.txt")
        belt(self.root, "ambre-x", version=1)
        self.assertEqual(len(self.findings()), 1)

    def test_a_stamped_crate_with_no_frozen_scaffold_is_refused(self):
        frozen(self.root, "ambre-x", missing="scaffold.txt")
        belt(self.root, "ambre-x", version=1)
        self.assertEqual(len(self.findings()), 1)

    def test_a_stamped_crate_with_no_frozen_migrations_is_refused(self):
        frozen(self.root, "ambre-x", missing="migrations.txt")
        belt(self.root, "ambre-x", version=1)
        self.assertEqual(len(self.findings()), 1)

    def test_a_stamped_crate_with_no_frozen_readings_is_refused(self):
        frozen(self.root, "ambre-x", missing="readings.txt")
        belt(self.root, "ambre-x", version=1)
        self.assertEqual(len(self.findings()), 1)

    def test_frozen_readings_with_no_belt_are_refused(self):
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=1, missing=schema_freeze.READING_BELT)
        self.assertEqual(len(self.findings()), 1)

    def test_a_bump_with_no_section_to_compare_against_is_refused(self):
        # One finding per frozen file: none carries a `v2` section.
        frozen(self.root, "ambre-x")
        belt(self.root, "ambre-x", version=2)
        self.assertEqual(len(self.findings()), len(schema_freeze.FREEZES))

    def test_the_protocol_constants_are_not_a_stamp(self):
        # `ambre-core` declares `SCHEMA_VERSION_KEY` and
        # `SCHEMA_VERSION_ABSENT`, and freezes no shape. Reading either as a
        # stamp would make the shared crate a permanent finding.
        src = self.root / "crates" / "ambre-core" / "src"
        src.mkdir(parents=True)
        (src / "schema.rs").write_text(
            'pub const SCHEMA_VERSION_KEY: &str = "schema_version";\n'
            "pub const SCHEMA_VERSION_ABSENT: SchemaVersion = SchemaVersion(0);\n"
        )
        self.assertEqual(self.findings(), [])


if __name__ == "__main__":
    unittest.main()
