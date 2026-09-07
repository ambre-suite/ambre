# AMBRE -- FEATURE: SORT

**Type:** Vertical (feature)
**Implementation:** implemented -- both sorts ship; the dialog offers one column on a whole-row selection.

> A user orders a block of rows by one column, or by several in priority order. Whole rows travel,
> so a row keeps its other cells. Every collaborator sees the same order.

A sort orders rows; it never rewrites one. That is why a formula still reads the value it read
before, and why a hidden row stays hidden at its new position.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-113` | `CONC` | When two users sort one range at the same moment, every peer MUST end on one row order. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops`, whose operation set carries `SortRange` | verified |

### 1.2 Order a block of rows

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-004` | `POST` | A sort MUST reorder the rows themselves, so every cell of a row travels with it, inside the range and outside it. | this spec section 3; `features/ambre-spec-trame-rows-and-columns.md` | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, which reads a second column back in the new order, and `group4::sort_sheet_sorts_used_range_by_active_column` for a key column that is not the first | verified |
| `TRAME-178` | `FRAME` | A sort MUST NOT rewrite a cell. | this spec section 3, section 5 | none. Every sort test reads the values back after the move, which a rewrite producing the same result would satisfy just as well. The rule is held by the shape of `realize_sort` -- it emits `mov` alone -- and by no assertion | unverified |
| `TRAME-FEAT-010` | `POST` | A sort MUST order rows by their **evaluated** value. | this spec section 3 | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, `group4::sort_range_sum_block_is_stable_under_internal_sort`, whose key column holds literals a parse turns into numbers | verified |
| `TRAME-179` | `FRAME` | A sort MUST NOT move the header row, when one is declared. | this spec section 3 | `ambre_trame::logic::tests::group3::sort_range_descending_keeps_header`, `group4::sort_range_multi_pins_header_row`, which both read the header back in place | verified |
| `TRAME-FEAT-011` | `POST` | A multi-column sort MUST be lexicographic over its keys, each key applying its own direction. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_lexicographic_with_per_key_direction`, over a tie the second key breaks in the other direction | verified |
| `TRAME-180` | `POST` | Two rows whose keys all compare equal MUST keep the order they had. | this spec section 3 | none. `TRAME-FEAT-012` rests on this -- an already-ordered range only stays put because the sort is stable -- and no test seeds two rows that compare equal on every key | unverified |
| `TRAME-FEAT-012` | `FRAME` | A sort that changes no row order MUST NOT emit a CRDT delta. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_already_sorted_emits_no_delta`, `::sort_range_noops`, which both assert the outcome carries no update | verified |
| `TRAME-111` | `POST` | Two values of different types MUST order by the type rank, never interleaved. | this spec section 3 | none. Every sort test seeds one type per column, so the rank branch of `compare_cell_values` is reached by no assertion | unverified |
| `TRAME-112` | `POST` | A descending sort MUST reverse the whole comparison, so an empty cell orders first. | this spec section 5 | none. `ambre_trame::logic::tests::group3::sort_range_descending_keeps_header` drives a descending sort over four filled cells, and no test sorts a column holding a blank | unverified |
| `TRAME-181` | `POST` | A sort MUST order the range whatever values it holds. | this spec section 3 | none, and the comparator contradicts it. `POWER(-1, 0.5)` and `LOG(-1)` return a `Number` holding NaN under no guard, and `compare_cell_values` answers "equal" to every pair one takes part in: NaN ties with 1 and with 2 while 1 orders before 2. That is not a total order, and the standard sort is entitled to abort on one -- section 6 | unverified |
| `TRAME-114` | `POST` | A sort MUST order the rows a filter hides together with the visible ones. | this spec section 2, row 3; section 5 | none. `ambre_trame::logic::tests::group5::shared_filter_hidden_follows_sort` covers the mark following its row (`TRAME-FEAT-002`), not the hidden row taking part in the ordering | unverified |
| `TRAME-182` | `FRAME` | A sort MUST NOT move a row outside the range it was given. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_sum_block_is_stable_under_internal_sort`, whose two formula rows sit below the sorted block and are read back at their own positions | verified |
| `TRAME-183` | `FRAME` | A sort MUST NOT change which cell is active, nor the selected rectangle. | this spec section 4 | none. `after_reorder` touches neither, so the active cell keeps its **position** and the row that was under it has moved away -- section 6 | unverified |
| `TRAME-FEAT-013` | `POST` | A sort invoked without an explicit range MUST resolve it from the tracked selection, in the Logic thread. | this spec section 4 | `ambre_trame::logic::tests::group4::sort_selection_resolves_against_tracked_selection`, `::sort_selection_multi_resolves_against_tracked_selection`, `::sort_sheet_sorts_used_range_by_active_column` | verified |
| `TRAME-115` | `FAIL` | A sort naming a sheet other than the active one MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_noops`, which asserts the outcome carries neither an update nor a single display event | verified |
| `TRAME-184` | `FAIL` | A sort left with no key inside the range MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_no_keys_is_noop`, over an empty key list and over a key naming a column outside the range | verified |
| `TRAME-185` | `FAIL` | A sort over fewer than two sortable rows MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | none. The guard is written, and no test drives a one-row range or a two-row range whose first row is the header | unverified |

