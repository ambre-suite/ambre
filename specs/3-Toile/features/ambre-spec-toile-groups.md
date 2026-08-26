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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-009` | A group's box MUST be the derived union of its members' boxes, never independently authored. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members`, `::derive_populates_the_group_edge_on_members`, `::moving_a_group_translates_its_members` | verified |
| `TOILE-010` | Ungrouping MUST preserve the surviving children and their relative order; deleting a group MUST cascade to its members. | this spec section 3 | `ambre_toile::logic::convergence_tests::ungroup_keeps_children_and_order`, `::deleting_a_group_cascades_to_members` | verified |
| `TOILE-052` | A group MUST enter the paint order below its lowest member, so the members keep their relative order above it. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_unions_bounds_and_sits_below_members` | verified |
| `TOILE-053` | A transform on a group MUST reach every leaf of its subtree, however deep the nesting. | this spec section 3 | `ambre_toile::logic::convergence_tests::moving_a_group_translates_its_members`; the nested case is inside the two-peer property, which generates a group of a group (`TOILE-017`) | verified |
| `TOILE-054` | A group left with no member MUST be removed, so the caches stay equal to a rebuild. | this spec section 3; `TOILE-004` | `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` holds the equality the pruning exists for. No test drives a group to empty and asserts it is gone | unverified |
| `TOILE-055` | A group MUST refuse to bind fewer than two distinct members, and MUST refuse to bind an element to its own ancestor. | this spec section 3 | none. Both refusals are in the binding handler and neither has a test; the ancestor case is the one a user reaches by selecting a group with one of its own members | unverified |
| `TOILE-056` | Reading which container an element belongs to MUST NOT cost a walk of the whole board. | `ambre-spec-budgets.md section 1`; this spec section 6 | none, and **the rule fails today**: the lookup materializes the entire element map and scans it, and the visible-set walk calls it once per visible element -- section 6 | unverified |

* **Budget.** the transform command under 1 ms and a remote change under 2 ms, two rows of
  `ambre-spec-budgets.md section 1`. `TOILE-056` is why neither holds on a large board: the
  derivation is quadratic in the element count, and no bench measures it.
* **Boundary.** none of its own: the binding lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`). The group box is derived and never persisted, so it never leaves
  at all.
* **Verification.** 4 `verified`, 3 `unverified`. The binding, the derived box and the cascade are
  held five ways. The gaps are the housekeeping -- pruning and the two refusals -- and one
  performance rule the code answers with the opposite.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| groups two shapes | moves one of those shapes | the group, and the shape at its new position | the group box widened to cover the move, on both screens (`TOILE-009`) | `logic::convergence_tests::group_unions_bounds_and_sits_below_members` |
| moves the group | moves one member | both translations, applied to the member | the member offset by the sum, the others by the group's move alone | `logic::convergence_tests::moving_a_group_translates_its_members` |
| ungroups | drags the group | the binding gone; the drag applied to whatever it resolved to | the elements loose, in their order | `logic::convergence_tests::ungroup_keeps_children_and_order` |
| deletes the group | edits a member | the members gone, and the edit landing on nothing | an empty region on both screens (`TOILE-010`) | `logic::convergence_tests::deleting_a_group_cascades_to_members` |

---

## 3. MODEL

**Containers.** A group is an element of the `elements` map whose kind is the group kind, with a
membership list nested under it (`ambre-spec-toile-schema.md section 3`). It reuses the frame's
membership machinery (`features/ambre-spec-toile-frames.md`).

**What a group does not carry.** No geometry. The element map of a group holds the kind, the rotation,
the lock flag and a style register, and no coordinates at all: its box lives only in the derived
bounds cache and the spatial index (`TOILE-009`).

**Commands** (`ambre-bus`), one undo item each: `GroupElements { ids }` and
`UngroupElements { group }`.

**Steps, when a group is bound:**

1. keep the existing, distinct ids in the order requested;
2. drop any id that is an ancestor of another id in the set, keeping the descendants
   (`TOILE-055`);
3. stop when fewer than two members survive;
4. detach each member from its current container, then prune anything the detach emptied;
5. take the lowest place any member holds in the paint order;
6. mint the group, write its membership list, and insert the group at that place
   (`TOILE-052`);
7. commit once, then recompute every group's box to a fixed point.

**Steps, when a group is unbound:** remove the group element and its paint-order entry, re-parent its
children to the group's own parent, and drop the dead reference from that parent before appending
them; then commit and recompute (`TOILE-010`).

**How a transform reaches the members.** A move expands each selected id to the leaves of its subtree
and writes each leaf's coordinates, then recomputes the group boxes bottom-up (`TOILE-053`). A group
is never resized: its box is derived, so there is nothing to write.

**How the UI resolves a click.** Each painted element carries the id of its immediate enclosing group,
and the UI walks that edge upward (`features/ambre-spec-toile-selection.md`). Producing that one field
is what `TOILE-056` fails on.

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

**What a group looks like.** Nothing of its own: no fill, no border, no title. What the user sees is
one selection box around the union of the members, with the handles on it
(`features/ambre-spec-toile-selection.md`).

**Entering a group.** A double-click peels one nesting level, so the next click reaches a member;
`Escape` leaves. Both are view state and neither touches the document
(`features/ambre-spec-toile-selection.md`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A group's box is derived, never stored | two peers writing a box for the same group would race for a value that is a function of other values, and the loser's box would contradict the members it is supposed to contain. Deriving it makes the contradiction unrepresentable | storing the union, refreshed on every member change |
| A group reuses the frame's membership machinery | the two differ in what they paint and in what a delete does, not in how a set of children is held. One list type, one detach rule, one ordering helper | a second container model for groups |
| A group enters the order below its lowest member | the members must keep painting in the order they had, and a group that paints nothing has no reason to be anywhere else | inserting the group above the selection, which reverses the members |
| An empty group is deleted | a group is created with two members and only shrinks; one with none is meaningless, and a rebuild from the document would never materialize it, so leaving it makes the incremental caches disagree with a rebuild | keeping it, as a container a user could refill |
| Deleting a group deletes its members | the binding is the object the user selected, so a delete on it is a delete on what it holds. A frame is the opposite, because a frame is a region | detaching the members, which turns one gesture into a silent ungroup |
| An ancestor selected with its own descendant loses | the request is contradictory, and keeping the ancestor would empty it during the detach and prune it mid-operation, leaving the new group holding an id nobody has | refusing the whole request |

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
  undos. The ids that are not groups are rejected no-ops.
* **A group cannot be resized** (`TOILE-009`). Scaling the members under one drag is not designed, and
  the resize command names a single element, so the handles a group shows can move it and rotate it
  and not stretch it.
* **The pruning is proven only through the rebuild equality** (`TOILE-054`). The property test would
  catch a group left behind, because the caches would stop matching a rebuild, and no test states the
  rule directly.
* **Nesting has no depth limit.** A group may hold a group, to any depth, and the box recomputation
  runs to a fixed point over the whole set each time. A pathological nesting is neither refused nor
  measured.
