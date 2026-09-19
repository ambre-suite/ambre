# AMBRE -- FEATURE: AUTOFILL

**Type:** Vertical (feature)
**Implementation:** implemented -- the modifier rules are richer than any document stated (section 5).

> A user types 1 and 2, grabs the small square at the corner of the selection and drags down. The
> column continues the series. Dragging a block of text instead repeats it, and its formulas move
> with it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 Drag the fill handle

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-067` | `POST` | A numeric pattern in the source MUST continue across the filled range. | this spec section 3 | `ambre_trame::logic::tests::group5::fill_auto_extrapolates_numeric_series`; `::fill_series_single_cell_increments_by_one`; `group6::fill_merged_series_increments_per_block` | none | verified |
| `TRAME-UI-068` | `POST` | A source with no numeric pattern MUST be repeated verbatim. | this spec section 3 | `ambre_trame::logic::tests::group6::fill_merged_source_tiles_and_remerges_blocks` -- the only text source in the suite, which takes the merged path | partial -- on the plain path nothing fills text: one test forces `Copy` over numbers, the other fills a formula; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-069` | `POST` | A filled formula MUST have its relative references shifted by its own offset from the source. | this spec section 3 | `ambre_trame::logic::tests::group5::fill_auto_copies_text_and_reindexes_formula` | none | verified |
| `TRAME-UI-070` | `POST` | A source tiled by equal merged blocks MUST fill block by block, not cell by cell. | this spec section 3 | `ambre_trame::logic::tests::group6::fill_merged_source_tiles_and_remerges_blocks`; `::fill_merged_series_increments_per_block`; `::fill_multi_merged_source_continues_pattern` | none | verified |
| `TRAME-UI-134` | `POST` | A fill MUST write every cell between the source and the edge of the drag. | this spec section 3 | none | unheld -- both paths walk whole blocks and leave a partial tail unwritten in silence. What keeps the tail empty is the UI snapping the destination, not a guard the engine holds; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-073` | `POST` | A fill MUST be one undo item. | this spec section 3 | none | unheld -- `fill_range` opens one undo group, as the paste does, and the fill tests assert the values only; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-071` | `FRAME` | A fill MUST NOT write outside the rectangle the drag covered. | this spec section 3 | none | unheld -- `fill_range` refuses a destination that does not contain the source and walks only the destination, and no test drives a malformed range; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-072` | `FRAME` | A drag that ends where it started MUST write nothing. | this spec section 4 | none | unheld -- both sides guard it, the release returns early and the engine returns early, and neither guard is asserted; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-074` | `FRAME` | A fill MUST NOT touch a sheet other than the active one. | this spec section 3 | none | unheld -- `fill_range` returns empty when the command names another sheet, which is a guard nothing exercises; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-UI-133` | `FRAME` | A fill whose command did not reach the Logic thread MUST NOT grow the selection over cells it did not write. | `1-Foundations/ambre-spec-message-bus.md` (`BUS-018`) | qa-manual -- DOM glue held by using the application (`1-Foundations/ambre-spec-testing.md section 2`), the early return on the release path, reached by dragging the handle while the outbox refuses | none | qa-manual |

* **Budget.** the 50-cell batch of `ambre-spec-budgets.md section 1`, in < 1 ms: a fill is
  the operation that row was written for. No bench measures it.
* **Boundary.** none of its own: the fill writes cells to the document, and `ZK-001` seals every
  payload before it leaves the client.
* **Verification.** 3 `verified`, 6 `untested`, 1 `qa-manual`. What is proven is what the fill
  produces on the numeric and the merged paths. What is loose is what it refuses, and one claim is
  withdrawn. `TRAME-UI-068` cited two tests that assert something else: one forces `Copy` over a
  numeric source, the other fills a formula (`TRAME-UI-069`). Neither fills text, and every
  remaining gap is a guard that survives a rewrite of the code around it.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| fills `A2:A9` from `A1` | types into `A5` | one value in `A5`, elected by LWW | both grids agree; the later writer wins that cell | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops`, whose op set carries `Fill` and `SetText` |
| fills a column | inserts a row inside the filled range | the filled values on their own rows, split by the new one | both see the series interrupted, not renumbered | the same property, whose op set carries `Fill` and `InsertRow` |
| fills from a merged source | unmerges the source | the fill already written, in blocks | A's filled blocks stay merged; only the source splits | none, and no rule covers it -- section 6 |

---

## 3. MODEL

