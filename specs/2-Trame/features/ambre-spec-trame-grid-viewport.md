# AMBRE -- FEATURE: THE GRID VIEWPORT
**Type:** Vertical (feature)

> A user scrolls a sheet of 100 000 rows. Only the cells on screen, plus about a screen above and
> below, ever leave the Logic thread. A cell the user has not been sent reads as pending or as
> blank, never as a stale value.

The viewport is local view state: a scroll writes nothing to the document and reaches no
collaborator. The frozen pane is the one exception: the document stores it, so every collaborator
shows the same frozen rows and columns.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-007` | `INV` | Hit-testing MUST be the exact inverse of the geometry it walks, under zoom and under variable row and column sizes. | this spec section 2 | `ambre_trame::ui::grid::tests::group1::zoom_scales_metrics_and_round_trips_hit_test`; `::variable_metrics_round_trip_pixel_and_hit_test`; `::axis_pointer_resolution_follows_variable_metrics`; `::filter_button_hit_test_matches_drawn_rect`; `ambre_trame::render::tests::variable_metrics_shift_cells_and_hit_test` -- each holds one modifier, the variable-metric round trips running at 100% and the zoom round trip on default metrics | none | verified |
| `TRAME-UI-143` | `INV` | The geometry the renderer draws MUST agree with the geometry hit-testing walks. | this spec section 2 | `ambre_trame::ui::renderer::tests::the_drawn_cell_is_the_hit_tested_cell` -- under a zoom, resized axes, hidden rows and a scroll | none | verified |
| `TRAME-UI-024` | `INV` | The cache MUST hold no cell outside the last requested window. | this spec section 2 | `ambre_trame::ui::grid::tests::viewport::a_scroll_evicts_every_cell_outside_the_window` | none | verified |
| `TRAME-UI-012` | `OBS` | Every fill colour drawn by the renderer MUST come from the theme palette; no literal colour in render code. | this spec section 3; `ambre-spec-ui.md` (design tokens) | `ambre_trame::ui::renderer::tests::renderer_uses_only_palette_colors_for_fills` -- its fixture holds one unstyled cell, so a cell the user coloured is outside what it measures (`features/ambre-spec-trame-cell-styles.md`) | none | verified |
| `TRAME-UI-013` | `OBS` | The renderer MUST NOT paint outside the grid region, whatever the canvas size or the scroll position. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_strip_skips_when_outside_grid`; `ambre_trame::ui::grid::tests::group2::grid_bottom_floors_at_header_on_tiny_canvas`; `::header_strip_starts_at_top_of_canvas` | none | verified |
| `TRAME-UI-027` | `OBS` | A cell the user has just edited MUST read as unconfirmed until its computed value arrives. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::dirty_and_error_lifecycle`; `ambre_trame::ui::renderer::tests::dirty_cell_gets_a_translucent_veil` | none | verified |
| `TRAME-UI-028` | `OBS` | A band entering the viewport by a scroll MUST NOT be painted as pending. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_scroll_strip_base_is_blank_not_pending_grey`; `::draw_cells_full_repaint_paints_pending_then_grid` | none | verified |
| `TRAME-UI-191` | `CONC` | Two peers that freeze rows and columns at once MUST both end on the pane that holds both bands. | this spec section 2 | `ambre_trame::logic::frozen::tests::rows_and_columns_frozen_at_once_both_hold` | none | verified |
| `TRAME-UI-192` | `OBS` | A frozen row or column MUST stay drawn and hit-tested at its place while the rest of its axis scrolls. | this spec section 3 | `ambre_trame::ui::renderer::tests::a_frozen_band_stays_drawn_and_hit_under_a_scroll`; `::a_frozen_pane_draws_four_clipped_panes`; `ambre_trame::ui::grid::tests::viewport::a_scroll_keeps_the_frozen_bands_in_the_cache`; `::a_change_in_a_frozen_band_re_fetches_the_window`; `ambre_trame::logic::frozen::tests::an_answer_far_from_the_top_carries_the_frozen_bands` | none | verified |
| `TRAME-UI-193` | `INV` | The scroll origin MUST NOT enter a frozen band. | this spec section 2 | `ambre_trame::ui::grid::tests::viewport::the_scroll_origin_never_enters_a_frozen_band` | none | verified |
| `TRAME-UI-049` | `OBS` | The painted grid MUST be offset by the sub-cell scroll remainder. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 The user scrolls

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-004` | `POST` | A scroll MUST request no more than the visible range plus a bounded padding. | this spec section 2 | `ambre_trame::ui::grid::tests::viewport::a_scroll_requests_the_screen_and_a_bounded_padding` | none | verified |
| `TRAME-UI-144` | `POST` | A scroll MUST request every cell the user can see. | this spec section 2 | `ambre_trame::ui::grid::tests::group2::scroll_requests_every_narrow_column_on_screen` | none | verified |
| `TRAME-UI-025` | `POST` | At most one viewport request MUST leave the UI thread per frame. | this spec section 2 | `ambre_trame::ui::view::tests::the_viewport_queue_coalesces_to_one_request` -- every request source collapses to one drain; `::a_dropped_request_is_re_offered_not_doubled` -- a dropped request is re-offered, never doubled | none | verified |
| `TRAME-UI-145` | `FRAME` | A scroll MUST NOT change which cell is active or which range is selected. | this spec section 2 | `ambre_trame::ui::grid::tests::viewport::a_scroll_keeps_the_active_cell_and_the_selection` | none | verified |
| `TRAME-UI-175` | `POST` | A selection or a fill handle held past an edge of the cells MUST scroll the grid towards it, and the range MUST reach the nearest edge cell. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_selection_held_below_the_cells_scrolls_and_follows`; `ambre_trame::ui::grid::tests::kinetic::a_drag_held_at_an_edge_scrolls_and_keeps_its_edge_cell` | none | verified |

