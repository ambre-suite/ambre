# AMBRE -- FEATURE: THE ZOOM MODEL

**Type:** Horizontal (framework)

> A user zooms with the same keys, the same wheel and the same pinch in every editor, and reads the
> factor from one chip. What zooms is the view alone: no collaborator sees it, and the document does
> not carry it.

Three editors zoom three spaces: a grid, an endless board, a column of pages. One model carries
them all -- a factor, a clamp, a fixed point, one ladder of presets. This document owns the model; each
application maps it onto its own space, and `ambre-spec-chrome.md` owns the chip.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `ZOOM`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-004` | `INV` | `Ctrl` with `=`, `-` or `0`, `Ctrl` with the wheel, and a trackpad pinch MUST zoom in every application. | this spec section 3 | `ambre_core::zoom::tests::a_wheel_step_zooms_the_way_it_turns` -- the engine turns a wheel step into a factor; `ambre_trame::ui::view::view_tests::a_ctrl_wheel_zooms_about_the_cell_under_the_pointer`; `ambre_toile::ui::view::view_tests::a_ctrl_wheel_zooms_about_the_pointer`; `ambre_app::host::prose::tests::a_ctrl_wheel_zooms_the_page_about_the_pointer` -- wasm tier, the event is consumed so the browser does not zoom the tab, and a browser reports a trackpad pinch as that same wheel | none | verified |
| `ZOOM-006` | `INV` | The preset ladder MUST be one ladder for every application, read by the shared chip. | this spec section 2; `ambre-spec-chrome.md`, `CHROME-014` | type-level -- the ladder is one function, `ambre_core::zoom::ladder`, the chip takes a range and no list of its own, and no application holds a ladder; `ambre_core::zoom::tests::every_editor_reads_its_presets_off_the_one_ladder`; `ambre_core::zoom::tests::a_step_moves_along_the_ladder_inside_the_range`; `ambre_chrome::tests::chrome_ui::the_zoom_chip_lists_the_one_ladder_inside_its_range` -- the chip lists the stops inside the range it is given; `ambre_commands::tests::the_zoom_chords_step_the_zoom_in_every_application`; `ambre_trame::ui::view::view_tests::a_zoom_chord_steps_the_grid_and_sends_only_a_cull`; `ambre_toile::ui::view::view_tests::a_zoom_chord_steps_the_camera_and_sends_only_a_cull`; `ambre_prose::input_tests::a_zoom_key_steps_the_zoom_and_moves_nothing_else` -- a chord steps each editor to the next stop | none | verified |

### 1.2 Zoom the view

The user presses a chord, turns the wheel under `Ctrl`, or pinches. One factor moves, about one
point.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-002` | `POST` | A zoom MUST clamp to the application's supported range, away from zero, whatever the input. | this spec section 2 | `ambre_core::zoom::tests::a_zoom_clamps_to_its_range_whatever_the_input` -- any factor, a non-finite one included, lands inside the range; type-level -- each application builds its factor through its own `ZoomRange`, and the chip clamps a click through the same range | none | verified |
| `ZOOM-003` | `POST` | A zoom about a point MUST leave that point under the pointer, in the application's own mapping, as far as the view's own bounds let it. | this spec section 2 | `ambre_core::zoom::tests::a_zoom_about_a_point_keeps_the_content_under_it` -- the engine's transform, one axis; `ambre_toile::ui::view::view_tests::a_ctrl_wheel_zooms_about_the_pointer` -- the board point stays within a thousandth of a pixel; `ambre_prose::view::zoom::tests::a_zoom_about_a_point_keeps_the_point_under_it`; `ambre_app::host::prose::tests::a_ctrl_wheel_zooms_the_page_about_the_pointer` -- wasm tier; `ambre_trame::ui::view::view_tests::a_ctrl_wheel_zooms_about_the_cell_under_the_pointer` -- the grid keeps the cell under the pointer | none | verified |
| `ZOOM-005` | `POST` | `Ctrl` with `0` MUST return the factor to one, a hundred per cent. | this spec section 3 | `ambre_core::zoom::tests::a_reset_returns_to_one`; `ambre_commands::tests::the_zoom_chords_step_the_zoom_in_every_application`; `ambre_trame::ui::view::view_tests::a_zoom_chord_steps_the_grid_and_sends_only_a_cull`; `ambre_toile::ui::view::view_tests::a_zoom_chord_steps_the_camera_and_sends_only_a_cull`; `ambre_prose::input_tests::a_zoom_key_steps_the_zoom_and_moves_nothing_else` | none | verified |
| `ZOOM-001` | `FRAME` | A zoom MUST emit no CRDT delta. | this spec section 2 | `ambre_trame::ui::view::view_tests::a_zoom_chord_steps_the_grid_and_sends_only_a_cull`; `ambre_toile::ui::view::view_tests::a_zoom_chord_steps_the_camera_and_sends_only_a_cull` -- a chord sends the Logic thread a viewport cull and no edit; `ambre_prose::input_tests::a_zoom_key_steps_the_zoom_and_moves_nothing_else` -- a zoom key sends nothing at all | none | verified |
| `ZOOM-008` | `FRAME` | A zoom MUST reach no collaborator. | this spec section 2; `ambre-spec-presence.md` | type-level -- neither `Scale` nor `Zoom` implements a serde trait. The presence payloads carry no view coordinate: Toile sends world points, Prose a CRDT cursor, and Trame sends none; `ambre_toile::ui::view::view_tests::a_zoom_about_the_pointer_broadcasts_no_presence_sample` -- the sample built after a zoom about the pointer is the one built before it, so none is broadcast; `ambre_trame::ui::view::view_tests::a_zoom_chord_steps_the_grid_and_sends_only_a_cull`; `ambre_toile::ui::view::view_tests::a_zoom_chord_steps_the_camera_and_sends_only_a_cull`; `ambre_prose::input_tests::a_zoom_key_steps_the_zoom_and_moves_nothing_else` -- a zoom sends no presence command. It is not the same rule as emitting no delta: a presence sample is broadcast and written to no document | none | verified |

