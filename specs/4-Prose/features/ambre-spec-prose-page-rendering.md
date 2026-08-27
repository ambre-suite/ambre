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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-UI-006` | A full text layout or a table-of-contents recompute MUST NOT run in the UI thread: both arrive already computed. | this spec section 2; `features/ambre-spec-prose-pagination.md` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-014` | Every colour drawn MUST come from the generated theme tokens; a literal colour MUST NOT appear in render code. | this spec section 3; `ambre-spec-design-tokens.md` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-016` | A layout older than the one already applied MUST be dropped, so a late arrival MUST NOT replace a newer one. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-017` | Partial repaint MUST use the shared content-repaint contract; a second repaint mechanism MUST NOT exist beside it. | this spec section 2; `ambre-spec-frame.md section 2` | not built -- lands with the "One repaint contract" Story (`ambre-roadmap.md` E8) | unimplemented |
| `PROSE-UI-018` | Block drawing MUST be written once against the drawing surface: the screen and the PDF MUST NOT be two implementations. | this spec section 2; `ambre-spec-drawing-surface.md section 2` | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5); `ambre-spec-pdf-export.md`, `PDF-001` is the export half of the same rule | unimplemented |
| `PROSE-UI-019` | The run cache MUST be bounded: it MUST evict rather than grow with the document. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-020` | Building a page's drawing MUST cost what is visible, never what the document holds: an annotation anchored outside the visible pages MUST NOT be walked. | this spec section 2 | not built -- lands with the "Page rendering" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the display-list build and the whole page paint, two rows of
  `ambre-spec-budgets.md section 2`; the run cache and the vertex buffers, two rows of
  `ambre-spec-budgets.md section 6`. The second of those exceeds the shared UI-thread total, and
  the register says so.
* **Boundary.** none: this thread paints view state, where `UI-THREAD-004` bans a network, crypto
  or persistence path. What it draws arrives as an event and leaves as nothing.
* **Verification.** 7 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-UI-018` is the one row a compiler can hold before a test exists, because a second
  implementation cannot hide behind a generic parameter.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Shaping happens on the Logic thread and the UI thread only draws | pagination already shapes every block to measure where it breaks, so shaping again on the render thread would duplicate that work and put an unbounded cost inside the frame. The shared heap makes handing the result over free, which removes the usual reason to shape where you draw | the shared UI-thread shaping path the single-line cell editor uses, which is right for one short line and wrong for a page |
| The shaped run carries no GPU handle | it has to cross a thread boundary, and a device handle cannot. Splitting the run in two puts the immovable half where the device is and keeps the movable half movable | one run type carrying its atlas entries, which would pin the shaped output to the thread that rasterized it |
| The GPU state is built on first paint, not on receipt | a document sends runs for blocks the reader may never scroll to, and rasterizing them all would spend the atlas on text nobody sees | building the draw state when the run arrives, which makes a remote edit far off screen cost a raster |
| Hit-testing reads the shaped data, never a live layout pass | a click must resolve on a block that was culled and never painted, and re-shaping it inside the pointer handler is the shaping-on-the-render-thread rule broken through the back door | shaping the block under the pointer on demand |
| The run cache is bounded and evicts | a cache proportional to the document is a memory leak with a slow fuse: a hundred-page document would hold every block's glyphs for the session. Fifty pages of runs is more than any reader has scrolled through recently | caching every block, or caching only what is visible, which would re-raster on every scroll reversal |
| Partial repaint is the shared contract, not a Prose one | two repaint mechanisms means two places a repaint bug can live, and the shared one already models what Prose needs. Prose simply never uses the scroll-band mode | a Prose-specific dirty-page mechanism, which is what the design started as |
| One drawing implementation for screen and PDF | a page is the same picture on both, and two implementations disagree on the first thing that is hard -- a wavy underline, a struck-through run, a footnote separator. Parameterizing the drawing on the surface makes divergence a compile error rather than a rendering bug | a separate PDF painter reading the same layout |
| A stale layout is dropped, not merged | layouts are whole values, so a late one is not a delta to reconcile: applying it would move the page a reader is looking at backwards. Dropping it costs a frame at worst | applying whatever arrives last |

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
* **A page taller than the viewport at minimum zoom is not ruled.** The sheet is drawn whole and
  clipped by the viewport, and no rule says what a block overflowing its own page frame looks like.
