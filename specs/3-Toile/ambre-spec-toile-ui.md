# AMBRE — SPEC: TOILE UI (Rendering & Interactions)
**Type:** Vertical (Feature / Application — UI)
**Status:** Draft
**Implementation:** partial

> Toile render + interaction layer running in the **UI (render) thread**: consumes the `DisplayViewport`
> produced by `ToileState` (`toile-logic §9`), tessellates and paints the visible set through the shared
> `DrawSurface` (WgpuSurface) at 60+ FPS with continuous pan/zoom, and translates pointer/keyboard/touch
> input into `ToileCommand`s. It owns no CRDT state and never touches `LoroDoc`.
>
> **Scope of this spec:** the GPU pipeline, viewport transform, culling-driven paint, hit-testing, the tool
> palette (select / vector pen / sticky / shape / connector / freehand / text), inline text editing via
> Parley, and the collaboration overlays' **render** side (multi-cursors, live strokes, laser). The engine,
> spatial index and CRDT model are in `toile-logic`; presentation mode, auto-layout, boolean ops, votes and
> reactions are in `toile-features`; import/export and media fetch are in `toile-interop`.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- Maintain the viewport transform (`world <-> screen`) with continuous zoom (0.1x .. 10x) and pan.
- Tessellate and paint the culled visible set each frame from `DisplayViewport`; never iterate the full doc.
- Exact hit-testing (rotation, path fill, stroke width) refining the R-tree pre-filter from `toile-logic`.
- Own the tool state machine and emit `ToileCommand`s; stream ephemeral presence while a gesture is in flight.
- Inline rich-text editing (Parley/Swash) for sticky notes and text elements.
- Render remote presence (cursors, in-flight strokes, laser pointer, selection halos).

**Dependencies:**

Toile-ui is the `ui` module of the `ambre-toile` app crate. Per `ambre-spec-architecture.md §1`, an app crate
depends on `ambre-bus + ambre-draw + ambre-canvas + ambre-chrome + ambre-commands + leptos` — **not** on raw
`wgpu`/`lyon`/`swash` (those live once in `ambre-canvas`) and **not** on `ambre-logic`/`ambre-sync`.

| Crate | Role | Rationale |
|---|---|---|
| `ambre-bus` | `ToileCommand`/`ToileEvent`, `DisplayViewport`, `CommandSink` | Typed UI<->Logic data plane |
| `ambre-canvas` | `WgpuSurface`, `QuadRenderer`, `GlyphAtlas`, `TextCache`, `FontContext`, GPU bootstrap + rAF | **The** content render runtime — Toile reuses it, does not re-implement it (`ambre-spec-canvas.md`) |
| `ambre-draw` | `DrawSurface` trait, re-exports `kurbo`/`peniko`/`parley` | Abstract render contract; `RecordingSurface` (feature `testing`) for host tests |
| `ambre-chrome` | `AppShell`, `Toolbar`/`Menu`/`Dialog`/`ContextMenu`, keybinding manager | Tool palette, menus, dialogs = **DOM/Leptos chrome**, not canvas |
| `ambre-commands` | `Command`/`CommandId`/`CommandState`/keybindings | Toile tools & shortcuts are registry commands, rendered by `ambre-chrome` |
| `kurbo` | Geometry (`Affine`, `BezPath`, `Point`, `Rect`) | Viewport transform, hit-test, path math (via `ambre-draw` re-export) |
| `flume` | UI<->Logic channels | Command emission + event intake |
| `leptos` | Toile-specific chrome components | Built on `ambre-chrome` |

> **No raw `wgpu`/`lyon`/`swash`, no `loro`.** The GPU pipeline (tessellation, glyph atlas, shaders) is owned by
> `ambre-canvas`; Toile paints through its `WgpuSurface: DrawSurface`. Arbitrary paths (shapes, connectors,
> smoothed strokes) and infinite pan/zoom rely on `WgpuSurface::{fill_path, stroke_path, push_transform,
> pop_transform}` — the `ambre-canvas` methods listed as its own CANVAS P1 items for "Toile"
> (`ambre-spec-canvas.md §D`). The render thread never links the CRDT — it consumes `DisplayViewport` only.