### 1.3 Zoom to a selection

The user asks for the selection to fill the view. The target may be empty.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-007` | `POST` | A zoom to a selection MUST frame it and clamp. | this spec section 3 | `ambre_core::zoom::tests::a_box_is_framed_whole_and_an_empty_one_is_ignored` -- the frame fits the box whole, clamped; `ambre_toile::ui::view::view_tests::a_zoom_to_the_selection_frames_it_and_keeps_the_selection`; `ambre_trame::ui::grid::tests::viewport::a_zoom_to_the_selection_shows_it_whole_and_keeps_it`; `ambre_trame::ui::grid::tests::viewport::a_zoom_to_the_selection_clamps_to_the_grid_range` | none | verified |
| `ZOOM-011` | `POST` | A zoom to a selection MUST frame it in the part of the view that no floating surface of the shell covers. | this spec section 3; `ambre-spec-chrome.md` | `ambre_toile::ui::viewport::tests::a_fit_lands_clear_of_the_floating_surfaces` -- the box lands whole inside the uncovered part and centred on it, and centred alone when the clamp binds; `ambre_toile::ui::viewport::tests::the_clearance_matches_the_stylesheet` -- the strips carry the offsets the stylesheet gives the palette and the selection bar | none | verified |
| `ZOOM-010` | `FRAME` | A zoom to a selection MUST NOT change the selection. | this spec section 3; derived | `ambre_toile::ui::view::view_tests::a_zoom_to_the_selection_frames_it_and_keeps_the_selection`; `ambre_trame::ui::grid::tests::viewport::a_zoom_to_the_selection_shows_it_whole_and_keeps_it` -- the selection compares equal before and after | none | verified |
| `ZOOM-009` | `FAIL` | A zoom to an empty target MUST be ignored, never applied as a degenerate factor. | this spec section 3 | `ambre_core::zoom::tests::a_box_is_framed_whole_and_an_empty_one_is_ignored` -- a box with no width or no height, or one not finite, gives no frame; `ambre_toile::ui::view::view_tests::a_zoom_to_the_selection_frames_it_and_keeps_the_selection` -- a box of no area moves nothing, and an empty selection publishes a notice and asks nothing | none | verified |

* **Budget.** none of its own: a zoom recomputes one view transform and repaints, which is the frame
  cost the application's viewport already carries in `ambre-spec-budgets.md`. This document adds no
  operation of its own.
* **Boundary.** none: a zoom is local view state. It writes no document, so nothing here reaches the
  relay, holds a key or opens a socket. `ambre-spec-zero-knowledge.md` seals every payload that does
  leave the client.
* **Verification.** 11 `verified`. The three editors share the engine, the ladder, the clamp, the
  keys and the chip. They also share the wheel under `Ctrl` and the zoom about a point. The zoom to
  a selection is shared by the board and the grid. Section 5 names what is left.

  Writing the rules before the code is what makes the three added ones cheap. `ZOOM-008`, because
  reaching no collaborator is not the same rule as emitting no delta once presence exists.
  `ZOOM-009`, because an empty target has a failure of its own. `ZOOM-010`, the frame condition that
  keeps the gesture repeatable.

---

## 2. MODEL

**Containers.** none. The factor and its anchor are view state on the UI thread; nothing about a
zoom is persisted or shared.

**The value.** One factor, screen pixels per content unit, clamped away from zero. Each application
maps it onto its own space -- a grid cell, a world unit, a page point.

```rust
/// An application's clamp: the floor and the ceiling of its factor.
pub struct ZoomRange<S: ScalesTo<View>> { floor: Scale<S, View>, ceiling: Scale<S, View> }

