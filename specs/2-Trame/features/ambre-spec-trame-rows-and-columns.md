# AMBRE -- FEATURE: ROWS AND COLUMNS

**Type:** Vertical (feature)
**Implementation:** partial -- inserting, deleting and reordering are built; four of their rules the
code contradicts, and one of the four contradicts it across the wire.

> A user inserts a row above the one they are on, deletes the three they selected, or drags a column
> to another position. Everything on those lines moves with them, and every formula still adds up.

Three operations on one pair of lists. They are one feature because they share the whole mechanism:
a line is a stable identity, a cell is keyed by the pair of identities it sits at, and that is what
makes an insert one operation instead of a rewrite of the sheet below it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-085` | `INV` | A merge MUST cover the same cells after a line is inserted, deleted or moved. | this spec section 4 | `ambre_trame::logic::tests::group3::merge_follows_structural_insert`. It drives the insert alone: the delete and the drag reach the same rebuild, and no test reads a merge after either | verified |
| `TRAME-170` | `INV` | The height, the width and the hidden mark of a line MUST follow that line when the lines move. | this spec section 3 | none. All three are keyed by the line's identity, so they follow by the same construction the cells do; no test reads a height, a width or a hidden row after a structural operation | unverified |
| `TRAME-091` | `CONC` | Two users inserting at the same position MUST both get a line. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` (proptest), whose op set carries an insert and a delete on both axes | verified |

### 1.2 Insert a line

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-080` | `POST` | Inserting a line MUST move every line after it, with the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_row_shifts_cells_down`, `::insert_and_delete_column_shift_cells` | verified |
| `TRAME-087` | `POST` | An insert from a menu or a header MUST land relative to the cell the user is on. | this spec section 4 | `ambre_trame::logic::tests::group2::insert_intents_land_relative_to_active_cell` | verified |
| `TRAME-086` | `FRAME` | An insert past the end of the used range MUST write nothing. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_beyond_used_range_is_noop`, which asserts neither an event nor a delta comes out | verified |
| `TRAME-171` | `FRAME` | Inserting a line inside a block a formula adds up MUST NOT push the last line of that block out of it. | this spec section 5 | none, and the code does the **opposite**: the insert reseeds the engine, which re-binds every reference by position, so `=SUM(A1:A10)` still covers ten positions and the value that was on the tenth is now on the eleventh, outside the sum. Section 5 claims the two products agree here; they do not | unverified |

### 1.3 Delete a line

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-081` | `POST` | Deleting a line MUST leave none of its cells in the document. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges`, which asserts the absence through a snapshot round trip | verified |
| `TRAME-082` | `POST` | Deleting a line MUST leave none of its formatting in the document either. | this spec section 6 | none. The purge covers the cell styles, and the test asserts the text alone; the width or height of the removed line, and its hidden-row mark, are never dropped | unverified |
| `TRAME-172` | `FRAME` | Deleting a line MUST NOT drop a cell that sits on another line. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges`, which asserts the lines above and below the removed one keep their values across the round trip | verified |

### 1.4 Drag a line to another slot

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-083` | `POST` | Moving a line MUST carry the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data`, `::move_column_carries_cells` | verified |
| `TRAME-084` | `FRAME` | Moving a line MUST change no formula result. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data`, which reads the formula at its new position and gets its old answer | verified |
| `TRAME-173` | `FRAME` | Releasing a line on the slot it came from MUST write nothing. | this spec section 4 | none. The move returns before the list operation when the two positions are equal, so no delta and no event come out, and no test drives the case | unverified |

### 1.5 Delete the selected lines

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-174` | `POST` | Deleting a selection of lines MUST remove every line it spans. | this spec section 4 | `ambre_trame::logic::tests::group2::delete_selected_rows_drops_the_whole_span`, over a three-row span | verified |
| `TRAME-088` | `POST` | Deleting a selection of lines MUST undo as one step. | this spec section 6 | none, and the code does the **opposite**: each line is deleted and committed on its own, so a five-row delete takes five undos. It contradicts `TRAME-017`, whose cited tests drive the batch-write path instead | unverified |
| `TRAME-175` | `POST` | Every line a selection deletes MUST reach the collaborators. | this spec section 6 | none, and the code does the **opposite**: each line flushes its own delta and only the last one is returned, so the others are exported, dropped, and never re-offered. A five-row delete reaches a peer as one | unverified |
| `TRAME-176` | `FRAME` | Deleting a selection of lines MUST NOT touch a line outside it. | this spec section 4 | `ambre_trame::logic::tests::group2::delete_selected_rows_drops_the_whole_span`, which asserts the line above the span and the line below it keep their values | verified |

