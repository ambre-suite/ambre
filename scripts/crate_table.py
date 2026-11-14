#!/usr/bin/env python3
"""The crate table and the crate map state the graph cargo resolves, and no other.

The crate graph is the boundary of the stack: one binary per instance, and no
runtime check (`specs/1-Foundations/ambre-spec-architecture.md section 2`). A
reader learns that graph from two tables, and nothing read either: the crate
table of the architecture spec, and the crate map of `README.md`. Both drifted.
The table missed crates and granted `ambre-crypto` to the launcher.

The gate reads `cargo metadata --no-deps`, which states each manifest as
written, and refuses a table that differs (`ARCH-106`):

* each table carries one row per workspace member, and no other row;
* a row of the architecture table names each direct dependency on a workspace
  crate, and no other one. A dev dependency is out: no shipped binary links it;
* an edge cargo resolves on one target alone, or only under a feature, carries
  its qualifier after the name: `(wasm only)`, `(host only)`, `(build only)`,
  `(optional)`, joined by a comma in that order.

A dependency of the `Depends on` cell is a crate name in backticks. Words
around the names are free, so a cell may name a third-party stack in prose.

Run: `just crate-boundaries`. Exits non-zero on a difference.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ARCHITECTURE = ROOT / "specs" / "1-Foundations" / "ambre-spec-architecture.md"
README = ROOT / "README.md"

# The header row each table opens with: the gate reads the table under it.
ARCHITECTURE_HEADER = "| Crate | Depends on | Why the line is there |"
README_HEADER = "| Crate | Purpose |"

WASM = "wasm"
HOST = "host"

# Each `target` string a manifest uses, and the platforms it selects. A new
# `cfg` fails the gate until it is placed here, rather than being read wrong.
PLATFORMS = {
    None: frozenset({WASM, HOST}),
    'cfg(target_arch = "wasm32")': frozenset({WASM}),
    'cfg(not(target_arch = "wasm32"))': frozenset({HOST}),
}

# The qualifiers, in the order a cell writes them.
ORDER = ("build only", "wasm only", "host only", "optional")

CRATE_CELL = re.compile(r"^`(ambre-[a-z0-9-]+)`$")
DEPENDENCY = re.compile(r"`(ambre-[a-z0-9-]+)`(?:\s*\(([^)]*)\))?")

Graph = dict[str, dict[str, tuple[str, ...]]]


def metadata() -> dict:
    """The manifests of the workspace as cargo reads them, with no dependency resolved."""
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo metadata fails: {result.stderr.strip()}")
    return json.loads(result.stdout)


def qualifier(entries: list[dict]) -> tuple[str, ...]:
    """The qualifier of one edge, from every manifest entry that declares it."""
    platforms: set[str] = set()
    for entry in entries:
        target = entry.get("target")
        if target not in PLATFORMS:
            raise RuntimeError(f"an edge under `{target}`, which `PLATFORMS` does not place")
        platforms |= PLATFORMS[target]
    found = []
    if all(entry.get("kind") == "build" for entry in entries):
        found.append("build only")
    if platforms == {WASM}:
        found.append("wasm only")
    if platforms == {HOST}:
        found.append("host only")
    if all(entry.get("optional") for entry in entries):
        found.append("optional")
    return tuple(found)


def graph(meta: dict) -> Graph:
    """Each workspace member, and its direct dependencies on members with their qualifier."""
    members = set(meta["workspace_members"])
    packages = [p for p in meta["packages"] if p["id"] in members]
    names = {p["name"] for p in packages}
    resolved: Graph = {}
    for package in packages:
        entries: dict[str, list[dict]] = {}
        for dependency in package["dependencies"]:
            if dependency.get("kind") == "dev" or dependency["name"] not in names:
                continue
            entries.setdefault(dependency["name"], []).append(dependency)
        resolved[package["name"]] = {name: qualifier(e) for name, e in entries.items()}
    return resolved


def table(text: str, header: str) -> list[list[str]]:
    """The cells of each body row of the one table that opens on `header`."""
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if line.strip() == header]
    if len(starts) != 1:
        raise RuntimeError(f"{len(starts)} tables open on `{header}`, where the gate reads one")
    rows = []
    for line in lines[starts[0] + 2 :]:
        if not line.startswith("|"):
            break
        rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    return rows


def crate_of(cell: str, errors: list[str], where: str) -> str | None:
    """The crate a `Crate` cell names, or None with an error."""
    match = CRATE_CELL.match(cell)
    if match is None:
        errors.append(f"{where}: the crate cell `{cell}` names no single crate in backticks")
        return None
    return match.group(1)


def stated_edges(cell: str, errors: list[str], where: str) -> dict[str, tuple[str, ...] | None]:
    """The dependencies a `Depends on` cell names, each with its qualifier.

    An edge whose qualifier does not parse maps to None: the error already names
    it, so the comparison skips it rather than report it a second time.
    """
    edges: dict[str, tuple[str, ...] | None] = {}
    for name, raw in DEPENDENCY.findall(cell):
        parts = tuple(part.strip() for part in raw.split(",")) if raw else ()
        unknown = [part for part in parts if part not in ORDER]
        if unknown:
            errors.append(f"{where}: `{name}` carries `({raw})`, where a qualifier is one of {', '.join(ORDER)}")
        misordered = not unknown and (
            list(parts) != sorted(parts, key=ORDER.index) or len(set(parts)) != len(parts)
        )
        if misordered:
            errors.append(f"{where}: `{name}` writes `({raw})` out of the order {', '.join(ORDER)}")
        if name in edges:
            errors.append(f"{where}: `{name}` is named twice")
            continue
        edges[name] = None if unknown or misordered else parts
    return edges


def written(name: str, parts: tuple[str, ...]) -> str:
    return f"`{name}` ({', '.join(parts)})" if parts else f"`{name}`"


def check_architecture(rows: list[list[str]], resolved: Graph) -> list[str]:
    """One line per difference between the architecture table and the graph."""
    errors: list[str] = []
    seen: set[str] = set()
    for cells in rows:
        if len(cells) != 3:
            errors.append(f"architecture table: the row `{' | '.join(cells)}` has {len(cells)} cells, not 3")
            continue
        crate = crate_of(cells[0], errors, "architecture table")
        if crate is None:
            continue
        where = f"architecture table, row `{crate}`"
        if crate in seen:
            errors.append(f"{where}: the crate has a second row")
            continue
        seen.add(crate)
        if crate not in resolved:
            errors.append(f"{where}: no workspace member has that name")
            continue
        stated = stated_edges(cells[1], errors, where)
        actual = resolved[crate]
        for name, parts in sorted(stated.items()):
            if parts is None:
                continue
            if name not in actual:
                errors.append(f"{where}: names {written(name, parts)}, a dependency cargo does not resolve")
            elif actual[name] != parts:
                errors.append(
                    f"{where}: writes {written(name, parts)}, where cargo resolves {written(name, actual[name])}"
                )
        for name, parts in sorted(actual.items()):
            if name not in stated:
                errors.append(f"{where}: omits {written(name, parts)}, a dependency cargo resolves")
    for crate in sorted(resolved.keys() - seen):
        errors.append(f"architecture table: the workspace member `{crate}` has no row")
    return errors


def check_readme(rows: list[list[str]], resolved: Graph) -> list[str]:
    """One line per difference between the crate map of the README and the members."""
    errors: list[str] = []
    seen: set[str] = set()
    for cells in rows:
        crate = crate_of(cells[0], errors, "README crate map")
        if crate is None:
            continue
        if crate in seen:
            errors.append(f"README crate map: `{crate}` has a second row")
        elif crate not in resolved:
            errors.append(f"README crate map: `{crate}` names no workspace member")
        seen.add(crate)
    for crate in sorted(resolved.keys() - seen):
        errors.append(f"README crate map: the workspace member `{crate}` has no row")
    return errors


def main() -> int:
    try:
        resolved = graph(metadata())
        errors = check_architecture(table(ARCHITECTURE.read_text(), ARCHITECTURE_HEADER), resolved)
        errors += check_readme(table(README.read_text(), README_HEADER), resolved)
    except RuntimeError as err:
        print(f"crate_table: {err}", file=sys.stderr)
        return 1
    for line in errors:
        print(f"crate_table: {line}", file=sys.stderr)
    if not errors:
        edges = sum(len(deps) for deps in resolved.values())
        print(f"crate_table: both tables state the {len(resolved)} members and the {edges} edges cargo resolves")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
