# AMBRE — SPEC: TOILE LOGIC (Infinite Canvas Engine)
**Type:** Vertical (Feature / Application — Logic)
**Status:** Draft

> Toile collaborative infinite-canvas engine: CRDT state management (`LoroDoc`) for freely positioned
> elements, connectors, frames and freehand strokes; a spatial index (R-tree) for O(log N) viewport
> culling; client-side connector routing and stroke smoothing; synchronous derivation of a
> `DisplayViewport` inside `handle_command` / `apply_remote`; and production of data-oriented structures
> for rendering.
>
> **Scope of this spec:** The engine (`ToileState`) that runs in the **Logic thread**. Per the two-instance
> model (`ambre-spec-architecture.md §1/§2.B`), the Logic thread is a `wasm_thread` Web Worker of the **Main**
> WASM instance, sharing one linear memory with the UI thread over `flume`; the **Sync** worker is a *separate*
> WASM instance reached over `postMessage`, not a third `wasm_thread`. Toile is a complete Logic + UI application —
> rendering (QuadRenderer / vector tessellation / DrawSurface), viewport interactions, and the UI thread
> are covered in `ambre-spec-toile-ui.md`. Presentation mode, auto-layout, boolean path ops, and
> collaboration overlays (votes, reactions, highlighters) are covered in `ambre-spec-toile-features.md`.
> SVG / Figma / Miro / PDF / image interoperability and MinIO media handling are covered in
> `ambre-spec-toile-interop.md`.
>
> **State registry.** Toile state is a concrete value, not a trait object. The Logic thread holds a typed
> registry `HashMap<DocumentId, DocState>` where `enum DocState { Trame(TrameState), Toile(ToileState), … }`,
> and dispatch is a `match` on `(DocState, AppCommand)` — no trait object, no `Box<dyn Any>`, no
> `downcast`. `ToileState::handle_command` returns a typed [`ToileOutcome`]; the `ambre-logic` dispatcher
> reads it and forwards display events to the UI and CRDT/ephemeral payloads to Sync over bare `flume`
> channels. See `ambre-spec-architecture.md §6`.
>
> **Plug-in point.** `ToileState` slots into the existing registry exactly like `TrameState`
> (`ambre-logic/src/lib.rs`): add the `Toile(ToileState)` variant to `enum DocState`, and replace the
> `DocType::Toile => None` branch in `DocState::new` with `Some(DocState::Toile(ToileState::new()))`. No
> other wiring changes — the outbound channels, the `flume::Selector` loop and the persistence path are
> document-type agnostic.
>
> **Embedded routing = no routing at all.** The `docs` registry is **flat**: it is keyed by `DocumentId`, and
> a `doc_embed` child (§2.I) is an ordinary entry in it, opened by the ordinary `OpenDocument` path. A command
> for the child is addressed straight to the child -- `LogicCommand::App { document_id: child, .. }` -- because
> the child's own view holds the child's id and emits on the same channel. Nesting therefore adds **no**
> dispatch step, no recursion and no depth bound: the parent Toile's `LoroDoc` is untouched simply because no
> command for the child ever reaches it. A command for a document absent from the registry is dropped with a
> warning, never synthesised.
>
> `LoroDoc` is instantiated in the Logic thread and never leaves it.

---

## 0. PRODUCT IDENTITY & POSITIONING

**Toile is a collaborative infinite whiteboard with a presentation mode — a Miro, not a Figma.**

Its job is *visual thinking, alignment, and delivery for teams*: brainstorming, workshops, diagramming,
sticky-note synthesis, then turning that board into a deck. The canvas is a **shared workspace**, optimized for
real-time co-presence and fluidity, not a **vector-fabrication tool** optimized for pixel-perfect production.

