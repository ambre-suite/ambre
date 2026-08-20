# AMBRE -- SPEC: PROSE / UI
**Type:** Vertical (Feature/Widget)
**Status:** Draft
**Implementation:** design-only

> Prose presentation layer: wgpu/Parley **content** rendering (DrawSurface) of the page
> column, caret, selections, collaborative cursors, and inline comment/track-change
> highlights; page-based virtual scroll; input via the shared transparent `<textarea>`
> (keyboard/IME/clipboard). The application **chrome** (contextual toolbar, slash menu,
> ruler, comment side panel, accept/reject) is **DOM/Leptos** (`ambre-chrome`), not canvas.
> No CRDT mutation is performed here -- every mutation is a `ProseCommand` to the Logic thread.

> **Architecture note.** The client uses the **two-instance model** -- a **Main** WASM
> instance (UI thread + Logic thread sharing one linear memory via `wasm_thread`, typed
> `flume` values) and a **separate Sync worker** -- see `ambre-spec-architecture.md section 2.B`.
> For this spec:
> - **Typed `flume` transport, no `postMessage`/postcard on the UI <-> Logic path.** The UI sends `LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }` and receives `UiEvent::Viewport { event: AppEvent::Prose(ProseEvent) }` -- Rust values **by move** on `flume`, no serialization on the UI side. (`postMessage`/postcard is only the Main <-> Sync hop, invisible to this layer.)
> - **The Prose view is an `AppView` variant.** Like `TrameView`, the Prose presentation exposes the inherent driver methods (`update`/`plan_repaint`/`paint_content`/`paint_overlay`/`resize`/`handle_input`) that the `FrameHost` `match`es over -- adding Prose is a new `AppView::Prose(ProseView)` arm in `ambre-app` (see `ambre-spec-canvas.md section A`). The GPU pipeline (`QuadRenderer`, `WgpuSurface`) lives in `ambre-canvas`; the `FrameHost` driver lives in `ambre-app`.
> - **Canvas / DOM boundary (`ambre-spec-ui.md section 2.A`).** The canvas owns **content** (page column, caret, selections, content hit-testing, remote cursors, inline highlights). The DOM (Leptos, `ambre-chrome` + a specific `prose_chrome`) owns **all chrome** (contextual toolbar, slash menu, ruler, comment panel, dialogs, menus). Input is captured by DOM listeners and the transparent `<textarea>` -- **winit is not used** (`ambre-spec-ui.md section 6`).
> - Memory is **shared** across UI <-> Logic (single heap): the `ProseCommand` is **moved**, not copied.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles (**canvas content** unless noted):
- wgpu/Parley rendering (DrawSurface) of visible pages in the viewport (display list).
- Virtual scroll: culling of pages outside the viewport, smooth scrolling management.
- Text caret, selection (mouse, keyboard, touch); keyboard/IME input via the shared transparent `<textarea>` (`ambre-chrome`).
- Remote collaborative cursors; inline comment and track-change highlights.
- **DOM chrome** (Leptos, `ambre-chrome`/`prose_chrome`): contextual toolbar, slash menu, interactive ruler, comment side panel, accept/reject affordances.
- Clipboard: the host-driven, Logic-authoritative `ClipboardBlob` flow (`clipboard_write`/`poll_clipboard`/`paste_clipboard`, `ambre-spec-ui.md section 2.E`).

This component delegates:
- All CRDT mutations -> Logic thread via `flume`: `LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }`.
- Pagination calculation (`PageLayout`) -> `prose-engine` (Logic thread).
- Effective style resolution -> `prose-features` (Logic thread).
- Import/export conversions -> `prose-interop` (Logic thread).

**Dependencies:**

| Crate / API | Role |
|-------------|------|
| `wgpu` + `QuadRenderer` | GPU rendering: vertex/fragment shaders, WebGL2 default |
| `lyon` | Path tessellation (shadows, highlights, wavy underlines) |
| `parley` | Typographic layout: line breaking, shaping, glyph metrics |
| `swash` | Glyph rasterization -> GlyphAtlas GPU |
| `flume` | Inter-thread channels: send `LogicCommand`, receive `UiEvent` (typed values by move) |
| `ambre-bus` | Typed IPC types: `LogicCommand`/`UiEvent`, `ProseCommand`/`ProseEvent`, `AppCommand`/`AppEvent`, `ClipboardBlob` |
| `ambre-canvas` | GPU pipeline (`QuadRenderer`, `WgpuSurface`), GPU bootstrap + rAF loop, per-frame value types (`RepaintPlan`, `InputEvent`). The `FrameHost` driver + `AppView` enum live in `ambre-app` |
| `ambre-draw` | `DrawSurface` trait (parameterized screen/PDF rendering) |
| `ambre-chrome` + `leptos` | DOM chrome toolkit: the transparent `<textarea>` (keyboard/IME/clipboard pipe), and the base components the specific `prose_chrome` (toolbar, slash menu, ruler, comment panel) builds on |
| `ambre-commands` | Command registry / keybindings shared with the chrome (`CommandId` -> `LogicCommand`) |

Input events reach the view as `InputEvent` (from `ambre-canvas`) via `AppView::handle_input`;
DOM listeners and the transparent `<textarea>` are wired by `ambre-app`/`ambre-chrome`.
**No `winit`** (`ambre-spec-ui.md section 6`). `navigator.clipboard` is driven by the **host**, not
by this crate (`ambre-spec-ui.md section 2.E`).

All `web_sys::*` resources (`HTMLCanvasElement`, `GPUDevice`, `GPUQueue`) are `!Send`
and confined to the UI thread. They never travel through a `flume` channel.

**Invariants:** prefix `PROSE-UI`. Grammar and status values are normative in `ambre-spec-architecture.md section 4`; the roll-up is `ambre-spec-testing.md section 7`. Every row is `unimplemented` -- `crates/ambre-prose` is empty -- and lands with `ARCH P2` (Epic `E5`).

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-001` | The UI thread MUST hold no document data: no `LoroDoc`, no `ProseState`, only view state and what arrives as `ProseEvent`. | section 1; section 6 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-002` | Every mutation MUST leave the UI thread as a `ProseCommand`; there MUST NOT be a second model of the document. | section 1; section 6 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-003` | Sends to the Logic thread MUST be non-blocking (`try_send`): the UI thread MUST NOT wait on it. | section 1 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-004` | Keystrokes MUST be displayed optimistically from a local draft buffer, never by mutating shared state on the UI side. | section 1; section 2.C | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-005` | The draft buffer MUST be discarded when the confirming `ProseEvent` arrives, and MUST NOT survive a divergence between what was typed and what the document accepted. | section 2.C | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-006` | A full Parley layout or a TOC recompute MUST NOT run in the UI thread: both arrive pre-computed as `PageLayout`. | section 1; section 2.B | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-007` | Only visible pages plus one on each side MUST be built into vertex batches; the full document MUST NOT be walked per frame. | section 2.B | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-008` | Hit-testing MUST be the exact inverse of the drawn layout, under zoom and scroll. | section 2.E-bis | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-009` | A click that lands in no glyph MUST resolve to the nearest valid caret position, never to no position. | section 2.E-bis | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-010` | Scroll position and selection MUST stay local: neither MUST enter the document. | section 6 | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-011` | A remote cursor MUST come from an encrypted ephemeral and MUST be keyed by the Sync-resolved `UserId`. | section 2.I; section 6 | not built -- lands with `ARCH P2` (Epic `E5`); needs the `UserId` directory of `SYNC P4` | unimplemented |
| `PROSE-UI-012` | The contextual toolbar, the slash menu and the ruler MUST be DOM chrome, not canvas overlays. | section 2.F; section 2.G; section 2.H | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-013` | A paste MUST be routed through the Logic thread as a command: the UI MUST NOT insert clipboard content into its own model. | section 2.K | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-014` | Every colour drawn MUST come from the theme palette; no literal colour in render code. | section 2.D; `ambre-spec-ui.md` | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-UI-015` | `ambre-prose`'s UI module MUST contain no `unsafe`. | README pillar "Security" | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |

> **Invariant coverage.** 15 `unimplemented`. The 16 ms frame budget of section 1 is deliberately **not** a row: a number with no `criterion` bench behind it cannot be falsified, and turning section 5's budgets into invariants is its own pass (`ambre-spec-testing.md section 7`, seeding order).
---

## 2. TECHNICAL DESIGN & STATE

