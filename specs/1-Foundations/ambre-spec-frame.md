# AMBRE -- FEATURE: THE FRAME

**Type:** Horizontal (framework)
**Implementation:** partial -- the loop, the two layers and the fatal screen ship. The frame
read-out is Trame's alone, and no accessibility tree exists.

> One frame, from the browser's tick to the pixels on the screen. Every application draws its
> content through this one loop, and none of them owns a loop of its own.

The content of a document -- a grid, a page, a board -- is drawn by our code and never by the DOM.
The path from an event to the pixels is therefore ours to measure. The chrome around it is
`ambre-spec-chrome.md`, the boundary between the two is `ambre-spec-ui.md`, and neither is restated
here.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-render-assets.md`, `ambre-spec-drawing-surface.md` and `ambre-spec-high-density.md`:
one numbering over the four documents the rendering layer produces.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-001` | `INV` | A GPU resource MUST NOT leave the UI thread. | this spec section 2; `ambre-spec-architecture.md section 2` | type-level -- `wgpu::Device`, `HTMLCanvasElement` and the atlas textures are `!Send`, so moving one to another thread does not compile | none | verified |
| `CANVAS-008` | `INV` | Rendering code MUST contain no `unsafe`. | README pillar "Security" | none | unheld -- a count gives zero `unsafe` in `ambre-canvas`, `ambre-draw` and `ambre-pdf`, and no crate carries `#![forbid(unsafe_code)]`, so what holds the pillar is a reader counting rather than a build refusing; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-057` | `INV` | A crate that needs `unsafe` MUST stay encapsulated behind the rendering layer, never reached from an application. | README pillar "Security"; this spec section 2 | none | unheld -- `wgpu` and `swash` are declared by the rendering crates, and no `deny.toml` entry refuses an application crate taking either directly, unlike the cryptography and CRDT bans, which do; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-009` | `INV` | The canvas element MUST NOT be the end of the accessibility tree: what it draws MUST also have a presence in the page. | this spec section 3; `ambre-spec-accessibility.md` | none | unheld -- a focusable input element and a hidden live region sit beside the canvas and carry the focused unit, which is the mechanism that replaced the platform tree this row used to name. One view of three writes to them, and nothing observes it; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-016` | `INV` | The frame read-out MUST be available in every application, not in one. | this spec section 3 | not built | unbuilt -- lands with the `One performance read-out` Story (`ambre-roadmap.md` E8). `PerfPublisher` ships and Trame is its only consumer | unimplemented |
| `CANVAS-070` | `INV` | A frame MUST paint every canvas the active view shows, each under its own repaint plan. | this spec section 2; `ambre-spec-chrome.md section 2` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 The loop runs a frame

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-004` | `PRE` | GPU initialization MUST have completed before the rAF loop is started. | this spec section 2 | none | unheld -- the call order in `host.rs` resolves `bootstrap_gpu` before `start_raf_loop`, and a reversed order compiles and runs. What it costs is silent: Firefox calibrates vsync on the empty frames and the whole loop is paced wrong afterwards; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-005` | `POST` | A frame MUST fit the 16.6 ms budget at 60 FPS on the reference device. | `ambre-spec-budgets.md section 2`, the frame phases | none | unheld -- a developer reads the frame time by hand off the perf chip. No `criterion` threshold holds the budget, and nobody has taken the mobile measurement it is written against; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-010` | `POST` | The present phase MUST run on every frame, whatever the repaint plan decides. | this spec section 2 | none | unheld -- phase 5 sits outside the conditional in `render`, and a skipped present leaves the swapchain showing whatever it held. Nothing reads back a pixel; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-011` | `POST` | The rAF loop MUST keep ticking while the content is static, so an inbound event is drained on the next frame. | this spec section 2 | none | unheld -- the loop re-arms unconditionally, and a regression looks like a document that stops updating rather than like a crash; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-003` | `FRAME` | A layer planned `Reuse` MUST skip both its CPU batch build and its GPU upload. | this spec section 2 | none | unheld -- `RepaintPlan` decides per layer in `render/renderer.rs`, and nothing observes the skip. A regression costs frame time and still looks correct on screen, which is why it is the one frame condition here that no reviewer catches either; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 The loop is paused

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-012` | `POST` | A paused loop MUST hold its last frame on screen. | this spec section 2; `ambre-spec-chrome.md`, `CHROME-011` | none | unheld -- `RafControl` is an `AtomicBool` the frame callback reads before it does any work, and nothing reads a pixel back to see what the surface then shows; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-058` | `FRAME` | A paused loop MUST NOT dispatch and MUST NOT render. | this spec section 2 | none | unheld -- the two halves fail apart: skipping the render and still dispatching drains the inbound queue into a surface nobody repaints, so the events are consumed and their effect is never drawn. Nothing asserts that both are skipped; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 A fatal condition is signalled

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-013` | `POST` | A fatal condition MUST latch: the first message MUST be the one a reader gets. | this spec section 2; `ambre-spec-architecture.md section 2` | `ambre_canvas::fatal::tests::first_message_wins_and_latches` -- nothing is latched to begin with, and the first of two messages is what reads back | none | verified |
| `CANVAS-059` | `FRAME` | A later fatal condition MUST NOT overwrite the first. | this spec section 2; `ambre-spec-architecture.md section 2` | `ambre_canvas::fatal::tests::first_message_wins_and_latches` -- a second signal leaves the first message the one read, and a thread dying takes its neighbours with it, so the last message to arrive is the least informative one | none | verified |
| `CANVAS-014` | `POST` | Once the process is fatal, the frame MUST paint a terminal screen instead of holding the last good frame. | this spec section 2 | none | unheld -- the frame callback reads the latch and switches, and no test observes what the canvas then shows. Holding the last good frame is the failure that looks like nothing is wrong; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 The theme changes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-015` | `POST` | A theme switch MUST repaint the content. | this spec section 2; `ambre-spec-design-tokens.md` | none | unheld -- each view reads its palette from `ThemeControl` per frame, so a switch is picked up on the next tick; nothing drives a switch and reads what is drawn; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-060` | `FRAME` | The renderer MUST hold no colour of its own. | this spec section 2; `ambre-spec-design-tokens.md` | none | unheld -- the palette is read per frame and the second colour store has nowhere to live today. Nothing refuses the field that would add one, and a cached colour survives exactly one theme switch before it is wrong; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the frame phases of `ambre-spec-budgets.md section 2`, cited by `CANVAS-005`, and the
  vertex-buffer and texture rows of `ambre-spec-budgets.md section 6`. Not one is measured by a
  bench: a developer reads the frame time by hand off the perf chip.
* **Boundary.** none: this loop opens no socket, holds no key and writes nothing. The one network
  path the layer opens is the same-origin font fetch of `ambre-spec-render-assets.md`, which
  `UI-THREAD-004` names as the single exception.
* **Verification.** 3 `verified`, 13 `untested`, 2 `unimplemented`. The verified rules are the ones
  a host tier can reach: a type-level `!Send` and a process-global latch. Every untested rule is a
  property of a frame already presented, and no headless browser reads a pixel back from a wgpu
  canvas. The gap is therefore the whole loop, rather than thirteen gaps.
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
so an application tests its view on the host tier with no browser. Toile's two paint passes take
any `DrawSurface`, so its host tests paint them on a recording surface.

```rust
// inherent methods on each app view (for example ambre_trame::ui::TrameView),
// dispatched by AppView's match arms:
fn update(&mut self);                                   // drain inbound (link try_recv) -> dirty flags, no GPU
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