**Invariants:** prefix `TOILE-UI`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4`; the cross-spec roll-up is `ambre-spec-testing.md §7`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-001` | The render side MUST NOT link the CRDT: it consumes `DisplayViewport` and emits `ToileCommand`s. | §1 Dependencies | structural: `loro` appears nowhere under `ambre-toile/src/ui/` except in four explanatory comments -- no type, no call; `deny.toml` bans `loro` outside the four app crates (`SYNC-011`) | verified |
| `TOILE-UI-002` | `world_to_screen` and `screen_to_world` MUST round-trip, and a zoom about a point MUST leave that world point fixed. | §3 | `ambre_toile::ui::viewport::tests::round_trip_is_identity`, `::zoom_about_keeps_anchor_world_point_fixed`, `::zoom_by_center_keeps_center_world_point_fixed`, `::center_maps_to_screen_center` | verified |
| `TOILE-UI-003` | Zoom MUST stay inside 0.1x..10x, whatever the input. | §1; §3 | `ambre_toile::ui::viewport::tests::zoom_clamps_to_range`, `::fit_bounds_clamps_tiny_rect_to_max_zoom`, `::reset_zoom_returns_to_100_percent` | verified |
| `TOILE-UI-004` | A fit MUST centre the target and scale it with padding; an empty rect MUST be ignored rather than produce a degenerate transform. | §3 | `ambre_toile::ui::viewport::tests::fit_bounds_centers_and_scales_with_padding`, `::fit_bounds_ignores_empty_rect`, `::fit_target_matches_fit_bounds`, `::glide_interpolates_center_and_zoom` | verified |
| `TOILE-UI-005` | Hit-testing MUST be exact -- rotation, path fill and stroke width -- over the culled candidate set, and MUST return the front-most element. | §4 | `ambre_toile::ui::hit::tests::rect_inside_and_outside`, `::rotated_rect_uses_local_frame`, `::ellipse_corner_is_a_miss`, `::triangle_fill`, `::open_path_hits_near_the_stroke_only`, `::stroke_proximity`, `::topmost_wins_front_to_back`, `::marquee_selects_covered_elements` | verified |
| `TOILE-UI-006` | A locked element and a group MUST NOT be hit directly. | §4; §2.E note of `toile-logic` | `ambre_toile::ui::hit::tests::locked_never_hits`, `::group_never_hits` | verified |
| `TOILE-UI-007` | A click MUST resolve to the top-most enclosing group unless the user has entered that group's scope. | §5ter | `ambre_toile::ui::tool::tests::click_on_group_member_selects_top_most_group`, `::scoped_click_selects_the_member_inside_the_scope`, `::ungrouped_element_resolves_to_itself`, `::pointer_down_on_a_member_selects_the_group` | verified |
| `TOILE-UI-008` | A gesture MUST emit exactly one command, on release; a click that moves nothing MUST emit none. | §5 | `ambre_toile::ui::tool::tests::resize_preview_then_release_emits_exactly_one_command`, `::click_without_movement_emits_no_command`, `::click_selects_then_drag_emits_move`, `::drag_corner_handle_emits_resize`, `::drag_rotate_handle_emits_rotate` | verified |
| `TOILE-UI-009` | Escape or a tool switch MUST cancel an in-flight gesture without emitting a command. | §5 | `ambre_toile::ui::tool::tests::switching_tools_cancels_in_flight_gesture`, `::escape_reverts_an_armed_creation_tool_to_select`, `::escape_on_the_select_tool_reports_no_tool_change`, `::connector_release_on_empty_emits_nothing` | verified |
| `TOILE-UI-010` | Snapping MUST quantize to the grid step, and the step MUST stay legible across the whole zoom range. | §3 | `ambre_toile::ui::grid::tests::snap_quantizes_to_step`, `::step_is_power_of_two_multiple_of_base`, `::spacing_never_illegible_across_zoom_sweep` | verified |
| `TOILE-UI-011` | The content walk MUST balance every `push_transform` with its `pop_transform`. | §2 | `ambre_toile::ui::render::tests::content_walk_balances_transform_and_paints_each_node`, `::content_walk_applies_animation_override` | verified |
| `TOILE-UI-012` | A sticky MUST paint as a filled card whose ink is independent of its fill and whose font size tracks the card size. | §5quinquies.A; `toile-logic §2.B` | `ambre_toile::ui::render::tests::sticky_paints_a_colored_card`, `::sticky_font_size_tracks_the_card_and_is_clamped`, `::sticky_font_size_auto_unless_overridden`, `::text_ink_precedence`, `::text_element_paints_no_card` | verified |
| `TOILE-UI-013` | An image whose media is not resolved MUST paint a placeholder, never nothing. | §2; `toile-interop §5` | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` | verified |
| `TOILE-UI-014` | Remote presence MUST render per peer with distinct colours, and MUST leave no trace in the document. | §8 | `ambre_toile::ui::presence::tests::two_peers_get_distinct_colours`, `::cursor_draws_arrow_and_label_chip`, `::selection_ghosts_outline_visible_elements_only`, `::live_stroke_draws_a_polyline`, `::laser_accumulates_a_fading_trail_and_clears_on_lift`; the document half is `TOILE-016` | verified |
| `TOILE-UI-015` | An embedded document MUST map one child pixel to one screen pixel, through zoom and rotation. | §2; `toile-logic §2.I` | `ambre_toile::ui::embed::tests::a_child_pixel_is_a_screen_pixel`, `::transform_survives_a_rotated_embed`, `::child_size_scales_with_zoom`, `::child_origin_is_the_embeds_top_left`, `::screen_aabb_contains_a_rotated_quad` | verified |
| `TOILE-UI-016` | At most one embed MUST hold focus, and focus MUST be dropped when its element leaves the frame. | §2; `toile-logic §2.I` | `ambre_toile::ui::embed::tests::only_the_focused_embed_comes_back_focused`, `::focus_is_dropped_when_its_element_leaves_the_frame`, `::escape_blurs_only_when_something_is_focused`, `::a_click_outside_returns_focus_to_the_board`, `::a_drag_from_an_embed_does_not_focus_it` | verified |
| `TOILE-UI-017` | An embed session MUST open once per visible embed and MUST close only after the full debounce, so scrolling past it does not thrash the session. | §2; `toile-logic §2.I` | `ambre_toile::ui::embed::tests::a_visible_embed_opens_once_and_only_once`, `::close_waits_the_full_debounce`, `::a_scroll_oscillation_never_thrashes_the_session`, `::returning_before_the_deadline_cancels_the_close`, `::two_embeds_of_the_same_document_share_one_session` | verified |
| `TOILE-UI-018` | Every context-menu entry and every menu-bar item MUST resolve to a command, with no duplicate id across menus. | §5ter; §5quinquies | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve`, `::group_ungroup_resolve`, `::lock_unlock_resolve`, `::reorder_one_per_element`, `ambre_toile::ui::menu_bar::tests::every_menu_has_items`, `::menu_bar_ids_are_distinct`, `::edit_menu_carries_undo_redo_and_reuses_clipboard_ids`, `ambre_toile::ui::palette::tests::tool_for_command_maps_every_tool_id`, `::tool_chords_are_distinct` | verified |
| `TOILE-UI-019` | A style edit from the floating bar MUST touch only the field it edits, and a mixed selection MUST report `None` rather than a wrong common value. | §5quinquies | `ambre_toile::ui::floatbar::tests::style_edit_touches_only_its_field`, `::common_fill_is_none_when_mixed`, `::common_fill_survives_when_shared`, `ambre_toile::ui::style::tests::differing_fill_flags_only_fill_mixed`, `::identical_elements_are_not_mixed` | verified |
| `TOILE-UI-020` | The UI thread MUST NOT block on the Logic thread: at most one `SetViewport` per input frame, painting driven by the last `DisplayViewport` received. | §1; §9 | none. The double-buffering is implemented in the render loop and no test drives two viewport updates inside one frame to assert the coalescing | unverified |
| `TOILE-UI-021` | The render loop MUST NOT repaint when nothing changed. | §1; §9 | none. The dirty flag exists; nothing asserts a clean frame produces no draw call, and no `criterion` bench guards the 60 FPS budget of §11 | unverified |
| `TOILE-UI-022` | A comment pin MUST be painted in screen space and MUST NOT be occluded by board content, whatever the z-order. | §5sexies; `toile-logic §9` | not built -- lands with `toile-features §9` Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-UI-023` | The comments panel MUST hold no authoritative state: it re-reads a thread on `CommentsUpdated` and emits commands for every change. | §5sexies | not built -- lands with `toile-features §9` Phase 2 (Epic `E4`) | unimplemented |
| `TOILE-UI-024` | A pin badge MUST come from `DisplayViewport.comments` and MUST NOT require reading the panel or the thread bodies. | §5sexies | not built -- lands with `toile-features §9` Phase 2 (Epic `E4`) | unimplemented |

> **Invariant coverage.** 19 `verified`, 2 `unverified` (`TOILE-UI-020`, `TOILE-UI-021` -- both about the render loop, both implemented and unexercised), 3 `unimplemented` (the comments panel, §5sexies). "No `unsafe`" is crate-wide: `TOILE-023`.
---

## 1bis. UI-THREAD INTEGRATION (`AppView` / chrome / commands)

Toile plugs into the shared UI-thread runtime exactly like Trame (`ambre-spec-architecture.md §1/§6`,
`ambre-spec-canvas.md §A`), not as a bespoke render loop:

- **`AppView::Toile(ToileView)`.** `ToileView` is a variant of the closed `AppView` enum (in `ambre-app`) and
  exposes the inherent driver methods the `FrameHost` dispatches every frame:
  `update` (drain `UiEvent` via `flume try_recv` -> dirty flags), `plan_repaint` -> `RepaintPlan`,
  `paint_content(&mut WgpuSurface, &mut FontContext, &mut TextCache, strip)`, `paint_overlay(.., &FrameStats)`,
  `resize`, `handle_input` (pointer/keyboard on the canvas area). The whiteboard content is the `Layer::Content`
  pass; selection halos, transform handles and presence are the `Layer::Overlay` pass.
- **Chrome is DOM/Leptos, not canvas.** The tool palette, top menus, style/property panels, color pickers,
  share/present dialogs and the presenter panel are **Leptos components** built on `ambre-chrome` (the app's
  `chrome` module — `ambre-spec-ui.md L59`), rendered in the DOM around the `<canvas>`. Only the board content
  and its in-canvas overlays (halos, handles, live presence) are drawn on the canvas.
- **Content-anchored floating UI.** The selection/context toolbar that must follow an element as the board
  pans/zooms is the DOM element **transform-synced each frame** — the exact case `ambre-spec-ui.md L31` calls
  out **by name for Toile**. Its screen position is recomputed from `ViewportTransform` every frame.
- **Tools & shortcuts are commands.** Each Toile tool and action is a `Command` in the `ambre-commands` registry
  (`CommandId`, `CommandState`, keybindings), surfaced through `ambre-chrome`'s palette/toolbar/keybinding
  manager. The `Tool` state machine (§5) is the *canvas-side* consumer of the active tool; it does not own the
  key map — a keybinding resolves to a `Command` that emits the `ToileCommand` (`command_to_app`, the Trame
  pattern). The UI thread holds no undo stack: `Ctrl+Z`/`Ctrl+Shift+Z` are commands emitting
  `ToileCommand::Undo`/`Redo` (`toile-logic §8 bis`).

---

## 2. RENDERING ARCHITECTURE

```
DisplayViewport (postcard, from Logic)
        │  double-buffered swap on arrival
        ▼
ViewportTransform (world<->screen Affine)  ── pan/zoom, no Logic round-trip for the transform itself
        │  push_transform on the surface (WgpuSurface CPU-side matrix)
        ▼
paint_content ── walks DisplayViewport.nodes, emits ABSTRACT DrawSurface calls
        │        (fill_rect | fill_path/stroke_path | draw_glyphs | push_clip | image quad)
        ▼
