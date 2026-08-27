# AMBRE -- FEATURE: FIND ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- searching notes ships; a frame has no title to find.

> A user types a word and the board takes them to it, wherever it is -- including far off screen.
> Every match is outlined; `Enter` steps to the next one, `Shift+Enter` to the previous.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-find.md`,
`FIND-001` through `FIND-005`. Toile carries no replace (section 5); its own turn is what a board
match is, and in what order.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-079` | Hits MUST be returned in paint order, so stepping follows the stacking rather than the storage. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` | verified |
| `TOILE-080` | An element the paint order omits MUST still be found, after the ordered hits and in an order every peer computes alike. | this spec section 3 | none. The fallback appends the unlisted text-bearing elements sorted by identity, and no test drives an element missing from the order. The paint walk carries no such fallback (`TOILE-058`) | unverified |
| `TOILE-082` | A frame MUST be findable by its title. | this spec section 3 | none, and **the rule fails today**: the walk reads a frame's title field and no command writes one, so the branch can never match -- section 6 | unverified |

* **Budget.** none of its own: `ambre-spec-budgets.md` budgets no search on a board. The walk is
  linear in the element count and runs on every keystroke, which is the figure a board of fifty
  thousand elements would need and nobody has measured.
* **Boundary.** none: the query never leaves the client and the results never enter the document. The
  panel is chrome on the UI thread, where `UI-THREAD-004` bans a network, crypto or persistence path.
* **Verification.** 1 `verified`, 2 `unverified`. The order is held; the gaps are the two edges -- an
  unlisted element, and a frame title that cannot exist. The shared rules and their coverage are in
  `ambre-spec-find.md`.

---

## 2. WHAT EACH USER SEES

A search touches no shared state, so this section carries no table
(`ambre-spec-template.md`). `ambre-spec-find.md`, `FIND-001`, is the rule that says so.

---

## 3. MODEL

* **Containers** -- none. A search reads the `elements` map and writes nothing.

**Command and answer** (`ambre-bus`, the shared shape is `ambre-spec-find.md section 3`):

```rust
/// Refinements, both local to the panel.
pub struct FindOptions { pub match_case: bool, pub whole_word: bool }

/// `hits` in paint order; `focus` is the element to recentre on.
pub struct FindResults { pub hits: Vec<ElementId>, pub focus: Option<ElementId> }
```

`Find { query, options, from, backward }` carries the match to resume from and the direction.

**What a board match is** (the shared walk is `ambre-spec-find.md section 3`):

* a note's body, a text element's body, and a frame's title -- a field nothing writes, which is why
  `TOILE-082` fails; every other kind matches nothing, a shape and a picture included;
* the hits come back in paint order, so stepping follows the stacking (`TOILE-079`);
* the text-bearing elements the paint order omits are appended, sorted by identity, so an unlisted
  element is still found (`TOILE-080`);
* both sides are folded to lower case unless the case option is set, then compared as substrings; the
  whole-word option requires a non-alphanumeric character on each side.

**Code:**

* `crates/ambre-toile/src/logic/state/find.rs` -- the walk, the fallback, the stepping and the
  comparison;
* `ui/find.rs` -- which hit the panel calls current;
* `chrome/find.rs` -- the panel itself.

---

## 4. SURFACE

The shared chords, the empty-query clear and the reveal are `ambre-spec-find.md section 4`. What a
match looks like is Toile's own:

* **An outline around each visible hit**, drawn in the overlay pass over the board
  (`features/ambre-spec-toile-board-rendering.md`). A hit outside the camera has no outline until the
  camera reaches it, which the recentre does for the current one
  (`features/ambre-spec-toile-viewport.md`).
* **The panel holds no state of its own** beyond the query and the two toggles. The hits and the
  current match come from the answer, so nothing on the UI thread can disagree with the board.

---

## 5. DECISIONS

The shared decisions -- the scan in the Logic thread, the panel with no cursor, the empty-query
clear -- are `ambre-spec-find.md section 5`. Toile's own:

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Hits come back in paint order | the order has to be stable across two searches and meaningful to a user stepping through it. Paint order is both, and it is already materialized | the storage order, which is a hash order and changes |
| Replace is out | replacing across free-form elements means rewriting bodies the user cannot all see, on a surface where a note is a drawing as much as a text. The cost is high and the gesture is rare | find and replace, as Trame has it (`2-Trame/features/ambre-spec-trame-find-replace.md`) |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- the live shared session, the undebounced walk -- are in
`ambre-spec-find.md section 6`. Toile's own:

* **A frame cannot be found by name** (`TOILE-082`). The walk reads a frame's title field, no command
  writes one, and no section of the corpus declared the field until the format table did
  (`ambre-spec-toile-schema.md`). The branch is unreachable, so a board organized into named frames
  is searchable only through the notes inside them.
* **The fallback has no test** (`TOILE-080`). An element missing from the paint order is found here
  and is invisible on the board (`TOILE-058`), so the two halves of the same design disagree and only
  one of them is exercised.
* **A match inside a group is not revealed.** The recentre moves the camera and enters no group scope,
  so a hit inside a group is outlined and not selectable
  (`features/ambre-spec-toile-selection.md`).
* **Nothing counts the hits.** The panel has no "three of eleven" read-out, so a user stepping a long
  list cannot tell when they have come round.
