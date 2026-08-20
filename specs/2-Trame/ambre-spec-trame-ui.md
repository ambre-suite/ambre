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
| `ambre-pdf` | Shared PDF surface (`KrillaSurface`, owns `krilla`) | Foundation-owned; the Logic thread instantiates `ambre_pdf::KrillaSurface`. Trame depends on `ambre-pdf`, not on `krilla` directly. |
| `ambre-bus` | Typed data plane: `TrameCommand` / `TrameEvent` / `DisplayCell` | UI<->Logic values carried by move on `flume` (no serialization) |
| `wasm-bindgen` | JS <-> Rust bridge | Canvas / DOM glue on the UI thread |
| `web-sys` | Browser APIs (Canvas, clipboard, file download) | DOM access from UI thread |
| `postcard` | Serialization | Only the `ClipboardPayload` (OS clipboard blob); **not** the UI<->Logic hop |

**Invariants:** prefix `TRAME-UI`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4` ("Invariants (normative)"); the cross-spec roll-up is `ambre-spec-testing.md §7`. Rows `001..015` cover the built surface. Rows `016..023` cover multi-user cursors (§9), sparklines (§10), charts (§11) and PDF export (§12), which have no implementing code -- `ambre-trame` does not yet depend on the shared `ambre-pdf` crate (which owns `krilla`) -- so they carry `Status: unimplemented` and cite the Story that will build them (architecture §4).

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
| `TRAME-UI-017` | A remote cursor MUST be identified by `UserId`, never by `pub_key` or connection: one collaborator on two devices MUST NOT render as two people. The Logic presence table deduplicates by `UserId` before the whole set reaches the UI. | §9 | not built -- lands with `SYNC P4` / `SRV P1` (Epic `E5`) | unimplemented |
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

**Canvas-side routing** (`ambre-trame/src/ui/view/frame.rs::dispatch_trame`), one arm per variant:

| `TrameEvent` | Canvas effect |
|---|---|
| `ViewportData` | cache the payload, mark dirty |
| `DataReset`, `CondFormatsUpdated` | re-request the viewport |
| `SheetList` | refresh the tab strip |
| `SelectRange` | set the selection, park input, announce |
| `FilterApplied` / `FilterCleared` | set / clear the funnel-badge column |
| `FindResult` | set the highlight set and navigate |
| `Error` | log and announce |
| `ExportComplete` | trigger the browser download |
| `Clipboard` | build the transport blob, mark it pending for the host |
| every chrome-only variant (`StyleApplied`, `UndoState`, `MergeRejected`, `MergeConflictDetected`, `SelectionStats`, `ColumnValues`, `ReplaceDone`, `PasteRejected`, `SortComplete`, validation/protection, `ImportComplete`) | explicit no-op on the canvas -- the chrome relay owns them |

The match names every variant and carries **no `_` arm**, so a new variant is a compile error rather than a silent drop (`TRAME-UI-006`).


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

    /// Remote cursors -- the whole live peer set, replaced wholesale from each
    /// `TrameEvent::RemoteCursors` (like Prose's `Vec<RemoteCursor>`), so a departed
    /// peer drops with no separate removal path. Each `user_id` was resolved by
    /// `ambre-sync` before reaching here.
    remote_cursors: Vec<RemoteCursor>,

    /// Undo/Redo state
    can_undo: bool,
    can_redo: bool,
}
```

**Visible viewport calculation** (`ui/grid/state.rs`). `col_offsets` / `row_offsets` are cumulative pixel offsets, so a pixel maps to an index by binary search (`partition_point`, saturating at 0). The visible range is the pair of indices at the scroll origin and at the far edge, each clamped to the last offset -- the clamp guards the empty-offsets case, where a bare `len - 1` would underflow.

**Requesting a window from the Logic thread** (`ui/grid/state.rs`):

