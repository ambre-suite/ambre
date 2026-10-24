# AMBRE -- FEATURE: THE DRAWING SURFACE

**Type:** Horizontal (framework)
**Implementation:** partial -- the trait ships, and so do the screen and PDF back ends. Clipping
rejects by bounding box, and a gradient brush draws nothing.

> The dozen calls a renderer may make: a rectangle, a line, a path, a glyph run, an image, a clip,
> a transform. One walk over the document draws it, and the surface underneath decides whether the
> result is a screen or a PDF page.

An application that exported by walking its document a second time would drift: the page would stop
matching the screen one feature at a time. So there is one walk, and the two destinations differ
only in the surface the caller hands it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-render-assets.md` and `ambre-spec-high-density.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-002` | `INV` | Application render code MUST NOT name a GPU type: it draws through the shared surface alone. | this spec section 2 | crate-graph -- neither `ambre-trame` nor `ambre-toile` declares `wgpu`, and their render tests drive `RecordingSurface` on host | none | verified |
| `CANVAS-025` | `INV` | The drawing crate MUST declare no GPU and no PDF dependency. | this spec section 2; `ambre-spec-architecture.md section 2` | crate-graph -- `ambre-draw` declares `kurbo`, `peniko`, `parley` and `ambre-core`, and a `wgpu` or `krilla` line added there fails the build of every host-tier render test | none | verified |
| `CANVAS-026` | `INV` | The drawing crate MUST decode nothing: a raster image MUST arrive as pixels the caller already decoded. | this spec section 2 | crate-graph -- `ambre-draw` declares no codec, and `ImageSource` borrows a slice it never owns | none | verified |
| `CANVAS-055` | `INV` | The PDF back end MUST NOT be in the startup bundle. | this spec section 2; `ambre-spec-budgets.md section 5` | crate-graph -- `ambre-app` declares neither `krilla` nor `ambre-pdf`, and Toile's `pdf` feature is off by default, so nothing links krilla into a client build | none | verified |
| `CANVAS-056` | `INV` | A drawing primitive MUST take geometry and paint alone, and no screen-only state. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, reads every method of `DrawSurface` and refuses a parameter whose type is not a rectangle, a line, a colour, an affine, a path, a stroke, a brush, a glyph run or an image. It refuses a bare number not named as a width, so a `density: f64` fails though its type is listed | none | verified |
| `CANVAS-028` | `INV` | The PDF back end MUST run in the Logic thread. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a Rust line naming `krilla` or `ambre_pdf` outside `crates/ambre-pdf/` and the `src/logic/` directories | none | verified |
| `CANVAS-030` | `INV` | A surface that does not implement an optional primitive MUST do nothing, never fail. | this spec section 2 | type-level -- each optional method carries a default empty body, so a back end that omits it compiles and draws nothing | none | verified |
| `CANVAS-061` | `OBS` | A vector edge MUST reach the screen antialiased. | this spec section 2 | not built | unbuilt -- lands with the `Antialiased vector geometry` Story (`ambre-roadmap.md` E6). Every screen pipeline runs at one sample and the solid shader returns its vertex colour unmodified, so a diagonal is a hard staircase and only glyphs escape, through their own coverage atlas | unimplemented |
| `CANVAS-063` | `OBS` | An antialiased edge MUST keep its width in screen pixels at every zoom. | this spec section 2 | not built | unbuilt -- lands with the `Antialiased vector geometry` Story (`ambre-roadmap.md` E6). Tessellation is in model space, so a width in model units is a band that thickens as the board zooms in | unimplemented |
| `CANVAS-064` | `OBS` | Antialiasing MUST NOT mark the inside of a shape: only its silhouette MAY be softened. | this spec section 2 | not built | unbuilt -- lands with the `Antialiased vector geometry` Story (`ambre-roadmap.md` E6). It is the rule that separates a contour band from a per-triangle coverage term, which softens every internal edge of the tessellation and draws a seam along each one | unimplemented |
| `CANVAS-065` | `OBS` | A minified picture MUST be sampled through a mip chain, never through its full-size texels alone. | this spec section 2 | `ambre_canvas::render::mipmap::tests::the_chain_halves_down_to_one_texel`; `::a_flat_image_keeps_halving_its_long_axis` -- the descent; `::a_reduced_texel_is_the_mean_of_its_block` -- the averaging; `::an_odd_dimension_clamps_instead_of_overrunning` -- the odd edge; `::a_picture_is_sampled_trilinearly` -- the sampler every picture is bound with | none | verified |
| `CANVAS-066` | `INV` | An uploaded picture MUST be reused across frames, under a bounded budget that drops the least recently drawn first. | this spec section 2; `ambre-spec-budgets.md section 6` | `ambre_canvas::render::renderer::image_cache_tests::the_least_recently_drawn_entry_is_evicted_first`; `::a_hit_stamps_the_entry_with_the_current_frame`; `::reinserting_a_key_replaces_its_cost`; `::a_key_at_another_size_is_a_miss`; `::consecutive_quads_of_one_picture_are_one_run` -- the run merge that keeps one picture to one draw | none | verified |

### 1.2 Draw one document onto a surface

One walk over the document. The surface the caller hands it decides whether the result is a screen
or a page, and the walk is not told which.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-029` | `POST` | One render walk MUST serve the screen and the page. | this spec section 2 | `ambre_toile::logic::interop::pdf_export::tests::the_page_and_the_screen_share_one_walk` -- one board walked onto both, the page's record the screen's inside the frame clip | none | verified |
| `CANVAS-027` | `POST` | Every `push_clip` and `push_transform` MUST be matched by its pop before the surface is finished. | this spec section 2 | `ambre_pdf::tests::zero_area_clip_stays_balanced` -- the degenerate case, where an unbalanced push corrupts the rest of the page | none | verified |
| `CANVAS-074` | `FRAME` | The walk an application shares between the screen and the page MUST take a view of the document and a region in document coordinates, and nothing else. | this spec section 2; `CANVAS-029` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, finds each file that implements `DocumentWalk`, refuses a module its crate does not list in `WALK_REACH` and names no screen-state type above its tests; proptest -- `ambre_prose::view::paint::tests::the_screen_draws_the_walk_at_any_camera`, `ambre_trame::ui::renderer::tests::the_screen_draws_the_walk_at_any_camera` and `ambre_toile::logic::interop::pdf_export::tests::the_screen_draws_the_walk_at_any_camera` each vary the zoom, the scroll and the canvas, walk the same region onto a fresh surface with no camera, and find those calls unchanged inside the screen's. The density never reaches a walk, since `DrawSurface` exposes none (`CANVAS-056`); `ambre_canvas::render::surface::tests::a_zoomed_glyph_is_rasterized_at_its_screen_size` -- the back end rasterizes a run drawn in document units at the size it covers | none | verified |
| `CANVAS-062` | `FRAME` | Antialiasing MUST NOT add a render pass, a compute dispatch or a multisampled attachment. | this spec section 2; `ambre-spec-frame.md section 4` | not built | unbuilt -- lands with the `Antialiased vector geometry` Story (`ambre-roadmap.md` E6). It is the rule that keeps the edge treatment inside the decision `ambre-spec-frame.md` already took against a compute rasterizer, and a WebGL2 back end has no compute stage to run one on | unimplemented |

