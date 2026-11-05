# AMBRE -- FEATURE: ROWS AND COLUMNS

**Type:** Vertical (feature)
**Implementation:** partial -- inserting, deleting and reordering are built. Five rules are broken
across the wire: two peers that add one line at once add two lines.

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
| `TRAME-170` | `INV` | The height, the width and the hidden mark of a line MUST follow that line when the lines move. | this spec section 3 | `ambre_trame::logic::tests::group11::a_line_keeps_its_size_and_its_hidden_mark_when_it_moves` | none | verified |
| `TRAME-091` | `CONC` | Two users inserting at the same position MUST both get a line. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest whose op set carries an insert and a delete on both axes | none | verified |
| `TRAME-261` | `CONC` | Two peers writing at once in one line past the used range MUST both land on that line. | this spec section 3 | none | fails -- each peer materializes the line under an id it mints, so the merge keeps two lines. Two first writes in `A2` and `B2` leave the second value in `B3` on both peers. The documents converge, so no convergence test sees it. The fix is the derived uid of section 3, "A line a write adds"; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | failing |
| `TRAME-262` | `CONC` | Two peers writing at once on two different lines past the used range MUST each land on the line they wrote. | this spec section 3 | none | fails -- `A2` from one peer and `B3` from the other add three lines, so `B3` reads one row lower on both peers. The spike of 2026-11-05 reproduced it on host; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | failing |
| `TRAME-263` | `CONC` | An insert above the last line MUST NOT separate two concurrent first writes into the line after it. | this spec section 3 | none | fails -- each peer adds its own line after the last one, so the two values sit on two rows. The spike of 2026-11-05 reproduced it on host; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | failing |
| `TRAME-264` | `CONC` | A move of a new line MUST carry the value another peer wrote into that line at the same moment. | this spec section 3 | none | fails -- the other peer's value sits on a line of its own, so it stays behind when the line moves. The spike of 2026-11-05 reproduced it on host; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | failing |
| `TRAME-265` | `CONC` | A delete of a line MUST keep a value that a peer which never received the line wrote into it at the same moment. | this spec section 3 | none | unheld -- the value holds today because it sits on a line of its own; the derived uid makes the two lines one, and no test holds the value across that change; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | untested |
| `TRAME-266` | `CONC` | Undoing one of two concurrent first writes in one new line MUST leave the other value on that line. | this spec section 3 | none | fails -- the two writes sit on two lines, so the value that stays is one row away from where it was typed. The spike of 2026-11-05 reproduced it on host; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | failing |
| `TRAME-273` | `CONC` | Two first writes that derive two different uids MUST each keep its value, on a line of its own. | this spec section 3 | none | unheld -- each value holds today on its own minted line; the derived uid keeps two lines only when the two writers disagree on the line before or its generation. That split is a documented limit, and the planned mechanism is a proptest over those races; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | untested |
| `TRAME-274` | `CONC` | A `#REF!` that a delete produces MUST NOT read a value again, whatever uid a peer derives later. | this spec section 3; `TRAME-007` | none | unheld -- no uid repeats today, so no `#REF!` reads a value again; once a uid can repeat, the element binding and the freeze of `TRAME-277` each hold it, and no test drives a repeat after a delete; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | untested |
| `TRAME-278` | `CONC` | A formula that cites a line deleted at the same moment MUST read `#REF!`. | this spec section 3; `TRAME-007` | none | unheld -- the formula reads `#REF!` today through the uid lookup; once a uid can repeat, the element binding of section 3 holds it, and no test drives the four-peer race; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | untested |
| `TRAME-276` | `CONC` | Dropping a generation below the highest one a line carries MUST NOT let a later write derive a uid already derived after that line. | this spec section 3 | not built | unbuilt -- lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | unimplemented |

### 1.2 Insert a line

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-080` | `POST` | Inserting a line MUST move every line after it, with the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_row_shifts_cells_down`; `::insert_and_delete_column_shift_cells` | none | verified |
| `TRAME-087` | `POST` | An insert from a menu or a header MUST land relative to the cell the user is on. | this spec section 4 | `ambre_trame::logic::tests::group2::insert_intents_land_relative_to_active_cell` | none | verified |
| `TRAME-086` | `FRAME` | An insert past the end of the used range MUST write nothing. | this spec section 3 | `ambre_trame::logic::tests::group2::insert_beyond_used_range_is_noop` -- it asserts neither an event nor a delta comes out | none | verified |
| `TRAME-171` | `FRAME` | Inserting a line inside a block a formula adds up MUST NOT push the last line of that block out of it. | this spec section 5 | `ambre_trame::logic::tests::group8::an_insert_inside_a_range_widens_it` | none | verified |

