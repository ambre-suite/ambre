# AMBRE -- FEATURE: GROUPS

**Type:** Vertical (feature)
**Implementation:** partial -- grouping ships; a write that touches a group still walks the board.

> A user selects several elements and binds them: from then on one click takes the set, and one drag
> moves it. Ungrouping gives the elements back, in place.

A group paints nothing. It exists to make a set behave as one object, and its box is the box of what
it holds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-149` | `INV` | An element MUST belong to at most one group or frame. | this spec section 3 | `ambre_toile::logic::structure_tests::an_element_sits_in_one_group_or_frame_at_most` -- a member framed out of its group, then grouped out of its frame, then two members of one frame grouped into a group that takes their place in it, each list read back and the caches equal to a rebuild | none | verified |
| `TOILE-009` | `INV` | A group's box MUST be the union of its members' boxes. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members` -- the union of two corner rects; `ambre_toile::logic::convergence_tests::moving_a_group_translates_its_members` -- the same union again after a translation; `ambre_toile::logic::convergence_tests::a_member_framed_out_of_its_group_shrinks_the_group` -- the union shrinks when a frame takes a member | none | verified |
| `TOILE-275` | `INV` | The recomputation of the group boxes MUST end, even on a box that never compares equal to itself. | this spec section 3; `TOILE-274` | `ambre_toile::logic::state::caches::peer_geometry_tests::a_group_over_a_nan_member_settles` -- a member a peer moved to NaN, through a delta and a rebuild, then a NaN box forged into the caches: the full and the incremental recomputation each return | none | verified |
| `TOILE-276` | `INV` | A membership cycle a peer writes MUST NOT stop a move or an alignment of a group in the cycle. | this spec section 3 | `ambre_toile::logic::state::groups::tests::a_group_cycle_a_peer_writes_ends_a_move_and_an_alignment` -- a group in its own list and two groups in each other's, through a delta, so both groups keep a box: the move reaches each leaf once, and the alignment expands both groups and returns | none | verified |
| `TOILE-UI-398` | `INV` | A click on a member of a membership cycle MUST resolve to one element. | this spec section 3; `TOILE-UI-119` | `ambre_toile::ui::tool::tests::a_click_into_a_group_cycle_ends` -- two groups that name each other as parent: an unscoped click picks the last group of the chain, and a scoped click the member | none | verified |
| `TOILE-150` | `INV` | A group MUST carry no coordinates of its own. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_group_takes_no_box_from_a_resize_or_a_rotation` -- a resize aimed at a group writes no change, and a rotation leaves no box of its own in the caches. The group seed writes a kind, a rotation, a lock flag and a style register, and no x, y, width or height | none | verified |
| `TOILE-054` | `INV` | A group left with no member MUST be removed, so the caches stay equal to a rebuild. | this spec section 3; `TOILE-004` | `ambre_toile::logic::structure_tests::a_group_left_with_no_member_is_removed` -- one member deleted and the last framed away: the group gone from the board and the order, and the caches equal to a rebuild | none | verified |
| `TOILE-056` | `INV` | Reading which container an element belongs to MUST NOT cost a walk of the whole board. | `ambre-spec-budgets.md section 1`; this spec section 3 | `ambre_toile::logic::convergence_tests::a_container_read_walks_no_board` -- a selection read and a camera move resolve the element map zero times, and still name each member's group and frame. `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` holds the cache equal to a rebuild | none | verified |
| `TOILE-UI-119` | `OBS` | Every painted element MUST name its immediate enclosing group, or none when it has no group. | this spec section 3 | `ambre_toile::logic::convergence_tests::derive_populates_the_group_edge_on_members` -- it reads the edge off the derived viewport for both members and for the group itself, three assertions for both cases | none | verified |
| `TOILE-266` | `CONC` | When concurrent binds put one element in two groups, every peer MUST name the same enclosing group, the one with the smallest id. | this spec section 3; `TOILE-UI-119` | `ambre_toile::logic::convergence_tests::an_element_bound_into_two_groups_names_one_on_both_peers` | none | verified |
| `TOILE-UI-120` | `OBS` | A group MUST paint nothing of its own: no fill, no border, no title. | this spec section 4 | `ambre_toile::ui::render::tests::a_derived_group_paints_nothing_of_its_own` -- the group node the engine derives, painted alone, draws what an empty board draws | none | verified |

### 1.2 Bind a selection into a group

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-052` | `POST` | A group MUST enter the paint order below its lowest member, so the members keep their relative order above it. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members` -- its second assertion, where the paint order reads group, then the two members in their prior order | none | verified |
| `TOILE-151` | `POST` | An id the board no longer holds MUST be dropped from the binding. | this spec section 3 | `ambre_toile::logic::structure_tests::a_binding_keeps_what_it_can_hold_and_moves_nobody` -- an id a peer deleted while it was still selected, absent from the binding | none | verified |
| `TOILE-152` | `POST` | A selected element that encloses another selected element MUST be dropped from the binding. | this spec section 3; section 5 | `ambre_toile::logic::structure_tests::a_binding_keeps_what_it_can_hold_and_moves_nobody` -- a group selected with one of its own members, absent from the binding | none | verified |
| `TOILE-246` | `POST` | The enclosed elements of such a selection MUST be bound. | this spec section 3; section 5 | `ambre_toile::logic::structure_tests::a_binding_keeps_what_it_can_hold_and_moves_nobody` -- the selected member of that group read back in the binding | none | verified |
| `TOILE-153` | `FRAME` | Binding a group MUST NOT move any member. | this spec section 3 | `ambre_toile::logic::structure_tests::a_binding_keeps_what_it_can_hold_and_moves_nobody` -- every member's box read back after the binding, equal to its box before | none | verified |
| `TOILE-055` | `FAIL` | A binding left with fewer than two distinct members MUST create no group. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_group_of_one_answers_an_error` -- one member yields a `GroupNeedsTwo` error and no group | none | verified |
| `TOILE-273` | `FAIL` | A group the board refuses to write MUST leave every member in the group or frame that held it. | this spec section 3 | `ambre_toile::logic::state::groups::cache_tests::a_refused_group_leaves_each_member_where_it_was` -- a detached board refuses the group, and the framed member's list and caches read back unchanged | none | verified |

