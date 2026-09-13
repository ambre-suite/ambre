# AMBRE -- FEATURE: THE STATUS BAR

**Type:** Vertical (feature)
**Implementation:** partial -- the band and the aggregates ship; nothing drives the refresh under test.

> A user selects a handful of elements and reads how many there are and what box they occupy,
> without drawing a rectangle to find out. A thin band under the board shows the tally, the combined
> bounds and the current zoom.

The band holds no state of its own: it displays an answer the Logic thread computes and forgets.
That is why it appears in no history and reaches no collaborator.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-053` | `OBS` | The band MUST show how many elements are selected. | this spec section 3 | `ambre_toile::ui::status::tests::multi_element_shows_count_and_bounds` on the `3 elements` caption and `::single_element_shows_count_only` on the `1 element` one. The third test this row cited asserts neither caption | verified |
| `TOILE-UI-106` | `OBS` | The combined bounds MUST appear only from two elements up: a lone element MUST show its tally and nothing else. | this spec section 3 | `ambre_toile::ui::status::tests::single_element_shows_count_only`, which hands the band a one-element reply that does carry a rectangle and asserts the bounds caption is absent. It is the half `TOILE-UI-053` was carrying | verified |
| `TOILE-UI-107` | `OBS` | An empty selection MUST leave the band empty, never a zero. | this spec section 3 | `ambre_toile::ui::status::tests::empty_selection_shows_nothing` on the absent tally caption: the tally is nothing at zero rather than a zero, and the row it was cited under said only that a tally is shown | verified |
| `TOILE-UI-054` | `OBS` | A world value MUST read with no decimal when it is whole and with one otherwise. | this spec section 3 | `ambre_toile::ui::status::tests::fractional_world_values_get_one_decimal` on the fractional half and `::multi_element_shows_count_and_bounds` on the whole one. The stable width the row claimed does not follow: a number widens with its magnitude, and only the decimals are held here | verified |

### 1.2 Ask for the aggregates

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-020` | `POST` | The aggregates MUST cover every selected element, including those outside the viewport. | this spec section 2 | `ambre_toile::logic::convergence_tests::selection_stats_spans_offscreen_elements`, on a union spanning two elements a thousand units apart. The viewport half is structural rather than asserted: the Logic thread holds no viewport, so the walk has nothing to cull against. The two other tests this row cited answer a different request | verified |
| `TOILE-116` | `POST` | An empty id list MUST ask for the whole board's extent, which is what a fit reads. | this spec section 2 | `ambre_toile::logic::convergence_tests::selection_stats_empty_ids_span_whole_board`, and the rule lived in the command table alone. An empty list is the one input that does not mean nothing | verified |
| `TOILE-117` | `POST` | A tally of zero MUST come back with no rectangle. | this spec section 2 | none. `ambre_toile::logic::convergence_tests::selection_stats_empty_ids_on_empty_board_is_none` drives the empty board alone, where the walk takes the whole-board branch; a selection whose every id the board has dropped tallies zero down the other branch, and nothing drives it | unverified |
| `TOILE-UI-108` | `POST` | An answer MUST reach the request that asked for it. | this spec section 2 | none, and **the rule fails today**: the event carries no token, and four surfaces ask -- the band on every selection change, `Fit`, zoom-to-selection and the find focus. The view spends a pending fit on whichever answer lands first, so a band refresh in flight when a user hits `Fit` frames the selection instead of the board -- section 5. Answered by the `An answer names the request that asked for it` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-118` | `FRAME` | An id the board no longer holds MUST NOT stop the walk: the tally answers for the elements that remain. | this spec section 2 | none. The walk skips an id the bounds cache misses, and no test mixes a live id with a stale one, so a walk that stopped at the miss or counted it would pass | unverified |
| `TOILE-UI-056` | `FRAME` | A request for the aggregates MUST emit no delta. | this spec section 2 | type-level: the handler returns a display-only outcome, whose sync half is empty by construction, so no path exists for a delta to leave | verified |

### 1.3 Follow the selection

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-055` | `POST` | A change of selection MUST ask for fresh aggregates. | this spec section 2 | none. The comparison and the request run once per frame on the view, and no test drives a selection change through it | unverified |
| `TOILE-UI-109` | `POST` | A selection that becomes empty MUST clear the band, and MUST ask for nothing. | this spec section 2 | none, and it is the half `TOILE-UI-055` was carrying. The branch is not a shortcut: passing the empty selection on would read as the whole board (`TOILE-116`) and the band would report a selection of everything. Nothing drives either branch | unverified |
| `TOILE-UI-110` | `FRAME` | A frame in which the selection did not change MUST ask for nothing. | this spec section 2 | none. The per-frame comparison is the only thing between a growing marquee and one walk over the whole selection per frame, and no test runs two frames on one selection | unverified |

