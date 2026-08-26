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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-020` | The aggregates MUST be computed by the Logic thread over the whole selection, including elements outside the viewport. | this spec section 2 | `ambre_toile::logic::convergence_tests::selection_stats_spans_offscreen_elements`, `::selection_stats_empty_ids_span_whole_board`, `::selection_stats_empty_ids_on_empty_board_is_none` | verified |
| `TOILE-UI-053` | The band MUST show the element tally, and the combined bounds only where a union of two or more is meaningful. | this spec section 3 | `ambre_toile::ui::status::tests::empty_selection_shows_nothing`, `::single_element_shows_count_only`, `::multi_element_shows_count_and_bounds` | verified |
| `TOILE-UI-054` | A world value MUST read with no decimal when it is whole and with one otherwise, so the band keeps a stable width. | this spec section 3 | `ambre_toile::ui::status::tests::fractional_world_values_get_one_decimal`, `::multi_element_shows_count_and_bounds` | verified |
| `TOILE-UI-055` | The band MUST follow the selection: a change MUST ask for fresh aggregates, and an empty selection MUST clear it. | this spec section 2 | none. The comparison and the request run once per frame on the view, and no test drives a selection change through it | unverified |
| `TOILE-UI-056` | A request for the aggregates MUST emit no delta. | this spec section 2 | type-level: the handler returns a display-only outcome, whose sync half is empty by construction, so no path exists for a delta to leave | verified |

* **Budget.** none: the tallies are one walk over the selection against a cache the engine already
  maintains, and `ambre-spec-budgets.md` budgets no aggregate. The band paints inside
  the frame phases of section 5.B of that document.
* **Boundary.** none: the band reads and shows. It writes nothing to the document, opens no path, and
  `UI-THREAD-004` bans a network, crypto or persistence path on the thread it paints on.
* **Verification.** 4 `verified`, 1 `unverified`. The Logic half is held three ways and the
  read-only guarantee holds by construction. The one gap is the join between the two: what asks for
  a refresh, and when.

---

## 2. MODEL

**Containers.** None. The aggregates are derived on each request and stored nowhere.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::RequestSelectionStats { ids }` | UI to Logic | the selected ids. An empty list asks for the whole board, which is what a fit reads (`features/ambre-spec-toile-viewport.md`) |
| `ToileEvent::SelectionStats { count, bounds }` | Logic to UI | the tally and the union rectangle, in world units; the rectangle is absent on an empty board |

**Display shape** (`ambre-toile`, the chrome side):

```rust
pub struct SelStats {
    pub count:  u32,
    pub bounds: Option<Bounds>,   // x, y, w, h in world units; None when nothing is selected
}
```

**Steps, when the Logic thread answers a request:**

1. read each id's bounds out of the geometry cache the engine maintains, skipping an id the board no
   longer holds;
2. tally the ones found, and union their rectangles;
3. answer the tally and the union, with no rectangle when the tally is zero;
4. return a display-only outcome, so nothing reaches the Sync thread (`TOILE-UI-056`).

**Steps, on the UI side:**

1. once per frame, compare the live selection with the one last asked about;
2. on a change, clear the band when the selection is empty, and ask for fresh aggregates otherwise
   (`TOILE-UI-055`);
3. the reply is mirrored into one signal the band reads, and into the pending fit when one is
   waiting.

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
| the bounds | the combined origin and extent, shown from two elements up |
| the zoom chip | the shared zoom model (`ambre-spec-zoom.md`), rendered by the chrome chip (`ambre-spec-chrome.md`, `CHROME-014`) |
| the frame chip | the frame rate, expanding on click into the frame timings |

**How a number reads.** A whole number shows with no decimals, any other with one, so the band keeps
a stable width as the selection changes (`TOILE-UI-054`).

**What the user does not see.** A lone element shows its tally and no box, because its own selection
box already says where it is.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The Logic thread computes the aggregates | a selection can be far larger than the viewport, and the UI thread holds the culled set alone. Sending the elements up to tally them would move the whole selection across a channel to compute two numbers | unioning the visible bounds on the UI thread, which answers for the visible part alone |
| An empty id list means the whole board | a fit needs the extent of a board the UI has never seen, and it is the same walk over the same cache. One command answers both questions | a second command for the board extent, with a second walk to keep in step |
| The band shows no box for a lone element | the element already wears a selection box, so the numbers would repeat what the screen says, and the band would resize on every click | showing the bounds from one element up |
| A world value carries at most one decimal | board coordinates are coarse and the band sits under a moving selection; more precision makes the width jitter without telling the user anything | the full value, or a fixed two decimals |
| The request carries the ids | the selection is the UI thread's, and it is the only place that knows it, unlike a grid where the Logic thread already tracks a rectangle for its own commands | letting the Logic thread track the selection, which would give two sources for one list |
| The zoom chip is shared with the spreadsheet | it is the same control over the same kind of value, and two implementations would drift in their preset ladder | a board-specific zoom read-out |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The refresh has no test** (`TOILE-UI-055`). The comparison runs on the frame hook, and the two
  branches under it -- clear, or ask -- are reached by nothing, while the walk below and the captions
  above are both covered. It is a comparison of two lists on the view, so the host tier reaches it.
* **A request per selection change is unbounded.** A marquee that grows element by element asks once
  per frame it changes in, each answer a walk over the whole selection. Nothing debounces it and no
  budget covers it.
* **A dangling id is dropped in silence.** An id the board no longer holds is skipped, so the tally
  answers for what still exists. Nothing states whether that is the intent, and a stale selection
  reads as a smaller one rather than as an error.
* **The band shows no geometry of a single element.** A user selecting one element sees a tally of
  one and nothing else, where its own origin and size are what a single selection usually asks for.
* **No aggregate is selectable or copyable.** A user cannot lift a number out of the band.
