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

# A fence is any line that starts with ``` -- the info string (its tag) may be
# anything. Tracking every fence, whatever its tag, is what keeps the open/close
# pairing correct: a tag we do not scan (`text`, `sql`, ...) still toggles the
# fence, so code that follows it is not mistaken for prose (the bug this file
# once had -- a single ```text desynchronised every fence after it).
FENCE = re.compile(r"^```(.*)$")

# Fence tags allowed in the spec corpus. `rust` carries Rust shapes (checked for
# bodies below); `text` and the untagged fence carry diagrams / pseudo-code; the
# rest are genuine non-Rust artefacts (schemas, configs, styles). `lua` is NOT
# allowed: an embedded script is implementation and belongs in the source, shown
# here as `text` if a reference is truly needed (ambre-spec-template.md §0).
ALLOWED_TAGS = {"", "rust", "text", "json", "toml", "yaml", "sql", "css", "html", "bash", "sh", "ron"}

# Lines that betray Rust living outside a ```rust fence -- the tag is wrong and
# the body checker would never see them.
RUST_HINT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|impl|mod)\b")


def fence_tag(line: str) -> str:
    """The base language token of a fence open line (`” ```rust,ignore ”` -> `rust`)."""
    info = FENCE.match(line).group(1).strip()
    return re.split(r"[,\s]", info)[0] if info else ""


def bodies(text: str):
    """Yield (line_no, size, first_line) for every fn body inside a rust fence."""
    lines = text.splitlines()
    in_fence = False
    scan_this = False
    start = 0
    buf: list[str] = []
    for i, line in enumerate(lines, 1):
        if FENCE.match(line):
            if not in_fence:
                in_fence, scan_this, start, buf = True, fence_tag(line) in ("rust", ""), i, []
            else:
                if scan_this:
                    yield from scan(buf, start)
                in_fence = scan_this = False
            continue
        if in_fence and scan_this:
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


def fence_issues(text: str):
    """Yield (line_no, kind, detail) for every fence defect in one spec.

    Three kinds, all of which would let Rust hide from `bodies()`:
      - `unterminated` : a fence is opened and never closed.
      - `bad-tag`      : the fence tag is not in `ALLOWED_TAGS` (e.g. `lua`).
      - `rust-untagged`: a Rust shape sits in a non-`rust` fence -- wrong tag.
    """
    lines = text.splitlines()
    in_fence = False
    open_line = 0
    tag = ""
    for i, line in enumerate(lines, 1):
        if FENCE.match(line):
            if not in_fence:
                in_fence, open_line, tag = True, i, fence_tag(line)
                if tag not in ALLOWED_TAGS:
                    yield i, "bad-tag", tag
            else:
                in_fence = False
            continue
        # Real Rust opens a block with `{`; pseudo-code (`fn page() -> f32:`) does
        # not, so a trailing `{` is what tells a mis-tagged shape from prose.
        if in_fence and tag != "rust" and RUST_HINT.match(line) and line.rstrip().endswith("{"):
            yield i, "rust-untagged", f"in ```{tag or '(none)'} fence: {line.strip()[:60]}"
    if in_fence:
        yield open_line, "unterminated", f"```{tag or '(none)'}"


def main() -> int:
    check = "--check" in sys.argv
    total = 0
    files = 0
    fence_defects = 0
    for path in sorted(SPECS.rglob("*.md")):
        if "reviews/" in str(path.relative_to(SPECS)):
            continue  # scratch review logs, not part of the normative corpus
        text = path.read_text()
        rel = path.relative_to(SPECS)
        found = list(bodies(text))
        fences = list(fence_issues(text))
        if found:
            files += 1
            print(f"\n{rel}  ({len(found)} bodies, {sum(f[1] for f in found)} lines)")
            for lineno, size, sig in found:
                print(f"  {lineno:>5}  {size:>4} lines  {sig[:88]}")
                total += size
        for lineno, kind, detail in fences:
            fence_defects += 1
            print(f"\n{rel}:{lineno}  FENCE {kind}: {detail}")
    print(f"\n{total} lines of function bodies across {files} specs; {fence_defects} fence defects")
    if check and (total or fence_defects):
        print(
            "\nERROR: a spec states what and why, the source states how "
            "(ambre-spec-template.md §0). Rust shapes live only in ```rust fences "
            "and carry no bodies; collapse a body to `{ /* ... */ }` plus a pointer "
            "to the real file, and tag every fence per the template's Fences rule.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
