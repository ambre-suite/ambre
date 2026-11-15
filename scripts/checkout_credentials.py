#!/usr/bin/env python3
"""No checkout of a CI job leaves the job token on disk.

`actions/checkout` writes the token of the job into `.git/config` by default,
so that a later `git push` authenticates. Every later step of the job can then
read the token: a script of the repository, an action, and a build script of a
dependency that `cargo` compiles. `ARCH-113` states the rule. The gate reads
each workflow under `.github/workflows/` and each action under
`.github/actions/`, and fails on each of these:

* a step that uses `actions/checkout` and sets no `persist-credentials`;
* a value other than `false` or `true`, such as an expression, which the gate
  cannot read;
* `persist-credentials: true` on a step whose `name` does not read
  `Checkout to push: <reason>`. A step that pushes names why it keeps the token,
  so a reviewer reads the reason in the step and in the run log;
* no checkout read at all, since a gate that reads nothing passes in silence.

The YAML files are read line by line, by indentation, so the gate needs no YAML
library. It reads block mappings alone, which is the form each workflow uses. A
flow mapping such as `with: { persist-credentials: false }` reads as no value,
and fails.

Run: `just checkout-credentials`. It writes nothing.
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GLOBS = (".github/workflows/*.yml", ".github/workflows/*.yaml",
         ".github/actions/**/action.yml", ".github/actions/**/action.yaml")

USES = re.compile(r"^\s*(- )?uses:\s*['\"]?actions/checkout(@\S*)?['\"]?\s*(#.*)?$")
KEY = re.compile(r"^(\s*)([A-Za-z0-9_.-]+):\s*(.*)$")
PUSH_NAME = re.compile(r"^Checkout to push: \S.*$")
KEPT = "persist-credentials"


@dataclass(frozen=True)
class Checkout:
    """One checkout step: the file, the line of its `uses`, its name and its value."""

    path: str
    line: int
    name: str | None
    kept: str | None


def strip_comment(value: str) -> str:
    """A value without its trailing comment and its quotes."""
    value = re.sub(r"\s+#.*$", "", value).strip()
    return value.strip("\"'")


def indent_of(line: str) -> int:
    """The column of the first character that is not a space."""
    return len(line) - len(line.lstrip())


def step_of(lines: list[str], index: int) -> list[str]:
    """The lines of the list item that holds line `index`: from its dash down
    to the next line, not blank and not a comment, at the dash's column or less."""
    head = index
    while head > 0 and not lines[head].lstrip().startswith("- "):
        head -= 1
    dash = indent_of(lines[head])
    step = [lines[head]]
    for line in lines[head + 1:]:
        if line.strip() and not line.lstrip().startswith("#") and indent_of(line) <= dash:
            break
        step.append(line)
    return step


def checkouts(path: str, text: str) -> list[Checkout]:
    """Each step of one workflow that uses `actions/checkout`, with its name and value."""
    lines = text.splitlines()
    out = []
    for index, line in enumerate(lines):
        if line.lstrip().startswith("#") or not USES.match(line):
            continue
        name = kept = None
        for other in step_of(lines, index):
            if other.lstrip().startswith("#"):
                continue
            found = KEY.match(other.replace("- ", "  ", 1))
            if not found:
                continue
            if found.group(2) == "name":
                name = strip_comment(found.group(3))
            elif found.group(2) == KEPT:
                kept = strip_comment(found.group(3))
        out.append(Checkout(path, index + 1, name, kept))
    return out


def breach(step: Checkout) -> str | None:
    """Whether one checkout leaves the token on disk without a named reason."""
    where = f"{step.path}:{step.line}"
    if step.kept is None:
        return (f"{where}: `actions/checkout` sets no `{KEPT}`, so the job token stays "
                f"in `.git/config`: write `{KEPT}: false` under `with:`")
    if step.kept == "false":
        return None
    if step.kept != "true":
        return f"{where}: `{KEPT}: {step.kept}` is neither `false` nor `true`"
    if step.name is None or not PUSH_NAME.match(step.name):
        return (f"{where}: `{KEPT}: true` keeps the job token for every later step: "
                f"name the step `Checkout to push: <reason>`, or set `false`")
    return None


def breaches(files: dict[str, str]) -> tuple[list[str], int]:
    """Every breach of the planted workflows, and the count of checkouts read."""
    found = [step for path, text in sorted(files.items()) for step in checkouts(path, text)]
    out = [b for b in map(breach, found) if b is not None]
    if not found:
        out.append("no `actions/checkout` step read under `.github/`")
    return out, len(found)


def workflows() -> dict[str, str]:
    """The text of each workflow and action of the repository, by relative path."""
    paths = {p for pattern in GLOBS for p in ROOT.glob(pattern)}
    return {str(p.relative_to(ROOT)): p.read_text(encoding="utf-8") for p in sorted(paths)}


def main() -> int:
    errors, count = breaches(workflows())
    if errors:
        print("\n".join(errors), file=sys.stderr)
        print(f"checkout credentials: {len(errors)} breach(es) in {count} checkouts",
              file=sys.stderr)
        return 1
    print(f"checkout credentials: {count} checkouts read, none keeps the job token "
          "without a named reason")
    return 0


if __name__ == "__main__":
    sys.exit(main())
