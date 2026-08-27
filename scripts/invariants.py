#!/usr/bin/env python3
"""Generate the invariant ledger from the spec tables, and check their rules.

`ambre-invariant-ledger.md` is the roll-up of every per-spec invariant table. A
hand-maintained roll-up is the same failure mode as a hand-maintained test list:
it is believed, and it is wrong (it was, by one, for a week). So the file is
written by this script and by nothing else, like each application's feature
inventory. Nobody reads it; CI reads it.

The rules it enforces, beyond generating the file:

  * a status must be one of the five declared values;
  * an `unimplemented` row must name the Story that will build it ("lands with");
  * **that Story must resolve to an open row of `ambre-roadmap.md`.** The template
    says "that story is not `[x]` until the row's status has moved", and nothing
    held it: this script looked for the substring "lands with" and never read the
    roadmap. So a story could be ticked, renamed or moved under another epic and
    373 rows would keep pointing at nothing. The name is the join key, hence
    three failures: a name no row carries, a name two rows carry, and a name on a
    row that is `[x]` or that closed into `ambre-roadmap-journal.md`. The epic in
    the citation is checked too when it is there, so a moved story cannot leave a
    stale key behind;
  * **every mechanism a built row cites must resolve to a `fn` in the workspace.**
    The corpus demands it -- "a test name that resolves to a `fn` behind a Cargo
    feature no job enables is the same claim as a name that resolves to nothing"
    -- and nothing held it. The corpus once cited 82 test names of which 23
    resolved; that was cleaned by hand, so it was free to come back. It had:
    `TRAME-UI-011` cited `::editor_preview_display`, which never existed, under
    status `verified`. An `unimplemented` row is exempt: its Verification cell
    names a Story, not a test;
  * **every audit finding a row cites must exist, and every finding must be
    cited.** The template makes the citation the join between a finding and the
    rule that closes it -- "the finding stops drifting from the code in silence"
    -- and nothing held either end. Both rotted: `SRV-029` cited `audit #3`,
    which is not a finding id, and eight findings that a remediation wave closed
    had no rule at all, so nothing would go red the day the fix was reverted. A
    finding with no rule to state is declared in `NO_RULE` below, with its
    reason;
  * an id is never reused, and a prefix has no gap in its numbering;
  * **the counts under a table are the table's own.** The `Verification` answer
    is written by hand, beside a table that is not, so the two drift: nine
    documents were wrong the day this check landed, and `ambre-spec-membership.md`
    claimed one `verified` row fewer than it carries. A status the answer leaves
    out claims zero, so an omission is compared like a figure. The presence of
    the label stays `spec_shape.py`, which holds the three answers together.

What this still cannot see: a row citing a test that runs, passes, and does not
test the rule. That is `TRAME-FEAT-004`'s defect, and it needs a probe or
`cargo-mutants`, not a parser.

Run: `just invariants` (fail when the file on disk is not what this would
write), `just invariants-write` (rewrite it). The check compares the whole text,
so a spec edited without a regeneration turns CI red instead of leaving a stale
roll-up behind.
"""

import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "specs"
CRATES = ROOT / "crates"
TEMPLATE = "ambre-spec-template.md"  # carries example rows, not real invariants
LEDGER = "1-Foundations/ambre-invariant-ledger.md"  # generated here, nowhere else
ROADMAP = "ambre-roadmap.md"  # the one schedule: every Story is a row of it
JOURNAL = "ambre-roadmap-journal.md"  # closed epics, cut there verbatim
AUDIT = "security-audit.md"  # numbered findings, cited by the rows that close them
STATUSES = ("verified", "unverified", "human-review", "qa-manual", "unimplemented")
LOOSE = ("unverified", "human-review", "qa-manual")  # built, and no test would fail
APPS = ("1-Foundations", "2-Trame", "3-Toile", "4-Prose")

