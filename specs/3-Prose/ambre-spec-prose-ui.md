# AMBRE — SPEC: PROSE / UI
**Type:** Vertical (Feature/Widget)
**Status:** Draft

> Prose presentation layer: wgpu/Parley rendering (DrawSurface), interactive rich editor
> (cursor, selection, IME), page-based virtual scroll, floating toolbar, slash command,
> collaborative cursors, comment display, and track changes. Everything runs in the UI
> thread — no CRDT mutation is performed directly here.

> **Model note (Model B).** **3-thread shared-memory architecture** (`flume` channels, single WASM binary) — see `ambre-spec-architecture.md`. For this spec:
> - **Typed `flume` transport, no `postMessage`/postcard.** The UI sends `LogicCommand::App { document_id, command: AppCommand::Prose(ProseCommand) }` and receives `UiEvent::Viewport/Presence { event: AppEvent::Prose(ProseEvent) }` — Rust values **by move** on `flume`, no serialization on the UI side.
> - **The Prose view is a `DocView`.** Like `TrameView`, Prose presentation implements the `DocView` trait from `ambre-canvas` (`FrameHost` driver: `update`/`plan_repaint`/`paint_content`/`paint_overlay`) — see `ambre-spec-canvas.md`. The GPU pipeline (`QuadRenderer`, `WgpuSurface`) and the `host` live in `ambre-canvas`.
> - Memory is **shared** (single heap): there is no longer "memory isolation per Worker". The `ProseCommand` is **not copied** between threads — it is moved.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles:
- wgpu/Parley rendering (DrawSurface) of visible pages in the viewport (display list).
- Virtual scroll: culling of pages outside the viewport, smooth scrolling management.
- Text cursor, selection (mouse, keyboard, touch), and IME management.
- Contextual floating toolbar (selection → inline formatting).
- Slash command (`/`) for block insertion.
- Interactive ruler for margins and indents.
- Display of remote collaborative cursors.
- Display of comments (highlights + side panel) and track changes.
- Clipboard management (copy/paste text and HTML).

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
| `web_sys` | `Clipboard API`, `navigator.clipboard`, `InputEvent` (IME), `onresize` |
| `flume` | Inter-thread channels: send `LogicCommand`, receive `UiEvent` (typed values by move) |
| `ambre-bus` | Typed IPC types: `LogicCommand`/`UiEvent`, `ProseCommand`/`ProseEvent`, `AppCommand`/`AppEvent` |
| `ambre-canvas` | Frame driver (`FrameHost`/`DocView`), GPU pipeline (`QuadRenderer`, `WgpuSurface`) |
| `ambre-draw` | `DrawSurface` trait (parameterized screen/PDF rendering) |

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
/// (type `ParleyRunData`, postcard-serializable) and converted to `ParleyRun`
/// (UI type, with Parley layout cache) on receipt.
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

