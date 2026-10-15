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

**The network leaves from the Sync worker** (`ARCH-005`). The workspace hands
every crate the browser's request and socket types, so the manifests prove
nothing on their own. Two lines are read instead: a manifest naming a
transport crate outside the worker and the relay, and a Rust line naming a
network call outside them. The two reads `UI-THREAD-004` names, the font fetch
and the sign-in configuration, are the stated exception, file by file.

**The worker writes nothing to the browser's storage** (`SYNC-054`,
`SYNC-060`). A decrypted blob and a session token both live in its memory, so
no Rust line of `ambre-sync` names browser storage, the browser database, the
cache or a cookie.

**No request carries a passphrase** (`ZK-012`). The worker holds the
passphrase in `SecretBytes`, which serializes so that it can cross the client
bus. So no serialized struct of `ambre-sync` declares a secret-typed field or
a field named for a passphrase or a password, and no JSON key or serde rename
spells one.

**Every crate takes the workspace lints** (`ARCH-032`). A lint denied in the
root table binds only the crates that inherit it, and a new crate starts with
the default set. So the root carries `[workspace.lints]`, and every member of
the workspace inherits it.

**The PDF back end runs in the Logic thread** (`CANVAS-028`). The crate graph
keeps krilla out of the startup bundle, but says nothing about which thread
calls it. So outside `ambre-pdf`, no Rust line names krilla, `ambre_pdf` or
`KrillaSurface` unless it sits under a crate's `src/logic/`.

**The renderer holds no colour** (`CANVAS-060`). Every colour it paints or
clears to comes from the view's palette. So no line of `render/` above its
tests names a colour constant or builds one from components.

**No font is embedded in a binary** (`CANVAS-018`). The faces are fetched from
the page origin. So no Rust line includes the bytes of a font file.

**The relay logs no payload** (`SRV-001`, `ZK-006`). A log line or a span may
carry identifiers, lengths and sequence numbers. So no log macro or span of
`ambre-server` names a value called for a payload, a blob, a body or its
bytes, unless it reads only the length. A payload under a neutral name is
left to the reviewer.

**Two instances, one client per tab** (`ARCH-001`, `ARCH-011`, `SYNC-056`). The
page loads two artefacts: the launcher, and the Sync worker as a worker of its
own. No crate but the worker depends on it, so the two never merge. The
launcher spawns one worker, and every frame it creates is sandboxed. No line
names a shared worker or a service worker, which would serve two tabs.

**Each thread declares what it may reach** (`ARCH-003`, `SYNC-096`). The Logic
runtime declares no render runtime and no interface toolkit. The Sync worker
reaches neither, nor the graphics stack, through any crate of ours.

**The wire types and the worker's messages** (`ARCH-006`, `BUS-004`,
`BUS-021`). The worker and the relay redeclare no type of `ambre-protocol`.
The worker allocates no request id, since its counter is not the Main
instance's. And it names no application message, so a payload crosses it as
bytes.

**One engine, one relay per frame** (`ARCH-007`, `ARCH-010`). The launcher
starts the Logic runtime from one line. The frame loop hands the route-free
relay to `drive_frame` as its first step, and that relay drains the Logic to
Sync queue.

**One kinetic model** (`KINETIC-001`). Pan, flick, wheel momentum and edge
scroll all come from `kinetic.rs`. A second copy of the model needs a decay,
a friction or a velocity of its own. So no Rust line outside that file names
one above its tests.
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

# The crates a network call may leave from: the Sync worker, and the relay,
# which is a server and not one of the two client instances. The browser belt
# is neither instance either: it is a test harness that serves the build and
# speaks WebDriver, and it ships nothing.
NETWORK_CRATES = ("crates/ambre-sync/", "crates/ambre-server/", "crates/ambre-belt/")
# The two reads `UI-THREAD-004` names as its exception: the same-origin font
# fetch and the pre-session sign-in configuration.
NETWORK_EXCEPTIONS = ("crates/ambre-canvas/src/host.rs", "crates/ambre-app/src/platform.rs")
TRANSPORT_CRATES = re.compile(
    r"^\s*(gloo-net|reqwest|tungstenite|tokio-tungstenite|ewebsock|ws_stream_wasm)\s*=",
    re.MULTILINE,
)
NETWORK_CALL = re.compile(
    r"\bfetch_with_(?:request|str)\w*|\bWebSocket::new\w*|\bXmlHttpRequest\b|\bEventSource\b"
    r"|\bsend_beacon\b|\bRtcPeerConnection\b|\bgloo_net\b|\breqwest\b"
)

# The Sync worker's source, and what reaching the browser's storage looks like
# from it: web storage, the browser database, the cache, the private file
# system or a cookie.
WORKER = "crates/ambre-sync/src"
STORAGE_CALL = re.compile(
    r"\blocal_storage\b|\bsession_storage\b|\bStorage\b|\bindexed_db\b|\bIdb\w+"
    r"|\bcaches\b|\bCacheStorage\b|\bStorageManager\b|\bget_directory\b|[Cc]ookie"
)

