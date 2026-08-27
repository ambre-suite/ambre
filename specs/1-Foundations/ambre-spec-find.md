# AMBRE -- FEATURE: FIND, AND REPLACE

**Type:** Horizontal (framework)
**Implementation:** partial -- Trame and Toile search the whole document and Trame replaces in it;
Prose is design-only, and the one live session the "One find and replace" Story wants is unbuilt
everywhere.

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `FIND-001` | A search MUST be pure local view state: it MUST NOT change the document. | this spec section 2 | Toile `ambre_toile::logic::convergence_tests::find_matches_in_z_order_and_is_case_insensitive_by_default`, `::find_focus_wraps_after_from`, `::find_backward_steps_to_previous_hit_wrapping`, `::find_honors_match_case_and_whole_word`, `::find_ignores_non_text_elements_and_empty_query`, each asserting no delta; Trame `find_next` returns a display-only outcome but takes `&mut self`, so only review holds it there; Prose design-only | verified |
| `FIND-002` | A search MUST cover the whole scope, never only what the viewport holds; the walk MUST run in the Logic thread. | this spec section 3 | Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps`; Toile structural -- the walk reads the element maps in the Logic thread, which holds no camera, and `::find_matches_in_z_order_and_is_case_insensitive_by_default` matches an element outside the default camera; Prose runs in the Logic thread by decision, design-only | verified |
| `FIND-003` | Stepping MUST wrap in both directions, and a resume point that is no longer a match MUST restart the walk. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_focus_wraps_after_from`, `::find_backward_steps_to_previous_hit_wrapping`; Trame `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` holds the forward wrap; Prose design-only | verified |
| `FIND-004` | The current match MUST be brought into view. | this spec section 4 | Trame elects the match and moves the active cell to it (`ambre_trame::logic::tests::group6::find_next_matches_and_wraps`); Toile resolves the recentre target and the camera reach is the viewport's (`3-Toile/features/ambre-spec-toile-viewport.md`); the scroll itself is per-application UI, asserted end to end by none; Prose design-only (`PROSE-FEAT-037` retired) | unverified |
| `FIND-005` | An empty query MUST clear the highlight and MUST be the only signal for it, with no separate clear command. | this spec section 3 | Toile `ambre_toile::logic::convergence_tests::find_ignores_non_text_elements_and_empty_query`; Trame clears the highlight on close; Prose design-only | verified |
| `FIND-006` | A replace-all MUST be a single undo item, whatever the match count. | this spec section 3; `ambre-spec-undo.md` | Trame `ambre_trame::logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group`; Prose design-only; Toile carries no replace -- section 5 | verified |
| `FIND-007` | A replace MUST rewrite the active match and then advance to the next. | this spec section 4 | Trame `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances`; Prose design-only; Toile carries no replace -- section 5 | verified |

* **Budget.** none of its own: a search is one linear walk over the scope, and
  `ambre-spec-budgets.md` budgets no search; a replace lands under the batch row of
  `ambre-spec-budgets.md section 1`. The walk runs on every keystroke and no benchmark measures it.
* **Boundary.** none of its own: a search reads the document in the Logic thread and the query never
  leaves the client; a replace is a document write, and `ZK-001` seals every payload before it
  leaves the client.
* **Verification.** 6 `verified`, 1 `unverified`, 0 `unimplemented`. The rules that make the feature
  worth having -- it writes nothing, it reaches past the screen, it wraps, an empty query clears, a
  replace-all is one undo -- are each held in Trame or Toile. What is loose is the reveal, which is
  each application's own scroll. Prose is design-only and adds no code to any row.

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
   backward, wrapping either way, and the first hit when the resume point is no longer one
   (`FIND-003`);
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
| `Escape`, or the close button | dismisses the panel and clears the highlight (`FIND-005`) |

**The panel is chrome** (`ambre-spec-chrome.md`), pinned to a corner of the surface, never painted on
the canvas: a text field with a few toggles, which the shared toolkit already builds. It holds only
the query and its options; the hits and the current match come from the answer, so nothing on the UI
thread can disagree with the document. Which chord opens find is `ambre-spec-commands.md`.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The scan runs in the Logic thread | the UI cache holds one viewport, so a search there would find only what is already on screen -- and a document is mostly past the edge of the screen | scanning the UI cache, which silently bounds the search by the scroll position |
| The panel holds no cursor and no hit list | the Logic thread already tracks the origin and computes the hits, and a second copy of the index is the one that drifts under a merge or a wrap | the panel walking its own hit list against its own index |
| An empty query is the clear signal | the panel needs a way to say "no highlight" and the engine needs no second command for it | a separate clear command |
| A search writes nothing | a search is one reader's attention; two readers searching different words in one document must not see each other's highlights, nor undo each other's searches | recording the session in the document, which shares one reader's search with every peer |
| A replace-all is one undo item | a replace over a thousand matches is one decision, and undoing it one match at a time is not an undo | one undo item per rewritten match |
| Replace is the text applications' | replacing across a board's free-form elements rewrites bodies the user cannot all see, on a surface where a note is a drawing as much as a text; the cost is high and the gesture is rare | one replace model for every application, which Toile declines (`3-Toile/features/ambre-spec-toile-find.md`) |

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
