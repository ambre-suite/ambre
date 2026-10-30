# AMBRE -- FEATURE: ELEMENTS ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- the lifecycle ships, align and distribute with it. The lock misses
the delete, and half the commands swallow an id nobody minted.

> Everything on a board is an element: a note, a shape, a picture, a frame, a stroke. One set of
> operations moves them, resizes them, rotates them, restyles them, locks them and deletes them,
> whatever they are.

The kinds differ only by the content they carry. That is why a frame needs no lifecycle of its own,
and why one table below covers the whole board.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-186` | `INV` | Every cache the board derives -- the paint order, the bounds and the spatial index -- MUST equal what a rebuild from the document alone produces. The one exception is the box of a group in a membership cycle (`TOILE-276`). | this spec section 3 | proptest -- `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` draws a random operation sequence and compares the three caches against a rebuild | none | verified |
| `TOILE-033` | `INV` | A locked element MUST NOT be changed by any command but the lock itself. | this spec section 3 | `ambre_toile::logic::convergence_tests::locked_element_ignores_move` -- the move alone (`TOILE-012`); `ambre_toile::logic::structure_tests::a_locked_element_refuses_every_command_but_the_lock` -- the resize, the rotate, the restyle, the body overwrite, the delete and the reorder, each writing nothing, a group holding the locked element kept whole by a delete, then the unlock that lets the delete through; `ambre_toile::logic::convergence_tests::two_peers_converge` -- the lock drawn among the operations two peers interleave | none | verified |
| `TOILE-274` | `INV` | Each stored coordinate and size MUST be read, and written, held on its own inside the world bound, and one that is not finite MUST read as zero, as a rotation that is not finite does. | this spec section 3; `TOILE-186` | `ambre_toile::logic::state::caches::peer_geometry_tests::a_peer_nan_coordinate_panics_no_index` -- a NaN, an infinity and a huge size on one element of eight, through a delta and a reopen: no panic, each box finite, eight leaves indexed, and a move after it; `::a_drag_past_the_world_bound_holds_at_the_bound` -- a drag past the bound stores the bound, the stored corner equals the corner read, and a nudge back loses nothing | none | verified |
| `TOILE-187` | `CONC` | Two peers that applied different operations to one board MUST hold the same elements, the same box per element and the same paint order. | this spec section 2 | proptest -- `ambre_toile::logic::convergence_tests::two_peers_converge` draws two operation sequences and compares the elements, the boxes and the paint order | none | verified |

### 1.2 Place an element on the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-188` | `PRE` | The caller MUST hand over a birth rectangle of usable size. | this spec section 3 | `ambre_toile::logic::structure_tests::a_birth_box_of_no_usable_size_is_refused` -- an empty, a negative and a non-finite box and a non-finite corner, each refused with no write, and a flat box with one side still accepted | none | verified |
| `TOILE-031` | `POST` | A newly created element MUST enter the paint order, the bounds cache and the spatial index in the same commit that mints it. | this spec section 3 | `ambre_toile::logic::convergence_tests::create_registers_bounds_and_layer` -- the population, the box and the paint order; proptest -- `ambre_toile::logic::convergence_tests::caches_match_a_full_rebuild` is the only one that reaches the index | none | verified |
| `TOILE-035` | `POST` | A create MUST seed the style a kind is born with. | this spec section 3 | `ambre_toile::logic::structure_tests::each_kind_is_born_with_its_own_style` -- each kind's created element read back against the style its kind seeds | none | verified |
| `TOILE-189` | `POST` | A colour picked at placement MUST override the fill alone, leaving the rest of the seeded style as the kind sets it. | this spec section 3 | structural -- the handler builds the per-kind style, then assigns the picked colour to its fill field and to nothing else | none | verified |
| `TOILE-190` | `FRAME` | A create MUST NOT touch an element already on the board, nor reorder what it lands over. | this spec section 3 | structural -- the handler inserts one element map, one bounds entry and one index entry, and appends to the paint order | none | verified |
| `TOILE-022` | `FAIL` | A derived kind (`group`, `drawing`) MUST NOT be creatable through the create command, and the refusal MUST answer an error. | this spec section 3 | `ambre_toile::logic::convergence_tests::group_and_drawing_are_not_creatable` | none | verified |
| `TOILE-191` | `FAIL` | A seed that does not match the kind MUST be refused, and MUST leave the board unchanged. | this spec section 3 | `ambre_toile::logic::structure_tests::a_seed_of_another_kind_is_refused` -- a mismatched seed answers its error, and the document's state is the one it had | none | verified |