### 1.3 Move a group

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-154` | `POST` | A transform on a group MUST reach every member. | this spec section 3 | `ambre_toile::logic::convergence_tests::moving_a_group_translates_its_members` -- both members at their translated rects | none | verified |
| `TOILE-053` | `POST` | A transform on a group MUST reach every leaf of its subtree, however deep the nesting. | this spec section 3 | `ambre_toile::logic::structure_tests::a_move_reaches_every_leaf_of_a_nested_group` -- a move of a group nested three deep, every leaf read back at its own box plus the delta | none | verified |
| `TOILE-155` | `FRAME` | A transform on a group MUST NOT resize it: its box is derived. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_group_takes_no_box_from_a_resize_or_a_rotation` -- a resize aimed at a group whose member a peer deleted writes no change, and after a rotation its caches equal the other peer's and a rebuild's | none | verified |

### 1.4 Ungroup

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-010` | `POST` | Ungrouping MUST preserve the surviving children and their relative order. | this spec section 3 | `ambre_toile::logic::convergence_tests::ungroup_keeps_children_and_order` -- its second assertion, where the paint order after the unbind reads the two children in their prior order | none | verified |
| `TOILE-156` | `POST` | Ungrouping MUST remove the group itself. | this spec section 3 | `ambre_toile::logic::convergence_tests::ungroup_keeps_children_and_order` -- its first assertion, where the group has no box after the unbind | none | verified |
| `TOILE-157` | `POST` | The children MUST move to the group or frame that held the group, or to the board when none did. | this spec section 3 | `ambre_toile::logic::structure_tests::a_nested_ungroup_hands_its_children_up_and_moves_nothing` -- a nested group ungrouped: its children read back in the enclosing group's list; `ambre_toile::logic::convergence_tests::ungrouping_a_framed_group_hands_its_children_to_the_frame` -- a group a frame holds: its children read back in the frame's list, on both peers; `ambre_toile::logic::state::groups::cache_tests::a_top_level_ungroup_hands_its_children_to_the_board` -- a group no group or frame holds ungrouped: each child reads back with no parent, and the caches equal a rebuild | none | verified |
| `TOILE-158` | `FRAME` | Ungrouping MUST NOT move any element. | this spec section 3 | `ambre_toile::logic::structure_tests::a_nested_ungroup_hands_its_children_up_and_moves_nothing` -- every element's box read back after the ungroup, equal to its box before | none | verified |
| `TOILE-159` | `FAIL` | Ungrouping an id that is not a group MUST change nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::an_ungroup_of_a_non_group_answers_an_error` -- a plain rect yields a `NotAGroup` error and nothing moves | none | verified |
| `TOILE-264` | `POST` | An ungroup whose list holds a group MUST unbind its groups and answer nothing for the ids that are not groups. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_mixed_ungroup_answers_nothing_for_its_non_groups` -- a group between two rects unbinds with no error, and the two rects alone answer one `NotAGroup` | none | verified |
| `TOILE-160` | `FAIL` | Ungrouping an id the board does not hold MUST tell the user. | this spec section 3 | `ambre_toile::logic::structure_tests::ungrouping_a_missing_id_says_so` -- the unknown-element error read back, with no change | none | verified |

### 1.5 Delete a group

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-161` | `POST` | Deleting a group MUST delete its members. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_a_group_cascades_to_members` -- an empty board and an empty paint order after deleting the group of two | none | verified |
| `TOILE-162` | `FRAME` | Deleting a group MUST NOT remove an element outside its own subtree. | this spec section 3 | `ambre_toile::logic::structure_tests::deleting_a_group_spares_a_bystander` -- a bystander overlapping the group, read back alone on the board and in the order | none | verified |

* **Budget.** the transform command under 1 ms and a remote change under 2 ms, two rows of
  `ambre-spec-budgets.md section 1`. Neither is shown on a large board: a write that touches a
  group still walks the board, section 6, and no bench measures it.
* **Boundary.** none of its own: the binding lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`). The group box is derived and never persisted.
* **Verification.** 30 `verified`. What holds is each gesture, and the two refusals a chord over a
  single element or a non-group answers. A mixed ungroup unbinds its groups in silence. A resize
  aimed at a group writes nothing, and a rotation leaves its box derived, even with a member a peer
  deleted. The binding filter, the nested cases and the one-container rule are driven by hand.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| groups two shapes | moves one of those shapes | the group, and the shape at its new position | the group box widened to cover the move, on both screens (`TOILE-009`) | `logic::convergence_tests::group_unions_bounds_and_sits_below_members` |
