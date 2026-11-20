# AMBRE -- FEATURE: THE PAGE VIEWPORT

**Type:** Vertical (feature)

> A document is a centred column of pages the reader scrolls through and zooms. Only the pages on
> screen and their two neighbours are ever built, so a three-hundred-page document costs the same
> frame as a one-page one.

The scroll offset and the zoom are local view state: they write nothing to the document and reach
no collaborator. There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-007` | `INV` | Only the visible pages plus one on each side MUST be built for a frame; the whole document MUST NOT be walked per frame. | this spec section 2 | `ambre_prose::view::column::tests::a_frame_builds_the_visible_pages_and_one_more_on_each_side`; `ambre_prose::view::paint::tests::a_long_document_builds_only_the_pages_around_the_canvas` | none | verified |
| `PROSE-UI-021` | `INV` | The visible page range MUST NOT be inverted, for every scroll offset and every layout, an empty one included. | this spec section 2 | `ambre_prose::view::column::tests::the_visible_range_is_never_inverted_and_names_only_pages_the_layout_has` | none | verified |
| `PROSE-UI-123` | `INV` | The visible page range MUST NOT name a page the layout does not have. | this spec section 2 | `ambre_prose::view::column::tests::the_visible_range_is_never_inverted_and_names_only_pages_the_layout_has` | none | verified |
| `PROSE-UI-022` | `INV` | Every document position inside the column MUST resolve to a page; only a layout with no page at all MUST resolve to none. | this spec section 2 | `ambre_prose::view::column::tests::every_position_resolves_to_a_page_unless_there_is_none`; `ambre_prose::view::column::tests::a_layout_with_no_page_resolves_nothing` | none | verified |
| `PROSE-UI-124` | `INV` | A position falling in a gutter MUST resolve to the nearer of the two page edges it lies between. | this spec section 2 | `ambre_prose::view::column::tests::a_position_in_a_gutter_goes_to_the_nearer_page_edge` | none | verified |
| `PROSE-UI-023` | `INV` | A point of a page and its screen point MUST map to each other: converting one to the other and back MUST return the point it started from, up to floating-point rounding. | this spec section 2 | `ambre_prose::view::column::tests::a_page_point_and_its_screen_point_map_to_each_other` -- both ways, at every zoom, offset and width, a page wider than the canvas included. A point in a gutter resolves to a page edge first (`PROSE-UI-124`) | none | verified |
| `PROSE-UI-024` | `INV` | Zoom MUST stay inside a quarter to four times, whatever the input. | this spec section 2 | `ambre_prose::view::zoom::tests::a_zoom_stays_in_range_and_keeps_the_offset_in_the_column` -- any factor asked, a NaN, an infinity and zero included | none | verified |
| `PROSE-UI-025` | `INV` | The scroll offset MUST stay between the top of the column and its last scrollable position, after a scroll, a zoom, a relayout or an autoscroll tick. | this spec section 2 | `ambre_prose::input_tests::the_offset_stays_in_the_column_after_every_path` -- a scroll, a zoom, a zoom about a point, a relayout and four hundred autoscroll ticks held past the end; `ambre_prose::view::zoom::tests::a_zoom_stays_in_range_and_keeps_the_offset_in_the_column`; `ambre_prose::view::tests::a_shorter_layout_brings_the_offset_back_inside_the_column` | none | verified |
| `PROSE-UI-026` | `INV` | The page column MUST stay centred at every zoom and every viewport width. | this spec section 2 | `ambre_prose::view::zoom::tests::the_column_stays_centred_or_pinned_at_every_zoom` | none | verified |
| `PROSE-UI-125` | `INV` | A page wider than the viewport MUST leave no gap on either side: its left edge MUST lie between zero and the viewport's width less the page's. | this spec section 2 | `ambre_prose::view::zoom::tests::a_wide_page_is_reachable_edge_to_edge`; `ambre_prose::view::zoom::tests::the_column_stays_centred_or_pinned_at_every_zoom` | none | verified |
| `PROSE-UI-251` | `INV` | The horizontal offset MUST range over the whole width of a page wider than the viewport, from its left edge to the offset showing its right edge, never past either. | this spec section 2 | `ambre_prose::view::zoom::tests::a_wide_page_is_reachable_edge_to_edge` -- a scroll past either edge stops at it, at every zoom and width | none | verified |

### 1.2 Scroll the column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-126` | `POST` | A scroll MUST move the column by the distance the gesture asks for, up to the clamp. | this spec section 3 | `ambre_prose::view::tests::a_scroll_moves_by_the_gesture_up_to_the_ends_and_nothing_else` | none | verified |
| `PROSE-UI-127` | `FRAME` | A scroll MUST NOT change the zoom, MUST NOT rebuild the column and MUST NOT write to the document. | derived | `ambre_prose::view::tests::a_scroll_moves_by_the_gesture_up_to_the_ends_and_nothing_else` | none | verified |
| `PROSE-UI-211` | `POST` | A frame whose built pages the layout in force does not carry MUST ask the Logic thread once for those pages and a margin around them. | this spec section 2 | `ambre_prose::view::tests::a_scroll_asks_for_the_pages_around_the_canvas` | none | verified |
| `PROSE-UI-217` | `POST` | A flick released with speed MUST keep the column moving, each frame slower than the last, until it stops or meets an end of the column, where it MUST stop. | this spec section 3 | `ambre_prose::input_tests::a_flick_slows_down_and_stops_at_the_end_of_the_column` | none | verified |
| `PROSE-UI-218` | `FRAME` | A press, a key, a new scroll or a new finger MUST stop the motion a flick left, and the motion MUST NOT write to the document. A layout that arrives MUST NOT stop it: the pages a scroll asks for would. | derived | `ambre_prose::input_tests::a_press_stops_a_flick_and_the_motion_writes_nothing` | none | verified |
| `PROSE-UI-212` | `POST` | Revealing a block the layout in force does not carry MUST ask for its pages, then scroll to it once a layout carries it. | this spec section 2 | `ambre_prose::view::tests::revealing_a_block_not_carried_asks_for_it_then_shows_it` | none | verified |

