#!/usr/bin/env python3
"""Each frozen artifact, checked against the commit that froze it.

A frozen artifact outlives its generator by design: a writer emits the current
version only, so the tree of a later commit can no longer build it. What the
tree cannot show, the history can. This check resolves each artifact of
`fixtures/frozen/` to the commit that added it, and reads three things there.

**The freeze is the bump** (`DOCFILE-008`). The live manifest the artifact was
copied from carries the frozen version pair just before that commit, and a
greater one at it. A freeze taken a commit late, or with no bump, is refused.
A bump of the container version freezes every document type in that commit.

**The bytes came from a generator run from a seed** (`FIXTURE-005`). The frozen
bytes sat in `fixtures/current/` at or before the parent of the freeze commit:
that is the last writer of the outgoing version. The generator is then rerun
at that commit, in a throwaway worktree, and must rebuild the same bytes from
the seed its manifest names. Bytes no generator wrote, or a later writer's,
never sat there and are refused.

**The generator read nothing besides its seed** (`FIXTURE-032`). The generator
modules of that same commit are read for a file read, by a scan of their text.
The rustc driver of `just primitives` holds the rule on the tree of today; a
commit of the past is text alone, since building it through the driver would
cost a build per artifact. An artifact rebuilt from a seed by such a generator
came from no user's document.

**New artifacts, not the whole set.** A rerun builds the generator at an old
commit, which costs a build. A frozen artifact never changes, so the check of
the commit that landed it stays true: `--base` names the revision whose
artifacts are taken as checked, `origin/main` by default, and `--all` reads
every one. A push of the base branch itself reads against its parent.

**A check that cannot read the history proves nothing**, so a missing base or
a history cut short is an error naming the fetch that fixes it.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import re
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FROZEN = "fixtures/frozen"
CURRENT = "fixtures/current"
# The subdirectory of the checkout's target that a rerun builds into.
RERUN_TARGET = "freeze-provenance"
# The live corpus, rewritten at the commit the frozen bytes were taken from.
REGENERATE = "cargo run --quiet -p ambre-logic --features fixtures --bin ambre-fixtures -- generate"
# The corpus driver reads and writes the corpus by design, so it is the one
# `fixtures.rs` that holds no generator.
FIXTURE_DRIVER = "crates/ambre-logic/src/fixtures.rs"
# Naming any of these reads something besides the seed: a file, or the host.
FILE_READS = re.compile(
    r"\bstd::fs\b|\bfs::|\bFile::|\bOpenOptions\b|\binclude_bytes!|\binclude_str!"
    r"|\bread_to_string\b|\bstd::env\b|\benv::var"
)


def git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)


def sha(root: Path, rev: str) -> str | None:
    """The commit `rev` names, or `None` when the history does not hold it."""
    done = git(root, "rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}")
    return done.stdout.strip() or None


def resolve(root: Path, base: str) -> str:
    """The revision whose artifacts are taken as checked: the base, or its
    parent when the branch is the base itself."""
    resolved = sha(root, base)
    if resolved is None:
        raise LookupError(base)
    if resolved == sha(root, "HEAD"):
        parent = sha(root, f"{base}~1")
        if parent is None:
            raise LookupError(f"{base}~1")
        return parent
    return resolved


def blob(root: Path, rev: str, rel: str) -> str | None:
    """The object id of `rel` at `rev`, or `None` when the revision lacks it."""
    done = git(root, "rev-parse", "--verify", "--quiet", f"{rev}:{rel}")
    return done.stdout.strip() or None


def text_at(root: Path, rev: str, rel: str) -> str | None:
    done = git(root, "show", f"{rev}:{rel}")
    return done.stdout if done.returncode == 0 else None


def pair(manifest: dict) -> tuple[int, int]:
    """The version pair a manifest names: the container, then the schema."""
    return (manifest["ambre_version"], manifest["schema_version"])


def manifest_at(root: Path, rev: str, rel: str) -> dict | None:
    text = text_at(root, rev, rel)
    return json.loads(text) if text is not None else None


def frozen_artifacts(root: Path) -> list[str]:
    return sorted(p.relative_to(root).as_posix() for p in (root / FROZEN).glob("*/*.ambre"))


def freeze_commit(root: Path, rel: str) -> str | None:
    """The newest commit that adds `rel`."""
    done = git(root, "log", "--diff-filter=A", "--format=%H", "-n1", "--", rel)
    return done.stdout.strip() or None


def source_commit(root: Path, parent: str, live: str, want: str) -> str | None:
    """The newest commit, at or before `parent`, whose live artifact `live`
    holds the object `want`."""
    if blob(root, parent, live) == want:
        return parent
    # Newest first, each commit that put the object in or took it out. The
    # newest holder is the one that put it in, or the parent of one that took
    # it out.
    done = git(root, "log", "--format=%H", f"--find-object={want}", parent, "--", live)
    for commit in done.stdout.split():
        if blob(root, commit, live) == want:
            return commit
        before = sha(root, f"{commit}~1")
        if before is not None and blob(root, before, live) == want:
            return before
    return None


def generator_modules(paths: list[str]) -> list[str]:
    """The paths that hold a generator: each `fixtures.rs` of a crate's source,
    the driver aside. Read off the tree rather than listed, so a generator a new
    application adds is read without anyone naming it here."""
    return sorted(
        p
        for p in paths
        if p.startswith("crates/") and "/src/" in p and p.endswith("/fixtures.rs") and p != FIXTURE_DRIVER
    )


def file_reads(source: str) -> list[tuple[int, str]]:
    """Each line of a generator module that reads besides its seed, with what it
    names. The test module is not read: it builds no artifact, and a comment
    that cites a rule is no read."""
    found = []
    for number, line in enumerate(source.splitlines(), 1):
        if line.strip() == "#[cfg(test)]":
            break
        hit = FILE_READS.search(line.split("//", 1)[0])
        if hit:
            found.append((number, hit.group(0)))
    return found


def generator_reads(root: Path, rev: str) -> list[str]:
    """Each read besides the seed in the generator modules of `rev`."""
    paths = git(root, "ls-tree", "-r", "--name-only", rev, "--", "crates").stdout.split()
    found = []
    for rel in generator_modules(paths):
        for line, token in file_reads(text_at(root, rev, rel) or ""):
            found.append(f"{rel}:{line}: `{token}`")
    return found


def rerun_target(root: Path, env: dict[str, str]) -> Path:
    """The target directory of a rerun: a directory of its own, under the
    target of the checkout.

    Cargo takes a binary that a worktree build leaves in a shared target as
    fresh for the checkout. A rerun that built into the checkout's own target
    would leave there a binary of an old commit, anchored to a worktree that is
    gone. The next `just fixtures-check` would then run that binary unbuilt.
    The directory persists, so a later rerun builds incrementally.
    """
    return Path(env.get("CARGO_TARGET_DIR") or root / "target") / RERUN_TARGET


def regenerate(root: Path, rev: str, live: str, command: str) -> bytes | str:
    """The live artifact `live` as the generator of `rev` rebuilds it, or why
    it could not be rebuilt."""
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(rerun_target(root, env))
    with tempfile.TemporaryDirectory() as tmp:
        tree = Path(tmp) / "tree"
        added = git(root, "worktree", "add", "--detach", str(tree), rev)
        if added.returncode != 0:
            return f"no worktree at {rev[:12]}: {added.stderr.strip()}"
        try:
            done = subprocess.run(
                shlex.split(command), cwd=tree, env=env, capture_output=True, text=True
            )
            if done.returncode != 0:
                return f"the generator at {rev[:12]} failed: {done.stderr.strip()[-400:]}"
            out = tree / live
            return out.read_bytes() if out.exists() else f"the generator at {rev[:12]} wrote no {live}"
        finally:
            git(root, "worktree", "remove", "--force", str(tree))


def doc_types_at(root: Path, rev: str, under: str, rels: list[str] | None = None) -> set[str]:
    """The document types the manifests under `under` at `rev` name."""
    if rels is None:
        rels = git(root, "ls-tree", "-r", "--name-only", rev, "--", under).stdout.split()
    types = set()
    for rel in rels:
        if rel.endswith(".manifest"):
            manifest = manifest_at(root, rev, rel)
            if manifest is not None:
                types.add(manifest["doc_type"])
    return types


def check_artifact(root: Path, rel: str, command: str) -> tuple[list[str], list[str]]:
    """The errors and the notes of one frozen artifact."""
    errors: list[str] = []
    notes: list[str] = []
    frozen = json.loads((root / (rel.removesuffix(".ambre") + ".manifest")).read_text())
    live = f"{CURRENT}/{frozen['generator']}.ambre"
    live_manifest = f"{CURRENT}/{frozen['generator']}.manifest"
    want = git(root, "hash-object", rel).stdout.strip()

    commit = freeze_commit(root, rel)
    if commit is None:
        return [f"{rel}: no commit adds it, so nothing shows where it came from"], notes
    parent = sha(root, f"{commit}~1")
    if parent is None:
        return [
            f"{rel}: the history stops at its freeze commit {commit[:12]}, so the outgoing "
            "writer cannot be read. Run `git fetch --unshallow origin`."
        ], notes
    if blob(root, commit, rel) != want:
        errors.append(f"{rel}: changed since its freeze at {commit[:12]} (FIXTURE-018)")

    # The freeze is the bump (`DOCFILE-008`).
    before = manifest_at(root, parent, live_manifest)
    after = manifest_at(root, commit, live_manifest)
    if before is None or pair(before) != pair(frozen):
        was = "no live artifact" if before is None else f"version {pair(before)}"
        errors.append(
            f"{rel}: the writer before {commit[:12]} emitted {was}, not the frozen "
            f"{pair(frozen)}, so the freeze is not in the bump commit (DOCFILE-008)"
        )
    if after is None or pair(after) <= pair(frozen):
        was = "no live artifact" if after is None else f"version {pair(after)}"
        errors.append(
            f"{rel}: {commit[:12]} leaves {was}, raising no version past the frozen "
            f"{pair(frozen)}, so the freeze is in no bump commit (DOCFILE-008)"
        )
    elif after["ambre_version"] > frozen["ambre_version"]:
        added = git(root, "diff-tree", "--no-commit-id", "--name-only", "--diff-filter=A", "-r",
                    commit, "--", FROZEN).stdout.split()
        frozen_types = doc_types_at(root, commit, FROZEN, added)
        for doc_type in sorted(doc_types_at(root, parent, CURRENT) - frozen_types):
            errors.append(
                f"{rel}: the container bump at {commit[:12]} freezes no {doc_type} artifact "
                "(DOCFILE-008)"
            )

    # The bytes came from a generator run from a seed (`FIXTURE-005`).
    source = source_commit(root, parent, live, want)
    if source is None:
        errors.append(
            f"{rel}: its bytes never sat in {live} before {commit[:12]}, so no generator "
            "run from a seed wrote them (FIXTURE-005)"
        )
        return errors, notes
    if source != parent:
        notes.append(
            f"{rel}: taken from the writer at {source[:12]}, older than the outgoing "
            f"writer at {parent[:12]}"
        )
    seeded = manifest_at(root, source, live_manifest)
    if seeded is None or (seeded["seed"], seeded["generator_version"]) != (
        frozen["seed"],
        frozen["generator_version"],
    ):
        errors.append(
            f"{rel}: the live manifest at {source[:12]} names another seed or generator "
            "version than the frozen one (FIXTURE-005)"
        )

    # The generator read nothing besides its seed (`FIXTURE-032`).
    for read in generator_reads(root, source):
        errors.append(f"{rel}: the generator at {source[:12]} reads besides its seed: {read} (FIXTURE-032)")

    rebuilt = regenerate(root, source, live, command)
    if isinstance(rebuilt, str):
        errors.append(f"{rel}: {rebuilt} (FIXTURE-005)")
    elif rebuilt != (root / rel).read_bytes():
        errors.append(
            f"{rel}: the generator at {source[:12]} rebuilds other bytes from its seed, "
            "so the frozen ones are no run of it (FIXTURE-005)"
        )
    return errors, notes


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        default="origin/main",
        help="the revision whose frozen artifacts are taken as checked (default: origin/main)",
    )
    parser.add_argument("--all", action="store_true", help="check every frozen artifact")
    parser.add_argument("--regenerate", default=REGENERATE, help="the command that rewrites the live corpus")
    args = parser.parse_args(argv)

    base = None
    if not args.all:
        try:
            base = resolve(root, args.base)
        except LookupError as missing:
            print(
                f"ERROR: cannot read `{missing.args[0]}`, so no frozen artifact is checked and "
                "nothing is proved. Fetch the history first: `git fetch --unshallow origin`, "
                "and CI sets `fetch-depth: 0` on the checkout step.",
                file=sys.stderr,
            )
            return 1

    errors: list[str] = []
    checked = 0
    taken = 0
    for rel in frozen_artifacts(root):
        if base is not None and blob(root, base, rel) is not None:
            taken += 1
            continue
        found, notes = check_artifact(root, rel, args.regenerate)
        checked += 1
        print(f"{rel}: {'refused' if found else 'traced to its seed'}")
        for note in notes:
            print(f"  note: {note}")
        errors.extend(found)

    on = "every artifact" if base is None else f"base {base[:12]}"
    print(f"\n{on}: {checked} frozen artifacts checked at their freeze commit, {taken} taken as checked")
    for line in errors:
        print(f"  {line}")
    if errors:
        print("FAIL freeze_provenance.py", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
