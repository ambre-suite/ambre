#!/usr/bin/env python3
"""What `merge_generated.py` does to a merge of two branches that each add a contract row.

The scenario copies this work tree into a fresh repository in a temporary
directory, so it reads the real corpus and the real generators. Two branches
each add one row to a different spec, then regenerate and commit. The merge and
the rebase of one onto the other then run through the driver. No git call is
stubbed: the defect worth catching is in what git hands the driver.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import merge_generated as mg  # noqa: E402  -- the module under test

ROOT = Path(__file__).resolve().parent.parent

# The regenerations a branch runs before it commits, and the gates that read
# the generated files.
WRITERS = ("scripts/contracts.py", "scripts/feature_inventory.py")
GATES = (
    ("scripts/contracts.py",),
    ("scripts/contract_index.py",),
    ("scripts/feature_inventory.py",),
)

# One row per branch, each appended to the table that holds the anchor row.
# Each one is `unimplemented` and bound to an open Story, so the corpus stays
# valid on each branch and after the merge.
ROWS = {
    "accessibility": (
        "specs/1-Foundations/ambre-spec-accessibility.md",
        "`CHROME-029`",
        "| `CHROME-117` | `INV` | A drawn focus ring MUST also reach the page. | this spec "
        "section 2 | not built | unbuilt -- lands with the `Drawn content reaches a screen "
        "reader` Story (`ambre-roadmap.md` E6) | unimplemented |",
    ),
    "calls": (
        "specs/1-Foundations/ambre-spec-call-media.md",
        "`CALL-001`",
        "| `CALL-044` | `INV` | A chunk of a call MUST carry the epoch of its sender key. | this "
        "spec section 3 | not built | unbuilt -- lands with the `Audio calls` Story "
        "(`ambre-roadmap.md` E5) | unimplemented |",
    ),
}

# Git reads no configuration of the person who runs the test: no hook, no
# identity, no driver installed by hand.
HERMETIC = {
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.org",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.org",
}


def next_free(text: str, row: str) -> str:
    """`row` under the next free id of its prefix in `text`.

    A branch of the corpus may take the id a row of `ROWS` names, so the fixture
    numbers each row after the highest id of its prefix in the corpus `text`, as
    a new row would. One prefix may span several documents.
    """
    prefix = re.match(r"\| `([A-Z][A-Z-]*)-\d+` \|", row)
    if prefix is None:
        return row
    taken = [int(n) for n in re.findall(rf"\| `{prefix.group(1)}-(\d+)` \|", text)]
    width = len(re.search(r"-(\d+)`", row).group(1))
    fresh = f"{prefix.group(1)}-{max(taken, default=0) + 1:0{width}d}"
    return re.sub(r"`[A-Z][A-Z-]*-\d+`", f"`{fresh}`", row, count=1)


def append_row(path: Path, anchor: str, row: str) -> None:
    """Append `row` after the last row of the table that holds `anchor`."""
    text = path.read_text()
    specs = next(p for p in path.parents if p.name == "specs")
    corpus = "\n".join(doc.read_text() for doc in sorted(specs.rglob("*.md")))
    row = next_free(corpus, row)
    lines = text.split("\n")
    at = next(i for i, line in enumerate(lines) if line.startswith(f"| {anchor} |"))
    while at + 1 < len(lines) and lines[at + 1].startswith("| `"):
        at += 1
    lines.insert(at + 1, row)
    path.write_text("\n".join(lines))


class Scenario:
    """A copy of this work tree, committed as `main`, with the driver installed."""

    def __init__(self, root: Path):
        self.root = root
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(("GIT_", "GITHEAD_"))} | HERMETIC
        files = subprocess.run(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            cwd=ROOT, capture_output=True, check=True,
        ).stdout.decode().split("\0")
        for rel in filter(None, files):
            source = ROOT / rel
            if source.is_file():  # a file deleted in the work tree is skipped
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, root / rel)
        self.git("init", "-q", "-b", "main")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base")

    def git(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        return subprocess.run(["git", *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True, check=check)

    def install(self) -> None:
        self.git("config", f"merge.{mg.DRIVER}.driver",
                 "python3 scripts/merge_generated.py %O %A %B %P")

    def python(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True)

    def branch(self, name: str) -> None:
        """A branch off `main` that adds the row `name` and regenerates."""
        path, anchor, row = ROWS[name]
        self.git("checkout", "-q", "-b", name, "main")
        append_row(self.root / path, anchor, row)
        for writer in WRITERS:
            self.python(writer, "--write")
        self.git("commit", "-q", "-am", f"add a row to {name}")

    def conflicts(self) -> list[str]:
        return self.git("diff", "--name-only", "--diff-filter=U").stdout.split()

    def gate_failures(self) -> list[str]:
        failures = []
        for gate in GATES:
            run = self.python(*gate)
            if run.returncode != 0:
                failures.append(f"{' '.join(gate)}: {run.stdout[-600:]}{run.stderr[-600:]}")
        return failures

    def generated_changed(self, a: str, b: str) -> list[str]:
        names = self.git("diff", "--name-only", a, b).stdout.split()
        return [n for n in names if mg.is_generated(n)]


class MergeOfTwoRows(unittest.TestCase):
    """Two branches that each add a row merge with no hand resolution."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="ambre-merge-test-")
        cls.repo = Scenario(Path(cls.tmp.name))
        cls.repo.branch("accessibility")
        cls.repo.branch("calls")
        cls.tip = cls.repo.git("rev-parse", "accessibility").stdout.strip()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def setUp(self):
        self.repo.git("merge", "--abort", check=False)
        self.repo.git("rebase", "--abort", check=False)
        self.repo.git("config", "--unset", f"merge.{mg.DRIVER}.driver", check=False)
        self.repo.git("checkout", "-q", "-f", "accessibility")
        self.repo.git("reset", "-q", "--hard", self.tip)

    def test_both_branches_change_the_same_generated_files(self):
        # The scenario is only a test when each side touches what the other does.
        shared = set(self.repo.generated_changed("main", "accessibility")) & set(
            self.repo.generated_changed("main", "calls"))
        self.assertGreaterEqual(len(shared), 2, shared)

    def test_without_the_driver_a_text_merge_leaves_a_generated_file_wrong(self):
        # A text merge either conflicts on a generated file, or merges the two
        # sets of counts line by line into a file no generator writes.
        run = self.repo.git("merge", "--no-edit", "calls", check=False)
        conflicts = self.repo.conflicts()
        self.assertTrue(all(mg.is_generated(path) for path in conflicts), conflicts)
        if run.returncode == 0:
            self.assertNotEqual(self.repo.gate_failures(), [])
        else:
            self.assertTrue(conflicts, run.stdout + run.stderr)

    def test_a_merge_conflicts_on_no_file_and_the_gates_pass(self):
        self.repo.install()
        run = self.repo.git("merge", "--no-edit", "calls", check=False)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(self.repo.conflicts(), [])
        self.assertEqual(self.repo.gate_failures(), [])
        self.assertEqual(self.repo.git("status", "--porcelain").stdout, "")

    def test_a_rebase_conflicts_on_no_file_and_the_gates_pass(self):
        self.repo.install()
        self.repo.git("checkout", "-q", "-B", "calls-rebased", "calls")
        run = self.repo.git("rebase", "accessibility", check=False)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertEqual(self.repo.gate_failures(), [])
        self.assertEqual(self.repo.git("status", "--porcelain").stdout, "")

    def test_a_cherry_pick_keeps_our_side_and_names_the_regeneration(self):
        # Git names no other commit to a lone cherry-pick: the file is stale,
        # the driver says so, and the gate refuses it.
        self.repo.install()
        run = self.repo.git("cherry-pick", "calls", check=False)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn(mg.REGENERATE, run.stderr)
        self.assertNotEqual(self.repo.gate_failures(), [])


