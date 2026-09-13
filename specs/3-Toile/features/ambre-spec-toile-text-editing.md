# AMBRE -- FEATURE: TEXT EDITING ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** partial -- a body is edited in place; it is one whole value, so a typist loses a
sentence, and no gesture that opens or closes the editor has a test.

> A user double-clicks a note and types into it, where it sits, at the size it will keep. Placing a
> new note opens its editor straight away, so the next thing typed goes in rather than placing
> another note.

The editor is the shared one every application uses. What is Toile's is which element is being
edited, and how the body looks while it is.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The editing core -- the buffer, the caret,
the selection, the composition queue -- is `ambre-spec-text-editor.md`, shared with Trame's cell
editor and not restated here.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-183` | INV | At most one editor is open at a time, and it names exactly one element. | this spec section 3 | structural, and the type is the proof: the view holds `Option<InlineEditor>` and the wrapper holds one `ElementId` | verified |
| `TOILE-UI-184` | INV | An editor MUST NOT open on an element that carries no body: only a note and a text element are editable. | this spec section 4 | none. The open returns early on any other kind, and nothing drives it. The kind test the old row cited (`::palette::tests::editable_tag_guards_text_entry`) holds which DOM tag counts as typing, not which element carries a body | unverified |
| `TOILE-UI-063` | OBS | The body under edit MUST be drawn in the ink the committed body uses. | this spec section 3 | `ambre_toile::ui::render::tests::text_ink_precedence`, `::text_element_paints_no_card`; the editor seeds its colour from `ambre_toile::ui::render::resolved_text_ink`, the function those two drive | verified |
| `TOILE-UI-185` | OBS | The body under edit MUST be drawn at the size the committed body uses: the element's own, or for a note one that tracks the card. | this spec section 3 | `ambre_toile::ui::render::tests::sticky_font_size_auto_unless_overridden`, `::sticky_font_size_tracks_the_card_and_is_clamped`; the editor seeds its size from `ambre_toile::ui::render::sticky_or_style_font_size`, the function those two drive | verified |
| `TOILE-UI-186` | OBS | The body under edit MUST carry the element's opacity, like the committed one. | this spec section 3 | none, and the rule fails today -- section 6. Answered by the `The inline editor shows the committed body` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-187` | OBS | The body under edit MUST carry the underline and the strikethrough the committed body paints. | this spec section 3 | none, and the rule fails today -- section 6. Answered by the `The inline editor shows the committed body` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-188` | OBS | An element whose style size is zero MUST look the same under edit as outside it. | this spec section 3 | none, and the rule fails today -- section 6. Answered by the `The inline editor shows the committed body` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-189` | OBS | The body under edit MUST start at the committed body's origin and wrap at its width, so entering edit does not reflow it. | this spec section 3 | none. Both passes inset the element box by `ambre_toile::ui::edit::InlineEditor::pad`, so it holds by construction, and no test paints the two and compares | unverified |
| `TOILE-UI-190` | OBS | A rotated note MUST keep its rotation while it is edited. | this spec section 4 | none. `ambre_toile::ui::render::tests::rotated_sticky_body_rotates_its_glyphs` holds the committed body; the overlay rotates about the same centre and no test paints it | unverified |
| `TOILE-UI-191` | OBS | The committed body MUST NOT be painted under the editor: one body is on screen, not two. | this spec section 3 | none. Every test of the content pass passes no element under edit, so the branch that drops it is never taken | unverified |
| `TOILE-UI-192` | OBS | An open editor MUST show a caret blinking on a one-second cycle, so the element reads as editable. | this spec section 4 | none. The blink is a frame-clock parity in the paint pass, and no test drives two frames a half-cycle apart | unverified |
| `TOILE-UI-065` | OBS | Formatting MUST be applicable to a range inside one body, not to the element alone. | this spec section 3 | not built -- lands with the "Rich text on the board" Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Place a note or a text element

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-059` | POST | Placing a note or a text element MUST open its editor on the element just placed. | this spec section 4 | none. The tool arms a deferred open and the view opens it when the element's geometry lands, and no test drives the two halves together. The create half is `ambre_toile::ui::tool::tests::sticky_click_creates_sticky_with_empty_text_and_picked_fill` | unverified |
| `TOILE-UI-193` | POST | Placing a note or a text element MUST return the tool to select, so the next click does not place a second one. | this spec section 4 | none. The tool reverts where the placement command is emitted, and no test reads the tool after a placement | unverified |
| `TOILE-UI-194` | POST | The deferred open MUST be claimed by the element the tool placed, and by no other. | this spec section 4 | none, and the rule fails today -- section 6. Answered by the `The inline editor shows the committed body` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-195` | FAIL | Given an element the visible set does not yet carry, the deferred open MUST be kept until it does, not dropped. | this spec section 4 | none, and the rule fails today -- section 6. Answered by the `Toile reads the board, not the screen` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-196` | FRAME | Opening an editor MUST NOT write to the document, move the camera, or change the board: it is view state, and the placement's own write is the only one. | this spec section 3 | none. The open reads the visible set and sets view fields, and no test asserts the absence of a command | unverified |

### 1.3 Double-click a note, or click one already selected

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-061` | POST | A double-click on a note or a text element MUST open its editor with the body kept. | this spec section 4 | none. The two entries differ only in what the editor is seeded with, and no test opens either | unverified |
| `TOILE-UI-197` | POST | A double-click MUST leave the caret at the end of the kept body, not select it. | this spec section 4 | none. The shared core seeds the caret at the end on its first flush, and nothing drives the Toile entry | unverified |
| `TOILE-UI-198` | POST | A click on an element that is already the whole selection MUST open its editor, so a second click edits what the first selected. | this spec section 4 | none. The press arms a candidate and the release opens it, and no test drives a press and a release | unverified |
| `TOILE-UI-199` | FAIL | Given a press that moved before its release, the click MUST NOT open an editor: a drag is a move, not an entry. | this spec section 4 | none. The release compares the travel against the click slop, and no test drives a drag over a selected element | unverified |
| `TOILE-UI-200` | POST | Opening an editor MUST make its element the whole selection, so the chrome tracks what is being typed into. | this spec section 4 | none. The open overwrites the selection with the one id, and no test reads the selection after an open | unverified |
| `TOILE-UI-201` | FRAME | Entering an editor MUST NOT change the body, the style or the geometry of the element: entering is not editing. | this spec section 3 | none. The open clones the body into the core and writes no command, and no test asserts it | unverified |

