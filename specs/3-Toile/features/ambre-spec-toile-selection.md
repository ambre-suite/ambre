# AMBRE -- FEATURE: SELECTION ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- picking, the marquee and the handles are built; a marquee and a click
disagree on what they select, select-all ignores the lock, and a connector cannot be picked.

> A user clicks an element and gets it, not the one behind it. Dragging over empty board sweeps up
> what it covers. What is selected wears a box, four corner squares and a rotate handle, tilted with
> the element.

Selection is local view state: never persisted, and a collaborator's arrives as presence rather than
as a document read (`features/ambre-spec-toile-presence.md`). There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-005` | INV | A pick MUST test the element's exact geometry over the culled candidate set: the rotation undone first, a fill by its winding, a stroke by the distance to its outline. | this spec section 2 | `ambre_toile::ui::hit::tests::rect_inside_and_outside`, `::rotated_rect_uses_local_frame`, `::ellipse_corner_is_a_miss`, `::triangle_fill`, `::open_path_hits_near_the_stroke_only`, `::stroke_proximity` | verified |
| `TOILE-UI-223` | INV | Of the elements a point falls inside, the front-most MUST be the one picked. | this spec section 2 | `ambre_toile::ui::hit::tests::topmost_wins_front_to_back` | verified |
| `TOILE-UI-006` | INV | A locked element MUST NOT be picked. | this spec section 2; `features/ambre-spec-toile-elements.md section 3` | `ambre_toile::ui::hit::tests::locked_never_hits` | verified |
| `TOILE-UI-224` | INV | A group node MUST NOT be picked directly: it paints nothing, so a point inside it is a point on the space between its members. | this spec section 2; section 4 | `ambre_toile::ui::hit::tests::group_never_hits` | verified |
| `TOILE-UI-225` | INV | A hairline MUST stay clickable at every zoom. | this spec section 2 | none: the pick floor is four world units, so it shrinks on screen as the user zooms out and swells as they zoom in -- **the rule holds at one zoom level** -- section 5 | unverified |
| `TOILE-UI-226` | INV | The whole edge of the selection box MUST be grabbable, even though only the corners draw a square. | this spec section 3 | `ambre_toile::ui::tool::tests::edge_handle_at_picks_sides_corners_and_misses` | verified |
| `TOILE-UI-227` | INV | A handle MUST answer a hover only when exactly one element is selected, the select tool is armed and no drag is in flight. | this spec section 3 | `ambre_toile::ui::tool::tests::hover_handle_only_for_a_single_select_at_rest` | verified |
| `TOILE-UI-228` | INV | A selection MUST NOT reach any container: it is per user and per moment. | this spec section 4; `ambre-spec-toile-schema.md section 2` | verified: structural, and the absence is the proof -- the id list is a field on the view and the schema declares no container for it | verified |
| `TOILE-UI-035` | INV | A connector MUST be pickable, so it can be selected, styled and captioned. | this spec section 2; `features/ambre-spec-toile-connectors.md section 3` | not built -- lands with the "Connector labels" Story (`ambre-roadmap.md` E4) | unimplemented |
| `TOILE-UI-034` | OBS | The selection box and its handles MUST follow a single element's rotation. | this spec section 3 | `ambre_toile::ui::render::tests::overlay_rotates_box_and_handles_for_a_rotated_element`, `ambre_toile::ui::tool::tests::hover_handle_follows_the_element_rotation` | verified |
| `TOILE-UI-229` | OBS | A single selection MUST wear four corner squares and a rotate handle above the top edge. | this spec section 3 | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` | verified |
| `TOILE-UI-230` | OBS | A multi-selection MUST wear an upright union box and no resize handle, having no shared angle to resize along. | this spec section 3; section 4 | none: no test paints a two-element selection | unverified |
| `TOILE-UI-231` | OBS | A locked element MUST NOT wear a selection box. | this spec section 3 | none: the overlay reads the id list and never the lock, and select-all puts a locked element in that list -- **the rule fails today** -- section 5 | unverified |

