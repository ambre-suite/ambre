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
wire and a fixture benchmark. What stays here is a stylesheet, a page, a
shader, a string a check reads as text, and the chrome rules a later story
moves.

**Two instances, one client per tab** (`ARCH-001`, `ARCH-011`, `SYNC-056`). The
page loads two artefacts: the launcher, and the Sync worker as a worker of its
own, and names no worker two tabs would share. Every frame the code creates is
sandboxed. The manifests and the spawn are `just primitives`.

**One composed name** (`CHROME-080`). The placeholder of an unnamed document
is spelled in `composed.rs` alone, whose type converts to no `String`. So no
Rust line outside that file spells it above its tests."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The manifest of the shared chrome toolkit.
CHROME = "crates/ambre-chrome/Cargo.toml"

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
# The one file that renders the native modal, and what a modal looks like.
MODAL_OWNER = "crates/ambre-chrome/src/dialog.rs"
MODAL = re.compile(r"<dialog\b")
# A control that reports on every pixel of a drag.
DRAG_INPUT = re.compile(r'type\s*=\s*"(?:range|color)"')
# The command that creates a named document, which the page never sends. A
# new document is created on its first open, by the one function below.
CREATE = re.compile(r"\bCreateDocument\b")
LAZY_CREATE = "crates/ambre-sync/src/updates.rs"
# The crates that may hash a key, the one that may only for its fixtures, and
# the one UI file that builds an invitation (`CHROME-049`, `CHROME-050`).
CRYPTO_DEP = re.compile(r"^ambre-crypto\s*=.*$", re.MULTILINE)
# The call worker seals and opens the chunks of a call (`CALL-003`): it holds the
# sender keys the Sync worker hands it, and hashes no key of a user.
CRYPTO_HOLDERS = (
    "crates/ambre-crypto/Cargo.toml",
    "crates/ambre-sync/Cargo.toml",
    "crates/ambre-call/Cargo.toml",
)
CRYPTO_FIXTURES = "crates/ambre-logic/Cargo.toml"
INVITE = re.compile(r"\bInviteToDocument\b")
INVITE_OWNER = "crates/ambre-app/src/tofu.rs"
# A two-state ARIA attribute written so that its false state vanishes, a
# panel claiming to be a modal, a rendered menu, and what drives one from the
# keyboard (`ambre-spec-accessibility.md`).
TWO_STATE_OMITTED = re.compile(r"aria-(?:pressed|expanded|checked|selected)\s*=[^\n]*\bthen_some\b")
PANEL_MODAL = re.compile(r'role\s*=\s*"(?:alert)?dialog"|\baria-modal\b')
MENU_ROLE = re.compile(r'role\s*=\s*"menu"')
MENU_KEYS = re.compile(r"\bon_menu_keydown\b|\b(?:event_)?menu_key\b")
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
# A width threshold spelled anywhere else: a width query, or a media read.
THRESHOLD = re.compile(r"\((?:min|max)-width:\s*\d+(?:px|em|rem)\s*\)|\bmatch_media\b|\bmatchMedia\b")
# A media read that asks for the user's motion setting, which no width rides.
MOTION_QUERY = re.compile(r"\(prefers-reduced-motion:\s*reduce\)")

# The page, its three artefacts, and what would serve two tabs or merge them.
PAGE = "crates/ambre-app/index.html"
RUST_LINK = re.compile(r'<link\s+data-trunk\s+rel="rust"[^>]*>', re.DOTALL)
WORKER_LINK = re.compile(r'href="\.\./ambre-sync/Cargo\.toml"[^>]*data-type="worker"')
# The call worker: a third instance the page spawns for a call alone (`ARCH-001`).
CALL_WORKER_LINK = re.compile(r'href="\.\./ambre-call/Cargo\.toml"[^>]*data-type="worker"')
SHARED_WORKER = re.compile(r"\bSharedWorker\b|\bServiceWorker\w*|\bserviceWorker\b|\bservice_worker\b")
FRAME_CREATE = re.compile(r'create_element\(\s*"iframe"\s*\)')


# The one file that composes a document's name for display, and the placeholder
# a composition outside it would spell again.
COMPOSED_NAME = "crates/ambre-chrome/src/composed.rs"
PLACEHOLDER = re.compile(r'"Untitled(?:"| - )')



