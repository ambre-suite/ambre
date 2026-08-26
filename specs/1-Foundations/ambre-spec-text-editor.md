# AMBRE -- FEATURE: THE INLINE TEXT EDITOR

**Type:** Horizontal (framework)
**Implementation:** partial -- the core ships and two applications drive it; only one of the two has
a belt proving it drives it rather than reimplementing it.

> Typing inside one unit of text drawn on the canvas: a spreadsheet cell, a sticky note, a label on
> a shape. The caret, the selection, the word jumps and the composition are one implementation, and
> the application supplies only the box and what to do with the result.

The unit is formattable, not rich: one weight, one slant, one family, one size and one colour for
the whole of it (`CANVAS-042`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with the
other four documents of the rendering layer. Two rows carry `UI-THREAD` instead: this core is the
component the cross-application arbitration of `ambre-spec-ui.md section 2` names, so its two
unbuilt rules live beside the design they constrain rather than in the thread's own document.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `CANVAS-036` | An editing action MUST be recorded at input time without a font, and applied at paint time. | this spec section 2 | type-level: an `EditIntent` names no font type and no layout type, so an action that needs a live font context has no shape to be queued in | verified |
| `CANVAS-037` | Every inline editor in the suite MUST drive this one core: a wrapper MUST NOT reimplement caret motion or selection. | this spec section 2; `ambre-spec-ui.md section 2` | `ambre_trame::ui::grid::tests::editor_parity::parity_caret_motion_and_insert`, `::parity_shift_selection`, `::parity_select_replace_and_delete`, `::parity_backspace_and_delete_forward`, `::parity_selection_collapse_on_arrows` replay one operation corpus through the wrapper and through the core. Toile's wrapper has no such belt | unverified |
| `CANVAS-038` | Closing an edit that changed nothing MUST commit nothing. | this spec section 2 | none. The body at open time is kept and compared on close, and no test drives an open-and-close with no keystroke between them | unverified |
| `CANVAS-039` | A pointer position MUST reach the core in the editor's own frame, never in screen space. | this spec section 2 | none. The host subtracts the text origin before it queues a pointer intent, which is what keeps this core free of a camera. Nothing refuses a caller that passes a screen point | unverified |
| `CANVAS-040` | A composition preview MUST be replaceable in place, and MUST read as unconfirmed text. | this spec section 2; `ambre-spec-text-input.md` | `ambre_canvas::editor::tests::ime_preedit_run_is_underlined` | verified |
| `CANVAS-041` | The caret and the selection MUST be drawn as paths, so a caller's transform carries them. | this spec section 2; `ambre-spec-drawing-surface.md` | none. Both use `fill_path`, which a rotation maps, rather than `fill_rect`, which maps a bounding box alone. No test observes a rotated edit | unverified |
| `CANVAS-042` | The core MUST hold one style for the whole unit: it MUST NOT carry a per-range style. | this spec section 2 | type-level: the pinned style is one value with five fields, so a range-addressed style has nowhere to be stored | verified |
| `UI-THREAD-014` | Every editor in the suite MUST take its input pipe, its composition machine, its caret and its selection model from this one substrate. | this spec section 2; `ambre-spec-ui.md section 2` | not built -- lands with `One text-editing substrate` (`ambre-roadmap.md` E8). `CANVAS-037` holds the caret and selection half for the two wrappers that exist; the input pipe is a second crate and the third application has no crate at all | unimplemented |
| `UI-THREAD-018` | The moving end of a selection MUST be named the focus and the fixed one the anchor, in every application and in this core. | this spec section 2 | not built -- lands with `One name for the moving selection end` (`ambre-roadmap.md` E8). This core and the board already say focus; the grid says active beside its anchor, so one selection has two vocabularies and nothing refuses a third | unimplemented |

* **Budget.** the text-layout row of `ambre-spec-budgets.md section 2` -- under 2 ms for a
  dirty paragraph, typically under 0.5 ms -- which this core pays on the UI thread on every flush,
  because the editor re-shapes its unit on every edit. Nothing measures it.
* **Boundary.** none: the core holds a string in memory, reaches no relay and opens no socket. What
  the user types leaves it as a commit the application turns into a document operation, and the
  keyboard path that feeds it is `ambre-spec-text-input.md`.
* **Verification.** 3 `verified`, 4 `unverified`, 2 `unimplemented`. The three verified rules are
  the two the compiler holds and the one a host-tier test reaches. The parity belt is the
  interesting cell: it is the only mechanism in the corpus that proves a wrapper delegates rather
  than reimplements, and it exists for one of the two wrappers.

---

## 2. MODEL

**CRDT containers.** none. The core holds a plain string; the application decides which container
the commit lands in, and `2-Trame/features/ambre-spec-trame-cell-editing.md` and
`3-Toile/features/ambre-spec-toile-text-editing.md` each own that half.

**The seam, and the constraint that forces it.** Parley's `PlainEditor` owns its layout and
re-shapes on every edit, so every operation -- a move, a selection, a hit test, a render -- needs a
live font context. The frame host holds that context during the paint pass alone
(`ambre-spec-frame.md`). So the editor cannot be driven when the input arrives.

1. input time: the host maps the pointer into the editor's local frame and records a font-free
   `EditIntent` in a queue (`CANVAS-036`, `CANVAS-039`);
2. paint time: `flush` re-applies the pinned style and the geometry, then drains the queue through
   the driver;
3. still paint time: `render` draws the selection band, the glyphs and the caret;
4. any time: reading the committed buffer needs no font context, so a view emits its commit
   straight from the flush.

**The intents.** One enum, and the whole editing vocabulary is in it.

| Group | Intents |
|---|---|
| insertion | `Insert`, `Newline` |
| deletion | `Backspace`, `Delete`, `DeleteSelection` |
| caret and selection | `Left`, `Right`, `Up`, `Down`, `LineStart`, `LineEnd`, each with a `select` flag, and the horizontal pair with a `word` flag; `SelectAll` |
| pointer | `PointerDown`, `PointerDrag`, `WordAt` |
| composition | `Compose`, `ComposeEnd` |

`select` is what the Shift key means and `word` is what Control or Alt means, decided by the host
and not by the core: the chord belongs to `ambre-spec-ui.md section 2`.

**The pinned style.** Captured by the host when the edit opens and re-applied on every flush, so
the text being edited looks like the text that was there.

```rust
pub struct TextEditorStyle {
    pub weight: u16,
    pub italic: bool,
    pub family: String,
    pub base_size: f32,
    pub color: Color,
}
```

The family is resolved through the shared font resolver (`CANVAS-017`), so the in-edit body takes
the face the committed one has. `base_size` is in world units and the zoom multiplies it at each
flush, because the editor holds no camera.

**The coordinate frame.** The core lays out in a local frame whose origin is the text's top-left
corner, with the zoom baked into the font size and the wrap width. The host translates by that
origin when it renders, and may push a transform first (`CANVAS-041`).

**What the host owns.** The core is deliberately ignorant, and this table is the boundary.

| The core owns | The host owns |
|---|---|
| the buffer, the caret, the selection, the composition | which unit is being edited, and where its box is |
| the layout, and the re-shape on every edit | the style, recomputed per frame and pinned in |
| the intent queue and its flush | the keyboard chords, and what each one queues |
| the drawing of the band, the glyphs and the caret | the caret blink timer, and the selection tint |
| the caret rectangle, for an anchor | placing the transparent `<textarea>` on that rectangle |
| the committed text, and whether it changed | turning a commit into a document operation |

**Two wrappers**, Trame's cell editor and Toile's element editor. Each is a shim: a method queues
one intent and adds nothing. Trame's belt replays an operation corpus through the wrapper and the
bare core, comparing after every step, so a shim wired to the wrong intent fails (`CANVAS-037`).

**Code:** `crates/ambre-canvas/src/editor.rs` holds the core, the intents and the pinned style.
The wrappers are `crates/ambre-trame/src/ui/grid` with its rendering in
`crates/ambre-trame/src/ui/renderer/edit.rs`, and `crates/ambre-toile/src/ui/edit.rs`. The parity
belt is `crates/ambre-trame/src/ui/grid/tests/editor_parity.rs`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| an edit opens on a unit | the existing text is seeded and the caret lands at its end |
| a character is typed | it appears at the caret at the next frame, in the unit's own style |
| Enter is pressed in a multi-line unit | a hard line break, and the text wraps within the box |
| an arrow key is pressed | the caret moves one place, and any selection collapses |
| Shift and an arrow key | the selection extends by one place |
| Control or Alt and a horizontal arrow | the caret jumps by a word |
| Home or End | the caret goes to the start or the end of the visual line |
| a click inside the text | the caret lands under the pointer |
| a drag, or Shift and a click | the selection extends to the pointer |
| a double click | the word under the pointer is selected |
| an input method composes | the composing text shows underlined, and each keystroke replaces it |
| the edit closes unchanged | nothing is committed |

**The keyboard reaches this through the transparent `<textarea>`** of
`ambre-spec-text-input.md`. The host slides that element onto the caret rectangle this core
reports, so an input method opens its window in the right place.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One editing core for the suite | the caret, the word jump, the selection and the composition are the same behaviour in a cell and on a sticky note, and they are the behaviours users notice when they differ. Two implementations means two sets of edge cases and two bug reports for one complaint | an editor per application, which is what existed before the core and what the parity belt now guards against returning |
| The intent queue, rather than editing at input time | `PlainEditor` re-shapes on every operation, so it needs a font context, and the font context exists during the paint pass alone. The queue is what lets input handling stay pure and testable on the host tier, with no GPU and no browser | driving the editor from the input handler, which needs a second font context on the UI thread, or holding a live layout across frames, which is the same memory twice |
| The style is pinned, not read live | the host recomputes the style each frame anyway -- the zoom, the theme and the toolbar all move it -- so pinning at open and re-applying at flush costs one assignment and keeps the core free of every source those values come from | the core reading a theme, a zoom and a toolbar state, which is three dependencies for a struct that holds a string |
| One style for the whole unit | a cell and a sticky note carry one look, and giving the core a range-addressed style would make it a rich-text editor for two callers that have no rich text. The document that does have it is a different design entirely (`CANVAS-042`) | a per-range style model, which is `4-Prose/` and belongs there |
| The caret and the band are filled paths | Toile rotates an element, and a rotation applied to an axis-aligned fill maps its bounding box rather than the shape. A path is mapped whole, so one code path serves the rotated and the unrotated case | `fill_rect`, which is cheaper and wrong on a rotated element |
| The wrapper is proven by replay, not by reading | a shim wired to the wrong intent compiles, runs, and is wrong in one key out of twelve. Replaying an operation corpus through both editors and comparing after each step is the only mechanism that catches it, and reading the shim is not | a review of the wrapper, which is what let the mapping be wrong in the first place |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Toile's wrapper has no parity belt.** `CANVAS-037` is held for one of the two applications,
  and Toile's wrapper is the larger of the two: it carries a rotation, a card background and a
  growing box.
* **The parity corpus is single-line ASCII.** It is what a spreadsheet cell holds, so the belt
  proves nothing about a wrapped line, a word jump across a line break, or a grapheme cluster.
* **Nothing rules on undo inside an edit.** The core has no history: a wrapper that wants
  Control and Z during an edit has nothing to call, and `UI-THREAD-016` bounds the document's undo
  rather than this one.
* **The clipboard is the host's, and unruled here.** Cut and copy read `selected_text`, and what a
  paste inserts is decided by `ambre-spec-clipboard.md`. No row joins the two.
* **The shared substrate of `UI-THREAD-014` is wider than this core.** That rule names the input
  pipe, the composition state machine, the caret and the selection model for every editor,
  including Prose's, which does not use this core at all. This document is the built part of it,
  and the two are reconciled when `One text-editing substrate` lands.
* **Nothing bounds the buffer.** A unit holds a `String` with no cap, and no row says what a
  hundred-thousand-character cell does to the re-shape on every keystroke.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The key events, the composition events and the transparent `<textarea>` | `ambre-spec-text-input.md` |
| Which chord means extend, and which means jump by word | `ambre-spec-ui.md section 2` |
| What a copy and a paste carry | `ambre-spec-clipboard.md` |
| Which font face a family label resolves to | `ambre-spec-render-assets.md`, `CANVAS-017` |
| The primitives the caret, the band and the glyphs are drawn with | `ambre-spec-drawing-surface.md` |
| When the flush and the render happen | `ambre-spec-frame.md` |
| Which unit is edited, and what a commit does to the document | `2-Trame/features/ambre-spec-trame-cell-editing.md`, `3-Toile/features/ambre-spec-toile-text-editing.md` |
