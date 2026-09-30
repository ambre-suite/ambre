# AMBRE -- FEATURE: ROWS AND COLUMNS

**Type:** Vertical (feature)
**Implementation:** partial -- inserting, deleting and reordering are built. Four of their rules the
code contradicts, and one of the four contradicts it across the wire.

> A user inserts a row above the one they are on, deletes the three they selected, or drags a column
> to another position. Everything on those lines moves with them, and every formula still adds up.

Three operations on one pair of lists. They are one feature because they share the whole mechanism.
A line is a stable identity, and a cell is keyed by the pair of identities it sits at. That is what
makes an insert one operation instead of a rewrite of the sheet below it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-085` | `INV` | A merge MUST cover the same cells after a line is inserted, deleted or moved. | this spec section 4 | `ambre_trame::logic::tests::group3::merge_follows_structural_insert` -- it drives the insert alone, the delete and the drag reaching the same rebuild with no test reading a merge after either | none | verified |
| `TRAME-170` | `INV` | The height, the width and the hidden mark of a line MUST follow that line when the lines move. | this spec section 3 | none | unheld -- all three are keyed by the line's identity, so they follow by the same construction the cells do, and no test reads a height, a width or a hidden row after a structural operation; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-091` | `CONC` | Two users inserting at the same position MUST both get a line. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest whose op set carries an insert and a delete on both axes | none | verified |

### 1.2 Insert a line

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-080` | `POST` | Inserting a line MUST move every line after it, with the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_row_shifts_cells_down`; `::insert_and_delete_column_shift_cells` | none | verified |
| `TRAME-087` | `POST` | An insert from a menu or a header MUST land relative to the cell the user is on. | this spec section 4 | `ambre_trame::logic::tests::group2::insert_intents_land_relative_to_active_cell` | none | verified |
| `TRAME-086` | `FRAME` | An insert past the end of the used range MUST write nothing. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_beyond_used_range_is_noop` -- it asserts neither an event nor a delta comes out | none | verified |
| `TRAME-171` | `FRAME` | Inserting a line inside a block a formula adds up MUST NOT push the last line of that block out of it. | this spec section 5 | none | fails -- the insert reseeds the engine, which re-binds every reference by position, so `=SUM(A1:A10)` still covers ten positions and the value that was on the tenth is now on the eleventh, outside the sum. Section 5 claims the two products agree here, and they do not; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |

### 1.3 Delete a line

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-081` | `POST` | Deleting a line MUST leave none of its cells in the document. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges` -- it asserts the absence through a snapshot round trip | none | verified |
| `TRAME-082` | `POST` | Deleting a line MUST leave none of its formatting in the document either. | this spec section 6 | `ambre_trame::logic::tests::group2::deleted_line_leaves_no_formatting` | none | verified |
| `TRAME-172` | `FRAME` | Deleting a line MUST NOT drop a cell that sits on another line. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges` -- it asserts the lines above and below the removed one keep their values across the round trip | none | verified |

### 1.4 Drag a line to another slot

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-083` | `POST` | Moving a line MUST carry the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data`; `::move_column_carries_cells` | none | verified |
| `TRAME-084` | `FRAME` | Moving a line MUST change no formula result. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data` -- it reads the formula at its new position and gets its old answer | none | verified |
| `TRAME-173` | `FRAME` | Releasing a line on the slot it came from MUST write nothing. | this spec section 4 | none | unheld -- the move returns before the list operation when the two positions are equal, so no delta and no event come out, and no test drives the case; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Delete the selected lines

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-174` | `POST` | Deleting a selection of lines MUST remove every line it spans. | this spec section 4 | `ambre_trame::logic::tests::group2::delete_selected_rows_drops_the_whole_span` -- over a three-row span | none | verified |
| `TRAME-088` | `POST` | Deleting a selection of lines MUST undo as one step. | this spec section 6 | `ambre_trame::logic::tests::group2::delete_selected_rows_undoes_as_one_step` | none | verified |
| `TRAME-175` | `POST` | Every line a selection deletes MUST reach the collaborators. | this spec section 6 | `ambre_trame::logic::tests::group2::delete_selected_rows_reaches_peers_whole` | none | verified |
| `TRAME-176` | `FRAME` | Deleting a selection of lines MUST NOT touch a line outside it. | this spec section 4 | `ambre_trame::logic::tests::group2::delete_selected_rows_drops_the_whole_span` -- it asserts the line above the span and the line below it keep their values | none | verified |

### 1.6 Undo an insert

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-090` | `POST` | Undoing an insert MUST put every later line back where it was. | this spec section 3 | `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index` | none | verified |
| `TRAME-177` | `FRAME` | Undoing an insert MUST NOT empty the lines it moves back. | this spec section 3 | `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index` -- it reads both shifted lines after the undo and gets their values | none | verified |

### 1.7 Type into a cell far below the sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-089` | `FAIL` | A write more than ten thousand lines past the used range MUST be refused. | this spec section 5 | none | unheld -- the cap is a constant read in one branch of the materialization path, no test enters the branch, and a dropped write is silent apart from a log line; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the `SheetIndex` rebuild of 10k rows in < 10 ms
  (`ambre-spec-budgets.md section 1`): a structural operation is what that row was written
  for. No bench measures it.
* **Boundary.** none of its own: an insert, a delete and a move are document writes, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 16 `verified`, 1 `failing`, 3 `untested`. What holds is the single-line gesture
  and the frame conditions around it. What is loose is the metrics that follow a line, the drag that
  lands on its own slot and the guard past the used range. The failure matters: an insert in a
  summed block pushes its last line out of the sum (`TRAME-171`).

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

* **An insert inside a summed block drops the block's last line** (`TRAME-171`). The insert reseeds
  the engine, and the reseed re-binds `=SUM(A1:A10)` against the new layout by position. The new
  line joins the sum, and the value that was on the tenth line, now on the eleventh, leaves it. The
  total changes by two lines' worth for an operation that wrote none.
* **Nothing reads a metric after a structural operation** (`TRAME-170`, `TRAME-173`). A height, a
  width and a hidden mark are keyed by the line's identity. The argument that they follow the line
  is the argument that the cells do, and the cells have tests. A drag released on its own
  slot writes nothing for the same untested reason.
* **The materialization cap is a branch nothing enters** (`TRAME-089`). Ten thousand lines past the
  used range, the write is dropped with a log line. No test, no event, and nothing tells the user
  their value did not land.
* **A row delete does not break the formulas that read it.** The delete path rebinds every formula
  from its text, so a reference to the deleted line reads the shifted cell rather than an error. It
  is the divergence recorded as `TRAME-008` in `features/ambre-spec-trame-formula-engine.md`, and
  this is the operation that produces it.
* **Nothing inserts or deletes more than one line at a time.** There is no "insert three rows", only
  three inserts. The multi-line delete removes its lines in one commit, so it is one undo item and
  one delta (`TRAME-088`, `TRAME-175`).
