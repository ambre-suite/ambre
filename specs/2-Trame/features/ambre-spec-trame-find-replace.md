# AMBRE -- FEATURE: FIND AND REPLACE

**Type:** Vertical (feature)
**Implementation:** implemented -- the shared session the "One find and replace" Story wants is not.

> A user looks for a value in a sheet of 100 000 rows. The search runs where the document is, not
> where the screen is, so a match twenty thousand rows down is found and scrolled to.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-075` | A search MUST cover the whole scope, never only the cells the viewport holds. | this spec section 3 | `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` | verified |
| `TRAME-UI-076` | Find next MUST land on the first match after the active cell, and MUST wrap at the end. | this spec section 3 | `ambre_trame::logic::tests::group6::find_next_matches_and_wraps` | verified |
| `TRAME-UI-077` | Case sensitivity and whole-cell matching MUST apply to a search as to a replace. | this spec section 4 | `ambre_trame::logic::tests::group6::find_next_honors_case_and_whole_cell` | verified |
| `TRAME-UI-078` | A plain cell MUST be matched on its displayed value. | this spec section 3 | `ambre_trame::logic::tests::group6::find_next_matches_and_wraps`, `::replace_all_gates_formula_cells` | verified |
| `TRAME-UI-079` | A formula cell MUST be matched on its source text, and only when the user asks for it. | this spec section 3 | `ambre_trame::logic::tests::group6::replace_all_gates_formula_cells` | verified |
| `TRAME-UI-080` | A case-insensitive replace MUST leave the rest of the cell byte for byte. | this spec section 3 | `ambre_trame::logic::find::tests::case_insensitive_replace_survives_a_non_length_preserving_lowercase`, `::case_insensitive_replace_handles_the_ordinary_cases`, `::whole_cell_rewrite_swaps_the_entire_content` | verified |
| `TRAME-UI-081` | Replace MUST rewrite the active match and advance to the next. | this spec section 4 | `ambre_trame::logic::tests::group6::replace_one_replaces_active_and_advances` | verified |
| `TRAME-UI-082` | Replace all MUST be one undo item, whatever the number of matches. | this spec section 3 | `ambre_trame::logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group` | verified |
| `TRAME-UI-083` | A workbook-scoped replace MUST reach every sheet, whichever tab is active. | this spec section 4 | `ambre_trame::logic::tests::group6::replace_all_spans_sheets` | verified |
| `TRAME-UI-084` | A search MUST NOT change the document. | this spec section 3 | none. `find_next` returns a display-only outcome, which carries no delta -- but it takes `&mut self`, so nothing but review holds the rule | unverified |

* **Budget.** none: a search is one walk over the cells a sheet holds, and
  `ambre-spec-budgets.md` budgets no search. A replace lands under the batch row of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a replace is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 9 `verified`, 1 `unverified`. This is the best-held feature of the area, for
  one structural reason: the whole behaviour lives in the Logic thread, and the panel above it only
  carries fields. `TRAME-UI-084` is the one rule stated about an absence, and absences are what the
  suite states least.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| replaces every `draft` with `final` | types `draft` into a cell A already passed | the new cell keeps `draft` | A's count says the sheet is clean, and one `draft` is on screen | none. `ReplaceAll` is not in the convergence property's op set |
| replaces in the whole workbook | renames a sheet | both, independently | the replace lands on the sheet by its id, so the rename does not move it | none |
| searches | edits any cell | unchanged by the search | A's highlight may point at a cell that no longer matches | none, and no rule covers it -- section 6 |

---

## 3. MODEL

**Containers.** None. A search writes nothing; a replace writes `cells_{sheet}`, declared in
`ambre-spec-trame-schema.md section 3`.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::FindNext { query, match_case, whole_cell }` | UI to Logic | the query and its two options. The origin is the active cell the Logic thread already tracks |
| `TrameEvent::FindResult { matches, active }` | Logic to UI | every hit in row-major order, and the index of the one to move to |
| `TrameCommand::ReplaceAll { query, replacement, match_case, whole_cell, in_formulas, all_sheets }` | UI to Logic | the query, its replacement and the four options |
| `TrameCommand::ReplaceOne { query, replacement, match_case, whole_cell, in_formulas }` | UI to Logic | the same, for the active match alone |
| `TrameEvent::ReplaceDone { replaced }` | Logic to UI | how many cells were rewritten |

**Panel state**, held in the chrome, never in the document:

```rust
pub struct FindQuery {
    pub query: String,
    pub match_case: bool,
    pub whole_cell: bool,
}
```

**The search, in steps:**

1. Fold the query when the search is case-insensitive.
2. Walk the used range in row-major order, reading each cell's displayed value.
3. A formula cell is read from its source text instead, and only when the option is set.
4. Collect every hit; elect the first one strictly after the active cell, wrapping to the top.
5. A replace rewrites the raw text of each hit, inside one undo group, and answers with the count.

**Code.** `logic/find.rs` for the scan, the match and the rewrite; `chrome/find_replace.rs` for the
panel.

---

## 4. SURFACE

| Gesture | Effect |
|---|---|
| `Ctrl+F` | opens the panel in find mode: the query field alone |
| `Ctrl+H` | opens it in replace mode: query, replacement, and the two extra options |
| `Enter`, or Find next | sends the query and moves the active cell to the elected match |
| Replace | rewrites the active match, then advances (`TRAME-UI-081`) |
| Replace all | rewrites every match in scope, and toasts the count |
| `Escape`, or the close button | dismisses the panel and clears the highlight |

| Option | Applies to | Effect |
|---|---|---|
| `match_case` | both modes | stops the case folding |
| `whole_cell` | both modes | demands an exact cell, not a substring |
| `in_formulas` | replace only | admits formula cells, matched on their source text |
| `all_sheets` | replace only | widens the scope from the active sheet to the workbook |

**Rendering.** The panel is a DOM component pinned to the top-right of the grid, the same chrome
pattern as the filter popover and the format bar, never a canvas overlay. It shows the position of
the active match in the total. The matches themselves are local view state and reach no
collaborator.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The scan runs in the Logic thread | the UI cache holds one viewport, so a search there would find only what was already on screen -- the same argument as the authoritative copy | scanning the UI cache, which silently bounds the search by the scroll position |
| The panel carries no cursor | the Logic thread already tracks the active cell for formatting and structural intents, so a second copy of the search origin could disagree with it | sending the origin with every request |
| A plain cell matches on what is displayed, a formula on its source | a user searching for `42` means the number they can read; a user searching inside formulas means the text they typed. One rule for both would make a number format decide whether a cell matches | matching the raw text everywhere, which hides formatted values |
| Replace all is one undo group | a replace over a thousand cells is one decision, and undoing it one cell at a time is not an undo | one undo item per rewritten cell |
| The scope is the sheet, unless asked | a workbook-wide rewrite is the destructive case, so it is the one the user has to opt into | defaulting to the workbook |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A stale highlight.** The match list is a snapshot. A local edit, or a remote one, can empty a
  cell the highlight still points at, and nothing invalidates it. The two-peer table's last row.
* **The shared find session.** The "One find and replace" Story replaces this whole-sheet re-scan
  with one live match session shared by every app. This document describes the Trame-only form that
  exists today, and it does not know which of its rules will survive that move.
* **No test drives the panel.** Nine rules are held in the Logic thread and none of them covers the
  fields above it: the option toggles, the mode switch and the `Escape` clear are asserted nowhere.
* **A replace inside a filtered sheet.** The scan walks the used range, so it rewrites rows the
  filter hides. No rule says whether a hidden row is in scope.