# An id minted once, then withdrawn because the rule it carried left the
# application for a shared document. The id is **never reused** -- that is what
# this register buys -- and the gap check below reads it so a withdrawal is
# declared here rather than passing as a numbering hole. Each entry names the
# shared row that carries the rule today, so the trail from an old citation is
# one lookup. Nothing else may be entered: a row whose rule is still this
# application's is narrowed or split, never withdrawn.
RETIRED = {
    "PROSE-020": "PRESENCE-003",
    "PROSE-021": "PRESENCE-004",
    "PROSE-UI-011": "PRESENCE-001, PRESENCE-003, PRESENCE-005",
    "PROSE-UI-012": "UI-THREAD-001",
    "TRAME-UI-016": "PRESENCE-006",
    "TRAME-UI-017": "PRESENCE-005",
    "TRAME-UI-018": "PRESENCE-004",
    "TOILE-UI-072": "PRESENCE-006",
    "TRAME-UI-066": "UI-THREAD-009",
    "TRAME-122": "UI-THREAD-016",
    "TOILE-077": "UI-THREAD-016",
    "TRAME-018": "UNDO-001",
    "TRAME-116": "UNDO-002",
    "TRAME-117": "UNDO-005",
    "TRAME-118": "UNDO-003",
    "TRAME-119": "UNDO-004",
    "TRAME-120": "UNDO-006",
    "TOILE-015": "UNDO-001",
    "TOILE-072": "UNDO-005",
    "TOILE-073": "UNDO-002",
    "TOILE-074": "UNDO-003",
    "TOILE-075": "UNDO-006",
    "TOILE-076": "UNDO-004",
    "PROSE-022": "UNDO-001",
    "PROSE-FEAT-012": "UNDO-001",
    "PROSE-FEAT-032": "UNDO-002",
}

# A finding that states no rule, and why. It is the `RETIRED` register applied to
# the audit: the coverage check below reads it, so a finding lands here by
# decision instead of passing as a hole. Nothing else belongs here -- a finding
# about behaviour is a row in the spec of the component it constrains.
NO_RULE = {
    "C3": "false positive: the comparison it reports is already constant-time",
    "M8": "a module declared and unreachable, which states no behaviour to hold",
}

# What a Sources cell writes to cite a finding: ``audit `H9` ``, or a list of
# them. The ids inside are read back and checked, so ``audit `#3` `` fails as a
# malformed citation rather than resolving to nothing in silence.
AUDIT_CITE = re.compile(r"audit\s+(`[^`]+`(?:\s*,\s*`[^`]+`)*)")
FINDING = re.compile(r"^[CHMN]\d{1,2}$")
# Where the audit declares one: a section heading, a row of a status table, or a
# story of a remediation wave. A finding named in running prose is a mention.
FINDING_HEAD = re.compile(r"^###\s+(.*)$")
FINDING_ROW = re.compile(r"^\|\s*`?([CHMN]\d{1,2})`?\s*\|")
FINDING_STORY = re.compile(r"^-\s*\[[ x]\]\s*\*\*([CHMN]\d{1,2})\*\*")

ROW = re.compile(
    r"^\|\s*`(?P<id>[A-Z][A-Z0-9-]*-\d{3})`\s*\|"
    r"(?P<rest>.*)\|\s*(?P<status>[a-z-]+)\s*\|\s*$"
)


# A backticked Rust path in a Verification cell: `crate::module::name`, or the
# `::name` shorthand for "same module as the path just before it". A single
# segment is prose, and a CamelCase last segment is a type or an enum variant,
# not something `fn` declares.
CITED = re.compile(
    r"`(::[A-Za-z_]\w*|[A-Za-z_]\w*(?:::[A-Za-z_]\w*)+)`"
)
FN = re.compile(r"\bfn\s+([a-z_]\w*)")

