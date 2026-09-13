# AMBRE -- FEATURE: THE BOARD VIEWPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the camera, the wheel and the grid ship; snapping and the minimap do not.

> A user pans and zooms an endless plane. The board scales under the cursor, the grid keeps a
> readable spacing at every scale, and the wheel behaves like the device in the user's hand.

The camera is local view state: it writes nothing to the document and reaches no collaborator. There
is no two-peer table below, because two users looking at one board see nothing of each other's
camera.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-002` | INV | A screen point and a world point MUST map to each other exactly, in both directions. | this spec section 2 | `ambre_toile::ui::viewport::tests::round_trip_is_identity` walks three points out and back, and `::center_maps_to_screen_center` pins the one point the mapping is built around | verified |
| `TOILE-UI-003` | INV | The zoom MUST stay inside 0.1x..10x, whatever the input asks for. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_clamps_to_range` drives a hundred doublings and two hundred halvings into both stops, and `::fit_bounds_clamps_tiny_rect_to_max_zoom` holds the stop a fit would blow past | verified |
| `TOILE-UI-278` | INV | The camera MUST reach no collaborator: what a user looks at MUST NOT enter the document. | this spec section 1; `ambre-spec-zero-knowledge.md section 2` | structural, and the construction is the proof: the Logic handler for a camera assigns one field and returns a display-only outcome, so a camera produces no delta to broadcast and no write to persist | verified |
| `TOILE-UI-010` | OBS | The grid spacing MUST stay legible across the whole zoom range. | this spec section 3 | `ambre_toile::ui::grid::tests::spacing_never_illegible_across_zoom_sweep` sweeps the zoom over three decades and asserts the floor at each | verified |
| `TOILE-UI-279` | OBS | The grid step MUST stay a multiple of the board unit, and MUST never go finer than it. | this spec section 3; section 5 | `ambre_toile::ui::grid::tests::step_is_power_of_two_multiple_of_base` asserts both at seven zooms. The unit it holds is the render module's constant, not the board's own (section 5) | verified |
| `TOILE-UI-280` | OBS | A grid line MUST stay one pixel wide at every zoom. | this spec section 3; `features/ambre-spec-toile-board-rendering.md section 3` | none. The width is a literal passed to a screen-space stroke that no test observes, in a paint helper nothing exercises | unverified |
| `TOILE-UI-281` | INV | The rectangle the UI hands over for culling MUST carry a zoom the Logic thread can divide by. | this spec section 2; section 5 | none, and the code says otherwise: the clamp is a private function of the camera, the culling type is three public fields with a default, and its one method divides by the zoom without a guard. Nothing on the Logic side clamps, though a comment says it does | unverified |
| `TOILE-UI-025` | INV | While the board asks for snapping, every committed geometry MUST sit on a multiple of the grid step. | this spec section 3; section 5 | not built -- lands with the "Snap to the board grid" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-UI-030` | INV | The minimap MUST NOT write to the document: dragging its rectangle moves the camera alone. | this spec section 5 | not built -- lands with the "The facilitation layer" Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Drag the board around

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-026` | POST | A pan MUST move the content by the pointer's own displacement, at every zoom. | this spec section 2 | `ambre_toile::ui::viewport::tests::pan_moves_content_with_the_drag` asserts the screen displacement of a painted world point equals the drag. It holds at one zoom, 2x, which is the one thing that exercises the division at all | verified |
| `TOILE-UI-282` | FRAME | A pan MUST NOT change the zoom, and MUST NOT change what is selected. | this spec section 2 | none. The pan touches one field of the camera and the pointer handler returns before the tool machine sees the move, but no test reads the zoom back after a pan | unverified |
| `TOILE-UI-283` | POST | A pan MUST ask for the visible set of the place it landed on. | this spec section 2 | none. The drag path queues one cull request per move and the wheel path queues another, and no test drives either handler -- section 5 | unverified |

### 1.3 Zoom in and out

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-284` | POST | A zoom about a point MUST leave the world point under that point fixed on screen. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_about_keeps_anchor_world_point_fixed` maps the anchor to world before and after and asserts it did not move | verified |
| `TOILE-UI-285` | POST | A zoom that hits a stop MUST still hold its anchor, rather than drift while the zoom refuses to change. | this spec section 2 | none. The clamp test drives three hundred events into both stops and reads the zoom alone; the anchor it passes is never mapped back | unverified |
| `TOILE-UI-286` | POST | A zoom the user asks for without a cursor MUST scale the board in place, and returning to 100 % MUST keep the view where it is. | this spec section 2; `features/ambre-spec-toile-menus.md section 3` | `ambre_toile::ui::viewport::tests::zoom_by_center_keeps_center_world_point_fixed` for the button, `::reset_zoom_returns_to_100_percent` for the chip, which asserts the centre survives the reset | verified |
| `TOILE-UI-287` | FRAME | A zoom MUST NOT move an element: it changes what the user looks at and nothing they own. | this spec section 2; `TOILE-UI-278` | structural, and the signature is the proof: the camera owns three fields and no document handle, so a zoom has nothing else to write | verified |

