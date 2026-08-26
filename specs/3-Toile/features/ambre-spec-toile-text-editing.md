# AMBRE -- FEATURE: TEXT EDITING ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** partial -- a body is edited in place; it is one whole value, so a typist loses a sentence.

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-059` | Placing a note or a text element MUST open its editor and MUST return the tool to select. | this spec section 4 | none. The tool arms the deferred open and the view opens it when the element arrives, and no test drives the two halves together. The create half is `ambre_toile::ui::tool::tests::sticky_click_creates_sticky_with_empty_text_and_picked_fill` | unverified |
| `TOILE-UI-060` | Typing a printable character on one selected text-bearing element MUST enter its editor and replace the body. | this spec section 4 | partial: `ambre_toile::ui::palette::tests::editable_tag_guards_text_entry` holds which kinds accept it. Nothing drives the keystroke through the view, and nothing asserts the replacement | unverified |
| `TOILE-UI-061` | A double-click MUST open the editor with the body kept and the caret at its end. | this spec section 4 | none. The two entries differ only in what the editor is seeded with, and no test opens either | unverified |
| `TOILE-UI-062` | Leaving the editor MUST commit the body and MUST keep the element selected. | this spec section 4 | none. `Escape` closes the editor and leaves the selection, and the same key clears a selection when no editor is open, so the two behaviours share one handler and neither is driven | unverified |
| `TOILE-UI-063` | The body under edit MUST match its committed look: the same ink, and the same size. | this spec section 3 | `ambre_toile::ui::render::tests::text_ink_precedence`, `::sticky_font_size_auto_unless_overridden`, `::sticky_font_size_tracks_the_card_and_is_clamped`, `::text_element_paints_no_card`; the editor is seeded from the same two helpers those tests drive | verified |
| `TOILE-UI-064` | A body edit MUST be one undo item, however many keystrokes it took. | this spec section 3; `TOILE-014` | none. The editor holds the buffer and sends one whole-body write when it closes, so the granularity holds by construction, and nothing drives an edit and counts the items | unverified |
| `TOILE-UI-065` | Formatting MUST be applicable to a range inside one body, not to the element alone. | this spec section 3 | not built -- lands with the "Rich text on the board" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** a keystroke to a draft under 2 ms and text layout of a dirty paragraph under 2 ms, two
  rows of `ambre-spec-budgets.md section 1` and section 5.B. The editor echoes locally and
  shapes at paint time, so no keystroke waits for a round trip, and neither figure has a bench.
* **Boundary.** none: the editor is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. The committed body enters the document and leaves as bytes the Sync
  worker seals (`ZK-001`).
* **Verification.** 1 `verified`, 5 `unverified`, 1 `unimplemented`. What the body looks like is held
  four ways, because the editor and the renderer share the helpers that decide it. Every gesture that
  opens, closes or seeds the editor is unheld: this is the least-verified surface of the application.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types into a note | watches | nothing until Alice leaves the editor | Alice's letters locally, and Bob's screen unchanged until the commit | none -- section 6 |
| types "hello" into a note | types "world" into the same note | one of the two bodies, whole | one sentence, and the other user's typing gone (`TOILE-030`) | none, and the rule fails -- `ambre-spec-toile-schema.md section 1` |
| leaves the editor without changing anything | watches | nothing: an unchanged body is no write | no change, and no undo item | none -- section 6 |
| deletes the note being edited elsewhere | is editing it | the deletion | Bob's editor over an element that is gone | none -- section 6 |

The second row is the defect the format records: the body is one value two writers race for, not a
sequence they merge. Every other row of this table is about one user's own editor.

---

## 3. MODEL

**Containers.** None of its own. The body is one field on the element, declared in
`ambre-spec-toile-schema.md section 3` and carrying the rule it breaks (`TOILE-030`).

**Command** (`ambre-bus`): `SetText { id, text }` -- the whole body, written once when the editor
closes. It refuses any element that carries no body field, and a locked one
(`TOILE-022`, `TOILE-033`).

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

**Steps, when an editor opens:** find the element in the visible set, read its body, seed the core with
the pinned style, and select the whole body when the entry was a keystroke rather than a
double-click (`TOILE-UI-060`, `TOILE-UI-061`).

**Steps, when it closes:** ask the core for the committed body, which answers nothing when it is
unchanged, and send one whole-body write for a real change (`TOILE-UI-064`). The local painted body is
updated at the same moment so the committed look does not flash the old value for a frame.

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
| a double-click on a note or a text element | opens the editor, body kept, caret at the end (`TOILE-UI-061`) |
| a printable character, one text-bearing element selected, no editor | opens the editor and replaces the body (`TOILE-UI-060`) |
| the space bar, in the same situation | pans instead: space is excluded from entry (`features/ambre-spec-toile-viewport.md`) |
| `Escape` | commits and closes; the element stays selected (`TOILE-UI-062`) |
| copy, cut and paste with an editor open | act on the characters, not the elements (`features/ambre-spec-toile-clipboard.md`) |
| a double-click on a connector | would open the same editor on its caption, and cannot: no connector is pickable (`TOILE-UI-035`) |

**What the user sees.** A caret blinking on a one-second cycle, so the field reads as editable, and the
selection band and glyphs painted over the element in the overlay pass
(`features/ambre-spec-toile-board-rendering.md`). A note's glyphs rotate with the card.

**Replace-on-type is the spreadsheet behaviour**, deliberately: it matches Trame's cell, so one gesture
means one thing across the suite (`features/ambre-spec-trame-cell-editing.md`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The editor is the shared core, wrapped | the buffer, the caret, the selection model and the composition queue are the same problem in every application, and a second implementation is a second set of input bugs | a Toile-specific editor |
| The editor holds the body until it closes | a keystroke that waited for a round trip would be visible as lag, and one write per keystroke would make one sentence a hundred undo items | writing each keystroke through the document |
| The editor and the renderer share the style helpers | any divergence is visible as a jump at the moment the user stops typing, which is the least forgiving moment | seeding the editor from the style register directly |
| A note's body tracks the card unless overridden | a note is read at whatever zoom it sits at, and a fixed size makes a large note mostly empty. An explicit size still wins, so the control is there when it is wanted | a fixed size, with the user resizing the text |
| A note's ink is fixed, not its fill | the fill is the card, so drawing the body in it would make every note blank | using the fill for the text |
| Placing an element opens its editor | a user who placed a note is about to type in it, and the alternative is that their first keystroke places a second note | leaving the tool armed |
| An unchanged body is not a write | leaving an editor is not an edit, and a write would broadcast a change and consume an undo item for nothing | committing unconditionally |

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
* **No gesture that opens or closes the editor has a test** (`TOILE-UI-059` to `TOILE-UI-062`). Four
  rules, four behaviours in the view, and nothing drives any of them: a regression in the open path
  would ship silently.
* **Nothing happens when the edited element is deleted.** The editor holds the element's identity and no
  handler closes it when the element leaves the visible set, so an editor can be left over a hole and
  its commit will be refused.
* **A body has no wrap width of its own.** The editor wraps to the element's width, so narrowing a note
  reflows its body and nothing bounds the result: text can overflow the card.
