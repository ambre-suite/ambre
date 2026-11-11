#!/usr/bin/env python3
"""One toolchain for the whole repository: the dated nightly of `rust-toolchain.toml`.

`ARCH-075` states the rule, and `ambre-spec-architecture.md section 2`, the
toolchain, states why and how a bump moves the date. The gate reads every file
the repository tracks outside `vendor/` and fails on each place that names a
toolchain or a channel anywhere but the root `rust-toolchain.toml`:

* a second toolchain file, at any depth: a nested workspace such as `fuzz/`
  inherits the root file through the directory lookup of rustup;
* a `+toolchain` argument, `RUSTUP_TOOLCHAIN`, `--toolchain` or a rustup verb
  that selects or installs a toolchain, in a script, a recipe, a workflow, a
  manifest, a non-test Rust file or a shell fence of a Markdown file;
* a `toolchain:` input or an action that installs Rust, in a workflow;
* a `rust-version` key in a manifest, since a pinned nightly is the one
  version the workspace builds with;
* a Rust container image, which carries a compiler of its own.

It also fails when the root file pins no dated nightly: `channel` must read
`nightly-YYYY-MM-DD`. In Markdown, a shell fence is read whole, and an inline
code span is read when it starts with a command word: `cargo +nightly build`
is a command, while a sentence that quotes `+nightly` alone names a pattern.

**A bump is a story of its own.** The gate reads the pin at the base revision,
`origin/main` by default, or its parent on a push of `main` itself, as
`schema_freeze.py` does. When the date moved since the base, a story row of
`specs/ambre-roadmap.md` or of the journal must name the new
`nightly-YYYY-MM-DD`. A base the gate cannot read is an error, never a pass.

A Rust test file, `*tests.rs` or a file under a
`tests/` directory, may quote a command line as data. It is held to what
selects a toolchain when a test spawns a process.

Run: `just toolchain-pin`. It writes nothing.
"""

from __future__ import annotations

import argparse
import datetime
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from schema_freeze import resolve  # noqa: E402  -- one definition of the base revision

ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN = "rust-toolchain.toml"
TOOLCHAIN_NAMES = {"rust-toolchain", "rust-toolchain.toml"}

# This gate and its tests quote every pattern they refuse.
SELF = {"scripts/toolchain_pin.py", "scripts/test_toolchain_pin.py"}

CHANNEL = re.compile(r'^channel\s*=\s*"nightly-(\d{4}-\d{2}-\d{2})"\s*$', re.MULTILINE)

COMMAND = [
    (re.compile(r"(?<![\w-])\+(nightly|stable|beta)\b"), "a `+channel` argument"),
    (re.compile(r"\b(cargo|rustc|rustdoc)\s+\+\S"), "a `+toolchain` argument"),
    (re.compile(r"\bRUSTUP_TOOLCHAIN\b"), "a `RUSTUP_TOOLCHAIN` override"),
    (re.compile(r"--toolchain\b"), "a `--toolchain` flag"),
    (
        re.compile(r"\brustup\s+(default|override|toolchain|run|update|install)\b"),
        "a rustup verb that selects or installs a toolchain",
    ),
    (re.compile(r"(\bimage:|^\s*FROM)\s+\S*\brust:"), "a Rust container image"),
]

WORKFLOW = [
    (re.compile(r"^\s*(-\s+)?toolchain\s*:"), "a `toolchain:` input"),
    (
        re.compile(r"\buses:\s*\S*(rust-toolchain|actions-rs/toolchain|setup-rust)"),
        "an action that installs a toolchain",
    ),
]

# A Rust test file may quote a command line as the data it checks, so it is held
# to what selects a toolchain when the test spawns a process: the override, the
# flag, the rustup verbs, and a `"+toolchain"` argument passed on its own.
TEST = [
    COMMAND[2],
    COMMAND[3],
    COMMAND[4],
    (re.compile(r'"\+(nightly|stable|beta|\d+\.\d+)[\w.-]*"'), "a `+toolchain` argument"),
]

MANIFEST = [(re.compile(r"^\s*rust-version\b"), "a `rust-version` key")]

SCRIPT_SUFFIXES = {".toml", ".yml", ".yaml", ".sh", ".py", ".mjs", ".js", ".rs"}
SCRIPT_NAMES = {"justfile", "Dockerfile"}
SHELL_FENCES = {"sh", "bash", "shell", "console", "zsh"}
FENCE = re.compile(r"^\s*```+\s*([\w-]*)")

# An inline code span of Markdown, and the words that make one a command line.
INLINE = re.compile(r"`([^`\n]+)`")
COMMAND_WORD = re.compile(r"^\s*((cargo|rustc|rustdoc|rustup|trunk|just|env)(\s|$)|RUSTUP_TOOLCHAIN=)")

# The plan files, where the story of a bump lives, and the shape of a story row.
PLANS = ("specs/ambre-roadmap.md", "specs/ambre-roadmap-journal.md")
STORY_ROW = re.compile(r"^\|\s*`\[[ x]\]`")


def is_test_file(path: str) -> bool:
    """A Rust test file, which may quote a toolchain as the data it checks."""
    return path.endswith("tests.rs") or "/tests/" in f"/{path}"


def patterns(path: str) -> list[tuple[re.Pattern[str], str]]:
    """The patterns one file is held to, from its place and its name. Empty: not read."""
    name = Path(path).name
    suffix = Path(path).suffix
    if path in SELF or path == TOOLCHAIN:
        return []
    if suffix == ".rs" and is_test_file(path):
        return TEST
    found: list[tuple[re.Pattern[str], str]] = []
    if path.startswith(".github/") and suffix in {".yml", ".yaml"}:
        found += WORKFLOW
    if name == "Cargo.toml":
        found += MANIFEST
    if (
        suffix in SCRIPT_SUFFIXES
        or name in SCRIPT_NAMES
        or name.startswith("Dockerfile")
        or path.startswith((".githooks/", ".cargo/"))
    ):
        found += COMMAND
    return found


