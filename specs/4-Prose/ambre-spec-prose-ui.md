# AMBRE — SPEC: PROSE / UI
**Type:** Vertical (Feature/Widget)
**Status:** Draft

> Prose presentation layer: wgpu/Parley **content** rendering (DrawSurface) of the page
> column, caret, selections, collaborative cursors, and inline comment/track-change
> highlights; page-based virtual scroll; input via the shared transparent `<textarea>`
> (keyboard/IME/clipboard). The application **chrome** (contextual toolbar, slash menu,
> ruler, comment side panel, accept/reject) is **DOM/Leptos** (`ambre-chrome`), not canvas.
> No CRDT mutation is performed here — every mutation is a `ProseCommand` to the Logic thread.

> **Architecture note.** The client uses the **two-instance model** — a **Main** WASM
> instance (UI thread + Logic thread sharing one linear memory via `wasm_thread`, typed
> `flume` values) and a **separate Sync worker** — see `ambre-spec-architecture.md §2.B`.
> For this spec:
> - **Typed `flume` transport, no `postMessage`/postcard on the UI ↔ Logic path.** The UI sends `LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }` and receives `UiEvent::Viewport { event: AppEvent::Prose(ProseEvent) }` — Rust values **by move** on `flume`, no serialization on the UI side. (`postMessage`/postcard is only the Main ↔ Sync hop, invisible to this layer.)
> - **The Prose view is an `AppView` variant.** Like `TrameView`, the Prose presentation exposes the inherent driver methods (`update`/`plan_repaint`/`paint_content`/`paint_overlay`/`resize`/`handle_input`) that the `FrameHost` `match`es over — adding Prose is a new `AppView::Prose(ProseView)` arm in `ambre-app` (see `ambre-spec-canvas.md §A`). The GPU pipeline (`QuadRenderer`, `WgpuSurface`) lives in `ambre-canvas`; the `FrameHost` driver lives in `ambre-app`.
> - **Canvas / DOM boundary (`ambre-spec-ui.md §2.A`).** The canvas owns **content** (page column, caret, selections, content hit-testing, remote cursors, inline highlights). The DOM (Leptos, `ambre-chrome` + a specific `prose_chrome`) owns **all chrome** (contextual toolbar, slash menu, ruler, comment panel, dialogs, menus). Input is captured by DOM listeners and the transparent `<textarea>` — **winit is not used** (`ambre-spec-ui.md §6`).
> - Memory is **shared** across UI ↔ Logic (single heap): the `ProseCommand` is **moved**, not copied.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles (**canvas content** unless noted):
- wgpu/Parley rendering (DrawSurface) of visible pages in the viewport (display list).
- Virtual scroll: culling of pages outside the viewport, smooth scrolling management.
- Text caret, selection (mouse, keyboard, touch); keyboard/IME input via the shared transparent `<textarea>` (`ambre-chrome`).
- Remote collaborative cursors; inline comment and track-change highlights.
- **DOM chrome** (Leptos, `ambre-chrome`/`prose_chrome`): contextual toolbar, slash menu, interactive ruler, comment side panel, accept/reject affordances.
- Clipboard: the host-driven, Logic-authoritative `ClipboardBlob` flow (`clipboard_write`/`poll_clipboard`/`paste_clipboard`, `ambre-spec-ui.md §2.E`).

This component delegates:
- All CRDT mutations → Logic thread via `flume`: `LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }`.
- Pagination calculation (`PageLayout`) → `prose-engine` (Logic thread).
- Effective style resolution → `prose-features` (Logic thread).
- Import/export conversions → `prose-interop` (Logic thread).

**Dependencies:**

| Crate / API | Role |
|-------------|------|
| `wgpu` + `QuadRenderer` | GPU rendering: vertex/fragment shaders, WebGL2 default |
| `lyon` | Path tessellation (shadows, highlights, wavy underlines) |
| `parley` | Typographic layout: line breaking, shaping, glyph metrics |
| `swash` | Glyph rasterization → GlyphAtlas GPU |
| `flume` | Inter-thread channels: send `LogicCommand`, receive `UiEvent` (typed values by move) |
| `ambre-bus` | Typed IPC types: `LogicCommand`/`UiEvent`, `ProseCommand`/`ProseEvent`, `AppCommand`/`AppEvent`, `ClipboardBlob` |
| `ambre-canvas` | GPU pipeline (`QuadRenderer`, `WgpuSurface`), GPU bootstrap + rAF loop, per-frame value types (`RepaintPlan`, `InputEvent`). The `FrameHost` driver + `AppView` enum live in `ambre-app` |
| `ambre-draw` | `DrawSurface` trait (parameterized screen/PDF rendering) |
| `ambre-chrome` + `leptos` | DOM chrome toolkit: the transparent `<textarea>` (keyboard/IME/clipboard pipe), and the base components the specific `prose_chrome` (toolbar, slash menu, ruler, comment panel) builds on |
| `ambre-commands` | Command registry / keybindings shared with the chrome (`CommandId` → `LogicCommand`) |

