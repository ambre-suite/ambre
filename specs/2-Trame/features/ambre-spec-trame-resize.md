# AMBRE -- FEATURE: COLUMN AND ROW RESIZE
**Type:** Vertical (feature)
**Implementation:** implemented -- an auto-fit measures the cells the viewport
holds, not the column (section 6).

> A user drags the boundary of a column header and the column takes the new width, for everyone.
> A double-click on that boundary sizes it to its content. The width belongs to the column, so it
> follows the data when the columns move.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-036` | `IO` | A size a user set MUST still be there after a reload. | this spec section 3 | `ambre_trame::logic::tests::group7::resize_survives_snapshot_roundtrip` -- a width exported and reimported, the height sharing the same write path | none | verified |
| `TRAME-UI-037` | `INV` | A size MUST stay with its own column: inserting, deleting or moving a column MUST NOT transfer it to another. | this spec section 3 | none | unheld -- the entry is keyed by the column's stable id rather than by its position, which is what would make the rule true, and no test moves a sized column; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-038` | `CONC` | When two users size one column at the same moment, every peer MUST end on one size. | this spec section 2, row 1 | `ambre_trame::logic::tests::group7::concurrent_resize_of_same_col_converges_lww` | none | verified |

### 1.2 Drag a boundary

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-039` | `POST` | A row MUST NOT be sized below its minimum height, wherever the pointer goes. | this spec section 2, row 4 | `ambre_trame::logic::tests::group7::resize_row_round_trips_and_clamps_to_minimum` -- it reads back the clamped height, and the drag guide is clamped on the same minimum before the release | none | verified |
| `TRAME-UI-126` | `POST` | A column MUST NOT be sized below its minimum width, wherever the pointer goes. | this spec section 2, row 4 | none | unheld -- `resize_col` clamps on `MIN_COL_WIDTH` in `logic/metrics.rs` and no test reads a width back below it: it is the half `TRAME-UI-039` was carrying, and the two clamps are separate lines of code; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-040` | `POST` | A stored size MUST NOT depend on the zoom the user was viewing at. | this spec section 3 | none | unheld -- the release divides the on-screen extent back out by the zoom, and no test drives a resize at any zoom but one; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-041` | `POST` | A resize MUST reflow the grid before its confirmation arrives. | this spec section 3 | none | unheld -- the local size map is written before the command is sent, and nothing asserts the grid reflowed without the answer; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-127` | `FRAME` | A resize whose command did not reach the document MUST NOT stay in the grid. | this spec section 5; derived | none | fails -- `send_resize_col` writes the local map, then sends on the lossy class, and `send_command_lossy` returns nothing: a full channel logs a warning the caller never sees, so the grid keeps a width the document never took. It is the shape `BUS-018` bans, and the paste path at least learns of the drop; lands with the `A dropped lossy request does not leave a stale screen` Story (`ambre-roadmap.md` E7) | failing |

### 1.3 Fit a boundary to its content

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-042` | `POST` | A double-click on a boundary MUST size the column to its content. | this spec section 4 | none | partial -- the measurement reads the cells the viewport holds, so the rule holds for what is on screen and no more (section 6); lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-128` | `FRAME` | An auto-fit MUST NOT shrink a column whose content it could not measure. | this spec section 4; derived | none | fails -- `resolve_autofit` walks the viewport cache, and a column whose cells are all outside it measures zero and is set to the default width. A double-click below the loaded window narrows a full column to the width of an empty one; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |

* **Budget.** none: a resize writes one entry in the size register and repaints, and
  `ambre-spec-budgets.md` budgets no resize. The repaint sits in the frame phases of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a size is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 3 `verified`, 2 `failing`, 5 `untested`. What holds is the document plane: the
  write, the reload and the concurrent race each have a test. What is loose is the UI thread, where
  the pointer meets the zoom and the cache, and the column clamp nothing reads back. A resize goes
  out through a call that returns nothing, so a dropped command leaves a width no peer has. An
  auto-fit measures the viewport cache, so a distant double-click shrinks a column to the default;
  both failures are on the send path.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| drags column B wider | drags the same boundary narrower, at the same moment | one width, the later write's | the same column width on both screens | `ambre_trame::logic::tests::group7::concurrent_resize_of_same_col_converges_lww` |