### 1.6 Undo an insert

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-090` | `POST` | Undoing an insert MUST put every later line back where it was. | this spec section 3 | `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index` | verified |
| `TRAME-177` | `FRAME` | Undoing an insert MUST NOT empty the lines it moves back. | this spec section 3 | the same test, which reads both shifted lines after the undo and gets their values | verified |

### 1.7 Type into a cell far below the sheet

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-089` | `FAIL` | A write more than ten thousand lines past the used range MUST be refused. | this spec section 5 | none. The cap is a constant read in one branch of the materialization path, and no test enters the branch; a dropped write is silent apart from a log line | unverified |

* **Budget.** the `SheetIndex` rebuild of 10k rows in < 10 ms
  (`ambre-spec-budgets.md section 1`): a structural operation is what that row was written
  for. No bench measures it.
* **Boundary.** none of its own: an insert, a delete and a move are document writes, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 13 `verified`, 7 `unverified`, 0 `unimplemented`. Twelve rows became twenty,
  and one new mechanism came with them, so the ratio deserves the note section 5 of the migration
  contract asks for: the four new `verified` rows are frame conditions the completeness rule
  forced out of hiding, and each one is an assertion two already-cited tests were making without a
  rule to hold. Four of the seven gaps are rules the code contradicts, and `TRAME-175` is the one
  that matters: a multi-line delete flushes one delta per line and returns only the last, so the
  peers see one row go where four went. `TRAME-171` is the second: an insert inside a summed block
  pushes its last line out of the sum, which section 5 says cannot happen. The other three are
  untested rather than broken -- the metrics that follow a line, the drag that lands on its own
  slot, and the guard past the used range.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| inserts a row at 3 | inserts a row at 3 | two new rows, in a canonical order | two blank rows where one was asked for, on both screens (`TRAME-091`) | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` |
| deletes row 3 | writes into row 3 | the row gone, and the write on whichever line the identity kept | the same grid, and B's value either lost with the row or on the line below | the same property, whose op set carries `DeleteRow` and `SetText` |
| drags column B to the front | writes into column B | the column moved, with the write on it | both see the value in the moved column (`TRAME-083`) | the same property, whose op set carries `MoveColumn` and `SetText` |
| inserts a row above a merge | reads the merge | the merge, over the same cells | the block unchanged, one row lower (`TRAME-085`) | `ambre_trame::logic::tests::group3::merge_follows_structural_insert`, one peer |

---

## 3. MODEL

**Containers.** `rows_{sheet}` and `cols_{sheet}`, declared in
`ambre-spec-trame-schema.md section 3`: an ordered list whose element value is the line's stable
identity. The order is the position, the value is the identity, and no cell key changes when the
order does.

**Why an insert is one operation.** A cell is keyed by identities, never by position, so an
insertion moves every cell below it without rewriting one key. The same primitive carries a move,
and a sort (`TRAME-004`).

**The three operations, and what each one costs:**

| Operation | Document | Afterwards |
|---|---|---|
| insert | one insertion, carrying a freshly minted identity | rebuild the position cache, rebuild the merges, reseed the engine, the styles and the filter |
| delete | one removal, then a purge of every cell and style keyed by the removed identity | the same tail |
| move | one move, and nothing else | rebuild the position cache only. The engine is **not** reseeded: each reference resolves to its line's new position by itself (`TRAME-084`) |

**Virtual space**, past the used range:

| Question | Answer |
|---|---|
| How large the grid is | 100 000 rows by 1 000 columns |
| What the lists hold | the used range only. Past it a line has no element and no storage |
| An insert out there | a no-op (`TRAME-086`) |
| A write out there | materializes every line up to it, unless the jump exceeds ten thousand (`TRAME-089`) |

**Code.** `logic/structural.rs` -- `insert_axis`, `delete_axis`, `move_axis`,
`delete_selected_axis`, `purge_axis`, `after_structural`; `logic/cells.rs` -- `materialize` and the
cap; `ui/view/input.rs` and `ui/view/interact.rs` -- the reorder drag and the insert intents.

---

## 4. SURFACE

**Where each operation is triggered:**

| Trigger | Operation | Specified in |
|---|---|---|
| the context menu, on a cell or a header | insert or delete, rows or columns | `features/ambre-spec-trame-context-menu.md` |
| a click on a header boundary affordance | an insert at that boundary | `features/ambre-spec-trame-headers.md` |
| a drag of a header body | a move of that line | this document |

**The reorder drag**, on a header already selected as a whole axis:

| Step | Behaviour |
|---|---|
| press on a selected header | starts a reorder rather than a new selection |
| move | a bold guide marks the boundary the line would land at |
| release on another slot | the line moves there, carrying everything on it (`TRAME-083`) |
| release on its own slot | nothing is sent |

**Two properties of every insert and delete:**

| Property | Detail |
|---|---|
| an insert carries no coordinates | the menu and the header affordance both move the tracked active cell onto the target line, then send an intent -- above, below, left, right (`TRAME-087`) |
| the new line is selected | so the insertion reads as a change, not as a silent shift |
| merges are recomputed first | a masked cell is out of the dependency graph before the engine reseeds (`TRAME-085`) |
| the hidden rows need no work | the set is keyed by identity, so a filter survives a shift untouched |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A line is a stable identity carried as the element's value | keying a cell by its row and column number | the position is what changes; the identity is what a cell key, a merge, a filter and a compiled reference all hold. Making the position the key would mean rewriting all four on every insert |
| The lists are dense over the used range only | a list per virtual row | a 100 000-row list per sheet costs a hundred thousand elements to say nothing |
| The lists are dense over the used range only | a run-length encoding of the empty stretches | it buys back the storage but loses the direct index: reaching a position means walking the runs, on the path every viewport fill takes |
| A write far past the used range is dropped | materializing whatever is asked | one accidental keystroke in the row-number box would otherwise materialize tens of thousands of lines on every peer, for one value |
| A write far past the used range is dropped | clamping the write to the high-water mark | it silently lands the value on a line the user did not name, which is worse than not landing it: the sheet looks written and the number is in the wrong row |
| A move does not reseed the engine | reseeding after every structural operation, as a delete does | every reference resolves through the position cache, and a move only permutes it. Re-parsing every formula to reach the same answer is the cost a stable identity exists to avoid |
| A delete purges the removed line's cells | leaving the entries and filtering them at read time | a cell keyed by a deleted identity is unreachable and would ride in the document and every sync payload for ever |
| An insert from any surface goes through one intent | each surface computing a grid position and sending it | the menu, the header affordance and a future keyboard chord then resolve the position in one place, and cannot disagree about what "above" means |
| An insert rewrites no formula | rewriting every dependent formula at insert time, as Excel does | Excel's implicit rewrite of `=SUM(A1:A10)` into `=SUM(A1:A11)` is not idempotent: two users inserting into the same range at once apply two rewrites and converge on a formula neither wrote. A range endpoint is positional, so the new line falls inside the block and is counted with no edit at all -- and the block's old last line falls out of it, which is where the two products part (`TRAME-171`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A multi-line delete reaches a collaborator as one line** (`TRAME-175`). The handler deletes
  each line through the single-line path, and that path flushes a CRDT delta of its own. The loop
  keeps the last outcome and drops the rest, and the export cursor has already moved past them, so
  the earlier removals are never offered again. Locally the sheet is right; on every other screen
  it is not, and nothing reconciles the two. It is the same loop `TRAME-088` names, seen from the
  wire instead of from the undo stack.
* **An insert inside a summed block drops the block's last line** (`TRAME-171`). The insert reseeds
  the engine, and the reseed re-binds `=SUM(A1:A10)` against the new layout by position. The new
  line joins the sum, and the value that was on the tenth line, now on the eleventh, leaves it. The
  total changes by two lines' worth for an operation that wrote none.
* **Nothing reads a metric after a structural operation** (`TRAME-170`, `TRAME-173`). A height, a
  width and a hidden mark are keyed by the line's identity, so the argument that they follow the
  line is the argument that the cells do -- and the cells have tests. A drag released on its own
  slot writes nothing for the same untested reason.
* **Deleting a span of lines takes one undo per line** (`TRAME-088`). The handler deletes from the
  far edge inward and commits each line, so a five-row delete is five undo items. `TRAME-017` says
  one command is one undo item, and this is a command that is not. A user who deletes ten rows and
  presses undo gets one row back.
* **A deleted line leaves its metrics behind** (`TRAME-082`). The cells and the cell styles are
  purged; the width or height stored for that line, and its hidden-row mark, keep their entries in
  the document under an identity nothing can resolve. A sheet edited for long enough accumulates
  them, and the same leak happens one level up when a sheet is deleted (`TRAME-076`).
* **The materialization cap is a branch nothing enters** (`TRAME-089`). Ten thousand lines past the
  used range, the write is dropped with a log line. No test, no event, and nothing tells the user
  their value did not land.
* **A row delete does not break the formulas that read it.** The delete path rebinds every formula
  from its text, so a reference to the deleted line reads the shifted cell rather than an error. It
  is the divergence recorded as `TRAME-008` in `features/ambre-spec-trame-formula-engine.md`, and
  this is the operation that produces it.
* **Nothing inserts or deletes more than one line at a time.** There is no "insert three rows", only
  three inserts, and the multi-line delete is a loop over single deletes -- which is where
  `TRAME-088` comes from.
