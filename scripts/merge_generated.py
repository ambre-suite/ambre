#!/usr/bin/env python3
"""The merge driver of the generated spec files: it regenerates them, never merges them.

The contract ledger, the contract index, the contract graph and the inventories
are committed outputs of `contracts.py`, `contract_index.py` and
`feature_inventory.py`. Two branches that each add a contract row both change
them, so a text merge conflicts on four or five files. Neither side is right: the
right content is what the generators write from the merged sources.

`.gitattributes` names each such file with `merge=ambre-generated`. Git calls
this script for such a file when both sides changed it, with the arguments
`%O %A %B %P`, and keeps what the script leaves in `%A`. Install it once per
clone, as `CONTRIBUTING.md` section 3 states:

    git config merge.ambre-generated.driver "python3 scripts/merge_generated.py %O %A %B %P"

**Git merges in memory, so the work tree holds our sources, not the merged
ones.** The script therefore rebuilds the merged sources itself. It names the
other side, merges the two commits with `git merge-tree --write-tree`, extracts
that tree into a temporary directory and runs the three generators there, with
`--write`. The outputs of one merged tree are kept under the git directory, so
the second to fifth file of one merge cost no second run.

The other side is known in two cases:

  * `git merge` and `git pull` set `GITHEAD_<commit>` for each merged head.
  * `git rebase` writes the commit it picks as the last line of
    `rebase-merge/done`, and the merge base is the parent of that commit.

Elsewhere -- a single `git cherry-pick`, `git revert`, `git am -3`, an octopus
merge -- git names no other commit. A source conflict also leaves no merged
sources to read. In each of these cases the script keeps our side, exits 0 and
says on standard error which commands regenerate the files. `just contracts`,
`just index` and `just inventory` then refuse a stale file, so a wrong file
never passes a gate.

Run by git only. `test_merge_generated.py` merges two real branches of this
repository through it.
"""

from __future__ import annotations

import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

# The driver's name in `.gitattributes` and in `git config merge.<name>.driver`.
DRIVER = "ambre-generated"

# Set for the nested `git merge-tree`: there the driver keeps our side at once,
# because the outer run regenerates every generated file of that tree anyway.
NESTED = "AMBRE_MERGE_GENERATED_NESTED"

# The generators, run in the merged tree in this order. Each one writes its
# files unconditionally under `--write`, so a file absent after the run means a
# generator that failed before it wrote.
GENERATORS = (
    "scripts/contracts.py",
    "scripts/contract_index.py",
    "scripts/feature_inventory.py",
)

# What the three generators write, and nothing else. `.gitattributes` names the
# same set, and `test_merge_generated.py` holds the two together.
GENERATED = (
    re.compile(r"^specs/1-Foundations/ambre-contract-(ledger\.md|index\.json|graph\.json)$"),
    re.compile(r"^specs/[^/]+/ambre-[a-z]+-inventory\.md$"),
)

# The regeneration a person runs when the driver could not.
REGENERATE = "just contracts-write && just index-write && just inventory-write"

GITHEAD = re.compile(r"^GITHEAD_([0-9a-f]{40}|[0-9a-f]{64})$")


def is_generated(path: str) -> bool:
    """Whether a repository path is one of the files the generators write."""
    return any(pattern.match(path) for pattern in GENERATED)


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    """One git call in the current repository, its output decoded."""
    return subprocess.run(["git", *args], capture_output=True, text=True, check=check)


def git_dir() -> Path:
    """The git directory of the work tree git runs the driver in."""
    return Path(git("rev-parse", "--absolute-git-dir").stdout.strip())


