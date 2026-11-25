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
| `PROSE-UI-258` | `OBS` | The selection MUST report to the context panel a reading of each value it edits, and a value two parts of the selection disagree on MUST read mixed. | this spec section 3; `CHROME-210` | `ambre_prose::logic::selection::tests::the_reading_reports_each_value_and_mixed_where_parts_disagree`; `ambre_prose::tests::chrome_panel::with_a_selection_the_panel_shows_each_text_and_paragraph_control` -- wasm tier, each control reads its value; `ambre_prose::tests::chrome_panel::a_mixed_paragraph_reads_mixed_in_the_panel` -- wasm tier | none | verified |
| `PROSE-UI-262` | `OBS` | The bar MUST show, in order, bold, italic, underline, the link, a paragraph style list and a comment control; a comment from it MUST open a thread over the part of the selection in its first block. | this spec section 3; `CHROME-208` | `ambre_prose::tests::chrome_ui::the_format_bar_follows_the_mockup_and_opens_its_fields` -- wasm tier, the order of the controls, the list and the two fields; `ambre_prose::input_tests::a_comment_from_the_bar_anchors_in_the_first_block_of_the_selection` | none | verified |
| `PROSE-UI-263` | `OBS` | A quote MUST be laid out italic and set in from the left, unless the paragraph names a left indent of its own. | this spec section 3; `ambre-spec-prose-schema.md section 5` | `ambre_prose::logic::selection::tests::a_quote_is_laid_out_italic_and_set_in` | none | verified |
| `PROSE-UI-253` | `OBS` | With no text selected, the context panel MUST show the page format, the orientation and the four margins the document holds. | `ambre-spec-prose-schema.md section 3`; `CHROME-163` | `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_page_the_layout_holds` -- wasm tier, the paper, the orientation and the four margins of a landscape page; `ambre_prose::logic::bands::tests::a_page_write_sets_the_whole_format_and_one_undo_takes_it_back` -- the layout carries the format the panel reads | none | verified |

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
| `PROSE-UI-188` | `POST` | The link control MUST apply a link to the whole selection, or remove the one it carries. | this spec section 3 | `ambre_prose::logic::selection::tests::a_link_covers_the_selection_and_none_takes_it_off`; `ambre_prose::input_tests::each_panel_write_is_one_command_over_the_selection_and_one_undo_item` -- an address of blanks takes the link off; `ambre_prose::tests::chrome_ui::the_format_bar_follows_the_mockup_and_opens_its_fields` -- wasm tier, the field of the bar hands the address to the view | none | verified |
| `PROSE-UI-189` | `POST` | A heading or quote control MUST set its style on every block the selection covers. | this spec section 3; `PROSE-UI-102` | `ambre_prose::logic::selection::tests::a_heading_a_quote_and_the_body_land_on_every_block`; `ambre_prose::tests::chrome_panel::with_a_selection_the_panel_shows_each_text_and_paragraph_control` -- wasm tier, the quote chip hands its kind to the view | none | verified |
| `PROSE-UI-190` | `POST` | A colour or size control MUST apply its value to the whole selection, and a value the registry refuses MUST be shown to the author. | this spec section 3; `PROSE-FEAT-022` | `ambre_prose::logic::selection::tests::a_colour_a_highlight_and_a_size_reach_the_whole_selection` -- a size past the cap is refused, and the document is left as it was | none | verified |
| `PROSE-UI-259` | `POST` | An alignment, line spacing or indent control MUST write its property on every block the selection covers, and one undo MUST take it back. | this spec section 3; `CHROME-173` | `ambre_prose::logic::selection::tests::a_paragraph_property_over_two_blocks_is_one_undo_item`; `ambre_prose::tests::chrome_panel::with_a_selection_the_panel_shows_each_text_and_paragraph_control` -- wasm tier, the alignment and the left indent each hand one paragraph write | none | verified |
| `PROSE-UI-260` | `POST` | A list control MUST turn every paragraph of the selection into a list item, and the body control MUST turn each back, its text kept. | this spec section 3; `features/ambre-spec-prose-lists.md` | `ambre_prose::logic::selection::tests::a_selection_becomes_a_list_and_back` -- each direction is one undo item; `ambre_prose::tests::chrome_panel::with_a_selection_the_panel_shows_each_text_and_paragraph_control` -- wasm tier, the numbered list hands its kind to the view | none | verified |
| `PROSE-UI-264` | `FAIL` | A paragraph style the tree cannot hold where one block of the selection sits MUST refuse the whole command, and MUST write nothing. | this spec section 3; `PROSE-095` | `ambre_prose::logic::selection::tests::a_heading_inside_a_list_item_refuses_the_whole_command` | none | verified |
| `PROSE-UI-265` | `POST` | Each write of the context panel MUST reach the document as one command over the selection the view holds. | this spec section 3; `PROSE-UI-098`; `CHROME-173` | `ambre_prose::input_tests::each_panel_write_is_one_command_over_the_selection_and_one_undo_item` -- each of the eight writes, and one undo restores the document | none | verified |
| `PROSE-UI-261` | `POST` | With no text selected, the context panel MUST show the body font of the document, and a pick MUST write it, one undo taking it back. | `ambre-spec-prose-schema.md section 3`; `CHROME-163` | `ambre_prose::logic::selection::tests::the_body_font_is_written_and_reaches_every_run` -- the layout carries the font the panel reads; `ambre_prose::tests::chrome_panel::with_no_selection_the_panel_shows_the_default_fonts` -- wasm tier | none | verified |
| `PROSE-UI-266` | `POST` | With no text selected, the context panel MUST show the heading font of the document, a pick MUST write it, and every heading MUST be set in it. | `ambre-spec-prose-schema.md section 3`; `PROSE-UI-261` | `ambre_prose::logic::selection::tests::the_heading_font_reaches_every_heading_and_no_body_run` -- each of the six levels takes the font and the shaper sets the heading in it, a body run keeps the body font, an `ff` mark still wins, the Body font entry deletes the key, and one undo takes each write back; `ambre_prose::tests::chrome_panel::the_heading_font_picker_names_the_body_font_and_hands_each_pick` -- wasm tier, the picker names the Body font while the key is absent and hands each pick to the view | none | verified |
| `PROSE-UI-191` | `POST` | Superscript, subscript and clearing the formatting MUST each reach a control of the context panel. | `ambre-spec-prose-schema.md section 3`; `CHROME-164` | `ambre_prose::tests::chrome_panel::superscript_subscript_and_clearing_reach_a_control_of_the_panel` -- wasm tier, the three controls show over a selection and each hands its id to the view; `ambre_prose::logic::marks::tests::superscript_from_the_panel_applies_and_reads_back_on`; `ambre_prose::logic::marks::tests::clearing_a_selection_over_two_blocks_is_one_undo_item` | none | verified |
| `PROSE-UI-252` | `POST` | A page control of the context panel MUST write the format, the orientation or a margin of the document, and one undo MUST take that write back. | `ambre-spec-prose-schema.md section 3`; `CHROME-163`; `CHROME-203` | `ambre_prose::tests::chrome_panel::a_page_control_hands_the_whole_format_to_the_view` -- wasm tier, a margin stepper and the orientation each hand the whole format, one field changed; `ambre_prose::logic::bands::tests::a_page_write_sets_the_whole_format_and_one_undo_takes_it_back` -- the command writes the format, the layout is measured on it, one undo restores the page | none | verified |
| `PROSE-UI-254` | `POST` | Clearing the formatting MUST remove every formatting mark over the whole selection, and one undo MUST take it back, whatever the number of blocks. | `ambre-spec-prose-schema.md section 3`; `PROSE-UI-191` | `ambre_prose::logic::marks::tests::clearing_a_selection_over_two_blocks_is_one_undo_item` -- bold and italic over two blocks clear in one command, and one undo puts both back | none | verified |