1. Clamp the new scroll origin at 0.
2. Take the visible index range above.
3. Derive an **adaptive padding** from the gesture velocity -- roughly 20 rows when slow, 50 by default, 100 when fast. The velocity is a **local UI heuristic**: `ScrollViewport` carries no `velocity_px_ms` and no `sheet_id`, and the Logic thread resolves the active sheet itself.
4. Emit `TrameCommand::ScrollViewport` with a fresh monotonic `version` and the padded window (half the padding on each side vertically, five columns each side horizontally).

The `version` is what makes a late response droppable (`TRAME-UI-005`).

### B. Updating `col_offsets` / `row_offsets`

Each `DisplayViewport` carries the widths and heights of the range it covers. For every index in that range, compare the incoming size with the cached one; on the first difference, rebuild the cumulative offsets **from that index onward** and stop comparing. Indices outside the viewport keep the default size (80 x 20 px). The rebuild is suffix-only, so a resize far down the sheet does not re-walk the head.

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
> are honoured above. `wrap` rows -- and rows whose text carries explicit newlines
> (`Alt+Enter` in the in-cell editor, §6) -- may need a taller `row_height`: both make a
> cell taller than one line. Auto-height is a Logic-thread concern -- it returns the measured
> height in `DisplayViewport.row_heights` and the UI does not remeasure. Until auto-height is
> implemented, a multi-line cell simply lays out its lines and is clipped to the current row
> height (like any vertical overflow); the user can raise the row height manually to reveal
> the rest.
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

Frozen cells are rendered in a separate pass, on top of the scrolled content, without the scroll offset:

1. Nothing to do when neither axis is frozen.
2. Walk the current viewport's cells and keep those whose row index is below `frozen_rows` **or** whose column index is below `frozen_cols` (the Logic thread already includes them in the viewport).
3. Position each one on its frozen axis at its absolute offset -- the scroll is subtracted only on the axis that is *not* frozen -- then draw it with the ordinary cell painter.
4. Stroke the separation line at the frozen boundary on each frozen axis.

**Freeze command from the UI.** Setting `frozen_rows` / `frozen_cols` is a write to local view state and a re-render. It produces no `TrameCommand` and no CRDT delta.

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

* **Column border** -> `TrameCommand::ResizeCol { version, sheet_id, col, width_px }`, the width taken from the pointer position minus the column's left edge plus the scroll, floored at `MIN_COL_WIDTH` (~24 px). **Row border** -> `ResizeRow`, symmetric on the vertical axis.
* Any other header zone releases without emitting.
* Logic writes `col_widths_{sheet}` / `row_heights_{sheet}` (LWW per uid) and answers with a refreshed viewport carrying the new sizes; the UI rebuilds its offsets (§3.B).
* When a whole column or row is selected, the resize applies to every selected band.

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

Handled in `ui/view/interact.rs`. Two modes, decided by whether the inline editor is open.

**Always, before anything else:** `Ctrl/Cmd+Z` -> `Undo`, `Ctrl/Cmd+Shift+Z` -> `Redo`.

**Grid mode** (no editor):

| Key | Effect |
|---|---|
| Arrows | move the selection by one cell; with `Shift`, **extend** it (move `active`, keep `anchor`, §8) |
| `Tab` / `Enter` | move right / down, never extending |
| `Home` / `End` | first cell of the row / last non-empty cell of the row |
| `PageUp` / `PageDown` | scroll by one viewport height |
| `Delete` / `Backspace` | `SetCellText` with an empty string -- there is **no** dedicated `ClearCell` command, an empty commit is the clear path |
| `F2` | open the editor on the active cell |
| any printable character | open the editor seeded with that character |

**Editor mode:** the editor is multi-line (the shared `ambre_canvas::TextEditor`
core over parley `PlainEditor`, §6). `Escape` cancels without emitting a command
(`TRAME-UI-015`); `Enter` commits and moves down, `Tab` commits and moves right,
`Ctrl/Cmd+Enter` fills the whole selection with the typed value. `Alt+Enter`
inserts a newline in the cell (the only way to add a line, since bare `Enter`
commits). Arrow `Up`/`Down` move the caret between lines (with `Shift`, extend the
selection) -- inside the editor they never navigate cells; cell navigation by
arrows applies in grid mode only. `Left`/`Right`/`Home`/`End`, `Backspace`/
`Delete`, and printable characters go to the editor. Because every edit is queued
as a font-free intent and applied at the paint flush (a live `FontContext` exists
only then), the four committing chords defer the commit to just after that flush.

