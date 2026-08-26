# AMBRE -- FEATURE: CLIPBOARD AND DUPLICATE

**Type:** Vertical (feature)
**Implementation:** implemented -- copy, cut, paste and duplicate all ship.

> A user copies a selection and pastes it, on this board or another, in this tab or another. What
> comes back is the same drawing with new identities, so pasting onto the source is safe.

A copy carries the selection whole: the arrows inside it, the grouping inside it, and nothing that
would point outside it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The transport half is
`ambre-spec-clipboard.md`, shared with the other applications and not restated here.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-013` | A paste or duplicate MUST re-mint every identity; an arrow wholly inside the copy MUST survive and a half-copied one MUST be dropped. | this spec section 3 | `ambre_toile::logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied`, `::copy_does_not_mutate_and_paste_remints_offset`, `::clipboard_payload_postcard_roundtrips` | verified |
| `TOILE-067` | A copy MUST NOT change the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset`; the command is classified as non-mutating in `ambre-bus`, so the permission gate never treats it as a write | verified |
| `TOILE-068` | A copy MUST read the whole selection, not the part on screen. | this spec section 3 | `ambre_toile::logic::convergence_tests::selection_stats_spans_offscreen_elements` holds the same property for the neighbouring walk; the copy runs on the Logic thread over the element maps, so the viewport is out of reach by construction | verified |
| `TOILE-069` | A copied payload MUST carry no identity, so it can be pasted onto another board. | this spec section 3 | `ambre_toile::logic::convergence_tests::paste_reproduces_selection_on_a_separate_board`, `::clipboard_payload_postcard_roundtrips` | verified |
| `TOILE-070` | A paste and a duplicate MUST each be one undo item, however many elements they land. | this spec section 3; `TOILE-014` | `ambre_toile::logic::convergence_tests::duplicate_a_group_is_one_undo_item` | verified |
| `TOILE-071` | A copy of a group MUST carry its members, and their binding, without carrying its parent. | this spec section 3 | `ambre_toile::logic::convergence_tests::duplicate_a_group_is_one_undo_item` covers the subtree landing as one item; nothing asserts that the pasted group is bound to its own copies and to nothing outside the set | unverified |

* **Budget.** the paste under 1 ms, the transform-command row of
  `ambre-spec-budgets.md section 1`. A paste reseeds every derived cache from the document
  rather than inserting incrementally, so the figure is per board and not per pasted element, and no
  bench measures it.
* **Boundary.** the payload leaves the client. It is written to the operating system clipboard under a
  private type, which is outside the sealed path (`ZK-001`), by the user's own copy gesture and inside
  the browser's activation window. The plain-text half is left empty, so nothing readable by another
  application leaves (`ambre-spec-clipboard.md`).
* **Verification.** 5 `verified`, 1 `unverified`. The identity re-minting, the arrow rule and the
  cross-board round trip are held six ways -- the best-covered surface of the application after the
  index. The gap is what a copied group is bound to.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| copies two shapes and an arrow between them | watches | nothing: a copy writes nothing | no change on either screen (`TOILE-067`) | `logic::convergence_tests::copy_does_not_mutate_and_paste_remints_offset` |
| pastes onto the same board | watches | two more shapes and one more arrow, with fresh identities | the copy offset from the source, on both screens | `logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied` |
| copies one end of an arrow | pastes it | the shape alone | no arrow to nowhere (`TOILE-013`) | `logic::convergence_tests::paste_keeps_fully_copied_connector_drops_half_copied` |
| copies a selection on one board | pastes it on another | the same drawing, in that board's own identities | the same shapes, arrows and grouping (`TOILE-069`) | `logic::convergence_tests::paste_reproduces_selection_on_a_separate_board` |
| duplicates a group | deletes one of its members | the copy whole, and the original short one member | one intact copy, and one gap in the original | `logic::convergence_tests::duplicate_a_group_is_one_undo_item` |

---

## 3. MODEL

**Containers.** None of its own. A copy reads the `elements`, `connectors` and `layers` containers; a
paste writes them (`ambre-spec-toile-schema.md section 3`).

**The payload** (`ambre-bus`) is self-contained and identity-free: elements referenced by slot,
arrows whose two ends are slots, container membership as parent and child slots, and the source box so
a paste can be placed:

```rust
pub struct ClipboardPayload {
    pub elements:   Vec<ClipElement>,   // slot-indexed; no source identity leaks
    pub connectors: Vec<ClipConnector>, // both ends remapped to slots
    pub membership: Vec<ClipMembership>, // container slot -> child slots
    pub origin:     kurbo::Rect,        // source box, for placement
}
```