### 1.3 The sort dialog

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-UI-101` | `OBS` | The dialog MUST offer exactly the columns the sort will order. | this spec section 4 | none. `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply` opens the dialog over a cell rectangle, which is the case that agrees. **The rule fails on a whole-row selection** -- section 6 | unverified |
| `TRAME-UI-150` | `INV` | The dialog MUST always hold at least one level, and never more levels than the range has columns. | this spec section 4 | none. Both bounds are carried by a disabled button, and the test clicks "Add sort level" once over a three-column range, which reaches neither bound | unverified |

### 1.4 Apply the levels, or give up

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-UI-151` | `PRE` | The selection MUST NOT move while the dialog is open. | this spec section 4 | none. The columns the dialog offers are a snapshot taken when it opened, and the range the sort orders is the selection the Logic thread tracks when it applies. Nothing holds the two together -- section 6 | unverified |
| `TRAME-UI-152` | `POST` | Applying MUST emit one key per level the user kept, in the order they are listed, carrying the header toggle as it stands. | this spec section 4 | `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply`, which adds a level, toggles the header and reads both back off the emitted request | verified |
| `TRAME-UI-102` | `POST` | A column repeated over two levels MUST order the rows once, at its first level. | this spec section 4 | `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply`, which asserts the two emitted keys name different columns | verified |
| `TRAME-UI-153` | `FRAME` | Cancelling the dialog, clicking its backdrop or closing it MUST NOT order anything. | this spec section 4 | none. The three paths run one `close`, which sets the open state to `None` and calls nothing else; no test clicks any of them | unverified |

* **Budget.** none: a sort is one permutation over the rows a sheet holds, and
  `ambre-spec-budgets.md` budgets no sort. The moves it emits land under the batch
  row of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a sort emits moves into the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 12 `verified`, 12 `unverified`. The permutation is held, in the shape the user
  cares about: whole rows travel, the header stays, an already-ordered range writes nothing, and a
  row outside the range does not move. What is loose is everything the comparator decides. The type
  rank and the blank were already known gaps; the pass adds a third, which is not a gap but a
  defect: `POWER(-1, 0.5)` puts a NaN in a cell, the comparator ties it with every number, and the
  sort loses the total order it needs. The counts moved 8/4 to 12/12 over one new mechanism --
  `sort_range_sum_block_is_stable_under_internal_sort`, which held nothing before. The other four
  new `verified` rows cite assertions that were already in the cited tests but were reached by no
  rule: the header read back in place, the refusal that carries no acknowledgement, the two emitted
  keys. That is the split paying for itself, not coverage appearing.

---

## 2. WHAT EACH USER SEES

Two peers, one sheet. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sorts a range ascending | sorts the same range descending | one row order, elected by last-writer-wins | the same order, on both screens | `logic::convergence_tests::two_peers_converge_under_interleaved_ops` |
| sorts a range | writes into a row of it | the text on its own row, wherever the sort left it | the value travels with its row (`TRAME-004`) | the same property, whose operation set carries `SortRange` and `SetCellText` |
| filters column A | sorts the same range | the filter and the new row order | the same rows hidden, at their new positions | `logic::tests::group5::shared_filter_hidden_follows_sort` |
| sorts a range under a total | reads the total | the permuted rows | the same total: a range is a positional block (`TRAME-005`) | `logic::tests::group4::sort_range_sum_block_is_stable_under_internal_sort` |
| sorts a range | deletes a row of it | the remaining rows, ordered | the deleted row gone, the order kept | the same property, whose operation set carries `SortRange` and `DeleteRow` |

---

