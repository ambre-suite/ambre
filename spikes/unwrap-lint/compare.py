#!/usr/bin/env python3
"""Compare the syn unwrap report with the sites of the spike lint.

Usage: compare.py SYN_REPORT LINT_SITES. SYN_REPORT is the output of
`ambre-primitives unwrap --ambiguous`; LINT_SITES the output of run.sh.
A site matches on its crate, file, line, newtype and accessor.
"""
import re
import sys
from collections import Counter


def syn_sites(path):
    refused, ambiguous = {}, {}
    for raw in open(path):
        line = raw.rstrip("\n")
        target = refused
        if line.startswith("ambiguous: "):
            target, line = ambiguous, line[len("ambiguous: "):]
        m = re.match(r"(\S+) \| (\S+):(\d+) \| (\S+)::(\w+) -> (.*)$", line)
        if not m:
            continue
        krate, file, no, newtype, acc, callee = m.groups()
        target[(krate, file, int(no))] = (newtype, acc, callee.strip())
    return refused, ambiguous


def lint_sites(path):
    sites = {}
    for raw in open(path):
        parts = raw.rstrip("\n").split("\t")
        if len(parts) != 4:
            continue
        krate, loc, what, origin = parts
        file, no = loc.rsplit(":", 1)
        m = re.match(r"crates/([^/]+)/(.*)$", file)
        if not m:
            continue
        nt, rest = what.split("::", 1)
        acc, callee = rest.split(" -> ", 1)
        sites.setdefault((m.group(1), m.group(2), int(no)), []).append((nt, acc, callee, origin))
    return sites


def main():
    refused, ambiguous = syn_sites(sys.argv[1])
    lint = lint_sites(sys.argv[2])
    both = [k for k in refused if k in lint]
    syn_only = [k for k in refused if k not in lint]
    lint_only = [k for k in lint if k not in refused]
    from_ambiguous = [k for k in lint_only if k in ambiguous]
    unseen = [k for k in lint_only if k not in ambiguous]
    cleared = [k for k in ambiguous if k not in lint]
    print(f"syn refusals: {len(refused)}, syn ambiguous: {len(ambiguous)}, lint sites: {len(lint)}")
    print(f"refused by both: {len(both)}")
    print(f"refused by syn alone: {len(syn_only)}")
    for k in sorted(syn_only):
        print("  ", " | ".join(map(str, k)), refused[k])
    print(f"refused by the lint, ambiguous for syn: {len(from_ambiguous)}")
    for k in sorted(from_ambiguous):
        print("  ", " | ".join(map(str, k)), lint[k], "syn:", ambiguous[k])
    print(f"refused by the lint, silent for syn: {len(unseen)}")
    for k in sorted(unseen):
        print("  ", " | ".join(map(str, k)), lint[k])
    print(f"ambiguous for syn, cleared by the lint: {len(cleared)}")
    print("   by accessor:", dict(Counter(ambiguous[k][1] for k in cleared).most_common(12)))


main()