WgpuSurface: DrawSurface  (from ambre-canvas) ── QuadRenderer + GlyphAtlas do the tessellation/atlas/GPU
```

**"What to draw" vs "how to draw".** Toile owns only the **content walk** (`paint_content`/`paint_overlay`),
emitting abstract `DrawSurface` calls — the same seam as Trame (`ambre-spec-architecture.md §4`). The
tessellation, glyph atlas, shaders and GPU submission are `ambre-canvas`'s `WgpuSurface`/`QuadRenderer`/
`GlyphAtlas`. So the Toile content walk is **host-testable** against `RecordingSurface` (mock `DrawSurface`,
`ambre-draw` feature `testing`): assert the ordered `DrawOp` sequence with no GPU. The batching/caching notes
below (§2bis) describe `ambre-canvas` behaviour Toile relies on, not a pipeline Toile re-implements.

**Transform ownership.** Pan and zoom update `ViewportTransform` **locally and immediately** for a jank-free
gesture; the same `Viewport` is forwarded to Logic as `SetViewport` so the next `DisplayViewport` is culled to
the new bounds. Between the gesture and the fresh cull, the last painted set is transformed on the GPU, so
panning never waits on a Logic round-trip (the < 15 ms display-latency budget).

**Layered passes (back to front):**
1. Board background + adaptive magnetic grid (grid step chosen from `zoom`).
2. **Content pass — a single walk of `layers`** (`toile-logic §2.G`), painting each `NodeId` in order:
   frame backgrounds/borders/labels, shapes, images, drawings, text, sticky, embeds, **and connectors** —
   all interleaved by their z-order (a connector can sit behind or in front of elements; a frame's children,
   by invariant §2.G, always follow their frame). There is **no** separate "frames below / connectors above"
   pass — `DisplayViewport` already delivers them in one z-ordered sequence.
3. Selection halos + transform handles (local, above content).
4. Presence overlay (remote cursors, live strokes, laser, selection ghosts).

## 2bis. TESSELLATION & BATCHING

- **Rects / sticky / images** → instanced quads (one draw call for all quads of a batch).
- **Ellipses / arbitrary `shape` paths / connectors / smoothed drawings** → `lyon` fill/stroke tessellation to
  triangle meshes, cached per element keyed by `(geom_hash, tolerance)` where `tolerance` derives from `zoom`
  (retessellate only when the flattening tolerance bucket changes, not every zoom delta).
- **Text** → Parley layout → Swash glyph atlas (cached), drawn as textured quads.
- **Drawings** → the Logic-smoothed polyline is stroked by `lyon` with a width curve from `StrokePoint.pressure`.

**Heavy-path budget.** Tessellation runs on the render thread, so a pathological `BezPath` (tens of thousands
of segments — a dense import, a huge freehand blob) must not stall a frame. A per-frame tessellation budget
caps the segment count retessellated in one frame; a path above a segment threshold is tessellated
**incrementally across frames** (or on a spare tick), and until its mesh is ready the element paints its
**bounding-box placeholder** (fill at reduced opacity). The cache (keyed by `(geom_hash, tolerance_bucket)`)
then serves it at full detail. This bounds worst-case frame time without a second worker; if profiling later
demands it, moving tessellation off-thread is a Phase 2 option (§13), not an MVP requirement.

---

## 3. VIEWPORT: PAN, ZOOM, GRID

```rust
pub struct ViewportTransform {
    pub center: kurbo::Point, // world point at screen center
    pub zoom:   f64,          // clamped 0.1 ..= 10.0
    pub size:   kurbo::Size,  // physical pixels
}

impl ViewportTransform {
    pub fn world_to_screen(&self) -> kurbo::Affine { /* translate(-center) * scale(zoom) * translate(size/2) */ }
    pub fn screen_to_world(&self) -> kurbo::Affine { self.world_to_screen().inverse() }

