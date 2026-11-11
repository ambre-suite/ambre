# AMBRE -- FEATURE: SHARED FILTER
**Type:** Vertical (feature)

> A user hides the rows of a column that fail a test. Every collaborator sees the same hidden rows.
> A hidden row keeps its value and still counts in every formula: a filter is a view over the data,
> never a change to it.

The personal views over it are the same predicate for one user alone. They are not built.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-002` | `INV` | A hidden row MUST stay hidden when the data moves: a sort or an insert MUST NOT transfer the hidden mark to another row. | this spec section 3 | `ambre_trame::logic::tests::group5::shared_filter_hidden_follows_sort` | none | verified |
| `TRAME-FEAT-003` | `INV` | A hidden row MUST still count in every formula: a filter changes what a user sees, never what the sheet holds. | this spec section 2, row 5 | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` | none | verified |
| `TRAME-FEAT-005` | `INV` | Every collaborator MUST see the same hidden rows as the user who applied the filter. | this spec section 2, row 3 | `ambre_trame::logic::tests::group5::shared_filter_applies_on_remote_import` | none | verified |
| `TRAME-FEAT-031` | `INV` | The hidden rows MUST agree with the data: an edit to a filtered column MUST re-test that row. | this spec section 2, row 1 | `ambre_trame::logic::tests::group5::shared_filter_recomputes_on_edit` -- it drives the condition kind; `::filter_by_values_survives_a_reload` -- the values kind reads back and re-tests | none | verified |
| `TRAME-FEAT-030` | `INV` | A filter MUST NOT outlive the column it tests. | this spec section 3 | `ambre_trame::logic::tests::group5::filter_goes_with_its_column` | none | verified |
| `TRAME-FEAT-006` | `CONC` | When two users filter one sheet at the same moment, every peer MUST end on one filter and one hidden set. | this spec section 2, row 4 | `ambre_trame::logic::tests::group5::concurrent_filters_converge_lww`; `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest that drives `ApplyFilter` and `ClearFilter` and asserts the elected hidden set | none | verified |
| `TRAME-FEAT-025` | `CONC` | A filter that wins a concurrent race MUST be one a user applied, whole: no peer MUST see one user's column joined to another user's operator or operand. | this spec section 2, row 7; section 6 | `ambre_trame::logic::tests::group5::concurrent_filters_elect_a_whole_definition` -- two peers filter at once and the elected definition is one of the two whole ones, on both peers | none | verified |
| `TRAME-UI-008` | `OBS` | A hidden row MUST occupy no height. | this spec section 4 | `ambre_trame::render::tests::cell_rect_collapses_over_hidden_rows` -- it reads the rectangle of a cell below a hidden row and finds it risen by that row's height | none | verified |
| `TRAME-UI-173` | `OBS` | A click MUST land on the row the user sees, never on the one the collapse moved out of the way. | this spec section 4 | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav` -- it hit-tests a point over the collapsed range and reads back the visible row | none | verified |
| `TRAME-UI-154` | `OBS` | Arrow-key navigation MUST step over a hidden row. | this spec section 4 | `ambre_trame::ui::grid::tests::group2::filtered_rows_collapse_geometry_and_nav` -- the `nav` half of the same case | none | verified |
| `TRAME-FEAT-027` | `OBS` | Every column of the filtered range MUST carry a button on the range's first row. | this spec section 4 | `ambre_trame::ui::renderer::tests::autofilter_buttons_drawn_on_region_header_row`; `ambre_trame::ui::grid::tests::group1::filter_button_hit_test_matches_drawn_rect`; `::filter_button_scales_with_zoom` | none | verified |
| `TRAME-FEAT-043` | `OBS` | The button MUST show whether a filter is active on its column. | this spec section 4 | `ambre_trame::ui::renderer::tests::autofilter_button_shows_funnel_on_filtered_column` | none | verified |
| `TRAME-FEAT-046` | `OBS` | The user MUST be told how many rows a filter hid. | this spec section 4 | `ambre_trame::tests::chrome_ui::the_filter_toast_says_how_many_rows_it_hid` | none | verified |
| `TRAME-FEAT-029` | `OBS` | What the status bar totals MUST be what a formula over the same cells totals, save text that reads as a number and a boolean, which only the formula counts. | this spec section 4 | `ambre_trame::logic::tests::group5::status_bar_total_matches_sum_under_a_filter`; `ambre_trame::logic::tests::group4::selection_stats_sums_as_a_formula_over_the_same_cells_does` -- a small term between two large ones, terms that cancel near the largest number, and an overflow | none | verified |
| `TRAME-FEAT-004` | `IO` | A filter MUST read back from the document as the filter the user applied. | this spec section 2, row 1 | `ambre_trame::logic::tests::group5::filter_by_values_survives_a_reload` -- a values filter exports, reloads and still hides the same row | none | verified |
| `TRAME-FEAT-009` | `IO` | A filter MUST hide the same rows after a reload as before it, for every operator it offers. | this spec section 3 | `ambre_trame::logic::tests::group5::condition_pass_covers_every_operator`; `::filter_pass_values` -- the predicate of a values filter; `::filter_by_values_survives_a_reload` -- the definition reads back from the document | none | verified |

