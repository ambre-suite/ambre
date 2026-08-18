#!/usr/bin/env python3
"""Fail when a spec carries a function body.

`ambre-spec-template.md §0` allows shapes and forbids implementations: a
`struct`, an `enum`, a trait or a public signature is the contract others must
respect and belongs in a spec; a function body is a second, uncompiled copy of
the code that drifts silently. The corpus has paid for that twice -- a
`trame-ui` snippet that contradicted its own sentence, and `proptest!` bodies
describing a test suite that did not exist.

This makes the rule mechanical instead of aspirational.

Run: `just spec-code` (list), `just spec-code --check` (fail on any body).
"""

import re
import sys
from pathlib import Path

SPECS = Path(__file__).resolve().parent.parent / "specs"

# `fn name(...) ... {` on one line, or a signature whose brace lands on the next.
# A trailing `;` is a declaration, not a body.
SIGNATURE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+")
FENCE = re.compile(r"^```(\w*)\s*$")


def bodies(text: str):
    """Yield (line_no, size, first_line) for every fn body inside a rust fence."""
    lines = text.splitlines()
    in_fence = False
    start = 0
    buf: list[str] = []
    for i, line in enumerate(lines, 1):
        m = FENCE.match(line)
        if m and not in_fence:
            in_fence, start, buf = m.group(1) in ("rust", ""), i, []
            continue
        if m and in_fence:
            yield from scan(buf, start)
            in_fence = False
            continue
        if in_fence:
            buf.append(line)


def scan(block: list[str], start: int):
    depth = 0
    open_at = None
    for i, line in enumerate(block):
        if depth == 0 and SIGNATURE.match(line) and not line.rstrip().endswith(";"):
            # a signature inside a trait/impl block stub ends with `;`
            if "{" in line or (i + 1 < len(block) and block[i + 1].strip().startswith("{")):
                open_at = i
        if open_at is not None:
            depth += line.count("{") - line.count("}")
            if depth <= 0 and "{" in "".join(block[open_at : i + 1]):
                size = i - open_at + 1
                if size > 2:  # `fn f() { }` over two lines is a shape, not a body
                    yield start + open_at + 1, size, block[open_at].strip()
                open_at, depth = None, 0


def main() -> int:
    check = "--check" in sys.argv
    total = 0
    files = 0
    for path in sorted(SPECS.rglob("*.md")):
        found = list(bodies(path.read_text()))
        if not found:
            continue
        files += 1
        rel = path.relative_to(SPECS)
        print(f"\n{rel}  ({len(found)} bodies, {sum(f[1] for f in found)} lines)")
        for lineno, size, sig in found:
            print(f"  {lineno:>5}  {size:>4} lines  {sig[:88]}")
            total += size
    print(f"\n{total} lines of function bodies across {files} specs")
    if check and total:
        print(
            "\nERROR: a spec states what and why; the source states how "
            "(ambre-spec-template.md §0). Replace the body with the algorithm in "
            "a few lines plus a pointer to the real file.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