### 1.3 Move, resize or rotate an element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-032` | `POST` | A move MUST translate the element's rotation-expanded box, so culling and connector routing stay exact. | this spec section 3; `TOILE-005` | `ambre_toile::logic::convergence_tests::move_translates_bounds` | none | verified |
| `TOILE-192` | `POST` | A resize MUST record the dragged rectangle normalized, so a negative drag is still a rectangle. | this spec section 3 | `ambre_toile::logic::convergence_tests::resize_updates_bounds` | none | verified |
| `TOILE-193` | `POST` | A rotate MUST grow the recorded box to the corners the angle sweeps. | this spec section 3 | `ambre_toile::logic::convergence_tests::rotate_expands_the_aabb` -- it reads the four bounds against the sweep | none | verified |
| `TOILE-012` | `FRAME` | A move MUST NOT touch a locked element. | this spec section 3 | `ambre_toile::logic::convergence_tests::locked_element_ignores_move` | none | verified |
| `TOILE-036` | `FAIL` | A resize, a rotate or a body overwrite naming an element the board does not hold MUST answer an error and MUST leave the document unchanged. | this spec section 3 | `ambre_toile::logic::structure_tests::a_write_to_a_missing_element_is_refused` -- a resize, a rotate and a body overwrite on a missing id, each answering the error with the document unchanged | none | verified |
| `TOILE-194` | `FAIL` | A move or a restyle naming an element the board does not hold MUST answer an error too. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_move_naming_a_missing_element_answers_an_error`; `::a_restyle_naming_a_missing_element_answers_an_error` -- each names a ghost id and reads a `NoSuchElement` error back | none | verified |

### 1.4 Overwrite the body of a note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-195` | `POST` | Given a body unchanged since the base it names, a body overwrite MUST leave it reading exactly the text written; an edit since the base merges in (`TOILE-UI-381`). | this spec section 3 | `ambre_toile::logic::convergence_tests::set_text_overwrites_sticky_body` | none | verified |
| `TOILE-196` | `FRAME` | A body overwrite MUST NOT touch the element's geometry, its style or its place in the paint order. | this spec section 3 | structural -- the handler writes one field of one element map | none | verified |
| `TOILE-197` | `FAIL` | A body overwrite MUST reject an element that carries no body, and MUST answer an error. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_text_rejects_a_non_text_element` | none | verified |

### 1.5 Delete an element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-034` | `POST` | Deleting an element MUST drop its paint-order entry, its bounds and its index entry in the commit that removes it. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it reads the population and the paint order once the removal lands | none | verified |
| `TOILE-198` | `POST` | Deleting an element MUST remove every connector that touched it at either endpoint. | this spec section 3 | `ambre_toile::logic::convergence_tests::delete_drops_owned_connectors` -- it deletes one of two joined elements and leaves the board with the other alone | none | verified |
| `TOILE-199` | `POST` | Deleting a group MUST take its members with it. | this spec section 3 | `ambre_toile::logic::convergence_tests::deleting_a_group_cascades_to_members` | none | verified |
| `TOILE-271` | `POST` | Deleting an element MUST drop its id from every membership list that names it, a frame's as well as a group's. | this spec section 3; `TOILE-149` | `ambre_toile::logic::convergence_tests::a_delete_takes_the_id_off_its_frame_list` -- the frame's list read back on the deleter and on the peer the delta reached; `::a_pruned_group_leaves_its_frame_list` -- a group the delete emptied leaves the frame that held it; `::a_delete_concurrent_with_a_reorder_leaves_no_frame_member_behind` -- the history `TOILE-017` found, each peer's caches equal to a rebuild; `ambre_toile::logic::state::groups::cache_tests::a_stored_frame_list_naming_a_deleted_element_still_edits` -- a board an older build stored with the stale entry loads, and a remove or a frame delete keeps the caches equal to a rebuild | none | verified |
| `TOILE-200` | `FRAME` | A delete MUST NOT remove an element that is neither named nor a member of one that is. | this spec section 3 | structural -- the handler expands the named ids to their subtrees once and walks that set alone | none | verified |
| `TOILE-201` | `FAIL` | A delete naming an element the board does not hold MUST answer an error. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_delete_naming_a_missing_element_answers_an_error` -- it names a ghost id and reads a `NoSuchElement` error back | none | verified |

### 1.6 Lock and unlock an element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-202` | `POST` | Locking or unlocking MUST write the flag on every named element. | this spec section 3 | structural -- `lock_elements` walks the named ids and writes the flag on each element map it finds | none | verified |
| `TOILE-239` | `POST` | Locking or unlocking MUST NOT be refused by the flag it is clearing. | this spec section 3 | structural -- it is the one command over an existing element that does not read the lock before writing, which is what makes an unlock reachable | none | verified |
| `TOILE-203` | `FRAME` | Locking or unlocking MUST NOT change an element's geometry, its style or its place in the paint order. | this spec section 3 | structural -- the handler writes one boolean field per named element | none | verified |

