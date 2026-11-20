# AMBRE -- FEATURE: THE ZOOM MODEL

**Type:** Horizontal (framework)

> A user zooms with the same keys, the same wheel and the same pinch in every editor, and reads the
> factor from one chip. What zooms is the view alone: no collaborator sees it, and the document does
> not carry it.

Three editors zoom three spaces: a grid, an endless board, a column of pages. One model carries
them all -- a factor, a clamp, a fixed point, one ladder of presets. This document owns the model; each
application maps it onto its own space, and `ambre-spec-chrome.md` owns the chip.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `ZOOM`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-004` | `INV` | `Ctrl` with `=`, `-` or `0`, `Ctrl` with the wheel, and a trackpad pinch MUST zoom in every application. | this spec section 3 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-006` | `INV` | The preset ladder MUST be one ladder for every application, read by the shared chip. | this spec section 2; `ambre-spec-chrome.md`, `CHROME-014` | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |

### 1.2 Zoom the view

The user presses a chord, turns the wheel under `Ctrl`, or pinches. One factor moves, about one
point.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-002` | `POST` | A zoom MUST clamp to the application's supported range, away from zero, whatever the input. | this spec section 2 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-003` | `POST` | A zoom about a point MUST leave that point under the pointer, in the application's own mapping. | this spec section 2 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-005` | `POST` | `Ctrl` with `0` MUST return the factor to one, a hundred per cent. | this spec section 3 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-001` | `FRAME` | A zoom MUST emit no CRDT delta. | this spec section 2 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-008` | `FRAME` | A zoom MUST reach no collaborator. | this spec section 2; `ambre-spec-presence.md` | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8). It is not the same rule as emitting no delta: a presence sample is broadcast and written to no document, so a factor riding one would reach every peer while `ZOOM-001` still held | unimplemented |

### 1.3 Zoom to a selection

The user asks for the selection to fill the view. The target may be empty.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `ZOOM-007` | `POST` | A zoom to a selection MUST frame it and clamp. | this spec section 3 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8) | unimplemented |
| `ZOOM-010` | `FRAME` | A zoom to a selection MUST NOT change the selection. | this spec section 3; derived | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8). The gesture reads a selection to compute a factor, and an application that grew or collapsed it while framing would make the command unrepeatable | unimplemented |
| `ZOOM-009` | `FAIL` | A zoom to an empty target MUST be ignored, never applied as a degenerate factor. | this spec section 3 | not built | unbuilt -- lands with the `One zoom model` Story (`ambre-roadmap.md` E8). It is the half `ZOOM-007` carried, and the one with a failure mode of its own: framing nothing is a division by a zero-sized box, which clamps to the range bound rather than to anything the user asked for | unimplemented |

* **Budget.** none of its own: a zoom recomputes one view transform and repaints, which is the frame
  cost the application's viewport already carries in `ambre-spec-budgets.md`. This document adds no
  operation of its own.
* **Boundary.** none: a zoom is local view state. It writes no document, so nothing here reaches the
  relay, holds a key or opens a socket. `ambre-spec-zero-knowledge.md` seals every payload that does
  leave the client.
* **Verification.** 0 `verified`, 10 `unimplemented`. The one engine is unbuilt. Toile
  clamps and maps its own zoom today, Trame zooms its render without keys or pinch, and Prose has no
  zoom at all. Section 5 names the three per-application states.

  Writing the rules before the code is what makes the three added ones cheap. `ZOOM-008`, because
  reaching no collaborator is not the same rule as emitting no delta once presence exists.
  `ZOOM-009`, because an empty target has a failure of its own. `ZOOM-010`, the frame condition that
  keeps the gesture repeatable.

---

## 2. MODEL

**Containers.** none. The factor and its anchor are view state on the UI thread; nothing about a
zoom is persisted or shared.

**The value.** One factor, screen pixels per content unit, clamped away from zero. Each application
maps it onto its own space -- a grid cell, a world unit, a page point.

```rust
/// The shared zoom factor and the ladder of presets the chip steps through.
/// `min` and `max` are the application's own clamp; `factor` is 1.0 at 100 %.
/// `S` is the space the application zooms: `Doc` for a grid or a page,
/// `Board` for a board.
pub struct Zoom<S: ScalesTo<View>> { pub factor: Scale<S, View>, pub min: Scale<S, View>, pub max: Scale<S, View> }
```

**The preset ladder.** One ordered set of stops: 50 %, 75 %, 100 %, 125 %, and up. Every
application shares it, and so does the chip that lists them (`ambre-spec-chrome.md`,
`CHROME-014`). An
application clamps the ladder to its own range; it does not invent a ladder of its own (`ZOOM-006`).

**A zoom, in steps.** The same four steps everywhere; only the mapping in step 3 is the
application's own.

1. a command or a gesture asks to zoom in, out, to a preset, or to a selection;
2. the factor is multiplied and clamped to the range (`ZOOM-002`);
3. the anchor is recomputed so the point under the pointer stays fixed, in the application's own
   mapping (`ZOOM-003`);
4. the view repaints and the chip reads the new factor (`ambre-spec-chrome.md`, `CHROME-014`).

**Code.** not built -- the one engine lands with the `One zoom model` Story (`ambre-roadmap.md` E8).
Until then the mapping lives in each application's viewport (section 5).

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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One zoom engine behind the shared chip | a zoom read-out and a preset ladder per application | two engines behind one chip drift in their preset ladder and their clamp, so the read-out means a different thing per application while looking the same |
| Zoom is local view state, never a document register | a zoom register on the document, shared like content | a factor in the document makes two users fight over one camera, and it converges nothing: a zoom is a way of looking, and it is nobody else's business |
| The factor is one number, mapped per application | a shared coordinate space every application projects into | a grid cell, a world unit and a page point are three spaces, and forcing one space on them buys a projection nobody needs: the factor over them is already one and the fixed-point rule is one. Sharing the number and mapping it locally is what lets the chip stay one component |
| Each application keeps its own clamp | one clamp for every application | it either caps the board or lets a page zoom past use: a board zooms from a bird's eye to a detail and a page from a quarter to four times, so the range is a property of the space, not of the model |
| Three per-application viewports stand beside this one | one viewport document for the suite | the model is one and the mapping is not: culling a grid, an endless board and a column of pages are three designs with three sets of tests, and collapsing them here would report three states as one |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The one engine is unbuilt.** Each application zooms in its own viewport today. The shared
  factor, the shared ladder and the shared commands are a design bound to the
  `One zoom model` Story (`ambre-roadmap.md` E8), not code.
* **Toile zooms, without the shared model.** Toile clamps its camera and maps it
  (`3-Toile/features/ambre-spec-toile-viewport.md`), and its zoom ids resolve to no board command
  (`3-Toile/features/ambre-spec-toile-menus.md`). Nothing yet reads the shared ladder.
* **Trame zooms its render alone.** The grid scales, but no keyboard, wheel or pinch gesture reaches
  it, and there is no zoom-to-selection (`2-Trame/features/ambre-spec-trame-grid-viewport.md`).
* **Prose zooms alone too.** The page zooms on its own ladder, through the chip and the keys, and
  keeps the top of the canvas in place. The pinch and the anchor on the caret wait for this
  model (`4-Prose/features/ambre-spec-prose-viewport.md`).
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