# What a passphrase in a request body looks like: a field of a serialized
# struct, or a string that a JSON body or a serde rename uses as a key.
SECRET_NAME = r"\w*(?:passphrase|password)\w*"
SERIALIZE_DERIVE = re.compile(r"#\[derive\([^)]*\bSerialize\b")
STRUCT_OPEN = re.compile(r"\bstruct\s+\w+[^;]*\{\s*$")
SECRET_FIELD = re.compile(
    rf"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:({SECRET_NAME})\s*:|\w+\s*:.*\b(Secret(?:Bytes|String))\b)",
    re.IGNORECASE,
)
SECRET_KEY = re.compile(rf"\"({SECRET_NAME})\"\s*:|rename\s*=\s*\"({SECRET_NAME})\"", re.IGNORECASE)


# The root manifest, its member list, its lint table, and the two lines that
# make a member inherit that table.
WORKSPACE = "Cargo.toml"
MEMBERS = re.compile(r"^members\s*=\s*\[([^\]]*)\]", re.MULTILINE)
LINT_TABLE = re.compile(r"^\[workspace\.lints(?:\.\w+)?\]", re.MULTILINE)
INHERITS_LINTS = re.compile(r"^\[lints\]\s*\n\s*workspace\s*=\s*true\b", re.MULTILINE)

# The PDF back end's own crate, the one directory per crate allowed to call
# it, and what calling it looks like from Rust.
PDF_CRATE = "crates/ambre-pdf/"
LOGIC_DIR = "/src/logic/"
PDF_CALL = re.compile(r"\bkrilla\b|\bambre_pdf\b|\bKrillaSurface\b")

# The renderer's source, and what a colour of its own looks like there: a
# named constant or one built from components.
RENDERER = "crates/ambre-canvas/src/render"
COLOR_LITERAL = re.compile(r"\bColor::(?:[A-Z][A-Z0-9_]*\b|from_rgba?8\b|from_rgb\b|new\b)")
TESTS_START = re.compile(r"^\s*#\[cfg\(test\)\]")

# What embedding a font looks like: an include macro naming a font file.
FONT_EMBED = re.compile(r"\binclude_bytes!\s*\([^)]*\.(?:ttf|otf|ttc|woff2?)\b", re.IGNORECASE)

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
CRYPTO_HOLDERS = ("crates/ambre-crypto/Cargo.toml", "crates/ambre-sync/Cargo.toml")
CRYPTO_FIXTURES = "crates/ambre-logic/Cargo.toml"
INVITE = re.compile(r"\bInviteToDocument\b")
INVITE_OWNER = "crates/ambre-app/src/tofu.rs"
# A two-state ARIA attribute written so that its false state vanishes, a
# panel claiming to be a modal, a rendered menu, and what drives one from the
# keyboard (`ambre-spec-accessibility.md`).
TWO_STATE_OMITTED = re.compile(r"aria-(?:pressed|expanded|checked|selected)\s*=[^\n]*\bthen_some\b")
PANEL_MODAL = re.compile(r'role\s*=\s*"(?:alert)?dialog"|\baria-modal\b')
MENU_ROLE = re.compile(r'role\s*=\s*"menu"')
MENU_KEYS = re.compile(r"\bon_menu_keydown\b|\bmenu_key\b")
# The content's keyboard pipe, the order it must sit in, and what would take
# it out of the order or out of the accessibility tree.
PIPE = "crates/ambre-canvas/src/text_input.rs"
PIPE_ORDER = '("tabindex", "0")'
PIPE_HIDDEN = re.compile(r"\baria-hidden\b|\binert\b|\"tabindex\",\s*\"-")
# What holding the document looks like: a CRDT container, an export, or an
# engine's state. A tree node's identifier is an address, not the document.
ENGINE = re.compile(
    r"\bLoro[A-Z]\w*|\bloro::(?!TreeID\b)\w+|\bExportMode\b|\b(?:Trame|Toile|Prose|Murmure)State\b"
)
# What converting a file looks like: the CSV reader or writer. A conversion
# runs in the Logic thread alone (`TRAME-IO-001`).
CONVERSION = re.compile(r"\bcsv::\w+")
# What waiting looks like: the conduit's parking handle or its parking send,
# a parked thread, a futex, a condition, or an executor run to completion.
BLOCKING = re.compile(
    r"\bWaiter\b|\.wait\(\)|\bthread::park\w*|\bAtomics::wait\w*|\bblock_on\b"
    r"|\bWorkerSender\b|\bCondvar\b"
)
# What drawing looks like from a page element: a canvas, or a context on one.
DRAWING = re.compile(
    r"\bHtmlCanvasElement\b|\bget_context\w*|\bCanvasRenderingContext2d\b|\bOffscreenCanvas\w*"
    r"|\bWebGl\w*|create_element\(\"canvas\"\)|<canvas\b"
)
# The surface itself: the drawing crate, the host that boots it, and the
# shell cell that holds the element.
DRAWING_OWNERS = (
    "crates/ambre-canvas/src/",
    "crates/ambre-app/src/host/",
    "crates/ambre-chrome/src/app_shell.rs",
)
# The persistence paths `UI-THREAD-004` names: the local store, and the
# sign-in's own state for one tab and for the development identity.
STORAGE_OWNERS = (
    "crates/ambre-logic/src/persistence.rs",
    "crates/ambre-app/src/signin.rs",
    "crates/ambre-app/src/platform.rs",
)
# The storage call as the browser's interface spells it, so a field that
# happens to be named `caches` is not one.
CLIENT_STORAGE = re.compile(
    r"\blocal_storage\b|\bsession_storage\b|\bweb_sys::Storage\b|\bindexed_db\b|\bIdb[A-Z]\w+"
    r"|\.caches\(\)|\bCacheStorage\b|\bStorageManager\b|\bget_directory\b|\.cookie\(\)|\bset_cookie\b"
)
# What leaves the subset both graphics back ends share.
COMPUTE = re.compile(
    r"\bComputePass\w*|\bComputePipeline\w*|\bcreate_compute_pipeline\b|\bBufferUsages::STORAGE\b"
    r"|\bBufferBindingType::Storage\b|\bStorageTexture\w*|var<storage|@compute|\btexture_storage_\w+"
)

