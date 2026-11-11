# AMBRE -- FEATURE: THE BOARD VIEWPORT

**Type:** Vertical (feature)

> A user pans and zooms an endless plane. The board scales under the cursor, and the grid keeps a
> readable spacing at every scale. The wheel behaves like the device in the user's hand.

The camera is local view state: it writes nothing to the document and reaches no collaborator. There
is no two-peer table below, because two users looking at one board see nothing of each other's
camera.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-002` | `INV` | A screen point and a world point MUST map to each other exactly, in both directions. | this spec section 2 | `ambre_toile::ui::viewport::tests::round_trip_is_identity` -- it walks three points out and back; `ambre_toile::ui::viewport::tests::center_maps_to_screen_center` -- it pins the one point the mapping is built around | none | verified |
| `TOILE-UI-003` | `INV` | The zoom MUST stay inside 0.1x..10x, whatever the input asks for. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_clamps_to_range` -- it drives a hundred doublings and two hundred halvings into both stops; `ambre_toile::ui::viewport::tests::fit_bounds_clamps_tiny_rect_to_max_zoom` -- it holds the stop a fit would blow past | none | verified |
| `TOILE-UI-278` | `INV` | The camera MUST reach no collaborator: what a user looks at MUST NOT enter the document. | this spec section 1; `ambre-spec-zero-knowledge.md section 2` | structural -- the Logic handler for a camera assigns one field and returns a display-only outcome, so a camera produces no delta to broadcast and no write to persist | none | verified |
| `TOILE-UI-010` | `OBS` | The grid spacing MUST stay legible across the whole zoom range. | this spec section 3 | `ambre_toile::ui::grid::tests::spacing_never_illegible_across_zoom_sweep` -- it sweeps the zoom over three decades and reads the floor at each | none | verified |
| `TOILE-UI-279` | `OBS` | The grid step MUST stay a multiple of the board unit, never finer than it. | this spec section 3; section 5 | `ambre_toile::ui::grid::tests::step_is_power_of_two_multiple_of_base` -- it reads both at seven zooms; `ambre_toile::ui::grid::tests::a_board_paints_its_grid_on_its_own_unit` -- the painted lines on units of 20, 32 and 50, and an unusable unit falling back; `ambre_toile::ui::grid::tests::a_grid_unit_is_finite_and_positive` -- a zero, negative or non-finite unit is refused, and the step search ends on the smallest positive one; `ambre_toile::logic::state::derive::tests::a_frame_carries_the_boards_grid_unit` -- the register reaches the frame | none | verified |
| `TOILE-UI-280` | `OBS` | A grid line MUST stay one pixel wide at every zoom. | this spec section 3; `features/ambre-spec-toile-board-rendering.md section 3` | `ambre_toile::ui::grid::tests::a_grid_line_is_one_pixel_at_every_zoom` -- five zooms, and no transform pushed | none | verified |
| `TOILE-UI-281` | `INV` | The rectangle the UI hands over for culling MUST carry a zoom the Logic thread can divide by. | this spec section 2 | `ambre_toile::message::tests::any_zoom_culls_a_finite_rect` | none | verified |
| `TOILE-UI-025` | `INV` | While the board asks for snapping, every committed geometry MUST sit on a multiple of the grid step. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Snap to the board grid` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-UI-030` | `INV` | The minimap MUST NOT write to the document: dragging its rectangle moves the camera alone. | this spec section 5 | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TOILE-UI-421` | `INV` | Snapping MUST move a world point to the nearest multiple of the grid step on each axis. | this spec section 3; `TOILE-UI-025` | `ambre_toile::ui::grid::tests::snap_quantizes_to_step` -- on a step of 20, (23, -31) lands on (20, -40) and (31, 9) on (40, 0) | none | verified |

### 1.2 Drag the board around

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-026` | `POST` | A pan MUST move the content by the pointer's own displacement, at every zoom. | this spec section 2 | `ambre_toile::ui::viewport::tests::pan_moves_content_with_the_drag` -- it reads the screen displacement of a painted world point equal to the drag, at 2x, the one zoom that exercises the division at all | none | verified |
| `TOILE-UI-282` | `FRAME` | A pan MUST NOT change the zoom, and MUST NOT change what is selected. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_pan_keeps_the_zoom_and_the_selection_and_asks_where_it_landed` -- the zoom and the selection read the same after a wheel pan | none | verified |
| `TOILE-UI-378` | `POST` | A finger pressed on the empty board with the select tool, or with the hand, MUST pan the board. | this spec section 3 | `ambre_toile::ui::kinetic::tests::a_finger_pans_the_empty_board_and_grabs_or_draws_otherwise` -- the choice; `ambre_toile::ui::view::view_tests::a_finger_pans_only_the_empty_board` -- the view's hit test on the empty board, and the camera moving under the finger | none | verified |
| `TOILE-UI-379` | `FRAME` | A finger pressed on an element, on a handle or with a drawing tool MUST NOT pan the board. | this spec section 3 | `ambre_toile::ui::kinetic::tests::a_finger_pans_the_empty_board_and_grabs_or_draws_otherwise` -- the choice; `ambre_toile::ui::view::view_tests::a_finger_pans_only_the_empty_board` -- an element, the rotate knob and the pen, through the view's hit test | none | verified |
| `TOILE-UI-380` | `POST` | A gesture held past an edge of the canvas MUST pan the board towards it, the same screen distance at every zoom. | this spec section 3 | `ambre_toile::ui::kinetic::tests::a_step_moves_the_same_screen_distance_at_any_zoom` -- the speed and the step; `ambre_toile::ui::view::view_tests::a_drag_held_at_an_edge_pans_the_board_each_frame` -- the view pans once a frame, by the same screen step at 1x and 2x | none | verified |
| `TOILE-UI-283` | `POST` | A pan MUST ask for the visible set of the place it landed on. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_pan_keeps_the_zoom_and_the_selection_and_asks_where_it_landed` -- one request after a wheel pan, for the camera the pan left; `ambre_toile::ui::view::view_tests::a_drag_held_at_an_edge_pans_the_board_each_frame` -- a request after each edge step | none | verified |