* **Budget.** the krilla row of `ambre-spec-budgets.md section 5`, counted once for the whole suite
  and kept out of the startup bundle by `CANVAS-028`. `ambre-spec-budgets.md section 2` charges the
  drawing calls to the vertex-batch and render-pass phases.
* **Boundary.** none: a surface writes to a texture or to a byte buffer, holds no key and opens no
  socket. A PDF leaves the machine only when the user sends it, and `ambre-spec-docfile.md` owns
  what a plaintext export owes them.
* **Verification.** 12 `verified`, 4 `unimplemented`. The crate graph, the compiler and three
  source gates hold most of it; a test walks a board onto the page and the screen. A walk
  reaches only its listed modules, and a property test per application finds it unchanged under
  any camera (`CANVAS-074`). A back end may read the screen to
  rasterize, a density for a tolerance: that changes the pixels, not the calls.

---

## 2. MODEL

**CRDT containers.** none. A surface is a sink: it reads no document and writes no state.

**The trait.** Declared in `ambre-draw`, which depends on `kurbo`, `peniko` and `parley` for the
geometry and typography types, and on `ambre-core` for nothing but identifiers.

```rust
// ambre-draw/src/lib.rs
// Dependencies: kurbo, peniko, parley only -- no wgpu, no krilla

/// peniko::Brush implements Default (Brush::Solid(TRANSPARENT)) and satisfies
/// parley::Brush bounds. This is the brush type used in Parley Layouts.
pub type ParleyBrush = peniko::Brush;

pub trait DrawSurface {
    // -- Geometry: required methods ----------------------------------------
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f64);
    fn stroke_line(&mut self, line: Line, color: Color, width: f64);

    // -- Text: required method ---------------------------------------------
    fn draw_glyphs(&mut self, run: &GlyphRun<'_, ParleyBrush>, brush: &Brush, transform: Affine);

    // -- Clip stack: required methods --------------------------------------
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);

    // -- Arbitrary paths: default no-op (implement for KrillaSurface / Toile)
    fn fill_path(&mut self, path: &BezPath, brush: &Brush) {}
    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, stroke: &Stroke) {}

    // -- Raster images: default no-op (implement for WgpuSurface / KrillaSurface)
    fn draw_image(&mut self, image: &ImageSource<'_>, dest: Rect) {}

    // -- Transform stack: default no-op (implement for Toile pan/zoom, Prose margins)
    fn push_transform(&mut self, transform: Affine) {}
    fn pop_transform(&mut self) {}
}
```