/// UI-side Parley run type. Converted from `ParleyRunData` (wire type)
/// on receipt of `ProseEvent::RunsUpdated`.
pub struct ParleyRun {
    pub data:   ParleyRunData,            // serialized data (glyphs, positions, metrics)
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

```rust
// ambre-prose/src/ui/input/keyboard.rs

pub fn handle_key_event(
    event: &winit::event::KeyEvent,
    state: &mut ProseUiState,
    to_logic: &flume::Sender<LogicCommand>,   // channel to the Logic thread (try_send)
) {
    use winit::keyboard::{Key, NamedKey, ModifiersState};

    match (&event.logical_key, event.state) {
        // Cursor navigation
        (Key::Named(NamedKey::ArrowLeft),  Pressed) => move_cursor(state, Direction::Left,  event.modifiers),
        (Key::Named(NamedKey::ArrowRight), Pressed) => move_cursor(state, Direction::Right, event.modifiers),
        (Key::Named(NamedKey::ArrowUp),    Pressed) => move_cursor(state, Direction::Up,    event.modifiers),
        (Key::Named(NamedKey::ArrowDown),  Pressed) => move_cursor(state, Direction::Down,  event.modifiers),
        (Key::Named(NamedKey::Home),       Pressed) => move_cursor(state, Direction::LineStart, event.modifiers),
        (Key::Named(NamedKey::End),        Pressed) => move_cursor(state, Direction::LineEnd,   event.modifiers),

        // Editing
        (Key::Named(NamedKey::Backspace),  Pressed) => send_cmd(cmd_tx, delete_backward(state)),
        (Key::Named(NamedKey::Delete),     Pressed) => send_cmd(cmd_tx, delete_forward(state)),
        (Key::Named(NamedKey::Enter),      Pressed) => send_cmd(cmd_tx, split_block(state)),
        (Key::Named(NamedKey::Tab),        Pressed) => send_cmd(cmd_tx, indent_or_next_cell(state)),

        // Formatting shortcuts
        (Key::Character("b"), Pressed) if ctrl(event) => send_cmd(cmd_tx, toggle_mark(state, MARK_BOLD)),
        (Key::Character("i"), Pressed) if ctrl(event) => send_cmd(cmd_tx, toggle_mark(state, MARK_ITALIC)),
        (Key::Character("u"), Pressed) if ctrl(event) => send_cmd(cmd_tx, toggle_mark(state, MARK_UNDERLINE)),
        (Key::Character("z"), Pressed) if ctrl(event) && !shift(event) => send_cmd(cmd_tx, ProseCommand::Undo),
        (Key::Character("z"), Pressed) if ctrl(event) && shift(event)  => send_cmd(cmd_tx, ProseCommand::Redo),
        (Key::Character("y"), Pressed) if ctrl(event) => send_cmd(cmd_tx, ProseCommand::Redo),

        // Slash command
        (Key::Character("/"), Pressed) if state.cursor_at_block_start() => {
            state.slash_menu = Some(SlashMenuState::open(state.cursor_position()));
        }

        // Printable character -> draft buffer + ProseCommand
        (Key::Character(c), Pressed) if !ctrl(event) => {
            let c = c.chars().next().unwrap();
            state.draft_buffer.push_char(c, &cursor_block(state), cursor_offset(state));
            send_cmd(cmd_tx, ProseCommand::InsertText {
                block_id: cursor_block(state),
                pos:      cursor_offset(state),
                text:     c.to_string(),
            });
            advance_cursor(state, 1);
        }

        _ => {}
    }
}
```

#### IME (Input Method Editor)

IME (Japanese, Korean, Chinese, Arabic…) is handled via the `compositionstart`,
`compositionupdate`, `compositionend` events of `web_sys`. During composition, the
intermediate text is displayed in the draft buffer with a composition underline (style
distinct from mark underlines). `ProseCommand::InsertText` is only sent on
`compositionend`.

```rust
// ambre-prose/src/ui/input/ime.rs

pub struct ImeState {
    pub composing:      bool,
    pub composition:    String,   // intermediate composition text
    pub comp_start_pos: u32,
}

impl ImeState {
    pub fn on_composition_start(&mut self) {
        self.composing = true;
        self.composition.clear();
    }

    pub fn on_composition_update(&mut self, data: &str) {
        self.composition = data.to_string();
        // Display in draft buffer with composition style
    }

    pub fn on_composition_end(&mut self, data: &str) -> ProseCommand {
        self.composing = false;
        // Send the confirmed final text
        ProseCommand::InsertText {
            block_id: todo!(),
            pos:      self.comp_start_pos,
            text:     data.to_string(),
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

### F. Floating Toolbar

The floating toolbar appears above the selection on `PointerUp` with a non-empty
selection. It disappears as soon as the selection is canceled.

```rust
// ambre-prose/src/ui/toolbar.rs

#[derive(Debug, Clone, Default)]
pub struct ToolbarState {
    pub visible:  bool,
    pub position: Vec2,        // screen coordinates of the selection's bottom-center
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

Each button sends a `ProseCommand` to the Logic thread:
- Toggle mark → `ProseCommand::ApplyMark` or `ProseCommand::RemoveMark`.
- Change block type → `ProseCommand::SetBlockStyle`.

The toolbar is rendered via `DrawSurface` (not DOM/HTML) — overlay in screen coordinates,
z-order above pages.

---

### G. Slash Command (`/`)

The slash command opens when `/` is typed at the beginning of a block (empty block or
position 0). It displays a filterable menu of block types and templates.

```rust
// ambre-prose/src/ui/slash_menu.rs

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    pub position:  Vec2,        // cursor position at open
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

The ruler is rendered via `DrawSurface` in screen coordinates (fixed position at the
top, independent of vertical scroll).

---

### I. Collaborative Cursors

Remote cursors are received via `ProseEvent::RemoteCursors` from the Logic thread
(which decrypts them from `Ephemeral` messages and resolves `pub_key → UserId` via
`ambre-sync/trusted_keys`). Each peer is assigned a color derived from their `UserId`
(hash → HSL palette) — identical to Trame's algorithm so the same collaborator has
the same color across all apps in the suite.

```rust
// ambre-prose/src/ui/remote_cursor.rs

fn user_color(user_id: &Uuid) -> u32 {
    // Deterministic: hash of UserId -> hue in HSL (saturation 70%, lightness 55%)
    // Returns RGBA u32. Aligned with color_for_user() from ambre-spec-trame-ui.
    let bytes = user_id.as_bytes();
    let hue = (u64::from_le_bytes(bytes[..8].try_into().unwrap())
        .wrapping_mul(2654435761) >> 32) as f32
        / u32::MAX as f32 * 360.0;
    hsla_to_rgba(hue, 0.70, 0.55, 1.0)
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
1. **Inline highlight:** semi-transparent yellow rectangle under the annotated range (mark `"cm"`).
2. **Side panel:** list of comment threads, vertically aligned with their anchor in the text.

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

The side panel is a `DrawSurface` overlay to the right of the page, independently
scrollable. A click on a comment in the panel positions the document scroll on the
corresponding anchor.

#### Track Changes

Suggestions are displayed inline in the text:
- **Suggested insertion:** green underlined text, very light green background.
- **Suggested deletion:** red strikethrough text, very light red background.

A floating banner appears on hover over the suggestion with `✓ Accept` /
`✗ Reject` buttons, which send `ProseCommand::AcceptSuggestion` or
`ProseCommand::RejectSuggestion`.

---

### K. Clipboard

```rust
// ambre-prose/src/ui/clipboard.rs

/// Copy: produce text/plain and text/html from the current selection.
pub async fn copy_selection(state: &ProseUiState) {
    let (text, html) = render_selection_to_clipboard(state);
    let clipboard = web_sys::window().unwrap().navigator().clipboard();
    // ClipboardItem with both formats
    let item = web_sys::ClipboardItem::new_with_record_from_str(&[
        ("text/plain", &text),
        ("text/html",  &html),
    ]);
    clipboard.write(&item.into()).await.ok();
}

/// Paste: prefer text/html (richer), fallback text/plain.
pub async fn paste(state: &ProseUiState, cmd_tx: &JsValue) {
    let clipboard = web_sys::window().unwrap().navigator().clipboard();
    let items = clipboard.read().await.ok();
    // Attempt text/html -> ProseCommand::ImportDocument { format: Html, data }
    // Fallback text/plain -> ProseCommand::InsertText { text }
}
```

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

| Flow | Data | Risk | Mitigation |
|------|---------|--------|------------|
| `flume` UI → Logic thread | Typed `ProseCommand` (in `LogicCommand::App`) | Plaintext data in the **shared** WASM heap of the threads | Normal: the heap is only accessible from inside the user's browser process (sandbox). The value is **moved** between threads — no serialization, no boundary copy, no external exposure. |
| Clipboard `navigator.clipboard.write()` | Text and HTML of the selection | The browser may expose the clipboard to other tabs | Browser standard — outside AMBRE's control |
| Clipboard `navigator.clipboard.read()` | HTML pasted from an external source | Malicious HTML | Systematic passage through `import_html` (html5ever sanitizes) |
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
| `user_color()`: two distinct user_ids → distinct colors | Hash collision check on 1000 ids |
| `compute_selection_bounds()`: single-block and multi-block selection | Mocked Parley geometry |
| Keyboard shortcuts: Ctrl+B → `ApplyMark(bold)`, Ctrl+Z → `Undo` | `KeyEvent` simulation on `handle_key_event()` |
| Slash menu: filtering by query "tab" → items containing "tab" | `SlashMenuState::filter()` |
| IME: compositionstart → update → end → `InsertText` | Simulation of 3 events |

**WASM tests (`wasm-bindgen-test`):**

| Case | Method |
|-----|---------|
| wgpu rendering: page rendered without panic on 800×600 canvas | `wasm-bindgen-test` with headless canvas |
| `flume` UI → Logic thread: `LogicCommand::App` emitted and routed to `ProseState` | Mock `flume` channels (host test) |
| Clipboard paste: HTML → `ImportDocument { format: Html }` | Mock `navigator.clipboard` |

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
| **Parley shaping in the UI thread** | Shaping is O(n_glyphs) and can exceed 16ms on a long block. Moving it to the Logic thread frees the UI thread for pure rendering, guaranteeing 60 FPS even on mobile. |
| **DOM/HTML for editor rendering** | Rejected. Unpredictable layout, impossible to control pagination, JS/WASM interop cost. |
| **`contentEditable`** | Non-deterministic behavior across browsers, inconsistent IME handling, impossible to control for pagination and virtual scroll. |
| **Full-page rendering each frame** | 100 pages × full rendering = blown budget. Virtual scroll with culling of non-visible pages is mandatory. |
| **Floating DOM/HTML toolbar** | Would require leaving WASM context for a single widget. `DrawSurface` toolbar maintains rendering consistency. |
| **Canvas 2D (non-WebGL)** | Insufficient performance for high-quality typographic rendering. wgpu (WebGL2/WebGPU) is AMBRE's foundational choice. |
| **Synchronous CRDT confirmation before display** | Logic thread round-trip ~1–3ms perceptible at high typing cadence. The draft buffer absorbs this latency without compromising consistency. |
| **Selection via `window.getSelection()` DOM** | Outside WASM context. Selection is managed entirely via Parley metrics and pointer events. |
| **`requestIdleCallback` for shaping** | Not available in WASM Workers. The P1–P4 priority scheduler of `ambre-app` replaces `requestIdleCallback` in the Logic thread. |
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
- [ ] Prose `DocView`: `update` (drain `UiEvent` `flume`) / `plan_repaint` / `paint_content` / `paint_overlay` wired to `FrameHost`.
- [ ] Unit tests keyboard, hit_test, draft buffer.

**Phase 2 — Toolbar, Slash, IME (Week 3)**
- [ ] Floating toolbar: display on selection, toggle marks, change block type.
- [ ] Slash command: menu, filtering, selection, block insertion.
- [ ] IME: `compositionstart`/`update`/`end`, composition draft.
- [ ] Double-click → word selection.
- [ ] Unit tests slash menu, IME, toolbar.

**Phase 3 — Collaboration & Annotations (Week 4)**
- [ ] Remote cursors: `RemoteCursors` receipt, bar + label + selection rendering, animation.
- [ ] Comment highlights + side panel (thread list).
- [ ] Track changes highlights (green/red) + accept/reject banner on hover.
- [ ] Unit tests `user_color()`, highlight rendering.

**Phase 4 — Ruler, Clipboard, Inertial Scroll (Week 5)**
- [ ] Interactive ruler: `DrawSurface` rendering, drag handles, `SetParagraphStyle` on pointer up.
- [ ] Clipboard: copy (text/plain + text/html), paste (HTML preference → import_html).
- [ ] Inertial scroll: `scroll_velocity`, physical deceleration (linear friction).
- [ ] Optimizations: frame budget profiling, reducing allocations on `build_display_list()` hot path.

**Phase 5 — Spell Check UI (post-MVP)**
- [ ] `ProseEvent::SpellErrors` receipt, wavy underline rendering via `fill_path` lyon.
- [ ] Right-click context menu on underlined word: suggestions, "Add to dictionary".