/// The shared engine: one factor, clamped to the application's range.
/// `S` is the space the application zooms: `Doc` for a grid or a page,
/// `Board` for a board.
pub struct Zoom<S: ScalesTo<View>> { factor: Scale<S, View>, range: ZoomRange<S> }
```

**The range.** Each application supplies its own clamp, and the ranges need not match. Trame zooms
from 50 % to 200 %, Prose from 25 % to 400 %, and Toile from 10 % to 1000 %. The range is a property
of the space, not of the model (section 4).

**The preset ladder.** One ordered set of thirteen stops, from 10 % up to 1000 %. It spans Toile's
range, the widest. Its stops are dense near 100 %: 50 %, 75 %, 100 %, 125 %, 150 %, 200 %.

Every application shares it, and so does the chip that lists them (`ambre-spec-chrome.md`,
`CHROME-014`). The chip lists only the stops inside the application's range. No application holds
a ladder of its own (`ZOOM-006`).

A step moves to the next stop inside the range. A step past either end keeps the factor.

**A zoom, in steps.** The same four steps everywhere; only the mapping in step 3 is the
application's own.

1. a command or a gesture asks to zoom in, out, to a preset, or to a selection;
2. the factor is multiplied and clamped to the range (`ZOOM-002`);
3. the anchor is recomputed so the point under the pointer stays fixed, in the application's own
   mapping (`ZOOM-003`);
4. the view repaints and the chip reads the new factor (`ambre-spec-chrome.md`, `CHROME-014`).

**Code.** `crates/ambre-core/src/zoom.rs` holds the engine, the range and the ladder, next to
`Scale`. `ambre_commands::shared::zoom_step` reads the three chords for every application. Step 3
still lives in each application's viewport (section 5).

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| `Ctrl` with `=` or `-` | zoom in or out by one preset step (`ZOOM-004`) |
| `Ctrl` with `0` | return to a hundred per cent (`ZOOM-005`) |
| `Ctrl` with the wheel, a trackpad pinch | zoom about the pointer (`ZOOM-003`, `ZOOM-004`) |
| a click on the zoom chip | the shared preset ladder opens, the current factor marked (`ambre-spec-chrome.md`) |
| a zoom-to-selection command | frame the selection clear of the floating surfaces and clamp; an empty target does nothing (`ZOOM-007`, `ZOOM-011`) |
| the zoom entries in a menu | the same intents, through a command (`ambre-spec-commands.md`) |

The user reads the factor from one chip, and the keys, the wheel and the pinch behave the same in
every editor.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One zoom engine behind the shared chip | a zoom read-out and a preset ladder per application | two engines behind one chip drift in their preset ladder and their clamp, so the read-out means a different thing per application while looking the same |
| Zoom is local view state, never a document register | a zoom register on the document, shared like content | a factor in the document makes two users fight over one camera, and it converges nothing: a zoom is a way of looking, and it is nobody else's business |
| The factor is one number, mapped per application | a shared coordinate space every application projects into | a grid cell, a world unit and a page point are three spaces, and forcing one space on them buys a projection nobody needs: the factor over them is already one and the fixed-point rule is one. Sharing the number and mapping it locally is what lets the chip stay one component |
| Each application keeps its own clamp | one clamp for every application | it either caps the board or lets a page zoom past use: a board zooms from a bird's eye to a detail and a page from a quarter to four times, so the range is a property of the space, not of the model |
| Three per-application viewports stand beside this one | one viewport document for the suite | the model is one and the mapping is not: culling a grid, an endless board and a column of pages are three designs with three sets of tests, and collapsing them here would report three states as one |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A two-finger touch pinch is not built.** A trackpad pinch arrives as a wheel with `Ctrl`, and
  zooms in every editor. A touch screen raises pointer events with no pointer identity, so no editor
  can tell two fingers apart (`ambre-spec-ui.md`, `UI-THREAD-013`).
* **Prose frames no selection.** No command of the page asks for one, so `ZOOM-007` holds on the
  board and the grid alone. A key and the chip keep the top of the canvas in place; the anchor on
  the caret is not ruled (`4-Prose/features/ambre-spec-prose-viewport.md`).
* **The grid frames with no strip.** No floating surface sits over the cells of a grid: its bars are
  docked above it. `ZOOM-011` holds there with nothing to keep clear, and no test of the grid reads
  it.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The status-bar chip that shows the factor, and its preset menu | `ambre-spec-chrome.md`, `CHROME-014` |
| How a grid culls and maps under zoom | `2-Trame/features/ambre-spec-trame-grid-viewport.md` |
| How an endless board culls and maps under zoom | `3-Toile/features/ambre-spec-toile-viewport.md` |
| How a page column centres and maps under zoom | `4-Prose/features/ambre-spec-prose-viewport.md` |
| Which chords mean zoom in, out and reset | `ambre-spec-commands.md` |