### 1.7 Align or distribute a selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-037` | `POST` | Aligning or distributing a selection MUST be one convergent batch and one undo item. | this spec section 4 | `ambre_toile::logic::state::elements::inner_tests::an_alignment_is_one_undo_item` -- it reads every box back after one `Undo`; proptest -- `ambre_toile::logic::convergence_tests::two_peers_converge` draws an align over every live element, the one operation that rewrites the whole board's coordinates at once | none | verified |
| `TOILE-204` | `FRAME` | Aligning or distributing MUST NOT move an element the selection does not hold, and MUST NOT move any element on the axis it does not work on. | this spec section 4 | `ambre_toile::logic::state::elements::inner_tests::an_alignment_moves_only_what_it_names` -- it reads back a bystander and the untouched axis of a named box | none | verified |
| `TOILE-234` | `FRAME` | A locked element MUST take no part in an alignment: it MUST NOT move, and it MUST NOT set the edge the others align to. | this spec section 4 | `ambre_toile::logic::state::elements::inner_tests::a_locked_element_takes_no_part` -- a locked box 500 units to the left neither moves nor drags the others onto its edge | none | verified |
| `TOILE-235` | `PRE` | An alignment MUST need two boxes. | this spec section 4 | `ambre_toile::logic::align::tests::a_lone_box_is_already_aligned` -- the lone box an alignment writes nothing for | none | verified |
| `TOILE-242` | `PRE` | A distribution MUST need three boxes. | this spec section 4 | `ambre_toile::logic::align::tests::a_pair_has_nothing_to_distribute` -- the pair a distribution writes nothing for | none | verified |
| `TOILE-243` | `PRE` | Below those counts, and where every translation comes out zero, the operation MUST write nothing -- no commit, no undo item, no delta. | this spec section 4 | `ambre_toile::logic::state::elements::inner_tests::an_alignment_that_changes_nothing_mints_no_undo_item` -- the silent handler | none | verified |

* **Budget.** a command that writes one register and re-derives the visible set under 1 ms, the
  transform-command row of `ambre-spec-budgets.md section 1`. No bench measures it. A move over a
  selection is one register pair per leaf, so the figure is per element and the drag rate is the
  multiplier (`features/ambre-spec-toile-tools.md`).
* **Boundary.** none of its own: an element lives in the Logic thread's document and leaves as bytes
  the Sync worker seals (`ZK-001`). No lifecycle command opens a socket or holds a key.
* **Verification.** 35 `verified`. Align and distribute holds through nine tests over the pure
  geometry and five over the handler. A place in the 2-peer property strategy lets `TOILE-037` claim
  convergence rather than assert it. The lock is driven through every command it binds. A birth is
  held on its box, its seed and its style.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a note right | moves the same note down | both translations, applied to the two coordinates independently | the note at the diagonal of the two moves | `logic::convergence_tests::two_peers_converge` |
| restyles a shape red | restyles the same shape blue | one of the two whole styles, never a blend | the same colour on both screens (`TOILE-018`) | `logic::convergence_tests::two_peers_converge` |
| deletes a shape | resizes the same shape | the deletion: a resize of an absent element writes nothing | the shape gone on both screens | `logic::convergence_tests::caches_match_a_full_rebuild` |
| locks a shape | drags the same shape | the shape where it was | nothing moves, and neither user is told why | `logic::convergence_tests::locked_element_ignores_move` |

The lock is advisory and local to the operation, not a permission: it refuses the write on the peer
that attempts it. Who may write at all is `ambre-spec-architecture.md section 2`.

Every refusal in that column is silent (`TOILE-033`). A locked element drops the write and answers
with an empty change list. The user who dragged it sees the drag snap back and reads no notice.
Section 6 carries what that costs.

---

## 3. MODEL

**Containers.** The `elements` map and the `layers` list, both declared in
`ambre-spec-toile-schema.md section 3`. An element is one map under a minted id; the field layout per
kind is that document's field table.

**Commands** (`ambre-bus`), one per operation and one undo item each:

