#!/usr/bin/env python3
"""No format converter in the two instances: each one lives in the converter module.

The converter module holds every file format Ambre reads or writes, and the
Logic thread fetches it on the first conversion
(`specs/1-Foundations/ambre-spec-architecture.md section 2`, `ARCH-078`).
A converter that slips back into the Main instance or the Sync worker costs
every user its download, whether they convert or not.

The rule depends on features and targets: the Logic thread links the engine of
the module on the host, and only its loader on wasm. So the gate reads the
graph cargo resolves for each instance alone, on the wasm target, with its
normal dependencies: `cargo tree -p <crate> --target wasm32-unknown-unknown
-e normal`. A crate of `CONVERTERS` in that graph is a breach.

Run: `just converter-boundary`. Exits non-zero on a breach.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The crate of each wasm instance the page loads.
INSTANCES = {"Main": "ambre-app", "Sync": "ambre-sync"}

# The converters that live in the module alone: the PDF writer and the CSV
# reader and writer. A converter that moves into the module joins the list.
CONVERTERS = ("krilla", "ambre-pdf", "csv")


def linked(crate: str) -> list[str]:
    """The package names `crate` links on the wasm target, one per line of `cargo tree`."""
    result = subprocess.run(
        [
            "cargo",
            "tree",
            "--locked",
            "-p",
            crate,
            "--target",
            "wasm32-unknown-unknown",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo tree fails for {crate}: {result.stderr.strip()}")
    return [line.split(" ")[0] for line in result.stdout.splitlines() if line.strip()]


def breaches(instances: dict[str, list[str]]) -> list[str]:
    """One line per converter an instance links."""
    found = []
    for name, packages in instances.items():
        for converter in CONVERTERS:
            if converter in packages:
                found.append(f"the {name} instance links `{converter}`: it belongs in the converter module")
    return found


def main() -> int:
    try:
        instances = {name: linked(crate) for name, crate in INSTANCES.items()}
    except RuntimeError as err:
        print(f"converter_boundary: {err}", file=sys.stderr)
        return 1
    found = breaches(instances)
    for line in found:
        print(f"converter_boundary: {line}", file=sys.stderr)
    if not found:
        print(f"converter_boundary: neither instance links {', '.join(CONVERTERS)}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
