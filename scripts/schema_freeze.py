#!/usr/bin/env python3
"""The frozen schema sections of `crates/*/schema/*.txt`, held append-only.

Each application freezes, under its compiled `SCHEMA_VERSION`, the key set and
the value types a build writes (`SCHEMA-018`), in `shapes.txt`. It also freezes
the identity its scaffolding is written under (`AUTHOR-024`), in
`scaffold.txt`, and what each migration step writes (`AUTHOR-024`), in
`migrations.txt`. Every file below says "shape" for all three. A belt
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

**A freeze with no belt holds nothing**, and a belt is what makes `SCHEMA-009`
falsifiable. The mechanism of that rule is a list of three test paths, one per
application, which a fourth application does not join by existing. So the
pairing is read here rather than trusted: a crate that freezes a shape carries
the belt that compares it, a crate that stamps a `SCHEMA_VERSION` freezes a
shape, and the compiled version has a section to be compared against.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BELT = "the_shape_a_build_writes_is_the_shape_frozen_for_its_version"
SCAFFOLD_BELT = "the_scaffold_a_build_writes_is_the_scaffold_frozen_for_its_version"
MIGRATION_BELT = "each_migration_step_writes_what_its_version_froze"
# Each frozen file, and the belt that compares a build against it.
FREEZES = {
    "shapes.txt": BELT,
    "scaffold.txt": SCAFFOLD_BELT,
    "migrations.txt": MIGRATION_BELT,
}
# `== v3`: the line that opens a section. Everything above the first one is the
# file's comment header, which explains the format and is free to be reworded.
MARK = "== "


def sections(text: str) -> dict[str, str]:
    """The body of each `== vN` section, keyed by its marker line.

    The blank lines that close a body are the separator before the next
    marker, not part of the section: appending a section must not read as an
    edit of the one above it.
    """
    out: dict[str, str] = {}
    name = None
    body: list[str] = []
    for line in text.splitlines():
        if line.startswith(MARK):
            if name is not None:
                out[name] = "\n".join(body).rstrip("\n")
            name = line.strip()
            body = []
        elif name is not None:
            body.append(line)
    if name is not None:
        out[name] = "\n".join(body).rstrip("\n")
    return out


def frozen_files(base: str) -> list[Path]:
    """Every shape file to read: the ones on disk, plus the ones the base has.

    Globbing the disk alone would let a deleted file pass unseen, which is the
    loudest way to edit a section.
    """
    out = {p for name in FREEZES for p in ROOT.glob(f"crates/*/schema/{name}")}
    done = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", base, "crates/"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    for line in done.stdout.splitlines():
        if any(line.endswith(f"/schema/{name}") for name in FREEZES):
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


# `SCHEMA_VERSION` alone: the protocol's `SCHEMA_VERSION_KEY` and
# `SCHEMA_VERSION_ABSENT` are not a stamped number.
STAMP = re.compile(r"^pub const SCHEMA_VERSION\s*:\s*u32\s*=\s*(\d+)", re.MULTILINE)


def crate_of(path: Path) -> str:
    """The crate directory a path sits under."""
    return path.relative_to(ROOT).parts[1]


def stamped_crates() -> dict[str, int]:
    """Each crate declaring a `SCHEMA_VERSION`, and the number it declares."""
    out: dict[str, int] = {}
    for path in ROOT.glob("crates/*/src/**/*.rs"):
        found = STAMP.search(path.read_text())
        if found:
            out[crate_of(path)] = int(found.group(1))
    return out


def check_belts(errors: list[str]) -> int:
    """Every freeze paired with its belt, and every stamp with its freeze.

    Neither half is redundant. A shape file with no belt is a text nothing
    compares a build against. A stamped crate with no shape file is an
    application that raises a number no layout is bound to. Each frozen file
    is read on its own, with its own belt.
    """
    stamped = stamped_crates()
    read: set[str] = set(stamped)
    for name, belt in FREEZES.items():
        frozen = {crate_of(p): p for p in ROOT.glob(f"crates/*/schema/{name}")}
        read |= set(frozen)
        for crate, path in sorted(frozen.items()):
            belts = list((ROOT / "crates" / crate / "src").rglob("*.rs"))
            if not any(belt in p.read_text() for p in belts):
                errors.append(
                    f"crates/{crate}: a frozen `{name}` with no belt. Nothing "
                    f"compares a build against it, so add `{belt}`"
                )
            version = stamped.get(crate)
            if version is None:
                errors.append(
                    f"crates/{crate}: a frozen `{name}` in a crate that stamps no version"
                )
            elif f"{MARK}v{version}" not in path.read_text():
                errors.append(
                    f"crates/{crate}: `SCHEMA_VERSION` is {version} and `{name}` "
                    f"carries no `{MARK}v{version}` section to compare against"
                )

        for crate in sorted(set(stamped) - set(frozen)):
            errors.append(
                f"crates/{crate}: a stamped `SCHEMA_VERSION` with no frozen "
                f"`{name}`. A number that binds nothing raises on nothing"
            )
    return len(read)


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

    errors: list[str] = []
    paired = check_belts(errors)

    print(f"\nbase {base[:12]}: {held} sections held, {added} new, "
          f"{len(edited)} edited, {len(dropped)} dropped; "
          f"{paired} crates read for a shape and its belt")
    for line in errors:
        print(f"  {line}")
    for line in edited:
        print(f"  edited  {line}")
    for line in dropped:
        print(f"  dropped {line}")

    if args.check and errors:
        print(
            "\nERROR: a shape freeze and its belt came apart. The freeze is only "
            "worth the test that compares a build against it, and `SCHEMA-009` "
            "rests on that pairing holding for every application, including the "
            "next one.",
            file=sys.stderr,
        )
        return 1
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
