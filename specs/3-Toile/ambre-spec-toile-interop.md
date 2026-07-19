# AMBRE — SPEC: TOILE INTEROP (Import / Export & Media)
**Type:** Vertical (Feature / Application — Interop)
**Status:** Draft

> Toile import/export and asset lifecycle: importing SVG, PNG/JPG, Figma (JSON) and Miro diagrams into the
> board CRDT; exporting multi-page PDF (via `krilla`/`KrillaSurface`), high-resolution SVG/PNG, and a
> standalone Web-Showcase (WASM); and the media pipeline (MinIO-backed blobs, placeholder-then-load, local
> cache) under the Zero-Knowledge constraint.
>
> **Scope of this spec:** format converters and the media transfer/caching path. It produces/consumes the
> element model of `toile-logic §2`, reuses the render pipeline of `toile-ui` and the `DrawSurface`
> (`WgpuSurface` for on-screen, `KrillaSurface` for PDF), and treats presentation order (`toile-features §1`)
> as the page/slide sequence for PDF and Web-Showcase export. It does not redefine the engine or renderer.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- **Import:** SVG, PNG/JPG, Figma (JSON), Miro → a batch of `ToileCommand::CreateElement` (frames included, via
  `kind: Frame` — a frame is an element, `toile-logic §2.F`) applied atomically to `ToileState`.
- **Export:** multi-page PDF (one page per presentation frame, or the whole board), high-res SVG/PNG raster,
  standalone Web-Showcase WASM bundle.