### 1.4 Turn the wheel

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-027` | POST | An automatic pointer mode MUST classify each wheel event from that event's own signature. | this spec section 3 | `ambre_toile::ui::view::wheel_tests::line_or_page_delta_mode_is_a_mouse`, `::horizontal_component_is_a_trackpad` and `::large_vertical_notch_is_a_mouse` hold the three signals, strongest first | verified |
| `TOILE-UI-288` | POST | An ambiguous wheel event MUST leave the latched guess unchanged, so one gesture does not alternate between panning and zooming. | this spec section 3; section 4 | `ambre_toile::ui::view::wheel_tests::fine_vertical_glide_is_ambiguous` holds the pure classifier returning nothing. The latch is a field the handler writes, and no test sets it, sends an ambiguous event and reads it back | unverified |
| `TOILE-UI-028` | POST | `Ctrl` with the wheel, and a trackpad pinch, MUST zoom in every pointer mode. | this spec section 3 | none. The rule is three arms of one match and no test drives the handler at all, only the pure classifier under it | unverified |
| `TOILE-UI-029` | FRAME | A wheel event MUST NOT touch a pointer mode the user picked: their choice freezes the detection. | this spec section 3; section 4 | none. The classifier has one caller, the automatic arm, so the rule holds by construction today -- but nothing guards that fact and no test forces a mode and sends a contrary event | unverified |
| `TOILE-UI-289` | POST | The wheel in the trackpad mode MUST pan both axes, so a two-finger glide moves the plane diagonally. | this spec section 3 | none. The handler pans by the raw event deltas and nothing reads the camera back; which way a scroll moves the board is not ruled anywhere -- section 5 | unverified |

### 1.5 Fit the board or the selection

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-004` | POST | A fit MUST centre the target and scale it to the tighter axis, with padding, so the content never touches the edge. | this spec section 2 | `ambre_toile::ui::viewport::tests::fit_bounds_centers_and_scales_with_padding` asserts the centre, the governing axis and the padded fit on both axes; `::fit_target_matches_fit_bounds` holds the pure solver to the same answer | verified |
| `TOILE-UI-290` | FAIL | A fit over a rectangle with no area MUST leave the camera exactly as it was, rather than invent one. | this spec section 2; section 4 | `ambre_toile::ui::viewport::tests::fit_bounds_ignores_empty_rect` compares the whole camera before and after, and `::fit_target_matches_fit_bounds` asserts the solver answers nothing | verified |
| `TOILE-UI-291` | FAIL | A fit over a rectangle whose extent is not finite MUST refuse the same way. | this spec section 2 | none. The guard tests both extents for finiteness and no test passes an infinite or undefined one, so the half of the refusal that protects against a computed rectangle is unheld | unverified |
| `TOILE-UI-292` | FRAME | A fit MUST NOT change the canvas it fits into. | this spec section 2 | structural, and the construction is the proof: the fit assigns the centre and the zoom and reads the size, so the rectangle it solves against cannot be the one it moves | verified |
| `TOILE-UI-293` | POST | A fit over the whole board MUST frame the extent it asked for, and not the answer to somebody else's question. | this spec section 2; section 5 | none, and **the rule fails today**: the request carries no correlator and the reply carries none either, so a pending fit consumes whichever extent lands first -- including the one the status bar asked for -- and a reply with no extent drops the fit without a notice. Answered by the `An answer names the request that asked for it` Story (`ambre-roadmap.md` E7) | unverified |

### 1.6 Glide to a slide

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-294` | POST | A camera glide MUST reproduce where it started at the start, land exactly on its target at the end, and interpolate both the centre and the zoom between. | this spec section 2; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::viewport::tests::glide_interpolates_center_and_zoom` asserts the two endpoints exactly and the linear midpoint of each field | verified |
| `TOILE-UI-295` | FRAME | A glide MUST NOT change the canvas: it moves the camera through it. | this spec section 2 | the same test asserts the size is carried through unchanged, and the glide is a pure function returning a new camera rather than mutating one | verified |