### 1.4 Type over a selected note

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-060` | POST | Typing a printable character with one text-bearing element selected and no editor open MUST enter its editor. | this spec section 4 | none. Nothing drives a keystroke through the view. The old row cited a DOM-tag guard (`ambre_toile::ui::palette::tests::editable_tag_guards_text_entry`), which holds neither the entry nor the kinds | unverified |
| `TOILE-UI-202` | POST | The character that opened the editor MUST replace the body, not extend it: one gesture means one thing across the suite. | this spec section 4 | none. The open queues a select-all and the character is then inserted over it, and nothing drives the keystroke or asserts the replacement | unverified |
| `TOILE-UI-203` | FRAME | The character that opened the editor MUST NOT be swallowed: it is the first one typed into it. | this spec section 4 | none. The handler returns unconsumed so the text pipe still emits it, and no test asserts the body after one keystroke | unverified |
| `TOILE-UI-204` | FAIL | Given the space bar, entry MUST be refused: space pans the board (`features/ambre-spec-toile-viewport.md`). | this spec section 4 | none. The entry excludes the one key, and no test presses it over a selected note | unverified |
| `TOILE-UI-205` | FAIL | Given a selection that is not exactly one text-bearing element, typing MUST NOT open an editor: the key stays a shortcut. | this spec section 4 | none. The entry resolves the selection through the visible set, and no test types over an empty, a multiple or a shape selection | unverified |

### 1.5 Leave the editor

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-062` | POST | Leaving the editor MUST commit the body as one whole-body write. | this spec section 4 | none. `Escape` closes the editor and the same key clears a selection when none is open, so the two behaviours share one handler and neither is driven | unverified |
| `TOILE-UI-206` | POST | Leaving the editor MUST keep its element selected, so the next gesture acts on what was just typed into. | this spec section 4 | none. The selection is set at the open and the close does not touch it, and no test reads it after a close | unverified |
| `TOILE-UI-064` | POST | A body edit MUST be one undo item, however many keystrokes it took. | this spec section 3; `TOILE-014` | none. The editor holds the buffer and sends one whole-body write when it closes, so the granularity holds by construction, and nothing drives an edit and counts the items | unverified |
| `TOILE-UI-207` | FAIL | Given a body unchanged since the open, leaving MUST write nothing: leaving an editor is not an edit. | this spec section 5 | none. The core answers nothing when the buffer matches the body it was seeded with, and no test closes an untouched editor | unverified |
| `TOILE-UI-208` | FRAME | A commit MUST NOT touch anything but the body: not the bounds, not the style, not the camera, not the tool. | this spec section 3 | none. One command carries the id and the text, and no test asserts what a close leaves alone | unverified |
| `TOILE-UI-209` | FAIL | Given a locked element, the screen MUST NOT show a body the write will be refused (`TOILE-033`). | this spec section 3 | none, and the rule fails today -- section 6. Answered by the `The inline editor shows the committed body` Story (`ambre-roadmap.md` E7) | unverified |
| `TOILE-UI-210` | FAIL | Given a pan that carries the edited element off screen, the editor MUST NOT close: scrolling is not leaving. | this spec section 4 | none, and the rule fails today -- section 6. Answered by the `Toile reads the board, not the screen` Story (`ambre-roadmap.md` E7) | unverified |