Input events reach the view as `InputEvent` (from `ambre-canvas`) via `AppView::handle_input`;
DOM listeners and the transparent `<textarea>` are wired by `ambre-app`/`ambre-chrome`.
**No `winit`** (`ambre-spec-ui.md §6`). `navigator.clipboard` is driven by the **host**, not
by this crate (`ambre-spec-ui.md §2.E`).

All `web_sys::*` resources (`HTMLCanvasElement`, `GPUDevice`, `GPUQueue`) are `!Send`
and confined to the UI thread. They never travel through a `flume` channel.

**Invariants:**

- The UI thread never owns a `LoroDoc` nor a `ProseState`. It is **stateless** with respect to content: it displays the `PageLayout` and Parley runs received from the Logic thread.
- Any user input **immediately** produces a `ProseCommand` sent to the Logic thread via `flume` (`try_send`, non-blocking). Optimistic display (showing keystrokes before CRDT confirmation) is handled by a **local draft buffer** (see section 2.C).
- The frame budget is **16 ms** (`requestAnimationFrame` at 60 FPS). The vertex batch build must never exceed this budget on the hot path (visible pages ± 1).
- Blocking operations (full Parley layout, TOC recompute) **never** execute in the UI thread — they come from the Logic thread as a pre-computed `PageLayout`.
- No `unsafe` in `ambre-prose/ui`.

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
    pub gutter_px:        f32,      // space between pages (default: 16px)
    pub total_height_px:  f32,      // page_count * (page_height + gutter)
}

impl PageColumn {
    /// Returns the page indices intersecting the viewport.
    pub fn visible_pages(&self, viewport: &Viewport) -> RangeInclusive<u32> {
        let first = (viewport.scroll_y / (self.page_height_px + self.gutter_px)).floor() as u32;
        let last  = ((viewport.scroll_y + viewport.height_px) /
                      (self.page_height_px + self.gutter_px)).ceil() as u32;
        first..=last.min(self.page_count() - 1)
    }

    /// Absolute Y coordinate (from the document top) of the top of page i.
    pub fn page_top_y(&self, page_index: u32) -> f32 {
        page_index as f32 * (self.page_height_px + self.gutter_px)
    }
}
```

**Block culling strategy:**

Only blocks whose `BlockLayout::page_start` is in `visible_pages()` are included
in the DrawSurface display list. Blocks on non-visible pages are neither shaped
nor rasterized — their `ParleyRun`s are cached but not sent to `wgpu`.

**Parley cache (`RunCache`):**

```rust
// ambre-prose/src/ui/run_cache.rs

/// Per-block Parley run cache. Runs are pre-shaped in the Logic thread
/// (type `ParleyRunData`, moved by value on `flume` — no serialization) and
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
pub struct ParleyRun {
    pub data:   ParleyRunData,            // typed run data (glyphs, positions, metrics)
    pub layout: Option<parley::Layout<peniko::Brush>>,  // layout cache, built on first paint
}

impl From<ParleyRunData> for ParleyRun {
    fn from(data: ParleyRunData) -> Self {
        Self { data, layout: None }
    }
}
```

> **Note on the two types:** `ParleyRunData` (defined in `prose-engine`) is the **typed** DTO
> that travels from the Logic thread to the UI thread, by move on `flume`, inside
> `ProseEvent::RunsUpdated` → `AppEvent::Prose` (shared memory, no serialization).
> `ParleyRun` (defined in `prose-ui`) wraps `ParleyRunData` and adds an optional
> `parley::Layout` cache — built lazily on the first `paint()` via
> `TextCache::get_or_build`, reused as long as the block is not dirty. The UI thread never
> shapes directly — it calls `draw_glyphs` on pre-shaped layouts.

---

### C. Draft Buffer (Optimistic Display)

The UI thread → Logic thread → UI thread round-trip latency is typically 1–3 ms,
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
}

impl DraftBuffer {
    /// Called on each keystroke before sending the ProseCommand to the Logic thread.
    pub fn push_char(&mut self, c: char, block_id: &TreeID, pos: u32) {
        if self.block_id.as_ref() != Some(block_id) {
            self.clear();
            self.block_id = Some(block_id.clone());
            self.insert_pos = pos;
        }
        self.pending_text.push(c);
    }

    /// Called on receipt of ProseEvent::DocUpdated.
    /// The Logic thread confirmed: the draft is integrated into the official runs.
    pub fn clear(&mut self) {
        self.pending_text.clear();
        self.block_id = None;
        self.insert_pos = 0;
    }
}
```

The draft buffer is rendered via `DrawSurface::draw_glyphs` as a temporary run with the
current cursor style, positioned after the block's last confirmed run. In case of
desynchronization (a remote delta arrives before local confirmation), the draft is
canceled and the cursor repositioned to the confirmed CRDT position.

---

### D. DrawSurface Rendering — Display List

On each `requestAnimationFrame` (if `dirty`), the display list is built from
`ProseUiState` and submitted to `QuadRenderer` for GPU flush.

Block rendering functions (`render_block_on`, `render_selection_on`, etc.) are
parameterized on the `DrawSurface` trait defined in `ambre-draw`. The UI thread
instantiates `WgpuSurface` — the same code is reused by `prose-interop` with
`KrillaSurface` for PDF export in the Logic thread.