### 1.3 Delete a line

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-081` | `POST` | Deleting a line MUST leave none of its cells in the document. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges` -- it asserts the absence through a snapshot round trip | none | verified |
| `TRAME-082` | `POST` | Deleting a line MUST leave none of its formatting in the document either. | this spec section 6 | `ambre_trame::logic::tests::group2::deleted_line_leaves_no_formatting` | none | verified |
| `TRAME-277` | `POST` | Deleting a line MUST rewrite each formula reference to a line the sheet no longer holds into a literal `#REF!`, in the same undo step. | this spec section 3; `TRAME-008` | not built | unbuilt -- lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | unimplemented |
| `TRAME-269` | `POST` | Deleting a line that two peers added at once MUST leave no copy of that line on the grid. | this spec section 3 | not built | unbuilt -- lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | unimplemented |
| `TRAME-172` | `FRAME` | Deleting a line MUST NOT drop a cell that sits on another line. | this spec section 3 | `ambre_trame::logic::tests::group2::delete_row_shifts_cells_up_and_purges` -- it asserts the lines above and below the removed one keep their values across the round trip | none | verified |

### 1.4 Drag a line to another slot

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-083` | `POST` | Moving a line MUST carry the values on it. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data`; `::move_column_carries_cells` | none | verified |
| `TRAME-084` | `FRAME` | Moving a line MUST change no formula result. | this spec section 3 | `ambre_trame::logic::tests::group2::move_row_carries_cells_and_formula_follows_data` -- it reads the formula at its new position and gets its old answer | none | verified |
| `TRAME-268` | `FRAME` | Moving a line MUST NOT show a second copy of a line that two peers added at once. | this spec section 3 | not built | unbuilt -- lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | unimplemented |
| `TRAME-173` | `FRAME` | Releasing a line on the slot it came from MUST write nothing. | this spec section 4 | `ambre_trame::logic::tests::group11::a_line_released_on_its_slot_writes_nothing` | none | verified |

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
| `TRAME-089` | `FAIL` | A write more than ten thousand lines past the used range MUST be refused. | this spec section 5 | `ambre_trame::logic::tests::group11::a_write_past_the_line_cap_is_refused` -- the line before the cap is written | none | verified |
| `TRAME-267` | `FRAME` | A write past the used range MUST NOT turn a `#REF!` to a deleted line back into a value. | this spec section 3; `TRAME-007` | none | unheld -- a minted uid is never minted twice, so the reference stays broken today; the element binding, the freeze of `TRAME-277` and the generation of the line before each keep the reference broken once a uid can repeat, and no test holds it across that change; lands with the `Two first writes in one new row keep one row` Story (`ambre-roadmap.md` E7) | untested |

* **Budget.** the `SheetIndex` rebuild of 10k rows in < 10 ms
  (`ambre-spec-budgets.md section 1`): a structural operation is what that row was written
  for. No bench measures it, nor the one hash a derived uid adds per line. The walk that re-anchors
  and freezes formula references answers to the structural edit row of that section.
* **Boundary.** none of its own: an insert, a delete and a move are document writes, and `ZK-001`
  seals every payload before it leaves the client. A derived uid hashes sealed content only.