### A. UI Thread State (`ProseUiState`)

The UI thread maintains minimal state, derived from messages received from the Logic thread.
It never duplicates CRDT content.

```rust
// ambre-prose/src/ui/state.rs

pub struct ProseUiState {
    // Received from the Logic thread via UiEvent::Viewport { event: AppEvent::Prose(..) }
    layout:           PageLayout,
    /// Page-column geometry (page count, page/gutter heights, total height), rebuilt from
    /// `layout` on each `LayoutUpdated`. Owns `visible_pages`/`page_top_y`/`doc_y_to_page`.
    page_column:      PageColumn,
    /// Version of the last applied layout -- older layouts are dropped (anti-flickering).
    /// Aligned with the `version` field of `ProseEvent::LayoutUpdated`.
    layout_version:   u64,
    run_cache:        RunCache,           // ParleyRunData -> ParleyRun, LRU 500 blocks
    remote_cursors:   Vec<RemoteCursor>,
    toc:              TableOfContents,
    comments:         Vec<UiComment>,       // visible comments (resolved positions)
    suggestions:      Vec<UiSuggestion>,    // track changes suggestions (resolved positions)
    spell_errors:     HashMap<TreeID, Vec<SpellError>>, // post-MVP

    // Local UI state (not sent to the Logic thread except on mutation)
    viewport:         Viewport,
    cursor:           Option<TextCursor>,
    selection:        Option<TextSelection>,
    /// Drag-selection anchor captured on PointerDown, immune to remote deltas for the drag
    /// duration (P-120). `None` when no drag is in progress.
    drag_anchor:      Option<(TreeID, u32)>,
    /// Armed autoscroll velocity while a drag pointer sits past the viewport edge (P-121).
    /// `None` when the drag is inside the viewport. Integrated by `tick_autoscroll` each frame.
    autoscroll_v:     Option<f32>,
    draft_buffer:     DraftBuffer,          // optimistic input (see 2.C)
    toolbar:          ToolbarState,
    slash_menu:       Option<SlashMenuState>,
    ruler:            RulerState,
    comment_panel:    CommentPanelState,
    scroll_velocity:  f32,                  // for inertial scrolling
}

#[derive(Debug, Clone)]
pub struct Viewport {
    pub scroll_y:     f32,    // vertical offset in px from the top of the document
    pub width_px:     f32,
    pub height_px:    f32,
    pub device_pixel_ratio: f32,
    /// Horizontal offset (px) of the page column's left edge inside the viewport.
    /// The page column is centered, so `page_margin_left_px = ((width_px / zoom) - page_width_px).max(0.0) / 2.0`,
    /// recomputed on `resize`/zoom. Used by `build_display_list` as the page origin X.
    pub page_margin_left_px: f32,
    /// User zoom factor (1.0 = 100%). Independent of `device_pixel_ratio` (which is the
    /// physical/CSS-pixel ratio the browser imposes). Doc<->screen conversions multiply by
    /// `zoom * device_pixel_ratio`. Clamped to `[ZOOM_MIN, ZOOM_MAX]` (0.25 .. 4.0). See the
    /// Interaction model (2.E) for the Ctrl+=/-/0 and pinch handlers.
    pub zoom:         f32,
}

#[derive(Debug, Clone)]
pub struct TextCursor {
    pub block_id: TreeID,
    pub offset:   u32,
    pub affinity: CursorAffinity,  // Before | After (for line breaks)
    pub x_goal:   Option<f32>,     // memorized X position for up/down navigation
}

#[derive(Debug, Clone)]
pub struct TextSelection {
    pub anchor: (TreeID, u32),   // selection start (stable)
    pub focus:  (TreeID, u32),   // selection end (mobile)
}

#[derive(Debug, Clone, Copy)]
pub enum CursorAffinity { Before, After }
```

---

### B. Virtual Scroll & Culling

The document is displayed as a **column of pages** with gutters between them.
Virtual scroll culls pages outside the viewport to never rasterize more than
3 pages simultaneously (previous page + current page + next page).

```rust
// ambre-prose/src/ui/scroll.rs

pub struct PageColumn {
    pub page_height_px:   f32,
    pub page_width_px:    f32,      // page trim width (used to center the column, P-135 zoom)
    pub gutter_px:        f32,      // space between pages (default: 16px)
    pub page_count:       u32,      // number of pages in the current PageLayout (0 for an empty layout)
    pub total_height_px:  f32,      // page_count * (page_height + gutter)
}

impl PageColumn {
    /// Number of pages in the layout. A freshly created / empty document still has one
    /// paginated page (the engine never emits a zero-page layout for a live doc), but a
    /// layout can transiently be empty before the first `LayoutUpdated`; both cases are
    /// handled by `visible_pages` returning an empty range.
    pub fn page_count(&self) -> u32 { self.page_count }

    /// Returns the page indices intersecting the viewport, in document pixels.
    /// Robust to an empty layout (`page_count == 0`) and to an out-of-range `scroll_y`
    /// (P-43 underflow, P-134 clamp): the returned range is always empty or in `[0, last]`
    /// with `first <= last`, never inverted.
    pub fn visible_pages(&self, viewport: &Viewport) -> RangeInclusive<u32> { /* ... */ }

    /// Absolute Y coordinate (from the document top) of the top of page i.
    pub fn page_top_y(&self, page_index: u32) -> f32 { /* ... */ }

    /// Inverse of `page_top_y`: maps a document Y (any value, including a gutter or a margin)
    /// to the page it belongs to and the Y within that page. A Y in the gutter between page i
    /// and i+1 is assigned to the nearer page edge (clamped into `[0, page_height_px]`), so a
    /// pointer never lands "between" pages with no page (P-148). Returns `None` only for an
    /// empty layout.
    pub fn doc_y_to_page(&self, doc_y: f32) -> Option<(u32, f32)> { /* ... */ }
}
```

**Block culling strategy:**

Only blocks whose `BlockLayout::page_start` is in `visible_pages()` are included
in the DrawSurface display list. Blocks on non-visible pages are neither shaped
nor rasterized -- their `ParleyRun`s are cached but not sent to `wgpu`.

**Parley cache (`RunCache`):**

```rust
// ambre-prose/src/ui/run_cache.rs

/// Per-block Parley run cache. Runs are pre-shaped in the Logic thread
/// (type `ParleyRunData`, moved by value on `flume` -- no serialization) and
/// converted to `ParleyRun` (UI type, with Parley layout cache) on receipt.
pub struct RunCache {
    inner: HashMap<TreeID, CachedRuns>,
    /// Maximum capacity: 500 blocks (~50 pages). LRU eviction beyond.
    lru:   LruCache<TreeID, ()>,
}

pub struct CachedRuns {
    pub runs:      Vec<ParleyRun>,
    pub height_px: f32,
    pub dirty:     bool,  // true if a CRDT delta touched this block since the last shape
}

/// UI-side Parley run type. Converted from `ParleyRunData` (typed, moved by value)
/// on receipt of `ProseEvent::RunsUpdated`.
///
/// `ParleyRunData` is **already shaped** in the Logic thread (glyph ids, per-glyph advances,
/// cluster<->byte map, line boxes, asc/ descent). The UI thread therefore never runs Parley
/// line-breaking or shaping on the render thread (P-38, and the "no shaping on the UI thread"
/// pillar, section 7). What the UI builds lazily is only the GPU draw state -- the glyph
/// atlas entries and the positioned quad buffer -- keyed by the run; that is a swash raster +
/// upload, not a Parley layout pass.
pub struct ParleyRun {
    pub data:  ParleyRunData,   // pre-shaped run data (glyph ids, advances, cluster map, metrics)
    /// GPU draw state for this run: atlas glyph handles + a positioned quad buffer, built on
    /// first paint from the pre-shaped `data` via `TextCache::get_or_build` and reused while
    /// the block is not dirty. This is rasterization/upload, not shaping. `None` until first
    /// painted; a culled block never builds it (see hit_test fallback, P-137).
    pub draw:  Option<RunDrawState>,
}

impl From<ParleyRunData> for ParleyRun {
    fn from(data: ParleyRunData) -> Self { /* ... */ }
}

impl ParleyRun {
    /// Hit-test / caret geometry works off the **pre-shaped** `data` (cluster map + advances),
    /// so it is available even when `draw` is `None` (block never painted or culled). This is
    /// what lets pointer hit-testing on an off-screen block succeed without a Parley layout
    /// pass (P-137). See `hit_test` in the Interaction model.
    pub fn hit_test_x(&self, x_in_run: f32) -> u32 { self.data.offset_for_x(x_in_run) }
    pub fn x_for_offset(&self, offset: u32) -> f32  { self.data.x_for_offset(offset) }
}
```

