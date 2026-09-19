# AMBRE -- FEATURE: THE GRID VIEWPORT
**Type:** Vertical (feature)
**Implementation:** partial -- scrolling, the request window and variable metrics are built. Freeze
is not, and the request window does not cover a screen of narrow columns.

> A user scrolls a sheet of 100 000 rows. Only the cells on screen, plus about a screen above and
> below, ever leave the Logic thread. A cell the user has not been sent reads as pending or as
> blank, never as a stale value.

The viewport is local view state: it writes nothing to the document and reaches no collaborator.
There is no two-peer table below, because two users scrolling one sheet see nothing of each other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-007` | `INV` | Hit-testing MUST be the exact inverse of the geometry it walks, under zoom and under variable row and column sizes. | this spec section 2 | `ambre_trame::ui::grid::tests::group1::zoom_scales_metrics_and_round_trips_hit_test`; `::variable_metrics_round_trip_pixel_and_hit_test`; `::axis_pointer_resolution_follows_variable_metrics`; `::filter_button_hit_test_matches_drawn_rect`; `ambre_trame::render::tests::variable_metrics_shift_cells_and_hit_test` -- each holds one modifier, the variable-metric round trips running at 100% and the zoom round trip on default metrics | none | verified |
| `TRAME-UI-143` | `INV` | The geometry the renderer draws MUST agree with the geometry hit-testing walks. | this spec section 2 | none | unheld -- `ui/grid/geometry.rs` and `render/layout.rs` walk the same sizes twice, and each of the two tests above closes the loop inside one of them. Nothing compares a drawn rect with a hit-tested one (section 5); lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-024` | `INV` | The cache MUST hold no cell outside the last requested window. | this spec section 2 | none | unheld -- `scroll_viewport_cmd` retains only the requested window, and no test drives a cell out of it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-012` | `OBS` | Every fill colour drawn by the renderer MUST come from the theme palette; no literal colour in render code. | this spec section 3; `ambre-spec-ui.md` (design tokens) | `ambre_trame::ui::renderer::tests::renderer_uses_only_palette_colors_for_fills` -- its fixture holds one unstyled cell, so a cell the user coloured is outside what it measures (`features/ambre-spec-trame-cell-styles.md`) | none | verified |
| `TRAME-UI-013` | `OBS` | The renderer MUST NOT paint outside the grid region, whatever the canvas size or the scroll position. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_strip_skips_when_outside_grid`; `ambre_trame::ui::grid::tests::group2::grid_bottom_floors_at_header_on_tiny_canvas`; `::header_strip_starts_at_top_of_canvas` | none | verified |
| `TRAME-UI-027` | `OBS` | A cell the user has just edited MUST read as unconfirmed until its computed value arrives. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::dirty_and_error_lifecycle`; `ambre_trame::ui::renderer::tests::dirty_cell_gets_a_translucent_veil` | none | verified |
| `TRAME-UI-028` | `OBS` | A band entering the viewport by a scroll MUST NOT be painted as pending. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_scroll_strip_base_is_blank_not_pending_grey`; `::draw_cells_full_repaint_paints_pending_then_grid` | none | verified |
| `TRAME-UI-049` | `OBS` | The painted grid MUST be offset by the sub-cell scroll remainder. | this spec section 5 | not built | unbuilt -- lands with the `Post-V1 surface` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 The user scrolls

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-004` | `POST` | A scroll MUST request no more than the visible range plus a bounded padding. | this spec section 2 | none | unheld -- the window is built in `ui/grid/state.rs::scroll_viewport_cmd` and nothing compares the requested bounds with the visible range. The padding is not the constant it reads as: the row window is stretched by the count of every hidden row the cache knows, not by those inside the window; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-144` | `POST` | A scroll MUST request every cell the user can see. | this spec section 2 | none | fails -- on the column axis the window is sized from the uniform metric, and a resized column can be a third of it, so a screen of narrow columns runs past the last one requested. Those columns are never asked for, and the same window evicts them if an earlier one had brought them in (section 5). The row axis survives by margin, not by rule; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |
| `TRAME-UI-025` | `POST` | At most one viewport request MUST leave the UI thread per frame. | this spec section 2 | `ambre_trame::ui::view::tests::the_viewport_queue_coalesces_to_one_request` -- every request source collapses to one drain; `::a_dropped_request_is_re_offered_not_doubled` -- a dropped request is re-offered, never doubled | none | verified |
| `TRAME-UI-145` | `FRAME` | A scroll MUST NOT change which cell is active or which range is selected. | this spec section 2 | none | unheld -- `scroll_by` writes the origin and reads nothing else, but no test drives it: `scroll_by`, `scroll_viewport_cmd` and `accepted_version` are named by no test in the crate; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 The wheel turns

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-147` | `POST` | A wheel notch, a line delta or a page delta MUST be applied once and in full, keeping the sub-cell remainder. | this spec section 3 | none | unheld -- `handle_wheel` classifies the sample and feeds the accumulator, and no test drives the wheel path at all (section 5); lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-026` | `FRAME` | A single wheel notch MUST NOT start a momentum coast. | this spec section 3 | none | unheld -- `handle_wheel` classifies the sample and zeroes the velocity, and no test drives the wheel path at all; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-146` | `FRAME` | The wheel held with `Ctrl` MUST NOT scroll the grid. | this spec section 3 | none | unheld -- `handle_wheel` returns before reading the delta, which is also what hands the gesture to the browser, and no test drives it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 An answer arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-005` | `POST` | An answer whose version is lower than the last accepted one MUST be dropped. | this spec section 2 | none | unheld -- the rule is in the code (`ui/grid/state.rs`, `if version < self.accepted_version`) and nothing exercises it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-148` | `POST` | A row or column an answer reports at the default size MUST read as the default, not as the size it held before. | this spec section 2; `ambre-spec-trame-schema.md section 4` ("absence is a value") | none | unheld -- `on_viewport_data` removes the entry instead of storing the default, and every metric test writes the maps directly: no test sends an answer that carries sizes; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-149` | `FRAME` | An answer MUST NOT move the scroll origin. | this spec section 2 | none | unheld -- `on_viewport_data` writes the caches and the hidden-row set and touches neither `top_row` nor `left_col`, but nothing asserts it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Freeze a band of rows or columns

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-029` | `FRAME` | Freezing rows or columns MUST reach no collaborator: it MUST produce no change to the document. | this spec section 5 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`. The scroll round trip is in < 2 ms
  and the scroll viewport response in < 15 ms. The `ScrollViewport` send is in < 1 ms and the
  render of a `TrameEvent::ViewportData` in < 5 ms. Only the first two have a bench.
