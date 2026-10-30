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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-113` | `CONC` | When two users sort one range at the same moment, every peer MUST end on one row order. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest whose operation set carries `SortRange` | none | verified |

### 1.2 Order a block of rows

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-004` | `POST` | A sort MUST reorder the rows themselves, so every cell of a row travels with it, inside the range and outside it. | this spec section 3; `features/ambre-spec-trame-rows-and-columns.md` | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows` -- it reads a second column back in the new order; `ambre_trame::logic::tests::group4::sort_sheet_sorts_used_range_by_active_column` -- a key column that is not the first | none | verified |
| `TRAME-178` | `FRAME` | A sort MUST NOT rewrite a cell. | this spec section 3, section 5 | `ambre_trame::logic::tests::group13::a_sort_rewrites_no_cell` -- the cells map is the same before and after, while the order changed | none | verified |
| `TRAME-FEAT-010` | `POST` | A sort MUST order rows by their **evaluated** value. | this spec section 3 | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`; `ambre_trame::logic::tests::group4::sort_range_sum_block_is_stable_under_internal_sort` -- their key column holds literals a parse turns into numbers | none | verified |
| `TRAME-179` | `FRAME` | A sort MUST NOT move the header row, when one is declared. | this spec section 3 | `ambre_trame::logic::tests::group3::sort_range_descending_keeps_header`; `ambre_trame::logic::tests::group4::sort_range_multi_pins_header_row` -- both read the header back in place | none | verified |
| `TRAME-FEAT-011` | `POST` | A multi-column sort MUST be lexicographic over its keys, each key applying its own direction. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_lexicographic_with_per_key_direction` -- over a tie the second key breaks in the other direction | none | verified |
| `TRAME-180` | `POST` | Two rows whose keys all compare equal MUST keep the order they had. | this spec section 3 | `ambre_trame::logic::tests::group13::equal_keys_keep_their_order` -- ascending and descending; `::a_sort_orders_text_whatever_its_case` -- two texts that differ in case alone | none | verified |
| `TRAME-255` | `POST` | A sort MUST order two texts whatever their case. | this spec section 3 | `ambre_trame::logic::tests::group13::a_sort_orders_text_whatever_its_case` -- two texts that differ in case alone keep their order; `ambre_trame::logic::helpers::tests::comparator_is_a_total_order_with_nan` -- over texts that differ in case | none | verified |
| `TRAME-FEAT-012` | `FRAME` | A sort that changes no row order MUST NOT emit a CRDT delta. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_already_sorted_emits_no_delta`; `::sort_range_noops` -- both assert the outcome carries no update | none | verified |
| `TRAME-111` | `POST` | Two values of different types MUST order by the type rank, never interleaved. | this spec section 3 | `ambre_trame::logic::tests::group13::mixed_types_order_by_rank` | none | verified |
| `TRAME-112` | `POST` | A descending sort MUST reverse the whole comparison, so an empty cell orders first. | this spec section 5 | `ambre_trame::logic::tests::group13::a_descending_sort_puts_the_blank_first` | none | verified |
| `TRAME-181` | `POST` | A sort MUST order the range whatever values it holds. | this spec section 3 | `ambre_trame::logic::helpers::tests::comparator_is_a_total_order_with_nan`; `ambre_trame::formula::functions::tests::power_without_a_real_root_is_a_num_error` | none | verified |
| `TRAME-114` | `POST` | A sort MUST order the rows a filter hides together with the visible ones. | this spec section 2, row 3; section 5 | `ambre_trame::logic::tests::group13::a_hidden_row_is_ordered_too` | none | verified |
| `TRAME-182` | `FRAME` | A sort MUST NOT move a row outside the range it was given. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_sum_block_is_stable_under_internal_sort` -- its two formula rows sit below the sorted block and are read back at their own positions | none | verified |
| `TRAME-183` | `FRAME` | A sort MUST leave the active cell and the selected rectangle at their positions. | this spec section 4 | `ambre_trame::logic::tests::group4::sort_keeps_active_cell_and_selection_in_place` | none | verified |
| `TRAME-FEAT-013` | `POST` | A sort invoked without an explicit range MUST resolve it from the tracked selection, in the Logic thread. | this spec section 4 | `ambre_trame::logic::tests::group4::sort_selection_resolves_against_tracked_selection`; `::sort_selection_multi_resolves_against_tracked_selection`; `::sort_sheet_sorts_used_range_by_active_column` | none | verified |
| `TRAME-115` | `FAIL` | A sort naming a sheet other than the active one MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_noops` -- it asserts the outcome carries neither an update nor a single display event | none | verified |
| `TRAME-184` | `FAIL` | A sort left with no key inside the range MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | `ambre_trame::logic::tests::group4::sort_range_multi_no_keys_is_noop` -- it runs over an empty key list and over a key naming a column outside the range | none | verified |
| `TRAME-185` | `FAIL` | A sort over fewer than two sortable rows MUST change nothing, and MUST NOT be acknowledged. | this spec section 3 | `ambre_trame::logic::tests::group13::a_sort_of_fewer_than_two_rows_changes_nothing` | none | verified |