* **Budget.** a keystroke to a draft under 2 ms and text layout of a dirty paragraph under 2 ms, two
  rows of `ambre-spec-budgets.md section 1` and section 5.B. The editor echoes locally and
  shapes at paint time, so no keystroke waits for a round trip, and neither figure has a bench.
* **Boundary.** none: the editor is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The committed body enters the document and leaves as bytes the Sync
  worker seals (`ZK-001`).
* **Verification.** 3 `verified`, 31 `unverified`, 1 `unimplemented`. The rows went 7 to 35, and the
  four tests that backed the one verified row before back two of the three after: the split found no
  mechanism, and withdrew one. What it withdrew was a DOM-tag guard offered as holding which elements
  accept a keystroke, which is not what it asserts. What is held is what the body looks like, along
  the two axes the editor and the renderer read from a shared helper: the ink and the size. Four more
  axes came out of that same row and none of them holds; three fail today, and one of those shows a
  body a locked element will not keep. Every gesture that opens or closes the editor is unheld, and
  five that were not written down at all -- a click-to-edit entry, a drag that must not open one, a
  keystroke that must not be swallowed, a deferred open another element can steal, an editor a pan
  closes under the user -- came out of reading the view. This is the least-verified surface of the
  application.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types into a note | watches | nothing until Alice leaves the editor | Alice's letters locally, and Bob's screen unchanged until the commit | none -- section 6 |
| types "hello" into a note | types "world" into the same note | one of the two bodies, whole | one sentence, and the other user's typing gone (`TOILE-030`) | none, and the rule fails -- `ambre-spec-toile-schema.md section 1` |
| leaves the editor without changing anything | watches | nothing: an unchanged body is no write | no change, and no undo item | none -- section 6 |
| deletes the note being edited elsewhere | is editing it | the deletion | Bob's editor closing on its own, and his half-typed body written back or refused | none, and the rule fails today -- section 6 |
| pans the board while editing | watches | nothing | Alice's editor closing the moment the note leaves the screen (`TOILE-UI-210`) | none, and the rule fails today -- section 6 |

The second row is the defect the format records: the body is one value two writers race for, not a
sequence they merge. Every other row of this table is about one user's own editor.

---

## 3. MODEL

**Containers.** None of its own. The body is one field on the element, declared in
`ambre-spec-toile-schema.md section 3` and carrying the rule it breaks (`TOILE-030`).

