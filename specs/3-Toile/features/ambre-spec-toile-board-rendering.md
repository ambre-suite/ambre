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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-001` | The render side MUST NOT link the CRDT: it consumes the culled visible set and emits commands. | this spec section 2; `ambre-spec-architecture.md section 2` | structural: `loro` appears nowhere under `ambre-toile/src/ui/` except in four explanatory comments -- no type, no call; `deny.toml` bans `loro` outside the four app crates (`SYNC-011`) | verified |
| `TOILE-UI-011` | The content walk MUST balance every pushed transform with its pop. | this spec section 2 | `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node`, `::content_walk_applies_animation_override` | verified |
| `TOILE-UI-012` | A note MUST paint as a filled card whose ink is independent of its fill, and whose body size tracks the card. | this spec section 3; `features/ambre-spec-toile-styles.md section 3` | `ambre_toile::ui::render::tests::sticky_paints_a_colored_card`, `::sticky_font_size_tracks_the_card_and_is_clamped`, `::sticky_font_size_auto_unless_overridden`, `::text_ink_precedence`, `::text_element_paints_no_card` | verified |
| `TOILE-UI-013` | An image whose media is not resolved MUST paint a placeholder, never nothing. | this spec section 3; `features/ambre-spec-toile-media.md section 4` | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` | verified |
| `TOILE-UI-020` | At most one cull request MUST leave the UI thread per frame. | this spec section 2 | type-level: the pending request is one `Option<Viewport>` on the view, so a second request in the same frame replaces the first, and the frame flush is its only send site | verified |
| `TOILE-UI-021` | The render loop MUST NOT repaint when nothing changed. | this spec section 2 | none. The dirty flags are read once per frame and cleared there; nothing asserts that a clean frame plans no draw, and no bench guards the frame budget of `ambre-spec-budgets.md section 2` | unverified |
| `TOILE-UI-043` | The content pass MUST be a single walk of the visible sequence, elements and connectors interleaved in the order it carries. | this spec section 2 | `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node`, which asserts the emitted draw calls follow the sequence | verified |
| `TOILE-UI-044` | A body MUST be painted in screen space so its glyphs stay crisp, and MUST carry its element's rotation. | this spec section 3 | `ambre_toile::ui::render::tests::rotated_sticky_body_rotates_its_glyphs`, `::sticky_font_size_tracks_the_card_and_is_clamped` | verified |
| `TOILE-UI-045` | The board background MUST paint under the content, and presentation focus MUST drop the grid. | this spec section 3; `TOILE-UI-083` | none. The background and the grid are one call at the head of the content pass, taking the focus flag, and no test drives either branch | unverified |
| `TOILE-UI-046` | The overlay MUST paint the selection box, its handles, the gesture preview and the find highlights above the content. | this spec section 3 | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection`, `::overlay_box_follows_gesture_override_bounds`, `::overlay_outlines_visible_find_hits`, `::overlay_rotates_box_and_handles_for_a_rotated_element` | verified |

* **Budget.** the board paint under 16 ms at 2 000 visible elements, a row of
  `ambre-spec-budgets.md section 2`, beside the frame phases it sits inside. An idle board
  costs zero GPU pass, which is a rule and therefore `TOILE-UI-021` rather than a figure. Neither has
  a bench, which is why that row is `unverified`.
* **Boundary.** none: the walk paints view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. It reads no document and holds no key (`TOILE-UI-001`).
* **Verification.** 8 `verified`, 2 `unverified`. Both gaps are the frame itself rather than what it
  draws: the dirty flag and the background branch are the two behaviours the render tests step over,
  because each is a decision of the loop and not of the walk.

---

## 2. MODEL

**Containers.** None. The render side reads the visible set it is handed and writes nothing.

**What the Logic thread hands over** (`ambre-bus`): one snapshot per relevant change, carrying the
camera it was culled against, a single back-to-front sequence of nodes -- elements and connectors
interleaved by z-order -- and the comment pins, kept apart because a pin is not z-ordered with the
board. Its shape is declared in `features/ambre-spec-toile-engine.md section 3`; this document does
not restate it.

**The frame, in steps:**

1. drain the inbound events, marking what changed;
2. flush at most one cull request for the current camera (`TOILE-UI-020`);
3. plan the repaint from the dirty flags: a content change repaints the content, anything else
   repaints the overlay alone, and a clean frame plans neither (`TOILE-UI-021`);
4. paint the background and the grid in screen space, then push the camera and walk the sequence
   once (`TOILE-UI-043`), then pop it and lay the bodies out in screen space;
5. paint the overlay: the gesture preview, the find highlights, then the selection box and its
   handles (`TOILE-UI-046`).

**What marks the frame dirty.** A new visible set, a camera change, an in-flight gesture, an open
inline editor, a presentation transition, and the frame-rate chip's own tick. Each is cleared in the
same call that reads it, so a flag never survives the frame that honoured it.

**What draws, and what the canvas layer draws.** This feature owns the walk and the draw calls it
emits. The tessellation and the shaders are `ambre-spec-drawing-surface.md`, the glyph atlas is
`ambre-spec-render-assets.md` and the GPU submission is
`ambre-spec-frame.md`, which is why the walk is testable on the host against a recording surface:
the assertion is the ordered sequence of draw calls, with no GPU.

**Code:**

* `crates/ambre-toile/src/ui/render.rs` -- the content walk, the body pass and the overlay;
* `ui/grid.rs` -- the background and the grid lines;
* `ui/view.rs` -- the event drain, the dirty flags, the repaint plan and the two paint entries.

---

## 3. SURFACE

**The passes, back to front.**

| Pass | Holds |
|---|---|
| background | the board fill, then the adaptive grid, in screen space (`TOILE-UI-045`) |
| content | the visible sequence under the camera: frames, shapes, images, strokes, notes, embeds and connectors, interleaved by z-order |
| bodies | notes and text elements, laid out in screen space at a zoom-scaled size, rotated with their element (`TOILE-UI-044`) |
| overlay | the gesture preview, the find highlights, the selection box and its handles |

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

**During a transition.** A presentation transition hands the walk a per-element override -- bounds,
rotation, opacity and fill -- and the same override carries an in-flight gesture, so one code path
paints both (`features/ambre-spec-toile-presentation.md section 4`). An element the cull drops takes
no override at all, which is what stops a fade half-way (`TOILE-UI-085`).

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One walk of one sequence | a connector can sit behind or in front of an element, and a frame's children must follow their frame. Any split into passes re-orders them, so the z-order would have to be rebuilt inside the renderer from data it does not hold | separate passes for frames, elements and connectors |
| The render side never links the CRDT | it is the only mechanical way to hold the thread contract: the board lives in one document on one thread, and a render path that could read it would eventually read it | reading the document behind a discipline |
| The walk emits abstract draw calls | it makes the whole feature host-testable against a recording surface, and it is the same seam the spreadsheet uses, so the GPU stack is written once | painting through the GPU types directly |
| Bodies are laid out in screen space | glyphs rasterized at a world size and scaled would blur at every zoom, and the atlas would hold one entry per zoom level | shaping text in world space with the rest of the walk |
| An unresolved image paints a placeholder | media arrives after the board does, and painting nothing makes an image look deleted rather than pending | leaving the element out of the walk until its pixels land |
| A clean frame plans no draw | an infinite board is mostly idle, and the pillar is a battery on a phone, not a frame rate on a desktop | painting every frame and relying on the GPU to be fast |
| Any content change repaints the whole content | the board is transformed on the GPU for a pan, so the common cases are a full change or no change at all; a partial strip would have to be derived from a diff nothing computes | a dirty-rectangle scheme, as the grid uses for a scroll |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing measures the frame** (`TOILE-UI-021`). The three Toile rows of
  `ambre-spec-budgets.md section 2` -- the board paint at 2 000 elements, the pan and zoom
  latency, the presence overlay -- have no bench, and the idle board is a rule with no figure at all.
  The dirty flag itself is a pure function of six fields and a host test reaches it.
* **The heavy-path budget is a design, not code.** A pathological path is described as tessellated
  across frames behind a bounding-box placeholder, with a cache keyed by geometry and tolerance.
  Nothing caps the work a single frame does, so one dense import can stall it.
* **The tessellation cache is the canvas layer's, and unmeasured here.** The tolerance bucketing this
  feature relies on is owned by `ambre-spec-drawing-surface.md`; no test of this crate observes a
  cache hit.
* **Comment pins are not painted.** The visible set carries a field for them and the overlay has no
  pass for it, which is consistent: the whole subject lands with the "Comments" Story
  (`ambre-roadmap.md` E4) and its rules live with it.
* **A differential visible set is unspecified.** Every push carries the whole culled set, including
  what the UI already holds. The design names a patch, keyed on change size; nothing states when a
  patch is correct against an out-of-order full push.