| widens column B | -- | the width against B's column id | the width again after a reload | `::resize_survives_snapshot_roundtrip` |
| widens column B | inserts a column before it | the same width, still against B's column id | the widened column, now one place to the right | none -- see section 6 |
| drags a row boundary past the minimum | -- | the minimum | the row at its minimum, not shorter (`TRAME-UI-039`) | `::resize_row_round_trips_and_clamps_to_minimum` |
| drags a column boundary past the minimum | -- | the minimum | the column at its minimum, not thinner (`TRAME-UI-126`) | none -- see section 6 |
| selects columns B to D, then drags one boundary | -- | three widths, all equal | the three columns at the dragged width | none -- see section 6 |

---

## 3. MODEL

**Containers**, declared in `ambre-spec-trame-schema.md section 3`:

| Container | Key | Value | LWW unit |
|---|---|---|---|
| `col_widths_{sheet}` | the column's stable id | width in px (`f64`) | the column (`TRAME-UI-038`) |
| `row_heights_{sheet}` | the row's stable id | height in px (`f64`) | the row |

A column with no entry takes the default width. A column reset to the default drops its entry
rather than storing the default (`ambre-spec-trame-schema.md section 4`, "absence is a value").

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ResizeCol { version, sheet_id, col, width_px }` | UI to Logic | the column's position and its new width, in unzoomed px (`TRAME-UI-040`) |
| `TrameCommand::ResizeRow { version, sheet_id, row, height_px }` | UI to Logic | the row analogue |
| `TrameEvent::DataReset` | Logic to UI | the confirmation: repaint, then adopt the sizes the next answer carries |

**View state.** Two fields of `TrameUiState`, holding only the sizes that differ from the default,
plus the live guide:

```rust
col_widths: HashMap<u32, f32>,            // keyed by grid position, mirrors the container
row_heights: HashMap<u32, f32>,
resize_preview: Option<ResizePreview>,    // Col(x) or Row(y), in physical px
```

**Steps, on a drag:**

1. a press inside the boundary band arms the drag, instead of starting a selection;
2. each move draws a guide at the prospective edge, clamped at the minimum (`TRAME-UI-039`,
   `TRAME-UI-126`);
3. the release divides the on-screen extent by the zoom, then clamps again (`TRAME-UI-040`);
4. the local size map takes the value first, so the grid reflows at once (`TRAME-UI-041`), and a
   dropped send puts nothing back (`TRAME-UI-127`);
5. one command per column goes out -- every column of the band when a whole-axis selection covers
   the dragged one;
6. the Logic thread mints or resolves the column's stable id, writes the size against it, and
   answers with a repaint. The next viewport answer carries the sizes back
   (`features/ambre-spec-trame-grid-viewport.md section 2`).

**What the Logic thread refuses.** A command naming a sheet other than the active one is dropped.
So is a column more than the materialization cap past the used range
(`features/ambre-spec-trame-rows-and-columns.md`): sizing a column no data has reached writes nothing.

**Code:**

* `crates/ambre-trame/src/logic/metrics.rs` -- the two containers, the clamp, the id resolution;
* `ui/view/input.rs` -- the arming, the guide, the release;
* `ui/view/interact.rs` -- the outgoing commands, the band expansion, the auto-fit measurement;
* `ui/renderer/overlay.rs` -- the guide line;
* `ui/grid/state.rs` -- the sizes adopted from an answer.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| press inside the boundary band of a column header, then drag | a vertical guide follows the pointer; the release sets the width |
| the same on a row header boundary | a horizontal guide; the release sets the height |
| release anywhere else in the header strip | nothing is emitted |
| drag a boundary of a column inside a whole-axis selection | every column of the band takes the dragged width |
| double-click on a boundary | sizes that column to the widest content the viewport holds, floored at the minimum (`TRAME-UI-042`) |
| double-click on a boundary of a column with nothing cached | the column returns to the default size, whatever it holds further down (`TRAME-UI-128`) |

The boundary band is four pixels each side of the edge, and its upper half hosts the inline insert
affordance instead (`features/ambre-spec-trame-headers.md section 3`). While a resize is armed the
pointer shows the axis it will move, and a hit anywhere else in the strip keeps its own cursor.

**Auto-fit measurement.** The widths are measured with the text layout the renderer uses, over the
cached cells of that column. The widest one plus the cell padding becomes the width. The
measurement needs the font context, which only the paint owns, so a double-click is recorded and
resolved on the next frame.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The size is keyed by the column's stable id | keying by the column index, as the wire command does | a position moves under an insert, a delete and a reorder, and a size keyed by one would jump to a neighbour. The id is what the cells, the merges and the filters are already keyed by |
| One size per column is one LWW unit | a width per user | a column has one width on screen, so per-user widths would have to be collapsed on read anyway, and the collapse is the arbitration this decision already makes |
| One size per column is one LWW unit | refusing the second write | two users sizing one column is a race with one sensible answer, and there is nothing inside a width to merge |
| The stored size is unzoomed | storing what was on screen | the zoom is a local reading preference, so a resize made at 150 % must not send everyone else a size half again too wide |
| The grid reflows before the confirmation | reflowing only on the answer | the drag is a direct manipulation and the answer costs a round trip through the Logic thread; waiting would make every resize feel detached |
| The auto-fit is measured on the UI thread | measuring on the Logic thread, over the whole column | the fonts, the layout and the zoom all live there, and the Logic thread has no text measurement at all |
| A resize for another sheet is dropped | applying it to whichever sheet is active | the command carries the sheet it was made on, and the sizes of a sheet the user has left are not what a stale command should change |
| The boundary band splits into an insert half and a resize half | a modifier | the two gestures start on the same pixels, and a modifier for a pointer gesture is worse than a target |
| The boundary band splits into an insert half and a resize half | a separate affordance elsewhere in the strip | the insert lands at the boundary, so an affordance sitting anywhere else has to say which boundary it means, which the boundary itself says for free |

---

## 6. NOT BUILT, AND NOT YET RULED

* **An auto-fit measures the viewport, not the column.** The cache holds the requested window
  alone, so content further down the column does not count and the column comes out too narrow.
  The Logic thread holds the whole column and has no text measurement; nothing states which side
  should own it. Its worst case is `TRAME-UI-128`: measuring nothing is not read as "measure
  later" but as "there is nothing". The column is set to the default width.
* **A resize is sent on the droppable class, and the sender cannot tell** (`TRAME-UI-127`).
  `send_command_lossy` returns nothing, so a full channel is a log line and the local map has
  already taken the width. Every other document mutation of this crate goes out lossless, and the
  paste path reads the error back. `BUS-018` is the shared rule and it is `unimplemented`.
* **A band resize is not one write.** One drag emits one command per column, each on its own, so a
  full channel can land some of the band and drop the rest. The screen shows one width and the
  document holds two. Nothing rules whether a band is one gesture or several.
* **Nothing drives a sized column through a structural change** (`TRAME-UI-037`). The keying makes
  the rule true by construction, which is exactly the claim a test is cheapest to make.
* **Nothing drives a resize at a zoom other than one** (`TRAME-UI-040`). The zoom is the one
  factor that turns a local preference into a shared value.
* **A resize made on a sheet the user has just left is dropped silently.** No event tells the user,
  and no rule says whether the command should have been retargeted or refused.
* **A band resize has no test** (section 2, row 5). One drag emits one command per column, and
  nothing asserts the count or that a mixed selection is left alone.
* **A row that wraps its text has no automatic height.** The height is whatever a user set, so a
  wrapped or multi-line cell is clipped until it is resized by hand
  (`features/ambre-spec-trame-cell-rendering.md section 5`).
