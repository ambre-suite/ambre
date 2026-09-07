# AMBRE -- FEATURE: PAGE RENDERING

**Type:** Vertical (feature)
**Implementation:** design-only

> What a page looks like on the canvas: the text of every visible block, the caret, the selections,
> the remote carets and the inline highlights, painted from runs the engine shaped once. The same
> drawing code produces the exported PDF.

Nothing painted here is shared state: the runs come from the document, the colours come from the
theme, and a repaint writes nothing back. There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-006` | `INV` | A full text layout or a table-of-contents recompute MUST NOT run in the UI thread: both arrive already computed. | this spec section 2; `features/ambre-spec-prose-pagination.md` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-113` | `INV` | A shaped run handed from the side that shapes to the side that draws MUST NOT carry a GPU handle or any other handle bound to one thread. | this spec section 4 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-114` | `INV` | Resolving the position under a pointer MUST read the runs already shaped and MUST NOT shape a block: a block that was culled and never painted MUST resolve like any other. | this spec section 4 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-019` | `INV` | The run cache MUST be bounded: it MUST evict rather than grow with the document. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-018` | `INV` | Block drawing MUST be written once against the drawing surface: the screen and the PDF MUST NOT be two implementations. | this spec section 2; `ambre-spec-drawing-surface.md section 2` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5); `ambre-spec-pdf-export.md`, `PDF-001` is the export half of the same rule | unimplemented |
| `PROSE-UI-017` | `INV` | Partial repaint MUST use the shared content-repaint contract; a second repaint mechanism MUST NOT exist beside it. | this spec section 2; `ambre-spec-frame.md section 2` | not built -- lands with the "One repaint contract" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-014` | `INV` | Every colour drawn MUST come from the generated theme tokens; a literal colour MUST NOT appear in render code. | this spec section 3; `ambre-spec-design-tokens.md` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-115` | `OBS` | A visible page MUST be drawn with its sheet, the blocks the layout places on it, and the header, the footer and the footnote band the layout carries for it. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-116` | `OBS` | A block the layout breaks across two pages MUST be drawn on both: the continuation MUST NOT be left out. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-117` | `OBS` | A selection MUST be drawn under the glyphs it covers, and an inline highlight over them. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-118` | `OBS` | A remote caret and its label MUST NOT hide local text: they MUST be drawn after the glyphs of the page. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Paint a page

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-119` | `POST` | Painting a page MUST build the GPU state of a run the first time that run is drawn, and MUST reuse it afterwards. | this spec section 4 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-020` | `FRAME` | Painting a page MUST NOT walk an annotation anchored outside the pages it draws. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-120` | `FRAME` | Painting a page MUST NOT write to the document, and MUST NOT build the GPU state of a run it does not draw. | derived | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Apply a layout

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-UI-121` | `POST` | A layout newer than the one in force MUST replace it, and the version it carries MUST become the version the view holds. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-016` | `FAIL` | A layout older than the one already applied MUST be dropped, so a late arrival MUST NOT replace a newer one. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-122` | `FRAME` | Applying a layout MUST NOT write to the document and MUST NOT shape a block. | derived | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
* **Budget.** the display-list build and the whole page paint, two rows of
  `ambre-spec-budgets.md section 2`; the run cache and the vertex buffers, two rows of
  `ambre-spec-budgets.md section 6`. The second of those exceeds the shared UI-thread total, and
  the register says so.
* **Boundary.** none: this thread paints view state, where `UI-THREAD-004` bans a network, crypto
  or persistence path. What it draws arrives as an event and leaves as nothing.
