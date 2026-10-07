# AMBRE -- FEATURE: SELECTION ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- picking, the marquee and the handles are built. A marquee and a click
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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-005` | `INV` | A pick MUST test the element's exact geometry over the culled candidate set: the rotation undone first, a fill by its winding, a stroke by the distance to its outline. | this spec section 2 | `ambre_toile::ui::hit::tests::rect_inside_and_outside`; `ambre_toile::ui::hit::tests::rotated_rect_uses_local_frame`; `ambre_toile::ui::hit::tests::ellipse_corner_is_a_miss`; `ambre_toile::ui::hit::tests::triangle_fill`; `ambre_toile::ui::hit::tests::open_path_hits_near_the_stroke_only`; `ambre_toile::ui::hit::tests::stroke_proximity` | none | verified |
| `TOILE-UI-223` | `INV` | Of the elements a point falls inside, the front-most MUST be the one picked. | this spec section 2 | `ambre_toile::ui::hit::tests::topmost_wins_front_to_back` | none | verified |
| `TOILE-UI-006` | `INV` | A locked element MUST NOT be picked. | this spec section 2; `features/ambre-spec-toile-elements.md section 3` | `ambre_toile::ui::hit::tests::locked_never_hits` | none | verified |
| `TOILE-UI-224` | `INV` | A group node MUST NOT be picked directly: it paints nothing, so a point inside it is a point on the space between its members. | this spec section 2; section 4 | `ambre_toile::ui::hit::tests::group_never_hits` | none | verified |
| `TOILE-UI-225` | `INV` | A hairline MUST stay clickable at every zoom. | this spec section 2 | none | fails -- the pick floor is four world units, so it shrinks on screen as the user zooms out and swells as they zoom in: the rule holds at one zoom level, section 5; lands with the `A hairline stays clickable at every zoom` Story (`ambre-roadmap.md` E7) | failing |
| `TOILE-UI-226` | `INV` | The whole edge of the selection box MUST be grabbable, even though only the corners draw a square. | this spec section 3 | `ambre_toile::ui::tool::tests::edge_handle_at_picks_sides_corners_and_misses` | none | verified |
| `TOILE-UI-227` | `INV` | A handle MUST answer a hover only when exactly one element is selected, the select tool is armed and no drag is in flight. | this spec section 3 | `ambre_toile::ui::tool::tests::hover_handle_only_for_a_single_select_at_rest` | none | verified |
| `TOILE-UI-228` | `INV` | A selection MUST NOT reach any container: it is per user and per moment. | this spec section 4; `ambre-spec-toile-schema.md section 2` | structural -- the id list is a field on the view and the schema declares no container for it | none | verified |
| `TOILE-UI-385` | `INV` | A selection MUST drop an element the board no longer holds, a peer's deletion included. | this spec section 2 | `ambre_toile::logic::convergence_tests::selection_data_names_the_ids_a_peer_deleted` -- a peer's delete applied, the selection's data names the lost id and only it; `ambre_toile::ui::tool::tests::a_selection_drops_the_ids_the_board_lost` -- the view's reduction drops those ids and keeps the others in order; `ambre_toile::ui::view::view_tests::a_peer_deletion_reaches_the_selection_through_update` -- that answer, drained by the frame, reaches the view's selection | none | verified |
| `TOILE-UI-035` | `INV` | A connector MUST be pickable: a click on an arrow, on its line or on its caption, MUST be able to name it. | this spec section 2; `features/ambre-spec-toile-connectors.md section 3` | `ambre_toile::ui::label::tests::a_click_on_the_line_picks_the_arrow`; `ambre_toile::ui::label::tests::a_click_on_the_caption_picks_the_arrow`; `ambre_toile::ui::label::tests::the_top_most_arrow_wins` | none | verified |
| `TOILE-UI-349` | `INV` | A picked connector MUST be selectable, so an arrow can be deleted, re-aimed, restyled and reordered like anything else on the board. | this spec section 2; `features/ambre-spec-toile-connectors.md section 4` | `ambre_toile::ui::arrow::tests::an_arrow_answers_delete_and_the_z_order`; `ambre_toile::ui::arrow::tests::a_release_over_a_shape_re_aims_that_end` -- together, what a selected arrow answers; structural -- the selection is its own field beside the element selection, exclusive with it (`TOILE-UI-354`), and an arrow has no box, so it joins no marquee, group or transform | none | verified |
| `TOILE-UI-386` | `INV` | A selected arrow MUST stay selected wherever the camera moves, and leave the selection only when the board loses it. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_selected_arrow_outlives_the_frame_and_leaves_with_the_board` | none | verified |
| `TOILE-UI-034` | `OBS` | The selection box and its handles MUST follow a single element's rotation. | this spec section 3 | `ambre_toile::ui::render::tests::overlay_rotates_box_and_handles_for_a_rotated_element`; `ambre_toile::ui::tool::tests::hover_handle_follows_the_element_rotation` | none | verified |
| `TOILE-UI-229` | `OBS` | A single selection MUST wear four corner squares and a rotate handle above the top edge. | this spec section 3 | `ambre_toile::ui::render::tests::overlay_draws_box_and_handles_for_single_selection` | none | verified |
| `TOILE-UI-230` | `OBS` | A multi-selection MUST wear an upright union box and no resize handle, having no shared angle to resize along. | this spec section 3; section 4 | none | unheld -- no test paints a two-element selection; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-231` | `OBS` | A locked element MUST NOT wear a selection box. | this spec section 3 | `ambre_toile::ui::tool::tests::selection_bounds_skips_a_locked_element` -- a locked-only selection has no box, and a mixed selection's box covers the unlocked element alone | none | verified |

### 1.2 Click an element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-007` | `POST` | A click MUST resolve to the top-most enclosing group unless the user has entered that group's scope. | this spec section 2 | `ambre_toile::ui::tool::tests::click_on_group_member_selects_top_most_group`; `ambre_toile::ui::tool::tests::scoped_click_selects_the_member_inside_the_scope`; `ambre_toile::ui::tool::tests::ungrouped_element_resolves_to_itself`; `ambre_toile::ui::tool::tests::pointer_down_on_a_member_selects_the_group` | none | verified |
| `TOILE-UI-232` | `POST` | A click on an element already selected MUST keep the whole selection, so a multi-selection drags as one. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_click_on_a_selected_element_keeps_the_whole_selection` -- a marquee takes two stickies, and a press then a release on one keeps both | none | verified |
| `TOILE-UI-233` | `POST` | A click on empty board MUST clear the selection. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_click_selects_and_a_click_on_empty_board_clears` -- the view driven by a press and a release, the selection read after each click | none | verified |
| `TOILE-UI-234` | `FRAME` | A press MUST NOT write to the document. | this spec section 4 | type-level -- the press returns a local selection or nothing, and the command variant is reachable only from a release that moved | none | verified |