`ParleyBrush` is `peniko::Brush` and not `peniko::Color`, because `parley::Brush` demands `Default`
and only the brush has one. A caller passes the render colour explicitly.

**The three implementations.** Each lives in the crate that owns its back end, never in
`ambre-draw` (`CANVAS-025`).

| Surface | Crate | Thread | Back end |
|---|---|---|---|
| `WgpuSurface<'a>` | `ambre-canvas` | UI | the quad batch, with a CPU-side clip stack applied before a vertex is pushed |
| `KrillaSurface<'a, 'b>` | `ambre-pdf` | Logic, lazy-loaded | `krilla::surface::Surface`, borrowed from the page rather than owned |
| `RecordingSurface` | `ambre-draw`, behind the `testing` feature | host | a list of the calls made, which is what makes a render walk testable with no browser and no GPU |

```rust
// ambre-canvas/src/render -- wgpu backend, UI Thread
pub struct WgpuSurface<'a> {
    renderer: &'a mut QuadRenderer,
    clip_stack: Vec<Rect>,  // clip applied CPU-side before push vertex
}

// ambre-pdf -- krilla backend, Logic thread, lazy-loaded
pub struct KrillaSurface<'a, 'b> {
    surface: &'a mut krilla::surface::Surface<'b>,
}
```

**Paths, on the screen.** A `BezPath` becomes a `lyon::path::Path`, and lyon tessellates it in model
space. A fill uses the non-zero rule, and a stroke maps the width, cap, join and miter limit of
`kurbo::Stroke`. The flattening tolerance counts in screen pixels, divided by the transform's
scale. That scale is the square root of the linear part's determinant, so a zoomed path stays
smooth. The output vertices go through the active transform into the solid-colour quad batch, with
no pipeline of their own.

**Edges.** A filled or stroked path reaches the screen as a triangle soup with a hard boundary.
Geometry softens its silhouette, rather than the pipeline (`CANVAS-061`). Around the outline it
already walked, the tessellator emits a band whose outer vertices carry zero alpha and whose inner
vertices carry the fill's own.

