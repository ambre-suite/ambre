# AMBRE -- FEATURE: THE TRAME STATUS BAR

**Type:** Vertical (feature)
**Implementation:** partial -- the band ships and totals the whole selection; the band itself has no
host test (section 5).

> A user selects a block of cells and reads its total without writing a formula. A thin band under
> the grid shows the sum, the average, the count, the minimum and the maximum of the selection.

The band holds no state of its own: it displays an answer the Logic thread computes and forgets.
That is why it appears in no undo history and reaches no collaborator.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-103` | `INV` | The aggregates MUST be computed by the Logic thread, never by the UI thread. | this spec section 2 | type-level -- the reply `TrameEvent::SelectionStats` carries a count and the numeric totals, with no cell in them, and the signal the band reads has one writer, the launcher's relay on that event (`ambre-app`); `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells` -- the Logic side | none | verified |
| `TRAME-UI-106` | `INV` | A query for the aggregates MUST emit no CRDT delta. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells` -- it asserts the outcome carries no update | none | verified |
| `TRAME-UI-104` | `OBS` | An aggregate MUST cover the whole selection, not the part the UI thread holds. | this spec section 2; section 4 | `ambre_trame::logic::tests::group4::selection_stats_covers_the_whole_band` -- a whole-column band tallies an off-screen row; `::selection_stats_totals_a_disjoint_selection` -- two rectangles are tallied together | none | verified |
| `TRAME-UI-105` | `OBS` | `count` MUST tally every non-empty cell. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells` -- a column holding numbers, a blank and a word, asserting the count of the non-empty ones | none | verified |
| `TRAME-UI-172` | `OBS` | A numeric aggregate MUST tally the numbers alone. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells` -- the same column, where the word and the blank leave the sum, the average and the extremes untouched | none | verified |
| `TRAME-UI-107` | `OBS` | The band MUST show a numeric aggregate only when the selection holds a number. | this spec section 3 | `ambre_trame::tests::chrome_ui::a_selection_with_no_number_shows_no_numeric_aggregate`; `::status_bar_renders_label_and_aggregates` | none | verified |
| `TRAME-UI-108` | `OBS` | A selection of one cell MUST show no aggregate. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_lone_cell_asks_for_no_aggregate`; `ambre_trame::ui::view::lifecycle::tests::a_single_cell_clears_and_sends_nothing` | none | verified |
| `TRAME-UI-110` | `OBS` | A row the filter hides MUST count in an aggregate, as it counts in a formula over the same cells. | this spec section 4 | `ambre_trame::logic::tests::group5::status_bar_total_matches_sum_under_a_filter` | none | verified |
| `TRAME-UI-135` | `OBS` | An aggregate MUST read with no decimals when it is a whole number, and with two otherwise. | this spec section 3 | `ambre_trame::tests::chrome_ui::an_aggregate_reads_whole_or_with_two_decimals` | none | verified |
| `TRAME-UI-178` | `OBS` | A cell value that is not a finite number MUST count in `count` and in no numeric aggregate. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_leaves_a_value_that_is_not_finite_out_of_the_totals` -- a typed `NaN`, two infinities and `1e999`, which read as text, beside two numbers; type-level -- the totals hold a `CellNumber`, which refuses a value that is not finite | none | verified |
| `TRAME-UI-179` | `OBS` | A sum whose exact total is past the largest finite number MUST read `#NUM!`, and so MUST its average. | this spec section 2; section 3 | `ambre_trame::logic::tests::group4::selection_stats_sums_as_a_formula_over_the_same_cells_does` -- the sum, in two orders, beside `=SUM` over the same cells; `ambre_bus::app::trame::event::tests::an_overflowed_sum_leaves_no_average_and_exact_extremes` -- the average; `ambre_trame::tests::chrome_ui::a_sum_past_the_largest_number_reads_num` -- the band | none | verified |

### 1.2 Select a different block

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-109` | `POST` | A change to the selection MUST refresh the aggregates. | this spec section 2 | `ambre_trame::ui::grid::tests::group1::extending_a_header_band_makes_the_selection_multi` -- the extend path yields a multi-cell band; `ambre_trame::ui::view::lifecycle::tests::a_sent_request_keeps_the_last_answer` -- a multi-cell band sends the request | none | verified |
| `TRAME-UI-136` | `FRAME` | A refresh whose request did not reach the Logic thread MUST NOT leave the band showing the totals of another selection. | `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) | `ambre_trame::ui::view::lifecycle::tests::a_dropped_request_clears_the_band` -- the clear decision in `stats_band_clears`; `::a_single_cell_clears_and_sends_nothing`; `::a_sent_request_keeps_the_last_answer` -- the other two sides | none | verified |

* **Budget.** none: the tallies are one walk over the selection, and
  `ambre-spec-budgets.md` budgets no aggregate. The band paints inside the frame
  phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the band reads and shows. It writes nothing to the document, opens no path,
  and `UI-THREAD-004` bans a network, crypto or persistence path on the thread it paints on.
* **Verification.** 13 `verified`. The walk, the tallies, the read-only guarantee, the
  whole-selection coverage, the refresh on a selection change and the clear decision each have a
  host test. The band, its number format and its overflow are read back on the wasm tier. The
  count and the numeric aggregates are two rows (`TRAME-UI-105`, `TRAME-UI-172`) over one test.

---

