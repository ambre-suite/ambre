# AMBRE -- FEATURE: GROUPS

**Type:** Vertical (feature)
**Implementation:** partial -- grouping ships; the parent lookup scans the whole board per element.

> A user selects several elements and binds them: from then on one click takes the set, and one drag
> moves it. Ungrouping gives the elements back, in place.

A group paints nothing. It exists to make a set behave as one object, and its box is the box of what
it holds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-149` | `INV` | An element MUST belong to at most one container, a group or a frame. | this spec section 3 | none: the binding detaches a member from its current container and the frame handler does the same, and no test puts one element in two lists and reads the second one back | unverified |
| `TOILE-009` | `INV` | A group's box MUST be the union of its members' boxes. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members` asserts the union of two corner rects; `::moving_a_group_translates_its_members` asserts it again after a translation | verified |
| `TOILE-150` | `INV` | A group MUST carry no coordinates of its own. | this spec section 3 | structural, and the construction is the proof: the group seed writes a kind, a rotation, a lock flag and a style register, and no x, y, width or height, so there is no field a peer could author a box into | verified |
| `TOILE-054` | `INV` | A group left with no member MUST be removed, so the caches stay equal to a rebuild. | this spec section 3; `TOILE-004` | none: `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` would catch a group left behind, because a rebuild never materializes a childless group's box and the incremental cache would keep the stale one. No test drives a group to empty and asserts it is gone | unverified |
| `TOILE-056` | `INV` | Reading which container an element belongs to MUST NOT cost a walk of the whole board. | `ambre-spec-budgets.md section 1`; this spec section 6 | none, and **the rule fails today**: the lookup materializes the entire element map and scans it, and the visible-set walk calls it once per visible element -- section 6 | unverified |
| `TOILE-UI-119` | `OBS` | Every painted element MUST name its immediate enclosing group, and MUST name none when it has no group. | this spec section 3 | `ambre_toile::logic::convergence_tests::derive_populates_the_group_edge_on_members` reads the edge off the derived viewport for both members and for the group itself, three assertions for the two halves of the rule | verified |
| `TOILE-UI-120` | `OBS` | A group MUST paint nothing of its own: no fill, no border, no title. | this spec section 4 | none: the derivation emits the group as a node with a derived box, and no test reads a group node's style back -- what the user sees around it is the selection box, which another document owns | unverified |

### 1.2 Bind a selection into a group

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-052` | `POST` | A group MUST enter the paint order below its lowest member, so the members keep their relative order above it. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members`, its second assertion: the paint order reads group, then the two members in their prior order | verified |
| `TOILE-151` | `POST` | An id the board no longer holds MUST be dropped from the binding. | this spec section 3 | none: the filter reads the bounds cache for each requested id, and no test asks to bind an id a peer has just deleted | unverified |
| `TOILE-152` | `POST` | A selected element that encloses another selected element MUST be dropped from the binding, and the enclosed ones MUST be bound. | this spec section 3; section 5 | none: the filter walks each member's subtree looking for a sibling of the request, and no test selects a group together with one of its own members -- section 6 | unverified |
| `TOILE-153` | `FRAME` | Binding a group MUST NOT move any member. | this spec section 3 | none: the binding writes a membership list and a paint-order entry, and the binding test reads the group's own box back and never a member's | unverified |
| `TOILE-055` | `FAIL` | A binding left with fewer than two distinct members MUST create no group. | this spec section 3 | none, and the refusal is silent: the handler returns an empty outcome with no notice, so a user pressing the chord on one element sees nothing at all -- section 6 | unverified |

