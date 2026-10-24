# AMBRE -- FEATURE: TEXT EDITING ON AN ELEMENT

**Type:** Vertical (feature)
**Implementation:** partial -- a body is edited in place with its committed look, and a placement
opens its own editor. Two typists merge, even across an open editor. No gesture that opens or
closes the editor has a test.

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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-183` | `INV` | At most one editor MUST be open on the board at a time. | this spec section 3 | type-level -- the view holds one `Option<InlineEditor>`, so a second open editor has nowhere to live | none | verified |
| `TOILE-UI-361` | `INV` | An open editor MUST name exactly one element. | this spec section 3 | type-level -- the wrapper holds one `ElementId`, so an editor over two elements is unrepresentable | none | verified |
| `TOILE-UI-184` | `INV` | An editor MUST NOT open on an element that carries no body: only a note and a text element are editable. | this spec section 4 | `ambre_toile::ui::view::view_tests::an_element_without_a_body_opens_no_editor` -- a second click, a double-click and a key over a shape | none | verified |
| `TOILE-UI-063` | `OBS` | The body under edit MUST be drawn in the ink the committed body uses. | this spec section 3 | `ambre_toile::ui::render::tests::text_ink_precedence` -- the precedence; `ambre_toile::ui::render::tests::text_element_paints_no_card` -- the text element; structural -- the editor seeds its colour from `ambre_toile::ui::render::resolved_text_ink`, the function those two drive | none | verified |
| `TOILE-UI-185` | `OBS` | The body under edit MUST be drawn at the size the committed body uses: the element's own, or for a note one that tracks the card. | this spec section 3 | `ambre_toile::ui::render::tests::sticky_font_size_auto_unless_overridden` -- the element's own size; `ambre_toile::ui::render::tests::sticky_font_size_tracks_the_card_and_is_clamped` -- the note tracking its card; structural -- the editor seeds its size from `ambre_toile::ui::render::sticky_or_style_font_size`, the function those two drive | none | verified |
| `TOILE-UI-186` | `OBS` | The body under edit MUST carry the element's opacity, like the committed one. | this spec section 3 | `ambre_toile::ui::edit::tests::the_pinned_style_carries_opacity_and_decorations` -- the pinned ink's alpha carries the element opacity, as the committed pass bakes it | none | verified |
| `TOILE-UI-187` | `OBS` | The body under edit MUST carry the underline and the strikethrough the committed body paints. | this spec section 3 | `ambre_toile::ui::edit::tests::the_pinned_style_carries_opacity_and_decorations` -- the pinned style seeds both decorations, and the editor paints them in the body ink | none | verified |
| `TOILE-UI-188` | `OBS` | An element whose style size is automatic MUST look the same under edit as outside it. | this spec section 3 | `ambre_toile::ui::edit::tests::an_automatic_size_text_keeps_its_drawn_size_under_edit` -- the pinned size is the one the board draws, the free text default included | none | verified |
| `TOILE-UI-189` | `OBS` | The body under edit MUST start at the committed body's origin and wrap at its width, so entering edit does not reflow it. | this spec section 3 | `ambre_toile::ui::view::view_tests::the_body_under_edit_replaces_the_committed_one_in_place` -- each glyph run lands where the committed run did | none | verified |
| `TOILE-UI-190` | `OBS` | A rotated note MUST keep its rotation while it is edited. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_rotated_note_keeps_its_rotation_while_edited` -- each run lands where the rotated committed run did, the pushed rotation folded in | none | verified |
| `TOILE-UI-191` | `OBS` | The committed body MUST NOT be painted under the editor: one body is on screen, not two. | this spec section 3 | `ambre_toile::ui::view::view_tests::the_body_under_edit_replaces_the_committed_one_in_place` -- the open editor paints as many runs as the committed body, not twice as many | none | verified |
| `TOILE-UI-192` | `OBS` | An open editor MUST show a caret blinking on a one-second cycle, so the element reads as editable. | this spec section 4 | `ambre_toile::ui::view::view_tests::the_caret_blinks` -- the caret is painted at the start of the cycle, hidden at its middle, and painted again at its end | none | verified |
| `TOILE-UI-381` | `CONC` | A close MUST keep a peer's edit to the same body that arrived while the editor was open. | this spec section 3; `TOILE-030` | `ambre_toile::logic::convergence_tests::an_open_editor_keeps_a_peers_edit` -- a peer's edit lands between the open and the close, and both writers' words survive on both replicas; structural -- the close sends the body the editor opened on as the base (`ambre_toile::ui::edit::InlineEditor::opened_text`) | none | verified |
| `TOILE-UI-382` | `OBS` | Leaving the editor, on a note body or on an arrow's caption, MUST NOT take off the screen, even for a frame, a peer's words the document keeps. | this spec section 3; `TOILE-UI-381`; `TOILE-263` | `ambre_toile::logic::convergence_tests::a_close_echo_paints_what_the_document_keeps` -- on a note and on a caption, the echo merged from the opened, shown and typed texts keeps the peer's words and is the text Logic writes; `ambre_toile::logic::text_merge::tests::a_diff_rebuilds_its_target` -- the merge's diff is exact; structural -- both echoes call `ambre_toile::logic::text_merge::merge_text`, the function Logic writes through | none | verified |
| `TOILE-UI-065` | `OBS` | Formatting MUST be applicable to a range inside one body, not to the element alone. | this spec section 3 | not built | unbuilt -- lands with the `Rich text on the board` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Place a note or a text element

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-059` | `POST` | Placing a note or a text element MUST open its editor on the element just placed. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_placed_note_opens_its_editor_and_the_tool_returns_to_select` -- the click, the create reply, then the frame that carries the note | none | verified |
| `TOILE-UI-193` | `POST` | Placing a note or a text element MUST return the tool to select, so the next click does not place a second one. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_placed_note_opens_its_editor_and_the_tool_returns_to_select` -- it reads the tool after the placement | none | verified |
| `TOILE-UI-194` | `POST` | The deferred open MUST be claimed by the element the tool placed, and by no other. | this spec section 4 | `ambre_toile::logic::state::elements::inner_tests::a_create_echoes_its_correlate_token` -- a create echoes its token, and the view claims the reply carrying the token it armed, so a paste or a group cannot | none | verified |
| `TOILE-UI-195` | `FAIL` | Given an element the visible set does not yet carry, the deferred open MUST be kept until it does, not dropped. | this spec section 4 | structural -- the view resolves the deferred element against the selection data Logic sends, off-screen included, and retries each frame until it resolves, so a culled-but-created element is never dropped | none | verified |
| `TOILE-UI-196` | `FRAME` | Opening an editor MUST NOT write to the document, move the camera, or change the board: it is view state, and the placement's own write is the only one. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_second_click_on_the_sole_selection_opens_its_editor` -- no write and the same camera after the open, a frame and a paint | none | verified |

