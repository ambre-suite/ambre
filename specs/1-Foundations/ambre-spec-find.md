# AMBRE -- FEATURE: FIND, AND REPLACE

**Type:** Horizontal (framework)
**Implementation:** partial -- Trame and Toile search the whole document and Trame replaces in it;
Prose is design-only, and the one live session the
"One find and replace" Story (`ambre-roadmap.md` E8) wants is unbuilt everywhere.

> A user types a word and the application takes them to it, wherever it is -- including far past the
> edge of the screen. Searching changes nothing and is that reader's own; replacing is an edit like
> any other.

A search runs where the document lives, not where it is painted: that is the only way a match beyond
the viewport can be found at all. This document owns the model; each application owns what its text
is, the order the hits come back in, and whether it replaces.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this feature realizes
is normative in `ambre-spec-message-bus.md`. The prefix is `FIND`.

### 1.1 Search, and step through the matches

One gesture with two directions: the walk is the same whichever way the cursor moves, so forward and
backward share a table rather than repeating one.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-002` | `POST` | A search MUST cover the whole scope, never only what the viewport holds. | this spec section 3 | Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps`; Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` | partial -- the walk is held, the bound is not. Both drive a walk that reads the whole map, and neither builds a camera or places a match outside one, so a viewport bound added to either walk would fail nothing | unverified |
| `FIND-003` | `POST` | Stepping MUST elect the hit adjacent to the resume point in the direction asked, wrapping at either end. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_focus_wraps_after_from` -- it steps off the first hit and reads the second back, then steps off the last and reads the first, so an engine that re-elected the current one fails it; `::find_backward_steps_to_previous_hit_wrapping` -- the other direction; Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` -- the forward case | none | verified |
| `FIND-010` | `POST` | A step with no usable resume point -- none given, or one that is no longer a match -- MUST elect the first hit. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` -- a search with no resume point focuses the first hit | partial -- the half that is held is the fresh search. Nothing drives the other half, a resume point edited away between two keystrokes, in any application | unverified |
| `FIND-004` | `POST` | The current match MUST be brought into view. | this spec section 4; `3-Toile/features/ambre-spec-toile-viewport.md`, the camera reach a recentre uses | none | unheld -- Trame elects the match and moves the active cell to it (`ambre_trame::logic::tests::group6::find_next_matches_and_wraps`), and Toile resolves the recentre target, but the scroll itself is per-application UI and no test drives it end to end | unverified |
| `FIND-005` | `FAIL` | An empty query MUST clear the highlight. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_ignores_non_text_elements_and_empty_query` -- the empty query answers with nothing | partial -- Trame clears the highlight on close and no test reads it back | unverified |
| `FIND-001` | `FRAME` | A search MUST NOT change the document. | this spec section 2 | Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default`; `::find_focus_wraps_after_from`; `::find_backward_steps_to_previous_hit_wrapping`; `::find_honors_match_case_and_whole_word`; `::find_ignores_non_text_elements_and_empty_query` -- each asserting no delta | partial -- Trame's `find_next` answers a display-only outcome but takes `&mut self`, so only review holds it there | unverified |

### 1.2 Replace the active match

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-007` | `POST` | A replace MUST rewrite the active match and then advance to the next. | this spec section 4 | Trame `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances` -- over a matching active cell and a non-matching one | none | verified |
| `FIND-008` | `FRAME` | A replace MUST NOT change any text outside the active match. | this spec section 4 | Trame `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances` -- on the assertion that reads the next match back as its old text after the rewrite | none | verified |

### 1.3 Replace every match in scope

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-006` | `POST` | A replace-all MUST be a single undo item, whatever the match count. | this spec section 3; `ambre-spec-undo.md` | Trame `ambre_trame::logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group`; `::replace_all_spans_sheets` -- the cross-sheet path, which writes through a different door | none | verified |
| `FIND-009` | `FRAME` | A replace-all MUST NOT change any text that no reported match covers. | this spec section 3; `2-Trame/features/ambre-spec-trame-find-replace.md` (`TRAME-UI-138`), the sheet-scope half | none | unheld -- Trame's `::replace_all_spans_sheets` drives the widest scope, where nothing is outside it, and no test rewrites a document holding text the query never matched. This is the row that keeps a replace-all from reaching past its own hit list | unverified |

