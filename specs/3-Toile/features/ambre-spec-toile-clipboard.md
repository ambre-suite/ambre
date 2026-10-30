# AMBRE -- FEATURE: CLIPBOARD AND DUPLICATE

**Type:** Vertical (feature)
**Implementation:** partial -- copy, cut, paste and duplicate ship; a plain paste does not.

> A user copies a selection and pastes it, on this board or another, in this tab or another. What
> comes back is the same drawing with new identities, so pasting onto the source is safe.

A copy carries the selection whole: the arrows inside it, the grouping inside it, and nothing that
would point outside it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The transport half is
`ambre-spec-clipboard.md`, shared with the other applications and not restated here.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-069` | `INV` | A copied payload MUST carry no identity of the board it came from: every element, every arrow end and every membership MUST be named by its rank in the payload alone. | this spec section 3 | `ambre_toile::logic::convergence_tests::paste_reproduces_selection_on_a_separate_board` -- it pastes into a board that has never seen the source, where an identity the payload carried would resolve to nothing. The round-trip test this row also cited holds the row below | none | verified |
| `TOILE-130` | `IO` | A payload written to the clipboard and read back MUST be the payload that was written. | this spec section 3; `ambre-spec-clipboard.md` | `ambre_toile::logic::convergence_tests::clipboard_payload_postcard_roundtrips` -- it compares the decoded payload with the encoded one, which is what a round trip holds rather than the identity rule above. The payload it drives carries neither a membership nor an arrow, so two of the four parts cross no encoder | none | verified |

### 1.2 Copy a selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-068` | `POST` | A copy MUST read the whole selection, not the part on screen. | this spec section 3 | structural -- the copy runs on the Logic thread over the element maps, and that thread holds no viewport, so nothing exists to cull against. The neighbouring test this row cited walks the aggregates instead (`features/ambre-spec-toile-status-bar.md`) | none | verified |
| `TOILE-131` | `POST` | A copied group MUST carry its members, however deep the grouping goes. | this spec section 3 | `ambre_toile::logic::convergence_tests::duplicate_a_group_is_one_undo_item` -- on the element count: one copied id lands three elements, so the subtree walk is driven. It is the half `TOILE-071` was carrying, and the half a count can hold | none | verified |
| `TOILE-132` | `POST` | A copied frame MUST NOT carry its members: a frame is a region, and only a binding travels with what it binds. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_copied_frame_carries_no_member_it_was_not_copied_with` -- a frame copied alone lands empty, and one copied with its member binds the copy | none | verified |
| `TOILE-133` | `POST` | A payload MUST carry the box of what was copied, so a paste knows where the selection sat. | this spec section 3 | `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` -- it reads the source box off the payload and compares it with the copied element. Unwritten until now: section 3 carried the placement step, and no row said the payload holds the box that step needs | none | verified |
| `TOILE-134` | `POST` | An arrow MUST be copied only when both of the elements it joins are copied. | this spec section 3 | `ambre_toile::logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied` -- it reads an empty arrow list off a one-endpoint copy and a single arrow off a two-endpoint one. It is the half `TOILE-013` was carrying, and it is a property of the copy rather than of the paste | none | verified |
| `TOILE-067` | `FRAME` | A copy MUST NOT change the document, and MUST NOT reach the other users. | this spec section 3 | `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` -- the absent delta and the unchanged element count; type-level -- the command is classified as non-mutating in `ambre-bus`, so the permission gate never treats it as a write | none | verified |

### 1.3 Paste or duplicate

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-013` | `POST` | A paste or a duplicate MUST re-mint every identity, so pasting onto the board that was copied is safe. | this spec section 3 | `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` -- it pastes onto the source board and reads back an id that differs from the one copied, beside a count that grew. The arrow half is `TOILE-134`, and the round trip it also cited holds `TOILE-130` | none | verified |
| `TOILE-135` | `POST` | A paste MUST put the copied box at the point it is asked for. | this spec section 3 | `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` -- it reads the pasted box back at the requested point, two hundred units off the source. Unwritten until now: the offset lived in section 3 as a step, and a paste that ignored the point would still land a fresh element | none | verified |
| `TOILE-070` | `POST` | A paste and a duplicate MUST each be one undo item, however many elements they land. | this spec section 3; `TOILE-014` | `ambre_toile::logic::convergence_tests::duplicate_a_group_is_one_undo_item` -- the single undo takes a duplicated group of three back out | none | verified |
| `TOILE-071` | `POST` | A pasted group MUST be bound to its own copies, and to nothing outside the set that was copied. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_pasted_group_binds_its_own_copies` -- it reads the pasted group's members back, and the source group's | none | verified |
| `TOILE-136` | `POST` | A pasted arrow MUST join the copies, never the elements that were copied. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_pasted_arrow_joins_the_copies` -- it reads both ends of the pasted arrow | none | verified |
| `TOILE-137` | `POST` | A pasted frame MUST carry its speaker notes. | this spec section 3; `TOILE-051` | `ambre_toile::logic::convergence_tests::a_pasted_frame_carries_its_notes_and_takes_an_element` -- the notes read back off the copy, and two peers' first edits of them converging, which a scalar field would not | none | verified |
| `TOILE-138` | `POST` | A pasted frame MUST be able to take an element, as any frame can. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_pasted_frame_carries_its_notes_and_takes_an_element` -- the same frame, copied alone, taking an element afterwards | none | verified |
| `TOILE-139` | `FRAME` | A paste MUST NOT touch what is already on the board. | this spec section 3 | structural -- every write of a paste goes under an identity minted from the peer and a counter that only grows, so no path reaches an element the board already holds | none | verified |
| `TOILE-260` | `POST` | A plain paste MUST land the plain text as one text element at the pointer, whatever payload the clipboard or this tab's own copy holds. | this spec section 4; `PROSE-UI-227` | not built | unbuilt -- lands with the `Paste as plain text in Toile` Story (`ambre-roadmap.md` E6). A paste with no payload returns before it writes anything, so text from outside lands nowhere | unimplemented |
| `TOILE-140` | `FRAME` | A paste of an empty payload MUST write nothing and reach nobody. | this spec section 3 | `ambre_toile::logic::convergence_tests::an_empty_paste_writes_nothing` -- no delta, no create, and the same history | none | verified |

* **Budget.** the paste under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. A paste reseeds every derived cache from the document rather
  than inserting incrementally, so the figure is per board and not per pasted element.
* **Boundary.** the payload leaves the client. It is written to the operating system clipboard under
  a private type, outside the sealed path (`ZK-001`), by the user's own copy gesture. The plain-text
  half is left empty, so nothing readable by another application leaves.
* **Verification.** 17 `verified`, 1 `unimplemented`. What holds is the round trip, the pasted box,
  and what a pasted set is bound to. Tests read back a pasted group's members, a pasted arrow's ends,
  and a frame copied with and without its member.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| copies two shapes and an arrow between them | watches | nothing: a copy writes nothing | no change on either screen (`TOILE-067`) | `logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` |
| pastes onto the same board | watches | two more shapes and one more arrow, with fresh identities | the copy under the cursor on both screens (`TOILE-013`, `TOILE-135`) | `logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied` |
| copies one end of an arrow | pastes it | the shape alone | no arrow to nowhere (`TOILE-134`) | `logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied` |
| copies a selection on one board | pastes it on another | the same drawing, in that board's own identities | the same shapes, arrows and grouping (`TOILE-069`) | `logic::convergence_tests::paste_reproduces_selection_on_a_separate_board` |
| duplicates a group | deletes one of its members | the copy whole, and the original short one member | one intact copy, and one gap in the original (`TOILE-131`) | `logic::convergence_tests::duplicate_a_group_is_one_undo_item` |

---

## 3. MODEL

**Containers.** None of its own. A copy reads the `elements`, `connectors` and `layers` containers; a
paste writes them (`ambre-spec-toile-schema.md section 3`).

**The payload** (`ambre-bus`) is self-contained and identity-free. Elements are referenced by slot,
arrows have two ends that are slots, and a membership is parent and child slots. The source
box is carried too, so a paste can be placed:

```rust
pub struct ClipboardPayload {
    pub elements:   Vec<ClipElement>,   // slot-indexed; no source identity leaks
    pub connectors: Vec<ClipConnector>, // both ends remapped to slots
    pub membership: Vec<ClipMembership>, // parent slot -> child slots
    pub origin:     kurbo::Rect,        // source box, for placement
}
```

**Commands** (`ambre-bus`): `CopyElements { ids }`, answered by an event carrying the payload;
`PasteElements { payload, at }`; `DuplicateElements { ids, delta }`, which is a copy and a paste fused
into one undo item. Cut is not a command: the surface sends a copy and then a delete.

**Steps, when a selection is copied:**

1. expand every id to its whole subtree, so a copied group carries its members and a copied frame
   does not (`TOILE-131`, `TOILE-132`);
2. order the captured set by the paint order, and give each element a slot;
3. capture each element's scalar fields, container fields excluded, and union the boxes into the
   source box (`TOILE-133`). A frame's speaker notes travel beside those fields, as their text, because
   they are a shared container and not a scalar (`TOILE-137`);
4. remap each group's and frame's membership to child slots, keeping only children in the set;
5. keep an arrow only when both of its ends are in the set (`TOILE-134`);
6. answer with the payload, having written nothing (`TOILE-067`).

**Steps, when a payload is pasted:**

1. stop on an empty payload, having written nothing (`TOILE-140`);
2. mint every fresh identity up front, so membership and arrow ends can be remapped before anything is
   written (`TOILE-013`, `TOILE-139`);
3. write each element with its captured fields, adding the placement offset to its coordinates
   (`TOILE-135`);
4. push each onto the top of the paint order. A frame is given the two containers it is born with.
   Its membership list, so the copy takes an element (`TOILE-138`), and its notes text, seeded with
   what was captured (`TOILE-137`, `TOILE-051`);
5. rebuild each group's and frame's membership list with the new identities (`TOILE-071`);
6. write each arrow with its ends remapped (`TOILE-136`);
7. commit once, reseed the derived caches, and answer with the new elements
   (`TOILE-070`).

The placement offset is the vector that puts the source box at the requested point, so a paste lands
under the cursor (`TOILE-133`, `TOILE-135`). A duplicate uses a small fixed nudge instead.

**Code:**

* `crates/ambre-toile/src/logic/state/clipboard.rs` -- the capture and the materialization;
* `logic/state/codec.rs` -- the scalar capture and the offset write;
* `ui/view.rs` -- the two host-driven halves: the synchronous copy answer and the paste.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the browser copy event, with a selection | sends `CopyElements`, defers, and mirrors the answer to the clipboard next frame |
| the browser cut event | the same, then `DeleteElements` on the same selection |
| the browser paste event | sends `PasteElements` at the pointer's world position |
| `Ctrl+Shift+V`, or its menu entry | `TOILE-260`, on the `Paste as plain text in Toile` Story (`ambre-roadmap.md` E6) |
| `Ctrl+D` | sends `DuplicateElements` with a small diagonal nudge |
| Copy, Cut, Paste and Duplicate, in the edit and context menus | the same four paths (`features/ambre-spec-toile-menus.md`) |

**Inside a text edit, the clipboard is text.** With the inline editor open, copy and cut take the
selected characters, and paste inserts characters at the caret. An element payload means nothing in
a text field (`features/ambre-spec-toile-text-editing.md`).

**Why a copy is answered rather than read.** The UI thread holds only what is on screen, so it cannot
build the payload for a selection that reaches past the edge. It asks, and the Logic thread answers
from the document (`TOILE-068`), which is also why the answer names nothing of this board
(`TOILE-069`).

**Same-tab fidelity.** The answered payload is kept in memory as well as written to the clipboard.
A paste that still owns the clipboard prefers the memory copy. A failed asynchronous write to the
operating system therefore costs nothing inside the tab (`ambre-spec-clipboard.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The payload carries slots, never identities | carrying the source identities and re-minting on collision | it needs a collision test the second board cannot run. A pasted element must not collide with its source, and a payload that travels to another board cannot carry that board's identities; slots make both cases the same code |
| A half-copied arrow is dropped | keeping it dangling | it would arrive pointing at an element that is not there, which is the dangling state, deliberately entered. A copy is a user asking for a self-contained piece |
| A half-copied arrow is dropped | re-binding it to the source element on the same board | it ties the copy to an element the user did not copy, and the rule would then differ by destination: the same paste onto another board has no source element to bind to |
| The identities are minted before anything is written | writing then patching | membership and arrow ends have to be remapped, and remapping after the write means a second pass over what was just written |
| A copy is answered by the Logic thread | building the payload from what is displayed | the selection can reach past the screen, and the UI thread holds the screen; guessing is a silent truncation, asking is one round trip |
| Cut is a copy and a delete, not a command | a cut command | the two halves already exist and the order is guaranteed by the queue, so a third command would only duplicate them |
| A duplicate is fused into one undo item | a copy item and a paste item | the user made one gesture, and two items would make the first `Ctrl+Z` leave the copy behind |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A frame is the one kind whose containers the copy knows about.** The capture reads scalars and
  names the notes explicitly. A container added to another kind would travel as whatever its deep
  value flattens to, or not at all, and nothing would say so.