### 1.3 Double-click a note, or click one already selected

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-061` | `POST` | A double-click on a note or a text element MUST open its editor with the body kept. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_double_click_keeps_the_body_and_a_close_writes_it_once` -- a double-click through the view opens the editor seeded with the sticky's body | none | verified |
| `TOILE-UI-197` | `POST` | A double-click MUST leave the caret at the end of the kept body, not select it. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_double_click_keeps_the_body_and_a_close_writes_it_once` -- two keystrokes after the open extend the body rather than replace it | none | verified |
| `TOILE-UI-198` | `POST` | A click on an element that is already the whole selection MUST open its editor, so a second click edits what the first selected. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_second_click_on_the_sole_selection_opens_its_editor` -- the first click selects only, the second opens | none | verified |
| `TOILE-UI-199` | `FAIL` | Given a press that moved before its release, the click MUST NOT open an editor: a drag is a move, not an entry. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_drag_over_the_selected_note_opens_no_editor` | none | verified |
| `TOILE-UI-200` | `POST` | Opening an editor MUST make its element the whole selection, so the chrome tracks what is being typed into. | this spec section 4 | `ambre_toile::ui::view::view_tests::an_open_editor_is_the_whole_selection` -- a double-click inside a selection of two | none | verified |
| `TOILE-UI-201` | `FRAME` | Entering an editor MUST NOT change the body, the style or the geometry of the element: entering is not editing. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_second_click_on_the_sole_selection_opens_its_editor` -- no write, the body opened on, and the same bounds | none | verified |