**Command** (`ambre-bus`): `SetText { id, text }` -- the whole body, written once when the editor
closes. It refuses any element that carries no body field, and a locked one
(`TOILE-197`, `TOILE-033`).

**View state, while an editor is open.** The element under edit, and the shared editing core seeded
with the body and with a pinned style:

```rust
pub struct InlineEditor {
    /// The element being edited: its body is authoritative here until close.
    id: ElementId,
    /// The shared buffer, caret, selection and intent queue.
    core: ambre_canvas::TextEditor,
}
```

**How the pinned style is derived** (`TOILE-UI-063`). Both the editor and the renderer call the
same two helpers, so the text does not change style at the seam:

* the ink is the explicit text colour when the element carries one; otherwise a fixed readable ink for
  a note, whose fill is the card and not the text, and the theme colour for a text element;
* the size is the element's own when it carries a positive one; otherwise, for a note, a size that
  tracks the card, clamped at both ends.

The pinned style stops there. It carries no opacity, no underline and no strikethrough, and it floors
the size where the committed body does not, so four of the six axes the body is drawn along are
shared by construction and three of them diverge (`TOILE-UI-186` to `TOILE-UI-188`, section 6). The
origin and the wrap width are shared for real: both passes inset the element box by the same padding,
so the body does not re-wrap at the seam (`TOILE-UI-189`).

**Steps, when an editor opens:** find the element in the visible set, read its body, seed the core with
the pinned style, make it the whole selection (`TOILE-UI-200`), and select the whole body when the
entry was a keystroke rather than a double-click (`TOILE-UI-060`, `TOILE-UI-061`). An element the
visible set does not carry opens nothing, silently (`TOILE-UI-195`).

**Steps, when it closes:** ask the core for the committed body, which answers nothing when it is
unchanged (`TOILE-UI-207`), and send one whole-body write for a real change (`TOILE-UI-064`). The
local painted body is updated at the same moment so the committed look does not flash the old value
for a frame -- and it is updated before the write is judged, which is how a locked element shows a
body it will not keep (`TOILE-UI-209`, section 6).

**Code:**

* `crates/ambre-toile/src/ui/edit.rs` -- the wrapper: which element, and the pinned style;
* `ui/render.rs` -- the two helpers both sides share, and the painted body;
* `ui/view.rs` -- the open, the close, the caret blink and the local echo;
* `logic/state/elements.rs` -- the whole-body write and its two refusals.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the note or text tool, released | creates the element and opens its editor, tool back to select (`TOILE-UI-059`) |
| a double-click on a note or a text element | opens the editor, body kept, caret at the end (`TOILE-UI-061`, `TOILE-UI-197`) |
| a click on an element that is already the whole selection | opens the editor: a second click edits what the first selected (`TOILE-UI-198`) |
| the same press, released after a move | moves the element, and opens nothing (`TOILE-UI-199`) |
| a printable character, one text-bearing element selected, no editor | opens the editor and replaces the body (`TOILE-UI-060`, `TOILE-UI-202`) |
| the same character, with nothing or several things selected | stays a shortcut (`TOILE-UI-205`) |
| the space bar, in the same situation | pans instead: space is excluded from entry (`features/ambre-spec-toile-viewport.md`) |
| `Escape` | commits and closes; the element stays selected (`TOILE-UI-062`, `TOILE-UI-206`) |
| a click outside the element | commits and closes the same way |
| a pan that carries the element off screen | closes the editor and commits, which it must not (`TOILE-UI-210`) |
| copy, cut and paste with an editor open | act on the characters, not the elements (`features/ambre-spec-toile-clipboard.md`) |
| a double-click on a connector | would open the same editor on its caption, and cannot: no connector is pickable (`TOILE-UI-035`) |

**What the user sees.** A caret blinking on a one-second cycle, so the field reads as editable
(`TOILE-UI-192`), and the selection band and glyphs painted over the element in the overlay pass,
with the committed body dropped from the content pass so one body is on screen and not two
(`TOILE-UI-191`, `features/ambre-spec-toile-board-rendering.md`). A note's glyphs rotate with the
card, under edit as committed, and a pointer is unrotated the same way so a click lands on the glyph
it hit (`TOILE-UI-190`).

