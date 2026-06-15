# AMBRE — SPEC: AMBRE-TRAME UI (Rendering & Interactions)
**Type:** Vertical (Feature / Application — UI)
**Status:** Draft

> Rendering and interaction layer of the Trame spreadsheet. Runs exclusively in the **UI thread** (main JS thread). Receives `TrameViewUpdate` (via `LogicEnvelope::ViewportUpdate`) from the Logic thread via `postMessage`, renders the grid via `WgpuSurface` (QuadRenderer + GlyphAtlas), and translates user events into `TrameCommand` sent back to the Logic thread. PDF export will be generated in the **Logic thread** via `KrillaSurface` (implementation of the shared `DrawSurface` trait) so as not to block the UI thread.
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
| `wasm-bindgen` | JS <-> Rust bridge for postMessage | Glue UI thread <-> Logic thread |
| `web-sys` | Browser APIs (Canvas, clipboard, file download) | DOM access from UI thread |
| `postcard` | IPC serialization/deserialization | `LogicEnvelope`, `TrameCommand`, `TrameViewUpdate` |

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

```rust
// ambre-app/src/lib.rs -- current IPC handler
fn handle_logic_envelope(&mut self, envelope: LogicEnvelope) {
    if let LogicEnvelope::ViewportUpdate { document_id: _, payload } = envelope
        && let Ok(update) = postcard::from_bytes::<TrameViewUpdate>(&payload)
    {
        self.trame.on_viewport_data(update);  // updates cell_cache
        self.dirty = true;
    }
}
```

**Future envelopes to handle (not implemented yet):**

