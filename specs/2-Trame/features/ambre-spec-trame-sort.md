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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-004` | A sort MUST be a pure permutation of the row order (`rows_{sheet}.mov`); it MUST NOT rewrite a cell. | this spec section 3; `features/ambre-spec-trame-rows-and-columns.md` | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, `group4::sort_range_multi_already_sorted_emits_no_delta`, `group2::move_row_carries_cells_and_formula_follows_data` | verified |
| `TRAME-FEAT-010` | A sort MUST order rows by their **evaluated** value, and MUST pin the header row when one is declared. | this spec section 3 | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, `::sort_range_descending_keeps_header`, `group4::sort_range_multi_pins_header_row` | verified |
| `TRAME-FEAT-011` | A multi-column sort MUST be lexicographic over its keys, each key applying its own direction. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_lexicographic_with_per_key_direction`, `::sort_range_multi_no_keys_is_noop` | verified |
| `TRAME-FEAT-012` | A sort that changes no row order MUST emit no CRDT delta. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_already_sorted_emits_no_delta`, `::sort_range_noops` | verified |
| `TRAME-FEAT-013` | A sort invoked without an explicit range MUST resolve it from the tracked selection, in the Logic thread. | this spec section 4 | `ambre_trame::logic::tests::group4::sort_selection_resolves_against_tracked_selection`, `::sort_selection_multi_resolves_against_tracked_selection`, `::sort_sheet_sorts_used_range_by_active_column` | verified |
| `TRAME-111` | Two values of different types MUST order by the type rank, never interleaved. | this spec section 3 | none. Every sort test seeds one type per column, so the rank branch of `compare_cell_values` is reached by no assertion | unverified |
| `TRAME-112` | A descending sort MUST reverse the whole comparison, so an empty cell orders first. | this spec section 5 | none. `ambre_trame::logic::tests::group3::sort_range_descending_keeps_header` drives a descending sort over four filled cells, and no test sorts a column holding a blank | unverified |
| `TRAME-113` | When two users sort one range at the same moment, every peer MUST end on one row order. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops`, whose operation set carries `SortRange` | verified |
| `TRAME-114` | A sort MUST order the rows a filter hides together with the visible ones. | this spec section 2, row 3; section 5 | none. `ambre_trame::logic::tests::group5::shared_filter_hidden_follows_sort` covers the mark following its row (`TRAME-FEAT-002`), not the hidden row taking part in the ordering | unverified |
| `TRAME-115` | A sort naming a sheet other than the active one MUST change nothing. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_noops` | verified |
| `TRAME-UI-101` | The sort dialog MUST offer exactly the columns the sort will order. | this spec section 4 | none. `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply` opens the dialog over a cell rectangle, which is the case that agrees. **The rule fails on a whole-row selection** -- section 6 | unverified |
| `TRAME-UI-102` | A column repeated over two levels MUST order the rows once, at its first level. | this spec section 4 | `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply` | verified |

* **Budget.** none: a sort is one permutation over the rows a sheet holds, and
  `ambre-spec-budgets.md` budgets no sort. The moves it emits land under the batch
  row of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a sort emits moves into the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 8 `verified`, 4 `unverified`. The permutation and the comparator are held by
  the host tests; what is loose is the comparator's second half -- the type rank and the blank --
  and the two rules that join the sort to another feature: the filter, and the selection the dialog
  reads.

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
`true`. Two errors, and two empty cells, compare equal, so a stable sort leaves them in place. A
number that is not a number compares equal rather than poisoning the order.

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A sort permutes the row order and rewrites no cell | each row carries an identity, so its cells follow the move at no cost, and the compiled formulas stay valid. Rewriting the cells would produce one delta per cell and reseed the whole engine | writing the sorted values back into the cells, which is what a positional model must do |
| A reference follows its row, and a range stays a positional block | a user sorting under a total expects the total to hold, and a user pointing at a cell expects to keep pointing at that cell. The two answers are consistent because they answer different questions | binding range endpoints to row identities, which lets them cross under a sort and invert the rectangle |
| Values of different types order by a fixed rank | a column mixing numbers and text has no natural order, and a rank at least makes the result stable and explainable. Empty cells rank last, which keeps a partly filled column readable | comparing the display text of every value, which orders `10` before `9` |
| A descending sort reverses the whole comparison | one comparator, one direction flag, and no special case to keep in step with the other kinds | Excel's rule, which keeps blanks last in both directions and needs the blank case written twice |
| Concurrent sorts are resolved by last-writer-wins on the row order | a sort is one intent over one list, so electing the whole list gives an order a user actually asked for. Merging two orders would give one nobody asked for | merging the two permutations |
| The single-column sort takes no dialog | the two menu entries are the whole gesture, and a dialog for one column and one direction is two clicks for nothing | one dialog for both sorts |
| A sort orders the hidden rows too | the hidden mark is keyed by row identity, so it travels with its row and needs no recompute after the move | Excel's rule, which leaves filtered rows in place and sorts around them |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The dialog reads a different selection from the one the sort orders** (`TRAME-UI-101`). The
  canvas publishes the selection rectangle, and a whole-row band publishes column zero for both
  corners. So "Sort range..." over selected rows offers column A alone, while the Logic thread
  orders the full used width. A whole-sheet selection is the other end of the same defect: the
  picker lists one option per column of the virtual grid.
* **The type rank and the blank have no test** (`TRAME-111`, `TRAME-112`). Both are pure functions
  of two values, and `compare_cell_values` is reachable from the host tier, so this is the cheapest
  gap in the document to close.
* **A sort orders the rows a filter hides** (`TRAME-114`), which Excel and Sheets do not. The rule
  is written down and the behaviour is deliberate, and no test states either.
* **A sort of a range holding a merge is unspecified.** A covered cell holds no value, so it orders
  as empty, and nothing says what happens to the block.
* **The dialog forgets its levels.** It resets to one ascending level on every open, so a user
  refining a three-level sort rebuilds it each time.
