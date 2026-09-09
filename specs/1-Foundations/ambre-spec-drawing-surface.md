# AMBRE -- FEATURE: THE DRAWING SURFACE

**Type:** Horizontal (framework)
**Implementation:** partial -- the trait, the screen back end and the PDF back end ship; clipping
is a bounding-box reject and a gradient brush is ignored.

> The dozen calls every renderer is allowed to make: a rectangle, a line, a path, a glyph run, an
> image, a clip, a transform. One walk over the document draws it, and the surface underneath
> decides whether the result is a screen or a PDF page.

An application that exported by walking its document a second time would drift: the page would stop
matching the screen one feature at a time. So there is one walk, and the difference between the two
destinations is which surface it is handed.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-render-assets.md` and `ambre-spec-high-density.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `CANVAS-002` | `INV` | Application render code MUST NOT name a GPU type: it draws through the shared surface alone. | this spec section 2 | crate graph: neither `ambre-trame` nor `ambre-toile` declares `wgpu`, and their render tests drive `RecordingSurface` on host | verified |
| `CANVAS-025` | `INV` | The drawing crate MUST declare no GPU and no PDF dependency. | this spec section 2; `ambre-spec-architecture.md section 2` | crate graph: `ambre-draw` declares `kurbo`, `peniko`, `parley` and `ambre-core`, and a `wgpu` or `krilla` line added there fails the build of every host-tier render test | verified |
| `CANVAS-026` | `INV` | The drawing crate MUST decode nothing: a raster image MUST arrive as pixels the caller already decoded. | this spec section 2 | crate graph: `ambre-draw` declares no codec, and `ImageSource` borrows a slice it never owns | verified |
| `CANVAS-055` | `INV` | The PDF back end MUST NOT be in the startup bundle. | this spec section 2; `ambre-spec-budgets.md section 5` | crate graph: `ambre-app` declares neither `krilla` nor `ambre-pdf`, and Toile's `pdf` feature is off by default, so nothing links krilla into a client build | verified |
| `CANVAS-028` | `INV` | The PDF back end MUST run in the Logic thread. | this spec section 2 | none. It is the half the crate graph does not hold: keeping krilla out of the startup bundle says nothing about which thread calls it, and a PDF built on the UI thread would block the frame for as long as the document is long | unverified |
| `CANVAS-030` | `INV` | A surface that does not implement an optional primitive MUST do nothing, never fail. | this spec section 2 | type-level: each optional method carries a default empty body, so a back end that omits it compiles and draws nothing | verified |
| `CANVAS-061` | `OBS` | A vector edge MUST reach the screen antialiased. | this spec section 2 | not built -- lands with the "Antialiased vector geometry" Story (`ambre-roadmap.md` E6); every screen pipeline runs at one sample and the solid shader returns its vertex colour unmodified, so a diagonal is a hard staircase and only glyphs escape, through their own coverage atlas | unimplemented |
| `CANVAS-063` | `OBS` | An antialiased edge MUST keep its width in screen pixels at every zoom. | this spec section 2 | not built -- lands with the "Antialiased vector geometry" Story (`ambre-roadmap.md` E6); tessellation is in model space, so a width in model units is a band that thickens as the board zooms in | unimplemented |
| `CANVAS-064` | `OBS` | Antialiasing MUST NOT mark the inside of a shape: only its silhouette may be softened. | this spec section 2 | not built -- lands with the "Antialiased vector geometry" Story (`ambre-roadmap.md` E6); it is the rule that separates a contour band from a per-triangle coverage term, which softens every internal edge of the tessellation and draws a seam along each one | unimplemented |
| `CANVAS-065` | `OBS` | A minified picture MUST be sampled through a mip chain, never through its full-size texels alone. | this spec section 2 | `ambre_canvas::render::mipmap::tests::the_chain_halves_down_to_one_texel`, `::a_flat_image_keeps_halving_its_long_axis` hold the descent, `::a_reduced_texel_is_the_mean_of_its_block` the averaging. That the sampler is the trilinear one is held by construction and by no test: no host tier reads a rendered frame (`ambre-spec-testing.md section 2`) | unverified |
| `CANVAS-066` | `INV` | An uploaded picture MUST be reused across frames, under a bounded budget that drops the least recently drawn first. | this spec section 2; `ambre-spec-budgets.md section 6` | `ambre_canvas::render::renderer::image_cache_tests::the_least_recently_drawn_entry_is_evicted_first`, `::a_hit_stamps_the_entry_with_the_current_frame`, `::reinserting_a_key_replaces_its_cost`, `::a_key_at_another_size_is_a_miss`; the run merge that keeps one picture to one draw is `::consecutive_quads_of_one_picture_are_one_run` | verified |

### 1.2 Draw one document onto a surface