* **Verification.** 17 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-018` and `PROSE-UI-113` are the two rows a compiler can hold before a test exists: a
  second drawing implementation cannot hide behind a generic parameter, and a run carrying a
  device handle does not cross a thread. The four drawn rows are screenshot work, which is the
  weakest mechanism this workspace uses, and `PROSE-UI-116` is the one of them a layout fixture
  can hold instead -- a block broken across two pages is drawn twice or it is not.

---

## 2. MODEL

**Containers.** None. Every colour is the theme's and every run is derived.

**The shaped run, and what makes it crossable.** The engine shapes a block once, on the Logic
thread, and hands the result over by move. Both threads share one heap, so nothing is serialized.

```rust
/// One shaped run: the output of the shaper, complete enough to draw and to hit-test,
/// and free of any GPU or thread-bound handle. Produced by pagination, consumed here.
pub struct ParleyRunData {
    /// The glyphs, in visual order, with the advance of each.
    pub glyphs:      Vec<PositionedGlyph>,
    /// Cluster boundaries, joining a glyph range to a byte range of the block body.
    pub clusters:    Vec<Cluster>,
    /// The line boxes the run was broken into, with the baseline of each.
    pub lines:       Vec<LineBox>,
    pub ascent:      f32,
    pub descent:     f32,
    /// The resolved face and size, so the raster stage needs no style lookup.
    pub font:        FontKey,
}

