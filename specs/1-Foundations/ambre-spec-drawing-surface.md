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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CANVAS-002` | Application render code MUST NOT name a GPU type: it draws through the shared surface alone. | this spec section 2 | crate graph: neither `ambre-trame` nor `ambre-toile` declares `wgpu`, and their render tests drive `RecordingSurface` on host | verified |
| `CANVAS-025` | The drawing crate MUST declare no GPU and no PDF dependency. | this spec section 2; `ambre-spec-architecture.md section 2` | crate graph: `ambre-draw` declares `kurbo`, `peniko`, `parley` and `ambre-core`, and a `wgpu` or `krilla` line added there fails the build of every host-tier render test | verified |
| `CANVAS-026` | The drawing crate MUST decode nothing: a raster image MUST arrive as pixels the caller already decoded. | this spec section 2 | crate graph: `ambre-draw` declares no codec, and `ImageSource` borrows a slice it never owns | verified |
| `CANVAS-027` | Every `push_clip` and `push_transform` MUST be matched by its pop before the surface is finished. | this spec section 2 | `ambre_pdf::tests::zero_area_clip_stays_balanced` holds the degenerate case, where an unbalanced push corrupts the rest of the page | verified |
| `CANVAS-028` | The PDF back end MUST run in the Logic thread, and MUST NOT be in the startup bundle. | this spec section 2; `ambre-spec-budgets.md section 5` | crate graph: `ambre-app` declares neither `krilla` nor `ambre-pdf`, and Toile's `pdf` feature is off by default, so nothing links krilla into a client build | verified |
| `CANVAS-029` | One render walk MUST serve the screen and the page: a drawing primitive MUST NOT hold screen-only state. | this spec section 2 | none. The trait declares no screen type, so the rule holds by its signature. Nothing refuses a back end that reads a frame counter or a device pixel ratio behind it | unverified |
| `CANVAS-030` | A surface that does not implement an optional primitive MUST do nothing, never fail. | this spec section 2 | type-level: each optional method carries a default empty body, so a back end that omits it compiles and draws nothing | verified |

* **Budget.** the krilla row of `ambre-spec-budgets.md section 5` -- about 400 KB raw, about
  120 KB gzipped, counted once for the whole suite and kept out of the startup bundle by
  `CANVAS-028`. The drawing calls themselves are charged to the vertex-batch and render-pass phases
  of `ambre-spec-budgets.md section 2`.
* **Boundary.** none: a surface writes to a texture or to a byte buffer, holds no key and opens no
  socket. A PDF the user exports leaves the machine only when the user sends it, and
  `ambre-spec-docfile.md` owns what a plaintext export owes them.
* **Verification.** 6 `verified`, 1 `unverified`, 0 `unimplemented`. This is the best-held document
  of the rendering layer, and the reason is structural: five of the six rules are held by the crate
  graph or by the compiler, which the frame path cannot use because a frame is a runtime property.
  The one gap, `CANVAS-029`, is a rule about what an implementation must refrain from storing, and
  no crate-graph check can read that.

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

**Images.** `ImageSource<'a> { pixels: &'a [u8], width: u32, height: u32 }` is a borrow over
already-decoded RGBA8, row-major and without stride padding (`CANVAS-026`). Decoding is the
caller's job. The image is scaled to fill `dest`, so the aspect ratio is the caller's too.

| Back end | What `draw_image` does | State |
|---|---|---|
| `KrillaSurface` | hands the pixels to krilla as a raster image, under its `raster-images` feature | built |
| `WgpuSurface` | uploads the pixels to a texture and draws a textured quad through the active transform | design; it lands with the screen media cache of `ambre-spec-render-assets.md` |
| a back end without raster support | the default no-op, so a text and vector export still succeeds (`CANVAS-030`) | built |

Until the screen path lands, an image element paints the placeholder rectangle of
`3-Toile/features/ambre-spec-toile-media.md`, and the trait carries no placeholder logic of its own.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One trait, in a crate that draws nothing | it is what lets an application render without naming a GPU type (`CANVAS-002`) and test its whole render walk on the host tier against `RecordingSurface`. A trait declared in the wgpu crate would drag wgpu into every application and into every test | a rendering trait in `ambre-canvas`, or a pair of render walks, one per destination |
| The PDF back end is its own crate | PDF export translates the same primitives whatever the document is, so `ambre-pdf` serves every application and is billed once in `ambre-spec-budgets.md section 5`. Inside an application crate it would be written a second time by the second application to want it | `KrillaSurface` inside the first application that needed a PDF |
| krilla, with default features off | the simple-text helper and rayon buy nothing here -- the glyph runs are already shaped, and there is no thread pool on the WASM target -- while they cost weight in a chunk that is fetched on a user's first export | krilla with its defaults, and a PDF writer of our own |
| An optional primitive defaults to a no-op | a back end gains a capability when it can, and until then an export loses that one element instead of failing. A required method would mean each new primitive breaks every surface on the day it is declared | every method required, which is the same trait with a worse migration |
| An image arrives decoded | a codec dependency in `ambre-draw` reaches every crate that draws, including the ones that never show a picture, and puts a parser for untrusted bytes in the middle of the render path | the surface decoding what it is handed |
| The clip and transform stacks are the caller's to balance | krilla has one instruction stack and the screen surface has a vector, so a contract that both obey is cheaper than a stack that guesses. An unbalanced push is a bug in the walk, and it shows up as a corrupt page | a surface that closes what the caller left open, which hides the walk's bug and picks an arbitrary order to close it in |

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
