# AMBRE -- FEATURE: SHARED FILTER
**Type:** Vertical (feature)
**Implementation:** partial -- the shared filter ships; the personal views over it do not.

> A user hides the rows of a column that fail a test. Every collaborator sees the same hidden rows.
> A hidden row keeps its value and still counts in every formula: a filter is a view over the data,
> never a change to it.

The personal views over it are the same predicate for one user alone. They are not built.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-FEAT-002` | `INV` | A hidden row MUST stay hidden when the data moves: a sort or an insert MUST NOT transfer the hidden mark to another row. | this spec section 3 | `ambre_trame::logic::tests::group5::shared_filter_hidden_follows_sort` | verified |
| `TRAME-FEAT-003` | `INV` | A hidden row MUST still count in every formula: a filter changes what a user sees, never what the sheet holds. | this spec section 2, row 5 | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` | verified |
| `TRAME-FEAT-005` | `INV` | Every collaborator MUST see the same hidden rows as the user who applied the filter. | this spec section 2, row 3 | `ambre_trame::logic::tests::group5::shared_filter_applies_on_remote_import` | verified |
| `TRAME-FEAT-031` | `INV` | The hidden rows MUST agree with the data: an edit to a filtered column MUST re-test that row. | this spec section 2, row 1 | `ambre_trame::logic::tests::group5::shared_filter_recomputes_on_edit` drives the condition kind alone. **The values kind fails this rule today** -- the stored list reads back empty, so the first edit after a filter by values hides every row (section 6) | unverified |
| `TRAME-FEAT-030` | `INV` | A filter MUST NOT outlive the column it tests. | this spec section 3 | none, and the code does the opposite: `write_filter_hidden` returns 0 when the column uid no longer resolves, having already emptied the hidden set, while `filters_{sheet}` keeps the definition -- so the funnel stays lit on a column that is gone and no row is hidden | unverified |
| `TRAME-FEAT-006` | `CONC` | When two users filter one sheet at the same moment, every peer MUST end on one filter and one hidden set. | this spec section 2, row 4 | `ambre_trame::logic::tests::group5::concurrent_filters_converge_lww`; the 2-peer convergence proptest drives `ApplyFilter` and `ClearFilter` and asserts the elected hidden set (`logic::convergence_tests::two_peers_converge_under_interleaved_ops`) | verified |
| `TRAME-FEAT-025` | `CONC` | A filter that wins a concurrent race MUST be one a user applied, whole: no peer MUST see one user's column joined to another user's operator or operand. | this spec section 2, row 7; section 6 | none. `concurrent_filters_converge_lww` asserts that the two peers agree, not that the winner is intact, so a mixed definition passes it. **The rule fails today** -- section 6. Answered by the `A filter converges as one definition` Story (`ambre-roadmap.md` E7) | unverified |
| `TRAME-UI-008` | `OBS` | A hidden row MUST occupy no height. | this spec section 4 | `ambre_trame::render::tests::cell_rect_collapses_over_hidden_rows` reads the rectangle of a cell below a hidden row and finds it risen by that row's height | verified |
| `TRAME-UI-173` | `OBS` | A click MUST land on the row the user sees, never on the one the collapse moved out of the way. | this spec section 4 | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav` hit-tests a point over the collapsed range and reads back the visible row | verified |
| `TRAME-UI-154` | `OBS` | Arrow-key navigation MUST step over a hidden row. | this spec section 4 | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav` (the `nav` half of the same case) | verified |
| `TRAME-FEAT-027` | `OBS` | Every column of the filtered range MUST carry a button on the range's first row. | this spec section 4 | `ambre_trame::ui::renderer::tests::autofilter_buttons_drawn_on_region_header_row`, `ambre_trame::ui::grid::tests::group1::filter_button_hit_test_matches_drawn_rect`, `::filter_button_scales_with_zoom` | verified |
| `TRAME-FEAT-043` | `OBS` | The button MUST show whether a filter is active on its column. | this spec section 4 | `ambre_trame::ui::renderer::tests::autofilter_button_shows_funnel_on_filtered_column` | verified |
| `TRAME-FEAT-046` | `OBS` | The user MUST be told how many rows a filter hid. | this spec section 4 | none. `chrome::commands` turns `FilterApplied` into an info toast; no test reads a toast, so the count is verified up to the event and no further | unverified |
| `TRAME-FEAT-029` | `OBS` | What the status bar totals MUST be what a formula over the same cells totals. | this spec section 4 | none, and the two disagree: `selection_stats` skips every row in `hidden_rows`, while `TRAME-FEAT-003` keeps those rows in `SUM`. One column then reads two different totals, one in the bar and one in a cell, with nothing to explain the gap | unverified |
| `TRAME-FEAT-004` | `IO` | A filter MUST read back from the document as the filter the user applied. | this spec section 2, row 1 | none, and the values kind fails it: `persist_filter_def` writes the list as a child container, `load_active_filter` reads the map's flat value where that child is a handle, and falls back to an empty list -- section 6 | unverified |
| `TRAME-FEAT-009` | `IO` | A filter MUST hide the same rows after a reload as before it, for every operator it offers. | this spec section 3 | `ambre_trame::logic::tests::group5::condition_pass_covers_every_operator` and `::filter_pass_values_and_op_string_roundtrip` exercise the predicate and the operator spelling in memory; neither reads a definition back from the document, which is the half the rule is about | unverified |