> **Note on the two types:** `ParleyRunData` (defined in `prose-engine`) is the **typed** DTO
> that travels from the Logic thread to the UI thread, by move on `flume`, inside
> `ProseEvent::RunsUpdated` -> `AppEvent::Prose` (shared memory, no serialization).
> `ParleyRun` (defined in `prose-ui`) wraps `ParleyRunData` and adds an optional
> `RunDrawState` (glyph atlas handles + positioned quad buffer) -- built lazily on the first
> `paint_content()` via `TextCache::get_or_build`, reused as long as the block is not dirty.
> That lazy build is a swash raster + GPU upload, **not** a Parley shaping/line-break pass:
> the UI thread never shapes on the render thread (P-38, section 7). Caret and hit-test
> geometry read the pre-shaped `data` directly, so they work before `draw` is built and on
> culled blocks (P-137).

---

### C. Draft Buffer (Optimistic Display)

The UI thread -> Logic thread -> UI thread round-trip latency is typically 1-3 ms,
but perceptible on fast typing. The **draft buffer** displays keystrokes immediately
without waiting for CRDT confirmation.

```rust
// ambre-prose/src/ui/draft.rs

pub struct DraftBuffer {
    /// Text typed since the last confirmed ProseEvent::DocUpdated.
    /// Displayed overlaid on the confirmed Parley run.
    pub pending_text: String,
    pub block_id:     Option<TreeID>,
    pub insert_pos:   u32,
    /// True while an IME composition is in progress (rendered with the composition style,
    /// dotted underline). `pending_text` then holds the intermediate composed text.
    pub composing:    bool,
}

impl DraftBuffer {
    /// True when there is nothing to display / confirm.
    pub fn is_empty(&self) -> bool { self.block_id.is_none() && self.pending_text.is_empty() }

    /// Append committed text at (block_id, pos). Re-anchors (clearing first) only when the
    /// draft moves to a different block; consecutive keystrokes in the same block accumulate.
    /// Used by handle_text_input for non-composition input.
    pub fn push_str(&mut self, text: &str, block_id: &TreeID, pos: u32) { /* ... */ }

    /// Start an IME composition at (block_id, pos). Clears any prior draft.
    pub fn begin_composition(&mut self, block_id: TreeID, pos: u32) { /* ... */ }

    /// Replace the composed preview text (compositionupdate). Canvas preview only, no command.
    pub fn set_composition(&mut self, text: &str) { /* ... */ }

    /// End composition; returns the anchor (block_id, pos) at which InsertText must be emitted.
    /// The anchor is the *current* `(block_id, insert_pos)`, which reanchor_after_remote_delta
    /// may have moved since begin_composition (P-131), so compositionend never emits at a stale
    /// begin position. Falls back to insert_pos 0 of the anchored block if unset.
    pub fn end_composition(&mut self) -> (TreeID, u32) { /* ... */ }

    /// Called on receipt of a *confirming* ProseEvent::DocUpdated (see the clearing rule below).
    /// The Logic thread confirmed: the draft is integrated into the official runs.
    pub fn clear(&mut self) { /* ... */ }

    /// Reanchor the draft after a remote delta arrived mid-typing/mid-composition (P-07, P-131).
    /// `shift` is the net offset change the delta applied at or before `insert_pos` in this
    /// block (supplied by the state layer from the delta). Keeps the pending text and, for a
    /// composition, keeps `composing` true so compositionend still emits at the corrected pos.
    /// If the delta deleted the anchored block, the draft is dropped (returns false).
    pub fn reanchor(&mut self, block_still_alive: bool, shift: i64) -> bool { /* ... */ }
}
```

**Clearing rule (P-07).** The draft is cleared **only** by a `DocUpdated` that actually
confirms this UI's own pending input for the anchored block. An empty-delta `DocUpdated`
(e.g. `on_media_downloaded` emits `DocUpdated { delta: vec![] }`) and a `DocUpdated` whose
delta touches only *other* blocks (foreign origin) must **not** clear the draft -- otherwise an
image finishing download would wipe in-progress keystrokes. On receipt, `update()` applies:

On `ProseEvent::DocUpdated { delta, .. }`, `ProseUiState::update()` branches on the delta:

- **Empty delta** (media download, presence, any no-content event) -- the draft is
  kept untouched.