# A content view reading the window rather than the box it is given.
CONTENT_VIEWS = (
    "crates/ambre-trame/src/ui",
    "crates/ambre-trame/src/render",
    "crates/ambre-toile/src/ui",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view",
)
WINDOW_READ = re.compile(
    r"\binner_(?:width|height)\b|\bouter_(?:width|height)\b|\bavail_(?:width|height)\b"
    r"|\.screen\(\)|\bmatch_media\b"
)


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
    """`CHROME-030`, `-031`, `-056`, `-034`: the widths live in one sheet.

    The sheet queries three widths and hides nothing inside one. No other file
    spells a width query, and no content view reads the window.
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
    for path in sources(CONTENT_VIEWS):
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            found = WINDOW_READ.search(line)
            if found:
                errors.append(f"{rel}:{number}: a content view reads the window: `{found.group(0)}`")
    return held


def check_modals_and_drags(errors: list[str]) -> int:
    """`CHROME-011`, `-015`, `-037`, `-084`: what a chrome control may spell.

    One file renders the native modal. A drag control lives in a file that
    debounces, and the UI thread never builds a document itself.
    """
    files = sources(UI_THREAD)
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        lines = above_tests(path)
        debounces = any("Debouncer" in line for _, line in lines)
        for number, line in lines:
            found = MODAL.search(line)
            if found and rel != MODAL_OWNER:
                errors.append(f"{rel}:{number}: a modal outside the shared dialog: `{found.group(0)}`")
            found = DRAG_INPUT.search(line)
            if found and not debounces:
                errors.append(f"{rel}:{number}: a drag control that is not debounced: `{found.group(0)}`")
            found = CREATE.search(line)
            if found:
                errors.append(f"{rel}:{number}: the UI thread creates a document: `{found.group(0)}`")
    lazy = read(LAZY_CREATE)
    body = lazy.split("fn trigger_lazy_create", 1)[1].split("\n    }\n", 1)[0] if lazy else ""
    names = re.findall(r"name_enc:\s*[^,\n]+", body)
    if names != ["name_enc: None"]:
        errors.append(f"{LAZY_CREATE}: the open-time create does not send `name_enc: None`: {names}")
    return len(files) + 1


def check_key_verification(errors: list[str]) -> int:
    """`CHROME-049`, `CHROME-050`: who may hash a key, and who may invite.

    The worker alone declares the crypto crate; the Logic crate may for its
    fixtures only. On the UI thread one file builds an invitation, behind the
    trust decision.
    """
    manifests = sorted(ROOT.glob("crates/*/Cargo.toml"))
    for path in manifests:
        rel = path.relative_to(ROOT).as_posix()
        for found in CRYPTO_DEP.finditer(path.read_text()):
            if rel in CRYPTO_HOLDERS:
                continue
            if rel == CRYPTO_FIXTURES and "optional = true" in found.group(0):
                continue
            errors.append(f"{rel}: declares the crypto crate outside the worker: {found.group(0)}")
    files = sources(UI_THREAD)
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel == INVITE_OWNER:
            continue
        for number, line in above_tests(path):
            found = INVITE.search(line)
            if found:
                errors.append(f"{rel}:{number}: an invitation built past the trust decision: `{found.group(0)}`")
    return len(manifests) + len(files)


def check_accessibility(errors: list[str]) -> int:
    """`CHROME-025`, `-026`, `-028`, `-066`, `-067`, `-068`, `-070`: what a control may spell.

    A two-state attribute always carries a value. Only the shared dialog
    claims a modal role, so every modal is the platform's own. A file that
    renders a menu drives it from the keyboard. The pipe stays in the tab
    order and in the accessibility tree.
    """
    files = sources(UI_THREAD)
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        lines = above_tests(path)
        for number, line in lines:
            found = TWO_STATE_OMITTED.search(line)
            if found:
                errors.append(f"{rel}:{number}: a two-state attribute that drops its false state: `{found.group(0)}`")
            found = PANEL_MODAL.search(line)
            if found and rel != MODAL_OWNER:
                errors.append(f"{rel}:{number}: a panel claiming a modal role: `{found.group(0)}`")
        menus = [number for number, line in lines if MENU_ROLE.search(line)]
        if menus and not any(MENU_KEYS.search(line) for _, line in lines):
            errors.append(f"{rel}:{menus[0]}: a menu with no keyboard: it names none of `on_menu_keydown`, `menu_key` and `event_menu_key`")
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
    launcher, the Sync worker and the call worker -- names no shared worker, and
    every frame the code creates is sandboxed.

    The manifests and the spawn of a worker are read by `just primitives`, from
    `cargo metadata` and through the rustc driver. An HTML page and the
    attribute a frame takes are text neither reads.
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
    files = rust_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("crates/ambre-server/"):
            continue
        lines = above_tests(path)
        text = path.read_text()
        test_only = "wasm_bindgen_test" in text
        if not test_only and any(FRAME_CREATE.search(line) for _, line in lines):
            if not any('"sandbox"' in line for _, line in lines):
                errors.append(f"{rel}: creates a frame and never sandboxes it")
    return len(files) + 1


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
    controls = check_modals_and_drags(errors)
    keys = check_key_verification(errors)
    access = check_accessibility(errors)
    composed = check_composed_names(errors)
    instances = check_instances(errors)
    print(
        f"{subset} shaders read for a compute stage or a storage binding, "
        f"{widths} files read for a width spelled twice, "
        f"{controls} files read for a modal, a drag or a named creation, "
        f"{keys} manifests and files read for a key hash or an invitation, "
        f"{access} UI files read for a silent state, a panel modal, a mute menu or a hidden pipe, "
        f"{composed} Rust files read for a name composed outside its type, "
        f"{instances} files read for a third artefact, a shared worker or an unsandboxed frame"
    )
    for line in errors:
        print(f"  {line}")

    if errors:
        print(
            "\nERROR: a boundary was crossed. No shader leaves the shared "
            "subset, the chrome's widths live in one stylesheet, one file "
            "renders the modal, a drag control debounces, a document is created "
            "unnamed, the worker alone hashes a key, an invitation waits on the "
            "trust decision, a two-state attribute says false, only the shared "
            "dialog is a modal, a menu answers the keyboard, the pipe stays "
            "reachable, one file composes a document's name, the page loads its "
            "instances and shares no worker, and every frame is sandboxed. The "
            "crate boundaries and the code rules rustc and cargo read are `just "
            "primitives`.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
