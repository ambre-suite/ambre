# AMBRE -- FEATURE: THE CONTEXTUAL TOOLBAR

**Type:** Vertical (feature)

> Select a few words and a small bar appears above them. It shows what the selection already is --
> bold, italic, a heading -- and lets the author change it.

The bar shows a state and sends a command; it owns neither. What a mark means is
`features/ambre-spec-prose-inline-styles.md`, what a heading level is
`features/ambre-spec-prose-headings.md`, and where the selection is
`features/ambre-spec-prose-selection.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-054` | `OBS` | Each control MUST report the state of the whole selection. | this spec section 3 | `ambre_prose::input_tests::a_selection_bold_in_part_reads_mixed` | none | verified |
| `PROSE-UI-096` | `OBS` | A selection that is formatted in part MUST be reported as mixed, never as the state of either end. | this spec section 3 | `ambre_prose::input_tests::a_selection_bold_in_part_reads_mixed`; `ambre_prose::tests::chrome_ui::the_format_bar_follows_its_anchor_and_reports_each_mark` -- wasm tier: a mixed mark reads `aria-pressed="mixed"` | none | verified |
| `PROSE-UI-055` | `OBS` | The bar MUST stay over the text it was raised on while the page scrolls or zooms. | this spec section 3 | `ambre_prose::input_tests::the_bar_follows_its_text_through_a_scroll_and_a_zoom` | none | verified |
| `PROSE-UI-057` | `INV` | The bar MUST NOT hold document content: the state it renders MUST arrive from the Logic thread with the selection it describes. | this spec section 3; `PROSE-UI-001` | `ambre_prose::input_tests::the_marks_shown_are_the_ones_read_for_this_selection` | none | verified |
| `PROSE-UI-097` | `INV` | The state a control shows MUST be rebuilt from the current selection; a mark MUST NOT be carried over from the selection before it. | this spec section 5 | `ambre_prose::input_tests::the_marks_shown_are_the_ones_read_for_this_selection` | none | verified |
| `PROSE-UI-056` | `INV` | A control and the keyboard chord for the same formatting MUST resolve to one command id. | this spec section 3; `ambre-spec-chrome.md` | `ambre_prose::commands::tests::a_control_and_its_chord_resolve_to_one_command_id`; `ambre_belt::prose::tests::a_chord_and_its_control_toggle_one_mark` -- browser tier: `Ctrl+B` bolds a selected word, and the Bold control then takes the same mark off; `ambre_app::platform::tests::a_modifier_chord_on_a_typing_pipe_is_the_pages` -- wasm tier: a chord with the modifier on the page's pipe reaches the keymap, and a chrome field keeps it | none | verified |
| `PROSE-UI-098` | `INV` | The bar MUST NOT have a path into the document of its own: every control MUST reach it through a command the plane already declares. | this spec section 5 | `ambre_prose::input_tests::a_mixed_control_applies_its_mark_to_the_whole_selection`; `ambre_prose::tests::chrome_ui::a_control_press_hands_its_command_back_and_keeps_the_focus` -- wasm tier: a click on each control writes its declared command id, and its press keeps the focus | none | verified |
| `PROSE-UI-192` | `INV` | One gesture on a control MUST be taken back by one undo, whatever the number of blocks it covers. | derived | `ambre_prose::input_tests::a_mixed_control_applies_its_mark_to_the_whole_selection` | none | verified |
| `PROSE-UI-253` | `OBS` | With no text selected, the context panel MUST show the page format, the orientation and the four margins the document holds. | `ambre-spec-prose-schema.md section 3`; `CHROME-163` | `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds` -- wasm tier, the paper, the orientation and the four margins of a landscape page; `ambre_prose::logic::bands::tests::a_page_write_sets_the_whole_format_and_one_undo_takes_it_back` -- the layout carries the format the panel reads | none | verified |
| `PROSE-UI-269` | `OBS` | The selection MUST report one reading per value control to the panel: the value every selected run or block holds, or mixed where two disagree. | this spec section 3; `CHROME-210`; `PROSE-UI-057` | `ambre_prose::logic::formatting::tests::the_selection_reads_each_value_uniform_or_mixed` -- the colour and the alignment, over one block and over two; `ambre_prose::input_tests::a_text_section_write_is_one_command_over_the_selection` -- the view shows the size and the alignment it wrote | none | verified |
| `PROSE-UI-270` | `OBS` | Over a selection, the text section MUST show a control for the font, the size, the text colour, the highlight, the paragraph style, the alignment, the list, the indent and the link, and each MUST hand its write to the view. | this spec section 3; `CHROME-164`; `CHROME-219` | `ambre_prose::tests::chrome_panel::the_text_section_shows_each_control_and_hands_its_write` -- wasm tier: every control carries its marker, no native control shows, the readings show, and the list, alignment, indent and link each hand one write | none | verified |