# The shell stylesheet, the three width queries it may hold, and what hiding
# looks like inside one.
STYLESHEET = "crates/ambre-app/styles.css"
WIDTH_QUERIES = ("(max-width: 599px)", "(min-width: 600px)", "(min-width: 1025px)")
WIDTH_MEDIA = re.compile(r"@media[^{]*\((?:min|max)-width:[^)]*\)[^{]*\{")
HIDING = re.compile(r"\bdisplay:\s*none\b|\bvisibility:\s*hidden\b")
# A width threshold spelled anywhere else: a width query, or a media read.
THRESHOLD = re.compile(r"\((?:min|max)-width:\s*\d+(?:px|em|rem)\s*\)|\bmatch_media\b|\bmatchMedia\b")
# The relay's log lines, and the words that name a payload in them.
RELAY = "crates/ambre-server/src"
LOG_MACRO = re.compile(
    r"\b(?:tracing::|log::)?(?:(?:info|warn|error|debug|trace)(?:_span)?|println|eprintln|print|eprint)!\s*\("
    r"|#\[(?:tracing::)?instrument\b"
)
PAYLOAD_NAME = re.compile(
    r"\b\w*(?:payload|blob|plaintext|ciphertext|cipher|sealed|bytes|body)\w*\b", re.IGNORECASE
)
PAYLOAD_SAFE = re.compile(r"_(?:len|size|count|key|id|hash)$", re.IGNORECASE)
STRING_LITERAL = re.compile(r'"(?:[^"\\]|\\.)*"')
INTERPOLATION = re.compile(r"\{(\w+)")

# The page, its two artefacts, and what would serve two tabs or merge them.
PAGE = "crates/ambre-app/index.html"
RUST_LINK = re.compile(r'<link\s+data-trunk\s+rel="rust"[^>]*>', re.DOTALL)
WORKER_LINK = re.compile(r'href="\.\./ambre-sync/Cargo\.toml"[^>]*data-type="worker"')
SYNC_MANIFEST = "crates/ambre-sync/Cargo.toml"
SYNC_DEP = re.compile(r"^ambre-sync\s*=", re.MULTILINE)
WORKER_SPAWN = re.compile(r"\bWorker::new\w*\s*\(")
WORKER_SPAWNER = "crates/ambre-app/src/platform.rs"
SHARED_WORKER = re.compile(r"\bSharedWorker\b|\bServiceWorker\w*|\bserviceWorker\b|\bservice_worker\b")
FRAME_CREATE = re.compile(r'create_element\(\s*"iframe"\s*\)')

# What the Logic runtime and the Sync worker may not reach.
LOGIC_MANIFEST = "crates/ambre-logic/Cargo.toml"
RENDER_OR_TOOLKIT = ("ambre-canvas", "ambre-chrome", "ambre-app", "wgpu", "vello", "leptos")
SYNC_UNREACHABLE = RENDER_OR_TOOLKIT + (
    "ambre-draw", "ambre-trame", "ambre-toile", "ambre-prose", "ambre-murmure", "ambre-logic",
)
PATH_DEP = re.compile(r'^([\w-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"\.\./([\w-]+)"', re.MULTILINE)

