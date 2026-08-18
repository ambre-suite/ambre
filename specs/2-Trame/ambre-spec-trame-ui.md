# AMBRE — SPEC: AMBRE-TRAME UI (Rendering & Interactions)
**Type:** Vertical (Feature / Application — UI)
**Status:** Draft
**Implementation:** partial

> Rendering and interaction layer of the Trame spreadsheet. Runs exclusively in the **UI thread**. Receives typed [`TrameEvent`](../../crates/ambre-bus/src/app/trame/event.rs) values (carried by move inside `AppEvent::Trame` over the shared-memory `flume` plane -- **no** `postMessage`, **no** postcard on this hop; `-logic.md` §9, `spec-ambre-architecture.md §2/§6`) from the Logic thread, renders the grid via `WgpuSurface` (QuadRenderer + GlyphAtlas), and translates user events into `TrameCommand` sent back to the Logic thread the same way. PDF export will be generated in the **Logic thread** via `KrillaSurface` (implementation of the shared `DrawSurface` trait) so as not to block the UI thread.
>
> This spec covers: virtual scrolling, wgpu/lyon/Parley/Swash rendering pipeline, inline editing, formula bar, autocompletion, freeze, sparklines, charts, multi-user cursors, and PDF export via `krilla`.
>
> **Implementation status:** basic grid rendering operational (headers, rows, cells, text, active cell, FPS overlay). Inline editing, formula bar, freeze, sparklines, charts, multi-user cursors: to be implemented.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**
- Grid rendering via wgpu/QuadRenderer at 60+ FPS
- Virtual scrolling: only visible cells + padding are rendered
- User input handling → `TrameCommand` to Logic thread
- Formula bar and inline editor
- Function and cell reference autocompletion
- Row/column freeze
- Sparklines (mini-charts in cells)
- wgpu/lyon charts (bar, line, pie)
- Multi-user cursors and selections
- PDF export: shared rendering code via `DrawSurface` (executed in the Logic thread via `KrillaSurface`); the UI thread triggers the download upon receiving `TrameEvent::ExportComplete`

**What this module does NOT do:**
- Formula calculation (Logic thread)
- CRDT management (Logic thread)
- Encryption/network (Sync thread)

**Dependencies:**

| Crate / API | Role | Rationale |
|---|---|---|
| `wgpu` (via `QuadRenderer`) | GPU vertex/fragment rendering | WebGL2 by default, WebGPU opt-in. Zero compute shader. |
| `lyon` | Path tessellation | Rects, grid lines, borders → triangles. |
| `parley` | Text layout and shaping | BiDi, line breaking, HarfRust/Skrifa shaping. |
| `swash` | Glyph rasterization | Glyphs → bitmap R8 → GlyphAtlas GPU. |
| `ambre-draw` | `DrawSurface` trait + types | Unified screen/PDF interface. |
| `krilla` | PDF generation | Instantiated in the **Logic thread** via `KrillaSurface`. |
| `ambre-bus` | Typed data plane: `TrameCommand` / `TrameEvent` / `DisplayCell` | UI<->Logic values carried by move on `flume` (no serialization) |
| `wasm-bindgen` | JS <-> Rust bridge | Canvas / DOM glue on the UI thread |
| `web-sys` | Browser APIs (Canvas, clipboard, file download) | DOM access from UI thread |
| `postcard` | Serialization | Only the `ClipboardPayload` (OS clipboard blob); **not** the UI<->Logic hop |

**Invariants:** prefix `TRAME-UI`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4` ("Invariants (normative)"); the cross-spec roll-up is `ambre-spec-testing.md §7`. Rows `001..015` cover the built surface. Rows `016..023` cover multi-user cursors (§9), sparklines (§10), charts (§11) and PDF export (§12), which have no implementing code -- `ambre-trame` declares neither `krilla` nor `ambre-pdf` -- so they carry `Status: unimplemented` and cite the Story that will build them (architecture §4).

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-001` | The UI thread MUST hold no document data -- only view state and the `DisplayViewport` received from the Logic thread. | §16 "Fundamental invariant" | structural: **no occurrence of `LoroDoc` or `loro::` anywhere under `ambre-trame/src/ui/`**; `deny.toml` bans `loro` outside the four app crates (`SYNC-011`) | verified |
| `TRAME-UI-002` | Every document mutation MUST leave the UI thread as a `TrameCommand`; the UI MUST NOT write document state locally. | §16 | type-level: the single outbound path is `TrameView::send_command_lossless` (`ui/view/lifecycle.rs`), which accepts a `TrameCommand` and wraps it in `LogicCommand::App` -- there is no other sender | verified |
| `TRAME-UI-003` | The UI thread MUST NOT block on the Logic channel: a full channel drops the command with a log, it never waits. | architecture §6; `ambre-spec-ui.md §1` | type-level: `send_command_lossless` is `try_send` and returns `Result<(), SendError>`; `SendError::Full` is handled at every call site | verified |
| `TRAME-UI-004` | Only the visible viewport plus its adaptive padding MUST be requested from the Logic thread. | §3.A | none. The heuristic lives in `ui/grid/state.rs` (velocity -> 20/50/100 rows) and no test asserts the requested bounds against the visible range | unverified |
| `TRAME-UI-005` | A `ViewportData` whose `version` is lower than the last accepted one MUST be discarded. | §2.B; §3 anti-flicker | none. The rule **is implemented** (`ui/grid/state.rs`: `if version < self.accepted_version { return; }`) and nothing exercises it; the deleted §14 named an `outdated_viewport_version_ignored` test that never existed | unverified |
| `TRAME-UI-006` | The canvas dispatch over `TrameEvent` MUST be exhaustive -- no wildcard arm -- so a new variant is a compile error, not a silent drop. | §2.B; §18 | compiler: `ui/view/frame.rs::dispatch_trame` carries **no `_` arm**; adding a variant fails the build. (The §2.B excerpt used to end on a `_ => {}` that contradicted its own sentence and the code; corrected.) | verified |
| `TRAME-UI-007` | Hit-testing MUST be the exact inverse of the drawn geometry, under zoom and under variable row/column metrics. | §3; §4bis | `ambre_trame::ui::grid::tests::group1::zoom_scales_metrics_and_round_trips_hit_test`, `::variable_metrics_round_trip_pixel_and_hit_test`, `::axis_pointer_resolution_follows_variable_metrics`, `::filter_button_hit_test_matches_drawn_rect`, `ambre_trame::render::tests::variable_metrics_shift_cells_and_hit_test` | verified |
| `TRAME-UI-008` | A filtered-out row MUST collapse both the drawn geometry and keyboard navigation. | §5.A; §5.D | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav`, `ambre_trame::render::tests::cell_rect_collapses_over_hidden_rows` | verified |
| `TRAME-UI-009` | A merged block MUST select as a whole, and navigation MUST step over it rather than into it. | §8; §5.B | `ambre_trame::ui::grid::tests::group1::merge_block_selects_as_a_whole_and_nav_steps_over_it`, `::drag_selection_expands_to_whole_merge`, `::fill_handle_sits_on_merge_extent_corner`, `ambre_trame::ui::grid::tests::group2::snap_active_to_master_rehomes_after_merge` | verified |
| `TRAME-UI-010` | An optimistic commit MUST NOT display the formula source as its own result. | §6.C | `ambre_trame::ui::grid::tests::group2::optimistic_commit_does_not_echo_formula_as_result` | verified |
| `TRAME-UI-011` | The viewport MUST carry each cell's raw text alongside its displayed value, so reopening the editor shows the formula. | §6.A | `ambre_trame::ui::grid::tests::group2::viewport_data_preserves_formula_raw_for_editing`, `::editor_preview_display` | verified |
| `TRAME-UI-012` | Every fill colour drawn by the renderer MUST come from the theme palette; no literal colour in render code. | §4.A; `ambre-spec-ui.md` (design tokens) | `ambre_trame::ui::renderer::tests::renderer_uses_only_palette_colors_for_fills` | verified |
| `TRAME-UI-013` | The renderer MUST NOT paint outside the grid region, whatever the canvas size or scroll offset. | §2.A; §4 | `ambre_trame::ui::renderer::tests::draw_cells_strip_skips_when_outside_grid`, `::draw_cells_scroll_strip_base_is_blank_not_pending_grey`, `ambre_trame::ui::grid::tests::group2::grid_bottom_floors_at_header_on_tiny_canvas`, `::header_strip_starts_at_top_of_canvas` | verified |
| `TRAME-UI-014` | A cell in error MUST be marked visually from its displayed text; there MUST NOT be an `is_error` field on `DisplayCell`. | §2.B note | `ambre_trame::ui::renderer::tests::errored_cell_gets_a_corner_marker`, `::dirty_cell_gets_a_translucent_veil` | verified |
| `TRAME-UI-015` | Escape MUST cancel an edit in progress without emitting any `TrameCommand`. | §6.C | none. The handler exists (`ui/view/interact.rs`), the absence of an outbound command is asserted nowhere -- and an absence is exactly what a test has to state | unverified |
| `TRAME-UI-016` | A remote cursor's colour MUST be derived deterministically from its `UserId`, so the same collaborator keeps one colour across peers and reloads. | §9 | not built -- lands with `SYNC P4` / `SRV P1` (Epic `E5`), which supplies the spoof-proof `from_user_id` | unimplemented |
| `TRAME-UI-017` | A remote cursor MUST be keyed by `UserId`, never by `pub_key` or connection: one collaborator on two devices MUST NOT render as two people. | §9 | not built -- lands with `SYNC P4` / `SRV P1` (Epic `E5`) | unimplemented |
| `TRAME-UI-018` | A remote peer's `display_name` MUST be treated as cosmetic and self-asserted; it MUST NOT be used for identity or colouring. | §9; `trame-logic §7` | not built -- lands with `SYNC P4` / `SRV P1` (Epic `E5`) | unimplemented |
| `TRAME-UI-019` | Presence MUST leave no trace in the document: a cursor move MUST NOT produce a CRDT delta nor force a viewport re-derive. | §9 | not built -- lands with `SYNC P4` / `SRV P1` (Epic `E5`) | unimplemented |
| `TRAME-UI-020` | A sparkline MUST be derived on the UI thread from `DisplayViewport` data alone; it MUST NOT require a second round trip to the Logic thread. | §10 | not built -- lands with §18 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-UI-021` | A chart MUST be clipped to its host cell range and MUST NOT paint outside the grid. | §11 | not built -- lands with §18 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-UI-022` | PDF export MUST reuse the same `DrawSurface` render code as the screen, through `KrillaSurface` in the Logic thread; the UI thread MUST only trigger the download. | §12 | not built -- lands with §18 Phase 2 (Epic `E6`); `TRAME-IO-020` is the same rule seen from the interop side | unimplemented |
| `TRAME-UI-023` | A PDF export MUST be pure vector: no rasterisation of text or shapes. | §12 "Pure vector" | not built -- lands with §18 Phase 2 (Epic `E6`) | unimplemented |

> **Invariant coverage.** 11 `verified`, 4 `unverified`, 8 `unimplemented`. The four gaps are `TRAME-UI-004`, `-005`, `-015` and the render-loop pair -- rules that ship in the code today with nothing guarding them, which is the only kind of row that is a risk.

---

## 2. RENDERING ARCHITECTURE

### A. Rendering Loop (requestAnimationFrame)

Trame rendering is driven by `UiState::render()` in `ambre-app/src/lib.rs`. Trame-specific state lives in `TrameUiState` (`ambre-app/src/trame/grid.rs`). Drawing is done by `draw_grid()` in `ambre-app/src/trame/renderer.rs`, generic over `DrawSurface`.

```rust
// ambre-app/src/trame/grid.rs -- Trame UI thread state (current state)
pub struct TrameUiState {
    pub canvas_width:    f64,
    pub canvas_height:   f64,
    pub top_row:         usize,
    pub left_col:        usize,
    pub viewport_rows:   usize,
    pub viewport_cols:   usize,
    pub active_row:      usize,
    pub active_col:      usize,
    pub cell_cache:      HashMap<(usize, usize), String>,
}

// ambre-app/src/trame/renderer.rs -- draw_grid generic over DrawSurface
pub fn draw_grid<S: DrawSurface>(
    surface:       &mut S,
    state:         &TrameUiState,
    font_cx:       &mut FontContext,
    text_cache:    &mut TextCache,
    text_enabled:  bool,
    render_fps:    f32,
    last_frame_ms: f32,
) {
    // 1. Full white background
    // 2. "Pending" cell zone (light gray, not yet received)
    // 3. White fill for received cells (cell_cache)
    // 4. Vertical grid lines (stroke_line)
    // 5. Horizontal grid lines (stroke_line)
    // 6a. Active column header tint
    // 6b. Active row header tint
    // 7. Column header background (on top of tints)
    // 8. Row header background
    // 9. Header separators (stroke_line)
    // 10. Active cell border (stroke_rect, 2px blue)
    // 11. Text: cell body + column labels + row numbers (draw_glyphs)
    // 12. FPS overlay (fill_rect + draw_glyphs)
}
```

### B. Update from the Logic Thread

The Logic thread delivers **typed** `TrameEvent` values by move (no postcard, no
`LogicEnvelope` on this hop). The single `UiEvent::Viewport(AppEvent::Trame(ev))`
arm dispatches them **exhaustively** -- no `_ => {}` wildcard, so a new variant is
a compile error, not a silent drop (see §18 and the render-worker
`dispatch_trame`):

```rust
// ambre-app / ambre-trame -- typed TrameEvent dispatch (canvas side)
fn dispatch_trame(&mut self, ev: TrameEvent) {
    match ev {
        TrameEvent::ViewportData { .. } => { self.on_viewport_data(ev); self.dirty = true; }
        TrameEvent::DataReset | TrameEvent::CondFormatsUpdated { .. } => self.request_viewport(),
        TrameEvent::SheetList { sheets, active } => self.set_tabs(sheets, active),
        TrameEvent::SelectRange { r1, c1, r2, c2 } => self.set_selection(r1, c1, r2, c2),
        TrameEvent::FilterApplied { col, .. } => self.filtered_col = Some(col),
        TrameEvent::FilterCleared => self.filtered_col = None,
        TrameEvent::FindResult { matches, active } => self.set_find_highlights(matches, active),
        TrameEvent::Error { message } => self.log_and_announce(&message),
        // chrome-only variants (StyleApplied, UndoState, MergeRejected,
        // MergeConflictDetected, SelectionStats, ColumnValues, ReplaceDone,
        // PasteRejected, Clipboard, SortComplete, validation/protection,
        // ImportComplete, ExportComplete) are handled by the chrome relay and
        // are explicit no-ops on the canvas -- each named, none wildcarded.
        // (This excerpt elides them; the real match in `ui/view/frame.rs` spells
        //  every one out and carries no `_` arm -- `TRAME-UI-006`.)
    }
}
```

> **`FormulaError` / `EditAck` / `ImportProgress` are not `TrameEvent` variants.**
> A `#REF!`/parse error surfaces as the affected cell's displayed `text` inside
> `ViewportData` (the UI draws a corner marker when that text is an error string;
> there is no `DisplayCell::is_error` field), edits are
> optimistic (no ack), and there is no mid-import progress event (interop §7). The
> chrome relay routes user-facing variants (`MergeRejected`, `ExportComplete` ->
> `trigger_download`, ...) via `notify_event` / `mirror_event` / `apply_style_event`.