### 1.3 Zoom the page

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-128` | `POST` | A zoom about a point MUST leave the document point that was under it there, as far as the clamp of `PROSE-UI-025` and the centring of `PROSE-UI-026` let it. | this spec section 2 | `ambre_prose::view::zoom::tests::a_zoom_about_a_point_keeps_the_point_under_it` -- on each axis the clamp leaves free, whatever factor is asked; `ambre_app::host::prose::tests::a_ctrl_wheel_zooms_the_page_about_the_pointer` -- wasm tier, a wheel held with `Ctrl` keeps the point within half a pixel | none | verified |
| `PROSE-UI-129` | `FRAME` | A zoom MUST NOT move the caret, MUST NOT change the selection and MUST NOT write to the document: the factor, the column's left edge and the scroll offset are all it moves. | this spec section 2 | `ambre_prose::input_tests::a_zoom_key_steps_the_zoom_and_moves_nothing_else` -- the caret, the selection and the commands sent, across four zoom keys | none | verified |
| `PROSE-UI-250` | `POST` | A preset picked on the zoom chip MUST zoom the page to it. A zoom key MUST move the zoom to the next preset above or below it, or back to a hundred per cent. | this spec section 3; `1-Foundations/ambre-spec-chrome.md` | `ambre_prose::view::zoom::tests::a_step_moves_one_preset_and_stops_at_the_ends` -- each step and both ends of the ladder; `ambre_app::host::prose::tests::the_zoom_chip_closes_the_band_and_zooms_the_page` -- wasm tier, a preset picked zooms the page, and a key moves the chip | none | verified |

### 1.4 Resize the canvas

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-130` | `FRAME` | A canvas resize MUST NOT change the scroll offset and MUST NOT change the zoom. | this spec section 3 | `ambre_prose::view::tests::a_resize_keeps_the_offset_and_the_zoom` | none | verified |