* **Verification.** 20 `verified`, 5 `failing`, 5 `untested`, 4 `unimplemented`. What holds is the
  single-line gesture, the frame conditions around it, and the summed block an insert widens. What
  is loose is two peers adding one line at once, and the guard past the used range.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| inserts a row at 3 | inserts a row at 3 | two new rows, in a canonical order | two blank rows where one was asked for, on both screens (`TRAME-091`) | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` |
| deletes row 3 | writes into row 3 | the row gone, and the write on whichever line the identity kept | the same grid, and B's value either lost with the row or on the line below | the same property, whose op set carries `DeleteRow` and `SetText` |
| drags column B to the front | writes into column B | the column moved, with the write on it | both see the value in the moved column (`TRAME-083`) | the same property, whose op set carries `MoveColumn` and `SetText` |
| writes `A2`, the first write in row 2 | writes `B2` at the same moment | one row 2, carrying both values | both values on row 2 (`TRAME-261`) | not built |
| inserts a row above row 1, then writes `A3` | writes `B2` at the same moment | one new line after the old last line | both values on that line, row 3 on both screens (`TRAME-263`) | not built |
| deletes the new row 2 | writes `C2` without having received row 2 | row 2 deleted, and the line kept for the value written into it | `C2` kept on that line, the other values gone with the delete (`TRAME-265`) | not built |
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

**A line a write adds.** A write past the used range adds its lines under a **derived uid**. An
insert keeps a minted uid. Two peers that agree on the last line then derive one uid for the next
line, and their two cells key one line (`TRAME-261`).

| Step | Rule |
|---|---|
| input | the sheet id, the axis, the uid of the line before, and the generation that line carries |
| the line before | the last line of the axis on the writing peer, or the line this write derived just before. An empty axis takes the axis root instead |
| generation | one more than the highest generation `line_gen_{sheet}` records for the line before, or 0 when it records none |
| record | the write adds that generation under the line before, and drops the one below it, in the commit that pushes the line (`TRAME-276`) |
| hash | BLAKE2b, the workspace hash, over a fixed domain string and the four inputs, cut to 96 bits |
| spelling | 64 bits fill the `peer` half and 32 bits the `counter` half, so a reader parses it as any uid (`TRAME-248`) |
| counter | the uid spends none. The element that carries it spends one, as an insert does (`TRAME-242`) |

**What the derivation reads.** The line before, and the record that line carries. It never reads
the rest of the axis, and keeps no record of deleted uids. Two peers that hold the line before as
their last line, and the same record under it, derive one uid. An insert, a delete or a move above
that line changes neither input (`TRAME-263`).

**No uid comes back.** Each derivation raises the generation of the line before. A line derived,
then deleted or moved away, leaves its generation behind. The next write after the same line then
derives a new uid (`TRAME-271`). Without it, a write after a line moved away derives a uid still
live, and two lines become one. A delete purges the record keys of the line it removes.

**A delete freezes `#REF!`.** As a spreadsheet does, the delete rewrites each reference to the
deleted line into a literal `#REF!` token. A formula then holds no uid of a deleted line, so a uid
derived again revives nothing (`TRAME-274`, `TRAME-277`).

| Question | Answer |
|---|---|
| which walk | the re-anchor walk every structural edit already runs over every formula of every sheet: one more case, no second walk |
| which references | each single reference to a line the new layout lacks: the deleted line, and any line a concurrent delete removed |
| undo | the rewrite rides the delete's commit, so undoing the delete restores each uid reference |
| a formula written against the line by a peer that has not received the delete | it cites the element, which is gone, so it reads `#REF!`. The next structural edit freezes it (`TRAME-278`) |
| the frozen literal, now that the element binding closes the revival | kept. The formula bar shows `#REF!`, as in a spreadsheet, and an export writes it. The literal drops the element suffix from the text, and a read needs no lookup. The binding then guards the window before the freeze alone |
| cost | the walk parses every formula of every sheet. Measured on 2026-11-05 at 10k formula cells, it takes about 345 ms on the target phone, against the < 10 ms of the structural edit row of `ambre-spec-budgets.md section 1`. The `A structural edit within its budget on a large sheet` Story (`ambre-roadmap.md` E6) narrows it. The freeze adds one case to that walk. The dependency graph indexes each cell and range a formula reads, so it could narrow the walk to the formulas that read the edited lines. That is an optimization of the whole walk, which the budget row tracks, and the freeze does not need it |
| `TRAME-007`, `TRAME-008` | both still hold: the freeze writes the `#REF!` the lookup already reads |


**Two pushes of one uid.** Each peer pushes its own element, so the merged list holds the uid twice.
The element with the smallest element id is the line, and every other element of the uid is hidden
(`TRAME-270`). No write removes a hidden element: a reference may be bound to it. The axis view
maps each grid position to its list index. A delete removes every element of the uid, and a move
moves the visible one (`TRAME-268`, `TRAME-269`).

