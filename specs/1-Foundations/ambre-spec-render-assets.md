# AMBRE -- FEATURE: THE RENDER ASSETS

**Type:** Horizontal (framework)
**Implementation:** partial -- the fonts, the two glyph atlases and the layout cache ship; the icon
pipeline and the shared image cache are designs.

> What the renderer loads before it can draw anything: the fonts a script needs, the glyphs
> rasterized from them, the icons, and the images a document embeds. Loaded once, cached bounded,
> shared by every application.

A user who types Japanese, pastes an emoji and drops a photograph asks the same thing of the
renderer three times: find the resource, put it on the GPU, and do not keep it for ever. The rules
below are that, and nothing about how a frame is drawn (`ambre-spec-frame.md`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-drawing-surface.md` and `ambre-spec-high-density.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CANVAS-006` | A script-specific font MUST be fetched only when text needing it appears, and MUST be fetched once. | this spec section 2 | `ambre_canvas::fonts::tests::observe_latches_each_script_once`, `::latin_and_symbols_never_trigger`, `::drain_is_empty_without_fetches` | verified |
| `CANVAS-007` | Every decoded image MUST pass through one bounded cache, whichever application decoded it. | this spec section 2; `ambre-spec-budgets.md section 6` | not built -- lands with `One media cache` (`ambre-roadmap.md` E8). Toile holds a `MediaCache` in its own Logic module and no other application has one | unimplemented |
| `CANVAS-017` | Every font family label MUST resolve to a loaded face before shaping; an unmatched label MUST NOT reach the shaper. | this spec section 2 | `ambre_canvas::text_cache::tests::family_labels_map_to_registered_faces`, `::registered_face_names_pass_through` | verified |
| `CANVAS-018` | A font MUST NOT be embedded in a binary: it MUST be fetched at run time, from the same origin. | this spec section 3; `ambre-spec-budgets.md section 5` | none. No manifest and no source names a font file, so the rule holds by absence. Nothing refuses the `include_bytes!` that would break it, and `UI-THREAD-004` holds the same-origin half alone | unverified |
| `CANVAS-019` | A layout mixing text and colour emoji MUST paint both: a colour glyph through the colour atlas, a mask glyph through the alpha atlas. | this spec section 2 | none. An `is_color` flag on the atlas slot routes each glyph to its pipeline, and no test reads a pixel back. A regression paints a black box where an emoji belongs | unverified |
| `CANVAS-020` | The colour atlas MUST NOT be allocated at full size before the first colour glyph appears. | this spec section 2; `ambre-spec-budgets.md section 6` | none. The texture is a 1x1 placeholder until `color_ready` flips, and nothing asserts the 16 MB allocation is absent from an emoji-free session | unverified |
| `CANVAS-021` | The glyph atlas MUST hold a bounded working set: on overflow it MUST repack by least recent use, never grow. | this spec section 2; `ambre-spec-budgets.md section 6` | none. The compaction retains each glyph's bitmap so a repack re-uploads without re-rasterizing, and no test drives an atlas past its capacity | unverified |
| `CANVAS-022` | The layout cache MUST be bounded: it MUST evict by least recent use, never clear itself whole. | this spec section 2 | none. `evict_if_oversized` keeps the hottest three quarters above a cap the frame passes in, and no test drives the cache past that cap | unverified |
| `CANVAS-023` | An icon MUST stay a vector buffer: it MUST NOT be rasterized into a bitmap atlas. | this spec section 2 | not built -- lands with `The icon pipeline` (`ambre-roadmap.md` E6) | unimplemented |
| `CANVAS-024` | An icon MUST be tessellated at build time, never on the frame path. | this spec section 2 | not built -- lands with `The icon pipeline` (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** four rows of `ambre-spec-budgets.md section 6` -- fonts in memory under 10 MB,
  the wgpu textures under 20 MB with the 4 MB alpha atlas and the lazily allocated 16 MB colour
  atlas inside them, the icon atlas under 1 MB -- plus the `< 1 ms` cold-cache shape figure of
  `section 2` and, on the binary side, the `usvg` plus `lyon`, `swash`, `image` and icon-blob
  rows of `section 5`. None of the six is read by a check.
* **Boundary.** the font fetch is the one network path this layer opens, and `UI-THREAD-004` names
  it as the single exception to the UI thread's ban. It is same-origin by `CANVAS-018`, so no
  third party learns which scripts a user reads. This layer holds no key and reaches no relay.
* **Verification.** 2 `verified`, 5 `unverified`, 3 `unimplemented`. The two verified rules are the
  two that are pure functions of a string: which script a codepoint belongs to, and which face a
  family label resolves to. The five gaps are all GPU-resident state -- an atlas, a texture, a
  cache -- and reading one back needs the browser tier that does not exist
  (`ambre-spec-testing.md section 8`).

---

## 2. MODEL

**CRDT containers.** none. An asset is derived from the document, never stored in it.

**The three font tiers.** One shape, three lifetimes.

```rust
pub struct FontConfig {
    /// Base fonts loaded via fetch() from the Ambre server (same origin)
    pub base: Vec<FontData>,
    /// Additional fonts loaded on demand (CJK, Arabic, Devanagari, color emoji...)
    pub lazy: Vec<FontData>,
    /// System fonts detected (native only -- not available on WASM)
    pub system: Vec<FontData>,
}
```

| Tier | Faces | When it loads | Coverage |
|---|---|---|---|
| base | Noto Sans in four styles, DejaVu Serif, DejaVu Sans Mono | at startup, one `fetch` per thread | Latin, Latin Extended, Cyrillic, Greek |
| lazy | CJK, Arabic, Devanagari, and Noto Color Emoji | the first time Parley meets a codepoint the base faces do not cover (`CANVAS-006`) | one face per script, fetched once |
| system | whatever `fontdb` finds | never on the web target | a native desktop, which is not a target today |

The UI thread and the Logic thread each fetch independently and the browser's HTTP cache
deduplicates: one download, two `FontContext` in one linear memory. All bundled faces are OFL 1.1
or equally permissive (`crates/ambre-app/fonts/LICENSES.md`). Under Trunk the files sit in
`crates/ambre-app/fonts/` and are copied to `dist/fonts/`.

**Family resolution is shared** (`CANVAS-017`). Every renderer and every inline editor maps a
toolbar label -- `Sans`, `Serif`, `Monospace`, `Default`, a CSS generic, or an unknown string --
through `registered_family_name` to a loaded face. The mapping is idempotent: a face name maps to
itself. An unmatched `Named` family handed to Parley shapes zero glyphs, which is the failure this
rule exists to make impossible.

**Shaping.** Parley delegates to HarfRust and Skrifa, both pure Rust. The Unicode bidirectional
algorithm and the UAX #14 line-breaking algorithm are Parley's, so Arabic and Hebrew need no code
of ours.

**From a string to pixels.** Four stages, and a cache between each pair.

1. `TextCache` keys a Parley `Layout` on the text, the size, the wrap width, the alignment, the
   weight, the slant, the emergency-break flag and the family. A hit costs nothing; a miss shapes
   (`CANVAS-022`).
2. every string the cache shapes is offered to the lazy loader, which fetches a face the first time
   it sees a codepoint the base faces cannot cover (`CANVAS-006`).
3. Swash rasterizes each glyph, keyed on the font blob id, the glyph id and the size.
4. the glyph lands in one of two atlases, and its slot records which (`CANVAS-019`).

| Atlas | Format | Size | Holds | Allocation |
|---|---|---|---|---|
| alpha | `R8Unorm` | 2048x2048 | every mask glyph, tinted by the vertex colour | at startup |
| colour | `Rgba8Unorm` | 2048x2048 | a `Content::Color` glyph, straight alpha, drawn untinted | lazily, on the first colour glyph (`CANVAS-020`) |

Both use a shelf packer, a one-pixel pad against bilinear bleed, and LRU compaction that re-queues
retained bitmaps rather than calling Swash again (`CANVAS-021`). Two pipelines share the
`GlyphVertex` layout and the same blending; `draw_glyphs` routes each glyph by `slot.is_color`, so
one layout paints digits and emoji correctly.

**The emoji face is a bitmap build.** Noto Color Emoji in its CBDT form, because swash 0.2.7 does
not rasterize COLRv1. It is lazy, never in the startup bundle, and every application gets colour
emoji through `draw_glyphs` with no work of its own.

**The icons, as designed.** A vector pipeline with no run-time cost, and no code today.

```rust
/// Index into the compiled icon atlas. Each icon is a pre-tessellated vertex buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon(pub u16);
```

1. a build script scans `assets/icons/*.svg`;
2. `usvg` parses each file into curves;
3. the curves become a `lyon::path::Path`;
4. lyon tessellates it into `VertexBuffers<Vertex, u16>`;
5. the buffers are serialized into one blob, included with `include_bytes!`;
6. `Icon(u16)` indexes an offset in that blob.

At run time the blob loads once and each icon is one draw call through the paths pipeline
(`CANVAS-023`, `CANVAS-024`).

**The images.** A decoded image is RGBA8 pixels, produced by the `image` crate with only the PNG,
JPEG and WebP decoders compiled in. One bounded LRU serves every consumer -- a board picture, a
sheet chart, an inline illustration -- so the memory budget is one number rather than the sum of
three nobody set together (`CANVAS-007`). Toile is the heaviest consumer, so the **cap** may need
revisiting; the **mechanism** stays common.

**Code:** `crates/ambre-canvas/src/fonts.rs` holds the script classifier and the lazy loader,
`crates/ambre-canvas/src/text_cache.rs` the layout cache and `registered_family_name`, and
`crates/ambre-canvas/src/render/atlas.rs` the two atlases, the shelf packer and the compaction.
The icon pipeline and the shared image cache have no code.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the application starts | the base faces are fetched from the same origin and text is drawable |
| a character outside the base faces appears | its face is fetched once, and the text reflows when it arrives |
| an emoji appears | the colour atlas is allocated on the first one, and the emoji paints in colour |
| a font family is picked in a toolbar | the label resolves to a loaded face, and an unknown label falls back to the sans |
| the same string is drawn again | it is served from the layout cache with no shaping |
| a document embeds an image | it is decoded once and served from the shared cache on every later frame |
| an icon is drawn | its pre-tessellated vertex buffer is submitted, sharp at any density |

**Two rows above are designs.** No icon exists in the tree, and the shared image cache does not
exist: an image reaches a surface only through the path
`3-Toile/features/ambre-spec-toile-media.md` describes.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Fonts are fetched, never embedded | both the UI thread and the Logic thread need the same faces, and a fetch from the same origin is deduplicated by the browser's HTTP cache: one download, two contexts. It also takes about 600 KB of raw weight out of the WASM binary, which `ambre-spec-budgets.md section 5` counts | `include_bytes!`, which pays the weight twice and caches nothing |
| The fonts are served from the Ambre server | a font request carries the page that made it and the scripts the reader needs. Sent to a third party that is a profile of what a user reads, and it is sent on a document nobody else is supposed to see | Google Fonts or any other CDN |
| A script-specific face is fetched on first sight | bundling CJK, Arabic, Devanagari and colour emoji at startup costs every user the union of every script. Fetching on the first uncovered codepoint costs each user their own | one bundle covering everything, or a per-document declaration of the scripts it uses, which a paste falsifies immediately |
| Two atlases, not one | text dominates the working set and a mask glyph costs one byte a texel. One RGBA atlas for everything pays four times the memory so that emoji, which are rare, can share a pipeline with digits, which are not | a single `Rgba8Unorm` atlas, and a colour atlas allocated at startup, which spends 16 MB on documents that hold no emoji |
| The emoji face is the CBDT bitmap build | swash 0.2.7 rasterizes no COLRv1 table, so the vector build shapes and paints nothing. A bitmap emoji at atlas resolution is what the user actually sees | the COLRv1 build, which is smaller and currently invisible |
| Shaping is pure Rust | a C binding is an unaudited dependency inside the process that holds the plaintext document, and the pillar in `README.md` refuses it. HarfRust is HarfBuzz ported by the same people | HarfBuzz through a C binding |
| The family label is resolved in one place | an unmatched `Named` family shapes zero glyphs, so the failure is invisible text rather than an error. Resolving in each renderer means each one gets to have the bug once | each renderer mapping its own labels, which is what `CANVAS-017` withdrew |
| Icons stay vector | a tessellated path is sharp at any density, and `ambre-spec-high-density.md` then has nothing to say about icons. The tessellation is a build-time cost, so the frame path pays a draw call and nothing else | a bitmap texture atlas, which blurs on a high-density screen and needs a second set at each density |
| One image cache for the suite | three caches means the memory budget is the sum of three caps nobody set together, and a picture pasted from a board into a sheet is decoded twice | a decoded-image cache per application, which is what exists today |

---

## 5. NOT BUILT, AND NOT YET RULED

* **No icon exists.** There is no `assets/icons/` directory, no `Icon` type and no tessellation
  step: `crates/ambre-canvas/build.rs` generates the design tokens and nothing else. The whole
  pipeline of section 2 is a design (`The icon pipeline`, `ambre-roadmap.md` E6).
* **The shared image cache does not exist.** Toile holds a `MediaCache` in its Logic thread, keyed
  by content hash, and it is a fetch deduplicator rather than a decoded-pixel cache. No application
  shares it and no budget row is read against it (`CANVAS-007`).
* **No atlas is measured.** The 4 MB alpha atlas, the 16 MB colour atlas and the under 10 MB font
  budget are figures in `ambre-spec-budgets.md section 6` that nothing reports, so an
  overflow is discovered as a dropped frame.
* **The layout cache is bounded by its caller.** `evict_if_oversized` takes the cap as a parameter
  and the frame passes 5 000, so the bound is a literal in the render path rather than a budget.
* **Nothing rules on a font that fails to load.** A `fetch` for a lazy face can 404 or time out,
  and no row says what the text does then.
* **System fonts are named and unreachable.** `FontConfig::system` exists in the shape and the web
  target can never fill it. It stays for a native desktop that is not a target
  (`README.md`).

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| When a frame runs, and which layers it rebuilds | `ambre-spec-frame.md` |
| The call a renderer makes to paint a glyph or an image | `ambre-spec-drawing-surface.md` |
| What a high-density display changes about rasterization | `ambre-spec-high-density.md` |
| Why the UI thread may open this one network path at all | `ambre-spec-ui.md`, `UI-THREAD-004` |
| The memory, binary and shaping budgets these figures come from | `ambre-spec-budgets.md` |
| How a picture is uploaded, hashed, fetched and placed in a document | `3-Toile/features/ambre-spec-toile-media.md`, `4-Prose/features/ambre-spec-prose-images.md` |
| How a page is paginated in the Logic thread, and why it needs the same faces | `4-Prose/features/ambre-spec-prose-pagination.md` |
