# AMBRE -- SPEC: Content Rendering Pipeline (`ambre-canvas`)
**Type:** Horizontal (Foundation -- UI Thread)
**Status:** Draft
**Implementation:** partial

> GPU pipeline, frame lifecycle, GPU bootstrap + rAF driver (`bootstrap_gpu` / `start_raf_loop`), assets (icons, fonts, GlyphAtlas), `DrawSurface` abstraction, PDF export (krilla), and HiDPI handling. The frame **driver** (`FrameHost`) and the closed `AppView` registry live one layer up in `ambre-app` (see section A). This document covers **only content rendering** in the `<canvas>` (`ambre-canvas` crate), shared across all applications. The application **chrome** (menus, toolbars, dialogs, panels) is **DOM/Leptos** (`ambre-spec-chrome.md`); the canvas/DOM boundary, command registry, and design tokens are in the umbrella `ambre-spec-ui.md`. Everything executes in the **UI Thread** (page's main thread) -- no business logic, no network, no crypto in this layer.
>
> **Architecture note.** Per the two-instance model (`ambre-spec-architecture.md section 1/section 2.B`), the **Main** WASM instance hosts the **UI thread** and the **Logic thread** (a `wasm_thread` Web Worker) sharing one linear memory over `flume`; the **Sync** worker is a *separate* WASM instance over `postMessage`. Rendering lives in the UI thread. On the rendering side: (1) the layer lives in the **`ambre-canvas`** crate; (2) the displayed content arrives from the Logic thread as **typed `UiEvent` values** drained from a `flume` channel (no serialization on the UI side). See `ambre-spec-architecture.md`.

---

## 1. OBJECTIVES & CONSTRAINTS

* **Scope:**

  This document covers the **`ambre-canvas`** crate: the UI thread GPU primitives (`host.rs`: `bootstrap_gpu`/`start_raf_loop`) and the **content** rendering pipeline in the `<canvas>`. The DOM chrome (Leptos) hosting this canvas is in `ambre-spec-chrome.md`.

  - **Frame driver:** `FrameHost` (in `ambre-app`, downstream of every app crate) orchestrates a frame over the closed `AppView` enum; each view exposes the inherent driver methods (`update`/`plan_repaint`/`paint_content`/`paint_overlay`/`resize`/`handle_input`). `ambre-canvas` only provides the GPU bootstrap + rAF loop the driver consumes.
  - **Rendering pipeline:** `requestAnimationFrame` loop, CPU-side batching, GPU submission via `QuadRenderer` (vertex/fragment shaders only -- no compute), 2-layer model `Layer::{Content, Overlay}`.
  - **Asset pipeline:** SVG icons compiled build-time via lyon, font loading via fetch, GPU `GlyphAtlas`.
  - **`DrawSurface` abstraction:** unified screen/PDF trait defined in `ambre-draw`.
  - **PDF export:** `KrillaSurface` implementing `DrawSurface` via krilla (Logic thread, lazy-loaded).
  - **HiDPI handling:** `devicePixelRatio`, logical vs physical coordinates, `wgpu::Surface` resize.

  **Out of scope:** DOM chrome (Leptos), `DisplayMode`, adaptive breakpoints (`ambre-spec-chrome.md`); canvas/DOM boundary, command registry, design tokens (`ambre-spec-ui.md`); app business logic (Logic thread), crypto/sync pipeline (Sync thread), system tray, native OS menus, video codec.

* **Design principles:**
  - **Content-Canvas:** the high-frequency **content** (Trame grid, Prose text, Toile whiteboard) is drawn in the `<canvas>`; it does not go through the DOM, its event->GPU frame path is deterministic and measurable. The **chrome** is DOM/Leptos (`ambre-spec-chrome.md`). Applications use an infinite canvas or a high-performance grid.
  - **Optimistic UI:** the interface instantly reflects local edits **before** CRDT confirmation. The CRDT delta is sent to the Logic thread in parallel -- rendering does not wait for the round-trip.
  - **Mobile-First:** all performance budgets target a mid-range 2024 smartphone (see `ambre-spec-architecture.md` section 5). Adaptive design is the norm, not a late optimization.

* **Dependencies:**

  **Rendering stack:**
  | Crate | Role | Technical Details |
  |:---|:---|:---|
  | `wgpu` | GPU abstraction | WebGL2 by default (Mesa/Intel Firefox compositor bug), WebGPU opt-in via `?webgpu`. Single cross-platform backend. |
  | `lyon` | Path tessellation | Beziers/rects -> triangles. Vertex/fragment shaders only, zero compute overhead. |
  | `parley` | Text Layout | Advanced typographic layout, shaping (HarfRust/Skrifa), bidi, line breaking. UI Thread (screen rendering) + Logic thread (document pagination). |
  | `swash` | Glyph rasterization | Rasterizes Parley glyphs -> alpha bitmap -> GPU GlyphAtlas (R8Unorm, 2048x2048, LRU eviction). Color emoji (CBDT bitmap) rasterize to RGBA -> parallel color atlas (Rgba8Unorm). |
  | `peniko` | Color/brush types | `Color`, `Brush`. Shared `ambre-draw` dependency. |
  | `kurbo` | Geometric types | `Rect`, `Point`, `Affine`, `BezPath`, `Stroke`. Shared `ambre-draw` dependency. |
  | `accesskit` | Accessibility | A11y tree exposed to OS screen readers. Direct integration, no Masonry. |

  **Complementary dependencies:**
  | Crate | Role | Rationale |
  |:---|:---|:---|
  | `usvg` | SVG parsing | Build-time: SVG -> curves -> vertex buffer via lyon. |
  | `krilla` | PDF export | Implements `DrawSurface` via `KrillaSurface` (Logic thread, lazy-loaded). |
  | `flume` | Inter-thread channels (Logic <-> UI) | Typed receives. The view drains `UiEvent` from the `Receiver` each frame (`AppView::update`, non-blocking `try_recv`). |
  | `smallvec` | Inline storage for short lists (<8 items) | Avoids heap on hot path. |
  | `image` | Bitmap decoding | Pure Rust. Only activated decoders (PNG, JPEG, WebP) compiled -- feature flags. |

  > **Inbound transport note.** content to display arrives from the Logic thread as **typed `UiEvent` values** on a shared `flume` channel (common linear memory). The view drains it **without blocking** at the start of each frame (`AppView::update` -> `from_logic.try_recv()` loop until `Empty`), applies mutations, and raises its dirty flags. No serialization on the UI side: payloads are typed `AppEvent` (for example `TrameEvent::ViewportData`). The UI thread **never blocks**; on the outbound side it sends via `try_send` (drop-on-`Full`, the next viewport supersedes via `version`). See `ambre-spec-architecture.md section 2`.

  No dependency on a native toolkit (GTK, Qt, Win32). No unaudited C bindings in our code.

* **Invariants:**
  - wgpu, `QuadRenderer`, `GlyphAtlas`, and associated `web_sys` resources (`HTMLCanvasElement`, `GPUDevice`) are **confined to the UI Thread**. `!Send`, never transit to other threads.
  - **The UI thread never blocks**: inbound drain via `try_recv`, outbound send via `try_send`.
  - Conditional rendering via `RepaintPlan` (dirty flags): CPU phases (build vertex batch) and GPU phases (upload + draw) are skipped for `Reuse` layers.
  - The rAF loop starts only after GPU init complete (avoids Firefox 60 Hz vsync calibration on empty frames).
  - No `unsafe` in AMBRE code. Third-party crates with unsafe (wgpu, swash) are encapsulated.
  - Total frame budget is <= 16.6 ms at 60 FPS (see section 5 for breakdown).
  - No `fetch` or `WebSocket` calls in the rendering layer -- exception: lazy font loading from same origin.

---

## 2. TECHNICAL DESIGN & STATE

### A. Frame Driver: `FrameHost` & `AppView`

The UI runtime runs on the **page's main thread**, symmetrical to `ambre_logic::run` / `ambre_sync::run`. Responsibility is split across two crates so that the closed view registry can name app crates without creating a cycle:

- **`ambre-canvas`** -- app-agnostic GPU primitives only: `GpuState` (wgpu Device/Queue/Surface + `QuadRenderer` + `FontContext` + `TextCache`), `bootstrap_gpu` (async wgpu init), `start_raf_loop` (the `requestAnimationFrame` driver over a frame callback), and the per-frame value types (`RepaintPlan`, `FrameStats`, `InputEvent`, `WgpuSurface`). It also owns the **shared unit-text editor**: `TextEditor` (+ `TextEditorStyle`, the font-free `EditIntent` queue, `CARET_WIDTH`) -- one formattable, multi-line text unit (not rich text) over parley `PlainEditor`, driven by intents applied at the paint flush (a live `FontContext` exists only then) and rendered via `DrawSurface` (selection band + caret drawn with `fill_path`, so a caller transform such as rotation is honoured). Both Toile (sticky/text elements) and Trame (cells) drive this one core. It knows neither spreadsheet nor document.
- **`FrameHost`** (`ambre-app/src/host.rs`) -- app-aware, **downstream of every app crate**. Owns the `GpuState` it gets from `bootstrap_gpu`, measures frame timings (`FrameStats`), decides which layers to encode from the view's `RepaintPlan`, presents the swapchain, and dispatches every frame over the closed **`AppView`** enum.
- **`AppView`** (`ambre-app/src/host.rs`) -- a closed sum over document types (`AppView { Trame(TrameView), ... }`), symmetric to the `AppCommand`/`DocState` data plane (no trait object, no `Box<dyn Any>`, exhaustiveness checked by the compiler). Each variant's view exposes the **inherent** driver method set; the `FrameHost` `match`es over the enum to call them. Views are GPU-agnostic at the type level (only name `WgpuSurface`, which compiles on all targets), so apps test their view **without a browser**.

```rust
// inherent methods on each app view (for example ambre_trame::ui::TrameView),
// dispatched by AppView's match arms:
fn update(&mut self);                                   // drain inbound (flume try_recv) -> dirty flags, no GPU
fn plan_repaint(&mut self, hud_tick: bool) -> RepaintPlan;
fn paint_content(&mut self, s: &mut WgpuSurface, fc: &mut FontContext, tc: &mut TextCache, strip: Option<Rect>);
fn paint_overlay(&mut self, s: &mut WgpuSurface, fc: &mut FontContext, tc: &mut TextCache, stats: &FrameStats);
fn resize(&mut self, width: u32, height: u32);
fn handle_input(&mut self, ev: &InputEvent) -> bool;   // true => host calls preventDefault()
```

`ambre-app`'s `run_with(canvas_id, make_view)` receives a closure constructing the active `AppView`, wires DOM listeners (`keydown` -> `handle_input`, `onresize` -> `resize`), launches async `bootstrap_gpu` then `start_raf_loop`, and notifies the page via `ambreReady` once ready. Because `FrameHost` + `AppView` live in the launcher (not `ambre-canvas`), the closed enum can name `TrameView` without a dependency cycle.

> **Why the driver lives in `ambre-app`.** The closed `AppView` enum names app crates (`TrameView`, ...), so it must sit **downstream** of them; `FrameHost` travels with it. `ambre-canvas` stays upstream with only the GPU primitives + value types, depending on no app crate. See `ambre-spec-architecture.md section 6/section 7`.

### B. Rendering Pipeline & Frame Lifecycle

`FrameHost` runs a direct `requestAnimationFrame` loop (no Winit/Masonry). Each frame (`FrameHost::render`) follows this sequential pipeline:

Within the `requestAnimationFrame` callback (`raf_timestamp: f64`):

1. `view.update()` (~0.05 ms) -- drains `Receiver<UiEvent>` (`flume try_recv`), applies, raises dirty flags. A `Disconnected` becomes `FatalError` (Logic dead, see architecture section 2.B).
2. HUD refresh gate (`hud_tick`) -- overlay refreshed at most every 500 ms (avoids rebuild on idle frames).
3. `view.plan_repaint(hud_tick)` -> `RepaintPlan` -- decides `ContentRepaint { Full | Scroll { dx, dy } | Reuse }` + `Option<strip>` and `OverlayRepaint { Redraw | Reuse }`.
4. Build CPU-side vertex batch (~0.5-1 ms, CONDITIONAL: active layer) -- `quad_renderer.begin_frame(w, h)`; `WgpuSurface::new(&mut quad_renderer)`; if content active, `surface.begin_layer(Layer::Content)` then `view.paint_content(.., strip)`; if overlay redraw, `surface.begin_layer(Layer::Overlay)` then `view.paint_overlay(.., &stats)`; `text_cache.evict_if_oversized(5_000)`.
5. GPU upload + render pass (~0.3-1 ms, ALWAYS: present required) -- `surface.get_current_texture()`; `quad_renderer.flush_dirty_rect(.., plan.content, plan.overlay)`; `queue.submit(encoder.finish())`.
6. `surface_texture.present()`.

**Generic 2-layer model (`Layer`).** Rendering distinguishes two layers (generic rename, no longer says "Trame"):
- `Layer::Content` -- **persistent** layer, offscreen ping-pong, scroll-band reuse (`ContentRepaint { Full, Scroll { dx_px, dy_px }, Reuse }`; `content_strip: Option<Rect>` = area exposed by a scroll).
- `Layer::Overlay` -- **transient** layer (swapchain), where the perf HUD, headers, and active border live (`OverlayRepaint { Redraw, Reuse }`).

**Conditional rendering.** Phase 4 is skipped for any `Reuse` layer; phase 5 (present) always runs to avoid an empty canvas. rAF stays active to drain inbound messages even when the UI is static. `FrameStats` (fps/frame/drain/draw/gpu) measured by the host are passed to `paint_overlay` for the HUD.

> The debug HUD currently lives in the Trame renderer (minor coupling via `FrameStats`) -- future candidate to become a generic `FrameHost` overlay.
>
> **Target state -- shared perf telemetry (X69).** The `PerfPublisher` (FPS chip + timings panel) is
> a shared canvas primitive wired uniformly in every app, not surfaced by Trame alone: Prose and
> Toile join, available at least in debug. Tracked in the cross-app factoring Story
> (`ambre-roadmap.md`).

**HiDPI is a surface service (X5).** `device_pixel_ratio` scaling (a Retina / 4K screen has 2+
physical pixels per CSS pixel) is applied **once** by the shared `ambre-canvas` surface -- the
drawing buffer is sized in physical pixels and the projection scales by the ratio -- so every app
(Trame, Prose, Toile) renders crisply without repeating the concern. App UI specs reference this
service rather than each declaring their own `device_pixel_ratio` handling.

**Double buffering:** `present_mode: PresentMode::AutoNoVsync`, `desired_maximum_frame_latency: 2`. No forced vsync -- lets the browser manage cadence to reach 190-200 Hz on high-refresh screens.

**Default GPU backend -- WebGL2:**
wgpu uses the GL backend (WebGL2) by default on WASM. WebGPU is opt-in via `?webgpu` in the URL. Reason: Mesa/Intel Firefox compositor bug -- GPU pixels are correct (confirmed by readback) but the browser compositor does not display them on-screen. The GL backend bypasses this bug as it goes through a different compositor path.

`bootstrap_gpu()` (`ambre-canvas/src/host.rs`) reads the `?webgpu` URL flag and
picks `wgpu::Backends::BROWSER_WEBGPU` when it is set, `wgpu::Backends::GL`
(WebGL2, the default) otherwise; it requests `wgpu::Limits::downlevel_webgl2_defaults()`
so the surface stays GL-compatible.

Vertex/fragment shaders (`QuadRenderer`) are identical regardless of backend -- no quality degradation in GL.

---

### C. Asset Pipeline

#### C.1 SVG Icons -> Vertex Buffers (lyon)

AMBRE icons are provided as SVG source (individual files in `assets/icons/`). They are tessellated into vertex buffers at build time via a build script (`build.rs`):

```rust
/// Index into the compiled icon atlas. Each icon is a pre-tessellated vertex buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon(pub u16);
```

**Compilation pipeline (build-time):**

1. `build.rs` scans `assets/icons/*.svg`
2. Each SVG is parsed via `usvg::Tree::from_str()`
3. usvg curves are converted to `lyon::path::Path`
4. Lyon tessellates paths into `VertexBuffers<Vertex, u16>`
5. Buffers are serialized and concatenated into a binary blob included via `include_bytes!`
6. The `Icon(u16)` index maps to an offset in the blob

**Runtime:** the blob is loaded at startup. Each `Icon(u16)` resolves to a vertex buffer submitted as a draw call via `QuadRenderer`'s `paths` pipeline. Icons stay vectorial (no bitmap texture) and adapt to DPI without blur.

**Budget:** full atlas < 200 KB uncompressed for ~500 icons. Load < 5 ms.

#### C.2 Fonts & Typography (Parley)

Font loading and management go through Parley (via its HarfRust/Skrifa backend):

**Loading strategy:**

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

- **Base fonts (required):** three families back the formatting toolbar's family choices -- **Noto Sans** (sans, regular/bold/italic/bold-italic), **DejaVu Serif** (serif), and **DejaVu Sans Mono** (monospace) -- served as static assets from the Ambre server (same origin -- no privacy leak). The UI thread and the Logic thread do an independent `fetch()` at startup -- the browser's HTTP cache deduplicates: single physical download, two `FontContext` in shared memory. Coverage: Latin/Latin-Extended, Cyrillic, Greek. **Family resolution is shared:** every app's text renderers **and** inline editors map a toolbar label (`Sans`/`Serif`/`Monospace`/`Default`, plus any unknown or CSS-generic name) to one of these loaded faces through `ambre_canvas::registered_family_name`, so a label resolves to the same face across apps and no family is ever passed to Parley as an unmatched `Named` (which would shape zero glyphs). **Local dev (Trunk):** TTF files are in `crates/ambre-app/fonts/` and copied to `dist/fonts/` via `<link data-trunk rel="copy-dir" href="fonts" />`.
- **Additional fonts (lazy):** for CJK, Arabic, Devanagari, color emoji and other scripts, fonts are loaded on demand via `fetch()` from the static server. Loading triggers when Parley encounters a codepoint absent from base fonts (fallback chain). Color emoji uses **Noto Color Emoji** (CBDT bitmap build); all bundled faces are OFL 1.1 / permissive (see `crates/ambre-app/fonts/LICENSES.md`).
- **System fonts (native desktop -- post-MVP):** Parley can query system fonts via `fontdb`. Not available on WASM.

> **`include_bytes!` ruled out:** Fonts are not embedded in the WASM binary via `include_bytes!`. This lightens it by ~600 KB raw and enables deduplication via the browser's HTTP cache. Both the UI thread and the Logic thread need the fonts -- `fetch()` from the Ambre server (same origin) creates no privacy leak.

> **Logic thread + Parley note:** The Logic thread uses Parley for **document pagination** (widows/orphans, keep-with-next on titles, image positioning, manual page breaks, footnotes). Fonts are therefore necessary in the Logic thread to compute typographic metrics. Pagination must be identical between screen display and PDF export.

**Shaping & Bidi:**
- Parley delegates shaping to HarfRust/Skrifa (pure Rust -- HarfBuzz port by Google Fonts, not a C binding).
- Bidirectional support (Arabic, Hebrew) is handled by the Unicode Bidi algorithm built into Parley.
- Line breaking follows the Unicode UAX #14 algorithm via Parley.

**Interaction with rendering:**
- Parley produces `Layout<ParleyBrush>` containing positioned `GlyphRun`s (glyphs + coordinates).
- Layouts are cached in `TextCache` (key = text + font_size + max_width + alignment). Cache-miss -> Parley shape; cache-hit -> zero-cost.
- Swash rasterizes each glyph (glyph_id + size) into an R8 alpha bitmap, uploaded to `GlyphAtlas` (wgpu::Texture R8Unorm 2048x2048, LRU eviction). The glyph pipeline draws glyphs as textured quads.
- **Color glyphs (color emoji).** `GlyphAtlas` also owns a parallel **RGBA color atlas** (`Rgba8Unorm` 2048x2048, its own shelf packer + LRU compaction). `get_or_rasterize` branches on Swash's `image.content`: `Content::Color` (straight-alpha RGBA) packs into the color atlas with `AtlasSlot::is_color = true`; `Content::Mask` stays on the R8 path. A second pipeline (`color_glyph.wgsl`, shared `GlyphVertex` layout, same `ALPHA_BLENDING`) samples the RGBA atlas and outputs the texel directly, ignoring the vertex tint. `draw_glyphs` routes each glyph by `slot.is_color`, so a single layout mixing text and emoji paints digits (tinted mask) and emoji (color) correctly. Keeping two atlases (rather than one RGBA atlas for all glyphs) avoids paying 4x texture memory for text, which dominates the working set. The emoji face is **Noto Color Emoji, CBDT embedded-bitmap build** (COLRv1 is not rasterized by swash 0.2.7); it is lazy-loaded (`LazyFont::Emoji`), never in the startup bundle. This is shared infrastructure: color emoji works for every app through `draw_glyphs` (Toile reaction badges, Murmure message reactions, inline emoji) with no per-app work.
- `DrawSurface::draw_glyphs(&run, &brush, transform)` is the unified screen/PDF interface.

---

### D. `DrawSurface` Abstraction & PDF Export (krilla)

`krilla` generates high-fidelity PDF from vector primitives. To let applications render indifferently to screen or PDF, an abstraction trait `DrawSurface` is defined in the **`ambre-draw`** crate (source of truth, see `ambre-draw/src/lib.rs`):

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

> **`ParleyBrush = peniko::Brush`**: unlike `peniko::Color` (= `AlphaColor<Srgb>`), `peniko::Brush` implements `Default` (required by `parley::Brush`). Callers pass the render color explicitly as `&Brush` to `draw_glyphs`.

> **`fill_path` / `stroke_path`**: implemented on `WgpuSurface` via lyon and on `KrillaSurface` via krilla primitives. On `WgpuSurface` the `BezPath` is converted to a `lyon::path::Path` and tessellated in model space (Fill = NonZero rule, Stroke maps `kurbo::Stroke` width/cap/join/miter-limit); the flattening tolerance is expressed in screen pixels and divided by the transform's scale (sqrt of the linear-part determinant) so zoomed paths stay smooth. Output vertices are mapped to screen space through the active transform and appended to the solid-color quad batch (`QuadRenderer::push_mesh`) -- no dedicated pipeline.
>
> **Known limitations (first implementation):**
> - **Solid brush only.** In `fill_path` / `stroke_path` only `Brush::Solid` is honored; gradient brushes are ignored (Phase 3), matching `draw_glyphs`. Raster images are painted through the dedicated `draw_image` method, not through a brush.
> - **Clip is bounding-box reject, not per-triangle trim.** A path whose (transformed, stroke-padded) bbox misses the active clip is skipped entirely; a path straddling a clip edge is drawn whole. Arbitrary path geometry cannot be trimmed cheaply on the CPU the way axis-aligned rects/lines/glyphs are. True clipping (GPU scissor for axis-aligned clips, or stencil) is deferred until a consumer needs a partially-clipped path.
> - **Stroke width scales with zoom.** Tessellation is in model units, so the transform scales the stroke -- correct for Toile pan/zoom; a hairline (screen-constant width) would need dividing the width by the transform scale.

> **`draw_image`**: paints a decoded raster image into the world-space `dest` rect (the element AABB after the active transform). `ImageSource<'_>` is a borrowed view over already-decoded RGBA8 pixels plus their native `width`/`height` -- decoding is the caller's job (Toile interop decodes on import; the screen renderer decodes from the media cache), so `ambre-draw` stays codec-free (`kurbo`/`peniko`/`parley` only). The image is scaled to fill `dest`; aspect ratio is the caller's responsibility (Toile stores the element rect at the imported aspect). Implementations: `WgpuSurface` uploads the pixels to a `wgpu::Texture` and draws a textured quad through the active transform; `KrillaSurface` hands the pixels to krilla as a raster image (the `ambre-pdf` crate enables krilla's `raster-images` feature). A surface without raster support keeps the default no-op, so text/vector export still succeeds. Per section 5.B, a Toile image whose pixels have not yet arrived from the media cache paints a placeholder rect via `fill_rect` and is repainted once `draw_image` has pixels; the trait itself carries no placeholder logic. **Delivery note:** `KrillaSurface::draw_image` (the PDF export consumer) ships first; the `WgpuSurface` texture path lands with the screen media cache (the `MediaDownloaded` -> decoded-image store that populates it), since the on-screen source does not exist until then -- until it does, `WgpuSurface` keeps the trait default and image elements paint the section 5.B placeholder on screen.
>
> **`ImageSource`**: a lightweight borrow, `struct ImageSource<'a> { pixels: &'a [u8], width: u32, height: u32 }` (RGBA8, row-major, no stride padding), defined in `ambre-draw` alongside the trait. It never owns or decodes pixels.

> **`push_transform` / `pop_transform`**: default no-ops, required for Toile (infinite pan/zoom) and Prose (page margins). `WgpuSurface` will implement them with a CPU-side transform matrix before GPU submission.

Concrete implementations live in consumer crates, not in `ambre-draw`:

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

> **Architecture note:** `KrillaSurface` executes in the **Logic thread** (not the UI Thread). The `krilla`, `docx-rs`, `html5ever`, `pulldown-cmark` crates are **lazy-loaded** (WASM bundle splitting), not in the startup bundle.

> **Crate placement (`ambre-pdf`).** `KrillaSurface` lives in its own **`ambre-pdf`** crate (deps: `ambre-draw` + `krilla`, `default-features = false`), not inside an app crate: PDF export is app-agnostic (it only translates `DrawSurface` primitives to krilla calls), so every app renders through the same surface. The drawing API is on `krilla::surface::Surface` (obtained via `page.surface()`), so the wrapper borrows the `Surface`, not the `Page`. krilla's surface is top-left / Y-down like `DrawSurface` -- no coordinate flip. The clip stack and transform stack both map onto krilla's single push/pop instruction stack (`push_clip`/`push_transform` -> a push, `pop_*` -> `Surface::pop`); `DrawSurface`'s balanced-nesting contract keeps them paired. Text runs currently carry no `ToUnicode` mapping (glyph shapes render, but PDF text is not yet selectable) -- threading the cluster source text is a later refinement.

#### Peer color -- `ambre_draw::peer::peer_color` (canonical, cross-app)

Every application paints remote collaborators (cursors, selections, presence badges)
in a **stable per-user color**. To guarantee that the same collaborator gets the same
color in Prose, Trame, and Toile, the color function is defined **once** in `ambre-draw`
and every app calls it -- no app reimplements the hash.

```rust
// ambre-draw/src/peer.rs
use ambre_core::UserId;
use peniko::Color;

/// Deterministic per-user color, identical across all apps in the suite.
/// Seeded on the **canonical `UserId`** (spoof-proof, resolved by `ambre-sync`,
/// see `ambre-spec-sync.md` "Presence identity") -- never on the per-device
/// `pub_key`, which changes on key rotation. The id bytes are folded into a hue
/// with FNV-1a and mapped through HSL at a fixed saturation/lightness, so every
/// peer color is vivid, legible on light and dark backgrounds, and distinct from
/// its neighbours; the result is always fully opaque (callers apply per-element
/// alpha themselves).
pub fn peer_color(user_id: &UserId) -> Color { /* ... */ }
```

> **Dependency note.** Seeding on the canonical `UserId` (not raw `Uuid`) keeps the
> newtype end-to-end through every presence path, so `ambre-draw` takes a light
> dependency on **`ambre-core`** (types only -- no GPU, no crypto). This is the sole
> exception to the "`kurbo`/`peniko`/`parley` only" rule (see `ambre-spec-architecture.md`).
> Consumers: `prose` (`prose-ui` remote cursors), `trame` (`color_for_user`),
> `toile` (peer presence) -- all call `ambre_draw::peer::peer_color`, none reimplement it.

---

### E. High DPI / Scaling

> `DisplayMode`, adaptive breakpoints, and the per-mode adaptation table are defined in `ambre-spec-chrome.md section 2.E`. This section only covers HiDPI wiring on the rendering side.

On `window.onresize`, the host calls `AppView::resize` with the logical width (`canvas.client_width()`, browser dp); the view adapts its content layout. (`DisplayMode` on the **chrome** side is CSS-driven -- `ambre-spec-chrome.md section 2.E`.)

DPI is handled via the browser's `devicePixelRatio`:

- **Vector paths (lyon):** rects and strokes adapt to physical DPI without blur.
- **Glyphs (Swash atlas):** `scale` is passed in `TextCache::new(scale)` and in `LayoutContext::ranged_builder(font_cx, text, scale, ...)`. Glyphs are rasterized at physical resolution -- cache-key includes `size * scale`.
- **Logical vs physical metrics:** the view reasons in logical coordinates (f64). `QuadRenderer::begin_frame(w, h)` receives the canvas physical dimensions and builds the NDC matrix.
- **Resize:** `window.onresize` -> `canvas.set_width/height(physical)` -> `FrameHost::resize(w, h)` (reconfigures `wgpu::Surface` + reallocates offscreens) -> `AppView::resize` (the view raises a full repaint).

Canvas-side constants needed for content rendering -- `scale_factor` (`devicePixelRatio`), logical width, sans/mono font families -- provided by **generated tokens** (`theme.rs`, see section F and `ambre-spec-ui.md section 2.D`).

---

### F. Content Theme (design tokens)

> **Design tokens** (single source `tokens.ron` -> `theme.css` for DOM chrome + `theme.rs` constants for canvas) are defined in `ambre-spec-ui.md section 2.D`. This section only documents their consumption on the content rendering side.

The content renderer reads its themed colors and metrics from **generated constants** (`theme.rs`) -- no color cache in the view or `QuadRenderer`. The dark/light switch (triggered on the UI side) re-injects the current constant set and sets `dirty = true`: content repaints at the next frame with the new values.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

The rendering layer consists of pure drawing primitives. ZK audit:

- **No network outbound flows** in this layer. Displayed content is provided by the Logic thread as **typed `UiEvent` values** drained from a `flume` channel (shared memory) -- no network, no deserialization.
- **No direct persistence**: layout states (ratios, expansions) are sent up to the Logic thread (which decides on persistence) only on explicit request.
- **Fonts:** additional fonts loaded on demand via `fetch()` come from the AMBRE static server (same origin) -- no request to third-party CDNs (Google Fonts and others).
- **Invariant:** zero `fetch`/WebSocket calls in this layer (exception: lazy font loading from same origin). Zero exfiltration surface.

---

## 4. QUALITY & TESTABILITY

### A. Rendering Pipeline Tests

- **Dirty flag + conditional rendering:** test that `dirty=false` skips the build vertex batch phase, by counting draw calls on a `RecordingSurface`.
- **GPU backend:** test GL vs WebGPU selection via mock `url_flag("webgpu")`.
- **`DrawSurface` contract:** the app render layer is tested against `RecordingSurface` (the mock `DrawSurface` in `ambre-draw`, feature `testing`) -- assert the ordered `DrawOp` sequence (geometry, colours, clip balance, glyph-run counts) on host, no GPU. The wgpu pipeline itself (`QuadRenderer`/`WgpuSurface`/`GlyphAtlas` tessellation, shaders, atlas packing) needs a real `wgpu::Device` and is **not unit-tested**: thin, stable plumbing validated visually (`/run`). `KrillaSurface` (PDF) is verified by simple-primitive snapshot.
### B. Asset Tests

- **Icon pipeline:** verify that the compiled blob covers all SVGs in `assets/icons/`, vertex count > 0 per icon.
- **`TextCache`:** test cache-hit (zero Parley call on second identical request) and LRU eviction under limit.
- **`GlyphAtlas`:** test LRU eviction -- fill beyond capacity, verify oldest glyphs evicted.
- **Font fallback:** test that Parley triggers lazy loading on codepoint absent from base fonts.

### C. Mocks & Environment

- Headless tests via `RecordingSurface` (mock `DrawSurface`, `ambre-draw` feature `testing`) -- no wgpu required.
- `web_sys` mockable via `cfg(test)` feature flag.

### D. Benchmarks

- `TextCache` shape: < 1 ms for 100 distinct layouts (cold cache).
- Icon atlas load: < 5 ms at startup for 500 icons.
- Full frame (dirty=true, typical scene): < 3 ms CPU.

---

## 5. PERFORMANCE & RESOURCES

### A. Frame Budget (16.6 ms @ 60 FPS)

| Phase | Budget | Notes |
|:---|:---|:---|
| `AppView::update` (drain flume `UiEvent`) | < 0.1 ms | Inbound IPC, dirty flag mutations |
| Content hit-test + input dispatch (in view) | < 0.2 ms | `AppView::handle_input` (pointer/keyboard on canvas area) |
| View state update | < 0.5 ms | Internal state mutations, transition logic |
| Text layout (Parley, dirty paragraph) | < 2 ms | UI-thread shaping for the in-cell editor case (Trame cells, multi-line). **Prose does NOT shape on the UI thread** -- pagination shapes in the Logic thread and ships pre-shaped `ParleyRunData`, so this row is ~0 for Prose (`ambre-spec-prose-logic.md section 2.B`, `ambre-spec-prose-ui.md section B`). Typical < 0.5 ms |
| Content accessibility diff (AccessKit) | < 0.2 ms | OS notifications on delta only |
| Build vertex batch (dirty only) | < 1 ms | `QuadRenderer::begin_frame` + draw scene. Zero allocation in steady state |
| GPU upload + render pass | < 1 ms | `write_buffer` + up to 3 draw calls per layer (quads, mask glyphs, color glyphs). Typical 0.3-0.5 ms |
| **Total target** | **< 5 ms typical** | 11 ms margin for complex cases |

> **Architecture alignment:** the P1 scroll round-trip (UI -> Logic -> UI) is budgeted at < 15 ms in `ambre-spec-architecture.md` section 5. UI rendering consumes < 5 ms; `flume` transport (move in shared memory) is negligible (< 1 us/hop, no serialization or boundary memcpy), leaving most of the budget for Logic thread processing.

### B. WASM Impact

| Component | Impact | Notes |
|:---|:---|:---|
| `usvg` + `lyon` (build-time icons) | ~80 KB | Icon pipeline, precompiled vertex buffers |
| `swash` | ~120 KB | Glyph rasterization |
| Fonts (Noto Sans + Mono) | 0 KB binary | ~600 KB fetch (HTTP cache), Regular + Bold, Latin/Cyrillic/Greek |
| Icon atlas blob (vertex buffers) | < 200 KB | ~500 tessellated icons |
| `image` (PNG/JPEG/WebP) | ~60 KB | Feature-flagged, only activated decoders |

> **Note:** the heavy render crates live in the **Main** instance (UI + Logic, shared memory); wgpu/lyon/swash are linked once there and deduplicated with the rest (the separate Sync instance links none of them). See `ambre-spec-architecture.md` section 5 for the global binary budget < 2 MB gzip.

### C. UI Thread Memory Budget

| Component | Budget | Notes |
|:---|:---|:---|
| CPU vertex buffers (QuadRenderer) | < 5 MB | Proportional to visible draw calls count |
| wgpu textures + buffers | < 20 MB | Swap chain, staging buffers, GlyphAtlas (2048x2048 R8 = 4 MB). The RGBA color atlas (2048x2048 = 16 MB) is **allocated lazily** on the first color glyph -- a 1x1 placeholder otherwise -- so emoji-free documents pay nothing. |
| View state (content) | < 5 MB | Proportional to visible content (cells, text runs) |
| Fonts in memory | < 10 MB | Base fonts (fetch) + lazy-loaded fonts |
| Icon atlas | < 1 MB | Compiled vector blob |
| **Total UI Thread** | **< 41 MB** | Within the global budget < 100 MB (see `ambre-spec-architecture.md` section 5) |

### D. Measured Throughput and Driver Notes

Empirical figures (release build, `opt-level = z` + LTO) validating the `AutoNoVsync` cadence of section 4:

| Setup | Steady-state | Notes |
|:---|:---|:---|
| Intel HD + Firefox, WebGL2 (default) | 190-200 Hz, 3-6 ms/frame | Scroll included (~24 Parley cache-misses/frame, ~0.05 ms/miss release) |
| Nvidia + Firefox, WebGPU (`?webgpu`) | ~240 Hz | Dips to ~100 Hz during scroll, recovers after |
| Chrome, WebGL2 | ~150 Hz | No 60 Hz warmup |

> **Firefox/Wayland vsync calibration.** Firefox on Wayland starts the rAF source at 60 Hz and only switches to the display's native refresh after the first tab change. This is a browser/compositor behaviour, not observable or fixable from JS/WASM. It compounds with the empty-frame calibration issue that already gates the rAF start until after GPU init (see section 1 / section 2): both are Firefox vsync-source quirks, distinct from the Mesa/Intel compositor bug that drives the WebGL2-by-default decision (section 4).

---

## 6. REJECTED ARCHITECTURAL DECISIONS

> This section covers rejections specific to the **content rendering layer**. **UI model** rejections (100% canvas UI vs DOM, Xilem/Masonry, winit) are in `ambre-spec-ui.md section 6`; **chrome**-specific rejections (egui, ribbon, `<dialog>`, CSS Anchor) in `ambre-spec-chrome.md section 6`.

* **Vello:** Rejected for content rendering. Vello rasterizes via 8-12 serial compute shader passes; ~50-200 us per `dispatch_workgroups()` in browser WebGPU (sandboxed validation, IPC to GPU process, bindgen indirection), barriers incompatible with mobile TBDR, WebGL2 fallback = CPU rasterizer (`vello_cpu`, ~5-10 FPS without WASM threads/SIMD). The `QuadRenderer` graphics pipeline (vertex/fragment, 1 draw call, 0 compute) holds 60+ FPS, WebGL2 included.

* **Vello hybrid (`vello_hybrid`):** Rejected. Even without compute shaders (runs on WebGL2), it remains a generalist 2D vector rasterization engine that is superfluous: Ambre's content (grid, text, whiteboard) renders as instanced quads + glyph atlas, without a path rasterizer on the hot path.

* **DOM/CSS for *content* rendering:** Rejected. Opaque pipeline (style recalc, layout, composite), 2-5 frame latency between JS mutation and GPU frame; a 100k-cell spreadsheet or infinite whiteboard does not reach 60 FPS there. Content is drawn by our code (`QuadRenderer`, `GlyphAtlas`, lyon), event->frame path deterministic. (The **chrome**, on the other hand, is DOM/Leptos -- `ambre-spec-chrome.md`.)

* **Canvas 2D (`web_sys::CanvasRenderingContext2d`):** Rejected. Would exit the unified GPU pipeline. The HSL color picker wheel and Toile paths are lyon/wgpu meshes.

* **Winit (on WASM):** Not used. Events are captured via `web_sys` (`addEventListener` keydown/pointer, `ResizeObserver`) wired in the `host`. Indirection without benefit + event ownership conflict with DOM chrome. (Web-only: no native target.)

* **CSS-like animations (declarative keyframes):** Rejected in favor of manual lerp in the rAF callback. Animation runtime unwarranted for simple transitions.

* **Google Fonts CDN / third-party fonts:** Rejected. User data exfiltration. Fonts served from the Ambre server (same origin).

* **`include_bytes!` for fonts:** Rejected. Both the UI Thread and the Logic thread need the fonts; loading them via `fetch()` (same origin, HTTP cache) lightens the binary by ~600 KB and enables browser deduplication.

* **HarfBuzz (C binding) for shaping:** Rejected. Parley uses HarfRust (pure Rust) + Skrifa. No C binding.

* **Bitmap texture atlas for icons:** Rejected. Lyon icons are vectorial -- adapt to DPI without blur.

---

## 7. INTEGRATION PLAN (ROADMAP)

> Detailed backlog of this spec; cross-cutting scheduling (milestones) lives in `specs/ambre-roadmap.md`.

### CANVAS P0 -- Rendering Foundation

- [x] GPU bootstrap (`ambre-canvas/src/host.rs`): `wgpu::Instance`, `wgpu::Adapter`, WebGL2 by default, WebGPU opt-in `?webgpu`
- [x] `FrameHost` + closed `AppView` enum (in `ambre-app`) + `run_with(canvas_id, make_view)` (view injection by closure); `ambre-canvas` exposes `bootstrap_gpu`/`start_raf_loop`. (Was `FrameHost<V: DocView>` + `DocView` trait in `ambre-canvas` -- switched in ARCH P2.)
- [x] Direct `requestAnimationFrame` loop (no Winit/Masonry), `RepaintPlan` (dirty flags), rAF after GPU init, `ambreReady` hook
- [x] 2-layer model `Layer::{Content, Overlay}` (`ContentRepaint`/`OverlayRepaint`, scroll-strip reuse)
- [x] `QuadRenderer`: quads pipeline (fill_rect/stroke_rect/stroke_line) + glyphs pipeline (GlyphAtlas R8Unorm)
- [x] `WgpuSurface` implementing `DrawSurface` (CPU-side clip)
- [x] `GlyphAtlas`: Swash rasterization -> wgpu::Texture, ShelfPacker, LRU eviction
- [x] `TextCache`: cached Parley layouts (key text+size+width+align), FxHashMap, eviction by count
- [x] Load base fonts (NotoSans-Regular.ttf) via `fetch()` + Parley `FontContext`
- [x] `DrawSurface` trait in `ambre-draw`: `ParleyBrush = peniko::Brush`, fill_path/stroke_path/push_transform/pop_transform default no-ops
- [ ] Icon type + `build.rs` pipeline: SVG -> usvg -> lyon -> vertex buffer blob
- [x] Inbound drain: `AppView::update` drains `Receiver<UiEvent>` (`flume try_recv`) -> mutations -> dirty flags; `Disconnected`/`FatalError` -> fatal state
- [x] Generated canvas tokens (`scale_factor`, sans/mono font families, content colors) from `tokens.ron` -> `theme.rs` (`build.rs` codegen), consumed by the Trame renderer (`Palette`) and `TextCache` -- see `ambre-spec-ui.md section 2.D`
- [x] Headless tests via mock `DrawSurface` (`ambre_draw::testing::RecordingSurface` + `FixedClock`,
  behind the `testing` feature): drive the real Trame grid renderer off-main-thread and assert on the
  recorded `DrawOp` sequence (palette colours, grid/header geometry, clip balance). The renderer's
  `web_sys::Performance` timing dependency is abstracted behind the `ambre_draw::Clock` trait.

### CANVAS P1 -- Integration

- [x] `KrillaSurface`: `DrawSurface` implementation for PDF export (Logic thread, lazy-loaded). New **`ambre-pdf`** crate (krilla backend); geometry / glyph / path / clip / transform mapped to krilla, headless PDF-serialization tests. *(ToUnicode text mapping deferred -- glyphs render, text not yet selectable.)*
- [x] `push_transform` / `pop_transform` on `WgpuSurface` (Prose margins, Toile pan/zoom)
- [x] `fill_path` / `stroke_path` on `WgpuSurface` via lyon. `BezPath` -> lyon `Path`, tessellated in model space (Fill/Stroke tessellators, NonZero fill rule, tolerance scaled by the transform's determinant so zoomed paths stay smooth), vertices mapped to screen space and pushed to the solid-color quad batch (`push_mesh`). Solid brush only (gradients Phase 3); clip is bbox-reject, not per-triangle trim (Toile, Trame charts)
- [x] Lazy loading CJK/Arabic/Devanagari fonts (trigger on missing codepoint). `LazyFontLoader` (`ambre-canvas/src/fonts.rs`): `TextCache` scans each shaped string on a cache miss (`observe`); the first codepoint outside the base fonts (classified into `Cjk`/`Arabic`/`Devanagari` by Unicode block) kicks off a same-origin `fetch()` (`spawn_local`, at most once per script). Fetched faces land in a shared inbox drained per-frame by `FrameHost` (`poll_lazy_fonts`): registered into the shared `FontContext`, layout cache cleared, full repaint forced so tofu boxes re-shape with real glyphs. *(Assets under `crates/ambre-app/fonts/`: `NotoSansSC-Regular.otf` (CJK), `NotoSansArabic-Regular.ttf`, `NotoSansDevanagari-Regular.ttf`; a missing file logs a warning and leaves tofu.)*
- [x] `ambreFatal` **terminal screen** (panic -> canvas terminal render) wired in `host`/`ambre-app`. *(The `panic::set_hook` JS hook itself is done -- see `ambre-spec-architecture.md section 7 ARCH P1`; only the canvas-side terminal screen remains. Roadmap Epic `E2`.)*

### CANVAS P2 -- Optimization & Polish

- [ ] Frame budget profiling: layout/paint/GPU per-phase breakdown
- [ ] Elimination of remaining allocations on the paint path (steady state)
- [ ] Full AccessKit audit on content (view)
- [ ] Dynamic dark/light theme: canvas token re-injection + global invalidation (repaint dirty)
- [ ] `GlyphAtlas`: atlas size tuning and LRU policy based on measured memory profiles
- [ ] Internationalization of static labels, RTL Toolbar/Breadcrumb support
- [ ] Non-regression benchmarks: frame budget, TextCache cold cache, icon atlas load
- [ ] **Path clipping on `WgpuSurface`** (deferred from CANVAS P1). Today `fill_path`/`stroke_path` only bbox-reject against the active clip: a path straddling a clip edge is drawn whole. Implement real clipping when a consumer needs a partially-clipped path -- GPU scissor for axis-aligned clips (the common Toile/Prose case), stencil for the general case. Also expose a hairline (screen-constant stroke width) option, since path strokes currently scale with the transform zoom.
- [ ] **DEBT -- headless GPU rendering for visual tests.** Headless Chromium
      (SwiftShader/ANGLE) does not composite our offscreen ping-pong Content
      layer: the canvas reads back blank/white, so screenshot-driven verification
      of any canvas content (grid, and the `ambreFatal` terminal screen) is
      impossible in CI/agent environments. The render path is provable only by
      tracing/logs there (confirmed for the fatal screen). Investigate a real-GPU
      runner, a software path that survives the offscreen blit, or a `DrawOp`
      golden-image harness off the `RecordingSurface` as the visual oracle.