This positioning is deliberate and follows from the suite's identity: Ambre targets privacy-conscious European
teams and organizations (the Miro audience), not professional UI/UX designers (the Figma audience — who choose
Figma for its ecosystem, dev-handoff and design tokens, none of which are Ambre's value). Real-time CRDT
co-presence is our pillar and is the heart of the whiteboard job; competing with Figma on vector-fabrication
depth is out of scope and out of audience.

**In scope (the core job, held to MVP quality):**
freehand drawing, shapes, smart connectors, sticky notes, frames, text; real-time collaboration overlays
(live cursors, in-flight strokes, laser, votes, reactions); **presentation mode** (frames as slides, Smart
Animate) — the differentiator versus a plain whiteboard and Ambre's answer to slide decks; import of
PDF / image / SVG.

**Experimental (implemented, but *not* in the MVP definition of done):**
the constraint-based **auto-layout solver** (`toile-features §2`, `logic/autolayout.rs`) and **vector boolean
operations** (`toile-features`, `logic/boolean.rs`). These belong to the Figma fabrication metier, not the
whiteboard job. They exist in the tree and stay there, but receive **no MVP stabilization budget** and carry no
UI/quality promise; they are candidates for V2 or removal (see `ambre-roadmap.md`). Figma / Miro import is a
**one-shot migration** convenience, not a central feature.

> **Rule of thumb for scope decisions:** if a capability serves *a team thinking together or presenting*, it is
> core. If it serves *a designer fabricating a precise artifact*, it is experimental or out of scope.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- Owns the single-board `LoroDoc` (elements, connectors, frames, layers, drawings, media anchors).
- Maintains the spatial index (`rstar` R-tree) incrementally on every local and remote mutation.
- Resolves the visible set for a `Viewport` in O(log N) and derives a `DisplayViewport` for the UI thread.
- Computes connector geometry (anchors, routing) client-side — **never** a CRDT mutation on element move.
- Smooths freehand strokes (Catmull-Rom) at derivation time; stores only raw sampled points in the CRDT.
- Exports CRDT deltas to the Sync thread via `SyncCommand::SendUpdate`.

**What it delegates (out of scope here):**
- Rasterization / tessellation / GPU submission → `toile-ui`.
- Presentation sequencing, Smart Animate interpolation, auto-layout solving, boolean path ops → `toile-features`.
- Any import/export or MinIO fetch → `toile-interop`.

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-core` | `Uuid`, `DocType`, `PeerID` | Shared foundation |
| `ambre-bus` | `AppCommand`/`AppEvent` (thin sums) + the per-app content plane `app/toile.rs` (`ToileCommand`/`ToileEvent`/`Display*`/model), `SyncCommand`, `LogicEvent`, `CommandSink` (test seam) | Typed inter-thread data plane + IPC types |
| `ambre-draw` | `DrawSurface` trait, re-exports `kurbo`/`peniko`/`parley` | Shared rendering screen (WgpuSurface) / PDF (KrillaSurface) — required by architectural convention |
| `loro` | Single-board CRDT | `LoroMap`, `LoroMovableList` (layer z-order), `LoroText` (text runs) |
| `rstar` | Element R-tree | O(log N) viewport culling and hit spatial pre-filter |
| `kurbo` | 2D geometry (`BezPath`, `Point`, `Rect`, `Affine`) | Connector routing, stroke geometry, bounds math (re-exported by `ambre-draw`) |
| `flume` | Logic<->Sync channels (prod) + native test bridges | Dispatcher forwards `SyncCommand`s; tests use the `CommandSink` seam (`MockSyncHandle`) |
| `postcard` | Serialization of `ToileCommand` / `ToileEvent` / `DisplayViewport` | Compact, consistent with inter-thread IPC protocol |

> **Bus — ephemeral channel (already present).** Presence (cursors, live strokes, laser, selection) is relayed
> but never historised. It uses the existing `SyncCommand::SendEphemeral { document_id, payload: Vec<u8> }`
> (encrypted by the Sync thread, relay-only, not appended to CRDT history), distinct from
> `SyncCommand::SendUpdate` (CRDT delta). The inbound path is `LogicEvent::RemoteEphemeral { document_id,
> payload }`. Toile postcard-encodes its `PresencePayload` into `payload`; no bus change is required.

> **`ruzstd` absent** — compression delegated to the Sync thread.
> **No `petgraph`** — Toile has **no dependency graph**. Elements are independent (unlike Trame cells); there is
> no recompute cascade. Connectors reference element ids but are pure client-side render geometry.
> **No `rayon` (MVP)** — culling is O(log N + k) with k = visible set; tessellation parallelism, if ever
> needed, belongs to `toile-ui`, not the Logic thread.

**Invariants:**
- Derivation is **synchronous** in each `handle_command` / `apply_remote`: the state returns all events
  (including `ViewportData`) in `ToileOutcome.display` / the returned `Vec<ToileEvent>`.
- `LoroDoc` confined to the Logic thread: only exits via `export_updates()` → `Vec<u8>`.
- The R-tree is a **derived cache**, never authoritative: it is rebuilt from `LoroDoc` on load and updated
  incrementally; a full rebuild always yields the same tree as the incremental path (tested).
- Connector geometry and stroke smoothing are **never** persisted in Loro — they are recomputed on read.
- Element identity is a minted stable `ElementId` (`{peer:016x}_{counter:08x}`); positional keys are never used.
- No `unsafe`.

---

## 2. CRDT MODEL

### A. Element identity — minted, move-invariant `ElementId`

Every element, connector and frame carries an owned stable id `"{peer:016x}_{counter:08x}"`, where `peer`
is the document `PeerID` and `counter` a per-session monotonic counter (the §2.A principle shared with
Trame rows and sheets). Two peers that concurrently create an element mint **distinct** ids — both survive,
neither overwrites the other. An id, once minted, lives in the CRDT and never changes; moving, resizing or
restyling an element only mutates fields **inside** its `LoroMap`, so every reference (a connector endpoint,
a frame child list, a layer entry) keeps resolving.

> **Why not a positional or slot key.** Z-order must be reorderable and elements must be freely moved; a
> position-derived key would be invalidated by every reorder. Owning the id as a value (never a Loro op id,
> which `LoroMovableList::mov` reassigns — see Trame `logic §2.A`) makes it reorder-invariant.

### B. CRDT Schema

```
LoroDoc
├── "board" (LoroMap)                 → {name, bg_color: u32, grid_size: f64, grid_snap: bool}
│
├── "elements" (LoroMap)              → all board nodes, key = ElementId (frames included, see §2.F)
│   └── "{elem_id}" (LoroMap)
│       ├── "type"    : String        → "sticky" | "shape" | "text" | "image" | "frame" | "group"
│       │                               | "drawing" | "embed" | "doc_embed"
│       │                               (see §2.C; group = §2.F bis; doc_embed = nested Ambre doc, §2.I)
│       ├── "x","y"   : f64           → world position (top-left of AABB, pre-rotation)
│       ├── "w","h"   : f64           → world size
│       ├── "rot"     : f64           → radians, about the AABB center
│       ├── "locked"  : bool          → top-level field, NOT part of ElementStyle
│       ├── "link"    : Binary?       → optional postcard(LinkTarget) — hyperlink / go-to-frame (§2.K).
│       │                               Absent = no link. Top-level, NOT part of ElementStyle (a link is
│       │                               navigation behavior, not a visual attribute — same reasoning as `locked`).
│       ├── "style"   : Binary        → postcard(ElementStyle) — one LWW register per element (see §2.E)
│       └── "payload" : (per type)    → see §2.C
│
├── "connectors" (LoroMap)           → key = ConnectorId
│   └── "{conn_id}" (LoroMap)        → {from_elem, from_anchor, to_elem, to_anchor, path_type,
│                                        arrows: {start: bool, end: bool}, style: Binary,
│                                        label: LoroText}   // edge caption, char-CRDT (§5.B); empty = no label
│
├── "layers" (LoroMovableList<NodeId>) → global z-order, back-to-front paint order.
│                                        NodeId = Element(ElementId) | Connector(ConnectorId) — frames and
│                                        connectors share the single z-order (§2.G).
│
└── "media" (LoroMap)                → key = blake2b(plaintext); value = {w, h, mime, thumb: Binary}
                                        (`thumb` = inline micro-thumbnail placeholder; full blob lives
                                        content-addressed on MinIO. No back-pointer: N elements may share
                                        one hash — dedup, see `toile-interop §5`.)
```

`NodeId` is the z-order node identity:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NodeId { Element(ElementId), Connector(ConnectorId) }
```

> **Target vs as-built.** As with Trame, `style` is stored as **one postcard-encoded LWW register per element**,
> not fanned out over per-attribute keys — this is the convergence invariant (§2.E). A shared style dictionary
> (dedup) is deferred behind a benchmark.
>
> **`locked` placement.** `locked` is a top-level element field, not an `ElementStyle` member — a lock is a
> lifecycle flag (blocks hit-test and edit), not a visual attribute, and must not race with a restyle on the
> single style register.

### C. Per-type payload

| `type` | `payload` shape | Notes |
|---|---|---|
| `sticky` | `LoroText` (body) | Rich text delegated to §2.D. A sticky is a **filled, square card** (Miro-style: a plain rectangle, `style.corner = 0`, born square) with a small bottom-right drop shadow. Its `style.fill` is the card background (a pastel from the palette, ui §5quinquies.A), never `None`; the body is drawn on top in a fixed readable ink independent of `fill`, at a **font size that tracks the card size** (bigger note -> bigger text). This is the one kind whose per-kind default fill is not white (§2.E). Rendered as `DisplayGeom::Sticky`, distinct from `text`. |
| `shape` | `String` kind (`rect`/`ellipse`/`triangle`/`line`/`path`) + `Binary` `BezPath` for `path` | Vector nodes edited via §toile-features boolean ops. |
| `text` | `LoroText` (rich runs) | Parley layout in `toile-ui`. |
| `image` | `String` blob hash (into `media`) | Async load in `toile-interop`. Element `w`/`h` are the display size and are authoritative; the media entry's `w`/`h` are the native resolution (load info). |
| `frame` | `LoroMap` `{children: LoroMovableList<ElementId>, notes: LoroText, autolayout: Binary}` | A frame is an ordinary element (§2.F). `children` = membership, `notes` = collaborative speaker notes, `autolayout` = optional `postcard(AutoLayout)` (`toile-features §2`). |
| `group` | `LoroMap` `{children: LoroMovableList<ElementId>}` | A group is a frame-like container **without** chrome (no background/border, no notes, no autolayout) — §2.F bis. `children` = membership; geometry (`x`/`y`/`w`/`h`) is the derived union of its members. |
| `drawing` | `Binary` `Vec<StrokePoint>` | See §4. Single-author, never mutated after creation. |
| `embed` | `String` url | Sandboxed **web** iframe render in `toile-ui`. Not an Ambre document. |
| `doc_embed` | `LoroMap` `{document_id: Uuid, doc_type: DocType}` | A **nested Ambre document** (Prose/Trame). MVP in-place editable, role-gated (§2.I). Distinct from `embed` (a web url). |
| `card` *(DEFERRED, post-MVP)* | `LoroMap` `{title: LoroText, tags: LoroMovableList<TagId>, assignee: Option<UserId>, due: Option<i64>}` | Agile-team card (kanban/backlog). A richer sticky with structured fields; renders like a sticky with a tag row + assignee chip. Out of MVP; see roadmap. |

> **Tags (DEFERRED, post-MVP).** A board-level tag palette (`"tags"` top-level `LoroMap`: `TagId ->
> {label: String, color: u32}`) shared across the board, referenced by `card.tags` (and, later, addable to
> `sticky`). Tags enable grouping/filtering ("show only #blocked"). Kept out of MVP with `card` — both belong to
> the agile-team workflow layer, not the core whiteboard. When added, tag filtering is a **local view** (like
> Find §8 quater), never a CRDT mutation.

```rust
/// One sampled point of a freehand stroke. 12 bytes, fixed layout.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32, // 0.0 ..= 1.0, 1.0 when the input device reports none
}
```

### D. Rich text on sticky notes and text elements

Sticky and text bodies are `LoroText` so that two authors typing in the same note converge character-wise
(the Prose CRDT text pattern), not last-write-wins on the whole string. Layout (line breaking, run styling)
is a `toile-ui` concern; the Logic thread only owns the `LoroText` and exposes its rendered runs in the
`DisplayViewport`.

### E. Style register — one LWW binary per element

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ElementStyle {
    pub fill:        Option<u32>,   // ARGB, None = no fill
    pub stroke:      Option<u32>,
    pub stroke_w:    f32,
    pub opacity:     f32,           // 0.0 ..= 1.0
    pub corner:      f32,           // corner radius for rect/sticky
    pub font:        FontSpec,      // family, size, weight, italic, underline,
                                    // strikethrough — for sticky/text. The decoration
                                    // set matches Trame cells (shared format controls).
    pub shadow:      Option<Shadow>,
    pub text_color:  Option<u32>,   // text/foreground ARGB; None = kind default ink
                                    // (sticky ink, or theme colour for a text element).
                                    // Distinct from `fill` (card/shape background).
    // Schema note: the register is one postcard blob decoded whole; postcard does
    // NOT tolerate a missing/extra field, so any layout change makes a blob written
    // by an older binary fall back to `ElementStyle::default()` (`from_bytes` ->
    // `unwrap_or_default`). Acceptable pre-1.0 (no persisted-format guarantee yet);
    // a real migration path (versioned register) is owed before GA.
}
```

Storing the whole style as **one register** avoids the multi-register tear (e.g. a fill update racing a
stroke update leaving an inconsistent half-applied style) that a per-attribute fan-out suffers — the exact
defect enumerated in Trame `logic §11 bis.F`. Concurrent restyles resolve by LWW on the single register: all
peers converge to one style, deterministically.

**Per-kind defaults.** Element birth seeds the style register with a per-kind default, not a bare
`ElementStyle::default()`. Every kind uses the base default (white fill, no stroke) **except `sticky`**, whose
default is the sticky yellow fill with **no border and `corner = 0`** (a plain rectangular card, Miro-style), so
a freshly placed, still-empty note is immediately visible as a card (a `Text` element, by contrast, has no card
and is invisible until the user types — that is intended). The Sticky tool overrides the fill per placement via
`CreateElement.fill` (the palette colour, ui §5quinquies.A). The sticky card is painted with a small bottom-
right drop shadow. A sticky also seeds `font.size = 0` (the `STICKY_AUTO_FONT_SIZE` sentinel): its body font
tracks the card size unless the style inspector sets a positive size, which overrides the auto-sizing (ui §6).
Its body ink defaults to a fixed readable colour (never `style.fill`, which is the card background); the style
inspector can set an explicit `text_color` that wins over that default for both sticky and text elements.

### F. Frames are ordinary elements

A frame is **not** a separate entity: it lives in the `elements` map with `type = "frame"`, carries the same
geometry (`x`/`y`/`w`/`h`/`rot`/`locked`/`style`) as any element, and sits in `layers`. Therefore
`MoveElements` / `ResizeElement` / `RotateElement` / `DeleteElements` / `Reorder` operate on frames **for
free** — there are no frame-specific lifecycle commands. Only the frame-specific *content* commands exist
(`SetFrameChildren`, `EditNotes`, `SetAutoLayout`, §8), and they mutate the frame's payload sub-map.

> **Why not a separate `frames` map.** A parallel map would need its own move/resize/delete/reorder commands
> (a whole duplicated lifecycle) and would split z-order across two structures. Modelling a frame as an element
> collapses both problems: one lifecycle, one `layers`.

### F bis. Groups (MVP)

A **group** binds a set of elements so they select and transform as one, without the visual chrome of a frame.
It reuses the frame membership machinery (§2.F): a group is an ordinary element with `type = "group"` and a
`{children: LoroMovableList<ElementId>}` payload — so `MoveElements` / `RotateElement` / `DeleteElements` /
`Reorder` / `LockElements` apply to it for free. It differs from a frame in three ways: it paints nothing
(no fill/border, no `DisplayGeom`, see §9), it carries no notes/autolayout, and its geometry is the **derived
union** of its members' bounds (a group is never independently resized; resizing scales its members — Phase 2).

- **`GroupElements { ids }`** mints a new group `ElementId` (§2.A), moves each id into the group's `children`,
  and inserts the group `NodeId` into `layers` **just below its lowest member** so the members keep painting in
  their current relative order above it. A member already in another group is first removed from it (a node
  belongs to at most one group). Nesting is allowed (a group may be a child of another group).
- **`UngroupElements { group }`** removes the group element and re-parents its children to the group's own
  parent (the board, or an enclosing group), preserving their `layers` order. The children survive; only the
  binding is dropped. Deleting a group with `DeleteElements` instead deletes the members too (ordinary cascade).
- **Bounds.** The group's stored `x`/`y`/`w`/`h` are recomputed as the union AABB whenever a member moves,
  resizes, joins or leaves; this feeds the R-tree (§3) and selection hit-test. Bounds are derived, never an
  independent source of truth.
- **Convergence.** Membership is a `LoroMovableList` like a frame's `children`; concurrent group edits converge
  by the same rules (§2.G frame/child ordering invariant applies with the group standing in for the frame).

### G. Unified z-order (`layers`)

`layers` is the single authority for paint order and holds `NodeId`s (elements **and** connectors), so a
connector can sit behind or in front of elements (the Figma model), not forced above everything.

**Frame / child ordering invariant.** A frame paints its background+border when its own `NodeId` is reached in
`layers`; its children paint later. The engine maintains the invariant *a child's `layers` index is greater
than its frame's* on `SetFrameChildren` / reparent / `Reorder` (a reorder that would violate it clamps to just
above the frame). This gives correct visual nesting without a separate per-frame z-stack.

### H. Deletion cleanup & `layers` well-formedness

- **`DeleteElements`** removes the id from `layers`, from any frame's `children`, and drops the element's
  locally-owned connectors (§5.A) — one convergent local batch.
- **Well-formedness is derived, not assumed.** Paint order is `layers`; any id present in `elements` but
  **absent** from `layers` (e.g. a concurrent op dropped its entry) is painted **last**, ordered by
  `ElementId` — deterministic and convergent on all peers. Any `NodeId` in `layers` whose target is gone is
  **skipped** at derivation (same stance as a dangling connector, §5.A). No id is ever duplicated: create
  pushes exactly once; a defensive de-dup at derivation keeps the first occurrence.

### I. Nested Ambre documents (`doc_embed`)

A Toile can embed **other Ambre documents** (a Prose, a Trame), not just web urls. This is the composition
feature of `ambre-spec-architecture.md §2.B/§2.D`, distinct from the sandboxed web `embed`:

- **CRDT footprint in the Toile.** Only the *nesting relationship* lives in the Toile's `LoroDoc`: a
  `doc_embed` element whose payload is `{document_id, doc_type}` plus the usual geometry (`x`/`y`/`w`/`h`/`rot`).
  The embedded document has its **own** `DocumentId`, `DocumentKey` and `LoroDoc` — never inlined here (the
  server sees only the encrypted Toile content, no embedding table, per architecture §2.B).
- **Lifecycle = ordinary element.** Move/resize/rotate/delete/reorder/lock and z-order (`layers`) apply for
  free, exactly like a frame (§2.F). Deleting the `doc_embed` element removes the *reference*, not the target
  document.
- **Opening the child.** When a `doc_embed` becomes visible, the Logic dispatcher issues
  `OpenDocument { document_id }` for the child so the Sync worker adds a session on the single multiplexed
  WebSocket (N parallel sessions, already supported — architecture §2.B, sync §2.A); `CloseDocument` when it
  scrolls out or the Toile closes.
  **Hysteresis is mandatory.** Visibility comes from the §3 R-tree cull set, which flips on every pan; binding
  session lifecycle directly to it would thrash the WebSocket (open/close storm on a scroll oscillation, and a
  re-open costs a members fetch + key unwrap + snapshot hydration). So: open as soon as the embed enters the
  cull set, but close only after it has stayed out for a debounce window (`EMBED_CLOSE_DEBOUNCE_MS`, 5 s), and
  cancel the pending close if it re-enters. Closing the Toile closes every child immediately, no debounce.
- **MVP = in-place editable.** The Logic thread opens a **full** `DocState` for the child (not a read-only
  viewer): it applies the child's `RemoteUpdate`s **and** emits `SendUpdate` for the child when it is edited,
  subject to the caller's `Role` on that document. Input is routed to the focused embed (§below); an embed the
  user only has viewer rights on stays read-only, enforced by the child app exactly as when opened standalone.
  **Where the `Role` comes from.** A child is opened by the ordinary `OpenDocument` path, so it carries its own
  `Role` exactly like a standalone document: Sync resolves this client's own member entry at open and hands the
  role to Logic on `DocumentReady` (architecture §2.D, sync § "Presence identity" neighbourhood). Nesting adds
  nothing here — the parent Toile's role does **not** propagate to the child, and a user may well be `Editor` on
  a board while only `Viewer` on a Trame embedded in it. The client-side gate is **UX only** (do not let the user
  type into a document the server will refuse); the server remains authoritative and already rejects a
  read-only write on the WS `Update` path.
- **Command routing = direct, by the child's own id.** The Logic `docs` registry is flat and keyed by
  `DocumentId` (§1), so the child needs no envelope and no recursion: `FrameHost` owns the child's `AppView`,
  that view holds the child's `DocumentId`, and it emits `LogicCommand::App { document_id: child, .. }` on the
  same UI->Logic channel as any top-level view. The command reaches the child's `DocState` in one lookup and its
  outcome (`display` + `sync`) is fanned out normally, so the child's edits reach the child's own Sync session.
  The parent Toile's `LoroDoc` is untouched because no command for the child ever reaches it.
  **Why there is no `Embedded` variant.** An earlier draft of this section specified
  `ToileCommand::Embedded { child, command: Box<AppCommand> }`. It cannot work and is not needed. It cannot
  work because `ToileView` is unable to *build* the inner `AppCommand`: turning a pointer event into (say) a
  `TrameCommand` requires the child `TrameView`'s selection and edit state, and `ambre-toile` may not name
  `TrameView` or `TrameCommand` — that is the ARCH P2 cycle. What crosses into the child is therefore the raw
  `InputEvent`, not a command; the child view translates it and addresses the result itself. And it is not
  needed because the registry is flat, so there is nothing for the envelope to route around.
- **Input focus.** At most one `doc_embed` holds keyboard/pointer focus at a time. Focus is **local UI state**
  owned by `ToileView`, never CRDT. A single click inside an unfocused embed focuses it (and selects it on the
  board) without reaching the child; a click outside, or `Escape`, returns focus to the board. While an embed is
  focused, `FrameHost` maps each canvas `InputEvent` into the child's coordinate space and hands it to the child
  `AppView`'s own `handle_input` — the child then emits its own commands (above). The mapping is
  `child_px = ((elem.transform.inverse() * (camera.screen_to_world() * screen_pt)) - bounds.origin) * zoom`,
  which handles a rotated embed correctly. Screen-space *deltas* (wheel) need no scaling: a child pixel is a
  screen pixel by construction, since the same `zoom` scales both.
- **Render = driven by the launcher, not by `toile-ui`.** The child is painted through its own `AppView` content
  pass, clipped to the `doc_embed` rect — but **`ambre-toile` cannot do that itself**: `AppView` is the closed
  enum that lives in `ambre-app`, *downstream* of every app crate (architecture §6, §7 ARCH P2). A view naming
  `AppView` from inside `ambre-toile` is the exact cycle that moved `AppView`/`FrameHost` into the launcher in
  the first place, and no trait object may reintroduce it (§2.D; the `DocView` trait was deliberately removed).
  So the composition inverts: `ToileView` **publishes** its visible embeds (`document_id`, `doc_type`,
  screen-space rect, `focused`) and paints only their background/frame placeholder; `FrameHost` owns the child
  `AppView`s (`HashMap<DocumentId, AppView>`, built through the same construction path as a top-level view) and
  paints each one after the parent's content pass, inside a `push_clip(rect)` / `pop_clip` pair. The nesting is
  a launcher concern because the launcher is the only layer allowed to know every app type.
  **Per-document fan-out.** The Logic->UI channel is a single shared stream, and a view ignores the
  `document_id` on the events it drains (it assumes every event on its channel is its own). That holds only while
  one view reads one channel, so a child cannot share the top-level stream. The relay therefore **demultiplexes**:
  each child registers a private `UiEvent` sender keyed by its `document_id`, and the relay routes every
  doc-tagged event (`UiEvent::routed_document_id`) whose id is a live child to that channel instead of the
  top-level view's. Dropping the child (its embed left the cull set) drops the receiver, and the relay reclaims
  the slot on its next send. A child is also built in **embedded mode**: it reuses the one transparent text pipe
  but must not `focus()` it on construction (the top-level document owns the caret until the user clicks in), and
  its chrome signals (formula bar, zoom chip, status band, float bar) are throwaway — those are DOM singletons of
  the top-level document, so a child writes into dead signals no chrome reads.
  **Clip limitation (accepted).** `DrawSurface::push_clip` is rectangular and is resolved against the transform
  live at push time, so a **rotated** `doc_embed` clips to its axis-aligned bounding box, not to its rotated
  quad: a rotated embed can bleed into its AABB corners. Accepted for the MVP — rotating a nested document is a
  degenerate gesture — and revisited only if a clip-path op lands in `ambre-draw`.
  When focused, the child also receives `handle_input`; when not, it paints but does not consume input.

### J. Comment anchoring (model reference)

Comments are a **shared, persisted collaboration primitive** and, following the same placement as reactions
(their CRDT schema lives in `toile-features §4.A`, not here), the comment-thread CRDT schema, thread/reply
semantics and resolution are specified in `toile-features §4.D`. Only the *anchoring* rule is a core-model
concern and is fixed here: a comment thread is anchored **either** to an element (a stable `ElementId`, §2.A —
the pin tracks the element as it moves, and goes dangling like a connector §5.A if that element is deleted) **or**
to a free world-space point (`x`,`y`) when dropped on empty canvas. The anchor is therefore
`CommentAnchor = Element(ElementId) | Point { x: f64, y: f64 }`. Anchored pins are surfaced in the
`DisplayViewport` (§9) so `toile-ui` can paint and hit-test them; the thread bodies themselves are fetched by the
comment panel on demand, never streamed with every viewport.

### K. Hyperlinks & go-to-frame (`LinkTarget`)

Any element may carry an optional `link` (the top-level `Binary?` field, §2.B) turning it into a navigable
target — a whiteboard affordance (clickable legend, table-of-contents frame, external reference) that is also
useful in presentation mode. It is a per-element LWW register (postcard), never part of `ElementStyle` (it is
navigation behavior, not a visual attribute — same reasoning as `locked`).

```rust
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum LinkTarget {
    Url(String),              // external web URL, opened sandboxed by the shell
    Frame(ElementId),         // recenter/zoom the camera to a frame on THIS board (a frame is an element, §2.F)
    Document(uuid::Uuid),     // open another Ambre document (e.g. a Prose doc or a sibling board)
}
```

Following a link is a **local, non-mutating** action driven by `toile-ui` (§click behavior): `Frame` runs a
camera fly-to, `Url`/`Document` are handed to the shell. A `Frame` link whose target was deleted is inert
(surfaced like a dangling reference, never auto-cleared — data is never destroyed). Setting/clearing the link is
one CRDT register write (`SetLink`, §8) and one undo item.

---

## 3. SPATIAL INDEX (CULLING)

With tens of thousands of elements, iterating all of them per frame is impossible. An R-tree (`rstar`)
filters the elements intersecting the viewport in O(log N + k), k = visible count.

```rust
use rstar::{RTree, RTreeObject, AABB};

/// R-tree leaf: an element's world-space axis-aligned bounds + its id.
#[derive(Clone)]
pub struct ElementBounds {
    pub id:     ElementId,
    pub min:    [f64; 2],  // rotation-expanded AABB (see note)
    pub max:    [f64; 2],
}

impl RTreeObject for ElementBounds {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope { AABB::from_corners(self.min, self.max) }
}

pub struct SpatialIndex {
    tree: RTree<ElementBounds>,
}

pub struct Viewport {
    pub center: kurbo::Point, // world coordinates
    pub zoom:   f64,          // 0.1 ..= 10.0
    pub size:   kurbo::Size,  // screen size in physical pixels
}

impl Viewport {
    /// The world-space rectangle currently visible: the screen box mapped back
    /// through the inverse view transform. Drives `query_visible`.
    pub fn to_world_rect(&self) -> kurbo::Rect {
        let half = kurbo::Vec2::new(self.size.width, self.size.height) / (2.0 * self.zoom);
        kurbo::Rect::from_center_size(self.center, (2.0 * half.x, 2.0 * half.y).into())
    }
}

impl SpatialIndex {
    /// O(N) build at load; asserted identical to the incremental path in tests.
    pub fn build<'a>(elems: impl Iterator<Item = ElementBounds>) -> Self {
        Self { tree: RTree::bulk_load(elems.collect()) }
    }

    /// O(log N + k). Returned ids are paint-ordered by the caller via `layers`.
    pub fn query_visible(&self, vp: &Viewport) -> impl Iterator<Item = ElementId> + '_ {
        let r = vp.to_world_rect();
        self.tree
            .locate_in_envelope_intersecting(&AABB::from_corners(
                [r.x0, r.y0], [r.x1, r.y1],
            ))
            .map(|e| e.id)
    }

    pub fn insert(&mut self, b: ElementBounds)         { self.tree.insert(b); }
    pub fn remove(&mut self, id: ElementId)            { /* locate + remove_at */ }
    pub fn update(&mut self, b: ElementBounds)         { self.remove(b.id); self.insert(b); }

    /// Point hit pre-filter (screen->world done by caller). Exact hit-test
    /// (rotation, path fill, stroke width) is refined in toile-ui.
    pub fn query_point(&self, p: kurbo::Point) -> impl Iterator<Item = ElementId> + '_ {
        self.tree.locate_all_at_point(&[p.x, p.y]).map(|e| e.id)
    }
}
```

> **Rotation-expanded AABB.** A rotated element's index bounds are the AABB of its rotated corners, so the
> R-tree over-approximates. This never drops a visible element (safe for culling); the tight per-element
> hit-test is refined in `toile-ui`. `w`/`h`/`x`/`y`/`rot` are the source of truth; `ElementBounds` is derived.

**Incremental maintenance.** On every applied mutation (local `handle_command` or remote `apply_remote`) that
touches an element's geometry, the state calls `SpatialIndex::update`. Create → `insert`, delete → `remove`.
The tree is a pure cache: after any remote import batch the state may issue a single bulk `update` rather than
per-op churn.

---

## 4. FREEHAND DRAWING

Each stroke is one `"drawing"` element owning a `Vec<StrokePoint>`. Because a stroke belongs to a single
author and is never mutated after commit, **no CRDT conflict is possible** — concurrent strokes are
independent element inserts. The Logic thread stores only the raw sampled points; **Catmull-Rom smoothing is
recomputed at derivation** (and again on the GPU path in `toile-ui`), never persisted.

```rust
/// Derivation-time smoothing. Pure, deterministic, no allocation on the hot path
/// beyond the output buffer. Returns a flattened polyline the UI tessellates.
pub fn smooth_stroke(points: &[StrokePoint], out: &mut Vec<kurbo::Point>) {
    out.clear();
    // Catmull-Rom through sampled points; endpoints duplicated for tangents.
    // (implementation: standard CR spline, `segments_per_span` from zoom in toile-ui)
}
```

Streaming a live stroke (before commit) is an **ephemeral presence** payload, not a CRDT delta — see §7 and
`toile-ui §multi-cursors`. Only the finished stroke becomes a `"drawing"` element.

---

## 5. CONNECTORS

A connector references a source and target `ElementId` plus an anchor on each (`Top`/`Bottom`/`Left`/`Right`/
`Center` or a normalized `(u, v)` on the element's AABB). It carries a `path_type` (`Straight`/`Elbow`/`Curved`)
and a style. **Connector geometry is pure client compute**: when an element moves, its connectors are
recomputed at derivation — *no CRDT mutation on the connector*. This keeps element drags to a single register
write and makes connectors free of merge conflicts.

```rust
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum Anchor { Top, Bottom, Left, Right, Center, Free { u: f32, v: f32 } }

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum PathType { Straight, Elbow, Curved }

