# AMBRE -- FEATURE: THE INLINE TEXT EDITOR

**Type:** Horizontal (framework)
**Implementation:** partial -- the core ships and two applications drive it. Only one of the two has
a belt proving it drives it rather than reimplementing it.

> Typing inside one unit of text drawn on the canvas: a spreadsheet cell, a sticky note, a label on
> a shape. The caret, the selection, the word jumps and the composition are one implementation. The
> application supplies only the box and what to do with the result.

The unit is formattable, not rich: one pinned style for the whole of it. A host may pin spans over
it, each a range in a look of its own. The core keeps none of its own (`CANVAS-042`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CANVAS`, shared with the
other four documents of the rendering layer. Two rows carry `UI-THREAD` instead: this core is the
component the cross-application arbitration of `ambre-spec-ui.md section 2` names. Its two unbuilt
rules live beside the design they constrain, rather than in the thread's own document.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-042` | `INV` | The core MUST hold one style for the whole unit, and only the spans its host pinned: an edit MUST NOT move, add or drop a span. | this spec section 2 | `ambre_canvas::editor::tests::an_edit_moves_no_span` -- two backspaces leave the pinned spans as they were, and a range the edit left past the buffer renders with no panic | none | verified |
| `CANVAS-078` | `INV` | With spans pinned, the caret, the selection, a click and a line move MUST read where the spans draw the text. | this spec section 2 | `ambre_canvas::editor::tests::a_click_lands_on_the_char_a_span_draws_under_it` -- a bold span widens twenty chars: the caret, End and a click land at the wide x, and with no span the plain layout serves again | none | verified |
| `CANVAS-079` | `OBS` | A span MUST draw its range in its own weight, slant, family, ink, underline and tint. | this spec section 2 | `ambre_canvas::editor::tests::a_span_draws_its_range_in_its_own_look` -- each of weight, slant and family shapes a second face, and a recorded render finds the span ink, the underline and the tint | none | verified |
| `CANVAS-080` | `INV` | With spans pinned, a run of line moves MUST keep its column through a shorter line. | this spec section 2 | `ambre_canvas::editor::tests::a_line_move_over_spans_keeps_its_column` -- two moves up through a one-char line land at the column they left, in one flush or across two, and a move left first aims at a new column | none | verified |
| `CANVAS-082` | `INV` | The layout the core hands out MUST be the one it draws: the spans' own while it holds spans, the plain one otherwise. | this spec section 2 | `ambre_canvas::editor::tests::the_layout_handed_out_is_the_one_drawn` -- none before the first flush, wider under a bold span, and plain again with no span | none | verified |
| `CANVAS-048` | `INV` | A composition preview MUST be replaceable in place. | this spec section 2; `ambre-spec-text-input.md` | `ambre_canvas::editor::tests::ime_preedit_run_is_underlined` -- after the composition ends, its second assertion reads the buffer back as the composed run alone, so a preview appended beside its own commit fails | none | verified |
| `CANVAS-040` | `OBS` | A composition preview MUST read as unconfirmed text. | this spec section 2; `ambre-spec-text-input.md` | `ambre_canvas::editor::tests::ime_preedit_run_is_underlined` -- its first assertion finds the underline on the composing run, and its last asserts the committed run has lost it | none | verified |
| `CANVAS-041` | `INV` | The caret and the selection MUST be drawn as paths, so a caller's transform carries them. | this spec section 2; `ambre-spec-drawing-surface.md` | `ambre_canvas::editor::tests::the_caret_and_selection_are_drawn_as_paths` -- it renders a selection and a caret under a pushed rotation and finds both as path fills and no rect fill, which a rotation maps by its bounding box alone | none | verified |
| `CANVAS-037` | `INV` | Every inline editor in the suite MUST drive this one core: a wrapper MUST NOT reimplement caret motion or selection. | this spec section 2; `ambre-spec-ui.md section 2` | `ambre_trame::ui::grid::tests::editor_parity::parity_caret_motion_and_insert`; `::parity_shift_selection`; `::parity_select_replace_and_delete`; `::parity_backspace_and_delete_forward`; `::parity_selection_collapse_on_arrows` -- the five replay one operation corpus through the wrapper and through the core; `ambre_toile::ui::view::view_tests::the_board_editor_drives_the_shared_core` -- Toile's belt replays its keys through the board view and the intents through a bare core, and compares the text and the selection after every step | none | verified |
| `UI-THREAD-014` | `INV` | Every editor in the suite MUST take its input pipe, its composition machine, its caret and its selection model from this one substrate. | this spec section 2; `ambre-spec-ui.md section 2` | not built | unbuilt -- lands with the `One text-editing substrate` Story (`ambre-roadmap.md` E8). `CANVAS-037` holds the caret and selection half for the two wrappers that exist; the input pipe is a second crate and the third application has no crate at all | unimplemented |
| `UI-THREAD-018` | `INV` | The moving end of a selection MUST be named the focus and the fixed one the anchor, in every application and in this core. | this spec section 2 | not built | unbuilt -- lands with the `One name for the moving selection end` Story (`ambre-roadmap.md` E8). This core and the board already say focus; the grid says active beside its anchor, so one selection has two vocabularies and nothing refuses a third | unimplemented |

### 1.2 Record an editing action

A key, a pointer or a composition event arrives when no font context is available. The host records
what was asked for and stops there.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-036` | `POST` | An editing action MUST be recorded without a font. | this spec section 2 | type-level -- an `EditIntent` names no font type and no layout type, so an action that needs a live font context has no shape to be queued in | none | verified |
| `CANVAS-039` | `PRE` | A pointer position MUST reach the core in the editor's own frame, never in screen space. | this spec section 2 | `ambre_toile::ui::view::view_tests::a_press_reaches_the_editor_in_its_own_frame` -- a press on the first character of a sticky placed far from the origin puts the caret there, so a screen point would land past the end; `ambre_trame::ui::view::view_tests::a_press_reaches_the_editor_in_its_own_frame` -- the same press on a cell away from the corner of the grid | none | verified |
| `CANVAS-049` | `FRAME` | Recording an editing action MUST NOT change the buffer. | this spec section 2, the seam; derived | `ambre_canvas::editor::tests::recording_an_action_leaves_the_buffer_alone` -- three actions recorded, and the buffer and the selection read back unchanged before the flush | none | verified |

### 1.3 Flush the queue at paint time

The paint pass has the font context. It re-applies the pinned style and the geometry, then drains
everything recorded since the last frame.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-047` | `POST` | Every recorded action MUST be applied at paint time. | this spec section 2 | `ambre_canvas::editor::tests::every_recorded_action_applies_at_the_flush` -- two inserts with motion between them, and one flush shows each one at its place | none | verified |
| `CANVAS-050` | `FRAME` | A flush MUST NOT leave a recorded action in the queue. | this spec section 2, the seam; derived | `ambre_canvas::editor::tests::a_flush_leaves_the_queue_empty` -- it reads the queue empty after a flush, and a second flush with nothing recorded leaves the buffer as it was | none | verified |
| `CANVAS-077` | `POST` | A recorded replace MUST swap the exact byte range it names, leave the caret after its text, and change nothing when the range is off a char boundary. | this spec section 2 | `ambre_canvas::editor::tests::a_replace_swaps_its_range_and_puts_the_caret_after_it` -- a query becomes a name with the caret after it, and a range inside a char or past the end leaves the buffer | none | verified |

### 1.4 Close an edit

The host compares the body it captured when the edit opened against the one the core holds.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CANVAS-038` | `FAIL` | Closing an edit that changed nothing MUST commit nothing. | this spec section 2 | `ambre_canvas::editor::tests::an_unchanged_buffer_commits_nothing` -- a caret that only moved yields no commit; `ambre_toile::ui::view::view_tests::an_untouched_editor_closes_without_a_write` -- an editor opened and closed with no keystroke sends no command | none | verified |

* **Budget.** the text-layout row of `ambre-spec-budgets.md section 2`: under 2 ms for a dirty
  paragraph. This core pays it on the UI thread on every flush, because the editor re-shapes its
  unit on every edit. Nothing measures it.
* **Boundary.** none: the core holds a string in memory, reaches no relay and opens no socket. What
  the user types leaves it as a commit the application turns into a document operation, and the
  keyboard path that feeds it is `ambre-spec-text-input.md`.
* **Verification.** 16 `verified`, 2 `unimplemented`. Host-tier tests hold the queue, the drain at
  paint time, the drawn paths and the no-op close. A parity belt per wrapper proves each one
  delegates to the core. What is loose is the two rules nothing has built yet.

---

## 2. MODEL

**CRDT containers.** none. The core holds a plain string; the application decides which container
the commit lands in, and `2-Trame/features/ambre-spec-trame-cell-editing.md` and
`3-Toile/features/ambre-spec-toile-text-editing.md` each own that half.

**The seam, and the constraint that forces it.** Parley's `PlainEditor` owns its layout and
re-shapes on every edit. Every operation -- a move, a selection, a hit test, a render -- needs a
live font context. The frame host holds that context during the paint pass alone
(`ambre-spec-frame.md`). So the editor cannot be driven when the input arrives.

1. input time: the host maps the pointer into the editor's local frame and records a font-free
   `EditIntent` in a queue (`CANVAS-036`, `CANVAS-039`, `CANVAS-049`);
2. paint time: `flush` re-applies the pinned style and the geometry, then drains the queue through
   the driver (`CANVAS-047`, `CANVAS-050`);
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
and not by the core. The chord belongs to `ambre-spec-ui.md section 2`.

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

**The spans.** A host may pin spans over the pinned style, each a byte range with its own weight,
slant, family, ink, underline and tint. The host pins them again after each flush that changed the
text, so the core neither moves nor keeps one (`CANVAS-042`). A range off a char boundary draws
nothing.

Parley's `PlainEditor` lays its text out with one style. So the spans get a second layout, built
from the same text at the same wrap width (`CANVAS-079`). Every geometry is read from that layout:
the caret, the band, the height, and the outcome of a click or a line move (`CANVAS-078`). With no
span, the core takes the one-layout path it had before. A line move keeps its column (`CANVAS-080`).
The core hands out the layout it draws, so a host can draw beside the text (`CANVAS-082`).

**The coordinate frame.** The core lays out in a local frame whose origin is the text's top-left
corner. The zoom is baked into the font size and the wrap width. The host translates by that
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
one intent and adds nothing. Each application's belt replays an operation corpus through its
wrapper and through the bare core, comparing after every step. A shim wired to the wrong intent
therefore fails (`CANVAS-037`).

**Code:** `crates/ambre-canvas/src/editor.rs` holds the core, the intents and the pinned style.
The wrappers are `crates/ambre-trame/src/ui/grid` with its rendering in
`crates/ambre-trame/src/ui/renderer/edit.rs`, and `crates/ambre-toile/src/ui/edit.rs`. The parity
belts are `crates/ambre-trame/src/ui/grid/tests/editor_parity.rs` and
`crates/ambre-toile/src/ui/view_tests.rs`.

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

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One editing core for the suite | an editor per application | it is what existed before the core and what the parity belt now guards against returning. The caret, the word jump, the selection and the composition are the same behaviour in a cell and on a sticky note, and they are the behaviours users notice when they differ: two implementations means two sets of edge cases and two bug reports for one complaint |
| The intent queue, rather than editing at input time | driving the editor from the input handler | it needs a second font context on the UI thread: `PlainEditor` re-shapes on every operation, and the font context exists during the paint pass alone. The queue is what lets input handling stay pure and testable on the host tier, with no GPU and no browser |
| The intent queue, rather than editing at input time | holding a live layout across frames | it is the same memory twice, and it keeps a shaped layout alive outside the pass that can shape it |
| The style is pinned, not read live | the core reading a theme, a zoom and a toolbar state | it is three dependencies for a struct that holds a string. The host recomputes the style each frame anyway -- the zoom, the theme and the toolbar all move it -- so pinning at open and re-applying at flush costs one assignment |
| The host pins the spans | a per-range style model the core keeps up to date | it makes the core a rich-text editor. A cell and a sticky note carry one look, and the composer already keeps its marks beside the draft, so a second copy in the core would drift from it. The document that does have rich text is a different design entirely (`CANVAS-042`), and it is `4-Prose/` |
| A second layout for the spans | patching `PlainEditor`'s own layout | parley 0.7 pushes only its default styles when it re-shapes, and offers no hook for ranged ones. The second layout costs one more shape per flush with spans, and none without (`CANVAS-078`) |
| The caret and the band are filled paths | `fill_rect` | it is cheaper and wrong on a rotated element: Toile rotates, and a rotation applied to an axis-aligned fill maps its bounding box rather than the shape. A path is mapped whole, so one code path serves the rotated and the unrotated case |
| The wrapper is proven by replay, not by reading | a review of the wrapper | it is what let the mapping be wrong in the first place. A shim wired to the wrong intent compiles, runs, and is wrong in one key out of twelve; replaying an operation corpus through both editors and comparing after each step is the only mechanism that catches it |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The parity corpus is single-line ASCII.** It is what a spreadsheet cell holds. The belt proves
  nothing about a wrapped line, a word jump across a line break, or a grapheme cluster.
* **Nothing rules on undo inside an edit.** The core has no history, so a wrapper that wants
  Control and Z during an edit has nothing to call. `UI-THREAD-016` bounds the document's undo
  rather than this one.
* **The clipboard is the host's, and unruled here.** Cut and copy read `selected_text`, and what a
  paste inserts is decided by `ambre-spec-clipboard.md`. No row joins the two.
* **The shared substrate of `UI-THREAD-014` is wider than this core.** That rule names the input
  pipe, the composition state machine, the caret and the selection model for every editor. That
  includes Prose's, which does not use this core at all. This document is the built part of it,
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
