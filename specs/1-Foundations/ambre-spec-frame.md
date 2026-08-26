# AMBRE -- FEATURE: THE FRAME

**Type:** Horizontal (framework)
**Implementation:** partial -- the loop, the two layers and the fatal screen ship; the frame
read-out is Trame's alone, and no accessibility tree exists.

> One frame, from the browser's tick to the pixels on the screen. Every application draws its
> content through this one loop, and none of them owns a loop of its own.

The content of a document -- a grid, a page, a board -- is drawn by our code and never by the DOM,
so the path from an event to the pixels is ours to measure. The chrome around it is
`ambre-spec-chrome.md`, the boundary between the two is `ambre-spec-ui.md`, and neither is restated
here.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-render-assets.md`, `ambre-spec-drawing-surface.md` and `ambre-spec-high-density.md`:
one numbering over the four documents the rendering layer produces.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CANVAS-001` | A GPU resource MUST NOT leave the UI thread. | this spec section 2; `ambre-spec-architecture.md section 2` | type-level: `wgpu::Device`, `HTMLCanvasElement` and the atlas textures are `!Send`, so moving one to another thread does not compile | verified |
| `CANVAS-003` | A layer planned `Reuse` MUST skip both its CPU batch build and its GPU upload. | this spec section 2 | none. `RepaintPlan` decides per layer in `render/renderer.rs`, and nothing observes the skip. A regression costs frame time and still looks correct on screen | unverified |
| `CANVAS-004` | The rAF loop MUST NOT start before GPU initialization has completed. | this spec section 2 | none. The call order in `host.rs` resolves `bootstrap_gpu` before it calls `start_raf_loop`. A reversed order makes Firefox calibrate vsync on empty frames | unverified |
| `CANVAS-005` | A frame MUST fit the 16.6 ms budget at 60 FPS on the reference device. | `ambre-spec-budgets.md section 2`, the frame phases | none. A developer reads the frame time by hand off the perf chip. No `criterion` threshold gates the budget, and nobody has taken the mobile measurement it is written against | unverified |
| `CANVAS-008` | Rendering code MUST contain no `unsafe`; the crates that need it (`wgpu`, `swash`) MUST stay encapsulated behind it. | README pillar "Security" | a count gives zero `unsafe` in `ambre-canvas`, `ambre-draw` and `ambre-pdf`. No crate carries `#![forbid(unsafe_code)]`, so the pillar holds by observation only | unverified |
| `CANVAS-009` | The canvas element MUST NOT be the end of the accessibility tree: what it draws MUST also have a presence in the page. | this spec section 3; `ambre-spec-accessibility.md` | none. A focusable input element and a hidden live region sit beside the canvas and carry the focused unit, which is the mechanism that replaced the platform tree this row used to name. One view of three writes to them, and nothing observes it | unverified |
| `CANVAS-010` | The present phase MUST run on every frame, whatever the repaint plan decides. | this spec section 2 | none. Phase 5 sits outside the conditional in `render`, and a skipped present leaves the swapchain showing whatever it held. Nothing reads back a pixel | unverified |
| `CANVAS-011` | The rAF loop MUST keep ticking while the content is static, so an inbound event is drained on the next frame. | this spec section 2 | none. The loop re-arms unconditionally, and a regression looks like a document that stops updating rather than like a crash | unverified |
| `CANVAS-012` | A paused loop MUST hold its last frame: it MUST NOT dispatch and MUST NOT render. | this spec section 2 | none. `RafControl` is an `AtomicBool` the frame callback reads before it does any work, and nothing asserts that both halves are skipped | unverified |
| `CANVAS-013` | A fatal condition MUST latch once: the first message MUST win over every later one. | this spec section 2; `ambre-spec-architecture.md section 2` | `ambre_canvas::fatal::tests::first_message_wins_and_latches` | verified |
| `CANVAS-014` | Once the process is fatal, the frame MUST paint a terminal screen instead of holding the last good frame. | this spec section 2 | none. The frame callback reads the latch and switches, and no test observes what the canvas then shows | unverified |
| `CANVAS-015` | A theme switch MUST repaint the content, and the renderer MUST hold no colour of its own. | this spec section 2; `ambre-spec-design-tokens.md` | none. Each view reads its palette from `ThemeControl` per frame, so the second colour store has nowhere to live today. Nothing refuses the field that would add one | unverified |
| `CANVAS-016` | The frame read-out MUST be available in every application, not in one. | this spec section 3 | not built -- lands with `One performance read-out` (`ambre-roadmap.md` E8). `PerfPublisher` ships and Trame is its only consumer | unimplemented |