* **A copied frame leaves its members behind** (`TOILE-132`). The subtree walk descends into a group
  and not into a frame, so copying a frame alone lands an empty region. It follows the frame rule
  that a deletion keeps its members (`features/ambre-spec-toile-frames.md`). A user who copies a
  frame to copy its content must select the content too.
* **A copy loses what is filed under an identity.** Reactions and comments are stored beside the
  board under the element's own id, and a paste mints a new one. The copy arrives with neither
  (`features/ambre-spec-toile-reactions.md`, `features/ambre-spec-toile-comments.md`). Whether a
  duplicate should carry its reactions is unruled, and a copy across boards plainly cannot.
* **A paste reseeds every derived cache.** The materialization rebuilds the bounds map, the index and
  the paint order from the whole document rather than inserting the pasted nodes. Pasting five
  elements onto a large board costs a full rebuild (`ambre-roadmap.md`, "Engine and render
  optimization").
* **A paste from another application is ignored.** Only the private payload is decoded. Pasted text
  drops on the board rather than becoming a note, and a pasted image drops rather than being imported
  (`features/ambre-spec-toile-import-export.md`).
* **The plain-text mirror is empty.** A copy out of Ambre into another application therefore yields
  nothing, by decision. No rule says what the text form of a drawing should be.
* **An image is copied by reference.** A copied picture carries its content hash, so the two copies
  share one stored blob. What a paste onto a board that cannot reach that blob shows is
  `features/ambre-spec-toile-media.md`.