One walk over the document. What it is handed decides whether the result is a screen or a page, and
the walk is not told which.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `CANVAS-029` | `POST` | One render walk MUST serve the screen and the page. | this spec section 2 | none. The trait declares no screen type, so the rule holds by its signature, and no test walks one document onto both surfaces and compares what each recorded | unverified |
| `CANVAS-027` | `POST` | Every `push_clip` and `push_transform` MUST be matched by its pop before the surface is finished. | this spec section 2 | `ambre_pdf::tests::zero_area_clip_stays_balanced` holds the degenerate case, where an unbalanced push corrupts the rest of the page | verified |
| `CANVAS-056` | `FRAME` | A drawing primitive MUST NOT hold screen-only state. | this spec section 2 | none. It is the half `CANVAS-029` carried, and the one an implementation breaks rather than a caller: nothing refuses a back end that reads a frame counter or a device pixel ratio behind the trait, and the page then stops matching the screen one primitive at a time | unverified |
| `CANVAS-062` | `FRAME` | Antialiasing MUST NOT add a render pass, a compute dispatch or a multisampled attachment. | this spec section 2; `ambre-spec-frame.md section 4` | not built -- lands with the "Antialiased vector geometry" Story (`ambre-roadmap.md` E6); it is the rule that keeps the edge treatment inside the decision `ambre-spec-frame.md` already took against a compute rasterizer, and a WebGL2 back end has no compute stage to run one on | unimplemented |

* **Budget.** the krilla row of `ambre-spec-budgets.md section 5` -- about 400 KB raw, about
  120 KB gzipped, counted once for the whole suite and kept out of the startup bundle by
  `CANVAS-028`. The drawing calls themselves are charged to the vertex-batch and render-pass phases
  of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: a surface writes to a texture or to a byte buffer, holds no key and opens no
  socket. A PDF the user exports leaves the machine only when the user sends it, and
  `ambre-spec-docfile.md` owns what a plaintext export owes them.
* **Verification.** 7 `verified`, 4 `unverified`, 4 `unimplemented`. This is still the best-held
  document of the rendering layer, and the reason is structural: five of the six verified rules are
  held by the crate graph or by the compiler, which the frame path cannot use because a frame is a
  runtime property. The pass split the two rows whose second half no crate graph can read.
  `CANVAS-028` claimed the PDF back end runs on the Logic thread and stays out of the startup
  bundle, on one crate-graph mechanism that holds only the second: keeping krilla unlinked says
  nothing about which thread calls it, and a PDF built on the UI thread blocks the frame for as
  long as the document is long. `CANVAS-029` claimed one walk serves both destinations and that no
  primitive holds screen-only state; the second is what an implementation breaks rather than a
  caller, and it is `CANVAS-056`.

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
    fn stroke_line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, color: Color, width: f64);

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

**Paths, on the screen.** A `BezPath` becomes a `lyon::path::Path` and is tessellated in model
space: a fill uses the non-zero rule, a stroke maps the width, cap, join and miter limit of
`kurbo::Stroke`. The flattening tolerance is expressed in screen pixels and divided by the
transform's scale -- the square root of the linear part's determinant -- so a zoomed path stays
smooth. The output vertices go through the active transform into the solid-colour quad batch, with
no pipeline of their own.

**Edges.** A filled or stroked path reaches the screen as a triangle soup with a hard boundary, so
its silhouette is softened by geometry rather than by the pipeline (`CANVAS-061`): the tessellator
emits, around the outline it already walked, a band whose outer vertices carry zero alpha and whose
inner vertices carry the fill's own. The interpolator does the gradient, so no fragment derivative
and no second pipeline is involved -- the band rides the existing vertex buffer and the existing
draw call (`CANVAS-062`), which is what keeps a WebGL2 back end drawing the same frame as a WebGPU
one.

Two properties follow from where the band is generated. It is built in model space like the rest of
the mesh, so its width is divided by the transform's scale exactly as `PATH_TOLERANCE_PX` is, and a
zoomed board keeps a one-pixel edge rather than a thickening halo (`CANVAS-063`). And it is built
from the outline alone, never from the interior triangulation, so no edge inside the shape is
softened and no seam is drawn across it (`CANVAS-064`) -- the failure that a coverage term computed
per fragment would produce instead (section 4).

**Images.** `ImageSource<'a> { pixels: &'a [u8], width: u32, height: u32 }` is a borrow over
already-decoded RGBA8, row-major and without stride padding (`CANVAS-026`). Decoding is the
caller's job. The image is scaled to fill `dest`, so the aspect ratio is the caller's too.

| Back end | What `draw_image` does | State |
|---|---|---|
| `KrillaSurface` | hands the pixels to krilla as a raster image, under its `raster-images` feature | built |
| `WgpuSurface` | uploads the pixels once, keeps the texture under a budget, and draws a textured quad through the active transform | built; what has no caller is the cache that would hand it a board's pixels (`3-Toile/features/ambre-spec-toile-media.md`) |
| a back end without raster support | the default no-op, so a text and vector export still succeeds (`CANVAS-030`) | built |

An element whose pixels have not reached the walk still paints the placeholder rectangle of
`3-Toile/features/ambre-spec-toile-media.md`; the trait carries no placeholder logic of its own.