* **Budget.** none of its own: a search is one linear walk over the scope, and
  `ambre-spec-budgets.md` budgets no search; a replace lands under the batch row of
  `ambre-spec-budgets.md section 1`. The walk runs on every keystroke and no benchmark measures it.
* **Boundary.** none of its own: a search reads the document in the Logic thread and the query never
  leaves the client; a replace is a document write, and `ZK-001` seals every payload before it
  leaves the client.
* **Verification.** 4 `verified`, 6 `unverified`, 0 `unimplemented`. What the feature is for holds:
  it wraps, it elects the adjacent hit, and a replace stays inside its match. A replace-all is one
  undo item. Six rows stay loose, and four of them are loose in one half only. `FIND-002` is the
  reach. Both engines read the whole map, and no test places a match where a viewport bound would
  hide it, so the rule rests on a walk that does not ask. `FIND-004` is
  the reveal, each application's own scroll, which no test drives end to end. `FIND-010` holds the
  fresh search, never the resume point an edit takes away between two keystrokes. `FIND-009` is the
  frame condition of a replace-all, and the only replace-all any test runs covers everything.
  `FIND-001` and `FIND-005` hold in Toile and rest on review in Trame. Prose is design-only and adds
  no code to any row.

---

## 2. WHAT EACH USER SEES

A search touches no shared state, so a searcher's highlights reach no collaborator (`FIND-001`). Only
a replace writes, so this table is the replace's turn; each application adds its own scenarios in its
own document.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| searches for a word | edits any cell or element | unchanged by the search | Bob sees nothing of her search; her highlight may point at what no longer matches (`FIND-001`) | Toile `logic::convergence_tests::find_ignores_non_text_elements_and_empty_query` |
| replaces every match | types the old word into a place Alice already passed | the replacements, and Bob's new word | Alice's count says the scope is clean, and one old word is on screen | none -- the convergence property carries no replace, section 6 |
| presses undo after that replace-all | watches | the replacements taken back | the original words, in one step (`FIND-006`) | Trame `logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group` |

---

## 3. MODEL

**Containers.** None of its own. A search writes nothing; a replace writes the text containers the
other features own, which is why one replace-all is one undo item rather than one per cell.

**Commands and events.** Each application names its own set (`ambre-bus`); the shape is one.

| Shape | Direction | Carries |
|---|---|---|
| `<App>Command::Find { query, options, from, backward }` | UI to Logic | the query, its options, the match to resume from and the direction |
| `<App>Event::FindResult { hits, current }` | Logic to UI | every hit in the application's stable order, and the one to reveal |
| `<App>Command::Replace{One,All} { query, replacement, options }` | UI to Logic | the query, its replacement and the options; text applications only |
| `<App>Event::ReplaceDone { replaced }` | Logic to UI | how many matches were rewritten |

**Panel state**, held in the chrome, never in the document:

```rust
/// The query and its refinements. What "whole" means -- a whole cell, a whole
/// word -- is each application's, and the option set is theirs to widen.
pub struct FindQuery { pub query: String, pub match_case: bool }
```

**Steps, when a query arrives.** The same walk everywhere; step 2 is the application's own order.

1. answer with nothing when the query is empty, which is how the panel clears its highlight
   (`FIND-005`);
2. walk the whole scope in the application's stable order and keep each text-bearing item that
   matches, folding both sides to lower case unless the case option is set (`FIND-002`);
3. resolve the reveal target: the hit after the resume point, or the one before it when stepping
   backward, wrapping either way (`FIND-003`), and the first hit when there is no usable resume
   point (`FIND-010`);
4. answer with the hits and the target, having written nothing (`FIND-001`).