**A reference binds to the element.** A formula reference to a line stores the uid and the id of
the element it saw. Loro offers no element identity that survives a move: a cursor fails at the
first `mov` (spike of 2026-11-05). The element id therefore rides in the value, as the uid does.

| Question | Answer |
|---|---|
| element value | `"{uid}#{element}"` for a derived line, the element minted from the session counter. A minted line keeps the bare uid, and its element id is that uid |
| a live element, visible or hidden | reads the visible line of its uid. The merge of two first writes reads one line |
| a deleted element | reads `#REF!`, even when another element now carries the uid (`TRAME-278`) |
| lookup | the view maps an element id to its uid, then the uid to a position: two hash lookups per reference |
| reference text | the element suffix only for a derived line: up to 26 more bytes per reference. A minted line, and a frozen `#REF!`, carry none |
| `TRAME-007`, `TRAME-008` | both hold, now through the element: a deleted element reads `#REF!` whatever uid comes back |

**One type holds the repeat rule.** A deduplicated axis view is the only form in which Trame code
reads or writes the line order (`TRAME-275`). One module builds the view and owns the raw list:

| Part | What holds it |
|---|---|
| the view | built only inside the axis module, from a private constructor; a `compile_fail` doctest shows a build outside it refused |
| a position | resolved through the view alone, which maps it to the list index of the visible element |
| an element | resolved through the view alone, to its line or to `#REF!` |
| the raw list | private to the axis module; a gate refuses an axis container name outside that module, since a string compiles anywhere |

**The cases the two spikes of 2026-11-05 drove**, on host, through `TrameState` peers that exchange
snapshots. The first salted the uid against the local axis; the second, which this section keeps,
reads the generation alone. The third drove the element binding on raw Loro documents. Each
answered inside its 2-hour box.