* **Budget.** the frame phases of `ambre-spec-budgets.md section 2`, cited by `CANVAS-005`,
  and the `< 5 MB` vertex-buffer row plus the `< 20 MB` texture row of
  `ambre-spec-budgets.md section 6`. Not one of them is measured by a bench: a developer
  reads the frame time by hand off the perf chip.
* **Boundary.** none: this loop opens no socket, holds no key and writes nothing. The one network
  path the layer opens at all is the same-origin font fetch of `ambre-spec-render-assets.md`, which
  `UI-THREAD-004` names as the single exception to the UI thread's ban.
* **Verification.** 2 `verified`, 10 `unverified`, 1 `unimplemented`. The two verified rules are the
  two a host tier can reach: a type-level `!Send` and a process-global latch. Every unverified rule
  is a property of a frame that has already been presented, and no headless browser reads a pixel
  back from a wgpu canvas (`ambre-spec-testing.md section 8`), so the gap is the whole loop and it
  is one gap, not ten.

---

## 2. MODEL

**CRDT containers.** none. A frame reads the display payload the Logic thread pushed and writes
nothing back to the document.

**Two crates, and the cycle that decides the split.** The closed view registry names application
crates, so it must sit downstream of them; the GPU primitives must sit upstream of them.

| Crate | Owns | Why there |
|---|---|---|
| `ambre-canvas` | `GpuState`, `bootstrap_gpu`, `start_raf_loop`, `RafControl`, the per-frame value types `RepaintPlan`, `FrameStats`, `InputEvent`, `WgpuSurface`, and the editing core of `ambre-spec-text-editor.md` | app-agnostic, upstream of every application |
| `ambre-app` | `FrameHost`, the closed `AppView` enum, the DOM listeners, and `run_with(canvas_id, make_view)` | it names `TrameView`, so it is downstream of every application (`ambre-spec-architecture.md section 2`) |

**The driver method set.** Each variant of `AppView` exposes these as inherent methods, and
`FrameHost` matches over the enum to call them. A view names `WgpuSurface` and no other GPU type,
so an application tests its view on the host tier with no browser.

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

**The frame, in steps.** One `requestAnimationFrame` callback, no Winit and no retained scene
graph. The figures are the phase budgets of `ambre-spec-budgets.md section 2`.

1. `view.update()` -- drain `Receiver<UiEvent>` with `try_recv` until `Empty`, apply, raise the
   dirty flags. A `Disconnected` latches fatal (`CANVAS-013`).
2. the HUD gate -- the overlay is refreshed at most every 500 ms, so an idle frame rebuilds nothing.
3. `view.plan_repaint(hud_tick)` -- the plan below, plus an optional strip.
4. build the CPU vertex batch, **per layer and conditional** (`CANVAS-003`).
5. upload and encode the render pass, **always** (`CANVAS-010`).
6. present.

**The repaint plan.** Two layers, one plan each. The names are generic: no layer is named after an
application.

| Layer | Kind | Plan values | Holds |
|---|---|---|---|
| `Layer::Content` | persistent, offscreen ping-pong | `Full`, `Scroll { dx_px, dy_px }`, `Reuse` | the document itself, plus `content_strip: Option<Rect>`, the area a scroll exposed |
| `Layer::Overlay` | transient, straight to the swapchain | `Redraw`, `Reuse` | the perf read-out, the headers, the active border |

A `Scroll` shifts the previous frame with `copy_texture_to_texture` and redraws the entering strip
alone. `FrameStats` -- fps, frame, drain, draw, gpu -- is measured by the host and handed to
`paint_overlay`.

**The GPU backend, and the two browser quirks that decide it.**

| Decision | Value | Cause |
|---|---|---|
| Backend | `wgpu::Backends::GL` (WebGL2) by default, `BROWSER_WEBGPU` under `?webgpu` | a Mesa and Intel compositor bug on Firefox: the pixels are correct on readback and the compositor does not show them. The GL path goes through another compositor |
| Limits | `wgpu::Limits::downlevel_webgl2_defaults()` | it keeps the surface GL-compatible whichever backend is picked |
| Present | `PresentMode::AutoNoVsync`, `desired_maximum_frame_latency: 2` | the browser manages the cadence, which is what reaches 190 to 200 Hz on a high-refresh screen |
| Shaders | vertex and fragment only, four pipelines, no compute | the WebGL2 backend has no compute pass at all (`UI-THREAD-005`) |