### 1.3 The wheel turns

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-147` | `POST` | A wheel notch, a line delta or a page delta MUST be applied once and in full, keeping the sub-cell remainder. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_wheel_delta_lands_once_and_keeps_its_remainder` | none | verified |
| `TRAME-UI-026` | `FRAME` | A single wheel notch MUST NOT start a momentum coast. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_wheel_notch_starts_no_coast` | none | verified |
| `TRAME-UI-146` | `POST` | The wheel held with `Ctrl` MUST zoom the grid about the cell under the pointer. | this spec section 3; `1-Foundations/ambre-spec-zoom.md`, `ZOOM-003` | `ambre_trame::ui::view::view_tests::a_ctrl_wheel_zooms_about_the_cell_under_the_pointer` -- the row stays under the pointer, and the column unless the grid is at its first column, and the selection stays | none | verified |

### 1.4 An answer arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-005` | `POST` | An answer whose version is lower than the last accepted one MUST be dropped. | this spec section 2 | `ambre_trame::ui::grid::tests::group2::a_stale_viewport_answer_is_dropped_by_its_version` | none | verified |
| `TRAME-UI-148` | `POST` | A row or column an answer reports at the default size MUST read as the default, not as the size it held before. | this spec section 2; `ambre-spec-trame-schema.md section 4` ("absence is a value") | `ambre_trame::ui::grid::tests::viewport::a_default_size_in_an_answer_clears_the_stored_one` | none | verified |
| `TRAME-UI-149` | `FRAME` | An answer MUST NOT move the scroll origin. | this spec section 2 | `ambre_trame::ui::grid::tests::viewport::an_answer_does_not_move_the_scroll_origin` | none | verified |

### 1.5 Freeze a band of rows or columns

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-029` | `POST` | Freezing rows or columns MUST store the frozen pane in the document, so a collaborator who receives the change shows the same frozen lines. | this spec section 2; `ambre-spec-trame-schema.md section 3` | `ambre_trame::logic::frozen::tests::a_freeze_reaches_a_peer_and_one_undo_takes_it_back` -- the delta carries the pane to a second peer, its viewport answer carries it, and one undo takes it back on both | none | verified |
| `TRAME-UI-194` | `FRAME` | Freezing rows or columns MUST NOT change a cell, a line, a size or the frozen pane of another sheet. | this spec section 2 | `ambre_trame::logic::frozen::tests::a_freeze_writes_the_frozen_pane_alone` -- the whole document but the pane map of the active sheet reads the same either side of a freeze | none | verified |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`. The scroll round trip is in < 2 ms
  and the scroll viewport response in < 15 ms. The `ScrollViewport` send is in < 1 ms and the
  render of a `TrameEvent::ViewportData` in < 5 ms. Only the first two have a bench.
* **Boundary.** none: the viewport is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses is a request and a display slice, never
  document data (`TRAME-UI-001`).
* **Verification.** 23 `verified`, 1 `unimplemented`. What is drawn or hit-tested holds, and so does
  the one-slot queue that caps a frame at one request (`TRAME-UI-025`). The state machines hold on
  the host tier too. They are the window, the version guard, the eviction, the wheel and the metric
  adoption, and the wheel is driven through the view.

---

## 2. MODEL

**Containers.** One: `frozen_{sheet}`. It holds one count per axis, each under its own key. Two
peers who freeze two axes therefore keep both (`TRAME-UI-191`). An absent key
freezes nothing. A stored count leaves at least one line of its axis to scroll.