### 1.2 Filter a column

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-FEAT-034` | `PRE` | The rows MUST be tested on their computed value, never on their text. | this spec section 3 | none. `apply_filter` runs `compute_all_dirty` before the pass, so the order holds today; no test filters a column of formulas, which is the only case that would tell the two apart | unverified |
| `TRAME-FEAT-032` | `POST` | Applying a filter MUST replace the previous one, never add to it. | this spec section 3 | none. `write_filter_hidden` and `persist_filter_def` each clear their map first, and no test applies a second filter over a first | unverified |
| `TRAME-FEAT-045` | `POST` | Applying a filter MUST hide every row that fails the test and no other. | this spec section 3 | `ambre_trame::logic::tests::group4::shared_filter_by_values_hides_nonmatching`, `::shared_filter_condition_greater_than` assert the hidden mark row by row, one per filter kind | verified |
| `TRAME-FEAT-033` | `POST` | Applying a filter MUST report the number of rows it hid. | this spec section 4 | `ambre_trame::logic::tests::group4::shared_filter_by_values_hides_nonmatching`, `::shared_filter_condition_greater_than` both match `FilterApplied { hidden_row_count }` against the rows they hid | verified |
| `TRAME-FEAT-007` | `POST` | An undo MUST remove a filter in one step, like any other change to the sheet. | this spec section 2, row 6 | `ambre_trame::logic::tests::group5::shared_filter_is_undoable`, `ambre_trame::logic::tests::group2::filtered_column_edit_is_one_undo_item` | verified |
| `TRAME-FEAT-035` | `FRAME` | Applying a filter MUST NOT change any cell value. | this spec section 5 | partial: `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` shows the filtered column's total unchanged, which observes that column alone. Nothing watches the rest of the sheet | unverified |
| `TRAME-FEAT-036` | `FRAME` | An edit outside the filtered column MUST NOT rewrite the hidden set. | this spec section 3 | none, and the code does the opposite: `finish_cell_writes` calls `recompute_active_filter` after every cell write, whatever its column, and that call clears and rewrites `filter_hidden_{sheet}` whole -- so each keystroke on a filtered sheet broadcasts one delta per hidden row and grows the history for an unchanged result | unverified |
| `TRAME-FEAT-008` | `FAIL` | A filter that cannot apply MUST leave the sheet unchanged. | this spec section 4 | `ambre_trame::logic::tests::group5::shared_filter_noop_guards` (wrong sheet, virtual column), `::clear_filter_unhides_and_noops_when_absent` | verified |
| `TRAME-FEAT-044` | `FAIL` | A filter that cannot apply MUST reach no collaborator. | this spec section 4 | `ambre_trame::logic::tests::group5::shared_filter_noop_guards` asserts `update.is_none()` on each guard | verified |
| `TRAME-FEAT-037` | `FAIL` | A filter that cannot apply MUST say so. | this spec section 4 | none, and the code does the opposite: each guard returns `display_only(vec![])`, so the popover closes on a column that was never filtered and the user is told nothing | unverified |

### 1.3 Clear the filter

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-FEAT-038` | `POST` | Clearing MUST remove both the definition and the hidden rows. | this spec section 3 | `ambre_trame::logic::tests::group5::clear_filter_unhides_and_noops_when_absent` (the `unhides` half) | verified |
| `TRAME-FEAT-039` | `FRAME` | Clearing MUST NOT change any cell value. | this spec section 5 | none. `clear_filter` empties the two filter maps and touches no cell map; nothing asserts it | unverified |
| `TRAME-FEAT-040` | `FAIL` | Clearing with no filter active MUST reach no collaborator. | this spec section 4 | `ambre_trame::logic::tests::group5::clear_filter_unhides_and_noops_when_absent` (the `noops_when_absent` half), `::shared_filter_noop_guards` | verified |