### 1.3 Zoom in and out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-284` | `POST` | A zoom about a point MUST leave the world point under that point fixed on screen. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_about_keeps_anchor_world_point_fixed` -- it maps the anchor to world before and after and reads it unmoved | none | verified |
| `TOILE-UI-285` | `POST` | A zoom that hits a stop MUST still hold its anchor, rather than drift while the zoom refuses to change. | this spec section 2 | `ambre_toile::ui::viewport::tests::a_zoom_at_a_stop_holds_its_anchor` -- both stops, the anchor mapped back | none | verified |
| `TOILE-UI-286` | `POST` | A zoom the user asks for without a cursor MUST scale the board in place. | this spec section 2; `features/ambre-spec-toile-menus.md section 3` | `ambre_toile::ui::viewport::tests::zoom_by_center_keeps_center_world_point_fixed` -- it maps the centre before and after the button's zoom and reads it unmoved | none | verified |
| `TOILE-UI-360` | `POST` | Returning to 100 % MUST keep the view where it is. | this spec section 2; `features/ambre-spec-toile-menus.md section 3` | `ambre_toile::ui::viewport::tests::reset_zoom_returns_to_100_percent` -- it reads the zoom and the centre the chip leaves behind | none | verified |
| `TOILE-UI-287` | `FRAME` | A zoom MUST NOT move an element: it changes what the user looks at and nothing they own. | this spec section 2; `TOILE-UI-278` | type-level -- the camera owns three fields and no document handle, so a zoom has nothing else to write | none | verified |

