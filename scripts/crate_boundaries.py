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

**A generator reads only its seed** (`FIXTURE-032`). A real document and a
generated one are both Loro bytes. A generator that reads no file and nothing
of the host builds from the seed alone, so what it rebuilds came from no user.
No line of a generator module may read a file, embed one, or read the host's
environment.

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
passphrase in `Confidential<Passphrase>`, which serializes so that it can cross the client
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

**The relay logs no payload** (`SRV-170`). A log line or a span may
carry identifiers, lengths and sequence numbers. So no log macro or span of
`ambre-server` names a value called for a payload, a blob, a body or its
bytes, unless it reads only the length. A payload under a neutral name
passes that scan, so `SRV-001` adds a second: a file that reads a payload's
bytes -- `StoredPayload::expose_bytes`, a ciphertext's field, or a
ciphertext taken whole -- holds no log line. Every other binding is a
`StoredPayload`, which has no `Debug`.

**A fixture benchmark reports no frame rate** (`FIXTURE-024`). A fixture is a
document, not a screen, so a figure measured on one is the cost of an
operation. No code line of a benchmark that loads a fixture names a frame rate.

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

**One kinetic model** (`KINETIC-012`). Pan, flick, wheel momentum and edge
scroll all come from `kinetic.rs`. A second copy of the model needs a decay,
a friction or a velocity of its own. So no Rust line outside that file names
one above its tests.

**One composed name** (`CHROME-080`). The placeholder of an unnamed document
is spelled in `composed.rs` alone, whose type converts to no `String`. So no
Rust line outside that file spells it above its tests.

**An identifier typed on the bus** (`BUS-029`). Two identifiers carried as
two strings swap without a compile error. So no field of `ambre-bus` named
`id`, `*_id` or `*_ids` carries a bare string or integer above its tests.

**An identifier typed on the JSON wire** (`SRV-179`). The core identifier
writes its bytes through serde, so a JSON field held it as a string. So no
row or route of the relay, and no type of the HTTP layer of Sync, holds a
field named `id`, `*_id` or `*_ids` as a `String` or a `Uuid`. A serde field
that holds a core identifier names its text adapter.

**A closed set typed on the JSON wire** (`SRV-180`). A word held as a string
is parsed again by whoever reads it, and three parsers of the document type
disagreed on case and on a fallback. So the same rows, routes and types hold
no `doc_type`, `role`, `kind` or `status` field as a `String`. The refusal
of the relay joins them, so its `error` code is an enum too (`SRV-182`).

**An answer of the relay is a type** (`SRV-181`). A body built with `json!`
escapes both checks above, and a renamed field changes the wire unseen. So
no route, refusal or middleware of the relay builds one above its tests.

**A walk reads no screen state** (`CANVAS-074`). The screen and the page draw
one walk per application, so a walk that reached the screen would print a
caret, a selection or a zoom on the page. A file that implements
`DocumentWalk` reaches only the modules its crate lists below, and names no
type of the screen's state above its tests.
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

# The corpus driver reads and writes the corpus by design, so it is the one
# `fixtures.rs` that holds no generator.
FIXTURE_DRIVER = "crates/ambre-logic/src/fixtures.rs"

