#!/usr/bin/env python3
"""The boundaries that neither rustc nor cargo reads, held by a scan of the text.

Each is true of the tree today, and each was held by nothing: the rule read
as a fact about the current source rather than as a check the next commit has
to pass. Read them as one question -- what is reachable from where.

The crate boundaries that a manifest or a resolved name settles moved to
`just primitives`. The judge reads the manifests from `cargo metadata`, and
its rustc driver reads each path once it resolves, so an alias, a re-export
and the code a macro writes name what they reach (`ambre-spec-testing.md`
section 2). The driver also reads the types and the bodies a rule needs: a
generator's reads, a request id, a passphrase field, a payload in a relay log,
the switchboard's reach, the launcher's wiring, and the code rules of the UI
thread, the renderer, the kinetic model, a Trame line order, the bus, the JSON
wire, a fixture benchmark, and the chrome rules: a crypto crate a manifest
declares, an invitation, a named create, a window read, a media read, a
modal, a drag control, a two-state attribute, a menu's keyboard and an
unsandboxed frame. What stays here is a stylesheet, a page, a
shader, and a string a check reads as text.

**Two instances, one client per tab** (`ARCH-001`, `ARCH-011`, `SYNC-056`). The
page loads three artefacts: the launcher, the Sync worker and the call worker,
and names no worker two tabs would share. The manifests, the spawn and the
sandbox of a frame are `just primitives`.

**One composed name** (`CHROME-080`). The placeholder of an unnamed document
is spelled in `composed.rs` alone, whose type converts to no `String`. So no
Rust line outside that file spells it above its tests."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The crates, and the one file that reads the display density.
CRATES = "crates"
DENSITY = "crates/ambre-canvas/src/density.rs"


TESTS_START = re.compile(r"^\s*#\[cfg\(test\)\]")
INLINE_MODULE = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+\w+\s*\{")


# The code that runs on the UI thread: the page's own crates, and the view,
# render and chrome modules of each application.
UI_THREAD = (
    "crates/ambre-app/src",
    "crates/ambre-chrome/src",
    "crates/ambre-canvas/src",
    "crates/ambre-trame/src/ui",
    "crates/ambre-trame/src/render",
    "crates/ambre-trame/src/chrome",
    "crates/ambre-toile/src/ui",
    "crates/ambre-toile/src/chrome",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view",
    "crates/ambre-prose/src/chrome",
)
# A two-state ARIA attribute written so that its false state vanishes, a
# panel claiming to be a modal, a rendered menu, and what drives one from the
# keyboard (`ambre-spec-accessibility.md`).
PANEL_MODAL = re.compile(r'role\s*=\s*"(?:alert)?dialog"|\baria-modal\b')
# The one file whose dialog may claim a modal role (`CHROME-028`).
MODAL_OWNER = "crates/ambre-chrome/src/dialog.rs"
# The content's keyboard pipe, the order it must sit in, and what would take
# it out of the order or out of the accessibility tree.
PIPE = "crates/ambre-canvas/src/text_input.rs"
PIPE_ORDER = '("tabindex", "0")'
PIPE_HIDDEN = re.compile(r"\baria-hidden\b|\binert\b|\"tabindex\",\s*\"-")
# What leaves the subset both graphics back ends share in a shader. The Rust
# side is `just primitives` (`UI-THREAD-005`).
COMPUTE = re.compile(r"var<storage|@compute|\btexture_storage_\w+")

# The shell stylesheet, the three width queries it may hold, and what hiding
# looks like inside one.
STYLESHEET = "crates/ambre-app/styles.css"
WIDTH_QUERIES = ("(max-width: 599px)", "(min-width: 600px)", "(min-width: 1025px)")
WIDTH_MEDIA = re.compile(r"@media[^{]*\((?:min|max)-width:[^)]*\)[^{]*\{")
HIDING = re.compile(r"\bdisplay:\s*none\b|\bvisibility:\s*hidden\b")
# A width threshold spelled anywhere else: a width query, or a media read in
# a page or a script. The media read of Rust is `just primitives` (`CHROME-056`).
THRESHOLD = re.compile(r"\((?:min|max)-width:\s*\d+(?:px|em|rem)\s*\)|\bmatchMedia\b")
# A media read that asks for the user's motion setting, which no width rides.
MOTION_QUERY = re.compile(r"\(prefers-reduced-motion:\s*reduce\)")

# The page, its three artefacts, and what would serve two tabs or merge them.
PAGE = "crates/ambre-app/index.html"
RUST_LINK = re.compile(r'<link\s+data-trunk\s+rel="rust"[^>]*>', re.DOTALL)
WORKER_LINK = re.compile(r'href="\.\./ambre-sync/Cargo\.toml"[^>]*data-type="worker"')
# The call worker: a third instance the page spawns for a call alone (`ARCH-001`).
CALL_WORKER_LINK = re.compile(r'href="\.\./ambre-call/Cargo\.toml"[^>]*data-type="worker"')
SHARED_WORKER = re.compile(r"\bSharedWorker\b|\bServiceWorker\w*|\bserviceWorker\b|\bservice_worker\b")


# The one file that composes a document's name for display, and the placeholder
# a composition outside it would spell again.
COMPOSED_NAME = "crates/ambre-chrome/src/composed.rs"
PLACEHOLDER = re.compile(r'"Untitled(?:"| - )')



def read(rel: str) -> str:
    """The file's text, or an empty string when it is gone."""
    path = ROOT / rel
    return path.read_text() if path.exists() else ""


