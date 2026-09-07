# AMBRE -- FEATURE: FRAMES

**Type:** Vertical (feature)
**Implementation:** partial -- membership and ordering ship; nothing on the board joins an element to a frame.

> A frame is a titled region of the board. It holds the elements a user puts in it, paints behind
> them, and doubles as a slide when the board is presented.

A frame is an ordinary element with a membership list. That is why moving one, resizing one and
deleting one need no code of their own.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-011` | `INV` | A frame's children MUST paint after the frame: a child's place in the paint order MUST be later than its frame's. | this spec section 3 | `ambre_toile::logic::convergence_tests::frame_children_paint_after_their_frame`, which creates both children before the frame so the clamp has to fire, and reads the paint order back. The two membership tests this row also cited never read that order | verified |
| `TOILE-049` | `INV` | An element MUST belong to at most one container: joining a frame MUST detach it from its group. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_detaches_from_a_group`, which reads both memberships back | verified |
| `TOILE-051` | `CONC` | A frame's speaker-notes container MUST exist from its birth, so two peers editing it first MUST converge. | this spec section 3; `TOILE-029` | `ambre_toile::logic::convergence_tests::concurrent_notes_edits_converge` | verified |

### 1.2 Add a selection to a frame

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-099` | `POST` | Adding elements to a frame MUST make each a member, and removing them MUST take each out. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_adds_and_removes_membership` reads the membership back after an add of two and a remove of one | verified |
| `TOILE-100` | `POST` | An element removed and added again in one gesture MUST end up last in the frame. | this spec section 3 | none, and it is the half `TOILE-099` was carrying. The handler removes before it adds for exactly this, and the membership test does its add and its remove in two separate commands | unverified |
| `TOILE-048` | `POST` | A membership change MUST be one undo item, the clamping of the paint order included. | this spec section 3 | `ambre_toile::logic::convergence_tests::set_frame_children_is_one_undo_item` creates the frame before the child, so the child already paints above it and the clamp never fires; **the rule fails when it does** -- the two commits leave in one broadcast and split into two undo items, so the first undo reverts the reordering alone -- section 6 | unverified |
| `TOILE-101` | `FRAME` | An element the frame cannot take MUST NOT stop the rest: the frame itself, an id the board does not hold and a current member are skipped, and the other elements of the same gesture still join. | this spec section 3 | none. Each of the three is a `continue` in the add loop, and no test mixes an eligible id with an ineligible one | unverified |
| `TOILE-102` | `FRAME` | A membership change that changes nothing MUST write nothing and broadcast nothing. | this spec section 3 | none. The handler returns before any commit when neither list moved, and no test sends a gesture with no effect | unverified |
| `TOILE-050` | `FAIL` | A membership change aimed at an element that is not a frame MUST be refused, and MUST write nothing. | this spec section 3 | none. The handler returns silently, with no event at all, where a frame the board does not hold returns an unknown-element error, so a surface cannot tell the refusal from a no-op. Neither path is tested | unverified |

* **Budget.** the membership change under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. The clamp is a loop that re-materializes the whole paint
  order per moved child, so the figure is per child out of order and no bench measures it.
* **Boundary.** none of its own: a frame and its membership live in the Logic thread's document and
  leave as bytes the Sync worker seals (`ZK-001`).
* **Verification.** 4 `verified`, 5 `unverified`. `TOILE-011` cited three tests and only one reads a
  paint order; the other two read memberships, which is what `TOILE-099` now says and they now hold.
  `TOILE-048` said `none` while a test of it exists -- it creates the frame before the child, so the
  clamp it is about never fires, and the row is now honest about what it covers. `TOILE-050` was four
  refusals under one verdict, and they are not one behaviour: a non-frame stops the gesture, while an
  id the board does not hold is skipped and the rest of the gesture proceeds (`TOILE-101`). The
  re-append the removal order exists for (`TOILE-100`) and the empty change (`TOILE-102`) were
  unwritten.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a shape to a frame | adds the same shape to another frame | the shape in one of the two frames, never in both | the shape in the frame that won, and no duplicate | `logic::convergence_tests::set_frame_children_detaches_from_a_group` |
| adds a shape to a frame | brings that shape to the back | the membership, and a paint order the clamp corrected | the shape above its frame on both screens (`TOILE-011`) | `logic::convergence_tests::frame_children_paint_after_their_frame` |
| edits the frame's speaker notes | edits the same notes at the same moment | both sentences, character by character | one merged text, with neither sentence lost | `logic::convergence_tests::concurrent_notes_edits_converge` |
| deletes the frame | moves a child of it | the frame gone; the child moved and no longer a member | the child alone on the board | `logic::convergence_tests::caches_match_a_full_rebuild` |

Deleting a frame does not delete its members: only a group cascades
(`features/ambre-spec-toile-groups.md`).

---

## 3. MODEL

**Containers.** A frame is an element of the `elements` map whose kind is the frame kind. It owns two
nested containers, both declared in `ambre-spec-toile-schema.md section 3`: a membership list, and a
speaker-notes text. Its optional layout register belongs to
`features/ambre-spec-toile-auto-layout.md`.

**Command** (`ambre-bus`): `SetFrameChildren { frame, add, remove }`, meant as one undo item.
Creating, moving, resizing, rotating, reordering and deleting a frame use the ordinary element
commands (`features/ambre-spec-toile-elements.md`).

