# AMBRE -- FEATURE: PAGE RENDERING

**Type:** Vertical (feature)
**Implementation:** partial -- the painter is built; no host mounts it on the canvas yet

> What a page looks like on the canvas. The text of every visible block, the caret, the selections,
> the remote carets and the inline highlights, painted from runs the engine shaped once. The same
> drawing code produces the exported PDF.

Nothing painted here is shared state: the runs come from the document, the colours come from the
theme, and a repaint writes nothing back. There is no two-peer table below.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-006` | `INV` | A full text layout or a table-of-contents recompute MUST NOT run in the UI thread: both arrive already computed. | this spec section 2; `features/ambre-spec-prose-pagination.md` | `ambre_logic::dispatch::tests::the_view_draws_a_layout_the_logic_thread_computed` | none | verified |
| `PROSE-UI-113` | `INV` | A shaped run handed from the side that shapes to the side that draws MUST NOT carry a GPU handle or any other handle bound to one thread. | this spec section 4 | `ambre_prose::layout::run::tests::a_shaped_layout_moves_to_another_thread_and_draws_there` | none | verified |
| `PROSE-UI-114` | `INV` | Resolving the position under a pointer MUST read the runs already shaped, never shaping a block. | this spec section 4 | `ambre_prose::view::hit::tests::a_block_never_painted_resolves_a_point_off_its_shaped_runs` | none | verified |
| `PROSE-UI-172` | `INV` | A block that was culled and never painted MUST resolve a position like any other. | this spec section 4 | `ambre_prose::view::hit::tests::a_block_never_painted_resolves_a_point_off_its_shaped_runs` | none | verified |
| `PROSE-UI-019` | `INV` | The run cache MUST be bounded: it MUST evict rather than grow with the document. | this spec section 2 | `ambre_prose::view::cache::tests::the_run_painted_least_recently_is_the_one_dropped`; `ambre_prose::view::paint::tests::the_run_cache_stays_bounded_over_a_long_scroll` | none | verified |
| `PROSE-UI-018` | `INV` | Block drawing MUST be written once against the drawing surface: the screen and the PDF MUST NOT be two implementations. | this spec section 2; `ambre-spec-drawing-surface.md section 2` | `ambre_prose::view::paint::tests::a_long_document_builds_only_the_pages_around_the_canvas` | none | verified |
| `PROSE-UI-017` | `INV` | Partial repaint MUST use the shared content-repaint contract; a second repaint mechanism MUST NOT exist beside it. | this spec section 2; `ambre-spec-frame.md section 2` | not built | unbuilt -- lands with the `One repaint contract` Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-014` | `INV` | Every colour drawn MUST come from the generated theme tokens; a literal colour MUST NOT appear in render code. | this spec section 3; `ambre-spec-design-tokens.md` | `ambre_prose::view::paint::tests::every_colour_comes_from_the_palette_or_a_peer` | none | verified |
| `PROSE-UI-115` | `OBS` | A visible page MUST be drawn with its sheet, the blocks the layout places on it, and the header, the footer and the footnote band the layout carries for it. | this spec section 2 | `ambre_prose::view::paint::tests::a_page_carries_its_bands_and_its_footnotes_below_a_separator` | none | verified |
| `PROSE-UI-116` | `OBS` | A block the layout breaks across two pages MUST be drawn on both: the continuation MUST NOT be left out. | this spec section 2 | `ambre_prose::view::paint::tests::a_block_split_across_two_pages_is_drawn_on_both` | none | verified |
| `PROSE-UI-117` | `OBS` | A selection MUST be drawn under the glyphs it covers, and an inline highlight over them. | this spec section 2 | `ambre_prose::view::paint::tests::selections_go_under_the_glyphs_and_highlights_over_them` | none | verified |
| `PROSE-UI-118` | `OBS` | A remote caret and its label MUST NOT hide local text: they MUST be drawn after the glyphs of the page. | this spec section 2 | `ambre_prose::view::paint::tests::a_remote_caret_is_painted_after_every_glyph_of_its_page` | none | verified |
| `PROSE-UI-207` | `OBS` | A bold or italic mark MUST reach the glyphs of the text it covers. | this spec section 2 | `ambre_prose::logic::marks::tests::a_bold_or_italic_mark_reaches_the_glyphs` | none | verified |
| `PROSE-UI-209` | `OBS` | An underline or strike-through mark MUST be drawn across the text it covers. | this spec section 2 | `ambre_prose::view::paint::tests::an_underline_and_a_strike_through_are_drawn_across_their_text` | none | verified |
| `PROSE-UI-210` | `OBS` | A size, font or colour mark MUST reach the glyphs of the text it covers. | this spec section 2 | not built | unbuilt -- lands with the `The format bar's other controls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Paint a page

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-119` | `POST` | Painting a page MUST build the GPU state of a run the first time that run is drawn. | this spec section 4 | `ambre_prose::view::paint::tests::a_run_is_built_on_its_first_paint_and_reused_after` | none | verified |
| `PROSE-UI-173` | `POST` | Painting a page MUST reuse a run's GPU state after the first time that run is drawn. | this spec section 4 | `ambre_prose::view::paint::tests::a_run_is_built_on_its_first_paint_and_reused_after` | none | verified |
| `PROSE-UI-020` | `FRAME` | Painting a page MUST NOT walk an annotation anchored outside the pages it draws. | this spec section 2 | `ambre_prose::view::paint::tests::highlights_are_walked_on_the_blocks_drawn_and_no_other` | none | verified |
| `PROSE-UI-120` | `FRAME` | Painting a page MUST NOT write to the document, and MUST NOT build the GPU state of a run it does not draw. | derived | `ambre_prose::view::paint::tests::a_long_document_builds_only_the_pages_around_the_canvas`; `ambre_prose::view::paint::tests::a_run_is_built_on_its_first_paint_and_reused_after` | none | verified |