### B. Mouse / Touch

**Pointer down** resolves the pixel to a cell (§4bis hit-test). A left press on the cell that is *already* active, with no editor open, starts editing; any other left press collapses the selection to that single cell, scrolls it into view and re-requests the viewport. A press outside the grid falls through to the header zones (§4bis).

**Pointer drag** extends the selection to the cell under the pointer and re-requests the viewport. The anchor never moves during a drag.

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
```

Each entry maps to one command, built from the current selection and the active sheet, then closes the menu:

| Entry | Command |
|---|---|
| Insert row above / below | `InsertRow { before_row: row }` / `{ before_row: row + 1 }` |
| Insert column left / right | `InsertCol { before_col: col }` / `{ before_col: col + 1 }` |
| Delete rows / columns | one `DeleteRow` / `DeleteCol` **per index in the selection span, in descending order** so the remaining positions stay valid -- each is its own undo item |
| Merge cells | `MergeCells` over the selection bounds; a covered non-empty cell answers `MergeRejected`, surfaced as a toast (§2) |
| Unmerge cells | `UnmergeCells` at the active cell |
| Sort ascending / descending | `SortRange` over the selection bounds, reference column = the range's first column, `has_header` false by default (toggled in the menu) |
| Filter by column | opens the filter popover (§5.D), no command yet |
| Clear filter | `ClearFilter { sheet_id }` |

Every command carries a fresh monotonic `version`.


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

`SortKey { col: u32 /* index within range */, ascending: bool }`, decreasing priority; the
Logic thread sorts lexicographically on `sort_keys` and replies `SortComplete` (which clears
`sort_pending`, sec.2). `ambre-trame/src/ui/view/interact.rs`.

```rust
fn on_sort_dialog_apply(&mut self, keys: Vec<SortKey>, has_header: bool) { /* ... */ }
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

Sends one `ApplyFilter { sheet_id, col, kind: FilterKind::Values | Condition, values,
condition }` and closes the popover. `ambre-trame/src/ui/view/interact.rs`.

```rust
fn apply_filter_from_popover(&mut self, sheet_id: SheetId, col: u32, p: &FilterPopover) { /* ... */ }
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

`on_copy` arms the marching-ants anchor + `cut_pending`, sends the parameterless `CopyRange`,
and returns `CopyOutcome::Deferred` (the host `prevent_default`s and writes nothing yet). The
reply `TrameEvent::Clipboard(payload)` is built from the full document; the view stashes a
`ClipboardBlob` (TSV + postcard) in memory for a same-tab paste and marks it pending, and the
host mirrors it to the OS clipboard the next frame (`poll_clipboard`). `on_paste` prefers the
in-memory blob when the OS `text/plain` mirror still matches (full fidelity), else decodes
`blob.ambre` into `PasteRange` at the active cell (Logic reindexes relative formulas); a cut
then clears the source range. `ambre-trame/src/ui/view/interact.rs`, `frame.rs`.

```rust
fn on_copy(&mut self, cut: bool) -> CopyOutcome { /* ... */ }
fn on_paste(&mut self, blob: ClipboardBlob) { /* ... */ }
```

Keyboard: Ctrl+C / Ctrl+X / Ctrl+V ride the native clipboard events (they are not decoded in
`on_key_down`). Context menu: add `Copy`, `Cut`, `Paste` to `ContextMenuItem` (sec.5.C).
Render: animated marching-ants (dashed `stroke_rect`) around `clipboard_anchor` until Escape or
the next copy. A `PasteRejected` event (protected range) surfaces a toast.

### F. Autofill (Fill Handle) -- MVP

A small blue square is drawn at the bottom-right of the selection. Dragging it extends the
selection to a destination range and emits `FillRange`; the Logic thread chooses `Series`
(detected numeric/date pattern) or `Copy`.

Sends `FillRange { src, dest = src + dragged extent, mode: FillMode::Auto }` -- the Logic
thread picks `Copy` vs `Series`. `ambre-trame/src/ui/view/interact.rs`.

```rust
fn on_fill_handle_drag_end(&mut self, dest: (u32, u32)) { /* ... */ }
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