The interpolator does the gradient, so the band needs no fragment derivative and no second
pipeline. It rides the existing vertex buffer and the existing draw call
(`CANVAS-062`). That is what keeps a WebGL2 back end drawing the same frame as a WebGPU one.

Two properties follow from where the tessellator builds the band. The band lives in model space
like the rest of the mesh. Its width divides by the transform's scale exactly as
`PATH_TOLERANCE_PX` does, so a zoomed board keeps a one-pixel edge rather than a thickening halo
(`CANVAS-063`). The tessellator builds it from the outline alone, never from the interior
triangulation. No edge inside the shape softens and no seam crosses it (`CANVAS-064`). A coverage
term computed per fragment would produce that failure instead (section 4).

**Images.** `ImageSource<'a> { pixels: &'a [u8], width: u32, height: u32 }` is a borrow over
already-decoded RGBA8, row-major and without stride padding (`CANVAS-026`). Decoding is the
caller's job. The back end scales the image to fill `dest`, so the aspect ratio is the caller's too.

| Back end | What `draw_image` does | State |
|---|---|---|
| `KrillaSurface` | hands the pixels to krilla as a raster image, under its `raster-images` feature | built |
| `WgpuSurface` | uploads the pixels once, keeps the texture under a budget, and draws a textured quad through the active transform | built; what has no caller is the cache that would hand it a board's pixels (`3-Toile/features/ambre-spec-toile-media.md`) |
| a back end without raster support | the default no-op, so a text and vector export still succeeds (`CANVAS-030`) | built |

An element whose pixels have not reached the walk still paints the placeholder rectangle of
`3-Toile/features/ambre-spec-toile-media.md`; the trait carries no placeholder logic of its own.

**The screen path uploads a picture once and samples it through a chain** (`CANVAS-065`,
`CANVAS-066`). Two properties make the screen path what it is. `ImageSource` carries a `key`, an
identity its caller already has: Toile's own media address. The cache therefore recognises a
texture on the next frame, instead of hashing the buffer to find it. The CPU reduces the mip chain
at upload, because the alternatives are a render pass per level or a compute dispatch.
`ambre-spec-frame.md section 4` refuses that dispatch.

The sampler is trilinear, so a picture crossing between two levels under a zoom does it without a
step. A VRAM budget bounds the uploads and evicts the least recently drawn first
(`ambre-spec-budgets.md section 6`). Section 5 says what that budget does when it binds.

The walk draws pictures between the shapes and the text, one draw per run of consecutive quads
sharing a texture. That is a paint order, not the board's z-order. A shape drawn after a picture
still lands under it, exactly as text already lands over both. The order Toile means is the
`paint-order rewrite` Story (`ambre-roadmap.md` E6).

**The PDF back end.** The drawing API is on `krilla::surface::Surface`, obtained from
`page.surface()`, so the wrapper borrows the surface rather than the page. Krilla is top-left and
Y-down like this trait, so no coordinate flips. The clip stack and the transform stack both map
onto krilla's single instruction stack. Each push is a push, each pop is a `Surface::pop`, and
`CANVAS-027` is what keeps them paired.

**Code:** `crates/ambre-draw/src/lib.rs` holds the trait, `ImageSource` and the re-exports;
`crates/ambre-draw/src/testing.rs` holds `RecordingSurface` behind the `testing` feature;
`crates/ambre-canvas/src/render/surface.rs` holds `WgpuSurface` and the tessellation;
`crates/ambre-pdf/src/lib.rs` holds `KrillaSurface`.

---

## 3. SURFACE

The user touches no drawing call. What they see of this document is that two destinations agree.