### 1.2 Click an element

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-007` | POST | A click MUST resolve to the top-most enclosing group unless the user has entered that group's scope. | this spec section 2 | `ambre_toile::ui::tool::tests::click_on_group_member_selects_top_most_group`, `::scoped_click_selects_the_member_inside_the_scope`, `::ungrouped_element_resolves_to_itself`, `::pointer_down_on_a_member_selects_the_group` | verified |
| `TOILE-UI-232` | POST | A click on an element already selected MUST keep the whole selection, so a multi-selection drags as one. | this spec section 3 | none: no test presses on a member of a selection it did not just make | unverified |
| `TOILE-UI-233` | POST | A click on empty board MUST clear the selection. | this spec section 3 | none: the press arms a marquee and clears nothing, so the selection survives until the release -- section 5 | unverified |
| `TOILE-UI-234` | FRAME | A press MUST NOT write to the document. | this spec section 4 | verified: structural, and the type is the proof -- the press returns a local selection or nothing, and the command variant is reachable only from a release that moved | verified |

### 1.3 Drag over empty board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-031` | POST | A marquee MUST take every element it covers. | this spec section 2 | `ambre_toile::ui::hit::tests::marquee_selects_covered_elements` | verified |
| `TOILE-UI-235` | POST | A marquee MUST replace the selection on release, not while it is dragged. | this spec section 3 | `ambre_toile::ui::tool::tests::drag_empty_makes_a_marquee_then_selects` | verified |
| `TOILE-UI-236` | POST | A marquee MUST return the same target a click on the same element would return. | this spec section 2; section 4 | none: the marquee returns raw members and never climbs the group chain, so one gesture selects a group and the other its parts -- **the rule fails today** -- section 5 | unverified |
| `TOILE-UI-237` | POST | A rotated element MUST be judged by the shape the user sees, not by the upright box around it. | this spec section 2 | none: containment is read on the unrotated bounds, so a tilted shape is taken by a rectangle that does not cover it -- **the rule fails today** -- section 5 | unverified |
| `TOILE-UI-238` | FRAME | A marquee MUST NOT take a locked element or a group node. | this spec section 2 | none: `ambre_toile::ui::hit::tests::marquee_selects_covered_elements` covers one element inside and one outside, and neither is locked nor a group | unverified |

### 1.4 Enter and leave a group scope

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-032` | POST | A double-click MUST enter one nesting level of the group under it. | this spec section 3 | none: nothing drives the double-click handler, which lives on the view | unverified |
| `TOILE-UI-239` | POST | `Escape` MUST leave the scope and clear the selection in one press. | this spec section 3; `features/ambre-spec-toile-tools.md` | none: nothing drives the key handler, and section 3 read as two presses until this row | unverified |
| `TOILE-UI-240` | FRAME | A double-click on an element in no group MUST NOT change the scope. | this spec section 3 | none: nothing drives the double-click handler | unverified |

### 1.5 Select the whole board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-033` | POST | Select-all MUST cover the whole board, not the part the UI thread holds. | this spec section 3; section 4 | none: the walk is over the culled set, so **the rule fails off screen** -- section 5 | unverified |
| `TOILE-UI-241` | POST | Select-all MUST obey the exclusions a pick obeys: no locked element, no group node. | this spec section 3 | none: the walk drops connectors and nothing else, so a locked element joins the selection and the next delete takes it -- **the rule fails today** -- section 5 | unverified |
| `TOILE-UI-242` | FRAME | Select-all MUST NOT alter the group scope the user has entered. | this spec section 3 | verified: structural, and the construction is the proof -- the walk writes the id list and the redraw flag, and no other field | verified |

* **Budget.** the content hit-test and input dispatch under 0.2 ms, one row of
  `ambre-spec-budgets.md section 2`. It is a walk over the culled set, which the spatial index
  already bounds (`TOILE-005`), and no bench measures it. A stroke costs the distance to every
  segment of its outline, which the cull does not bound: one freehand element under the pointer is
  as many distance tests as it has points.
* **Boundary.** none: a selection is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. It leaves the client only as presence, which the Sync worker
  seals like any other frame (`ZK-005`).
* **Verification.** 14 `verified`, 13 `unverified`, 1 `unimplemented`, over twenty distinct
  mechanisms where the flat table cited nineteen. Almost nothing new is proved: one row cited seven
  hit tests for one conjunction, and the split hands each test the rule it actually asserts. The
  gaps are the gestures, not the geometry -- the scope is entered and left by handlers no test
  drives, and four rules on the marquee and select-all fail today.

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
   floored at a pick radius (`TOILE-UI-225`, four world units, so it is a pick radius on screen at
   one zoom level and a different one everywhere else);
5. the first element that passes wins (`TOILE-UI-005`);
6. resolve the hit against the group scope (`TOILE-UI-007`).

**Resolving a hit to a target.** Every element carries the id of its immediate enclosing group. With
no scope entered, the walk climbs that edge to the top-most group, so a group transforms as one.
Inside a scope, it stops one level below the scope, so a click reaches a member or an immediate
sub-group. A hit outside the scope falls back to the top-most group.

**The marquee, in steps:** take every element whose bounds the rectangle fully contains; for one that
straddles the border, keep it when the rectangle holds its centre or when its geometry contains that
centre; skip a locked element and a group (`TOILE-UI-031`, `TOILE-UI-238`).