Firefox on Wayland starts the rAF source at 60 Hz and switches to the display's refresh only after
the first tab change. It is a browser and compositor behaviour, observable from neither JS nor WASM.
It compounds with the empty-frame calibration that `CANVAS-004` gates the loop against.

**Measured cadence.** A release build, `opt-level = z` plus LTO. These figures are what validates
`AutoNoVsync` and the WebGL2 default above.

| Setup | Steady state | Notes |
|:---|:---|:---|
| Intel HD plus Firefox, WebGL2 (default) | 190-200 Hz, 3-6 ms per frame | scrolling included, about 24 Parley cache misses per frame at about 0.05 ms each |
| Nvidia plus Firefox, WebGPU (`?webgpu`) | about 240 Hz | dips to about 100 Hz during a scroll, then recovers |
| Chrome, WebGL2 | about 150 Hz | no 60 Hz warmup |

**The fatal latch.** A process-global `AtomicBool` plus its message, in `ambre-canvas`. The UI and
Logic threads share one linear memory, so either can set it; the Sync worker is a separate instance
and reaches it as a `UiEvent::FatalError` the view forwards. The `window.ambreFatal` DOM overlay is
the last resort for a panic on the UI thread itself, where the loop dies and the canvas can no
longer repaint.

**The theme.** The content renderer reads its colours and metrics from the generated constants of
`ambre-spec-design-tokens.md` and caches none of them. A switch flips `ThemeControl` and raises
the dirty flag; the next frame paints the new palette (`CANVAS-015`).

**Code:** `crates/ambre-canvas/src/host.rs` holds `bootstrap_gpu`, `start_raf_loop` and
`RafControl`; `crates/ambre-canvas/src/render/renderer.rs` holds the pipelines, the layer batches
and the ping-pong pair; `crates/ambre-canvas/src/view.rs` holds the per-frame value types;
`crates/ambre-canvas/src/fatal.rs` holds the latch; `crates/ambre-canvas/src/perf.rs` holds
`PerfPublisher`. The driver is `crates/ambre-app/src/host/frame.rs` and its DOM wiring is
`crates/ambre-app/src/host/run.rs`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the browser ticks | one frame runs, whether or not anything changed |
| the content changed | the content layer is rebuilt, in full or over the strip a scroll exposed |
| nothing changed | both layers are reused and the swapchain is presented again |
| the window is resized | the drawing buffer is resized, the GPU surface is reconfigured, and the view repaints in full |
| a modal opens over the canvas | the loop is paused, the canvas holds its last frame, and a blur over it costs nothing |
| the theme is switched | the content repaints at the next frame in the new palette |
| a thread dies | the canvas is replaced by a terminal screen carrying the first message latched |
| the developer opens the read-out | the frame time, the drain time and the GPU time, refreshed at most twice a second |