    /// Zoom about a screen anchor (cursor), keeping the anchored world point fixed.
    pub fn zoom_about(&mut self, screen_anchor: kurbo::Point, factor: f64) { /* ... */ }
}
```

> **`ViewportTransform` (UI) vs `Viewport` (Logic).** They carry identical fields (`center`/`zoom`/`size`) but
> are deliberately two types: `ViewportTransform` is the render thread's GPU transform, updated locally and
> immediately on every pan/zoom for a jank-free gesture (§2); `Viewport` (`toile-logic §3`) is the culling
> input. The UI forwards each change as `SetViewport(Viewport)` so Logic re-culls, but never waits on it to
> paint. The two are kept in phase by that one-way `SetViewport` flow; Logic never pushes a transform back.

**Navigation gestures.** The board is a 2D infinite plane, not a vertical document, so the natural wheel
gesture differs by device: on a **mouse** the wheel should *zoom* about the cursor (map-like); on a
**trackpad** two-finger scroll should *pan* both axes and pinch should zoom. There is no single correct
default, so the view carries a `PointerMode` preference (View menu, §5quinquies.E):

| `PointerMode` | Wheel | Ctrl+wheel / pinch |
|---|---|---|
| `Auto` (default) | zoom or pan, per detected device | zoom |
| `Mouse` | zoom about cursor | zoom |
| `Trackpad` | pan (both axes) | zoom |

`Ctrl`+wheel (and a trackpad pinch, which the browser reports as a `ctrl_key` wheel event) **always** zooms,
in every mode. Panning is *also* always available by dragging -- the Hand tool or `Space`-drag in any tool
(§5) -- independent of the wheel mode.

**Auto-detection.** In `Auto`, each `WheelEvent` is classified from its signature, strongest signal first:
a non-pixel `delta_mode` (line/page) is a classic wheel -> **mouse**; a non-zero `dx` is a two-axis glide ->
**trackpad**; a large `|dy|` with no `dx` is a discrete notch -> **mouse**. Fine ambiguous glides leave the
latched guess unchanged, which **starts at `Trackpad`** -- the gentler wrong default (a mouse user's wheel
merely pans instead of zooming, versus a trackpad user's two-finger scroll zooming abruptly). Choosing `Mouse`
or `Trackpad` in the View menu **forces** the mode and freezes detection (the user chose, we obey -- Miro's
model). The preference is view-local (§11bis); MVP keeps it in-memory (not persisted across sessions).

**Adaptive magnetic grid.** The grid step is chosen so the on-screen spacing stays in a comfortable band as
`zoom` changes (powers of the board `grid_size`); snapping (when `board.grid_snap`) quantizes drag/resize
results to the current step before the command is emitted.

> **Minimap (DEFERRED — post-MVP).** A large infinite board is hard to navigate by pan/zoom alone, so Miro shows
> a corner **minimap**: a scaled-down overview of the whole board's content extent with a draggable viewport
> rectangle for instant jump-to. In Toile this is a pure UI concern — a second small viewport painted last in the
> render loop (§9), fed by the board content bounds (a cheap union already available from the R-tree root AABB,
> `toile-logic §3`), with a drag on the rectangle emitting a local camera move. No CRDT, no Logic change beyond
> exposing the content extent. Toggled from the View menu (§5quinquies.E). Out of MVP scope; see the roadmap.

---

## 4. HIT-TESTING

Input point → `screen_to_world` → R-tree candidate set (`query_point`, `toile-logic §3`) → exact test per
candidate, front-to-back, first hit wins:

```rust
fn exact_hit(elem: &DisplayElement, world_pt: kurbo::Point) -> bool { /* ... */ }
```

Marquee (rubber-band) selection intersects the drag rect against the candidate AABBs, then refines to exact
geometry only for partially covered elements.

**Nested document embeds (`toile-logic §2.I`).** A `doc_embed` is hit-tested like any element for board-level
gestures (select/move/resize). But when it holds **focus**, pointer and keyboard events inside its rect are
**forwarded to the child** (in-place editing, MVP). What is forwarded is the raw `InputEvent`, mapped into the
child's coordinate space — not a command: `ToileView` cannot build the child's `AppCommand` without naming the
child's view type, which is the ARCH P2 cycle. So `ToileView` publishes the focused embed and `FrameHost`, which
owns the child `AppView`, performs the mapping and calls the child's own `handle_input`; the child emits its own
commands against its own `DocumentId`. A single click inside an unfocused embed focuses it (the click itself
does not reach the child); a click outside, or `Escape`, returns focus to the board. Only one embed is focused
at a time.

Focus is **local UI state owned by `ToileView`**, never CRDT and never derived: `derive.rs` builds
`DisplayGeom::DocEmbed` from the `LoroDoc` alone, so it always emits `focused: false` and the view overlays its
own focus on top. A remote peer's focus is not our focus.

`ToileView` does **not** paint the child — it cannot name `AppView` without a dependency cycle
(`toile-logic §2.I` "Render"). It paints the embed's placeholder frame and **publishes** the visible embeds
(id, type, screen-space rect, focused) for `FrameHost`, which owns the child `AppView`s and paints them clipped
over the parent's content pass.

**Comment pins.** `DisplayViewport.comments` (`toile-logic §9`) is hit-tested **before** the element pass — a pin
floats above content (screen-space fixed radius, so it is not scaled by zoom). A click on a pin opens the thread
panel (§5sexies) on that thread; the Comment tool (§5) clicks elsewhere to drop a new one. Pins are painted last,
after the node pass and presence overlay, in the §9 render loop.

**Following a link (`LinkTarget`, `toile-logic §2.K`).** A `DisplayElement.linked` element shows a link
affordance (a small glyph on hover). Activation is modal: with the **Select** tool, `Ctrl/Cmd`-click (or a click
on the affordance glyph) follows the link; in **presentation mode** (`toile-features §1`) a plain click follows
it, since editing is off. Following is entirely local — `Frame` runs a camera fly-to the target frame's bounds,
`Url` is handed to the shell (sandboxed new tab), `Document` asks the shell to open that Ambre document. A plain
Select click still just selects, so linking never blocks editing.

---

## 5. TOOLS (INPUT STATE MACHINE)

A single explicit state machine owns the active tool; each tool consumes pointer events and, on gesture end,
emits one `ToileCommand`. Mid-gesture it streams ephemeral presence (live cursor, in-flight stroke) so
collaborators see the action before it commits.

```rust
pub enum Tool {
    Select,                         // move / resize / rotate / marquee
    Pen,                            // Bezier vector path: click = node, drag = handles (see §5bis)
    Sticky(u32),                    // click-drop a sticky note (ARGB card colour), enter inline edit
    // The colour rides the tool so a click can place the note atomically in the
    // picked colour (CreateElement.fill, logic §2.C). The palette's Sticky button
    // opens a colour picker that arms Sticky(colour); the bare-key chord (N) arms
    // the default sticky yellow. aria-pressed compares by discriminant, so any
    // Sticky(_) reads active regardless of colour (§5quinquies.A).
    Shape(ShapeKind),               // drag-rect to size a rect/ellipse/triangle/line
    Connector,                      // drag from element anchor to element anchor
    Draw,                           // freehand: sample points, stream, CommitStroke on release
    Text,                           // click to place a text element, enter inline edit
    Comment,                        // click an element or empty canvas to drop a comment thread (§4.D, features §4.D)
    Hand,                           // pan-only (space-drag also pans in any tool)
}
```

| Tool | Gesture | Emitted command | Streamed presence |
|---|---|---|---|
| Select | drag body | `MoveElements` | live selection + ghost position |
| Select | drag handle | `ResizeElement` / `RotateElement` | — |
| Select | drag empty | marquee (local) → `SelectionChanged` | — |
| Pen | click / drag | `CreateElement{shape:path}` on close | node preview |
| Sticky/Text | click | `CreateElement` + open inline editor | cursor |
| Shape | drag-rect | `CreateElement` | rubber-band |
| Connector | anchor→anchor | `CreateConnector` | rubber-band from source anchor |
| Draw | press-move-release | `CommitStroke` | per-sample live stroke frame |
| Comment | click element / empty | `CreateComment { anchor, body }` (features §5) then open composer | — |

**Live transform preview.** During a Select move / resize / rotate the UI shows the real object transforming
in place, not just the selection box (Miro parity). The view feeds the in-flight transform into the same
per-element override the Smart-Animate slide pass uses (`AnimatedElement`: bounds + rotation, holding
opacity/fill), so the fill and reflowed text follow the drag every frame. This preview is **100% local** — no
`send_edit` while the pointer is down, so it is instant (<15 ms) and never churns the CRDT. Exactly **one**
`MoveElements` / `ResizeElement` / `RotateElement` still commits on release, preserving one undo item per
gesture (`toile-logic §949/§961`). The selection box + handles follow the preview bounds.

**Snapping & alignment.** During Select drags the UI computes local snap guides (element edges/centers,
frame bounds, grid) and quantizes before emitting — a pure UI concern, never persisted.

**Keyboard.** UI-thread key mappings that emit commands (the UI holds no undo stack itself):

| Shortcut | Command |
|---|---|
| `Ctrl/Cmd+Z` / `Ctrl/Cmd+Shift+Z` (or `Ctrl+Y`) | `Undo` / `Redo` (`toile-logic §8 bis`) |
| `Delete` / `Backspace` | `DeleteElements` on the selection |
| `Ctrl/Cmd+C` / `Ctrl/Cmd+X` | `CopyElements` / `CopyElements`+`DeleteElements` (cut) — §5ter |
| `Ctrl/Cmd+V` | `PasteElements { payload, at: cursor }` (§5ter) |
| `Ctrl/Cmd+D` | `DuplicateElements { ids, delta: small nudge }` |
| `Ctrl/Cmd+G` / `Ctrl/Cmd+Shift+G` | `GroupElements` / `UngroupElements` (§2.F bis) |
| `Ctrl/Cmd+A` | select all (local `SelectionChanged`) |
| Arrow keys / `Shift`+Arrow | `MoveElements` by 1 px / 10 px (nudge) |
| `Escape` | cancel gesture / exit group scope / clear selection / **return to the Select tool** |
| `Space`-drag | pan in any tool |

`Escape` unwinds one layer of transient state per press, most-transient first: a focused embed blurs; then any
in-flight gesture is cancelled and, if a creation tool (Sticky/Text/Shape/Pen/Connector/Draw) is armed, the
active tool reverts to Select (so the armed-but-unused tool does not silently keep placing elements); then an
entered group scope is left; then the selection is cleared. Reverting the tool writes the shared active-tool
signal so the palette's pressed state follows (§5quinquies.A).

Alt-drag on the Select tool duplicates the dragged selection (`DuplicateElements` with the drag delta).

## 5bis. VECTOR PEN & NODE EDITING

The Pen tool builds a `kurbo::BezPath` client-side (nodes + control handles); double-click closes/finishes and
emits a single `CreateElement{ kind: Shape(Path), content: InitContent::Path(bezpath), .. }` (`toile-logic §8`). Editing existing nodes
(select a `shape:path`, drag its nodes/handles) emits `RestyleElements`/geometry updates. Boolean operations
(Union/Subtract/Intersect/Exclude) are computed in `toile-features` and land as a new/edited path element.

---

## 5ter. CLIPBOARD, GROUPING & CONTEXT MENU

**Clipboard (`toile-logic §8 ter`).** Copy is a two-step round-trip: the UI emits `CopyElements { ids }`, Logic
replies `ToileEvent::Clipboard(payload)`, the UI stashes the `ClipboardPayload` in memory **and** mirrors a
serialized copy to the OS clipboard (an app-private MIME type) so paste survives across tabs. Paste emits
`PasteElements { payload, at }` with `at` = the cursor (or viewport center on a keyboard paste); Logic re-mints
ids and returns the new selection via `ElementCreated`/`SelectionChanged`. Cut = copy then `DeleteElements`.
Duplicate (`Ctrl+D` or alt-drag) is a single `DuplicateElements`. The UI never re-mints ids or rewrites the
CRDT itself — it only carries the opaque payload.

**Grouping.** `Ctrl+G` on a multi-selection emits `GroupElements { ids }`; `Ctrl+Shift+G` on a selected group
emits `UngroupElements { group }`. A `DisplayGeom::Group` node paints nothing (`toile-logic §9`); the UI uses it
only to resolve selection, walking the `DisplayElement.group` edge (the immediate enclosing group, carried on
every member — `toile-logic §9`) upward through the group chain: a single click on any member selects the
**top-most enclosing group** (its union bounds get the transform handles), and a **double-click enters the group
scope** (a local, never-synced UI field) so the next click selects the member one level inside it -- peeling one
nesting level per double-click (`Escape` leaves the scope). Marquee still selects members directly; a transform
on a selected group is one `MoveElements`/`RotateElement` over the group element, which carries its children for
free.

**Context menu.** Right-click (or long-press) over the selection opens the `ambre-chrome` `ContextMenu` (DOM,
not canvas — §1). Entries map one-to-one to existing commands, greyed out when inapplicable:
Cut / Copy / Paste / Duplicate (§5ter), Delete (`DeleteElements`), Group / Ungroup (`GroupElements` /
`UngroupElements`), Bring to Front / Send to Back / Forward / Backward (`Reorder` with the matching `ZTarget`),
Lock / Unlock (`LockElements`), Add to / Remove from frame (`SetFrameChildren`), Align / Distribute
(`ArrangeElements`, enabled on a multi-selection), Add / Edit / Remove link (`SetLink`, single selection), and
Comment (`CreateComment` anchored to the element). On empty canvas the menu offers Paste, Select-All, and Comment
here (a `Point`-anchored thread).

---

## 5quater. FIND (SEARCH ON BOARD)

`Ctrl+F` opens a **DOM chrome panel** (`ambre-chrome` `FindPanel`, a floating panel pinned to a board corner,
the same plumbing as the context menu §5ter) — never a canvas `DrawSurface` overlay. Because the UI holds only
the culled `DisplayViewport`, a match on an off-screen sticky is invisible to it, so the search runs in the
**Logic thread**: the panel emits `Find { query, options, from, backward }` and Logic replies
`ToileEvent::FindResults { hits, focus }` (`toile-logic §8 quater`). The panel highlights the visible `hits`
and recenters the camera on `focus`; `Enter` / `Shift+Enter` step forward / backward through the hits (each a
fresh `Find` with `from` = the current match and the matching `backward` flag, Logic doing the wrapping).
`Escape` or the close button dismisses the panel and clears the highlight; the highlight also clears when the
query changes.

Options are local Leptos signals on the panel: `match_case`, `whole_word`. Matches and the highlight are
**local view state**, never synced and never an undo item. **Replace is out of MVP** (marginal on a free-form
canvas — `toile-logic §12`); the panel is find-only.

---

## 5quinquies. CHROME SURFACES (DOM)

All the persistent app furniture around the `<canvas>` is DOM/Leptos built on `ambre-chrome` (§1bis), never
canvas-drawn. Each surface is a thin reactive view over `ambre-commands` registry state: a control reads a
`CommandState` (enabled/active/value) and, on activation, emits the command's `ToileCommand` through the same
`command_to_app` path as a keybinding (§1bis). No chrome surface owns document state or an undo stack.

### A. Tool palette (primary toolbar)
A vertical (or floating) `ambre-chrome` `Toolbar` exposing one button per Toile tool — Select, Vector Pen,
Shape, Connector, Draw (freehand), Sticky, Text, Comment (§5) — plus zoom/hand affordances. Each button is a `Command`
whose `CommandState.active` reflects the current `Tool`, so exactly one tool reads as selected; clicking emits
`SetTool` (the same signal the keyboard shortcut resolves to). The canvas-side `ToolMachine` stays the sole
consumer of the active tool (§1bis); the palette only drives the selection. Tool sub-options that are not
per-element style (e.g. the Shape kind, the connector routing mode) are local palette signals folded into the
`CreateElement` the tool emits.

**Sticky colour picker.** The Sticky button is a split affordance: clicking it opens a small colour popover (a
row of pastel swatches from the app token palette). Picking a swatch arms `Tool::Sticky(colour)` and closes the
popover; the click then places notes in that colour (folded into `CreateElement.fill`, `toile-logic §2.C`),
atomically — no create-then-restyle. The colour is remembered as the palette's live sticky colour, so the next
sticky reuses it until changed. The bare-key chord `N` arms the default sticky yellow without opening the
popover. This mirrors the Miro / Figma-whiteboard sticky flow.

**Cursor feedback.** The canvas cursor reflects the active tool so the user sees what a click will do: a
**note-shaped cursor** while the Sticky tool is armed (an inlined SVG data URI), a crosshair for the other
creation tools (Text/Shape/Pen/Connector/Draw), a grab hand for the Hand tool, and the default arrow for Select.
The cursor is a pure function of the active-tool signal (`cursor_css`, host-tested), applied as a CSS cursor on
the canvas element by the chrome — it never touches the canvas render pass.

On the Select tool the cursor is additionally a function of the **pointer position** over a single selection
(Miro parity): hovering a side shows the directional resize cursor for that axis (`ns-resize` / `ew-resize`),
a corner shows the diagonal (`nwse-resize` / `nesw-resize`), and the rotate handle shows a **rotation cursor**
(an inlined SVG: a nearly-closed circle with an arrow head) — the
whole edge is grabbable even though only the corners draw a square (§7); a resize / rotate / move in flight
holds the matching cursor (`grabbing` for a move) for the whole gesture. The view publishes this
position-dependent cursor into a shared `Option<Cow<'static, str>>` signal (`resize_cursor`, host-tested); the
chrome's cursor effect prefers it and falls back to `cursor_css` when it is `None`. For a **single** selected
element the box, its handles and the resize cursors all **rotate with the element** (the camera carries no
rotation, so the element's world angle is also its screen angle): the box is drawn as four rotated edges
(`stroke_line`, since `stroke_rect` degrades a rotated rect to its upright AABB on the WgpuSurface), its handles
sit at the rotated corners, and hit-testing de-rotates the pointer into the element's frame. The directional
resize cursor also tilts to the exact angle: an axis-aligned element uses the crisp native CSS keyword
(`ns-resize` / `nwse-resize` / ...), while a rotated one uses a **custom SVG cursor** -- the Lucide
"move-horizontal" double arrow rotated to `resize_axis_base(handle) + rotation` -- because native resize cursors
are fixed OS bitmaps that cannot rotate; the nearest CSS keyword rides along as the data-URI fallback. A
**multi-selection** keeps an axis-aligned union box.

### B. Style / property inspector
A DOM side panel, shown only when the selection is non-empty, that edits the selected elements'
`ElementStyle` (`toile-logic §2.E`): fill (background), stroke, `stroke_w`, `opacity`, plus a **Text** section —
text (foreground) colour (`text_color`), font family, size, and the four decorations bold / italic / underline /
strikethrough. The whole text cluster is built from the shared `ambre_chrome` controls the floating bar
(§5quinquies.D) and Trame's cell toolbar reuse: `FontPicker` (family; a label resolves to the same face via
`ambre_canvas::registered_family_name`), `FontSizeSelect` (size), and `FormatToggle` (each decoration). The size
dropdown's "Auto" entry maps to the sticky auto-size sentinel and a chosen size sets a positive override
(`toile-logic §2.E` / §6). Edits are debounced and
emitted as one `RestyleElements { ids, style }`
over the whole selection (never per-attribute — the register is written whole, matching the single-register
model of `toile-logic §2.E`); text-run edits route through `RestyleElements` or `EditText` as in §6. The panel
also surfaces the z-order actions (Bring to Front / Send to Back / Forward / Backward -> `Reorder` with the
matching `ZTarget`) and Lock / Unlock (`LockElements`), mirroring the context menu (§5ter). Values shown are
the common value across the selection, or a mixed-state indicator when they differ. The panel is pure view
state; it holds no authoritative style.