- **Delta that confirms the anchored block** (this UI's own edit came back) -- the
  pending text is now in the official runs, so the draft buffer is cleared.
- **Any other (foreign/remote) delta** -- the still-pending draft is *reanchored*
  over the applied shift (P-131) rather than cleared: `effect_on_block` reports
  whether the anchored block is still alive and by how much it shifted. Only if the
  block was deleted does the draft drop and the cursor collapse to the confirmed
  CRDT position.

The draft buffer is rendered via `DrawSurface::draw_glyphs` as a temporary run with the
current cursor style (or the composition style while `composing`), positioned after the
block's last confirmed run.

---

### D. DrawSurface Rendering -- Display List

On each `requestAnimationFrame` (if `dirty`), the display list is built from
`ProseUiState` and submitted to `QuadRenderer` for GPU flush.

Block rendering functions (`render_block_on`, `render_selection_on`, etc.) are
parameterized on the `DrawSurface` trait defined in `ambre-draw`. The UI thread
instantiates `WgpuSurface` -- the same code is reused by `prose-interop` with
`KrillaSurface` for PDF export in the Logic thread.

This is the body of the Prose `AppView`'s `paint_content` driver method. It adopts the
**Foundation driver signature** (`ambre-spec-canvas.md section A`): the surface plus the
`FontContext`/`TextCache`/`strip` the glyph cache and partial repaint need (P-39). `TextCache`
supplies each run's `RunDrawState` on first paint (raster + upload, not shaping, P-38); `strip`
is the dirty-rectangle set from `plan_repaint`, so only touched pages are re-emitted. All
`render_*_on` helpers take `&mut S` (not `&mut &mut S`): `build_display_list` holds `surface:
&mut S` and forwards it by reborrow as `surface`, never `surface` (P-22).

```rust
// ambre-prose/src/ui/renderer.rs

/// Called by FrameHost as AppView::Prose(view).paint_content(surface, fc, tc, strip).
pub fn build_display_list<S: DrawSurface>(
    state:   &ProseUiState,
    surface: &mut S,
    fc:      &mut FontContext,
    tc:      &mut TextCache,
    strip:   &RepaintStrip,      // dirty pages/rects from plan_repaint (partial repaint)
) { /* ... */ }
```

> **`RepaintStrip` honors the shared Content-layer contract (X49).** It is a **specialization** of
> the Foundation `ContentRepaint { Full | Scroll | Reuse }` + `content_strip` contract
> (`ambre-spec-canvas.md`), not a parallel mechanism: Prose maps its dirty pages / rects onto
> `Full` and `Reuse` and simply does not use the GPU scroll-band `Scroll { dx, dy }` mode (page
> pagination reflows rather than scroll-blits a band). Trame and Toile use the same contract, `Scroll`
> included. Tracked in the cross-app factoring Story (`ambre-roadmap.md`).

#### UI Color Palette

| Element | Color | Opacity |
|---------|---------|---------|
| Text cursor | `#1A73E8` (blue) | 100% |
| Local selection | `#1A73E8` | 20% |
| Comment highlight | `#FBBC04` (yellow) | 40% |
| Track changes insert | `#34A853` (green) | 30% |
| Track changes delete | `#EA4335` (red) | 30% -- strikethrough text |
| Remote cursor peer A | Color assigned to peer | 80% (bar) + 15% (selection) |
| Spell error | `#EA4335` (red) | wavy underline |

---

### E. Input Event Handling

#### Keyboard

Keyboard input reaches the view as `InputEvent` through `AppView::handle_input` (there is
**no `winit`** -- `ambre-spec-ui.md section 6`). The physical source is the shared transparent
`<textarea>` overlay (`ambre-spec-chrome.md section 2.C`): it owns editing focus (a click in the
canvas calls `textarea.focus()`), and DOM `keydown`/`beforeinput` are normalized by
`ambre-app`/`ambre-chrome` into `InputEvent` before dispatch. `handle_input` returns `true`
when the host must `preventDefault()`.

```rust
// ambre-prose/src/ui/input/keyboard.rs

/// Dispatched by FrameHost via AppView::Prose(view).handle_input(ev).
/// Returns true if the event was consumed (host calls preventDefault()).
pub fn handle_key(
    ev: &InputEvent,                          // from ambre-canvas (normalized DOM event)
    state: &mut ProseUiState,
    to_logic: &flume::Sender<LogicCommand>,   // channel to the Logic thread (try_send)
) -> bool { /* ... */ }
```

Printable Latin text is **not** matched key-by-key here: it flows through the textarea's
`beforeinput` event, which `ambre-app` turns into `InputEvent::TextInput { text }`. The view
then pushes it to the draft buffer and emits `ProseCommand::InsertText`, and the host clears
the textarea (`ambre-spec-chrome.md section 2.C`).

```rust
// InputEvent::TextInput { text } branch (committed, non-composition input)
pub fn handle_text_input(text: &str, state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) { /* ... */ }
```

#### Selection-aware editing

Every content-mutating edit (typing, Backspace, Delete, Enter, rich paste) first deletes
the active selection, so the two most common editing operations -- type-to-replace and
delete-selection -- have a single, shared path. The primitive is `take_selection`: it
normalizes the current selection to document order, emits one `ProseCommand::DeleteRange`
(the cross-block delete + block-merge primitive, `prose-logic section C`), collapses the caret to
the range start, and clears `state.selection`. It is a no-op (returns `false`) when there is
no selection, so the caret-only paths below are unchanged when nothing is selected.

```rust
// ambre-prose/src/ui/input/edit.rs

/// If a selection is active, delete it (DeleteRange) and collapse the caret to its start.
/// Returns true iff a selection was consumed. Callers that then insert/split act at the
/// collapsed caret. No selection -> no command, returns false.
fn take_selection(state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) -> bool { /* ... */ }

/// Backspace: delete the selection if any, else the grapheme before the caret. At offset 0
/// with no selection, merge with the previous block via a zero-width cross-block DeleteRange
/// (the block-merge case of prose-logic section C).
fn delete_backward(state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) { /* ... */ }

/// Delete (forward): selection if any, else the grapheme after the caret; at block end,
/// pull the next block up (merge) via a zero-width cross-block DeleteRange.
fn delete_forward(state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) { /* ... */ }

/// Enter: delete the selection first, then split the (now collapsed) block at the caret.
fn split_block(state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) { /* ... */ }
```

Ordering on the `flume` channel is preserved, so the `DeleteRange` always applies before the
following `InsertText`/`SplitBlock`; grouping both under one undo step is Logic's concern
(undo granularity, `prose-features section I`).

#### IME (Input Method Editor)

IME (Japanese, Korean, Chinese, Vietnamese...) is **not owned by `prose-ui`**: it is handled by
the shared transparent `<textarea>` pipe in `ambre-chrome` (`ambre-spec-chrome.md section 2.C`). The
textarea captures `compositionstart -> compositionupdate* -> compositionend`; `ambre-app`
forwards them to the view as `InputEvent::Composition { phase, text }`. During composition the
view renders the intermediate text in the draft buffer with a **composition style** (dotted
underline, distinct from mark underlines) so the caret and preview appear on the canvas while
the textarea stays invisible; the IME popup is positioned by the textarea (`transform:
translate` under the caret). `ProseCommand::InsertText` is emitted only on `compositionend`.

```rust
// ambre-prose/src/ui/input/ime.rs -- pure view-side composition state
// (no web_sys here: composition DOM events are captured by the ambre-chrome textarea
//  and delivered as InputEvent::Composition).
pub fn handle_composition(
    phase: CompositionPhase,   // Start | Update | End (from InputEvent::Composition)
    text: &str,
    state: &mut ProseUiState,
    to_logic: &flume::Sender<LogicCommand>,
) { /* ... */ }
```

#### Mouse & Touch

Pointer input arrives as **`InputEvent` variants** (from `ambre-canvas`, `ambre-spec-canvas.md
section A`) -- there is no separate `PointerEvent` type on this path (P-136). The DOM `pointerdown`/
`pointermove`/`pointerup` listeners in `ambre-app` normalize to
`InputEvent::PointerDown/PointerMove/PointerUp { x, y, buttons, click_count, ctrl, shift }`,
where `x`/`y` are **CSS pixels relative to the canvas top-left** (not doc coordinates) and
`click_count` disambiguates single/double/triple click. `handle_pointer` returns `true` when
the host must `preventDefault()`. All coordinate math and hit-testing live in the Interaction
model below; this handler only routes.

```rust
// ambre-prose/src/ui/input/pointer.rs

/// Dispatched by FrameHost via AppView::Prose(view).handle_input(ev) for the pointer variants.
pub fn handle_pointer(
    ev: &InputEvent,
    state: &mut ProseUiState,
    to_logic: &flume::Sender<LogicCommand>,
) -> bool { /* ... */ }
```

---

### E-bis. Interaction Model (coordinates, hit-test, cursor, drag, zoom)

This subsection defines, **once**, every handler the keyboard/pointer/IME dispatchers above
reference. All caret positions use the **canonical unit of `prose-logic section C`: a UTF-8 byte
offset within the block `LoroText`** (P-130); grapheme boundaries are computed on that byte
string. Doc coordinates are document pixels (pre-zoom, pre-scroll); screen coordinates are CSS
pixels relative to the canvas.

**Coordinate conversion (`screen_to_doc`, P-135 zoom, P-148 page step).**

```rust
// ambre-prose/src/ui/input/coords.rs

/// Screen (canvas CSS px) -> document position. Inverts the exact transform build_display_list
/// applies: subtract the centered page-column left margin, divide by zoom, add scroll_y. Then
/// resolve which page the doc-Y lands on and the Y within that page (P-148); a gutter Y snaps to
/// the nearer page edge via PageColumn::doc_y_to_page. Returns the page-local position so hit_test
/// can index BlockLayout without re-deriving the page.
pub fn screen_to_doc(x: f32, y: f32, vp: &Viewport, col: &PageColumn) -> DocPos { /* ... */ }

pub enum DocPos {
    OnPage { page: u32, x_in_page: f32, y_in_page: f32 },
    Empty,
}
```

**Hit-test (`hit_test`).** Maps a `DocPos` to `(TreeID, byte_offset)`.

```rust
// ambre-prose/src/ui/input/hit_test.rs

/// Content hit-test. Robust to every degenerate case the layout can present:
///  - Empty layout / zero-page doc (DocPos::Empty): returns None; the caller (PointerDown on an
///    empty doc) then asks Logic to create the first block (P-147) rather than positioning a caret.
///  - No BlockLayout under y_in_page (click in a page margin, above the first / below the last
///    block): fall back to the nearest block edge on that page, then to the page's first/last
///    block (P-137, P-119) -- a click never returns "nothing" on a non-empty page.
///  - Leaf blocks with no text (Image, HRule, PageBreak) and inter-cell gaps: snap to the block's
///    boundary offset (0 before, len after) by the pointer side (P-138).
///  - Header / footer / footnote bands: hit-test their pre-shaped runs too, so a caret can land in
///    editable header/footer/footnote text (P-139).
/// The horizontal offset within a run comes from the pre-shaped data (ParleyRun::hit_test_x),
/// which works even when the block was culled and never painted (draw == None, P-137).
pub fn hit_test(pos: DocPos, state: &ProseUiState) -> Option<(TreeID, u32)> { /* ... */ }
```

**Cursor movement (`move_cursor`, `advance_cursor`).** `move_cursor` owns the `x_goal`
lifecycle (P-124): vertical moves (`Up`/`Down`/`PageUp`/`PageDown`) set `x_goal` on first use and
reuse it across a run of vertical moves; any horizontal or word move, click, or edit clears it.

```rust
// ambre-prose/src/ui/input/cursor.rs

pub enum Direction {
    Left, Right, WordLeft, WordRight,          // horizontal (P-123 for the Word* pair)
    Up, Down, PageUp, PageDown,                // vertical, x_goal-driven (P-124/P-125)
    LineStart, LineEnd, DocStart, DocEnd,      // Home/End, Ctrl+Home/End (P-126/P-142)
}

/// Move (or, with `extend`, grow the selection to) the new caret. When `extend` is false the
/// selection is collapsed to the caret (P-122). Vertical moves cross wrapped lines, page breaks,
/// and block boundaries transparently by walking BlockLayout line boxes (P-125). Home/End honor
/// the *wrapped* line under the caret and the caret affinity at a soft-wrap point (P-126).
pub fn move_cursor(state: &mut ProseUiState, dir: Direction, extend: bool) { /* ... */ }

/// Common tail: place the caret, keep/clear x_goal, either extend or collapse the selection,
/// then REVEAL the caret (P-149) -- if the new caret line box falls outside the viewport,
/// `scroll_y` is nudged the minimum amount to bring it inside (with a small margin), clamped
/// to `[0, max_scroll]` (P-134). Every keyboard move funnels through here, so caret navigation
/// can never leave the caret off-screen. Never mutates the CRDT.
fn apply_caret(state: &mut ProseUiState, next: TextCursor, x_goal: Option<f32>, extend: bool) { /* ... */ }

/// After a local InsertText/composition commit, advance the caret by `n` bytes in the same block.
/// Clears x_goal (a horizontal edit invalidates the memorized column, P-124) and reveals the
/// caret through the same P-149 rule, so typing at the bottom edge scrolls the new line into view.
pub fn advance_cursor(state: &mut ProseUiState, n: u32) { /* ... */ }
```

**Drag selection & autoscroll (`update_drag_selection`, autoscroll, P-120/P-121/P-144).**

```rust
// ambre-prose/src/ui/input/drag.rs

/// Extend the active drag. The anchor is `state.drag_anchor` captured on PointerDown, immune to
/// remote deltas (P-120). If the pointer is above/below the viewport, arm autoscroll so the
/// selection keeps growing while the document scrolls (P-121). The resulting selection may be
/// backwards (focus before anchor); selection_bounds/render walk it in normalized order and clamp
/// culled pages to the viewport edges (P-144).
pub fn update_drag_selection(state: &mut ProseUiState, x: f32, y: f32) { /* ... */ }

/// Called every frame while a drag autoscroll is armed: advance scroll_y by the armed velocity,
/// clamp to [0, max_scroll] (P-134), then re-run the hit-test at the last pointer Y so the
/// selection focus tracks the newly revealed text.
pub fn tick_autoscroll(state: &mut ProseUiState) { /* velocity-integrate + clamp + re-extend */ }
```

**Zoom (`apply_zoom`, P-135).** Ctrl+`=`/`-`/`0` and pinch adjust `Viewport::zoom`, clamped to
`[ZOOM_MIN, ZOOM_MAX]`; `page_margin_left_px` is recomputed so the column stays centered, and the
zoom is anchored on the caret (or pinch focus) so that point stays put on screen. Zoom never
mutates the CRDT.

```rust
// ambre-prose/src/ui/input/zoom.rs

pub fn apply_zoom(state: &mut ProseUiState, factor: f32, focus_screen: Vec2) { /* ... */ }
```

**Selection & transient-UI helpers referenced above.**
- `select_all` (P-142) sets the selection anchor to the first block offset 0 and the focus to the
  last block's end (`doc_edge(true)`), then shows the toolbar. It is bound to **`Ctrl/Cmd+A`** in
  `handle_key` (X70); the handler is otherwise unreachable by keyboard, and Trame binds the same
  chord (`ambre-spec-trame-ui.md sec.9`). The event is consumed (`preventDefault`).
- `clear_transient_ui` (Escape, P-142) drops `selection`, closes `slash_menu`, hides the toolbar,
  and cancels any in-flight drag/autoscroll; it never mutates the CRDT.
- `select_word_at` / `select_paragraph_at` resolve word / block boundaries off the pre-shaped run
  data at the hit-tested offset.

**Post-mutation and collaboration rules.**
- **Undo/redo caret (P-145).** `ProseEvent::DocUpdated` carrying an undo/redo delta repositions
  the caret to the delta's primary edit point (`reposition_cursor_to_confirmed`) rather than
  leaving it at a now-invalid offset; a selection is collapsed to that point.
- **Remote-cursor jump rule (P-146).** A remote cursor whose new position is **non-adjacent** to
  its previous one skips the 100ms interpolation (section I) and snaps, so a collaborator jumping across
  the document does not draw a cursor sliding through unrelated text.
- **IME reanchor (P-131).** Covered by `DraftBuffer::reanchor` (section C): a remote delta mid-composition
  shifts `insert_pos`, so `compositionend` emits `InsertText` at the corrected byte offset.
- **Slash menu race/close (P-132, P-133).** `/` opens the menu optimistically but still flows to
  `beforeinput` so the model records the character (round-trip is idempotent, P-132); the menu
  closes on `Backspace` that erases the `/`, on a space, or on any pointer click (P-133).
- **Empty-doc first block (P-147).** A `PointerDown` returning `DocPos::Empty` / `hit_test == None`
  on a zero-block doc sends `ProseCommand::SplitBlock`-equivalent block creation to Logic and parks
  the caret at the new block start once `LayoutUpdated` arrives. This is the **local** recreate-on-
  open guard only; the **CRDT convergence** guarantee for two peers concurrently emptying the doc
  is `PROSE-027` (deterministic re-insertion, `prose-logic sec.2.A`), not this rule.

- **Link click-navigation (P-151).** A `MARK_LINK` span carries a `link: Option<String>` URL
  (`sec.7`) but is inert without an intent: `hit_test` resolves only `(TreeID, offset)`, so a click
  lands the caret and never opens the target. On a `PointerDown` with `ctrl` (or `Cmd` on macOS)
  whose hit offset falls inside a `MARK_LINK` run, `handle_pointer` resolves the run's URL and asks
  the host to open it -- `LogicCommand`-routed to the shared open-URL host affordance
  (`RestCommand`-adjacent, `ambre-spec-sync.md`), never a raw `window.open` from WASM. A plain click
  (no modifier) still only places the caret, so a link stays editable. Hover paints the link
  affordance cursor. No CRDT mutation.

**Hit-testing recap.** `hit_test` finds the block by bisection on `BlockLayout::y_in_page`
(O(log n)) within the resolved band, then reads the offset from the block's **pre-shaped** run
data (`ParleyRun::hit_test_x`) -- never a live Parley layout pass, so it works on culled,
never-painted blocks (P-137).

---

### F. Contextual Toolbar (DOM chrome)

The contextual toolbar is a **DOM Leptos component** (`prose_chrome::FormatBar`, built on
`ambre-chrome`), like Trame's `FormatBar` -- **not** a canvas `DrawSurface` overlay
(`ambre-spec-ui.md section 2.A`, `ambre-spec-chrome.md section 2.B`). It appears anchored above the
selection on `PointerUp` with a non-empty selection and hides when the selection is canceled.
The canvas view computes the anchor rectangle and raises a Leptos signal (Canvas -> DOM via
signals, `ambre-spec-ui.md section 2.C`); the toolbar element is **transform-synced** to that anchor
each frame so it tracks scroll. Its buttons invoke `CommandId`s from `ambre-commands`, which
map to `ProseCommand`s.

The view publishes the current mark state (`ActiveMarks`) for the chrome to render button
toggles; it holds no canvas-drawn toolbar geometry.

```rust
// ambre-prose/src/ui/toolbar.rs -- state PUBLISHED to the Leptos FormatBar (not drawn on canvas)

#[derive(Debug, Clone, Default)]
pub struct ToolbarState {
    pub visible:  bool,
    pub anchor:   Vec2,        // canvas-space anchor (selection bottom-center); chrome transform-syncs to it
    pub active:   ActiveMarks, // current mark state on the selection
}

#[derive(Debug, Clone, Default)]
pub struct ActiveMarks {
    pub bold:          TriState,   // On | Off | Mixed
    pub italic:        TriState,
    pub underline:     TriState,
    pub strikethrough: TriState,
    pub code:          TriState,
    pub link:          Option<String>,   // URL if the entire selection is a link
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TriState { #[default] Off, On, Mixed }
```

**Toolbar buttons:**

`B` `I` `U` `S` `<>` (inline code) | `[link]` | `H1` `H2` `H3` (change block type)
| `"` (blockquote) | `[more]` (color picker, font size)

Each button invokes a `CommandId` (`ambre-commands`) resolved to a `ProseCommand`:
- Toggle mark -> `ProseCommand::ApplyMark` or `ProseCommand::RemoveMark`.
- Change block type -> `ProseCommand::SetBlockStyle`.

The toolbar is a DOM Leptos element (`prose_chrome::FormatBar`), transform-synced to the
selection anchor -- no canvas z-order concern, the browser compositor layers DOM over canvas.
Overflow (`...`) and mobile bottom-sheet behavior follow `ambre-spec-chrome.md section 2.B`.

---

### G. Slash Command (`/`)

The slash command opens when `/` is typed at the beginning of a block (empty block or
position 0). It is a **DOM Leptos menu** (`prose_chrome`, on `ambre-chrome`'s menu
primitives -- roving tabindex, flip-on-overflow), anchored under the caret via a Leptos signal
the canvas view raises (Canvas -> DOM). It is **not** a canvas overlay. It displays a
filterable list of block types and templates; the view holds the query/selection state it
publishes to the menu.

```rust
// ambre-prose/src/ui/slash_menu.rs -- state PUBLISHED to the Leptos slash menu

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    pub anchor:    Vec2,        // caret position at open (canvas space); the DOM menu anchors under it
    pub query:     String,      // text typed after `/`
    pub items:     Vec<SlashItem>,
    pub selected:  usize,
}

#[derive(Debug, Clone)]
pub struct SlashItem {
    pub label:    &'static str,
    pub icon:     Icon,            // Icon(u16) -- lyon vertex buffer compiled at build time
    pub command:  SlashCommand,
}

#[derive(Debug, Clone)]
pub enum SlashCommand {
    InsertBlock(BlockType),
    InsertTable { rows: u32, cols: u32 },
    InsertToc,
    InsertHRule,
    InsertPageBreak,
    SetHeading(u8),
    SetCodeBlock { lang: Option<String> },
}
```

**Default items (filtered by `query`):**

| Label | Icon | Command |
|-------|-------|---------|
| Text | [para] | `InsertBlock(Paragraph)` |
| Heading 1 | H1 | `SetHeading(1)` |
| Heading 2 | H2 | `SetHeading(2)` |
| Heading 3 | H3 | `SetHeading(3)` |
| Bullet list | [bullet] | `InsertBlock(ListItem { Bullet })` |
| Numbered list | 1. | `InsertBlock(ListItem { Ordered })` |
| Task list | [check] | `InsertBlock(ListItem { Todo })` |
| Table | [grid] | `InsertTable { rows: 3, cols: 3 }` |
| Code | `</>` | `SetCodeBlock { lang: None }` |
| Separator | -- | `InsertHRule` |
| Page break | [pagebreak] | `InsertPageBreak` |
| Table of contents | [toc] | `InsertToc` |

Selection via `^` `v` `Enter`. Close via `Escape` or click outside the menu.
The `/` and `query` are removed from the body text before sending the block command.

---

### H. Interactive Ruler

The ruler is displayed at the top of the editor, aligned with the current page width.
It allows adjusting by direct drag:
- Left indent (`indent_left`)
- First line indent (`indent_first`)
- Right indent (`indent_right`)

Values are sent to the Logic thread via `ProseCommand::SetParagraphStyle` on
`on_pointer_up` (not real-time during drag to avoid CRDT command flood).

```rust
// ambre-prose/src/ui/ruler.rs

#[derive(Debug, Clone)]
pub struct RulerState {
    pub visible:       bool,
    pub page_left_px:  f32,    // X position of the page's left margin
    pub content_width_px: f32,
    pub dragging:      Option<RulerHandle>,
}

#[derive(Debug, Clone, Copy)]
pub enum RulerHandle {
    IndentFirst,
    IndentLeft,
    IndentRight,
}
```

The ruler is a **DOM Leptos component** (`prose_chrome`), fixed at the top of the editor
chrome (independent of vertical canvas scroll) -- not a canvas `DrawSurface` band. Its drag
handles emit `ProseCommand::SetParagraphStyle` on `on_pointer_up`.

---

### I. Collaborative Cursors

Remote cursors are received via `ProseEvent::RemoteCursors` from the Logic thread.
The author identity is the **Sync-resolved `from_user_id`** (spoof-proof,
`ambre-spec-sync.md section 3`): `ambre-sync` decrypts the `Ephemeral` message and resolves
the canonical `UserId` before delivery, so the Logic thread never resolves identity
itself. The peer-authored payload carries only the cursor position and a self-asserted,
cosmetic `display_name` -- never a `user_id`. Each peer is assigned a color derived from
that `UserId` via the **shared cross-app helper** `ambre_draw::peer::peer_color` -- the
same function Trame and Toile call, so a given collaborator has the same color across
all apps in the suite. Prose does not reimplement the hash (see `ambre-spec-canvas.md section D`).

```rust
// ambre-prose/src/ui/remote_cursor.rs

fn user_color(user_id: &UserId) -> peniko::Color { /* ... */ }
```

Rendering:
- **Cursor bar:** vertical 2px line, peer color, height = line height.
- **Label:** `display_name` badge displayed above the bar, peer-colored background.
- **Remote selection:** semi-transparent rectangle (15% opacity) over the selected range.

Remote cursors animate with a CSS-like transition (linear position interpolation over
100ms) to mask network latency.

---

### J. Comments & Track Changes Display

#### Comments

Comments are displayed in two places:
1. **Inline highlight (canvas content):** semi-transparent yellow rectangle under the annotated range (mark `"cm"`), drawn by the view.
2. **Side panel (DOM chrome):** a Leptos `prose_chrome` panel listing comment threads, vertically aligned with their anchor. The view publishes each thread's `anchor_y`; the panel is a DOM element, not a canvas overlay.

```rust
// ambre-prose/src/ui/comments.rs

#[derive(Debug, Clone)]
pub struct UiComment {
    pub comment_id: Uuid,
    pub anchor_y:   f32,    // absolute Y coordinate of the anchor start (from doc top)
    pub resolved:   bool,
    pub thread:     Vec<UiReply>,
}

#[derive(Debug, Clone)]
pub struct UiReply {
    pub author:     String,
    pub text:       String,
    pub created_at: u64,
}
```

The side panel is a DOM Leptos element to the right of the page, independently scrollable
(the browser handles its scroll). A click on a comment in the panel raises a signal that
scrolls the canvas document to the corresponding anchor.

#### Track Changes

Suggestion **highlights are canvas content** (drawn inline by the view):
- **Suggested insertion:** green underlined text, very light green background.
- **Suggested deletion:** red strikethrough text, very light red background.

The `Accept` / `Reject` affordance is a **DOM Leptos popover** (`prose_chrome`),
anchored (transform-synced) to the hovered suggestion via a Leptos signal the view raises;
its buttons invoke `ProseCommand::AcceptSuggestion` / `RejectSuggestion`. It is not a canvas
overlay.

---

### K. Clipboard

Prose uses the shared, **Logic-authoritative, host-driven** clipboard model
(`ambre-spec-ui.md section 2.E`). `prose-ui` **never** calls `navigator.clipboard` -- the host
(`ambre-app`) performs the OS clipboard I/O inside the user-activation window. The transport
type is the app-agnostic `ClipboardBlob { text: String, ambre: Option<Vec<u8>> }` (`ambre-bus`):
`text` is the `text/plain` mirror; `ambre` is Prose's opaque payload (postcard of a typed
Prose clipboard payload -- rich runs + block structure) under the private `application/x-ambre`
MIME type. There is **no `text/html`** on the OS clipboard and **no viewport culling** -- the
full-fidelity payload is built by the Logic thread from the whole CRDT.