### 1.4 Type over a selected note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-060` | `POST` | Typing a printable character with one text-bearing element selected and no editor open MUST enter its editor. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_typed_character_opens_the_editor_and_replaces_the_body` | none | verified |
| `TOILE-UI-202` | `POST` | The character that opened the editor MUST replace the body, not extend it: one gesture means one thing across the suite. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_typed_character_opens_the_editor_and_replaces_the_body` -- the close writes the one character, not the body extended | none | verified |
| `TOILE-UI-203` | `FRAME` | The character that opened the editor MUST NOT be swallowed: it is the first one typed into it. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_typed_character_opens_the_editor_and_replaces_the_body` -- the key is returned unconsumed, and the character reaches the body | none | verified |
| `TOILE-UI-204` | `FAIL` | Given the space bar, entry MUST be refused: space pans the board (`features/ambre-spec-toile-viewport.md`). | this spec section 4 | `ambre_toile::ui::view::view_tests::the_space_bar_opens_no_editor` | none | verified |
| `TOILE-UI-205` | `FAIL` | Given a selection that is not exactly one text-bearing element, typing MUST NOT open an editor: the key stays a shortcut. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_key_over_anything_but_one_note_opens_no_editor` -- no selection, two notes, and a shape | none | verified |

### 1.5 Leave the editor

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-062` | `POST` | Leaving the editor MUST commit the body as one whole-body write. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_double_click_keeps_the_body_and_a_close_writes_it_once` -- two keystrokes, then `Escape` and a frame, leave as one `SetText` carrying the opened body as its base | none | verified |
| `TOILE-UI-383` | `POST` | Leaving the editor MUST commit within the body commit budget, merge included. | this spec section 3; `ambre-spec-budgets.md section 1` | `callgrind` -- the `body_commit` operation of `crates/ambre-toile/benches/instructions.rs`, under its ceiling in the CI `instructions` job | none | verified |
| `TOILE-UI-206` | `POST` | Leaving the editor MUST keep its element selected, so the next gesture acts on what was just typed into. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_close_writes_the_body_once_and_keeps_the_element_selected` | none | verified |
| `TOILE-UI-064` | `POST` | A body edit MUST be one undo item, however many keystrokes it took. | this spec section 3; `TOILE-014` | `ambre_toile::ui::view::view_tests::a_close_writes_the_body_once_and_keeps_the_element_selected` -- three keystrokes leave as one write; `ambre_toile::logic::convergence_tests::a_body_write_is_one_undo_item` -- one undo restores the body | none | verified |
| `TOILE-UI-207` | `FAIL` | Given a body unchanged since the open, leaving MUST write nothing: leaving an editor is not an edit. | this spec section 5 | `ambre_toile::ui::view::view_tests::an_untouched_editor_closes_without_a_write` -- an editor opened and closed through the view sends no command | none | verified |
| `TOILE-UI-208` | `FRAME` | A commit MUST NOT touch anything but the body: not the bounds, not the style, not the camera, not the tool. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_close_writes_the_body_once_and_keeps_the_element_selected` -- one body write, and the camera, the tool, the selection and the bounds as they were | none | verified |
| `TOILE-UI-209` | `FAIL` | Given a locked element, the screen MUST NOT show a body the write will be refused (`TOILE-033`). | this spec section 3 | `ambre_toile::logic::state::elements::inner_tests::a_locked_element_refuses_a_text_write` -- it reads a locked sticky's body back unchanged after the write | none | verified |
| `TOILE-UI-210` | `FAIL` | Given a pan that carries the edited element off screen, the editor MUST NOT close: scrolling is not leaving. | this spec section 4 | structural -- the editor resolves its target's geometry from the selection data Logic sends, off-screen included, so a culled element no longer reads as gone | none | verified |
| `TOILE-UI-387` | `FAIL` | Given an element a peer deletes while the editor is open on it, the editor MUST close and tell the user the element is gone. | this spec section 4; `features/ambre-spec-toile-selection.md` (`TOILE-UI-385`) | `ambre_toile::ui::view::view_tests::an_editor_on_an_element_a_peer_deleted_closes_without_a_write_and_says_so` -- the frame drops the element, the refetch answers it missing, and the editor closes with no write and one notice | none | verified |
| `TOILE-UI-388` | `FAIL` | Given an arrow a peer deletes while its caption is open in the editor, the editor MUST close with no write and tell the user the arrow is gone. | this spec section 4; `features/ambre-spec-toile-connectors.md` (`TOILE-045`) | `ambre_toile::ui::view::view_tests::a_caption_editor_on_an_arrow_a_peer_deleted_closes_without_a_write_and_says_so` -- the board names the arrow it lost, and the editor closes with no write and one notice | none | verified |
| `TOILE-UI-389` | `FAIL` | Given a closing write the board refuses because its target is gone, the user MUST be told once that the edit was not saved. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_write_refused_for_its_lost_target_is_told_and_another_refusal_is_not` -- a body closed blind before the first selection answer: the refusal naming it raises one notice, a second raises none, and a refusal naming another id raises none | none | verified |