The containment test reads the upright bounds, not the rotated geometry, so a long shape turned
forty-five degrees is judged by a box larger than itself and is missed by a rectangle that covers
it (`TOILE-UI-237`). And the marquee stops at the raw hits: it does not climb the group chain the
way a click does, so the same two elements come back as their group under a click and as themselves
under a rectangle (`TOILE-UI-236`).

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
| a click on an element | selects the resolved target, replacing the selection, unless that element is already in it (`TOILE-UI-232`) |
| a click on empty board | clears the selection, on the release rather than the press (`TOILE-UI-233`) |
| a drag from empty board | draws a marquee and takes what it covers on release (`TOILE-UI-031`) |
| a double-click on a group | enters its scope, one level per press (`TOILE-UI-032`) |
| `Escape` | leaves the scope and clears the selection, both in one press (`TOILE-UI-239`) |
| `Ctrl/Cmd+A` | selects the board, lock and groups included (`TOILE-UI-033`, `TOILE-UI-241`) |
| a right-click | keeps a selection it lands in, and collapses to the hit otherwise (`features/ambre-spec-toile-menus.md`) |

**What a selection looks like.** A box with four corner squares and a rotate handle above the top
edge (`TOILE-UI-229`). A single element tilts the whole set with it (`TOILE-UI-034`); a
multi-selection stays upright and drops the corner squares, keeping the rotate handle alone
(`TOILE-UI-230`). A locked element is not picked, and wears no handles because it cannot be a
single selection through the pick -- but select-all can put one in the box (`TOILE-UI-231`). The
hovered handle drives the cursor (`features/ambre-spec-toile-tools.md`).

**What the user does not see.** A collaborator's selection: it travels as presence and paints as a
ghost outline, never as this box (`features/ambre-spec-toile-presence.md section 4`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The index pre-filters and the UI thread refines | testing exact geometry in the Logic thread | it moves every path and every stroke across a channel per click. The spatial index answers in the order of the visible count and indexes rotation-expanded boxes, so it over-approximates on purpose, and refining on the UI thread keeps the exact geometry next to the code that drew it |
| A group is not pickable and a click resolves upward | picking the group by its union bounds | it selects the group from empty space inside it. A group paints nothing, so a pick there is a pick on the space between its members, and resolving upward from a member is what makes a group behave as one object |
| The scope is peeled one level per double-click | a double-click reaching the leaf element directly | nesting is arbitrary, and a rule that jumps to the innermost member makes a two-level group unusable for a user who wanted the sub-group |
| A locked element is skipped by the pick, not greyed | keeping it pickable and refusing the transform later | locking exists to let a user draw over a background, so refusing the click is the whole feature. A visible difference would be noise on a board built of colour |
| A straddling element is judged by its centre | an exact intersection | it costs a tessellation per candidate per frame of the drag. The centre test is one point, and it is wrong only for a shape whose centre lies outside itself |
| A single selection rotates its box, a multi-selection does not | rotating the union box by the first member's angle | a multi-selection has no shared angle, so one member's angle tilts the box against the others |
| Selection stays out of the document | a selection register, synchronized | selection is per user and per moment. Persisting it puts a cursor in the history and makes two users fight over one field |

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
* **Select-all takes what a click may not** (`TOILE-UI-241`, `TOILE-UI-231`). The walk drops
  connectors and keeps everything else, so `Ctrl/Cmd+A` selects locked elements and group nodes
  that no pick would return, wraps a locked element in a selection box, and hands the whole list to
  the next delete. The lock stops the pointer and nothing else.
* **A marquee and a click do not select the same thing** (`TOILE-UI-236`). A click on a grouped
  element selects its top-most group; a rectangle over that same group selects its members. Two
  gestures for one job, and the group behaves as one object under only one of them.
* **The pick radius is in world units** (`TOILE-UI-225`). The floor that keeps a hairline clickable
  is four board units, not four pixels, so it is four pixels at one zoom and a fortieth of one at
  forty. Zoomed out, a thin line cannot be hit; zoomed in, a stroke grabs from far away.
* **A double-click never leaves a scope** (`TOILE-UI-032`). It enters one level, and only `Escape`
  goes back out -- taking the selection with it in the same press (`TOILE-UI-239`), so a user
  cannot step out one level and keep what they had.
* **Nothing adds to a selection.** There is no modifier that extends it: a click replaces, and a
  marquee replaces. A user cannot select two elements far apart without a marquee that covers what
  lies between them.
* **The group scope has no test and no visible state.** Entering and leaving it is a field on the
  view, and nothing on screen says which level a click will reach.
* **A rotate handle has no angle read-out and no constraint.** The design names neither a snap to
  fifteen degrees nor a displayed angle, so a user cannot rotate to a round value on purpose.
