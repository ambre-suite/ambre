# AMBRE -- FEATURE: BOARD RENDERING

**Type:** Vertical (feature)

> The board paints what the Logic thread says is visible, in the order it says, and nothing else. An
> idle board costs no frame. A board of fifty thousand elements costs the same as the two thousand on
> screen.

The render side holds no document data: it consumes one culled, z-ordered snapshot per change and
emits abstract draw calls. What tessellates them, atlases the glyphs and talks to the GPU is the
shared canvas runtime (`ambre-spec-drawing-surface.md`, `ambre-spec-render-assets.md`,
`ambre-spec-frame.md`), reused rather than re-implemented.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-001` | `INV` | The render side MUST NOT link the CRDT: it consumes the culled visible set and emits commands. | this spec section 2; `ambre-spec-architecture.md section 2` | structural -- `loro` appears nowhere under `ambre-toile/src/ui/` except in four explanatory comments: no type, no call; cargo-deny -- `deny.toml` bans `loro` outside the four app crates, `ambre-logic` and `ambre-crdt` (`SYNC-011`) | none | verified |
| `TOILE-UI-011` | `INV` | The content walk MUST leave the transform stack as it found it: every push balanced by its pop. | this spec section 2 | `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node`; `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- both read the depth back at zero when the walk returns | none | verified |
| `TOILE-UI-043` | `OBS` | Elements and connectors MUST overlap on screen in the one order the board gives them, a frame's children with their frame. | this spec section 2; `features/ambre-spec-toile-z-order.md section 1` | `ambre_toile::logic::convergence_tests::the_painted_sequence_is_the_board_order` -- a frame child sits right after its frame, and a connector between two elements; `ambre_toile::ui::render::tests::the_walk_paints_in_the_order_it_is_handed` -- the fills land in the sequence order | none | verified |
| `TOILE-UI-012` | `OBS` | A note MUST paint as a filled card. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::sticky_paints_a_colored_card` -- it reads the fill and the shadow off the surface | none | verified |
| `TOILE-UI-356` | `OBS` | A text element MUST paint no card. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::text_element_paints_no_card` -- it finds neither fill nor shadow behind the glyphs | none | verified |
| `TOILE-UI-262` | `OBS` | A note's body MUST stay legible whatever the card colour: its ink is chosen for it, not taken from the fill. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::text_ink_precedence` -- the choice as a pure function; `ambre_toile::ui::render::tests::a_note_body_is_drawn_in_an_ink_that_reads_on_its_card` -- light, mid and dark cards, each with runs, each run at a contrast of four or more | none | verified |
| `TOILE-UI-263` | `OBS` | A note's body MUST size itself to the card, within a legible range, unless the style names a size. | this spec section 3 | `ambre_toile::ui::render::tests::sticky_font_size_tracks_the_card_and_is_clamped` -- the growth and the two clamps; `ambre_toile::ui::render::tests::sticky_font_size_auto_unless_overridden` -- the sentinel and the override | none | verified |
| `TOILE-UI-013` | `OBS` | An image whose media is not resolved MUST paint a placeholder, never nothing. | this spec section 3; `features/ambre-spec-toile-media.md section 4` | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- one image draw at the element box and one placeholder fill beside it | none | verified |
| `TOILE-UI-264` | `OBS` | An element the walk cannot draw MUST hold its place: a group paints nothing, an embed and a nested document paint a box until their child is composited. | this spec section 3; `features/ambre-spec-toile-embeds.md section 3` | `ambre_toile::ui::render::tests::a_group_paints_nothing_and_an_embed_holds_its_place` -- a group draws no op, and a web embed and a nested document each a box | none | verified |
| `TOILE-UI-044` | `OBS` | A body MUST be laid out in board units and painted under the camera, as the page lays it out; the surface rasterizes its glyphs at their screen size (`CANVAS-074`). | this spec section 3 | `ambre_toile::ui::render::tests::a_body_is_laid_out_in_board_units_under_the_camera` -- each run inside the camera push, at the same board point at 1x and 2x, below the padded top of the card | none | verified |
| `TOILE-UI-265` | `OBS` | A rotated element MUST carry its body and the rules through it round with it. | this spec section 3 | `ambre_toile::ui::render::tests::a_rotated_note_carries_its_body_and_rules_round` -- the shipped font shapes the body, and every run and both rules carry the angle | none | verified |
| `TOILE-UI-276` | `OBS` | An element a transition or an in-flight gesture overrides MUST paint that override's colour and opacity, not the ones its style carries. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::render::tests::content_walk_applies_animation_override` -- the interpolated blue at half alpha reaches the surface, a colour the default white style cannot produce | none | verified |
| `TOILE-UI-277` | `OBS` | An element so overridden MUST paint at the override's bounds and rotation. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::render::tests::an_override_moves_its_own_element_and_no_other` -- the image lands at the override bounds, under the override rotation | none | verified |
| `TOILE-UI-358` | `OBS` | An element carrying no override MUST paint where it committed. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::render::tests::an_override_moves_its_own_element_and_no_other` -- the other image lands at its committed bounds, under no rotation | none | verified |
| `TOILE-UI-266` | `OBS` | The element under the inline editor MUST NOT be painted twice: the static body gives way to the live buffer. | this spec section 2; `features/ambre-spec-toile-text-editing.md section 3` | `ambre_toile::ui::render::tests::the_note_under_the_editor_paints_no_body` -- the edited note paints its card and no run, and the other note its body | none | verified |
| `TOILE-UI-045` | `OBS` | The board background MUST paint under the content, filling the canvas. | this spec section 3 | `ambre_toile::ui::render::tests::the_background_comes_first_and_focus_drops_the_grid` -- the first op of the pass fills the canvas; `ambre_toile::ui::grid::tests::the_background_fills_the_canvas_first` -- with the grid and without it | none | verified |
| `TOILE-UI-267` | `OBS` | Presentation focus MUST drop the grid, so the audience sees the slide and not the board. | this spec section 3; `TOILE-UI-083` | `ambre_toile::ui::render::tests::the_background_comes_first_and_focus_drops_the_grid` -- grid dots without focus, none under it | none | verified |
| `TOILE-UI-046` | `OBS` | The selection box and its handles MUST paint above the content. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` -- the box and the five handles; `ambre_toile::ui::render::tests::overlay_rotates_box_and_handles_for_a_rotated_element` -- the rotated edges and the rotated handle centres | none | verified |
| `TOILE-UI-362` | `OBS` | The selection box and its handles MUST follow an in-flight gesture rather than the committed geometry. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_box_follows_gesture_override_bounds` -- the live bounds | none | verified |
| `TOILE-UI-268` | `OBS` | Only a single selection MUST carry the resize handles; several elements MUST show their union box alone. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` -- the single case, counted as five squares; `ambre_toile::ui::render::tests::several_selected_elements_show_their_union_box_alone` -- one box, the union, and the rotate knob as the only square | none | verified |
| `TOILE-UI-269` | `OBS` | Every find match in view MUST be outlined. | this spec section 3; `features/ambre-spec-toile-find.md section 1` | `ambre_toile::ui::render::tests::overlay_outlines_visible_find_hits` -- one outline per hit, with both hits on screen; `ambre_toile::ui::render::tests::a_match_out_of_view_gets_no_outline` -- a hit off screen draws nothing | none | verified |
| `TOILE-UI-357` | `OBS` | The current find match MUST stand out from the rest. | this spec section 3; `features/ambre-spec-toile-find.md section 1` | `ambre_toile::ui::render::tests::the_current_match_is_outlined_apart_from_the_rest` -- the current match in the accent at the heavier width, the others in a softer tint | none | verified |
| `TOILE-UI-270` | `OBS` | A gesture in flight MUST show its preview above the content, tracking the geometry it will commit. | this spec section 3; `features/ambre-spec-toile-tools.md section 1` | `ambre_toile::ui::render::tests::every_preview_tracks_the_geometry_it_will_commit` -- the marquee, the shapes, the connector, the pen line and the stroke, each at its world geometry; `ambre_toile::ui::tool::tests::gesture_overrides_track_resize_move_and_rotate` -- a resize, a move and a rotate in flight each override the geometry they will commit | none | verified |
| `TOILE-UI-420` | `OBS` | A rounded card and an ellipse MUST paint the outline their geometry defines. | this spec section 3 | `ambre_toile::ui::render::tests::a_rounded_card_keeps_the_shape_of_its_arcs` -- at five radii, the box of kurbo's rounded rectangle and its area to a thousandth; `ambre_toile::ui::render::tests::an_ellipse_keeps_the_shape_of_its_arcs` -- at four sizes, the box of kurbo's ellipse within 0.1 and its area to a thousandth | none | verified |

