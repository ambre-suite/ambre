# AMBRE -- FEATURE: HIGH-DENSITY DISPLAYS

**Type:** Horizontal (framework)
**Implementation:** implemented -- `ambre-canvas` reads the density once, and a view is handed
logical pixels alone.

> A screen with two or more physical pixels per CSS pixel. The content stays sharp on it. The
> drawing buffer, the glyphs and the paths all agree on which of the two units they are
> counting.

Getting this wrong is not subtle: the text is blurry, or half the grid is off screen. Getting it
right is one multiplication, and the whole difficulty is doing that multiplication once.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-render-assets.md` and `ambre-spec-drawing-surface.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-031` | `INV` | The display density MUST be read by one component. | this spec section 2 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, which refuses a Rust line outside `crates/ambre-canvas/src/density.rs` naming the browser's density | none | verified |
| `CANVAS-043` | `INV` | An application MUST NOT read the display density. | this spec section 2; section 4, row 4 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, which refuses a Rust line outside `crates/ambre-canvas/src/density.rs` naming the browser's density | none | verified |
| `CANVAS-032` | `INV` | A view MUST reason in logical coordinates. | this spec section 2 | type-level -- `SurfaceSize` keeps the density private to `ambre-canvas`, so a view has no factor to scale by; ci-gate -- `scripts/crate_boundaries.py` refuses a second read of the density | none | verified |
| `CANVAS-044` | `INV` | A physical size MUST NOT cross the boundary into a view. | this spec section 2 | type-level -- `SurfaceSize::physical` is crate-private, and `resize` takes the logical size | none | verified |
| `CANVAS-033` | `OBS` | A glyph MUST be rasterized at the physical resolution of the display, never at the logical one. | this spec section 2 | `ambre_canvas::render::surface::tests::a_glyph_is_rasterized_at_the_physical_size` | partial -- the test reads the raster size and the box the surface asks for. Nothing reads a rendered glyph back; lands with the `The frame and the render path held` Story (`ambre-roadmap.md` E9) | partial |

### 1.2 The display density changes

The user drags the window to a screen of another density, or zooms the page. The browser reports it
through a query that fires once and then stops matching.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-035` | `POST` | A density change MUST resize the drawing buffer, even when the CSS size did not change. | this spec section 2 | none | unheld -- the launcher arms a `matchMedia` watch on the current density, because a `ResizeObserver` does not fire when only the density moves. No test drives the path; lands with the `The frame and the render path held` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-045` | `POST` | A density change MUST leave a watch armed on the new density. | this spec section 2, the resize path; section 4, row 3 | none | unheld -- the handler re-arms a fresh query for the new density, because the query that just fired no longer matches. The first implementation watched for the existence of the feature instead and fired never, and no test would have caught it; lands with the `The frame and the render path held` Story (`ambre-roadmap.md` E9) | untested |
| `CANVAS-034` | `POST` | A density change MUST re-rasterize every glyph at the new density. | this spec section 2 | `ambre_canvas::render::surface::tests::a_density_change_asks_for_a_new_raster` | none | verified |
| `CANVAS-046` | `FRAME` | A density change MUST NOT change the logical geometry of anything a view laid out. | this spec section 2, the two units; derived | `ambre_canvas::density::tests::a_density_change_keeps_the_logical_size`; type-level -- a view is handed `SurfaceSize::logical` alone | none | verified |
| `CANVAS-071` | `POST` | A scroll shift that lands between two device pixels MUST repaint the content in full. | this spec section 2, the scroll copy | `ambre_canvas::density::tests::a_shift_between_two_device_pixels_repaints_in_full`; `::a_scroll_shift_is_copied_in_device_pixels` | none | verified |

* **Budget.** none of its own. A denser display multiplies the pixels a frame covers. The cost
  lands on the phases of `ambre-spec-budgets.md section 2` and on the texture row of
  `ambre-spec-budgets.md section 6`, charged where they are budgeted. No row here adds a figure.
* **Boundary.** none: the display density is read from the browser and never leaves the client. It
  is a fingerprinting signal the page already carries, and nothing here sends it anywhere.
* **Verification.** 7 `verified`, 1 `partial`, 2 `untested`. The two watches are driven by no test,
  and no rendered glyph is read back.
---

## 2. MODEL

**CRDT containers.** none. The display density is a property of the machine, never of the document.

**Two units, and the rule for each.**

| Unit | What it counts | Who works in it |
|---|---|---|
| logical, or CSS pixel | what a layout is written against: a 14 pt font, a 24 px row | every view, the pointer events, the chrome, the caret the `<textarea>` is placed at |
| physical, or device pixel | what the GPU fills | the drawing buffer, the offscreen pair, the glyph raster |

`devicePixelRatio` is the factor between them. It is 1 on an ordinary screen, 2 on a typical
high-density one, and a fraction on a scaled desktop.

**One reader.** `crates/ambre-canvas/src/density.rs` reads the factor and builds a `SurfaceSize`.
That value carries both sizes. Only the logical one is readable outside `ambre-canvas`.

**Where the factor applies.** Five places, all inside `ambre-canvas`.

| Surface | Rule |
|---|---|
| the drawing buffer | sized in physical pixels: `canvas.width` is the CSS width times the density |
| the projection | `QuadRenderer::begin_frame` maps the logical size onto the whole buffer |
| the glyphs | rasterized at the font size times the density, and drawn back over their logical box (`CANVAS-033`) |
| the vector paths | flattened at a tolerance divided by the density, so a curve stays smooth |
| the scroll copy | a logical shift is copied in device pixels. One that lands between two of them repaints in full (`CANVAS-071`) |

**The resize path.** Two events reach the same handler, because the browser separates them.

1. the CSS box changed -- a `ResizeObserver` on the canvas fires;
2. only the density changed. A `matchMedia` query on the current `dppx` fires, and the launcher
   arms a fresh query afterwards because the old one no longer matches (`CANVAS-045`);
3. either way the backing store is resized and `GpuState::configure` reallocates the surface and
   the offscreen pair. `AppView::resize` hears the logical size and raises a full repaint.

**Code:** `crates/ambre-canvas/src/density.rs` reads the density and sizes the backing store,
`crates/ambre-canvas/src/host.rs` configures the GPU, and `crates/ambre-app/src/host/run.rs` arms
both watches.

---

## 3. SURFACE

The user touches nothing here. What they see is whether the content is sharp.

| Trigger | Behaviour |
|---|---|
| the application opens on a high-density screen | text and vector content are sharp, at the screen's own resolution, and laid out at the same size as on any other screen |
| the browser page zoom is changed | the density changes, the buffer is resized, and the content repaints at the new resolution |
| the window is dragged to a screen of another density | the buffer is resized and every glyph is rasterized again for the new screen (`CANVAS-034`) |
| the window is resized on one screen | the buffer follows the CSS box and the view repaints in full |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The buffer is sized in physical pixels | a logical-size buffer scaled up by the compositor | it is exactly the blur this document exists to avoid. Sizing the buffer in physical pixels and carrying the factor in the projection is the one place the browser gives a choice at all, and taking it there leaves every later stage unitless: a quad, a path and a glyph are mapped by the same matrix. Any other split makes each drawing call ask which unit it was handed |
| A glyph is rasterized at the physical size | rasterizing once at the logical size and letting the transform scale it | a glyph is a bitmap in an atlas, so it is the one thing the projection cannot fix after the fact: scaling a 14 px raster to 28 px is blur, and nothing downstream recovers it |
| The density is watched with a re-armed `matchMedia` query | a `ResizeObserver` alone | it does not fire when the CSS box is unchanged, which is the common case: the window keeps its logical size and only the backing scale moves |
| The density is watched with a re-armed `matchMedia` query | one permanent resolution query | a `MediaQueryList` fires only when its own threshold is crossed, so a feature-existence query on `(resolution)` never fires at all -- which is what the first implementation did. The current density is the threshold, so each change needs a new query |
| One component reads the density | each view reading the browser for itself, which is what the code once did | four independent readers are four chances to sample it at a different moment, and a value captured at view construction is stale for the rest of the session. It is also what makes a rule statable at all: with four readers there is nothing to point a test at |
| Views lay out in logical pixels | views laying out in physical pixels, scaled by the density they read | every metric becomes two numbers, and a view that forgets one draws its content at half size on a dense screen. That is what the code did until this document's rule landed |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing rules on a hairline at a fractional density.** A scaled desktop reports values such as
  1.25 and 1.5. A one-pixel logical rule then covers a pixel and a quarter, and no row says how it
  is drawn.
* **Nothing bounds the density.** A buffer at a density of 3 covers nine times the pixels of one at
  1. No budget row of `ambre-spec-budgets.md` is written per density.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| When a frame runs, and what a resize makes it rebuild | `ambre-spec-frame.md` |
| The layout cache, the glyph atlases and what a raster costs | `ambre-spec-render-assets.md` |
| The drawing calls the projection maps | `ambre-spec-drawing-surface.md` |
| The breakpoints, and how the chrome adapts to a narrow window | `ambre-spec-adaptive-layout.md` |
| The design tokens a view reads its metrics from | `ambre-spec-design-tokens.md` |
| The frame, memory and texture budgets a denser display consumes | `ambre-spec-budgets.md` |