# The wire types, and what the worker may not name.
PROTOCOL = "crates/ambre-protocol/src"
TYPE_DECL = re.compile(r"^\s*pub(?:\([^)]*\))?\s+(?:struct|enum)\s+([A-Z]\w*)", re.MULTILINE)
REQUEST_ALLOC = re.compile(r"\bRequestId::next\b")
ENGINE_SPAWN = re.compile(r"\bambre_logic::run\(")
APP_SRC = "crates/ambre-app/src"
APP_LIB = "crates/ambre-app/src/lib.rs"
FRAME_HOST = "crates/ambre-app/src/host/run.rs"
FRAME_RELAY = re.compile(r"drive_frame\(\s*&\w+,\s*&mut pre_frame\b")
SYNC_DRAIN = re.compile(r"\.drain\(&logic_to_sync_rx\b")
APP_MESSAGE = re.compile(
    r"\bambre_bus::app\b|\b(?:Trame|Toile|Prose|Murmure|App)(?:Command|Event)\b|\bLogicCommand\b|\bUiEvent\b"
)

# The shared kinetic model, and the words a second copy of it would need.
KINETIC_MODEL = "crates/ambre-draw/src/kinetic.rs"
KINETIC_TERM = re.compile(r"\b\w*(?:friction|decay|velocit|half_?life|inertia|damping)\w*\b", re.IGNORECASE)

# The two failure paths that reach the page with no queue: the panic hook of the
# shared instance, and the error event of the Sync worker.
PANIC_HOOK = "std::panic::set_hook("
WORKER_ERROR = "worker.set_onerror("
FATAL_LATCH = "ambre_canvas::signal_fatal("

# What runs on a failing thread's way out. The client build aborts on a panic,
# so no destructor runs then; a destructor in a thread that writes a document
# would still run on any other exit.
CARGO_CONFIG = ".cargo/config.toml"
BUILD_STD = re.compile(r"build-std\s*=\s*\[([^\]]*)\]")
EXIT_WRITERS = ("crates/ambre-logic/src", "crates/*/src/logic", "crates/ambre-sync/src")
DESTRUCTOR = re.compile(r"\bimpl\b[^{]*\bDrop\s+for\b")

# The engines' command handlers, found by layout so an engine added later is read
# too, and the names a handler that sends on a queue would need.
ENGINE_LOGIC = "crates/*/src/logic"
QUEUE_SEND = re.compile(
    r"\b(?:Sender|WorkerSender|Outbox)\b|\bconduit::|\.try_send\(|\bsend_to_(?:sync|ui|persist)\b"
)

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


def check_network(errors: list[str]) -> int:
    """`ARCH-005`: no transport crate and no network call outside the worker."""
    held = 0
    for path in sorted(ROOT.glob("crates/*/Cargo.toml")):
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith(NETWORK_CRATES):
            continue
        for found in TRANSPORT_CRATES.finditer(path.read_text()):
            errors.append(f"{rel}: declares the transport crate `{found.group(1)}`")
        held += 1
    files = sorted((ROOT / CRATES).rglob("*.rs"))
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith(NETWORK_CRATES) or rel in NETWORK_EXCEPTIONS or "/target/" in rel:
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = NETWORK_CALL.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: a network call outside the Sync worker: `{found.group(0)}`")
        held += 1
    if not held:
        errors.append(f"{CRATES}: no crate read, so the network boundary holds nothing")
    return held


def check_storage(errors: list[str]) -> int:
    """`SYNC-054`, `SYNC-060`: the worker names no browser storage."""
    files = sorted((ROOT / WORKER).rglob("*.rs"))
    if not files:
        errors.append(f"{WORKER}: gone from the tree, so the storage boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = STORAGE_CALL.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: the Sync worker reaches browser storage: `{found.group(0)}`")
    return len(files)