---

## 3. VIRTUAL SCROLLING

### A. Principle

Only cells within the visible viewport + adaptive padding are requested from and rendered by the Logic thread. The rest does not exist on the UI thread side.

```rust
pub struct GridUiState {
    /// Scroll position in pixels
    scroll_x: f32,
    scroll_y: f32,

    /// Grid area dimensions (excluding headers)
    grid_width:  f32,
    grid_height: f32,

    /// Cumulative column/row dimensions (for pixel -> index mapping)
    col_offsets: Vec<f32>,  // col_offsets[i] = start pixel of column i
    row_offsets: Vec<f32>,

    /// Freeze
    frozen_rows: u32,
    frozen_cols: u32,

    /// Current selection
    selection: Selection,

    /// Active inline editor
    editing: Option<InlineEditor>,

    /// Remote cursors -- key = UserId (resolved by ambre-sync before reaching here)
    remote_cursors: HashMap<UserId, RemoteCursor>,

    /// Undo/Redo state
    can_undo: bool,
    can_redo: bool,
}
```

**Visible viewport calculation:**

```rust
fn visible_range(&self) -> (u32, u32, u32, u32) {
    let col_start = self.col_at_pixel(self.scroll_x) as u32;
    // `saturating_sub(1)` guards the empty-offsets case: `len() == 0` would
    // underflow `len as u32 - 1` (unlike col_at_pixel, which already saturates).
    let col_end   = self.col_at_pixel(self.scroll_x + self.grid_width + 1.0).min(
        (self.col_offsets.len() as u32).saturating_sub(1)
    );
    let row_start = self.row_at_pixel(self.scroll_y) as u32;
    let row_end   = self.row_at_pixel(self.scroll_y + self.grid_height + 1.0).min(
        (self.row_offsets.len() as u32).saturating_sub(1)
    );
    (row_start, row_end, col_start, col_end)
}

fn col_at_pixel(&self, x: f32) -> usize {
    // Binary search in col_offsets
    self.col_offsets.partition_point(|&o| o <= x).saturating_sub(1)
}
```

**Sending ScrollViewport to the Logic thread:**

```rust
fn on_scroll(&mut self, delta_x: f32, delta_y: f32) {
    self.scroll_x = (self.scroll_x + delta_x).max(0.0);
    self.scroll_y = (self.scroll_y + delta_y).max(0.0);

    let (row_start, row_end, col_start, col_end) = self.visible_range();
    // Adaptive padding: the velocity is a *local* UI heuristic for how much to
    // pad -- it is NOT carried on the wire. `ScrollViewport` has no
    // `velocity_px_ms`/`sheet_id` field; the Logic thread resolves the active
    // sheet itself and just fills the requested window.
    let velocity = (delta_x.abs() + delta_y.abs()) / 16.0; // estimated px/ms
    let padding = if velocity < 1.0 { 20u32 } else if velocity > 10.0 { 100 } else { 50 };

    self.logic_tx.send(AppCommand::Trame(TrameCommand::ScrollViewport {
        version:   self.next_version(),
        row_start: row_start.saturating_sub(padding / 2),
        row_end:   row_end + padding / 2,
        col_start: col_start.saturating_sub(5),
        col_end:   col_end + 5,
    }));
}
```

### B. Updating `col_offsets` / `row_offsets`

```rust
fn update_offsets_from_viewport(&mut self, vp: &DisplayViewport) {
    // Incremental offset update from viewport col_widths/row_heights
    // Columns/rows outside the viewport keep their default value
    let default_col_width  = 80.0f32;
    let default_row_height = 20.0f32;

    for (i, &w) in vp.col_widths.iter().enumerate() {
        let col_idx = vp.col_start as usize + i;
        if col_idx < self.col_offsets.len() {
            // Rebuild offsets from col_idx if the width changed
            if (self.col_width_at(col_idx) - w).abs() > 0.5 {
                self.rebuild_col_offsets_from(col_idx);
            }
        }
    }
}
```

---

## 4. WGPU RENDERING PIPELINE (DrawSurface)

### A. Cell Rendering

> **DrawSurface architecture:** all rendering goes through `draw_grid<S: DrawSurface>()` in `ambre-app/src/trame/renderer.rs`. The same code drives `WgpuSurface` (screen) and, eventually, `KrillaSurface` (PDF). No direct reference to wgpu or GPU types in the cell rendering code.
>
> **Current state:** `draw_grid` covers background, headers, grid lines, text, active cell border, and FPS overlay. The rendering below (custom borders, error indicators, dirty opacity) is to be implemented.

```rust
// Cell rendering via DrawSurface (generic screen + PDF)
fn draw_cell<S: DrawSurface>(
    surface:    &mut S,
    cell:       &DisplayCell,
    x: f64, y: f64, w: f64, h: f64,
    font_cx:    &mut FontContext,
    text_cache: &mut TextCache,
) {
    // Cells covered by a merge: do not render
    if cell.is_merged && cell.merge_span.is_none() { return; }

    // Cell background
    let bg = cell.style.bg_color.unwrap_or(Color::WHITE);
    // TODO: reduced opacity for dirty cells (alpha = 0.5)
    // DrawSurface does not yet support opacity -- to add with fill_path + Brush::Solid with alpha
    surface.fill_rect(Rect::new(x, y, x + w, y + h), bg);

    // Text
    if !cell.display_text.is_empty() {
        let align = match cell.style.h_align {
            HAlign::Auto => if cell.display_text.parse::<f64>().is_ok() { Alignment::End } else { Alignment::Start },
            HAlign::Left   => Alignment::Start,
            HAlign::Center => Alignment::Center,
            HAlign::Right  => Alignment::End,
        };
        let fg = cell.style.fg_color.unwrap_or(Color::BLACK);
        // Wrap: constrain to the cell width so Parley breaks lines. No wrap: let the
        // text overflow into empty right-neighbour cells (Excel behaviour, sec.4.C).
        let size  = cell.style.size.unwrap_or(11.0);
        let max_w = if cell.style.wrap { (w - 6.0) as f32 } else { f32::MAX };
        let layout = text_cache.get_or_build(font_cx, &cell.display_text, size, max_w, align);
        // Vertical alignment (replaces the hard-coded centring).
        let y_off = match cell.style.v_align {
            VAlign::Top    => 0.0,
            VAlign::Middle => ((h - layout.height() as f64) / 2.0).max(0.0),
            VAlign::Bottom =>  (h - layout.height() as f64).max(0.0),
        };
        let brush = Brush::Solid(fg);
        // Clip to the cell, except a non-wrapped overflowing text spills up to the next
        // non-empty cell (overflow_clip_width scans the viewport row).
        let clip_w = if cell.style.wrap { w } else { overflow_clip_width(cell, w) };
        surface.push_clip(Rect::new(x, y, x + clip_w, y + h));
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let tx = x + 3.0 + run.offset() as f64;
                let ty = y + y_off + run.baseline() as f64;
                surface.draw_glyphs(&run, &brush, Affine::translate((tx, ty)));
            }
        }
        surface.pop_clip();
    }

    // Custom borders (cell.style.border via fill_path/lyon), error indicator: TODO
}
```

> **Wrap / vertical alignment / overflow (MVP):** `CellStyle.wrap` and `CellStyle.v_align`
> are honoured above. `wrap` rows may need a taller `row_height` -- the Logic thread returns
> the measured height in `DisplayViewport.row_heights`; the UI does not remeasure.
>
> **Decision (overflow vs ellipsis):** a non-wrapped value that exceeds its column spills
> over **empty** right-neighbour cells (`overflow_clip_width` extends the clip up to the
> first non-empty cell in the viewport row), matching Excel/Sheets. If the neighbour is
> non-empty the text is clipped at the cell edge (no ellipsis in MVP). Borders/fills are
> still clipped to the owning cell.

### B. Headers (Columns A, B, C... / Rows 1, 2, 3...)

> **Implemented in `draw_grid`** (see `ambre-app/src/trame/renderer.rs`). Active column/row highlighting is handled by `fill_rect` in light blue tint before the opaque header background, then re-applied on top.

```rust
// Current pattern in draw_grid (simplified):
// 6a. Active column header tint (before opaque bg)
surface.fill_rect(Rect::new(x_active_col, 0.0, x_active_col + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
// 7. Column header background (opaque, on top -- except active tint)
surface.fill_rect(Rect::new(0.0, 0.0, w, HEADER_HEIGHT), COLOR_HEADER_BG);
// Re-apply active tint on top of the opaque bg
surface.fill_rect(Rect::new(x_active_col, 0.0, x_active_col + COL_WIDTH, HEADER_HEIGHT), COLOR_HEADER_ACTIVE);
// Labels (via TextCache + draw_glyphs)
// ...

// Range header highlight (MVP, sec.8): a header is tinted when its col/row is inside
// the selection bounds, full opaque for a whole row/column selection.
// let bg = if selection.contains_col(col) { COLOR_SELECTION_HEADER } else { COLOR_HEADER_BG };
```

### C. Freeze (Frozen Rows/Columns)

Frozen cells are rendered in a separate pass, on top of the scroll, without scroll offset.

```rust
fn draw_freeze_overlay(&mut self) {
    let frozen_rows = self.ui_state.frozen_rows;
    let frozen_cols = self.ui_state.frozen_cols;
    if frozen_rows == 0 && frozen_cols == 0 { return; }

    // Render frozen cells from current_viewport
    // (they are included in the viewport by the Logic thread)
    if let Some(vp) = &self.current_viewport {
        for cell in &vp.cells {
            let is_frozen_row = cell.row_index < frozen_rows;
            let is_frozen_col = cell.col_index < frozen_cols;
            if !is_frozen_row && !is_frozen_col { continue; }

            // Fixed position: no scroll subtraction
            let x = if is_frozen_col {
                self.col_offset(cell.col_index) + self.header_width // absolute position
            } else {
                self.col_offset(cell.col_index) - self.scroll_x + self.header_width
            };
            let y = if is_frozen_row {
                self.row_offset(cell.row_index) + self.header_height
            } else {
                self.row_offset(cell.row_index) - self.scroll_y + self.header_height
            };

            self.draw_single_cell(cell, x, y);
        }
    }

    // Freeze separation line
    let sep = Color::from_rgb8(100, 100, 100);
    if frozen_cols > 0 {
        let sep_x = self.col_offset(frozen_cols as u32) + self.header_width;
        surface.stroke_line(sep_x, 0.0, sep_x, self.grid_height, sep, 2.0);
    }
    if frozen_rows > 0 {
        let sep_y = self.row_offset(frozen_rows as u32) + self.header_height;
        surface.stroke_line(0.0, sep_y, self.grid_width, sep_y, sep, 2.0);
    }
}
```

**Freeze command from the UI:**

```rust
// Local state only -- not in Loro, no TrameCommand
// Freeze rows/cols are a view preference, not shared data
fn handle_freeze_rows(&mut self, row: u32) {
    self.ui_state.frozen_rows = row;
    // Triggers a re-render -- no message to the Logic thread
}
```

> **Decision:** Freeze is local state (like Google Sheets). Not synchronized between collaborators — each user has their own freeze view.

---

## 4bis. COLUMN / ROW HEADER INTERACTIONS (MVP)

> **Why this is MVP.** The autofill handle received a thorough UX pass; the headers did not.
> Today the only entry point for insert/delete is the grid context menu, and there is no
> whole-column/row selection, no hover affordance, and no resize. This section raises the
> header interaction to the same bar as the fill handle. Selection, hover cursors and the
> inline insert affordance are **UI-thread view state + existing typed commands**. The
> **resize** leg (sec.4bis.C) is the one part that needs a **new data plane**: the
> `col_widths_{sheet}` / `row_heights_{sheet}` Loro maps and the `ResizeCol` / `ResizeRow`
> commands are introduced under this story (`-logic` §2.B/§9), alongside a variable-metric
> `GridLayout`. They do not exist yet -- only the uniform-metric layout does.

The header band is divided into three hit zones per cell, resolved in `cell_at_pixel`'s
header sibling `header_hit(x, y) -> HeaderHit`:

```rust
enum HeaderHit {
    ColBody(u32),          // header body -> whole-column select / drag-extend
    RowBody(u32),
    ColBorder(u32),        // +/-4px around the right border of col i -> resize col i
    RowBorder(u32),        // resize row i
    ColInsert(u32),        // small "+" affordance shown on hover at a boundary
    RowInsert(u32),
    Corner,                // top-left corner -> select-all
}
```

### A. Whole-column / whole-row selection (virtual)

Left-clicking a column header selects the **entire** column; clicking a row header selects
the entire row. The UI does not know the document height, so a whole-axis selection is a
distinct selection kind, not a cell rectangle clamped to a guessed bound:

```rust
pub enum SelKind { Cells, Cols, Rows }   // added to Selection (sec.8)
```

`anchor`/`active` still carry the band endpoints (the perpendicular coordinate is ignored for
`Cols`/`Rows`). Drag along the header extends the band; **Shift+click** extends, **Ctrl/Cmd+click**
adds a disjoint band (`Selection.extra`, sec.8). The top-left **corner** selects the whole sheet.
Rendering: the selected band's cells get the range fill, and the band's headers render with the
**opaque active-header tint** (full column/row, sec.4.B) so the selection reads even though no
cell rectangle bounds it on screen.