**Several canvases in one view.** A Murmure view shows a channel, a thread and a second pane, each
in its own canvas (`ambre-spec-chrome.md section 2`). One GPU device serves them all, and each
canvas has its own surface. The one `requestAnimationFrame` callback runs steps 3 to 6 once per
canvas, each with its own plan (`CANVAS-070`). A canvas planned `Reuse` on both layers costs one
present. Trame, Toile and Prose show one canvas, and the loop runs the steps once for them.

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
Logic threads share one linear memory, so either can set it. The Sync worker is a separate
instance, and reaches it as a `UiEvent::FatalError` the view forwards. The `window.ambreFatal` DOM
overlay is the last resort for a panic on the UI thread itself. There the loop dies, and the canvas
can no longer repaint.

**The theme.** The content renderer reads its colours and metrics from the generated constants of
`ambre-spec-design-tokens.md` and caches none of them. A switch flips `ThemeControl` and raises
the dirty flag; the next frame paints the new palette (`CANVAS-015`).

**Code:** `crates/ambre-canvas/src/host.rs` holds `bootstrap_gpu`, `start_raf_loop` and
`RafControl`. `crates/ambre-canvas/src/render/renderer.rs` holds the pipelines, the layer batches
and the ping-pong pair. `crates/ambre-canvas/src/view.rs` holds the per-frame value types;
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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The content is drawn by our code, never by the DOM | DOM and CSS for content | it is what the chrome uses and what the chrome is right to use, but the DOM pipeline is opaque -- style recalc, layout, composite -- and puts 2 to 5 frames between a mutation and the pixels. A 100 000-cell sheet or an infinite board does not hold 60 FPS there, and the path from an event to a frame stops being measurable, which is what every budget of `ambre-spec-budgets.md section 2` is written against. |
| A quad and glyph pipeline, no vector rasterizer | Vello | its 8 to 12 serial compute passes cost 50 to 200 us per dispatch in a browser, its barriers suit no mobile tile-based GPU, and its WebGL2 fallback is a CPU rasterizer at 5 to 10 FPS. The content is instanced quads plus a glyph atlas: a grid, a text page and a board need no path rasterizer on the hot path. |
| A quad and glyph pipeline, no vector rasterizer | `vello_hybrid` | it drops the compute passes and stays a generalist rasterizer we would use for none of its generality. One draw call per pipeline and zero compute holds 60 FPS with WebGL2 included. |
| One GPU pipeline, and nothing beside it | Canvas 2D through `web_sys::CanvasRenderingContext2d` | a second drawing API means a second coordinate space, a second clip model and a second set of colour rules, for the two or three shapes that reached for it. The colour wheel and the board paths are lyon meshes like everything else. |
| WebGL2 by default, WebGPU behind a flag | WebGPU by default | it is faster on the machines where it works and blank on the ones where it does not. The Mesa and Intel compositor bug shows a blank canvas on correct pixels, which is the worst failure mode available: it looks like our bug. The vertex and fragment shaders are identical on both backends, so the default costs no quality. |
| The browser sets the cadence | forced vsync | it pins every machine to the slowest assumption we could make about it. `AutoNoVsync` with a frame latency of 2 lets a 240 Hz screen run at 240 Hz. |
| The driver lives in the launcher, the primitives in the canvas crate | one crate holding both | it is a dependency cycle: the closed `AppView` enum names `TrameView`, so it cannot sit upstream of Trame. Splitting the loop from its primitives is what keeps `ambre-canvas` free of any application dependency. |
| The driver lives in the launcher, the primitives in the canvas crate | a `Box<dyn AppView>` trait object | it trades the compiler's exhaustiveness check for nothing: the set of views is closed and known at the launcher, so dynamic dispatch buys no extensibility anyone wants. |
| Events come from `web_sys`, wired in the host | Winit on WASM | it is an indirection with no native target behind it to justify it. The DOM chrome already owns the listeners it needs, and a second event owner over the same canvas is a conflict rather than a layer. |
| Animation is a lerp in the frame callback | a declarative keyframe runtime | a transition here is a value moving over a few frames, and the callback already runs every frame. |
| The perf read-out is a canvas primitive | a per-application HUD | it is what exists today and what `CANVAS-016` withdraws: a frame read-out that exists in one application measures one application. The chip and the panel are wired the same way everywhere or they compare nothing. |