- **Media:** encrypt-then-upload blobs to MinIO; download with a low-res placeholder first; local cache for
  instant zoom/pan.

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-bus` | `ToileCommand`/`ToileEvent`, media messages | Import lands as commands; media status as events |
| `ambre-draw` | `DrawSurface`, `KrillaSurface`, re-exports `kurbo`/`peniko` | PDF and vector export via the shared surface — architectural convention |
| `krilla` | PDF generation | Pure-Rust PDF writer behind `KrillaSurface` (already used by Trame/Prose export) |
| `usvg` / `resvg` | SVG parse → tree | Pure-Rust SVG import; `usvg` normalizes to a render tree we map to elements. Built with `default-features = false` (no font stack): the browser has no system fonts and Toile keeps text editable (§2.A) |
| `roxmltree` | SVG `<text>` extraction | Reads editable `<text>` bodies/positions from the SVG source (usvg would flatten them to glyph paths, §2.A) |
| `image` | PNG/JPG decode/encode | Pure-Rust raster codecs for image import + PNG export |
| `serde_json` | Figma / Miro document parse | Their exchange formats are JSON |
| `lyon` | Raster rasterization of vector export (PNG) | Via `ambre-canvas` tessellation for high-res raster |
| `postcard` | Web-Showcase embedded board snapshot | Compact self-contained payload |

> **Encryption/compression are delegated.** AEAD (XChaCha20-Poly1305) and zstd live in the Sync/crypto layer;
> this module hands **plaintext** blobs to that layer and never implements crypto itself (Zero-Knowledge pillar).

**Invariants:**
- Import is **atomic**: a whole file becomes one undoable batch; a parse failure imports nothing and surfaces
  `ToileEvent::ImportFailed` with a reason.
- Import never trusts remote geometry blindly: coordinates are clamped and element counts bounded (§2.D) to
  avoid a hostile file exhausting memory or the R-tree.
- Export is deterministic: the same board + selection yields byte-stable SVG/PDF structure (modulo timestamps).
- No plaintext media or document content reaches the server except as ciphertext blobs on MinIO (§3, §5).
- No `unsafe` in our code.

---

## 2. IMPORT

### A. SVG

`usvg` parses to a normalized tree; we walk it and map nodes to Toile elements:

| SVG node | Toile element |
|---|---|
| `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polygon>` | `shape` (kind + `ElementStyle` fill/stroke) |
| `<path>` | `shape:path` carrying the `kurbo::BezPath` |
| `<text>` | `text` element (Parley runs) |
| `<image>` | `image` element → blob into the media pipeline (§5) |
| `<g>` | `group` (logical grouping, `toile-logic §2.F bis`) with the children as its `children` list — not a `frame` (frames carry chrome/notes an SVG group does not) |

> **usvg normalizes primitives to paths.** `usvg` flattens every primitive
> (`<rect>`, `<circle>`, `<line>`, ...) into a `Path`, so a vector node arrives as
> `shape:path`; an axis-aligned rectangle contour is recognized back into
> `shape:rect` (the common, safe case), everything else keeps its exact
> `BezPath`.
>
> **Text is read from source, not from usvg.** usvg only exposes `<text>` by
> flattening it to glyph *paths*, which needs a font database — impossible in the
> browser (no system fonts) and destructive (it drops the editable string). So the
> usvg font stack is **disabled** (`default-features = false`, which also trims the
> WASM binary, §8) and `<text>` is read directly from the SVG source with
> `roxmltree`: the raw body plus its `x`/`y`/`font-size` become an editable `text`
> element rendered through Toile's own Parley stack. Phase 0 reads top-level
> `x`/`y` only (no transform / `tspan` offset resolution).
>
> **Groups: rebuilt after apply (Phase 1).** A Toile `group` is born only via
> `GroupElements { ids }` (`toile-logic §2.F bis`), whose `ElementId`s are minted
> by the engine *after* `CreateElement` is applied — ids the offline converter
> cannot know. The converter therefore speaks in **slots**: it emits the children
> as elements (group transform baked into their coordinates via `abs_transform`, so
> geometry stays world-space) plus an `ImportGroup { members }` list whose members
> reference slots or earlier groups. Groups are emitted **innermost-first**
> (post-order) so a nested member always refers to an already-resolved entry. The
> apply pass mints the ids, then issues `GroupElements` inside the same undo group,
> reconstructing the `<g>` hierarchy exactly. No group is flattened and none is
> listed in the import report as lost.

Gradients/filters not representable in `ElementStyle` are rasterized to an `image` fallback (lossy but faithful),
flagged in the import report.

### B. Raster (PNG / JPG)

Decoded by `image`; placed as a single `image` element; the pixels go through the media pipeline (§5). Large
images are downscaled to a screen-reasonable placeholder for immediate display, full-res fetched lazily.

### C. Figma / Miro (JSON)

A dedicated converter maps the foreign document model to ours:

- **Figma:** frames→frames, rectangles/ellipses/vectors→shapes (vector networks flattened to `BezPath`), text→
  text, components→plain elements (MVP: no component instancing).
- **Miro:** sticky notes→sticky, shapes→shapes, connectors/lines→connectors (`toile-logic §5`), frames→frames.

> **Auto-layout deferred (MVP).** Figma auto-layout would map to `AutoLayout`
> (`toile-features §2`), but `SetAutoLayout` targets an `ElementId` minted only
> after apply — the same post-apply problem as groups and connectors, and it would
> add another slot-resolved field to `ImportBatch`. For the MVP an auto-layout
> frame imports as a plain frame with its children at their resolved absolute
> positions, and the drop is listed in the import report. It lands with a later
> slice.

Unmapped constructs are dropped and listed in the import report (`ToileEvent::ImportReport { imported, skipped }`).

### D. Safety limits

A hostile or huge file must not exhaust the client. Import enforces: max element count, max nesting depth, world
coordinate clamp, and max decoded image dimensions; exceeding any bound aborts with `ImportFailed` (nothing is
applied — the atomicity invariant).

### E. Application to the CRDT

The converter yields a `Vec<ToileCommand>` the dispatcher applies in one batch inside a single undo group, then
the engine derives one `DisplayViewport`. Each element is born fully formed via `CreateElement`'s `content:
InitContent` seed (`toile-logic §8`): a `<path>` carries `InitContent::Path(BezPath)`, an `<image>` carries
`InitContent::Image(hash)`, a `<text>` carries `InitContent::Text(..)` — no create-then-patch round-trip. Concurrency during an import is ordinary CRDT merge — imported elements
carry freshly minted `ElementId`s, so they never collide with a collaborator's edits.

> **Responsiveness on a large import (yield + progress).** "One undo group" is not "one blocking loop": a big file
> (a 5 000-node SVG) is **parsed off the render path**, then its `CreateElement`s are applied in **bounded batches
> that yield** between chunks so pointer/`SetViewport` commands keep flowing and the Logic thread never freezes —
> the `trame-interop §7` batched-yield pattern (there, 500 cells per chunk). Atomicity is preserved: the whole
> file still lands under a single `UndoManager` item (one `doc.commit()` at the end, `toile-logic §8bis`), and a
> parse failure detected *before* any command is applied still imports nothing (`ImportFailed`, §1). For imports
> above a duration threshold the dispatcher emits `ToileEvent::ImportProgress { percent }` so the UI can show a
> progress affordance; short imports skip straight to `ImportReport`.

### F. Templates (DEFERRED — post-MVP)

Templates are Miro's adoption backbone: a new board (or a region of one) seeded from a preset — mind map,
retrospective, kanban, flowchart, brainstorm. Architecturally a template is **just a saved `ImportBatch`**: a
self-contained, id-free bundle of elements/connectors/frames (the same `ClipboardPayload`-shaped structure as
copy/paste, `toile-logic §8 ter`) that instantiates through the **exact same path as an import** — a
`Vec<ToileCommand>` applied in one undo group with freshly minted ids (§2.E). No new engine concept is required:
a template is authored by copying a selection, and applied by pasting/importing it at a drop point.

MVP ships **no template gallery** (it is content + curation work, not architecture). When built, the store is a
bundle of these payloads (shipped as static assets, or user-saved to their own space); nothing about it touches
the Zero-Knowledge model, since a template carries only element data and is applied client-side like any paste.
See the roadmap.

---

## 3. EXPORT

### A. Multi-page PDF (`krilla` / `KrillaSurface`)

Each presentation frame (`toile-features §1`, in `presentation.order`) becomes one PDF page, rendered through the
**same** `DrawSurface` code as the screen but targeting `KrillaSurface` — so vector fidelity is preserved (text
stays text, paths stay paths). Whole-board export (no presentation) emits a single page sized to the content
bounds. Images are embedded from the media cache at full resolution.

> **Page = frame spatial rect, clipped.** A page shows exactly what the audience
> sees in present mode: the walk is culled to the frame's **spatial** rect (world
> AABB), not to the frame's declared child list, and the surface pushes a clip on
> that rect for the whole page. The clip is load-bearing, not cosmetic. The present
> camera fits a frame by the tighter of the two axes (`fit_target` takes the `min`
> of the x/y fit), which leaves slack on the other axis; without the clip that
> slack would pull neighbouring board content onto the page. One page per frame,
> one content walk, byte-stable output (§1 determinism).

> **Thread & loading.** `KrillaSurface` runs in the **Logic thread**, not the UI thread, and `krilla` is
> **lazy-loaded** (WASM bundle splitting), consistent with `ambre-spec-canvas.md §D`. Only the offscreen **PNG**
> render-back (§3.C) uses a UI-thread `WgpuSurface`. PDF export therefore drives the content walk from the Logic
> side against `KrillaSurface`, reusing the same abstract `DrawSurface` calls as `paint_content`.

### B. High-resolution SVG

The board (or selection) is serialized to SVG by mapping each element back to its SVG counterpart (the inverse of
§2.A): shapes→primitives, `shape:path`→`<path>`, text→`<text>`, images→embedded/`<image>`, connectors→routed
`<path>`. Vector-clean, resolution-independent.

### C. High-resolution PNG

The `toile-ui` tessellation path renders the target region to an offscreen `WgpuSurface` at a caller-chosen scale
factor, read back and encoded by `image`. Used when a raster deliverable is required.

### D. Web-Showcase (standalone WASM)

A self-contained bundle: a postcard snapshot of the board (or the presentation deck) embedded in a minimal WASM
viewer that reuses the `toile-ui` renderer read-only (pan/zoom/present, no editing, no Sync). Media is inlined as
data or bundled. The output is a static, shareable page with no server dependency.

> **Zero-Knowledge note.** Web-Showcase export is a **deliberate declassification** by the user: it produces a
> plaintext, shareable artifact outside the E2EE boundary. It must be an explicit user action with a clear
> confirmation, never automatic.

---

## 4. PRESENTATION-AWARE EXPORT

PDF and Web-Showcase honor `presentation.order` when a deck exists: page/slide N = frame N. Speaker notes
(`toile-features §1.D`) can optionally be exported as PDF page annotations or a Web-Showcase presenter panel.
Smart Animate transitions are **not** exported to PDF (static medium); Web-Showcase may replay them since it runs
the live renderer.

---

## 5. MEDIA PIPELINE (MinIO)

> **Reuses the existing service plane — no bespoke channel.** Media is already modelled on the `ambre-bus`
> service plane (`ambre-spec-architecture.md §2.D`): `RestCommand::UploadMedia { document_id, data, mime_type }`
> / `DownloadMedia { document_id, content_hash }`, answered by `RestEvent::MediaUploaded { content_hash }` /
> `MediaDownloaded { content_hash, data }`, implemented Sync-side (SYNC P3). Toile does **not** invent its own
> media events: an upload/download is a UI-allocated `RequestId` routed UI -> Logic -> Sync -> S3 and back.
> Encryption, zstd and the **content hashing** all happen in the **Sync thread** (sole crypto consumer); Toile
> never hashes or encrypts on the Logic side.

### A. Content-addressed, encrypted blobs

On image import, the plaintext bytes travel up as `RestCommand::UploadMedia`; the **Sync thread** compresses
(zstd), encrypts (XChaCha20-Poly1305), computes the content hash (BLAKE2b), uploads the ciphertext to MinIO under
an opaque object id, and returns `RestEvent::MediaUploaded { content_hash }`. That `content_hash` becomes the
`media` map key and the `image` element's payload (`toile-logic §2.B/§2.C`). **The server stores only
ciphertext**; it never sees pixels (Zero-Knowledge pillar). Content addressing dedups identical images across the
board automatically.

### B. Placeholder-then-load

On first display of an `image` element whose blob is not cached, the UI paints a low-resolution placeholder (a
tiny embedded thumbnail stored inline in the `media` entry, or a solid box) and issues a
`RestCommand::DownloadMedia { document_id, content_hash }`. The Sync thread fetches + decrypts + decompresses off
the render thread and replies `RestEvent::MediaDownloaded { content_hash, data }`; the Logic thread decodes,
hands the pixels to the `toile-ui` texture path, and the element repaints. Loading/failure surface through the
ordinary `RequestId` correlation and `ApiError` (no dedicated `ToileEvent` variant).

### C. Local cache

Decoded (or ciphertext) blobs are cached locally (keyed by hash) so zoom/pan and revisits are instant and cause
no refetch. The cache is bounded (LRU) and never holds plaintext beyond the session per the confidentiality
posture.

---

## 6. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

- **Boundary audit:** the only outbound flow is **ciphertext media blobs** to MinIO (§5.A) via the Sync/crypto
  layer, and the routing metadata (object id, document id) the server already sees for any document. No board
  geometry, text, or pixels leave in plaintext.
- **Deliberate declassification:** Web-Showcase export (§3.D) is the single path that produces plaintext outside
  the boundary; it is explicit, user-confirmed, and clearly labeled.
- **Cryptography:** delegated to the shared crypto layer (XChaCha20-Poly1305 AEAD, zstd before encryption); this
  module never rolls its own primitives and holds no keys.
- **Import trust:** foreign files are untrusted input — bounded and clamped (§2.D) before touching the CRDT.

---

## 7. QUALITY & TESTABILITY
> See the transverse test architecture in `ambre-spec-architecture.md §4`.

- **Test tiers:** **host** for every converter (SVG/Figma/Miro→commands, and export serializers), the safety
  limits, and the media hashing/placeholder state machine — all pure and host-testable. **wasm** for the PDF
  export path (proving `krilla` + Parley + swash link and run under `wasm32-unknown-unknown`), the browser
  download path, and off-UI-thread font loading. No **db** tier here; MinIO/upload paths are exercised
  against the crypto/Sync seam with a mock object store (server-side MinIO integration belongs to the Sync spec).
  The §3.C PNG offscreen render-back needs a real GPU adapter in headless Chrome and lands with §3.C, not with
  this phase (a flaky GPU test is worse than none).
- **Unit tests:** round-trip SVG import→export→import stability for each primitive; Figma/Miro fixture files →
  expected command batch (golden); every safety-limit branch aborts atomically with `ImportFailed`; PDF page
  count == presentation frame count; media state machine (Loading→Ready, Loading→Failed, cache hit skips fetch);
  content-addressing dedup (two identical images → one blob). Target: every mapped node type and every reject path.
- **CRDT convergence:** N/A directly — import produces ordinary `CreateElement` batches whose convergence is
  covered by `toile-logic §10`. A regression test asserts an import applied concurrently with a remote edit
  merges without id collision (fresh `ElementId`s).
- **Mocks & Environment:** `CommandSink` to capture import batches; a mock object store for media; golden fixture
  files for foreign formats; headless wasm for render-back. No live network, no live MinIO.
- **Benchmarks:** `criterion` on importing a 5 000-node SVG (target below) and PDF export of a 50-slide deck.
- **Definition of Done (tested):** converter unit + golden fixtures + media state-machine tests + one wasm
  export test green in CI.

---

## 8. PERFORMANCE & RESOURCES

| Operation | Budget | Rationale |
|---|---|---|
| SVG import (5 000 nodes) | < 500 ms, off the render thread | One-shot user action; must not freeze the UI. |
| PDF export (50 slides) | < 3 s | Acceptable for an explicit export; runs off the render frame. |
| Placeholder paint on first image display | < 15 ms | The display-latency budget — never block on the network. |
| Media cache hit (zoom/pan revisit) | 0 refetch | Local cache (§5.C). |

**WASM impact:** `usvg`/`resvg`, `image`, and `krilla` are the notable binary contributors; they are loaded on
the interop path and shared with Prose/Trame export where possible to stay within the total < 2 MB gzipped
budget.

> **Lazy-loading not yet delivered.** Phase 1 gates `krilla` behind a `pdf` cargo
> feature, which controls **compilation**, not runtime loading: the code still
> links into the main wasm module. True lazy-loading (a second wasm-bindgen entry
> point plus a dynamic `import()`) is a build-system slice deferred to a later
> phase; until then the interop path does not claim the §8 bundle-splitting budget
> for `krilla`.

---

## 9. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Reason |
|---|---|
| **Server-side rendering / conversion** | Violates the Blind Relay + Zero-Knowledge pillars; all import/export is client-side WASM. |
| **Storing media plaintext on the server** | Only ciphertext blobs on MinIO; server never sees pixels (§5.A). |
| **Automatic/implicit Web-Showcase publish** | Declassifies E2EE content; must be an explicit, confirmed user action (§3.D). |
| **Non-atomic import (stream elements as they parse)** | A mid-file failure would leave a corrupt partial board; import is one atomic undoable batch (§1). |
| **Trusting foreign-file coordinates/counts** | Memory-exhaustion vector; imports are bounded and clamped (§2.D). |
| **C/OpenSSL-backed codecs (e.g. libpng/native PDF)** | Rust-first pillar — `image`, `usvg`, `krilla` are pure Rust. |

---

## 10. INTEGRATION PLAN (ROADMAP)

> Tracked as Stories of Epic **E4** (Toile MVP) in `specs/ambre-roadmap.md`.

**Phase 0 - Core standalone** — landed as the pure, host-tested `ambre-toile::logic::interop` module (converters return an `ImportBatch { commands, media, report }`; no dispatcher/Sync wiring yet, that is Phase 1).
- [x] SVG import / export (primitives + paths + text) — `svg_import` (usvg + `roxmltree` text pass, §2.A) / `svg_export` (deterministic, byte-stable)
- [x] Raster image import — `raster` (PNG/JPG decode + placeholder thumbnail, §2.B)
- [x] Safety limits (size / element caps, decompression bounds) — `limits` (element/depth caps, coordinate clamp, image-dimension bound; every breach aborts atomically, §2.D)
- [x] Media hashing + placeholder state machine — `media` (`Loading→Ready/Failed` + LRU cache + content-addressed dedup, keyed by an injected `ContentHash`; real blake2b stays Sync-side per §5)
- [x] Host tests + golden fixtures — node-type mapping, round-trip stability, every reject path, media state machine

**Phase 1 - Integration**
- [x] PDF export via `KrillaSurface` honoring presentation order — one page per presentation frame, culled and
  clipped to the frame's spatial rect (§3.A); `draw_image` added to `DrawSurface` (`ambre-spec-canvas.md §D`) so
  images embed; fonts eager-fetched at Logic-worker boot so text stays vector. `krilla` behind a `pdf` feature;
  runtime lazy-loading deferred (§8).
- [x] Figma / Miro converters (MVP: no component instancing — components map to plain elements, §2). Figma
  auto-layout deferred and reported (§2.C).
- [x] Media pipeline wired to the crypto / Sync layer + mock object store — import routes plaintext through the
  existing `UploadMedia`/`DownloadMedia` service plane (§5); the atomic import applies only once every content
  hash has returned. Creates the `media` CRDT map (`toile-logic §2.B`).
- [x] wasm export tests — PDF export, browser download, and off-thread font loading under
  `wasm32-unknown-unknown`.

**Phase 2 - Optimization**
- [ ] Streaming / bounded import for very large files (batched-yield apply keeping the Logic thread responsive + `ImportProgress`, §2.E)
- [ ] PNG render-back tiling
- [ ] Web-Showcase bundle-size trimming
- [ ] Media LRU tuning

**Deferred - post-MVP**
- [ ] Template gallery (§2.F): built-in preset boards (mind map / retro / kanban / flowchart) applied through
      the existing import path as an id-free `ImportBatch`; content + curation work, no new engine concept