* **Budget.** a keystroke to a draft under 2 ms and text layout of a dirty paragraph under 2 ms
  (`ambre-spec-budgets.md section 1` and section 5.B). The editor echoes locally and shapes at paint
  time, so no keystroke waits for a round trip. A close holds the body commit row (`TOILE-UI-383`).
  No figure has a bench.
* **Boundary.** none: the editor is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. The committed body leaves sealed (`ZK-001`).
* **Verification.** 41 `verified`, 1 `unimplemented`. View tests drive every entry and every
  refusal. A close is read back as one body write, and one undo restores the body. Paint tests show
  the body under edit replaces the committed one in place. What is owed is formatting on a range
  (`TOILE-UI-065`).

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types into a note | watches | nothing until Alice leaves the editor | Alice's letters locally, and Bob's screen unchanged until the commit | none -- section 6 |
| types "hello" into a note | types "world" into the same note | both words, merged character by character | the same body holding both words, once both edits arrive (`TOILE-030`) | `logic::convergence_tests::two_authors_typing_in_one_note_both_keep_their_words` |
| types "please" into a note | has the same note open, and adds "and eggs" | both changes, merged from the body Bob opened on | both changes, from the moment Bob leaves the editor (`TOILE-UI-381`, `TOILE-UI-382`) | `logic::convergence_tests::an_open_editor_keeps_a_peers_edit` |
| leaves the editor without changing anything | watches | nothing: an unchanged body is no write | no change, and no undo item | none -- section 6 |
| deletes the note being edited elsewhere | is editing it | the deletion | Bob's editor closing on its own, his half-typed body dropped, and a notice saying why (`TOILE-UI-387`) | `ui::view::view_tests::an_editor_on_an_element_a_peer_deleted_closes_without_a_write_and_says_so` |
| deletes the arrow whose caption is being edited | is editing it | the deletion | Bob's editor closing on its own, with the same notice (`TOILE-UI-388`) | `ui::view::view_tests::a_caption_editor_on_an_arrow_a_peer_deleted_closes_without_a_write_and_says_so` |
| deletes the note before Bob's view has its data | has just opened it | the deletion | Bob's editor closing, its write refused, and the same notice (`TOILE-UI-389`) | `ui::view::view_tests::a_write_refused_for_its_lost_target_is_told_and_another_refusal_is_not` |
| pans the board while editing | watches | nothing | Alice's editor staying open while the note is off screen (`TOILE-UI-210`) | structural |

The second row is the only one with two writers: the body is a text they merge, not one value they
race for. Every other row of this table is about one user's own editor.

---

## 3. MODEL

**Containers.** None of its own. The body is one text on the element, seeded at its birth and
declared in `ambre-spec-toile-schema.md section 3` (`TOILE-030`).

**Command** (`ambre-bus`): `SetText { id, base, text }` -- the whole body, written once when the
editor closes, and the body it opened on. The handler merges the change from `base` to `text` into
the stored body, then writes the edits the merge found into its text. So an edit that landed while
the editor was open keeps its characters (`TOILE-UI-381`). The text is spared a diff of its own,
which over a 4 KB body cost forty times the budget (`TOILE-UI-383`). It
refuses any element that carries no body field, and a locked one
(`TOILE-197`, `TOILE-033`).

**View state, while an editor is open.** The element under edit, and the shared editing core seeded
with the body and with a pinned style:

```rust
pub struct InlineEditor {
    /// What is being edited -- an element's body, a connector's label or a
    /// frame's title: authoritative here until close.
    target: EditTarget,
    /// The shared buffer, caret, selection and intent queue.
    core: ambre_canvas::TextEditor,
}
```

**How the pinned style is derived** (`TOILE-UI-063`). Both the editor and the renderer call the
same two helpers, so the text does not change style at the seam:

