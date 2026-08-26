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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-002` | A screen point and a world point MUST map to each other exactly, and a zoom about a point MUST leave that world point fixed. | this spec section 2 | `ambre_toile::ui::viewport::tests::round_trip_is_identity`, `::zoom_about_keeps_anchor_world_point_fixed`, `::zoom_by_center_keeps_center_world_point_fixed`, `::center_maps_to_screen_center` | verified |
| `TOILE-UI-003` | Zoom MUST stay inside 0.1x..10x, whatever the input. | this spec section 2 | `ambre_toile::ui::viewport::tests::zoom_clamps_to_range`, `::fit_bounds_clamps_tiny_rect_to_max_zoom`, `::reset_zoom_returns_to_100_percent` | verified |
| `TOILE-UI-004` | A fit MUST centre the target and scale it with padding; an empty rectangle MUST be ignored rather than produce a degenerate camera. | this spec section 2 | `ambre_toile::ui::viewport::tests::fit_bounds_centers_and_scales_with_padding`, `::fit_bounds_ignores_empty_rect`, `::fit_target_matches_fit_bounds`, `::glide_interpolates_center_and_zoom` | verified |
| `TOILE-UI-010` | The grid spacing MUST stay legible across the whole zoom range, and the step MUST stay a multiple of the board unit. | this spec section 3 | `ambre_toile::ui::grid::tests::spacing_never_illegible_across_zoom_sweep`, `::step_is_power_of_two_multiple_of_base` | verified |
| `TOILE-UI-025` | A drag or a resize MUST quantize to the current grid step before its command is emitted, while the board asks for snapping. | this spec section 3; section 5 | not built -- lands with the "Snap to the board grid" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TOILE-UI-026` | A pan MUST move the content by the pointer's own displacement, at every zoom. | this spec section 2 | `ambre_toile::ui::viewport::tests::pan_moves_content_with_the_drag` | verified |
| `TOILE-UI-027` | An automatic pointer mode MUST classify each wheel event from its own signature, and an ambiguous one MUST leave the latched guess unchanged. | this spec section 3 | `ambre_toile::ui::view::wheel_tests::line_or_page_delta_mode_is_a_mouse`, `::horizontal_component_is_a_trackpad`, `::large_vertical_notch_is_a_mouse`, `::fine_vertical_glide_is_ambiguous` | verified |
| `TOILE-UI-028` | `Ctrl` with the wheel, and a trackpad pinch, MUST zoom in every pointer mode. | this spec section 3 | none. The branch is the first arm of the wheel handler and no test drives the handler at all, only the device classifier of `TOILE-UI-027` | unverified |
| `TOILE-UI-029` | A pointer mode the user picks MUST freeze the detection, so a wheel event never overrides the choice. | this spec section 3; section 4 | none. The three menu entries assign the mode and the automatic arm is the only caller of the classifier; nothing asserts a forced mode survives a wheel event | unverified |
| `TOILE-UI-030` | Dragging the minimap's viewport rectangle MUST move the camera alone: it MUST produce no change to the document. | this spec section 5 | not built -- lands with the "The facilitation layer" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the pan and zoom latency under 15 ms, a row of
  `ambre-spec-budgets.md section 2`. It
  is held by construction rather than by a bench: the camera is applied locally and the cull request
  that follows it is asynchronous (`TOILE-UI-020`, `features/ambre-spec-toile-board-rendering.md`).
* **Boundary.** none: the camera is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What crosses to the Logic thread is a rectangle to cull
  against, never document data (`TOILE-UI-001`).
* **Verification.** 6 `verified`, 2 `unverified`, 2 `unimplemented`. The two gaps are one gap: the
  wheel handler ships with nothing guarding it, while the pure classifier under it is covered four
  ways. It is a state machine over three fields and a host test reaches it.

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
inverse always exists, because the zoom is clamped away from zero (`TOILE-UI-003`).

**Two types for one rectangle.** `ViewportTransform` is the render thread's camera; `Viewport`
(`ambre-bus`) is the Logic thread's culling input. They carry the same three fields and are
deliberately distinct: the camera is applied locally and immediately, and a copy of it is forwarded
as a cull request the UI never waits on. Logic never pushes a camera back.

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
(`features/ambre-spec-toile-status-bar.md`).

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
| the wheel, in the trackpad mode | pans both axes; a pinch zooms |
| the wheel, in the automatic mode | classified per event, then zooms or pans (`TOILE-UI-027`) |
| `Ctrl` with the wheel, a pinch | zooms, in every mode (`TOILE-UI-028`) |
| a drag with the Hand tool, with `Space` held, or with the middle button | pans (`TOILE-UI-026`) |
| the zoom control and the View menu | zoom in, zoom out, 100 %, fit the board, fit the selection (`features/ambre-spec-toile-menus.md`) |
| a canvas resize | keeps the centre and the zoom, and re-culls |

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The camera is applied locally and the cull request follows it | a board is a plane the user drags directly: waiting for the Logic thread before painting would put the round trip inside every gesture, and the content already on screen only needs a different transform to be correct | routing the pan and the zoom through the Logic thread, which is what a virtual-scroll grid does because its content is fetched by index |
| The render camera and the culling rectangle are two types | they carry the same fields and have opposite lifetimes: one is written every pointer move, the other is read once per cull. One type would make it possible to assign the stale culling copy back onto the live camera | one shared `Viewport` type, which the earlier design carried |
| The wheel has no single default | the same event means "zoom" on a mouse and "pan" on a trackpad, and the browser reports both the same way. A wrong fixed default makes half the users fight the board on every scroll | picking one behaviour and leaving the other device to `Ctrl` |
| An ambiguous glide leaves the guess unchanged | the classifier runs per event, so flipping on a weak signal makes the board alternate between panning and zooming inside one gesture | classifying every event, with no latch |
| The grid step is a multiple of the board unit | the grid is magnetic: a step that is not a multiple of the unit snaps content off the unit it was authored on | a step derived only from the pixel spacing |
| A fit refuses an empty rectangle | fitting a zero-area rectangle is a division by zero, and its two plausible answers -- maximum zoom, or no move -- differ; refusing states which one | clamping the extent to a minimum, which invents a rectangle the caller did not ask for |
| The minimap stays a camera, not a document property | it is a way of looking, and two users navigate independently; sharing the rectangle would make them fight over one camera | a synchronized overview, as a board register |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Snapping is a function nothing calls** (`TOILE-UI-025`). The quantizer and the step selection are
  written and host-tested, the board carries a snapping flag written once at creation, and no gesture
  reads either: a drag emits the raw pointer position. So the grid is drawn and is not magnetic.
* **The board's grid unit is not read.** The step is derived from a constant in the render module,
  not from the `board` register the schema declares, so a board authored on a different unit snaps
  and paints on the default one.
* **The minimap** (`TOILE-UI-030`): the design is a second small camera painted last, fed by the
  union of the board's content -- an extent the Logic thread already answers for a whole-board fit --
  with a drag on its rectangle moving the main camera. No field carries it and nothing paints it.
* **The whole wheel path has no test**, in a crate whose rule is that an interaction lands with a
  geometry round trip and a recording of what it painted (`ambre-spec-testing.md section 6`). The
  classifier under it is covered four ways, which makes the gap the plumbing rather than the
  decision.
* **A pointer mode does not survive a reload.** The preference is held in memory on the view, and
  nothing writes it to local storage, so a user who forces a mode forces it again on the next visit.
