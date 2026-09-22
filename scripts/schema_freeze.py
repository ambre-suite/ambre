#!/usr/bin/env python3
"""The frozen schema sections of `crates/*/schema/shapes.txt`, held append-only.

Each application freezes, under its compiled `SCHEMA_VERSION`, the key set and
the value types a build writes (`PROSE-029`, `TRAME-026`, `TOILE-028`). A belt
in the crate compares a build against that text. The belt alone is weak: a
developer whose build moved the shape can regenerate the section in place and
watch the belt go green, with the version never raised and every peer on the
old version now reading a document it cannot parse.

**A section is the record of what one version wrote**, so it is append-only.
This check reads the same file on a base revision and refuses any section of
that revision whose body differs today, or which is gone. A version the base
does not carry is new, and free.

**The base, not the whole history.** Replaying every commit of the file would
be the literal reading of the rule and would cost a clone of the repository.
The base revision is the line that matters: below it a section is published and
peers hold documents written under it, above it the section is still a draft of
the branch that is writing it. `--base` names the revision, `origin/main` by
default; a push of the base branch itself falls back to its parent, so a commit
landing straight on it is read too.

**A check that cannot read the base proves nothing**, so a missing revision is
an error naming the fetch that fixes it, never a silent pass.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GLOB = "crates/*/schema/shapes.txt"
# `== v3`: the line that opens a section. Everything above the first one is the
# file's comment header, which explains the format and is free to be reworded.
MARK = "== "


def sections(text: str) -> dict[str, str]:
    """The body of each `== vN` section, keyed by its marker line."""
    out: dict[str, str] = {}
    name = None
    body: list[str] = []
    for line in text.splitlines():
        if line.startswith(MARK):
            if name is not None:
                out[name] = "\n".join(body)
            name = line.strip()
            body = []
        elif name is not None:
            body.append(line)
    if name is not None:
        out[name] = "\n".join(body)
    return out


def frozen_files(base: str) -> list[Path]:
    """Every shape file to read: the ones on disk, plus the ones the base has.

    Globbing the disk alone would let a deleted file pass unseen, which is the
    loudest way to edit a section.
    """
    out = set(ROOT.glob(GLOB))
    done = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", base, "crates/"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    for line in done.stdout.splitlines():
        if line.endswith("/schema/shapes.txt"):
            out.add(ROOT / line)
    return sorted(out)


def at(revision: str, path: Path) -> str | None:
    """The file's text at a revision, or `None` when the revision lacks it."""
    rel = path.relative_to(ROOT).as_posix()
    done = subprocess.run(
        ["git", "show", f"{revision}:{rel}"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    return done.stdout if done.returncode == 0 else None


def resolve(base: str) -> str:
    """The revision to read the published sections from.

    A branch that is the base itself carries no diff against it, so the parent
    commit is read instead: a section edited by a commit landing straight on
    the base branch is then caught like any other.
    """
    def sha(rev: str) -> str | None:
        done = subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}"],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        return done.stdout.strip() or None

    resolved = sha(base)
    if resolved is None:
        raise LookupError(base)
    if resolved == sha("HEAD"):
        parent = sha(f"{base}~1")
        if parent is None:
            raise LookupError(f"{base}~1")
        return parent
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit non-zero on a finding")
    parser.add_argument(
        "--base",
        default="origin/main",
        help="the revision holding the published sections (default: origin/main)",
    )
    args = parser.parse_args()

    try:
        base = resolve(args.base)
    except LookupError as missing:
        print(
            f"ERROR: cannot read `{missing.args[0]}`, so no section is checked and "
            "nothing is proved. Fetch the base branch first: a shallow clone has "
            "to run `git fetch --depth=2 origin main`, and CI sets `fetch-depth` "
            "on the checkout step.",
            file=sys.stderr,
        )
        return 1

    edited: list[str] = []
    dropped: list[str] = []
    held = 0
    added = 0
    for path in frozen_files(base):
        rel = path.relative_to(ROOT).as_posix()
        was = at(base, path)
        if was is None:
            print(f"{rel}: new since the base, {len(sections(path.read_text()))} sections")
            continue
        now = sections(path.read_text()) if path.exists() else {}
        for name, body in sections(was).items():
            if name not in now:
                dropped.append(f"{rel} {name}")
            elif now[name] != body:
                edited.append(f"{rel} {name}")
            else:
                held += 1
        new = set(now) - set(sections(was))
        added += len(new)
        gone = "" if path.exists() else ", gone from the tree"
        print(f"{rel}: {len(sections(was))} published, {len(new)} new{gone}")

    print(f"\nbase {base[:12]}: {held} sections held, {added} new, "
          f"{len(edited)} edited, {len(dropped)} dropped")
    for line in edited:
        print(f"  edited  {line}")
    for line in dropped:
        print(f"  dropped {line}")

    if args.check and (edited or dropped):
        print(
            "\nERROR: a frozen shape section changed after it was published. A "
            "section is the record of what one `SCHEMA_VERSION` wrote, and peers "
            "hold documents written under it: editing it makes the belt green "
            "against a shape no reader agrees with. Restore the section, raise "
            "`SCHEMA_VERSION`, and append a new one below it.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