* **Budget.** none: the tallies are one walk over the selection against a cache the engine already
  maintains, and `ambre-spec-budgets.md` budgets no aggregate. The band paints inside
  the frame phases of section 5.B of that document.
* **Boundary.** none: the band reads and shows. It writes nothing to the document, opens no path, and
  `UI-THREAD-004` bans a network, crypto or persistence path on the thread it paints on.
* **Verification.** 7 `verified`, 6 `unverified`. Five rows become thirteen, and the coverage did
  not move: no assertion was added, and the seven tests the document already cited are the seven it
  cites now. What changed is what they are held to. `TOILE-020` stacked three tests under one
  verdict, and two of them answer a different request: an empty id list means the whole board
  (`TOILE-116`), and a zero tally comes back without a rectangle (`TOILE-117`) down a branch the
  test never takes. `TOILE-UI-053` was the tally and the two-element threshold under one verdict
  (`TOILE-UI-106`), and neither said what an empty selection shows (`TOILE-UI-107`).
  `TOILE-UI-055` was the refresh and its opposite branch (`TOILE-UI-109`), and neither is driven.
  Three rules were unwritten: the frame that asks for nothing (`TOILE-UI-110`), the stale id the
  walk skips (`TOILE-118`), and the answer that reaches whoever asked -- which fails today
  (`TOILE-UI-108`).

---

## 2. MODEL