```rust
// ambre-prose/src/ui/renderer.rs

pub fn build_display_list<S: DrawSurface>(
    state: &ProseUiState,
    surface: &mut S,
) {
    let visible = state.layout_column().visible_pages(&state.viewport);

    for page_idx in visible {
        let page_origin = Vec2::new(
            state.viewport.page_margin_left_px,
            state.layout_column.page_top_y(page_idx) - state.viewport.scroll_y,
        );

        // 1. Page shadow and background
        render_page_background_on(&mut surface, page_origin, &state.layout);

        // 2. Header
        //    Parley runs are pre-shaped in the Logic thread (pagination Phase 3.B)
        //    with dynamic fields already resolved (f:pn = page number, etc.).
        //    The UI thread does not shape -- it assembles runs from layout.header_runs[page_idx].
        //    If different_first_page and page_idx == 0: header_first runs are in header_runs[0].
        let header_height = if page_idx == 0 {
            state.layout.header_first_height_px
        } else {
            state.layout.header_height_px
        };
        if header_height > 0.0 {
            if let Some(runs) = state.layout.header_runs.get(page_idx as usize) {
                let header_origin = page_origin + Vec2::new(
                    state.layout.margin_left_px,
                    state.layout.margin_top_px,
                );
                render_runs_on(&mut surface, runs, header_origin);
            }
        }

        // 3. Visible blocks on this page
        //    y_in_page in BlockLayout is relative to the start of the content area
        //    (after margin_top + header_height). The vertical offset is:
        //    page_origin.y + margin_top + header_height + block.y_in_page
        let content_top = state.layout.margin_top_px + header_height;
        for block_layout in state.layout.blocks_on_page(page_idx) {
            let block_origin = page_origin + Vec2::new(
                state.layout.margin_left_px,
                content_top + block_layout.y_in_page,
            );
            render_block_on(&mut surface, block_layout, block_origin, state);
        }

        // 4. Footnotes
        //    Positioned at the bottom of the content area, above the footer.
        //    A horizontal separator (line 1/3 width) separates them from the body.
        //    Each footnote's Parley runs are pre-shaped (FootnoteLayout.runs,
        //    from pagination Phase 0) -- the UI thread directly assembles.
        if let Some(footnotes) = state.layout.footnotes_per_page.get(page_idx as usize) {
            if !footnotes.is_empty() {
                let footer_height = if page_idx == 0 {
                    state.layout.footer_first_height_px
                } else {
                    state.layout.footer_height_px
                };
                let page_bottom = state.layout.page_height_px - state.layout.margin_bottom_px
                                  - footer_height;
                let footnotes_total: f32 = footnotes.iter().map(|f| f.height_px).sum();
                let sep_y = page_bottom - footnotes_total - FOOTNOTE_SEP_HEIGHT;

                // Separator
                render_footnote_separator_on(&mut surface,
                    page_origin + Vec2::new(state.layout.margin_left_px, sep_y),
                    state.layout.content_width_px / 3.0);

                // Footnote text (pre-shaped runs in FootnoteLayout.runs)
                let mut fn_y = sep_y + FOOTNOTE_SEP_HEIGHT;
                for footnote in footnotes {
                    let fn_origin = page_origin + Vec2::new(
                        state.layout.margin_left_px, fn_y);
                    render_runs_on(&mut surface, &footnote.runs, fn_origin);
                    fn_y += footnote.height_px;
                }
            }
        }

        // 5. Footer
        //    Pre-shaped runs in layout.footer_runs[page_idx] (Phase 3.B).
        let footer_height = if page_idx == 0 {
            state.layout.footer_first_height_px
        } else {
            state.layout.footer_height_px
        };
        if footer_height > 0.0 {
            if let Some(runs) = state.layout.footer_runs.get(page_idx as usize) {
                let footer_origin = page_origin + Vec2::new(
                    state.layout.margin_left_px,
                    state.layout.page_height_px - state.layout.margin_bottom_px - footer_height,
                );
                render_runs_on(&mut surface, runs, footer_origin);
            }
        }
    }

    // 6. Text cursor (blinking)
    if let Some(cursor) = &state.cursor {
        render_cursor_on(&mut surface, cursor, state);
    }

    // 7. Selection (semi-transparent blue highlight)
    if let Some(sel) = &state.selection {
        render_selection_on(&mut surface, sel, state);
    }

    // 8. Remote collaborative cursors
    for rc in &state.remote_cursors {
        render_remote_cursor_on(&mut surface, rc, state);
    }

    // 9. Comment highlights
    render_comment_highlights_on(&mut surface, &state.comments, state);

    // 10. Track changes highlights
    render_suggestion_highlights_on(&mut surface, &state.suggestions, state);

    // 11. Draft buffer (optimistic keystrokes)
    render_draft_on(&mut surface, &state.draft_buffer, state);

    // 12. Spell check underlines (post-MVP)
    #[cfg(feature = "spellcheck")]
    render_spell_errors_on(&mut surface, &state.spell_errors, state);
}
```

#### UI Color Palette

