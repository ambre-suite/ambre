# AMBRE -- FEATURE: THE ROW AND COLUMN HEADERS
**Type:** Vertical (feature)
**Implementation:** partial -- the strip announced a highlight for an ordinary
selection that nothing draws (section 5).

> Two fixed strips name the rows and the columns, show where the selection is, and carry the
> gestures that act on a whole axis: take it, size it, reorder it, insert beside it. A point in a
> strip means one of those and never two.

The strips hold nothing of their own: a label is derived from a position, and every gesture they
carry belongs to another feature. There is no two-peer table, because no collaborator sees a header.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-043` | A column MUST be labelled by the spreadsheet convention: A to Z, then AA. | this spec section 3 | `ambre_trame::render::tests::col_label_matches_spreadsheet_convention` | verified |
| `TRAME-UI-044` | A point in a strip MUST resolve to exactly one gesture. | this spec section 3 | `ambre_trame::ui::grid::tests::group1::header_hit_resolves_zones`, `::header_boundary_splits_insert_and_resize`, `::axis_pointer_resolution_follows_variable_metrics` | verified |
| `TRAME-UI-045` | The header of the active row and of the active column MUST be marked. | this spec section 3 | `ambre_trame::render::tests::col_headers_draw_bg_separator_and_active_tint`, `::row_headers_skip_tint_when_inactive` | verified |
| `TRAME-UI-046` | A header under the pointer MUST be marked as reachable, unless it already carries the active mark. | this spec section 3 | `ambre_trame::render::tests::col_headers_draw_hover_tint`, `::col_headers_skip_hover_tint_on_active_column`, `::row_headers_draw_hover_tint`, `::row_headers_draw_corner_hover_tint` | verified |
| `TRAME-UI-047` | A whole-axis selection MUST be visible in the strip, since no rectangle bounds it on screen. | this spec section 3 | none. The band tint is drawn in `ui/renderer/overlay.rs` and no recording asserts it, though the band itself is covered (`TRAME-UI-033`) | unverified |
| `TRAME-UI-048` | The inline insert MUST insert at the boundary its affordance sits on. | this spec section 3 | none. The affordance resolves to the right boundary (`TRAME-UI-044`), and nothing drives the click through to the insert it fires | unverified |

* **Budget.** none of its own: the strips paint from the same viewport slice the grid does, inside
  the frame phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the strips are view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path.
* **Verification.** 4 `verified`, 2 `unverified`. What the strips reach is specified where it
  lives: a band selection in `features/ambre-spec-trame-selection.md` (`TRAME-UI-033`), a size in
  `features/ambre-spec-trame-resize.md`, the move a reorder emits in
  `features/ambre-spec-trame-rows-and-columns.md` (`TRAME-083`), and the filter buttons of the
  first row in `features/ambre-spec-trame-shared-filter.md` (`TRAME-FEAT-027`).

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
| a row label | its one-based number |
| the active row and column | a tinted header (`TRAME-UI-045`) |
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
| a boundary band, upper half | inserts a column left of it, or a row above it, through the intent the context menu already uses (`TRAME-UI-048`) |
| a boundary band, double-click | sizes that column or row to its content (`TRAME-UI-042`) |
| the right button, anywhere in a strip | the context menu, scoped to that axis: the structural and sort entries, without the cell entries (`features/ambre-spec-trame-context-menu.md section 3`) |

An insert from the strip carries no coordinates: the tracked active cell is moved onto the target
axis first, and the intent the menu sends is fired unchanged, so one path resolves the position.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One point resolves to one zone, and the boundary band splits in half | the resize and the insert start on the same pixels, and a hit target is easier to explain and to test than a modifier | a modifier to choose between them; an insert affordance elsewhere in the strip |
| A whole-axis selection is drawn in the strip, opaquely | it has no rectangle to bound it on screen, so without the strip the user cannot see what an axis operation will act on | the range fill alone, which stops at the last fetched row |
| A press on a header already selected grabs it instead of reselecting it | reselecting what is already selected does nothing, so the gesture is free, and dragging a selected column is how a spreadsheet moves one | a modifier to start the reorder |
| An insert from the strip fires the same intent as the menu | two paths resolving a position against the same document is two chances to disagree; the affordance is a shortcut, not a second rule | a command carrying the boundary |
| A hovered header drops its tint when it is the active one | two marks on one header read as two states, and the active mark is the one that carries meaning | tinting both, layered |
| The row strip walks visible rows only | a hidden row occupies no height, so a zone for it would be a target with no pixels | keeping a zone per absolute row |

---

## 5. NOT BUILT, AND NOT YET RULED

* **An ordinary selection does not reach the strip.** The design announced a header highlight for
  every row and column inside the selection bounds; the predicates for it exist and nothing calls
  them, so only the active row and column are marked (`TRAME-UI-045`).
* **The band tint is unrecorded** (`TRAME-UI-047`), in the one case where the strip is the only
  place the selection is visible.
* **The click through an insert affordance is untested** (`TRAME-UI-048`). It moves the tracked
  active cell before firing the intent, so a dropped command leaves the active cell moved and
  nothing inserted, and no rule covers that half-step.
* **The reorder drag is described and not ruled.**
  `features/ambre-spec-trame-rows-and-columns.md` owns the move itself (`TRAME-083`, `TRAME-084`)
  and describes the drag as a gesture table. What the strip owes it -- the grab, the guide and the
  drop slot -- carries no rule and no test, and no rule says what two users who move the same line
  get.