**Replace-on-type is the spreadsheet behaviour**, deliberately: it matches Trame's cell, so one gesture
means one thing across the suite (`features/ambre-spec-trame-cell-editing.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The editor is the shared core, wrapped | a Toile-specific editor | the buffer, the caret, the selection model and the composition queue are the same problem in every application, so a second implementation is a second set of input bugs |
| The editor holds the body until it closes | writing each keystroke through the document | a keystroke that waits for a round trip is visible as lag, and one write per keystroke makes one sentence a hundred undo items |
| The editor and the renderer share the style helpers | seeding the editor from the style register directly | any divergence is visible as a jump at the moment the user stops typing, which is the least forgiving moment |
| A note's body tracks the card unless overridden | a fixed size, with the user resizing the text | a note is read at whatever zoom it sits at, so a fixed size makes a large note mostly empty. An explicit size still wins, so the control is there when it is wanted |
| A note's ink is fixed, not its fill | using the fill for the text | the fill is the card, so drawing the body in it makes every note blank |
| Placing an element opens its editor | leaving the tool armed | a user who placed a note is about to type in it, and with the tool still armed their first keystroke places a second note |
| An unchanged body is not a write | committing unconditionally | leaving an editor is not an edit, and the write broadcasts a change and consumes an undo item for nothing |
| A click on an element that is already selected edits it | the double-click as the only entry | the second click on a note is always meant for its body, so requiring a double-click there costs a gesture for nothing, and a drag still moves it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A body is one value, not a sequence** (`TOILE-030`,
  `ambre-spec-toile-schema.md section 1`). Two users typing into one note keep one sentence between
  them. Three sections of the corpus rested on the opposite, and one of them offered it as the pattern
  for comment bodies, which is why that feature carries the same gap
  (`ambre-spec-toile-comments.md`).
* **Formatting applies to the element, not to a range** (`TOILE-UI-065`). Bold, italic, the family and
  the size live in the element's style register, so a word cannot be emphasized inside a sentence.
  The controls in the panel and the floating bar look like text controls and act on the whole element
  (`features/ambre-spec-toile-styles.md`).
* **No gesture that opens or closes the editor has a test** (`TOILE-UI-059` to `TOILE-UI-062`,
  `TOILE-UI-193` to `TOILE-UI-210`). Four entries, one exit, and nothing drives any of them: a
  regression in the open path would ship silently. What is held is what the body looks like once it is
  open, which is the half that does not involve the user.
* **The pinned style is not the committed style** (`TOILE-UI-186` to `TOILE-UI-188`). It carries the
  ink and the size and nothing else, so entering edit drops the underline and the strikethrough,
  forces the body opaque whatever the element's opacity, and floors the size at one pixel where the
  committed pass does not. An element whose size is zero -- reachable from the panel
  (`features/ambre-spec-toile-styles.md`) -- is invisible on the board and visible under edit.
* **The local echo runs ahead of the refusal** (`TOILE-UI-209`). The committed body is painted into
  the local viewport before the write is sent, and the write is what checks the lock
  (`TOILE-033`). A locked note therefore shows the new body until the next viewport
  arrives and takes it away.
* **A pan closes the editor** (`TOILE-UI-210`). The editor is driven from the culled set, and an
  element the set no longer carries is treated as gone: the editor commits and closes. Scrolling the
  board mid-sentence ends the sentence. The deleted-elsewhere case runs through the same branch, so
  it commits into a hole rather than leaving an editor over one.
* **The deferred open is not tied to an element** (`TOILE-UI-194`, `TOILE-UI-195`). Placing a note
  arms a flag, and the next created element -- a paste, a group, a boolean result -- claims it. The
  same open is dropped without trace when the placed element falls outside the visible set, which is
  the only reason the flag exists.
* **A body has no wrap width of its own.** The editor wraps to the element's width, so narrowing a note
  reflows its body and nothing bounds the result: text can overflow the card.