### 1.2 Paint one frame

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-021` | `POST` | A frame in which nothing changed MUST plan no draw. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_frame_repaints_only_what_changed` -- a second plan with nothing new plans neither pass | none | verified |
| `TOILE-UI-271` | `POST` | A content change MUST repaint the content; anything else MUST repaint the overlay alone. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_frame_repaints_only_what_changed` -- new data and a move repaint the content, while a click that selects and a marquee repaint the overlay alone | none | verified |
| `TOILE-UI-272` | `POST` | An open inline editor or a gesture in flight MUST keep the overlay repainting for as long as it lasts. | this spec section 2 | `ambre_toile::ui::view::view_tests::an_editor_or_a_gesture_keeps_the_overlay_repainting` -- three quiet plans each, with the editor open and with a drag held | none | verified |
| `TOILE-UI-273` | `FRAME` | Painting a frame MUST NOT read or write the document: it paints the snapshot it was handed. | this spec section 2; `ambre-spec-architecture.md section 2` | type-level -- both paint entries take the culled snapshot by reference and no document handle, so the walk has nothing else to read | none | verified |

### 1.3 Ask for what is on screen

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-020` | `POST` | At most one cull request MUST leave the UI thread per frame. | this spec section 2 | type-level -- the pending request is one `Option<Viewport>` on the view, so a second request in the same frame replaces the first, and the frame flush is its only send site | none | verified |
| `TOILE-UI-274` | `FRAME` | Asking for the visible set MUST NOT stall the frame. | this spec section 2; `BUS-004` | structural -- the flush is a try-send on a bounded queue and the frame continues on either arm, so no paint waits on the Logic thread | none | verified |
| `TOILE-UI-275` | `FAIL` | A cull request the queue refuses MUST NOT leave the board painting a stale set. | this spec section 2 | type-level -- the flush reads the outcome of the send and puts the camera back into `pending_viewport` on anything but `Sent`, so the slot is empty only once a request left, and the next frame offers the same camera again (`BUS-024`) | none | verified |