### 1.5 Rebuild the column from a new layout

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-131` | `POST` | A layout that arrives MUST rebuild the column: the page count, the page box and the total height MUST come from that layout. | this spec section 2 | `ambre_prose::view::tests::a_newer_layout_replaces_the_one_in_force_and_rebuilds_the_column` | none | verified |
| `PROSE-UI-132` | `FRAME` | Rebuilding the column MUST NOT change the zoom and MUST NOT write to the document. | derived | `ambre_prose::view::tests::applying_a_layout_keeps_the_zoom_shapes_nothing_and_writes_nothing` | none | verified |

* **Budget.** the scroll of one page, a row of `ambre-spec-budgets.md section 1`. The culling costs
  nothing worth a row: the visible range is arithmetic over a uniform page height, not a search.
* **Boundary.** none: the scroll offset and the zoom are view state on the UI thread, where
  `UI-THREAD-004` bans a network, crypto or persistence path. `PROSE-UI-010`
  (`features/ambre-spec-prose-engine.md section 1`) keeps both out of the document.
* **Verification.** 23 `verified`. The state rows are property tests over the column in
  `crates/ambre-prose`, being arithmetic over a few numbers; the others are host tests of the view.
  The zoom about the pointer is also held at the wasm tier. The flick rows drive the ticks by hand,
  as the host does once a frame.

---

## 2. MODEL

**Containers.** None. Nothing here is persisted and nothing is shared.

**The column.** Pages of one size, stacked with a fixed gutter between them, centred horizontally.
The column is rebuilt from the layout on every layout that is applied.

```rust
/// The geometry of the page column, derived from the page layout. Owns which pages are
/// visible, where each page starts, and the inverse of that mapping.
pub struct PageColumn {
    /// The page trim width, which is what the column is centred on.
    pub page_width:      Px<Doc>,
    pub page_height:     Px<Doc>,
    pub gutter:          Px<Doc>,
    /// Zero for a layout that has not arrived yet.
    pub page_count:      Len<Page>,
    pub total_height:    Px<Doc>,
}

impl PageColumn {
    /// The pages intersecting the viewport. `None` for an empty layout, and never
    /// inverted for any scroll offset (`PROSE-UI-021`).
    pub fn visible_pages(&self, viewport: &Viewport) -> Option<Span<Page>>;
    /// Where a page starts, measured from the top of the column.
    pub fn page_top_y(&self, page: Pos<Page>) -> Px<Doc>;
    /// The inverse: the page a document Y belongs to, and the Y within that page. A Y in
    /// a gutter is assigned to the nearer page edge (`PROSE-UI-022`). `None` only for an
    /// empty layout.
    pub fn doc_y_to_page(&self, doc_y: Px<Doc>) -> Option<(Pos<Page>, Px<Doc>)>;
    /// The left edge of the column on the canvas: centred when the page fits
    /// (`PROSE-UI-026`), at the horizontal offset when it does not (`PROSE-UI-125`).
    pub fn left(&self, viewport: &Viewport) -> Px<View>;
}
```

**The camera** (`ambre-prose`):

```rust
pub struct Viewport {
    /// From the top of the column, in document pixels.
    pub scroll_y:     Px<Doc>,
    /// How far the canvas's left edge sits past a page wider than it, in document pixels.
    /// Zero while the page fits, which centres it (`PROSE-UI-251`).
    pub scroll_x:     Px<Doc>,
    /// The canvas, in screen pixels.
    pub width:        Px<View>,
    pub height:       Px<View>,
    /// The user's zoom, one at a hundred per cent (`PROSE-UI-024`).
    pub zoom:         Scale<Doc, View>,
}
```

**Screen to document, and back.** The conversion inverts exactly what the renderer applies
(`PROSE-UI-023`): subtract the column's left margin, divide by the zoom, add the scroll offset.
The result is resolved to a page before it is handed on, so a caller indexes the layout without
re-deriving which page it is on.

```rust
impl PageColumn {
    /// A canvas point as a position in the document.
    pub fn screen_to_doc(&self, viewport: &Viewport, at: Point<Px<View>>) -> DocPos;
    /// A point of a page as a canvas point, the inverse (`PROSE-UI-023`).
    pub fn doc_to_screen(&self, viewport: &Viewport, page: Pos<Page>, x_in_page: Px<Doc>, y_in_page: Px<Doc>) -> Point<Px<View>>;
}