On a **multi-selection** the inspector also shows an **Arrange** row — align Left/HCenter/Right/Top/VCenter/
Bottom and distribute Horizontal/Vertical — each button emitting one `ArrangeElements { ids, op }`
(`toile-logic §8`); the distribute buttons enable only at >= 3 elements. On a **single** selection it shows a
**Link** control (`toile-logic §2.K`): a small popover to set/clear a `LinkTarget` (URL, a frame on this board
picked from a list, or another document), emitting `SetLink { id, link }`. Both mirror entries also present in
the Object menu and context menu (§5ter/§E) so the surfaces never diverge.

### C. Color picker
A reusable `ambre-chrome` popover component consumed by the inspector (fill / stroke) and the text-run
controls. It edits an ARGB `u32` (the `ElementStyle` color encoding, `toile-logic §2.E`) with a swatch grid on
the app's token palette plus a custom hue/alpha picker. Committing a color folds into the same debounced
`RestyleElements`; there is no separate color command.

### D. Content-anchored floating selection toolbar
The compact action bar that follows the selection as the board pans/zooms — the case `ambre-spec-ui.md L31`
names for Toile (§1bis). It is a DOM element whose screen position is recomputed from `ViewportTransform`
**every frame** (transform-synced), anchored to the selection's world-space bounds. It offers the highest-
frequency actions: a **text-format cluster** in the same order as Trame's toolbar — bold / italic / underline /
strikethrough (shared `ambre_chrome::FormatToggle`), font family (shared `FontPicker`), size (shared
`FontSizeSelect`), text colour and fill (shared `ColorPopover`, ARGB) — seeded from the same common-style
`StyleModel` the side inspector reads (§5quinquies.B), followed by command buttons (duplicate, delete,
group/ungroup, lock) — a superset-free subset of the context menu (§5ter) so the two never diverge. Each style
control emits ONE changed field (`ui::floatbar::StyleEdit`); the view re-derives the selection's common style
**live** and overwrites only that field, so back-to-back bar edits never clobber one another (unlike the
inspector's accumulating working copy), folding the result into one whole-register `RestyleElements` (§2.E). It
hides during an active gesture (move/resize/rotate) and when the selection is empty.

### E. Top menu bar & zoom control
A top `ambre-chrome` `Menu` bar for low-frequency actions grouped by menu (File / Edit / View / Object /
Present), each entry a registry `Command` with its keybinding shown — including the entries that have no
palette button (Undo/Redo -> `ToileCommand::Undo`/`Redo`, Select All, the presentation entry point that opens
the sequence panel of `toile-features §9`). The **Object** menu carries Align / Distribute (`ArrangeElements`)
and Add / Edit / Remove link (`SetLink`); the **View** menu toggles the comments panel (§5sexies) and the
minimap (§3, deferred). A **zoom control** (zoom out / % / zoom in / Fit / Zoom-to-
selection) lives here and mirrors the `zoom %` the status bar reports (§8bis); zoom changes update
`ViewportTransform` locally and forward `SetViewport` to Logic (§3), never a round-trip for the transform
itself.

> **Scope note (MVP).** These surfaces are the editing chrome that Focus Mode hides (`toile-features §1.B`).
> The share dialog is the app-agnostic chrome already delivered under `E2` (Document sharing UI); the present/
> playback controls and presenter overlay panel are owned by `toile-features §9`, not duplicated here.

## 5sexies. COMMENTS PANEL (DOM)

A `ambre-chrome` side panel for the anchored comment threads (`toile-features §4.D`), same plumbing as the Find
panel (§5quater) — DOM/Leptos over the command channel, never a canvas overlay. It opens when a comment **pin**
is clicked (§4), when the Comment tool drops a new thread, or from the View menu (§E, board-wide list). It shows
the selected thread's messages (author, relative time from the display-only `ts`, body) with a composer, and a
Resolve/Reopen toggle (`ResolveThread`); the board-wide mode lists all threads filterable by open/resolved and
recenters the camera on a thread's anchor when picked. Posting/replying/editing emit `CreateComment` /
`ReplyComment` / `EditComment`; the panel re-reads a thread on `ToileEvent::CommentsUpdated` and opens the
composer on `CommentThreadOpened`. Pin badges (message count, resolved state) come from `DisplayViewport.comments`
and need no panel read. The panel holds no authoritative state.

---

## 6. INLINE TEXT EDITING (PARLEY)

Sticky and text elements edit in place: the UI opens a Parley-backed editor over the element, maps keystrokes to
`TextDelta`s, and emits `EditText { id, delta }` so the change flows through the element's `LoroText` (character
CRDT, `toile-logic §2.D`). Layout — line breaking, run styling, cursor/selection geometry — is Parley; glyphs
are rasterized by Swash into the shared atlas. The formula-bar/toolbar equivalents for rich text (bold, size,
color) mutate `ElementStyle`/runs and emit `RestyleElements` or `EditText`.

