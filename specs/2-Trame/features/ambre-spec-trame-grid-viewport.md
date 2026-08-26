# AMBRE -- FEATURE: THE GRID VIEWPORT
**Type:** Vertical (feature)
**Implementation:** partial -- scrolling, the request window and variable metrics are built; freeze is not.

> A user scrolls a sheet of 100 000 rows. Only the cells on screen, plus about a screen above and
> below, ever leave the Logic thread. A cell the user has not been sent reads as pending or as
> blank, never as a stale value.

The viewport is local view state: it writes nothing to the document and reaches no collaborator.
There is no two-peer table below, because two users scrolling one sheet see nothing of each other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-004` | Only the visible range, plus a bounded padding, MUST be requested from the Logic thread. | this spec section 2 | none. The window is built in `ui/grid/state.rs::scroll_viewport_cmd` and nothing compares the requested bounds with the visible range | unverified |
| `TRAME-UI-005` | An answer whose version is lower than the last accepted one MUST be discarded. | this spec section 2 | none. The rule is in the code (`ui/grid/state.rs`: `if version < self.accepted_version { return; }`) and nothing exercises it | unverified |
| `TRAME-UI-007` | Hit-testing MUST be the exact inverse of the drawn geometry, under zoom and under variable row and column sizes. | this spec section 2 | `ambre_trame::ui::grid::tests::group1::zoom_scales_metrics_and_round_trips_hit_test`, `::variable_metrics_round_trip_pixel_and_hit_test`, `::axis_pointer_resolution_follows_variable_metrics`, `::filter_button_hit_test_matches_drawn_rect`, `ambre_trame::render::tests::variable_metrics_shift_cells_and_hit_test` | verified |
| `TRAME-UI-012` | Every fill colour drawn by the renderer MUST come from the theme palette; no literal colour in render code. | this spec section 3; `ambre-spec-ui.md` (design tokens) | `ambre_trame::ui::renderer::tests::renderer_uses_only_palette_colors_for_fills`. The fixture holds one unstyled cell, so a cell the user coloured is outside what it measures (`features/ambre-spec-trame-cell-styles.md`) | verified |
| `TRAME-UI-013` | The renderer MUST NOT paint outside the grid region, whatever the canvas size or the scroll position. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_strip_skips_when_outside_grid`, `ambre_trame::ui::grid::tests::group2::grid_bottom_floors_at_header_on_tiny_canvas`, `::header_strip_starts_at_top_of_canvas` | verified |
| `TRAME-UI-024` | A cell held from an earlier window MUST be dropped when it falls outside the new one. | this spec section 2 | none. `scroll_viewport_cmd` retains only the requested window, and no test drives a cell out of it | unverified |
| `TRAME-UI-025` | At most one viewport request MUST leave the UI thread per frame. | this spec section 2 | type-level: the pending request is one `Option<TrameCommand>` (`ui/view/frame.rs`), so a second request in the same frame replaces the first, and the frame flush is its only send site | verified |
| `TRAME-UI-026` | A single wheel notch MUST NOT start a momentum coast. | this spec section 3 | none. `handle_wheel` classifies the sample and zeroes the velocity, and no test drives the wheel path at all | unverified |
| `TRAME-UI-027` | A cell the user has just edited MUST read as unconfirmed until its computed value arrives. | this spec section 3 | `ambre_trame::ui::grid::tests::group2::dirty_and_error_lifecycle`, `ambre_trame::ui::renderer::tests::dirty_cell_gets_a_translucent_veil` | verified |
| `TRAME-UI-028` | A band entering the viewport by a scroll MUST NOT be painted as pending. | this spec section 3 | `ambre_trame::ui::renderer::tests::draw_cells_scroll_strip_base_is_blank_not_pending_grey`, `::draw_cells_full_repaint_paints_pending_then_grid` | verified |
| `TRAME-UI-029` | Freezing rows or columns MUST reach no collaborator: it MUST produce no change to the document. | this spec section 5 | not built -- lands with the "Grid surface" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-UI-049` | The painted grid MUST be offset by the sub-cell scroll remainder. | this spec section 5 | not built -- lands with the "Post-V1 surface" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** four rows of `ambre-spec-budgets.md section 1`: the scroll round trip in
  < 2 ms, the scroll viewport response in < 15 ms, the `ScrollViewport` send in < 1 ms, and the
  render of a `TrameEvent::ViewportData` in < 5 ms. Only the first two have a bench.
* **Boundary.** none: the viewport is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses is a request and a display slice, never
  document data (`TRAME-UI-001`).
* **Verification.** 6 `verified`, 4 `unverified`, 2 `unimplemented`. The four gaps are one gap: the
  request window, the version guard, the eviction and the whole wheel path ship with nothing
  guarding them, and each is a state machine on the UI thread that a host test can drive.

---

## 2. MODEL

**Containers.** None. The viewport persists nothing. It reads the sizes of the rows and columns it
covers out of the answer it receives; `col_widths_{sheet}` and `row_heights_{sheet}` belong to the
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
2. pad the visible range: one screen of rows each side, floored at 20, plus the count of rows the
   active filter hides in that window, and five columns each side;
3. drop every cached cell outside that window (`TRAME-UI-024`), then rebuild the hidden-row set,
   because an evicted cell carried the flag it was derived from;
4. stamp a fresh monotonic version and hold the command in the one-slot queue (`TRAME-UI-025`);
5. the frame flush sends it, dropping it if the channel is full -- the next scroll supersedes it.

**Steps, on every answer:**

1. discard it when its version is lower than the last accepted one (`TRAME-UI-005`);
2. adopt the sizes it carries, storing only those that differ from the default, so a row reset to
   the default falls back to the uniform metric rather than keeping a stale entry;
3. merge its cells into the cache and clear their unconfirmed marks (`TRAME-UI-027`).

**Geometry.** A pixel maps to an index by walking the variable sizes from the scroll origin, and an
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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The axis scrolls by whole rows and columns, with the sub-cell remainder kept in an accumulator | the grid is a lattice: an origin that is an index makes every mapping between a pixel and a cell exact, and a touchpad glide still feels continuous because the accumulator absorbs the remainder | a pixel scroll origin, which is what the earlier design carried and what the code never built |
| The padding is one screen each side, floored at 20 rows | a page jump moves the origin by exactly one screen, so a smaller margin lands outside the cache and flashes the entering rows | a padding derived from the gesture speed, 20 to 100 rows, which the earlier design prescribed and no code implements |
| A late answer is dropped on a version compare | the Logic thread answers each request, and a slow one arriving after a fast one would repaint the grid with the wrong window | a sequence number per axis; cancelling the outstanding request, which the channel cannot do |
| A cell outside the new window is evicted | a cell that stays in the cache after leaving the window is a value nothing will refresh, and it reads as authoritative | keeping every cell ever received, which is unbounded on a 100 000-row sheet |
| A band entering by a scroll is painted blank, not pending | the pending fill is grey, and a grey flash on every scroll reads as a fault rather than as a wait | painting the pending fill uniformly, which is what a full repaint does |
| Only a high-resolution pixel gesture gets momentum | a notched wheel emits one large discrete delta, and feeding it to a velocity estimate sends the sheet flying from a single click | one path for every scroll source |
| Freeze stays local | a frozen pane is a reading preference and screens differ; sharing it would make two users fight over one number | freezing as a document property, synchronized to every collaborator |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Freeze** (`TRAME-UI-029`): the design is a second pass drawing the frozen bands at their
  absolute offsets, with the scroll subtracted only on the free axis, and a separator stroked at
  each boundary. No field carries it and nothing draws it.
* **A pixel-smooth axis** (`TRAME-UI-049`). The remainder is accumulated but never drawn: the grid
  still steps by a whole row. A smooth axis means offsetting the painted content by the remainder,
  which the "Post-V1 surface" Story carries.
* **Touch and pinch to zoom** carry no rule here, and none is owed: touch input is horizontal --
  `Ctrl` with the wheel is handed to the browser and no two-finger gesture reaches the grid, in
  Trame as in Toile -- so the rule is `UI-THREAD-013`, on the shared zoom engine, and it ships with
  "One zoom model" at V1.
* **The whole wheel and momentum path has no test**, in a crate whose rule is that an interaction
  lands with a geometry round trip and a recording of what it painted
  (`ambre-spec-testing.md section 6`). It is pure state over an injected clock, so nothing
  about it needs a browser.
* **Two implementations of one geometry.** Hit-testing walks the sizes in `ui/grid/geometry.rs` and
  drawing walks them again in `render/layout.rs`. `TRAME-UI-007` is what holds the two together,
  and it holds them by test rather than by construction: one of them calling the other would make
  the rule true by type.
* **A differential answer** is unspecified: every answer carries the whole window, including the
  cells the UI already holds. The earlier design named a patch event, gated on a share of changed
  cells; nothing states when a patch is correct against an out-of-order full answer.