| Shape | Carries |
|---|---|
| `CreateElement { kind, at, size, content, fill }` | the kind, the birth rectangle, the type-specific seed, and an optional fill override |
| `MoveElements { ids, delta }` | a translation over a whole selection, groups expanded to their leaves |
| `ResizeElement { id, new }` | the new rectangle, normalized so a negative drag is still a rectangle |
| `RotateElement { id, radians }` | an absolute angle about the box centre |
| `RestyleElements { ids, style }` | one whole style register per element (`TOILE-018`) |
| `SetText { id, base, text }` | the whole body of a note or a text element and the body it was edited from, merged into its text (`TOILE-030`, `TOILE-UI-381`) |
| `LockElements { ids, locked }` | the advisory lock, set or cleared |
| `AlignElements { ids, op }` | one of eight one-shot batches over the selection: six alignments and two distributions (`TOILE-037`) |
| `DeleteElements { ids }` | the removal, cascading through a group's members |

**Steps, when an element is created:**

1. refuse a derived kind (`TOILE-022`), refuse a seed that does not match the kind
   (`TOILE-191`), and refuse a birth box of no usable size (`TOILE-188`). A usable size is finite and
   not negative, with at least one side, so a level pen stroke is still taken. The corner must be
   finite too;
2. mint an id (`ambre-spec-toile-schema.md section 2`) and open the element map under it;
3. write the geometry block, the lock flag and the per-kind style (`TOILE-035`), with the picked
   fill applied over its fill field alone (`TOILE-189`);
4. seed the type-specific fields, and for a frame its membership list and its notes container
   (`features/ambre-spec-toile-frames.md`);
5. push the element onto the top of the paint order;
6. record the rotation-expanded box in the bounds cache and the spatial index (`TOILE-031`);
7. commit once, then answer with the minted id and a fresh visible set.

**Steps, when an element is deleted:**

1. expand every listed id to its whole subtree, so deleting a group takes its members
   (`TOILE-199`). A frame is a region, not a binding, so deleting one keeps its members
   (`features/ambre-spec-toile-frames.md`);
2. remove each element map, and stop when nothing was there (`TOILE-201`);
3. drop each removed id from every membership list that names it, a frame's as well as a group's
   (`TOILE-271`). The removal is a document write, so a peer refreshing that container unlinks the
   id too;
4. remove every connector that referenced a removed id at either endpoint (`TOILE-198`);
5. strip both the elements and those connectors from the paint order;
6. drop the bounds and index entries, then prune a group the deletion emptied (`TOILE-054`);
7. commit once (`TOILE-034`).

**The rotation-expanded box.** The bounds cache holds the axis-aligned box of the *rotated* element,
which is larger than its own rectangle. The index therefore over-approximates on purpose: the exact
geometry is tested on the UI thread (`features/ambre-spec-toile-selection.md`).

**A stored box is read, never trusted.** A peer can store any number in a coordinate, a size or a
rotation. The R-tree panics on a NaN corner, and a NaN box never settles a group fixpoint. So the
field readers read a value that is not finite as zero, and hold a finite one inside the import world
bound (`TOILE-274`). The board still loads: a bad value never refuses the document. The index also
reads a box that is not finite as the empty box at the origin, for any other caller.

Every write of a box holds it inside the same bound. That covers a create, a move, a resize, an
alignment, a paste, a stroke and a boolean result. A value read back is then the value written. The
bound holds each field on its own, so the far edge of a box can reach twice the bound.

**Code:**

* `crates/ambre-toile/src/logic/state/elements.rs` -- every handler above, plus the commit-free
  variants an import batches into one undo item;
* `state/codec.rs` -- the rotation-expanded box, the per-kind seed and the field readers;
* `state/derive.rs` -- how each kind becomes something to paint.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| a creation tool released on the board | `CreateElement` with the tool's kind (`features/ambre-spec-toile-tools.md`) |
| a drag on a selection | `MoveElements` once, on release |
| a drag on a corner or an edge handle | `ResizeElement` once, on release |
| a drag on the rotate handle | `RotateElement` once, on release |
| `Delete` or `Backspace` | `DeleteElements` over the selection |
| an arrow key, alone or with `Shift` | `MoveElements` by one world unit, or ten |
| the style controls | `RestyleElements` (`features/ambre-spec-toile-styles.md`) |
| Lock and Unlock, in the object menu or the context menu | `LockElements` (`features/ambre-spec-toile-menus.md`) |
| the eight align and distribute entries, in the object menu or the context menu | `AlignElements` (`features/ambre-spec-toile-menus.md`) |

**What a lock looks like.** A locked element shows no handles and cannot be picked, so it stops
absorbing clicks over a background someone drew on
(`features/ambre-spec-toile-selection.md`). The flag binds every command but the lock itself
(`TOILE-033`). A delete skips a locked element, and keeps whole any group that holds one. A reorder of
either writes nothing.