```rust
// ambre-prose/src/ui/clipboard.rs -- driver methods called by FrameHost (no web_sys here)

/// Native copy/cut: request the authoritative build from Logic; nothing serialized UI-side.
pub fn clipboard_write(&mut self, cut: bool) -> CopyOutcome { /* ... */ }

/// Called by the host right after update(): if Logic replied (ProseEvent::Clipboard),
/// return the ClipboardBlob to mirror to the OS clipboard next frame (poll_clipboard).
pub fn poll_clipboard(&mut self) -> Option<ClipboardBlob> { /* ... */ }

/// Native paste: prefer the in-memory blob when the OS text mirror still matches it
/// (full fidelity), else decode blob.ambre; otherwise fall back to the plain-text mirror.
/// The `ClipboardBlob` carries only { text, ambre } -- there is NO text/html on the OS
/// clipboard (Foundation clipboard model, `ambre-spec-ui.md section 2.E`), so paste has exactly
/// two outcomes. Importing a foreign HTML/docx/md *document* is a separate flow (file open
/// or drag-drop -> ProseCommand::ImportDocument), not a clipboard paste.
pub fn paste_clipboard(&mut self, blob: ClipboardBlob) { /* ... */ }
```

The Logic thread builds `ProseEvent::Clipboard { text, ambre }` from the CRDT selection
(`prose-logic`); the view stashes it in memory (same-tab full-fidelity paste) and hands it to
the host via `poll_clipboard`. The in-memory stash is `own_clipboard: Option<OwnClipboard>`
with `struct OwnClipboard { text: String, payload: ProseClipboardPayload }`, set in `update()`
from `ProseEvent::Clipboard` (the same `payload` type carried by `ProseCommand::PasteRich` and
returned by `decode_ambre`). It is what the mirror gate in `paste_clipboard` consults. **Cut**
clears the source range after the copy command, Prose's own policy.