| Case | Converged line | Row |
|---|---|---|
| first writes in `A2` and `B2` | one line, both values | `TRAME-261` |
| one peer writes row 2, the other row 3 | rows 2 and 3, the first derived by both | `TRAME-262` |
| a third peer inserts, deletes, moves and writes above the last line; one writer receives part of it, the other none | one line, both values: a proptest, 256 cases | `TRAME-263` |
| a move of the new line races the other first write | the moved line carries both values | `TRAME-264` |
| a third peer, which never received the line, writes into it while it is deleted | that line survives, with that value only | `TRAME-265` |
| an undo of one of the two writes | the other value stays on the line | `TRAME-266` |
| a write after the line was deleted, or moved away | a new uid; the reference to the deleted line stays `#REF!` | `TRAME-267` |
| one writer received a delete, a move or a sort of the last line, or a line appended below it | two lines, each value on its own | `TRAME-273` |
| one writer received a line derived after the last line and then deleted, the other never received it | two lines; the other writer derives the deleted uid again. Through the uid lookup the spike measured, a `#REF!` to it read a value; the freeze keeps it `#REF!` | `TRAME-273`, `TRAME-274` |
| four peers: one derives the line, a second binds a formula to it, the first deletes it, a fourth that never had it derives the uid again | the formula reads `#REF!`: its element is gone | `TRAME-278` |
| four peers push, delete, move, bind and sync in any order: a proptest, 512 cases | every reference to a live element reads the line, every other one `#REF!` | `TRAME-278` |
| a document of schema version 1 | read unchanged: its lines hold minted uids, none twice | `TRAME-272` |

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
| A move reseeds the engine only when it re-anchored a range | reseeding after every structural operation, as a delete does | a cell reference resolves through the position cache, and a move only permutes it. Re-parsing every formula to reach the same answer is the cost a stable identity exists to avoid |
| A delete purges the removed line's cells | leaving the entries and filtering them at read time | a cell keyed by a deleted identity is unreachable and would ride in the document and every sync payload for ever |
| An insert from any surface goes through one intent | each surface computing a grid position and sending it | the menu, the header affordance and a future keyboard chord then resolve the position in one place, and cannot disagree about what "above" means |
| A line a write adds takes a derived uid | a minted uid, then a merge of the two lines after the fact | every peer that imports the split writes the fold, and the fold rewrites every cell key of one line. A viewer cannot write, so a viewer keeps the split |
| A line a write adds takes a derived uid | a uid the relay assigns | the relay is blind and orders no structure. A peer offline could then write past the used range only after it reconnects |
| A line a write adds takes a derived uid | a uid derived from the grid position | a delete above, then a write, derives the uid of a line still live: two lines become one, and their cells collide |
| A line a write adds takes a derived uid | a uid derived from the set of every uid of the axis | any concurrent insert or delete, anywhere on the axis, changes the set. Most real races then still split the line |
| A line a write adds takes a derived uid | a hash keyed with the document key | the uid never leaves the sealed document, so a key hides nothing. A key rotation would then change the uid of every line not yet written |
| A line a write adds takes a derived uid | a 64-bit non-cryptographic hash spread over 96 bits | 96 bits cut from a 64-bit hash still collide at 64 bits. BLAKE2b is already the workspace hash |
| A delete freezes each reference to its line into a literal `#REF!` | keep the uid lookup, and accept that a repeated uid revives the reference | a peer that never received a deleted line derives its uid again, and every `#REF!` to that line then reads that peer's value. A broken formula would compute again in silence |
| A delete freezes each reference to its line into a literal `#REF!` | freeze a stale reference when a remote delta arrives | the import becomes a write: every peer writes the same rewrite, and a viewer may not write at all |
| The line before carries a generation | derive again from the line before alone | the next write after a deleted line takes its uid back. Every `#REF!` to the deleted line then reads a value again |
| The line before carries a generation | a salt raised while the uid is live in the axis or recorded in a `retired_{sheet}` map of deleted uids | the salt reads the local axis and the local record, so two peers that received different inserts or deletes may derive two uids. The record also grows with every delete, and its pruning waits on every `#REF!` that names a deleted uid. The generation keeps no deleted uid at all |
| The generation is a grow-only key per line | one last-writer-wins value per line | a concurrent write of a lower generation can win. The next derivation then repeats a uid still live after that line, and two lines become one |
| The generation is a grow-only key per line | a Loro counter per line, under a map key | two peers creating the counter at once race on the key, and the losing counter drops its increments. A root counter per line instead fills the document root with one container per line |
| The smallest element id is the line, and no write removes a hidden element | remove the repeat at import time | the import becomes a write: every peer writes the same removal, and a viewer may not write at all |
| The smallest element id is the line, and no write removes a hidden element | the first element is the line, and the next positional write removes the later ones | the removal deletes a live element, so a reference bound to it reads `#REF!`. A move past a hidden element would also make that element the first one |
| The smallest element id is the line, and no write removes a hidden element | remove the later elements, and record which element each one merged into | a second map, chains of records to follow, and a race between a record and a delete. The smallest id needs no state |
| A reference binds to the element | accept that a formula written during a delete reads a value when a fourth peer derives the uid again | a broken formula computes again in silence, in a race no user can see. The binding costs at most 26 bytes on a reference to a derived line |
| A reference binds to the element | bind to the Loro cursor of the element | a `mov` replaces the list item the cursor names, so the cursor fails after one move. Loro keeps a stable element id inside its state, and no public call reads it |
| An insert keeps each range corner on its line | a positional range, which counts the new line with no edit | the block's old last line then falls out of it, and the total changes for an operation that wrote no value. A corner names its line, so the range widens over what is inserted inside it (`TRAME-171`), and two concurrent inserts widen it twice, which is what both users meant |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing reads a metric after a structural operation** (`TRAME-170`, `TRAME-173`). A height, a
  width and a hidden mark are keyed by the line's identity. The argument that they follow the line
  is the argument that the cells do, and the cells have tests. A drag released on its own
  slot writes nothing for the same untested reason.
* **The materialization cap is a branch nothing enters** (`TRAME-089`). Ten thousand lines past the
  used range, the write is dropped with a log line. No test, no event, and nothing tells the user
  their value did not land.
* **Two writers that disagree on the line before still split a new line** (`TRAME-273`). A delete,
  a move or a sort of the last line, or a line appended below it, gives one writer another line
  before. A line derived then deleted, received by one writer alone, gives it another generation.
  The values then sit on two lines, as they all do today.
* **Nothing inserts or deletes more than one line at a time.** There is no "insert three rows", only
  three inserts. The multi-line delete removes its lines in one commit, so it is one undo item and
  one delta (`TRAME-088`, `TRAME-175`).