**The read-out is Trame's today.** `PerfPublisher` is a shared primitive and one application wires
it, so the chip and the timings panel exist in the spreadsheet alone (`CANVAS-016`).

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The content is drawn by our code, never by the DOM | the DOM pipeline is opaque -- style recalc, layout, composite -- and puts 2 to 5 frames between a mutation and the pixels. A 100 000-cell sheet or an infinite board does not hold 60 FPS there, and the path from an event to a frame stops being measurable, which is what every budget of `ambre-spec-budgets.md section 2` is written against | DOM and CSS for content, which is what the chrome uses and what the chrome is right to use |
| A quad and glyph pipeline, no vector rasterizer | the content is instanced quads plus a glyph atlas: a grid, a text page and a board need no path rasterizer on the hot path. One draw call per pipeline and zero compute holds 60 FPS with WebGL2 included | Vello, whose 8 to 12 serial compute passes cost 50 to 200 us per dispatch in a browser, whose barriers suit no mobile tile-based GPU, and whose WebGL2 fallback is a CPU rasterizer at 5 to 10 FPS. And `vello_hybrid`, which drops the compute passes and stays a generalist rasterizer we would use for none of its generality |
| One GPU pipeline, and nothing beside it | a second drawing API means a second coordinate space, a second clip model and a second set of colour rules, for the two or three shapes that reached for it. The colour wheel and the board paths are lyon meshes like everything else | Canvas 2D through `web_sys::CanvasRenderingContext2d` |
| WebGL2 by default, WebGPU behind a flag | the Mesa and Intel compositor bug shows a blank canvas on correct pixels, which is the worst failure mode available: it looks like our bug. The vertex and fragment shaders are identical on both backends, so the default costs no quality | WebGPU by default, which is faster on the machines where it works and blank on the ones where it does not |
| The browser sets the cadence | `AutoNoVsync` with a frame latency of 2 lets a 240 Hz screen run at 240 Hz. A forced vsync pins every machine to the slowest assumption we could make about it | forced vsync |
| The driver lives in the launcher, the primitives in the canvas crate | the closed `AppView` enum names `TrameView`, so it cannot sit upstream of Trame. Splitting the loop from its primitives is what keeps `ambre-canvas` free of any application dependency | one crate holding both, which is a dependency cycle, and a `Box<dyn AppView>` trait object, which trades the compiler's exhaustiveness check for nothing |
| Events come from `web_sys`, wired in the host | the DOM chrome already owns the listeners it needs, and a second event owner over the same canvas is a conflict rather than a layer | Winit on WASM, an indirection with no native target behind it to justify it |
| Animation is a lerp in the frame callback | a transition here is a value moving over a few frames, and the callback already runs every frame | a declarative keyframe runtime |
| The perf read-out is a canvas primitive | a frame read-out that exists in one application measures one application. The chip and the panel are wired the same way everywhere or they compare nothing | a per-application HUD, which is what exists today and what `CANVAS-016` withdraws |

---

## 5. NOT BUILT, AND NOT YET RULED

* **No frame is verifiable in CI.** A headless browser reads back a blank frame from a wgpu canvas,
  so nine of the thirteen rows above have no mechanism available to them at all. The Story is
  `Visual tests of canvas content` (`ambre-roadmap.md` E9), and until it lands the contract is held
  by review.
* **GPU context loss is handled and unruled.** `ambre-app` listens for `webglcontextlost`, calls
  `preventDefault` so the browser will fire the restore, and reconfigures the surface on
  `webglcontextrestored`. No row covers the path, and no test drives it.
* **The HUD is drawn by the Trame renderer.** The overlay layer is generic and its one consumer is
  not, so `FrameStats` couples the shared host to one application's painter (`CANVAS-016`).
* **Nothing bounds the vertex batch.** `ambre-spec-budgets.md section 6` budgets the CPU
  vertex buffers at under 5 MB, and no row and no check reads the number.
* **Allocation-free painting is a target, not a rule.** The steady state is meant to allocate
  nothing on the frame path, and `CANVAS P2 -- frame quality` owns it. No row states it, because
  nothing can currently observe it.
* **One view of three gives its content a presence in the page.** The mechanism is shared and the
  callers are not, so a board and a page draw content that reaches no screen reader at all
  (`CANVAS-009`, `ambre-spec-accessibility.md`).

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the canvas and the DOM each own, and the two channels between them | `ambre-spec-ui.md section 2` |
| Which subset of WebGPU and WebGL2 the content stays inside | `ambre-spec-ui.md`, `UI-THREAD-005` |
| Where the design tokens come from | `ambre-spec-design-tokens.md` |
| The menus, the dialogs, the panels and the status bar around the canvas | `ambre-spec-chrome.md` |
| The input `<textarea>` over the canvas, and the events it receives | `ambre-spec-text-input.md` |
| What a screen reader is told about what the canvas drew | `ambre-spec-accessibility.md` |
| How a `UiEvent` reaches the view, and what a thread does before it dies | `ambre-spec-architecture.md section 2` |
| The phase budgets, the memory budgets and the binary budget | `ambre-spec-budgets.md` |
| The fonts, the glyph atlases, the icons and the decoded images a frame draws | `ambre-spec-render-assets.md` |
| The primitives a view calls, and their PDF back end | `ambre-spec-drawing-surface.md` |
| The editing core a view flushes during its paint pass | `ambre-spec-text-editor.md` |
| What a high-density display changes | `ambre-spec-high-density.md` |