/// Arrowheads are directional decoration on the connector, persisted in the CRDT
/// (a restyle-class LWW field), and surfaced per-end on the DisplayConnector.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct EndCaps { pub start: bool, pub end: bool }

pub struct Connector {
    pub id:        ConnectorId,
    pub from:      ElementId,
    pub from_anch: Anchor,
    pub to:        ElementId,
    pub to_anch:   Anchor,
    pub path_type: PathType,
    pub arrows:    EndCaps,
    // `label` is the connector's own `LoroText` (§5.B), not mirrored into this
    // routing struct; the deriver reads it straight from the CRDT for the DisplayConnector.
}

impl Connector {
    /// Routes the connector against the current element bounds. `None` when an
    /// endpoint element was deleted (the connector is then dropped from the
    /// DisplayViewport and, if the deletion was local, garbage-collected -- §5.A).
    pub fn route(&self, bounds: &HashMap<ElementId, kurbo::Rect>) -> Option<kurbo::BezPath> {
        let a = anchor_point(bounds.get(&self.from)?, self.from_anch);
        let b = anchor_point(bounds.get(&self.to)?,   self.to_anch);
        Some(match self.path_type {
            PathType::Straight => line(a, b),
            PathType::Elbow    => elbow(a, self.from_anch, b, self.to_anch),
            PathType::Curved   => cubic(a, self.from_anch, b, self.to_anch),
        })
    }
}
```

### B. Connector labels (edge captions)

A connector carries an optional **label** — the text placed on the edge ("depends on", "yes/no", a flow
condition) that makes a diagram readable. It is stored as the connector's own `LoroText` (§2.B `connectors`
map), **char-CRDT** so two authors captioning the same edge converge exactly like a sticky body (§2.D), never
LWW-on-the-whole-string. An empty `LoroText` = no label (nothing painted). The label has no geometry of its own:
`toile-ui` lays it out at the routed path's midpoint (a `Free`-anchored offset is a post-MVP refinement), so it
follows the edge as endpoints move — no CRDT mutation on element drag, consistent with connector routing. Editing
it is `EditConnectorLabel` (§8), one undo item per editor-exit run (same granularity as `EditText`).

### A. Dangling connectors

When an endpoint element is deleted, its connectors become dangling. On a **local** delete the state removes
the orphaned connectors in the same command (a convergent, intentional cleanup). Under **concurrency** (Alice
deletes the element, Bob keeps editing the connector) a connector may converge referencing a missing id; it is
simply **skipped** at derivation (`route` returns `None`) and surfaced as `ToileEvent::DanglingConnector` so the
UI can offer a reconnect — data is never destroyed, mirroring the Trame merge-conflict stance (`logic §6`).

---

## 6. CRDT CONFLICTS

Elements are independent, so conflicts are rare and cheap:

| Scenario | Resolution |
|---|---|
| Two users move the same element | LWW on its `x`/`y` registers — acceptable for a whiteboard (matches Miro/Figma). |
| Two users restyle the same element | LWW on the single `style` register (§2.E) — no half-applied tear. |
| Concurrent creates | Distinct `ElementId`s (distinct `PeerID`) — both survive. |
| Concurrent inserts into a frame's `children` / into `layers` | `LoroMovableList` orders them canonically. |
| Concurrent stroke drawing | Independent `"drawing"` inserts — no shared state. |
| Delete vs edit of the same element | Loro tombstone wins the map entry; dependent connectors go dangling (§5.A), value never corrupted. |

There is **no recompute cascade** (Toile has no dependency graph), so a convergence test only needs to assert
identical `LoroDoc` state and identical derived `SpatialIndex`, not identical computed values.

> **`locked` is advisory, not enforced.** The `locked` flag (§2.B) blocks hit-test and edit **locally**; it is
> soft-enforcement under the blind relay, exactly like Trame's validation/protection (`trame-features §5/§6`). A
> collaborator who locked it concurrently — or a modified client — can still move it, and its `x`/`y` registers
> still resolve by LWW like any other element. It prevents accidental edits by official clients; it is not an
> integrity guarantee. A per-user ACL (owner/allowed lists) is intentionally out of scope (over-scoped for a
> zero-knowledge whiteboard).

---

## 7. INTERNAL STATE

```rust
pub struct ToileState {
    doc:      loro::LoroDoc,
    undo:     loro::UndoManager,                 // local-only undo/redo (§8 bis), created on open
    index:    SpatialIndex,