def check_passphrase(errors: list[str]) -> int:
    """`ZK-012`: no request body of the worker names a passphrase."""
    files = sorted((ROOT / WORKER).rglob("*.rs"))
    if not files:
        errors.append(f"{WORKER}: gone from the tree, so the passphrase boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        derived = False
        depth = 0
        for number, line in enumerate(path.read_text().splitlines(), 1):
            text = code(line)
            found = SECRET_KEY.search(text)
            if found:
                key = found.group(1) or found.group(2)
                errors.append(f"{rel}:{number}: a request key names a passphrase: `{key}`")
            if depth:
                field = SECRET_FIELD.match(text) if depth == 1 else None
                if field:
                    name = field.group(1) or field.group(2)
                    errors.append(f"{rel}:{number}: a serialized field names a passphrase: `{name}`")
                depth += text.count("{") - text.count("}")
            elif SERIALIZE_DERIVE.search(text):
                derived = True
            elif derived and STRUCT_OPEN.search(text):
                derived = False
                depth = 1
            elif derived and not text.strip().startswith("#["):
                derived = False
    return len(files)


def check_lints(errors: list[str]) -> int:
    """`ARCH-032`: the root carries a lint table, and every member inherits it."""
    root = read(WORKSPACE)
    if not LINT_TABLE.search(root):
        errors.append(f"{WORKSPACE}: no `[workspace.lints]` table, so a crate has nothing to inherit")
        return 0
    members = MEMBERS.search(root)
    if not members:
        errors.append(f"{WORKSPACE}: no `members` list, so no crate is read for the lint table")
        return 0
    held = 0
    for member in re.findall(r'"([^"]+)"', members.group(1)):
        rel = f"{member}/Cargo.toml"
        if not INHERITS_LINTS.search(read(rel)):
            errors.append(f"{rel}: does not inherit the workspace lints (`[lints]` `workspace = true`)")
        held += 1
    return held


def rust_files() -> list[Path]:
    """Every Rust file of the crates, build output left out."""
    return [p for p in sorted((ROOT / CRATES).rglob("*.rs")) if "/target/" not in p.as_posix()]


def check_pdf(errors: list[str]) -> int:
    """`CANVAS-028`: the PDF back end is named from a Logic directory alone."""
    files = rust_files()
    held = 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith(PDF_CRATE) or LOGIC_DIR in rel:
            continue
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = PDF_CALL.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: the PDF back end outside the Logic thread: `{found.group(0)}`")
        held += 1
    if not held:
        errors.append(f"{CRATES}: no Rust file read, so the PDF boundary holds nothing")
    return held


def check_renderer_colors(errors: list[str]) -> int:
    """`CANVAS-060`: no line of the renderer above its tests names a colour."""
    files = sorted((ROOT / RENDERER).rglob("*.rs"))
    if not files:
        errors.append(f"{RENDERER}: gone from the tree, so the colour boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            if TESTS_START.match(line):
                break
            found = COLOR_LITERAL.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: the renderer holds a colour of its own: `{found.group(0)}`")
    return len(files)


def check_fonts(errors: list[str]) -> int:
    """`CANVAS-018`: no Rust line embeds a font file."""
    files = rust_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = FONT_EMBED.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: a font embedded in the binary: `{found.group(0)}`")
    if not files:
        errors.append(f"{CRATES}: no Rust file read, so the font boundary holds nothing")
    return len(files)


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
        decl = re.compile(r"#\[cfg\((?:test|all\(test\b[^\]]*)\)\]\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+" + re.escape(name) + r"\s*;")
        if decl.search(declaring.read_text()):
            return True
        path = declaring


def above_tests(path: Path) -> list[tuple[int, str]]:
    """Each numbered line of the file above its test module, comments removed.
    A file whose module is declared for the tests has no such line."""
    if declared_for_tests(path):
        return []
    lines: list[tuple[int, str]] = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if TESTS_START.match(line):
            break
        lines.append((number, code(line)))
    return lines


def check_ui_thread(errors: list[str]) -> int:
    """`UI-THREAD-001`, `-002`, `-020`, `-050`, `-051`: what the UI thread may name.

    No CRDT and no engine state, no file conversion, no blocking wait, and no
    canvas or drawing context outside the surface (`TRAME-UI-001`,
    `TRAME-IO-001`).
    """
    files = sources(UI_THREAD)
    if not files:
        errors.append(f"{UI_THREAD[0]}: gone from the tree, so the UI thread reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            found = ENGINE.search(line)
            if found:
                errors.append(f"{rel}:{number}: the UI thread holds the document: `{found.group(0)}`")
            found = CONVERSION.search(line)
            if found:
                errors.append(f"{rel}:{number}: the UI thread converts a file: `{found.group(0)}`")
            found = BLOCKING.search(line)
            if found:
                errors.append(f"{rel}:{number}: the UI thread waits: `{found.group(0)}`")
            found = DRAWING.search(line)
            if found and not rel.startswith(DRAWING_OWNERS):
                errors.append(f"{rel}:{number}: a page element draws: `{found.group(0)}`")
    return len(files)


def check_client_storage(errors: list[str]) -> int:
    """`UI-THREAD-004`: browser storage is reached from the paths the rule names."""
    files = [
        path
        for path in rust_files()
        if not path.relative_to(ROOT).as_posix().startswith(NETWORK_CRATES)
        and path.relative_to(ROOT).as_posix() not in STORAGE_OWNERS
    ]
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            found = CLIENT_STORAGE.search(line)
            if found:
                errors.append(f"{rel}:{number}: a persistence path the rule does not name: `{found.group(0)}`")
    if not files:
        errors.append(f"{CRATES}: no Rust file read, so the persistence boundary holds nothing")
    return len(files)


def check_gpu_subset(errors: list[str]) -> int:
    """`UI-THREAD-005`: no compute pass and no storage buffer, in Rust or WGSL."""
    files = rust_files() + sorted(
        p for p in (ROOT / CRATES).rglob("*.wgsl") if "/target/" not in p.as_posix()
    )
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in enumerate(path.read_text().splitlines(), 1):
            found = COMPUTE.search(code(line))
            if found:
                errors.append(f"{rel}:{number}: outside the shared graphics subset: `{found.group(0)}`")
    if not files:
        errors.append(f"{CRATES}: no source read, so the graphics subset holds nothing")
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
            if found:
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
            errors.append(f"{rel}:{menus[0]}: a menu with no keyboard: it names neither `on_menu_keydown` nor `menu_key`")
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


def log_invocations(lines: list[tuple[int, str]]) -> list[tuple[int, str]]:
    """Each log macro or span of the lines, joined until its brackets close."""
    found: list[tuple[int, str]] = []
    index = 0
    while index < len(lines):
        number, line = lines[index]
        start = LOG_MACRO.search(line)
        if not start:
            index += 1
            continue
        text = line[start.start():]
        depth = text.count("(") + text.count("[") - text.count(")") - text.count("]")
        while depth > 0 and index + 1 < len(lines):
            index += 1
            more = lines[index][1]
            text += " " + more
            depth += more.count("(") + more.count("[") - more.count(")") - more.count("]")
        found.append((number, text))
        index += 1
    return found


def payload_names(invocation: str) -> list[str]:
    """The names of the invocation that call a payload and read more than its length."""
    names = INTERPOLATION.findall(" ".join(STRING_LITERAL.findall(invocation)))
    rest = STRING_LITERAL.sub(" ", invocation)
    names += [m.group(0) for m in PAYLOAD_NAME.finditer(rest) if not rest[m.end():].startswith(".len()")]
    return [n for n in names if PAYLOAD_NAME.fullmatch(n) and not PAYLOAD_SAFE.search(n)]


def check_payload_logs(errors: list[str]) -> int:
    """`SRV-001`, `ZK-006`: no log line or span of the relay names a payload."""
    files = sources((RELAY,))
    if not files:
        errors.append(f"{RELAY}: gone from the tree, so the relay's log lines are unread")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, invocation in log_invocations(above_tests(path)):
            if invocation.startswith("#[") and "skip_all" in invocation:
                continue
            for name in payload_names(invocation):
                errors.append(f"{rel}:{number}: a log line or span names a payload: `{name}`")
    return len(files)


def manifest_dependencies(manifest: str) -> set[str]:
    """The names the manifest's dependency tables declare, dev and build tables left out."""
    names: set[str] = set()
    table = ""
    for line in manifest.splitlines():
        header = re.match(r"^\s*\[([^\]]+)\]", line)
        if header:
            table = header.group(1)
            continue
        if table.endswith("dependencies") and "dev-" not in table and "build-" not in table:
            entry = re.match(r"^\s*([\w-]+)\s*=", line)
            if entry:
                names.add(entry.group(1))
    return names


def path_closure(crate: str) -> set[str]:
    """Every crate of ours the crate reaches through its normal path dependencies."""
    seen: set[str] = set()
    todo = [crate]
    while todo:
        name = todo.pop()
        if name in seen:
            continue
        seen.add(name)
        manifest = read(f"{CRATES}/{name}/Cargo.toml")
        declared = manifest_dependencies(manifest)
        todo.extend(target for dep, target in PATH_DEP.findall(manifest) if dep in declared)
    return seen


def check_instances(errors: list[str]) -> int:
    """`ARCH-001`, `ARCH-011`, `SYNC-056`: two artefacts, one worker, nothing shared."""
    page = read(PAGE)
    if not page:
        errors.append(f"{PAGE}: gone from the tree, so the artefacts the page loads are unread")
        return 0
    links = RUST_LINK.findall(page)
    workers = [link for link in links if WORKER_LINK.search(link)]
    launchers = [link for link in links if "href=" not in link]
    if len(links) != 2 or len(workers) != 1 or len(launchers) != 1:
        errors.append(f"{PAGE}: the page loads {len(links)} wasm artefacts, not the launcher and the Sync worker")
    if SHARED_WORKER.search(page):
        errors.append(f"{PAGE}: the page names a worker that two tabs would share")
    if "cdylib" not in read(SYNC_MANIFEST):
        errors.append(f"{SYNC_MANIFEST}: the Sync worker no longer builds an artefact of its own")
    manifests = sorted((ROOT / CRATES).glob("*/Cargo.toml"))
    for path in manifests:
        rel = path.relative_to(ROOT).as_posix()
        if rel != SYNC_MANIFEST and SYNC_DEP.search(path.read_text()):
            errors.append(f"{rel}: depends on the Sync worker, which merges it into a second instance")
    files = rust_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith("crates/ambre-server/"):
            continue
        lines = above_tests(path)
        for number, line in lines:
            found = WORKER_SPAWN.search(line)
            if found and rel != WORKER_SPAWNER:
                errors.append(f"{rel}:{number}: a worker spawned outside the launcher: `{found.group(0)}`")
            found = SHARED_WORKER.search(line)
            if found:
                errors.append(f"{rel}:{number}: a worker that two tabs would share: `{found.group(0)}`")
        text = path.read_text()
        test_only = "wasm_bindgen_test" in text
        if not test_only and any(FRAME_CREATE.search(line) for _, line in lines):
            if not any('"sandbox"' in line for _, line in lines):
                errors.append(f"{rel}: creates a frame and never sandboxes it")
    return len(manifests) + len(files) + 1


def check_thread_reach(errors: list[str]) -> int:
    """`ARCH-003`, `SYNC-096`: the Logic runtime and the worker reach no rendering."""
    logic = read(LOGIC_MANIFEST)
    if not logic or not read(SYNC_MANIFEST):
        errors.append(f"{LOGIC_MANIFEST} or {SYNC_MANIFEST}: gone from the tree, so the thread reach is unread")
        return 0
    for name in sorted(manifest_dependencies(logic) & set(RENDER_OR_TOOLKIT)):
        errors.append(f"{LOGIC_MANIFEST}: the Logic runtime declares `{name}`")
    closure = path_closure("ambre-sync")
    for crate in sorted(closure):
        manifest = f"{CRATES}/{crate}/Cargo.toml"
        reached = (manifest_dependencies(read(manifest)) | {crate}) & set(SYNC_UNREACHABLE)
        for name in sorted(reached):
            errors.append(f"{manifest}: the Sync worker reaches `{name}` through `{crate}`")
    return len(closure) + 1


def check_worker_messages(errors: list[str]) -> int:
    """`ARCH-006`, `BUS-004`, `BUS-021`: one wire type, no worker id, no app message."""
    wire = {name for path in sources((PROTOCOL,)) for name in TYPE_DECL.findall(path.read_text())}
    if not wire:
        errors.append(f"{PROTOCOL}: declares no type, so the wire boundary holds nothing")
        return 0
    files = sources((WORKER, "crates/ambre-server/src"))
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            found = TYPE_DECL.match(line)
            if found and found.group(1) in wire:
                errors.append(f"{rel}:{number}: redeclares the wire type `{found.group(1)}`")
            if not rel.startswith(WORKER):
                continue
            found = REQUEST_ALLOC.search(line) or APP_MESSAGE.search(line)
            if found:
                errors.append(f"{rel}:{number}: the worker allocates an id or names an app message: `{found.group(0)}`")
    return len(files)


def check_engine_and_relay(errors: list[str]) -> int:
    """`ARCH-007`, `ARCH-010`: one Logic start, and a relay that drains before the draw."""
    files = sources((APP_SRC,))
    starts = [
        f"{path.relative_to(ROOT).as_posix()}:{number}"
        for path in files
        for number, line in enumerate(path.read_text().splitlines(), 1)
        if ENGINE_SPAWN.search(code(line))
    ]
    if len(starts) != 1:
        errors.append(f"{APP_SRC}: starts the Logic runtime {len(starts)} times, not once: {starts}")
    if not FRAME_RELAY.search(read(FRAME_HOST)):
        errors.append(f"{FRAME_HOST}: the frame loop does not run `pre_frame` as its relay")
    if not SYNC_DRAIN.search(read(APP_LIB)):
        errors.append(f"{APP_LIB}: the relay does not drain the Logic to Sync queue")
    return len(files)


def check_failure_latches(errors: list[str]) -> int:
    """`ARCH-014`: each thread failure latches the page without a queue. A panic
    latches from the hook, and a failed Sync worker from its error event."""
    source = read(APP_LIB)
    hook = source.find(PANIC_HOOK)
    # The hook's body, up to the call that closes it.
    body = source[hook : source.find("}));", hook)] if hook >= 0 else ""
    if FATAL_LATCH not in body:
        errors.append(f"{APP_LIB}: the panic hook does not latch the fatal screen")
    source = read(WORKER_SPAWNER)
    handler = source.find(WORKER_ERROR)
    # The closure the handler installs, declared just above it.
    closure = source[source.rfind("Closure::", 0, handler) : handler] if handler >= 0 else ""
    if FATAL_LATCH not in closure:
        errors.append(f"{WORKER_SPAWNER}: the Sync worker's error event does not latch the fatal screen")
    return 2


def check_exit_writes(errors: list[str]) -> int:
    """`ARCH-015`: a failing thread writes nothing on its way out. The client
    build aborts on a panic, and no thread that writes a document has a
    destructor to run."""
    found = BUILD_STD.search(read(CARGO_CONFIG))
    if not found or '"panic_abort"' not in found.group(1) or "panic_unwind" in found.group(1):
        errors.append(f"{CARGO_CONFIG}: the client build does not abort on a panic")
    dirs = sorted(d for pattern in EXIT_WRITERS for d in ROOT.glob(pattern))
    files = sorted({path for directory in dirs for path in directory.rglob("*.rs")})
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            if DESTRUCTOR.search(line):
                errors.append(f"{rel}:{number}: a thread that writes a document runs a destructor on its way out")
    return len(files) + 1


def check_handlers(errors: list[str]) -> int:
    """`BUS-010`: an engine's command handler names no queue. It returns an
    outcome, and the dispatcher fans it out."""
    dirs = sorted(ROOT.glob(ENGINE_LOGIC))
    if not dirs:
        errors.append(f"{ENGINE_LOGIC}: no engine found, so no handler is read")
        return 0
    files = [path for directory in dirs for path in sorted(directory.rglob("*.rs"))]
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            found = QUEUE_SEND.search(line)
            if found:
                errors.append(f"{rel}:{number}: a command handler sends on a queue: `{found.group(0)}`")
    return len(files)


def check_kinetic_copy(errors: list[str]) -> int:
    """`KINETIC-001`: no Rust line outside the shared model names a decay or a velocity."""
    if not KINETIC_TERM.search(read(KINETIC_MODEL)):
        errors.append(f"{KINETIC_MODEL}: names no decay, so the kinetic boundary holds nothing")
        return 0
    files = rust_files()
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel == KINETIC_MODEL:
            continue
        for number, line in above_tests(path):
            found = KINETIC_TERM.search(line)
            if found:
                errors.append(f"{rel}:{number}: a kinetic model outside `kinetic.rs`: `{found.group(0)}`")
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
    network = check_network(errors)
    storage = check_storage(errors)
    secrets = check_passphrase(errors)
    lints = check_lints(errors)
    pdf = check_pdf(errors)
    fonts = check_fonts(errors)
    colors = check_renderer_colors(errors)
    ui_thread = check_ui_thread(errors)
    stores = check_client_storage(errors)
    subset = check_gpu_subset(errors)
    widths = check_widths(errors)
    controls = check_modals_and_drags(errors)
    keys = check_key_verification(errors)
    access = check_accessibility(errors)
    relay_logs = check_payload_logs(errors)
    kinetic = check_kinetic_copy(errors)
    instances = check_instances(errors)
    reach = check_thread_reach(errors)
    worker_messages = check_worker_messages(errors)
    engine = check_engine_and_relay(errors)
    handlers = check_handlers(errors)
    latches = check_failure_latches(errors)
    exits = check_exit_writes(errors)
    print(
        f"{manifests} manifests read for the fixture feature, "
        f"{generators} generator functions read for CRDT calls, "
        f"{toolkits} shared toolkit read for an application dependency, "
        f"{launcher} launcher files read for application logic, "
        f"{density} Rust files read for a density read, "
        f"{network} manifests and Rust files read for a network call, "
        f"{storage} worker files read for browser storage, "
        f"{secrets} worker files read for a passphrase in a request, "
        f"{lints} member manifests read for the workspace lints, "
        f"{pdf} Rust files read for the PDF back end, "
        f"{fonts} Rust files read for an embedded font, "
        f"{colors} renderer files read for a colour of their own, "
        f"{ui_thread} UI-thread files read for the document, a wait or a drawing, "
        f"{stores} client files read for a persistence path, "
        f"{subset} sources read for a compute pass or a storage buffer, "
        f"{widths} files read for a width spelled twice, "
        f"{controls} files read for a modal, a drag or a named creation, "
        f"{keys} manifests and files read for a key hash or an invitation, "
        f"{access} UI files read for a silent state, a panel modal, a mute menu or a hidden pipe, "
        f"{relay_logs} relay files read for a payload in a log line, "
        f"{kinetic} Rust files read for a second kinetic model, "
        f"{instances} files read for a third instance or a shared worker, "
        f"{reach} manifests read for a thread reaching the rendering, "
        f"{worker_messages} worker and relay files read for a wire copy, a worker id or an app message, "
        f"{engine} launcher files read for a second engine or a relay out of the frame, "
        f"{handlers} engine files read for a handler that sends on a queue, "
        f"{latches} launcher files read for a failure that waits on a queue, "
        f"{exits} build and thread files read for a write on the way out"
    )
    for line in errors:
        print(f"  {line}")

    if args.check and errors:
        print(
            "\nERROR: a crate boundary was crossed. Fixture machinery must stay "
            "out of the client build, a generator must drive `handle_command` "
            "rather than the CRDT, the shared chrome must name no "
            "application type, the launcher must name no application's "
            "command, event or command id, only density.rs may read the "
            "display density, the network leaves from the Sync worker alone, "
            "the worker writes nothing to browser storage, no request "
            "it sends names a passphrase, every crate inherits the "
            "workspace lints, the PDF back end is called from the Logic "
            "thread alone, the renderer holds no colour, no font is "
            "embedded in a binary, the UI thread holds no document, waits on "
            "nothing and draws on the surface alone, browser storage is reached "
            "from the named paths, no shader leaves the shared subset, the "
            "chrome's widths live in one stylesheet, one file renders the modal, "
            "a drag control debounces, a document is created unnamed, the worker "
            "alone hashes a key, an invitation waits on the trust decision, a "
            "two-state attribute says false, only the shared dialog is a "
            "modal, a menu answers the keyboard, the pipe stays reachable, the "
            "relay logs no payload, one file holds the kinetic model, the page "
            "loads two instances and shares no worker, neither the Logic runtime "
            "nor the worker reaches the rendering, and the worker redeclares no "
            "wire type, allocates no request id and names no app message. The launcher "
            "starts one engine, and every frame relays before it draws. A command "
            "handler returns an outcome and names no queue. A panic and a failed "
            "worker each latch the fatal screen without a queue, and a failing thread "
            "writes nothing on its way out.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
