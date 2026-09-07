# AMBRE -- FEATURE: THE PAGE VIEWPORT

**Type:** Vertical (feature)
**Implementation:** design-only

> A document is a centred column of pages the reader scrolls through and zooms. Only the pages on
> screen and their two neighbours are ever built, so a three-hundred-page document costs the same
> frame as a one-page one.

The scroll offset and the zoom are local view state: they write nothing to the document and reach
no collaborator, so there is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-007` | `INV` | Only the visible pages plus one on each side MUST be built for a frame; the whole document MUST NOT be walked per frame. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-021` | `INV` | The visible page range MUST NOT be inverted, for every scroll offset and every layout, an empty one included. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-123` | `INV` | The visible page range MUST NOT name a page the layout does not have. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-022` | `INV` | Every document position inside the column MUST resolve to a page; only a layout with no page at all MUST resolve to none. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-124` | `INV` | A position falling in a gutter MUST resolve to the nearer of the two page edges it lies between. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-023` | `INV` | A document point and a screen point MUST map to each other exactly: converting one to the other and back MUST return the point it started from. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-024` | `INV` | Zoom MUST stay inside a quarter to four times, whatever the input. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-025` | `INV` | The scroll offset MUST stay between the top of the column and its last scrollable position, after a scroll, a zoom, a relayout or an autoscroll tick. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-026` | `INV` | The page column MUST stay centred at every zoom and every viewport width. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-125` | `INV` | The left edge of the column MUST NOT go negative: a page wider than the viewport MUST pin the column to the left edge instead. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.2 Scroll the column

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-126` | `POST` | A scroll MUST move the column by the distance the gesture asks for, up to the clamp. | this spec section 3 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-127` | `FRAME` | A scroll MUST NOT change the zoom, MUST NOT rebuild the column and MUST NOT write to the document. | derived | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Zoom about a point

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-128` | `POST` | A zoom about a point MUST leave the document point that was under it there: the anchor MUST keep its screen position. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-129` | `FRAME` | A zoom MUST NOT move the caret, MUST NOT change the selection and MUST NOT write to the document: the factor, the column's left edge and the scroll offset are all it moves. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.4 Resize the canvas

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-130` | `FRAME` | A canvas resize MUST NOT change the scroll offset and MUST NOT change the zoom. | this spec section 3 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Rebuild the column from a new layout

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-131` | `POST` | A layout that arrives MUST rebuild the column: the page count, the page box and the total height MUST come from that layout. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-132` | `FRAME` | Rebuilding the column MUST NOT change the zoom and MUST NOT write to the document. | derived | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the scroll of one page, a row of `ambre-spec-budgets.md section 1`. The
  culling itself costs nothing worth a row: the visible range is arithmetic over a uniform page
  height, not a search.
* **Boundary.** none: the scroll offset and the zoom are view state on the UI thread, where
  `UI-THREAD-004` bans a network, crypto or persistence path, and `PROSE-UI-010`
  (`features/ambre-spec-prose-engine.md section 1`) keeps both out of the document.
* **Verification.** 17 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`. Six of
  the state rows are the degenerate cases -- an empty layout, a position in a gutter, a scroll past
  the end, a zoom out of range, an inverted range, a page wider than the window -- and each names a
  shape a property test reaches directly, because all six are pure arithmetic over three numbers.
  `PROSE-UI-023` and `PROSE-UI-128` are round trips over generated points, which is the same
  mechanism at a stronger setting. The frame conditions are the weak half: nothing but a review
  holds them until the view state is one struct a test can compare before and after.

---

## 2. MODEL

**Containers.** None. Nothing here is persisted and nothing is shared.

**The column.** Pages of one size, stacked with a fixed gutter between them, centred horizontally.
The column is rebuilt from the layout on every layout that is applied.

```rust
/// The geometry of the page column, derived from the page layout. Owns which pages are
/// visible, where each page starts, and the inverse of that mapping.
pub struct PageColumn {
    pub page_height_px:  f32,
    /// The page trim width, which is what the column is centred on.
    pub page_width_px:   f32,
    pub gutter_px:       f32,
    /// Zero for a layout that has not arrived yet.
    pub page_count:      u32,
    pub total_height_px: f32,
}

impl PageColumn {
    /// The pages intersecting the viewport, in document pixels. Empty for an empty
    /// layout, and never inverted for any scroll offset (`PROSE-UI-021`).
    pub fn visible_pages(&self, viewport: &Viewport) -> RangeInclusive<u32>;
    /// Where a page starts, measured from the top of the column.
    pub fn page_top_y(&self, page_index: u32) -> f32;
    /// The inverse: the page a document Y belongs to, and the Y within that page. A Y in
    /// a gutter is assigned to the nearer page edge (`PROSE-UI-022`). `None` only for an
    /// empty layout.
    pub fn doc_y_to_page(&self, doc_y: f32) -> Option<(u32, f32)>;
}
```