| Element | Color | Opacity |
|---------|---------|---------|
| Text cursor | `#1A73E8` (blue) | 100% |
| Local selection | `#1A73E8` | 20% |
| Comment highlight | `#FBBC04` (yellow) | 40% |
| Track changes insert | `#34A853` (green) | 30% |
| Track changes delete | `#EA4335` (red) | 30% — strikethrough text |
| Remote cursor peer A | Color assigned to peer | 80% (bar) + 15% (selection) |
| Spell error | `#EA4335` (red) | wavy underline |

---

### E. Input Event Handling

#### Keyboard

Keyboard input reaches the view as `InputEvent` through `AppView::handle_input` (there is
**no `winit`** — `ambre-spec-ui.md §6`). The physical source is the shared transparent
`<textarea>` overlay (`ambre-spec-chrome.md §2.C`): it owns editing focus (a click in the
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
) -> bool {
    let InputEvent::KeyDown { key, ctrl, shift, .. } = ev else { return false };
    match key.as_str() {
        // Cursor navigation (view-local, no CRDT mutation)
        "ArrowLeft"  => { move_cursor(state, Direction::Left,  shift); true }
        "ArrowRight" => { move_cursor(state, Direction::Right, shift); true }
        "ArrowUp"    => { move_cursor(state, Direction::Up,    shift); true }
        "ArrowDown"  => { move_cursor(state, Direction::Down,  shift); true }
        "Home"       => { move_cursor(state, Direction::LineStart, shift); true }
        "End"        => { move_cursor(state, Direction::LineEnd,   shift); true }

        // Editing
        "Backspace"  => { send_cmd(to_logic, delete_backward(state)); true }
        "Delete"     => { send_cmd(to_logic, delete_forward(state));  true }
        "Enter"      => { send_cmd(to_logic, split_block(state));     true }
        "Tab"        => { send_cmd(to_logic, indent_or_next_cell(state)); true }

        // Formatting shortcuts (also surfaced via the DOM chrome command registry)
        "b" if ctrl => { send_cmd(to_logic, toggle_mark(state, MARK_BOLD));      true }
        "i" if ctrl => { send_cmd(to_logic, toggle_mark(state, MARK_ITALIC));    true }
        "u" if ctrl => { send_cmd(to_logic, toggle_mark(state, MARK_UNDERLINE)); true }
        "z" if ctrl && !shift => { send_cmd(to_logic, ProseCommand::Undo); true }
        "z" if ctrl &&  shift => { send_cmd(to_logic, ProseCommand::Redo); true }
        "y" if ctrl => { send_cmd(to_logic, ProseCommand::Redo); true }

        // Slash menu: opening `/` at block start raises a Leptos signal (DOM chrome menu),
        // it is NOT a canvas overlay (see section G).
        "/" if state.cursor_at_block_start() => {
            state.open_slash_menu(state.cursor_position());  // -> Leptos signal
            false // the `/` still flows to beforeinput so the model records it
        }
        _ => false, // printable text is handled by beforeinput / composition below
    }
}
```

Printable Latin text is **not** matched key-by-key here: it flows through the textarea's
`beforeinput` event, which `ambre-app` turns into `InputEvent::TextInput { text }`. The view
then pushes it to the draft buffer and emits `ProseCommand::InsertText`, and the host clears
the textarea (`ambre-spec-chrome.md §2.C`).

```rust
// InputEvent::TextInput { text } branch (committed, non-composition input)
pub fn handle_text_input(text: &str, state: &mut ProseUiState, to_logic: &flume::Sender<LogicCommand>) {
    let (block_id, pos) = (cursor_block(state), cursor_offset(state));
    state.draft_buffer.push_str(text, &block_id, pos);
    send_cmd(to_logic, ProseCommand::InsertText { block_id, pos, text: text.to_string() });
    advance_cursor(state, text.chars().count() as u32);
}
```

#### IME (Input Method Editor)

IME (Japanese, Korean, Chinese, Vietnamese…) is **not owned by `prose-ui`**: it is handled by
the shared transparent `<textarea>` pipe in `ambre-chrome` (`ambre-spec-chrome.md §2.C`). The
textarea captures `compositionstart → compositionupdate* → compositionend`; `ambre-app`
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
) {
    match phase {
        CompositionPhase::Start  => state.draft_buffer.begin_composition(cursor_block(state), cursor_offset(state)),
        CompositionPhase::Update => state.draft_buffer.set_composition(text),   // canvas preview only
        CompositionPhase::End    => {
            let (block_id, pos) = state.draft_buffer.end_composition();
            send_cmd(to_logic, ProseCommand::InsertText { block_id, pos, text: text.to_string() });
        }
    }
}
```

#### Mouse & Touch