**Align and distribute** (`TOILE-037`) is one command, `AlignElements { ids, op }`, over eight
operations. Six put every selected box on an edge or a centre line of the selection's **union box**.
Two equalize the **gaps** between three or more boxes, with the extremes pinned. The reference is
always the selection itself, never the viewport and never a frame, so the selection as a whole never
moves.

The boxes are the derived rotation-expanded ones the picker already uses, so a rotated element
aligns by what the user sees. A group is one box and carries its subtree, the way a drag does. A
locked element is dropped before the union is measured (`TOILE-234`). It cannot move, and a locked
full-page background setting the reference edge would swallow the whole operation. Every translation
lands under one `doc.commit()`, so a peer sees a whole alignment or none of it.

The entries are the eight rows of the **Object menu**. The context menu shows them only where they
apply: the six alignments from two selected elements, the two distributions from three
(`features/ambre-spec-toile-menus.md`). No keyboard chord and no toolbar button: the floating
selection toolbar carries style, not geometry.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Every kind shares one lifecycle | frame-specific and group-specific commands, which the earlier design carried | they duplicate move, resize, rotate, delete and reorder once per kind, and split the paint order across as many structures. A frame, a group and a note differ only by the content they carry, so a move is a move |
| Align and distribute is a batch of coordinates | a persisted alignment rule on the container | it makes two peers fight over a constraint neither can see. The result is a set of positions, and the positions are what converges |
| A distribution equalizes the gaps between boxes | equalizing the distance between their centres | on a board of notes and shapes the sizes differ, and even centres leave uneven whitespace -- the thing the user is actually looking at. Even centres are the office-suite convention; even gaps are the whiteboard one, and Toile is a whiteboard |
| A distribution keeps the extremes where they are, even when the boxes overlap | clamping the gap at zero, the way the auto-layout solver does | pinning the extremes is what makes the operation idempotent and what makes two peers agree on the result. A clamp would push an extreme, so running it twice would walk the selection across the board |
| A locked element is dropped before the union is measured | keeping it in the union as an immovable guide | a guide is a feature nobody asked for here, and the failure mode is severe: a locked background covering the board makes every alignment snap the selection to a corner of the page |
| A create is one commit, and an import is one commit for the whole batch | a commit inside every write | it makes an import of 400 elements 400 undo items. A user placing a note wants one undo and a user importing a file wants one undo, and both need the same handler, so the write and the commit are separate steps |
| The lock refuses the write rather than hiding the element | a visual lock state, with the refusal on the transform | greying the element out would make a board built of colour unreadable, and locking exists so a user can draw over a background and keep clicking through it |
| The bounds cache holds the rotated box | indexing the unrotated rectangle | it loses a rotated element at the edge of the screen. The index answers "what might be visible", and a rotated element sticks out of its own rectangle; over-approximating there costs one exact test per candidate and never drops a hit |
| A rejected command answers an error and writes nothing | a best-effort write over the ids that resolved | a partial write on a bad id is the one failure a convergent document cannot recover from, because the other peers receive it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Select-all takes locked elements** (`TOILE-UI-033`). It walks what is displayed rather than
  what is pickable. The delete that follows spares them, but the selection still names elements no
  command will change.
* **A refusal says nothing to anyone** (`TOILE-033`). Every handler that honours the lock returns an
  empty change list, not an error. No notice, no cursor, no log, and the same silence for the user
  who attempted the write as for the peers who did not.
* **The property tests never overwrite a body.** The operation strategy the two of them draw from
  now locks and unlocks, but carries no body overwrite.
* **Align and distribute has no chord and no button** (`TOILE-037`). It is reachable from the Object
  menu and the context menu only. The floating selection toolbar carries style, and neither the
  style inspector nor the keyboard offers it.
* **Nothing aligns an arrow** (`TOILE-037`). The command names elements. A selected connector --
  which has a selection of its own now (`features/ambre-spec-toile-connectors.md`) -- is not
  something the operation can take. An arrow has no box to align, only two ends, and what a
  distribution would mean for one is not designed.
* **A resize does not scale what is inside.** A resized frame keeps its children where they were.
  A resized group is not refused either, though its box is derived. The write lands in the group's
  own map and the bounds are re-read from it. The pass that follows overwrites the box with the
  union of the members (`features/ambre-spec-toile-groups.md`). The user gets one undo item and a
  delta for a change nobody can see; scaling a subtree is not designed.
* **No kind carries a name.** An element has no user-facing label, so a board of two hundred shapes
  offers nothing to search but the bodies of its notes (`features/ambre-spec-toile-find.md`).