### 1.4 List the values of a column

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-FEAT-026` | `POST` | The values offered for a column MUST hold each distinct visible value once. | this spec section 4 | `ambre_trame::logic::tests::group4::active_column_values_are_distinct_sorted_and_visible` | verified |
| `TRAME-FEAT-041` | `POST` | The values MUST be ordered as the column reads them, not as text. | this spec section 4 | none, and the code does the opposite: `active_column_values` sorts the display strings, so a column of numbers lists `10` before `9`. The test asserts the sort the code performs, not the order a user expects | unverified |
| `TRAME-FEAT-042` | `FRAME` | Listing the values MUST reach no collaborator. | this spec section 3 | type-level: the handler returns `TrameOutcome::display_only`, which builds `update: None` -- a read-only query cannot carry a delta | verified |

### 1.5 Keep a filter of one's own

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-FEAT-015` | `POST` | Activating a personal filter view MUST recompute its hidden rows against the current document, never trust the stored set. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-014` | `FRAME` | A personal filter view MUST NOT emit any CRDT delta: it is local state, persisted to IndexedDB alone. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-016` | `FRAME` | Activating a personal filter view MUST NOT be an undo item. | this spec section 3 | not built -- lands with the "Data tools" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none: the filter is one predicate pass over the rows a sheet holds, and
  `ambre-spec-budgets.md` budgets no filter.
* **Boundary.** none of its own: the filter is shared, so its definition is a document write, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 18 `verified`, 14 `unverified`, 3 `unimplemented`. The rows went 15 to 35 and
  the mechanisms they cite went 21 to 23: the split manufactured no coverage. Every new `verified`
  is one half of a conjunction the tests already carried whole -- `collapse_geometry_and_nav`,
  `unhides_and_noops_when_absent`, `noop_guards` each name their two halves. The `unverified` rows
  are where the split found the other half: four of them state a rule the code contradicts today,
  and section 6 states each once.

---

## 2. WHAT EACH USER SEES

Two peers, one sheet. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| filters column A to show `a` alone | writes `a` into a row Alice's filter hides | the filter and Bob's text | the row returns, for both users | `logic::tests::group5::shared_filter_recomputes_on_edit` |
| filters column A | sorts the same range | the filter and the new row order | the same rows hidden, at their new positions | `logic::tests::group5::shared_filter_hidden_follows_sort` |
| filters column A | opens the sheet after her | one filter, one hidden set | the same collapsed grid; Bob's client applies the set, it does not re-test the rows | `logic::tests::group5::shared_filter_applies_on_remote_import` |
| filters column A on "greater than 1" | filters column A on "less than 3" at the same moment | one operator and one operand, each elected on its own | the same rows hidden for both, from a test that may join one user's operator to the other's operand (`TRAME-FEAT-025`) | `logic::tests::group5::concurrent_filters_converge_lww` |
| filters column A | sums column A | the filter alone | a total over every row, hidden ones included | `logic::tests::group5::shared_filter_does_not_affect_sum` |
| filters column A, then undoes | -- | no filter | every row visible again, for both | `logic::tests::group5::shared_filter_is_undoable` |
| filters column A by values | filters column C by condition at the same moment | all five keys at once, from both users | every row hidden, on both peers: a filter neither user applied | none -- see section 6 |