// Find / Find Next resolves the search origin from the tracked active cell (Logic keeps it via
// SetActiveCell, so the panel carries no cursor), sends FindNext { query, match_case,
// whole_cell }, and the reply FindResult { matches, active } drives the highlight + scroll to
// `active`. Replace All sends ReplaceAll { query, replacement, match_case, whole_cell,
// in_formulas, all_sheets } and toasts ReplaceDone { replaced } via chrome::notify_event.
// ambre-trame/src/ui/view/interact.rs.
fn on_find_next(&mut self, p: &FindReplaceState) { /* ... */ }
fn on_replace_all(&mut self, p: &FindReplaceState) { /* ... */ }
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

fn on_toolbar_action(&mut self, action: ToolbarAction) { /* ... */ }
```

Each action maps to one `StylePatch` field (`Bold` toggles the active cell's `bold`, `AlignV`
sets `v_align`, `NumberFmt` sets `fmt`, `Wrap` sets `wrap`, ...) and sends `SetCellStyle {
range, patch }` over the selection; `Merge` reuses `merge_selection` (sec.5.C). The reply
`StyleApplied { version }` refreshes the viewport (Logic reformats `display_text`).
`ambre-trame/src/ui/view/interact.rs`.

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

The editor field is drawn by the shared `ambre_canvas::TextEditor` core (the same
unit-text engine Toile uses for sticky/text elements, §canvas). Trame supplies the
cell geometry and pins the cell's formatting (weight, italic, size, ink) each
frame; the core lays out the buffer, resolves queued pointer hits against that
layout, and paints the selection band, glyphs (including the live IME preedit) and
the blinking caret.

The layout **wraps to the cell content width** and honours explicit newlines
(`Alt+Enter`), so a cell edits multi-line. The field **grows downward** to fit
every line, overflowing the cell over the rows below (Excel/Sheets behaviour), and
is never shorter than the row: a layout that fits one row stays vertically centred
(matching the static cell), a taller one top-aligns from the cell top. The field
is clipped to its own -- possibly taller -- rectangle intersected with the live
clip; downward growth only ever covers cells below, never the header strip. The
following sketch predates the shared core and is illustrative only.

`draw_inline_editor` paints the active cell's white background + blue focus border under the
shared `TextEditor` core (the core draws the buffer, selection and caret). `draw_formula_bar`
shows the active cell reference (`A1`) and its raw value -- or the live editor content while
editing. Both are illustrative here; the shipped bar is DOM chrome (see the sec.6 note).
`ambre-trame/src/ui/renderer/`.

```rust
fn draw_inline_editor(&mut self) { /* ... */ }
fn draw_formula_bar(&mut self) { /* ... */ }
```

### C. Commit and Cancel

`commit_editing` takes the open editor, sends `SetCellText { row, col, text }` (an empty `text`
clears the cell -- no `ClearCell`) then `CommitEdit` to close the undo group (one edit = one
undo item, `spec-ambre-trame-logic.md sec.8`), and moves the selection per `direction`.
`cancel_editing` just drops the editor -- no `TrameCommand`, no Loro mutation.
`ambre-trame/src/ui/view/interact.rs`.

```rust
fn commit_editing(&mut self, direction: CommitDirection) { /* ... */ }
fn cancel_editing(&mut self) { /* ... */ }
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

