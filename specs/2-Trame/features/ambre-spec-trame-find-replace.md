# AMBRE -- FEATURE: TRAME FIND AND REPLACE

**Type:** Vertical (feature)
**Implementation:** implemented -- the shared session the
`One find and replace` Story (`ambre-roadmap.md` E8) wants is not.

> A user looks for a value in a sheet of 100 000 rows. The search runs where the document is, not
> where the screen is, so a match twenty thousand rows down is found and scrolled to.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-find.md`,
`FIND-001` through `FIND-015`. The search writes nothing, covers the whole scope, wraps, reveals its
match, and the replace-all is one undo item. Trame's own turn is what a cell match is.

### 1.1 What counts as a match

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-077` | `INV` | Case sensitivity and whole-cell matching MUST apply to a search as to a replace. | this spec section 4 | `ambre_trame::logic::tests::group6::find_next_honors_case_and_whole_cell` -- both options reach both gestures through one predicate | none | verified |
| `TRAME-UI-078` | `INV` | A plain cell MUST be matched on its displayed value. | this spec section 3 | `ambre_trame::logic::tests::group7::find_matches_the_formatted_value` | none | verified |
| `TRAME-UI-079` | `INV` | A formula cell MUST be matched on its source text, and only when the user asks for it. | this spec section 3 | `ambre_trame::logic::tests::group6::replace_all_gates_formula_cells` -- it runs over both settings of the option, and the option is a replace option: a find has none, so it matches a formula on its value (section 6) | none | verified |

### 1.2 Replace every match in the workbook

The scope toggle is Trame's own: `ambre-spec-find.md` owns the replace, this table owns what a
workbook scope reaches and what it leaves alone.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-083` | `POST` | A workbook-scoped replace MUST reach every sheet, whichever tab is active. | this spec section 4 | `ambre_trame::logic::tests::group6::replace_all_spans_sheets` -- it rewrites a match on an inactive sheet and reads it back | none | verified |
| `TRAME-UI-080` | `POST` | A case-insensitive replace MUST leave the rest of the cell byte for byte. | this spec section 3 | `ambre_trame::logic::find::tests::case_insensitive_replace_survives_a_non_length_preserving_lowercase`; `::case_insensitive_replace_handles_the_ordinary_cases`; `::whole_cell_rewrite_swaps_the_entire_content` -- one rewrite serves both gestures | none | verified |
| `TRAME-UI-138` | `FRAME` | A workbook-scoped replace MUST NOT change which sheet the user is looking at. | this spec section 4 | `ambre_trame::logic::tests::group10::a_workbook_replace_keeps_the_active_sheet` | none | verified |

* **Budget.** none: a search is one walk over the cells a sheet holds, and `ambre-spec-budgets.md`
  budgets no search. A replace lands under the batch row of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a replace is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 6 `verified`; the shared rules and their coverage are `ambre-spec-find.md`. What
  holds is what a Trame match is. Both options reach both gestures, a formula is read only on
  request, and a rewrite preserves the bytes around the match. A workbook replace keeps the active
  sheet, which a test reads back.

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
| `TrameCommand::FindNext { query, options }` | UI to Logic | the query and the options, read for case and extent; the origin is the tracked active cell |
| `TrameEvent::FindResult { matches, active }` | Logic to UI | every hit in row-major order, and the index to move to |
| `TrameCommand::ReplaceAll { query, replacement, options }` | UI to Logic | the query, its replacement and the four options |
| `TrameCommand::ReplaceOne { query, replacement, options }` | UI to Logic | the same, for the active match alone; the sheet scope is not read |
| `TrameEvent::ReplaceDone { replaced }` | Logic to UI | how many cells were rewritten |

**What a Trame match is** (the shared walk is `ambre-spec-find.md section 3`):

* a plain cell matches on its displayed value, number format included, so a search for `50.0%`
  finds the percentage the user reads (`TRAME-UI-078`);
* a formula cell matches on its source text, and only when the scope is `SearchScope::Formulas`
  (`TRAME-UI-079`);
* a case-insensitive replace rewrites the match alone and leaves the rest of the cell byte for byte
  (`TRAME-UI-080`);
* the row-major order is Trame's, and the origin is the active cell the Logic thread tracks.

The *find text* is the query as the user typed it, never empty (`FindText`). An empty query clears
the highlight and never reaches a scan (`FIND-005`). The *find options* are the refinements a find
applies to its query. Trame widens the shared set to four, one enum per panel checkbox:

```rust
pub struct FindOptions {
    pub case: CaseMatch,      // Insensitive | Sensitive
    pub extent: MatchExtent,  // Substring | WholeCell
    pub scope: SearchScope,   // Values | Formulas
    pub sheets: SheetScope,   // Active | All
}
```

**Code.** `logic/find.rs` for the scan, the match and the rewrite; `chrome/find_replace.rs` for the
panel.

---

## 4. SURFACE

The shared chords and the reveal are `ambre-spec-find.md section 4`. Trame's own options:

| Option | Applies to | Effect |
|---|---|---|
| `case` | both modes | `Sensitive` stops the case folding (`TRAME-UI-077`) |
| `extent` | both modes | `WholeCell` demands an exact cell, not a substring (`TRAME-UI-077`) |
| `scope` | replace only | `Formulas` admits formula cells, matched on their source text (`TRAME-UI-079`) |
| `sheets` | replace-all only | `All` widens the walk from the active sheet to the workbook (`TRAME-UI-083`) |

* **Rendering** -- the panel is a DOM component pinned to the top-right of the grid. It is the same
  chrome pattern as the filter popover and the format bar. It shows the position of the active match in the
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

* **A find cannot reach into formulas.** The scope is a replace option. `FindNext` carries it but
  the scan does not read it, so `Ctrl+F` matches a formula cell on its result. There is no way to
  look for the text a user typed. Nothing rules whether the find panel should offer the toggle the
  replace panel already has.
* **No test drives the panel.** The rules are held in the Logic thread and none of them covers the
  fields above it. The option toggles, the mode switch and the `Escape` clear are asserted nowhere.
* **A replace inside a filtered sheet.** The scan walks the used range, so it rewrites rows the
  filter hides. No rule says whether a hidden row is in scope.