| Trigger | Behaviour |
|---|---|
| the content is drawn on screen | one walk over the document, through the screen surface |
| the document is exported to PDF | the same walk, through the PDF surface, in the Logic thread |
| the export runs for the first time | krilla is fetched then, not at startup |
| a shape crosses a clip edge | it is drawn whole on screen, which is the defect of section 5 |
| a shape carries a gradient | it paints as nothing, which is the second defect of section 5 |
| text is selected in an exported PDF | nothing is selected: the glyphs carry no character mapping |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One trait, in a crate that draws nothing | a rendering trait in `ambre-canvas` | it drags wgpu into every application and into every test. The trait as it stands is what lets an application render without naming a GPU type (`CANVAS-002`) and test its whole render walk on the host tier against `RecordingSurface` |
| One trait, in a crate that draws nothing | a pair of render walks, one per destination | two walks over the same document are two things to keep in step, and the export diverges from the screen with nothing to report it |
| The PDF back end is its own crate | `KrillaSurface` inside the first application that needed a PDF | PDF export translates the same primitives whatever the document is, so inside an application crate it would be written a second time by the second application to want it. As `ambre-pdf` it serves every application and is billed once in `ambre-spec-budgets.md section 5` |
| krilla, with default features off | krilla with its defaults | the simple-text helper and rayon buy nothing here -- the glyph runs are already shaped, and there is no thread pool on the WASM target -- while they cost weight in a chunk that is fetched on a user's first export |
| krilla, with default features off | a PDF writer of our own | the format is the dependency either way, and writing it here means owning fonts, compression and conformance for a chunk that is fetched on a user's first export |
| An optional primitive defaults to a no-op | every method required | it is the same trait with a worse migration: each new primitive breaks every surface on the day it is declared. With a no-op default a back end gains a capability when it can, and until then an export loses that one element instead of failing |
| An image arrives decoded | the surface decoding what it is handed | a codec dependency in `ambre-draw` reaches every crate that draws, including the ones that never show a picture, and it puts a parser for untrusted bytes in the middle of the render path |
| The silhouette is softened by a tessellated band | MSAA on the offscreen | it is the one antialiasing a mobile frame cannot absorb: a multisampled attachment multiplies the bandwidth of the render target on every frame, including the frames where nothing moved, and the device density that makes the aliasing bearable is exactly the one that makes the attachment expensive (`ambre-spec-budgets.md section 1`) |
| The silhouette is softened by a tessellated band | a supersampled offscreen, resolved on blit | it is uniformly correct and it costs four times the fragments and four times the target memory, on top of a buffer already sized in physical pixels. At a device ratio of 3 it renders the page at nine times its CSS area to soften an edge |
| The silhouette is softened by a tessellated band | a coverage term per triangle, from the fragment derivatives | the tessellator emits a triangle soup, so every internal edge is a boundary too: two adjacent triangles each cover half of the edge they share, the blend lands short of opaque, and a pale seam is drawn along every edge inside the shape (`CANVAS-064`). It is cheaper than the band and it looks worse than the aliasing it removes |
| The silhouette is softened by a tessellated band | a compute-shader path rasterizer (`vello`) | the same refusal `ambre-spec-frame.md section 4` already made for the pipeline, and for the same reasons: serial compute passes cost more per dispatch in a browser than the frame has to spend, and a WebGL2 back end has no compute stage at all. Antialiasing is not the reason to reopen it, and the two conditions that would are stated there (`CANVAS-062`) |
| The clip and transform stacks are the caller's to balance | a surface that closes what the caller left open | it hides the walk's bug and picks an arbitrary order to close it in. krilla has one instruction stack and the screen surface has a vector, so a contract both obey is cheaper than a stack that guesses: an unbalanced push is a bug in the walk, and it shows up as a corrupt page |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A clip is a bounding-box reject, not a trim.** The walk drops a path whose transformed and
  stroke-padded box misses the active clip, and draws a path straddling the edge whole. Trimming
  arbitrary geometry on the CPU is not cheap the way an axis-aligned rectangle, line or glyph is.
  A GPU scissor for the axis-aligned case, or a stencil for the general one, lands with
  `Clipped paths on the canvas` (`ambre-roadmap.md` E6).