fn compute_completions(&self, editor: &InlineEditor) -> Vec<Completion> { /* ... */ }
```

Only fires on a formula (`content` starts with `=`); it isolates the token under the cursor
(the last run of alphanumerics/`_`, guarding the `1..cursor` slice against a zero or
mid-codepoint index) and returns up to 8 `FUNCTION_COMPLETIONS` whose name starts with the
uppercased token. `ambre-trame/src/ui/` (autocompletion).

### B. Cell References

On a pure `A..Z` token (guarded so `letter_to_index`'s `b - b'A'` never underflows), suggests
the first 5 non-empty cells of the matching column from the current viewport, as `CellRef {
label, preview }`. `ambre-trame/src/ui/` (autocompletion).

```rust
fn compute_cell_ref_completions(&self, token: &str) -> Vec<Completion> { /* ... */ }
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
    pub fn single(row: u32, col: u32) -> Self { /* anchor == active */ }
    pub fn extend_to(&mut self, row: u32, col: u32) { self.active = (row, col); }
    pub fn active(&self) -> (u32, u32) { self.active }
    pub fn contains_col(&self, col: u32) -> bool { /* col in normalized [anchor.1, active.1] */ }
    pub fn contains_row(&self, row: u32) -> bool { /* row in normalized [anchor.0, active.0] */ }
}

// draw_selection paints the normalized anchor..active rectangle: a semi-transparent blue fill
// plus a blue border, in viewport pixels. ambre-trame/src/ui/renderer/overlay.rs.
fn draw_selection(&mut self) { /* ... */ }
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

fn draw_remote_cursors(&mut self) { /* ... */ }
fn on_remote_cursors(&mut self, cursors: Vec<RemoteCursorPayload>) { /* ... */ }
fn color_for_user(&self, user_id: &UserId) -> Color { /* ... */ }
```

`draw_remote_cursors` paints each peer's active-cell border + name badge in the peer colour.
`on_remote_cursors` receives the WHOLE live peer set (`Vec<RemoteCursorPayload>`, already
resolved to `(row, col)` and coloured Logic-side) and **replaces** `remote_cursors` wholesale --
a peer absent from the new set (left, or its anchor no longer resolves) drops on its own, so
there is no insert-only accumulation and no ghost cursor. `color_for_user` delegates to the
shared `ambre_draw::peer::peer_color` (same algorithm in Prose, Trame and Toile, seeded on the
canonical `UserId`), so a collaborator keeps one colour everywhere. The event is assembled
Logic-side from the Sync-resolved `from_user_id`; the peer `PresencePayload` never carries a
`user_id`, and the UI thread touches no cryptographic key. `ambre-trame/src/ui/`.

---

## 10. SPARKLINES

Sparklines are mini-charts rendered inside a cell. They are computed and rendered entirely on the UI thread side from `DisplayViewport` data.

```rust
// A sparkline is described in DisplayCell via style/raw fields; cell syntax
// "=SPARKLINE(A1:A10)" (Phase 2). For MVP the type is detected from a style field.
fn draw_sparkline(&mut self, cell: &DisplayCell, x: f32, y: f32, w: f32, h: f32) { /* ... */ }
```

Maps the cell's numeric `values` to a min/max-normalized polyline inside the `(x, y, w, h)` box
and strokes it segment by segment (`DrawSurface` has no `stroke_path`). Empty values draw
nothing. `ambre-trame/src/ui/renderer/`.

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

fn draw_chart(&mut self, obj: &DisplayFloatObject, chart: &ChartDef) { /* ... */ }
```

Extracts the chart data for `data_range` (from `current_viewport`, else requested from Logic),
paints the float object's white background, then dispatches on `chart.kind` to the per-kind
renderer (bar/line/pie/area) and draws the optional title. `ambre-trame/src/ui/renderer/`.

---

## 12. PDF EXPORT VIA `krilla`

### Principle: shared `DrawSurface` trait, PDF rendering in the Logic thread

The `DrawSurface` trait is defined in `ambre-draw` (see `spec-ambre-architecture.md` §2.D).
The shared drawing functions (`draw_cells_on`, `draw_grid_lines_on`, etc.) live in
`ambre-trame/src/render.rs` — they are parameterized over `DrawSurface` and documented in
`spec-ambre-trame-logic.md` §9 (shared rendering module).

- **UI thread**: instantiates `WgpuSurface` (defined in `ambre-app`) → live GPU rendering via the shared functions.
- **Logic thread**: instantiates `ambre_pdf::KrillaSurface` (Foundation-owned, in the shared `ambre-pdf` crate; lazy-loaded) → vector PDF export. The Logic thread emits `TrameEvent::ExportComplete`; the UI thread triggers the download.

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