    /// Derived-only caches, rebuilt from `doc`, never authoritative:
    bounds:   HashMap<ElementId, kurbo::Rect>,   // rotation-expanded AABBs, feeds culling + connector routing
    layers:   Vec<NodeId>,                       // materialized paint order from the `layers` LoroMovableList

    /// Ephemeral (never in Loro): live cursors, in-flight strokes, selections of peers.
    presence: PresenceState,

    counter:  u32,                               // per-session id minting counter
    peer:     u64,
    viewport: Viewport,                          // last viewport the UI reported (drives culling)

    /// Canonical per-user identifier (received via `LogicEvent::SessionReady`), used to key
    /// this peer's own persisted reactions/votes (`toile-features §4.A`). A stable `Uuid`,
    /// NOT `peer` (which is per-session/per-device and only mints element ids). Same field
    /// as `trame-logic §7`.
    my_user_id: Uuid,
}
```

`presence` (multi-cursors, live strokes, laser pointer, selection highlights) is transported as **ephemeral**
payloads over the Sync thread, never as CRDT deltas — it must not enter history and must not persist. See
`toile-ui §multi-cursors` for the render side.

> **Presence identity — canonical `UserId`, not `PeerID`.** A `PresenceUpdate` is keyed by a per-**user**
> `Uuid`, resolved from the sender's `pub_key` in `ambre-sync` (via `trusted_keys`) **before** the ephemeral
> frame reaches the Logic thread — the UI never touches a cryptographic key. `PeerID` mints element ids (§2.A,
> per-session/per-device) and is the wrong key for presence: the same user on two devices would otherwise
> appear as two collaborators in two colors. The peer **color** and name label therefore hash the `UserId`
> with the shared cross-app algorithm (`ambre_draw::peer::peer_color`), so a given collaborator is the **same color**
> in Prose, Trame and Toile (`trame-ui §9`).

---

## 8. INTERFACE CONTRACT (API)

### A. Commands (UI/Logic → `ToileState`)

```rust
pub enum ToileCommand {
    // Element lifecycle
    // `content` seeds type-specific data at birth (a Pen path, an imported image
    // hash, an embed url, initial text). `Sticky`/`Text` typically pass `None` and
    // fill the LoroText afterwards via `EditText`. `Drawing` is NOT creatable here
    // (a stroke is born only through `CommitStroke`); such a `kind` is rejected.
    CreateElement { kind: ElementKind, at: kurbo::Point, size: kurbo::Size, content: InitContent, fill: Option<u32> },
    MoveElements  { ids: Vec<ElementId>, delta: kurbo::Vec2 },
    ResizeElement { id: ElementId, new: kurbo::Rect },
    RotateElement { id: ElementId, radians: f64 },
    RestyleElements { ids: Vec<ElementId>, style: ElementStyle },
    DeleteElements { ids: Vec<ElementId> },
    LockElements  { ids: Vec<ElementId>, locked: bool },
    SetLink       { id: ElementId, link: Option<LinkTarget> }, // hyperlink / go-to-frame (§2.K); None clears