**Steps, on a replace** (text applications only). `ReplaceOne` rewrites the active match's raw text
and advances to the next (`FIND-007`); `ReplaceAll` rewrites every hit in scope inside one undo group
and answers with the count (`FIND-006`). What one undo item is, is `ambre-spec-undo.md`.

**Code.** none of its own -- each application's engine holds the walk, the match and the rewrite,
cited from its own find document.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+F` | opens the panel in find mode: the query field alone |
| `Ctrl+H` | opens it in replace mode, where the application replaces: query, replacement, options |
| `Enter` | resumes the search from the current match, forward, and reveals the elected one (`FIND-004`) |
| `Shift+Enter` | the same, backward (`FIND-003`) |
| Replace, Replace all | rewrites the active match then advances, or every match in scope (`FIND-006`, `FIND-007`) |
| `Escape`, or the close button | dismisses the panel and clears the highlight |

**The panel is chrome** (`ambre-spec-chrome.md`), pinned to a corner of the surface, never painted on
the canvas: a text field with a few toggles, which the shared toolkit already builds. It holds only
the query and its options; the hits and the current match come from the answer, so nothing on the UI
thread can disagree with the document. Which chord opens find is `ambre-spec-commands.md`.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The scan runs in the Logic thread | scanning the UI cache | the cache holds one viewport, so a search there would find only what is already on screen, and a document is mostly past the edge of the screen. It bounds the search by the scroll position and says nothing |
| The panel holds no cursor and no hit list | the panel walking its own hit list against its own index | the Logic thread already tracks the origin and computes the hits, and a second copy of the index is the one that drifts under a merge or a wrap |
| An empty query is the clear signal | a separate clear command | the panel needs a way to say "no highlight" and the engine needs no second command for it |
| A search writes nothing | recording the session in the document | a search is one reader's attention. Sharing it gives every peer one reader's highlights, and puts a search in the undo history of everyone else |
| A replace-all is one undo item | one undo item per rewritten match | a replace over a thousand matches is one decision, and undoing it one match at a time is not an undo |
| Replace is the text applications' | one replace model for every application | replacing across a board's free-form elements rewrites bodies the user cannot all see, on a surface where a note is a drawing as much as a text. The cost is high and the gesture is rare, which is why Toile declines it (`3-Toile/features/ambre-spec-toile-find.md`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The one live shared session is unbuilt.** Each application runs its own Logic-thread walk, and
  the "One find and replace" Story (`ambre-roadmap.md` E8) replaces the three with one match session
  and one live cursor. This document states the model they already share; which of each application's
  own rules survive that move is open until it lands.
* **A stale highlight.** The hit list is a snapshot. A local edit, or a remote one, can empty the
  target the highlight still points at, and no application invalidates it on a concurrent edit. It is
  the two-peer table's first row, and no rule covers it in any application.
* **The cost of a search is unruled.** The walk is linear in the item count and runs on every
  keystroke, undebounced in the engine, so a five-letter word on a large document is five full walks;
  no application caps the match count either.
* **What a whole-token match is, and what the case fold is, are undefined.** An ASCII fold, a Unicode
  one and a locale-aware one disagree on ordinary text, and what "whole" bounds -- a cell, a word --
  is each application's; no rule pins either across the applications.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What text is searchable, per application | `2-Trame/features/ambre-spec-trame-find-replace.md`, `TRAME-UI-078`, `TRAME-UI-079`; `3-Toile/features/ambre-spec-toile-find.md`, `TOILE-082`; `4-Prose/features/ambre-spec-prose-find-replace.md`, `PROSE-FEAT-033` |
| The order the hits come back in | each application's find document -- row-major, paint order, document order |
| Whether the application replaces, and the scope | `2-Trame/features/ambre-spec-trame-find-replace.md`, `TRAME-UI-083`; `4-Prose/features/ambre-spec-prose-find-replace.md`, `PROSE-FEAT-036`; Toile declines it |
| The whole-match unit and the extra options | `2-Trame/features/ambre-spec-trame-find-replace.md`, `TRAME-UI-077` |
| How a match is revealed | each application's viewport or selection document |
| Which chord opens find and replace | `ambre-spec-commands.md` |