---

## 3. MODEL

**Containers**, both declared in `ambre-spec-trame-schema.md section 3`:

* `filters_{sheet}` holds the definition over five keys: the column, the kind, and the operator,
  the operand or the value list the kind selects. An empty map is no filter.
* `filter_hidden_{sheet}` holds one entry per hidden row, keyed by the row identity, so the mark
  follows its row through a sort, an insert or a delete (`TRAME-FEAT-002`).

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ApplyFilter { sheet_id, col, kind: FilterKind }` | UI to Logic | the target column and the definition |
| `TrameCommand::ClearFilter { sheet_id }` | UI to Logic | the sheet |
| `CommandId::FilterColumn` / `ClearFilter` | chrome to Logic | no geometry: the Logic thread resolves the column against the active cell it tracks |
| `TrameCommand::ApplyFilterToActiveColumn { kind }` / `ClearActiveFilter` | UI to Logic | the definition alone, resolved against the tracked selection |
| `TrameCommand::RequestActiveColumnValues` | UI to Logic | nothing: a read-only query for the value list |
| `TrameEvent::ColumnValues { col, values }` | Logic to UI | the distinct visible values (`TRAME-FEAT-026`) |
| `TrameEvent::FilterApplied { hidden_row_count, col }` | Logic to UI | the count for the notice, the column for the button glyph |
| `TrameEvent::FilterCleared` | Logic to UI | the clearing, surfaced as a notice |

**Definition shape**, typed in `ambre-bus` and stored under the keys above:

* `FilterKind::Values(Vec<String>)` -- the listed values are the visible ones;
* `FilterKind::Condition { op: FilterOp, value }` -- one operator over one operand;
* `FilterOp` covers `eq`, `ne`, `gt`, `lt`, `gte`, `lte`, `contains`, `not_contains`,
  `starts_with`, `is_empty` and `is_not_empty` (`TRAME-FEAT-009`).

**Engine state.** One field of `TrameState`, and its mirror on the UI side:

```rust
/// Grid positions of the rows the active filter hides. Derived from
/// `filter_hidden_{sheet}` on every filter or structural change, never persisted.
hidden_rows: HashSet<u32>,
```

**Display shape.** A hidden row rides the viewport flagged, never dropped from it:

```rust
DisplayCell { is_filtered: true, /* the value and the style are unchanged */ }
```

**Steps, when a user applies a filter:**

1. resolve the column; a column with no data is a silent no-op (`TRAME-FEAT-008`);
2. bring the formula cache up to date, so each row is tested on its evaluated value, not its raw
   text;
3. write the identity of every failing row into `filter_hidden_{sheet}`, replacing the previous set;
4. store the definition in `filters_{sheet}`, replacing the previous one;
5. derive the grid positions and reply with the count.

**Steps, when the data changes:**

1. an edit to a cell of the filtered column re-tests the rows and rewrites the hidden set;
2. a sort, an insert or a delete rewrites nothing: the marks are keyed by row identity, so deriving
   the positions again is enough (`TRAME-FEAT-002`).

**Code:**

* `crates/ambre-trame/src/logic/filters.rs` -- apply, clear, recompute, the value list;
* `logic/helpers.rs` -- `filter_pass`, the predicate for one row;
* `ui/grid/` -- the hidden-row set, the button geometry, the collapsed axis;
* `ui/renderer/` -- the buttons and the funnel glyph;
* `chrome/commands.rs` -- the two context-menu entries.

**Personal filter views**, not built. Local state, held outside the document:

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::SaveFilterView { name, sheet_id, filter_col, kind }` | UI to Logic | one definition, to persist locally (`TRAME-FEAT-014`) |
| `TrameCommand::ActivateFilterView { view_id }` | UI to Logic | the view to apply; an unknown identity is a no-op |
| `TrameEvent::FilterViewSaved { view_id }` | Logic to UI | the identity minted for the saved view |