### 1.2 Filter a column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-034` | `PRE` | The rows MUST be tested on their computed value, never on their text. | this spec section 3 | `ambre_trame::logic::tests::group13::a_filter_reads_the_computed_value` | none | verified |
| `TRAME-FEAT-032` | `POST` | Applying a filter MUST replace the previous one, never add to it. | this spec section 3 | `ambre_trame::logic::tests::group13::a_second_filter_replaces_the_first` | none | verified |
| `TRAME-256` | `POST` | A condition MUST compare text whatever its case. | this spec section 3 | `ambre_trame::logic::tests::group5::a_condition_compares_text_whatever_its_case` -- equality, the order and the substring tests | none | verified |
| `TRAME-FEAT-045` | `POST` | Applying a filter MUST hide every row that fails the test and no other. | this spec section 3 | `ambre_trame::logic::tests::group4::shared_filter_by_values_hides_nonmatching`; `::shared_filter_condition_greater_than` -- both assert the hidden mark row by row, one per filter kind | none | verified |
| `TRAME-FEAT-033` | `POST` | Applying a filter MUST report the number of rows it hid. | this spec section 4 | `ambre_trame::logic::tests::group4::shared_filter_by_values_hides_nonmatching`; `::shared_filter_condition_greater_than` -- both match `FilterApplied { hidden_row_count }` against the rows they hid | none | verified |
| `TRAME-FEAT-007` | `POST` | An undo MUST remove a filter in one step, like any other change to the sheet. | this spec section 2, row 6 | `ambre_trame::logic::tests::group5::shared_filter_is_undoable`; `ambre_trame::logic::tests::group2::filtered_column_edit_is_one_undo_item` | none | verified |
| `TRAME-FEAT-035` | `FRAME` | Applying a filter MUST NOT change any cell value. | this spec section 5 | `ambre_trame::logic::tests::group13::a_filter_and_its_clear_keep_every_cell` -- the whole cells map compared; `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum` | none | verified |
| `TRAME-FEAT-036` | `FRAME` | An edit outside the filtered column MUST NOT rewrite the hidden set. | this spec section 3 | `ambre_trame::logic::tests::group5::edit_outside_the_filtered_column_leaves_the_hidden_set_alone` | none | verified |
| `TRAME-FEAT-008` | `FAIL` | A filter that cannot apply MUST leave the sheet unchanged. | this spec section 4 | `ambre_trame::logic::tests::group5::shared_filter_noop_guards` -- a wrong sheet and a virtual column; `::clear_filter_unhides_and_noops_when_absent` | none | verified |
| `TRAME-FEAT-044` | `FAIL` | A filter that cannot apply MUST reach no collaborator. | this spec section 4 | `ambre_trame::logic::tests::group5::shared_filter_noop_guards` -- it asserts `update.is_none()` on each guard | none | verified |
| `TRAME-FEAT-037` | `FAIL` | A filter that cannot apply MUST say so. | this spec section 4 | `ambre_trame::logic::tests::group5::shared_filter_noop_guards` | none | verified |