```rust
// ambre-prose/src/ui/input/pointer.rs

pub fn handle_pointer_event(event: &PointerEvent, state: &mut ProseUiState) {
    match event.kind {
        PointerDown => {
            let pos = screen_to_doc(event.x, event.y, &state.viewport);
            if let Some((block_id, offset)) = hit_test(pos, state) {
                state.cursor = Some(TextCursor::at(block_id, offset));
                state.selection = None;
                state.toolbar.hide();
                state.slash_menu = None;
            }
        }
        PointerMove if event.buttons & 1 != 0 => {
            // Drag -> selection
            let pos = screen_to_doc(event.x, event.y, &state.viewport);
            if let Some((block_id, offset)) = hit_test(pos, state) {
                if let Some(cursor) = &state.cursor {
                    state.selection = Some(TextSelection {
                        anchor: (cursor.block_id.clone(), cursor.offset),
                        focus:  (block_id, offset),
                    });
                }
            }
        }
        PointerUp => {
            if state.selection.is_some() {
                state.toolbar.show_at(selection_bounds(state));
            }
        }
        DoubleClick => {
            // Select the word under the cursor
            if let Some((block_id, offset)) = hit_test(screen_to_doc(event.x, event.y, &state.viewport), state) {
                let (word_start, word_end) = word_boundaries_at(&state.runs, &block_id, offset);
                state.selection = Some(TextSelection {
                    anchor: (block_id.clone(), word_start),
                    focus:  (block_id, word_end),
                });
                state.toolbar.show_at(selection_bounds(state));
            }
        }
        _ => {}
    }
}
```

**Hit testing:**

Hit test converts a screen coordinate to `(TreeID, offset)` in the text.
It uses `BlockLayout` to find the target block (O(log n) by bisection on
`y_in_page`), then Parley metrics (`hit_test_point()`) for the offset in the
`LoroText`.

---

### F. Contextual Toolbar (DOM chrome)

The contextual toolbar is a **DOM Leptos component** (`prose_chrome::FormatBar`, built on
`ambre-chrome`), like Trame's `FormatBar` — **not** a canvas `DrawSurface` overlay
(`ambre-spec-ui.md §2.A`, `ambre-spec-chrome.md §2.B`). It appears anchored above the
selection on `PointerUp` with a non-empty selection and hides when the selection is canceled.
The canvas view computes the anchor rectangle and raises a Leptos signal (Canvas → DOM via
signals, `ambre-spec-ui.md §2.C`); the toolbar element is **transform-synced** to that anchor
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

`B` `I` `U` `S` `<>` (inline code) | `🔗` (link) | `H1` `H2` `H3` (change block type)
| `"` (blockquote) | `···` (more → color picker, font size)

Each button invokes a `CommandId` (`ambre-commands`) resolved to a `ProseCommand`:
- Toggle mark → `ProseCommand::ApplyMark` or `ProseCommand::RemoveMark`.
- Change block type → `ProseCommand::SetBlockStyle`.

The toolbar is a DOM Leptos element (`prose_chrome::FormatBar`), transform-synced to the
selection anchor — no canvas z-order concern, the browser compositor layers DOM over canvas.
Overflow (`⋯`) and mobile bottom-sheet behavior follow `ambre-spec-chrome.md §2.B`.

---

### G. Slash Command (`/`)

The slash command opens when `/` is typed at the beginning of a block (empty block or
position 0). It is a **DOM Leptos menu** (`prose_chrome`, on `ambre-chrome`'s menu
primitives — roving tabindex, flip-on-overflow), anchored under the caret via a Leptos signal
the canvas view raises (Canvas → DOM). It is **not** a canvas overlay. It displays a
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
| Text | ¶ | `InsertBlock(Paragraph)` |
| Heading 1 | H1 | `SetHeading(1)` |
| Heading 2 | H2 | `SetHeading(2)` |
| Heading 3 | H3 | `SetHeading(3)` |
| Bullet list | • | `InsertBlock(ListItem { Bullet })` |
| Numbered list | 1. | `InsertBlock(ListItem { Ordered })` |
| Task list | ☑ | `InsertBlock(ListItem { Todo })` |
| Table | ⊞ | `InsertTable { rows: 3, cols: 3 }` |
| Code | `</>` | `SetCodeBlock { lang: None }` |
| Separator | — | `InsertHRule` |
| Page break | ⬜ | `InsertPageBreak` |
| Table of contents | ≡ | `InsertToc` |

Selection via `↑` `↓` `Enter`. Close via `Escape` or click outside the menu.
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
chrome (independent of vertical canvas scroll) — not a canvas `DrawSurface` band. Its drag
handles emit `ProseCommand::SetParagraphStyle` on `on_pointer_up`.

---

### I. Collaborative Cursors

Remote cursors are received via `ProseEvent::RemoteCursors` from the Logic thread.
The author identity is the **Sync-resolved `from_user_id`** (spoof-proof,
`ambre-spec-sync.md §3`): `ambre-sync` decrypts the `Ephemeral` message and resolves
the canonical `UserId` before delivery, so the Logic thread never resolves identity
itself. The peer-authored payload carries only the cursor position and a self-asserted,
cosmetic `display_name` — never a `user_id`. Each peer is assigned a color derived from
that `UserId` via the **shared cross-app helper** `ambre_draw::peer::peer_color` — the
same function Trame and Toile call, so a given collaborator has the same color across
all apps in the suite. Prose does not reimplement the hash (see `ambre-spec-canvas.md §D`).