```rust
pub struct FilterView {
    pub id:          String,
    pub name:        String,
    pub sheet_id:    SheetId,
    pub filter_col:  Uid,
    pub kind:        FilterKind,   // the same predicate the shared filter stores
    pub hidden_rows: HashSet<Uid>, // a cache, recomputed on each activation
}
```

A view is persisted to IndexedDB alone, under `filter_view:{user_id}:{sheet_id}:{view_id}`.
Activating one re-tests the rows, then sends the viewport without those that fail.

---

## 4. SURFACE

Everything the user touches. The menu carrying the two entries is
`features/ambre-spec-trame-context-menu.md`.

**Triggers.**

| Trigger | Sends | Behaviour |
|---|---|---|
| the button on the range's first row | opens the popover | a click selects the button's column first, so the popover and the menu path are one path with two entry points |
| context menu, "Filter by column" | opens the popover | over the active column. The entry reads "Clear filter" while a filter is active |
| context menu, "Clear filter" | `ClearActiveFilter` | with no filter active it is a no-op, and nothing reaches the other users |
| the popover, on apply | `ApplyFilterToActiveColumn { kind }` | the Logic thread resolves the column against the active cell it tracks |

**The popover** offers the two kinds:

* a checkbox list of the column's distinct visible values;
* one condition: an operator and an operand.

The list arrives through a read-only round trip -- `RequestActiveColumnValues`, then `ColumnValues`
-- so the UI evaluates nothing (`TRAME-FEAT-026`).

**The buttons** (`TRAME-FEAT-027`), on the first row of the data block around the active cell:

* one button per column of that block, at the right edge of its first-row cell;
* a chevron while the column is unfiltered, a funnel while a filter is active on it;
* always drawn, so a user reaches the popover before any filter exists;
* a funnel badge on the column letter is a second cue.

**The collapsed axis** (`TRAME-UI-008`):

* a hidden row occupies no height, and the row numbers keep their values;
* vertical navigation steps over it;
* hit-testing uses the geometry the drawing uses, so a click lands on the row the user sees.

**A personal view** carries a badge while it is active: the hidden rows are hidden for this user
alone.

**Events reaching the user.** `FilterApplied` and `FilterCleared` are chrome-only: the canvas tracks
the filtered column for the glyph, and the chrome relay turns each into a notice.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A filter hides rows and changes no value | dropping the rows from the data | a filter is a reading gesture, and it would make one an irreversible deletion of everyone's data |
| A filter hides rows and changes no value | excluding them from every formula | a user filters to read a subset, and expects the totals to hold |
| The author evaluates the rows, and the others apply the result | every peer re-tests the rows against its own state | one evaluation gives one answer; two peers evaluating a formula at different moments can disagree |
| The hidden set is keyed by row identity | marking a row by its position | a single insert invalidates it. An identity follows its row through a sort, an insert and a delete, with no recompute |
| The definition is stored in the document | keeping the filter in the local session, like a personal view | the filter is shared, so it must survive a reload and reach a peer that connects later |
| A filter is an ordinary change to the sheet | making a filter non-undoable, as Google Sheets does | the user then carries an exception in mind: one gesture on the sheet that the undo key does not answer |
| A filter is an ordinary change to the sheet | a separate history for view changes | two stacks have to be ordered against each other on every undo, and the ordinary path already undoes, redoes and converges |
| An edit recomputes the hidden set, a structural change does not | recomputing after every change | it costs one pass over the column for nothing: only an edit can change the outcome of the test, and a move changes positions alone |
| The buttons sit on the data's first row | the funnel badge on the column letter alone | it sits far from the values it describes; the affordance belongs where the data is, as it does in Excel |
| A personal view writes nothing to the document | a per-user container in the document | every peer would then merge and undo it. A personal view is one user's reading of shared data, so it must reach no collaborator and enter no history, and local storage is the only store that gives both |
| A personal view re-tests the rows on each activation | trusting the persisted set | it shows a stale view of live data: the document changed since the view was saved, so a stored hidden set names rows that may now pass. The predicate is cheap and the set is not a fact |
| The value list excludes hidden rows | listing every value of the column, including those the active filter hides | the list shows what the current view holds, so a second filter narrows the first |
| The popover is DOM chrome anchored under its column | drawing the popover on the `DrawSurface`, beside the cells it filters | it needs a scrolling list, a search field and checkboxes, which the chrome toolkit already gives; a canvas overlay would reimplement all three and take the keyboard from the grid |