**Containers.** None. The aggregates are derived on each request and stored nowhere.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::RequestSelectionStats { ids }` | UI to Logic | the selected ids. An empty list asks for the whole board, which is what a fit reads (`TOILE-116`, `features/ambre-spec-toile-viewport.md`) |
| `ToileEvent::SelectionStats { count, bounds }` | Logic to UI | the tally and the union rectangle, in world units; the rectangle is absent at a tally of zero (`TOILE-117`), and nothing on the event says which request it answers (`TOILE-UI-108`) |

**Display shape** (`ambre-toile`, the chrome side):

```rust
pub struct SelStats {
    pub count:  u32,
    pub bounds: Option<Bounds>,   // x, y, w, h in world units; None when nothing is selected
}
```

**Steps, when the Logic thread answers a request:**

1. read each id's bounds out of the geometry cache the engine maintains, skipping an id the board no
   longer holds (`TOILE-118`);
2. tally the ones found, and union their rectangles;
3. answer the tally and the union, with no rectangle when the tally is zero (`TOILE-117`);
4. return a display-only outcome, so nothing reaches the Sync thread (`TOILE-UI-056`).

**Steps, on the UI side:**

1. once per frame, compare the live selection with the one last asked about, and stop there when
   it has not moved (`TOILE-UI-110`);
2. on a change, clear the band when the selection is empty (`TOILE-UI-109`), and ask for fresh
   aggregates otherwise (`TOILE-UI-055`);
3. the reply is mirrored into one signal the band reads, and into the pending fit when one is
   waiting -- whichever request that reply came back from (`TOILE-UI-108`).

**Code:**

* `crates/ambre-toile/src/logic/state/mod.rs` -- the walk and the union;
* `ui/status.rs` -- the display shape and the two captions;
* `ui/view.rs` -- the per-frame comparison and the request;
* `chrome/status.rs` -- the band itself, over the shared toolkit.

---

## 3. SURFACE

A thin band along the bottom edge. It is Toile's parameterization of the shared band specified in
`ambre-spec-chrome.md`, which owns the slots below.

| Slot | Holds |
|---|---|
| the label | the application name |
| the tally | the number of selected elements, absent on an empty selection (`TOILE-UI-053`) |
| the bounds | the combined origin and extent, shown from two elements up (`TOILE-UI-106`) |
| the zoom chip | the shared zoom model (`ambre-spec-zoom.md`), rendered by the chrome chip (`ambre-spec-chrome.md`, `CHROME-014`) |
| the frame chip | the frame rate, expanding on click into the frame timings |

**How a number reads.** A whole number shows with no decimals, any other with one, so the band keeps
a stable width as the selection changes (`TOILE-UI-054`).

**What the user does not see.** A lone element shows its tally and no box, because its own selection
box already says where it is (`TOILE-UI-106`), and an empty selection shows neither, rather than a
zero (`TOILE-UI-107`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The Logic thread computes the aggregates | unioning the visible bounds on the UI thread | it answers for the visible part alone, and a selection can be far larger than the viewport. Sending the elements up instead would move the whole selection across a channel to compute two numbers |
| An empty id list means the whole board | a second command for the board extent | it is a second walk to keep in step with the first, over the same cache. A fit needs the extent of a board the UI has never seen, and one command answers both questions |
| The band shows no box for a lone element | showing the bounds from one element up | the element already wears a selection box, so the numbers would repeat what the screen says, and the band would resize on every click |
| A world value carries at most one decimal | the full value | the digit count changes with the value, so the band jitters in width under a moving selection |
| A world value carries at most one decimal | a fixed two decimals | the width is stable but the second digit is noise: board coordinates are coarse, and it moves without telling the user anything |
| The request carries the ids | letting the Logic thread track the selection | it would give two sources for one list. The selection is the UI thread's and it is the only place that knows it, unlike a grid where the Logic thread already tracks a rectangle for its own commands |
| The zoom chip is shared with the spreadsheet | a board-specific zoom read-out | it is the same control over the same kind of value, and two implementations would drift in their preset ladder |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The refresh has no test** (`TOILE-UI-055`, `TOILE-UI-109`, `TOILE-UI-110`). The comparison runs
  on the frame hook, and the two branches under it -- clear, or ask -- are reached by nothing, while
  the walk below and the captions above are both covered. It is a comparison of two lists on the
  view, so the host tier reaches it.
* **Any answer satisfies any request** (`TOILE-UI-108`). Four surfaces send the same command -- the
  band on every selection change, `Fit`, zoom-to-selection and the find focus -- and the event that
  comes back names neither the ids nor the sender. The view spends a pending fit on the first reply
  to land, so a band refresh still in flight when the user hits `Fit` frames the selection and the
  board's own extent, which arrives next, is dropped. The band pays the mirror image: a fit's answer
  for the whole board is mirrored into it as though it were the selection. A request id on the
  command, echoed on the event, is the fix, and nothing in the workspace carries one.
* **A request per selection change is unbounded.** A marquee that grows element by element asks once
  per frame it changes in, each answer a walk over the whole selection. Nothing debounces it and no
  budget covers it.
* **A dangling id is dropped in silence** (`TOILE-118`). An id the board no longer holds is skipped,
  so the tally answers for what still exists. Nothing states whether that is the intent, and a stale selection
  reads as a smaller one rather than as an error.
* **The band shows no geometry of a single element.** A user selecting one element sees a tally of
  one and nothing else, where its own origin and size are what a single selection usually asks for.
* **No aggregate is selectable or copyable.** A user cannot lift a number out of the band.
