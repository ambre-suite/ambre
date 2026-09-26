# AMBRE -- FEATURE: BOARD RENDERING

**Type:** Vertical (feature)
**Implementation:** partial -- the walk, the overlay and the dirty flag are built; nothing measures the frame.

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
| `TOILE-UI-001` | `INV` | The render side MUST NOT link the CRDT: it consumes the culled visible set and emits commands. | this spec section 2; `ambre-spec-architecture.md section 2` | structural -- `loro` appears nowhere under `ambre-toile/src/ui/` except in four explanatory comments: no type, no call; cargo-deny -- `deny.toml` bans `loro` outside the four app crates (`SYNC-011`) | none | verified |
| `TOILE-UI-011` | `INV` | The content walk MUST leave the transform stack as it found it: every push balanced by its pop. | this spec section 2 | `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node`; `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- both read the depth back at zero when the walk returns | none | verified |
| `TOILE-UI-043` | `OBS` | Elements and connectors MUST overlap on screen in the one order the board gives them, a frame's children with their frame. | this spec section 2; `features/ambre-spec-toile-z-order.md section 1` | none | unheld -- `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node` walks the sequence, but counts pushes and fills and asserts no order, and the visible set it builds holds no connector, so the word "interleaved" is exercised nowhere; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-012` | `OBS` | A note MUST paint as a filled card. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::sticky_paints_a_colored_card` -- it reads the fill and the shadow off the surface | none | verified |
| `TOILE-UI-356` | `OBS` | A text element MUST paint no card. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::text_element_paints_no_card` -- it finds neither fill nor shadow behind the glyphs | none | verified |
| `TOILE-UI-262` | `OBS` | A note's body MUST stay legible whatever the card colour: its ink is chosen for it, not taken from the fill. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::text_ink_precedence` -- the choice as a pure function | partial -- the paint is not held: the two tests that look at the glyph colour loop over runs a headless host may never shape, so both pass on a surface that drew no text at all, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-263` | `OBS` | A note's body MUST size itself to the card, within a legible range, unless the style names a size. | this spec section 3 | `ambre_toile::ui::render::tests::sticky_font_size_tracks_the_card_and_is_clamped` -- the growth and the two clamps; `ambre_toile::ui::render::tests::sticky_font_size_auto_unless_overridden` -- the sentinel and the override | none | verified |
| `TOILE-UI-013` | `OBS` | An image whose media is not resolved MUST paint a placeholder, never nothing. | this spec section 3; `features/ambre-spec-toile-media.md section 4` | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- one image draw at the element box and one placeholder fill beside it | none | verified |
| `TOILE-UI-264` | `OBS` | An element the walk cannot draw MUST hold its place: a group paints nothing, an embed and a nested document paint a box until their child is composited. | this spec section 3; `features/ambre-spec-toile-embeds.md section 3` | none | unheld -- three of the nine kinds the walk switches on (group, embed, doc-embed) are reached by no test, so the arm that paints nothing and the arm that paints a box are the same untested arm today; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-044` | `OBS` | A body MUST be laid out in screen space at a zoom-scaled size, so its glyphs stay crisp at any zoom. | this spec section 3 | none | unheld -- the two font-size tests are pure functions of the card and the style and take no camera, so nothing observes the zoom multiplying the size or the layout happening outside the camera transform, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-265` | `OBS` | A rotated element MUST carry its body and the rules through it round with it. | this spec section 3 | none | unheld -- `ambre_toile::ui::render::tests::rotated_sticky_body_rotates_its_glyphs` asserts the rotation on each glyph run it finds, and finds none on a host with no font: the test states so itself and holds vacuously, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-276` | `OBS` | An element a transition or an in-flight gesture overrides MUST paint that override's colour and opacity, not the ones its style carries. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | `ambre_toile::ui::render::tests::content_walk_applies_animation_override` -- the interpolated blue at half alpha reaches the surface, a colour the default white style cannot produce | none | verified |
| `TOILE-UI-277` | `OBS` | An element so overridden MUST paint at the override's bounds and rotation. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | none | unheld -- the one test of the override looks at the fill alone: it tweens the bounds by twenty units and never reads them back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-358` | `OBS` | An element carrying no override MUST paint where it committed. | this spec section 3; `features/ambre-spec-toile-presentation.md section 4` | none | unheld -- that same test never looks at the second element, the one it left untouched, so nothing holds that an override reaches its own element alone; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-266` | `OBS` | The element under the inline editor MUST NOT be painted twice: the static body gives way to the live buffer. | this spec section 2; `features/ambre-spec-toile-text-editing.md section 3` | none | unheld -- the text pass skips the id it is handed and no test hands it one, so a regression would show as doubled glyphs on screen and green tests; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-045` | `OBS` | The board background MUST paint under the content, filling the canvas. | this spec section 3 | none | unheld -- the background is one call at the head of the content pass and no test looks at it; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-267` | `OBS` | Presentation focus MUST drop the grid, so the audience sees the slide and not the board. | this spec section 3; `TOILE-UI-083` | none | unheld -- one test paints with focus on, but asserts a tweened fill and never the lines that should be missing, so both branches of the choice are unobserved; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-046` | `OBS` | The selection box and its handles MUST paint above the content. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` -- the box and the five handles; `ambre_toile::ui::render::tests::overlay_rotates_box_and_handles_for_a_rotated_element` -- the rotated edges and the rotated handle centres | none | verified |
| `TOILE-UI-362` | `OBS` | The selection box and its handles MUST follow an in-flight gesture rather than the committed geometry. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_box_follows_gesture_override_bounds` -- the live bounds | none | verified |
| `TOILE-UI-268` | `OBS` | Only a single selection MUST carry the resize handles; several elements MUST show their union box alone. | this spec section 3; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` -- the single case, counted as five squares | partial -- nothing paints a multi-selection, so the half that says what several elements do not get is untested; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-269` | `OBS` | Every find match in view MUST be outlined. | this spec section 3; `features/ambre-spec-toile-find.md section 1` | `ambre_toile::ui::render::tests::overlay_outlines_visible_find_hits` -- one outline per hit, with both hits on screen | partial -- it drives no off-screen match, so the words "in view" are held by nothing; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-357` | `OBS` | The current find match MUST stand out from the rest. | this spec section 3; `features/ambre-spec-toile-find.md section 1` | none | unheld -- `ambre_toile::ui::render::tests::overlay_outlines_visible_find_hits` passes a focused hit and asserts nothing about it, so the accent that tells the user where they are is unobserved; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-270` | `OBS` | A gesture in flight MUST show its preview above the content, tracking the geometry it will commit. | this spec section 3; `features/ambre-spec-toile-tools.md section 1` | none | unheld -- no test builds a preview of any kind, so the marquee, the four shapes, the connector, the pen and the stroke are seven unexercised arms of one function, section 5; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Paint one frame

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-021` | `POST` | A frame in which nothing changed MUST plan no draw. | this spec section 2 | none | unheld -- the plan is a pure function of six fields and a tick, and nothing asserts that a clean frame plans neither pass. No bench guards the frame budget of `ambre-spec-budgets.md section 2`; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-271` | `POST` | A content change MUST repaint the content; anything else MUST repaint the overlay alone. | this spec section 2 | none | unheld -- the split is the whole point of the plan: a rule that mistook one for the other would repaint the board on a cursor move and cost the idle budget nothing measures; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-272` | `POST` | An open inline editor or a gesture in flight MUST keep the overlay repainting for as long as it lasts. | this spec section 2 | none | unheld -- these two are not flags a frame clears but states it reads, so the board repaints every frame until the editor closes, which is the caret blink and the reason an idle board is only idle with nothing open; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
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
* **Verification.** 13 `verified`, 17 `untested`. What holds is the shape of the walk --
  its order, its inputs and the element bodies thirteen tests drive. What is loose is what it draws. The
  background, the grid, the gesture preview, the selection box and three arms of the walk are
  painted by no test. Two rows rest on assertions that hold vacuously on a host with no font.

---

## 2. MODEL

**Containers.** None. The render side reads the visible set it is handed and writes nothing.

**What the Logic thread hands over** (`ambre-bus`): one snapshot per relevant change. It carries
the camera it was culled against and a single back-to-front sequence of nodes, elements and
connectors interleaved by z-order. Beside that sequence rides the reaction tally per target, which
is not z-ordered with the board. It carries no comment pins: nothing does yet (section 5). Its shape is
declared in `features/ambre-spec-toile-engine.md section 3`; this document does not restate it.

**The frame, in steps:**

1. drain the inbound events, marking what changed;
2. flush at most one cull request for the current camera (`TOILE-UI-020`);
3. plan the repaint from the dirty flags: a content change repaints the content, anything else
   repaints the overlay alone. A clean frame plans neither (`TOILE-UI-021`);
4. paint the background and, unless a presentation holds focus, the grid, in screen space. Then
   push the camera, walk the sequence once (`TOILE-UI-043`) and pop it. The bodies are laid out in
   screen space, skipping the one element the inline editor sits over (`TOILE-UI-266`);
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
* `ui/grid.rs` -- the background and the grid lines;
* `ui/view.rs` -- the event drain, the dirty flags, the repaint plan and the two paint entries.

---

## 3. SURFACE

**The passes, back to front.**

| Pass | Holds |
|---|---|
| background | the board fill, then the adaptive grid, in screen space; presentation focus drops the grid (`TOILE-UI-045`, `TOILE-UI-267`) |
| content | the visible sequence under the camera: frames, shapes, images, strokes, notes, embeds and connectors, interleaved by z-order |
| bodies | notes and text elements, laid out in screen space at a zoom-scaled size, rotated with their element (`TOILE-UI-044`) |
| overlay | the gesture preview (`TOILE-UI-270`), the find highlights (`TOILE-UI-269`), then the selection box and its handles, which only a single selection carries (`TOILE-UI-268`) |
| above the overlay | the presence cursors, the reaction badges of the visible set, and the inline editor when one is open: driven here, owned elsewhere |

**How each kind paints.**

| Kind | Painted as |
|---|---|
| a rectangle, a frame | its fill, then its outline |
| an ellipse, a triangle, a path | the same, through a tessellated path |
| a freehand stroke | the smoothed polyline stroked at its width, no fill |
| a note | a drop shadow, then a coloured card; the body rides the screen-space pass in a fixed readable ink (`TOILE-UI-012`) |
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
| Bodies are laid out in screen space | shaping text in world space with the rest of the walk | glyphs rasterized at a world size and scaled would blur at every zoom, and the atlas would hold one entry per zoom level |
| An unresolved image paints a placeholder | leaving the element out of the walk until its pixels land | media arrives after the board does, and painting nothing makes an image look deleted rather than pending |
| A clean frame plans no draw | painting every frame and relying on the GPU to be fast | an infinite board is mostly idle, and the pillar is a battery on a phone, not a frame rate on a desktop |
| Any content change repaints the whole content | a dirty-rectangle scheme, as the grid uses for a scroll | a partial strip would have to be derived from a diff nothing computes. The board is transformed on the GPU for a pan, so the common cases are a full change or no change at all |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing measures the frame** (`TOILE-UI-021`). The three Toile rows of
  `ambre-spec-budgets.md section 2` have no bench: the board paint at 2 000 elements, the pan and
  zoom latency, the presence overlay. The idle board is a rule with no figure at all.
  The dirty flag itself is a pure function of six fields and a host test reaches it.
* **The heavy-path budget is a design, not code.** A pathological path is described as tessellated
  across frames behind a bounding-box placeholder, with a cache keyed by geometry and tolerance.
  Nothing caps the work a single frame does, so one dense import can stall it.
* **The tessellation cache is the canvas layer's, and unmeasured here.** The tolerance bucketing this
  feature relies on is owned by `ambre-spec-drawing-surface.md`; no test of this crate observes a
  cache hit.
* **The comment pins paint outside this walk.** They ride the visible set beside the reaction
  tallies. Their pass is the last overlay of the frame rather than a node of the content walk. No
  rule of this document covers them (`features/ambre-spec-toile-comments.md section 4`).
* **Two rows read `verified` on assertions that can hold vacuously.** The card ink and the rotated
  body are asserted by looping over the glyph runs the surface recorded. A host with no font shapes
  none -- both tests say so themselves. `TOILE-UI-265` is `unverified` for it and
  `TOILE-UI-262` keeps only the pure choice of ink, which takes no surface.
* **No test builds a gesture preview.** The marquee, the four shapes, the connector, the pen and the
  freehand stroke are seven arms of one function nothing reaches (`TOILE-UI-270`). The same silence
  covers the union box of a multi-selection (`TOILE-UI-268`) and the group, embed and doc-embed arms
  of the walk (`TOILE-UI-264`).
* **The background and the grid have no test at all**, in either branch of the presentation choice
  (`TOILE-UI-045`, `TOILE-UI-267`). The zoom-scaled body pass is held only by pure functions of the
  card that take no camera (`TOILE-UI-044`).
* **A differential visible set is unspecified.** Every push carries the whole culled set, including
  what the UI already holds. The design names a patch, keyed on change size; nothing states when a
  patch is correct against an out-of-order full push.