**The camera** (`ambre-prose`):

```rust
pub struct Viewport {
    /// From the top of the document, in document pixels.
    pub scroll_y:     f32,
    pub width_px:     f32,
    pub height_px:    f32,
    /// The ratio the browser imposes, not a user setting.
    pub device_pixel_ratio: f32,
    /// The left edge of the centred column inside the viewport (`PROSE-UI-026`).
    pub page_margin_left_px: f32,
    /// The user's zoom, one at a hundred per cent, clamped (`PROSE-UI-024`). Independent
    /// of the device ratio: a conversion multiplies by both.
    pub zoom:         f32,
}
```

**Screen to document, and back.** The conversion inverts exactly what the renderer applies
(`PROSE-UI-023`): subtract the column's left margin, divide by the zoom, add the scroll offset.
The result is resolved to a page before it is handed on, so a caller indexes the layout without
re-deriving which page it is on.

```rust
/// A screen point, in canvas pixels, as a position in the document.
pub fn screen_to_doc(x: f32, y: f32, vp: &Viewport, col: &PageColumn) -> DocPos;

pub enum DocPos {
    OnPage { page: u32, x_in_page: f32, y_in_page: f32 },
    /// The layout has no page at all.
    Empty,
}
```

`DocPos::Empty` is what a caller answers a click on an empty document with, and it is a distinct
answer from a click that lands in a page margin: the first has no page, the second has a page and
no block (`features/ambre-spec-prose-page-rendering.md`).

**Culling.** A block is built for a frame when the page it starts on is in the visible range, plus
one page on each side (`PROSE-UI-007`). A block outside that range is neither shaped nor
rasterized; its runs stay cached and unbuilt.

**Zoom.** A zoom multiplies the factor, clamps it (`PROSE-UI-024`), recomputes the column's left
margin so the column stays centred (`PROSE-UI-026`), and solves the scroll offset that keeps the
anchor -- the caret, or the focus of a pinch -- under the same screen point (`PROSE-UI-023`). It
writes nothing to the document.

```rust
/// Zoom by a factor about a screen point.
pub fn apply_zoom(state: &mut ProseUiState, factor: f32, focus_screen: Vec2);
```

**Scroll.** Every path that moves the scroll offset -- the wheel, a drag past the viewport edge, a
caret the keyboard has to reveal, a relayout that shortened the document -- clamps it to the
column's own range afterwards (`PROSE-UI-025`). A relayout is the case that matters: deleting the
last twenty pages of a document leaves an offset past the end of a column that no longer has one.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the wheel, a two-finger glide | scrolls the column vertically |
| a drag past the top or bottom edge | scrolls while the drag continues (`features/ambre-spec-prose-selection.md`) |
| a caret move that leaves the viewport | scrolls the minimum needed to bring the caret back, with a margin |
| `Ctrl` with `=`, `-` or `0` | zooms in, zooms out, returns to a hundred per cent |
| `Ctrl` with the wheel, a trackpad pinch | zooms about the pointer |
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

* **The zoom is Prose's own, and the shared model is a Story.** The clamp, the anchor and the
  centring are specified here; the "One zoom model" Story exists to make them one implementation
  across the applications, and until it lands the spreadsheet has no keyboard zoom at all.
* **Nothing rules on inertial scroll.** A momentum term is carried in the view state and no rule
  says how it decays, whether it survives a relayout, or what stops it at the ends of the column.
* **The zoom does not survive a reload.** It is view state and nothing writes it anywhere, so a
  reader who zooms to a hundred and fifty per cent does it again on the next visit.
* **A page wider than the viewport has no horizontal scroll.** The column pins to the left edge
  and the right side of the page is unreachable at a zoom the rules allow.
* **Nothing rules on a device pixel ratio that changes.** The ratio is carried in the view state
  and a conversion multiplies by it, so moving the window to a second screen changes every mapping
  `PROSE-UI-023` governs. Section 3 lists a canvas resize and not this, and `PROSE-UI-130` says
  what a resize leaves alone without saying whether a ratio change is one.
* **A pageless mode would replace this column.** `meta/pageless` is a container the schema carries
  as deferred; with it set, the layout is linear, the gutters go and the width follows the
  viewport -- which is a different geometry, not a variant of this one.