def code(line: str) -> str:
    """The line without its comment, so a rule a comment cites is not a breach."""
    return line.split("//", 1)[0]


def rust_files() -> list[Path]:
    """Every Rust file of the crates, build output left out."""
    return [p for p in sorted((ROOT / CRATES).rglob("*.rs")) if "/target/" not in p.as_posix()]


def sources(prefixes: tuple[str, ...]) -> list[Path]:
    """Every Rust file under each prefix, a prefix naming a file or a directory."""
    found: list[Path] = []
    for prefix in prefixes:
        path = ROOT / prefix
        if path.is_file():
            found.append(path)
        elif path.is_dir():
            found.extend(sorted(path.rglob("*.rs")))
    return found


def declared_for_tests(path: Path) -> bool:
    """Whether the module this file holds, or a module above it, is declared
    under `#[cfg(test)]` or `#[cfg(all(test, ...))]`. Such a file is test code
    from its first line."""
    while True:
        if path.name in ("lib.rs", "main.rs"):
            return False
        if path.name == "mod.rs":
            name, parent = path.parent.name, path.parent.parent
            outer = [parent / "mod.rs", parent.with_suffix(".rs"), parent / "lib.rs"]
        else:
            name, parent = path.stem, path.parent
            outer = [parent / "mod.rs", parent.with_suffix(".rs"), parent / "lib.rs"]
        declaring = next((f for f in outer if f.is_file()), None)
        if declaring is None:
            return False
        decl = re.compile(
            r"#\[cfg\((?:test|all\(test\b[^\]]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?mod\s+"
            + re.escape(name)
            + r"\s*;"
        )
        if decl.search(declaring.read_text()):
            return True
        path = declaring


def opens_test_module(lines: list[str], index: int) -> bool:
    """Whether line `index` is a `#[cfg(test)]` on an inline module. The same
    attribute on a statement, a function or a module declared elsewhere opens
    nothing, and the code after it is still the file's own."""
    if not TESTS_START.match(lines[index]):
        return False
    for line in lines[index + 1 :]:
        stripped = line.strip()
        if not stripped or stripped.startswith(("#[", "//")):
            continue
        return bool(INLINE_MODULE.match(line))
    return False


def above_tests(path: Path) -> list[tuple[int, str]]:
    """Each numbered line of the file above its test module, comments removed.
    A file whose module is declared for the tests has no such line."""
    if declared_for_tests(path):
        return []
    text = path.read_text().splitlines()
    lines: list[tuple[int, str]] = []
    for index, line in enumerate(text):
        if opens_test_module(text, index):
            break
        lines.append((index + 1, code(line)))
    return lines


def check_gpu_subset(errors: list[str]) -> int:
    """`UI-THREAD-005`: no compute stage and no storage binding in a shader.

    A shader is text no compile reads. The Rust side of the subset is `just
    primitives`, whose driver reads each wgpu name once it resolves.
    """
    files = sorted(p for p in (ROOT / CRATES).rglob("*.wgsl") if "/target/" not in p.as_posix())
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = COMPUTE.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: outside the shared graphics subset: `{found.group(0)}`")
    if not files:
        errors.append(f"{CRATES}: no shader read, so the graphics subset holds nothing")
    return len(files)


def check_widths(errors: list[str]) -> int:
    """`CHROME-030`, `-031`, `-056`: the widths live in one sheet.

    The sheet queries three widths and hides nothing inside one. No other file
    spells a width query. A content view that reads the window (`CHROME-034`)
    is `just primitives`.
    """
    sheet = read(STYLESHEET)
    if not sheet:
        errors.append(f"{STYLESHEET}: gone from the tree, so the widths read nothing")
        return 0
    depth, inside = 0, False
    for number, line in enumerate(sheet.splitlines(), 1):
        found = WIDTH_MEDIA.search(line)
        if found and depth == 0:
            query = re.search(r"\((?:min|max)-width:[^)]*\)", found.group(0))
            if query and query.group(0) not in WIDTH_QUERIES:
                errors.append(f"{STYLESHEET}:{number}: a width the chrome does not declare: `{query.group(0)}`")
            inside = True
        elif inside and HIDING.search(line):
            errors.append(f"{STYLESHEET}:{number}: a width query hides, so the narrow layout is not the base")
        depth += line.count("{") - line.count("}")
        if depth == 0:
            inside = False
    held = 1
    others = [
        p
        for p in (ROOT / CRATES).rglob("*")
        if p.suffix in (".rs", ".css", ".html") and "/target/" not in p.as_posix()
    ]
    for path in sorted(others):
        rel = path.relative_to(ROOT).as_posix()
        if rel in (STYLESHEET, DENSITY):
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = THRESHOLD.search(code(line) if path.suffix == ".rs" else line)
            if found and not MOTION_QUERY.search(line):
                errors.append(f"{rel}:{number}: a width spelled outside the stylesheet: `{found.group(0)}`")
        held += 1
    return held