A whole-axis selection feeds the existing **parameterless** intents unchanged: the Logic thread,
which tracks the selection (`SetActiveCell` carries the anchor corner) and knows the **used range**,
resolves "whole column C" to the used-range rectangle for sort / filter / `SetCellStyle` /
`DeleteSelectedCols` / `RequestSelectionStats`. This is the same UI->Logic contract already used
by every E3 feature; the UI never needs the document height (sec.16).

> **Relation to `RangeEndpoint::EntireAxis` (`-logic` §3.A).** The whole-axis UI selection is the
> interactive cousin of a formula's `A:A` / `1:1` -- and is resolved the **same way**: the Logic
> thread clamps the unbounded axis to the used range `[0..hwm]` (exactly `endpoint_pos`'s `entire`
> bound). They stay **separate types** -- `EntireAxis` lives in the compiled formula engine and is
> currently reserved (the MVP parser never emits it); `SelKind::Cols`/`Rows` is UI view state.
> No new data plane: the axis selection rides the existing tracked-selection contract.

### B. Hover cursor feedback

The canvas cursor (`#ambre-canvas` CSS `cursor`, generalised from `set_fill_cursor`, sec.5.F into
`TrameView::set_cursor(CursorKind)`) tracks `header_hit` on every pointer move:

| Zone | Cursor |
|---|---|
| `ColBody` | `s-resize`-style **down arrow** affordance (column-select) -- CSS `cursor: pointer` for MVP |
| `RowBody` | row-select pointer |
| `ColBorder` | `col-resize` |
| `RowBorder` | `row-resize` |
| `ColInsert` / `RowInsert` | `pointer` (over the "+" affordance) |
| grid body | default / fill-crosshair over the handle (sec.5.F) |

### C. Resize by dragging the header boundary

Pressing on a `ColBorder(i)` / `RowBorder(i)` zone arms a resize drag (`TrameView::resize_drag`,
the same arming pattern as `fill_drag`, sec.5.F) instead of a selection. Pointer moves draw a live
guide line at the prospective edge (`TrameUiState::resize_preview`, painted in
`ui::renderer::draw_overlay`); the release emits one command and grows nothing:

```rust
fn on_resize_drag_end(&mut self, hit: HeaderHit, px: f32) {
    let v = self.next_version();
    match hit {
        HeaderHit::ColBorder(c) => {
            let width = (px - self.col_left(c) + self.scroll_x).max(MIN_COL_WIDTH); // >= ~24px
            self.logic_tx.send(TrameCommand::ResizeCol { version: v, sheet_id: self.ui_state.active_sheet, col: c, width_px: width });
        }
        HeaderHit::RowBorder(r) => {
            let height = (px - self.row_top(r) + self.scroll_y).max(MIN_ROW_HEIGHT);
            self.logic_tx.send(TrameCommand::ResizeRow { version: v, sheet_id: self.ui_state.active_sheet, row: r, height_px: height });
        }
        _ => {}
    }
    // Logic writes col_widths_{sheet}/row_heights_{sheet} (LWW per uid) and replies with a
    // refreshed viewport carrying the new col_widths/row_heights; the UI rebuilds its offsets
    // (sec.3.B). When a whole column/row is selected, the resize applies to every selected band.
}
```

Resize is a **document mutation** (Loro, synchronised, undoable -- it is a per-uid LWW write, so
two users resizing the same column converge on LWW, like any cell). If the dragged column/row is
part of the current whole-axis selection, the new size is applied to **every** column/row in the
selection (one command per band, one undo group).

**Auto-fit on double-click (MVP).** Double-clicking a `ColBorder(i)` / `RowBorder(i)` zone fits the
column/row to its content. This is **UI-side** and reuses `ResizeCol` / `ResizeRow` with no new
command: the UI already owns the parley `font_cx` + `text_cache` and measures glyph runs, so it
walks the column's **cached** cells (`cell_cache`, viewport + padding), takes the max natural width
(`+2*CELL_PAD_X`, clamped to `MIN_COL_WIDTH`), and sends `ResizeCol { width_px }` (rows: max line
height -> `ResizeRow`). One undo group, like a drag resize.

> **Caveat (and the V1 upgrade path).** UI-side auto-fit fits the **cached** cells (what is on or
> near screen), not necessarily the whole column -- correct in the overwhelming majority of cases
> and instant. True whole-column auto-fit needs full-column text measurement, which would require a
> parley `FontContext` in the Logic worker (fonts live UI/gpu-side today); that stays a **V1**
> refinement (a Logic-side `AutoFitCol`/`AutoFitRow` measuring every materialized cell), not MVP.

### D. Inline insert "+" affordance

Hovering near a header boundary reveals a small "+" button (`ColInsert(i)` / `RowInsert(i)`),
drawn in `draw_overlay`. Clicking it inserts a column to the left of `i` (resp. a row above `i`)
through the existing parameterless intents: the UI first moves the tracked active cell onto column
`i` / row `i`, then fires `InsertColLeft` / `InsertRowAbove` (sec.5.C), so the Logic thread resolves
the position exactly as from the context menu. This gives a one-click inline insert without opening
the menu, matching the fill-handle's directness.

---

## 5. INPUT HANDLING

### A. Keyboard

```rust
fn on_key_down(&mut self, key: &KeyCode, modifiers: &Modifiers) {
    // Undo/Redo
    if modifiers.ctrl_or_cmd() && *key == KeyCode::KeyZ {
        if modifiers.shift() {
            self.logic_tx.send(TrameCommand::Redo);
        } else {
            self.logic_tx.send(TrameCommand::Undo);
        }
        return;
    }

    // Grid navigation. With Shift held, an arrow *extends* the selection
    // (move `active`, keep `anchor`) instead of collapsing it to a single cell.
    if self.ui_state.editing.is_none() {
        let extend = modifiers.shift();   // move_selection extends when true (sec.8)
        match key {
            KeyCode::ArrowUp    => self.move_selection(0, -1, extend),
            KeyCode::ArrowDown  => self.move_selection(0, 1, extend),
            KeyCode::ArrowLeft  => self.move_selection(-1, 0, extend),
            KeyCode::ArrowRight => self.move_selection(1, 0, extend),
            KeyCode::Tab        => self.move_selection(1, 0, false),
            KeyCode::Enter      => self.move_selection(0, 1, false),
            KeyCode::Home       => self.goto_cell(self.ui_state.selection.row, 0),
            KeyCode::End        => self.goto_end_of_row(),
            KeyCode::PageDown   => self.scroll_pages(1),
            KeyCode::PageUp     => self.scroll_pages(-1),
            KeyCode::Delete | KeyCode::Backspace => {
                // No dedicated ClearCell command: an empty SetCellText clears the
                // cell (same path a committed empty edit takes). The UI updates
                // its cache optimistically; no echo event is required.
                let (row, col) = self.ui_state.selection.active();
                self.logic_tx.send(TrameCommand::SetCellText { row, col, text: String::new() });
            }
            KeyCode::F2 => self.start_editing(EditTrigger::F2),
            _ => {
                // Character keystroke -> start editing
                if let Some(c) = key.to_char(modifiers) {
                    self.start_editing(EditTrigger::Char(c));
                }
            }
        }
    } else {
        // Editor active
        match key {
            KeyCode::Escape => self.cancel_editing(),
            KeyCode::Enter  => self.commit_editing(CommitDirection::Down),
            KeyCode::Tab    => self.commit_editing(CommitDirection::Right),
            _ => self.forward_key_to_editor(key, modifiers),
        }
    }
}
```

### B. Mouse / Touch

```rust
fn on_pointer_down(&mut self, x: f32, y: f32, button: MouseButton) {
    let (row, col) = self.cell_at_pixel(x, y);

    if button == MouseButton::Left {
        if let Some((row, col)) = (row, col) {
            if Some((row, col)) == self.ui_state.selection.active_cell()
                && self.ui_state.editing.is_none()
            {
                // Double-click -> editing
                self.start_editing(EditTrigger::Click);
            } else {
                // Single click -> selection
                self.ui_state.selection = Selection::single(row, col);
                self.ensure_cell_visible(row, col);
                // Request content of the new active cell
                self.request_viewport();
            }
        }
    }
}

fn on_pointer_drag(&mut self, x: f32, y: f32) {
    // Extend selection
    if let Some((row, col)) = self.cell_at_pixel(x, y) {
        self.ui_state.selection.extend_to(row, col);
        self.request_viewport();
    }
}
```

### C. Context Menu — Structural Ops, Merge, Sort, Filter (MVP)

Right-click on the grid (or on a header) opens a context menu. It is the single MVP
entry point for the structural / feature commands the Logic thread already handles but
which had no UI trigger: row/column insert-delete, merge, single-column sort, and the
shared filter.

> **Implementation note (supersedes the `DrawSurface` overlay below).** The structural
> / merge ops are wired through the **existing DOM context menu** (`ambre-chrome::ContextMenu`,
> populated by `trame_chrome::context_entries()`), not a canvas overlay. That menu already
> exists, already flips/keyboard-navigates/dismisses, and shares the single command path
> (`on_command` -> `command_to_app` -> `AppCommand`) with the toolbar, the selection bar and
> the keyboard -- the foundation for configurable command buttons shared across menu / toolbar
> / dynamic context menus. A parallel canvas menu would duplicate that machinery.
>
> Because the DOM menu dispatches **parameterless** `CommandId`s, the structural commands are
> parameterless intents (`InsertRowAbove`/`InsertRowBelow`/`InsertColLeft`/`InsertColRight`,
> `DeleteSelectedRows`/`DeleteSelectedCols`, `MergeSelection`, `UnmergeActive`). The Logic
> thread resolves their geometry against the **active cell / selection rectangle it tracks** --
> `SetActiveCell` now carries the anchor corner so the Logic thread keeps the full selection,
> exactly as it already keeps the active cell for formatting toggles (still local view state,
> sec.16). `MergeSelection` on a single cell is a silent no-op; on a non-empty covered cell it
> replies `MergeRejected` (toast via `chrome::notify_event`); `UnmergeActive` outside a merge
> is a no-op. Sort and filter entries (below) remain to do.
>
> The `DrawSurface`/`ContextMenuItem` design that follows is retained for reference but is
> not the implemented path.

The (reference) `DrawSurface` overlay design: items are positioned from the click point and
dismissed on Escape or outside click.

```rust
pub enum ContextMenuItem {
    InsertRowAbove, InsertRowBelow,
    InsertColLeft,  InsertColRight,
    DeleteRows,     DeleteCols,
    MergeCells,     UnmergeCells,    // Merge enabled only on a multi-cell selection
    SortAsc,        SortDesc,        // single-column sort on the active column
    FilterByColumn,                  // opens the filter popover for the active column
    ClearFilter,
}

fn on_context_menu_select(&mut self, item: ContextMenuItem) {
    let sel = &self.ui_state.selection;
    let (row, col) = sel.active();
    let sheet_id   = self.ui_state.active_sheet;
    let v          = self.next_version();
    match item {
        ContextMenuItem::InsertRowAbove =>
            self.logic_tx.send(TrameCommand::InsertRow { version: v, sheet_id, before_row: row }),
        ContextMenuItem::InsertRowBelow =>
            self.logic_tx.send(TrameCommand::InsertRow { version: v, sheet_id, before_row: row + 1 }),
        ContextMenuItem::InsertColLeft =>
            self.logic_tx.send(TrameCommand::InsertCol { version: v, sheet_id, before_col: col }),
        ContextMenuItem::InsertColRight =>
            self.logic_tx.send(TrameCommand::InsertCol { version: v, sheet_id, before_col: col + 1 }),
        // One command per row/col in the selection span (each = 1 undo item).
        ContextMenuItem::DeleteRows =>
            for r in sel.row_span_desc() {  // descending so positions stay valid
                self.logic_tx.send(TrameCommand::DeleteRow { version: self.next_version(), sheet_id, row: r });
            },
        ContextMenuItem::DeleteCols =>
            for c in sel.col_span_desc() {
                self.logic_tx.send(TrameCommand::DeleteCol { version: self.next_version(), sheet_id, col: c });
            },
        ContextMenuItem::MergeCells => {
            let (r1, c1, r2, c2) = sel.bounds();
            self.logic_tx.send(TrameCommand::MergeCells {
                version: v, sheet_id, row: r1, col: c1,
                row_span: r2 - r1 + 1, col_span: c2 - c1 + 1,
            });
            // A MergeRejected event (covered non-empty cell) surfaces a toast -- see sec.2.
        }
        ContextMenuItem::UnmergeCells =>
            self.logic_tx.send(TrameCommand::UnmergeCells { version: v, sheet_id, row, col }),
        ContextMenuItem::SortAsc =>
            self.send_sort(sheet_id, /* ascending */ true),
        ContextMenuItem::SortDesc =>
            self.send_sort(sheet_id, false),
        ContextMenuItem::FilterByColumn =>
            self.open_filter_popover(sheet_id, col),   // see sec.5.D
        ContextMenuItem::ClearFilter =>
            self.logic_tx.send(TrameCommand::ClearFilter { sheet_id }),
    }
    self.ui_state.context_menu = None;  // close
}

// Single-column sort over the current selection (or the used range if a whole
// column is selected). has_header defaults to false; toggled in the menu.
fn send_sort(&mut self, sheet_id: SheetId, ascending: bool) {
    let (r1, c1, r2, c2) = self.ui_state.selection.bounds();
    self.ui_state.sort_pending = true;
    self.logic_tx.send(TrameCommand::SortRange {
        version: self.next_version(), sheet_id,
        row_start: r1, row_end: r2, col_start: c1, col_end: c2,
        sort_col: 0,  // reference column = first column of the range
        ascending, has_header: false,
    });
}
```

**Menu polish (MVP).** The current DOM menu is an unstyled flat list ("fourre-tout"). For MVP
`context_entries()` returns **grouped, iconed** entries with separators, in stable order:

```text
[Cut]  [Copy]  [Paste]                      (MdiContentCut / Copy / Paste)
---
[Insert row above] [Insert row below]       (MdiTableRowPlusBefore / After)
[Insert column left] [Insert column right]  (MdiTableColumnPlusBefore / After)
[Delete rows] [Delete columns]              (MdiTableRowRemove / TableColumnRemove)
---
[Sort ascending] [Sort descending]          (MdiSortAscending / Descending)
[Sort range...]                             (MdiSort -> multi-column dialog, below)
[Filter by column] [Clear filter]           (MdiFilter / FilterOff)
---
[Merge cells] [Unmerge]                     (MdiTableMergeCells / ...)
```