### 1.2 Raise the bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-053` | `POST` | The bar MUST appear only over a non-empty selection. | this spec section 3 | `ambre_prose::input_tests::the_bar_shows_over_a_selection_and_goes_with_it`; `ambre_prose::tests::chrome_ui::the_format_bar_follows_its_anchor_and_reports_each_mark` -- wasm tier: the bar element is on the page only while the view gives it an anchor | none | verified |
| `PROSE-UI-099` | `FRAME` | Raising the bar MUST NOT write to the document, and MUST NOT move or shrink the selection it was raised on. | derived | `ambre_prose::input_tests::raising_the_bar_writes_nothing_and_keeps_the_selection` | none | verified |

### 1.3 Dismiss the bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-100` | `POST` | The bar MUST disappear when the selection is emptied or cancelled. | this spec section 3 | `ambre_prose::input_tests::the_bar_shows_over_a_selection_and_goes_with_it` | none | verified |
| `PROSE-UI-101` | `FRAME` | Dismissing the bar MUST NOT write to the document: a formatting applied before it closed MUST stay applied. | derived | `ambre_prose::input_tests::dismissing_the_bar_writes_nothing` | none | verified |

### 1.4 Format from a control

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-102` | `POST` | A control MUST apply or remove its formatting over the whole selection. | this spec section 4 | `ambre_prose::input_tests::a_mixed_control_applies_its_mark_to_the_whole_selection` | none | verified |
| `PROSE-UI-103` | `POST` | A control reporting mixed MUST apply its formatting to the whole selection rather than remove it. | this spec section 3 | `ambre_prose::input_tests::a_mixed_control_applies_its_mark_to_the_whole_selection` | none | verified |
| `PROSE-UI-104` | `FRAME` | A control MUST NOT change the text of the selection, its extent, or any formatting the control does not name. | derived | `ambre_prose::input_tests::a_control_leaves_the_text_the_selection_and_other_marks` | none | verified |
| `PROSE-UI-188` | `POST` | The link control MUST apply a link to the whole selection, or remove the one it carries. | this spec section 3 | not built | unbuilt -- lands with the `The format bar's other controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-189` | `POST` | A heading or quote control MUST set its style on every block the selection covers. | this spec section 3; `PROSE-UI-102` | not built | unbuilt -- lands with the `The format bar's other controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-190` | `POST` | A colour or size control MUST apply its value to the whole selection, and a value the registry refuses MUST be shown to the author. | this spec section 3; `PROSE-FEAT-022` | not built | unbuilt -- lands with the `The format bar's other controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-191` | `POST` | Superscript, subscript and clearing the formatting MUST each reach a control of the context panel. | `ambre-spec-prose-schema.md section 3`; `CHROME-164` | `ambre_prose::tests::chrome_panel::superscript_subscript_and_clearing_reach_a_control_of_the_panel` -- wasm tier, the three controls show over a selection and each hands its id to the view; `ambre_prose::logic::marks::tests::superscript_from_the_panel_applies_and_reads_back_on`; `ambre_prose::logic::marks::tests::clearing_a_selection_over_two_blocks_is_one_undo_item` | none | verified |
| `PROSE-UI-252` | `POST` | A page control of the context panel MUST write the format, the orientation or a margin of the document, and one undo MUST take that write back. | `ambre-spec-prose-schema.md section 3`; `CHROME-163`; `CHROME-203` | `ambre_prose::tests::chrome_panel::a_page_control_hands_the_whole_format_to_the_view` -- wasm tier, a margin stepper and the orientation each hand the whole format, one field changed; `ambre_prose::logic::bands::tests::a_page_write_sets_the_whole_format_and_one_undo_takes_it_back` -- the command writes the format, the layout is measured on it, one undo restores the page | none | verified |
| `PROSE-UI-254` | `POST` | Clearing the formatting MUST remove every formatting mark over the whole selection, and one undo MUST take it back, whatever the number of blocks. | `ambre-spec-prose-schema.md section 3`; `PROSE-UI-191` | `ambre_prose::logic::marks::tests::clearing_a_selection_over_two_blocks_is_one_undo_item` -- bold and italic over two blocks clear in one command, and one undo puts both back | none | verified |

