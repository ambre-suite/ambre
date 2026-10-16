# AMBRE -- FEATURE: FIND, AND REPLACE

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- the three applications search the whole document, and Trame and
Prose replace in it. The one live session the `One find and replace` Story (`ambre-roadmap.md` E8)
wants is unbuilt everywhere.

> A user types a word and the application takes them to it, wherever it is -- including far past the
> edge of the screen. Searching changes nothing and is that reader's own; replacing is an edit like
> any other.

A search runs where the document lives, not where it is painted. That is the only way a match
beyond the viewport can be found at all. This document owns the model; each application owns what its text
is, the order the hits come back in, and whether it replaces.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this feature realizes
is normative in `ambre-spec-message-bus.md`. The prefix is `FIND`.

### 1.1 Search, and step through the matches

One gesture with two directions. The walk is the same whichever way the cursor moves, so forward
and backward share a table rather than repeating one.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-002` | `POST` | A search MUST cover the whole scope, never only what the viewport holds. | this spec section 3 | Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps`; Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default`; Prose `ambre_prose::logic::find::tests::every_match_is_reported_in_document_order` -- Prose's Logic thread holds no viewport, so the walk has no window to stop at; Trame `ambre_trame::logic::tests::group6::a_search_reads_past_the_requested_window` -- a match far outside the scrolled window is found; Toile `ambre_toile::logic::convergence_tests::a_search_finds_a_hit_far_outside_the_camera` -- a hit the camera's frame does not hold is found and focused | none | verified |
| `FIND-003` | `POST` | Stepping MUST elect the hit adjacent to the resume point in the direction asked, wrapping at either end. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_focus_wraps_after_from` -- it steps off the first hit and reads the second back, then steps off the last and reads the first, so an engine that re-elected the current one fails it; `::find_backward_steps_to_previous_hit_wrapping` -- the other direction; Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` -- the forward case; Prose `ambre_prose::logic::find::tests::opening_walking_and_closing_write_nothing` -- a step forward, a step back and a wrap past the first | none | verified |
| `FIND-015` | `POST` | In Prose and Toile, a step with no usable resume point -- none given, or one that is no longer a match -- MUST elect the first hit. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default` -- a search with no resume point focuses the first hit; `::a_resume_point_edited_away_restarts_at_the_first_hit` -- a resume point whose text no longer matches restarts at the first hit, in both directions; Prose `ambre_prose::logic::find::tests::a_resume_point_edited_away_restarts_at_the_first_hit` -- a fresh search starts at the first hit, and a current match an edit removed restarts there | none | verified |
| `FIND-010` | `POST` | In Trame, a step with no usable resume point -- none given, or one that is no longer a match -- MUST elect the first hit. | this spec section 3 | not built | unbuilt -- lands with the `One find and replace` Story (`ambre-roadmap.md` E8). Trame resumes from the active cell, a position and not a hit: it is always given and no edit makes it unusable, so a fresh search starts after the cursor | unimplemented |
| `FIND-004` | `POST` | The current match MUST be brought into view. | this spec section 4; `3-Toile/features/ambre-spec-toile-viewport.md`, the camera reach a recentre uses | Trame `ambre_trame::ui::view::view_tests::the_current_match_is_brought_into_view` -- an answer naming a cell off screen scrolls the grid to it; Toile `ambre_toile::ui::view::view_tests::the_current_match_is_brought_into_view` -- the focus's bounds recentre the camera on a hit far outside it; Prose `ambre_prose::view::tests::each_step_of_a_search_brings_its_match_into_view` -- each step, the wrap included, scrolls the page to its match | none | verified |
| `FIND-005` | `FAIL` | An empty query MUST clear the highlight. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_ignores_non_text_elements_and_empty_query` -- the empty query answers with nothing; Trame `ambre_trame::ui::view::view_tests::an_empty_answer_clears_the_highlight` -- an answer with no match clears the highlight and leaves the cursor; Prose `ambre_prose::logic::find::tests::an_empty_query_clears_the_highlight` -- the empty query answers with no match and no current one | none | verified |
| `FIND-001` | `FRAME` | A search MUST NOT change the document. | this spec section 2 | Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default`; `::find_focus_wraps_after_from`; `::find_backward_steps_to_previous_hit_wrapping`; `::find_honors_match_case_and_whole_word`; `::find_ignores_non_text_elements_and_empty_query` -- each asserting no delta; Prose `ambre_prose::logic::find::tests::opening_walking_and_closing_write_nothing` -- the version vector reads back unchanged; Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` -- asserting no delta, and type-level: `find_next` takes `&self`, so it reaches no handle that writes the document's state | none | verified |

### 1.2 Replace the active match

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-011` | `POST` | In Prose and Trame, a replace MUST rewrite the active match and then advance to the next. | this spec section 4 | Trame `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances` -- over a matching active cell and a non-matching one; Prose `ambre_prose::logic::find::tests::replace_one_replaces_exactly_the_current_range` -- the second match rewritten and the next one current | none | verified |
| `FIND-007` | `POST` | In Toile, a replace MUST rewrite the active match and then advance to the next. | this spec section 4 | not built | unbuilt -- lands with the `One find and replace` Story (`ambre-roadmap.md` E8). Toile has no replace at all | unimplemented |
| `FIND-012` | `FRAME` | In Prose and Trame, a replace MUST NOT change any text outside the active match. | this spec section 4 | Trame `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances` -- on the assertion that reads the next match back as its old text after the rewrite; Prose `ambre_prose::logic::find::tests::replace_one_replaces_exactly_the_current_range` -- the two other matches read back unchanged | none | verified |
| `FIND-008` | `FRAME` | In Toile, a replace MUST NOT change any text outside the active match. | this spec section 4 | not built | unbuilt -- lands with the `One find and replace` Story (`ambre-roadmap.md` E8). Toile has no replace at all | unimplemented |