### 1.3 Move a group

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-154` | `POST` | A transform on a group MUST reach every member. | this spec section 3 | `ambre_toile::logic::convergence_tests::moving_a_group_translates_its_members` asserts both members at their translated rects | verified |
| `TOILE-053` | `POST` | A transform on a group MUST reach every leaf of its subtree, however deep the nesting. | this spec section 3 | none: the test above nests nothing, and the two-peer property (`TOILE-017`) generates a group of a group but asserts agreement, not correctness -- a move that stopped at the first level would leave both peers agreeing on it -- section 6 | unverified |
| `TOILE-155` | `FRAME` | A transform on a group MUST NOT resize it: its box is derived. | this spec section 3 | none: the resize command names one element and the group carries no geometry to write (`TOILE-150`), so the refusal is structural at the schema and unread at the command -- section 6 | unverified |

### 1.4 Ungroup

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-010` | `POST` | Ungrouping MUST preserve the surviving children and their relative order. | this spec section 3 | `ambre_toile::logic::convergence_tests::ungroup_keeps_children_and_order`, its second assertion: the paint order after the unbind reads the two children in their prior order | verified |
| `TOILE-156` | `POST` | Ungrouping MUST remove the group itself. | this spec section 3 | the same test, its first assertion: the group has no box after the unbind | verified |
| `TOILE-157` | `POST` | The children MUST be re-parented to the group's own container. | this spec section 3 | none: the test ungroups a top-level group, where the container is the board and the re-parenting is a no-op. The nested case, where the children join the enclosing group's list, is driven by nothing | unverified |
| `TOILE-158` | `FRAME` | Ungrouping MUST NOT move any element. | this spec section 3 | none: the unbind writes a membership list and a paint-order entry, and the test reads the order back, never a position | unverified |
| `TOILE-159` | `FAIL` | Ungrouping an id that is not a group MUST change nothing. | this spec section 3 | none, and the refusal is silent: the handler returns an empty outcome, which is what makes ungrouping a mixed selection a partial success nobody is told about -- section 6 | unverified |
| `TOILE-160` | `FAIL` | Ungrouping an id the board does not hold MUST tell the user. | this spec section 3 | none: the handler returns the missing-element error, and no test reads that notice back for this command | unverified |

### 1.5 Delete a group

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-161` | `POST` | Deleting a group MUST delete its members. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_a_group_cascades_to_members` asserts an empty board and an empty paint order after deleting the group of two | verified |
| `TOILE-162` | `FRAME` | Deleting a group MUST NOT remove an element outside its own subtree. | this spec section 3 | none: the delete expands the request to the subtree and strips exactly that set, plus the connectors it orphans, and no test puts a bystander on the board and reads it back | unverified |

* **Budget.** the transform command under 1 ms and a remote change under 2 ms, two rows of
  `ambre-spec-budgets.md section 1`. `TOILE-056` is why neither holds on a large board: the
  derivation is quadratic in the element count, and no bench measures it.