* the ink is the explicit text colour when the element carries one. Otherwise it is a fixed readable
  ink for a note, whose fill is the card and not the text. For a text element it is the theme colour;
* the size is the element's own when the style names one. For a note, an automatic size tracks the
  card, clamped at both ends. A text element has no card, so its automatic size is the default 12 points.

The pinned style carries the whole committed look. It bakes the opacity into the ink, keeps the
decorations, and uses the committed size, an automatic one included (`TOILE-UI-186` to `TOILE-UI-188`).
The origin and the wrap width are shared for real: both passes inset the element box by the same
padding. The body does not re-wrap at the seam (`TOILE-UI-189`).

**Steps, when an editor opens:** find the element in the visible set, read its body, and seed the
core with the pinned style. The element becomes the whole selection (`TOILE-UI-200`), and the whole
body is selected when the entry was a keystroke rather than a double-click (`TOILE-UI-060`,
`TOILE-UI-061`). An element the
visible set does not carry opens nothing, silently (`TOILE-UI-195`).

**Steps, when it closes:** ask the core for the committed body, which answers nothing when it is
unchanged (`TOILE-UI-207`). One whole-body write is sent for a real change (`TOILE-UI-064`). It
names the body the core opened on, so the handler merges it into the stored body (`TOILE-UI-381`). The
local painted body is updated at the same moment, so the committed look does not flash the old value
for a frame.

The echo is the same merge, over the body the viewport shows. A peer's words that landed while the
editor was open therefore stay painted (`TOILE-UI-382`). A locked element is not echoed: the
typed body is not shown, because the write will refuse it (`TOILE-UI-209`).

**The merge** (`logic/text_merge.rs`) is pure and holds no document, so the UI thread may run it
(`UI-THREAD-051`). It diffs the base against each side by character (Myers), past a common prefix and
suffix. Runs the two sides change apart both apply. A run both replace differently keeps the stored
text, then the typed one. Past 256 edit steps a side becomes one run between its prefix and suffix.

**Code:**

* `crates/ambre-toile/src/ui/edit.rs` -- the wrapper: which element, and the pinned style;
* `ui/render.rs` -- the two helpers both sides share, and the painted body;
* `ui/view.rs` -- the open, the close, the caret blink and the local echo;
* `logic/state/elements.rs` -- the whole-body write, merged from its base into the text, and its
  two refusals.

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
| a peer deletes the element under edit | closes with no write, and a warning notice says the edit was not saved (`TOILE-UI-387`) |
| a peer deletes the arrow whose caption is under edit | closes with no write and the same notice (`TOILE-UI-388`) |
| the board refuses the closing write because its target is gone | the same notice, once (`TOILE-UI-389`) |
| a pan that carries the element off screen | keeps the editor open (`TOILE-UI-210`), a frame's title included (`TOILE-UI-390`) |
| copy, cut and paste with an editor open | act on the characters, not the elements (`features/ambre-spec-toile-clipboard.md`) |
| a double-click on a connector | opens the same editor on its caption (`TOILE-UI-035`) |

**What the user sees.** A caret blinks on a one-second cycle, so the field reads as editable
(`TOILE-UI-192`). The selection band and the glyphs are painted over the element in the overlay
pass. The committed body is dropped from the content pass, so one body is on screen and not two
(`TOILE-UI-191`, `features/ambre-spec-toile-board-rendering.md`). A note's glyphs rotate with the
card, under edit as committed. A pointer is unrotated the same way, so a click lands on the glyph it
hit (`TOILE-UI-190`).

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

* **A peer's edit still in flight at the close.** The echo merges the body the viewport shows. An edit
  Logic has applied but whose viewport has not reached the view yet is missing from the echo. It was
  never on screen, so nothing leaves it (`TOILE-UI-382` holds), and it appears with that viewport.
* **A long clash duplicates text.** Past 256 edit steps, each side is one run. Two such runs that
  overlap keep both, with the base text they share written twice. A person must then delete one copy.
* **Formatting applies to the element, not to a range** (`TOILE-UI-065`). Bold, italic, the family and
  the size live in the element's style register, so a word cannot be emphasized inside a sentence.
  The controls in the panel and the floating bar look like text controls and act on the whole element
  (`features/ambre-spec-toile-styles.md`).
* **A body has no wrap width of its own.** The editor wraps to the element's width, so narrowing a note
  reflows its body and nothing bounds the result: text can overflow the card.