### 1.3 Apply a layout

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-121` | `POST` | A layout newer than the one in force MUST replace it, and the version it carries MUST become the version the view holds. | this spec section 2 | `ambre_prose::view::tests::a_newer_layout_replaces_the_one_in_force_and_rebuilds_the_column` | none | verified |
| `PROSE-UI-016` | `FAIL` | A layout older than the one already applied MUST be dropped, so a late arrival MUST NOT replace a newer one. | this spec section 2 | `ambre_prose::view::tests::a_late_layout_is_dropped_and_the_newer_one_stays` | none | verified |
| `PROSE-UI-122` | `FRAME` | Applying a layout MUST NOT write to the document and MUST NOT shape a block. | derived | `ambre_prose::view::tests::applying_a_layout_keeps_the_zoom_shapes_nothing_and_writes_nothing` | none | verified |
| `PROSE-UI-205` | `FRAME` | Applying the layout that answers an edit MUST NOT copy or rebuild the blocks and the pages the edit left alone: its cost on this thread MUST follow the edit, not the document. | this spec section 2 | `ambre_prose::view::tests::a_layout_is_taken_by_its_pointer`; `ambre_prose::view::tests::a_layout_the_view_drops_is_freed_by_the_logic_thread` | none | verified |
* **Budget.** the display-list build and the whole page paint (`ambre-spec-budgets.md section 2`);
  the run cache and the vertex buffers (`ambre-spec-budgets.md section 6`). The second of those
  exceeds the shared UI-thread total, and the register says so.
* **Boundary.** none: this thread paints view state, where `UI-THREAD-004` bans a network, crypto
  or persistence path. What it draws arrives as an event and leaves as nothing.
* **Verification.** 21 `verified`, 2 `unimplemented`. Every verified row is a host test in
  `crates/ambre-prose`, painting through the recording surface of `ambre-draw`. `PROSE-UI-113` is
  also compiler-held: the run type must be `Send`, `Sync` and `'static`. `PROSE-UI-006` is held in
  `crates/ambre-logic`: the Logic thread shapes, and the view draws what the bus hands it.
  `PROSE-UI-017` waits on the shared repaint contract.

---

## 2. MODEL

**Containers.** None. Every colour is the theme's and every run is derived.

**The shaped run, and what makes it crossable.** The engine shapes a block once, on the Logic
thread, to measure it. It keeps the result and hands it over by move with the layout. Both threads
share one heap, so nothing is serialized.

```rust
/// One shaped block: the shaper's layout broken into lines, the byte start of each code
/// point of its text, and its left indent. Free of any GPU or thread-bound handle
/// (`PROSE-UI-113`).
pub struct ParleyRunData { /* private */ }

impl ParleyRunData {
    /// The code-point offset under a point of the run, from the run's top-left corner.
    pub fn offset_at(&self, at: Point<Px<Doc>>) -> Pos<RunChar>;
    /// Where the caret sits for a code-point offset.
    pub fn caret_box(&self, offset: Pos<RunChar>) -> BoundingBox;
    /// One box per line a code-point range covers.
    pub fn range_boxes(&self, span: Span<RunChar>) -> Vec<BoundingBox>;
}

/// A shared handle on one shaped block. Two handles are equal when they are one run.
#[derive(Clone, Debug)]
pub struct ShapedRun(Arc<ParleyRunData>);

/// What the Logic thread hands the view: the page geometry of the document, and the
/// blocks, runs, notes and tables of the pages the view asked for (`PROSE-125`).
#[derive(Clone, Debug, Default)]
pub struct ShapedLayout {
    pub version: Seq<LayoutVersion>,
    pub layout:  Arc<PageLayout>,
    pub blocks:  HashMap<TreeID, ShapedRun>,
    pub notes:   HashMap<Id<Footnote>, ShapedRun>,
    pub tables:  HashMap<TreeID, Arc<TableShape>>,
    /* private: the cells, the flow order of every block, the pages carried, the two ends of
       the text, the blocks each page shows */
}
```