def shell_lines(text: str) -> list[tuple[int, str]]:
    """The numbered lines of a Markdown file that sit inside a shell fence."""
    lines: list[tuple[int, str]] = []
    inside = False
    shell = False
    for number, line in enumerate(text.splitlines(), 1):
        match = FENCE.match(line)
        if match:
            if inside:
                inside = False
            else:
                inside = True
                shell = match.group(1).lower() in SHELL_FENCES
            continue
        if inside and shell:
            lines.append((number, line))
    return lines


def inline_commands(text: str) -> list[tuple[int, str]]:
    """The numbered inline code spans of a Markdown file, outside a fence, that start with a command."""
    spans: list[tuple[int, str]] = []
    inside = False
    for number, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            inside = not inside
            continue
        if inside:
            continue
        for span in INLINE.findall(line):
            if COMMAND_WORD.match(span):
                spans.append((number, span))
    return spans


def is_read(path: str) -> bool:
    """Whether the gate reads one tracked file at all."""
    return Path(path).name in TOOLCHAIN_NAMES or path.endswith(".md") or bool(patterns(path))


def file_breaches(path: str, text: str) -> list[str]:
    """Each line of one tracked file that names a toolchain or a channel."""
    if Path(path).name in TOOLCHAIN_NAMES and path != TOOLCHAIN:
        return [f"{path}: a second toolchain file; the root `{TOOLCHAIN}` is the only one"]
    if path.endswith(".md"):
        held, lines = COMMAND, shell_lines(text) + inline_commands(text)
    else:
        held, lines = patterns(path), list(enumerate(text.splitlines(), 1))
    out = []
    for number, line in lines:
        for pattern, reason in held:
            if pattern.search(line):
                out.append(f"{path}:{number}: {reason}: {line.strip()}")
                break
    return out


def pin_breaches(text: str | None) -> list[str]:
    """Whether the root toolchain file pins a dated nightly."""
    if text is None:
        return [f"{TOOLCHAIN}: missing at the root"]
    dates = CHANNEL.findall(text)
    if len(dates) != 1:
        return [f'{TOOLCHAIN}: `channel` must read one "nightly-YYYY-MM-DD"']
    try:
        datetime.date.fromisoformat(dates[0])
    except ValueError:
        return [f"{TOOLCHAIN}: `{dates[0]}` is not a date"]
    return []


def date_of(text: str | None) -> str | None:
    """The date one toolchain file pins, or None when it pins no single dated nightly."""
    dates = CHANNEL.findall(text or "")
    return dates[0] if len(dates) == 1 else None


def bump_breaches(base: str | None, head: str | None, plans: list[str]) -> list[str]:
    """Whether a date moved since the base without a story row that names it.

    `plans` holds the text of the roadmap and the journal at the head.
    """
    was, now = date_of(base), date_of(head)
    if was is None or now is None or was == now:
        return []
    named = f"nightly-{now}"
    for text in plans:
        for line in text.splitlines():
            if STORY_ROW.match(line) and named in line:
                return []
    return [
        f"{TOOLCHAIN}: the date moved from {was} to {now}, and no story row of "
        f"`{PLANS[0]}` names `{named}`; a bump is a story of its own"
    ]


def at(revision: str, path: str) -> str | None:
    """One file's text at a revision, or None when the revision lacks it."""
    done = subprocess.run(
        ["git", "show", f"{revision}:{path}"], cwd=ROOT, capture_output=True, text=True
    )
    return done.stdout if done.returncode == 0 else None


def plans() -> list[str]:
    """The text of each plan file in the work tree."""
    return [(ROOT / p).read_text(encoding="utf-8") for p in PLANS if (ROOT / p).is_file()]


def files() -> list[tuple[str, str]]:
    """Each file git tracks or would track, outside `vendor/`, with its text."""
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout.decode()
    out = []
    for path in sorted(set(filter(None, listed.split("\0")))):
        if path.startswith("vendor/"):
            continue
        full = ROOT / path
        if not full.is_file():
            continue
        try:
            out.append((path, full.read_text(encoding="utf-8")))
        except UnicodeDecodeError:
            continue
    return out


def pinned() -> str | None:
    """The text of the root toolchain file, or None when it is missing."""
    path = ROOT / TOOLCHAIN
    return path.read_text(encoding="utf-8") if path.is_file() else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base",
        default="origin/main",
        help="the revision a bump is read against (default: origin/main)",
    )
    args = parser.parse_args()
    try:
        base = resolve(args.base)
    except LookupError as missing:
        print(
            f"ERROR: cannot read `{missing.args[0]}`, so no bump is checked and "
            "nothing is proved. A shallow clone runs `git fetch --depth=2 origin "
            "main`, and CI sets `fetch-depth` on the checkout step.",
            file=sys.stderr,
        )
        return 1
    text = pinned()
    errors = pin_breaches(text) + bump_breaches(at(base, TOOLCHAIN), text, plans())
    read = [(path, body) for path, body in files() if is_read(path)]
    for path, body in read:
        errors += file_breaches(path, body)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"toolchain pin: {len(errors)} breach(es) in {len(read)} files", file=sys.stderr)
        return 1
    channel = CHANNEL.search(text or "")
    date = channel.group(1) if channel else "?"
    print(f"toolchain pin: {len(read)} files read, one toolchain: nightly-{date}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
