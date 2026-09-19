# AMBRE -- FEATURE: THE TEXT INPUT PIPE

**Type:** Horizontal (framework)
**Implementation:** partial -- the pipe, the composition window and the clipboard events ship. No
test drives any of them, and one label is written for a spreadsheet.

> A text area that shows nothing, sits under the caret, and is the real keyboard focus of the
> canvas. Every keystroke, every input-method composition and every clipboard event the content
> receives arrives through this one element.

The canvas cannot take keyboard focus and get an input method with it. A browser-native editable
element therefore does the receiving, and the canvas does all the showing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `CHROME`, shared with the
other five documents of the DOM side.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-007` | `INV` | The pipe MUST sit at the caret's real position. | this spec section 2 | none | unheld -- the placement is a transform the view refreshes as the caret moves. If it drifts, the input-method window opens in the wrong corner of the screen, and no test in the wasm tier drives a composition; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-059` | `INV` | The pipe MUST display nothing: no visible text, no native caret, no selection highlight. | this spec section 2, the property table | none | unheld -- the rule is four style declarations in the shell stylesheet, and nothing reads them back. A drifting one draws a second caret or a doubled selection under the one the canvas paints; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-016` | `INV` | The pipe MUST be moved by a transform, never by an offset that reflows the page. | this spec section 2 | none | unheld -- the placement writes a transform and a height and nothing else, so the element stays off the layout path. Nothing refuses the offset that would put it back on; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Type a character

An ordinary keystroke reaches the buffer, is read whole, and is cleared. A navigation key never
gets there: it rides the chord path instead.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-061` | `POST` | A typed character MUST reach the view as one committed run. | this spec section 2, the three paths in | none | unheld -- the buffer listener forwards the whole buffer as one run, and no test in the wasm tier types into the pipe; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-017` | `POST` | The pipe MUST be emptied after every commit. | this spec section 2 | none | unheld -- the host clears after each committed run and an opening edit clears again. A missed clear repeats the whole buffer into the document on the next keystroke; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-062` | `FRAME` | The chord path MUST NOT consume a printable character. | this spec section 2, the three paths in; this spec section 3, row 5 | none | unheld -- every key reaches the view first, and the buffer event fires only for the keys it leaves unconsumed, because a consumed key has its default prevented. A view that consumed a letter would silence typing, and nothing asserts it does not; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-020` | `FAIL` | A key aimed at another editable element MUST NOT reach the content. | this spec section 2 | none | unheld -- the document listener tests the event's target against the pipe and against the editable tags, input, textarea, select and contenteditable, and returns early. Nothing refuses a listener added later without that test, which is a formula-bar Enter committing the cell twice; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Compose with an input method

The three composition events open a window in which the buffer holds a half-formed run and is not to
be read.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-063` | `POST` | The preview MUST come from the composition events. | this spec section 2, the composition window | none | unheld -- each update event forwards the running preview and the canvas draws it as unconfirmed text. No test drives an input method, in any tier; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-019` | `POST` | A composition MUST end by committing its final text. | this spec section 2; `ambre-spec-text-editor.md`, `CANVAS-040` | none | unheld -- the last composition event forwards the final run and the editing core replaces the preview with it. No test drives an input method, in any tier; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-064` | `POST` | A composition MUST end by clearing the preview. | this spec section 2, the composition window | none | unheld -- the last event lowers the flag and clears the buffer, and the editing core drops the preview when the final run replaces it. A preview left behind is text the canvas keeps drawing and the document does not hold; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-018` | `FRAME` | While a composition is running, the buffer MUST NOT be read. | this spec section 2 | none | unheld -- a shared flag is raised on the first composition event and lowered on the last, and the buffer listener returns early while it is up. Nothing asserts the two stay in step, and a stuck flag silences typing altogether; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Copy, cut or paste

The browser maps every clipboard chord, the legacy set included, onto three events on the pipe.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `CHROME-021` | `POST` | A copy, a cut and a paste MUST be serviced as the events the browser already sends. | this spec section 2; `ambre-spec-clipboard.md` | none | unheld -- three listeners sit on the pipe, so the legacy chords work for free. No test in the wasm tier raises one; lands with the `The UI-thread belts` Story (`ambre-roadmap.md` E9) | untested |
| `CHROME-060` | `FRAME` | Servicing a clipboard event MUST NOT decode its chord a second time. | this spec section 2; `ambre-spec-commands.md`, `UI-THREAD-011` | `ambre_commands::tests::the_keymap_binds_no_browser_serviced_chord`; `ambre_trame::ui::commands::tests::the_keymap_binds_no_clipboard_chord`; `ambre_toile::ui::commands::tests::the_clipboard_intents_carry_the_shared_identity` -- the seeding refuses the three identities, so a table that declares their chords still binds none of them and a fourth binding fails the belt rather than pasting twice | none | verified |
* **Budget.** the keystroke-to-draft row of `ambre-spec-budgets.md section 1`. This element
  is the first hop of that path, and the figure is measured nowhere.
* **Boundary.** the operating system clipboard, and nothing else. What the copy and the paste may
  carry is `UI-THREAD-006`; the pipe opens no socket, holds no key and writes no store.
* **Verification.** 1 `verified`, 12 `untested`. The one held row is the one that is pure data:
  `CHROME-060`, the chords a table may bind. The other twelve are loose for a single reason: every
  rule here is a property of a live browser with an input method attached. The wasm tier drives a
  document but not an input method (`ambre-spec-testing.md section 8`). The gap is one gap, not
  twelve.

---

## 2. MODEL

**CRDT containers.** none. The pipe carries a transient string; what a commit does to the document
belongs to the editor that consumes it.

**Why an invisible element.** A canvas element takes focus but gets no input method with it. An
input-method window anchors to the focused editable element, and reads its position from the layout.
So the design is not "a hidden field": the element must be at the caret, in the page, and merely
invisible.

| Property | Value | Why |
|---|---|---|
| text colour, caret colour, background | transparent | the canvas draws the text, the caret and the selection |
| selection highlight | transparent | a second highlight under the drawn one is a doubled selection |
| pointer events | none | a click in the content is the canvas's, and the view focuses the pipe itself |
| size | one line high, one pixel wide | it holds no visible text, and the width is what keeps it out of the way |
| position | a transform onto the caret rectangle | an offset would reflow, and a far-off placement opens the input-method window in a corner |
| autocorrect, autocapitalize, spellcheck | off | the document is the source of truth, not the buffer |

**The three paths in, and what separates them.**

| Path | Event | What the view receives |
|---|---|---|
| a chord | a key event on the document | the key and its modifiers, and a flag back saying whether the default was prevented |
| ordinary text | a buffer event on the pipe, outside a composition | the whole buffer as one committed run, after which the buffer is cleared |
| a composition | the three composition events | the running preview, then the final run |

The chord path is deliberately separate from the text path. Navigation and editing keys ride the
document listener even while the pipe holds focus. A printable character is left unconsumed, so the
buffer event fires for it.

**The composition window, in steps.** Latin typing never enters it.

1. the first composition event raises a shared flag;
2. the buffer listener returns early while the flag is up (`CHROME-018`);
3. each update event forwards the running preview, which the canvas draws as unconfirmed text;
4. the last event forwards the final run (`CHROME-019`), lowers the flag and clears the buffer
   (`CHROME-064`).

**The clipboard.** Three listeners sit on the pipe: copy, cut and paste. The browser is what maps
every chord onto them, the legacy set included. What the payload holds, how a same-tab paste is
recognised and what a cut means per application are `ambre-spec-clipboard.md`.

**Who owns what.** The element is created and placed by the host; the view decides where and when.

| The host owns | The view owns |
|---|---|
| creating the element and attaching it under the canvas cell | when it takes focus |
| the six listeners, and the flag they share | where it sits, refreshed as the caret moves |
| clearing the buffer after a committed run | what a committed run does to the document |

**One pipe, and a child view reuses it.** A document opened inside another gets the same element,
constructed in a mode that does not steal focus from the host document.

**Code:** `crates/ambre-canvas/src/text_input.rs` creates and places the element, and
`crates/ambre-app/src/host/run.rs` attaches the six listeners and owns the composition flag.
`crates/ambre-app/src/host/mod.rs` holds the test that keeps a key aimed at another field away from
the content. The placement callers are `crates/ambre-trame/src/ui/view/lifecycle.rs` and
`crates/ambre-toile/src/ui/view.rs`, and the style is `crates/ambre-app/styles.css`.

---

## 3. SURFACE

The user never sees this element. What they see is whether typing lands where the caret is.

| Trigger | Behaviour |
|---|---|
| a click in the content | the pipe takes focus, so the next keystroke reaches the document |
| a character is typed | it reaches the view as a committed run, and the buffer is emptied |
| an input method is used | the composing run shows underlined at the caret, and its window opens under the caret |
| the composition is accepted | the final run replaces the preview and becomes document text |
| an arrow key, Home or End | the chord path handles it and the buffer never sees it |
| a copy or a cut | the selection is written to the operating system clipboard |
| a paste | the clipboard is read back and handed to the view |
| the caret moves | the element follows it, so the next input-method window opens in the right place |
| a key typed in the formula bar or a dialog field | that field handles it, and the content does not |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| An invisible editable element, at the caret | a key listener on the canvas element | it gets no input method, no composition events and no mobile keyboard. An editable element is the only way to get an input method, a native clipboard and a mobile keyboard on canvas content, and every browser anchors the composition window to the focused editable element's box, so the element has to be where the caret is. |
| It is placed by a transform | writing an offset each frame | an offset puts a per-frame write back on the layout path, which is the one thing the whole rendering design exists to avoid. A transform is composited and does not reflow, so following the caret costs nothing per frame. |
| It is at the caret and not off screen | parking it at a large negative offset | the classic trick parks the element far off screen so it can never be seen, but the input-method window then opens there too: on a spreadsheet the candidate list appears in a corner of the screen with the text being typed in the middle. |
| The buffer is drained and cleared | keeping the buffer in step with the document | the document is the source of truth and the buffer is a courier. Keeping state in it means two models of the same text, and a missed clear repeats it. |
| The composition is gated by a flag | reading the buffer and deciding whether it looks finished | mid-composition the buffer holds a half-formed run whose shape differs per input method and per platform. The events say exactly when the run is provisional, so nothing has to guess. |
| Chords ride the document listener, text rides the pipe | one listener deciding both | it has to reimplement what a character key means per keyboard layout. A chord is a command and text is content, and they need different handling: one is consumed and prevented, the other is left to produce a buffer event. Separating them at the listener is what keeps a printable key from being decoded twice. |
| The clipboard is serviced, not decoded | binding the six chords in the keymap | the browser already maps both the modern and the legacy chords onto three events. Decoding the chords ourselves means owning that mapping per platform, and racing the browser's own handling. |
| One pipe for every view, including an embedded one | one element per view | two editable elements over one canvas means two focus owners, and the composition window follows whichever one the browser thinks is focused. A child view therefore borrows the parent's element rather than creating its own. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The accessible label is written for a spreadsheet.** The element is created in the shared crate
  with a fixed label and role description naming a spreadsheet cell. A board and a page therefore
  announce themselves as one. It is a defect of `ambre-spec-accessibility.md`'s surface, not of this pipe's
  behaviour. That is where the label rule belongs.
* **No test in any tier drives a composition.** Six of the seven rows above describe what happens
  between the first and last composition event. The mechanism that would observe it does not exist:
  a headless browser sends key events, not input-method events.
* **Nothing rules on a composition interrupted by a remote change.** A concurrent edit that moves
  the text under a running composition has no stated behaviour. `UI-THREAD-014` names the reanchor
  as part of the substrate it schedules.
* **Nothing bounds the buffer.** A paste of a very large run arrives as one committed string. No
  row says what the view does with it before it reaches the document.
* **The mobile keyboard is unruled.** The element is a text area with wrapping off. No row says
  what a soft keyboard's autocorrect, its return key or its selection handles do to it.
* **A second editable element over the canvas would break the pipe silently.** `UI-THREAD-001` says
  the pipe is the only one, and nothing refuses the second.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The caret, the selection and the composition preview inside one unit of text | `ambre-spec-text-editor.md` |
| Which chord means extend, and which means jump by word | `ambre-spec-ui.md section 2` |
| What a copy carries, and how a same-tab paste is recognised | `ambre-spec-clipboard.md` |
| What the canvas and the DOM each own | `ambre-spec-ui.md section 2` |
| The label the element announces, and the live region beside it | `ambre-spec-accessibility.md` |
| The menus, the modals and the status bar around the content | `ambre-spec-chrome.md` |
| When the caret rectangle is recomputed, and when the frame runs | `ambre-spec-frame.md` |
| What a committed run does to a cell, a note or a paragraph | `2-Trame/features/ambre-spec-trame-cell-editing.md`, `3-Toile/features/ambre-spec-toile-text-editing.md`, `4-Prose/features/ambre-spec-prose-text-editing.md` |