`KrillaSurface` is Foundation-owned: its `DrawSurface` implementation lives in the
shared **`ambre-pdf`** crate (deps `ambre-draw` + `krilla`), not in an app crate,
because every app (Prose, Toile, Trame) reuses the same surface. Foundations
mandates this placement (`1-Foundations/ambre-spec-canvas.md`, `ambre-pdf`
sections). Trame does not redefine it; it uses `ambre_pdf::KrillaSurface`.

The Trame-specific PDF driver -- pagination (`GridLayout` for PDF), the `ExportPdf`
command handling, and the calls into the shared drawing functions -- lives in
`ambre-trame/src/export_pdf.rs` (Logic thread, lazy-loaded). See
`spec-ambre-trame-logic.md` §9 for the code.

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

> **Delivered as DOM chrome.** The tab strip is a `trame_chrome` component (sec.18); the
> `DrawSurface` sketch below predates that move and is illustrative.

`draw_sheet_tabs` lays the tabs left to right along the bottom edge, each sized to its label,
the active one white with a thicker blue top border, then draws the trailing "+" button.
`on_sheet_tab_click` sends `ActivateSheet`, resets the scroll, and re-requests the viewport.
`ambre-trame/src/ui/renderer/`, `chrome/`.

```rust
fn draw_sheet_tabs(&mut self) { /* ... */ }
fn on_sheet_tab_click(&mut self, sheet_id: SheetId) { /* ... */ }
```

### Full Sheet Lifecycle (MVP)

The tab strip is the entry point for the four lifecycle commands the Logic thread
already implements (`-logic` sec.2.D). The `SheetList` event refreshes the strip after
each one.

Each handler is a thin intent send, the Logic thread owning every rule:
- `on_add_sheet_button` -> `CreateSheet { name }` (Logic mints a peer-scoped `sheet_id`, replies `SheetList`).
- `on_sheet_tab_rename_commit` -> `RenameSheet { sheet_id, name }` on a non-blank name.
- `on_sheet_tab_delete` -> `DeleteSheet { sheet_id }`; Logic rejects the last sheet (`Error` -> toast) and decides the active-sheet fallback.
- `on_sheet_tab_drag_drop` -> `MoveSheet { sheet_id, to_index }` (drop slot).

`ambre-trame/src/ui/view/interact.rs`, `chrome/`.

```rust
fn on_add_sheet_button(&mut self) { /* ... */ }
fn on_sheet_tab_rename_commit(&mut self, sheet_id: SheetId, name: String) { /* ... */ }
fn on_sheet_tab_delete(&mut self, sheet_id: SheetId) { /* ... */ }
fn on_sheet_tab_drag_drop(&mut self, sheet_id: SheetId, to_index: u32) { /* ... */ }
```

---

## 13bis. STATUS BAR (SELECTION AGGREGATES)

A thin band above the sheet-tab strip (sec.13) shows aggregates of the current selection,
like Excel/Sheets (Sum / Average / Count). The selection can exceed the viewport, so the
**Logic thread computes** the aggregates -- the UI cannot, holding only the visible cells
(sec.16). On each non-trivial selection change the UI requests them; for a single cell it
shows nothing.

> **Delivered as DOM Leptos chrome**: Trame's parameterization of the shared, app-agnostic
> `ambre-chrome` status bar (`ambre-spec-chrome.md sec.2.B` -- one status bar provided by
> `ambre-chrome`, parameterized per app), the same way `trame_chrome::FormatBar` is Trame's
> parameterization of the shared format bar. `trame_chrome::StatusBar` fills the app slots:
> the selection aggregates below and the perf HUD (fps + frame timings), the latter migrated
> out of the wgpu overlay as a permanent `<fps>` chip that expands a diagnostics panel on
> click. The `DrawSurface` sketch below predates the chrome-in-DOM decision. The request is
> parameterless (`RequestSelectionStats`): the Logic thread resolves the tracked selection
> itself.

