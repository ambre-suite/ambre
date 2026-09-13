# AMBRE -- FEATURE: HIGH-DENSITY DISPLAYS

**Type:** Horizontal (framework)
**Implementation:** partial -- the content is sharp on a high-density screen; the scaling is done
in three crates and no shared surface owns it.

> A screen with two or more physical pixels per CSS pixel. The content stays sharp on it, which
> means the drawing buffer, the glyphs and the paths all agree on which of the two units they are
> counting.

Getting this wrong is not subtle: the text is blurry, or half the grid is off screen. Getting it
right is one multiplication, and the whole difficulty is doing that multiplication once.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with
`ambre-spec-frame.md`, `ambre-spec-render-assets.md` and `ambre-spec-drawing-surface.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `CANVAS-031` | `INV` | The display density MUST be read by one component. | this spec section 2 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). `ambre-canvas`, the shared surface the rule names, reads it nowhere | unimplemented |
| `CANVAS-043` | `INV` | An application MUST NOT read the display density. | this spec section 2; section 4, row 4 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). Three crates read `device_pixel_ratio` across eleven call sites -- the launcher, Trame and Toile -- and two of the three are applications | unimplemented |
| `CANVAS-032` | `INV` | A view MUST reason in logical coordinates. | this spec section 2 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). Trame's grid works in physical pixels and divides by the density to place its caret | unimplemented |
| `CANVAS-044` | `INV` | A physical size MUST NOT cross the boundary into a view. | this spec section 2 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). `resize` is handed the backing-store size in physical pixels, and the pointer path multiplies each event by the density before the view sees it | unimplemented |
| `CANVAS-033` | `OBS` | A glyph MUST be rasterized at the physical resolution of the display, never at the logical one. | this spec section 2 | none. The density is passed to the layout cache and on to the Parley builder, so a glyph is shaped and rasterized at the physical size. Nothing reads back a rendered glyph, and blurry text is what a regression looks like | unverified |

### 1.2 The display density changes

The user drags the window to a screen of another density, or zooms the page. The browser reports it
through a query that fires once and then stops matching.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `CANVAS-035` | `POST` | A density change MUST resize the drawing buffer, even when the CSS size did not change. | this spec section 2 | none. The launcher arms a `matchMedia` watch on the current density, because a `ResizeObserver` does not fire when only the density moves. No test drives the path | unverified |
| `CANVAS-045` | `POST` | A density change MUST leave a watch armed on the new density. | this spec section 2, the resize path; section 4, row 3 | none. The handler re-arms a fresh query for the new density, because the query that just fired no longer matches. The first implementation watched for the existence of the feature instead and fired never, and no test would have caught it | unverified |
| `CANVAS-034` | `POST` | A density change MUST invalidate every cached layout. | this spec section 2 | none, and **the rule fails today**: the density is a field set once when the cache is built, with no setter, and it is absent from the cache key. A window dragged to a second screen keeps every layout shaped for the first one. Answered by the `One high-density rule` Story (`ambre-roadmap.md` E8) | unverified |
| `CANVAS-046` | `FRAME` | A density change MUST NOT change the logical geometry of anything a view laid out. | this spec section 2, the two units; derived | none. A density change is a change of resolution and never of layout, and no test reads a logical size back across one. It is the rule the independent readers of `CANVAS-043` are most likely to break, each at its own moment | unverified |

* **Budget.** none of its own. A denser display multiplies the pixels a frame covers, so the cost
  lands on the phases of `ambre-spec-budgets.md section 2` and on the texture row of
  `ambre-spec-budgets.md section 6`, which are charged where they are budgeted. No row here adds a figure.
* **Boundary.** none: the display density is read from the browser and never leaves the client. It
  is a fingerprinting signal the page already carries, and nothing here sends it anywhere.
* **Verification.** 0 `verified`, 5 `unverified`, 4 `unimplemented`. Nothing holds a single row,
  and one of the five unverified rules is known to be broken. That is the honest state: the
  behaviour is correct on a static display and undefined the moment the density moves, because no
  component owns it. The pass split two conjunctions whose halves fail differently -- reading the
  density in one place is not the same defect as an application reading it, and a view working in
  physical pixels is not the same defect as a physical size crossing the boundary -- and added two
  rows to the density change. `CANVAS-045` is the re-arming, which the first implementation got
  wrong and which the document argued as a decision and ruled nowhere; `CANVAS-046` is the frame
  condition, and it is the rule the independent readers are most likely to break.

---

## 2. MODEL

**CRDT containers.** none. The display density is a property of the machine, never of the document.

**Two units, and the rule for each.**

| Unit | What it counts | Who works in it |
|---|---|---|
| logical, or CSS pixel | what a layout is written against: a 14 pt font, a 24 px row | the document model, the chrome, the caret the `<textarea>` is placed at |
| physical, or device pixel | what the GPU fills | the drawing buffer, the projection matrix, the glyph raster |

`devicePixelRatio` is the factor between them. It is 1 on an ordinary screen, 2 on a typical
high-density one, and a fraction on a scaled desktop.

**Where the factor applies.** Four places, one rule each.

| Surface | Rule | State |
|---|---|---|
| the drawing buffer | sized in physical pixels, so `canvas.width` is the CSS width times the density | built, in the launcher |
| the projection | `QuadRenderer::begin_frame(w, h)` takes the physical size and builds the normalized-device matrix from it | built |
| the glyphs | the density is passed to the layout cache and to the Parley builder, so a glyph is rasterized at its physical size (`CANVAS-033`) | built |
| the vector paths | tessellated in model space and mapped through the projection, so they are sharp at any density with nothing to configure | built |

**The resize path.** Two events reach the same handler, because the browser separates them.

1. the CSS box changed -- a `ResizeObserver` on the canvas fires;
2. only the density changed -- a `matchMedia` query on the current `dppx` fires, and the launcher
   arms a fresh query afterwards because the old one no longer matches (`CANVAS-045`);
3. either way the backing store is resized, `FrameHost::resize` reconfigures the GPU surface and
   reallocates the offscreen pair, and `AppView::resize` raises a full repaint.

**The target, and the distance to it.** `CANVAS-031` and `CANVAS-032` say that the shared surface
scales once and hands a view logical coordinates. Today the launcher, Trame and Toile each read the
density for themselves, across eleven call sites, `resize` carries physical pixels, and Trame's
grid divides by the density again to place its caret. Every copy is correct and they are correct
separately, which is what `One high-density rule` withdraws.

**Code:** `crates/ambre-app/src/host/mod.rs` sizes the backing store,
`crates/ambre-app/src/host/run.rs` arms both watches, and
`crates/ambre-canvas/src/text_cache.rs` carries the density into the shaper. `ambre-canvas` itself
reads the browser's density nowhere.

---

## 3. SURFACE

The user touches nothing here. What they see is whether the content is sharp.

| Trigger | Behaviour |
|---|---|
| the application opens on a high-density screen | text and vector content are sharp, at the screen's own resolution |
| the browser page zoom is changed | the density changes, the buffer is resized, and the content repaints at the new resolution |
| the window is dragged to a screen of another density | the buffer is resized; the text keeps the resolution of the first screen (`CANVAS-034`) |
| the window is resized on one screen | the buffer follows the CSS box and the view repaints in full |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The buffer is sized in physical pixels | a logical-size buffer scaled up by the compositor | it is exactly the blur this document exists to avoid. Sizing the buffer in physical pixels and carrying the factor in the projection is the one place the browser gives a choice at all, and taking it there leaves every later stage unitless: a quad, a path and a glyph are mapped by the same matrix. Any other split makes each drawing call ask which unit it was handed |
| A glyph is rasterized at the physical size | rasterizing once at the logical size and letting the transform scale it | a glyph is a bitmap in an atlas, so it is the one thing the projection cannot fix after the fact: scaling a 14 px raster to 28 px is blur, and nothing downstream recovers it |
| The density is watched with a re-armed `matchMedia` query | a `ResizeObserver` alone | it does not fire when the CSS box is unchanged, which is the common case: the window keeps its logical size and only the backing scale moves |
| The density is watched with a re-armed `matchMedia` query | one permanent resolution query | a `MediaQueryList` fires only when its own threshold is crossed, so a feature-existence query on `(resolution)` never fires at all -- which is what the first implementation did. The current density is the threshold, so each change needs a new query |
| One component reads the density | each view reading the browser for itself, which is what the code does today | four independent readers are four chances to sample it at a different moment, and a value captured at view construction is stale for the rest of the session. It is also what makes a rule statable at all: with four readers there is nothing to point a test at |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The density is captured once per view.** Trame reads `device_pixel_ratio` when the view is
  built and keeps the value, so a density change after startup leaves the caret placement running
  on the old factor. The same holds for the layout cache (`CANVAS-034`).
* **`ambre-canvas` owns none of this.** The shared surface the rule names reads no density at all;
  the launcher and two applications do. Until `One high-density rule` lands there is no component
  for `CANVAS-031` to be verified against.
* **The pointer path scales before the view sees it.** Each pointer event is multiplied by a density
  captured when the listener was attached, so a view is handed physical coordinates and a density
  change after startup leaves the factor stale (`CANVAS-044`).
* **Nothing rules on a fractional density.** A scaled desktop reports values such as 1.25 and 1.5,
  the backing store is rounded, and no row says what the half pixel does to a hairline or to a grid
  line.
* **Nothing bounds the density.** A buffer at a density of 3 covers nine times the pixels of one at
  1, and no budget row of `ambre-spec-budgets.md` is written per density.
* **The chrome scales by itself.** The DOM side needs none of this: CSS is authored in logical
  pixels and the browser rasterizes it. The two halves of the interface therefore reach a
  high-density screen by two entirely different routes, and no document states that they agree.

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
