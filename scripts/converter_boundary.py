#!/usr/bin/env python3
"""No format converter in the two instances: each one lives in its family's module.

The converter modules hold every file format Ambre reads or writes, one module
per format family, and the Logic thread fetches each on the first conversion
of its family (`specs/1-Foundations/ambre-spec-architecture.md section 2`,
`ARCH-078`). A converter that slips back into the Main instance or the Sync
worker costs every user its download, whether they convert or not.

The rule depends on features and targets: the Logic thread links the engines
of the modules on the host, and only the loader on wasm. So the gate reads the
graph cargo resolves for each instance alone, on the wasm target, with its
normal dependencies: `cargo tree -p <crate> --target wasm32-unknown-unknown
-e normal`. A crate of `CONVERTERS` in that graph is a breach (`ARCH-078`).

Each module holds its own family alone (`ARCH-085`): a user who exports a CSV
never downloads the PDF writer. So the gate also reads the graph of
`ambre-convert` with each family's feature alone, as
`scripts/converter_build.py` builds it, and refuses a converter of another
family in it.

Run: `just converter-boundary`. Exits non-zero on a breach.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The crate of each wasm instance the page loads.
INSTANCES = {"Main": "ambre-app", "Sync": "ambre-sync"}

# The converters each family's module holds alone, by the family's feature of
# `ambre-convert`: the CSV reader and writer; the PDF writer; the Markdown
# parser and the HTML tree builder; the SVG reader and the XML parser of its
# text pass; the package layer of the office formats, its ZIP archive and the
# XML reader of their parts. A converter that moves into a module joins its
# family.
FAMILY_CONVERTERS = {
    "tables": ("csv",),
    "pdf": ("krilla", "ambre-pdf"),
    "markup": ("pulldown-cmark", "html5ever"),
    "images": ("usvg", "roxmltree"),
    "office": ("ambre-opc", "zip", "quick-xml", "jiff"),
}

# The converters two modules hold: the WebP decoder, which the images module
# runs and krilla links to embed a picture in a PDF. Neither instance may link
# one, and no family owns it.
SHARED_CONVERTERS = ("image-webp",)

# Every converter, which neither instance may link.
CONVERTERS = tuple(c for family in FAMILY_CONVERTERS.values() for c in family) + SHARED_CONVERTERS

# The crate the modules are built from.
MODULE_CRATE = "ambre-convert"


def linked(crate: str, features: str | None = None) -> list[str]:
    """The package names `crate` links on the wasm target, one per line of `cargo tree`."""
    feature_args = ["--features", features] if features else []
    result = subprocess.run(
        [
            "cargo",
            "tree",
            "--locked",
            "-p",
            crate,
            *feature_args,
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
                found.append(f"the {name} instance links `{converter}`: it belongs in a converter module")
    return found


def family_breaches(modules: dict[str, list[str]]) -> list[str]:
    """One line per converter of another family a family's module links."""
    found = []
    for family, packages in modules.items():
        for other, converters in FAMILY_CONVERTERS.items():
            if other == family:
                continue
            for converter in converters:
                if converter in packages:
                    found.append(
                        f"the {family} module links `{converter}`: it belongs in the {other} module"
                    )
    return found


def main() -> int:
    try:
        instances = {name: linked(crate) for name, crate in INSTANCES.items()}
        modules = {family: linked(MODULE_CRATE, family) for family in FAMILY_CONVERTERS}
    except RuntimeError as err:
        print(f"converter_boundary: {err}", file=sys.stderr)
        return 1
    found = breaches(instances) + family_breaches(modules)
    for line in found:
        print(f"converter_boundary: {line}", file=sys.stderr)
    if not found:
        print(f"converter_boundary: neither instance links {', '.join(CONVERTERS)}")
        print(f"converter_boundary: each of the {len(modules)} modules links its own family alone")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
