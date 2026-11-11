# AMBRE -- FEATURE: THE ROW AND COLUMN HEADERS
**Type:** Vertical (feature)

> Two fixed strips name the rows and the columns, show where the selection is, and carry the
> gestures that act on a whole axis. Take it, size it, reorder it, insert beside it. A point in a
> strip means one of those and never two.

The strips hold nothing of their own: a label is derived from a position, and every gesture they
carry belongs to another feature. There is no two-peer table, because no collaborator sees a header.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-043` | `OBS` | A column MUST be labelled by the spreadsheet convention: A to Z, then AA. | this spec section 3 | `ambre_trame::render::tests::col_label_matches_spreadsheet_convention` -- it runs over the two carries and the three-letter case | none | verified |
| `TRAME-UI-121` | `OBS` | A row MUST be labelled by its one-based number. | this spec section 3 | `ambre_trame::render::tests::row_label_is_the_one_based_number` -- the strip draws this label | none | verified |
| `TRAME-UI-044` | `INV` | A point in a strip MUST resolve to exactly one gesture. | this spec section 3 | `ambre_trame::ui::grid::tests::group1::header_hit_resolves_zones`; `::header_boundary_splits_insert_and_resize`; `::axis_pointer_resolution_follows_variable_metrics` | none | verified |
| `TRAME-UI-045` | `OBS` | The header of the active column MUST be marked. | this spec section 3 | `ambre_trame::render::tests::col_headers_draw_bg_separator_and_active_tint` -- it asserts the active tint on the column strip | none | verified |
| `TRAME-UI-187` | `OBS` | Every row and column header inside the selection bounds MUST be marked. | this spec section 5 | not built | unbuilt -- lands with the `Grid surface` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-UI-122` | `OBS` | The header of the active row MUST be marked. | this spec section 3 | `ambre_trame::render::tests::row_headers_draw_active_tint` -- one tint, over the active row alone | none | verified |
| `TRAME-UI-046` | `OBS` | A header under the pointer MUST be marked as reachable, unless it already carries the active mark. | this spec section 3 | `ambre_trame::render::tests::col_headers_draw_hover_tint`; `::row_headers_draw_hover_tint`; `::row_headers_draw_corner_hover_tint` -- the mark on both strips and the corner; `::col_headers_skip_hover_tint_on_active_column` -- the exception on the column strip alone | none | verified |
| `TRAME-UI-047` | `OBS` | A whole-axis selection MUST be visible in the strip, since no rectangle bounds it on screen. | this spec section 3 | `ambre_trame::ui::renderer::tests::a_whole_axis_selection_tints_its_strip` -- a column band and a row band | none | verified |