| moves the group | moves one member | both translations, applied to the member | the member offset by the sum (`TOILE-154`), the others by the group's move alone | `logic::convergence_tests::moving_a_group_translates_its_members` |
| ungroups | drags the group | the binding gone; the drag applied to whatever it resolved to | the elements loose, in their order (`TOILE-010`) | `logic::convergence_tests::ungroup_keeps_children_and_order` |
| deletes the group | edits a member | the members gone, and the edit landing on nothing | an empty region on both screens (`TOILE-010`) | `logic::convergence_tests::deleting_a_group_cascades_to_members` |

---

## 3. MODEL

**Containers.** A group is an element of the `elements` map whose kind is the group kind, with a
membership list nested under it (`ambre-spec-toile-schema.md section 3`). It reuses the frame's
membership machinery (`features/ambre-spec-toile-frames.md`).

**What a group does not carry.** No geometry. The element map of a group holds the kind, the rotation,
the lock flag and a style register, and no coordinates at all (`TOILE-150`). Its box lives only in
the derived bounds cache and the spatial index (`TOILE-009`).

**Commands** (`ambre-bus`), one undo item each: `GroupElements { ids }` and
`UngroupElements { groups }`.

**Steps, when a group is bound:**

1. keep the existing, distinct ids in the order requested (`TOILE-151`);
2. drop any id that is an ancestor of another id in the set, keeping the descendants
   (`TOILE-152`);
3. stop when fewer than two members survive (`TOILE-055`);
4. mint the group and write its membership list. A board that refuses either write refuses the
   group, with every member where it was (`TOILE-273`);
5. detach each member the list holds from the group or frame it sat in (`TOILE-149`), then prune
   anything the detach emptied (`TOILE-054`);
6. take the lowest place any member holds in the paint order;
7. insert the group at that place (`TOILE-052`). When every member sat in one frame, the group
   joins that frame in their place, so the frame keeps what it held (`TOILE-149`);
8. commit once, then recompute every group's box to a fixed point.

**Steps, when groups are unbound:** for each listed group, remove the group element. Every list that
names the group drops it, so no frame keeps a dead entry. The group's own parent then takes its
children (`TOILE-157`). That parent is the enclosing group, else the frame with the smallest id
that holds the group.

Then strip the paint-order entries in one rewrite. A frame that took children clamps them above
it (`TOILE-011`) before the commit, so the clamp undoes with the unbind. Commit once and recompute
(`TOILE-010`, `TOILE-156`, `TOILE-094`). An id that is not a group changes nothing (`TOILE-159`).
Beside a group it answers nothing, and a list with no group answers one error (`TOILE-264`). The
chord sends the whole selection rather than filter it: the selection payload may lag a click, and
the Logic thread reads the document itself.

**How a transform reaches the members.** A move expands each selected id to the leaves of its subtree
and writes each leaf's coordinates, then recomputes the group boxes bottom-up (`TOILE-154`,
`TOILE-053`). A group is never resized: its box is derived, so there is nothing to write
(`TOILE-155`).