```rust
// To add in handle_logic_envelope() when features are implemented
LogicEnvelope::FormulaError { row, col, error } =>
    self.trame.formula_errors.insert((row, col), error),
LogicEnvelope::SheetList { sheets } =>
    self.trame.sheets = sheets,
LogicEnvelope::UndoState { can_undo, can_redo } => {
    self.trame.can_undo = can_undo;
    self.trame.can_redo = can_redo;
}
// Filters (shared) -- MVP
LogicEnvelope::FilterApplied { hidden_row_count } =>
    self.trame.filter_active = hidden_row_count > 0,  // request_viewport already refreshed
LogicEnvelope::FilterCleared =>
    self.trame.filter_active = false,
// Sort -- MVP (viewport already refreshed by the Logic thread; just clears the busy flag)
LogicEnvelope::SortComplete => self.trame.sort_pending = false,
// Merges -- MVP
LogicEnvelope::MergeRejected { reason } =>
    self.trame.toast(ToastKind::Warn, &reason),
LogicEnvelope::MergeConflictDetected { master, covered } =>
    self.trame.mark_merge_conflict(master, covered),
LogicEnvelope::ExportComplete { filename, data } =>
    trigger_download(&filename, &data),  // via web_sys
```

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
    remote_cursors: HashMap<Uuid, RemoteCursor>,

    /// Undo/Redo state
    can_undo: bool,
    can_redo: bool,
}
```

**Visible viewport calculation:**

```rust
fn visible_range(&self) -> (u32, u32, u32, u32) {
    let col_start = self.col_at_pixel(self.scroll_x) as u32;
    let col_end   = self.col_at_pixel(self.scroll_x + self.grid_width + 1.0).min(
        self.col_offsets.len() as u32 - 1
    );
    let row_start = self.row_at_pixel(self.scroll_y) as u32;
    let row_end   = self.row_at_pixel(self.scroll_y + self.grid_height + 1.0).min(
        self.row_offsets.len() as u32 - 1
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
    let velocity = (delta_x.abs() + delta_y.abs()) / 16.0; // estimated px/ms

    // Adaptive padding: 20 rows if slow, 100 if fast
    let padding = if velocity < 1.0 { 20u32 } else if velocity > 10.0 { 100 } else { 50 };

    self.logic_tx.send(TrameCommand::ScrollViewport {
        version:        self.next_version(),
        sheet_id:       self.ui_state.active_sheet,
        row_start:      row_start.saturating_sub(padding / 2),
        row_end:        row_end + padding / 2,
        col_start:      col_start.saturating_sub(5),
        col_end:        col_end + 5,
        velocity_px_ms: velocity,
    });
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
        let layout = text_cache.get_or_build(font_cx, &cell.display_text, 11.0, (w - 6.0) as f32, align);
        let y_off = ((h - layout.height() as f64) / 2.0).max(0.0);
        let brush = Brush::Solid(fg);
        surface.push_clip(Rect::new(x, y, x + w, y + h));
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

    // Custom borders, error indicator: TODO (fill_path via lyon)
}
```

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

// TODO: multi-cell selection highlighting (not yet implemented)
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
    if frozen_cols > 0 {
        let sep_x = self.col_offset(frozen_cols as u32) + self.header_width;
        self.scene.stroke(&Stroke::new(2.0), Affine::IDENTITY,
            Color::from_rgb8(100, 100, 100), None,
            &Line::new((sep_x, 0.0), (sep_x, self.grid_height)));
    }
    if frozen_rows > 0 {
        let sep_y = self.row_offset(frozen_rows as u32) + self.header_height;
        self.scene.stroke(&Stroke::new(2.0), Affine::IDENTITY,
            Color::from_rgb8(100, 100, 100), None,
            &Line::new((0.0, sep_y), (self.grid_width, sep_y)));
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

    // Grid navigation
    if self.ui_state.editing.is_none() {
        match key {
            KeyCode::ArrowUp    => self.move_selection(0, -1),
            KeyCode::ArrowDown  => self.move_selection(0, 1),
            KeyCode::ArrowLeft  => self.move_selection(-1, 0),
            KeyCode::ArrowRight => self.move_selection(1, 0),
            KeyCode::Tab        => self.move_selection(1, 0),
            KeyCode::Enter      => self.move_selection(0, 1),
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
shared filter. The menu is a `DrawSurface` overlay (no DOM); items are positioned from
the click point and dismissed on Escape or outside click.

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

### D. Shared Filter Popover (MVP)

`FilterByColumn` opens a `DrawSurface` popover anchored under the active column header.
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

> **Personal filter views, multi-column sort, conditional formatting, validation** stay
> in Phase 2 (`-features` roadmap). Only the shared filter and single-column sort are MVP.

---

## 6. INLINE EDITOR AND FORMULA BAR

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
    self.scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None,
        &Rect::new(x, y, x + w, y + h));
    self.scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, Color::from_rgb8(26, 115, 232),
        None, &Rect::new(x, y, x + w, y + h));

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

    self.logic_tx.send(TrameCommand::EditCell {
        version:  self.next_version(),
        sheet_id: self.ui_state.active_sheet,
        row, col,
        raw: editor.content,
    });
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

    let cursor_text = &editor.content[1..editor.cursor];

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
    if token.len() < 1 { return vec![]; }

    // Find columns whose letter starts with the token
    if let Some(vp) = &self.current_viewport {
        let col_start = SheetIndex::letter_to_index(&token.to_uppercase());
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

Floating dropdown below the active cell (DrawSurface), with selected item highlighting. Navigation: up/down arrows, Tab to complete, Escape to close.

---

## 8. SELECTION

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

    // Semi-transparent blue background
    self.scene.fill(Fill::NonZero, Affine::IDENTITY,
        Color::from_rgba8(26, 115, 232, 40), None, &Rect::new(x1, y1, x2, y2));
    // Blue border
    self.scene.stroke(&Stroke::new(1.5), Affine::IDENTITY,
        Color::from_rgb8(26, 115, 232), None, &Rect::new(x1, y1, x2, y2));
}
```

---

## 9. MULTI-USER CURSORS

```rust
pub struct RemoteCursor {
    /// Canonical user identifier -- stable, per-user, not per-device.
    /// The pub_key -> UserId resolution is done in ambre-sync via trusted_keys
    /// before the Ephemeral payload reaches the Logic thread.
    pub user_id:      Uuid,
    pub color:        Color,         // color assigned locally by hashing the user_id
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
        self.scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, cursor.color, None,
            &Rect::new(x, y, x + w, y + h));

        // Name label (small colored badge at the top of the cell)
        if let Some(name) = &cursor.display_name {
            self.draw_cursor_label(name, cursor.color, x, y);
        }
    }
}

// Update from decrypted LogicEvent::RemoteEphemeral.
// The Logic thread has already resolved pub_key -> UserId via ambre-sync/trusted_keys
// before emitting TrameEvent::RemoteCursors. The UI thread never manipulates
// any cryptographic key.
fn on_remote_presence(&mut self, presence: RemotePresence) {
    // RemotePresence { user_id: Uuid, display_name, sheet_id, row, col }
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

fn color_for_user(&self, user_id: &Uuid) -> Color {
    // Deterministic: hash of UserId -> HSL hue
    // Identical to prose-ui::peer_color() algorithm for visual consistency
    // cross-app (same collaborator = same color in Prose and Trame).
    let bytes = user_id.as_bytes();
    let hue = (u64::from_le_bytes(bytes[..8].try_into().unwrap())
        .wrapping_mul(2654435761) >> 32) as f32
        / u32::MAX as f32 * 360.0;
    Color::from_hsl(hue as u16, 0.7, 0.55)
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

    self.scene.stroke(&Stroke::new(1.5), Affine::IDENTITY,
        Color::from_rgb8(26, 115, 232), None, &path);
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
    self.scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None,
        &Rect::new(x, y, x + w, y + h));

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

> **Rectangular clipping:** `DrawSurface::push_clip(Rect)` / `pop_clip()` are implemented CPU-side in `WgpuSurface` (clipping vertices outside the rect before push). For `KrillaSurface`, if krilla does not support native clipping at MVP, we emulate by **truncating text before rendering** (Parley measurement + ellipsis). Fidelity is slightly lower for overflow cells, but remains vector.

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

        let bg = if is_active { Color::WHITE } else { Color::from_rgb8(230, 230, 230) };
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, bg, None,
            &Rect::new(x, y, x + tab_width, y + tab_height));

        // Thicker top border if active
        let border_width = if is_active { 2.5 } else { 1.0 };
        let border_color = if is_active { Color::from_rgb8(26, 115, 232) } else { Color::GRAY };
        self.scene.stroke(&Stroke::new(border_width), Affine::IDENTITY, border_color, None,
            &Rect::new(x, y, x + tab_width, y + tab_height));

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

## 14. QUALITY & TESTABILITY

UI tests are run with `wasm-bindgen-test` in a headless browser (Chrome via `wasm-pack test --headless`).

```rust
// Virtual scrolling
#[wasm_bindgen_test]
fn scroll_requests_correct_viewport_range() { /* ... */ }

#[wasm_bindgen_test]
fn padding_adapts_to_scroll_velocity() { /* slow -> 20, fast -> 100 */ }

// Anti-flickering
#[wasm_bindgen_test]
fn outdated_viewport_version_ignored() { /* v=2 then v=1 -> v=1 dropped */ }

// Editor
#[wasm_bindgen_test]
fn escape_cancels_editing_no_command_sent() { }

#[wasm_bindgen_test]
fn enter_commits_and_moves_down() { }

// Autocompletion
#[wasm_bindgen_test]
fn function_completion_case_insensitive() { /* "sum" -> "SUM" */ }

// Freeze
#[wasm_bindgen_test]
fn frozen_rows_not_offset_by_scroll() { }

// Remote cursors
#[wasm_bindgen_test]
fn remote_cursor_color_deterministic_from_user_id() { }
```

---

## 15. PERFORMANCE

| Operation | Target | Note |
|---|---|---|
| Frame rendering at 60 FPS (budget) | < 16 ms | GPU-bound (vertex/fragment shaders) |
| Scroll (send ScrollViewport) | < 1 ms | UI thread non-blocking |
| Receive + render TrameViewUpdate | < 5 ms | postcard deserialization + vertex batch build |
| Autocompletion (computation + dropdown rendering) | < 2 ms | Static lookup, no round-trip |

---

## 16. SOURCE OF TRUTH MODEL BY STATE TYPE

A state can exist in multiple layers. The rule is simple: **a single source of truth, other layers are projections of it**.

| Category | Examples | Source of truth | Synchronized via SecSync |
|---|---|---|---|
| **Document mutation** | EditCell, InsertRow, MergeCells, styles | `LoroDoc` (Logic thread) | Yes |
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
- [x] Virtual scrolling + grid rendering *(delivered under `ARCH P2`; the shared `ambre_trame::render` module refactor §12 is still pending -- Epic `E3`)*
- [x] UI-only layers on top of shared rendering: scroll offset, dirty opacity, error indicators
- [x] Column/row headers (via `ambre_trame::render::draw_col_headers_on<S>` / `draw_row_headers_on<S>`, wired in `ui::renderer::draw_overlay`)
- [x] Single-cell selection *(via `ui::grid::Selection` -- `anchor == active`; click `select()` / arrow nav, active border + header tint in `draw_overlay`)*
- [x] Inline editor *(delivered under `CHROME P2` -- Trame WYSIWYG cell)*; [ ] formula bar
- [x] Sheet tabs + full lifecycle (create via "+", rename via double-click, delete/rename via tab context menu, reorder via drag) -> `CreateSheet` / `RenameSheet` / `DeleteSheet` / `MoveSheet` (sec.13) *(bottom strip in `ui::renderer::draw_sheet_tabs`; input + `SheetList` dispatch in `ui::view`; strip hydrated on the first `ScrollViewport`)*
- [x] Keyboard navigation (Arrows, Page Up, Page Down, Tab/Shift+Tab, Enter/Shift+Enter, Delete/Backspace, End, Home, Ctrl+End, Ctrl+Home) *(nav-mode dispatch in `ui::view`; absolute jumps via `TrameUiState::move_to` + cache-scan helpers `last_content_col`/`last_content_cell` for End/Ctrl+End -- UI-thread cache is the only used-range view, sec.16; Delete/Backspace clear via empty `SetCellText`; F2 opens the editor)*
- [x] `CommitEdit` on cell exit (closes the undo group -- 1 cell edit = 1 undo item, sec.6.C)
- [ ] Grid context menu (right-click, sec.5.C): row/column insert+delete (`InsertRow`/`InsertCol`/`DeleteRow`/`DeleteCol`), merge/unmerge (`MergeCells`/`UnmergeCells`)
- [ ] Cell merge rendering (`DisplayCell.is_merged` / `merge_span`) + `MergeRejected` / `MergeConflictDetected` handling (sec.2)
- [ ] Single-column shared sort (`SortRange` from the context menu, sec.5.C) + `SortComplete` dispatch
- [ ] Shared filter: filter popover (`ApplyFilter` / `ClearFilter`, sec.5.D) + funnel badge + `FilterApplied` / `FilterCleared` dispatch
- [ ] `on_logic_message` wiring: full `TrameEvent` dispatch (ViewportData, EditAck, FormulaError, SheetList, MergeRejected, MergeConflictDetected, FilterApplied, FilterCleared, SortComplete, ValidationError, UndoState, ImportProgress, ExportComplete)

**Phase 2 — V1**
- [ ] Multi-cell selection
- [ ] Row/column freeze
- [ ] Function autocompletion
- [ ] Multi-user cursors (`TrameEvent::RemoteCursors`, identified by `UserId`)
- [ ] Personal filter views + multi-column sort UI (`-features` Phase 2)
- [ ] Conditional formatting (`style_override` rendering)
- [ ] Column/row resize (drag)
- [ ] Context menu extensions (conditional formatting, data validation entries)

**Phase 3 — Post-V1**
- [ ] Sparklines
- [ ] wgpu/lyon charts (bar, line, pie)
- [ ] PDF export: UI thread receives `TrameEvent::ExportComplete` and triggers `trigger_download()` (PDF rendering in the Logic thread via `KrillaSurface`)
- [ ] Cell reference autocompletion
- [ ] Smooth scroll animations
- [ ] Touch / pinch-to-zoom mobile