### L. Content Accessibility (DOM mirror)

Content (page column, caret, selection, highlights) is painted on the canvas, which exposes
no accessibility tree. Foundations mandates the same remedy for every app: content a11y is a
**DOM mirror** -- an accessible `<textarea>` plus a live region -- not AccessKit
(`ambre-spec-chrome.md sec.2.F`, CHROME P3; the a11y tree on the web is the DOM). Prose meets
it with two DOM surfaces `ambre-app`/`ambre-chrome` already own:

- **Accessible text surface.** The shared transparent `<textarea>` (sec.2.E, the
  keyboard/IME pipe) is the screen-reader-navigable mirror of the editable content: it carries
  editing focus and holds the caret block's text, so a screen reader reads and caret-navigates
  the text the user is editing rather than a silent canvas.
- **Live region (`aria-live="polite"`).** The view announces caret context on selection
  change -- the block type and heading level, entering or leaving a table cell, footnote, or
  comment thread, and the accept/reject state of a tracked change. This is Prose's analog of
  Trame's active-cell announce (`ambre-spec-trame-ui.md sec.2.B`, the `SelectRange` announce
  arm); both feed the one live-region contract of CHROME P3.

Chrome (toolbar, slash menu, ruler, comment panel) inherits ARIA from `ambre-chrome` on its own
(sec.7 rejected-decisions note); this subsection is only about the **content** the canvas owns.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