### 1.5 Indent and leave a list over a selection

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-277` | `FAIL` | A key the value controls do not write MUST be refused before any block of the selection is written. | this spec section 3; `PROSE-UI-190` | `ambre_prose::logic::selection::tests::the_value_command_refuses_a_presence_mark_and_an_anchor` -- a presence key and an anchor key, and no block carries either | none | verified |
| `PROSE-UI-278` | `POST` | Leaving a list MUST keep the text and the order of every block of the selection, nested items included. | this spec section 3; `PROSE-UI-260`; `features/ambre-spec-prose-lists.md` | `ambre_prose::logic::selection::tests::leaving_a_nested_list_keeps_every_block_in_its_order` -- three items, two nested, all to the body in their order, and one undo | none | verified |
| `PROSE-UI-279` | `POST` | A paragraph style control MUST leave no block inside a paragraph or a heading. | this spec section 3; `PROSE-095`; `PROSE-UI-278` | `ambre_prose::logic::selection::tests::a_heading_from_an_item_takes_its_items_out` -- the nested item follows its heading; `ambre_prose::logic::selection::tests::leaving_a_nested_list_keeps_every_block_in_its_order` -- every block of a nested list lands at the top | none | verified |
| `PROSE-UI-267` | `POST` | An indent control MUST move every list item of the selection one level, and every other block one step of left indent, never left of the margin. | this spec section 3; `PROSE-FEAT-142`; `PROSE-FEAT-145` | `ambre_prose::logic::selection::tests::an_indent_moves_each_block_one_step`; `ambre_prose::input_tests::each_panel_write_is_one_command_over_the_selection_and_one_undo_item` -- the step in is one command and one undo item; `ambre_prose::tests::chrome_panel::with_a_selection_the_panel_shows_each_text_and_paragraph_control` -- wasm tier, the step in hands its step to the view | none | verified |
| `PROSE-UI-271` | `FRAME` | A write of the context panel MUST NOT change the text of the selection, or a property its control does not name. | derived; `PROSE-FEAT-179`; `PROSE-FEAT-183` | `ambre_prose::logic::selection::tests::a_text_section_write_leaves_the_text_and_every_other_property` -- a size and an alignment over two blocks leave the text, the bold and the heading level | none | verified |

* **Budget.** none of its own. The bar is repositioned inside the frame that paints the page, the
  row of `ambre-spec-budgets.md section 2`. A static element over the canvas costs a compositor
  recomposition and no layout pass (`ambre-spec-ui.md section 4`).
* **Boundary.** none: the bar reads view state and emits commands. It holds no key, opens no
  socket, and every mutation it starts is a `ProseCommand` (`PROSE-UI-002`).
* **Verification.** 36 `verified`. Every row runs on the host against the view state and the Logic
  thread, with no browser. The wasm tier adds the bar and the panel elements: their controls, their
  order and the value each hands to the view. The colours and sizes the panel writes do not reach
  the glyphs yet, which `PROSE-UI-210` records.
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
| the link control | opens the address field; Enter sends `MarkSelection` with the `a` key, or with no value to take the link off |
| the font, size, colour and highlight controls of the panel | `MarkSelection`, with the `ff`, `fs`, `fg` or `bg` key |
| the alignment, line spacing and indent controls of the panel | `SetSelectionParagraph`, naming the one property |
| the paragraph style list, the heading, quote, body and list chips | `SetSelectionKind`, on every paragraph, heading and list item the selection covers |
| the two indent steps of the panel | `IndentSelection`: a list item changes level, another block moves 36 pt of left indent (`PROSE-UI-267`) |
| the comment control | opens the comment field; Enter sends `AddComment` over the part of the selection in its first block |
| the body font and the heading font, with no selection | `SetDefaultFont`, naming the font; the Body font entry of the heading picker sends no family, which deletes the key |

Each of these commands writes every block it reaches, then commits once, so one undo takes the
gesture back (`CHROME-173`). The view adds the selection it holds to what a panel control asks
for: the panel holds no selection of its own (`PROSE-UI-265`).

**The reading.** `ReadMarks` answers with `SelectionValues` beside the mark states. It reads the
font, the size, the colour and the highlight. It reads the alignment, the line spacing, the two
indents and the paragraph style too. Each is a value or mixed (`PROSE-UI-258`). An absent mark
reads as no value: the block or the document decides.

**The blocks a paragraph control reaches.** A selection that ends at the start of a block selects
no character of it. The paragraph, kind and indent commands leave that block out, and so does the
reading. A list item that leaves its list takes its own items out after it, in their order. Only a
list item holds a block (`PROSE-UI-278`, `PROSE-UI-279`).

**A quote is a paragraph style.** The quote control writes the built-in `quote` style on a
paragraph, never a block type (`ambre-spec-prose-schema.md section 5`). The layout sets it italic
and in from the left by 24 points, unless the paragraph names a left indent (`PROSE-UI-263`).

**The bar holds nothing.** `ToolbarState` is view state, rebuilt from the selection and from what
the Logic thread reports about it. The bar never remembers a mark across a selection change
(`PROSE-UI-097`).

**Code:** `crates/ambre-prose/src/view/toolbar.rs` for the state and the commands a control sends,
`crates/ambre-prose/src/logic/marks.rs` for the fold. `crates/ambre-prose/src/logic/selection.rs`
holds the writes over a selection, and `ProseFormatBar` in `crates/ambre-prose/src/chrome/mod.rs`
the element. The bar shows bold, italic, underline, the link, the paragraph style list and
the comment (`PROSE-UI-262`). Strike-through, inline code and the colours live in the panel.

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

* **No control opens a suggestion.** The comment control opens a thread
  (`PROSE-UI-262`); a suggestion is still started by the mode alone
  (`features/ambre-spec-prose-track-changes.md section 6`).
* **A size, font or colour mark does not reach the glyphs yet** (`PROSE-UI-210`,
  `features/ambre-spec-prose-page-rendering.md`). The panel writes and reads each one; the
  shaper draws bold, italic, underline and strike-through alone.
* **The anchor has no rule when the selection leaves the window.** A selection scrolled off screen
  keeps an anchor with no pixels under it. Nothing says whether the bar hides, sticks to the
  edge or follows the anchor out of view.