The viewport reads the sizes of its rows and columns out of each answer. `col_widths_{sheet}` and
`row_heights_{sheet}` belong to the resize feature. `ambre-spec-trame-schema.md section 3` declares
all three.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end }` | UI to Logic | the padded window, in absolute indices. No sheet id: the Logic thread resolves the active sheet |
| `TrameEvent::ViewportData { version, row_start, col_start, cells, col_widths, row_heights, frozen }` | Logic to UI | one whole window of display cells, and the sizes of its rows and columns. `frozen` carries the pane and the sizes of the frozen lines before the window, and `cells` the frozen bands of the window |
| `TrameCommand::Freeze(FreezeIntent)` | UI to Logic | freeze the rows through the active cell's row, the columns through its column, or nothing. One undo item |

**View state.** Fields of `TrameUiState`:

```rust
top_row: u32,   // first visible row: the axis scrolls by index, not by pixel
left_col: u32,
cell_cache: HashMap<(u32, u32), CellData>,  // a miss draws as pending or blank
dirty_cells: HashSet<(u32, u32)>,           // edited locally, not yet confirmed
accepted_version: Seq<ViewportVersion>, // the last answer taken (`TRAME-UI-005`)
next_version: Seq<ViewportVersion>,
col_widths: HashMap<Pos<Col>, Px<Doc>>,            // only a size that differs from the default
row_heights: HashMap<Pos<Row>, Px<Doc>>,
frozen: FrozenPane,                                // adopted from each answer
zoom: f64,
```

**Steps, on every scroll:**

1. move the origin by whole rows and columns, clamped to the grid bounds;
2. pad the visible range by one screen of rows each side, floored at 20, plus the count of rows
   hidden by the active filter there. Five columns pad each side;
3. drop every cached cell outside that window (`TRAME-UI-024`), then rebuild the hidden-row set,
   because an evicted cell carried the flag it was derived from;
4. stamp a fresh monotonic version and hold the command in the one-slot queue (`TRAME-UI-025`);
5. the frame flush sends it, dropping it if the queue is full -- the next scroll supersedes it.

**Steps, on every answer:**

1. drop it when its version is lower than the last accepted one (`TRAME-UI-005`);
2. adopt the sizes it carries, storing only those that differ from the default. A row reset to the
   default then falls back to the uniform metric rather than keeping a stale entry;
3. merge its cells into the cache and clear their unconfirmed marks (`TRAME-UI-027`).

**Frozen pane.** The visible lines of an axis are its frozen lines, then the lines from the scroll
origin. The origin never enters the frozen band (`TRAME-UI-193`). A scroll keeps the cached cells of
each band. The cells layer draws the sheet in four panes, each under its own camera and clip
(`TRAME-UI-192`). A scroll under a frozen pane repaints the layer whole.

**Geometry.** A pixel maps to an index by walking the variable sizes from the scroll origin. An
index maps back the same way, scaled by the zoom (`TRAME-UI-007`). The row walk skips a row the
active filter hides, so the axis closes over it (`features/ambre-spec-trame-shared-filter.md`,
`TRAME-UI-008`).

**Code:**

* `crates/ambre-trame/src/ui/grid/state.rs` -- the window, the version, the eviction, the answer;
* `ui/grid/geometry.rs` -- pixel to index and back, for hit-testing;
* `render/layout.rs` -- the same walk, for drawing;
* `ui/view/input.rs` -- the wheel, the finger, the edge scroll and the scroll accumulator, over the
  shared model of `1-Foundations/ambre-spec-kinetic-scroll.md`;
* `ui/view/frame.rs` -- the one-slot queue and the per-frame flush;
* `logic/commands.rs` -- the window is recomputed and answered whole.

---

## 3. SURFACE

**Triggers.**

| Trigger | Behaviour |
|---|---|
| wheel, or two fingers on a touchpad | pixels accumulate; whole rows and columns are peeled off and the remainder is kept, so a slow glide advances one row at a time |
| a flick on a touchpad | the glide's speed carries the axis on once it pauses, decaying each frame until it dies or hits an edge |
| a finger dragged on the grid | the grid follows it by whole cells, and a flick keeps it moving; a finger that barely moves is a click |
| a selection or a fill handle dragged past the edge of the cells | the grid scrolls towards that edge, faster the deeper, and the range follows to the edge cell (`TRAME-UI-175`) |
| a wheel notch, a line delta or a page delta | applied once, with no coast (`TRAME-UI-026`) |
| `Ctrl` with the wheel, a trackpad pinch | zooms the grid about the cell under the pointer (`TRAME-UI-146`); the browser does not zoom the page |
| "Zoom to selection" in the cell or header menu | zooms so the selection shows whole, its first cell at the top-left, clamped to the grid's range; the selection stays (`1-Foundations/ambre-spec-zoom.md`, `ZOOM-007`) |
| the zoom menu in the status bar | scales the cells, both header strips and the cell font together, and re-requests the window |
| a navigation or a selection that leaves the screen | scrolls the target into view (`features/ambre-spec-trame-selection.md section 3`) |
| the first frame after the sheet opens | one request, with a full repaint |
| "Freeze up to this row" or "Freeze up to this column" in the cell or header menu | the rows down to the active cell's row, or the columns up to its column, stay on screen; a line marks the edge of each band |
| "Unfreeze rows and columns" in the same menus | nothing stays frozen |

**What a cell looks like before its value arrives.**

| State | Drawn as |
|---|---|
| never requested, on a full repaint | the pending fill over the whole grid region |
| entering by a scroll | the blank content background, never the pending fill (`TRAME-UI-028`) |
| edited locally, not yet confirmed | its optimistic value under a translucent veil (`TRAME-UI-027`) |
| in error | its error text plus a corner marker (`features/ambre-spec-trame-engine.md`, `TRAME-UI-014`) |

**Rendering.** The grid region starts below the column strip and right of the row strip, and the
painter never leaves it (`TRAME-UI-013`). Every fill comes from the theme palette
(`TRAME-UI-012`). The frame itself is the canvas layer's: the repaint plan is
`ambre-spec-frame.md`, the surface is `ambre-spec-drawing-surface.md`, and the glyph atlas is
`ambre-spec-render-assets.md`.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The axis scrolls by whole rows, the remainder kept in an accumulator | a pixel scroll origin | it is what the earlier design carried and what the code never built. The grid is a lattice: an origin that is an index makes every mapping between a pixel and a cell exact, and a touchpad glide still feels continuous because the accumulator absorbs the remainder. |
| The padding is one screen each side, floored at 20 rows | a padding derived from the gesture speed, 20 to 100 rows | the earlier design prescribed it and no code implements it, and it misses the case the padding exists for: a page jump moves the origin by exactly one screen, so a margin smaller than that lands outside the cache and flashes the entering rows. |
| A late answer is dropped on a version compare | a sequence number per axis | the window is one answer over two axes, so two counters can each be current while the pair they describe never existed. |
| A late answer is dropped on a version compare | cancelling the outstanding request | the queue cannot do it: a request already sent is answered, and the answer arrives whether the window still wants it or not. |
| A cell outside the new window is evicted | keeping every cell ever received | it is unbounded on a 100 000-row sheet, and a cell that stays in the cache after leaving the window is a value nothing will refresh, which reads as authoritative. |
| A band entering by a scroll is painted blank, not pending | painting the pending fill uniformly | it is what a full repaint does, and the pending fill is grey: a grey flash on every scroll reads as a fault rather than as a wait. |
| Only a high-resolution pixel gesture gets momentum, from the shared model | one path for every scroll source | a notched wheel emits one large discrete delta, and feeding it to a velocity estimate sends the sheet flying from a single click. |
| The frozen pane is stored in the document | a frozen pane as local view state, the earlier design | an Excel or OpenDocument file stores the pane with its sheet. A local pane loses it on import, and an export by another collaborator drops it. A frozen header row is a property of the table. |
| The Logic thread adds the frozen bands to each answer | a second request for the bands | two answers for one frame race under two versions, and a band could show a value older than the window beside it. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A pixel-smooth axis** (`TRAME-UI-049`). The remainder is accumulated but never drawn: the grid
  still steps by a whole row. A smooth axis means offsetting the painted content by the remainder,
  which the `Post-V1 surface` Story (`ambre-roadmap.md` POST-V1) carries.
* **A touch pinch** carries no rule here, and none is owed. A trackpad pinch zooms, as a wheel
  with `Ctrl`. A touch pinch reaches no editor: the rule is `UI-THREAD-013`, on the shared zoom
  engine, and it ships with "One zoom model" at V1.
* **The zoom about the pointer keeps a cell, not a pixel.** The grid scrolls by whole cells, so
  the cell under the pointer stays and the point within it may shift. It is not the engine's
  transform (`UI-THREAD-012`).
* **Two implementations of one geometry** (`TRAME-UI-143`). Hit-testing walks the sizes in
  `ui/grid/geometry.rs` and drawing walks them again in `render/layout.rs`. A test compares the two
  under a zoom, resized axes, hidden rows and a scroll. One of them calling the other would make
  the rule true by type.
* **A differential answer** is unspecified: every answer carries the whole window, including the
  cells the UI already holds. The earlier design named a patch event, gated on a share of changed
  cells. Nothing states when a patch is correct against an out-of-order full answer.