### 1.4 Turn the wheel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-027` | `POST` | An automatic pointer mode MUST classify each wheel event from that event's own signature. | this spec section 3 | `ambre_toile::ui::view::wheel_tests::line_or_page_delta_mode_is_a_mouse` -- the strongest signal, a line or page unit; `ambre_toile::ui::view::wheel_tests::horizontal_component_is_a_trackpad` -- the second, a two-axis glide; `ambre_toile::ui::view::wheel_tests::large_vertical_notch_is_a_mouse` -- the third, a discrete notch | none | verified |
| `TOILE-UI-288` | `POST` | An ambiguous wheel event MUST leave the latched guess unchanged, so one gesture does not alternate between panning and zooming. | this spec section 3; section 4 | `ambre_toile::ui::view::wheel_tests::fine_vertical_glide_is_ambiguous` -- it holds the pure classifier returning nothing; `ambre_toile::ui::view::view_tests::an_ambiguous_wheel_keeps_the_latched_guess` -- after a mouse latch and after a trackpad latch | none | verified |
| `TOILE-UI-028` | `POST` | `Ctrl` with the wheel, and a trackpad pinch, MUST zoom in every pointer mode. | this spec section 3 | `ambre_toile::ui::view::view_tests::ctrl_and_the_pinch_zoom_in_every_mode` -- the three modes | none | verified |
| `TOILE-UI-029` | `FRAME` | A wheel event MUST NOT touch a pointer mode the user picked: their choice freezes the detection. | this spec section 3; section 4 | `ambre_toile::ui::view::view_tests::a_picked_mode_ignores_a_contrary_wheel` -- a line wheel in the trackpad mode, a glide in the mouse mode, and the latch untouched | none | verified |
| `TOILE-UI-289` | `POST` | The wheel in the trackpad mode MUST pan both axes, moving the content against the delta as a page scrolls and as Miro does. | this spec section 3 | `ambre_toile::ui::kinetic::tests::the_wheel_moves_the_content_as_a_page_scrolls` -- the pan; `ambre_toile::ui::view::view_tests::the_trackpad_mode_pans_both_axes_against_the_delta` -- through the handler, in the forced mode | none | verified |

### 1.5 Fit the board or the selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-004` | `POST` | A fit MUST centre the target and scale it to the tighter axis, with padding, so the content never touches the edge. | this spec section 2 | `ambre_toile::ui::viewport::tests::fit_bounds_centers_and_scales_with_padding` -- it reads the centre, the governing axis and the padded fit on both axes; `ambre_toile::ui::viewport::tests::fit_target_matches_fit_bounds` -- it holds the pure solver to the same answer | none | verified |
| `TOILE-UI-290` | `FAIL` | A fit over a rectangle with no area MUST leave the camera exactly as it was, rather than invent one. | this spec section 2; section 4 | `ambre_toile::ui::viewport::tests::fit_bounds_ignores_empty_rect` -- it compares the whole camera before and after; `ambre_toile::ui::viewport::tests::fit_target_matches_fit_bounds` -- it reads the solver answering nothing | none | verified |
| `TOILE-UI-291` | `FAIL` | A fit over a rectangle whose extent is not finite MUST refuse the same way. | this spec section 2 | `ambre_toile::ui::viewport::tests::a_fit_over_an_extent_that_is_not_finite_is_refused` -- an infinite and a NaN extent on each axis | none | verified |
| `TOILE-UI-292` | `FRAME` | A fit MUST NOT change the canvas it fits into. | this spec section 2 | structural -- the fit assigns the centre and the zoom and reads the size, so the rectangle it solves against cannot be the one it moves | none | verified |
| `TOILE-UI-293` | `POST` | A fit over the whole board MUST frame the extent it asked for, and not the answer to somebody else's question. | this spec section 2; section 5 | `ambre_toile::logic::convergence_tests::selection_stats_echoes_the_request_kind` -- the fit tags its request, and the view frames only the reply that echoes that tag | none | verified |

### 1.6 Glide to a slide

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-294` | `POST` | A camera glide MUST reproduce where it started at the start, land exactly on its target at the end, and interpolate both the centre and the zoom between. | this spec section 2; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::viewport::tests::glide_interpolates_center_and_zoom` -- it reads the two endpoints exactly and the linear midpoint of each field | none | verified |
| `TOILE-UI-295` | `FRAME` | A glide MUST NOT change the canvas: it moves the camera through it. | this spec section 2 | `ambre_toile::ui::viewport::tests::glide_interpolates_center_and_zoom` -- it reads the size carried through unchanged; type-level -- the glide takes a camera and returns a new one rather than mutating it | none | verified |