### 1.7 Resize the canvas

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-296` | POST | A canvas resize MUST ask for the visible set of the new canvas, so an element the new edge uncovers arrives. | this spec section 3 | none. The resize entry is three statements and no test calls it -- section 5 | unverified |
| `TOILE-UI-297` | FRAME | A resize MUST NOT move the camera: the same world point stays under the screen centre, at the same scale. | this spec section 3 | none. The entry writes the size alone, and nothing reads the centre or the zoom back across a resize | unverified |
| `TOILE-UI-298` | FAIL | A canvas that reports no extent MUST be treated as the smallest one, never as a camera that cannot be inverted. | this spec section 2; `TOILE-UI-002` | none. The floor is a call on each axis and no test passes a zero or a negative, which is the one input that would make the mapping of `TOILE-UI-002` undefined | unverified |

* **Budget.** the pan and zoom latency under 15 ms, a row of
  `ambre-spec-budgets.md section 2`. It
  is held by construction rather than by a bench: the camera is applied locally and the cull request
  that follows it is asynchronous (`TOILE-UI-020`, `features/ambre-spec-toile-board-rendering.md`).
* **Boundary.** none: the camera is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses to the Logic thread is a rectangle to cull
  against, never document data (`TOILE-UI-001`).
* **Verification.** 15 `verified`, 14 `unverified`, 2 `unimplemented`. Ten rows became thirty-one
  over the same eighteen tests: the geometry was always covered and was stated in four rows that
  each carried three or four claims. Splitting them showed what the coverage does not reach -- the
  anchor at a clamp stop, a refused non-finite extent, every frame condition, and the whole handler
  layer above the pure functions, which is the wheel, the drag, the fit round trip and the resize.
  The old bullet called that one gap. It is the larger half of the document.

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

**Two types for one rectangle.** `ViewportTransform` is the render thread's camera; `Viewport`
(`ambre-bus`) is the Logic thread's culling input. They carry the same three fields and are
deliberately distinct: the camera is applied locally and immediately, and a copy of it is forwarded
as a cull request the UI never waits on. Logic never pushes a camera back. The clamp lives on one
side of that pair only: the culling type is three public fields, and the one thing the Logic thread
does with it divides by the zoom (`TOILE-UI-281`).

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

A fit over the whole board asks the Logic thread for the extent, because the UI holds the culled set
alone; the reply is the same aggregate the status bar consumes
(`features/ambre-spec-toile-status-bar.md`). Neither the question nor the answer carries a
correlator, and a pending fit takes the first reply that arrives (`TOILE-UI-293`).

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

**How a wheel event is classified.** Strongest signal first: a line or page unit is a classic wheel,
so a mouse; a non-zero horizontal component is a two-axis glide, so a trackpad; a large vertical
delta with no horizontal one is a discrete notch, so a mouse. A fine ambiguous glide leaves the
latched guess alone, and that latch starts on the trackpad -- the gentler wrong default, because a
mouse user's wheel then merely pans, where a trackpad user's two-finger scroll would zoom abruptly.

**The adaptive grid.** The step is the smallest multiple of the board unit whose on-screen spacing
is comfortable, doubled while the board is zoomed out and halved back while it is zoomed in, never
below the board unit (`TOILE-UI-010`). The grid paints under the content walk, in screen space, so a
line stays one pixel wide at every zoom (`features/ambre-spec-toile-board-rendering.md`).

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

* **Snapping is a function nothing calls** (`TOILE-UI-025`). The quantizer and the step selection are
  written and host-tested, the board carries a snapping flag written once at creation, and no gesture
  reads either: the quantizer has no caller in the workspace and a drag emits the raw pointer
  position. So the grid is drawn and is not magnetic.
* **The board's grid unit is not read.** The step is derived from a constant in the render module,
  not from the `board` register the schema declares, so a board authored on a different unit snaps
  and paints on the default one.
* **The minimap** (`TOILE-UI-030`): the design is a second small camera painted last, fed by the
  union of the board's content -- an extent the Logic thread already answers for a whole-board fit --
  with a drag on its rectangle moving the main camera. No field carries it and nothing paints it.
* **No handler above the pure geometry has a test**, in a crate whose rule is that an interaction
  lands with a geometry round trip and a recording of what it painted
  (`ambre-spec-testing.md section 6`). The wheel, the pan drag, the zoom entries and the resize all
  read or write the camera and none is driven: the latch a wheel event must not disturb
  (`TOILE-UI-288`, `TOILE-UI-029`), the direction a trackpad glide pans (`TOILE-UI-289`) and the
  cull request each of them queues (`TOILE-UI-283`, `TOILE-UI-296`) are unheld together.
* **A fit trusts the wrong reply** (`TOILE-UI-293`). The request for a board extent carries no
  correlator and the reply carries none, so the flag a pending fit sets is consumed by whichever
  aggregate lands first -- the status bar re-asks for one on every selection change. A reply with no
  extent clears the flag and moves nothing, so the fit is simply lost.
* **The clamp is claimed on both sides and written on one** (`TOILE-UI-281`). The camera clamps its
  zoom and a comment says the culling type matches; it does not. That type is a public struct with a
  default and one method, and the method divides by the zoom. Today the only producer in the app is
  the camera, so nothing is wrong -- and nothing keeps it that way.
* **Two refusals are half held.** A fit ignores a zero-area rectangle, which is tested, and a
  non-finite one, which is not (`TOILE-UI-291`); a canvas floors its extent at one pixel, which is
  the input that would make the mapping undefined, and nothing drives it (`TOILE-UI-298`).
* **A pointer mode does not survive a reload.** The preference is held in memory on the view, and
  nothing writes it to local storage, so a user who forces a mode forces it again on the next visit.