### 1.5 Format from the text section of the panel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-263` | `POST` | A font, size, text colour or highlight control MUST write its value over every block the selection covers in one command, and one undo MUST take it back. | this spec section 3; `CHROME-173` | `ambre_prose::logic::formatting::tests::a_value_over_two_blocks_is_one_undo_item` -- each of the four keys over two blocks, then one undo; `ambre_prose::input_tests::a_text_section_write_is_one_command_over_the_selection` -- the view sends one command for a write over two blocks | none | verified |
| `PROSE-UI-264` | `FAIL` | A value the registry refuses, or a key the value controls do not write, MUST be refused before any block of the selection is written. | this spec section 3; `PROSE-FEAT-022` | `ambre_prose::logic::formatting::tests::a_refused_value_writes_no_block_of_the_selection` -- a size sent under the colour key, a presence key and an anchor key | none | verified |
| `PROSE-UI-265` | `POST` | A paragraph style or an alignment control MUST set its property on every block the selection covers in one command, and one undo MUST take it back. | this spec section 3; `CHROME-173`; `PROSE-FEAT-025` | `ambre_prose::logic::formatting::tests::a_block_property_over_two_blocks_is_one_undo_item` -- an alignment and a heading level over two blocks; `ambre_prose::input_tests::a_text_section_write_is_one_command_over_the_selection` | none | verified |
| `PROSE-UI-266` | `POST` | The list control MUST turn every paragraph of the selection into a list item, and every item back into a paragraph, keeping the text and the order of the blocks. | this spec section 3; `features/ambre-spec-prose-lists.md` | `ambre_prose::logic::formatting::tests::a_paragraph_becomes_a_list_item_and_back` -- three paragraphs to bullets, two nested, then all out of the list in their order, and one undo | none | verified |
| `PROSE-UI-267` | `POST` | An indent control MUST move every list item of the selection one level, and every other block one step of left indent, never left of the margin. | this spec section 3; `PROSE-FEAT-142`; `PROSE-FEAT-145` | `ambre_prose::logic::formatting::tests::an_indent_moves_each_block_one_step` | none | verified |
| `PROSE-UI-268` | `POST` | The link field MUST put one link over the whole selection, or take it off when the field is emptied, and one undo MUST take the write back. | this spec section 3; `PROSE-UI-188` | `ambre_prose::logic::formatting::tests::a_selection_takes_a_link_and_gives_it_back`; `ambre_prose::input_tests::a_blank_link_takes_the_link_off_and_no_selection_sends_nothing` | none | verified |
| `PROSE-UI-271` | `FRAME` | A write of the text section MUST NOT change the text of the selection, or a property its control does not name. | derived; `PROSE-FEAT-179`; `PROSE-FEAT-183` | `ambre_prose::logic::formatting::tests::a_text_section_write_leaves_the_text_and_every_other_property` -- a size and an alignment over two blocks leave the text, the bold and the heading level | none | verified |