    // Arrange: align a selection to a shared edge/center, or distribute it evenly.
    // One convergent batch of x/y register writes (like MoveElements), one undo item.
    // Align uses the selection's union bounds as the reference; distribute needs >= 3
    // elements and spaces their centers evenly along the axis. Locked members are skipped.
    ArrangeElements { ids: Vec<ElementId>, op: ArrangeOp },

    // Z-order (over the unified NodeId layer list, §2.G)
    Reorder { node: NodeId, to: ZTarget },

    // Text
    EditText { id: ElementId, delta: TextDelta }, // applied to the element's LoroText

    // Connectors
    CreateConnector { from: ElementId, from_anch: Anchor, to: ElementId, to_anch: Anchor, path_type: PathType, arrows: EndCaps },
    RetargetConnector { id: ConnectorId, endpoint: End, to: ElementId, anch: Anchor },
    EditConnectorLabel { id: ConnectorId, delta: TextDelta }, // edge caption LoroText (§5.B)
    DeleteConnector { id: ConnectorId },

    // Freehand
    CommitStroke { points: Vec<StrokePoint>, style: ElementStyle },

    // Frames (a frame is an element, §2.F — created via CreateElement{kind:Frame};
    // only its content is frame-specific). Membership lives here; the remaining
    // frame-content commands (EditNotes, SetAutoLayout) are owned by toile-features §5.
    SetFrameChildren { frame: ElementId, add: Vec<ElementId>, remove: Vec<ElementId> },

    // Groups (§2.F bis). Group = frame-like container without chrome; select/transform as one.
    GroupElements   { ids: Vec<ElementId> },   // mints a group, moves ids into it
    UngroupElements { group: ElementId },      // drops the binding, re-parents children

    // Clipboard (§8 ter). Copy extracts a self-contained payload the UI stashes (and may
    // mirror to the OS clipboard); paste/duplicate re-mint ids Logic-side. Cut = Copy + Delete.
    CopyElements      { ids: Vec<ElementId> },                          // -> ToileEvent::Clipboard
    PasteElements     { payload: ClipboardPayload, at: kurbo::Point },  // re-mint + insert at cursor
    DuplicateElements { ids: Vec<ElementId>, delta: kurbo::Vec2 },      // copy+paste in one undo item

    // Find (search on board, §8 quater). Runs Logic-side over text-bearing elements
    // (sticky/text bodies, frame titles); returns the hits plus the element to
    // recenter on. Non-mutating, no CRDT effect. `from` is the current match to
    // resume from; `backward` steps to the previous hit rather than the next (both
    // wrapping). A `None` `from` starts a fresh search focusing the first hit.
    Find { query: String, options: FindOptions, from: Option<ElementId>, backward: bool },

    // Selection aggregates for the status bar (§8 quinquies). The selection may fall
    // outside the culled viewport, so the Logic thread (which owns every element's
    // geometry) computes the count + union bounds; the UI holds only ids.
    RequestSelectionStats { ids: Vec<ElementId> },

    // Nested Ambre documents (§2.I) need NO variant here. The child's edits are
    // addressed to the child's own DocumentId by the child's own view; they never
    // travel through the board. See §2.I "Command routing".

    // Undo / redo (local-only, §8 bis)
    Undo, Redo,

    // Persistence / compaction: ask the state for a full CRDT snapshot (answered by
    // export_snapshot below, not a display event). Emitted by the dispatcher on a
    // Sync SnapshotNeeded, or for local-persistence compaction. No CRDT mutation.
    RequestSnapshot,

    // Viewport (drives culling; no CRDT effect)
    SetViewport(Viewport),

    // Ephemeral presence (never persisted)
    Presence(PresencePayload), // cursor move, live-stroke sample, laser point, selection
}
```

Supporting types:

```rust
pub enum ZTarget { Front, Back, Forward, Backward, Index(usize) }

/// `Drawing` is a valid element *type* (it tags a stroke in `DisplayElement.kind`)
/// but is NOT a valid `CreateElement` argument: strokes are born only via
/// `CommitStroke`. `handle_command` rejects `CreateElement{kind: Drawing, ..}`.
/// `Group` is likewise not creatable via `CreateElement` — a group is born only
/// through `GroupElements` (§2.F bis); such a `kind` is rejected too.
pub enum ElementKind { Sticky, Shape(ShapeKind), Text, Image, Frame, Group, Drawing, Embed, DocEmbed }
pub enum ShapeKind { Rect, Ellipse, Triangle, Line, Path }
pub enum End { From, To } // which endpoint of a connector to retarget

/// Arrange operation for `ArrangeElements` (§8.A). Align snaps every selected
/// element to a shared edge/center of the selection's union bounds; Distribute
/// equalizes the gaps between element centers along one axis (needs >= 3 elements).
pub enum ArrangeOp {
    Align(AlignEdge),        // Left/HCenter/Right/Top/VCenter/Bottom
    Distribute(Axis),        // Horizontal | Vertical
}
pub enum AlignEdge { Left, HCenter, Right, Top, VCenter, Bottom }
pub enum Axis { Horizontal, Vertical }

/// Type-specific seed passed to `CreateElement`. The variant must match `kind`
/// (a mismatch, e.g. `Path` content on a `Sticky`, is rejected):
///   Sticky/Text  -> None (or `Text` for a pre-filled body)
///
/// `CreateElement.fill` is an optional creation-time override of the seeded
/// `style.fill`. Element birth seeds a per-kind default style (§2.E "Per-kind
/// defaults"); a `Some(argb)` fill overwrites the fill of that default (the rest
/// of the style is unchanged). The Sticky tool uses it to place a note in the
/// colour the user picked from the palette (ui §5quinquies.A), atomically — one
/// element, one undo item — rather than a create followed by a restyle. `None`
/// keeps the per-kind default (e.g. the sticky yellow).
///   Shape(Path)  -> Path (the Pen tool's postcard BezPath, ui §5bis)
///   Shape(other) -> None (geometry comes from `size`)
///   Image        -> Image (blob hash into `media`, interop §2.B/§5)
///   Embed        -> Embed (sandboxed url)
///   Frame        -> None (empty children + notes containers seeded at birth so
///                  concurrent notes edits share one LoroText and converge,
///                  features §1.D/§6; autolayout stays lazy, absent = free)
pub enum InitContent {
    None,
    Path(kurbo::BezPath),
    Image(String),  // blake2b plaintext hash, key into the `media` map
    Embed(String),  // web url (sandboxed iframe)
    DocEmbed { document_id: Uuid, doc_type: DocType }, // nested Ambre document (§2.I)
    Text(String),   // initial plain body, converted to the element's LoroText
}
// TextDelta is the shared rich-text delta from Prose / `ambre-bus` (not redefined here).

/// Self-contained, id-free snapshot of a selection produced by `CopyElements`.
/// Portable across boards and sessions: it carries plaintext snapshots (LoroText
/// bodies flattened, style/payload postcard blobs), the connectors whose BOTH
/// endpoints are inside the set, and group/frame membership remapped to local
/// slot indices. Media are referenced by their content hash only (blobs stay on
/// MinIO, deduped — `toile-interop §5`). A `doc_embed` is copied as the same
/// `document_id` (a second reference to the one nested document, not a clone).
pub struct ClipboardPayload {
    pub elements:   Vec<ClipElement>,      // slot-indexed; old ids never leak
    pub connectors: Vec<ClipConnector>,    // endpoints remapped to element slots
    pub membership: Vec<ClipMembership>,   // frame/group -> child slots
    pub origin:     kurbo::Rect,           // source bounds, for paste-at-cursor offset
}

/// Find options (§8 quater). Search runs against each element's plaintext body
/// (sticky/text) and frame title; `match_case` toggles case folding, `whole_word`
/// requires a word-boundary match. Replace is intentionally out of MVP scope
/// (marginal on a canvas) — see §12.
pub struct FindOptions { pub match_case: bool, pub whole_word: bool }

