#!/usr/bin/env python3
"""Fail when a spec carries a function body.

`ambre-spec-template.md §0` allows shapes and forbids implementations: a
`struct`, an `enum`, a trait or a public signature is the contract others must
respect and belongs in a spec; a function body is a second, uncompiled copy of
the code that drifts silently. The corpus has paid for that twice -- a
`trame-ui` snippet that contradicted its own sentence, and `proptest!` bodies
describing a test suite that did not exist.

This makes the rule mechanical instead of aspirational.

It also holds the citations the source makes back into the specs, in two
directions a reader otherwise trusts blindly:

  - **ownership.** A rule ID cited under `crates/ambre-X/src` belongs to a spec
    of X or of the foundations, or the file path names the application that
    owns it (`ambre-bus/src/app/prose.rs` cites Prose). A shared crate citing
    an application rule is a shared layer that knows one application.
  - **test and mechanism.** A test whose doc comment cites a rule is named by
    that rule's `Mechanism` cell. A test that claims a rule the row does not
    credit it with is either a missing mechanism or a false claim.

The reverse -- a test a `Mechanism` names must cite its rule -- is counted and
not enforced: the corpus never asked a test to cite, so it fails on most of them
and a list of those failures would freeze nothing useful.

The defects found when the check landed are listed in `ALLOW`, one per line.
The list only shrinks: a new defect fails, and so does a line no defect matches.

Run: `just spec-code`. It lists each body, and fails on any.
"""

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import corpus  # noqa: E402  -- the one reader of the Markdown

# The fence tags are the template's vocabulary, read from it. What each one
# holds is stated there, beside the table it is read from, and is not repeated
# here (`ambre-spec-template.md section 0`).
from template_vocabulary import FENCE_TAGS  # noqa: E402

# The contract rows, parsed in memory: the index file is built on demand, never committed.
import contract_index  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"
CRATES = ROOT / "crates"
ALLOW = Path(__file__).resolve().parent / "spec_code_citations.allow"

# `specs/audits/` holds the non-normative analysis documents that live under
# `specs/` without being specs. An audit quotes real source on purpose, so it is
# exempt from the "shapes only" rule, exactly like everything under `reviews/`.
# A directory rather than a list of names: the next audit is covered the day it
# lands, where a named list covers the documents someone remembered.
AUDITS = "audits/"

# `fn name(...) ... {` on one line, or a signature whose brace lands on the next.
# A trailing `;` is a declaration, not a body.
SIGNATURE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+")

# Fence tags allowed in the spec corpus, read from the table that states them.
# `rust` carries Rust shapes (checked for bodies below); `text` and the untagged
# fence carry diagrams / pseudo-code; `mermaid` carries a rendered diagram; the
# rest are genuine non-Rust artefacts. A tag off the table is refused, `lua`
# included: an embedded script is implementation and belongs in the source,
# shown here as `text` if a reference is truly needed.
ALLOWED_TAGS = frozenset(FENCE_TAGS)

# Lines that betray Rust living outside a ```rust fence -- the tag is wrong and
# the body checker would never see them.
RUST_HINT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|impl|mod)\b")

# A statement sitting at brace-depth 0 of a ```rust fence is loose implementation:
# it is code that is not part of any declaration (a `match`, a `let`, a loop, a
# `self.` call). `bodies()` never sees it because it is wrapped in no `fn`. A real
# spec fence at depth 0 holds only items -- `struct`/`enum`/`trait`/`impl`/`fn`
# signatures, `use`, `type`, `const`, attributes -- never a running statement.
STMT_HINT = re.compile(r"^\s*(?:let|match|for|while|loop|return|if|self)\b")


# A fence is any line that starts with ``` -- the info string (its tag) may be
# anything. `corpus.py` pairs every fence, whatever its tag, which is what keeps
# the open/close pairing correct: a tag we do not scan (`text`, `sql`, ...) still
# toggles the fence, so code that follows it is not mistaken for prose (the bug
# this file once had -- a single ```text desynchronised every fence after it).
fence_tag = corpus.fence_tag


def bodies(text):
    """Yield (line_no, size, first_line) for every fn body inside a rust fence."""
    for fence in corpus.as_document(text).fences:
        if fence.end is not None and fence.tag in ("rust", ""):
            yield from scan([line.text for line in fence.body], fence.start)


def is_empty_body(text: str) -> bool:
    """True when a `{ ... }` body carries no code -- only whitespace or comments.

    This is what tells a collapsed shape (`fn f(...) { /* ... */ }`, possibly over
    a multi-line signature) from a real implementation, replacing the old
    single-line `size > 2` heuristic that missed a body under a wrapped signature.
    """
    inner = text[text.find("{") + 1 : text.rfind("}")]
    inner = re.sub(r"/\*.*?\*/", "", inner, flags=re.S)  # block comments
    inner = re.sub(r"//[^\n]*", "", inner)               # line comments
    return inner.strip() == ""