impl ParleyRunData {
    /// The byte offset under a horizontal position inside the run.
    pub fn offset_for_x(&self, x: f32) -> u32;
    /// The inverse: where a byte offset sits inside the run.
    pub fn x_for_offset(&self, offset: u32) -> f32;
}
```

**The cache** (`ambre-prose`). One entry per block, holding the shaped runs, the block height and
whether a delta has touched the block since it was shaped. It is capped at five hundred blocks,
about fifty pages, and evicts least-recently-used beyond that (`PROSE-UI-019`).

```rust
/// The UI-side run: the shaped data, plus the GPU state built from it on first paint.
pub struct ParleyRun {
    pub data: ParleyRunData,
    /// Atlas handles and a positioned quad buffer. `None` until the run is first painted;
    /// a culled block never builds it. Building it is a raster and an upload, never a
    /// shaping pass (`PROSE-UI-006`).
    pub draw: Option<RunDrawState>,
}
```

Caret geometry and hit-testing read `data`, so both work on a block that was culled and never
painted (`features/ambre-spec-prose-viewport.md`).

**Building a page's drawing.** The driver signature is the shared one of
`ambre-spec-frame.md section 2`: the surface, the font context, the glyph cache, and the strip
of what is dirty.

```rust
/// Called by the frame host as the Prose view's content paint.
pub fn build_display_list<S: DrawSurface>(
    state:   &ProseUiState,
    surface: &mut S,
    fc:      &mut FontContext,
    tc:      &mut TextCache,
    strip:   &RepaintStrip,
);
```

The steps, per visible page:

1. paint the page sheet and its shadow, at the page origin the viewport resolves;
2. walk the blocks the layout places on that page, continuations of split blocks included;
3. for each block, take its runs from the cache, building the GPU state on first paint;
4. paint the selection rectangles under the glyphs, then the glyphs, then the inline highlights;
5. paint the remote carets and their labels last, so a collaborator never hides local text;
6. paint the header, the footer and the footnote band from the runs the layout carries.

Steps 4 and 5 skip any annotation anchored outside the visible page range, and the draft buffer
returns immediately when it is empty (`PROSE-UI-020`).

**What is repainted.** `RepaintStrip` is Prose's use of the shared content-repaint contract, not a
mechanism beside it (`PROSE-UI-017`): the dirty pages map onto the shared full and reuse modes, and
Prose does not use the scroll-band mode, because a page column reflows rather than blits.

**Dropping a stale layout.** Each layout carries the version the engine stamped it with, and the
view keeps the version it last applied. A layout arriving with an older version is dropped
(`PROSE-UI-016`), which is what stops a slow recomputation from repainting over a newer one.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

The user touches nothing here: this is what a page looks like once something else has decided what
is on it. What is drawn, and where each colour comes from:

| Drawn | Token | Opacity |
|---|---|---|
| the page sheet, its border and its shadow | the `surfaces` group | full |
| body text, headers, footers, footnotes | the surface's foreground, or the block's own colour mark | full |
| the text caret | `accent` | full |
| the local selection | `accent` | a fifth |
| a comment highlight | the `warning` status | two fifths |
| a suggested insertion | the `success` status | three tenths |
| a suggested deletion | the `error` status, struck through | three tenths |
| a remote caret and its selection | the peer colour (`ambre-spec-presence.md section 3`) | four fifths for the bar, a sixth for the selection |
| a spelling error | the `error` status, as a wavy underline | full |

Each of those names a generated token, read at paint time from the constants
`ambre-spec-design-tokens.md` declares and `ambre-spec-frame.md section 2` consumes -- which is
what `PROSE-UI-014` asks for and what a table of literal colours cannot give. The chrome surfaces
that sit over the canvas are DOM and are not painted here (`UI-THREAD-001`,
`features/ambre-spec-prose-toolbar.md`, `features/ambre-spec-prose-slash-command.md`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Shaping happens on the Logic thread and the UI thread only draws | the shared UI-thread shaping path the single-line cell editor uses | it is right for one short line and wrong for a page. Pagination already shapes every block to measure where it breaks, so shaping again on the render thread duplicates that work and puts an unbounded cost inside the frame, and the shared heap makes handing the result over free |
| The shaped run carries no GPU handle | one run type carrying its atlas entries | it pins the shaped output to the thread that rasterized it, and a device handle cannot cross a thread boundary. Split in two, the immovable half stays where the device is and the movable half stays movable |
| The GPU state is built on first paint, not on receipt | building the draw state when the run arrives | it makes a remote edit far off screen cost a raster. A document sends runs for blocks the reader may never scroll to, and rasterizing them all spends the atlas on text nobody sees |
| Hit-testing reads the shaped data, never a live layout pass | shaping the block under the pointer on demand | it is the shaping-on-the-render-thread rule broken through the back door, and a click must resolve on a block that was culled and never painted |
| The run cache is bounded and evicts | caching every block, or caching only what is visible | a cache proportional to the document is a memory leak with a slow fuse -- a hundred-page document holds every block's glyphs for the session -- while a visible-only cache re-rasters on every scroll reversal. Fifty pages of runs is more than any reader has scrolled through recently |
| Partial repaint is the shared contract, not a Prose one | a Prose-specific dirty-page mechanism | it is two places a repaint bug can live, and the shared contract already models what Prose needs. Prose simply never uses the scroll-band mode. It is also what the design started as |
| One drawing implementation for screen and PDF | a separate PDF painter reading the same layout | two implementations disagree on the first thing that is hard -- a wavy underline, a struck-through run, a footnote separator. A page is the same picture on both, and parameterizing the drawing on the surface makes divergence a compile error rather than a rendering bug |
| A stale layout is dropped, not merged | applying whatever arrives last | it moves the page a reader is looking at backwards. Layouts are whole values, so a late one is not a delta to reconcile, and dropping it costs a frame at worst |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The Prose frame budget exceeds the shared one, and neither document reconciles them.** A page
  paint is budgeted at fourteen milliseconds against a shared frame target of five typical
  (`ambre-spec-budgets.md section 2`). The two are not contradictory -- a full page repaint
  is a busy case, as a full board paint is for the whiteboard -- but nothing says which one a
  reviewer holds a regression against.
* **The UI-thread memory figures do not fit the shared total.** The run cache alone is budgeted at
  twenty megabytes and the vertex buffers and atlases for three pages at fifty of video memory,
  against a shared UI-thread total of forty-one (`ambre-spec-budgets.md section 6`). The
  rows now sit beside each other, which is the first time the arithmetic is visible.
* **The table above names tokens the catalogue does not spell out.** The design started from seven
  literal hex colours, beside a rule banning exactly that; the tokens replace them, and
  `ambre-spec-design-tokens.md` lists its catalogue by group rather than by name. Until a token
  has a name, `PROSE-UI-014` is checkable by a reviewer and by nothing else.
* **Nothing paints an orphaned comment or a broken cross reference.** Both are specified as
  surfaced to the reader, and neither has a drawn form here.
* **Nothing rules the order of two overlapping annotations.** Section 3 gives a comment
  highlight, a suggested insertion and a suggested deletion an opacity each, and a sentence can
  carry all three. `PROSE-UI-117` puts highlights over the glyphs and says nothing about which of
  them sits over which, so three overlapping marks have three orders and no rule between them.
* **A page taller than the viewport at minimum zoom is not ruled.** The sheet is drawn whole and
  clipped by the viewport, and no rule says what a block overflowing its own page frame looks like.
