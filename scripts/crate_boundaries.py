#!/usr/bin/env python3
"""Three boundaries a manifest line or a stray call could erase silently.

Each is true of the tree today, and each was held by nothing: the rule read
as a fact about the current source rather than as a check the next commit has
to pass. Read the three as one question -- what is reachable from where.

**Fixture machinery out of the client** (`FIXTURE-007`). The machinery sits
behind a non-default `fixtures` feature, and Cargo never enables a dependency
feature implicitly. Two lines would undo that: `ambre-app` naming the feature
on a dependency, or any crate folding it into its own `default`. Both are
refused here.

**A generator drives commands** (`FIXTURE-004`). A generator that wrote to the
`LoroDoc` directly would produce a document no user action can reach, so the
fixture would prove nothing. The reading side of the same files is legitimate
and lives in `impl` blocks (`fixture_shape`, `canonical_dump`); the generating
side is the free functions. No free function of a fixture module may name a
CRDT container.

**A shared control emits one field** (`SCHEMA-002`). `ambre-chrome` depends on
no application crate, so a shared control has no whole-style type to emit and
can only report the scalar it edits. That holds as long as the manifest says
so.

**The launcher holds no application logic** (`ARCH-018`). What a command does
belongs to its application's crate. The launcher hands each event to the
application, sends the commands the application builds, and routes a command
id to the application's handler. So no line of `ambre-app` names a variant of
an application's command or event, or one of its command ids.

**One component reads the density** (`CANVAS-031`, `CANVAS-043`). Views lay
out in logical pixels, and the factor to device pixels never leaves
`ambre-canvas`. A second read of it anywhere is a second scale a view could
apply on top of the renderer's. So no Rust line outside `density.rs` names it.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The crate that is built into the wasm artifact.
CLIENT = "crates/ambre-app/Cargo.toml"
FIXTURE_FEATURE = "fixtures"

# Fixture modules: the reading half sits in `impl` blocks, the generating half
# in free functions.
FIXTURE_MODULES = [
    "crates/ambre-trame/src/logic/fixtures.rs",
    "crates/ambre-toile/src/logic/state/fixtures.rs",
]

# Naming any of these reaches a CRDT container rather than a command.
CRDT_CALLS = re.compile(
    r"\bLoroDoc\b|\.doc\b|\bget_map\(|\bget_list\(|\bget_movable_list\(|\bget_text\(|\bget_tree\("
)

# The shared chrome toolkit, and the crates a whole-style type could come from.
CHROME = "crates/ambre-chrome/Cargo.toml"
APP_CRATES = ("ambre-bus", "ambre-trame", "ambre-toile", "ambre-prose", "ambre-logic")

# The launcher's source, and what naming an application's logic looks like
# there: a variant of its command or event, or one of its command ids. The
# id table itself (`COMMANDS`) is what the shell registers, so it stays legal.
LAUNCHER = "crates/ambre-app/src"
APP_VARIANT = re.compile(r"\b(?:Trame|Toile|Prose|Murmure)(?:Command|Event)::[A-Z]\w*")
APP_COMMAND_ID = re.compile(
    r"\b(?:trame_chrome|toile_chrome|prose_chrome|murmure_chrome"
    r"|ambre_(?:trame|toile|prose|murmure)::(?:\w+::)*(?:commands|chrome))"
    r"::(?!COMMANDS\b)[A-Z][A-Z0-9_]*\b"
)

# The one file allowed to read the display density, and what reading it looks
# like from Rust: the `web_sys` call or the JavaScript property it wraps.
CRATES = "crates"
DENSITY = "crates/ambre-canvas/src/density.rs"
DENSITY_READ = re.compile(r"\bdevice_pixel_ratio\b|\bdevicePixelRatio\b")


def read(rel: str) -> str:
    """The file's text, or an empty string when it is gone."""
    path = ROOT / rel
    return path.read_text() if path.exists() else ""


def dependency_features(manifest: str) -> list[tuple[str, str]]:
    """Every `features = [...]` list of the manifest, with the line it sits on."""
    return [(m.group(0), m.group(1)) for m in re.finditer(r"features\s*=\s*\[([^\]]*)\]", manifest)]


def default_features(manifest: str) -> str:
    """The body of the manifest's own `default = [...]`, or an empty string."""
    m = re.search(r"^\s*default\s*=\s*\[([^\]]*)\]", manifest, re.MULTILINE)
    return m.group(1) if m else ""