* **Budget.** none of its own. The bar is repositioned inside the frame that paints the page, the
  row of `ambre-spec-budgets.md section 2`. A static element over the canvas costs a compositor
  recomposition and no layout pass (`ambre-spec-ui.md section 4`).
* **Boundary.** none: the bar reads view state and emits commands. It holds no key, opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 28 `verified`, 3 `unimplemented`. Every built row runs on the host against the
  view state and the Logic thread, with no browser. The four open rows are the controls past the
  five marks. The DOM element itself renders `ToolbarState` and holds no rule of its own.
---

## 2. WHAT EACH USER SEES

The bar is local: it is raised by a selection, and a selection stays on the peer that made it
(`PROSE-UI-010`). What converges is what its controls write. The last column names the test, or
the row a scenario rests on when no test resolves it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| selects a sentence | selects nothing | nothing: a selection is local | a bar over Alice's sentence, and no bar for Bob | `ambre_prose::input_tests::the_bar_shows_over_a_selection_and_goes_with_it` |
| makes the sentence bold | reads it | the bold mark over the range | the bold mark in both documents, the control lit for Alice | `ambre_prose::input_tests::a_mixed_control_applies_its_mark_to_the_whole_selection` |
| selects a range half of which is bold | -- | nothing | the bold control mixed, neither on nor off | `ambre_prose::input_tests::a_selection_bold_in_part_reads_mixed` |
| makes it bold | makes the same range italic at the same moment | both marks, neither lost | both marks in both documents | none -- `PROSE-FEAT-008` |
| scrolls while the bar is open | -- | nothing | the bar still over the same words | `ambre_prose::input_tests::the_bar_follows_its_text_through_a_scroll_and_a_zoom` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the marks of the selected range and writes them
through the commands of `features/ambre-spec-prose-inline-styles.md`.

**The state the view publishes.** The bar is a DOM element, and the canvas view is what knows where
the selection is. The view publishes a state and the element renders it
(`ambre-spec-ui.md section 2`, Canvas to DOM by signal).

```rust
// ambre-prose/src/view/toolbar.rs -- published to the chrome component, never drawn on the canvas

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarState {
    /// The top-centre of the selection's first line, in view pixels. `None` hides the bar.
    pub anchor: Option<Point<Px<View>>>,
    /// `None` until the Logic thread answered for this very selection.
    pub active: Option<ActiveMarks>,
}

// ambre-prose/src/message/mod.rs -- read by `ProseCommand::ReadMarks`,
// answered by `ProseEvent::SelectionMarks` with the selection it describes

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveMarks {
    pub bold:          TriState,
    pub italic:        TriState,
    pub underline:     TriState,
    pub strikethrough: TriState,
    pub code:          TriState,
    /// The address, when the whole selection carries one link and the same one.
    pub link:          Option<LinkUrl>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TriState { #[default] Off, On, Mixed }
```

**Three states, not two.** A selection covering formatted and unformatted text is neither on nor
off. Reporting it as either makes the next click do the opposite of what the author expects
(`PROSE-UI-096`). `Mixed` is what a control shows, and a click on a mixed control applies the mark
to the whole range rather than removing it (`PROSE-UI-103`).

**Steps, on every selection change:**

1. the view reads the new selection; an empty one hides the bar and stops (`PROSE-UI-053`);
2. it sends `ReadMarks` for the range. The Logic thread folds its marks into `ActiveMarks`, one
   `TriState` per control, and answers with the range (`PROSE-UI-054`);
3. it computes the anchor rectangle from the selection's geometry and raises the signal;
4. the element positions itself against the anchor, and follows it each frame while it is visible
   (`PROSE-UI-055`).

**What a control sends.** A control invokes a command id of `ambre-commands`, the same id the
keyboard chord invokes. That id resolves to one command of the plane
(`features/ambre-spec-prose-engine.md section 3`). There is no toolbar-only path into the document
(`PROSE-UI-098`).

