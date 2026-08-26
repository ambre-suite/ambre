# AMBRE -- FEATURE: FIND ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** partial -- searching notes ships; a frame has no title to find.

> A user types a word and the board takes them to it, wherever it is -- including far off screen.
> Every match is outlined; `Enter` steps to the next one, `Shift+Enter` to the previous.

The search runs where the board lives, not where it is painted. That is the only way a match beyond
the edge of the screen can be found at all.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-019` | A search MUST be pure local view state: it MUST NOT change the document. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default`, `::find_focus_wraps_after_from`, `::find_backward_steps_to_previous_hit_wrapping`, `::find_honors_match_case_and_whole_word`, `::find_ignores_non_text_elements_and_empty_query` -- each asserts no delta was produced | verified |
| `TOILE-078` | A search MUST cover the whole board, not the part on screen. | this spec section 3 | structural, and the construction is the proof: the walk reads the element maps in the Logic thread, which holds no camera, so the visible set is out of its reach. `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` matches an element outside the default camera | verified |
| `TOILE-079` | Hits MUST be returned in paint order, so stepping follows the stacking rather than the storage. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` | verified |
| `TOILE-080` | An element the paint order omits MUST still be found, after the ordered hits and in an order every peer computes alike. | this spec section 3 | none. The fallback appends the unlisted text-bearing elements sorted by identity, and no test drives an element missing from the order. The paint walk carries no such fallback (`TOILE-058`) | unverified |
| `TOILE-081` | Stepping MUST wrap in both directions, and a match that is no longer one MUST restart the walk. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_focus_wraps_after_from`, `::find_backward_steps_to_previous_hit_wrapping` | verified |
| `TOILE-082` | A frame MUST be findable by its title. | this spec section 3 | none, and **the rule fails today**: the walk reads a frame's title field and no command writes one, so the branch can never match -- section 6 | unverified |

* **Budget.** none of its own: `ambre-spec-budgets.md` budgets no search on a board.
  The walk is linear in the element count and runs on every keystroke of the query, which is the
  figure a board of fifty thousand elements would need and nobody has measured.
* **Boundary.** none: the query never leaves the client and the results never enter the document. The
  panel is chrome on the UI thread, where `UI-THREAD-004` bans a network, crypto or persistence path.
* **Verification.** 4 `verified`, 2 `unverified`. The rules that make the feature worth having -- it
  writes nothing, it reaches past the screen, it wraps -- are each held. The gaps are the two edges:
  an unlisted element, and a frame title that cannot exist.

---

## 2. WHAT EACH USER SEES

A search touches no shared state, so this section carries no table
(`ambre-spec-template.md`). `TOILE-019` is the rule that says so.

---

## 3. MODEL

**Containers.** None. A search reads the `elements` map and writes nothing.

**Command and answer** (`ambre-bus`):

```rust
/// Refinements, both local to the panel.
pub struct FindOptions { pub match_case: bool, pub whole_word: bool }

/// `hits` in paint order; `focus` is the element to recentre on.
pub struct FindResults { pub hits: Vec<ElementId>, pub focus: Option<ElementId> }
```

`Find { query, options, from, backward }` carries the match to resume from and the direction. An
absent resume point starts a fresh search on the first hit.

**Steps, when a query arrives:**

1. answer with nothing when the query is empty, which is how the panel clears its highlight;
2. walk the paint order and keep each text-bearing element whose body matches (`TOILE-079`);
3. append the text-bearing elements the paint order omits, sorted by identity
   (`TOILE-080`);
4. resolve the recentre target: the hit after the resume point, or the one before it when stepping
   backward, wrapping either way, and the first hit when the resume point is no longer a hit
   (`TOILE-081`);
5. answer with the hits and the target, having written nothing (`TOILE-019`).

**What counts as searchable text.** A note's body and a text element's body, and a frame's title -- a
field nothing writes, which is why `TOILE-082` fails. Every other kind matches nothing, a shape and a
picture included.

**How a match is compared.** Both sides are folded to lower case unless the case option is set, then
compared as substrings. The whole-word option requires a non-alphanumeric character on each side, and
keeps looking past a badly bounded hit.

**Code:**

* `crates/ambre-toile/src/logic/state/find.rs` -- the walk, the fallback, the stepping and the
  comparison;
* `ui/find.rs` -- which hit the panel calls current;
* `chrome/find.rs` -- the panel itself.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| `Ctrl+F` | opens the panel, a floating chrome surface pinned to a board corner |
| typing in the field | sends a fresh search and replaces the highlight |
| `Enter` | sends a search resuming from the current match, forward |
| `Shift+Enter` | the same, backward |
| the two option toggles | send a fresh search with the new refinement |
| `Escape`, or the close button | dismisses the panel and clears the highlight |

**What a match looks like.** An outline around each visible hit, drawn in the overlay pass over the
board (`features/ambre-spec-toile-board-rendering.md`). A hit outside the camera has no outline until
the camera reaches it, which the recentre does for the current one
(`features/ambre-spec-toile-viewport.md`).

**The panel holds no state of its own** beyond the query and the two toggles. The hits and the current
match come from the answer, so nothing on the UI thread can disagree with the board.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The search runs in the Logic thread | the UI thread holds only what is painted, so a search there would silently miss every match past the edge of the screen -- and a whiteboard is mostly past the edge of the screen | searching the visible set, which is what the select-all defect does (`TOILE-UI-033`) |
| Hits come back in paint order | the order has to be stable across two searches and meaningful to a user stepping through it. Paint order is both, and it is already materialized | the storage order, which is a hash order and changes |
| The wrapping is decided by the engine, not the panel | the panel would need the hit list, the current index and the direction to agree with the engine's, and the third copy of an index is the one that drifts | the panel walking its own hit list |
| An empty query is the clear signal | the panel needs a way to say "no highlight" and the engine needs no second command for it | a separate clear command |
| Replace is out | replacing across free-form elements means rewriting bodies the user cannot all see, on a surface where a note is a drawing as much as a text. The cost is high and the gesture is rare | find and replace, as Trame has it (`features/ambre-spec-trame-find-replace.md`) |
| The panel is chrome, not painted on the canvas | it is a text field with two toggles, which the shared chrome toolkit already builds, and a canvas-drawn field would need its own text input | drawing the panel in the overlay pass |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A frame cannot be found by name** (`TOILE-082`). The walk reads a frame's title field, no command
  writes one, and no section of the corpus declared the field until the format table did
  (`ambre-spec-toile-schema.md`). The branch is unreachable, so a board organized into named frames
  is searchable only through the notes inside them.
* **The fallback has no test** (`TOILE-080`). An element missing from the paint order is found here and
  is invisible on the board (`TOILE-058`), so the two halves of the same design disagree and only one
  of them is exercised.
* **Every keystroke walks the whole board.** The query is not debounced in the engine and the walk is
  linear in the element count, so typing a five-letter word on a large board is five full walks.
* **A match inside a group is not revealed.** The recentre moves the camera and enters no group scope,
  so a hit inside a group is outlined and not selectable
  (`features/ambre-spec-toile-selection.md`).
* **Nothing counts the hits.** The panel has no "three of eleven" read-out, so a user stepping a long
  list cannot tell when they have come round.