### 1.3 Clear the filter

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-038` | `POST` | Clearing MUST remove both the definition and the hidden rows. | this spec section 3 | `ambre_trame::logic::tests::group5::clear_filter_unhides_and_noops_when_absent` -- the `unhides` half | none | verified |
| `TRAME-FEAT-039` | `FRAME` | Clearing MUST NOT change any cell value. | this spec section 5 | `ambre_trame::logic::tests::group13::a_filter_and_its_clear_keep_every_cell` | none | verified |
| `TRAME-FEAT-040` | `FAIL` | Clearing with no filter active MUST reach no collaborator. | this spec section 4 | `ambre_trame::logic::tests::group5::clear_filter_unhides_and_noops_when_absent` -- the `noops_when_absent` half; `::shared_filter_noop_guards` | none | verified |

### 1.4 List the values of a column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-026` | `POST` | The values offered for a column MUST hold each distinct visible value once. | this spec section 4 | `ambre_trame::logic::tests::group4::active_column_values_are_distinct_sorted_and_visible` | none | verified |
| `TRAME-FEAT-041` | `POST` | The values MUST be ordered as the column reads them, not as text. | this spec section 4 | `ambre_trame::logic::tests::group5::filter_values_are_ordered_as_the_column_reads` | none | verified |
| `TRAME-257` | `POST` | Two values that differ in case alone MUST each be listed once, next to each other. | this spec section 4 | `ambre_trame::logic::tests::group5::filter_values_list_each_case_once_and_together` | none | verified |
| `TRAME-FEAT-042` | `FRAME` | Listing the values MUST reach no collaborator. | this spec section 3 | type-level -- the handler returns `TrameOutcome::display_only`, which builds `update: None`, so a read-only query cannot carry a delta | none | verified |