class GeneratedSet(unittest.TestCase):
    """`.gitattributes` and the driver name the same files: what the generators write."""

    def test_the_attribute_marks_exactly_the_generated_files(self):
        files = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True,
                               text=True, check=True).stdout
        attrs = subprocess.run(["git", "check-attr", "--stdin", "merge"], cwd=ROOT,
                               input=files, capture_output=True, text=True, check=True)
        marked = sorted(line.split(":")[0] for line in attrs.stdout.splitlines()
                        if line.endswith(f": {mg.DRIVER}"))
        generated = sorted(path for path in files.split() if mg.is_generated(path))
        self.assertEqual(marked, generated)
        # The ledger and one inventory per layer, Markdown only: the index and
        # the graph are built on demand under `target/`, never committed.
        self.assertGreaterEqual(len(generated), 6)
        self.assertTrue(all(path.endswith(".md") for path in generated), generated)


class OtherSide(unittest.TestCase):
    """Which commit the driver merges with, read from what git leaves."""

    def test_a_merge_names_its_head(self):
        head = "a" * 40
        self.assertEqual(mg.other_side({f"GITHEAD_{head}": "x"}, Path("/nonexistent")),
                         (head, None))

    def test_an_octopus_merge_names_none(self):
        env = {f"GITHEAD_{'a' * 40}": "x", f"GITHEAD_{'b' * 40}": "y"}
        self.assertIsNone(mg.other_side(env, Path("/nonexistent")))

    def test_a_rebase_command_without_a_commit_names_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            done = Path(tmp) / "rebase-merge" / "done"
            done.parent.mkdir()
            done.write_text("exec make test\n")
            self.assertIsNone(mg.other_side({}, Path(tmp)))

    def test_no_merge_state_names_none(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(mg.other_side({}, Path(tmp)))


if __name__ == "__main__":
    unittest.main()