| Flow | Data | Risk | Mitigation |
|------|---------|--------|------------|
| `flume` UI -> Logic thread | Typed `ProseCommand` (in `LogicCommand::App`) | Plaintext data in the **shared** WASM heap of the threads | Normal: the heap is only accessible from inside the user's browser process (sandbox). The value is **moved** between threads -- no serialization, no boundary copy, no external exposure. |
| Clipboard write (host `navigator.clipboard.write`, `ClipboardBlob`) | `text/plain` mirror + opaque `x-ambre` payload of the selection | The browser may expose the clipboard to other tabs | Browser standard -- outside AMBRE's control; `prose-ui` never touches `navigator.clipboard` (host-driven, `ambre-spec-ui.md section 2.E`) |
| Document import (foreign HTML/docx via file open or drag-drop, `ProseCommand::ImportDocument`) | Untrusted external document bytes | Malicious HTML | Routed through `import_html` (html5ever sanitizes) in the Logic thread -- *not* a clipboard-paste path |
| Remote cursors | `display_name` received via `ProseEvent::RemoteCursors` | Already decrypted in the Logic thread | No additional exposure |

The UI thread never has access to cryptographic keys or the `LoroDoc`. It only displays
already-decrypted data transmitted by the Logic thread.

---

## 4. QUALITY & TESTABILITY

**Unit tests (host tier, `cfg(not(target_arch = "wasm32"))`):**

| Case | Method |
|-----|---------|
| `visible_pages()`: viewport overlaps pages 2 and 3 | `PageColumn::visible_pages()` with fixed values |
| `hit_test()`: click at known coordinate -> expected `(block_id, offset)` | Mock `BlockLayout` + mocked Parley metrics |
| `DraftBuffer`: push 5 chars -> clear -> empty | Direct assertions |
| `ambre_draw::peer::peer_color`: two distinct user_ids -> distinct colors | Hash collision check on 1000 ids (owned by `ambre-draw`; Prose only delegates) |
| `compute_selection_bounds()`: single-block and multi-block selection | Mocked Parley geometry |
| Keyboard shortcuts: Ctrl+B -> `ApplyMark(bold)`, Ctrl+Z -> `Undo` | `KeyEvent` simulation on `handle_key_event()` |
| Slash menu: filtering by query "tab" -> items containing "tab" | `SlashMenuState::filter()` |
| IME: compositionstart -> update -> end -> `InsertText` | Simulation of 3 events |

**WASM tests (`wasm-bindgen-test`):**

| Case | Method |
|-----|---------|
| wgpu rendering: page rendered without panic on 800x600 canvas | `wasm-bindgen-test` with headless canvas |
| `flume` UI -> Logic thread: `LogicCommand::App` emitted and routed to `ProseState` | Mock `flume` channels (host test) |
| `paste_clipboard`: `ClipboardBlob` with `ambre` -> `PasteRich`; blob without `ambre` -> plain-text `InsertText` (two outcomes only, no HTML branch) | Host-driven, `ClipboardBlob` fixtures (no `navigator.clipboard` in `prose-ui`) |

**Visual tests (snapshots):**

Snapshots of rendered pages via `QuadRenderer` compared to reference snapshots (PNG).
Tool: custom cargo script that exports the display list as PNG via `wgpu` headless.
Tolerance: pixel diff <= 2% (anti-aliasing varies by GPU).

---

## 5. PERFORMANCE & RESOURCES

**Frame budget (16ms at 60 FPS):**

| Step | Budget | Notes |
|-------|--------|-------|
| `build_display_list()` (3 visible pages) | < 4 ms | `BlockLayout` traversal + pre-shaped run assembly |
| Build vertex batch + GPU upload | < 4 ms | QuadRenderer + wgpu |
| Submit GPU + present frame | < 6 ms | GPU-bound, varies by hardware |
| **Total frame budget** | **< 14 ms** | 2ms margin for interleaved input events |

Steps 9-11 (comment / suggestion highlights + draft buffer) are **O(visible)**, not
O(total annotations): the highlight helpers skip any annotation anchored outside the
visible page range and the draft helper early-returns on an empty buffer, so the
`build_display_list()` < 4 ms budget holds at any comment / suggestion count.

**Input latency (P2 -- high priority):**

| Operation | Target |
|-----------|-------|
| Keystroke -> draft display (UI thread only) | < 2 ms |
| Keystroke -> CRDT confirmation (Logic thread round-trip) | < 15 ms |
| Scroll 1 page (virtual scroll, culling) | < 16 ms |
| Slash menu open | < 8 ms |
| Floating toolbar display | < 4 ms |

**UI thread memory:**

| Structure | Estimated size |
|-----------|---------------|
| `RunCache` (500 blocks, ~50 pages) | < 20 MB (Parley runs + glyphs) |
| `PageLayout` (100 pages, 300 blocks, 20 footnotes, header/footer runs) | < 200 KB |
| `ProseUiState` (excluding RunCache) | < 1 MB |
| Vertex buffers + GlyphAtlas wgpu (3 pages) | < 50 MB VRAM |

**WASM size (UI thread -- note: wgpu/lyon/Parley/swash are shared across all apps):**

| Component | Raw estimated | Gzip estimated |
|-----------|-----------|-------------|
| `ambre-prose/ui` (Prose UI logic) | ~50 KB | ~16 KB |
| `wgpu` + `lyon` + `swash` (shared) | ~800 KB | ~250 KB |
| `parley` (shared) | ~200 KB | ~60 KB |

> wgpu/lyon/Parley/swash are shared dependencies across all suite applications --
> their cost is amortized across the entire `ambre-app` binary.

---

---

## 6. SOURCE OF TRUTH MODEL BY STATE TYPE

A simple rule: **one source of truth, other layers are projections of it.**

