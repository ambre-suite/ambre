#!/usr/bin/env python3
"""Derive the invariant ledger from the spec tables, and check its rules.

The counts in `ambre-spec-testing.md §7` are a roll-up of every per-spec
invariant table. A hand-maintained roll-up is the same failure mode as a
hand-maintained test list: it is believed, and it is wrong (it was, by one, for
a week). This script derives it instead, and enforces the two rules that keep
`unimplemented` honest (`ambre-spec-architecture.md §4`):

  * a status must be one of the five declared values;
  * an `unimplemented` row must name the Story that will build it ("lands with").

Run: `just invariants`. Exits non-zero on any violation.
"""

import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

SPECS = Path(__file__).resolve().parent.parent / "specs"
TEMPLATE = "ambre-spec-template.md"  # carries example rows, not real invariants
STATUSES = ("verified", "unverified", "human-review", "qa-manual", "unimplemented")

ROW = re.compile(
    r"^\|\s*`(?P<id>[A-Z][A-Z0-9-]*-\d{3})`\s*\|"
    r"(?P<rest>.*)\|\s*(?P<status>[a-z-]+)\s*\|\s*$"
)


def main() -> int:
    per_spec: dict[str, Counter] = {}
    seen: dict[str, str] = {}
    errors: list[str] = []

    for path in sorted(SPECS.rglob("*.md")):
        if path.name == TEMPLATE:
            continue
        counts: Counter = Counter()
        for lineno, line in enumerate(path.read_text().splitlines(), 1):
            m = ROW.match(line)
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

            verification = rest.rsplit("|", 1)[-1]
            if status == "unimplemented" and "lands with" not in verification:
                errors.append(
                    f"{where}: `unimplemented` without a Story -- the Verification "
                    f"cell must read 'not built -- lands with <Story>'"
                )
            if status != "unimplemented" and "not built" in verification:
                errors.append(f"{where}: says 'not built' but status is {status!r}")

            counts[status] += 1
        if counts:
            per_spec[str(path.relative_to(SPECS))] = counts

    # gaps in a prefix's numbering: an id that was renumbered or dropped
    by_prefix: dict[str, list[int]] = defaultdict(list)
    for rid in seen:
        prefix, num = rid.rsplit("-", 1)
        by_prefix[prefix].append(int(num))
    for prefix, nums in sorted(by_prefix.items()):
        nums.sort()
        missing = sorted(set(range(1, nums[-1] + 1)) - set(nums))
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

    for e in errors:
        print(f"ERROR {e}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