pub enum DocPos {
    OnPage { page: Pos<Page>, x_in_page: Px<Doc>, y_in_page: Px<Doc> },
    /// The layout has no page at all.
    Empty,
}
```

`DocPos::Empty` is what a caller answers a click on an empty document with. It is a distinct
answer from a click that lands in a page margin. The first has no page, the second has a page and
no block (`features/ambre-spec-prose-page-rendering.md`).

**Culling.** A block is built for a frame when the page it starts on is in the visible range, plus
one page on each side (`PROSE-UI-007`). A block outside that range is neither shaped nor
rasterized; its runs stay cached and unbuilt.

**Window.** The view holds the pages the Logic thread last handed, not the document
(`PROSE-125`). The page count and the page box stay the document's, so the column and the scroll
range are whole. A frame whose built pages fall outside the window asks for them, four pages of
margin on each side (`PROSE-UI-211`). A page not carried yet draws as a blank sheet until the
answer comes.

A block revealed off the window is asked for by its id, and the view scrolls to it
once a layout carries it (`PROSE-UI-212`). The flow order and the two ends of the text stay
global, so a selection of the whole document holds without its blocks.

**Zoom.** The chip and the keys pick a factor on a ladder of nine presets, from a quarter to four
times (`PROSE-UI-250`). A key and the chip keep the line at the top of the canvas in place.

A wheel held with `Ctrl`, a trackpad pinch included, zooms about the pointer
(`PROSE-UI-128`). The factor comes from the one engine, clamped (`PROSE-UI-024`). The engine's
transform solves both offsets that keep the point under the pointer, then both are clamped
(`PROSE-UI-025`). A page that fits recentres (`PROSE-UI-026`), so only a page wider than the canvas
keeps the point sideways. It writes nothing to the document.

A page wider than the canvas scrolls sideways under the horizontal half of the wheel, from edge to
edge (`PROSE-UI-251`). A page that fits has no horizontal offset.

```rust
impl ProseUiState {
    /// Zoom about a screen point, through `ambre_core::Zoom::about`.
    pub fn zoom_about(&mut self, asked: Scale<Doc, View>, at: Point<Px<View>>) -> ZoomChange;
    /// Move a page wider than the canvas sideways, up to its edges.
    pub fn scroll_across(&mut self, dx: Px<View>);
    /// Show the pages at a factor, clamped to the ladder, the top of the canvas kept.
    pub fn zoom_to(&mut self, zoom: Scale<Doc, View>) -> ZoomChange;
    /// One preset up or down, or back to a hundred per cent (`PROSE-UI-250`).
    pub fn step_zoom(&mut self, step: ZoomStep) -> ZoomChange;
}
```

**Scroll.** Every path that moves the scroll offset clamps it to the column's own range afterwards
(`PROSE-UI-025`). Those paths are the wheel, a drag past the viewport edge, a caret the keyboard
has to reveal, and a relayout that shortened the document. A relayout is the case that matters.
Deleting the last twenty pages leaves an offset past the end of a column that no longer has one.

**Code:** `crates/ambre-prose/src/view/column.rs` holds the column and the viewport;
`view/mod.rs` the scroll, the resize and the layout that rebuilds the column. `view/zoom.rs` holds
the ladder, the clamp and the step.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the wheel, a two-finger glide | scrolls the column vertically, and a page wider than the canvas sideways (`PROSE-UI-251`) |
| a finger dragged on the page | scrolls the column, and a flick keeps it moving (`PROSE-UI-217`), on the shared model of `1-Foundations/ambre-spec-kinetic-scroll.md`; a finger that barely moves is a tap |
| a drag past the top or bottom edge | scrolls while the drag continues (`features/ambre-spec-prose-selection.md`) |
| a caret move that leaves the viewport | scrolls the minimum needed to bring the caret back, with a margin |
| `Ctrl` with `=`, `-` or `0` | zooms in, zooms out, returns to a hundred per cent (`PROSE-UI-250`) |
| a preset of the zoom chip, at the end of the status bar | zooms to that preset (`PROSE-UI-250`, `CHROME-147`) |
| `Ctrl` with the wheel, a trackpad pinch | zooms about the pointer (`PROSE-UI-128`) |
| a canvas resize | keeps the offset and the zoom, recentres the column, re-culls |
| a layout that changed the page count | rebuilds the column, then clamps the offset (`PROSE-UI-025`) |

The zoom control itself is chrome, not canvas, and lives with the rest of the application chrome.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Pages are a uniform column with a fixed gutter | a per-page offset table | it costs a binary search per frame for a document that has no section break, which is the only thing that would eventually need it. Uniform, the column's geometry is three numbers and the visible range is a division, and a scroll offset stays meaningful across a relayout because only the count changes |
| A gutter position snaps to the nearer page edge | returning no page for a gutter | it is what made a click between pages resolve to nothing. A pointer between two pages is a real event with no correct answer in the gutter itself, so returning nothing pushes the decision into every caller and each invents a different fallback |
| An empty layout is a first-class case | assuming at least one page | the engine guarantees it for a live document and cannot guarantee it before the first layout, and a document opens before its first layout arrives. Answering the empty column explicitly is cheaper than an underflow at every call site |
| The scroll offset is clamped after the fact, never assumed valid | validating the offset where it is set | four independent paths move it and a fifth changes what valid means. Clamping at each of the four leaves the fifth uncovered, and a relayout is exactly the fifth |
| Zoom is anchored on a point, not on the column centre | zooming about the viewport centre | it moves what the reader was looking at off screen, and they scroll to find it again. It is what a page-fit control does, and that is a different gesture |
| The zoom range is a quarter to four | an open-ended zoom | it produces two unusable states with no affordance to leave them: below a quarter a page of body text is unreadable and the reader is looking at a thumbnail we do not draw, and above four a line no longer fits the width, which is where zoom stops helping and reflow would be needed instead |
| The camera is applied locally and the culling follows | routing the scroll through the engine | it puts a round trip inside every scroll. Scrolling is direct manipulation and the content already on screen only needs a different offset to be correct; a virtually scrolled grid routes it because its content is fetched by index |
| Only what is on screen is built | painting the whole document each frame | it makes a long document unusable rather than slow: a hundred pages rebuilt per frame is the frame budget spent many times over, and the reader is looking at one of them. Culling by page also makes the cost of a document independent of its length |
| One page of overscan on each side | building only the visible pages | it flashes on a fast scroll. A scroll reveals at most a page before the next frame, so one page of overscan covers the reveal, and two doubles the build cost for a case that does not occur |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The anchor on the caret is not built.** A key and the chip keep the top of the canvas in place.
  A touch pinch is not built either (`1-Foundations/ambre-spec-ui.md`, `UI-THREAD-013`).
* **The page frames no selection.** No command asks for the selection to fill the canvas, so
  `ZOOM-007` holds on the board and the grid alone (`1-Foundations/ambre-spec-zoom.md`).
* **The zoom does not survive a reload.** It is view state and nothing writes it anywhere. A
  reader who zooms to a hundred and fifty per cent does it again on the next visit.
* **No scroll bar shows the horizontal offset.** The wheel reaches a wide page edge to edge, and
  nothing on screen says how much of it lies out of view.
* **A pageless mode would replace this column.** `meta/pageless` is a container the schema carries
  as deferred. With it set, the layout is linear, the gutters go and the width follows the
  viewport. That is a different geometry, not a variant of this one.