* **Budget.** the board paint under 16 ms at 2 000 visible elements, a row of
  `ambre-spec-budgets.md section 2`, beside the frame phases it sits inside. An idle board
  costs zero GPU pass, which is a rule and therefore `TOILE-UI-021` rather than a figure. Neither has
  a bench.
* **Boundary.** none: the walk paints view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. It reads no document and holds no key (`TOILE-UI-001`).
* **Verification.** 31 `verified`. The walk paints into a recording surface with the shipped font,
  so every glyph assertion reads real runs. The order, each arm, the overrides, the overlay and each
  preview are read back from the recorded ops. The repaint plan is driven through the view.

---

## 2. MODEL

**Containers.** None. The render side reads the visible set it is handed and writes nothing.

**What the Logic thread hands over** (`ambre-bus`): one snapshot per relevant change. It carries
the camera it was culled against and a single back-to-front sequence of nodes, elements and
connectors interleaved by z-order. Beside that sequence rides the reaction count per target, which
is not z-ordered with the board. It carries no comment pins: nothing does yet (section 5). Its shape is
declared in `features/ambre-spec-toile-engine.md section 3`; this document does not restate it.

**The frame, in steps:**

1. drain the inbound events, marking what changed;
2. flush at most one cull request for the current camera (`TOILE-UI-020`);
3. plan the repaint from the dirty flags: a content change repaints the content, anything else
   repaints the overlay alone. A clean frame plans neither (`TOILE-UI-021`);
4. paint the background and, unless a presentation holds focus, the grid, in screen space. Then
   push the camera, walk the sequence once (`TOILE-UI-043`) and pop it. The walk is the page's
   (`CANVAS-074`). Bodies and captions are laid out in board units inside it. The one element the
   inline editor sits over is skipped (`TOILE-UI-266`). The frame titles follow, in screen space;
5. paint the overlay: the gesture preview, the find highlights, then the selection box and its
   handles (`TOILE-UI-046`). Then come the presence cursors, the reaction badges and, when one is
   open, the inline editor -- three further passes this feature drives without owning.

**What marks the frame dirty.** A new visible set, a camera change, an in-flight gesture, an open
inline editor, a presentation transition, and the frame-rate chip's own tick. The flags among them
are cleared in the same call that reads them, so none survives the frame that honoured it. An
in-flight gesture and an open editor are not flags but states, and hold the overlay repainting for
as long as they last. That is what makes the caret blink (`TOILE-UI-272`).

**What draws, and what the canvas layer draws.** This feature owns the walk and the draw calls it
emits. The tessellation and the shaders are `ambre-spec-drawing-surface.md`, the glyph atlas is
`ambre-spec-render-assets.md` and the GPU submission is `ambre-spec-frame.md`. That is why the walk
is testable on the host against a recording surface. The assertion is the ordered sequence of draw
calls, with no GPU.

**Code:**

* `crates/ambre-toile/src/ui/render.rs` -- the content walk, the body pass and the overlay;
* `ui/grid.rs` -- the background and the grid dots;
* `ui/view.rs` -- the event drain, the dirty flags, the repaint plan and the two paint entries.

---

## 3. SURFACE

**The passes, back to front.**

