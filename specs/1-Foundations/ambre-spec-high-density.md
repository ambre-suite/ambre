# AMBRE -- FEATURE: HIGH-DENSITY DISPLAYS

**Type:** Horizontal (framework)
**Implementation:** partial -- the content is sharp on a high-density screen; the scaling is done
four times over and no shared surface owns it.

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CANVAS-031` | The display density MUST be read in one place; an application MUST NOT read it again. | this spec section 2 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). Four crates read `device_pixel_ratio` today -- the launcher, Trame and Toile -- and `ambre-canvas`, the shared surface the rule names, reads it nowhere | unimplemented |
| `CANVAS-032` | A view MUST reason in logical coordinates: a physical size MUST NOT cross the driver boundary. | this spec section 2 | not built -- lands with `One high-density rule` (`ambre-roadmap.md` E8). `resize` is handed the backing-store size in physical pixels, and Trame's grid works in physical pixels and divides by the density to place its caret | unimplemented |
| `CANVAS-033` | A glyph MUST be rasterized at the physical resolution of the display, never at the logical one. | this spec section 2 | none. The density is passed to the layout cache and on to the Parley builder, so a glyph is shaped and rasterized at the physical size. Nothing reads back a rendered glyph, and blurry text is what a regression looks like | unverified |
| `CANVAS-034` | A cached layout MUST be invalidated when the display density changes. | this spec section 2 | none, and **the rule fails today**: the density is a field set once in `TextCache::new` with no setter, and it is absent from the cache key. A window dragged to a second screen keeps every layout shaped for the first one | unverified |
| `CANVAS-035` | A density change MUST resize the drawing buffer, even when the CSS size did not change. | this spec section 2 | none. The launcher arms a `matchMedia` watch on the current density and re-arms it after each change, because a `ResizeObserver` does not fire when only the density moves. No test drives the path | unverified |

* **Budget.** none of its own. A denser display multiplies the pixels a frame covers, so the cost
  lands on the phases of `ambre-spec-budgets.md section 2` and on the texture row of
  `ambre-spec-budgets.md section 6`, which are charged where they are budgeted. No row here adds a figure.
* **Boundary.** none: the display density is read from the browser and never leaves the client. It
  is a fingerprinting signal the page already carries, and nothing here sends it anywhere.
* **Verification.** 0 `verified`, 3 `unverified`, 2 `unimplemented`. Nothing holds a single row,
  and one of the three unverified rules is known to be broken. That is the honest state: the
  behaviour is correct on a static display and undefined the moment the density moves, because no
  component owns it.

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
   arms a fresh query afterwards because the old one no longer matches (`CANVAS-035`);
3. either way the backing store is resized, `FrameHost::resize` reconfigures the GPU surface and
   reallocates the offscreen pair, and `AppView::resize` raises a full repaint.

**The target, and the distance to it.** `CANVAS-031` and `CANVAS-032` say that the shared surface
scales once and hands a view logical coordinates. Today the launcher, Trame and Toile each read the
density for themselves, `resize` carries physical pixels, and Trame's grid divides by the density
again to place its caret. Every copy is correct and they are correct separately, which is what
`One high-density rule` withdraws.

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

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The buffer is sized in physical pixels and the projection carries the factor | it is the one place the browser gives a choice at all, and taking it there makes every later stage unitless: a quad, a path and a glyph are all mapped by the same matrix. Any other split makes each drawing call ask which unit it was handed | a logical-size buffer scaled up by the compositor, which is exactly the blur this document exists to avoid |
| A glyph is rasterized at the physical size | a glyph is a bitmap in an atlas, so it is the one thing the projection cannot fix after the fact: scaling a 14 px raster to 28 px is blur, and nothing downstream recovers it | rasterizing once at the logical size and letting the transform scale it |
| The density is watched with a re-armed `matchMedia` query | a `MediaQueryList` fires only when its own threshold is crossed, so a feature-existence query on `(resolution)` never fires at all -- which is what the first implementation did. The current density is the threshold, so each change needs a new query | a `ResizeObserver` alone, which does not fire when the CSS box is unchanged, and one permanent resolution query, which fires never |
| One component reads the density | four independent readers are four chances to sample it at a different moment, and a value captured at view construction is stale for the rest of the session. It is also what makes a rule statable at all: with four readers there is nothing to point a test at | each view reading the browser for itself, which is what the code does today |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The density is captured once per view.** Trame reads `device_pixel_ratio` when the view is
  built and keeps the value, so a density change after startup leaves the caret placement running
  on the old factor. The same holds for the layout cache (`CANVAS-034`).
* **`ambre-canvas` owns none of this.** The shared surface the rule names reads no density at all;
  the launcher and two applications do. Until `One high-density rule` lands there is no component
  for `CANVAS-031` to be verified against.
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
