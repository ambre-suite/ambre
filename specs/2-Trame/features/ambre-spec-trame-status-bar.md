# AMBRE -- FEATURE: THE STATUS BAR

**Type:** Vertical (feature)
**Implementation:** partial -- the band ships; a whole-column total covers the visible rows alone.

> A user selects a block of cells and reads its total without writing a formula. A thin band under
> the grid shows the sum, the average, the count, the minimum and the maximum of the selection.

The band holds no state of its own: it displays an answer the Logic thread computes and forgets.
That is why it appears in no undo history and reaches no collaborator.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-103` | The aggregates MUST be computed by the Logic thread, never by the UI thread. | this spec section 2 | type-level: the UI holds only the cells of its window, and the reply `TrameEvent::SelectionStats` carries six numbers with no cell in them; `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells` drives the Logic side | verified |
| `TRAME-UI-104` | An aggregate MUST cover the whole selection, not the part the UI thread holds. | this spec section 2; section 4 | none. A cell rectangle is sent whole, so the rule holds there; a whole-column selection is clamped by the UI to its own cache. **The rule fails on a whole-column selection** -- section 5 | unverified |
| `TRAME-UI-105` | `count` MUST tally every non-empty cell, and the numeric aggregates MUST tally the numbers alone. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells`, over a column holding numbers, a blank and a word | verified |
| `TRAME-UI-106` | A query for the aggregates MUST emit no CRDT delta. | this spec section 2 | `ambre_trame::logic::tests::group4::selection_stats_aggregate_numeric_cells`, which asserts the outcome carries no update | verified |
| `TRAME-UI-107` | The band MUST show a numeric aggregate only when the selection holds a number. | this spec section 3 | `ambre_trame::tests::chrome_ui::status_bar_renders_label_and_aggregates` mounts the band over a selection holding three numbers; the branch that hides the four numeric aggregates is reached by no test | unverified |
| `TRAME-UI-108` | A selection of one cell MUST show no aggregate. | this spec section 3 | none. The refresh clears the band when the selection is not multi-cell, and no test drives a selection change | unverified |
| `TRAME-UI-109` | The band MUST follow the selection: every change to it MUST refresh the aggregates. | this spec section 2 | none. The refresh is called from the active-cell announcement, on the browser-only view, and nothing asserts the call | unverified |
| `TRAME-UI-110` | A row the filter hides MUST NOT count in an aggregate. | this spec section 4 | none. The walk skips the hidden set, and every stats test runs on an unfiltered sheet | unverified |

* **Budget.** none: the tallies are one walk over the selection, and
  `ambre-spec-budgets.md` budgets no aggregate. The band paints inside the frame
  phases of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: the band reads and shows. It writes nothing to the document, opens no path,
  and `UI-THREAD-004` bans a network, crypto or persistence path on the thread it paints on.
* **Verification.** 3 `verified`, 5 `unverified`. The Logic half is held: the walk, the tallies and
  the read-only guarantee all have a host test. The five gaps are the UI half -- what the band
  shows, when it refreshes, and which rows it covers -- and `TRAME-UI-104` is a rule the code
  answers with something else.

---

## 2. MODEL

**Containers.** None. The aggregates are derived on each request and stored nowhere.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::RequestSelectionStats` | UI to Logic | nothing: the Logic thread resolves the tracked selection itself (`TRAME-UI-103`) |
| `TrameEvent::SelectionStats { sum, avg, count, count_num, min, max }` | Logic to UI | the six numbers, and nothing else |

**Display shape** (`ambre-trame`, the chrome side):

```rust
pub struct SelStats {
    pub sum:       f64,
    pub avg:       f64,
    pub count:     u64,   // non-empty cells
    pub count_num: u64,   // numeric cells; zero hides sum, avg, min and max
    pub min:       f64,
    pub max:       f64,
}
```

**Steps, when the Logic thread answers a request:**

1. bring the formula cache up to date, so each cell is read at its computed value;
2. walk the tracked selection rectangle, skipping the rows the filter hides (`TRAME-UI-110`);
3. tally every non-empty cell into `count`;
4. tally each number into `count_num`, and fold it into the sum, the minimum and the maximum;
5. divide the sum by the numeric count for the average, and answer zero for an empty one;
6. reply with the six numbers, and emit no delta (`TRAME-UI-106`).

**Steps, on the UI side:**

1. a selection change clears the band, then asks for the aggregates when the selection is
   multi-cell (`TRAME-UI-108`);
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
| the zoom chip | the grid zoom menu, owned by `features/ambre-spec-trame-grid-viewport.md section 4` |
| the frame chip | the frame rate, expanding on click into the frame timings |

**How a number reads.** A whole number shows with no decimals, any other with two, so the band
keeps a stable width as the selection changes.

**What the user does not see.** One cell shows an empty band, because the cell already shows its own
value (`TRAME-UI-108`).

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The Logic thread computes the aggregates | the selection can be far larger than the window, and the UI thread holds the window alone. Sending the cells to the UI would move the whole selection across a channel to compute six numbers | totalling the cached cells on the UI thread, which answers for the visible part alone |
| The request carries no geometry | the Logic thread already tracks the selection for the sort, the style and the structural commands. A second copy travelling with the request would be a second source for one rectangle | sending the rectangle with the request |
| An aggregate skips the rows the filter hides | the band reports what the user is looking at, and a user who filters a column is asking about the rows that remain | totalling every row, which is what a formula does |
| A formula does not skip them | a formula's result must not depend on a view, or two peers filtered differently read two totals (`TRAME-021`). The band is not a document value, so it is free to follow the view | making `SUM` follow the filter, which is what `SUBTOTAL` will do explicitly |
| The band is DOM, not a drawn surface | it is text and two menus, and the renderer was shaping its glyphs on the GPU every frame for a read-out that changes on selection | a drawn band inside the canvas, which is what the earlier design carried |
| The numeric aggregates hide with no number | a sum of a text selection is zero, which reads as a fact rather than as an absence | showing zeros |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A whole-column total covers the visible rows alone** (`TRAME-UI-104`). A click on a column
  header selects the band, and the UI turns it into a rectangle whose last row is the last non-empty
  cell **in its own cache**, which holds the window and its margin. So the aggregate of a column
  longer than the screen is the aggregate of its first screen. The same clamp reaches every command
  that resolves against the selection: the sort, the style, the delete and the copy.
  `ambre-spec-trame-selection.md section 6` states it from the side that produces it.
* **The band has no host test.** Five rows say `none`, and four of them are about the UI half. The
  band itself is a pure function of one `SelStats` value, so the wasm tier can reach it; the refresh
  sits on the browser-only view and cannot be reached without one.
* **A merged block is counted by its cells.** A covered cell holds no value, so a block spanning
  four cells adds one to `count`, and nothing states whether that is the intent.
* **No aggregate is selectable.** A user cannot choose which aggregates the band shows, and cannot
  copy one out of it.