**Entering and leaving an edit.** Placing a sticky/text element opens its editor immediately and reverts the
active tool to Select, so the new element is selected and ready to type into rather than the next click dropping
another one (§5). The caret **blinks** (~1 s cycle) to signal the field is editable. Opening the editor makes the
edited element the selection, so leaving the edit keeps it selected:
- **Escape** commits and closes the editor (the caret disappears) but the element **stays selected** — it does
  not clear the selection or step out further; a second Escape then acts on the selection as usual (§5).
- With a single sticky/text element selected and no editor open, **typing a printable character** enters edit
  and **replaces** the whole body (Trame-cell parity, `trame-ui §5bis`). To edit the existing body instead,
  **double-click** it (which opens the editor with the body preserved, caret at end). Space is excluded from
  type-to-edit (it pans).

A sticky's body is drawn in the fixed readable ink (its `fill` is the card colour, not the text) at a size that
tracks the card (`render::sticky_font_size`), so the in-edit text matches the committed look.

**Connector labels (`toile-logic §5.B`).** `DisplayConnector.label` is painted as text centered on the routed
path midpoint (a small opaque pill behind it for legibility over crossing edges). Double-clicking a connector
opens the **same** Parley inline editor over that midpoint and emits `EditConnectorLabel { id, delta }` — the
connector's own `LoroText`, so two authors captioning one edge converge char-wise. An empty label paints nothing.

**Comment composer & messages.** The comment panel (§5sexies) reuses the same Parley editor for composing and
editing a message body; posting emits `CreateComment`/`ReplyComment` and self-edits emit `EditComment` (features
§5). A message body is a `LoroText`, so a self-edit converges like any other rich text.

---

## 7. SELECTION & TRANSFORM HANDLES

Selection is **local UI state** (never persisted; a peer's selection travels as presence). A selected set draws
a bounding box with a rotate handle and **4 corner resize squares**; multi-select transforms apply
proportionally. Only the corners draw a visual square — kept deliberately light — but the **whole edge** is
grabbable (Miro): a press anywhere along a side starts an axis resize, a corner a diagonal one. The band-based
hit-test (`edge_handle_at`, host-tested) does the picking and also drives the hover cursor (§5quinquies.A), so
horizontal / vertical resize stays available without cluttering the box with mid-side squares. Locked
elements (`DisplayElement.locked`, the top-level flag — `toile-logic §2.B`, not an `ElementStyle` member) are
skipped by hit-test and show no handles. A click that hits a grouped element selects the whole group (its union
bounds carry the handles); double-click enters the group to reach a member — the resolution rule of §5ter.

---

## 8. MULTI-USER PRESENCE (RENDER)

Presence arrives as `ToileEvent::PeerPresence` (ephemeral, `toile-logic §7`) and is painted in the top overlay
pass, above all content, never culled out:

- **Cursors:** each peer's pointer with a name label and a stable color. The color and label key off the
  peer's canonical **`UserId`** (resolved from its `pub_key` in `ambre-sync`, `toile-logic §7`), **not**
  `PeerID`, hashed with the shared cross-app algorithm (`ambre_draw::peer::peer_color`, the render-primitives crate
  every app's canvas already depends on -- the eventual `prose-ui` home the spec named does not exist yet, and
  the algorithm belongs in the shared draw layer) so the same collaborator keeps one color across Prose, Trame
  and Toile — and a single color even when signed in from two devices.
- **Live strokes:** an in-flight freehand stroke from a remote peer, rendered incrementally from streamed
  `StrokePoint` samples until the peer's `CommitStroke` turns it into a real `"drawing"` element.
- **Laser pointer / highlighter:** a temporary, fading trail (the brainstorming tool from Toile Todo §2.C) —
  purely ephemeral, decays on a timer, never a CRDT element.
- **Selection ghosts:** a peer's current selection outline in their color.

Presence render must be cheap: cursors/labels are instanced quads + cached glyph runs; laser trails are a small
ring buffer of points stroked by `lyon`.

---

## 8bis. STATUS BAR (SELECTION AGGREGATES)

A thin **DOM chrome band** (`ambre-chrome` `StatusBar`, like every other chrome surface — §1) along the bottom
edge shows aggregates of the current selection, the whiteboard analog of Trame's status bar (`trame-ui §13bis`):
**element count** and the selection's **combined bounds** (x / y / w / h in world units), plus the current
**zoom %**. The selection can exceed the viewport (culled elements are absent here), so on a non-trivial
selection change the UI emits `RequestSelectionStats { ids }` and the **Logic thread** computes
`ToileEvent::SelectionStats { count, bounds }` from its geometry cache (`toile-logic §8 quinquies`); an empty
or single-element selection shows only the zoom chip. The same band hosts the perf HUD (fps + frame timings),
kept off the WebGPU overlay so it stays DOM-selectable and theme-aware.

---

## 9. RENDER LOOP & DIRTY FLAG

A stable `requestAnimationFrame` loop paints only when a dirty flag is set: a new `DisplayViewport`, a local
transform change (pan/zoom), an in-flight gesture, or a live presence update all mark dirty. Idle boards cost
zero GPU work. (This mirrors the renderer-stack discipline already validated on the project: WebGL2 fallback
concerns are handled at the `WgpuSurface` layer, not here.)

**Paint order per frame:** (1) the culled `nodes` pass (elements + connectors, incl. connector labels §6),
back-to-front by z-order; (2) the presence overlay (§8); (3) the **comment pins** from
`DisplayViewport.comments`, screen-space fixed size, always on top so a pin is never occluded by board content
(`toile-logic §9`). The optional minimap (§3, deferred) paints last, in its own corner viewport.

---

## 10. QUALITY & TESTABILITY

> What this module guarantees, and what verifies each guarantee, is the table in §1. This section used to enumerate in prose the cases that "must" be unit-tested -- a paragraph nothing can check, in a module that had done the work: 164 host tests across `ui::viewport`, `ui::hit`, `ui::tool`, `ui::render`, `ui::embed`, `ui::presence`, `ui::menu`, `ui::menu_bar`, `ui::floatbar`, `ui::palette`, `ui::grid`, `ui::style` and `ui::status`.

* **Tiers.** **host** for everything above -- viewport transforms, hit-testing, the tool state machine, snapping, the render walk through `RecordingSurface`, the embed transform and session lifecycle, the chrome models. All of it is `cfg`-agnostic geometry and decision logic, which is why it runs on the fast tier. **wasm** for the `WgpuSurface` path, Parley layout on the GPU surface, and end-to-end input->command. No **db** tier.
* **Seams.** `CommandSink` to capture emitted commands; a fake `DisplayViewport` feed to drive paint tests; `RecordingSurface` for the draw-call sequence. Headless wasm for the GPU path, no live network.
* **Mandatory belts.** A new interaction lands with (a) a geometry round-trip -- drawn rect against hit-test, under zoom and rotation (`TOILE-UI-002`, `TOILE-UI-005`), (b) a gesture test asserting the exact command emitted **and** that a no-movement click emits nothing (`TOILE-UI-008`), and (c) a `RecordingSurface` assertion for anything it paints.
* **CRDT convergence.** N/A -- this module holds no CRDT state (`TOILE-UI-001`).
* **Benchmarks.** None today. §11's budgets -- a 2 000-element frame paint, tessellation cache hit rate over a zoom sweep, presence overlay at 20 cursors -- have no `criterion` bench, which is also why `TOILE-UI-021` is `unverified`.
* **Definition of Done.** An increment is `[x]` only when those belts are green in CI **and** no invariant in §1 is left without a `Status`.

---

## 11. PERFORMANCE

| Metric | Target | Rationale |
|---|---|---|
| Frame time (2 000 visible elements) | < 16 ms | 60 FPS with headroom (mobile-first pillar). |
| Pan/zoom input-to-paint latency | < 15 ms | Local transform, no Logic round-trip (§2). |
| Tessellation cache | retessellate only on tolerance-bucket change | Avoids per-zoom-delta CPU churn (§2bis). |
| Presence overlay (20 cursors + trails) | < 1 ms/frame | Instanced quads + small ring buffers (§8). |
| Idle board | 0 GPU passes | Dirty-flag gated rAF (§9). |
| Heavy path (> segment threshold) | never stalls a frame | Incremental tessellation + bbox placeholder (§2bis). |

**WASM impact:** the heavy render crates (`wgpu`/`lyon`/`parley`/`swash`) are linked **once in `ambre-canvas`**
and shared with Prose and Trame — Toile adds no new GPU dependency. Toile-ui's own contribution is thin (the
content walk, tool state machine, chrome components); the glyph atlas and tessellation caches it consumes are
`ambre-canvas` heap, counted there. Kept within the < 2 MB gzipped total budget.

