# AMBRE -- FEATURE: FIND AND REPLACE

**Type:** Vertical (feature)
**Implementation:** implemented -- the shared session the "One find and replace" Story wants is not.

> A user looks for a value in a sheet of 100 000 rows. The search runs where the document is, not
> where the screen is, so a match twenty thousand rows down is found and scrolled to.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-find.md`,
`FIND-001` through `FIND-007`: the search writes nothing, covers the whole scope, wraps, reveals its
match, and the replace-all is one undo item. Trame's own turn is what a cell match is.

### 1.1 What counts as a match

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-UI-077` | `INV` | Case sensitivity and whole-cell matching MUST apply to a search as to a replace. | this spec section 4 | `ambre_trame::logic::tests::group6::find_next_honors_case_and_whole_cell`, and both options reach both gestures through one predicate | verified |
| `TRAME-UI-078` | `INV` | A plain cell MUST be matched on its displayed value. | this spec section 3 | none. The scan reads the **computed** value, not the formatted one: a cell shown as `50.0%` is matched by `0.5` and not by what the user reads (`TRAME-108`). The cited tests carry no format, so they never separate the two | unverified |
| `TRAME-UI-079` | `INV` | A formula cell MUST be matched on its source text, and only when the user asks for it. | this spec section 3 | `ambre_trame::logic::tests::group6::replace_all_gates_formula_cells`, over both settings of the option. The option is a replace option: a find has none, so it matches a formula on its value -- section 6 | verified |

### 1.2 Replace every match in the workbook

