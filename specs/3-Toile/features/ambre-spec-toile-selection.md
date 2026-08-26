# AMBRE -- FEATURE: SELECTION ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- picking, the marquee and the handles are built; a connector cannot be picked.

> A user clicks an element and gets it, not the one behind it. Dragging over empty board sweeps up
> what it covers. What is selected wears a box, four corner squares and a rotate handle, tilted with
> the element.

Selection is local view state: never persisted, and a collaborator's arrives as presence rather than
as a document read (`features/ambre-spec-toile-presence.md`). There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-005` | A pick MUST be exact -- rotation, path fill and stroke width -- over the culled candidate set, and MUST return the front-most element. | this spec section 2 | `ambre_toile::ui::hit::tests::rect_inside_and_outside`, `::rotated_rect_uses_local_frame`, `::ellipse_corner_is_a_miss`, `::triangle_fill`, `::open_path_hits_near_the_stroke_only`, `::stroke_proximity`, `::topmost_wins_front_to_back` | verified |
| `TOILE-UI-006` | A locked element and a group MUST NOT be picked directly. | this spec section 2; `features/ambre-spec-toile-elements.md section 3` | `ambre_toile::ui::hit::tests::locked_never_hits`, `::group_never_hits` | verified |
| `TOILE-UI-007` | A click MUST resolve to the top-most enclosing group unless the user has entered that group's scope. | this spec section 2 | `ambre_toile::ui::tool::tests::click_on_group_member_selects_top_most_group`, `::scoped_click_selects_the_member_inside_the_scope`, `::ungrouped_element_resolves_to_itself`, `::pointer_down_on_a_member_selects_the_group` | verified |
| `TOILE-UI-031` | A marquee MUST take every element it covers, and MUST skip a locked element and a group. | this spec section 2 | `ambre_toile::ui::hit::tests::marquee_selects_covered_elements`, `ambre_toile::ui::tool::tests::drag_empty_makes_a_marquee_then_selects` | verified |
| `TOILE-UI-032` | A double-click MUST enter one nesting level of the group under it, and `Escape` MUST leave the scope. | this spec section 3 | partial: `ambre_toile::ui::tool::tests::scoped_click_selects_the_member_inside_the_scope` covers what a scope does to a click; nothing covers entering it or leaving it, both of which live on the view | unverified |
| `TOILE-UI-033` | Select-all MUST cover the whole board, not the part the UI thread holds. | this spec section 3; section 4 | none. The walk is over the culled set, so **the rule fails off screen** -- section 5 | unverified |
| `TOILE-UI-034` | The selection box and its handles MUST follow the element's rotation, and the whole edge MUST be grabbable even though only the corners draw a square. | this spec section 3 | `ambre_toile::ui::tool::tests::edge_handle_at_picks_sides_corners_and_misses`, `::hover_handle_follows_the_element_rotation`, `::hover_handle_only_for_a_single_select_at_rest`, `ambre_toile::ui::render::tests::overlay_rotates_box_and_handles_for_a_rotated_element` | verified |
| `TOILE-UI-035` | A connector MUST be pickable, so it can be selected, styled and captioned. | this spec section 2; `features/ambre-spec-toile-connectors.md section 3` | not built -- lands with the "Connector labels" Story (`ambre-roadmap.md` E4) | unimplemented |

* **Budget.** the content hit-test and input dispatch under 0.2 ms, one row of
  `ambre-spec-budgets.md section 2`. It is a walk over the culled set, which the spatial index
  already bounds (`TOILE-005`), and no bench measures it.
* **Boundary.** none: a selection is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. It leaves the client only as presence, which the Sync worker
  seals like any other frame (`ZK-005`).
* **Verification.** 5 `verified`, 2 `unverified`, 1 `unimplemented`. The geometry is held eleven ways
  and the gaps are both on the view: the group scope is entered and left by handlers no test drives,
  and select-all answers a different question from the one its rule asks.

---

## 2. MODEL

**Containers.** None. A selection writes nothing; the commands it feeds do.

**View state.** A list of element ids, plus the group scope the user has entered -- one optional id,
never synced.

**Picking, in steps:**

1. map the pointer to world through the camera inverse
   (`features/ambre-spec-toile-viewport.md`);
2. walk the culled sequence front to back, which is the paint order reversed;
3. skip a locked element and a group node, neither of which is a pick target
   (`TOILE-UI-006`);
4. undo the element's rotation, then test the geometry in its unrotated frame: a rectangle contains,
   an ellipse tests its normalized radius, a triangle and a closed path test the winding, an open
   path and a freehand stroke test the distance to their outline against half the stroke width,
   floored at a pick radius so a hairline stays clickable;
5. the first element that passes wins (`TOILE-UI-005`);
6. resolve the hit against the group scope (`TOILE-UI-007`).

**Resolving a hit to a target.** Every element carries the id of its immediate enclosing group. With
no scope entered, the walk climbs that edge to the top-most group, so a group transforms as one.
Inside a scope, it stops one level below the scope, so a click reaches a member or an immediate
sub-group. A hit outside the scope falls back to the top-most group.

**The marquee, in steps:** take every element whose bounds the rectangle fully contains; for one that
straddles the border, keep it when the rectangle holds its centre or when its geometry contains that
centre; skip a locked element and a group (`TOILE-UI-031`).

**Handles.** For a single selection the box is four rotated edges with a square at each rotated
corner, because a rotated rectangle drawn as a rectangle degrades to its upright bounding box on the
surface. The pick reads a band around each corner and each side, so a side resizes without drawing a
square (`TOILE-UI-034`). A multi-selection keeps an upright union box.

**Code:**

* `crates/ambre-toile/src/ui/hit.rs` -- the exact tests and the marquee;
* `ui/tool.rs` -- the group resolution, the handle bands and the hover handle;
* `ui/render.rs` -- the box, the handles and the rotation of both;
* `ui/view.rs` -- the scope, select-all and the right-click resolution.

---

## 3. SURFACE

**Triggers.**

| Trigger | Behaviour |
|---|---|
| a click on an element | selects the resolved target, replacing the selection |
| a click on empty board | clears the selection |
| a drag from empty board | draws a marquee and takes what it covers on release (`TOILE-UI-031`) |
| a double-click on a group | enters its scope, one level per press (`TOILE-UI-032`) |
| `Escape` | leaves the scope, then clears the selection (`features/ambre-spec-toile-tools.md`) |
| `Ctrl/Cmd+A` | selects the board (`TOILE-UI-033`) |
| a right-click | keeps a selection it lands in, and collapses to the hit otherwise (`features/ambre-spec-toile-menus.md`) |

**What a selection looks like.** A box with four corner squares and a rotate handle above the top
edge. A single element tilts the whole set with it; a multi-selection stays upright. A locked element
shows no handles and is not picked. The hovered handle drives the cursor
(`features/ambre-spec-toile-tools.md`).

**What the user does not see.** A collaborator's selection: it travels as presence and paints as a
ghost outline, never as this box (`features/ambre-spec-toile-presence.md section 4`).

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The index pre-filters and the UI thread refines | the spatial index answers in the order of the visible count, and it indexes rotation-expanded boxes, so it over-approximates on purpose. Refining on the UI thread keeps the exact geometry next to the code that drew it | testing exact geometry in the Logic thread, which would move every path and every stroke across a channel per click |
| A group is not pickable and a click resolves upward | a group paints nothing, so a pick on it is a pick on the space between its members. Resolving upward from a member is what makes a group behave as one object | picking the group by its union bounds, which selects it from empty space inside it |
| The scope is peeled one level per double-click | nesting is arbitrary, and a rule that jumps to the innermost member makes a two-level group unusable for a user who wanted the sub-group | double-click reaching the leaf element directly |
| A locked element is skipped by the pick, not greyed | locking exists to let a user draw over a background; refusing the click is the whole feature, and a visible difference would be noise on a board built of colour | keeping it pickable and refusing the transform later |
| A straddling element is judged by its centre | the exact answer is a geometry intersection per element per frame of the drag. The centre test is one point, and it is wrong only for a shape whose centre lies outside itself | an exact intersection, which costs a tessellation per candidate per frame |
| A single selection rotates its box, a multi-selection does not | a multi-selection has no shared angle, and picking one member's angle would tilt the box against the other members | rotating the union box by the first member's angle |
| Selection stays out of the document | it is per user and per moment; persisting it would put a cursor in the history and make two users fight over one field | a selection register, synchronized |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Select-all covers the visible board** (`TOILE-UI-033`). The walk reads the culled set, which is
  the viewport plus what the cull carries, so a board larger than the screen selects its visible part
  and the following command -- a delete, a style, a group -- applies to that part alone. The Logic
  thread already answers a whole-board aggregate for a fit, so the extent is reachable; the id list
  is not.
* **A connector cannot be picked** (`TOILE-UI-035`). The pick walk drops every connector node before
  testing, so an arrow cannot be selected, styled, deleted on its own or double-clicked to caption
  it. The captioning half is a Story; the selection half has no owner today.
* **Nothing adds to a selection.** There is no modifier that extends it: a click replaces, and a
  marquee replaces. A user cannot select two elements far apart without a marquee that covers what
  lies between them.
* **The group scope has no test and no visible state.** Entering and leaving it is a field on the
  view, and nothing on screen says which level a click will reach.
* **A rotate handle has no angle read-out and no constraint.** The design names neither a snap to
  fifteen degrees nor a displayed angle, so a user cannot rotate to a round value on purpose.