`ContextMenuItem` (`ambre-chrome`) gains an optional `icon: Option<IconId>` and a
`Separator` variant so the same shared command path renders icons + dividers (toolbar and
context menu still dispatch one `CommandId`, sec.5.C note). A **header right-click** opens a
header-scoped menu (Insert / Delete / Resize... / Sort / Filter on the clicked column or row),
sharing the same entries minus the cell-only ones.

**Multi-column sort (MVP).** "Sort range..." opens a small DOM dialog (`SortDialog` in
`trame_chrome`) listing ordered sort levels (column picker + asc/desc + a "data has header
row" toggle), with add/remove level. Applying emits the typed `SortRangeMulti`:

```rust
fn on_sort_dialog_apply(&mut self, keys: Vec<SortKey>, has_header: bool) {
    let (r1, c1, r2, c2) = self.ui_state.selection.bounds();   // whole-column sel resolved by Logic
    self.ui_state.sort_pending = true;
    self.logic_tx.send(TrameCommand::SortRangeMulti {
        version: self.next_version(), sheet_id: self.ui_state.active_sheet,
        row_start: r1, row_end: r2, col_start: c1, col_end: c2,
        sort_keys: keys, has_header,
    });
    // SortKey { col: u32 /* index within range */, ascending: bool }, decreasing priority.
    // Logic sorts lexicographically on sort_keys; SortComplete clears sort_pending (sec.2).
}
```

### D. Shared Filter Popover (MVP)

`FilterByColumn` opens a **DOM chrome popover** (Leptos `FilterPopover` in `trame_chrome`)
anchored under the active column header -- the same chrome pattern as the format bar
(sec.5bis) and the find/replace panel (sec.5.G), not a canvas `DrawSurface` overlay.
For MVP it offers the `values` kind (checkbox list of the column's distinct visible
values) and a minimal `condition` kind (op + value). Applying sends a single
`ApplyFilter`; the Logic thread recomputes `filter_hidden_{sheet}` and replies
`FilterApplied { hidden_row_count }`, after which the viewport no longer lists hidden
rows. A funnel badge is drawn on the filtered column header while `filter_active`.

```rust
fn apply_filter_from_popover(&mut self, sheet_id: SheetId, col: u32, p: &FilterPopover) {
    self.logic_tx.send(TrameCommand::ApplyFilter {
        sheet_id, col,
        kind:      p.kind,                 // FilterKind::Values | Condition
        values:    p.selected_values.clone(),
        condition: p.condition.clone(),
    });
    self.ui_state.context_menu = None;
}
```

**Filter feedback on the first row (MVP).** Today a filtered column is signalled only by a
funnel badge on the column-letter header -- weak feedback, far from the column-letter band the
data actually lives in. MVP moves the affordance to the **filter range's header row** (its first
row), Excel autofilter style: each column of the range draws a small dropdown **filter button** at
the right edge of its first-row cell (`ui::renderer::draw_overlay`, clipped to the cell), showing a
plain chevron when the column is unfiltered and a **funnel** glyph when a filter is active on it.
Clicking the button opens the existing `FilterPopover` (sec.5.D) anchored under that cell -- the
same popover the context-menu entry opens, so there is one filter code path with two entry points.

The filter range is the used range of the contiguous data block around the active cell (resolved
Logic-side, like sort). The button row is the range's first row; with a `has_header` data block the
buttons sit on the header labels (Excel), otherwise on row 1. The per-column filtered state comes
from `FilterApplied { col }` (already dispatched, §18), so the render worker tracks the set of
filtered columns and paints the funnel on exactly those buttons. The column-letter funnel badge is
kept as a secondary cue.

> **Personal filter views, conditional formatting, validation** stay in Phase 2 (`-features`
> roadmap). The shared filter (with first-row autofilter buttons), single-column sort, and
> multi-column sort are MVP.

### E. Clipboard (Copy / Cut / Paste) -- MVP

Copy is **authoritative**, built on the Logic thread from the whole sheet CRDT -- there is
**no viewport limit**: `Ctrl+A` then copy, or select-scroll-select then copy, is always
complete. The UI carries only the tracked selection; it emits `TrameCommand::CopyRange`
(parameterless, like the other selection-scoped intents), and Logic serialises the selected
cells (never culled) into a `ClipboardPayload` returned as `TrameEvent::Clipboard`. Paste of a
range also goes through the Logic thread, which reindexes relative formulas and applies styles.
This is the shared, cross-app clipboard model (`1-Foundations/ambre-spec-ui.md`): the OS write
happens lazily, one frame after the keypress, still inside the transient user-activation window.

```rust
// Ctrl+C / Ctrl+X -- request the authoritative build; nothing is serialised UI-side.
fn on_copy(&mut self, cut: bool) -> CopyOutcome {
    // In-cell text selection is synchronous text; a grid range defers to Logic.
    self.clipboard_anchor = Some(self.ui_state.selection.bounds());  // marching-ants
    self.cut_pending = cut;
    self.logic_tx.send(TrameCommand::CopyRange);
    CopyOutcome::Deferred            // host prevent_default()s, does not setData yet
}

// TrameEvent::Clipboard(payload) -- built from the full document; stash a ClipboardBlob
// (TSV + postcard) in memory (same-tab paste) and mark it pending; the host mirrors it to
// the OS clipboard next frame (poll_clipboard -> async_write_clipboard).

// Ctrl+V -- paste at the active cell. Reindexing happens in the Logic thread.
fn on_paste(&mut self, blob: ClipboardBlob) {
    // Prefer the in-memory blob when the OS text mirror still matches it (full fidelity),
    // else decode blob.ambre -> PasteRange, else grid_paste_text(blob.text).
    let (row, col) = self.ui_state.selection.active();
    self.logic_tx.send(TrameCommand::PasteRange {
        version: self.next_version(), sheet_id: self.ui_state.active_sheet,
        target_row: row, target_col: col, payload,
    });
    if self.cut_pending {
        // Clear the source range after a cut (empty SetCellText, like Delete).
        self.clear_range(self.clipboard_anchor.take());
        self.cut_pending = false;
    }
}
```

Keyboard: Ctrl+C / Ctrl+X / Ctrl+V ride the native clipboard events (they are not decoded in
`on_key_down`). Context menu: add `Copy`, `Cut`, `Paste` to `ContextMenuItem` (sec.5.C).
Render: animated marching-ants (dashed `stroke_rect`) around `clipboard_anchor` until Escape or
the next copy. A `PasteRejected` event (protected range) surfaces a toast.

### F. Autofill (Fill Handle) -- MVP

A small blue square is drawn at the bottom-right of the selection. Dragging it extends the
selection to a destination range and emits `FillRange`; the Logic thread chooses `Series`
(detected numeric/date pattern) or `Copy`.

```rust
fn on_fill_handle_drag_end(&mut self, dest: (u32, u32)) {
    let src  = self.ui_state.selection.bounds();
    let dest = self.extend_bounds(src, dest);   // src + dragged extent
    self.logic_tx.send(TrameCommand::FillRange {
        version: self.next_version(), sheet_id: self.ui_state.active_sheet,
        src: src.into(), dest: dest.into(), mode: FillMode::Auto,  // Logic picks Copy vs Series
    });
}
```
Hit-testing the handle uses a ~6px square at the selection's bottom-right corner; cursor
switches to a crosshair on hover.

**Merged-cell source (Excel behaviour).** When the source is tiled by equal merged blocks --
either one block or a uniform grid of identically sized merges -- the fill is block-aware. The
UI captures the merge-expanded source and its block size (`fill_source_block`, which checks the
whole rect is a uniform tiling); the drag preview then snaps the extension to a whole number of
blocks (`fill_dest`, a 2x1 merge fills two rows at a time) along the locked axis. On release the
Logic thread (`logic::fill::merged_source_block` + `fill_merged`) tiles the blocks: each lane
perpendicular to the drag axis extrapolates from its own stack of source blocks -- a numeric
`Series` progresses across blocks, otherwise the source blocks are copied cyclically (a copied
formula reindexes by the tile offset). Each tile master gets the value and its source block's
style, the tile's covered cells are cleared, and a merge entry is recorded for every tile -- so
the filled region is re-merged into blocks the same size as the source. The Ctrl single-vs-multi
test counts blocks, not raw cells, along the drag axis: a single block is one value (Ctrl ->
`Series` +1), several blocks are several values (Ctrl -> `Copy`). A plain (non-merged) source,
or a mix of merge sizes, keeps the per-cell fill unchanged.

### G. Find & Replace -- MVP

`Ctrl+F` / `Ctrl+H` open a **DOM chrome panel** (Leptos `FindReplace` component in
`trame_chrome`), the same approach as the grid context menu, the filter popover and the
format bar -- a floating panel pinned to the grid's top-right corner, not a canvas
`DrawSurface` overlay. The panel reuses the shared chrome plumbing: it is a piece of view
state in the chrome, and its actions cross the existing UI<->Logic command channel. Because
the UI cache only covers the viewport (sec.16), the search runs in the Logic thread:
`FindNext` returns the next match to scroll to; `ReplaceAll` mutates and returns the count.

`Ctrl+F` opens the panel in *find* mode (query field only); `Ctrl+H` opens it in *replace*
mode (query + replacement fields). Both chords route through the host's `keydown` listener
(like Copy/Cut/Paste, sec.5.E) into a UI-only intent that toggles the panel; `Escape` (or the
panel's close button) dismisses it and clears the match highlight. The panel carries the
search options as local Leptos signals: `match_case`, `whole_cell`, `in_formulas` (Replace
only), `all_sheets` (Replace only).

```rust
// FindReplace panel state, held in the chrome (DOM), not in the render worker.
struct FindReplaceState {
    query:       String,
    replacement: String,   // replace mode only
    match_case:  bool,
    whole_cell:  bool,
    in_formulas: bool,     // replace mode only
    all_sheets:  bool,     // replace mode only -> Scope::Workbook
}

// Find / Find Next: resolve the search origin from the tracked active cell, like the
// parameterless filter/sort intents (sec.5.C/5.D). The Logic thread keeps the active cell
// via SetActiveCell, so the panel need not carry the cursor.
fn on_find_next(&mut self, p: &FindReplaceState) {
    self.logic_tx.send(TrameCommand::FindNext {
        query: p.query.clone(), match_case: p.match_case, whole_cell: p.whole_cell,
    });
    // FindResult { matches, active } -> the chrome forwards `matches` (current-sheet hits)
    // and `active` (next match) to the render worker, which highlights the visible matches
    // and scrolls `active` into view.
}

fn on_replace_all(&mut self, p: &FindReplaceState) {
    self.logic_tx.send(TrameCommand::ReplaceAll {
        query: p.query.clone(), replacement: p.replacement.clone(),
        match_case: p.match_case, whole_cell: p.whole_cell,
        in_formulas: p.in_formulas, all_sheets: p.all_sheets,
    });
    // ReplaceDone { replaced } -> toast "N replaced" via chrome::notify_event.
}
```

The match scan compares against each cell's **displayed value** (`whole_cell` = exact
equality, else substring); `match_case` toggles case folding. `ReplaceAll` rewrites the raw
cell text; `in_formulas` controls whether formula-source cells (`=...`) are eligible. A
formula cell is special-cased: with `in_formulas` it matches and is rewritten on its **raw
formula text** ("search within formulas", e.g. `=A1` -> `=B2`), not on its evaluated result;
plain cells always match on the displayed value. `all_sheets` widens the scope from the
active sheet (`Scope::Sheet`) to the whole workbook (`Scope::Workbook`). The whole `ReplaceAll` is one undo group and replies `DataReset` so the
UI re-fetches the rewritten region. `FindResult` matches are local view state (never synced);
the highlight is cleared when the panel closes or the query changes.

---

## 5bis. TOOLBAR (MANUAL FORMATTING)

The formatting toolbar is a **DOM chrome bar** (Leptos `FormatBar` in
`trame_chrome::chrome::bars`) pinned above the formula bar (sec.6) -- the same approach as
the grid context menu (sec.5.C), the filter popover (sec.5.D) and the find/replace panel
(sec.5.G), reusing the shared chrome command channel rather than a canvas `DrawSurface`
overlay. Every control emits `TrameCommand::SetCellStyle { range, patch }` over the current
selection (the toggle buttons -- Bold/Italic/Underline -- and Undo/Redo route through the
shared `CommandId` path; the value controls emit a `StylePatch`); control state (e.g. the
bold toggle) is read from the **active cell's** resolved `CellStyle`, pushed to the chrome by
`TrameEvent::StyleApplied`. Styles are document mutations (LoroDoc, synchronised -- sec.16),
unlike freeze/selection which are local.

The `ToolbarAction` enum below is the conceptual action vocabulary; in the delivered Leptos
bar each control maps to either a `CommandId` (toggles, undo/redo, merge) or a `StylePatch`
field (the value controls).

```rust
pub enum ToolbarAction {
    Bold, Italic, Underline,                 // toggles
    Font(String), Size(f32),
    TextColor(Color), FillColor(Color),
    AlignH(HAlign), AlignV(VAlign),          // horizontal AND vertical
    NumberFmt(NumberFormat),                  // General / Number / Currency / Percent / Scientific / Fraction / Date / Time / Text / Custom
    IncDecimals(i8),                          // +/- decimal places on the active fmt
    Wrap(bool),
    Border(BorderEdit),                       // preset (None|All|Inner|Outline|Top|..) + pen (style+colour); per-edge model
    Merge,                                     // reuses MergeCells (sec.5.C)
}

fn on_toolbar_action(&mut self, action: ToolbarAction) {
    let range = self.ui_state.selection.bounds().into();
    let patch = match action {
        ToolbarAction::Bold       => StylePatch { bold: Some(!self.active_style().bold), ..Default::default() },
        ToolbarAction::AlignV(v)  => StylePatch { v_align: Some(v), ..Default::default() },
        ToolbarAction::NumberFmt(f)=> StylePatch { fmt: Some(f), ..Default::default() },
        ToolbarAction::Wrap(w)    => StylePatch { wrap: Some(w), ..Default::default() },
        // ... one field per control
        ToolbarAction::Merge      => return self.merge_selection(),  // existing path
        _ => self.patch_for(action),
    };
    self.logic_tx.send(TrameCommand::SetCellStyle {
        version: self.next_version(), sheet_id: self.ui_state.active_sheet, range, patch,
    });
    // StyleApplied { version } refreshes the viewport (display_text reformatted by Logic).
}
```