| Category | Examples | Source of truth | Synced via SecSync |
|-----------|---------|-----------------|------------------------|
| **Document mutation** | InsertText, ApplyMark, MoveBlock, InsertImage | `LoroDoc` (Logic thread) | Yes |
| **Shared metadata** | Comments, suggestions, custom styles, headers/footers, TOC config | `LoroDoc` (Logic thread) | Yes |
| **Local view** | Scroll position, selection, text cursor, inline editor, draft buffer, open slash menu, visible toolbar | `ProseUiState` (UI thread) | No |
| **Derived state** | `PageLayout`, `RunCache`, `TableOfContents`, `WordCount` | Logic thread (recomputed from `LoroDoc`) | No |
| **Local history** | Undo stack (`UndoManager`) | Logic thread (in memory, current session) | No |

**Fundamental invariant:** the UI thread contains no document data -- only view states
and data received from the Logic thread via `ProseEvent`. Every mutation goes through
a `ProseCommand`. There are no two models of the document: `LoroDoc` is the single
source of truth.

**Documented edge cases:**
- **Scroll position:** local only. Two collaborators have independent scroll positions -- identical to Google Docs.
- **Selection:** local. The remote selection is displayed via `RemoteCursor` (encrypted ephemeral), not stored in `LoroDoc`.
- **Draft buffer:** UI thread transient state only -- gone on disconnect. Never sent over the network.
- **`layout_version`:** derived from the Logic thread, used only for UI anti-flickering -- no business meaning.

---

## 7. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **Parley shaping in the UI thread** | Shaping is O(n_glyphs) and can exceed 16 ms on a long block. Crucially, **pagination already shapes every block in the Logic thread** (`prose-logic section 2.B`) to measure page breaks -- so shaping in the UI would *duplicate* that work and risk blocking the render thread (violating the "render thread never blocked by computation" pillar). The UI receives pre-shaped `ParleyRunData` and only draws glyphs. This is a deliberate Prose-specific departure from the generic UI-thread `TextCache` shaping path (`ambre-spec-canvas.md section D`, used for Trame single-line cells); the Foundation Prose-frame budgets are annotated accordingly (`ambre-spec-chrome.md section 5.A`, `ambre-spec-canvas.md section 5.A`). |
| **DOM/HTML for editor rendering** | Rejected. Unpredictable layout, impossible to control pagination, JS/WASM interop cost. |
| **`contentEditable`** | Non-deterministic behavior across browsers, inconsistent IME handling, impossible to control for pagination and virtual scroll. |
| **Full-page rendering each frame** | 100 pages x full rendering = blown budget. Virtual scroll with culling of non-visible pages is mandatory. |
| **Chrome (toolbar, slash menu, ruler, comment panel) drawn on the canvas via `DrawSurface`** | Rejected per the Foundation canvas/DOM boundary (`ambre-spec-ui.md section 2.A`): chrome is DOM/Leptos (`ambre-chrome`/`prose_chrome`), transform-synced to canvas anchors. Static DOM over a canvas is one layout pass then zero-cost compositor recomposition, and inherits ARIA a11y, focus trap, and IME for free -- none of which a canvas widget gets. Only **content** (caret, selections, highlights, remote cursors) stays on the canvas. |
| **`winit` for input** | Web-only target: winit is indirection without benefit and conflicts with DOM event ownership. Events are captured directly via `web-sys` (DOM listeners + the transparent `<textarea>`) and delivered as `InputEvent` (`ambre-spec-ui.md section 6`, `ambre-spec-chrome.md section 2.C`). |
| **`prose-ui` owning `navigator.clipboard` + a `text/html` clipboard payload** | Clipboard is host-driven and Logic-authoritative (`ambre-spec-ui.md section 2.E`): the host performs OS clipboard I/O in the activation window; the payload is `ClipboardBlob { text, ambre }` (opaque `x-ambre`), built from the full CRDT -- not `text/html`, not viewport-culled. |
| **Canvas 2D (non-WebGL)** | Insufficient performance for high-quality typographic rendering. wgpu (WebGL2/WebGPU) is AMBRE's foundational choice. |
| **Synchronous CRDT confirmation before display** | Logic thread round-trip ~1-3ms perceptible at high typing cadence. The draft buffer absorbs this latency without compromising consistency. |
| **Selection via `window.getSelection()` DOM** | Outside WASM context. Selection is managed entirely via Parley metrics and pointer events. |
| **`requestIdleCallback` in the Logic thread** | Not available in a `wasm_thread` worker (the Logic thread). Deferred/low-priority work there uses the P1-P4 priority scheduler of `ambre-logic`, not `requestIdleCallback`. (On the UI thread, where it *is* available, the host may use it for opportunistic layout -- `ambre-spec-chrome.md section 5.B`.) |
| **Layouts without versioning** | Without `version: u64` on `LayoutUpdated`, two pagination recalculations arriving out of order would produce stale display. The UI thread silently drops any `LayoutUpdated` whose version is lower than the last applied layout. |

---

## 8. INTEGRATION PLAN (ROADMAP)

**Phase 0 -- Static Rendering (Week 1)**
- [ ] `wgpu` + `QuadRenderer` setup on `HTMLCanvasElement` in the UI thread.
- [ ] `PageColumn` + `visible_pages()` + basic virtual scroll (vertical scroll only).
- [ ] Rendering of a mocked `PageLayout`: page background, shadows, page numbers.
- [ ] Rendering of pre-shaped Parley runs received from the Logic thread (static text, no cursor).
- [ ] `RunCache` with LRU eviction.
- [ ] Visual snapshot tests: page rendered correctly.

**Phase 1 -- Input & Cursor (Week 2)**
- [ ] `handle_key_event()`: cursor navigation (arrows, Home, End), printable characters.
- [ ] `DraftBuffer`: optimistic display + clear on `DocUpdated`.
- [ ] `hit_test()`: mouse click -> `(block_id, offset)`.
- [ ] `handle_pointer()`: `InputEvent::PointerDown` cursor, `PointerMove` drag selection, `PointerUp` toolbar.
- [ ] `DrawSurface` rendering of blinking cursor (2px fill_rect) and selection (alpha fill_rect).
- [ ] `flume` UI -> Logic thread send: `LogicCommand::App { command: AppCommand::Prose(ProseCommand) }` (`try_send`, non-blocking).
- [ ] Prose `AppView::Prose` variant: inherent `update` (drain `UiEvent` `flume`) / `plan_repaint` / `paint_content` / `paint_overlay` dispatched by `FrameHost`.
- [ ] Unit tests keyboard, hit_test, draft buffer.

**Phase 2 -- Toolbar, Slash, IME (Week 3)**
- [ ] Contextual toolbar: DOM Leptos `prose_chrome::FormatBar`, transform-synced to the selection anchor; toggle marks, change block type via `ambre-commands`.
- [ ] Slash command: DOM Leptos menu anchored under the caret, filtering, selection, block insertion.
- [ ] IME: consume `InputEvent::Composition` from the shared transparent `<textarea>` (`ambre-chrome`); canvas composition preview; `InsertText` on `compositionend`.
- [ ] Double-click -> word selection.
- [ ] Unit tests slash menu, IME, toolbar-state derivation.

**Phase 3 -- Collaboration & Annotations (Week 4)**
- [ ] Remote cursors: `RemoteCursors` receipt, bar + label + selection rendering, animation (canvas content).
- [ ] Comment inline highlights (canvas) + DOM Leptos side panel (thread list).
- [ ] Track changes highlights (green/red, canvas) + DOM accept/reject popover on hover.
- [ ] Unit tests `user_color()`, highlight rendering.

**Phase 4 -- Ruler, Clipboard, Inertial Scroll (Week 5)**
- [ ] Interactive ruler: DOM Leptos `prose_chrome` component, drag handles, `SetParagraphStyle` on pointer up.
- [ ] Clipboard: host-driven `ClipboardBlob` flow (`clipboard_write`/`poll_clipboard`/`paste_clipboard`); `ProseEvent::Clipboard` build in Logic; paste has two outcomes only (`PasteRich` / plain-text `InsertText`, no HTML branch).
- [ ] Selection-aware editing: `take_selection` (emit `DeleteRange`, collapse caret); type/Backspace/Delete/Enter/IME/paste delete an active selection first.
- [ ] Inertial scroll: `scroll_velocity`, physical deceleration (linear friction).
- [ ] Optimizations: frame budget profiling, reducing allocations on `build_display_list()` hot path.

**Phase 5 -- Spell Check UI (post-MVP)**
- [ ] `ProseEvent::SpellErrors` receipt, wavy underline rendering via `fill_path` lyon.
- [ ] Right-click context menu on underlined word: suggestions, "Add to dictionary".