## 2. MODEL

**Containers.** None. The aggregates are derived on each request and stored nowhere.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::RequestSelectionStats` | UI to Logic | nothing: the Logic thread resolves the tracked selection itself (`TRAME-UI-103`) |
| `TrameEvent::SelectionStats { count, numbers }` | Logic to UI | the count, and the totals of the numbers when there is one; nothing else |

**Display shape** (`ambre-trame`, the chrome side):

```rust
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct SelStats {
    pub count:   Tally<Cells>,          // non-empty cells
    pub numbers: Option<NumericStats>,  // `None` hides sum, avg, min and max
}

/// The totals of at least one number (`ambre-bus`): its count, sum, minimum
/// and maximum, and the average they give. The sum is the exact total the
/// Logic thread computes, and it and the average are `None` past the
/// largest finite number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericStats { /* private */ }

/// A cell number (`ambre-bus`): a number a cell holds, always finite.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct CellNumber(f64);
```

**The cell number.** The formula engine computes in `f64`. A cell's text comes from a keystroke, a
paste, an import or a peer. Text such as `NaN` or `1e999` reads as text, not as a number
(`TRAME-245`), and a formula whose result is not finite reads `#NUM!` (`TRAME-244`). A value that
is not finite is no cell number either way, so it enters no total (`TRAME-UI-178`).

**The one difference from `SUM`.** The band totals the cells that hold a number. `SUM` also counts
text that reads as a number (`TRAME-200`) and a boolean in a range, so over such cells the two
read two totals. Excel's own band leaves them out the same way. Every other total agrees
(`TRAME-FEAT-029`).

**Steps, when the Logic thread answers a request:**

1. bring the formula cache up to date, so each cell is read at its computed value;
2. walk every tracked selection rectangle, the rows the filter hides included (`TRAME-UI-110`);
3. tally every non-empty cell into `count`;
4. fold each cell number into the count, the minimum and the maximum. Sum them as `SUM` does,
   exactly and then rounded once, so their order changes nothing (`TRAME-FEAT-029`). The sum is
   an overflow only when that exact total is past the largest finite number (`TRAME-UI-179`);
5. leave the totals out when no number was found, so no aggregate reads an empty zero;
6. reply with the count and the totals, and emit no delta (`TRAME-UI-106`).

**Steps, on the UI side:**

1. a selection change asks for the aggregates when the selection is multi-cell, and clears the
   band otherwise (`TRAME-UI-108`). The request is lossy, so a full queue drops it and the band
   stays cleared (`TRAME-UI-136`);
2. the reply is mirrored into one signal, which the band reads.

**Code:**

* `crates/ambre-trame/src/logic/filters.rs` -- `selection_stats`, the walk;
* `chrome/status.rs` -- `StatusBar` and `SelStats`;
* `ui/view/lifecycle.rs` -- `refresh_selection_stats`, the request.

---

## 3. SURFACE

A thin band under the grid and over the sheet tabs. It is Trame's parameterization of the shared
band specified in `ambre-spec-chrome.md`, which owns the slots below.

| Slot | Holds |
|---|---|
| the label | the application name |
| the aggregates | `Count`, and `Sum`, `Avg`, `Min` and `Max` when the selection holds a number (`TRAME-UI-107`) |
| the zoom chip | the shared zoom model (`ambre-spec-zoom.md`), rendered by the chrome chip (`ambre-spec-chrome.md`, `CHROME-014`) |
| the frame chip | the frame rate, expanding on click into the frame timings |

**How a number reads.** A whole number shows with no decimals, any other with two, so the band
keeps a stable width as the selection changes. A sum that overflows reads `#NUM!`, as `=SUM` over
the same cells does, and its average reads the same (`TRAME-UI-179`).

**What the user does not see.** One cell shows an empty band, because the cell already shows its own
value (`TRAME-UI-108`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The Logic thread computes the aggregates | totalling the cached cells on the UI thread | it answers for the visible part alone: the selection can be far larger than the window, and the UI thread holds the window alone. Sending the cells up instead would move the whole selection across a queue to compute six numbers. |
| The request carries no geometry | sending the rectangle with the request | it would be a second source for one rectangle. The Logic thread already tracks the selection for the sort, the style and the structural commands. |
| An aggregate counts the rows the filter hides | skipping them | the band and a formula over the same cells would then read two totals, with nothing on screen to say why (`TRAME-FEAT-029`). |
| A formula does not skip them | making `SUM` follow the filter | it is what `SUBTOTAL` will do explicitly, and doing it in `SUM` makes a formula's result depend on a view, so two peers filtered differently read two totals (`TRAME-021`). |
| The band is DOM, not a drawn surface | a drawn band inside the canvas | it is what the earlier design carried, and it had the renderer shaping its glyphs on the GPU every frame for a read-out that changes on selection. The band is text and two menus. |
| The numeric aggregates hide with no number | showing zeros | a sum of a text selection is zero, which reads as a fact rather than as an absence. |
| A sum that overflows reads `#NUM!` | showing the infinity, or the largest finite number | an infinity is no cell number, and the largest finite number states a total the cells do not hold. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A merged block is counted by its cells.** A covered cell holds no value, so a block spanning
  four cells adds one to `count`, and nothing states whether that is the intent.
* **No aggregate is selectable.** A user cannot choose which aggregates the band shows, and cannot
  copy one out of it.