**How the UI resolves a click.** Each painted element carries the id of its immediate enclosing group
(`TOILE-UI-119`), and the UI walks that edge upward (`features/ambre-spec-toile-selection.md`).

**A membership cycle.** No command of ours puts a group inside itself, but a peer can write one. Each
walk of the group tree therefore stops where it has been. The leaf expansion enters each group once
(`TOILE-276`), and the UI walk stops at the first group already on its chain (`TOILE-UI-398`). The
box recomputation runs at most one pass more than the groups it walks (`TOILE-275`). A cycle
keeps its stale box on the incremental path, and that box can differ from a rebuild. This is the one
exception `TOILE-186` names.

**How the parent is found.** The Logic thread keeps a derived map from each element to the containers
whose membership list names it, one entry per occurrence. Each write to a membership list updates
the map in the same step. A remote change, an undo and an import reseed it in the pass that
rebuilds the boxes, at no walk of its own.

A read takes the smallest id among the entries still a group, or any that is a frame. It walks no
board (`TOILE-056`). Entries arrive in a different order on each peer, so the first entry would
not converge (`TOILE-266`).

**Code:**

* `crates/ambre-toile/src/logic/state/groups.rs` -- binding, unbinding, the parent lookup, the subtree
  walks, the pruning and the box recomputation;
* `logic/state/elements.rs` -- the leaf expansion a move uses, and the cascade a delete uses;
* `logic/state/derive.rs` -- the group's derived box, and the parent field on every painted element.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+G` on a selection of two or more | `GroupElements` over the selection |
| `Ctrl+Shift+G` on a selection | one `UngroupElements` over the selection |
| Group and Ungroup, in the object menu and the context menu | the same two commands (`features/ambre-spec-toile-menus.md`) |

**What a group looks like.** Nothing of its own: no fill, no border, no title (`TOILE-UI-120`). What
the user sees is one selection box around the union of the members, with the handles on it
(`features/ambre-spec-toile-selection.md`).

**Entering a group.** A double-click peels one nesting level, so the next click reaches a member;
`Escape` leaves. Both are view state and neither touches the document
(`features/ambre-spec-toile-selection.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A group's box is derived, never stored | storing the union, refreshed on every member change | two peers writing a box for the same group would race for a value that is a function of other values, and the loser's box would contradict the members it is supposed to contain. Deriving it makes the contradiction unrepresentable |
| An element's parent is a derived map | storing the parent in each element, as a field | it is a format change, and two peers binding one element at once would race on a register the membership lists already settle. The lists stay the truth; the map mirrors them, and a rebuild reseeds it |
| A group reuses the frame's membership machinery | a second container model for groups | the two differ in what they paint and in what a delete does, not in how a set of children is held. One list type, one detach rule, one ordering helper |
| A group enters the order below its lowest member | inserting the group above the selection | it reverses the members, and they must keep painting in the order they had; a group that paints nothing has no reason to be anywhere else |
| An empty group is deleted | keeping it, as a container a user could refill | a rebuild from the document would never materialize it, so the incremental caches would disagree with a rebuild. A group is created with two members and only shrinks; one with none is meaningless |
| Deleting a group deletes its members | detaching the members | it turns one gesture into a silent ungroup. The binding is the object the user selected, so a delete on it is a delete on what it holds -- a frame is the opposite, because a frame is a region |
| An ancestor selected with its own descendant loses | refusing the whole request | the request is contradictory but the user's intent is not, and answering nothing leaves them to find the offending pair themselves. Keeping the ancestor is what cannot be done: it would empty during the detach and be pruned mid-operation, leaving the new group holding an id nobody has |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A write that touches a group walks the board.** Recomputing the group boxes and pruning empty
  groups each resolve the whole element map. A bind, an unbind, a move and a delete pay that walk
  once or more, whatever their size. The read side no longer does (`TOILE-056`)
  (`ambre-roadmap.md`, "A board costs what it shows and what changed").
* **Re-parented children land at the end of the list** (`TOILE-157`). An ungroup appends them to
  the enclosing group's list, not at the place the group held. No rule says which place they keep.
* **A group cannot be resized** (`TOILE-155`). Scaling the members under one drag is not designed, and
  the resize command refuses a group. The handles a group shows still offer a stretch. The command
  writes nothing, so the box springs back to its members' union on release (`TOILE-UI-391`,
  `TOILE-UI-392`).
* **Nesting has no depth limit.** A group may hold a group, to any depth, and the box recomputation
  runs to a fixed point over the whole set each time. A pathological nesting is neither refused nor
  measured.