# Naming any of these reads something besides the seed: a file, or the host.
FILE_READS = re.compile(
    r"\bstd::fs\b|\bfs::|\bFile::|\bOpenOptions\b|\binclude_bytes!|\binclude_str!"
    r"|\bread_to_string\b|\bstd::env\b|\benv::var"
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
    rf"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:({SECRET_NAME})\s*:|\w+\s*:.*\b(Confidential)\b)",
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
INLINE_MODULE = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+\w+\s*\{")

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
MENU_KEYS = re.compile(r"\bon_menu_keydown\b|\b(?:event_)?menu_key\b")
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
# A pointer read relative to the element it lands on, and the two files that
# listen on the canvas (`UI-THREAD-001`). Elsewhere a canvas point has no use.
POINTER_READ = re.compile(r"\b(?:offset|layer)_[xy]\(")
POINTER_OWNERS = ("crates/ambre-app/src/host/", "crates/ambre-app/src/platform.rs")
# A pointer read relative to the page, and the canvas named by its id: the two
# halves of a canvas point an interface file would compute for itself.
PAGE_POINT = re.compile(r"\b(?:client|page|screen)_[xy]\(")
CANVAS_NAME = re.compile(r"\"\.?ambre-canvas\"")
# What hit-testing content looks like: the shared trait, each view's own
# resolver, and the camera inverse a resolver starts from.
HIT_CALL = re.compile(
    r"\b(?:hit_test|hit|cell_at|drag_cell_at|element_at|hit_topmost|place_at|marquee_hits"
    r"|exact_hit|screen_to_doc|screen_to_world)\("
)
# The interface components, which never hit-test content: the shared toolkit,
# each application's chrome, and the launcher outside the host.
INTERFACE = (
    "crates/ambre-chrome/src/",
    "crates/ambre-trame/src/chrome/",
    "crates/ambre-toile/src/chrome/",
    "crates/ambre-prose/src/chrome/",
    "crates/ambre-app/src/",
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
# A media read that asks for the user's motion setting, which no width rides.
MOTION_QUERY = re.compile(r"\(prefers-reduced-motion:\s*reduce\)")
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
# A reading of a payload's bytes: the one accessor of the relay's payload type,
# or a ciphertext taken whole by a function, bare or as the role of the
# generic `Payload<Ciphertext>`. The field of a ciphertext is private to
# `ambre-core`, so no relay file can read it.
PAYLOAD_BYTES = re.compile(
    r"\bexpose_bytes\b|:\s*&(?:\w+::)*(?:Payload\s*<\s*(?:\w+::)*)?Ciphertext\b"
)
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
# Each way the worker could mint a request id by name: the allocator, a
# number built with no family named, an issuer or a sequence number of the
# `Request` family, and the successor of the zero one. Inference can still
# hide the family, so the row's Mechanism claims these forms alone.
REQUEST_ALLOC = re.compile(
    r"\bnext_request\b|\bSeq\s*::\s*new\b|\bZERO\s*\.\s*next\b"
    r"|\b(?:Shared)?Issuer\s*(?:::\s*)?<\s*(?:\w+::)*Request\s*>"
    r"|\bSeq\s*::\s*<\s*(?:\w+::)*Request\s*>"
)
ENGINE_SPAWN = re.compile(r"\bambre_logic::run\(")
APP_SRC = "crates/ambre-app/src"
APP_LIB = "crates/ambre-app/src/lib.rs"
FRAME_HOST = "crates/ambre-app/src/host/run.rs"
# The relay is either borrowed whole, or shared with the hidden page's timer.
FRAME_RELAY = re.compile(
    r"drive_frame\(\s*&\w+,\s*(?:&mut pre_frame\b|\|\|\s*\(pre_frame\.borrow_mut\(\)\)\(\))"
)
SYNC_DRAIN = re.compile(r"\.drain\(&logic_to_sync_rx\b")
APP_MESSAGE = re.compile(
    r"\bambre_bus::app\b|\b(?:Trame|Toile|Prose|Murmure|App)(?:Command|Event)\b|\bLogicCommand\b|\bUiEvent\b"
)

# The shared kinetic model, and the words a second copy of it would need.
BENCH_FIXTURE = re.compile(r"fixture", re.IGNORECASE)
FRAME_RATE = re.compile(r"(?<![a-z])fps(?![a-z])|frames?[_ ]?per[_ ]?second|frame[_ ]?rate", re.IGNORECASE)

KINETIC_MODEL = "crates/ambre-draw/src/kinetic.rs"
KINETIC_TERM = re.compile(r"\b\w*(?:friction|decay|velocit|half_?life|inertia|damping)\w*\b", re.IGNORECASE)

# The drawing trait, and the closed list a primitive's parameters are held to.
# A bare number is geometry only under a width name, so a
# `density: f64` is refused although its type is listed. A length is
# `Px<Surface>`: the two words are listed, so a `Px<View>` is refused.
DRAW_TRAIT = "crates/ambre-draw/src/lib.rs"
DRAW_TRAIT_OPEN = re.compile(r"^pub trait DrawSurface\b")
PRIMITIVE = re.compile(r"\bfn\s+(\w+)\s*\(([^)]*)\)", re.S)
PRIMITIVE_TYPES = {
    "Rect", "Line", "Color", "f64", "Px", "Surface", "Affine", "BezPath", "Stroke", "Brush", "ParleyBrush", "GlyphRun",
    "ImageSource",
}
NUMBER_NAMES = {"width"}

# Each view's scroll setter, the signature that keeps it on a model step, and
# the offset a relative write elsewhere would move. A write that sets the
# offset outright names a target (the caret, a page, a cell) and stays free.
SCROLL_SETTERS = {
    "crates/ambre-prose/src/view/mod.rs": re.compile(r"^(\s*)pub fn scroll_by\(&mut self, \w+: ScrollStep\)"),
    "crates/ambre-toile/src/ui/viewport.rs": re.compile(r"^(\s*)pub fn pan_by_screen\(&mut self, \w+: ScrollStep\)"),
    "crates/ambre-trame/src/ui/view/scroll.rs": re.compile(r"^(\s*)fn accumulate\(&mut self, \w+: ScrollStep\)"),
    "crates/ambre-murmure/src/view.rs": re.compile(r"^(\s*)pub fn scroll_by\(&mut self, \w+: ScrollStep\)"),
}
SCROLL_APPS = ("crates/ambre-prose/src", "crates/ambre-toile/src", "crates/ambre-trame/src", "crates/ambre-murmure/src")
SCROLL_OFFSET = re.compile(r"\b(?:scroll_y|center|anchor_y)\s*(?:[+-]=|[+-](?!=))")
GRID_SCROLL = "crates/ambre-trame/src/ui/view/scroll.rs"
GRID_PAN = re.compile(r"\.scroll_by\(")
# A mouse drag is the one step built from two pointer positions, so only the
# board's pointer handler builds it.
DRAG_STEP = re.compile(r"\bScrollStep::dragged\(")
DRAG_CALLER = "crates/ambre-toile/src/ui/view.rs"

# The one file that composes a document's name for display, and the placeholder
# a composition outside it would spell again.
COMPOSED_NAME = "crates/ambre-chrome/src/composed.rs"
PLACEHOLDER = re.compile(r'"Untitled(?:"| - )')

# The structs holding the shell's signals: the launcher's wiring and each
# application's relay. A signal's payload names a piece of interface state.
SIGNAL_HOLDERS = {
    "crates/ambre-app/src/lib.rs": "Wiring",
    "crates/ambre-trame/src/chrome/relay.rs": "TrameRelay",
    "crates/ambre-toile/src/chrome/relay.rs": "ToileRelay",
}
# A view's own state: the structs of each application's view and render code.
VIEW_STATE = (
    "crates/ambre-trame/src/ui",
    "crates/ambre-trame/src/render",
    "crates/ambre-toile/src/ui",
    "crates/ambre-prose/src/ui.rs",
    "crates/ambre-prose/src/view",
)
STRUCT_OPEN = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?struct\s+(\w+)\b[^;(]*\{\s*$")
STRUCT_FIELD = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?([a-z_][a-z0-9_]*)\s*:\s*(.+?),?\s*$")
SIGNAL_FIELD = re.compile(r"^RwSignal<(.*)>$")
# A wrapper holds its payload's state, so a field matches through it.
TYPE_WRAPPERS = {"Option", "Vec", "VecDeque", "Box", "Rc", "Arc", "RefCell", "Cell"}
# A primitive or an identifier says nothing of the state it holds: a view keeps
# a hundred of each. Neither is matched (`UI-THREAD-003`).
PRIMITIVES = {"bool", "char", "str", "String", "Cow", "usize", "isize", "f32", "f64"} | {
    f"{sign}{bits}" for sign in "ui" for bits in (8, 16, 32, 64, 128)
}
# A view field a reviewer accepts beside a signal of its type, and why.
ONE_COPY_ACCEPTED = {
    "crates/ambre-trame/src/ui/view/mod.rs:TrameView.menu_scope": (
        "the host build holds no signal; the scope is written here and into the signal in one call"
    ),
    "crates/ambre-trame/src/ui/view/mod.rs:FormulaBar.action": (
        "the host build holds no signal; the frame copies the signal into it and clears the signal once served"
    ),
    "crates/ambre-toile/src/ui/tool.rs:ToolMachine.tool": (
        "the host build holds no signal; the frame reads the signal into it and publishes it back"
    ),
    "crates/ambre-toile/src/ui/style.rs:StyleModel.base": (
        "the selection's common style, which the inspector shows; the restyle signal carries an edit"
    ),
    "crates/ambre-toile/src/ui/view.rs:ToileView.present": (
        "the host build holds no signal; one PresentModeChanged event writes it here and into the signal"
    ),
    "crates/ambre-toile/src/ui/viewport.rs:ViewportTransform.zoom": (
        "the camera the frame draws at; the frame reads a chip pick from the zoom signal and publishes the camera back"
    ),
    "crates/ambre-toile/src/ui/view.rs:ToileView.zoom_published": (
        "the zoom the frame last published, which tells a chip pick in the signal from the frame's own write"
    ),
    "crates/ambre-toile/src/ui/present.rs:Transition.cam_to_zoom": (
        "the zoom a slide glide ends on, not the zoom the board shows"
    ),
    "crates/ambre-trame/src/ui/grid/mod.rs:TrameUiState.zoom": (
        "the host build holds no signal; the frame reads a chip pick from the signal and publishes its clamp back"
    ),
    "crates/ambre-trame/src/render/layout.rs:GridLayout.zoom": (
        "a copy the walk borrows for one paint, built from the grid state and dropped with the frame"
    ),
    "crates/ambre-prose/src/view/column.rs:Viewport.zoom": (
        "Prose trades no zoom signal: the zoom signal is the canvas apps' status chip"
    ),
}
# The modules a walk may reach, per application crate, and why each holds no
# screen state (`CANVAS-074`). A module not listed is refused, so a new reach
# is a reviewed line here.
WALK_REACH = {
    "ambre-prose": {
        "crate::layout::run": "the shaped runs the page lays out",
        "crate::view::cache": "the run cache, keyed by block and holding no camera",
        "crate::view": "the highlight types the block paints carry",
    },
    "ambre-trame": {
        "crate": "the sheet's dimensions",
        "crate::render": "the sheet's layout and cell drawing, in sheet coordinates",
    },
    "ambre-toile": {
        "crate::logic::animate": "the presented element's animated state",
        "crate::ui::label": "the label geometry, in board units",
    },
}
WALK_IMPL = re.compile(r"\bimpl\b[^{;]*\bDocumentWalk\s+for\b")
WALK_PATH = re.compile(r"\b(?:crate|super|self)(?:::(?:\w+|\*))+")
WALK_USE = re.compile(r"\buse\s+((?:crate|super|self)::[^;]*);", re.S)
SCREEN_STATE = re.compile(
    r"\b(?:\w*UiState|Viewport|ViewportTransform|TextEditor|CursorPosition|Clock|device_pixel_ratio)\b"
)
# The bus crate, whose identifier fields each carry a type of their own
# (`BUS-029`).
BUS = "crates/ambre-bus/src"
BUS_ID_FIELD = re.compile(r"\b((?:\w+_)?ids?)\s*:\s*(?!:)")
# The relay's rows and routes and the HTTP layer of Sync, whose identifiers
# each take their newtype and reach JSON as their text (`SRV-179`).
WIRE_DIRS = ("crates/ambre-server/src/routes", "crates/ambre-server/src/db")
WIRE_FILES = ("crates/ambre-sync/src/http.rs", "crates/ambre-server/src/error.rs")
WIRE_ID_NAME = re.compile(r"^(?:\w+_)?ids?$")
WIRE_UNTYPED = re.compile(r"\b(String|str|Uuid)\b")
CORE_ID = re.compile(
    r"\b(?:Document|User|Channel|Space|Meta|Segment|Category|Meeting|Snapshot|Org|Key)Id\b"
)
SERDE_DERIVE = re.compile(r"#\[derive\([^)]*\b(?:Serialize|Deserialize)\b")
TEXT_ADAPTER = re.compile(r"#\[serde\([^\]]*\bwith\s*=\s*\"ambre_core::serde_id::")
# Where the relay builds what it answers: each answer is a type, never a body
# built ad hoc (`SRV-181`).
ANSWER_SOURCES = (
    "crates/ambre-server/src/routes",
    "crates/ambre-server/src/error.rs",
    "crates/ambre-server/src/middleware",
)
AD_HOC_BODY = re.compile(r"\bjson!\s*[\(\[\{]")
# The fields whose word belongs to a closed set, parsed once at the edge into
# its enum (`SRV-180`). A refusal's `error` names its code (`SRV-182`).
WIRE_CLOSED_NAME = re.compile(r"^(?:doc_type|role|kind|status|error)$")
# A signal whose payload a reviewer accepts as a primitive, and why: free text
# has no unit and no closed set, so a type of its own would name nothing.
BARE_PAYLOAD_ACCEPTED = {
    "crates/ambre-app/src/lib.rs:Wiring.username": "free text: the display name the server reports",
    "crates/ambre-app/src/lib.rs:Wiring.formula_ref": "free text: the reference label the formula bar shows",
    "crates/ambre-app/src/lib.rs:Wiring.formula_text": "free text: the cell's raw text, as typed",
    "crates/ambre-app/src/lib.rs:Wiring.filter_values": "free text: the column's distinct display values",
    "crates/ambre-trame/src/chrome/relay.rs:TrameRelay.filter_values": (
        "free text: the column's distinct display values"
    ),
    "crates/ambre-app/src/lib.rs:Wiring.presenter_notes": "free text: the speaker notes of the slide",
    "crates/ambre-toile/src/chrome/relay.rs:ToileRelay.presenter_notes": "free text: the speaker notes of the slide",
}
# Each application's relay, keyed by the prefix of its event type. A view
# that keeps what an event says publishes it; the relay fills the rest.
EVENT_RELAYS = {
    "Trame": "crates/ambre-trame/src/chrome/relay.rs",
    "Toile": "crates/ambre-toile/src/chrome/relay.rs",
}
EVENT_ARM = re.compile(r"^(\s*)(?:\|\s*)?(?:Trame|Toile)Event::\w+\b")
EVENT_VARIANT = re.compile(r"\b(Trame|Toile)Event::(\w+)\b")
SIGNAL_WRITE = re.compile(r"\bself\s*\.\s*(\w+)\s*\.\s*(?:set|update)\(")
VIEW_STORE = re.compile(r"\bself\.((?:\w+\.)*\w+)\s*[-+*/|&^]?=(?!=)")
# A flag that asks for a repaint holds no state the event carries.
REPAINT_FLAGS = {"needs_full", "data_dirty"}
# An event whose view arm keeps a field while the relay writes a signal, the
# two holding different parts of it, and why.
ONE_EVENT_COPY_ACCEPTED = {
    "TrameEvent::SheetList -> sheets": "the view keeps the active sheet alone; the tab list is the strip's",
    "TrameEvent::FilterApplied -> filter_popover": (
        "the view keeps the filtered column and publishes the flag; the popover is chrome alone"
    ),
    "TrameEvent::FilterCleared -> filter_popover": (
        "the view keeps the filtered column and publishes the flag; the popover is chrome alone"
    ),
    "ToileEvent::SelectionStats -> sel_stats": (
        "the relay takes the band's reply alone; the view takes the bounds of a fit or a find focus"
    ),
    "ToileEvent::SlideChanged -> presenter_slide": "the view keeps the glide; the panel keeps the slide's position",
    "ToileEvent::SlideChanged -> presenter_notes": "the view keeps the glide; the panel keeps the speaker notes",
    "ToileEvent::SlideChanged -> presenter_frame": "the view keeps the glide; the panel keeps the notes' frame",
}

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


def generator_modules(paths: list[str]) -> list[str]:
    """The paths that hold a generator: each `fixtures.rs` of a crate's source,
    the driver aside. Read off the tree rather than listed, so a generator a new
    application adds is read without anyone naming it here."""
    return sorted(
        p
        for p in paths
        if p.startswith("crates/") and "/src/" in p and p.endswith("/fixtures.rs") and p != FIXTURE_DRIVER
    )


def file_reads(source: str) -> list[tuple[int, str]]:
    """Each line of a generator module that reads besides its seed, with what it
    names. The test module is not read: it builds no artifact."""
    found = []
    for number, line in enumerate(source.splitlines(), 1):
        if line.strip() == "#[cfg(test)]":
            break
        hit = FILE_READS.search(code(line))
        if hit:
            found.append((number, hit.group(0)))
    return found


def check_generator_reads(errors: list[str]) -> int:
    """`FIXTURE-032`: a generator reads no file and nothing of the host.

    Its output is then a function of the seed alone, so an artifact it rebuilds
    came from no user's document.
    """
    paths = [p.relative_to(ROOT).as_posix() for p in (ROOT / CRATES).rglob("fixtures.rs")]
    modules = generator_modules(paths)
    if not modules:
        errors.append(f"{CRATES}: no generator module found, so no generator read is checked")
        return 0
    for rel in modules:
        for line, token in file_reads(read(rel)):
            errors.append(f"{rel}:{line}: a generator reads besides its seed: `{token}`")
    return len(modules)


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
        text = path.read_text().splitlines()
        for number, line in enumerate(text, 1):
            if opens_test_module(text, number - 1):
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


def check_ui_thread(errors: list[str]) -> int:
    """`UI-THREAD-058`, `-002`, `-020`, `-050`, `-051`, `-060`: what the UI thread may name.

    No CRDT and no engine state, no file conversion, no blocking wait, and no
    canvas or drawing context outside the surface (`TRAME-UI-001`,
    `TRAME-IO-001`). No canvas point read outside the two files that listen on
    the canvas, and no content hit-test in an interface component.
    """
    files = sources(UI_THREAD)
    if not files:
        errors.append(f"{UI_THREAD[0]}: gone from the tree, so the UI thread reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        interface = rel.startswith(INTERFACE) and not rel.startswith(POINTER_OWNERS)
        page_point = canvas_name = None
        for number, line in above_tests(path):
            page_point = page_point or PAGE_POINT.search(line)
            canvas_name = canvas_name or CANVAS_NAME.search(line)
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
            found = POINTER_READ.search(line)
            if found and not rel.startswith(POINTER_OWNERS):
                errors.append(f"{rel}:{number}: a canvas point read off the host: `{found.group(0)}`")
            found = HIT_CALL.search(line)
            if found and rel.startswith(INTERFACE) and not rel.startswith(POINTER_OWNERS[0]):
                errors.append(f"{rel}:{number}: an interface component hit-tests content: `{found.group(0)}`")
        if interface and page_point and canvas_name:
            errors.append(
                f"{rel}: an interface component turns a page point into a canvas point: "
                f"`{page_point.group(0)}` beside `{canvas_name.group(0)}`"
            )
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
    """`SRV-170`: no log line or span of the relay names a payload."""
    files = sources((RELAY,))
    if not files:
        errors.append(f"{RELAY}: gone from the tree, so the relay's log lines are unread")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        lines = above_tests(path)
        invocations = log_invocations(lines)
        for number, invocation in invocations:
            if invocation.startswith("#[") and "skip_all" in invocation:
                continue
            for name in payload_names(invocation):
                errors.append(f"{rel}:{number}: a log line or span names a payload: `{name}`")
        # `SRV-001`: the bytes are read only where nothing is logged, so no
        # binding of them, whatever its name, sits beside a log line.
        reads = [(n, m.group(0)) for n, line in lines for m in [PAYLOAD_BYTES.search(line)] if m]
        if reads and invocations:
            number, found = reads[0]
            errors.append(f"{rel}:{number}: a file that logs reads a payload's bytes: `{found}`")
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
    """`KINETIC-012`: no Rust line outside the shared model names a decay or a velocity."""
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


def check_primitives(errors: list[str]) -> int:
    """`CANVAS-056`: every parameter of a drawing primitive is geometry or paint.

    The trait body is read up to its closing brace, comments removed, so a
    signature split over lines is read whole."""
    lines = read(DRAW_TRAIT).splitlines()
    start = next((i for i, line in enumerate(lines) if DRAW_TRAIT_OPEN.match(line)), None)
    if start is None:
        errors.append(f"{DRAW_TRAIT}: no `DrawSurface` trait, so the primitive boundary holds nothing")
        return 0
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("}")), len(lines))
    body = "\n".join(code(line) for line in lines[start + 1 : end])
    held = 0
    for name, params in PRIMITIVE.findall(body):
        held += 1
        for param in params.split(","):
            if ":" not in param:
                continue
            label, kind = (part.strip() for part in param.split(":", 1))
            idents = set(re.findall(r"(?<!')\b[A-Za-z_]\w*", kind)) - {"mut", "_"}
            foreign = sorted(idents - PRIMITIVE_TYPES)
            if foreign:
                errors.append(f"{DRAW_TRAIT}: `{name}` takes `{label}: {kind}`, which is neither geometry nor paint")
            elif kind == "f64" and label not in NUMBER_NAMES:
                errors.append(f"{DRAW_TRAIT}: `{name}` takes a bare number `{label}`, not a width")
    if not held:
        errors.append(f"{DRAW_TRAIT}: the `DrawSurface` trait declares no primitive, so the boundary holds nothing")
    return held


def check_scroll_sources(errors: list[str]) -> int:
    """`KINETIC-001`: each view's scroll setter takes a step of the shared
    model, and no other line of an application moves an offset by a distance.

    The setter's own body is the one place a relative write is read as the
    model's. The Trame grid pans by whole cells from its accumulator alone."""
    exempt: dict[str, range] = {}
    for rel, signature in SCROLL_SETTERS.items():
        lines = read(rel).splitlines()
        start = next((i for i, line in enumerate(lines) if signature.match(line)), None)
        if start is None:
            errors.append(f"{rel}: the scroll setter takes no model step, so a bare distance reaches the view")
            continue
        close = signature.match(lines[start]).group(1) + "}"
        end = next((i for i in range(start + 1, len(lines)) if lines[i] == close), len(lines))
        exempt[rel] = range(start + 1, end + 2)
    files = [path for app in SCROLL_APPS for path in sorted((ROOT / app).rglob("*.rs"))]
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel == GRID_SCROLL:
            continue
        for number, line in above_tests(path):
            if number in exempt.get(rel, ()):
                continue
            found = SCROLL_OFFSET.search(line)
            if found:
                errors.append(f"{rel}:{number}: an offset moved by a distance outside its setter: `{found.group(0).strip()}`")
            if rel.startswith("crates/ambre-trame/") and GRID_PAN.search(line):
                errors.append(f"{rel}:{number}: the grid panned outside its accumulator")
    for path in rust_files():
        rel = path.relative_to(ROOT).as_posix()
        if rel in (KINETIC_MODEL, DRAG_CALLER):
            continue
        for number, line in above_tests(path):
            if DRAG_STEP.search(line):
                errors.append(f"{rel}:{number}: a drag step built away from the board's pointer")
    return len(files)


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


def struct_fields(lines: list[tuple[int, str]]) -> list[tuple[str, int, str, str]]:
    """Each named field of each braced struct: the struct, the line, the field
    and its type. An attribute line or a field spread over lines is skipped."""
    fields: list[tuple[str, int, str, str]] = []
    current: str | None = None
    for number, line in lines:
        opened = STRUCT_OPEN.match(line)
        if opened:
            current = opened.group(1)
            continue
        if current is None:
            continue
        if line.strip().startswith("}"):
            current = None
            continue
        found = STRUCT_FIELD.match(line)
        if found and not line.strip().startswith("#"):
            fields.append((current, number, found.group(1), found.group(2).strip()))
    return fields


def state_type(ty: str) -> str | None:
    """The type a field holds state of: wrappers peeled, the path's last segment.
    `None` for a tuple, an array, a primitive or an identifier."""
    ty = ty.strip().lstrip("&").strip()
    generic = re.match(r"^([\w:]+)\s*<(.*)>$", ty)
    if generic:
        head = generic.group(1).split("::")[-1]
        if head in TYPE_WRAPPERS:
            return state_type(generic.group(2))
        ty = head
    if not re.fullmatch(r"[\w:]+", ty):
        return None
    name = ty.split("::")[-1]
    if name in PRIMITIVES or name.endswith("Id"):
        return None
    return name


def check_one_copy(errors: list[str]) -> int:
    """`UI-THREAD-059`: no view field holds a type a shell signal carries.

    The signal is the one copy of that state. A view field beside it agrees
    until one frame writes only one of them.
    """
    signals: dict[str, str] = {}
    for rel, holder in SIGNAL_HOLDERS.items():
        text = read(rel)
        lines = [(number, code(line)) for number, line in enumerate(text.splitlines(), 1)]
        payloads = [
            (field, found.group(1))
            for struct, _, field, ty in struct_fields(lines)
            if struct == holder and (found := SIGNAL_FIELD.match(ty))
        ]
        if not payloads:
            errors.append(f"{rel}: `{holder}` holds no signal, so no interface state is read")
            continue
        for field, payload in payloads:
            held = state_type(payload)
            if held:
                signals.setdefault(held, field)
    files = sources(VIEW_STATE)
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for struct, number, field, ty in struct_fields(above_tests(path)):
            if "Signal" in ty or f"{rel}:{struct}.{field}" in ONE_COPY_ACCEPTED:
                continue
            held = state_type(ty)
            if held in signals:
                errors.append(
                    f"{rel}:{number}: `{struct}.{field}` holds a `{held}`, which the `{signals[held]}` signal already holds"
                )
    return len(files)


def event_arms(lines: list[tuple[int, str]]) -> list[tuple[list[str], list[tuple[int, str]]]]:
    """Each match arm on an application event: the event types it names, as
    `TrameEvent::SheetList`, and its numbered lines, the pattern included."""
    arms = []
    index = 0
    while index < len(lines):
        found = EVENT_ARM.match(lines[index][1])
        if not found:
            index += 1
            continue
        indent = len(found.group(1))
        body = [lines[index]]
        index += 1
        while index < len(lines):
            line = lines[index][1]
            depth = len(line) - len(line.lstrip())
            closing = line.lstrip().startswith(("}", ")", "|"))
            if line.strip() and (depth < indent or (depth == indent and not closing)):
                break
            body.append(lines[index])
            index += 1
        # The pattern runs to the arrow; a variant named past it is a value.
        pattern = []
        for _, line in body:
            pattern.append(line.split("=>", 1)[0])
            if "=>" in line:
                break
        variants = [f"{app}Event::{name}" for app, name in EVENT_VARIANT.findall(" ".join(pattern))]
        arms.append((variants, body))
    return arms


def check_one_event_copy(errors: list[str]) -> int:
    """`UI-THREAD-003`: no event feeds both a view field and a shell signal.

    A view that keeps what an event says publishes it into the signal itself,
    so the relay writes no signal off an event a view arm stores from. Two
    writers would agree until one frame wrote only one of them.
    """
    written: dict[str, list[tuple[str, str]]] = {}
    arms = 0
    for app, rel in EVENT_RELAYS.items():
        lines = [(number, code(line)) for number, line in enumerate(read(rel).splitlines(), 1)]
        found = event_arms(lines)
        if not found:
            errors.append(f"{rel}: no arm on a `{app}Event`, so no relay write is read")
            continue
        for variants, body in found:
            arms += 1
            # Joined, as a call chain may put `.set(` on the next line.
            signals = [m.group(1) for m in SIGNAL_WRITE.finditer(" ".join(line for _, line in body))]
            for variant in variants:
                for signal in signals:
                    written.setdefault(variant, []).append((signal, f"{rel}:{body[0][0]}"))
    for path in sources(VIEW_STATE):
        rel = path.relative_to(ROOT).as_posix()
        for variants, body in event_arms(above_tests(path)):
            stores = [
                (number, m.group(1))
                for number, line in body
                for m in VIEW_STORE.finditer(line)
                if m.group(1) not in REPAINT_FLAGS
            ]
            if not stores:
                continue
            number, field = stores[0]
            for variant in variants:
                for signal, at in written.get(variant, []):
                    if f"{variant} -> {signal}" in ONE_EVENT_COPY_ACCEPTED:
                        continue
                    errors.append(
                        f"{rel}:{number}: `{variant}` sets `self.{field}` here, and {at} writes the `{signal}` signal off it"
                    )
    return arms


def bare_payload(ty: str) -> str | None:
    """The primitive, tuple or array a payload carries once its wrappers are
    peeled; `None` for a type of its own."""
    ty = ty.strip().lstrip("&").strip()
    generic = re.match(r"^([\w:]+)\s*<(.*)>$", ty)
    if generic:
        head = generic.group(1).split("::")[-1]
        if head in TYPE_WRAPPERS:
            return bare_payload(generic.group(2))
        return head if head in PRIMITIVES else None
    if ty.startswith(("(", "[")):
        return ty
    name = ty.split("::")[-1]
    return name if name in PRIMITIVES else None


def check_signal_types(errors: list[str]) -> int:
    """`UI-THREAD-060`: no shell signal carries a bare primitive.

    An identifier, a quantity with a unit or a closed set travels as a type of
    its own, so two payloads of one shape cannot be swapped. Free text on the
    accepted list stays a `String`.
    """
    held = 0
    for rel, holder in SIGNAL_HOLDERS.items():
        text = read(rel)
        lines = [(number, code(line)) for number, line in enumerate(text.splitlines(), 1)]
        for struct, number, field, ty in struct_fields(lines):
            found = SIGNAL_FIELD.match(ty)
            if struct != holder or not found:
                continue
            held += 1
            bare = bare_payload(found.group(1))
            if bare and f"{rel}:{holder}.{field}" not in BARE_PAYLOAD_ACCEPTED:
                errors.append(f"{rel}:{number}: the `{field}` signal carries a bare `{bare}`, not a type of its own")
    return held


def typed_fields(line: str) -> list[tuple[str, str]]:
    """Each `name: Type` pair a declaration line holds, inline variants
    included. A comma inside angle brackets stays in the type."""
    pairs = []
    for found in BUS_ID_FIELD.finditer(line):
        depth, end = 0, found.end()
        while end < len(line):
            ch = line[end]
            depth += {"<": 1, ">": -1}.get(ch, 0)
            if depth == 0 and ch in ",}":
                break
            end += 1
        pairs.append((found.group(1), line[found.end() : end]))
    return pairs


def check_bus_ids(errors: list[str]) -> int:
    """`BUS-029`: no message field of `ambre-bus` carries an identifier as a
    bare primitive.

    A field named `id`, `*_id` or `*_ids` takes a type of its own above the
    tests, so a sheet id passed where a name goes does not compile. A function
    parameter is not a message field, and its line is skipped.
    """
    files = sorted((ROOT / BUS).rglob("*.rs"))
    if not files:
        errors.append(f"{BUS}: gone from the tree, so the identifier boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        text = path.read_text().splitlines()
        for number, raw in enumerate(text, 1):
            if opens_test_module(text, number - 1):
                break
            line = code(raw)
            if re.search(r"\b(?:fn|let)\b", line):
                continue
            for field, ty in typed_fields(line):
                bare = bare_payload(ty)
                if bare:
                    errors.append(f"{rel}:{number}: the `{field}` field carries an identifier as a bare `{bare}`")
    return len(files)


def wire_fields(lines: list[tuple[int, str]]) -> list[tuple[int, str, str, bool, bool]]:
    """Each named field of each braced struct: the line, the field, its type,
    whether the struct derives a serde trait, and whether the field's own
    attributes name an identifier's text adapter. An attribute spread over
    lines is read whole."""
    fields: list[tuple[int, str, str, bool, bool]] = []
    serde, inside = False, False
    attrs: list[str] = []
    pending = ""
    for number, line in lines:
        stripped = line.strip()
        if pending or stripped.startswith("#["):
            pending += stripped
            if pending.count("[") <= pending.count("]"):
                attrs.append(pending)
                pending = ""
            continue
        if STRUCT_OPEN.match(line):
            serde = any(SERDE_DERIVE.search(a) for a in attrs)
            inside, attrs = True, []
            continue
        if not inside:
            attrs = []
            continue
        if stripped.startswith("}"):
            inside, attrs = False, []
            continue
        found = STRUCT_FIELD.match(line)
        if found:
            adapted = any(TEXT_ADAPTER.search(a) for a in attrs)
            fields.append((number, found.group(1), found.group(2).strip(), serde, adapted))
        attrs = []
    return fields


def check_wire_ids(errors: list[str]) -> int:
    """`SRV-179`: an identifier of the relay's rows and routes, or of the HTTP
    layer of Sync, takes its newtype and reaches JSON as its text.

    A field named `id`, `*_id` or `*_ids` holds no `String`, `str` or `Uuid`.
    A field of a serde struct that holds a core identifier names its adapter
    in `ambre_core::serde_id`, since the identifier alone writes its bytes.

    `SRV-180`: on the same files, a field named `doc_type`, `role`, `kind` or
    `status` holds its enum, never a `String` or a `str`.

    `SRV-182`: so does the `error` code of a refusal, on the relay's refusal
    and in the HTTP layer of Sync.
    """
    files = [p for d in WIRE_DIRS for p in sorted((ROOT / d).rglob("*.rs"))]
    files += [ROOT / f for f in WIRE_FILES if (ROOT / f).exists()]
    if not files:
        errors.append(f"{WIRE_DIRS[0]}: gone from the tree, so the wire identifier boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, field, ty, serde, adapted in wire_fields(above_tests(path)):
            untyped = WIRE_UNTYPED.search(ty)
            if WIRE_ID_NAME.match(field) and untyped:
                errors.append(f"{rel}:{number}: the `{field}` field carries an identifier as `{untyped.group(1)}`")
            elif serde and CORE_ID.search(ty) and not adapted:
                errors.append(f"{rel}:{number}: the `{field}` field writes its identifier as bytes, with no text adapter")
            elif WIRE_CLOSED_NAME.match(field) and untyped and untyped.group(1) != "Uuid":
                errors.append(f"{rel}:{number}: the `{field}` field carries a closed set as `{untyped.group(1)}`")
    return len(files)


def check_typed_answers(errors: list[str]) -> int:
    """`SRV-181`: a route, a refusal or a middleware of the relay answers with
    a type. A body built with `json!` escapes the closed-set and identifier
    checks above, so no line outside the tests builds one."""
    files = sources(ANSWER_SOURCES)
    if not files:
        errors.append(f"{ANSWER_SOURCES[0]}: gone from the tree, so the typed answer boundary reads nothing")
        return 0
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        for number, line in above_tests(path):
            if AD_HOC_BODY.search(line):
                errors.append(f"{rel}:{number}: an answer built with `json!`; give it a type")
    return len(files)


def check_bench_frame_rate(errors: list[str]) -> int:
    """`FIXTURE-024`: a benchmark that loads a fixture names no frame rate."""
    held = 0
    for path in sorted((ROOT / CRATES).glob("*/benches/*.rs")):
        rel = path.relative_to(ROOT).as_posix()
        lines = [(n, code(line)) for n, line in enumerate(path.read_text().splitlines(), 1)]
        if not any(BENCH_FIXTURE.search(line) for _, line in lines):
            continue
        held += 1
        for number, line in lines:
            found = FRAME_RATE.search(line)
            if found:
                errors.append(f"{rel}:{number}: a fixture benchmark reports a frame rate: `{found.group(0)}`")
    return held


def module_of(path: Path) -> list[str]:
    """The module path of a Rust file, given relative to its crate's `src`."""
    segments = list(path.parts)
    last = segments.pop()
    if last not in ("lib.rs", "main.rs", "mod.rs"):
        segments.append(last.removesuffix(".rs"))
    return ["crate", *segments]


def resolve(path: str, here: list[str]) -> str:
    """An absolute module path for a `crate::`, `super::` or `self::` path
    read in the module `here`, its last segment dropped as the item named."""
    segments = path.split("::")[:-1]
    out = list(here)
    if segments and segments[0] == "crate":
        out, segments = ["crate"], segments[1:]
    while segments and segments[0] in ("super", "self"):
        if segments.pop(0) == "super":
            out.pop()
    return "::".join(out + segments)


def use_paths(text: str) -> list[str]:
    """Each path a `use` names, its braces expanded one level at a time."""
    paths: list[str] = []
    for found in WALK_USE.finditer(text):
        pending = [re.sub(r"\s+", "", found.group(1))]
        while pending:
            path = pending.pop()
            if "{" not in path:
                paths.append(path.split(" as ")[0])
                continue
            head, body = path.split("{", 1)
            body = body[: body.rindex("}")]
            depth, start = 0, 0
            for i, ch in enumerate(body + ","):
                depth += {"{": 1, "}": -1}.get(ch, 0)
                if ch == "," and depth == 0:
                    item = body[start:i]
                    if item:
                        pending.append(head + item)
                    start = i + 1
    return paths


def check_walks(errors: list[str]) -> int:
    """`CANVAS-074`: a walk reaches the modules its crate lists and names no
    screen state above its tests."""
    held = 0
    for crate, reach in WALK_REACH.items():
        src = ROOT / CRATES / crate / "src"
        walks = [p for p in sorted(src.rglob("*.rs")) if any(WALK_IMPL.search(line) for _, line in above_tests(p))]
        if not walks:
            errors.append(f"{CRATES}/{crate}: no `DocumentWalk` implementation read, so the walk boundary holds nothing")
        for path in walks:
            held += 1
            rel = path.relative_to(ROOT).as_posix()
            here = module_of(path.relative_to(src))
            lines = above_tests(path)
            text = "\n".join(line for _, line in lines)
            reached = {resolve(p, here) for p in use_paths(text)}
            for number, line in lines:
                if line.lstrip().startswith(("use ", "pub use ")):
                    continue
                reached.update(resolve(p, here) for p in WALK_PATH.findall(line))
                found = SCREEN_STATE.search(line)
                if found:
                    errors.append(f"{rel}:{number}: the walk names screen state: `{found.group(0)}`")
            for module in sorted(reached - set(reach) - {"::".join(here)}):
                errors.append(f"{rel}: the walk reaches `{module}`, which `WALK_REACH` does not list")
    return held


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="exit non-zero on a breach")
    args = parser.parse_args()

    errors: list[str] = []
    manifests = check_client(errors)
    generators = check_generators(errors)
    generator_reads = check_generator_reads(errors)
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
    primitives = check_primitives(errors)
    scrolls = check_scroll_sources(errors)
    composed = check_composed_names(errors)
    copies = check_one_copy(errors)
    event_copies = check_one_event_copy(errors)
    signal_types = check_signal_types(errors)
    bus_ids = check_bus_ids(errors)
    wire_ids = check_wire_ids(errors)
    answers = check_typed_answers(errors)
    benches = check_bench_frame_rate(errors)
    instances = check_instances(errors)
    reach = check_thread_reach(errors)
    worker_messages = check_worker_messages(errors)
    engine = check_engine_and_relay(errors)
    handlers = check_handlers(errors)
    latches = check_failure_latches(errors)
    exits = check_exit_writes(errors)
    walks = check_walks(errors)
    print(
        f"{manifests} manifests read for the fixture feature, "
        f"{generators} generator functions read for CRDT calls, "
        f"{generator_reads} generator modules read for a file read, "
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
        f"{ui_thread} UI-thread files read for the document, a wait, a drawing or a hit-test, "
        f"{stores} client files read for a persistence path, "
        f"{subset} sources read for a compute pass or a storage buffer, "
        f"{widths} files read for a width spelled twice, "
        f"{controls} files read for a modal, a drag or a named creation, "
        f"{keys} manifests and files read for a key hash or an invitation, "
        f"{access} UI files read for a silent state, a panel modal, a mute menu or a hidden pipe, "
        f"{relay_logs} relay files read for a payload in a log line, "
        f"{kinetic} Rust files read for a second kinetic model, "
        f"{primitives} drawing primitives read for a screen-only parameter, "
        f"{scrolls} application files read for a scroll with no model step, "
        f"{composed} Rust files read for a name composed outside its type, "
        f"{copies} view files read for a second copy of a signal's state, "
        f"{event_copies} relay arms read for an event a view also keeps, "
        f"{signal_types} shell signals read for a bare primitive payload, "
        f"{bus_ids} bus files read for an identifier field of a primitive type, "
        f"{wire_ids} relay and HTTP files read for an identifier untyped or written as bytes or a closed set held as text, "
        f"{answers} relay files read for an answer built with `json!`, "
        f"{benches} fixture benchmarks read for a frame rate, "
        f"{instances} files read for a third instance or a shared worker, "
        f"{reach} manifests read for a thread reaching the rendering, "
        f"{worker_messages} worker and relay files read for a wire copy, a worker id or an app message, "
        f"{engine} launcher files read for a second engine or a relay out of the frame, "
        f"{handlers} engine files read for a handler that sends on a queue, "
        f"{latches} launcher files read for a failure that waits on a queue, "
        f"{exits} build and thread files read for a write on the way out, "
        f"{walks} walks read for a screen module or screen state"
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
            "nothing, draws and hit-tests on the surface alone, browser storage is reached "
            "from the named paths, no shader leaves the shared subset, the "
            "chrome's widths live in one stylesheet, one file renders the modal, "
            "a drag control debounces, a document is created unnamed, the worker "
            "alone hashes a key, an invitation waits on the trust decision, a "
            "two-state attribute says false, only the shared dialog is a "
            "modal, a menu answers the keyboard, the pipe stays reachable, the "
            "relay logs no payload, one file holds the kinetic model, a drawing "
            "primitive takes geometry and paint alone, one file "
            "composes a document's name, no view "
            "holds the state a shell signal holds, no event feeds both a view "
            "field and the relay's signal, no bus field carries an identifier as "
            "a bare primitive, an identifier on the JSON wire takes its newtype "
            "and its text adapter and a closed set its enum, a fixture "
            "benchmark reports no frame rate, the page "
            "loads two instances and shares no worker, neither the Logic runtime "
            "nor the worker reaches the rendering, and the worker redeclares no "
            "wire type, allocates no request id and names no app message. The launcher "
            "starts one engine, and every frame relays before it draws. A command "
            "handler returns an outcome and names no queue. A panic and a failed "
            "worker each latch the fatal screen without a queue, and a failing thread "
            "writes nothing on its way out. A walk reaches no screen state.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