---

## 11bis. SOURCE OF TRUTH MODEL BY STATE TYPE

A state can exist in several layers; the rule is **a single source of truth, every other layer is a projection
of it** (the Trame model, `trame-ui §16`). Toile has an unusually wide state surface, so the split is worth
stating explicitly:

| Category | Examples | Source of truth | Synced |
|---|---|---|---|
| **Document mutation** | create/move/resize/restyle/delete, connectors, frame & group membership, `layers` z-order, notes | `LoroDoc` (Logic thread) | Yes (CRDT delta) |
| **Ephemeral presence** | remote cursors, in-flight strokes, laser trails, peer selections | Sender's Logic thread → ephemeral Sync payload | Yes (ephemeral, never persisted) |
| **Local view** | camera (pan/zoom), local selection, active tool, group-enter scope, inline-editor state, find query + highlight, clipboard stash | UI state (some mirrored to IndexedDB / OS clipboard) | No |
| **Derived state** | `SpatialIndex`, `bounds` AABBs, materialized `layers`, the culled `DisplayViewport`, `SelectionStats` | Logic thread (rebuilt from `LoroDoc`) | No |
| **Local history** | undo / redo stack (`loro::UndoManager`) | Logic thread (in memory, this session) | No |

**Fundamental invariant:** the UI thread holds **no** document data — only view state and the `DisplayViewport`
pushed by Logic. Every mutation goes through a `ToileCommand`; there is one mental model of the board, and
`LoroDoc` is it.

**`ViewportPatch` — Phase 2 optimization.** Today Logic re-derives and pushes a full culled `DisplayViewport`
on each relevant change. When a remote delta touches only a few visible elements, Logic may instead send a
differential patch (the changed `DisplayElement`s + removals), which the UI applies onto the current viewport —
the same trade-off as Trame's `ViewportPatch` (`trame-ui §16`), and the analog of shrinking `DataReset` to an
invalidated range (`trame-logic §11 bis.A`). Full-viewport push stays the fallback; the patch/full decision
lives in Logic, keyed on change size.

---

## 12. REJECTED ARCHITECTURAL DECISIONS

| Rejected | Reason |
|---|---|
| **DOM / SVG overlay for text or cursors** | Breaks the single WebGPU pipeline and the 60 FPS budget (core pillars). Parley+Swash render text on the GPU. |
| **Round-tripping pan/zoom through Logic before painting** | Adds latency to every gesture; the transform is local, only culling is delegated (§2). |
| **Retessellating paths every zoom frame** | CPU blow-up; tolerance-bucketed cache is sufficient (§2bis). |
| **Selection / laser as CRDT state** | Ephemeral, per-user, must not persist or enter history — presence only (§8). |
| **Iterating the whole document per frame** | Impossible at 50 000 elements; the render thread paints only the culled `DisplayViewport` (§1). |
| **Re-implementing the GPU pipeline (own wgpu/lyon/swash stack)** | Duplicates `ambre-canvas`. Toile paints through `WgpuSurface: DrawSurface` and reuses `QuadRenderer`/`GlyphAtlas` (§1, `ambre-spec-canvas.md`). |
| **Choosing the GPU backend here** | Backend selection (WebGL2 by default for the Mesa/Intel compositor bug, WebGPU opt-in via `?webgpu`) is owned by `ambre-canvas::bootstrap_gpu`, not Toile. Our `DrawSurface` calls are backend-agnostic. |

---

## 13. INTEGRATION PLAN (ROADMAP)

> Tracked as Stories of Epic **E4** (Toile MVP) in `specs/ambre-roadmap.md`.

**Phase 0 - Core standalone**
- [x] `ViewportTransform` (pan / zoom, screen <-> world) with host tests
- [x] Batcher + quad pipeline + `lyon` path pipeline on `WgpuSurface`
- [x] Adaptive grid rendering
- [x] Exact hit-testing (geometry-level, host-tested)
- [x] The Select tool + tool state machine, host-tested

**Phase 1 - Integration**
- [x] Remaining tools: Pen, Shape, Connector, Draw, Sticky, Text (`ToolMachine`, host-tested)
- [x] Parley inline text editor
- [x] Keyboard shortcuts + clipboard round-trip (OS-clipboard mirror). Shared, Logic-authoritative
  clipboard model (`1-Foundations/ambre-spec-ui.md §E`); shortcuts decoded in `handle_key`.
- [x] Grouping selection-scope resolution (via the `DisplayElement.group` edge, host-tested — §5ter/§7)
- [x] Tool palette / primary toolbar wired to tool selection + active-tool state (§5quinquies.A).
  Floating vertical `ToolPalette` (`chrome::palette`) sharing the Trame command-bar styling; one
  button per tool with `aria-pressed` reflecting the current `Tool`; clicking (or the bare-key chords
  V/P/R/C/D/N/T/H, guarded against inline-editor typing) writes a shared `RwSignal<Tool>` the canvas
  view applies each frame. The active tool is UI-only, canvas-local state (§11bis), so it is not a
  `ToileCommand`; the pure command surface (ids, chords, id->Tool map, focus guard) lives in the
  host-tested `ui::palette`.