def scan(block: list[str], start: int):
    """Yield (line_no, size, first_line) for every real fn body in a rust fence.

    Depth is tracked only inside a detected body, so the `impl {` / `trait {`
    wrapper braces are ignored and a method is seen at the fence's top level. A
    signature can span several lines (one parameter per line): the opening brace
    is the first `{` reached before any signature-terminating `;`.
    """
    i, n = 0, len(block)
    while i < n:
        line = block[i]
        if not (SIGNATURE.match(line) and not line.rstrip().endswith(";")):
            i += 1
            continue
        # Walk the (possibly multi-line) signature to its body brace.
        j, opened = i, False
        while j < n:
            if "{" in block[j]:
                opened = True
                break
            if block[j].rstrip().endswith(";"):  # split declaration, no body
                break
            j += 1
        if not opened:
            i += 1
            continue
        # Track brace depth from the opening line until the body closes.
        depth, k = 0, j
        while k < n:
            depth += block[k].count("{") - block[k].count("}")
            if depth <= 0:
                break
            k += 1
        # A body closing on its opening line is a one-liner accessor -- a shape
        # the template tolerates as illustrative. Only a multi-line block with
        # real content is a forbidden implementation.
        if j != k and not is_empty_body("\n".join(block[i : k + 1])):
            yield start + i + 1, k - i + 1, block[i].strip()
        i = k + 1


def loose_impl(text):
    """Yield (line_no, first_line) for a running statement at depth 0 of a rust fence.

    Tracks brace depth inside each ```rust fence; a statement keyword seen while
    depth is 0 is code that belongs to no declaration -- the free-floating snippet
    (`match ... { }`, `let x = ...`) that `bodies()`, keyed on `fn`, cannot catch.
    """
    for fence in corpus.as_document(text).fences:
        if fence.tag != "rust":
            continue
        depth = 0
        for line in fence.body:
            if depth == 0 and STMT_HINT.match(line.text):
                yield line.no, line.text.strip()[:70]
            depth = max(0, depth + line.text.count("{") - line.text.count("}"))


def fence_issues(text):
    """Yield (line_no, kind, detail) for every fence defect in one spec.

    Three kinds, all of which would let Rust hide from `bodies()`:
      - `unterminated` : a fence is opened and never closed.
      - `bad-tag`      : the fence tag is not in `ALLOWED_TAGS` (e.g. `lua`).
      - `rust-untagged`: a Rust shape sits in a non-`rust` fence -- wrong tag.
    """
    for fence in corpus.as_document(text).fences:
        tag = fence.tag
        if tag not in ALLOWED_TAGS:
            yield fence.start, "bad-tag", tag
        # Real Rust opens a block with `{`; pseudo-code (`fn page() -> f32:`) does
        # not, so a trailing `{` is what tells a mis-tagged shape from prose.
        for line in fence.body if tag != "rust" else ():
            if RUST_HINT.match(line.text) and line.text.rstrip().endswith("{"):
                yield line.no, "rust-untagged", f"in ```{tag or '(none)'} fence: {line.text.strip()[:60]}"
        if fence.end is None:
            yield fence.start, "unterminated", f"```{tag or '(none)'}"


# A rule ID as the corpus mints them: `KINETIC-002`, `PROSE-UI-137`. Only an ID
# the index answers counts, so a lookalike (`UTF-008`) is never a citation.
RULE_ID = re.compile(r"\b[A-Z][A-Z0-9]*(?:-[A-Z]+)*-\d{3}\b")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)")
# `#[test]`, `#[tokio::test]`, `#[wasm_bindgen_test]`; never `#[cfg(test)]`, which
# marks a helper the tests share.
TEST_ATTR = re.compile(r"^\s*#\[(?:\w+::)*\w*test\b(?!\s*\))")
# A `Mechanism` cell names a test by its path; the last segment is the `fn`. A
# path may open on `::`, the continuation `ambre-spec-template.md` declares.
MECHANISM_FN = re.compile(r"`(?:\w*::)+(\w+)`")


def application(document: str) -> str | None:
    """The application a spec belongs to (`3-Toile/...` -> `toile`), or None for the foundations."""
    head = document.split("/", 1)[0]
    if head.startswith("1-"):
        return None
    return head.split("-", 1)[1].lower()


def owner_defects(rows: dict, path: str, text: str):
    """Yield (line_no, rule_id) for each rule this source file may not cite."""
    crate, _, inner = path.partition("/src/")
    crate = crate.removeprefix("crates/ambre-")
    for n, line in enumerate(text.splitlines(), 1):
        for rule in RULE_ID.findall(line):
            if rule not in rows:
                continue
            app = application(rows[rule]["document"])
            # `ambre-prose-form` is Prose's own leaf crate, which a module
            # may link: a crate named after an application belongs to it.
            if app is None or crate.split("-", 1)[0] == app or app in re.split(r"[/_.]", inner):
                continue
            yield n, rule