### 1.7 Resize the canvas

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-296` | `POST` | A canvas resize MUST ask for the visible set of the new canvas, so an element the new edge uncovers arrives. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_resize_keeps_the_camera_and_asks_for_the_new_canvas` -- one request, at the new size | none | verified |
| `TOILE-UI-297` | `FRAME` | A resize MUST NOT move the camera: the same world point stays under the screen centre, at the same scale. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_resize_keeps_the_camera_and_asks_for_the_new_canvas` -- the centre and the zoom read the same | none | verified |
| `TOILE-UI-298` | `FAIL` | A canvas that reports no extent MUST be treated as the smallest one, never as a camera that cannot be inverted. | this spec section 2; `TOILE-UI-002` | `ambre_toile::ui::view::view_tests::a_resize_keeps_the_camera_and_asks_for_the_new_canvas` -- a zero resize and a negative size are the one-pixel canvas, and the camera inverts | none | verified |

* **Budget.** the pan and zoom latency under 15 ms, a row of `ambre-spec-budgets.md section 2`. It
  is held by construction rather than by a bench: the camera is applied locally and the cull request
  that follows it is asynchronous (`TOILE-UI-020`).
* **Boundary.** none: the camera is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses to the Logic thread is a rectangle to cull
  against, never document data (`TOILE-UI-001`).
* **Verification.** 34 `verified`, 2 `unimplemented`. The pure functions of the camera hold the
  geometry. View tests drive the wheel in each mode, the finger, the edge pan and the resize, then
  read the cull request each handler queues. The grid is painted into a recording surface on the
  board's own unit. The two `unimplemented` rows are snapping and the minimap.

---

## 2. MODEL

**Containers.** None. The camera persists nothing. The board's grid unit and its snapping flag are
declared in `ambre-spec-toile-schema.md section 3`, under the `board` register.

**The camera** (`ambre-toile`):

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportTransform {
    pub center: Point,              // the world point under the screen centre
    pub zoom:   Scale<Board, View>, // screen pixels per world unit, clamped 0.1 ..= 10.0
    pub size:   Size,               // the canvas, in physical pixels
}
```

`screen = translate(size / 2) * scale(zoom) * translate(-center)`, applied to the world point. The
inverse always exists, because the zoom is clamped away from zero (`TOILE-UI-003`) and the canvas
never reports an empty extent (`TOILE-UI-298`).