**Steps, when membership changes:**

1. resolve the frame and refuse anything that is not one (`TOILE-050`);
2. remove the listed ids from the membership list first, so a remove and an add of one id re-appends
   it at the end (`TOILE-099`, `TOILE-100`);
3. for each added id, skip the frame itself, an id the board does not hold, and a current member,
   without stopping the gesture (`TOILE-101`);
4. detach each survivor from its enclosing group, so it belongs to one container
   (`TOILE-049`);
5. stop and write nothing when neither list changed (`TOILE-102`);
6. prune a group the move emptied;
7. commit;
8. clamp every child that sits below the frame in the paint order to just above it, and commit again
   if anything moved (`TOILE-011`, and the reason `TOILE-048` fails).

**Why the containers are seeded at birth.** The notes text and the membership list are created in the
frame's own creation write, not on the first edit. A lazy per-peer seed would mint two rival
containers under one key, and the merge would keep one and drop the other user's sentences. The rule
is the format's (`TOILE-029`), and the frame is the place the code applies it (`TOILE-051`).

**Code:**

* `crates/ambre-toile/src/logic/state/groups.rs` -- the membership handler, the clamp, and the tree
  helpers both frames and groups share;
* `logic/state/elements.rs` -- the birth seed of the two containers;
* `logic/state/derive.rs` -- the frame's paint node, and the snapshot the presentation pass takes of
  its declared children.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| the frame tool, dragged on the board | `CreateElement` with the frame kind (`features/ambre-spec-toile-tools.md`) |
| "Add to frame", in the context menu over a selection inside a frame | `SetFrameChildren` adding the selection (`features/ambre-spec-toile-menus.md`) |

**What a frame looks like.** A rectangle painted from its own style register, before its members,
so it reads as a background rather than a box drawn over them. Its title is designed and unwritten
(`features/ambre-spec-toile-find.md`).

**What no surface offers.** Removing an element from a frame, and dropping an element into a frame by
dragging it there. The context menu's one entry adds; nothing removes, and membership is never
implied by position.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A frame is an element | a parallel container map, with a duplicated lifecycle and a per-frame stack | as an element, move, resize, rotate, delete and reorder apply for free and there is one paint order rather than one per frame; only the frame-specific content needs handlers |
| Membership is declared, never spatial | membership by containment, recomputed on every move | a user drags a shape across a frame constantly, so every drag would change the document, and two overlapping frames would leave the membership ambiguous |
| The ordering rule is enforced, not assumed | trusting the writers | a concurrent reorder can put a child under its frame, and the frame would then paint over its own contents. Clamping is cheap and makes the nesting a property rather than a convention |
| The ordering rule is enforced, not assumed | painting the frame in a pass of its own | a frame painted in its own pass is always in front or always behind, so the paint order stops saying anything about two frames overlapping, and the author loses a control they had |
| An element belongs to one container | multiple membership, resolved at derivation | a shape in a frame and in a group has two owners for one position, and the two would fight over it on every transform |
| Deleting a frame keeps its members | cascading like a group | it makes a frame dangerous to delete. A frame is a region, so removing the region should not remove what was inside it; a group is the opposite, because a group is a binding |
| The notes and membership containers are born with the frame | seeding them on first use | the concurrent-first-write collision is silent and loses user text, and it costs two empty containers per frame to make impossible |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A membership change can take two undos** (`TOILE-048`). The handler commits the membership write,
  then clamps the paint order and commits again, so a frame given a child that painted below it
  produces two undo items: the first `Ctrl+Z` reverts the reordering and the child is still a member.
  The two tests are exactly complementary -- the undo test creates the frame before the child, so no
  clamp fires, and the ordering test creates the child first, so the clamp fires and nothing counts
  the items. The split is invisible to the network: both commits leave in the one delta the handler
  exports, so only the author of the change sees it.
* **A refusal and a no-op are the same event** (`TOILE-050`, `TOILE-102`). A gesture aimed at an
  element that is not a frame and a gesture that changes nothing both return with no event at all,
  where a frame the board does not hold returns an unknown-element error. A surface that wanted to
  tell the user why nothing happened has nothing to read.
* **Nothing puts an element into a frame by dragging it.** The only trigger is a context-menu entry
  that requires the selection to already sit inside the frame's box, and nothing takes an element out
  again. A frame is therefore a region a user can draw and can barely fill.
* **The membership list is seeded beside the notes and nothing proves it.** Both containers are born
  with the frame, and only the notes have a concurrent-first-write test (`TOILE-051`). Two peers each
  adding the first child to a fresh frame is the same collision, untested.
* **A frame has no title.** The search walks a frame's name field and no command writes it, so a frame
  is unnamed on the board and unfindable by name (`features/ambre-spec-toile-find.md`).
* **A resized frame does not carry its members.** The members keep their world positions, so shrinking
  a frame leaves them outside it and nothing says whether they are still members. They are.
* **Nesting a frame in a frame is neither allowed nor refused.** The handler refuses only the frame
  joining itself, so a frame may be added to another frame's membership, and the clamp then orders
  them; what that means for the presentation deck is not ruled
  (`features/ambre-spec-toile-presentation.md`).