/// Result of `Find` (§8 quater). `hits` are the matching elements in a stable
/// order (z-order); `focus` is the element to recenter the camera on (the next
/// hit after `from`, wrapping around). Purely local view state — never synced.
pub struct FindResults { pub hits: Vec<ElementId>, pub focus: Option<ElementId> }

/// Selection aggregates (§8 quinquies) for the status-bar band. `bounds` is the
/// union AABB of the selected elements in world space; `count` is the element
/// tally. Recomputed on demand, never persisted.
pub struct SelectionStats { pub count: u32, pub bounds: Option<kurbo::Rect> }
```

### D. Clipboard & duplication semantics (§8 ter)

`CopyElements` does **not** mutate the doc: it reads the selection and returns
`ToileEvent::Clipboard(ClipboardPayload)`; the UI stashes it (and may serialize it to the OS clipboard for
cross-tab paste). `PasteElements` / `DuplicateElements` are the mutating half and run entirely Logic-side:

- **Re-minting.** Every pasted element/connector/group gets a **fresh** `ElementId`/`ConnectorId` (§2.A) from
  the local counter — pasted content never collides with the source, so pasting onto the same board is safe.
  Old→new ids are remapped for connector endpoints and group/frame membership; a connector whose partner was
  **not** copied is dropped (it would dangle).
- **Placement.** Pasted nodes are offset so `origin` lands at `at` (the cursor, or a small fixed nudge for a
  duplicate), then pushed onto `layers` above the current top. `DuplicateElements` is `Copy`+`Paste` fused into
  a single undo item at `delta` (the alt-drag / `Ctrl+D` offset).
- **Cut** is not a distinct command: the UI issues `CopyElements` then `DeleteElements` on the same selection.
- **Undo.** A paste, a duplicate, a group and an ungroup are each **one** undo item (§8 bis).

### E. Find (search on board, §8 quater)

`Find` is **non-mutating** and runs entirely on the Logic thread — the UI holds only the culled
`DisplayViewport`, so a match on an off-screen sticky is invisible to it. The handler walks the text-bearing
elements (sticky/text `LoroText` bodies, frame titles) and returns `ToileEvent::FindResults { hits, focus }`:
`hits` are all matching `ElementId`s in z-order, `focus` is the hit adjacent to `from` in the requested
direction (`backward` steps to the previous hit, else the next; both wrapping), which the UI recenters the
camera on. `match_case` / `whole_word` refine the scan. Matches and the highlight are **local
view state**, never synced and never an undo item; the UI clears them when the panel closes or the query
changes. Replace is out of MVP scope (§12). The panel itself is DOM chrome (`toile-ui §5quater`), reached over
the existing command channel exactly like the context menu (§5ter).

### F. Selection aggregates (status bar, §8 quinquies)

`RequestSelectionStats { ids }` returns `ToileEvent::SelectionStats { count, bounds }` — the element tally and
the union AABB (world space) of the selection. Computed Logic-side because the selection can exceed the
viewport (culled elements are absent from the UI), reading the derived `bounds` cache (§7). Parameter-light and
non-mutating; the UI requests it on a non-trivial selection change and paints the band (`toile-ui §8bis`).
Pure view state, never persisted.

### B. Events (`ToileState` → UI)

```rust
pub enum ToileEvent {
    ViewportData(DisplayViewport),          // the derived visible set to paint (§9)
    ElementCreated(ElementId),
    SelectionChanged(Vec<ElementId>),
    Clipboard(ClipboardPayload),            // result of CopyElements; UI stashes / mirrors to OS clipboard (§8 ter)
    FindResults(FindResults),               // text-search hits + recenter target (§8 quater)
    SelectionStats(SelectionStats),         // status-bar aggregates for the selection (§8 quinquies)
    DanglingConnector(ConnectorId),
    PeerPresence(PresenceUpdate),           // remote cursors / strokes / lasers to render
    Error(ToileError),
}
```

> **Bus wrapping.** `ToileEvent` is a variant of the shared `AppEvent`; the Logic→UI transport wraps it as
> `UiEvent::Viewport { document_id, event }` for display events (`ViewportData`, `ElementCreated`, …) and as
> `UiEvent::Presence { document_id, event }` for `PeerPresence`. The dispatcher fills `document_id`; this spec
> only produces the `ToileEvent`.

### C. Outcome

```rust
/// Returned by handle_command; the ambre-logic dispatcher fans it out.
pub struct ToileOutcome {
    pub display: Vec<ToileEvent>,           // -> UI thread
    pub sync:    Vec<SyncCommand>,           // -> Sync thread (SendUpdate for CRDT, ephemeral for presence)
}

impl ToileState {
    pub fn handle_command(&mut self, cmd: ToileCommand) -> ToileOutcome { /* ... */ }

    /// Applies a remote delta batch (or a full snapshot — `doc.import` accepts both),
    /// updates the index incrementally, returns the display events to repaint. Presence
    /// arrives on a separate path (`apply_ephemeral`). A remote import is never re-broadcast.
    pub fn apply_remote(&mut self, update: &[u8]) -> Vec<ToileEvent> { /* ... */ }

    /// Applies a decrypted remote ephemeral (presence) frame -> PeerPresence events.
    /// The sender's pub_key is already resolved to a canonical UserId in ambre-sync
    /// (§7) before this is called; no CRDT mutation, no sync payload leaves here.
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<ToileEvent> { /* ... */ }

    /// Exports a full CRDT snapshot for local persistence (IndexedDB), server-side
    /// compaction, or a Sync SnapshotNeeded. Read-only. Mirrors trame-logic §8.
    pub fn export_snapshot(&self) -> Vec<u8> { self.doc.export(loro::ExportMode::Snapshot) }
}
```

**Data flow.** `handle_command` mutates `doc`, updates `index`/`bounds`/`layers`, re-derives the visible set,
and returns `display` (to UI) + `sync` (`SendUpdate` with `doc.export_updates()`, or a `SendEphemeral` presence
frame). `apply_remote` / `apply_ephemeral` are the symmetric inbound paths. All bare `flume`; tests use the
`CommandSink` seam.

> **Persistence & snapshot exchange (dispatcher-orchestrated, as Trame).** The `ambre-logic` dispatcher owns the
> transport, exactly as in `trame-logic §8`: whenever a `handle_command` outcome carried a `SendUpdate` (the doc
> changed), it also takes `state.export_snapshot()` and hands it to the local-persistence path (IndexedDB), so a
> board survives reload without replaying full history. A Sync `SnapshotNeeded` (or `ToileCommand::RequestSnapshot`)
> is answered by `SendSnapshot { document_id, payload: state.export_snapshot() }`; an inbound `RemoteSnapshot`
> flows through `apply_remote` (`doc.import` classifies delta vs snapshot). `SessionReady` / `ConnectionStatus`
> are handled by the dispatcher, not `ToileState` — except that `SessionReady` seeds `my_user_id` (§7).

---

## 8 bis. UNDO / REDO (LOCAL-ONLY)

Toile reuses the **same model as Trame** (`trame-features §4`): a `loro::UndoManager`, created on document
open and dropped on close, that undoes only the **local peer's** operations — remote edits are never undone
(the behaviour of every modern collaborative editor). No cross-session persistence.

```rust
// created in ToileState::new
let undo = loro::UndoManager::new(&doc);
```

**Granularity — one undo item per atomic user action:** a create, a move/resize/rotate drag (committed on
release), a restyle, a delete, a `CommitStroke`, a connector create/retarget, a `SetFrameChildren`, an
`ArrangeElements` (align/distribute batch), a `SetLink`, an `EditConnectorLabel` run (committed on editor exit),
an `EditText`/`EditNotes` run (committed on editor exit), an `AutoLayout` change, and — per `toile-interop §2.E` —
**a whole import batch as a single item**, a `GroupElements`/`UngroupElements`, and a `PasteElements`/
`DuplicateElements` (each one item, §8 ter). The `toile-features` commands that mutate the CRDT are undoable on
the same one-item-per-action rule: `SetSlideOrder`, `BooleanOp`, `React`, `EditNotes`, `SetAutoLayout`.
`CopyElements` mutates nothing and is not an undo item.
Ephemeral presence, `SetViewport`, and the local playback commands (`NextSlide`/`PrevSlide`/`GotoSlide`,
`EnterPresent`/`ExitPresent`) are **not** undoable (no doc mutation, exactly as Trame's personal filter view is
excluded).

> **Invariant — one `doc.commit()` per mutating command (undo granularity).** Every `handle_command` branch that
> mutates `LoroDoc` must end with `self.doc.commit()`. Loro groups all mutations since the last commit into a
> single `UndoManager` item, so without an explicit commit several successive `ToileCommand`s would collapse into
> one undo step, breaking the "one item per atomic user action" rule above (the exact footgun documented in
> `trame-logic §8`). Continuous gestures are the deliberate exception: a move/resize/rotate drag and an
> `EditText`/`EditNotes` run stream without committing and commit **once** on release / editor exit, so the whole
> gesture is one item. A live-stroke streams as ephemeral presence and commits once at `CommitStroke`.

```rust
fn handle_undo(&mut self) {
    if self.undo.undo().is_ok() { self.rebuild_after_undo_redo(); }
}
fn handle_redo(&mut self) {
    if self.undo.redo().is_ok() { self.rebuild_after_undo_redo(); }
}

/// Loro has reapplied a prior version in `doc`; rebuild the derived caches from
/// the diff (touched element bounds -> SpatialIndex update, layers/connectors
/// re-materialized) and emit a fresh DisplayViewport. Simpler than Trame: no
/// bytecode, no dependency graph to reseed.
fn rebuild_after_undo_redo(&mut self) { /* diff -> index/bounds/layers update -> ViewportData */ }
```

---

## 9. DATA-ORIENTED STRUCTURES (UI Thread)

The engine hands the UI a flat, ready-to-tessellate snapshot of exactly the visible set — no `LoroDoc` access
from the render thread, no per-element map lookups mid-frame.

```rust
/// One postcard-serialized frame of "what to paint", already culled + z-ordered.
/// This is the CRDT visible set only. Presence (cursors / live strokes / lasers)
/// is NOT carried here: it is ephemeral, arrives asynchronously outside the
/// `handle_command` cull cycle, and travels on its own channel as
/// `ToileEvent::PeerPresence` (§8.B, rendered by the ui §8 overlay pass). Keeping
/// it off `DisplayViewport` means a cursor move never forces a viewport re-derive.
pub struct DisplayViewport {
    pub viewport: Viewport,
    /// Single back-to-front sequence mirroring the `layers` walk (§2.G): elements
    /// and connectors interleaved by z-order. The UI paints it in one pass.
    pub nodes:    Vec<DisplayNode>,
    /// Anchored comment pins intersecting the viewport (§2.J, features §4.D). Not
    /// z-ordered with `nodes`: pins float above content and are painted last.
    pub comments: Vec<DisplayComment>,
}