| Control | Command |
|---|---|
| bold, italic, underline, strikethrough, inline code | `FormatSelection`, on the selected range, in one commit |
| the link control | `ApplyMark` with the `a` key, or `RemoveMark` with it |
| the heading controls | `SetHeadingLevel`, on every block the selection covers |
| the quote control | `SetBlockStyle`, with the built-in `quote` style |
| the overflow control | opens the colour and size controls, which write through `ApplyMark` |

**What the text section sends.** A value or block control of the panel hands the view one typed
write. The view turns it into one command over the selection, so one undo takes the gesture back
(`CHROME-173`).

| Control | Command |
|---|---|
| the font, the size, the text colour, the highlight | `MarkSelection` with the key and its value, or none to take the key off |
| the link field | `MarkSelection` with the `a` key; an emptied field takes the link off |
| the paragraph style | `FormatBlocks` with a heading level, or none for body text |
| the alignment | `FormatBlocks` with the paragraph settings, the alignment alone named |
| the list | `FormatBlocks` with a marker, or none to leave the list |
| the indent | `FormatBlocks`: a list item changes level, another block moves 36 pt of left indent |

`ActiveMarks` carries one `Reading` per value control beside the `TriState` of each mark. The
Logic thread folds the style the cascade resolves for each selected run, and the block properties
of each selected block (`PROSE-UI-269`). A selection that ends at the start of a block selects no
character of it. That block is left out of both.

**The bar holds nothing.** `ToolbarState` is view state, rebuilt from the selection and from what
the Logic thread reports about it. The bar never remembers a mark across a selection change
(`PROSE-UI-097`).

**Code:** `crates/ambre-prose/src/view/toolbar.rs` holds the state and the commands a control
sends. `crates/ambre-prose/src/logic/marks.rs` holds the fold, and
`crates/ambre-prose/src/logic/formatting.rs` the writes of the text section. `ProseFormatBar` in
`crates/ambre-prose/src/chrome/mod.rs` is the element. The five controls built are the marks;
the link, heading, quote and overflow controls are not.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| a pointer release leaving a non-empty selection | the bar appears above it |
| a keyboard selection reaching a non-empty range | the same |
| the selection is emptied, or a click lands outside it | the bar disappears |
| a control | applies or removes the formatting on the whole selection |
| the overflow control | reveals the controls the bar has no room for |
| a narrow window | the bar becomes a bottom sheet (`ambre-spec-adaptive-layout.md`) |

The element, its keyboard order and its dismissal are the shared chrome toolkit's, and are not
respecified here (`ambre-spec-chrome.md`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The bar is a DOM element over the canvas | drawing the bar on the `DrawSurface`, beside the text it formats | the canvas owns content and the DOM owns everything else (`UI-THREAD-001`), and a DOM element inherits keyboard order, focus and a screen-reader name from the browser. A canvas bar reimplements all three and is still not reachable by a screen reader |
| The view computes the anchor, the element follows it | letting the element hit-test the document | it puts a second geometry model in the chrome. Only the canvas knows where a selection is on screen and only the DOM can place an element, which is the boundary the framework already draws |
| A mixed selection is a third state | reporting the state of the selection's first character | it is what a two-state control does, and why it toggles the wrong way: the control lies about half the selections a user makes, and the lie decides what the next click does. Three states cost one enum variant |
| A control invokes a command id, never a command | wiring each control straight to a `ProseCommand` | it duplicates the mapping once per surface. The chord, the menu and the bar are three surfaces for one operation, and one id keeps them one operation and the keymap remappable |
| The bar carries no document state | caching the marks of the last selection to avoid a fold | it makes the bar wrong after any concurrent change. The UI thread holds no document (`PROSE-UI-001`), and a remembered mark is a second truth a remote edit puts out of step |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No control opens a comment or a suggestion.** Both features record the same absence, and the
  bar is where a selection-scoped gesture would live
  (`features/ambre-spec-prose-comments.md section 6`,
  `features/ambre-spec-prose-track-changes.md section 6`).
* **The anchor has no rule when the selection leaves the window.** A selection scrolled off screen
  keeps an anchor with no pixels under it. Nothing says whether the bar hides, sticks to the
  edge or follows the anchor out of view.