* **Boundary.** none of its own: the binding lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`). The group box is derived and never persisted, so it never leaves
  at all.
* **Verification.** 8 `verified`, 15 `unverified`. No mechanism was gained and none was lost: the
  same five tests carry nine distinct assertions between them, and each verified row but one now
  names the assertion it rests on instead of sharing a pile. `TOILE-009` was the clearest case --
  it cited a test that reads no box at all, only the group edge every painted element carries, which
  is a rule of its own (`TOILE-UI-119`). The eighth verified row is structural: a group has no field
  a box could be authored into. What the split exposes is that the gestures are held at their happy
  path and nowhere else: every refusal, every re-parenting, every frame condition and the whole of
  the nested case are unverified, and two of the refusals are silent besides.

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
the lock flag and a style register, and no coordinates at all (`TOILE-150`): its box lives only in the
derived bounds cache and the spatial index (`TOILE-009`).

**Commands** (`ambre-bus`), one undo item each: `GroupElements { ids }` and
`UngroupElements { group }`.

**Steps, when a group is bound:**

1. keep the existing, distinct ids in the order requested (`TOILE-151`);
2. drop any id that is an ancestor of another id in the set, keeping the descendants
   (`TOILE-152`);
3. stop when fewer than two members survive (`TOILE-055`);
4. detach each member from its current container (`TOILE-149`), then prune anything the detach
   emptied (`TOILE-054`);
5. take the lowest place any member holds in the paint order;
6. mint the group, write its membership list, and insert the group at that place
   (`TOILE-052`);
7. commit once, then recompute every group's box to a fixed point.

**Steps, when a group is unbound:** remove the group element and its paint-order entry, re-parent its
children to the group's own parent (`TOILE-157`), and drop the dead reference from that parent
before appending them; then commit and recompute (`TOILE-010`, `TOILE-156`).

**How a transform reaches the members.** A move expands each selected id to the leaves of its subtree
and writes each leaf's coordinates, then recomputes the group boxes bottom-up (`TOILE-154`,
`TOILE-053`). A group is never resized: its box is derived, so there is nothing to write
(`TOILE-155`).

**How the UI resolves a click.** Each painted element carries the id of its immediate enclosing group
(`TOILE-UI-119`), and the UI walks that edge upward (`features/ambre-spec-toile-selection.md`).
Producing that one field is what `TOILE-056` fails on.

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
| `Ctrl+Shift+G` on a selection | `UngroupElements` once per selected id |
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
| A group reuses the frame's membership machinery | a second container model for groups | the two differ in what they paint and in what a delete does, not in how a set of children is held. One list type, one detach rule, one ordering helper |
| A group enters the order below its lowest member | inserting the group above the selection | it reverses the members, and they must keep painting in the order they had; a group that paints nothing has no reason to be anywhere else |
| An empty group is deleted | keeping it, as a container a user could refill | a rebuild from the document would never materialize it, so the incremental caches would disagree with a rebuild. A group is created with two members and only shrinks; one with none is meaningless |
| Deleting a group deletes its members | detaching the members | it turns one gesture into a silent ungroup. The binding is the object the user selected, so a delete on it is a delete on what it holds -- a frame is the opposite, because a frame is a region |
| An ancestor selected with its own descendant loses | refusing the whole request | the request is contradictory but the user's intent is not, and answering nothing leaves them to find the offending pair themselves. Keeping the ancestor is what cannot be done: it would empty during the detach and be pruned mid-operation, leaving the new group holding an id nobody has |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The parent lookup scans the whole board** (`TOILE-056`). Reading which group an element belongs to
  materializes the entire element map and walks it looking for a membership list that names the id.
  The visible-set walk calls that lookup once per visible element, so deriving a viewport of a hundred
  elements on a board of fifty thousand walks fifty thousand entries a hundred times, every frame.
  Binding, unbinding, deleting and pruning call it inside their own loops. Nothing in the document
  stores an element's parent, which is the fix and a format change
  (`ambre-roadmap.md`, "Engine and render optimization").
* **Ungrouping a multi-selection is one command per id** (`TOILE-010`). The chord and the menu entry
  send one unbind per selected id, each committing separately, so ungrouping three groups takes three
  undos. The ids that are not groups are rejected no-ops (`TOILE-159`).
* **Both refusals are silent** (`TOILE-055`, `TOILE-159`). A binding of fewer than two distinct
  members and an unbind of something that is not a group both return an empty outcome: no error, no
  notice, no cursor change. A user who presses the chord on a single element and a user who presses
  it on a mixed selection see the same thing, which is nothing, and the second one got a partial
  result.
* **The nested transform rests on a property that cannot fail on it** (`TOILE-053`). The two-peer
  property generates a group of a group and asserts that the two peers agree. A move that reached
  only the first level of a subtree would leave both peers agreeing on the same wrong positions, so
  the property is an oracle of convergence and not of correctness. Nothing nests a group by hand and
  reads a leaf's coordinates back.
* **The ancestor filter is stated and untested** (`TOILE-152`). Selecting a group together with one
  of its own members is the request a user reaches by rubber-banding over a group; the code drops the
  ancestor and binds the members, which is a result, not a refusal, and no test states either.
* **Re-parenting is exercised only where it is a no-op** (`TOILE-157`). The unbind test ungroups a
  top-level group, so the branch that appends the children to an enclosing group's list -- and the
  order they land in, which is the end of the list and not their old place -- is driven by nothing.
* **A group cannot be resized** (`TOILE-155`). Scaling the members under one drag is not designed, and
  the resize command names a single element, so the handles a group shows can move it and rotate it
  and not stretch it.
* **The pruning is proven only through the rebuild equality** (`TOILE-054`). The property test would
  catch a group left behind, because the caches would stop matching a rebuild, and no test states the
  rule directly.
* **Nesting has no depth limit.** A group may hold a group, to any depth, and the box recomputation
  runs to a fixed point over the whole set each time. A pathological nesting is neither refused nor
  measured.