**Two types for one rectangle:** `ViewportTransform` is the render thread's camera; `Viewport` (`ambre-bus`)
is the Logic thread's culling input. They carry the same three fields and are deliberately distinct. The
camera is applied locally and immediately, and a copy of it is forwarded as a cull request the UI never waits
on. Logic never pushes a camera back. Both sides take the zoom range from `ambre-bus`. The culling type's
fields are public, so its one division reads the zoom through a clamp, and a NaN reads as 1
(`TOILE-UI-281`).

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::SetViewport(Viewport)` | UI to Logic | the rectangle to cull against. Coalesced to one per frame (`TOILE-UI-020`) |
| `ToileEvent::SelectionStats { count, bounds }` | Logic to UI | the union rectangle a fit frames. An empty id list asks for the whole board |

**Steps, on a zoom about a point:**

1. map the screen anchor back to world through the current inverse;
2. multiply the zoom and clamp it to the supported range (`TOILE-UI-003`);
3. solve the centre that puts the anchored world point back under the anchor, at the clamped zoom.

**Steps, on a fit:**

1. refuse a rectangle with a zero or non-finite extent, leaving the camera untouched;
2. grow the target extent by the padding fraction, so the content never touches the edge;
3. take the smaller of the two axis fits, clamped, and centre on the rectangle.

A fit over the whole board asks the Logic thread for the extent, because the UI holds the culled set alone.
The reply is the same aggregate the status bar consumes (`features/ambre-spec-toile-status-bar.md`). The
request carries a kind token the reply echoes, so a pending fit spends only its own answer
(`TOILE-UI-293`).

**Code:**

* `crates/ambre-toile/src/ui/viewport.rs` -- the camera, the zoom, the fit and the glide;
* `ui/grid.rs` -- the adaptive step, the quantizer and the grid paint;
* `ui/view.rs` -- the wheel handler, the device classifier, the pan drag and the zoom entries.

---

## 3. SURFACE

**Triggers.**

| Trigger | Behaviour |
|---|---|
| the wheel, in the mouse mode | zooms about the cursor |
| the wheel, in the trackpad mode | pans both axes, the content moving against the delta as a page does (`TOILE-UI-289`), and a glide coasts once it pauses (`1-Foundations/ambre-spec-kinetic-scroll.md`); a pinch zooms |
| the wheel, in the automatic mode | classified per event, then zooms or pans (`TOILE-UI-027`) |
| `Ctrl` with the wheel, a pinch | zooms, in every mode (`TOILE-UI-028`) |
| a drag with the Hand tool, with `Space` held, or with the middle button | pans (`TOILE-UI-026`) |
| a finger on the empty board with the select tool, or with the hand | pans, and a flick keeps the board moving (`TOILE-UI-378`); a finger that barely moves is a click |
| a finger on an element or a handle, or with a drawing tool | acts as the mouse: grabs, draws or places (`TOILE-UI-379`) |
| a move, a resize, a marquee or a stroke held past the edge of the canvas | pans towards that edge, faster the deeper, and the gesture follows (`TOILE-UI-380`) |
| the zoom control and the View menu | zoom in, zoom out, 100 %, fit the board, fit the selection (`features/ambre-spec-toile-menus.md`) |
| a canvas resize | keeps the centre and the zoom, and re-culls (`TOILE-UI-296`, `TOILE-UI-297`) |

**How a wheel event is classified,** strongest signal first: a line or page unit is a classic wheel, so a
mouse. A non-zero horizontal component is a two-axis glide, so a trackpad. A large vertical delta with no
horizontal one is a discrete notch, so a mouse. A fine ambiguous glide leaves the latched guess alone, and
that latch starts on the trackpad. That is the gentler wrong default: a mouse user's wheel then merely pans,
where a trackpad user's two-finger scroll would zoom abruptly.

**The adaptive grid.** The step is the smallest multiple of the board unit whose on-screen spacing is
comfortable. It doubles while the board is zoomed out and halves back while it is zoomed in, never below the
board unit (`TOILE-UI-010`). The unit is the `board` register's, carried on each frame; a unit that
is not finite and positive falls back to 20 (`TOILE-UI-279`). The grid paints under the content walk,
in screen space, so a line stays one pixel wide at every zoom
(`features/ambre-spec-toile-board-rendering.md`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The camera is applied locally and the cull request follows it | routing the pan and the zoom through the Logic thread | a board is a plane the user drags directly, so the round trip lands inside every gesture, and the content already on screen only needs a different transform to be correct. A virtual-scroll grid routes it because its content is fetched by index; a board's is not |
| The render camera and the culling rectangle are two types | one shared `Viewport` type, which the earlier design carried | the two carry the same fields and have opposite lifetimes: one is written every pointer move, the other is read once per cull. One type makes it possible to assign the stale culling copy back onto the live camera |
| The wheel has no single default | picking one behaviour and leaving the other device to `Ctrl` | the same event means "zoom" on a mouse and "pan" on a trackpad, and the browser reports both the same way, so a wrong fixed default makes half the users fight the board on every scroll |
| An ambiguous glide leaves the guess unchanged | classifying every event, with no latch | the classifier runs per event, so flipping on a weak signal makes the board alternate between panning and zooming inside one gesture |
| The grid step is a multiple of the board unit | a step derived only from the pixel spacing | the grid is magnetic, so a step that is not a multiple of the unit snaps content off the unit it was authored on |
| A fit refuses an empty rectangle | clamping the extent to a minimum | it invents a rectangle the caller did not ask for. Fitting a zero-area rectangle is a division by zero, and its two plausible answers -- maximum zoom, or no move -- differ, so refusing states which one |
| The minimap stays a camera, not a document property | a synchronized overview, as a board register | the minimap is a way of looking and two users navigate independently, so sharing the rectangle makes them fight over one camera |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Snapping is a function nothing calls** (`TOILE-UI-025`). The quantizer and the step selection are written
  and host-tested, and the board carries a snapping flag written once at creation. No gesture reads either:
  the quantizer has no caller in the workspace, and a drag emits the raw pointer position. So the grid is
  drawn and is not magnetic.
* **The minimap** (`TOILE-UI-030`): the design is a second small camera painted last, fed by the union of the
  board's content. That extent is the one the Logic thread already answers for a whole-board fit, and a drag
  on its rectangle moves the main camera. No field carries it and nothing paints it.
* **A pointer mode does not survive a reload.** The preference is held in memory on the view, and nothing
  writes it to local storage. A user who forces a mode forces it again on the next visit.