**When the two vector-rasterizer rows are reopened.** Their four arguments do not age together. The
refusal is reconsidered on two conditions **met at once**, and not on the state of browser compute
support alone:

1. WebGPU is the only back end this client ships. The WebGL2 fallback -- a CPU rasterizer at 5 to
   10 FPS -- is no longer part of the answer; **and**
2. a phase of `ambre-spec-budgets.md section 2` is missed on the reference device *because of the
   cost of the geometry itself*, measured. The cause is not how much of it a frame was handed.

The second condition is the one that decides. Two of the four arguments are outside the browser's
reach. A tile-based mobile GPU is made slower by a compute barrier, whatever the runtime does. A
grid, a text page and a board are instanced quads plus a glyph atlas. A generalist path rasterizer
stays generality we would pay for and not use.

A frame missed because the cull handed it too many elements is a cull to fix. Reopening this row
would answer a question nobody asked.

Nothing here schedules the review: the Story would be written in `ambre-roadmap.md` on the day both
conditions hold.

---

## 5. NOT BUILT, AND NOT YET RULED

* **No frame is verifiable in CI.** A headless browser reads back a blank frame from a wgpu canvas.
  Nine of the thirteen rows above have no mechanism available to them at all. The Story is
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
  callers are not. A board and a page draw content that reaches no screen reader at all
  (`CANVAS-009`, `ambre-spec-accessibility.md`).

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What the canvas and the DOM each own, and the two queues between them | `ambre-spec-ui.md section 2` |
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