pub enum DisplayNode {
    Element(DisplayElement),
    Connector(DisplayConnector),            // already routed to a BezPath (§5)
}

pub struct DisplayElement {
    pub id:        ElementId,
    pub kind:      ElementKind,
    pub transform: kurbo::Affine,           // world transform incl. rotation
    pub bounds:    kurbo::Rect,
    pub locked:    bool,                     // top-level element flag (§2.B), NOT from ElementStyle
    pub linked:    bool,                      // has a LinkTarget (§2.K) -> UI shows a link affordance / follows on click.
                                             // The target itself is fetched on demand (SetLink echo / hit), not per-frame.
    pub style:     ElementStyle,
    pub geom:      DisplayGeom,
    pub group:     Option<ElementId>,        // immediate enclosing group (§2.F bis); None = top-level.
                                             // UI resolves group-selection by walking this edge upward (§7).
}

pub enum DisplayGeom {
    Rect,
    Ellipse,
    Triangle,
    Path(kurbo::BezPath),
    Sticky(Vec<TextRun>),                   // sticky note: a filled, rounded card (style.fill) with a body drawn on top
    Text(Vec<TextRun>),                     // free text: Parley-ready rich runs, no card background
    Image(String),                          // media blob hash (toile-interop resolves the pixels)
    Stroke(Vec<kurbo::Point>),              // Logic-smoothed polyline (§4)
    Frame,                                  // background+border only; children are separate DisplayElements
    Group,                                   // paints nothing; carried only so the UI can resolve group selection (§2.F bis)
    Embed(String),                          // web url (sandboxed iframe, painted by the ui overlay)
    DocEmbed { document_id: Uuid, doc_type: DocType, focused: bool }, // nested Ambre doc (§2.I); ui paints the child AppView
}

pub struct DisplayConnector {
    pub id:    ConnectorId,
    pub path:  kurbo::BezPath,               // already routed against current bounds (§5)
    pub style: ElementStyle,
    pub arrows: EndCaps,                      // arrowheads per end, from the connector CRDT (§5)
    pub label: Option<Vec<TextRun>>,          // edge caption (§5.B), laid out at the path midpoint; None = empty
}

