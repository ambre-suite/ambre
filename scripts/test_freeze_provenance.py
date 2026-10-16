#!/usr/bin/env python3
"""What `freeze_provenance.py` reads in the history, and what it refuses.

Each case builds a real repository in a temporary directory. The defect worth
catching is in what the history hands back, so no git call is stubbed. The
generator is a shell line committed with the tree: the check reruns it at the
commit the frozen bytes were taken from, exactly as it reruns the real one.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import freeze_provenance as fp  # noqa: E402  -- the module under test

# The generator: it writes the live artifact from the text of `writer.txt`.
GEN = "gen.sh"
REGENERATE = f"sh {GEN}"
MODULE = "crates/ambre-toile/src/fixtures.rs"


class Repo:
    """A repository holding one board generator and its live artifact."""

    def __init__(self, root: Path):
        self.root = root
        self.run("init", "-q", "-b", "main")
        self.run("config", "user.email", "t@example.org")
        self.run("config", "user.name", "t")
        self.write(GEN, "cat writer.txt > fixtures/current/toile/board.ambre\n")
        self.write(MODULE, "fn build_board(s: &mut S) {\n    s.handle_command(C);\n}\n")

    def run(self, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=self.root, capture_output=True, text=True, check=True
        ).stdout.strip()

    def write(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def manifest(self, rel: str, version: tuple[int, int], doc_type: str = "Toile", seed: int = 7) -> None:
        name = rel.rsplit("/", 1)[-1].split("@")[0].removesuffix(".manifest")
        app = rel.split("/")[2]
        self.write(rel, json.dumps({
            "generator": f"{app}/{name}", "generator_version": 1, "seed": seed, "doc_type": doc_type,
            "ambre_version": version[0], "schema_version": version[1],
        }))

    def live(self, bytes_: str, version: tuple[int, int], committed: str | None = None) -> None:
        """The writer emits `bytes_`; the live artifact holds `committed`, by default
        what the writer emits."""
        self.write("writer.txt", bytes_)
        self.write("fixtures/current/toile/board.ambre", bytes_ if committed is None else committed)
        self.manifest("fixtures/current/toile/board.manifest", version)

    def freeze(self, bytes_: str, version: tuple[int, int]) -> str:
        stem = f"fixtures/frozen/toile/board@v{version[0]}-s{version[1]}"
        self.write(f"{stem}.ambre", bytes_)
        self.manifest(f"{stem}.manifest", version)
        return f"{stem}.ambre"

    def commit(self, message: str) -> str:
        self.run("add", "-A")
        self.run("commit", "-q", "-m", message)
        return self.run("rev-parse", "HEAD")

    def check(self, *args: str) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            code = fp.main(["--check", "--regenerate", REGENERATE, *args], root=self.root)
        return code, out.getvalue()


def bumped(repo: Repo) -> str:
    """The honest freeze: the writer moves from s1 to s2, and the s1 bytes are
    frozen in that same commit."""
    repo.live("board-s1", (1, 1))
    repo.commit("the s1 writer")
    repo.live("board-s2", (1, 2))
    repo.freeze("board-s1", (1, 1))
    return repo.commit("bump the board schema and freeze s1")


class FreezeProvenance(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Repo(Path(self.tmp.name))

    def tearDown(self):
        self.tmp.cleanup()

    def assertRefused(self, why: str, *args: str) -> None:
        code, out = self.repo.check("--all", *args)
        self.assertEqual(code, 1, out)
        self.assertIn(why, out)

    def test_a_freeze_in_its_bump_commit_traces_to_its_seed(self):
        bumped(self.repo)
        code, out = self.repo.check("--all")
        self.assertEqual(code, 0, out)
        self.assertIn("1 frozen artifacts checked", out)
        self.assertNotIn("note:", out)

    def test_a_freeze_from_the_later_writer_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.live("board-s2", (1, 2))
        repo.freeze("board-s2", (1, 1))
        repo.commit("freeze the new writer's bytes under the old name")
        self.assertRefused("no generator run from a seed wrote them")

    def test_bytes_no_generator_wrote_are_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.live("board-s2", (1, 2))
        repo.freeze("a user's board", (1, 1))
        repo.commit("freeze a real document")
        self.assertRefused("no generator run from a seed wrote them")

    def test_a_freeze_a_commit_late_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.live("board-s2", (1, 2))
        repo.commit("bump with no freeze")
        repo.freeze("board-s1", (1, 1))
        repo.commit("freeze afterwards")
        self.assertRefused("the freeze is not in the bump commit")

    def test_a_freeze_with_no_bump_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.freeze("board-s1", (1, 1))
        repo.commit("freeze with nothing bumped")
        self.assertRefused("the freeze is in no bump commit")

    def test_a_live_artifact_its_generator_does_not_rebuild_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1), committed="edited by hand")
        repo.commit("an s1 artifact no generator wrote")
        repo.live("board-s2", (1, 2))
        repo.freeze("edited by hand", (1, 1))
        repo.commit("bump and freeze")
        self.assertRefused("rebuilds other bytes from its seed")

    def test_a_generator_that_does_not_run_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.write(GEN, "exit 3\n")
        repo.commit("a broken generator")
        repo.live("board-s2", (1, 2))
        repo.freeze("board-s1", (1, 1))
        repo.commit("bump and freeze")
        self.assertRefused("failed")

    def test_another_seed_at_the_source_is_refused(self):
        repo = self.repo
        bumped(repo)
        repo.manifest("fixtures/frozen/toile/board@v1-s1.manifest", (1, 1), seed=8)
        repo.run("commit", "-q", "--amend", "-a", "--no-edit")
        self.assertRefused("names another seed")

    def test_a_generator_reading_a_file_at_the_source_is_refused(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.write(MODULE, 'fn build_board(s: &mut S) {\n    let b = include_bytes!("me.ambre");\n}\n')
        repo.commit("a generator that embeds a file")
        repo.live("board-s2", (1, 2))
        repo.freeze("board-s1", (1, 1))
        repo.commit("bump and freeze")
        self.assertRefused("reads besides its seed")

    def test_a_frozen_artifact_edited_after_its_freeze_is_refused(self):
        repo = self.repo
        rel = fp.FROZEN + "/toile/board@v1-s1.ambre"
        bumped(repo)
        repo.write(rel, "board-s1, touched")
        repo.commit("touch the frozen bytes")
        self.assertRefused("changed since its freeze")

    def test_a_freeze_from_an_older_writer_of_the_version_is_noted(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the first s1 writer")
        repo.write("other.txt", "x")
        last = repo.commit("the last commit holding the first writer's bytes")
        repo.live("board-s1 again", (1, 1))
        repo.commit("a second s1 writer")
        repo.live("board-s2", (1, 2))
        repo.freeze("board-s1", (1, 1))
        repo.commit("bump and freeze the first writer's bytes")
        code, out = repo.check("--all")
        self.assertEqual(code, 0, out)
        self.assertIn(f"taken from the writer at {last[:12]}, older than the outgoing writer", out)

    def test_a_container_bump_freezes_every_document_type(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.write("fixtures/current/trame/grid.ambre", "grid")
        repo.manifest("fixtures/current/trame/grid.manifest", (1, 1), doc_type="Trame")
        repo.commit("two document types")
        repo.live("board-v2", (2, 1))
        repo.freeze("board-s1", (1, 1))
        repo.commit("bump the container and freeze the board alone")
        self.assertRefused("freezes no Trame artifact")

    def test_an_artifact_the_base_carries_is_taken_as_checked(self):
        repo = self.repo
        bumped(repo)
        repo.run("branch", "base")
        repo.run("checkout", "-q", "-b", "work")
        repo.write("other.txt", "x")
        repo.commit("unrelated")
        code, out = repo.check("--base", "base")
        self.assertEqual(code, 0, out)
        self.assertIn("0 frozen artifacts checked at their freeze commit, 1 taken", out)

    def test_an_artifact_new_since_the_base_is_checked(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.run("branch", "base")
        repo.run("checkout", "-q", "-b", "work")
        repo.live("board-s2", (1, 2))
        repo.freeze("a user's board", (1, 1))
        repo.commit("freeze a real document")
        code, out = repo.check("--base", "base")
        self.assertEqual(code, 1, out)
        self.assertIn("no generator run from a seed wrote them", out)

    def test_a_push_of_the_base_reads_against_its_parent(self):
        repo = self.repo
        repo.live("board-s1", (1, 1))
        repo.commit("the s1 writer")
        repo.live("board-s2", (1, 2))
        repo.freeze("a user's board", (1, 1))
        repo.commit("freeze a real document straight on main")
        code, out = repo.check("--base", "main")
        self.assertEqual(code, 1, out)
        self.assertIn("1 frozen artifacts checked", out)

    def test_a_missing_base_is_an_error(self):
        bumped(self.repo)
        code, _ = self.repo.check("--base", "origin/nowhere")
        self.assertEqual(code, 1)

    def test_a_history_cut_at_the_freeze_is_an_error(self):
        bumped(self.repo)
        clone = Path(self.tmp.name) / "shallow"
        subprocess.run(
            ["git", "clone", "-q", "--depth=1", f"file://{self.repo.root}", str(clone)],
            check=True, capture_output=True,
        )
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            code = fp.main(["--check", "--all", "--regenerate", REGENERATE], root=clone)
        self.assertEqual(code, 1, out.getvalue())
        self.assertIn("git fetch --unshallow", out.getvalue())


if __name__ == "__main__":
    unittest.main()