### 1.3 Drag over empty board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-031` | `POST` | A marquee MUST take every element it covers. | this spec section 2 | `ambre_toile::ui::hit::tests::marquee_selects_covered_elements` | none | verified |
| `TOILE-UI-235` | `POST` | A marquee MUST replace the selection on release, not while it is dragged. | this spec section 3 | `ambre_toile::ui::tool::tests::drag_empty_makes_a_marquee_then_selects` | none | verified |
| `TOILE-UI-236` | `POST` | A marquee MUST return the same target a click on the same element would return. | this spec section 2; section 4 | `ambre_toile::ui::tool::tests::marquee_resolves_a_member_to_its_group` -- a marquee over a member resolves to the top-most group, the target a click returns | none | verified |
| `TOILE-UI-237` | `POST` | A rotated element MUST be judged by the shape the user sees, not by the upright box around it. | this spec section 2 | `ambre_toile::ui::hit::tests::marquee_judges_a_rotated_element_by_its_rotated_box` -- a marquee over the unrotated tip of a rotated long rect misses, and one over the rotated shape takes it | none | verified |
| `TOILE-UI-238` | `FRAME` | A marquee MUST NOT take a locked element or a group node. | this spec section 2 | none | unheld -- `ambre_toile::ui::hit::tests::marquee_selects_covered_elements` covers one element inside and one outside, and neither is locked nor a group; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Enter and leave a group scope

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-032` | `POST` | A double-click MUST enter one nesting level of the group under it. | this spec section 3 | none | unheld -- nothing drives the double-click handler, which lives on the view; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-239` | `POST` | `Escape` MUST leave the scope and clear the selection in one press. | this spec section 3; `features/ambre-spec-toile-tools.md` | `ambre_toile::ui::view::view_tests::escape_leaves_the_scope_and_clears_the_selection_in_one_press` -- one key event through the view empties both | none | verified |
| `TOILE-UI-240` | `FRAME` | A double-click on an element in no group MUST NOT change the scope. | this spec section 3 | none | unheld -- nothing drives the double-click handler; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Select the whole board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-033` | `POST` | Select-all MUST cover the whole board, not the part the UI thread holds. | this spec section 3; section 4 | `ambre_toile::logic::convergence_tests::board_ids_returns_every_element` -- Logic answers every element id, off-screen included, and select-all selects that list; `::board_ids_covers_an_element_the_order_omits` -- the answer reads the element map, so an element the `layers` order omits is still in it | none | verified |
| `TOILE-UI-241` | `POST` | Select-all MUST obey the exclusions a pick obeys: no locked element, no group node. | this spec section 3 | `ambre_toile::logic::convergence_tests::board_ids_excludes_locked_and_group_nodes` -- the board's id list carries the unlocked members and neither a locked element nor a group node | none | verified |
| `TOILE-UI-242` | `FRAME` | Select-all MUST NOT alter the group scope the user has entered. | this spec section 3 | structural -- the walk writes the id list and the redraw flag, and no other field | none | verified |

* **Budget.** the content hit-test and input dispatch under 0.2 ms, one row of
  `ambre-spec-budgets.md section 2`. It is a walk over the culled set, which the spatial index
  already bounds (`TOILE-005`), and no bench measures it. A stroke costs the distance to every
  segment of its outline, which the cull does not bound.
* **Boundary.** none: a selection is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. It leaves the client only as presence, sealed like any other
  frame (`ZK-005`).
* **Verification.** 26 `verified`, 1 `failing`, 4 `untested`. What holds is the geometry and
  select-all with the pick's exclusions. A locked element wears no box, and a deleted one leaves the
  selection. A click, a marquee and `Escape` are driven through the view. What is loose is the
  double-click into a scope, and the one failure is the hairline.

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
4. undo the element's rotation, then test the geometry in its unrotated frame. A rectangle
   contains, an ellipse tests its normalized radius, a triangle and a closed path test the winding.
   An open path and a freehand stroke test the distance to their outline against half the stroke
   width, floored at a pick radius (`TOILE-UI-225`). That radius is four world units, so it is four
   pixels on screen at one zoom level and a different count everywhere else;
5. the first element that passes wins (`TOILE-UI-005`);
6. resolve the hit against the group scope (`TOILE-UI-007`).

**Resolving a hit to a target.** Every element carries the id of its immediate enclosing group. With
no scope entered, the walk climbs that edge to the top-most group, so a group transforms as one.
Inside a scope, it stops one level below the scope, so a click reaches a member or an immediate
sub-group. A hit outside the scope falls back to the top-most group.

**The marquee, in steps:** take every element whose bounds the rectangle fully contains. For one
that straddles the border, keep it when the rectangle holds its centre or when its geometry contains
that centre. Skip a locked element and a group (`TOILE-UI-031`, `TOILE-UI-238`).

The containment test reads the rotated box, the shape the user sees, not the unrotated bounds
(`TOILE-UI-237`). The marquee resolves each hit the way a click does: up the group chain to the
top-most group (`TOILE-UI-236`).

**A selection that loses an element.** A board change, a peer's deletion included, makes the view
fetch the selection's data again (`TOILE-UI-384`). The answer names the requested ids the board no
longer holds, and the view drops them from the selection (`TOILE-UI-385`). Ids are never reused, so an
answer to an older selection drops only ids that are gone for good.

**Handles.** For a single selection the box is four rotated edges with a square at each rotated
corner. A rotated rectangle drawn as a rectangle would degrade to its upright bounding box on the
surface. The pick reads a band around each corner and each side, so a side resizes without drawing a
square (`TOILE-UI-034`). A multi-selection keeps an upright union box.

**Code:**

* `crates/ambre-toile/src/ui/hit.rs` -- the exact tests and the marquee;
* `ui/tool.rs` -- the group resolution, the handle bands, the hover handle and the drop of lost ids;
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
| `Ctrl/Cmd+A` | selects the board, lock and groups excluded (`TOILE-UI-033`, `TOILE-UI-241`) |
| a right-click | keeps a selection it lands in, and collapses to the hit otherwise (`features/ambre-spec-toile-menus.md`) |

**What a selection looks like.** A box with four corner squares and a rotate handle above the top
edge (`TOILE-UI-229`). A single element tilts the whole set with it (`TOILE-UI-034`); a
multi-selection stays upright and drops the corner squares, keeping the rotate handle alone
(`TOILE-UI-230`). A locked element is not picked, and wears no box on any surface, select-all
included (`TOILE-UI-231`). The hovered handle drives the cursor (`features/ambre-spec-toile-tools.md`).

**What the user does not see.** A collaborator's selection: it travels as presence and paints as a
ghost outline, never as this box (`features/ambre-spec-toile-presence.md section 4`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The index pre-filters and the UI thread refines | testing exact geometry in the Logic thread | it moves every path and every stroke across a queue per click. The spatial index answers in the order of the visible count and indexes rotation-expanded boxes, so it over-approximates on purpose, and refining on the UI thread keeps the exact geometry next to the code that drew it |
| A group is not pickable and a click resolves upward | picking the group by its union bounds | it selects the group from empty space inside it. A group paints nothing, so a pick there is a pick on the space between its members, and resolving upward from a member is what makes a group behave as one object |
| The scope is peeled one level per double-click | a double-click reaching the leaf element directly | nesting is arbitrary, and a rule that jumps to the innermost member makes a two-level group unusable for a user who wanted the sub-group |
| A locked element is skipped by the pick, not greyed | keeping it pickable and refusing the transform later | locking exists to let a user draw over a background, so refusing the click is the whole feature. A visible difference would be noise on a board built of colour |
| A straddling element is judged by its centre | an exact intersection | it costs a tessellation per candidate per frame of the drag. The centre test is one point, and it is wrong only for a shape whose centre lies outside itself |
| A single selection rotates its box, a multi-selection does not | rotating the union box by the first member's angle | a multi-selection has no shared angle, so one member's angle tilts the box against the others |
| Selection stays out of the document | a selection register, synchronized | selection is per user and per moment. Persisting it puts a cursor in the history and makes two users fight over one field |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The pick radius is in world units** (`TOILE-UI-225`). The floor that keeps a hairline clickable
  is four board units, not four pixels. It is four pixels at one zoom and a fortieth of one at
  forty. Zoomed out, a thin line cannot be hit; zoomed in, a stroke grabs from far away.
* **A double-click never leaves a scope** (`TOILE-UI-032`). It enters one level, and only `Escape`
  goes back out, taking the selection with it in the same press (`TOILE-UI-239`). A user cannot step
  out one level and keep what they had.
* **Nothing adds to a selection.** There is no modifier that extends it: a click replaces, and a
  marquee replaces. A user cannot select two elements far apart without a marquee that covers what
  lies between them.
* **The group scope has no visible state, and entering it has no test.** It is a field on the view,
  and nothing on screen says which level a click will reach.
* **A scope on a deleted group lingers.** A peer that deletes or ungroups the entered group leaves the
  scope naming an id the board lost. It is harmless: no hit lies inside it, so every click resolves to
  the top-most group, and `Escape` clears it.
* **A rotate handle has no angle read-out and no constraint.** The design names neither a snap to
  fifteen degrees nor a displayed angle, so a user cannot rotate to a round value on purpose.