def test_citations(text: str):
    """Yield (fn_name, rule_ids) for every test whose doc comment cites a rule."""
    lines = text.splitlines()
    for i, line in enumerate(lines):
        found = FN.match(line)
        if not found:
            continue
        attrs, doc = [], []
        j = i - 1
        while j >= 0 and lines[j].strip().startswith(("#[", "///")):
            (attrs if lines[j].strip().startswith("#[") else doc).append(lines[j])
            j -= 1
        if any(TEST_ATTR.match(a) for a in attrs):
            yield found.group(1), {r for d in doc for r in RULE_ID.findall(d)}


def citation_defects(rows: dict, sources: dict[str, str]):
    """Every ownership and test-mechanism defect, as (kind, subject, rule_id, where).

    Also returns how many tests a `Mechanism` names without the test citing it,
    the reverse direction this gate counts and does not enforce.
    """
    named: dict[str, set[str]] = {}
    for rule, row in rows.items():
        for name in MECHANISM_FN.findall(row["mechanism"]):
            named.setdefault(name, set()).add(rule)
    defects, cited = [], {}
    for path, text in sources.items():
        if "/src/" in path:
            for n, rule in owner_defects(rows, path, text):
                defects.append(("owner", path, rule, f"{path}:{n}"))
        for name, rules in test_citations(text):
            cited.setdefault(name, set()).update(rules)
            for rule in sorted(rules):
                if rule in rows and rule not in named.get(name, ()):
                    defects.append(("test", name, rule, path))
    uncited = sum(
        1 for name, rules in named.items() for r in rules if name in cited and r not in cited[name]
    )
    return defects, uncited


def read_allow() -> set[tuple[str, str, str]]:
    if not ALLOW.exists():
        return set()
    out = set()
    for line in ALLOW.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            kind, subject, rule = line.split()
            out.add((kind, subject, rule))
    return out


def check_citations() -> int:
    """Print the citation defects outside `ALLOW` and the stale lines of it; return their count."""
    rows = {r["id"]: r for r in contract_index.index()}
    sources = {
        str(p.relative_to(ROOT)): p.read_text() for p in sorted(CRATES.rglob("*.rs"))
        if "/target/" not in str(p)
    }
    defects, uncited = citation_defects(rows, sources)
    allow = read_allow()
    seen = {(k, s, r) for k, s, r, _ in defects}
    fresh = [d for d in defects if d[:3] not in allow]
    stale = sorted(allow - seen)
    for kind, subject, rule, where in fresh:
        if kind == "owner":
            print(f"\n{where}  OWNER: {rule} belongs to no spec of this crate or of the foundations")
        else:
            print(f"\n{where}  TEST: `{subject}` cites {rule}, whose `Mechanism` does not name it")
    for kind, subject, rule in stale:
        print(f"\n{ALLOW.name}: `{kind} {subject} {rule}` matches no defect -- delete the line")
    print(
        f"\n{len(defects)} citation defects, {len(defects) - len(fresh)} listed in {ALLOW.name}; "
        f"{uncited} tests a `Mechanism` names without the test citing the rule (counted)"
    )
    return len(fresh) + len(stale)


def main() -> int:
    total = 0
    files = 0
    fence_defects = 0
    for path in corpus.markdown(SPECS):
        rel_str = str(path.relative_to(SPECS))
        # Analysis artefacts, not normative specs: they quote real source (an audit
        # cites the vulnerable line as its evidence; a review logs a code smell), so
        # the "no implementation" rule does not apply.
        if "reviews/" in rel_str or AUDITS in rel_str:
            continue
        text = corpus.read(path)
        rel = path.relative_to(SPECS)
        found = list(bodies(text))
        fences = list(fence_issues(text))
        loose = list(loose_impl(text))
        if found:
            files += 1
            print(f"\n{rel}  ({len(found)} bodies, {sum(f[1] for f in found)} lines)")
            for lineno, size, sig in found:
                print(f"  {lineno:>5}  {size:>4} lines  {sig[:88]}")
                total += size
        for lineno, kind, detail in fences:
            fence_defects += 1
            print(f"\n{rel}:{lineno}  FENCE {kind}: {detail}")
        for lineno, sig in loose:
            fence_defects += 1
            print(f"\n{rel}:{lineno}  LOOSE-IMPL (statement at fence depth 0): {sig}")
    print(f"\n{total} lines of function bodies across {files} specs; {fence_defects} fence defects")
    citations = check_citations()
    if citations:
        print(
            "\nERROR: a rule cited in a crate belongs to that crate's application or to "
            "the foundations, and a test that cites a rule is named by its `Mechanism`. "
            "Move the citation, add the test to the row, or drop the claim.",
            file=sys.stderr,
        )
        return 1
    if total or fence_defects:
        print(
            "\nERROR: a spec states what and why, the source states how "
            "(ambre-spec-template.md §0). Rust shapes live only in ```rust fences "
            "and carry no bodies and no loose statements; collapse a body to "
            "`{ /* ... */ }` plus a pointer to the real file, turn a running snippet "
            "(match/let/for at fence depth 0) into prose, and tag every fence per "
            "the template's Fences rule.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
