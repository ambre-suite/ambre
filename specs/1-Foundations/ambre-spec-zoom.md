# AMBRE -- FEATURE: THE ZOOM MODEL

**Type:** Horizontal (framework)
**Implementation:** design-only -- each application zooms its own way; the one engine behind the
shared chip is the "One zoom model" Story.

> A user zooms with the same keys, the same wheel and the same pinch in every editor, and reads the
> factor from one chip. What zooms is the view alone: no collaborator sees it, and the document does
> not carry it.

Three editors zoom three spaces -- a grid, an endless board, a column of pages -- behind one model:
a factor, a clamp, a fixed point, one ladder of presets. This document owns the model; each
application maps it onto its own space, and `ambre-spec-chrome.md` owns the chip.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `ZOOM`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `ZOOM-001` | A zoom MUST stay local view state: it MUST emit no CRDT delta and reach no collaborator. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-002` | A zoom MUST clamp to the application's supported range, away from zero, whatever the input. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-003` | A zoom about a point MUST leave that point under the pointer, in the application's own mapping. | this spec section 2 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-004` | `Ctrl` with `=`, `-` or `0`, `Ctrl` with the wheel, and a trackpad pinch MUST zoom in every application. | this spec section 3 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-005` | `Ctrl` with `0` MUST return the factor to one, a hundred per cent. | this spec section 3 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-006` | The preset ladder MUST be one ladder for every application, read by the shared chip. | this spec section 2; `ambre-spec-chrome.md`, `CHROME-014` | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-007` | A zoom to a selection MUST frame it and clamp; an empty target MUST be ignored, never a degenerate factor. | this spec section 3 | not built -- lands with the "One zoom model" Story (`ambre-roadmap.md` E8) | unimplemented |

* **Budget.** none of its own: a zoom recomputes one view transform and repaints, which is the frame
  cost the application's viewport already carries in `ambre-spec-budgets.md`. This document adds no
  operation of its own.
* **Boundary.** none: a zoom is local view state. It writes no document, so nothing here reaches the
  relay, holds a key or opens a socket, and `ambre-spec-zero-knowledge.md` seals every payload that
  does leave the client.
* **Verification.** 0 `verified`, 0 `unverified`, 7 `unimplemented`. The one engine is unbuilt: Toile
  clamps and maps its own zoom today, Trame zooms its render without keys or pinch, and Prose has no
  zoom at all -- the three per-application states are named in section 5.

---

## 2. MODEL

**Containers.** none. The factor and its anchor are view state on the UI thread; nothing about a
zoom is persisted or shared.

**The value.** One factor, screen pixels per content unit, clamped away from zero. Each application
maps it onto its own space -- a grid cell, a world unit, a page point.

```rust
/// The shared zoom factor and the ladder of presets the chip steps through.
/// `min` and `max` are the application's own clamp; `factor` is 1.0 at 100 %.
pub struct Zoom { pub factor: f64, pub min: f64, pub max: f64 }
```

**The preset ladder.** One ordered set of stops -- 50 %, 75 %, 100 %, 125 %, and up -- shared by
every application and by the chip that lists them (`ambre-spec-chrome.md`, `CHROME-014`). An
application clamps the ladder to its own range; it does not invent a ladder of its own (`ZOOM-006`).

**A zoom, in steps.** The same four steps everywhere; only the mapping in step 3 is the
application's own.

1. a command or a gesture asks to zoom in, out, to a preset, or to a selection;
2. the factor is multiplied and clamped to the range (`ZOOM-002`);
3. the anchor is recomputed so the point under the pointer stays fixed, in the application's own
   mapping (`ZOOM-003`);
4. the view repaints and the chip reads the new factor (`ambre-spec-chrome.md`, `CHROME-014`).

**Code.** not built -- the one engine lands with the "One zoom model" Story. Until then the mapping
lives in each application's viewport (section 5).

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| `Ctrl` with `=` or `-` | zoom in or out by one preset step (`ZOOM-004`) |
| `Ctrl` with `0` | return to a hundred per cent (`ZOOM-005`) |
| `Ctrl` with the wheel, a trackpad pinch | zoom about the pointer (`ZOOM-003`, `ZOOM-004`) |
| a click on the zoom chip | the shared preset ladder opens, the current factor marked (`ambre-spec-chrome.md`) |
| a zoom-to-selection command | frame the selection and clamp; an empty target does nothing (`ZOOM-007`) |
| the zoom entries in a menu | the same intents, through a command (`ambre-spec-commands.md`) |

The user reads the factor from one chip, and the keys, the wheel and the pinch behave the same in
every editor.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One zoom engine behind the shared chip | the chip is one component over one kind of value, and two engines behind it drift in their preset ladder and their clamp, so the read-out then means a different thing per application | a zoom read-out and a preset ladder per application |
| Zoom is local view state, never a document register | a zoom is a way of looking, and a factor in the document makes two users fight over one camera. It converges nothing, because it is nobody else's business | a zoom register on the document, shared like content |
| The factor is one number, mapped per application | a grid cell, a world unit and a page point are three spaces, but the factor over them is one and the fixed-point rule is one. Sharing the number and mapping it locally is what lets the chip stay one component | a shared coordinate space every application projects into |
| Each application keeps its own clamp | a board zooms from a bird's eye to a detail and a page from a quarter to four times, so the range is a property of the space, not of the model | one clamp for every application, which either caps the board or lets a page zoom past use |
| Three per-application viewports stand beside this one | the model is one and the mapping is not: culling a grid, an endless board and a column of pages are three designs with three sets of tests, and collapsing them here would report three states as one | one viewport document for the suite |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The one engine is unbuilt.** Each application zooms in its own viewport today, so the shared
  factor, the shared ladder and the shared commands are a design bound to the "One zoom model" Story
  (`ambre-roadmap.md` E8), not code.
* **Toile zooms, without the shared model.** Toile clamps its camera and maps it
  (`3-Toile/features/ambre-spec-toile-viewport.md`), and its zoom ids resolve to no board command
  (`3-Toile/features/ambre-spec-toile-menus.md`). Nothing yet reads the shared ladder.
* **Trame zooms its render alone.** The grid scales, but no keyboard, wheel or pinch gesture reaches
  it, and there is no zoom-to-selection (`2-Trame/features/ambre-spec-trame-grid-viewport.md`).
* **Prose has no zoom.** The page column is drawn at one factor; the zoom the reader expects lands
  with this model (`4-Prose/features/ambre-spec-prose-viewport.md`).
* **Touch is not ruled.** Pinch is named here as a trigger; the touch pipeline that raises it is not
  specified in any application yet.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The status-bar chip that shows the factor, and its preset menu | `ambre-spec-chrome.md`, `CHROME-014` |
| How a grid culls and maps under zoom | `2-Trame/features/ambre-spec-trame-grid-viewport.md` |
| How an endless board culls and maps under zoom | `3-Toile/features/ambre-spec-toile-viewport.md` |
| How a page column centres and maps under zoom | `4-Prose/features/ambre-spec-prose-viewport.md` |
| Which chords mean zoom in, out and reset | `ambre-spec-commands.md` |
