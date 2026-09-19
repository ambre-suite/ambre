# AMBRE -- FEATURE: THE BOARD VIEWPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the camera, the wheel and the grid ship; snapping and the minimap do not.

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
| `TOILE-UI-279` | `OBS` | The grid step MUST stay a multiple of the board unit, never finer than it. | this spec section 3; section 5 | `ambre_toile::ui::grid::tests::step_is_power_of_two_multiple_of_base` -- it reads both at seven zooms | partial -- the unit it holds is the render module's constant, and the `board` register the schema declares is never read, so a board authored on another unit paints on the default one (section 5); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-280` | `OBS` | A grid line MUST stay one pixel wide at every zoom. | this spec section 3; `features/ambre-spec-toile-board-rendering.md section 3` | none | unheld -- the width is a literal passed to a screen-space stroke that no test observes, in a paint helper nothing exercises; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-281` | `INV` | The rectangle the UI hands over for culling MUST carry a zoom the Logic thread can divide by. | this spec section 2; section 5 | none | fails -- the clamp is a private function of the camera, the culling type is three public fields with a default, and its one method divides by the zoom without a guard. Nothing on the Logic side clamps, though a comment says it does; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |
| `TOILE-UI-025` | `INV` | While the board asks for snapping, every committed geometry MUST sit on a multiple of the grid step. | this spec section 3; section 5 | not built | unbuilt -- lands with the `Snap to the board grid` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-UI-030` | `INV` | The minimap MUST NOT write to the document: dragging its rectangle moves the camera alone. | this spec section 5 | not built | unbuilt -- lands with the `The facilitation layer` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Drag the board around

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-026` | `POST` | A pan MUST move the content by the pointer's own displacement, at every zoom. | this spec section 2 | `ambre_toile::ui::viewport::tests::pan_moves_content_with_the_drag` -- it reads the screen displacement of a painted world point equal to the drag, at 2x, the one zoom that exercises the division at all | none | verified |
| `TOILE-UI-282` | `FRAME` | A pan MUST NOT change the zoom, and MUST NOT change what is selected. | this spec section 2 | none | unheld -- the pan touches one field of the camera and the pointer handler returns before the tool machine sees the move, but no test reads the zoom back after a pan; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-283` | `POST` | A pan MUST ask for the visible set of the place it landed on. | this spec section 2 | none | unheld -- the drag path queues one cull request per move and the wheel path queues another, and no test drives either handler, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Zoom in and out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-284` | `POST` | A zoom about a point MUST leave the world point under that point fixed on screen. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_about_keeps_anchor_world_point_fixed` -- it maps the anchor to world before and after and reads it unmoved | none | verified |
| `TOILE-UI-285` | `POST` | A zoom that hits a stop MUST still hold its anchor, rather than drift while the zoom refuses to change. | this spec section 2 | none | unheld -- `ambre_toile::ui::viewport::tests::zoom_clamps_to_range` drives three hundred events into both stops and reads the zoom alone, and the anchor it passes is never mapped back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-286` | `POST` | A zoom the user asks for without a cursor MUST scale the board in place. | this spec section 2; `features/ambre-spec-toile-menus.md section 3` | `ambre_toile::ui::viewport::tests::zoom_by_center_keeps_center_world_point_fixed` -- it maps the centre before and after the button's zoom and reads it unmoved | none | verified |
| `TOILE-UI-360` | `POST` | Returning to 100 % MUST keep the view where it is. | this spec section 2; `features/ambre-spec-toile-menus.md section 3` | `ambre_toile::ui::viewport::tests::reset_zoom_returns_to_100_percent` -- it reads the zoom and the centre the chip leaves behind | none | verified |
| `TOILE-UI-287` | `FRAME` | A zoom MUST NOT move an element: it changes what the user looks at and nothing they own. | this spec section 2; `TOILE-UI-278` | type-level -- the camera owns three fields and no document handle, so a zoom has nothing else to write | none | verified |