def check_client(errors: list[str]) -> int:
    """`FIXTURE-007`: no manifest line reaches the fixture feature from the client."""
    held = 0
    client = read(CLIENT)
    if not client:
        errors.append(f"{CLIENT}: gone from the tree, so the client boundary reads nothing")
        return held
    for line, body in dependency_features(client):
        if FIXTURE_FEATURE in body:
            errors.append(f"{CLIENT}: the client enables `{FIXTURE_FEATURE}`: {line.strip()}")
    held += 1

    for path in sorted(ROOT.glob("crates/*/Cargo.toml")):
        rel = path.relative_to(ROOT).as_posix()
        if FIXTURE_FEATURE in default_features(path.read_text()):
            errors.append(f"{rel}: `{FIXTURE_FEATURE}` is in this crate's `default` feature set")
        held += 1
    return held


def free_functions(source: str) -> list[tuple[int, str]]:
    """Each free function of the file, as `(line number, body)`.

    A free function starts at column 0, which is what separates it from the
    `impl` blocks that hold the reading half of these modules. Its body runs to
    the next item at column 0.
    """
    lines = source.splitlines()
    starts = [i for i, line in enumerate(lines) if line.startswith("fn ")]
    bounds = [
        i
        for i, line in enumerate(lines)
        if line and not line[0].isspace() and not line.startswith("fn ")
    ] + [len(lines)]
    out = []
    for start in starts:
        end = next(b for b in bounds if b > start)
        out.append((start + 1, "\n".join(lines[start:end])))
    return out


def check_generators(errors: list[str]) -> int:
    """`FIXTURE-004`: no free function of a fixture module names a CRDT container."""
    held = 0
    for rel in FIXTURE_MODULES:
        source = read(rel)
        if not source:
            errors.append(f"{rel}: gone from the tree, so no generator is read")
            continue
        for line, body in free_functions(source):
            found = CRDT_CALLS.search(body)
            if found:
                errors.append(f"{rel}:{line}: a generator reaches the CRDT: `{found.group(0)}`")
            held += 1
    return held


def check_chrome(errors: list[str]) -> int:
    """`SCHEMA-002`: the shared toolkit cannot name an application's style type."""
    manifest = read(CHROME)
    if not manifest:
        errors.append(f"{CHROME}: gone from the tree, so the chrome boundary reads nothing")
        return 0
    for crate in APP_CRATES:
        if re.search(rf"^\s*{re.escape(crate)}\s*=", manifest, re.MULTILINE):
            errors.append(f"{CHROME}: the shared toolkit depends on `{crate}`")
    return 1


def code(line: str) -> str:
    """The line without its comment, so a rule a comment cites is not a breach."""
    return line.split("//", 1)[0]


def check_launcher(errors: list[str]) -> int:
    """`ARCH-018`: no line of the launcher names an application's logic."""
    root = ROOT / LAUNCHER
    files = sorted(root.rglob("*.rs"))
    if not files:
        errors.append(f"{LAUNCHER}: gone from the tree, so the launcher boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = APP_VARIANT.search(code(line)) or APP_COMMAND_ID.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: the launcher names application logic: `{found.group(0)}`")
    return len(files)


def check_density(errors: list[str]) -> int:
    """`CANVAS-031`: the density is read in `density.rs` and nowhere else."""
    if not DENSITY_READ.search(read(DENSITY)):
        errors.append(f"{DENSITY}: reads no density, so the density boundary holds nothing")
        return 0
    files = sorted((ROOT / CRATES).rglob("*.rs"))
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel == DENSITY or "/target/" in rel:
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = DENSITY_READ.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: reads the display density: `{found.group(0)}`")
    return len(files)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit non-zero on a breach")
    args = parser.parse_args()

    errors: list[str] = []
    manifests = check_client(errors)
    generators = check_generators(errors)
    toolkits = check_chrome(errors)
    launcher = check_launcher(errors)
    density = check_density(errors)
    print(
        f"{manifests} manifests read for the fixture feature, "
        f"{generators} generator functions read for CRDT calls, "
        f"{toolkits} shared toolkit read for an application dependency, "
        f"{launcher} launcher files read for application logic, "
        f"{density} Rust files read for a density read"
    )
    for line in errors:
        print(f"  {line}")

    if args.check and errors:
        print(
            "\nERROR: a crate boundary was crossed. Fixture machinery must stay "
            "out of the client build, a generator must drive `handle_command` "
            "rather than the CRDT, the shared chrome must name no "
            "application type, the launcher must name no application's "
            "command, event or command id, and only density.rs may read the "
            "display density.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
