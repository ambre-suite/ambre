#!/usr/bin/env python3
"""Fail when a commit message breaks a rule of `ai/ambre-git-strategy.md`.

Every other rule in that file is one an author satisfies by construction: the
subject is imperative because the author picks the verb, the forbidden trailer is
absent because the author does not type it. **The two rules with a number in them
are different.** They ask an author to count characters in text they are writing,
which nobody does, so both are satisfied by estimate and the estimate is wrong
often enough to matter.

`ambre-spec-template.md section 1.G` states the general case: a pillar without a
script is decoration. `ai/ambre-writing-style.md` says the same of itself and
calls the result drift. This file removes the last unguarded rule of the commit
path.

**It checks shape, never vocabulary.** Nothing here names a person, a tool or a
vendor. A rule below binds a human contributor exactly as it binds an assistant,
which is why the trailer that names the assisting tool is deliberately *not*
checked: its value is the author's to write, and a script asserting one value
would turn a project rule into a preference for one product.

What is not checked, and why: whether the subject is imperative. "Fold" and
"folding" are one dictionary lookup apart and the lookup is wrong on a project
noun, so a check would refuse correct subjects. That one stays a review rule.

Install, once per clone:

    git config core.hooksPath .githooks

Run by hand on any message file:

    python3 scripts/commit_msg.py <file>
"""

import re
import sys
from pathlib import Path

# `ai/ambre-git-strategy.md`: "Imperative subject, 72 characters maximum".
SUBJECT_MAX = 72

# Same file: "The body explains why, in 1 000 characters maximum, trailer
# excluded -- about four paragraphs."
BODY_MAX = 1000

# A trailer line, by git's own shape: a key, a colon, a value. The block of them
# at the end of a message is excluded from the body budget, which is what "trailer
# excluded" means.
TRAILER = re.compile(r"^[A-Za-z][A-Za-z0-9-]*:\s")

# `ai/ambre-git-strategy.md`: "`Co-Authored-by` is forbidden." Matched without
# regard to case, because git and its forges write it three ways.
CO_AUTHORED = re.compile(r"^co-authored-by:", re.IGNORECASE)

# `ai/ambre-writing-style.md`: "Printable ASCII only, code points 32 to 126,
# everywhere: prose, code, comments, commit messages, file names."
NON_ASCII = re.compile(r"[^\x20-\x7e]")

# Messages git writes itself, or writes for a later rewrite. Their subject is not
# the author's to shape, so the rules above do not apply.
GENERATED = ("merge ", "revert ", "fixup!", "squash!", "amend!")


def strip_comments(text: str) -> list[str]:
    """The lines git keeps, in order.

    Git drops every line opening with the comment character before it stores the
    message, and `--verbose` adds a whole diff below a comment marker. Reading
    what git stores, rather than what the editor showed, is what keeps the counts
    equal to the ones a reader of `git log` meets.
    """
    return [line for line in text.splitlines() if not line.startswith("#")]


def split(lines: list[str]) -> tuple[str, list[str], list[str]]:
    """One message as its subject, its body lines, and its trailer lines.

    The trailer block is the last paragraph, and only when **every** one of its
    lines is a trailer. A final paragraph of prose that happens to open on
    `Note: ` stays body, which is the conservative direction: it counts against
    the budget rather than escaping it.
    """
    subject = lines[0] if lines else ""
    rest = lines[1:]
    while rest and not rest[-1].strip():
        rest.pop()

    last = len(rest)
    start = last
    while start > 0 and rest[start - 1].strip():
        start -= 1
    tail = rest[start:last]
    if tail and all(TRAILER.match(line) for line in tail):
        return subject, rest[:start], tail
    return subject, rest, []


def defects(text: str) -> list[str]:
    """Yield one line per rule the message breaks, each naming the rule."""
    lines = strip_comments(text)
    out = []

    if not any(line.strip() for line in lines):
        return out  # git refuses an empty message on its own

    subject, body, _ = split(lines)
    if subject.lower().startswith(GENERATED):
        return out

    if len(subject) > SUBJECT_MAX:
        out.append(
            f"subject is {len(subject)} characters, {SUBJECT_MAX} maximum "
            "(`ai/ambre-git-strategy.md`, commit message)")
    if subject.rstrip().endswith("."):
        out.append(
            "subject ends on a period, and a subject carries none "
            "(`ai/ambre-git-strategy.md`, commit message)")
    if body and body[0].strip():
        out.append("a blank line MUST separate the subject from the body")

    # The budget is the prose a reader meets, so it is measured on the text git
    # stores: comments gone, trailers out, no trailing blank line.
    size = len("\n".join(body).strip())
    if size > BODY_MAX:
        out.append(
            f"body is {size} characters, {BODY_MAX} maximum, trailer excluded. "
            "A body that needs more is a commit that does more than one thing: "
            "split it (`ai/ambre-git-strategy.md`, commit message)")

    if any(CO_AUTHORED.match(line) for line in lines):
        out.append(
            "`Co-Authored-by` is forbidden; the last trailer names the "
            "assisting tool instead (`ai/ambre-git-strategy.md`)")

    for number, line in enumerate(lines, 1):
        found = NON_ASCII.search(line)
        if found:
            out.append(
                f"line {number} carries {found.group()!r}: printable ASCII only, "
                "code points 32 to 126 (`ai/ambre-writing-style.md`)")
            break

    return out


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: commit_msg.py <message file>", file=sys.stderr)
        return 2

    found = defects(Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace"))
    if not found:
        return 0

    print("\nThe commit message breaks a rule of `ai/ambre-git-strategy.md`:\n",
          file=sys.stderr)
    for defect in found:
        print(f"  * {defect}", file=sys.stderr)
    print("\nThe message you wrote is kept in `.git/COMMIT_EDITMSG`. Edit it and "
          "commit again.\n", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