### 1.4 Turn the wheel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-027` | `POST` | An automatic pointer mode MUST classify each wheel event from that event's own signature. | this spec section 3 | `ambre_toile::ui::view::wheel_tests::line_or_page_delta_mode_is_a_mouse` -- the strongest signal, a line or page unit; `ambre_toile::ui::view::wheel_tests::horizontal_component_is_a_trackpad` -- the second, a two-axis glide; `ambre_toile::ui::view::wheel_tests::large_vertical_notch_is_a_mouse` -- the third, a discrete notch | none | verified |
| `TOILE-UI-288` | `POST` | An ambiguous wheel event MUST leave the latched guess unchanged, so one gesture does not alternate between panning and zooming. | this spec section 3; section 4 | `ambre_toile::ui::view::wheel_tests::fine_vertical_glide_is_ambiguous` -- it holds the pure classifier returning nothing | partial -- the latch is a field the handler writes, and no test sets it, sends an ambiguous event and reads it back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-028` | `POST` | `Ctrl` with the wheel, and a trackpad pinch, MUST zoom in every pointer mode. | this spec section 3 | none | unheld -- the rule is three arms of one match and no test drives the handler at all, only the pure classifier under it; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-029` | `FRAME` | A wheel event MUST NOT touch a pointer mode the user picked: their choice freezes the detection. | this spec section 3; section 4 | none | unheld -- the classifier has one caller, the automatic arm, so the rule holds by construction today, but nothing guards that fact and no test forces a mode and sends a contrary event; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-289` | `POST` | The wheel in the trackpad mode MUST pan both axes, so a two-finger glide moves the plane diagonally. | this spec section 3 | none | unheld -- the handler pans by the raw event deltas and nothing reads the camera back, and which way a scroll moves the board is not ruled anywhere, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Fit the board or the selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-004` | `POST` | A fit MUST centre the target and scale it to the tighter axis, with padding, so the content never touches the edge. | this spec section 2 | `ambre_toile::ui::viewport::tests::fit_bounds_centers_and_scales_with_padding` -- it reads the centre, the governing axis and the padded fit on both axes; `ambre_toile::ui::viewport::tests::fit_target_matches_fit_bounds` -- it holds the pure solver to the same answer | none | verified |
| `TOILE-UI-290` | `FAIL` | A fit over a rectangle with no area MUST leave the camera exactly as it was, rather than invent one. | this spec section 2; section 4 | `ambre_toile::ui::viewport::tests::fit_bounds_ignores_empty_rect` -- it compares the whole camera before and after; `ambre_toile::ui::viewport::tests::fit_target_matches_fit_bounds` -- it reads the solver answering nothing | none | verified |
| `TOILE-UI-291` | `FAIL` | A fit over a rectangle whose extent is not finite MUST refuse the same way. | this spec section 2 | none | unheld -- the guard tests both extents for finiteness and no test passes an infinite or undefined one, so the half of the refusal that protects against a computed rectangle is unheld; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
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
| `TOILE-UI-296` | `POST` | A canvas resize MUST ask for the visible set of the new canvas, so an element the new edge uncovers arrives. | this spec section 3 | none | unheld -- the resize entry is three statements and no test calls it, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-297` | `FRAME` | A resize MUST NOT move the camera: the same world point stays under the screen centre, at the same scale. | this spec section 3 | none | unheld -- the entry writes the size alone, and nothing reads the centre or the zoom back across a resize; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-298` | `FAIL` | A canvas that reports no extent MUST be treated as the smallest one, never as a camera that cannot be inverted. | this spec section 2; `TOILE-UI-002` | none | unheld -- the floor is a call on each axis and no test passes a zero or a negative, which is the one input that would make the mapping of `TOILE-UI-002` undefined; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the pan and zoom latency under 15 ms, a row of `ambre-spec-budgets.md section 2`. It
  is held by construction rather than by a bench: the camera is applied locally and the cull request
  that follows it is asynchronous (`TOILE-UI-020`).
* **Boundary.** none: the camera is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses to the Logic thread is a rectangle to cull
  against, never document data (`TOILE-UI-001`).
* **Verification.** 16 `verified`, 1 `failing`, 13 `untested`, 2 `unimplemented`. What holds is the geometry:
  the pure functions of the camera, covered over eighteen tests, and the kind token that names a fit's
  reply (`TOILE-UI-293`). What is loose is the handler layer above them: the wheel, the drag, the fit
  round trip and the resize. Loose too are every frame condition, the anchor at a clamp stop and a
  refused non-finite extent.

---

## 2. MODEL

**Containers.** None. The camera persists nothing. The board's grid unit and its snapping flag are
declared in `ambre-spec-toile-schema.md section 3`, under the `board` register.