**Containers.** None of its own. A fill writes `cells_{sheet}`, `cellstyle_{sheet}` and, from a
merged source, `merges_{sheet}` -- all declared in `ambre-spec-trame-schema.md section 3`.

**Commands** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::FillRange { sheet_id, src, dest, mode }` | UI to Logic | the source rectangle, the destination that contains it, and the mode |
| `FillMode` | -- | `Auto` lets the engine choose; `Series` and `Copy` force one |

**View state.** One field of `TrameView`:

```rust
fill_drag: Option<(CellRect, (u32, u32))>,  // the source rectangle, and its block granularity
```

**The fill, in steps:**

1. The release resolves the drag axis: the destination extends the source on one side.
2. It counts the source values along that axis -- blocks, not cells, when the source is merged.
3. It picks the mode: `Auto` without the modifier, otherwise `Series` on a single value and `Copy`
   on several.
4. The Logic thread walks each fill line, extrapolating a numeric series or tiling the source.
5. Each written formula shifts by its own offset; each written cell carries the source style.
6. The selection grows to the whole filled rectangle.

**Code.** `logic/fill.rs` for the fill itself; `ui/view/interact.rs::end_fill_drag` for the gesture
and the mode; `ui/grid/geometry.rs` for the handle hit-test.

---

## 4. SURFACE

| Gesture | Effect |
|---|---|
| hover the handle | the cursor becomes a crosshair |
| drag the handle | a preview rectangle grows along one axis |
| release | the fill is sent, and the selection covers the result |
| hold the modifier at release | inverts the mode the engine would have chosen |

The handle is a small square at the bottom-right corner of the selection. On a merged block it
sits at the corner of the whole block (`features/ambre-spec-trame-cell-merges.md section 4`). The
modifier is read at release, not at press, so changing your mind mid-drag counts.

**Rendering.**

| What | Drawn as |
|---|---|
| the handle | a filled square at the selection corner |
| the drag | an outline over the destination, source included |
| the result | the ordinary selection outline over the filled rectangle |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The engine picks between a series and a copy | a mode chosen in the UI | it is what the modifier already does explicitly, and generalising it would mean shipping the pattern detector twice, or shipping the viewport's opinion of the source. The engine is the only side that holds the values, and the choice depends on them. |
| The modifier counts values along the drag axis | counting the source cells | it reads the wrong axis on any rectangle: a vertical drag fills column lines, so a source two rows tall carries two values whatever its width, and counting raw cells would call a wide single-row source "several values" and refuse to extrapolate it. |
| A merged source counts blocks | tiling by raw cells | it tears the merge geometry: a block is one value to the user, so a single block dragged down means "continue this one value", and counting its cells would make one block look like several values. |
| The fill is one command | a batch of cell writes assembled in the UI | one drag is one gesture and must be one undo item; a batch also hides the whole destination from the engine, which needs it before deciding. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The modifier rules were never written down.** The whole choice between `Series` and `Copy`, its
  axis, and its block counting live in one comment in `end_fill_drag`. The only claim the corpus
  carried is that the Logic thread chooses, which is true and leaves the user-visible half unstated.
  Section 5 above is the first record of it, and `TRAME-UI-067` and `-068` are the first rules.
* **A fill that races an unmerge.** The two-peer table's last row: the blocks are already written
  when the source splits, and nothing says whether they should follow.
* **A date series.** A detected date pattern has been promised in the corpus and is not built: the
  engine extrapolates numbers only, and no row above claims otherwise. It lands with the
  `A fill extrapolates a date series` Story (`ambre-roadmap.md` E6).
* **The gesture has no test.** The mode choice reads the pointer and the modifier, so it sits in
  `TrameView`, on the untestable side of the seam. It is the same wall as the edit session
  (`features/ambre-spec-trame-cell-editing.md section 6`). The destination geometry does not: it is
  a free function that takes the source, the drag corner and the block granularity and returns a
  rectangle. Nothing but habit keeps it untested.
* **A fill the channel dropped still moves the selection.** `TRAME-UI-133`: the release logs the
  full channel and grows the selection over the destination anyway. The user is left selecting a
  rectangle no peer ever wrote. The resize drops the same way (`features/ambre-spec-trame-resize.md
  section 6`), and there the stale value is at least visible. Here the cells stay as they were and
  only the outline lies.
* **A tail the engine leaves empty.** `TRAME-UI-134`: both paths walk whole blocks. A
  destination whose height is not a multiple of the block writes nothing on the remainder and
  reports no error. What keeps that from happening is the UI snapping the drag to whole blocks, not
  a guard the engine holds. A second caller would find the hole.