### 1.3 The sort dialog

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-101` | `OBS` | The dialog MUST offer exactly the columns the sort will order. | this spec section 4 | `ambre_trame::tests::chrome_ui::the_sort_reply_opens_the_dialog_over_its_columns`; `ambre_trame::logic::tests::group4::sort_range_query_spans_the_used_width_of_a_row_band` | none | verified |
| `TRAME-UI-150` | `INV` | The dialog MUST always hold at least one level, and never more levels than the range has columns. | this spec section 4 | `ambre_trame::tests::chrome_ui::the_dialog_holds_one_level_to_one_per_column` | none | verified |

### 1.4 Apply the levels, or give up

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-151` | `PRE` | The selection MUST NOT move while the dialog is open. | this spec section 4 | `ambre_trame::tests::chrome_ui::the_open_sort_dialog_is_modal` -- the sheet takes no pointer; `ambre_app::host::tests::a_key_inside_an_open_dialog_stays_there` -- the grid takes no key | none | verified |
| `TRAME-UI-152` | `POST` | Applying MUST emit one key per level the user kept, in the order they are listed, carrying the header toggle as it stands. | this spec section 4 | `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply` -- it adds a level, toggles the header and reads both back off the emitted request | none | verified |
| `TRAME-UI-102` | `POST` | A column repeated over two levels MUST order the rows once, at its first level. | this spec section 4 | `ambre_trame::tests::chrome_ui::sort_dialog_adds_levels_and_emits_on_apply` -- it asserts the two emitted keys name different columns | none | verified |
| `TRAME-UI-153` | `FRAME` | Cancelling the dialog, clicking its backdrop or closing it MUST NOT order anything. | this spec section 4 | `ambre_trame::tests::chrome_ui::closing_the_sort_dialog_sorts_nothing` -- cancel, the close button and the backdrop | none | verified |

* **Budget.** none: a sort is one permutation over the rows a sheet holds, and
  `ambre-spec-budgets.md` budgets no sort. The moves it emits land under the batch
  row of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a sort emits moves into the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 25 `verified`. What holds is the permutation, in the shape the user cares about:
  whole rows travel, the header stays, an already-ordered range writes nothing. A row outside the
  range stays put, and a refusal carries no acknowledgement. The comparator is held whole: the type
  rank, the blank, the stability and a NaN. The dialog is held in a browser: its bounds, its close
  paths, and its modality.

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
| `TrameCommand::SortRange { sheet_id, rect, sort_col, order, header }` | UI to Logic | the explicit range and one key |
| `TrameCommand::SortRangeMulti { sheet_id, rect, sort_keys, header }` | UI to Logic | the explicit range and the ordered keys |
| `TrameCommand::SortSelectionAsc` / `SortSelectionDesc` | UI to Logic | nothing: the range is the tracked selection, the key its first column |
| `TrameCommand::SortSheetAsc` / `SortSheetDesc` | UI to Logic | nothing: the range is the used one, the key the active column |
| `TrameCommand::SortSelectionMulti { sort_keys, has_header }` | UI to Logic | the keys alone, resolved against the tracked selection |
| `TrameEvent::SortComplete` | Logic to UI | the acknowledgement, paired with `DataReset` when the order changed |

**Key shape**, typed in `ambre-bus`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub col:   ColCount,   // offset within the range: 0 is its first column
    pub order: SortOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder { #[default] Ascending, Descending }
```

**The type rank**, applied when two values differ in type (`TRAME-111`):

| Rank | Value |
|---|---|
| 0 | a number |
| 1 | text |
| 2 | a boolean |
| 3 | an error |
| 4 | an empty cell |

Two values of one type compare naturally: numbers numerically, `false` before `true`. Text
compares whatever its case, through the fold the formula operators read, as Excel sorts it
(`TRAME-255`). Two errors, two empty cells, and two texts that differ in case alone compare
equal, so a stable sort leaves them in place (`TRAME-180`).

A number that is not a number ranks with the errors, so the comparator stays a total order
whatever a cell holds (`TRAME-181`). No source builds one today. A function or an operator
whose result is not finite answers `#NUM!` (`TRAME-244`). Text that spells one stays text
(`TRAME-245`).

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

Step 8 is what makes a reference follow its row. The bytecode is anchored to identities, so
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
| the column picker | offers the columns of the range the Logic thread resolves from the selection, the one the sort orders; a row band spans the used width (`TRAME-UI-101`) |
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
| A sort leaves the active cell and the selection at their positions (`TRAME-183`) | the selection following the rows it was drawn over | it is what Excel and Sheets do, and it is the one reading that holds for a lone cell: a selected cell following its row is a selection that moved. The sorted block is the same rectangle before and after |
| A NaN ranks with the errors, and `POWER` refuses to build one (`TRAME-181`) | ruling one of the two alone | the source guards keep a NaN out of a formula result and out of a cell's text (`TRAME-244`, `TRAME-245`), and the rank keeps the comparator total should a later source make one |
| A sort orders the hidden rows too | Excel's rule, leaving filtered rows in place and sorting around them | the hidden mark is keyed by row identity, so it travels with its row and needs no recompute after the move; sorting around the gaps would need one |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A refused sort says nothing** (`TRAME-115`, `TRAME-184`, `TRAME-185`). `SortComplete` raises a
  "Range sorted" toast, and the three refusals emit no event at all. A user who sorts a single-row
  selection, or a range on a sheet the UI has not activated, sees nothing change. No order change,
  no notice, and no way to tell the refusal from a command that was lost.
* **The dialog reads a snapshot, the sort reads the live selection** (`TRAME-UI-151`). The picker
  offers the columns of the rectangle captured when "Sort range..." opened. Applying sends
  `SortSelectionMulti`, whose range is whatever the Logic thread tracks at that moment. The two
  agree because the modal leaves the sheet inert, and the shell's key listener leaves a key aimed
  inside a dialog alone. A command that carried its range would hold the rule by construction.
* **A sort of a range holding a merge is unspecified.** A covered cell holds no value, so it orders
  as empty, and nothing says what happens to the block.
* **The dialog forgets its levels.** It resets to one ascending level on every open, so a user
  refining a three-level sort rebuilds it each time.