**The camera** (`ambre-toile`):

```rust
pub struct ViewportTransform {
    pub center: Point,   // the world point under the screen centre
    pub zoom:   f64,     // screen pixels per world unit, clamped 0.1 ..= 10.0
    pub size:   Size,    // the canvas, in physical pixels
}
```

`screen = translate(size / 2) * scale(zoom) * translate(-center)`, applied to the world point. The
inverse always exists, because the zoom is clamped away from zero (`TOILE-UI-003`) and the canvas
never reports an empty extent (`TOILE-UI-298`).

**Two types for one rectangle:** `ViewportTransform` is the render thread's camera; `Viewport` (`ambre-bus`)
is the Logic thread's culling input. They carry the same three fields and are deliberately distinct. The
camera is applied locally and immediately, and a copy of it is forwarded as a cull request the UI never waits
on. Logic never pushes a camera back. The clamp lives on one side of that pair only. The culling type is three
public fields, and the one thing the Logic thread does with it divides by the zoom (`TOILE-UI-281`).

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
| the wheel, in the trackpad mode | pans both axes (`TOILE-UI-289`); a pinch zooms |
| the wheel, in the automatic mode | classified per event, then zooms or pans (`TOILE-UI-027`) |
| `Ctrl` with the wheel, a pinch | zooms, in every mode (`TOILE-UI-028`) |
| a drag with the Hand tool, with `Space` held, or with the middle button | pans (`TOILE-UI-026`) |
| the zoom control and the View menu | zoom in, zoom out, 100 %, fit the board, fit the selection (`features/ambre-spec-toile-menus.md`) |
| a canvas resize | keeps the centre and the zoom, and re-culls (`TOILE-UI-296`, `TOILE-UI-297`) |

**How a wheel event is classified,** strongest signal first: a line or page unit is a classic wheel, so a
mouse. A non-zero horizontal component is a two-axis glide, so a trackpad. A large vertical delta with no
horizontal one is a discrete notch, so a mouse. A fine ambiguous glide leaves the latched guess alone, and
that latch starts on the trackpad. That is the gentler wrong default: a mouse user's wheel then merely pans,
where a trackpad user's two-finger scroll would zoom abruptly.

**The adaptive grid.** The step is the smallest multiple of the board unit whose on-screen spacing is
comfortable. It doubles while the board is zoomed out and halves back while it is zoomed in, never below the
board unit (`TOILE-UI-010`). The grid paints under the content walk, in screen space, so a line stays one
pixel wide at every zoom (`features/ambre-spec-toile-board-rendering.md`).

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
* **The board's grid unit is not read.** The step is derived from a constant in the render module, not from
  the `board` register the schema declares. A board authored on a different unit snaps and paints on the
  default one.
* **The minimap** (`TOILE-UI-030`): the design is a second small camera painted last, fed by the union of the
  board's content. That extent is the one the Logic thread already answers for a whole-board fit, and a drag
  on its rectangle moves the main camera. No field carries it and nothing paints it.
* **No handler above the pure geometry has a test.** This crate's rule is that an interaction lands with a
  geometry round trip and a recording of what it painted (`ambre-spec-testing.md section 6`). The wheel, the
  pan drag, the zoom entries and the resize all read or write the camera, and none is driven. Three go unheld
  together: the latch a wheel event must not disturb (`TOILE-UI-288`, `TOILE-UI-029`) and the direction a
  trackpad glide pans (`TOILE-UI-289`). The third is the cull request each of them queues (`TOILE-UI-283`,
  `TOILE-UI-296`).
* **The clamp is claimed on both sides and written on one** (`TOILE-UI-281`). The camera clamps its
  zoom and a comment says the culling type matches; it does not. That type is a public struct with a
  default and one method, and the method divides by the zoom. Today the only producer in the app is
  the camera, so nothing is wrong -- and nothing keeps it that way.
* **Two refusals are half held.** A fit ignores a zero-area rectangle, which is tested, and a non-finite one,
  which is not (`TOILE-UI-291`). A canvas floors its extent at one pixel, which is the input that would make
  the mapping undefined, and nothing drives it (`TOILE-UI-298`).
* **A pointer mode does not survive a reload.** The preference is held in memory on the view, and nothing
  writes it to local storage. A user who forces a mode forces it again on the next visit.