```rust
// ambre-prose/src/ui/remote_cursor.rs

fn user_color(user_id: &UserId) -> peniko::Color {
    // Canonical cross-app peer color -- seeded on the spoof-proof UserId.
    ambre_draw::peer::peer_color(user_id)
}
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

The `✓ Accept` / `✗ Reject` affordance is a **DOM Leptos popover** (`prose_chrome`),
anchored (transform-synced) to the hovered suggestion via a Leptos signal the view raises;
its buttons invoke `ProseCommand::AcceptSuggestion` / `RejectSuggestion`. It is not a canvas
overlay.

---

### K. Clipboard

Prose uses the shared, **Logic-authoritative, host-driven** clipboard model
(`ambre-spec-ui.md §2.E`). `prose-ui` **never** calls `navigator.clipboard` — the host
(`ambre-app`) performs the OS clipboard I/O inside the user-activation window. The transport
type is the app-agnostic `ClipboardBlob { text: String, ambre: Option<Vec<u8>> }` (`ambre-bus`):
`text` is the `text/plain` mirror; `ambre` is Prose's opaque payload (postcard of a typed
Prose clipboard payload — rich runs + block structure) under the private `application/x-ambre`
MIME type. There is **no `text/html`** on the OS clipboard and **no viewport culling** — the
full-fidelity payload is built by the Logic thread from the whole CRDT.

```rust
// ambre-prose/src/ui/clipboard.rs -- driver methods called by FrameHost (no web_sys here)

/// Native copy/cut: request the authoritative build from Logic; nothing serialized UI-side.
pub fn clipboard_write(&mut self, cut: bool) -> CopyOutcome {
    match &self.selection {
        None => CopyOutcome::Nothing,                    // browser default proceeds
        Some(sel) => {
            self.cut_pending = cut;
            self.send_cmd(ProseCommand::CopySelection { anchor: sel.anchor.clone(), focus: sel.focus.clone() });
            CopyOutcome::Deferred                         // host prevent_default()s, no setData yet
        }
    }
}

/// Called by the host right after update(): if Logic replied (ProseEvent::Clipboard),
/// return the ClipboardBlob to mirror to the OS clipboard next frame (poll_clipboard).
pub fn poll_clipboard(&mut self) -> Option<ClipboardBlob> {
    self.pending_clipboard.take()   // set in update() from ProseEvent::Clipboard { text, ambre }
}