### 1.5 Keep a filter of one's own

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-FEAT-015` | `POST` | Activating a personal filter view MUST recompute its hidden rows against the current document, never trust the stored set. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-014` | `FRAME` | A personal filter view MUST NOT emit any CRDT delta: it is local state, persisted to IndexedDB alone. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-FEAT-016` | `FRAME` | Activating a personal filter view MUST NOT be an undo item. | this spec section 3 | not built | unbuilt -- lands with the `Data tools` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none: the filter is one predicate pass over the rows a sheet holds, and
  `ambre-spec-budgets.md` budgets no filter.
* **Boundary.** none of its own: the filter is shared, so its definition is a document write, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 34 `verified`, 3 `unimplemented`. The definition is
  one register, so a concurrent race converges whole (`TRAME-FEAT-025`) and a filter by values reads
  back and re-tests correctly (`TRAME-FEAT-004`, `TRAME-FEAT-009`, `TRAME-FEAT-031`). The remaining
  gaps are stated once in section 6.

---

## 2. WHAT EACH USER SEES

Two peers, one sheet. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| filters column A to show `a` alone | writes `a` into a row Alice's filter hides | the filter and Bob's text | the row returns, for both users | `logic::tests::group5::shared_filter_recomputes_on_edit` |
| filters column A | sorts the same range | the filter and the new row order | the same rows hidden, at their new positions | `logic::tests::group5::shared_filter_hidden_follows_sort` |
| filters column A | opens the sheet after her | one filter, one hidden set | the same collapsed grid; Bob's client applies the set, it does not re-test the rows | `logic::tests::group5::shared_filter_applies_on_remote_import` |
| filters column A on "greater than 1" | filters column A on "less than 3" at the same moment | one whole definition, elected as one register | the same rows hidden for both, from a test that proves the winner is one of the two whole filters (`TRAME-FEAT-025`) | `logic::tests::group5::concurrent_filters_elect_a_whole_definition` |
| filters column A | sums column A | the filter alone | a total over every row, hidden ones included | `logic::tests::group5::shared_filter_does_not_affect_sum` |
| filters column A, then undoes | -- | no filter | every row visible again, for both | `logic::tests::group5::shared_filter_is_undoable` |
| filters column A by values | filters column C by condition at the same moment | one whole definition, column and kind together | every peer sees one of the two filters, whole: never a column joined to another's kind | `logic::tests::group5::concurrent_filters_elect_a_whole_definition` |

---

## 3. MODEL

**Containers**, both declared in `ambre-spec-trame-schema.md section 3`:

* `filters_{sheet}` holds the definition as one register: the column and the kind together,
  postcard-encoded whole. An empty map is no filter.
* `filter_hidden_{sheet}` holds one entry per hidden row, keyed by the row identity. The mark
  follows its row through a sort, an insert or a delete (`TRAME-FEAT-002`).

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ApplyFilter { sheet_id, col, kind: FilterKind }` | UI to Logic | the target column and the definition |
| `TrameCommand::ClearFilter { sheet_id }` | UI to Logic | the sheet |
| `StaticKey<Commands>::FilterColumn` / `ClearFilter` | chrome to Logic | no geometry: the Logic thread resolves the column against the active cell it tracks |
| `TrameCommand::ApplyFilterToActiveColumn { kind }` / `ClearActiveFilter` | UI to Logic | the definition alone, resolved against the tracked selection |
| `TrameCommand::RequestActiveColumnValues` | UI to Logic | nothing: a read-only query for the value list |
| `TrameEvent::ColumnValues { col, values }` | Logic to UI | the distinct visible values (`TRAME-FEAT-026`) |
| `TrameEvent::FilterApplied { hidden_row_count, col }` | Logic to UI | the count for the notice, the column for the button glyph |
| `TrameEvent::FilterCleared` | Logic to UI | the clearing, surfaced as a notice |

**Definition shape**, typed in `ambre-bus` and postcard-encoded whole into the register above:

* `FilterKind::Values(Vec<String>)` -- the listed values are the visible ones;
* `FilterKind::Condition { op: FilterOp, value }` -- one operator over one operand;
* `FilterOp` covers `eq`, `ne`, `gt`, `lt`, `gte`, `lte`, `contains`, `not_contains`,
  `starts_with`, `is_empty` and `is_not_empty` (`TRAME-FEAT-009`). A comparison and a substring
  test read text whatever its case, through the fold the formula operators read (`TRAME-256`).

**Engine state.** One field of `TrameState`, and its mirror on the UI side:

```rust
/// Grid positions of the rows the active filter hides. Derived from
/// `filter_hidden_{sheet}` on every filter or structural change, never persisted.
hidden_rows: HashSet<Pos<Row>>,
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
    pub sheet_id:    Id<Sheet>,
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

## 6. NOT BUILT, AND NOT YET RULED

* **A filter over a merged block** is unspecified. `features/ambre-spec-trame-cell-merges.md
  section 6` states the same gap from the other side.
* **`SUBTOTAL`** is not built: no function excludes the hidden rows today (`TRAME-FEAT-003` is the
  reason a user needs one).
* **Colour filters** are not built: the kinds are values and condition.
* **Personal filter views are decided and unbuilt** (`TRAME-FEAT-014..016`). The two commands and
  the reply exist in `ambre-bus`, and the Logic handlers return nothing. The storage, the badge and
  the list of saved views have no code at all.
* **A personal view over a shared filter is unruled.** Nothing states whether the two sets of hidden
  rows compose. Nothing states which one the buttons then report, or what a user sees when a
  collaborator clears the shared filter under a personal view.