Color pickers and the number-format menu are small **DOM popovers** anchored under their
toolbar button (`NumberFormatPopover`, `BordersPopover`, `ColorPopover`, `H/VAlignPopover`,
`FontPopover` -- same chrome pattern as the filter popover, sec.5.D, with theme-aware
palettes). `FontPopover` is a thin adapter over the **shared** `ambre_chrome::FontPicker`
(the same control Toile's element inspector uses, `ambre-spec-toile-ui.md sec.5quinquies.B`),
so a family label resolves to the same registered face across apps
(`ambre_canvas::registered_family_name`). The Bold / Italic / Underline / Strikethrough toggles
and the font-size dropdown are likewise the **shared** `ambre_chrome::FormatToggle` /
`FontSizeSelect` (the very controls Toile's inspector and selection bar reuse, in the same
order), so the text-format cluster is one implementation across both apps; the size dropdown's
"Default" entry maps to the cell body default. The borders control is a composed popover: a "pen" (line style + colour) over a
grid of preset icon buttons; a preset click applies the current pen to its edges. Borders are
stored per edge (each of top/right/bottom/left carries its own style + colour), so a cell can
mix edges, and the selection-level preset is resolved to concrete edges per cell
(merge-aware) by the Logic thread.

---

## 6. INLINE EDITOR AND FORMULA BAR

> **Implementation note.** The **formula bar** is a DOM chrome component
> (`trame_chrome::chrome::formula_bar`), like the format bar (sec.5bis) and the other chrome
> controls -- not a canvas `DrawSurface` band. The canvas merely reserves the strip above the
> grid (`ui::renderer`: "canvas minus the formula bar band"); the bar's cell-reference box and
> editable value field are DOM. The **inline editor** is a transparent DOM `<textarea>`
> overlaid on the active cell (the "textarea pipe"), so IME, selection and clipboard come for
> free; the canvas only paints the cell's white background + blue focus border underneath. The
> pseudo-code below predates that split and is kept for the data model (`InlineEditor`,
> `EditTrigger`) and the commit/cancel flow; the literal `DrawSurface` calls are illustrative.

### A. Structure

```rust
pub struct InlineEditor {
    pub row:     u32,
    pub col:     u32,
    pub content: String,    // value being edited
    pub cursor:  usize,     // cursor position within content (byte offset)
    pub trigger: EditTrigger,
}

pub enum EditTrigger {
    Char(char),  // character keystroke -> content = String::from(c)
    F2,          // F2 -> content = current cell value
    Click,       // double-click -> same as F2
}
```

### B. Inline Editor Rendering

```rust
fn draw_inline_editor(&mut self) {
    let editor = match &self.ui_state.editing { Some(e) => e, None => return };
    let x = self.col_offset(editor.col) - self.scroll_x + self.header_width;
    let y = self.row_offset(editor.row) - self.scroll_y + self.header_height;
    let w = self.col_width_at(editor.col as usize).max(100.0); // min-width for input
    let h = self.row_height_at(editor.row as usize);

    // White background + blue border
    let rect = Rect::new(x, y, x + w, y + h);
    surface.fill_rect(rect, Color::WHITE);
    surface.stroke_rect(rect, Color::from_rgb8(26, 115, 232), 2.0);

    // Text with blinking cursor
    // ... Parley rendering of editor content with cursor
}

fn draw_formula_bar(&mut self) {
    // Fixed area at the top of the grid
    // Displays:
    // - The active cell reference (e.g. "A1") in a box on the left
    // - The raw value of the active cell (or editor content if editing in progress)
    let cell_ref_label = self.active_cell_label(); // "A1", "B10", etc.
    let formula_content = if let Some(editor) = &self.ui_state.editing {
        editor.content.clone()
    } else {
        self.current_viewport.as_ref()
            .and_then(|vp| vp.cells.iter().find(|c| {
                let (row, col) = self.ui_state.selection.active();
                c.row_index == row && c.col_index == col
            }))
            .map(|c| c.raw.clone())
            .unwrap_or_default()
    };
    // ... DrawSurface rendering of the formula bar
}
```

### C. Commit and Cancel

```rust
fn commit_editing(&mut self, direction: CommitDirection) {
    let editor = match self.ui_state.editing.take() { Some(e) => e, None => return };
    let (row, col) = (editor.row, editor.col);

    // Positional edit at the active cell; an empty `text` clears it (no ClearCell).
    self.logic_tx.send(AppCommand::Trame(TrameCommand::SetCellText {
        row, col,
        text: editor.content,
    }));
    // Close the undo group on cell exit so the whole edit = 1 undo item
    // (spec-ambre-trame-logic.md sec.8 "doc.commit() per TrameCommand").
    self.logic_tx.send(TrameCommand::CommitEdit);

    // Move selection according to direction
    match direction {
        CommitDirection::Down  => self.move_selection(0, 1),
        CommitDirection::Right => self.move_selection(1, 0),
        CommitDirection::Stay  => {}
    }
}

fn cancel_editing(&mut self) {
    self.ui_state.editing = None;
    // No TrameCommand sent -- no Loro mutation
}
```

---

## 7. AUTOCOMPLETION

Autocompletion is entirely on the UI thread side — no round-trip to the Logic thread.

### A. Functions

```rust
// Static list of supported functions (compiled in the UI binary)
const FUNCTION_COMPLETIONS: &[FunctionCompletion] = &[
    FunctionCompletion { name: "SUM",     signature: "SUM(number1, [number2], ...)",
                         description: "Adds the values" },
    FunctionCompletion { name: "AVERAGE", signature: "AVERAGE(number1, [number2], ...)",
                         description: "Calculates the average" },
    // ... ~50 MVP functions
];

fn compute_completions(&self, editor: &InlineEditor) -> Vec<Completion> {
    if !editor.content.starts_with('=') { return vec![]; }

    // Guard: cursor may be 0 (before the '=') and must land on a char boundary;
    // `content[1..cursor]` would panic on `1..0` or a mid-codepoint index.
    let Some(cursor_text) = editor.content.get(1..editor.cursor.max(1)) else { return vec![]; };

    // Find the current token (last word after =, (, +, etc.)
    let token_start = cursor_text.rfind(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|i| i + 1).unwrap_or(0);
    let token = &cursor_text[token_start..];

    if token.is_empty() { return vec![]; }
    let token_upper = token.to_uppercase();

    FUNCTION_COMPLETIONS.iter()
        .filter(|f| f.name.starts_with(&token_upper))
        .map(|f| Completion::Function(f.clone()))
        .take(8)
        .collect()
}
```

### B. Cell References

```rust
// When the user types a single letter after "=" or an operator
// -> suggest matching columns with their contents
fn compute_cell_ref_completions(&self, token: &str) -> Vec<Completion> {
    // Guard: `letter_to_index` subtracts `b'A'` per byte, so a non-A..Z token
    // (empty, digits, lowercase after upper-casing is fine, but symbols) would
    // underflow. Only proceed on a pure A..Z token.
    let upper = token.to_uppercase();
    if upper.is_empty() || !upper.bytes().all(|b| b.is_ascii_uppercase()) { return vec![]; }

    // Find columns whose letter starts with the token
    if let Some(vp) = &self.current_viewport {
        let col_start = SheetIndex::letter_to_index(&upper);
        // Return the first N non-empty cells of the column
        vp.cells.iter()
            .filter(|c| c.col_index == col_start as u32 && !c.display_text.is_empty())
            .map(|c| Completion::CellRef {
                label: format!("{}{}", token.to_uppercase(), c.row_index + 1),
                preview: c.display_text.clone(),
            })
            .take(5)
            .collect()
    } else {
        vec![]
    }
}
```

### C. Autocompletion Dropdown Rendering

Floating **DOM chrome dropdown** (Leptos, anchored under the active cell -- it sits over the
inline editor's transparent textarea, which is itself DOM, so a canvas `DrawSurface` overlay
would be the odd one out; same chrome pattern as the filter/format/find popovers), with
selected item highlighting. Navigation: up/down arrows, Tab to complete, Escape to close.
*(Phase 2.)*

---

## 8. SELECTION

The selection is a rectangle from `anchor` to `active` (`anchor == active` is just the
single-cell case). **Multi-cell selection is MVP and IMPLEMENTED** -- it is a hard
prerequisite for Merge, Delete rows/cols, `SetCellStyle`, `PasteRange`, `FillRange`,
`SortRange` and the status bar, which all read `sel.bounds()`. Extension paths (all in
`ambre-trame/src/ui/`): mouse drag (`view::handle_input_nav` arms `selecting_drag`, pointer
moves call `select_extend` -> `Selection::extend_to`), **Shift+click** (`extend_cell`),
**Ctrl/Cmd+click** for disjoint ranges (`disjoin_cell` -> `Selection::add_disjoint`, pushing
the live rectangle into `Selection.extra`), **Shift+arrows / Shift+Page / Shift+Home/End**
(`move_cursor`/`move_to` take an `extend` flag), and **Shift+Ctrl+End** to grow to the last
cached non-empty cell. `InputEvent::PointerDown` carries `shift` / `ctrl` (Cmd folded into
`ctrl` by the host). Range rendering iterates `Selection::rects()` in `renderer::draw_overlay`
(translucent fill + 1px outline per rectangle). The 2px active-cell border is drawn **only
for a lone single-cell selection** (Excel behaviour): in a multi-cell or disjoint selection
the range fill + outline already mark the block, so a separate active-cell border would read
as a spurious second selection following the cursor. Each rendered rectangle is also expanded
to whole merged blocks (`expand_to_merges`) so a drag touching part of a merge selects the
block as one cell. **Whole row/column selection by clicking a header** (the virtual axis
selection, sec.4bis.A) and the header range highlight (sec.4.B) are **MVP**: `Selection`
carries a `kind: SelKind { Cells, Cols, Rows }`; `Cols`/`Rows` mark a whole-axis band whose
perpendicular extent is resolved by the Logic thread against the used range (sec.4bis.A, sec.16).
The range fill and header tint are driven from `kind` + `anchor`/`active`.

> **Disjoint selection** is modelled by `Selection { anchor, active, extra: Vec<CellRect> }`:
> `extra` holds the committed rectangles, `anchor..=active` is the live one. `is_multi()` gates
> the range fill so a lone single cell shows only the active border. Selection stays local
> view state (never synced, sec.16).

```rust
pub struct Selection {
    /// Selection anchor cell
    pub anchor: (u32, u32),
    /// Current active cell (may differ from anchor in multi-cell selection)
    pub active: (u32, u32),
}

impl Selection {
    pub fn single(row: u32, col: u32) -> Self {
        Self { anchor: (row, col), active: (row, col) }
    }
    pub fn extend_to(&mut self, row: u32, col: u32) { self.active = (row, col); }
    pub fn active(&self) -> (u32, u32) { self.active }

    pub fn contains_col(&self, col: u32) -> bool {
        let (c1, c2) = (self.anchor.1.min(self.active.1), self.anchor.1.max(self.active.1));
        col >= c1 && col <= c2
    }
    pub fn contains_row(&self, row: u32) -> bool {
        let (r1, r2) = (self.anchor.0.min(self.active.0), self.anchor.0.max(self.active.0));
        row >= r1 && row <= r2
    }
}

fn draw_selection(&mut self) {
    let (r1, c1) = (
        self.ui_state.selection.anchor.0.min(self.ui_state.selection.active.0),
        self.ui_state.selection.anchor.1.min(self.ui_state.selection.active.1),
    );
    let (r2, c2) = (
        self.ui_state.selection.anchor.0.max(self.ui_state.selection.active.0),
        self.ui_state.selection.anchor.1.max(self.ui_state.selection.active.1),
    );

    let x1 = self.col_offset(c1) - self.scroll_x + self.header_width;
    let y1 = self.row_offset(r1) - self.scroll_y + self.header_height;
    let x2 = self.col_offset(c2) - self.scroll_x + self.header_width
             + self.col_width_at(c2 as usize);
    let y2 = self.row_offset(r2) - self.scroll_y + self.header_height
             + self.row_height_at(r2 as usize);

    let rect = Rect::new(x1, y1, x2, y2);
    // Semi-transparent blue background
    surface.fill_rect(rect, Color::from_rgba8(26, 115, 232, 40));
    // Blue border
    surface.stroke_rect(rect, Color::from_rgb8(26, 115, 232), 1.5);
}
```

---

## 9. MULTI-USER CURSORS

```rust
pub struct RemoteCursor {
    /// Canonical user identifier -- stable, per-user, not per-device.
    /// Set to the Sync-resolved `from_user_id` (pub_key -> UserId resolved in ambre-sync
    /// via trusted_keys and delivered on LogicEvent::RemoteEphemeral); NEVER read from the
    /// peer-authored Ephemeral payload (a peer cannot forge another user's id).
    pub user_id:      UserId,
    pub color:        Color,         // shared cross-app peer color (ambre_draw::peer::peer_color)
    pub row:          u32,
    pub col:          u32,
    pub display_name: Option<String>,
}

fn draw_remote_cursors(&mut self) {
    for (_, cursor) in &self.ui_state.remote_cursors {
        let x = self.col_offset(cursor.col) - self.scroll_x + self.header_width;
        let y = self.row_offset(cursor.row) - self.scroll_y + self.header_height;
        let w = self.col_width_at(cursor.col as usize);
        let h = self.row_height_at(cursor.row as usize);

        // Colored border of the collaborator's active cell
        surface.stroke_rect(Rect::new(x, y, x + w, y + h), cursor.color, 2.0);

        // Name label (small colored badge at the top of the cell)
        if let Some(name) = &cursor.display_name {
            self.draw_cursor_label(name, cursor.color, x, y);
        }
    }
}

// Update from decrypted LogicEvent::RemoteEphemeral.
// The author identity is the Sync-resolved from_user_id (pub_key -> UserId done in
// ambre-sync/trusted_keys), assembled Logic-side into TrameEvent::RemoteCursors; the peer
// payload (PresencePayload) never carries a user_id. The UI thread never manipulates
// any cryptographic key.
fn on_remote_presence(&mut self, presence: RemotePresence) {
    // RemotePresence { user_id: UserId, display_name, sheet_id, row, col }
    // emitted by the Logic thread in TrameEvent::RemoteCursors
    let color = self.color_for_user(&presence.user_id);
    self.ui_state.remote_cursors.insert(presence.user_id, RemoteCursor {
        user_id: presence.user_id,
        color,
        row: presence.row,
        col: presence.col,
        display_name: presence.display_name,
    });
}

fn color_for_user(&self, user_id: &UserId) -> Color {
    // Deterministic per-user color via the shared cross-app helper
    // `ambre_draw::peer::peer_color` (same algorithm in Prose, Trame, and Toile, so a
    // given collaborator gets the same colour everywhere). Seeded on the canonical
    // `UserId`; the helper converts through its own `hsl_to_rgb` (`ambre-draw/src/peer.rs`)
    // and returns a peniko `Color::from_rgb8`.
    ambre_draw::peer::peer_color(user_id)
}
```

---

## 10. SPARKLINES

Sparklines are mini-charts rendered inside a cell. They are computed and rendered entirely on the UI thread side from `DisplayViewport` data.

```rust
// A sparkline is described in DisplayCell via style/raw fields
// Cell syntax: "=SPARKLINE(A1:A10)" (Phase 2 feature)
// For MVP: sparkline type detected from a special style field

fn draw_sparkline(&mut self, cell: &DisplayCell, x: f32, y: f32, w: f32, h: f32) {
    // Sparkline data is in cell.sparkline_data (Phase 2)
    // For now: mini line chart from numeric values in the range
    let values = match &cell.sparkline {
        Some(s) => &s.values,
        None    => return,
    };
    if values.is_empty() { return; }

    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(1.0);
    let n = values.len();
    let padding = 2.0f32;

    let mut path = BezPath::new();
    for (i, &v) in values.iter().enumerate() {
        let px = x + padding + (i as f32 / (n - 1).max(1) as f32) * (w - padding * 2.0);
        let py = y + h - padding - ((v - min) / range) as f32 * (h - padding * 2.0);
        if i == 0 { path.move_to((px, py)); } else { path.line_to((px, py)); }
    }

    // DrawSurface has no stroke_path; a sparkline polyline is drawn segment by
    // segment with stroke_line over the (px, py) points.
    let c = Color::from_rgb8(26, 115, 232);
    for w in points.windows(2) {
        surface.stroke_line(w[0].0, w[0].1, w[1].0, w[1].1, c, 1.5);
    }
}
```

---

## 11. CHARTS (wgpu/lyon)

Charts are `DisplayFloatObject` of kind `Chart(ChartDef)`. The Logic thread stores the chart definition in `"media"` Loro. The UI thread renders the chart via `DrawSurface` (fill_rect + fill_path lyon) in the float object area.

```rust
pub struct ChartDef {
    pub kind:        ChartKind,
    pub title:       Option<String>,
    pub data_range:  String,      // range in display format "A1:B10"
    pub series:      Vec<Series>,
}

pub enum ChartKind { Bar, Line, Pie, Area }

fn draw_chart(&mut self, obj: &DisplayFloatObject, chart: &ChartDef) {
    // Chart data is in the DisplayCell corresponding to data_range
    // Extracted from current_viewport if available, otherwise requested from the Logic thread
    let data = self.extract_chart_data(chart);
    if data.is_empty() { return; }

    let x = obj.x - self.scroll_x + self.header_width;
    let y = obj.y - self.scroll_y + self.header_height;
    let w = obj.width;
    let h = obj.height;

    // White background with shadow
    surface.fill_rect(Rect::new(x, y, x + w, y + h), Color::WHITE);

    match chart.kind {
        ChartKind::Bar  => self.draw_bar_chart(&data, x, y, w, h),
        ChartKind::Line => self.draw_line_chart(&data, x, y, w, h),
        ChartKind::Pie  => self.draw_pie_chart(&data, x, y, w, h),
        ChartKind::Area => self.draw_area_chart(&data, x, y, w, h),
    }

    // Title
    if let Some(title) = &chart.title {
        self.draw_chart_title(title, x, y, w);
    }
}
```

---

## 12. PDF EXPORT VIA `krilla`

### Principle: shared `DrawSurface` trait, PDF rendering in the Logic thread

The `DrawSurface` trait is defined in `ambre-draw` (see `spec-ambre-architecture.md` §2.D).
The shared drawing functions (`draw_cells_on`, `draw_grid_lines_on`, etc.) live in
`ambre-trame/src/render.rs` — they are parameterized over `DrawSurface` and documented in
`spec-ambre-trame-logic.md` §9 (shared rendering module).

- **UI thread**: instantiates `WgpuSurface` (defined in `ambre-app`) → live GPU rendering via the shared functions.
- **Logic thread**: instantiates `KrillaSurface` (defined in `ambre-trame/src/export_pdf.rs`, lazy-loaded) → vector PDF export. The Logic thread emits `TrameEvent::ExportComplete`; the UI thread triggers the download.

**Why PDF generation in the Logic thread and not the UI thread:**
Generating a PDF for a 100-sheet document can exceed 500ms. Blocking the UI thread
during that time would freeze the interface. The Logic thread can take the necessary time
without impacting FPS.

### Pure vector, no rasterization

High-fidelity PDF export requires that all primitives (rectangles, text, borders,
charts) be emitted directly as vectors to `krilla::Surface`.
**Rasterizing GPU pixels and re-embedding them in the PDF is forbidden** — this would
produce a multi-page screenshot, not a vector PDF.

> **Rectangular clipping:** `DrawSurface::push_clip(Rect)` / `pop_clip()` are implemented CPU-side in `WgpuSurface` (clipping vertices outside the rect before push). For `KrillaSurface`, if krilla does not support native clipping at MVP, we emulate by **truncating text at the cell edge before rendering** (Parley measurement) -- **no ellipsis**, to match the screen path exactly (§4.A "the text is clipped at the cell edge (no ellipsis in MVP)"). Keeping the same clip-at-edge rule is what preserves the "same code, same output" guarantee; an ellipsis on the PDF path only would break it.

> **Fonts on the Logic thread.** PDF text shaping needs a font database, which the
> screen path gets from the UI/GPU side. The Logic thread therefore seeds its own
> reusable **base-font layer** (a parley `FontContext` with the bundled families)
> eagerly at startup, so `KrillaSurface` can shape and emit vector glyphs without
> any UI/GPU dependency (landed: base-font layer + eager Logic-thread seeding).

### `DrawSurface` trait — source of truth in `ambre-draw`

The `DrawSurface` trait is defined in `ambre-draw` (see `spec-ambre-architecture.md` §2.D).
The Trame spec does not redefine its signature. See `spec-ambre-trame-logic.md` §9 for
the parameterized drawing functions (`draw_cells_on<S>`, etc.) and the concrete
implementations (`WgpuSurface`, `KrillaSurface`).

### `WgpuSurface` — UI thread (in `ambre-app`)

```rust
/// wgpu backend -- GPU screen rendering. Instantiated in ambre-app (UI thread).
pub struct WgpuSurface<'a> {
    renderer: &'a mut QuadRenderer,
    clip_stack: Vec<Rect>,  // CPU-side clipping applied
}
impl DrawSurface for WgpuSurface<'_> { /* ... */ }
```

### Usage from the UI thread

```rust
// UI thread -- in UiState::render()
let mut surface = WgpuSurface::new(&mut gpu.quad_renderer);
trame::renderer::draw_grid(&mut surface, &self.trame, &mut gpu.font_cx, &mut gpu.text_cache, ...);
```

### `KrillaSurface` and the PDF pipeline — Logic thread

`KrillaSurface` impl and the full PDF pipeline live in `ambre-trame/src/export_pdf.rs`
(Logic thread, lazy-loaded). See `spec-ambre-trame-logic.md` §9 for the code.

### GridLayout for PDF

```rust
pub struct GridLayout {
    pub col_offsets:   Vec<f32>,
    pub row_offsets:   Vec<f32>,
    pub col_widths:    Vec<f32>,
    pub row_heights:   Vec<f32>,
    pub header_width:  f32,
    pub header_height: f32,
    // For PDF: no scroll_x/scroll_y, offset = 0
    pub scroll_x:      f32,
    pub scroll_y:      f32,
}
```

---

## 13. SHEET TABS

```rust
fn draw_sheet_tabs(&mut self) {
    let tab_height = 24.0f32;
    let y = self.window_height - tab_height;
    let mut x = 0.0f32;

    for (sheet_id, sheet_name) in &self.ui_state.sheets {
        let is_active = *sheet_id == self.ui_state.active_sheet;
        let tab_width = self.measure_text(sheet_name) + 16.0;

        let rect = Rect::new(x, y, x + tab_width, y + tab_height);
        let bg = if is_active { Color::WHITE } else { Color::from_rgb8(230, 230, 230) };
        surface.fill_rect(rect, bg);

        // Thicker top border if active
        let border_width = if is_active { 2.5 } else { 1.0 };
        let border_color = if is_active { Color::from_rgb8(26, 115, 232) } else { Color::GRAY };
        surface.stroke_rect(rect, border_color, border_width);

        self.draw_tab_text(sheet_name, x, y, tab_width, tab_height);
        x += tab_width + 2.0;
    }

    // "+" button for new sheet
    self.draw_add_sheet_button(x, y);
}

fn on_sheet_tab_click(&mut self, sheet_id: SheetId) {
    self.logic_tx.send(TrameCommand::ActivateSheet { sheet_id });
    self.ui_state.active_sheet = sheet_id;
    self.scroll_x = 0.0;
    self.scroll_y = 0.0;
    self.request_viewport();
}
```

### Full Sheet Lifecycle (MVP)

The tab strip is the entry point for the four lifecycle commands the Logic thread
already implements (`-logic` sec.2.D). The `SheetList` event refreshes the strip after
each one.

```rust
// "+" button -> create. The Logic thread mints a peer-scoped sheet_id and replies SheetList.
fn on_add_sheet_button(&mut self) {
    self.logic_tx.send(TrameCommand::CreateSheet { name: self.next_sheet_name() });
}

// Double-click a tab -> inline rename editor on the tab label.
fn on_sheet_tab_rename_commit(&mut self, sheet_id: SheetId, name: String) {
    if !name.trim().is_empty() {
        self.logic_tx.send(TrameCommand::RenameSheet { sheet_id, name });
    }
}

// Right-click a tab -> tab context menu { Rename, Delete }.
// Delete is rejected by the Logic thread if it is the last sheet (TrameEvent::Error -> toast).
fn on_sheet_tab_delete(&mut self, sheet_id: SheetId) {
    self.logic_tx.send(TrameCommand::DeleteSheet { sheet_id });
    // Active-sheet fallback is decided by the Logic thread; UI updates active_sheet
    // from the next SheetList + ViewportData.
}

// Drag a tab horizontally -> reorder. to_index is the drop slot in the strip.
fn on_sheet_tab_drag_drop(&mut self, sheet_id: SheetId, to_index: u32) {
    self.logic_tx.send(TrameCommand::MoveSheet { sheet_id, to_index });
}
```

---

## 13bis. STATUS BAR (SELECTION AGGREGATES)

A thin band above the sheet-tab strip (sec.13) shows aggregates of the current selection,
like Excel/Sheets (Sum / Average / Count). The selection can exceed the viewport, so the
**Logic thread computes** the aggregates -- the UI cannot, holding only the visible cells
(sec.16). On each non-trivial selection change the UI requests them; for a single cell it
shows nothing.

> **Delivered as DOM Leptos chrome** (`trame_chrome::StatusBar`), like every other E3
> surface -- the `DrawSurface` sketch below predates the chrome-in-DOM decision. The request
> is parameterless (`RequestSelectionStats`): the Logic thread resolves the tracked selection
> itself. The same band also hosts the perf HUD (fps + frame timings), migrated out of the
> wgpu overlay -- a permanent `<fps>` chip that expands a diagnostics panel on click.

```rust
fn on_selection_changed(&mut self) {
    let sel = self.ui_state.selection;
    if sel.is_single() { self.status_stats = None; return; }
    // Parameterless: the Logic thread resolves the active sheet and the tracked
    // selection itself (kept current by SetActiveCell) -- no sheet_id/range field.
    self.logic_tx.send(AppCommand::Trame(TrameCommand::RequestSelectionStats));
    // SelectionStats { sum, avg, count, count_num, min, max } -> cache + redraw.
}

fn draw_status_bar(&mut self) {
    let Some(s) = &self.status_stats else { return };
    // e.g. "Sum: 1 234   Avg: 205.7   Count: 6"  (count_num drives Sum/Avg visibility)
    // DrawSurface fill_rect band + right-aligned draw_glyphs.
}
```

---

## 14. QUALITY & TESTABILITY

> What this module guarantees, and what verifies each guarantee, is the table in §1. This section holds only what is not a claim about the tree's contents.

* **Tiers, as measured.** This section used to open with "UI tests are run with `wasm-bindgen-test` in a headless browser" and list eight such tests, **none of which existed**. The real split is the opposite: **58 host tests** (`ui::grid`, `ui::renderer`, `render`) against **13 wasm tests** (`tests/chrome_ui.rs`). That is the host-extraction seam of architecture §4 working as intended -- geometry, hit-testing, selection, editor state and the draw call sequence are pure `data -> data` code and belong on the fast tier. The wasm tier keeps what is irreducibly DOM.
* **Seams.** `RecordingSurface` (`ambre-draw`, feature `testing`) for every "what to draw" assertion -- ordered `DrawOp` sequence, geometry, palette, clip balance -- with no GPU. `MockSyncHandle` / `CommandSink` for the outbound `TrameCommand`s. The wgpu pipeline itself ("how to draw") is `qa-manual` by decision (architecture §4).
* **Mandatory belts.** A new interaction lands with (a) a geometry round-trip -- drawn rect against hit-test, under zoom and variable metrics (`TRAME-UI-007`), and (b) a `RecordingSurface` assertion for anything it paints.
* **Definition of Done.** An increment is `[x]` only when those belts are green in CI **and** no invariant in §1 is left without a `Status`.

---

## 15. PERFORMANCE

| Operation | Target | Note |
|---|---|---|
| Frame rendering at 60 FPS (budget) | < 16 ms | GPU-bound (vertex/fragment shaders) |
| Scroll (send ScrollViewport) | < 1 ms | UI thread non-blocking |
| Receive + render `TrameEvent::ViewportData` | < 5 ms | typed value by move (no deserialization) + vertex batch build |
| Autocompletion (computation + dropdown rendering) | < 2 ms | Static lookup, no round-trip |

---

## 16. SOURCE OF TRUTH MODEL BY STATE TYPE

A state can exist in multiple layers. The rule is simple: **a single source of truth, other layers are projections of it**.

| Category | Examples | Source of truth | Synchronized via SecSync |
|---|---|---|---|
| **Document mutation** | SetCellText, InsertRow, MergeCells, styles | `LoroDoc` (Logic thread) | Yes |
| **Shared metadata** | Shared filters, conditional formatting, validations, named ranges | `LoroDoc` (Logic thread) | Yes |
| **Local view** | Freeze, personal filtered views, selection, inline editor, autocompletion | UI State / IndexedDB | No |
| **Derived state** | `formula_cache`, `dirty` set, `filter_hidden` | Logic thread (recomputed from LoroDoc) | No |
| **Local history** | Undo stack (`UndoManager`) | Logic thread (in memory, current session) | No |

**Fundamental invariant:** the UI thread contains no document data — only view states and the `DisplayViewport` received from the Logic thread. Every mutation goes through a `TrameCommand`. There are no two mental models of the document: `LoroDoc` is the single source of truth.

**`ViewportPatch` — Phase 2 optimization:**

When only a few cells change in the current viewport (remote P3 import on visible cells), the Logic thread can send a differential patch rather than a full viewport:

```rust
// Addition to TrameEvent -- Phase 2
TrameEvent::ViewportPatch {
    version:       u64,
    changed_cells: Vec<DisplayCell>,   // only the modified cells
}
```

The UI thread applies the patch on the existing `current_viewport`. Avoids serialization and deserialization of a full viewport for partial updates. The decision to send a patch vs a full viewport is in the Logic thread based on change size (threshold: < 10% of visible cells → patch).

---

## 17. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **DOM/HTML for grid rendering** | Insufficient performance at >10k visible cells. wgpu/QuadRenderer is the fundamental architectural choice of AMBRE. |
| **Canvas 2D (non-WebGL)** | Same reason + no quality antialiasing. wgpu (WebGL2/WebGPU) for consistency across all AMBRE apps. |
| **Freeze synchronized between collaborators** | Same behavior as Google Sheets: the freeze view is a local preference. Synchronizing it would generate unnecessary conflicts (each user has a different screen size). |
| **Autocompletion via Logic thread round-trip** | Unacceptable latency for an interactive feature. The function list is static and compiled into the UI binary. Cell values are in the current `DisplayViewport`. |
| **Remote cursors without encryption** | Ephemeral payloads are E2EE encrypted (see `spec-ambre-sync`). The server never sees the cursor or the name. The UI thread receives the already-decrypted payload from the Logic thread. |
| **GPU pixel rasterization for PDF** | Would turn the export into a multi-page screenshot. The `DrawSurface` trait abstracts drawing primitives — the rendering code is reused as-is to `KrillaSurface` for a purely vector PDF. |
| **PDF in the UI thread** | Generating a 100-sheet PDF can exceed 500ms — would block rendering. `KrillaSurface` is instantiated in the Logic thread; the UI thread receives `TrameEvent::ExportComplete` and triggers the download. |
| **Peer identification by `pub_key [u8;32]`** | The public key is a per-device cryptographic credential, not an identity. It changes on key rotation. `UserId: Uuid` is the canonical per-user identifier. The `pub_key → UserId` resolution is done in `ambre-sync` via `trusted_keys` — it does not leak into `RemoteCursor`. |
| **Business logic in the UI thread** | The UI thread computes no formulas, accesses no `LoroDoc`, never mutates the document directly. Everything goes through `TrameCommand`. Local states (selection, freeze, autocompletion) are view states, not document states. |

---

## 18. ROADMAP

> Tracked as Stories of Epic **E3** (Trame MVP) in `specs/ambre-roadmap.md`. Items already delivered
> under **E0**/**E1** (virtual scroll, inline cell editor) are checked below.

**Phase 1 — MVP**
- [x] Virtual scrolling + grid rendering *(delivered under `ARCH P2`; the shared `ambre_trame::render` module has since landed -- `GridLayout` + the `DrawSurface`-generic `draw_cells_on` / `draw_grid_lines_on` / `draw_col_headers_on` / `draw_row_headers_on`, composed by `ui::renderer`. Only the `KrillaSurface` PDF consumer of those primitives remains, in Phase 3.)*
- [x] UI-only layers on top of shared rendering: scroll offset, dirty opacity, error indicators
- [x] Column/row headers (via `ambre_trame::render::draw_col_headers_on<S>` / `draw_row_headers_on<S>`, wired in `ui::renderer::draw_overlay`)
- [x] Single-cell selection *(via `ui::grid::Selection` -- `anchor == active`; click `select()` / arrow nav, active border + header tint in `draw_overlay`)*
- [x] Multi-cell selection (sec.8): drop the `anchor == active` constraint, Shift+arrows / Shift+click extension (drag already in sec.5.B), range highlight (`draw_selection`) + range header highlight (sec.4.B), whole row/column selection by header click. *(Hard prerequisite for Merge/Delete-rows-cols/SetCellStyle/Paste/Fill/Sort/status-bar, all of which consume `sel.bounds()`.)*
- [x] Inline editor *(delivered under `CHROME P2` -- Trame WYSIWYG cell)*;
- [x] Formula bar
- [x] Sheet tabs + full lifecycle (create via "+", rename via double-click, delete/rename via tab context menu, reorder via drag) -> `CreateSheet` / `RenameSheet` / `DeleteSheet` / `MoveSheet` (sec.13) *(Originally a canvas bottom strip. **Since migrated to DOM chrome** by the E3 polish pass: an app-agnostic `<Tabs>` AppShell slot + the `SheetTabs` component (`trame_chrome::sheet_tabs`) -- tabs, "+", inline rename, `{Rename,Delete}` menu, HTML5 drag-reorder over `MoveSheet` -- fed by `SheetList`-mirrored signals. The canvas tab rendering, hit-testing and input routing are gone and `grid_bottom` spans the full canvas. The sec.13 sketch below is kept as the design rationale, not as the shipped shape.)*
- [x] Keyboard navigation (Arrows, Page Up, Page Down, Tab/Shift+Tab, Enter/Shift+Enter, Delete/Backspace, End, Home, Ctrl+End, Ctrl+Home) *(nav-mode dispatch in `ui::view`; absolute jumps via `TrameUiState::move_to` + cache-scan helpers `last_content_col`/`last_content_cell` for End/Ctrl+End -- UI-thread cache is the only used-range view, sec.16; Delete/Backspace clear via empty `SetCellText`; F2 opens the editor)*
- [x] `CommitEdit` on cell exit (closes the undo group -- 1 cell edit = 1 undo item, sec.6.C)
- [x] Grid context menu (right-click, sec.5.C): row/column insert+delete, merge/unmerge. *Delivered through the existing **DOM** context menu (`ambre-chrome::ContextMenu`, populated by `trame_chrome::context_entries()`) rather than a canvas `DrawSurface` overlay -- it reuses the shared command path (toolbar / keyboard / menu all dispatch one `CommandId`). The menu fires parameterless intents (`InsertRowAbove/Below`, `InsertColLeft/Right`, `DeleteSelectedRows/Cols`, `MergeSelection`, `UnmergeActive`); the Logic thread resolves their geometry against the active cell / selection rectangle it now tracks (`SetActiveCell` carries the anchor corner). See the sec.5.C note.*
- [x] Cell merge rendering (`DisplayCell.is_merged` / `merge_span`) + `MergeRejected` / `MergeConflictDetected` handling (sec.2) *(shared `render::draw_merges_on` primitive + `GridLayout::merge_rect`: paints over the covered cells' interior separators, lays the master's text across the span, re-strokes the span border. Wired into `ui::renderer::draw_cells` after the grid-line pass, iterating the bounded cache so a master pinned just outside a dirty strip still repaints the in-strip part of its span. `MergeRejected` -> warning toast and `MergeConflictDetected` -> info toast already dispatched via `trame_chrome::notify_event`.)*
- [x] Single-column shared sort (`SortRange` from the context menu, sec.5.C) + `SortComplete` dispatch *(parameterless `SortSelectionAsc`/`SortSelectionDesc` intents through the shared DOM command path -- `SORT_ASC`/`SORT_DESC` `CommandId`s in `trame_chrome` (`COMMANDS`, `command_to_app`, `context_entries`); the Logic thread resolves the geometry against the tracked selection (`sort_selection` -> `sort_range` on the active sheet, ref column = first column, no header). `SortComplete` -> info toast via `chrome::notify_event`. Two scopes per sec.5.C: `SortSelectionAsc`/`SortSelectionDesc` sort the selection rectangle on its first column; `SortSheetAsc`/`SortSheetDesc` sort the whole sheet's used range by the active cell's column (whole-column selection), keeping every row aligned across all columns.)*
- [x] Shared filter: filter popover (`ApplyFilter` / `ClearFilter`, sec.5.D) + funnel badge + `FilterApplied` / `FilterCleared` dispatch *(DOM `FilterPopover` leptos component in `trame_chrome`, opened by the `FILTER_COLUMN` UI-only intent and anchored under the active cell via the published selection anchor. Two MVP kinds: "by values" (checkbox list of the column's distinct visible values) and "by condition" (op + value). The values list is fed by a read-only round-trip -- `TrameCommand::RequestActiveColumnValues` -> `TrameEvent::ColumnValues { col, values }` (Logic resolves the active column, evaluates non-hidden rows through the formula cache, dedups+sorts). Apply/clear go through parameterless siblings resolved against the tracked active cell -- `ApplyFilterToActiveColumn { kind }` / `ClearActiveFilter` (the latter on the shared `CLEAR_FILTER` command path; both are context-menu entries). `FilterApplied` now carries `col` so the render worker tracks `filtered_col` and draws a funnel badge on the column header (`ui::renderer::draw_overlay`). `FilterApplied`/`FilterCleared` already toast via `chrome::notify_event`. **Row collapse:** filtered rows are hidden UI-side (the spec's "the UI Thread collapses such rows") -- `DisplayCell.is_filtered` flows into `CellData`, the UI derives a sorted `hidden_rows` set, and the row axis collapses over them through `GridLayout::{visible_offset,row_at_slot}` (cell positioning, row-number header, hit-testing) plus the UI's `cell_pixel_rect`/`range_pixel_rect`/`cell_at` and vertical navigation (which skip hidden rows). The Logic position<->stable-id contract is untouched: commands stay in absolute grid coordinates.)*
- [x] Formatting toolbar (sec.5bis): bold/italic/underline, font+size, text+fill color, H/V alignment, number format (text/number/currency/percent/scientific/fraction/date/time/custom + decimals), borders, wrap -> `SetCellStyle` + `StylePatch`; `StyleApplied` dispatch *(Delivered as a **DOM** chrome bar (`trame_chrome::chrome::bars::FormatBar`) pinned above the formula bar, not a canvas `DrawSurface` overlay -- same approach as the context menu / filter popovers, reusing the shared command channel. Bold/Italic/Underline/Undo/Redo go through `on_command` (`CommandId`); the value controls (font + size, text + fill colours, H/V alignment, number format, borders, wrap) emit a `StylePatch` via `on_style` -> `SetCellStyle`. Number format is a `123` icon popover; borders, colours and alignment are icon popovers with theme-aware palettes (`NumberFormatPopover`, `BordersPopover`, `ColorPopover`, `H/VAlignPopover`, `FontPopover`). Buttons reflect the active selection's style and grey out inactionable undo/redo and decimals controls. `StyleApplied` toasts via `chrome::notify_event`.)*
- [x] Clipboard (sec.5.E): copy/cut local from the viewport, paste via `PasteRange` (Logic reindexes formulas+styles), marching-ants, Ctrl+C/X/V + context-menu entries + Ctrl+Inser/Shift+Inser/Shift+Del; `PasteRejected` toast *(Copy/cut serialise the live selection from the viewport cache into a typed `ClipboardPayload` (`ambre-bus`, serde) written to the OS clipboard as base64 postcard under `application/x-ambre-trame` plus a tab-separated `text/plain` fallback; the host's `copy`/`cut`/`paste` DOM listeners service the Ctrl+C/X/V (and Ctrl+Insert/Shift+Insert/Shift+Del) chords. Paste sends `TrameCommand::PasteRange`; the Logic thread reindexes every relative reference by the paste offset (`formula::shift`, parse -> shift -> render; off-grid -> `#REF!`; AMBRE has no `$` absolute syntax) and writes the carried `CellStyle`/bold/italic verbatim, as one undo group. An external TSV paste lands as literals (`src == target`, no shift). Cut clears the source after paste; the source range crawls with animated marching-ants (`renderer::dashed_h`/`dashed_v`) until Escape or the next copy. Toolbar Copy/Cut/Paste buttons (`MdiContentCopy`/`MdiContentCut`/`MdiContentPaste`) and context-menu entries drive the browser clipboard via the shared command path (`COPY`/`CUT`/`PASTE` UI-only intents). Number format/decimals are not carried (the viewport does not surface them) and `PasteRejected` is wired through `chrome::notify_event` but only fires once range protection lands (Post-MVP).)*
- [x] Autofill / fill handle (sec.5.F): drag -> `FillRange` (Copy | Series) *(A small blue handle square is drawn at the selection's bottom-right corner (`ui::grid::fill_handle_rect`/`fill_handle_at`, `FILL_HANDLE_SIZE`); a press on it arms a fill drag (`TrameView::fill_drag`) instead of a new selection. Pointer moves grow a dashed destination preview (`TrameUiState::fill_preview`, drawn in `ui::renderer::draw_overlay`), locked to the dominant axis (`fill_dest`); the release emits `TrameCommand::FillRange { src, dest, mode }` and grows the selection over the filled range. Hovering the handle switches the canvas cursor to a crosshair (`TrameView::set_fill_cursor` on `#ambre-canvas`). The mode follows Excel/Sheets: a plain drag sends `Auto` (Logic picks Series vs Copy); holding Ctrl inverts it -- `Series` (+1) on a single-cell source, `Copy` on a multi-cell one. The Logic thread (`logic::fill`) reads the source cells from its document model and, per fill line (a column for a vertical fill, a row for a horizontal one), either extrapolates a numeric arithmetic `Series` or tiles/`Copy`s the source -- reindexing a copied formula's relative refs by its tile offset (`formula::shift`, as in `PasteRange`) and carrying each source cell's style (`apply_pasted_style`). `Auto` picks Series only when a line's source values form an arithmetic numeric progression. One undo group; replies `DataReset`.)*
- [x] Find & replace panel (sec.5.G): DOM chrome panel (Leptos `FindReplace` in `trame_chrome`, like the filter popover / format bar -- not a canvas overlay). Ctrl+F (find) / Ctrl+H (replace) -> `FindNext` / `ReplaceAll`; `FindResult` / `ReplaceDone` dispatch
- [x] Cell wrap + vertical alignment + empty-neighbour overflow rendering (sec.4.A/4.C) *(All three honoured in the shared `render::draw_cells_on` primitive. **Wrap** constrains the parley layout to the cell content width (`col_width - 2*CELL_PAD_X`) so lines break; **vertical alignment** comes from `cell_v_offset` (Top/Middle/Bottom, clamped `>= 0`). **Empty-neighbour overflow:** a non-wrapped single line wider than its column spills into the adjacent **empty** cells -- `spill_width` walks the neighbours in the spill direction (left text spills right, right text spills left, centred both ways), accumulating `col_width` until the first non-empty neighbour, the grid edge, or the text extent; the clip rect is extended over the covered empties and the run is shifted so its anchored edge stays put. A non-empty neighbour clips the text at the cell edge (no ellipsis in MVP). The UI caller (`ui::renderer::draw_cells`) feeds occupancy via an `is_empty` closure over `cell_cache` (a merge master keeps its text, so overflow stops at a merged block). Wrapped or multi-line (explicit-newline) text is always clipped to its own box.)*
- [x] Status bar (sec.13bis): selection aggregates via `RequestSelectionStats` -> `SelectionStats` *(Delivered as **DOM** Leptos chrome (`trame_chrome::StatusBar`), like every other E3 surface -- not a canvas `DrawSurface` band. `RequestSelectionStats` is **parameterless** (the Logic thread resolves the tracked selection rectangle itself, like the sort / filter / `RequestActiveColumnValues` siblings, rather than carrying `sheet_id`/`range` as the sec.13bis sketch showed). `logic::selection_stats` walks the selection (skipping filter-hidden rows) through the formula cache and replies `SelectionStats { sum, avg, count, count_num, min, max }`; `count_num` gates Sum/Avg/Min/Max visibility. The view (`ui::view`) requests it on every multi-cell selection change (`announce_active` -> `refresh_selection_stats`) and clears the `sel_stats` signal on a single cell; the launcher's event loop mirrors the reply into the status bar. **HUD migration:** the perf read-out (fps + frame/drain/draw/gpu/sub-phase timings + glyph misses) moved out of the wgpu overlay (`ui::renderer::draw_overlay` no longer shapes debug glyphs) into the same DOM status bar: the canvas view publishes a `PerfSnapshot` each frame (`ambre_canvas::PerfPublisher`), the relay pumps it (~2 Hz) into a signal, and the status bar shows a permanent `<fps> fps` chip that expands a diagnostics panel on click.)*
- [x] **Header interactions (sec.4bis): whole-column/row selection, hover cursors, resize, inline insert.** Header hit-zones (`HeaderHit`: body / border / insert / corner) resolved by `TrameUiState::header_hit`, variable-metric aware. (a) **Virtual axis selection** -- left-click/drag a header selects a whole column/row (`Selection.kind: SelKind { Cells, Cols, Rows }` + `Selection::axis`), Shift extends; corner = select-all; the perpendicular extent is bounded by the UI used-range cache (`last_content_cell`) and the band rendered as a full-axis tint in `draw_overlay`. (b) **Hover cursors** on `#ambre-canvas` (`TrameView::set_cursor`, generalising `set_fill_cursor`): col/row-resize on borders, select pointer on bodies, crosshair on the fill handle. (c) **Resize by dragging the header boundary** -> `ResizeCol` / `ResizeRow` (synchronised CRDT mutation, undoable, LWW per uid; new `logic/metrics.rs` + `col_widths_{sheet}`/`row_heights_{sheet}` maps; viewport now carries the per-axis slices into a **variable-metric `GridLayout`**), live guide line, `MIN_COL_WIDTH`/`MIN_ROW_HEIGHT` clamp, applied to every band of a whole-axis selection. **Double-click the boundary = auto-fit** (UI-side over cached cells via the parley font ctx, resolved in `paint_content`, reuses `ResizeCol`/`ResizeRow`). (d) **Inline "+" insert affordance** in the top/left sliver of a boundary band -> existing `InsertColLeft`/`InsertRowAbove` intents. *(Whole-axis selection is also the prerequisite for the resize-whole-column and used-range sort/filter behaviours.)*
- [x] **Context menu polish (sec.5.C): grouped, iconed entries + separators + header-scoped menu.** `MenuEntry` (`ambre-chrome`) became an enum -- `Item { command, label, icon: Option<icondata::Icon> }` + a `Separator` variant -- and the `ContextMenu` component now takes a reactive `Signal<Vec<MenuEntry>>`, renders the leading icon (`leptos_icons`) in a fixed gutter and the dividers (`.ambre-ctx-sep`), and skips separators in the roving-tabindex / typeahead navigation. `context_entries_for(CtxScope)` returns the grouped layout: cell scope = clipboard (`Cut`/`Copy`/`Paste`) | insert+delete | sort+filter | merge (`Merge`/`Unmerge`); header scope drops the cell-only clipboard + merge groups, keeping insert+delete | sort+filter. A header right-click selects the whole column/row (corner = select-all) and publishes `CtxScope::Header` through a launcher-owned `RwSignal` (set in the view's right-click `PointerDown`, which runs synchronously before the DOM `contextmenu` listener opens the menu); the shell derives the layout from a `Memo` over that scope. `CtxScope` lives in `ui::grid` (always compiled, so the host-tested input handler sets it) and is re-exported by the chrome. Belts: `wasm-bindgen-test`s in `chrome_ui.rs` -- `cell_menu_has_clipboard_and_merge`, plus the per-scope belts the later `CtxScope::{Cell,Column,Row,Corner}` split produced: `column_menu_keeps_column_ops_drops_cell_and_row_ops`, `row_menu_keeps_row_ops_drops_column_inserts`, `corner_menu_has_sheet_sort_only`, `filter_entry_toggles_with_filter_state`. *(The "Sort range..." multi-column dialog entry is the next story.)*
- [x] **Multi-column sort (sec.5.C): `SortRangeMulti` from a `SortDialog`** (ordered levels: column + asc/desc + has-header toggle, add/remove level). *(was `-features` / `-ui` Phase 2; promoted to MVP.)* *Delivered: the `SortDialog` DOM component (`trame_chrome::sort_dialog`) renders ordered sort levels (each a range-relative column picker + ascending/descending select), a "data has header row" toggle, and add/remove-level controls over a click-to-close backdrop. "Sort range..." is a new `SORT_RANGE_MULTI` `CommandId` in the context-menu sort/filter group (`MdiSortVariant`); like `FILTER_COLUMN`/`FIND`, it is a UI-only intent the launcher intercepts to open the dialog over a snapshot of the canvas-published selection bounds (a new `sel_bounds` `RwSignal`, set by the view alongside `ctx_scope`/`sel_stats` on each selection change, so the column picker offers exactly the selection's columns). Applying emits the **parameterless** `TrameCommand::SortSelectionMulti { sort_keys, has_header }` -- the same Logic-resolves-the-geometry convention as the single-column `SortSelectionAsc` (the launcher does not track the active sheet) -- which delegates to `sort_selection_multi` -> `sort_range_multi` on the active sheet. The typed ranged `SortRangeMulti` is also wired in the dispatcher. The sort is the same pure `rows_{sheet}` permutation as the single-column sort, only the comparator changes: lexicographic over `sort_keys` (decreasing priority, each key its own direction), `compare_cell_values` per column, uid-anchored bytecode preserved so references follow the data. Emits `SortComplete` (+ `DataReset` on a real reorder). Belts -- logic `tests.rs`: `sort_range_multi_lexicographic_with_per_key_direction`, `sort_selection_multi_resolves_against_tracked_selection`, `sort_range_multi_pins_header_row`, `sort_range_multi_no_keys_is_noop` (empty / out-of-range keys), `sort_range_multi_already_sorted_emits_no_delta`; chrome `chrome_ui.rs`: a DOM `wasm-bindgen-test` (`sort_dialog_adds_levels_and_emits_on_apply` -- mounts the dialog, adds a level, toggles the header row, clicks Sort and asserts the emitted `SortApply` carries two distinct keys + `has_header`), the `command_to_app(SORT_RANGE_MULTI).is_none()` UI-only-intent assertion, and `SORT_RANGE_MULTI` folded into the per-scope menu belts.*
- [x] **Filter feedback on the first row (sec.5.D): Excel-style autofilter buttons** drawn on the filter range's header row (chevron unfiltered / funnel when filtered), opening the existing `FilterPopover`. The render worker tracks the filtered-column set from `FilterApplied { col }`. *(The "filter range" is the Excel-style current region around the active cell, resolved UI-side from the viewport cache -- `TrameUiState::current_region` / `filter_button_rect` / `filter_button_at` (`ui/grid.rs`); the buttons are always-on so they double as the entry point even before any filter. `ui::renderer::draw_overlay` paints a bordered button per visible region column with a `draw_chevron` / `draw_funnel` glyph keyed off `filtered_col`; the column-letter funnel badge is kept as a secondary cue. A left-click selects the button's column and raises a one-shot `ambre_canvas::FilterOpenPublisher` intent (canvas->launcher, since the view owns no `CommandId` channel); the per-frame relay in `ambre-app` consumes it and runs the exact body of the context-menu `FILTER_COLUMN` branch -- one filter code path, two entry points. The filter operation itself stays Logic-side via `ApplyFilterToActiveColumn` / `RequestActiveColumnValues`.)*
- [x] `on_logic_message` wiring: full, **exhaustive** `TrameEvent` dispatch *(one entry point per side over the real `ambre-bus::TrameEvent` enum, no `_ => {}` wildcard so a future variant is a compile error rather than a silent drop). **Canvas side** -- `ui::view::TrameView::dispatch_trame` (called from the single `UiEvent::Viewport(AppEvent::Trame(..))` arm in `update`): `ViewportData` -> cache merge + `data_dirty`; `SheetList` -> tab strip; `Error` -> log + a11y announce; `FilterApplied`/`FilterCleared` -> funnel-badge column; `FindResult` -> highlight set + `nav_goto`; `DataReset`/`CondFormatsUpdated` -> viewport re-fetch; every chrome-only variant is an explicit no-op. **Chrome side** (per-frame relay in `ambre-app`, fanned out to `trame_chrome`): `notify_event` toasts the user-facing variants (rejections, validation error/warning, protection denied/violation, merge conflict, sort/filter completion, **`ReplaceDone` -> "N cells replaced"**), `mirror_event` maps `UndoState` to the Undo/Redo toggle enablement, `apply_style_event` drives the formatting toolbar from `StyleApplied`; `ColumnValues`/`FilterApplied`/`FilterCleared`/`FindResult`/`SelectionStats` also feed the filter popover / find counter / status bar inline (the canvas treats them all as no-ops). The original sketch also named `EditAck`, `FormulaError`, `ImportProgress`, `ExportComplete`: these are **not** enum variants -- a `#REF!`/`#DIV/0!` error surfaces as the affected cell's displayed `text` inside `ViewportData` (the UI draws a corner marker when the text is an error string; there is no `DisplayCell::is_error` field), edits are optimistic (no `EditAck`), and import/export are Phase 3.)*

- [x] **Leptos UI tests (test debt, Epic `ET`).** The Trame chrome (`trame_chrome`: `formula_bar`, `find_replace`, filter/borders/color popovers, `FormatBar`, `StatusBar`, context menu) carries no test today. Baseline: `wasm-bindgen-test` (`cargo test --target wasm32-unknown-unknown --features <chrome-test>`, headless Chrome) covers the chrome end to end -- presentation logic and DOM -- and is self-sufficient (`ambre-spec-architecture.md §4`). Recommended optimization (not mandatory): extract the *pure* parts (formula-bar parsing, find/replace match computation, style-state derivation, `TrameEvent` -> view-state mapping) out of the `cfg(wasm32)` gate into host-tested functions -- faster, coverage/proptest-friendly -- leaving only the thin DOM glue in wasm. The shared `render` primitives stay host (`RecordingSurface` mock `DrawSurface`). Note: feature *behaviour* (merge/autofill/sort/filter) is already host-testable in `ambre-trame::logic` -- this story only covers the chrome layer. **DoD:** at least the reactive/presentation belt + one DOM `wasm-bindgen-test` green in CI. *(Done: `crates/ambre-trame/tests/chrome_ui.rs` -- a `cfg(target_arch = "wasm32")` `wasm-bindgen-test` suite run under headless Chrome via `wasm-pack` (`just test-wasm`). Reactive/presentation belt: `apply_style_event` (StyleApplied -> StyleState signals), `mirror_event` (UndoState -> command store), the `command_to_app` CommandId -> AppCommand mapping. DOM belt: `StatusBar` and `FindReplace` mounted into a real document and asserted via `web-sys`. Wire-up: wasm-only dev-deps; `proptest` scoped to the host target (its `wait-timeout` dep does not compile on wasm32) with `convergence_tests` gated to non-wasm; the `test-wasm` CI job re-enabled and a `wasm` tier added to `run_tests.sh`. The recommended pure-extraction optimization was not pursued -- the belt runs the existing functions directly in wasm.)*

**Phase 2 — V1**
- [ ] Row/column freeze
- [ ] Function autocompletion
- [ ] Multi-user cursors (`TrameEvent::RemoteCursors`, identified by `UserId`)
- [ ] Personal filter views (`-features` Phase 2)
- [ ] Conditional formatting (`style_override` rendering)
- [ ] Whole-column auto-fit (Logic-side `AutoFitCol`/`AutoFitRow` measuring every materialized cell -- upgrade over the MVP cached-cells auto-fit, sec.4bis.C)
- [ ] Context menu extensions (conditional formatting, data validation entries)

**Phase 3 — Post-V1**
- [ ] Sparklines
- [ ] wgpu/lyon charts (bar, line, pie)
- [ ] PDF export: UI thread receives `TrameEvent::ExportComplete` and triggers `trigger_download()` (PDF rendering in the Logic thread via `KrillaSurface`)
- [ ] Cell reference autocompletion
- [ ] Smooth scroll animations
- [ ] Touch / pinch-to-zoom mobile