- [x] Style / property inspector -> debounced `RestyleElements` + z-order / lock (§5quinquies.B).
  DOM side panel `StyleInspector` (`chrome::style`), shown only for a non-empty selection. It edits
  fill / stroke / stroke width / opacity of the selected elements' `ElementStyle`; edits are coalesced
  by a shared `ambre_chrome::Debouncer` and written whole (§2.E) into a `RwSignal<Option<ElementStyle>>`
  the canvas view drains into one `RestyleElements { ids: selection, style }` (the view attaches the
  UI-only selection). Z-order (Bring to Front / Forward / Backward / Send to Back) and Lock / Unlock
  reuse the existing menu `CommandId`s through the shared `on_command` channel, mirroring the context
  menu (§5ter). The selection's common style + per-field "mixed" mask is a pure, host-tested surface
  (`ui::style::style_scope_for`) the view publishes alongside the context-menu scope.
- [x] Color picker popover (ARGB, shared by inspector + element fills) (§5quinquies.C). The shared control
  is delivered: `ambre_chrome::ColorPopover` (promoted from Trame's cell fg/bg picker) now takes an
  `alpha` flag and, in ARGB mode, adds an opacity slider; Trame's `FormatBar` and the Toile inspector
  both consume it. This closes the MVP scope. *(The **text-run** colour consumer is intentionally out of
  the MVP: rich text runs (§6) ride `EditText`/`TextDelta`, deferred to `E5` with Prose's shared text CRDT
  — see the `E5` story below. `FontSpec` gains its colour field there.)*
- [x] Content-anchored floating selection toolbar, transform-synced each frame (§5quinquies.D).
  DOM chrome `SelectionToolbar` (`chrome::floatbar`): a `position: fixed` bar the canvas view
  republishes every frame from the live selection x camera (`ui::floatbar::build_floatbar`), so its
  client anchor (selection top-edge midpoint, lifted with a CSS gap) follows pan/zoom. It hides on an
  empty selection, an active gesture (move/resize/rotate), a pan, or an open inline editor. Actions are the
  text-format cluster in Trame's toolbar order -- Bold / Italic / Underline / Strikethrough (shared
  `ambre_chrome::FormatToggle`), font family (shared `FontPicker`), size (shared `FontSizeSelect`), text colour
  and fill (shared `ColorPopover`, ARGB), seeded from the inspector `StyleModel` (§5quinquies.B) -- plus
  Duplicate / Delete / Group / Ungroup / Lock / Unlock, the latter each a `ui::menu` `CommandId` on the shared
  `on_command` path, a strict subset of the context menu (§5ter) so the two never diverge. Each style control
  emits ONE changed field (`ui::floatbar::StyleEdit`); the view re-derives the selection's common style live
  (`style_scope_for`) and overwrites only that field, folding it into one whole-register `RestyleElements`
  (§2.E) -- so successive bar edits never clobber one another. The pure placement + flags + common-fill fold
  live in the host-tested `ui::floatbar`.
- [x] Top menu bar + zoom control (Fit / Zoom-to-selection) (§5quinquies.E). DOM chrome
  `TopMenuBar` (`chrome::menu_bar`): the File/Edit/View/Object/Present menus render through the
  shared `ambre-chrome` `ContextMenu` (opened at each title), and the zoom cluster (out / `%`
  readout / in / Fit / Zoom-to-selection) carries the `.ambre-commandbar` styling of the tool
  palette. Every entry routes through the same `on_command` -> `toile_menu_cmd` channel as the
  context menu (§5ter); zoom actions mutate `ViewportTransform` locally (`zoom_by_center` /
  `reset_zoom` / `fit_bounds`) and forward `SetViewport`. Fit frames the whole board via an
  empty-ids `RequestSelectionStats` (Logic returns the union of all bounds); Zoom-to-selection
  frames the selection. The `%` readout mirrors the live `zoom` signal the view publishes each
  frame. Host-tested pure surface (`ui::menu_bar` layouts + `viewport` zoom/fit geometry).
- [x] `ambre-chrome` context menu (§5ter). Right-click on the canvas selects the topmost hit (group-
  resolved, §7) or clears to the empty-canvas scope, then publishes a pure `CtxScope` the DOM menu reads;
  the shared `ambre-chrome` `ContextMenu` renders the iconed entries `chrome::context_entries_for` builds
  from that scope (inapplicable rows omitted -- Group only for multi, Ungroup only with a group selected,
  Lock/Unlock by state, frame entries only inside a frame), and each row forwards its bare `CommandId`
  through the shared `on_command` sink into the view, which resolves it against its UI-only selection via
  `ui::menu::resolve_menu_command` (`drain_menu_cmd`/`apply_menu_command`). Paste + Select-all are UI-only
  intents serviced in the view. Pure surface host-tested in `ui::menu`; DOM/input covered by the wasm belt.
- [x] Find panel (§5quater). DOM chrome `FindPanel` (find-only) + `ToileCommand::Find` wiring;
  canvas-side highlight of visible hits and camera recenter on `focus` (via a `RequestSelectionStats`
  bounds lookup, since `FindResults` carries no geometry). Host-tested step/index helpers (`ui::find`).
- [x] Selection status bar (§8bis). DOM chrome `StatusBar`: selection count + combined world bounds on
  the left, shared interactive zoom + fps chips on the right, styled identically to Trame's band. The
  zoom/fps chips and app label are the shared `ambre-chrome` status toolkit (`ZoomChip`/`PerfChip`/
  `StatusLabel`), reused by both apps. The canvas view re-requests `SelectionStats` on each selection
  change and clears the band on an empty selection. Host-tested count/bounds formatting (`ui::status`).
- [x] Presence overlay wired to `PeerPresence` with cross-app `UserId` colors (§8). `ui::presence::PeerCursors`
  is the render-thread mirror of every peer's latest presence, fed by `ToileEvent::PeerPresence` (keyed by the
  spoof-proof canonical `UserId`, §7) and painted in the overlay pass above content: selection ghosts, in-flight
  live stroke, a recency-faded laser trail (a per-peer ring buffer, decayed by sample age so it needs no wall
  clock), and the pointer arrow + name chip. Every peer colour comes from the shared cross-app
  `ambre_draw::peer::peer_color` (a pure hash over the `UserId` seed bytes, so a collaborator keeps one colour across
  Prose / Trame / Toile). The paint entry point is pure over `DrawSurface`, host-tested against
  `RecordingSurface` with synthetic `PeerPresence` events (arrow/chip, ghosts of visible ids only, live-stroke
  polyline, laser accumulate/clear, distinct per-peer colours). The **end-to-end multi-user demo** is still
  **blocked on SYNC P4** (`pub_key` -> `UserId` resolution): until it lands, no remote frame reaches this thread,
  so the pass runs against synthetic events only (`toile-logic §9`).
- [x] wasm render / input tests. `crates/ambre-toile/tests/chrome_ui.rs` (headless Chrome via
  `just test-wasm`) mounts the real DOM chrome and asserts render + input: the selection status band
  (§8bis), the context menu (§5ter) end-to-end (the rows each `CtxScope` renders + a row click firing its
  `CommandId`), the tool palette switching the shared active tool (§5quinquies.A), the top-menu zoom
  cluster (§5quinquies.E) and the floating selection toolbar (§5quinquies.D) firing on the shared sink.
  The pure models stay host-tested in `ui::{status,menu,menu_bar,palette}`.

**Phase 1c - Miro-parity UI gaps (MVP)**
- [ ] Comment tool + pin render/hit-test + comments panel (`§4`/`§5sexies`) driven by `DisplayViewport.comments`
- [ ] Connector-label render at path midpoint + double-click inline edit -> `EditConnectorLabel` (`§6`)
- [ ] Link affordance + follow (`Ctrl`-click / present-mode click): camera fly-to frame, shell open url/doc (`§4`)
- [ ] Arrange row in the inspector + Object menu -> `ArrangeElements` (`§5quinquies.B`)

**Phase 2 - Optimization**
- [ ] Glyph-atlas eviction
- [ ] Tessellation-cache tuning
- [ ] Presence render batching
- [ ] Large-board frame-time profiling
- [ ] Minimap overview + jump-to (`§3`, deferred)