* **A gradient brush draws nothing.** `fill_path` and `stroke_path` honour `Brush::Solid` alone,
  which matches `draw_glyphs`. Nothing states what a caller should expect, so the gradient
  disappears in silence.
* **A stroke scales with the zoom.** Tessellation is in model units, so the transform scales the
  stroke width. That is right for a board that zooms and wrong for a hairline, which needs the
  width divided by the transform scale. It lands with the same Story as the clip.
* **Only the glyphs reach the screen antialiased** (`CANVAS-061`). Every screen pipeline runs at
  one sample (`sample_count: 1`, the default `MultisampleState`), and the solid shader returns its
  vertex colour with no coverage term. The final blit is `FilterMode::Nearest` by design, because
  the offscreen is 1:1 with the swapchain. So a diagonal or a curve is a hard staircase.
  * Glyphs escape it because the CPU rasterizes their coverage into an `R8Unorm` atlas, which the
    shader then multiplies in. The vector geometry has no equivalent. A high-density display hides
    it by paying twice the pixels (`ambre-spec-high-density.md`). That is why it reads as
    acceptable on a developer's machine and not on a 1x monitor.
  * The band of section 2 has a design and no build. The three rules around it (`CANVAS-062`,
    `CANVAS-063`, `CANVAS-064`) exist to keep its construction from drifting into the alternatives
    section 4 refuses.
* **Every upload lands in the frame that first draws it** (`CANVAS-065`). `paint_content` decodes,
  reduces and uploads a picture, so twenty pictures becoming visible at once cost twenty chains in
  that one frame. That is a stall exactly when the user opens a board.
  * The chain itself is cheap and paid once: the levels under level 0 add a third to the pixel
    count, not a multiple per level. The zoom costs nothing at all, because the sampler picks a
    level from its own derivatives, so no frame recomputes anything.
  * What is missing is an upload budget per frame. It would let the last pictures arrive a frame or
    two later, instead of holding the first frame back.
* **The texture budget thrashes before it refuses** (`CANVAS-066`). Zoomed far out over more
  pictures than `IMAGE_VRAM_BUDGET` holds, the eviction drops an entry the next frame asks for
  again. The renderer therefore re-uploads and re-reduces the same images every frame. What the
  eviction throws away is mostly levels that a far-out zoom never samples.
  * The fix is to truncate the chain to the scale the board actually draws a picture at. That needs
    the decoded pixels kept somewhere to re-upload from when the zoom comes back in: the CPU-side
    cache of `CANVAS-007`, which is unbuilt. Nothing measures where the budget starts to bind.
* **PDF text is not selectable.** A glyph run carries no `ToUnicode` mapping, so a reader shows the
  shapes and copies nothing. Threading the cluster source text through is a refinement nothing has
  scheduled.
* **A walk's view is trusted for what it carries.** The gate reads what a walk reaches, not what
  its caller puts in the view. Prose hands the walk the composition at the caret, so a page
  export that built its view from the screen's state would print it.
* **No golden PDF exists.** `ambre_pdf::tests` asserts that the bytes parse as a valid document and
  that four primitives reach it. Nothing compares a rendered page against a reference, so a
  regression in placement passes.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| When a frame runs, and which layers it rebuilds | `ambre-spec-frame.md` |
| The fonts, the glyph atlases and the decoded images the calls consume | `ambre-spec-render-assets.md` |
| What a high-density display changes about a tessellation tolerance | `ambre-spec-high-density.md` |
| How a peer's colour is derived from a `UserId` | `ambre-spec-presence.md section 3` |
| The crate graph these rules are read off, and the binary budget | `ambre-spec-architecture.md sections 2.E and 5.E` |
| What each application puts in its exported page, and what it drops | `2-Trame/features/ambre-spec-trame-pdf-export.md`, `3-Toile/features/ambre-spec-toile-pdf-export.md`, `4-Prose/features/ambre-spec-prose-pdf-export.md` |
| Which crate holds which test belt, and what runs in CI | `ambre-spec-testing.md section 6` |