/// Native paste: prefer the in-memory blob when the OS text mirror still matches it
/// (full fidelity), else decode blob.ambre; for a foreign text/html source the host passes
/// its bytes and the view routes them through import_html.
pub fn paste_clipboard(&mut self, blob: ClipboardBlob) {
    let (block_id, pos) = self.cursor_target();
    match self.decode_ambre(&blob) {
        Some(payload) => self.send_cmd(ProseCommand::PasteRich { block_id, pos, payload }),
        None if blob.is_html() =>
            self.send_cmd(ProseCommand::ImportDocument { format: ImportFormat::Html, data: blob.html_bytes() }),
        None => self.send_cmd(ProseCommand::InsertText { block_id, pos, text: blob.text }),
    }
}
```

The Logic thread builds `ProseEvent::Clipboard { text, ambre }` from the CRDT selection
(`prose-logic`); the view stashes it in memory (same-tab full-fidelity paste) and hands it to
the host via `poll_clipboard`. **Cut** clears the source range after the copy command, Prose's
own policy.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

| Flow | Data | Risk | Mitigation |
|------|---------|--------|------------|
| `flume` UI → Logic thread | Typed `ProseCommand` (in `LogicCommand::App`) | Plaintext data in the **shared** WASM heap of the threads | Normal: the heap is only accessible from inside the user's browser process (sandbox). The value is **moved** between threads — no serialization, no boundary copy, no external exposure. |
| Clipboard write (host `navigator.clipboard.write`, `ClipboardBlob`) | `text/plain` mirror + opaque `x-ambre` payload of the selection | The browser may expose the clipboard to other tabs | Browser standard — outside AMBRE's control; `prose-ui` never touches `navigator.clipboard` (host-driven, `ambre-spec-ui.md §2.E`) |
| Clipboard paste (foreign `text/html` source) | HTML pasted from an external source | Malicious HTML | Routed through `import_html` (html5ever sanitizes) |
| Remote cursors | `display_name` received via `ProseEvent::RemoteCursors` | Already decrypted in the Logic thread | No additional exposure |

The UI thread never has access to cryptographic keys or the `LoroDoc`. It only displays
already-decrypted data transmitted by the Logic thread.

---

## 4. QUALITY & TESTABILITY

**Unit tests (native, `cfg(not(target_arch = "wasm32"))`):**

| Case | Method |
|-----|---------|
| `visible_pages()`: viewport overlaps pages 2 and 3 | `PageColumn::visible_pages()` with fixed values |
| `hit_test()`: click at known coordinate → expected `(block_id, offset)` | Mock `BlockLayout` + mocked Parley metrics |
| `DraftBuffer`: push 5 chars → clear → empty | Direct assertions |
| `ambre_draw::peer::peer_color`: two distinct user_ids → distinct colors | Hash collision check on 1000 ids (owned by `ambre-draw`; Prose only delegates) |
| `compute_selection_bounds()`: single-block and multi-block selection | Mocked Parley geometry |
| Keyboard shortcuts: Ctrl+B → `ApplyMark(bold)`, Ctrl+Z → `Undo` | `KeyEvent` simulation on `handle_key_event()` |
| Slash menu: filtering by query "tab" → items containing "tab" | `SlashMenuState::filter()` |
| IME: compositionstart → update → end → `InsertText` | Simulation of 3 events |

**WASM tests (`wasm-bindgen-test`):**

| Case | Method |
|-----|---------|
| wgpu rendering: page rendered without panic on 800×600 canvas | `wasm-bindgen-test` with headless canvas |
| `flume` UI → Logic thread: `LogicCommand::App` emitted and routed to `ProseState` | Mock `flume` channels (host test) |
| `paste_clipboard`: `ClipboardBlob` with `ambre` → `PasteRich`; foreign `text/html` → `ImportDocument { format: Html }`; plain text → `InsertText` | Host-driven, `ClipboardBlob` fixtures (no `navigator.clipboard` in `prose-ui`) |

**Visual tests (snapshots):**

Snapshots of rendered pages via `QuadRenderer` compared to reference snapshots (PNG).
Tool: custom cargo script that exports the display list as PNG via `wgpu` headless.
Tolerance: pixel diff ≤ 2% (anti-aliasing varies by GPU).

---

## 5. PERFORMANCE & RESOURCES

**Frame budget (16ms at 60 FPS):**

| Step | Budget | Notes |
|-------|--------|-------|
| `build_display_list()` (3 visible pages) | < 4 ms | `BlockLayout` traversal + pre-shaped run assembly |
| Build vertex batch + GPU upload | < 4 ms | QuadRenderer + wgpu |
| Submit GPU + present frame | < 6 ms | GPU-bound, varies by hardware |
| **Total frame budget** | **< 14 ms** | 2ms margin for interleaved input events |

**Input latency (P2 — high priority):**

| Operation | Target |
|-----------|-------|
| Keystroke → draft display (UI thread only) | < 2 ms |
| Keystroke → CRDT confirmation (Logic thread round-trip) | < 15 ms |
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

**WASM size (UI thread — note: wgpu/lyon/Parley/swash are shared across all apps):**

| Component | Raw estimated | Gzip estimated |
|-----------|-----------|-------------|
| `ambre-prose/ui` (Prose UI logic) | ~50 KB | ~16 KB |
| `wgpu` + `lyon` + `swash` (shared) | ~800 KB | ~250 KB |
| `parley` (shared) | ~200 KB | ~60 KB |

> wgpu/lyon/Parley/swash are shared dependencies across all suite applications —
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

**Fundamental invariant:** the UI thread contains no document data — only view states
and data received from the Logic thread via `ProseEvent`. Every mutation goes through
a `ProseCommand`. There are no two models of the document: `LoroDoc` is the single
source of truth.

**Documented edge cases:**
- **Scroll position:** local only. Two collaborators have independent scroll positions — identical to Google Docs.
- **Selection:** local. The remote selection is displayed via `RemoteCursor` (encrypted ephemeral), not stored in `LoroDoc`.
- **Draft buffer:** UI thread transient state only — gone on disconnect. Never sent over the network.
- **`layout_version`:** derived from the Logic thread, used only for UI anti-flickering — no business meaning.

---

## 7. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **Parley shaping in the UI thread** | Shaping is O(n_glyphs) and can exceed 16 ms on a long block. Crucially, **pagination already shapes every block in the Logic thread** (`prose-logic §2.B`) to measure page breaks — so shaping in the UI would *duplicate* that work and risk blocking the render thread (violating the "render thread never blocked by computation" pillar). The UI receives pre-shaped `ParleyRunData` and only draws glyphs. This is a deliberate Prose-specific departure from the generic UI-thread `TextCache` shaping path (`ambre-spec-canvas.md §D`, used for Trame single-line cells); the Foundation Prose-frame budgets are annotated accordingly (`ambre-spec-chrome.md §5.A`, `ambre-spec-canvas.md §5.A`). |
| **DOM/HTML for editor rendering** | Rejected. Unpredictable layout, impossible to control pagination, JS/WASM interop cost. |
| **`contentEditable`** | Non-deterministic behavior across browsers, inconsistent IME handling, impossible to control for pagination and virtual scroll. |
| **Full-page rendering each frame** | 100 pages × full rendering = blown budget. Virtual scroll with culling of non-visible pages is mandatory. |
| **Chrome (toolbar, slash menu, ruler, comment panel) drawn on the canvas via `DrawSurface`** | Rejected per the Foundation canvas/DOM boundary (`ambre-spec-ui.md §2.A`): chrome is DOM/Leptos (`ambre-chrome`/`prose_chrome`), transform-synced to canvas anchors. Static DOM over a canvas is one layout pass then zero-cost compositor recomposition, and inherits ARIA a11y, focus trap, and IME for free — none of which a canvas widget gets. Only **content** (caret, selections, highlights, remote cursors) stays on the canvas. |
| **`winit` for input** | Web-only target: winit is indirection without benefit and conflicts with DOM event ownership. Events are captured directly via `web-sys` (DOM listeners + the transparent `<textarea>`) and delivered as `InputEvent` (`ambre-spec-ui.md §6`, `ambre-spec-chrome.md §2.C`). |
| **`prose-ui` owning `navigator.clipboard` + a `text/html` clipboard payload** | Clipboard is host-driven and Logic-authoritative (`ambre-spec-ui.md §2.E`): the host performs OS clipboard I/O in the activation window; the payload is `ClipboardBlob { text, ambre }` (opaque `x-ambre`), built from the full CRDT — not `text/html`, not viewport-culled. |
| **Canvas 2D (non-WebGL)** | Insufficient performance for high-quality typographic rendering. wgpu (WebGL2/WebGPU) is AMBRE's foundational choice. |
| **Synchronous CRDT confirmation before display** | Logic thread round-trip ~1–3ms perceptible at high typing cadence. The draft buffer absorbs this latency without compromising consistency. |
| **Selection via `window.getSelection()` DOM** | Outside WASM context. Selection is managed entirely via Parley metrics and pointer events. |
| **`requestIdleCallback` in the Logic thread** | Not available in a `wasm_thread` worker (the Logic thread). Deferred/low-priority work there uses the P1–P4 priority scheduler of `ambre-logic`, not `requestIdleCallback`. (On the UI thread, where it *is* available, the host may use it for opportunistic layout — `ambre-spec-chrome.md §5.B`.) |
| **Layouts without versioning** | Without `version: u64` on `LayoutUpdated`, two pagination recalculations arriving out of order would produce stale display. The UI thread silently drops any `LayoutUpdated` whose version is lower than the last applied layout. |

---

## 8. INTEGRATION PLAN (ROADMAP)

**Phase 0 — Static Rendering (Week 1)**
- [ ] `wgpu` + `QuadRenderer` setup on `HTMLCanvasElement` in the UI thread.
- [ ] `PageColumn` + `visible_pages()` + basic virtual scroll (vertical scroll only).
- [ ] Rendering of a mocked `PageLayout`: page background, shadows, page numbers.
- [ ] Rendering of pre-shaped Parley runs received from the Logic thread (static text, no cursor).
- [ ] `RunCache` with LRU eviction.
- [ ] Visual snapshot tests: page rendered correctly.

**Phase 1 — Input & Cursor (Week 2)**
- [ ] `handle_key_event()`: cursor navigation (arrows, Home, End), printable characters.
- [ ] `DraftBuffer`: optimistic display + clear on `DocUpdated`.
- [ ] `hit_test()`: mouse click → `(block_id, offset)`.
- [ ] `handle_pointer_event()`: PointerDown cursor, PointerMove selection, PointerUp toolbar.
- [ ] `DrawSurface` rendering of blinking cursor (2px fill_rect) and selection (alpha fill_rect).
- [ ] `flume` UI → Logic thread send: `LogicCommand::App { command: AppCommand::Prose(ProseCommand) }` (`try_send`, non-blocking).
- [ ] Prose `AppView::Prose` variant: inherent `update` (drain `UiEvent` `flume`) / `plan_repaint` / `paint_content` / `paint_overlay` dispatched by `FrameHost`.
- [ ] Unit tests keyboard, hit_test, draft buffer.

**Phase 2 — Toolbar, Slash, IME (Week 3)**
- [ ] Contextual toolbar: DOM Leptos `prose_chrome::FormatBar`, transform-synced to the selection anchor; toggle marks, change block type via `ambre-commands`.
- [ ] Slash command: DOM Leptos menu anchored under the caret, filtering, selection, block insertion.
- [ ] IME: consume `InputEvent::Composition` from the shared transparent `<textarea>` (`ambre-chrome`); canvas composition preview; `InsertText` on `compositionend`.
- [ ] Double-click → word selection.
- [ ] Unit tests slash menu, IME, toolbar-state derivation.

**Phase 3 — Collaboration & Annotations (Week 4)**
- [ ] Remote cursors: `RemoteCursors` receipt, bar + label + selection rendering, animation (canvas content).
- [ ] Comment inline highlights (canvas) + DOM Leptos side panel (thread list).
- [ ] Track changes highlights (green/red, canvas) + DOM accept/reject popover on hover.
- [ ] Unit tests `user_color()`, highlight rendering.

**Phase 4 — Ruler, Clipboard, Inertial Scroll (Week 5)**
- [ ] Interactive ruler: DOM Leptos `prose_chrome` component, drag handles, `SetParagraphStyle` on pointer up.
- [ ] Clipboard: host-driven `ClipboardBlob` flow (`clipboard_write`/`poll_clipboard`/`paste_clipboard`); `ProseEvent::Clipboard` build in Logic; foreign `text/html` paste → `import_html`.
- [ ] Inertial scroll: `scroll_velocity`, physical deceleration (linear friction).
- [ ] Optimizations: frame budget profiling, reducing allocations on `build_display_list()` hot path.

**Phase 5 — Spell Check UI (post-MVP)**
- [ ] `ProseEvent::SpellErrors` receipt, wavy underline rendering via `fill_path` lyon.
- [ ] Right-click context menu on underlined word: suggestions, "Add to dictionary".
