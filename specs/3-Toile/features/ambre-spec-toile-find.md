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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-111` | `OBS` | Every match the camera already shows MUST be outlined, one outline per match. | this spec section 4 | `ambre_toile::ui::render::tests::overlay_outlines_visible_find_hits` -- it paints two hits and counts two stroked boxes on an empty selection. The document cited nothing for the highlight at all | none | verified |
| `TOILE-UI-112` | `OBS` | The current match MUST be outlined differently from the rest, so a user stepping a long list can see where they are. | this spec section 4 | none | unheld -- the painter gives the focus the selection accent at a heavier width and the others a softer tint, and the test above counts the boxes without reading a colour or a width, so a painter that drew all hits alike would pass; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-113` | `OBS` | The panel MUST name the current match by its rank among the hits, not by its identity. | this spec section 4 | `ambre_toile::ui::find::tests::active_index_locates_focus_or_none` -- the second of three hits reads as rank two, which is the rank against identity the rule is about. The `k/n` caption itself is composed in the chrome panel, which is Leptos under the wasm target and out of the host tier | none | verified |
| `TOILE-UI-114` | `OBS` | A current match the hit list no longer holds MUST read as no rank, never as the first match. | this spec section 4 | `ambre_toile::ui::find::tests::active_index_locates_focus_or_none` -- an id absent from the list and an empty list, two assertions apart from the one above. The caption falls back to a zero, which the panel renders as `0/n` | none | verified |

### 1.2 Type a query

