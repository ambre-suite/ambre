# AMBRE -- FEATURE: THE RENDER ASSETS

**Type:** Horizontal (framework)
**Implementation:** partial -- the fonts, the two glyph atlases and the layout cache ship. The icon
pipeline and the shared image cache are designs.

> What the renderer loads before it can draw anything: the fonts a script needs, the glyphs
> rasterized from them, the icons, the embedded images. Loaded once, cached bounded, shared by every
> application.

A user who types Japanese, pastes an emoji and drops a photograph asks the same thing of the
renderer three times. Find the resource, put it on the GPU, do not keep it for ever. The rules
below are that, and nothing about how a frame is drawn (`ambre-spec-frame.md`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-drawing-surface.md` and `ambre-spec-high-density.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-017` | `INV` | Every font family label MUST resolve to a loaded face before shaping; an unmatched label MUST NOT reach the shaper. | this spec section 2 | `ambre_canvas::text_cache::tests::family_labels_map_to_registered_faces`; `::registered_face_names_pass_through` | none | verified |
| `CANVAS-081` | `INV` | A layout shaped with styled spans MUST be cached apart from the same text with other spans, and a span off the text MUST shape nothing. | this spec section 2; `MURMURE-565` | `ambre_canvas::text_cache::tests::each_set_of_spans_shapes_and_caches_apart` -- a bold span adds a face and widens the text, a span past the end shapes one face, and the same set hits | none | verified |
| `CANVAS-086` | `INV` | A layout MUST be cached apart for each alignment of the same text, so a justified or a left-aligned text is never served a start-aligned layout. | this spec section 2 | `ambre_canvas::text_cache::tests::each_alignment_of_a_text_is_cached_apart` -- six alignments of one text hold six entries, and the justified line lands its last glyph further right | none | verified |
| `CANVAS-084` | `INV` | A span's indent MUST start each line of the paragraphs it holds that far right, a wrapped line too, and keep the line within the width. | this spec section 2; `MURMURE-570` | `ambre_canvas::text_cache::tests::an_indent_starts_each_line_of_its_paragraphs_past_it` -- each wrapped line of the held paragraph starts at the indent and ends within the width, the paragraphs around it start at the edge, and CR LF counts as one break | none | verified |
| `CANVAS-067` | `INV` | A face the application has loaded MUST be held by every thread that measures text, never by the thread that fetched it alone. | this spec section 2; `ambre-spec-architecture.md section 2` | `ambre_logic::dispatch::tests::every_prose_document_takes_the_faces_whenever_it_was_opened` -- it opens one document before the delivery and one after, and reads the face count back off both | none | verified |
| `CANVAS-069` | `INV` | The context the UI thread shapes on MUST hold only the faces the application loaded into it: a face of the machine MUST NOT be discoverable from it. | this spec section 2; `PROSE-114` | `ambre_canvas::fonts::tests::a_built_context_discovers_no_face_of_the_machine` -- it asks for families a desktop carries; structural -- `font_context_from` builds on `font_context_from_bytes`, the one constructor, which starts from `ambre_draw::sealed_font_context`; `ambre_draw::text::tests::a_sealed_context_discovers_no_face_of_the_machine` -- that context, asked for the same families | none | verified |
| `CANVAS-018` | `INV` | A font MUST NOT be embedded in a binary. | this spec section 3; `ambre-spec-budgets.md section 5` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses an `include_bytes!` of a font file on any Rust line | none | verified |
| `CANVAS-051` | `INV` | A font MUST be fetched from the same origin. | this spec section 3; `UI-THREAD-004` | `ambre_canvas::fonts::tests::every_face_is_fetched_from_the_page_origin` -- every base and lazy face path, refused a scheme or a host | none | verified |
| `CANVAS-019` | `OBS` | A layout mixing text and colour emoji MUST paint both: a colour glyph through the colour atlas, a mask glyph through the alpha atlas. | this spec section 2 | `ambre_canvas::render::atlas::tests::an_emoji_rasterizes_to_colour_and_a_letter_to_a_mask` -- the raster; `ambre_canvas::render::surface::tests::a_mask_glyph_and_a_colour_glyph_land_in_their_own_batch` -- the batch each one is drawn through | none | verified |
| `CANVAS-020` | `INV` | The colour atlas MUST NOT be allocated at full size before the first colour glyph appears. | this spec section 2; `ambre-spec-budgets.md section 6` | `ambre_canvas::render::atlas::tests::the_color_atlas_is_grown_only_for_a_color_glyph` -- no request without a colour glyph, one with the first | none | verified |
| `CANVAS-087` | `INV` | A glyph whose raster size exceeds the atlas side MUST NOT reach Swash, and the atlas MUST allocate nothing for it. | this spec section 2; `ambre-spec-budgets.md section 6` | `ambre_canvas::render::atlas::tests::a_raster_past_the_atlas_side_is_refused_before_swash` -- one pixel past the side, a million, infinity and NaN give no raster, and the side itself rasterizes | none | verified |
| `CANVAS-088` | `INV` | A stored, received or pasted font size past the cap of its application MUST read as that cap, never as a decode error. | this spec section 2; `PROSE-FEAT-022` | `ambre_core::font::tests::a_size_past_the_cap_reads_as_the_cap` -- a million points decode as the ceiling of every application, and an in-range size keeps its bytes; `ambre_prose::style::tests::a_stored_size_past_the_prose_cap_reads_as_the_cap` -- a Prose mark, and a command still refused; `ambre_prose::logic::clipboard::html_read::tests::colours_and_sizes_land_canonical` -- a Prose paste; `ambre_bus::app::trame::style::tests::a_cell_size_past_the_cap_reads_as_the_cap` -- a Trame cell style; `ambre_bus::app::toile::tests::a_size_choice_past_the_cap_reads_as_the_cap` -- a Toile text size | none | verified |
| `CANVAS-007` | `INV` | Every decoded image MUST pass through one bounded cache, whichever application decoded it. | this spec section 2; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `One media cache` Story (`ambre-roadmap.md` E8). Toile holds a `MediaCache` in its own Logic module, and Murmure a shelf of its own in the view | unimplemented |
| `CANVAS-023` | `INV` | An icon MUST stay a vector buffer: it MUST NOT be rasterized into a bitmap atlas. | this spec section 2 | not built | unbuilt -- lands with the `The icon pipeline` Story (`ambre-roadmap.md` E6) | unimplemented |
| `CANVAS-024` | `INV` | An icon MUST be tessellated at build time, never on the frame path. | this spec section 2 | not built | unbuilt -- lands with the `The icon pipeline` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Fetch a script font

A codepoint the loaded faces cannot cover appears in a string being shaped. The face it needs is
fetched, and the strings that were shaped without it are dropped.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-006` | `POST` | A script-specific font MUST be fetched only when text needing it appears. | this spec section 2 | `ambre_canvas::fonts::tests::latin_and_symbols_never_trigger` -- every codepoint the base faces cover must classify to no lazy font, so a fetch armed for ordinary text fails | none | verified |
| `CANVAS-052` | `POST` | A script-specific font MUST be fetched once. | this spec section 2 | `ambre_canvas::fonts::tests::observe_latches_each_script_once` -- it reads the request count back after repeated observations of the same script and asserts it stays at one | none | verified |
| `CANVAS-053` | `FRAME` | Fetching a font MUST NOT block the frame that needed it. | this spec section 2; `ambre-spec-frame.md` | `ambre_canvas::fonts::tests::drain_is_empty_without_fetches` -- the empty case; `ambre_canvas::text_cache::tests::a_missing_script_arms_a_fetch_and_its_face_lands_later` -- the frame shapes at once, and the face lands on a later one | none | verified |
| `CANVAS-073` | `POST` | A face fetched after startup MUST be drained back for the other threads, and its delivery MUST reach every open document. | this spec section 2 | `ambre_canvas::fonts::tests::a_drained_face_comes_back_for_the_other_threads` -- the drain hands the registered bytes back; `ambre_logic::dispatch::tests::every_prose_document_takes_the_faces_whenever_it_was_opened` -- a later delivery reaches both documents | none | verified |
| `CANVAS-068` | `POST` | A face fetched after startup MUST travel to the other threads by the same path a base face travels. | this spec section 2 | `ambre_app::host::frame::tests::a_late_face_reaches_the_logic_thread_as_a_base_face_does` -- the base faces at boot and each late face leave through one function, and a face sent through it arrives as a font load with its bytes intact | none | verified |

### 1.3 Overflow an atlas or a cache

The working set outgrows its cap. Both stores answer the same way: drop the coldest entries and
keep the allocation where it is.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-021` | `POST` | The glyph atlas MUST repack by least recent use on overflow, never grow. | this spec section 2; `ambre-spec-budgets.md section 6` | `ambre_canvas::render::atlas::tests::an_overflow_repacks_the_hot_glyphs_at_the_next_frame` -- an atlas driven past its capacity keeps its size | none | verified |
| `CANVAS-022` | `POST` | The layout cache MUST evict by least recent use on overflow, never clear itself whole. | this spec section 2 | `ambre_canvas::text_cache::tests::an_overflow_evicts_the_coldest_layouts_and_keeps_the_rest` -- a cache driven past its cap | none | verified |
| `CANVAS-054` | `FRAME` | A repack or an eviction MUST NOT change what a surviving glyph or layout draws. | this spec section 2; derived | `ambre_canvas::render::atlas::tests::a_repack_moves_a_surviving_glyph_and_changes_nothing_it_draws` -- the repack; `ambre_canvas::text_cache::tests::an_eviction_changes_nothing_a_surviving_layout_draws` -- the eviction | none | verified |

* **Budget.** six rows: the memory and atlas caps of `ambre-spec-budgets.md section 6`, the
  `< 1 ms` cold-cache shape of `section 2`, and the binary rows of `section 5`. None is read by a
  check.
* **Boundary.** the font fetch is the one network path this layer opens, same-origin by
  `CANVAS-018` and the single exception `UI-THREAD-004` names to the UI thread's ban. This layer
  holds no key and reaches no relay.
* **Verification.** 20 `verified`, 3 `unimplemented`. The atlases and the layout cache are driven on
  the host past their capacity, and what survives is compared with what it drew before. The font
  fetch and its origin are read from the paths, and a source gate refuses an embedded face. The send
  of a late face runs in the browser tier, through the function the boot send calls.

---

## 2. MODEL

**CRDT containers.** none. An asset is derived from the document, never stored in it.

**The two font tiers.** One byte type, two lifetimes. There is no third: a face of the machine is
never read, on any thread (`CANVAS-069`, `PDF-007`, `PROSE-114`).

```rust
// ambre-canvas/src/fonts.rs

/// The base faces, fetched at startup from the same origin. A face that fails
/// to fetch is logged and omitted.
pub async fn fetch_base_font_faces() -> Vec<FontBytes>;

/// The lazy scripts (`ambre-draw`), so the Logic thread classifies a codepoint
/// without the graphics stack.
pub enum LazyFont { Cjk, Arabic, Devanagari, Emoji }

/// Which lazy scripts were requested, and the faces fetched since the last drain.
pub struct LazyFontLoader { /* private: the requested set and the inbox */ }

impl LazyFontLoader {
    /// Start one fetch per lazy script seen for the first time in `text`.
    pub fn observe(&mut self, text: &str);
    /// Register the arrived faces and hand their bytes back, for the other threads.
    pub fn drain_into(&mut self, font_cx: &mut parley::FontContext) -> Vec<FontBytes>;
}
```

| Tier | Faces | When it loads | Coverage |
|---|---|---|---|
| base | Noto Sans in four styles, DejaVu Serif, DejaVu Sans Mono | at startup, one `fetch` on the UI thread | Latin, Latin Extended, Cyrillic, Greek |
| lazy | CJK, Arabic, Devanagari, and Noto Color Emoji | the first time Parley meets a codepoint the base faces do not cover (`CANVAS-006`) | one face per script, fetched once |

The UI thread is the only one that can fetch. It fetches once and hands the bytes to the Logic
thread over the bus (`CANVAS-067`, `CANVAS-068`). Two `FontContext` in one linear memory
read the same blob. The Logic thread measures pages (`PROSE-115`) and exports PDF, and a face it
never received shapes into tofu.

A script face fetched later takes the same route, on the frame that registers it
(`CANVAS-068`). All bundled faces are OFL 1.1
or equally permissive (`crates/ambre-app/fonts/LICENSES.md`). Under Trunk the files sit in
`crates/ambre-app/fonts/` and are copied to `dist/fonts/`.

**Family resolution is shared** (`CANVAS-017`). Every renderer and every inline editor maps a
toolbar label through `registered_family_name` to a loaded face. A label is `Sans`, `Serif`,
`Monospace`, `Default`, a CSS generic, or an unknown string. The mapping is idempotent: a face name maps to
itself. An unmatched `Named` family handed to Parley shapes zero glyphs, which is the failure this
rule exists to make impossible.

**Shaping.** Parley delegates to HarfRust and Skrifa, both pure Rust. The Unicode bidirectional
algorithm and the UAX #14 line-breaking algorithm are Parley's, so Arabic and Hebrew need no code
of ours.

**From a string to pixels.** Four stages, and a cache between each pair.

1. `TextCache` keys a Parley `Layout` on the text, the size, the wrap width, the alignment, the
   weight, the slant, the emergency-break flag and the family. The styled spans join the key
   (`CANVAS-081`). A span's indent starts the lines of its paragraphs past it (`CANVAS-084`).
   A hit costs nothing; a miss shapes (`CANVAS-022`).
2. every string the cache shapes is offered to the lazy loader. It fetches a face the first time it
   sees a codepoint the base faces cannot cover (`CANVAS-006`).
3. Swash rasterizes each glyph, keyed on the font blob id, the glyph id and the size. A peer
   writes the size, so each application reads a size past its cap as the cap (`CANVAS-088`).
   The caps are those of the office counterparts: 1638 points for Prose, 409 for Trame and
   4000 for Toile. `FontSize` itself holds at most the highest of the three.
   A zoom can still multiply that size, so the atlas refuses a raster past its side before Swash
   allocates the mask (`CANVAS-087`).
4. the glyph lands in one of two atlases, and its slot records which (`CANVAS-019`).

| Atlas | Format | Size | Holds | Allocation |
|---|---|---|---|---|
| alpha | `R8Unorm` | 2048x2048 | every mask glyph, tinted by the vertex colour | at startup |
| colour | `Rgba8Unorm` | 2048x2048 | a `Content::Color` glyph, straight alpha, drawn untinted | lazily, on the first colour glyph (`CANVAS-020`) |

Both use a shelf packer, a one-pixel pad against bilinear bleed, and LRU compaction that re-queues
retained bitmaps rather than calling Swash again (`CANVAS-021`). Two pipelines share the
`GlyphVertex` layout and the same blending; `draw_glyphs` routes each glyph by `slot.atlas`, so
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
JPEG, WebP and GIF decoders compiled in. One bounded LRU serves every consumer: a board picture, a
sheet chart, an inline illustration. The memory budget is therefore one number rather than the sum
of three nobody set together (`CANVAS-007`). Toile is the heaviest consumer, so the **cap** may need
revisiting; the **mechanism** stays common.

**Code:** `crates/ambre-canvas/src/fonts.rs` holds the script classifier and the lazy loader, and
`crates/ambre-canvas/src/text_cache.rs` the layout cache and `registered_family_name`. The two
atlases, the shelf packer and the compaction are in `crates/ambre-canvas/src/render/atlas.rs`.
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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Fonts are fetched, never embedded | `include_bytes!` | it pays the weight twice and caches nothing. Both the UI thread and the Logic thread need the same faces, and a fetch from the same origin is deduplicated by the browser's HTTP cache: one download, two contexts. It also takes about 600 KB of raw weight out of the WASM binary, which `ambre-spec-budgets.md section 5` counts. |
| The fonts are served from the Ambre server | Google Fonts or any other CDN | a font request carries the page that made it and the scripts the reader needs. Sent to a third party, that is a profile of what a user reads, and it is sent on a document nobody else is supposed to see. |
| A script-specific face is fetched on first sight | one bundle covering everything | bundling CJK, Arabic, Devanagari and colour emoji at startup costs every user the union of every script. Fetching on the first uncovered codepoint costs each user their own. |
| A script-specific face is fetched on first sight | a per-document declaration of the scripts it uses | a paste falsifies it immediately: the codepoint arrives after the declaration was written, and the face it needs is the one nobody asked for. |
| Two atlases, not one | a single `Rgba8Unorm` atlas | it pays four times the memory so that emoji, which are rare, can share a pipeline with digits, which are not. Text dominates the working set and a mask glyph costs one byte a texel. |
| Two atlases, not one | a colour atlas allocated at startup | it spends 16 MB on documents that hold no emoji, which is most of them. |
| The emoji face is the CBDT bitmap build | the COLRv1 build | it is smaller and currently invisible: swash 0.2.7 rasterizes no COLRv1 table, so the vector build shapes and paints nothing. A bitmap emoji at atlas resolution is what the user actually sees. |
| Shaping is pure Rust | HarfBuzz through a C binding | a C binding is an unaudited dependency inside the process that holds the plaintext document, and the pillar in `README.md` refuses it. HarfRust is HarfBuzz ported by the same people. |
| The family label is resolved in one place | each renderer mapping its own labels | it is what `CANVAS-017` withdrew: an unmatched `Named` family shapes zero glyphs, so the failure is invisible text rather than an error, and each renderer gets to have the bug once. |
| Icons stay vector | a bitmap texture atlas | it blurs on a high-density screen and needs a second set at each density. A tessellated path is sharp at any density, so `ambre-spec-high-density.md` has nothing to say about icons; the tessellation is a build-time cost and the frame path pays a draw call and nothing else. |
| One image cache for the suite | a decoded-image cache per application | it is what exists today: three caches means the memory budget is the sum of three caps nobody set together, and a picture pasted from a board into a sheet is decoded twice. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **No icon exists.** There is no `assets/icons/` directory, no `Icon` type and no tessellation
  step: the one generator of `ambre-canvas`, `src/tokens.rs`, writes the design tokens and
  nothing else. The whole pipeline of section 2 is a design (`The icon pipeline`,
  `ambre-roadmap.md` E6).
* **The shared image cache does not exist.** Toile holds a `MediaCache` in its Logic thread, keyed
  by content hash, and it is a fetch deduplicator rather than a decoded-pixel cache. No application
  shares it and no budget row is read against it (`CANVAS-007`).
* **No atlas is measured.** The 4 MB alpha atlas, the 16 MB colour atlas and the under 10 MB font
  budget are figures in `ambre-spec-budgets.md section 6` that nothing reports. An overflow is
  discovered as a dropped frame.
* **The layout cache is bounded by its caller.** `evict_if_oversized` takes the cap as a parameter
  and the frame passes 5 000. The bound is a literal in the render path rather than a budget.
* **Nothing rules on a font that fails to load.** A `fetch` for a lazy face can 404 or time out,
  and no row says what the text does then.
* **System fonts are banned, not merely absent.** Every context is sealed in `ambre-draw`, so a
  native build discovers no face either. A native desktop that wanted them would lift the ban in a
  row of its own, and give up that two peers shape alike (`README.md`).

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