## 3. MODEL

**Containers.** None of its own. A sort moves elements of `rows_{sheet}`, declared in
`ambre-spec-trame-schema.md section 3` and owned by
`features/ambre-spec-trame-rows-and-columns.md`.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::SortRange { sheet_id, row_start, row_end, col_start, col_end, sort_col, ascending, has_header }` | UI to Logic | the explicit range and one key |
| `TrameCommand::SortRangeMulti { sheet_id, row_start, row_end, col_start, col_end, sort_keys, has_header }` | UI to Logic | the explicit range and the ordered keys |
| `TrameCommand::SortSelectionAsc` / `SortSelectionDesc` | UI to Logic | nothing: the range is the tracked selection, the key its first column |
| `TrameCommand::SortSheetAsc` / `SortSheetDesc` | UI to Logic | nothing: the range is the used one, the key the active column |
| `TrameCommand::SortSelectionMulti { sort_keys, has_header }` | UI to Logic | the keys alone, resolved against the tracked selection |
| `TrameEvent::SortComplete` | Logic to UI | the acknowledgement, paired with `DataReset` when the order changed |

**Key shape**, typed in `ambre-bus`:

```rust
pub struct SortKey {
    pub col:       u32,   // index within the range: 0 is its first column
    pub ascending: bool,
}
```

**The type rank**, applied when two values differ in type (`TRAME-111`):

| Rank | Value |
|---|---|
| 0 | a number |
| 1 | text |
| 2 | a boolean |
| 3 | an error |
| 4 | an empty cell |

Two values of one type compare naturally: numbers numerically, text by code point, `false` before
`true`. Two errors, and two empty cells, compare equal, so a stable sort leaves them in place
(`TRAME-180`). A number that is not a number compares equal to every other number, which is not an
order at all (`TRAME-181`, section 6).

**Steps, when a user sorts:**

1. reject a sheet that is not the active one, because only its values are positioned in the
   formula cache (`TRAME-115`);
2. clamp the range to the materialized rows, and drop a key whose column falls outside it;
3. pin the header row when one is declared, so the first sortable position is the one after it;
4. read each sortable row's evaluated value, and pair it with the row identity;
5. order the pairs by the comparator above, reversing it whole for a descending sort;
6. stop here when the order is unchanged, and acknowledge with no delta (`TRAME-FEAT-012`);
7. replay the order as one `mov` per displaced row, then commit once;
8. rebuild the position caches, keep the compiled formulas, and recompute.

Step 8 is what makes a reference follow its row: the bytecode is anchored to identities, so
preserving it and recomputing resolves every reference to its new position (`TRAME-005`).

**Code:**

* `crates/ambre-trame/src/logic/sort.rs` -- `sort_range`, `sort_range_multi`, `realize_sort`,
  `after_reorder`;
* `logic/helpers.rs` -- `compare_cell_values`, the comparator;
* `logic/structural.rs` -- `sort_selection`, `sort_sheet`, `sort_selection_multi`;
* `chrome/sort_dialog.rs` -- the dialog;
* `chrome/commands.rs` -- the context-menu entries.

---

## 4. SURFACE

Everything the user touches. The menu that carries the entries is specified in
`features/ambre-spec-trame-context-menu.md`.

**Triggers.**

| Trigger | Sends | Orders |
|---|---|---|
| context menu on a cell, "Sort ascending" or "Sort descending" | `SortSelectionAsc` / `SortSelectionDesc` | the tracked selection, by its first column |
| context menu on a column header | the same two | the same range: a whole-column selection reaches Logic as a rectangle |
| context menu on the corner | `SortSheetAsc` / `SortSheetDesc` | every materialized row, by the active column |
| context menu, "Sort range..." | opens the dialog | nothing yet: the dialog emits `SortSelectionMulti` on apply |

**The dialog**, a DOM component opened over the current selection:

| Part | Behaviour |
|---|---|
| a level | a column picker and a direction, listed in decreasing priority |
| "Add sort level" | appends a level on the first column no level uses; disabled once every column is used |
| "Remove sort level" | disabled on the last remaining level: a sort needs one key |
| "Data has header row" | pins the range's first row, and is off each time the dialog opens |
| the column picker | offers the columns of the selection the dialog was opened over (`TRAME-UI-101`) |
| "Sort" | drops a column a higher level already uses, then emits the levels (`TRAME-UI-102`) |
| the backdrop, "Cancel", the close button | close the dialog and emit nothing |

**What the user sees afterwards.** `SortComplete` acknowledges the command; the paired `DataReset`
makes the viewport re-fetch, so the grid repaints in the new order with the same values.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A sort permutes the row order and rewrites no cell | writing the sorted values back into the cells | it is what a positional model must do, and it produces one delta per cell and reseeds the whole engine. Each row here carries an identity, so its cells follow the move at no cost and the compiled formulas stay valid |
| A reference follows its row, and a range stays a positional block | binding range endpoints to row identities | the endpoints then cross under a sort and invert the rectangle. A user sorting under a total expects the total to hold, and a user pointing at a cell expects to keep pointing at that cell: the two answers differ because they answer different questions |
| Values of different types order by a fixed rank | comparing the display text of every value | it orders `10` before `9`. A column mixing numbers and text has no natural order, and a rank at least makes the result stable and explainable; empty cells rank last, which keeps a partly filled column readable |
| A descending sort reverses the whole comparison | Excel's rule, keeping blanks last in both directions | the blank case then has to be written twice and kept in step with the other kinds; reversing the comparison is one comparator and one direction flag |
| Concurrent sorts are resolved by last-writer-wins on the row order | merging the two permutations | it gives an order nobody asked for. A sort is one intent over one list, so electing the whole list at least yields an order a user did ask for |
| The single-column sort takes no dialog | one dialog for both sorts | the two menu entries are the whole gesture, and a dialog for one column and one direction is two clicks for nothing |
| A sort orders the hidden rows too | Excel's rule, leaving filtered rows in place and sorting around them | the hidden mark is keyed by row identity, so it travels with its row and needs no recompute after the move; sorting around the gaps would need one |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The dialog reads a different selection from the one the sort orders** (`TRAME-UI-101`). The
  canvas publishes the selection rectangle, and a whole-row band publishes column zero for both
  corners. So "Sort range..." over selected rows offers column A alone, while the Logic thread
  orders the full used width. A whole-sheet selection is the other end of the same defect: the
  picker lists one option per column of the virtual grid.
* **The comparator is not a total order** (`TRAME-181`). `POWER(-1, 0.5)` and `LOG(-1)` build a
  `Number` holding NaN, and the comparator answers "equal" to every pair a NaN takes part in. So
  NaN ties with 1, NaN ties with 2, and 1 orders before 2: the relation is not transitive. The
  standard sort detects an inconsistent comparator and aborts, which in the Logic thread costs the
  document. Nothing rules whether a NaN should be an error at the source -- the two builtins that
  produce one are the only ones with no finiteness guard -- or a rank of its own in the comparator.
* **The type rank, the blank and the stability have no test** (`TRAME-111`, `TRAME-112`,
  `TRAME-180`). All three are pure functions of two values, and `compare_cell_values` is reachable
  from the host tier, so this is the cheapest gap in the document to close. The stability is the one
  that carries weight elsewhere: `TRAME-FEAT-012` only holds because an ordered range sorts to
  itself.
* **A refused sort says nothing** (`TRAME-115`, `TRAME-184`, `TRAME-185`). `SortComplete` raises a
  "Range sorted" toast, and the three refusals emit no event at all. A user who sorts a single-row
  selection, or a range on a sheet the UI has not activated, sees no order change and no notice, and
  cannot tell the refusal from a command that was lost.
* **A sort leaves the active cell where it was** (`TRAME-183`). The position is kept, so the row
  under it is not: the cell the user had selected now holds another row's value, and a selection
  drawn over the sorted block covers different rows than the ones it covered. Nothing says whether
  the selection should follow the rows it was drawn over.
* **The dialog reads a snapshot, the sort reads the live selection** (`TRAME-UI-151`). The picker
  offers the columns of the rectangle captured when "Sort range..." opened; applying sends
  `SortSelectionMulti`, whose range is whatever the Logic thread tracks at that moment. The two
  agree today because the backdrop swallows the clicks that would move the selection, which is a
  property of the stylesheet, not of the contract.
* **A sort orders the rows a filter hides** (`TRAME-114`), which Excel and Sheets do not. The rule
  is written down and the behaviour is deliberate, and no test states either.
* **A sort of a range holding a merge is unspecified.** A covered cell holds no value, so it orders
  as empty, and nothing says what happens to the block.
* **The dialog forgets its levels.** It resets to one ascending level on every open, so a user
  refining a three-level sort rebuilds it each time.
