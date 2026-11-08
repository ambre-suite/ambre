#!/usr/bin/env python3
"""Compare the primitive allowlist with the `sig` sites of the census pass.

Usage: compare_sig.py ALLOWLIST CENSUS. A site matches on its crate, file,
kind and leaf name; an associated `const` matches a `const` line.
"""
import re
import sys
from collections import Counter


def leaf_of(item, kind):
    if kind == "field":
        return item.rsplit(".", 1)[-1]
    if kind in ("param", "ret"):
        head, _, rest = item.partition("(")
        return head.rsplit("::", 1)[-1] + "(" + rest
    return item.rsplit("::", 1)[-1]


def norm_kind(kind):
    return "const" if kind == "assoc-const" else kind


def allowlist(path):
    sites = Counter()
    for raw in open(path):
        if raw.startswith("#") or raw.startswith("key\t"):
            continue
        key = raw.split("\t")[0]
        parts = key.split(" | ")
        if len(parts) < 6:
            continue
        krate, file, _module, item, kind = parts[:5]
        sites[(krate, file, kind, leaf_of(item, kind))] += 1
    return sites


def census(path):
    sites, origins = Counter(), {}
    for raw in open(path):
        cols = raw.rstrip("\n").split("\t")
        if cols[0] != "sig" or cols[6] == "macro":
            continue
        _, _krate, loc, kind, leaf, _detail, origin = cols
        file = loc.rsplit(":", 1)[0]
        m = re.match(r"crates/([^/]+)/(.*)$", file)
        if not m:
            continue
        key = (m.group(1), m.group(2), kind, leaf)
        sites[key] += 1
        origins[key] = origin
    return sites, origins


def main():
    syn = allowlist(sys.argv[1])
    lint, origins = census(sys.argv[2])
    # An associated const reads as `assoc` here and may read as `const` there.
    for key in list(lint):
        if key[2] == "assoc" and key not in syn:
            alt = (key[0], key[1], "const", key[3])
            if alt in syn:
                lint[alt] += lint.pop(key)
                origins[alt] = origins.get(key, "")
    both = sum((syn & lint).values())
    syn_only = syn - lint
    lint_only = lint - syn
    print(f"allowlist sites: {sum(syn.values())}, census sites: {sum(lint.values())}, matched: {both}")
    print(f"allowlist alone: {sum(syn_only.values())}", dict(Counter(k[2] for k in syn_only.elements())))
    print(f"census alone: {sum(lint_only.values())}", dict(Counter((k[2], origins.get(k)) for k in lint_only.elements())))
    if len(sys.argv) > 3:
        for k in sorted(syn_only.elements())[: int(sys.argv[3])]:
            print("  syn  ", " | ".join(k))
        for k in sorted(lint_only.elements())[: int(sys.argv[3])]:
            print("  lint ", " | ".join(k), origins.get(k))


main()