The shared walk is `ambre-spec-find.md section 1.1`: that it covers the whole board, that it writes
nothing, that it wraps and that an empty query clears. This table owns what a Toile match is.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-079` | `POST` | Hits MUST be returned in paint order, so stepping follows the stacking rather than the storage. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` -- its one assertion is an ordered vector of two ids, so it falsifies the order as well as the folding below, which is why both rows cite it | none | verified |
| `TOILE-080` | `POST` | An element the paint order omits MUST still be found, after the ordered hits and in an order every peer computes alike. | this spec section 3 | none | unheld -- the fallback appends the unlisted elements the bounds map holds, sorted by identity, and no test drives an element missing from the order. The paint walk carries no such fallback (`TOILE-058`); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-142` | `POST` | An element carrying no searchable body MUST NOT match, whatever its geometry or style holds. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_ignores_non_text_elements_and_empty_query` -- a rectangle answers no hits for its own kind name | none | verified |
| `TOILE-082` | `POST` | A frame MUST be findable by its title. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_named_frame_is_found_by_its_title` -- an unnamed frame matching nothing, then the same frame found by the title it was given (`TOILE-247`) | none | verified |
| `TOILE-143` | `POST` | A query MUST match without regard to case unless the user asks otherwise. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` -- its hit vector names an element whose body differs from the query in case alone | none | verified |
| `TOILE-144` | `POST` | With the case option set, a body MUST match only where the case agrees. | this spec section 3 | none | unheld -- `ambre_toile::logic::convergence_tests::find_honors_match_case_and_whole_word` counts two hits out of three bodies rather than naming them, and the wrong two (the folded body and the exact one, dropping the longer word) also count two; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-145` | `POST` | With the whole-word option set, a match MUST be bounded by a non-alphanumeric character on each side. | this spec section 3 | `ambre_toile::logic::convergence_tests::find_honors_match_case_and_whole_word` -- its second half names the one surviving element rather than counting, so the body the query only prefixes is dropped | none | verified |
| `TOILE-UI-118` | `FRAME` | A search MUST NOT change the selection: a match is outlined, never selected. | this spec section 4 | none | unheld -- the branch that takes the answer writes the hit set, the current match and the pending recentre, and reaches no selection state, but that is a property of one branch as it stands today, not of a type, so a line added there would breach the rule and fail nothing; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Step to the next match

Where the step lands is `ambre-spec-find.md`. What the camera does about it is Toile's.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-115` | `POST` | Stepping MUST bring the camera onto the match it lands on. | this spec section 4 | none | unheld -- the answer carries ids and no geometry, so the view asks for the focus box with the aggregates command and moves the camera when the reply lands, and no test drives either leg; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-117` | `POST` | The camera MUST recentre on the answer to its own request. | this spec section 4 | `ambre_toile::logic::convergence_tests::selection_stats_echoes_the_request_kind` -- the find focus tags its request, and the view recentres only on the reply that echoes that tag | none | verified |
| `TOILE-UI-116` | `FRAME` | Recentring MUST NOT change the zoom: a match found far out MUST NOT change how much of the board is visible. | this spec section 4 | none | unheld -- the find path writes the camera centre alone where a fit writes the centre and the scale, and nothing reads the zoom back across a step; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** none of its own: `ambre-spec-budgets.md` budgets no search on a board. The walk is
  linear in the element count and runs on every keystroke, and nobody has measured it on a board of
  fifty thousand elements.
* **Boundary.** none: the query never leaves the client and the results never enter the document. The
  panel is chrome on the UI thread, where `UI-THREAD-004` bans a network, crypto or persistence path.
* **Verification.** 9 `verified`, 6 `untested`. The shared rules are `ambre-spec-find.md`; what is
  counted here is what a Toile match is. What holds is the walk over element bodies and frame
  titles, on eight tests, and the token naming the recentre request (`TOILE-UI-117`). What is loose
  is its two edges and the whole camera half, where nothing drives the recentre and no test reads
  the zoom back.

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

* a note's body, a text element's body, and a frame's title, which the deck panel writes
  (`TOILE-082`, `features/ambre-spec-toile-frames.md`). Every other kind matches nothing, a shape
  and a picture included (`TOILE-142`);
* the hits come back in paint order, so stepping follows the stacking rather than the storage
  (`TOILE-079`);
* the text-bearing elements the paint order omits are appended, sorted by identity. An unlisted
  element is still found, and every peer computes the same order (`TOILE-080`);
* both sides are folded to lower case (`TOILE-143`) unless the case option is set (`TOILE-144`),
  then compared as substrings. The whole-word option requires a non-alphanumeric character on each
  side, and keeps looking past a badly bounded hit (`TOILE-145`).

**Steps, when the answer reaches the view.** The hits become the highlight set and the target
becomes the current match (`TOILE-UI-111`, `TOILE-UI-112`). The view asks the engine for that
element's box, since the answer carries ids and no geometry. The reply moves the camera onto it
(`TOILE-UI-115`, `TOILE-UI-116`). Nothing on that path touches the selection (`TOILE-UI-118`).

**Code:**

* `crates/ambre-toile/src/logic/state/find.rs` -- the walk, the fallback, the stepping and the
  comparison;
* `ui/find.rs` -- which hit the panel calls current (`TOILE-UI-113`, `TOILE-UI-114`);
* `ui/view.rs` -- the highlight set and the recentre;
* `ui/render.rs` -- the outline per hit;
* `chrome/find.rs` -- the panel itself.

---

## 4. SURFACE

The shared chords, the empty-query clear and the reveal are `ambre-spec-find.md section 4`. What a
match looks like is Toile's own:

* **An outline around each visible hit**, drawn in the overlay pass over the board
  (`TOILE-UI-111`, `features/ambre-spec-toile-board-rendering.md`). The current one is drawn in the
  selection accent and the rest in a softer tint (`TOILE-UI-112`). A hit outside the camera has no outline
  until the camera reaches it, which the recentre does for the current one (`TOILE-UI-115`,
  `features/ambre-spec-toile-viewport.md`).
* **The panel holds no state of its own** beyond the query and the two toggles. The hits and the
  current match come from the answer, so nothing on the UI thread can disagree with the board. It
  reads out where the current match sits in the list (`TOILE-UI-113`). A match the list has dropped
  reads as no rank rather than as the first (`TOILE-UI-114`).

---

## 5. REJECTED ALTERNATIVES

The shared decisions -- the scan in the Logic thread, the panel with no cursor, the empty-query
clear -- are `ambre-spec-find.md section 5`. Toile's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Hits come back in paint order | the storage order | it is a hash order and it changes. The order has to be stable across two searches and meaningful to a user stepping through it; paint order is both, and it is already materialized |
| Replace is out | find and replace, as Trame has it (`2-Trame/features/ambre-spec-trame-find-replace.md`) | replacing across free-form elements means rewriting bodies the user cannot all see, on a surface where a note is a drawing as much as a text. The cost is high and the gesture is rare |
| The panel is chrome, not painted on the canvas | drawing the panel in the overlay pass | a canvas-drawn field would need its own text input; the panel is a text field with two toggles, which the shared chrome toolkit already builds |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- the live shared session, the undebounced walk -- are in
`ambre-spec-find.md section 6`. Toile's own:

* **The fallback has no test** (`TOILE-080`). An element missing from the paint order is found here
  and is invisible on the board (`TOILE-058`). The two halves of the same design disagree, and only
  one of them is exercised.
* **A match inside a group is not revealed.** The recentre moves the camera and enters no group scope,
  so a hit inside a group is outlined and not selectable
  (`features/ambre-spec-toile-selection.md`).
* **A fresh search backward lands on the first hit** (`ambre-spec-find.md`, `FIND-010`). With no
  resume point the direction is not read. `Shift+Enter` before a first answer walks forward from the
  top rather than to the last match. That last match is where a user asking for the previous one
  expects to be.
* **The current match's outline is not held** (`TOILE-UI-112`). The overlay test counts the boxes and
  reads neither colour nor width. The one thing that tells a stepping user where they are is the one
  thing nothing checks.
* **A step never changes the zoom** (`TOILE-UI-116`). The camera is recentred and not fitted. A
  match found while zoomed far in arrives at the same magnification, filling the screen with one
  note. Whether a find owes a fit is unruled.