### 1.2 Insert from a boundary

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-048` | `POST` | The inline insert MUST insert at the boundary its affordance sits on. | this spec section 3 | `ambre_trame::ui::view::view_tests::the_inline_insert_lands_on_its_boundary_and_sizes_nothing` -- a click on each strip, read at the command it sends | none | verified |
| `TRAME-UI-123` | `FRAME` | An insert fired from a boundary MUST NOT change the size of either axis that boundary separates. | this spec section 4; derived | `ambre_trame::ui::view::view_tests::the_inline_insert_lands_on_its_boundary_and_sizes_nothing` -- no resize is sent and no local size changes | none | verified |

* **Budget.** none of its own: the strips paint from the same viewport slice the grid does, inside
  the frame phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the strips are view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path.
* **Verification.** 9 `verified`, 1 `unimplemented`. What holds is the column strip: its label
  convention, and the tint that marks the active column. What is loose is the row strip, whose
  number has no rule of its own and whose tint is only ever asserted absent. The insert gesture is
  loose, untested end to end, sharing its pixels with the resize. The strips reach a band selection
  (`features/ambre-spec-trame-selection.md`), a size (`features/ambre-spec-trame-resize.md`), a
  reorder (`features/ambre-spec-trame-rows-and-columns.md`) and the first row's filter buttons
  (`features/ambre-spec-trame-shared-filter.md`), ruled where they live.

---

## 2. MODEL

**Containers.** None. A label is computed from a position, and every strip pixel is derived from
the sizes the viewport already holds (`features/ambre-spec-trame-grid-viewport.md section 2`).

**The zones.** One point in a strip resolves to one of seven zones (`TRAME-UI-044`):

| Zone | Reached by | Acts on |
|---|---|---|
| the body of a column header | a press | the whole column, as a band |
| the body of a row header | a press | the whole row |
| the lower half of a column boundary band | a press and drag, or a double-click | the width of the column left of it |
| the lower half of a row boundary band | the same | the height of the row above it |
| the upper half of a column boundary band | a click | an insert at that boundary |
| the upper half of a row boundary band | a click | an insert at that boundary |
| the top-left corner | a click | the whole sheet |

The boundary band is four pixels each side of an edge. The row strip walks visible rows only, so a
row the filter hides has no zone at all (`TRAME-UI-008`).

**View state.** Two fields of `TrameUiState`, both cleared when the pointer leaves:

```rust
hover_header: Option<HeaderHit>,   // the body or corner under the pointer
insert_hint: Option<HeaderHit>,    // the boundary whose insert affordance is shown
```

**Code:**

* `crates/ambre-trame/src/ui/grid/geometry.rs` -- the zone resolution;
* `render/headers.rs` -- the strips, the labels, the separators, the tints;
* `ui/renderer/overlay.rs` -- the band tint, the insert affordance, the resize guide;
* `ui/view/input.rs` -- the press paths and the hover tracking;
* `ui/view/interact.rs` -- the insert, the reorder release, the auto-fit request.

---

## 3. SURFACE

**What a strip draws.**

| Element | Drawn as |
|---|---|
| the strips | fixed bands, above and left of the grid, that the scroll never moves (`TRAME-UI-013`) |
| a column label | A to Z, then AA, from its position (`TRAME-UI-043`) |
| a row label | its one-based number (`TRAME-UI-121`) |
| the active column and the active row | a tinted header (`TRAME-UI-045`, `TRAME-UI-122`) |
| a header under the pointer | a lighter tint, dropped when that header is the active one (`TRAME-UI-046`) |
| a whole-axis selection | an opaque tint over the whole band, plus a fill over the grid body (`TRAME-UI-047`) |
| a hovered boundary | a small plus, and a cursor naming the axis a drag would move |
| a live resize | a guide line at the prospective edge (`features/ambre-spec-trame-resize.md section 4`) |
| a live reorder | a guide line at the boundary the grabbed axis would land on |
| the first row of a filtered range | a button per column (`features/ambre-spec-trame-shared-filter.md section 4`) |

**Where a press goes.**

| Press | Sends |
|---|---|
| a header body, outside the current band | takes that axis as a band and arms a sweep along the strip |
| a header body inside the current band | grabs it for a reorder: the release moves the column or row (`features/ambre-spec-trame-rows-and-columns.md section 4`) |
| a header body with `Shift`, on a band of the same axis | extends the band |
| the corner | takes every column as one band |
| a boundary band, lower half | arms a resize |
| a boundary band, upper half | inserts a column left of it, or a row above it, through the intent the context menu already uses (`TRAME-UI-048`), and sizes nothing (`TRAME-UI-123`) |
| a boundary band, double-click | sizes that column or row to its content (`TRAME-UI-042`) |
| the right button, anywhere in a strip | the context menu, scoped to that axis: the structural and sort entries, without the cell entries (`features/ambre-spec-trame-context-menu.md section 3`) |

An insert from the strip carries no coordinates. The tracked active cell is moved onto the target
axis first, and the intent the menu sends is fired unchanged. One path resolves the position. The
intent goes out through the outbox, and a refused send selects no band (`BUS-018`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One point resolves to one zone, the boundary band splitting in half | a modifier to choose between them | the resize and the insert start on the same pixels, and a hit target is easier to explain and to test than a modifier. |
| One point resolves to one zone, the boundary band splitting in half | an insert affordance elsewhere in the strip | it moves the gesture away from the boundary it acts on, so the user has to learn where the affordance lives rather than pointing at the line. |
| A whole-axis selection is drawn in the strip, opaquely | the range fill alone | it stops at the last fetched row. A whole-axis selection has no rectangle to bound it on screen, so without the strip the user cannot see what an axis operation will act on. |
| A press on a header already selected grabs it | a modifier to start the reorder | reselecting what is already selected does nothing, so the gesture is free, and dragging a selected column is how a spreadsheet moves one. |
| An insert from the strip fires the same intent as the menu | a command carrying the boundary | two paths resolving a position against the same document is two chances to disagree; the affordance is a shortcut, not a second rule. |
| A hovered header drops its tint when it is the active one | tinting both, layered | two marks on one header read as two states, and the active mark is the one that carries meaning. |
| The row strip walks visible rows only | keeping a zone per absolute row | a hidden row occupies no height, so a zone for it would be a target with no pixels. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **An ordinary selection does not reach the strip.** The design announced a header highlight for
  every row and column inside the selection bounds. The predicates for it exist and nothing calls
  them, so only the active row and column are marked (`TRAME-UI-045`).
* **The band tint is unrecorded** (`TRAME-UI-047`), in the one case where the strip is the only
  place the selection is visible.
* **The click through an insert affordance is untested** (`TRAME-UI-048`, `TRAME-UI-123`). It
  moves the tracked active cell before firing the intent, which goes out through the outbox, so a
  refused send selects no band (`BUS-018`). No test drives the click through to the insert.
* **The row strip is asserted only in the negative** (`TRAME-UI-121`, `TRAME-UI-122`). Every
  recording of it passes no active row and reads no label text. The tint that marks the active row
  and the number that names a row are both held by reading the code. The column strip has a
  test for each.
* **The reorder drag is described and not ruled.**
  `features/ambre-spec-trame-rows-and-columns.md` owns the move itself (`TRAME-083`, `TRAME-084`)
  and describes the drag as a gesture table. What the strip owes it -- the grab, the guide and the
  drop slot -- carries no rule and no test. No rule says what two users who move the same line get.