**A picture is uploaded once and sampled through a chain** (`CANVAS-065`, `CANVAS-066`). Two
properties make the screen path what it is. `ImageSource` carries a `key` -- an identity its caller
already has, Toile's own media address -- so a texture is recognised on the next frame instead of
the buffer being hashed to find it; and the mip chain is reduced on the CPU at upload, because the
alternatives are a render pass per level or a compute dispatch, and the dispatch is the stage
`ambre-spec-frame.md section 4` refuses. The sampler is trilinear, so a picture crossing between two
levels under a zoom does it without a step. Uploads are bounded by a VRAM budget and evicted
least-recently-drawn (`ambre-spec-budgets.md section 6`); what that budget does when it binds is
section 5.

Pictures are drawn between the shapes and the text, one draw per run of consecutive quads sharing a
texture. That is a paint order, not the board's z-order: a shape drawn after a picture still lands
under it, exactly as text already lands over both, and the order Toile means is the
`paint-order rewrite` Story (`ambre-roadmap.md` E6).

**The PDF back end.** The drawing API is on `krilla::surface::Surface`, obtained from
`page.surface()`, so the wrapper borrows the surface rather than the page. Krilla is top-left and
Y-down like this trait, so no coordinate flips. The clip stack and the transform stack both map
onto krilla's single instruction stack: each push is a push, each pop is a `Surface::pop`, and
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

* **A clip is a bounding-box reject, not a trim.** A path whose transformed and stroke-padded box
  misses the active clip is dropped whole; a path straddling the edge is drawn whole. Trimming
  arbitrary geometry on the CPU is not cheap the way an axis-aligned rectangle, line or glyph is.
  A GPU scissor for the axis-aligned case, or a stencil for the general one, lands with
  `Clipped paths on the canvas` (`ambre-roadmap.md` E6).
* **A gradient brush is ignored.** `fill_path` and `stroke_path` honour `Brush::Solid` alone, which
  matches `draw_glyphs`. Nothing states what a caller should expect, so a gradient silently paints
  nothing.
* **A stroke scales with the zoom.** Tessellation is in model units, so the transform scales the
  stroke width. That is right for a board that zooms and wrong for a hairline, which needs the
  width divided by the transform scale. It lands with the same Story as the clip.
* **Nothing is antialiased but the glyphs** (`CANVAS-061`). Every screen pipeline is built at one
  sample (`sample_count: 1`, the default `MultisampleState`), the solid shader returns its vertex
  colour with no coverage term, and the final blit is `FilterMode::Nearest` by design -- the
  offscreen is 1:1 with the swapchain. So a diagonal or a curve is a hard staircase. Glyphs escape
  it because their coverage is rasterized on the CPU into an `R8Unorm` atlas and multiplied in the
  shader; the vector geometry has no equivalent. A high-density display hides it by paying twice the
  pixels (`ambre-spec-high-density.md`), which is why it reads as acceptable on the machines it is
  developed on and not on a 1x monitor. The band of section 2 is designed and unbuilt; the three
  rules around it (`CANVAS-062`, `CANVAS-063`, `CANVAS-064`) exist to keep its construction from
  drifting into the alternatives section 4 refuses.
* **Every upload lands in the frame that first draws it** (`CANVAS-065`). A picture is decoded,
  reduced and uploaded inside `paint_content`, so twenty pictures becoming visible at once cost
  twenty chains in that one frame -- a stall exactly when a board is opened. The chain itself is
  cheap and paid once (the levels under level 0 add a third to the pixel count, not a multiple per
  level), and the zoom costs nothing at all: the sampler picks a level from its own derivatives, so
  no frame recomputes anything. What is missing is an upload budget per frame, which would let the
  last pictures arrive a frame or two later instead of holding the first frame back.
* **The texture budget thrashes before it refuses** (`CANVAS-066`). Zoomed far out over more
  pictures than `IMAGE_VRAM_BUDGET` holds, the eviction drops an entry the next frame asks for
  again, so the same images are re-uploaded and re-reduced every frame -- and the pyramid being
  evicted is mostly levels that far-out zoom never samples. Truncating the chain to the scale a
  picture is actually drawn at is what fixes it, and that needs the decoded pixels kept somewhere to
  re-upload from when the zoom comes back in: the CPU-side cache of `CANVAS-007`, which is unbuilt.
  Nothing measures where the budget starts to bind.
* **PDF text is not selectable.** A glyph run carries no `ToUnicode` mapping, so a reader shows the
  shapes and copies nothing. Threading the cluster source text through is a refinement nothing has
  scheduled.
* **`CANVAS-029` is held by a signature alone.** The trait declares no screen type, and nothing
  refuses an implementation that stores frame-local state behind it.
* **No golden PDF exists.** `ambre_pdf::tests` asserts that the bytes parse as a valid document and
  that four primitives reach it; nothing compares a rendered page against a reference, so a
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