The scope toggle is Trame's own: `ambre-spec-find.md` owns the replace, this table owns what a
workbook scope reaches and what it leaves alone.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-UI-083` | `POST` | A workbook-scoped replace MUST reach every sheet, whichever tab is active. | this spec section 4 | `ambre_trame::logic::tests::group6::replace_all_spans_sheets`, which rewrites a match on an inactive sheet and reads it back | verified |
| `TRAME-UI-080` | `POST` | A case-insensitive replace MUST leave the rest of the cell byte for byte. | this spec section 3 | `ambre_trame::logic::find::tests::case_insensitive_replace_survives_a_non_length_preserving_lowercase`, `::case_insensitive_replace_handles_the_ordinary_cases`, `::whole_cell_rewrite_swaps_the_entire_content`. One rewrite serves both gestures | verified |
| `TRAME-UI-138` | `FRAME` | A workbook-scoped replace MUST NOT change which sheet the user is looking at. | this spec section 4 | none. The other sheets are rewritten by id, through their cells map, and the active sheet is never switched; no test reads the active tab after the replace | unverified |

* **Budget.** none: a search is one walk over the cells a sheet holds, and `ambre-spec-budgets.md`
  budgets no search. A replace lands under the batch row of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a replace is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 4 `verified`, 2 `unverified`, 0 `unimplemented`. What a Trame match is -- both
  options reaching both gestures, a formula read only on request, a byte-preserving rewrite -- is
  held cell by cell, in the Logic thread, which is why this stays the best-held corner of the area:
  the panel above it only carries fields. The shared rules and their coverage are
  `ambre-spec-find.md`. Two rows are loose. `TRAME-UI-078` reads the computed value, so a formatted
  cell is matched on a text no one sees, which `TRAME-108` states from the other side.
  `TRAME-UI-138` is the frame condition of the workbook scope, and no test reads the active tab
  back.

---

## 2. WHAT EACH USER SEES

The shared scenario -- a search writes nothing, and a replace-all is one undo item -- is
`ambre-spec-find.md section 2`. Trame's own turn on the workbook and the formula cell:

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| replaces in the whole workbook | renames a sheet | both, independently | the replace lands on the sheet by its id, so the rename does not move it (`TRAME-UI-083`) | `logic::tests::group6::replace_all_spans_sheets` |
| replaces `=A1` across formula cells | reads a formatted number | the source rewritten, the number untouched | only the formulas the option admitted changed (`TRAME-UI-079`) | `logic::tests::group6::replace_all_gates_formula_cells` |

---

## 3. MODEL

* **Containers** -- none. A search writes nothing; a replace writes `cells_{sheet}`, declared in
  `ambre-spec-trame-schema.md section 3`.
* **Code** -- `logic/find.rs` for the scan, the match and the rewrite; `chrome/find_replace.rs` for
  the panel.

**Commands and events** (`ambre-bus`, the shared shape is `ambre-spec-find.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::FindNext { query, match_case, whole_cell }` | UI to Logic | the query and its two options; the origin is the active cell the Logic thread already tracks |
| `TrameEvent::FindResult { matches, active }` | Logic to UI | every hit in row-major order, and the index to move to |
| `TrameCommand::ReplaceAll { query, replacement, match_case, whole_cell, in_formulas, all_sheets }` | UI to Logic | the query, its replacement and the four options |
| `TrameCommand::ReplaceOne { query, replacement, match_case, whole_cell, in_formulas }` | UI to Logic | the same, for the active match alone |
| `TrameEvent::ReplaceDone { replaced }` | Logic to UI | how many cells were rewritten |

**What a Trame match is** (the shared walk is `ambre-spec-find.md section 3`):

* a plain cell matches on its displayed value, so a search for `42` finds the number the user reads
  (`TRAME-UI-078`);
* a formula cell matches on its source text, and only when the `in_formulas` option is set
  (`TRAME-UI-079`);
* a case-insensitive replace rewrites the match alone and leaves the rest of the cell byte for byte
  (`TRAME-UI-080`);
* the row-major order is Trame's, and the origin is the active cell the Logic thread tracks.

Trame widens the shared option set by one, which is what `whole_cell` means here:

```rust
pub struct FindQuery {
    pub query: String,
    pub match_case: bool,
    pub whole_cell: bool,
}
```

**Code.** `logic/find.rs` for the scan, the match and the rewrite; `chrome/find_replace.rs` for the
panel.

---

## 4. SURFACE

The shared chords and the reveal are `ambre-spec-find.md section 4`. Trame's own options:

| Option | Applies to | Effect |
|---|---|---|
| `match_case` | both modes | stops the case folding (`TRAME-UI-077`) |
| `whole_cell` | both modes | demands an exact cell, not a substring (`TRAME-UI-077`) |
| `in_formulas` | replace only | admits formula cells, matched on their source text (`TRAME-UI-079`) |
| `all_sheets` | replace only | widens the scope from the active sheet to the workbook (`TRAME-UI-083`) |

* **Rendering** -- the panel is a DOM component pinned to the top-right of the grid, the same chrome
  pattern as the filter popover and the format bar. It shows the position of the active match in the
  total. The matches themselves are local view state and reach no collaborator.

---

## 5. REJECTED ALTERNATIVES

The shared decisions -- the scan in the Logic thread, the panel with no cursor, the empty-query
clear, the one-undo replace-all -- are `ambre-spec-find.md section 5`. Trame's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A plain cell matches on what is displayed, a formula on its source | matching the raw text everywhere | it hides formatted values. A user searching for `42` means the number they can read, and a user searching inside formulas means the text they typed; one rule for both would make a number format decide whether a cell matches |
| The scope is the sheet, unless asked | defaulting to the workbook | a workbook-wide rewrite is the destructive case, so it is the one the user has to opt into |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- the live shared session, the stale highlight, the undebounced walk -- are in
`ambre-spec-find.md section 6`. Trame's own:

* **A format hides the text a user searched for** (`TRAME-UI-078`). The scan reads the value the
  engine computed, and the grid shows the value a format rendered. A cell displaying `50.0%` is
  found by `0.5` and missed by `50`, and a replace rewrites the raw text under a display no one
  matched. `features/ambre-spec-trame-number-formats.md section 5` states it from the format's side.
* **A find cannot reach into formulas.** `in_formulas` is a replace option; `FindNext` does not
  carry it, so `Ctrl+F` matches a formula cell on its result and there is no way to look for the
  text a user typed. Nothing rules whether the find panel should offer the toggle the replace panel
  already has.
* **No test drives the panel.** The rules are held in the Logic thread and none of them covers the
  fields above it: the option toggles, the mode switch and the `Escape` clear are asserted nowhere.
* **A replace inside a filtered sheet.** The scan walks the used range, so it rewrites rows the
  filter hides. No rule says whether a hidden row is in scope.