| Pass | Holds |
|---|---|
| background | the board fill, then the adaptive grid, in screen space; presentation focus drops the grid (`TOILE-UI-045`, `TOILE-UI-267`) |
| content | the visible sequence under the camera: frames, shapes, images, strokes, notes, embeds and connectors, interleaved by z-order |
| bodies | notes and text elements, laid out in board units under the camera, rotated with their element (`TOILE-UI-044`) |
| overlay | the gesture preview (`TOILE-UI-270`), the find highlights (`TOILE-UI-269`), then the selection box and its handles, which only a single selection carries (`TOILE-UI-268`) |
| above the overlay | the presence cursors, the reaction badges of the visible set, and the inline editor when one is open: driven here, owned elsewhere |

**How each kind paints.**

| Kind | Painted as |
|---|---|
| a rectangle, a frame | its fill, then its outline |
| an ellipse, a triangle, a path | the same, through a tessellated path |
| a freehand stroke | the smoothed polyline stroked at its width, no fill |
| a note | a drop shadow, then a coloured card; the body follows in the same walk, in a readable ink (`TOILE-UI-012`): the dark ink or white, whichever contrasts more with the card (`TOILE-UI-262`) |
| a text element | no card: the body alone, in its own colour |
| an image | its decoded pixels, or a placeholder box until they arrive (`TOILE-UI-013`) |
| an embed, a nested document | a placeholder box; the child is composited outside this walk (`features/ambre-spec-toile-embeds.md section 3`) |
| a group | nothing: it exists so a click can resolve to it (`features/ambre-spec-toile-selection.md`) |
| a connector | its routed path, stroked, at its own place in the sequence |

**During a transition.** A presentation transition hands the walk a per-element override: bounds,
rotation, opacity and fill (`TOILE-UI-276`, `TOILE-UI-277`). The same override carries an in-flight
gesture, so one code path paints both
(`features/ambre-spec-toile-presentation.md section 4`). An element the cull drops takes no
override at all, which is what stops a fade half-way (`TOILE-UI-085`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One walk of one sequence | separate passes for frames, elements and connectors | a connector can sit behind or in front of an element, and a frame's children must follow their frame; any split into passes re-orders them, so the z-order would have to be rebuilt inside the renderer from data it does not hold |
| The render side never links the CRDT | reading the document behind a discipline | a render path that could read the document would eventually read it. Cutting the link is the only mechanical way to hold the thread contract, since the board lives in one document on one thread |
| The walk emits abstract draw calls | painting through the GPU types directly | the feature stops being host-testable against a recording surface, and the GPU stack is written twice instead of once: this is the same seam the spreadsheet uses |
| Bodies are laid out in board units, inside the walk | shaping each body in screen space at a zoom-scaled size, outside the walk | the page would lay the body out at another size than the screen, so a line could break elsewhere on paper. The surface rasterizes a glyph at its screen size (`CANVAS-074`), so the body stays crisp at every zoom without a screen layout |
| An unresolved image paints a placeholder | leaving the element out of the walk until its pixels land | media arrives after the board does, and painting nothing makes an image look deleted rather than pending |
| A clean frame plans no draw | painting every frame and relying on the GPU to be fast | an infinite board is mostly idle, and the pillar is a battery on a phone, not a frame rate on a desktop |
| Any content change repaints the whole content | a dirty-rectangle scheme, as the grid uses for a scroll | a partial strip would have to be derived from a diff nothing computes. The board is transformed on the GPU for a pan, so the common cases are a full change or no change at all |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing measures the frame** (`TOILE-UI-021`). The three Toile rows of
  `ambre-spec-budgets.md section 2` have no bench: the board paint at 2 000 elements, the pan and
  zoom latency, the presence overlay. The idle board is a rule with no figure at all.
  The plan is held by host tests that drive the view, not by a timing.
* **The heavy-path plan is a design, not code.** A pathological path is described as tessellated
  across frames behind a bounding-box placeholder, with a cache keyed by geometry and tolerance.
  Nothing caps the work a single frame does, so one dense import can stall it.
* **The tessellation cache is the canvas layer's, and unmeasured here.** The tolerance bucketing this
  feature relies on is owned by `ambre-spec-drawing-surface.md`; no test of this crate observes a
  cache hit.
* **The comment pins paint outside this walk.** They ride the visible set beside the reaction
  counts. Their pass is the last overlay of the frame rather than a node of the content walk. No
  rule of this document covers them (`features/ambre-spec-toile-comments.md section 4`).
* **Some changes still repaint the content they do not touch.** A selection change, a press and a
  marquee mark the overlay alone (`TOILE-UI-271`). Other handlers, such as a restyle or a group
  scope change, still mark both layers. That costs a repaint, never a wrong frame.
* **A differential visible set is unspecified.** Every push carries the whole culled set, including
  what the UI already holds. The design names a patch, keyed on change size; nothing states when a
  patch is correct against an out-of-order full push.