def check_accessibility(errors: list[str]) -> int:
    """`CHROME-025`, `-028`, `-067`, `-068`, `-070`: what a control may spell.

    Only the shared dialog claims a modal role, so every modal is the
    platform's own. The pipe stays in the tab order and in the accessibility
    tree. A two-state attribute and a menu's keyboard (`CHROME-066`,
    `CHROME-026`) are `just primitives`.
    """
    files = sources(UI_THREAD)
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        lines = above_tests(path)
        for number, line in lines:
            found = PANEL_MODAL.search(line)
            if found and rel != MODAL_OWNER:
                errors.append(f"{rel}:{number}: a panel claiming a modal role: `{found.group(0)}`")
    pipe = ROOT / PIPE
    if not pipe.exists():
        errors.append(f"{PIPE}: gone from the tree, so the pipe's order and visibility are unread")
        return len(files)
    lines = above_tests(pipe)
    if not any(PIPE_ORDER in line for _, line in lines):
        errors.append(f"{PIPE}: the pipe no longer sets `{PIPE_ORDER}`, so it may leave the tab order")
    for number, line in lines:
        found = PIPE_HIDDEN.search(line)
        if found:
            errors.append(f"{PIPE}:{number}: the pipe is hidden or taken out of the order: `{found.group(0)}`")
    return len(files) + 1


def check_instances(errors: list[str]) -> int:
    """`ARCH-001`, `ARCH-011`, `SYNC-056`: the page loads three artefacts -- the
    launcher, the Sync worker and the call worker -- and names no shared worker.

    The manifests, the spawn of a worker and the sandbox of a frame are read by
    `just primitives`, from `cargo metadata` and through the rustc driver. An
    HTML page is text neither reads.
    """
    page = read(PAGE)
    if not page:
        errors.append(f"{PAGE}: gone from the tree, so the artefacts the page loads are unread")
        return 0
    links = RUST_LINK.findall(page)
    workers = [link for link in links if WORKER_LINK.search(link)]
    calls = [link for link in links if CALL_WORKER_LINK.search(link)]
    launchers = [link for link in links if "href=" not in link]
    if len(links) != 3 or len(workers) != 1 or len(calls) != 1 or len(launchers) != 1:
        errors.append(
            f"{PAGE}: the page loads {len(links)} wasm artefacts, not the launcher, the Sync worker "
            "and the call worker"
        )
    if SHARED_WORKER.search(page):
        errors.append(f"{PAGE}: the page names a worker that two tabs would share")
    return 1


def check_composed_names(errors: list[str]) -> int:
    """`CHROME-080`: no Rust line outside `composed.rs` spells the placeholder.

    A composition there returns a `ComposedName`, which converts to no
    `String`. One spelled elsewhere would be a `String` a rename takes.
    """
    if not PLACEHOLDER.search(read(COMPOSED_NAME)):
        errors.append(f"{COMPOSED_NAME}: spells no placeholder, so the composed-name boundary holds nothing")
        return 0
    files = rust_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel == COMPOSED_NAME:
            continue
        for number, line in above_tests(path):
            found = PLACEHOLDER.search(line)
            if found:
                errors.append(f"{rel}:{number}: a name composed outside `composed.rs`: `{found.group(0)}`")
    return len(files)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    args = parser.parse_args()

    errors: list[str] = []
    subset = check_gpu_subset(errors)
    widths = check_widths(errors)
    access = check_accessibility(errors)
    composed = check_composed_names(errors)
    instances = check_instances(errors)
    print(
        f"{subset} shaders read for a compute stage or a storage binding, "
        f"{widths} files read for a width spelled twice, "
        f"{access} UI files read for a panel modal or a hidden pipe, "
        f"{composed} Rust files read for a name composed outside its type, "
        f"{instances} page read for a fourth artefact or a shared worker"
    )
    for line in errors:
        print(f"  {line}")

    if errors:
        print(
            "\nERROR: a boundary was crossed. No shader leaves the shared "
            "subset, the chrome's widths live in one stylesheet, only the "
            "shared dialog is a modal, the pipe stays reachable, one file composes a document's name, the page loads its "
            "instances and shares no worker. The crate boundaries and the code "
            "rules rustc and cargo read are `just primitives`.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