---

## 6. BROKEN, NOT BUILT, OR NOT RULED

* **The stored value list is unreadable, so a filter by values hides every row after any edit**
  (`TRAME-FEAT-004`, `TRAME-FEAT-009`). The list is a child container of `filters_{sheet}`. The
  reader takes the map's flat value, where that child reads as a handle, and falls back to an empty
  list. No value matches it. The first evaluation is right, because it uses the definition the user
  just sent. The next edit re-reads the stored one and hides every row, on one peer, with no
  collaborator.
* **A concurrent race tears the definition** (`TRAME-FEAT-025`). The five keys are elected one by
  one. Two users filtering different columns at the same moment converge on one user's column
  joined to the other's operator and operand. Neither user applied that filter, and both peers agree
  on it. One register would hold the definition whole, which `ambre-spec-trame-schema.md section 6`
  already called for a cell style.
* **An edit outside the filtered column rewrites the whole hidden set** (`TRAME-FEAT-036`).
  `finish_cell_writes` calls `recompute_active_filter` after every cell write, whatever its column,
  and that call empties `filter_hidden_{sheet}` and refills it. One keystroke on a filtered sheet
  broadcasts one delta per hidden row for an unchanged result, against the section 5 decision that
  only an edit to the tested column can change the outcome.
* **A filter outlives the column it tests** (`TRAME-FEAT-030`). `write_filter_hidden` empties the
  hidden set, fails to resolve the column uid and returns 0, while `filters_{sheet}` keeps the
  definition. The funnel stays lit on a column that is gone and no row is hidden.
* **The status bar and the formulas total different cells** (`TRAME-FEAT-029`). `selection_stats`
  skips every hidden row; `TRAME-FEAT-003` keeps those rows in `SUM`. One column reads two totals,
  one in the bar and one in a cell, and nothing tells the user which is which.
* **A refusal is silent** (`TRAME-FEAT-037`). Each guard in `apply_filter` returns an empty
  `display_only`, so filtering a virtual column closes the popover and says nothing.
* **The value list is ordered as text** (`TRAME-FEAT-041`). `active_column_values` sorts the display
  strings, so a column of numbers offers `10` before `9`. The test asserts the order the code
  produces, not the order a reader expects.
* **A filter over a merged block** is unspecified. `features/ambre-spec-trame-cell-merges.md
  section 6` states the same gap from the other side.
* **`SUBTOTAL`** is not built: no function excludes the hidden rows today (`TRAME-FEAT-003` is the
  reason a user needs one).
* **Colour filters** are not built: the kinds are values and condition.
* **Personal filter views are decided and unbuilt** (`TRAME-FEAT-014..016`). The two commands and
  the reply exist in `ambre-bus`, and the Logic handlers return nothing. The storage, the badge and
  the list of saved views have no code at all.
* **A personal view over a shared filter is unruled.** Nothing states whether the two sets of hidden
  rows compose, which one the buttons then report, or what a user sees when a collaborator clears
  the shared filter under a personal view.