def other_side(environ: dict[str, str], gitdir: Path) -> tuple[str, str | None] | None:
    """The commit merged into HEAD, and the merge base when git imposes one.

    `None` when git names no single other commit: the merged sources are then
    out of reach.
    """
    heads = [m.group(1) for name in environ if (m := GITHEAD.match(name))]
    if len(heads) == 1:
        return heads[0], None
    if heads:
        return None  # an octopus merge: no two-way merged tree answers it
    done = gitdir / "rebase-merge" / "done"
    if not done.is_file():
        return None
    lines = [line.split() for line in done.read_text().splitlines() if line.strip()]
    # `pick`, `reword`, `edit`, `squash` and `fixup` name the picked commit
    # second. Any other command (`merge`, `exec`, `label`) carries none.
    if not lines or len(lines[-1]) < 2 or lines[-1][0] not in (
        "pick", "p", "reword", "r", "edit", "e", "squash", "s", "fixup", "f"
    ):
        return None
    picked = git("rev-parse", "--verify", "--quiet", f"{lines[-1][1]}^{{commit}}", check=False)
    parent = git("rev-parse", "--verify", "--quiet", f"{lines[-1][1]}^", check=False)
    if picked.returncode or parent.returncode:
        return None  # a root commit has no parent to serve as the merge base
    return picked.stdout.strip(), parent.stdout.strip()


def merged_tree(theirs: str, base: str | None) -> str | None:
    """The tree of HEAD merged with `theirs`, or `None` on a source conflict."""
    args = ["merge-tree", "--write-tree", "--no-messages"]
    if base is not None:
        args.append(f"--merge-base={base}")
    env = {**os.environ, NESTED: "1"}
    run = subprocess.run(
        ["git", *args, "HEAD", theirs], capture_output=True, text=True, env=env
    )
    if run.returncode != 0:
        return None  # 1 is a conflict, anything else an error: no tree to read
    return run.stdout.split("\n", 1)[0].strip()


def regenerate(tree: str, cache: Path) -> None:
    """Write into `cache` every generated file of `tree`, as the generators give it."""
    archive = subprocess.run(["git", "archive", "--format=tar", tree], capture_output=True, check=True)
    with tempfile.TemporaryDirectory(prefix="ambre-merge-") as scratch:
        root = Path(scratch)
        with tarfile.open(fileobj=io.BytesIO(archive.stdout)) as tar:
            tar.extractall(root, filter="data")
        stale = [p for p in root.rglob("*") if is_generated(p.relative_to(root).as_posix())]
        for path in stale:
            path.unlink()
        for generator in GENERATORS:
            # A generator exits 1 on a defect of the merged corpus and still
            # writes: the gates report the defect after the merge.
            subprocess.run(
                [sys.executable, str(root / generator), "--write"],
                cwd=root, capture_output=True, check=False,
            )
        if cache.exists():
            shutil.rmtree(cache)
        for path in root.rglob("*"):
            rel = path.relative_to(root).as_posix()
            if path.is_file() and is_generated(rel):
                target = cache / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target)
        (cache / ".complete").touch()


def resolve(path: str, environ: dict[str, str]) -> bytes | None:
    """The merged content of the generated file `path`, or `None` to keep ours."""
    gitdir = git_dir()
    side = other_side(environ, gitdir)
    if side is None:
        print(f"{DRIVER}: git names no other commit here, so {path} keeps our side. "
              f"Run `{REGENERATE}` and commit the result.", file=sys.stderr)
        return None
    tree = merged_tree(*side)
    if tree is None:
        print(f"{DRIVER}: a source file conflicts, so {path} keeps our side. "
              f"Resolve the sources, then run `{REGENERATE}`.", file=sys.stderr)
        return None
    store = gitdir / "ambre-merge-generated"
    cache = store / tree
    if not (cache / ".complete").exists():
        # One merged tree at a time: an earlier merge's outputs are dead.
        if store.exists():
            shutil.rmtree(store)
        regenerate(tree, cache)
    result = cache / path
    if not result.is_file():
        print(f"{DRIVER}: no generator wrote {path} in the merged tree, so it keeps "
              f"our side. Run `{REGENERATE}` and read the error.", file=sys.stderr)
        return None
    return result.read_bytes()


def main(argv: list[str]) -> int:
    if len(argv) != 5:
        print(f"usage: {argv[0]} %O %A %B %P (run by git as a merge driver)", file=sys.stderr)
        return 2
    _, _ancestor, ours, _theirs, path = argv
    if os.environ.get(NESTED):
        return 0  # `%A` already holds our side, which is what the nested merge keeps
    merged = resolve(path, dict(os.environ))
    if merged is not None:
        Path(ours).write_bytes(merged)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