### 1.3 Replace every match in scope

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `FIND-013` | `POST` | In Prose and Trame, a replace-all MUST be a single undo item, whatever the match count. | this spec section 3; `ambre-spec-undo.md` | Trame `ambre_trame::logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group`; `::replace_all_spans_sheets` -- the cross-sheet path, which writes through a different door; Prose `ambre_prose::logic::find::tests::a_replace_all_is_one_undo_item` | none | verified |
| `FIND-006` | `POST` | In Toile, a replace-all MUST be a single undo item, whatever the match count. | this spec section 3; `ambre-spec-undo.md` | not built | unbuilt -- lands with the `One find and replace` Story (`ambre-roadmap.md` E8). Toile has no replace at all | unimplemented |
| `FIND-014` | `FRAME` | In Prose and Trame, a replace-all MUST NOT change any text that no reported match covers. | this spec section 3; `2-Trame/features/ambre-spec-trame-find-replace.md` (`TRAME-UI-138`), the sheet-scope half | Trame `ambre_trame::logic::tests::group6::a_replace_all_leaves_every_unmatched_text_alone` -- the unmatched cells, an empty cell and the same word on a sheet outside the scope read back unchanged; Prose `ambre_prose::logic::find::tests::replace_all_replaces_every_match_and_nothing_else` | none | verified |
| `FIND-009` | `FRAME` | In Toile, a replace-all MUST NOT change any text that no reported match covers. | this spec section 3 | not built | unbuilt -- lands with the `One find and replace` Story (`ambre-roadmap.md` E8). Toile has no replace at all | unimplemented |

* **Budget.** none of its own: a search is one linear walk over the scope, and a replace lands
  under the batch row of `ambre-spec-budgets.md section 1`. The walk runs on every keystroke and no
  benchmark measures it.
* **Boundary.** none of its own: a search reads the document in the Logic thread and the query
  never leaves the client. A replace is a document write, and `ZK-001` seals every payload before
  it leaves.
* **Verification.** 10 `verified`, 5 `unimplemented`. The reach, the step, the reveal, the empty
  query and the read-only search hold in all three applications. The replace rows hold in Trame and
  Prose, and Toile has no replace to hold them. What is unbuilt besides is Trame's resume point, a
  cell position that no fresh search sets aside.

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

**Containers.** None of its own. A search writes nothing. A replace writes the text containers the
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
2. walk the whole scope in the application's stable order, and keep each text-bearing item that
   matches. Both sides fold to lower case unless the case option is set (`FIND-002`);
3. resolve the reveal target: the hit after the resume point, or the one before it when stepping
   backward, wrapping either way (`FIND-003`). With no usable resume point, the target is the first
   hit (`FIND-010`);
4. answer with the hits and the target, having written nothing (`FIND-001`).

**Steps, on a replace** (text applications only). `ReplaceOne` rewrites the active match's raw text
and advances to the next (`FIND-007`). `ReplaceAll` rewrites every hit in scope inside one undo
group and answers with the count (`FIND-006`). What one undo item is, is `ambre-spec-undo.md`.

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
the canvas. It is a text field with a few toggles, which the shared toolkit already builds. It
holds only the query and its options. The hits and the current match come from the answer, so
nothing on the UI thread can disagree with the document. Which chord opens find is `ambre-spec-commands.md`.

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

* **The one live shared session is unbuilt.** Each application runs its own Logic-thread walk. The
  `One find and replace` Story (`ambre-roadmap.md` E8) replaces the three with one match session and
  one live cursor. This document states the model they already share; which of each application's
  own rules survive that move is open until it lands.
* **A stale highlight.** The hit list is a snapshot. A local edit, or a remote one, can empty the
  target the highlight still points at, and no application invalidates it on a concurrent edit. It is
  the two-peer table's first row, and no rule covers it in any application.
* **The cost of a search is unruled.** The walk is linear in the item count and runs on every
  keystroke, undebounced in the engine. A five-letter word on a large document is therefore five
  full walks. No application caps the match count either.
* **The case fold and the whole-token match are undefined.** An ASCII fold, a Unicode one and a
  locale-aware one disagree on ordinary text. Which locale a fold would read is
  `ambre-spec-locale.md`'s, and which collation a tag selects is open there too. What "whole"
  bounds -- a cell, a word -- is each application's, and no rule pins either across the
  applications.

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