# The schedule side. An epic heading is numbered in the roadmap and bare in the
# journal; a story is the first cell of a row, behind its checkbox.
EPIC_HEAD = re.compile(r"^##\s+(?:\d+\.\s+)?(?P<key>[A-Z][A-Z0-9-]*)\b")
STORY_ROW = re.compile(r"^\|\s*`\[(?P<done>[ x])\]`\s*(?P<name>[^|]+?)\s*\|")
# What a Verification cell says after "lands with": the story, then an optional
# parenthesis naming the epic, then whatever note the cell adds. No story name
# carries a parenthesis or a sentence break, so the name ends at the first of
# either.
NOTE = re.compile(r";|\.\s|, and \*\*")
HINT = re.compile(r"\(([^)]*)\)")
EPIC_IN_HINT = re.compile(r"\b(post-V1|POST-V1|E[0-9]|E[A-Z])\b")


def story_key(name: str) -> str:
    """The join key of a story name: what two spellings of it have in common.

    A citation delimits the name with quotes or backticks and the roadmap does
    not, so both sides are stripped of them, folded to lower case and collapsed on
    whitespace. Everything else is compared exactly: the name is a join key, and a
    key that forgives a word is a key that joins the wrong row.
    """
    return re.sub(r"\s+", " ", name.replace("`", "").replace('"', "")).strip(" .").lower()


def story_keys(name: str) -> list[str]:
    """The key of a story, plus its phase code when the name carries one.

    `ARCH P8 -- persisted schema versioning` is cited 13 times as `ARCH P8`:
    section 1 of the roadmap makes the phase code a citation key of its own. So a
    name shaped `<code> -- <words>` registers under both, and the alias is held to
    the same uniqueness as the name.
    """
    keys = [story_key(name)]
    if " -- " in name:
        keys.append(story_key(name.split(" -- ")[0]))
    return keys


def read_schedule(path: Path) -> dict[str, tuple[str, bool, int, str]]:
    """Every story of one schedule file, per key: epic, ticked, line, name."""
    index: dict[str, tuple[str, bool, int, str]] = {}
    epic = "?"
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        head = EPIC_HEAD.match(line)
        if head:
            epic = head["key"]
            continue
        row = STORY_ROW.match(line)
        if row:
            for key in story_keys(row["name"]):
                index[key] = (epic, row["done"] == "x", lineno, row["name"].strip())
    return index


def duplicate_stories(path: Path) -> list[str]:
    """Each story name or phase code that two rows of one schedule carry.

    `read_schedule` keeps the last row per key, so a collision is invisible to a
    citation: it resolves, silently, to whichever row comes last. That is the
    defect this reports -- "PDF export" named a Prose story and a Trame one.
    """
    seen: dict[str, int] = {}
    errors: list[str] = []
    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        row = STORY_ROW.match(line)
        if not row:
            continue
        for key in story_keys(row["name"]):
            if key in seen:
                errors.append(
                    f"{path.name}:{lineno}: two stories share the name or code "
                    f"{key!r} (the other is line {seen[key]}) -- a story name is a "
                    f"join key, so an `unimplemented` row cannot name one of them"
                )
            seen[key] = lineno
    return errors


def cited_story(cell: str) -> tuple[str, str | None] | None:
    """The story a Verification cell lands with, and the epic it claims."""
    if "lands with" not in cell:
        return None
    text = NOTE.split(cell.split("lands with", 1)[1])[0].strip()
    epic = None
    hint = HINT.search(text)
    if hint:
        text = text[: hint.start()].strip()
        found = EPIC_IN_HINT.search(hint.group(1))
        epic = found.group(1) if found else None
    text = re.sub(r"\s+Story$", "", text).strip()
    text = re.sub(r'^the\s+(?=[`"])', "", text).strip()
    return text, epic