**Commands** (`ambre-bus`): `CopyElements { ids }`, answered by an event carrying the payload;
`PasteElements { payload, at }`; `DuplicateElements { ids, delta }`, which is a copy and a paste fused
into one undo item. Cut is not a command: the surface sends a copy and then a delete.

**Steps, when a selection is copied:**

1. expand every id to its whole subtree, so a copied group carries its members
   (`TOILE-071`);
2. order the captured set by the paint order, and give each element a slot;
3. capture each element's scalar fields, container fields excluded, and union the boxes into the
   source box;
4. remap each container's membership to child slots, keeping only children in the set;
5. keep an arrow only when both of its ends are in the set (`TOILE-013`);
6. answer with the payload, having written nothing (`TOILE-067`).

**Steps, when a payload is pasted:**

1. mint every fresh identity up front, so membership and arrow ends can be remapped before anything is
   written (`TOILE-013`);
2. write each element with its captured fields, adding the placement offset to its coordinates;
3. push each onto the top of the paint order;
4. rebuild each container's membership list with the new identities;
5. write each arrow with its ends remapped;
6. commit once, reseed the derived caches, and answer with the new elements
   (`TOILE-070`).

The placement offset is the vector that puts the source box at the requested point, so a paste lands
under the cursor. A duplicate uses a small fixed nudge instead.

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
| `Ctrl+D` | sends `DuplicateElements` with a small diagonal nudge |
| Copy, Cut, Paste and Duplicate, in the edit and context menus | the same four paths (`features/ambre-spec-toile-menus.md`) |

**Inside a text edit, the clipboard is text.** With the inline editor open, copy and cut take the
selected characters and paste inserts characters at the caret: an element payload means nothing in a
text field (`features/ambre-spec-toile-text-editing.md`).

**Why a copy is answered rather than read.** The UI thread holds only what is on screen, so it cannot
build the payload for a selection that reaches past the edge. It asks, and the Logic thread answers
from the document (`TOILE-068`).

**Same-tab fidelity.** The answered payload is kept in memory as well as written to the clipboard, and
a paste that still owns the clipboard prefers the memory copy -- so a failed asynchronous write to the
operating system costs nothing inside the tab (`ambre-spec-clipboard.md`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The payload carries slots, never identities | a pasted element must not collide with its source, and a payload that travels to another board cannot carry that board's identities. Slots make both cases the same code | carrying the source identities and re-minting on collision, which needs a collision test the second board cannot run |
| A half-copied arrow is dropped | it would arrive pointing at an element that is not there, which is the dangling state, deliberately entered. A copy is a user asking for a self-contained piece | keeping it dangling, or re-binding it to the source element on the same board |
| The identities are minted before anything is written | membership and arrow ends have to be remapped, and remapping after the write means a second pass over what was just written | writing then patching |
| A copy is answered by the Logic thread | the selection can reach past the screen, and the UI thread holds the screen. Asking is one round trip; guessing is a silent truncation | building the payload from what is displayed |
| Cut is a copy and a delete, not a command | the two halves already exist and the order is guaranteed by the channel, so a third command would only duplicate them | a cut command |
| A duplicate is fused into one undo item | the user made one gesture. Two items would make the first `Ctrl+Z` leave the copy behind | a copy item and a paste item |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing asserts what a copied group is bound to** (`TOILE-071`). The capture keeps only children
  inside the set, so a group copied without its parent should arrive unparented and bound to its own
  copies. The test that comes closest counts undo items.
* **A paste reseeds every derived cache.** The materialization rebuilds the bounds map, the index and
  the paint order from the whole document rather than inserting the pasted nodes, so pasting five
  elements onto a large board costs a full rebuild (`ambre-roadmap.md`, "Engine and render
  optimization").
* **A paste from another application is ignored.** Only the private payload is decoded, so pasted text
  drops on the board rather than becoming a note, and a pasted image drops rather than being imported
  (`features/ambre-spec-toile-import-export.md`).
* **The plain-text mirror is empty.** A copy out of Ambre into another application therefore yields
  nothing, by decision, and no rule says what the text form of a drawing should be.
* **An image is copied by reference.** A copied picture carries its content hash, so the two copies
  share one stored blob; what a paste onto a board that cannot reach that blob shows is
  `features/ambre-spec-toile-media.md`.