* **Boundary.** none: the viewport is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses is a request and a display slice, never
  document data (`TRAME-UI-001`).
* **Verification.** 6 `verified`, 1 `failing`, 10 `untested`, 2 `unimplemented`. What holds is what is
  drawn or hit-tested, and the one-slot queue that caps a frame at one request (`TRAME-UI-025`).
  What is loose is every state machine: the window, the version guard, the eviction, the wheel, the
  metric adoption. The one failure is the narrow-column window (`TRAME-UI-144`).

---

## 2. MODEL

**Containers.** None. The viewport persists nothing. It reads the sizes of the rows and columns it
covers out of the answer it receives. `col_widths_{sheet}` and `row_heights_{sheet}` belong to the
resize feature and are declared in `ambre-spec-trame-schema.md section 3`.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ScrollViewport { version, row_start, row_end, col_start, col_end }` | UI to Logic | the padded window, in absolute indices. No sheet id: the Logic thread resolves the active sheet |
| `TrameEvent::ViewportData { version, row_start, col_start, cells, col_widths, row_heights }` | Logic to UI | one whole window of display cells, and the sizes of its rows and columns |

**View state.** Fields of `TrameUiState`:

```rust
top_row: u32,   // first visible row: the axis scrolls by index, not by pixel
left_col: u32,
cell_cache: HashMap<(u32, u32), CellData>,  // a miss draws as pending or blank
dirty_cells: HashSet<(u32, u32)>,           // edited locally, not yet confirmed
accepted_version: u64,                      // the last answer taken (`TRAME-UI-005`)
next_version: u64,
col_widths: HashMap<u32, f32>,              // only a size that differs from the default
row_heights: HashMap<u32, f32>,
zoom: f64,
```

**Steps, on every scroll:**

1. move the origin by whole rows and columns, clamped to the grid bounds;
2. pad the visible range by one screen of rows each side, floored at 20, plus the count of rows
   hidden by the active filter there. Five columns pad each side;
3. drop every cached cell outside that window (`TRAME-UI-024`), then rebuild the hidden-row set,
   because an evicted cell carried the flag it was derived from;
4. stamp a fresh monotonic version and hold the command in the one-slot queue (`TRAME-UI-025`);
5. the frame flush sends it, dropping it if the channel is full -- the next scroll supersedes it.

**Steps, on every answer:**

1. drop it when its version is lower than the last accepted one (`TRAME-UI-005`);
2. adopt the sizes it carries, storing only those that differ from the default. A row reset to the
   default then falls back to the uniform metric rather than keeping a stale entry;
3. merge its cells into the cache and clear their unconfirmed marks (`TRAME-UI-027`).

**Geometry.** A pixel maps to an index by walking the variable sizes from the scroll origin. An
index maps back the same way, scaled by the zoom (`TRAME-UI-007`). The row walk skips a row the
active filter hides, so the axis closes over it (`features/ambre-spec-trame-shared-filter.md`,
`TRAME-UI-008`).

**Code:**

* `crates/ambre-trame/src/ui/grid/state.rs` -- the window, the version, the eviction, the answer;
* `ui/grid/geometry.rs` -- pixel to index and back, for hit-testing;
* `render/layout.rs` -- the same walk, for drawing;
* `ui/view/input.rs` -- the wheel, the momentum coast, the scroll accumulator;
* `ui/view/frame.rs` -- the one-slot queue and the per-frame flush;
* `logic/commands.rs` -- the window is recomputed and answered whole.

---

## 3. SURFACE

**Triggers.**

| Trigger | Behaviour |
|---|---|
| wheel, or two fingers on a touchpad | pixels accumulate; whole rows and columns are peeled off and the remainder is kept, so a slow glide advances one row at a time |
| a flick on a touchpad | the release speed carries the axis on, decaying each frame until it dies or hits an edge |
| a wheel notch, a line delta or a page delta | applied once, with no coast (`TRAME-UI-026`) |
| `Ctrl` with the wheel | left to the browser: the page zooms, the grid does not |
| the zoom menu in the status bar | scales the cells, both header strips and the cell font together, and re-requests the window |
| a navigation or a selection that leaves the screen | scrolls the target into view (`features/ambre-spec-trame-selection.md section 3`) |
| the first frame after the sheet opens | one request, with a full repaint |

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
| A late answer is dropped on a version compare | cancelling the outstanding request | the channel cannot do it: a request already sent is answered, and the answer arrives whether the window still wants it or not. |
| A cell outside the new window is evicted | keeping every cell ever received | it is unbounded on a 100 000-row sheet, and a cell that stays in the cache after leaving the window is a value nothing will refresh, which reads as authoritative. |
| A band entering by a scroll is painted blank, not pending | painting the pending fill uniformly | it is what a full repaint does, and the pending fill is grey: a grey flash on every scroll reads as a fault rather than as a wait. |
| Only a high-resolution pixel gesture gets momentum | one path for every scroll source | a notched wheel emits one large discrete delta, and feeding it to a velocity estimate sends the sheet flying from a single click. |
| Freeze stays local | freezing as a document property, synchronized to every collaborator | a frozen pane is a reading preference and screens differ; sharing it would make two users fight over one number. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Freeze** (`TRAME-UI-029`): the design is a second pass drawing the frozen bands at their
  absolute offsets, with the scroll subtracted only on the free axis. A separator is stroked at
  each boundary. No field carries it and nothing draws it.
* **A pixel-smooth axis** (`TRAME-UI-049`). The remainder is accumulated but never drawn: the grid
  still steps by a whole row. A smooth axis means offsetting the painted content by the remainder,
  which the "Post-V1 surface" Story (`ambre-roadmap.md` POST-V1) carries.
* **Touch and pinch to zoom** carry no rule here, and none is owed. Touch input is horizontal:
  `Ctrl` with the wheel is handed to the browser and no two-finger gesture reaches the grid, in
  Trame as in Toile. The rule is `UI-THREAD-013`, on the shared zoom engine, and it ships with
  "One zoom model" at V1.
* **The whole wheel and momentum path has no test.** The crate's rule is that an interaction
  lands with a geometry round trip and a recording of what it painted
  (`ambre-spec-testing.md section 6`). It is pure state over an injected clock, so nothing
  about it needs a browser.
* **A screen of narrow columns is not fetched** (`TRAME-UI-144`). `viewport_cols` sizes the request
  window from the uniform metric, the canvas width over the default column width; the grid is drawn
  from the per-column sizes. A column resized to under a third of the default puts three times as
  many columns on screen as the window asks for. The ones past its end stay blank: the next scroll
  evicts whatever the cache still held there, and no request ever names them. The row axis has the
  same shape but survives, its one-screen padding covering the ratio between the default height and
  its floor. The fix is to size the window by walking the metrics the grid draws -- the walk
  `TRAME-UI-143` already asks the two implementations to share.
* **The padding grows with the filter, not with the window** (`TRAME-UI-004`). The row window is
  extended by the count of every hidden row in the cache. The name and the comment say the hidden
  rows in the window. The two agree only while the cache holds one window. The eviction makes that
  true, and a late answer for a wider window undoes it.
* **Two implementations of one geometry** (`TRAME-UI-143`). Hit-testing walks the sizes in
  `ui/grid/geometry.rs` and drawing walks them again in `render/layout.rs`, one commented as
  mirroring the other. Nothing holds them together: each test closes a round trip inside one
  implementation, so the two could drift a pixel apart and stay green. One of them calling the
  other would make the rule true by type.
* **A differential answer** is unspecified: every answer carries the whole window, including the
  cells the UI already holds. The earlier design named a patch event, gated on a share of changed
  cells. Nothing states when a patch is correct against an out-of-order full answer.