def story_defects(
    where: str,
    cell: str,
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether one `unimplemented` row lands with an open story of the roadmap."""
    cite = cited_story(cell)
    if cite is None:
        return [
            f"{where}: `unimplemented` without a Story -- the Verification cell "
            f"must read 'not built -- lands with <Story>'"
        ]
    name, epic = cite
    name = name.strip('`"')  # the delimiters a citation puts around the name
    entry = schedule.get(story_key(name))
    if entry is None:
        gone = closed.get(story_key(name))
        if gone:
            return [
                f"{where}: lands with {name!r}, which closed under epic `{gone[0]}` "
                f"in {JOURNAL} -- an unbuilt row cannot wait on work that shipped"
            ]
        return [
            f"{where}: lands with {name!r}, and no row of {ROADMAP} carries that "
            f"name or phase code"
        ]
    story_epic, done, lineno, real = entry
    defects = []
    if done:
        defects.append(
            f"{where}: lands with {real!r}, which {ROADMAP}:{lineno} ticks `[x]` "
            f"-- a story stays open until every row it builds has left "
            f"`unimplemented`"
        )
    if epic and epic.upper() != story_epic.upper():
        defects.append(
            f"{where}: cites epic `{epic}`, and {real!r} lives under `{story_epic}`"
            f" ({ROADMAP}:{lineno})"
        )
    return defects


# A story cited anywhere else: a shipped row naming the Story that repairs it, or
# a section 5 bullet naming the one that will build the thing. The `lands with`
# form is the same citation with the parenthesis optional, and is read above.
CITED_STORY = re.compile(
    r"(?P<name>`[^`]+`|\"[^\"]+\")\s*(?:Story\s*)?"
    r"\(`ambre-roadmap\.md`\s*(?P<epic>[^),]*)"
)


def loose_citation_defects(
    line: str,
    where: str,
    handled: set[str],
    schedule: dict[str, tuple[str, bool, int, str]],
    closed: dict[str, tuple[str, bool, int, str]],
) -> list[str]:
    """Whether every other story this line cites resolves, and to the epic named.

    A row that ships and fails its own rule names the Story that repairs it, and
    that citation rots exactly like an `unimplemented` one: `One clipboard host`
    was cited under `E6` by three `unverified` rows while the story moved to `E7`.
    """
    defects = []
    for m in CITED_STORY.finditer(line):
        name = m["name"].strip('`"')
        if story_key(name) in handled:
            continue
        entry = schedule.get(story_key(name))
        if entry is None:
            if story_key(name) in closed:
                gone = closed[story_key(name)]
                defects.append(
                    f"{where}: cites the Story {name!r} as {ROADMAP}, and it closed"
                    f" under epic `{gone[0]}` in {JOURNAL}"
                )
            continue  # a backticked phrase that is not a story name
        story_epic, done, lineno, real = entry
        if done:
            defects.append(
                f"{where}: cites the Story {real!r}, which {ROADMAP}:{lineno} ticks"
                f" `[x]` -- the work it waits on has shipped"
            )
        elif m["epic"].strip() and m["epic"].strip().upper() != story_epic.upper():
            defects.append(
                f"{where}: cites {real!r} under epic `{m['epic'].strip()}`, and it "
                f"lives under `{story_epic}` ({ROADMAP}:{lineno})"
            )
    return defects


def declared_fns() -> tuple[dict[str, set[str]], set[str]]:
    """Every `fn` name in the workspace, per crate and pooled.

    Names, not paths: resolving a module path to a file would mean reimplementing
    Rust's module system in a regex, and the claim being checked is only that the
    mechanism exists. A cell naming the wrong module but the right function is a
    citation defect this does not catch, and a far smaller one than a name that
    resolves to nothing.
    """
    per_crate: dict[str, set[str]] = {}
    for source in sorted(CRATES.glob("*/*/**/*.rs")):
        crate = source.relative_to(CRATES).parts[0].replace("-", "_")
        per_crate.setdefault(crate, set()).update(FN.findall(source.read_text()))
    pooled = set().union(*per_crate.values()) if per_crate else set()
    return per_crate, pooled


def unresolved_citations(cell: str, per_crate: dict[str, set[str]], pooled: set[str]):
    """Yield each cited mechanism in `cell` that no `fn` declares."""
    for token in CITED.findall(cell):
        segments = token.lstrip(":").split("::")
        name = segments[-1]
        if name[:1].isupper():
            continue
        crate = segments[0] if segments[0].startswith("ambre_") else None
        pool = per_crate.get(crate, set()) if crate else pooled
        if name not in pool:
            where = f" in `{crate}`" if crate else " anywhere in the workspace"
            yield f"cites `{token}`, and no `fn {name}` exists{where}"


def declared_findings() -> dict[str, int]:
    """Every finding the audit declares, per id, with the line declaring it.

    Three shapes declare one: the heading of its section, a row of a status
    table, and a story of a remediation wave. Only the part of a heading before
    the dash is read, so `### H4 / H5 -- ...` declares two findings and the title
    beside them declares none.
    """
    found: dict[str, int] = {}
    for lineno, line in enumerate((SPECS / AUDIT).read_text().splitlines(), 1):
        ids: list[str] = []
        head = FINDING_HEAD.match(line)
        if head:
            title = head.group(1).split(" -- ")[0]
            ids = [t for t in re.split(r"[^A-Za-z0-9]+", title) if FINDING.match(t)]
        row = FINDING_ROW.match(line)
        if row:
            ids = [row.group(1)]
        story = FINDING_STORY.match(line)
        if story:
            ids = [story.group(1)]
        for fid in ids:
            found.setdefault(fid, lineno)
    return found


def cited_findings(line: str) -> list[str]:
    """Every token one row hands the word `audit`, whether or not it is an id."""
    tokens: list[str] = []
    for m in AUDIT_CITE.finditer(line):
        tokens += re.findall(r"`([^`]+)`", m.group(1))
    return tokens


def audit_coverage_defects(findings: dict[str, int], cited: set[str]) -> list[str]:
    """Whether every finding is held by a rule, or declared as holding none."""
    defects = []
    for fid, lineno in sorted(findings.items()):
        if fid in cited:
            if fid in NO_RULE:
                defects.append(
                    f"{AUDIT}:{lineno}: finding `{fid}` is cited by an invariant "
                    f"row and declared in `NO_RULE` -- one or the other"
                )
            continue
        if fid in NO_RULE:
            continue
        defects.append(
            f"{AUDIT}:{lineno}: finding `{fid}` is cited by no invariant row -- "
            f"state the rule that closes it in the spec of the component it "
            f"constrains, or declare it in `NO_RULE` with its reason"
        )
    for fid in sorted(NO_RULE):
        if fid not in findings:
            defects.append(
                f"`NO_RULE` carries `{fid}`, and {AUDIT} declares no such finding"
            )
    return defects


# The `Verification` answer under an invariant table, and the counts it opens
# on: `7 `verified`, 1 `unverified`, 1 `unimplemented`.` Both separators the
# corpus uses are read, the comma and the final "and".
ANSWER = re.compile(r"^\*\s+\*\*Verification\.\*\*\s+(?P<text>.*)$")
COUNT = re.compile(r"(?P<n>\d+)\s+`(?P<status>[a-z-]+)`")
COUNT_SEP = re.compile(r",\s+(?:and\s+)?")


def stated_counts(text: str) -> dict[str, int]:
    """The counts one `Verification` answer opens on, empty when it opens on none.

    Only the head of the bullet is read. What follows the counts names the gaps
    in prose and carries figures of its own -- "two of the four are rules the
    code answers with the opposite" -- and those figures are a review act, not a
    parse.
    """
    counts: dict[str, int] = {}
    pos = 0
    while (m := COUNT.match(text, pos)) is not None:
        counts[m["status"]] = int(m["n"])
        pos = m.end()
        sep = COUNT_SEP.match(text, pos)
        if sep is None:
            break
        pos = sep.end()
    return counts


def answer_defects(spec: str, answer: tuple[int, str] | None, counts: Counter) -> list[str]:
    """Whether the answer's counts are the counts of the table above it."""
    if answer is None:
        return []  # a missing label is `spec_shape.py`: one rule, one home
    lineno, text = answer
    where = f"{spec}:{lineno}"
    stated = stated_counts(text)
    if not stated:
        return [
            f"{where}: the `Verification` answer states no count -- it opens on "
            f"the count per status, then names the rows that are the gaps"
        ]
    defects = []
    for status in sorted(set(stated) | set(counts)):
        if status not in STATUSES:
            defects.append(
                f"{where}: the `Verification` answer counts {status!r}, which is "
                f"no status of the five"
            )
        elif stated.get(status, 0) != counts.get(status, 0):
            defects.append(
                f"{where}: the `Verification` answer says {stated.get(status, 0)} "
                f"`{status}` and the table holds {counts.get(status, 0)}"
            )
    return defects


def app_of(spec: str) -> str:
    """The application a spec belongs to, from the first component of its path."""
    head = spec.split("/", 1)[0]
    return head if head in APPS else "other"


def render_ledger(per_spec: dict[str, Counter], total: Counter) -> str:
    """The ledger file, whole. This is the only place its text is written."""
    shipped = sum(total[s] for s in STATUSES if s != "unimplemented")
    loose = sum(total[s] for s in LOOSE)
    by_app: dict[str, Counter] = {app: Counter() for app in APPS}
    for spec, counts in per_spec.items():
        by_app.setdefault(app_of(spec), Counter()).update(counts)

    lines = [
        "# AMBRE -- INVARIANT LEDGER",
        "",
        "**Type:** Inventory (generated)",
        "",
        "> The status of every invariant row in the corpus, rolled up. The rows themselves live in",
        "> the spec of the component they constrain, and this file reports their state, never intent.",
        "> Generated by `scripts/invariants.py` from the tables themselves, so it carries no fact of",
        "> its own.",
        "",
        "**Do not edit this file.** Edit the invariant row and run `just invariants-write`.",
        "",
        "* **Shipped** counts a row whose code exists. **Unbuilt** counts `unimplemented`, and the",
        "  split is the whole point: one number is a debt, the other is a plan.",
        "* **Loose** counts a shipped row that nothing would fail on: `unverified`, `human-review`,",
        "  `qa-manual`. That number is the coverage debt of the product.",
        "* How each layer is tested is `ambre-spec-testing.md`.",
        "",
        f"**{sum(total.values())} rows: {shipped} shipped, of which {loose} loose. "
        f"{total['unimplemented']} unbuilt.**",
        "",
        "| Status | " + " | ".join(a.split("-", 1)[-1] for a in APPS) + " | Total |",
        "| :--- | " + " | ".join(["---:"] * (len(APPS) + 1)) + " |",
    ]
    for status in STATUSES:
        cells = " | ".join(str(by_app[app][status] or "") for app in APPS)
        lines.append(f"| `{status}` | {cells} | {total[status]} |")

    lines += [
        "",
        "## THE SHIPPED SIDE, PER DOCUMENT",
        "",
        "> A document with no shipped row is omitted. `Loose` is the column to read.",
        "",
        "| Document | Rows | Shipped | `verified` | Loose | Unbuilt |",
        "| :--- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for spec, counts in sorted(per_spec.items()):
        built = sum(counts[s] for s in STATUSES if s != "unimplemented")
        if not built:
            continue
        slack = sum(counts[s] for s in LOOSE)
        lines.append(
            f"| `{Path(spec).name}` | {sum(counts.values())} | {built} | "
            f"{counts['verified']} | {slack or ''} | {counts['unimplemented'] or ''} |"
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    per_spec: dict[str, Counter] = {}
    seen: dict[str, str] = {}
    errors: list[str] = []
    per_crate, pooled = declared_fns()
    schedule = read_schedule(SPECS / ROADMAP)
    closed = read_schedule(SPECS / JOURNAL)
    errors += duplicate_stories(SPECS / ROADMAP)
    findings = declared_findings()
    cited_audit: set[str] = set()

    for path in sorted(SPECS.rglob("*.md")):
        if path.name == TEMPLATE:
            continue
        counts: Counter = Counter()
        answer: tuple[int, str] | None = None
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            m = ROW.match(line)
            # a story the row's own binding already answers for, checked below
            bound: set[str] = set()
            if m and m["status"] == "unimplemented":
                cite = cited_story(m["rest"].rsplit("|", 1)[-1])
                if cite:
                    bound.add(story_key(cite[0].strip('`"')))
            errors += loose_citation_defects(
                line, f"{path.relative_to(SPECS)}:{lineno}", bound, schedule, closed
            )
            bullet = ANSWER.match(line)
            if bullet:
                answer = (lineno, bullet["text"])
            if not m:
                continue
            rid, status, rest = m["id"], m["status"], m["rest"]
            where = f"{path.relative_to(SPECS)}:{lineno} {rid}"

            if status not in STATUSES:
                errors.append(f"{where}: unknown status {status!r}")
                continue
            if rid in seen:
                errors.append(f"{where}: duplicate id, already in {seen[rid]}")
            seen[rid] = str(path.relative_to(SPECS))

            for token in cited_findings(line):
                if not FINDING.match(token):
                    errors.append(
                        f"{where}: cites audit `{token}`, which is not a finding "
                        f"id -- a finding is cited by its number, `H9` or `N4`"
                    )
                elif token not in findings:
                    errors.append(
                        f"{where}: cites audit `{token}`, and {AUDIT} declares no "
                        f"finding of that number"
                    )
                else:
                    cited_audit.add(token)

            verification = rest.rsplit("|", 1)[-1]
            if status == "unimplemented":
                errors += story_defects(where, verification, schedule, closed)
            if status != "unimplemented" and "not built" in verification:
                errors.append(f"{where}: says 'not built' but status is {status!r}")
            if status != "unimplemented":
                for defect in unresolved_citations(verification, per_crate, pooled):
                    errors.append(f"{where}: {defect}")

            counts[status] += 1
        if counts:
            per_spec[str(path.relative_to(SPECS))] = counts
            errors += answer_defects(str(path.relative_to(SPECS)), answer, counts)

    errors += audit_coverage_defects(findings, cited_audit)

    # gaps in a prefix's numbering: an id that was renumbered or dropped
    by_prefix: dict[str, list[int]] = defaultdict(list)
    for rid in seen:
        prefix, num = rid.rsplit("-", 1)
        by_prefix[prefix].append(int(num))
    retired: dict[str, set[int]] = defaultdict(set)
    for rid in RETIRED:
        if rid in seen:
            errors.append(
                f"{rid}: retired in `invariants.py` and still declared by a spec"
                " -- a withdrawn id is never re-minted"
            )
        prefix, num = rid.rsplit("-", 1)
        retired[prefix].add(int(num))
    for prefix, nums in sorted(by_prefix.items()):
        nums.sort()
        missing = sorted(
            set(range(1, nums[-1] + 1)) - set(nums) - retired.get(prefix, set())
        )
        if missing:
            errors.append(
                f"{prefix}: gap in numbering, missing {missing} -- ids are a join "
                f"key and are never renumbered"
            )

    total: Counter = Counter()
    width = max(len(s) for s in per_spec)
    print(f"{'spec':<{width}}  " + "  ".join(f"{s:>13}" for s in STATUSES) + "   total")
    for spec, counts in per_spec.items():
        total += counts
        row = "  ".join(f"{counts[s] or '':>13}" for s in STATUSES)
        print(f"{spec:<{width}}  {row}   {sum(counts.values()):>5}")
    row = "  ".join(f"{total[s]:>13}" for s in STATUSES)
    print(f"{'TOTAL':<{width}}  {row}   {sum(total.values()):>5}")

    debt = total["verified"] + total["unverified"] + total["human-review"] + total["qa-manual"]
    print(
        f"\nshipped surface: {debt} invariants "
        f"({total['unverified']} unverified, {total['human-review']} human-review)"
    )
    print(f"specified, not built: {total['unimplemented']}")

    target = SPECS / LEDGER
    text = render_ledger(per_spec, total)
    if "--write" in sys.argv:
        target.write_text(text)
        print(f"\nwritten {target.relative_to(ROOT)}: {sum(total.values())} rows")
    else:
        current = target.read_text() if target.exists() else None
        if current is None:
            errors.append(f"{LEDGER}: missing -- run `just invariants-write`")
        elif current != text:
            errors.append(
                f"{LEDGER}: stale -- an invariant row changed and the ledger did not. "
                f"Run `just invariants-write`"
            )

    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