**The cache** (`ambre-prose`). The shaped data stays in the layout the view holds, so hit-testing
and the caret read it on a block never painted (`PROSE-UI-172`). What the view caches is the drawing
half: where each glyph run of a block sits. It is built on the first paint of a run
(`PROSE-UI-119`) and reused after (`PROSE-UI-173`). It holds five hundred runs, about fifty pages,
and drops the one painted least recently beyond that (`PROSE-UI-019`). The glyph atlas belongs to
the drawing surface.

```rust
/// The drawing state of the runs painted lately, bounded.
pub struct RunCache { /* private */ }

/// The colours a page is painted with, filled by the host from the theme tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePalette {
    pub sheet: Color, pub sheet_border: Color, pub shadow: Color, pub text: Color,
    pub accent: Color, pub warning: Color, pub success: Color, pub error: Color,
}

impl ProseUiState {
    /// Paint the pages the canvas shows, and one more on each side.
    pub fn paint<S: DrawSurface>(&mut self, surface: &mut S, palette: &PagePalette);
}

impl HitTest for ProseUiState {
    type Target = (BlockId, Pos<DocChar>);
    /// The block and the offset under a canvas point.
    fn hit_test(&self, at: Point<Px<View>>) -> Option<(BlockId, Pos<DocChar>)>;
}
```

The steps, per page built:

1. paint the page shadow, the sheet and its border, at the page origin the viewport resolves;
2. walk the blocks the layout places on that page, continuations of split blocks included;
3. for each block, clip to its fragment and take its drawing state from the cache;
4. paint the selection rectangles under the glyphs, then the glyphs, then the inline highlights;
5. paint the header, the footer and the footnote band from the runs the layout carries;
6. paint the remote carets last, so a collaborator never hides local text.

Step 4 reads the highlights indexed by block when they arrive, so a paint touches only the
annotations of the blocks it draws (`PROSE-UI-020`).

**What is repainted.** `RepaintStrip` is Prose's use of the shared content-repaint contract, not a
mechanism beside it (`PROSE-UI-017`). The dirty pages map onto the shared full and reuse modes.
Prose does not use the scroll-band mode, because a page column reflows rather than blits.

**Dropping a stale layout.** Each layout carries the version the engine stamped it with, and the
view keeps the version it last applied. A layout arriving with an older version is dropped
(`PROSE-UI-016`), which is what stops a slow recomputation from repainting over a newer one.

**Code:** `crates/ambre-prose/src/layout/run.rs` holds the shaped run; `view/paint.rs` the painter;
`view/cache.rs` the run cache; `view/hit.rs` the hit-test; `view/mod.rs` applies a layout.

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
`ambre-spec-design-tokens.md` declares and `ambre-spec-frame.md section 2` consumes. That is
what `PROSE-UI-014` asks for, and what a table of literal colours cannot give. The chrome surfaces
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
  (`ambre-spec-budgets.md section 2`). The two are not contradictory: a full page repaint
  is a busy case, as a full board paint is for the whiteboard. Nothing says which one a
  reviewer holds a regression against.
* **The UI-thread memory figures do not fit the shared total.** The vertex buffers and atlases
  for three pages are budgeted at fifty megabytes of video memory. The shared UI-thread total is
  forty-one (`ambre-spec-budgets.md section 6`). The
  rows now sit beside each other, which is the first time the arithmetic is visible. Settling it
  lands with the `The memory budgets add up` Story (`ambre-roadmap.md` E7).
* **The table above names tokens the catalogue does not spell out.** The design started from seven
  literal hex colours, beside a rule banning exactly that. The tokens replace them, and
  `ambre-spec-design-tokens.md` lists its catalogue by group rather than by name. Until a token
  has a name, `PROSE-UI-014` is checkable by a reviewer and by nothing else.
* **Nothing paints an orphaned comment or a broken cross reference.** Both are specified as
  surfaced to the reader, and neither has a drawn form here.
* **Nothing rules the order of two overlapping annotations.** Section 3 gives a comment
  highlight, a suggested insertion and a suggested deletion an opacity each, and a sentence can
  carry all three. `PROSE-UI-117` puts highlights over the glyphs and says nothing about which of
  them sits over which. Three overlapping marks have three orders and no rule between them.
* **The shaped half of the runs is not bounded.** Each measured block keeps its run on the Logic
  thread, about 150 KB a page. `PROSE-UI-019` bounds the drawing half only. The
  `Shaped runs bounded to the view` Story (`ambre-roadmap.md` E7) holds the rest.
* **A remote caret's label is a coloured tab, not a name.** Drawing the name would shape text on
  the UI thread, which this spec rules out, and no row says who shapes it instead.
* **A remote selection spanning blocks is indexed on the block of its focus.** The painter draws
  its other blocks only when that block is also built.
* **A page taller than the viewport at minimum zoom is not ruled.** The sheet is drawn whole and
  clipped by the viewport, and no rule says what a block overflowing its own page frame looks like.