/// Anchored comment pins in the visible set (§2.J, toile-features §4.D). Carried
/// alongside `nodes` so the UI can paint/hit-test markers without a LoroDoc read;
/// thread bodies are fetched on demand by the comment panel, never streamed here.
pub struct DisplayComment {
    pub thread:   CommentThreadId,
    pub at:       kurbo::Point,               // resolved world position (element anchor -> its current AABB, or free point)
    pub resolved: bool,                        // resolved threads render collapsed / muted
    pub count:    u32,                         // message count, for the pin badge
}
```

Comment pins ride in a sibling `DisplayViewport.comments: Vec<DisplayComment>` field (kept off `nodes` because a
pin is not z-ordered with board content and must never be occluded by it). `DisplayViewport` is the single
Logic→UI contract; it is rebuilt only for the visible set on each relevant
command, keeping the render thread free of business logic (the Trame `DisplayViewport` pattern, `logic §9`).

---

## 10. QUALITY & TESTABILITY
> See the transverse test architecture in `ambre-spec-architecture.md §4`.

- **Test tiers:** **host** (`cargo test`, x86_64, default) for the engine, index, routing and smoothing;
  **wasm** (`wasm-bindgen-test`) only where a `cfg(target_arch = "wasm32")` seam or GPU surface is exercised —
  the pure engine here is host-testable. No **db** tier (no server integration in this module).
- **Unit tests:** every command path, every no-op guard (locked element, missing id), every error path
  (`RetargetConnector` to a deleted element → `DanglingConnector`), and the id-minting counter monotonicity.
  Target: every predicate branch of `handle_command`, `Connector::route`, the anchor/axis math, and
  the culling query. Indicative coverage: all branches of routing (Straight/Elbow/Curved) and every `Anchor`.
- **Index equivalence property test (mandatory):** for a random op sequence, assert the **incrementally
  maintained** `SpatialIndex` equals a **from-scratch rebuild** — the core cache invariant (§1, §3).
- **CRDT convergence (mandatory):** a 2-peer convergence property test — interleaved op sequences via
  `MockSyncHandle` + `apply_remote` → identical final `LoroDoc` **and** identical derived `bounds`/`layers`/
  `index`. Include concurrent move (LWW), concurrent restyle (single-register, no tear), concurrent create
  (distinct ids), concurrent frame-children insert (`LoroMovableList` order), concurrent group membership
  (a node ends in at most one group), and delete-vs-edit (dangling connector, no corruption). Also assert the
  `layers` well-formedness rules of §2.H (missing-from-layers fallback order, skip of a dangling `NodeId`, no
  duplicate) and the frame/child z-order invariant (§2.G).
- **Clipboard & grouping (mandatory):** `PasteElements`/`DuplicateElements` re-mint every id (no collision with
  the source, internal connectors remapped, orphan connectors dropped, `origin`→`at` offset); a round-trip
  `CopyElements`→`PasteElements` on a *separate* `ToileState` reproduces the selection (cross-board portability).
  `GroupElements` then `UngroupElements` is identity on the surviving children (relative order + `layers`
  preserved); deleting a group cascades to its members; group bounds equal the union of member AABBs after a
  member move.
- **Find & selection stats:** `Find` returns hits in z-order with `match_case`/`whole_word` honored and a
  wrapping `focus`, matches only text-bearing elements (sticky/text bodies, frame titles), and mutates nothing;
  `RequestSelectionStats` returns the exact count and union AABB for a selection that includes an off-viewport
  element (Logic-side geometry, §8 quinquies).
- **Undo/redo:** local-only property test — an interleaved local+remote sequence, then `Undo` reverts **only**
  local ops and `rebuild_after_undo_redo` yields caches identical to a from-scratch rebuild of the reverted doc;
  an import batch undoes as a single item (`toile-interop §2.E`).
- **Persistence & presence inbound:** an `export_snapshot` -> a fresh state loaded from that snapshot round-trip
  reproduces identical `LoroDoc` + derived `bounds`/`layers`/`index` (§8); `apply_ephemeral` on a presence frame
  yields the expected `PeerPresence` keyed by the resolved `UserId` and mutates neither `doc` nor history (§7).
- **Reactions identity:** a same-user re-vote from two distinct `PeerID`s (same `my_user_id`) converges to **one**
  entry and one tally unit — the `UserId`-keyed guarantee of `toile-features §4.A`.
- **Mocks & Environment:** host seams `CommandSink` / `LogicSink` / `MockSyncHandle`; isolated, headless, no
  live network.
- **Benchmarks:** `criterion` on `query_visible` at N = 50 000 elements (target below), incremental
  `SpatialIndex::update`, and `Connector::route` for a 500-connector board.
- **Definition of Done (tested):** an increment is `[x]` only when unit + index-equivalence + convergence +
  at least one relevant integration test are green in CI.

---

## 11. PERFORMANCE & RESOURCES

| Operation | Budget | Rationale |
|---|---|---|
| `query_visible` @ N=50 000 | < 1 ms | Must fit inside a 16 ms frame with margin for tessellation (`toile-ui`). |
| Incremental `SpatialIndex::update` (single element) | < 50 µs | A drag mutates one element per input event at 60+ FPS. |
| `apply_remote` for a typical delta (< 32 KB) | < 2 ms | Keeps inter-user latency processing under the ~50 ms client budget. |
| `handle_command` (move/restyle) | < 1 ms | Single register write + one index update + visible-set re-derive. |
| Client memory @ 50 000 elements | proportional to element count, target < 100 MB total doc | Per the mobile-first pillar; R-tree overhead ~small multiple of leaf count. |

**WASM impact:** the Logic-thread footprint here is `rstar` + `kurbo` + `loro`; no GPU or font code (those are
`toile-ui`). Binary-size sensitive additions are avoided.

---

## 12. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Reason |
|---|---|
| **Dependency graph over elements (petgraph)** | Elements are independent; a whiteboard has no recompute cascade. Adds cost and complexity for nothing (unlike Trame). |
| **Persisting connector geometry / routed paths in Loro** | Would fire a CRDT mutation on every element move and create connector merge conflicts. Routing is pure client compute (§5). |
| **Persisting smoothed stroke curves** | Storage blow-up and re-derivation loss; raw sampled points + client-side Catmull-Rom is exact and single-author (§4). |
| **Positional / slot-id element keys** | Invalidated by reorder and free move; minted move-invariant `ElementId` is required (§2.A). |
| **Per-attribute style fan-out (many registers)** | Causes half-applied restyle tears under concurrency (Trame `logic §11 bis.F`). One postcard LWW register per element (§2.E). |
| **Authoritative spatial index (index as source of truth)** | The index must be a rebuildable cache so remote deltas can't desync it; `LoroDoc` is the only source of truth (§1, §3). |
| **Presence (cursors/strokes) as CRDT deltas** | Would bloat history and persist ephemeral data. Presence is an ephemeral Sync payload (§7). |
| **Presence identity / color keyed by `PeerID`** | `PeerID` is per-session/per-device, so one user on two devices would split into two colored cursors and clash with the other apps. Presence keys off the canonical `UserId` (resolved in `ambre-sync`) with the shared cross-app color hash (§7). |
| **Find-and-Replace on the canvas** | Replace across free-form elements is marginal versus its complexity; MVP ships Find-only (jump + highlight). Reconsider post-MVP (§8 quater). |
| **Separate `frames` map + frame-specific lifecycle commands** | Duplicates the whole move/resize/delete/reorder lifecycle and splits z-order across two structures. A frame is an element (§2.F); `layers` is the single z-order (§2.G). |
| **Global shared undo stack** | Would let a peer undo others' actions and break collaboration. Local-only `loro::UndoManager`, aligned with Trame (§8 bis). |
| **Connectors on a separate always-on-top pass** | Prevents placing a connector behind an element. Connectors live in `layers` as `NodeId`s (§2.G). |
| **HTML5 Canvas / SVG DOM engine** | Insufficient performance for 10 000+ objects (core `ambre-context §6`, Toile Todo §7). |

---

## 13. INTEGRATION PLAN (ROADMAP)

> Tracked as Stories of Epic **E4** (Toile MVP) in `specs/ambre-roadmap.md`.
> No phase ships `[x]` without its test belt green.

**Phase 0 - Core standalone**
- [x] `ToileState` + CRDT schema (§2): `elements`, `connectors`, `drawings`, unified `layers` z-order, one per-element `ElementStyle` register, `groups` (per-group `children` list, derived-only bounds), and `frames` content (per-frame `children` membership list, §2.F/§2.G z-order invariant) **landed**. The `media` map is created and populated by `toile-interop` in Phase 1 (raster/media pipeline), not the Logic engine.
- [x] Move-invariant `ElementId` minting (`{peer:016x}_{counter:08x}`, §1)
- [x] `SpatialIndex` (`rstar` R-tree) with the index-equivalence property test (indexed cull vs brute-force agree)
- [x] Element lifecycle commands (create / update / delete / transform / lock / reorder) with test belt
- [x] Group / ungroup (§2.F bis) with bounds-union derivation. **DoD:** green in `ambre-toile/src/convergence_tests.rs` (union bounds + below-members z-order, group move translates members, ungroup keeps children/order, delete cascades).
- [x] Clipboard: copy / paste / duplicate (§8 ter) with test belt. **DoD:** green in
  `ambre-toile/src/convergence_tests.rs` (copy is non-mutating; paste re-mints + offsets; a half-copied
  connector is dropped while a fully-copied one is reproduced; a duplicated group is one undo item; the
  payload postcard round-trips; paste reproduces the selection on a separate board).
- [x] Connector routing (§5) + endpoint binding (create / retarget / delete, dangling detection). Belt: retarget rebind, retarget-to-missing surfaces `DanglingConnector`, endpoint delete drops the connector from the paint set.
- [x] Stroke storage + smoothing (§4)
- [x] `DisplayViewport` derivation (culled visible set). Belt: rotation-expanded AABB, resize bounds, off-screen culling.
- [x] `ToileCommand` / `ToileEvent` consolidated enums (§8): lifecycle + groups + undo/redo (`GroupElements`/`UngroupElements`/`Undo`/`Redo`, `UndoState` event) + clipboard (`CopyElements`/`PasteElements`/`DuplicateElements`, `Clipboard` event, `ClipboardPayload` postcard round-trip) + frames (`SetFrameChildren`) landed in `ambre-toile`. `EditText` (Phase 1, blocked on Prose's shared `TextDelta`) and find land with their later slices. Nested documents need no variant at all (§2.I "Command routing").
- [x] 2-peer CRDT convergence property-test. **DoD:** green in `ambre-toile/src/convergence_tests.rs`. Op-set covers create / move-LWW / resize / rotate / restyle-register / delete / reorder / group / ungroup / connect / stroke; the assertion compares the full derived paint set (elements + routed connectors + group union bounds). Complemented by a `ToileState`-level rebuild-equivalence property-test (incremental caches == full rebuild from snapshot, the §1 invariant beyond the isolated `SpatialIndex`) -- which flushed out and fixed four empty-group / dangling-child-ref defects across group / ungroup / delete / frame membership.

**Phase 1 - Logic Thread Integration**
- [x] Logic-thread dispatch: `handle_command` returning `ToileOutcome` (§8), via typed `DocState` + `match` (`architecture §6`). Landed: the IPC contract (`ToileCommand`/`ToileEvent`/`Display*`/`Viewport`/model value types) moved into `ambre-bus::app::toile` (one module per app, mirrors the new Trame `app/trame/` layout; `ToileOutcome` stays engine-local like `TrameOutcome`), `AppCommand::Toile` / `AppEvent::Toile` variants, `DocState::Toile(Box<ToileState>)`, and the `(DocState::Toile, AppCommand::Toile)` dispatch arm fanning display/update/ephemeral/snapshot out through the shared `dispatch_outcome`.
- [x] Sync thread wiring: `SendUpdate` + `SendEphemeral` presence (both in `ambre-bus`); inbound `LogicEvent::RemoteEphemeral` -> `apply_ephemeral` (§8). `SendUpdate`/`SendEphemeral` fan-out wired; `apply_ephemeral` is an accepted no-op until the presence `ToileEvent` variant lands with `toile-ui §13`.
- [x] `apply_remote` accepting both deltas and snapshots (§8), plus `apply_remote_batch` for the server history replay on open (one import sweep + a single cache rebuild).
- [x] `export_snapshot` + persist-on-change to IndexedDB + the `RequestSnapshot` / `SnapshotNeeded` / `RemoteSnapshot` paths (§8). `RemoteSnapshot` -> `apply_remote`, `SnapshotNeeded` -> `export_snapshot`, persist-on-change via the dispatcher's `to_persist` leg, and `RestoreLocalSnapshot` restores a Toile board from the local IndexedDB store.
- [x] `my_user_id` seeded from `LogicEvent::SessionReady` (§7), keys reactions `toile-features §4.A`.
  `ToileState::set_user_id` / `user_id` (`Option<UserId>`); the `ambre-logic` dispatcher captures the
  `RestEvent::SessionReady` id, seeds every open document and any opened afterwards. **DoD** green in
  `ambre-logic/src/dispatch.rs` (open + future docs seeded) and `ambre-toile/src/convergence_tests.rs`.
- [x] `loro::UndoManager` wired to `Undo` / `Redo`, one `doc.commit()` per mutating command (§8bis). Landed early with the Phase-0 groups slice (`apply_history`, `UndoState` event); **DoD** green in `ambre-toile/src/convergence_tests.rs` (undo reverts create + redo restores; a group undoes as one item). Chrome affordance wiring stays with Phase 1 UI.
- [x] `Find` handler (§8 quater): non-mutating text scan over sticky/text bodies + frame titles, hits in
  `layers` z-order (with the §2.H missing-from-layers fallback), `match_case`/`whole_word` honored, wrapping
  `focus`; `ToileCommand::Find` + `FindOptions`/`FindResults` (`ambre-bus`) + `ToileEvent::FindResults`. Belt in
  `ambre-toile/src/logic/convergence_tests.rs`. Chrome panel wiring stays with Phase 1 UI (`toile-ui §5quater`).
- [x] `RequestSelectionStats` -> selection aggregates (count + union AABB, world space) for the status bar
  (§8 quinquies), computed Logic-side over the derived `bounds` cache so an off-viewport selection is counted.
- [x] Presence keyed by canonical `UserId` (§7): `PresencePayload` / `PresenceUpdate` (`ambre-bus`),
  `ToileCommand::Presence` -> postcard ephemeral frame (`ToileOutcome.ephemeral`, no display / no CRDT delta),
  `apply_ephemeral(from_user_id, payload)` -> `PresenceState` keyed by `UserId` -> `ToileEvent::PeerPresence`;
  `LogicEvent::RemoteEphemeral` now carries the resolved `from_user_id`. **DoD** green in
  `ambre-toile/src/convergence_tests.rs` (a presence frame yields the expected `PeerPresence` keyed by the
  resolved `UserId`, mutating neither the doc nor its history; a malformed frame is dropped). The **render**
  side (cursor / stroke / laser / selection overlay) stays with `toile-ui`. **Blocked downstream:** the
  `ambre-sync` `pub_key` -> `UserId` resolution (`resolve_ephemeral_author`) returns `None` until the member
  signing-key directory is wired, so inbound frames are dropped end-to-end for now (a sync/identity slice
  tracked in `ambre-spec-sync.md` "SYNC P4" and scheduled under `ARCH P6`, not Toile logic); the outbound
  broadcast and the whole Logic path are complete. *(The directory needs no server change: `GET /members`
  already returns each member's Ed25519 `signing_pub_key`.)*
- [ ] Presence render / overlay (remote cursors, live strokes, laser, selection halos, cross-app `UserId` colors): `ambre-spec-toile-ui.md` Phase 1.
- [x] Nested documents (`doc_embed`, §2.I) — the Toile half of `ARCH P6`. The element kind, its
  `{document_id, doc_type}` payload, the codec/derive legs and board-level hit-testing landed with Phase 0's
  element model; focus as view-local state, the pointer transform into child space, and child
  `OpenDocument`/`CloseDocument` with close hysteresis landed in slice 3.c (`ambre-toile/src/ui/embed.rs`); the
  launcher half — `FrameHost` owning the child `AppView`s, the per-document Logic->UI fan-out, embedded-mode
  construction, clipped painting and input routing — landed in slice 3.d (`ambre-app/src/host/`). No bus variant
  is needed: the registry is flat and the child's view addresses the child directly (§2.I "Command routing"). The
  child's `Role` lives in `ambre-logic` (`ambre-spec-architecture.md §2.D, §7 ARCH P6`).

**Phase 1c - Miro-parity engine gaps (MVP)**
- [ ] Connector labels (§5.B): `label` `LoroText` on the connector + `EditConnectorLabel` command; surfaced on
      `DisplayConnector.label`; convergence belt (concurrent captions merge char-wise)
- [ ] Hyperlinks / go-to-frame (§2.K): `link` element register + `LinkTarget` + `SetLink` command; `linked`
      flag on `DisplayElement` (navigation itself is `toile-ui`)
- [ ] Arrange (§8): `ArrangeElements { ids, op }` align/distribute, one convergent batch + one undo item
- [ ] Comment anchoring (§2.J): `DisplayComment` pins in `DisplayViewport.comments` (derive + skip on deleted
      anchor). Thread CRDT + commands are owned by `toile-features §4.D`

**Phase 2 - Optimization**
- [ ] Bulk index update on large remote batches
- [ ] Order-preserving layer diffing
- [ ] Connector routing memoization keyed by endpoint bounds
- [ ] Benchmark-gated style dedup dictionary

**Deferred - post-MVP element model**
- [ ] `card` element type + board-level `tags` palette (§2.C) — agile-team workflow layer