On a multi-cell selection `on_selection_changed` sends the parameterless
`RequestSelectionStats` (Logic resolves the tracked selection itself, no `sheet_id`/`range`)
and caches the reply `SelectionStats { sum, avg, count, count_num, min, max }`; a single cell
clears the band. `draw_status_bar` renders the cached stats right-aligned (`count_num` drives
Sum/Avg/Min/Max visibility). `ambre-trame/src/ui/`, `chrome/status.rs`.

```rust
fn on_selection_changed(&mut self) { /* ... */ }
fn draw_status_bar(&mut self) { /* ... */ }
```

---

## SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

### Boundary Audit

The UI thread runs no encryption and opens no socket (see §1: encryption and
network live in the Sync thread). Its outbound flows:

| Flow | Data | Encryption |
|------|------|------------|
| UI -> OS clipboard | `ClipboardPayload` as base64 postcard under the app-agnostic `application/x-ambre` MIME (`ambre-spec-ui.md sec.2.E` -- one private MIME for every app, each serializing its own payload behind the bytes), plus a `text/plain` fallback | Local OS boundary, user-initiated (copy/cut); never sent to `ambre-server` |
| UI -> browser download (`ExportComplete`) | Export bytes received from the Logic thread | Local file write, user-initiated; no server involvement |
| UI <-> Logic (`TrameCommand` / `TrameEvent`) | Rust values over `flume`, shared WASM memory | No serialization, no process boundary |
| Remote presence (inbound) | `TrameEvent::RemoteCursors` (whole peer set, `Vec<RemoteCursorPayload>`) already decrypted by the Logic thread; each `user_id` resolved in `ambre-sync` | The UI thread never touches ciphertext; ephemeral payloads are sealed in `ambre-sync` |

The UI thread holds no plaintext path to the server. `web-sys` clipboard and
download are local browser APIs. Presence reaches the UI already decrypted, and
the peer payload never carries a `user_id`.

### Cryptography

This audit identifies the flows only. The seal on CRDT and ephemeral payloads is
specified by the Sync and crypto specs in `1-Foundations/`; it MUST NOT be
re-derived here.

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
- [x] Clipboard (sec.5.E): copy/cut authoritative (the Logic thread builds the whole tracked selection from the document, no viewport limit), paste via `PasteRange` (Logic reindexes formulas+styles), marching-ants, Ctrl+C/X/V + context-menu entries + Ctrl+Inser/Shift+Inser/Shift+Del; `PasteRejected` toast *(Copy/cut emit the parameterless `TrameCommand::CopyRange`; the Logic thread serialises the whole tracked selection from the document -- never viewport-culled (sec.5.E), so `Ctrl+A`-then-copy is always complete -- into a typed `ClipboardPayload` (`ambre-bus`, serde) returned as `TrameEvent::Clipboard` and written to the OS clipboard as base64 postcard under the app-agnostic `application/x-ambre` MIME plus a tab-separated `text/plain` fallback; the host's `copy`/`cut`/`paste` DOM listeners service the Ctrl+C/X/V (and Ctrl+Insert/Shift+Insert/Shift+Del) chords. Paste sends `TrameCommand::PasteRange`; the Logic thread reindexes every relative reference by the paste offset (`formula::shift`, parse -> shift -> render; off-grid -> `#REF!`; AMBRE has no `$` absolute syntax) and writes the carried `CellStyle`/bold/italic verbatim, as one undo group. An external TSV paste lands as literals (`src == target`, no shift). Cut clears the source after paste; the source range crawls with animated marching-ants (`renderer::dashed_h`/`dashed_v`) until Escape or the next copy. Toolbar Copy/Cut/Paste buttons (`MdiContentCopy`/`MdiContentCut`/`MdiContentPaste`) and context-menu entries drive the browser clipboard via the shared command path (`COPY`/`CUT`/`PASTE` UI-only intents). Number format/decimals are not carried by the current payload and `PasteRejected` is wired through `chrome::notify_event` but only fires once range protection lands (Post-MVP).)*
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
