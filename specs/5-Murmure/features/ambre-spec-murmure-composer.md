# AMBRE -- FEATURE: THE MURMURE COMPOSER
**Type:** Vertical (feature)
**Implementation:** partial -- a plain-text composer at the foot of the channel's region.

> The field where a member writes. It takes light Markdown as it is typed, mentions and emoji, with
> no toolbar in the way. Enter sends, and each channel keeps its own draft.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-193` | `INV` | The composer MUST hold its marks as a layer over the inline editor of the canvas, restricted to the Murmure mark set. | this spec section 3; `CANVAS-076` | `ambre_murmure::view::tests::a_chord_puts_its_style_on_the_selection` -- a style is a mark over the draft, and the message sent carries it | none | verified |
| `MURMURE-194` | `INV` | The Murmure mark set MUST be bold, italic, inline code, code block, link, list, quote and mention. | this spec section 3; `MURMURE-008` | `ambre_murmure::style_tests::a_block_mark_travels_with_the_body` -- a code block, a quote and a list are marks a second peer reads, each under its key, and text at a block's edge stays out; `ambre_murmure::style_tests::a_style_is_a_mark_on_the_body_that_travels_with_it` -- and the inline styles | none | verified |
| `MURMURE-561` | `INV` | A link MUST hold an `http` or `https` address: the engine refuses another one, and reads one stored as text. | this spec section 3; `MURMURE-562` | `ambre_murmure::style_tests::a_style_outside_the_body_or_a_bad_link_is_refused` -- a send, an edit and a reply with another scheme are refused; `::a_link_to_another_scheme_reads_as_text` -- a stored `javascript:` link reads as text | none | verified |
| `MURMURE-562` | `INV` | Each style MUST be a mark on the body beside the mentions, carried through an edit, an amendment, a fold and a relocation. | this spec section 3; `ambre-spec-murmure-schema.md` | `ambre_murmure::style_tests::a_style_is_a_mark_on_the_body_that_travels_with_it` -- each style is a mark a second peer reads; `::runs_of_one_style_read_as_one` -- touching runs read as one; `::an_edit_rewrites_the_styles_of_the_body` -- an edit replaces them; `ambre_murmure::amending_tests::a_member_reads_the_amendments_over_the_segment` -- an amendment carries them; `ambre_murmure::amending_tests::a_change_refused_by_the_lock_is_replayed` -- and its replay; `ambre_murmure::folding_tests::the_lease_holder_folds_the_amendments_into_the_segment` -- a fold keeps them; `ambre_murmure::closing_tests::a_message_refused_by_the_lock_moves_to_the_active_segment` -- a relocated message keeps them; `ambre_murmure::view::tests::an_edit_of_the_styles_alone_is_sent` -- an edit of the styles alone is sent | none | verified |
| `MURMURE-564` | `OBS` | The composer MUST draw each style of its draft as it is typed. | this spec section 3; `MURMURE-193` | `ambre_murmure::view::tests::the_field_draws_each_mark_as_it_is_typed` -- markers typed past a wide char give a bold span, a code span on its tint and an underlined link, each at its byte range, and a draft with no mark draws none; `ambre_murmure::view::tests::a_block_marker_becomes_its_mark_and_the_field_draws_it` -- a quote's rule and a list's dot in the margin, a code block in the code face on its tint | none | verified |
| `MURMURE-570` | `OBS` | A quote and a list MUST draw their lines past their margin mark, in the composer and in the list. | this spec section 3; `MURMURE-564`; `MURMURE-565` | not built | unbuilt -- lands with the `Block marks indented` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-195` | `INV` | A draft MUST be kept per channel while the user moves between channels. | this spec section 3 | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` -- a draft typed in one channel is there when the user comes back from another | none | verified |
| `MURMURE-525` | `INV` | A draft MUST be kept per thread, apart from the draft of its channel. | this spec section 3; `MURMURE-195` | `ambre_murmure::view::tests::a_thread_keeps_its_own_draft_apart_from_the_channels` -- the thread's draft is not the channel's, and another thread opens on an empty one | none | verified |
| `MURMURE-530` | `INV` | A thread's draft MUST survive the thread closing and opening again. | this spec section 3; `MURMURE-525` | `ambre_murmure::view::tests::a_threads_draft_survives_the_thread_closing` -- the region dropped and built again shows the draft, and the field does not take the keys | none | verified |
| `MURMURE-196` | `OBS` | The composer MUST show no permanent formatting toolbar. | this spec section 3 | `ambre_murmure::view::tests::the_band_holds_a_field_and_no_toolbar` -- the band paints its fill, the field and its outline, and no other shape, under a list cut to the band's top | none | verified |
| `MURMURE-210` | `INV` | A draft MUST survive a reload of the page. | this spec section 3; `ambre-spec-launcher.md` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-211` | `INV` | A draft MUST NOT leave the device before it is sent. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-214` | `INV` | Eviction from the local store MUST NOT drop a draft. | this spec section 3; `RESID-009` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-430` | `INV` | The composer MUST be canvas content, fed by the shared input pipe. | this spec section 3; `UI-THREAD-014`; `ambre-spec-text-input.md` | `ambre_murmure::view::tests::the_composer_takes_the_pipe_until_escape` -- a press in the band focuses the pipe, and its text and keys reach the draft, never the list, until Escape; `ambre_murmure::view::tests::paste_copy_and_cut_go_through_the_draft` -- a paste lands in the draft, and a copy or a cut reads its selection; `ambre_murmure::view::tests::a_tap_in_the_band_focuses_the_composer` -- a tap focuses the pipe, and a press in the list takes the focus back | none | verified |
| `MURMURE-431` | `INV` | A formatting chord of the composer MUST take the chord Prose binds to the same mark, and put the mark on the selection or take it off. | this spec section 3; `UI-THREAD-019` | `ambre_murmure::markers::tests::a_chord_names_its_style` -- `b`, `i` and `e` name bold, italic and code; `::a_chord_puts_a_style_on_or_takes_it_off` -- a chord takes off a mark covering the whole selection; `ambre_murmure::view::tests::a_chord_puts_its_style_on_the_selection` -- the chord reaches the draft, and none changes an empty selection; `ambre_app::shell::murmure_regions::a_style_takes_the_chord_prose_binds_to_its_mark` -- each chord is the one Prose binds | none | verified |
| `MURMURE-432` | `OBS` | A selection in the composer MUST show the Prose format bar, with the Murmure marks alone. | this spec section 3; `MURMURE-194` | not built | unbuilt -- lands with the `The format bar of the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-544` | `OBS` | The box under a thread's composer MUST name Alt+Enter at the desktop width, and only there. | this spec section 3; `MURMURE-534`; `CHROME-057` | `ambre_murmure::view::tests::the_box_names_its_chord_at_the_desktop_width_only` -- the desktop width adds one run of the chord's glyphs, and a narrow window drops it; `ambre_app::shell::murmure_regions::the_canvas_reads_the_desktop_width_as_the_stylesheet_does` -- the host reads a desktop at 1025 pixels and not at 1024, where the stylesheet shows and hides a chord | none | verified |

### 1.2 Type Markdown

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-197` | `POST` | An inline Markdown marker MUST become its mark as soon as it closes. | this spec section 3 | `ambre_murmure::markers::tests::each_marker_becomes_its_mark_as_it_closes` -- bold, italic, code and a link, each with its chars out; `::a_marker_that_holds_no_text_or_no_address_stays_text` -- an empty marker stays text; `ambre_murmure::view::tests::a_closed_marker_becomes_its_mark_and_an_undo_gives_it_back` -- the draft takes the mark, one undo gives the chars back, and a marker closed in the frame of Enter is sent as its mark | none | verified |
| `MURMURE-198` | `FRAME` | A marker typed inside code MUST stay text. | this spec section 3 | `ambre_murmure::markers::tests::a_marker_typed_in_code_stays_text` -- a marker touching code closes nothing; `ambre_murmure::view::tests::a_marker_typed_in_code_stays_text` -- the asterisks stay in the code span of the draft; `ambre_murmure::markers::tests::a_block_marker_typed_in_a_code_block_stays_text` -- a block marker in code or under an open fence stays text | none | verified |
| `MURMURE-563` | `POST` | A block marker at the start of a line MUST become its mark as soon as it closes. | this spec section 3; `MURMURE-194` | `ambre_murmure::markers::tests::each_block_marker_becomes_its_mark_as_it_closes` -- a quote and a list item on their line break or the send, a code block on its fence; `::a_block_marker_that_holds_no_text_stays_text` -- an empty or misplaced marker stays text; `::a_block_marker_typed_in_a_code_block_stays_text` -- and one in code or under an open fence; `ambre_murmure::view::tests::a_block_marker_becomes_its_mark_and_the_field_draws_it` -- the draft takes each mark as typed, and the send closes the last line | none | verified |

### 1.3 Send

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-199` | `POST` | Enter MUST send the draft as a message. | this spec section 3; `features/ambre-spec-murmure-messages.md` | `ambre_murmure::view::tests::enter_sends_the_draft_and_shift_enter_breaks_the_line` -- Enter sends the draft's text at the page's clock, and the draft is gone | none | verified |
| `MURMURE-200` | `POST` | Shift+Enter MUST insert a line break. | this spec section 3 | `ambre_murmure::view::tests::enter_sends_the_draft_and_shift_enter_breaks_the_line` -- Shift+Enter puts a line break in the draft and sends nothing | none | verified |
| `MURMURE-201` | `FAIL` | Given an empty draft, Enter MUST send nothing. | this spec section 3 | `ambre_murmure::view::tests::an_empty_or_blank_draft_sends_nothing` -- an empty draft, and one of white space alone, send nothing and stay | none | verified |
| `MURMURE-212` | `POST` | Sending a message MUST delete its stored draft. | this spec section 3 | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-202` | `FRAME` | Sending MUST NOT clear the draft of another channel. | this spec section 3 | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` -- a send in one channel leaves the draft of the other as it was, unsent | none | verified |
| `MURMURE-523` | `POST` | Enter in the composer of a thread MUST send the draft as a reply to the thread's root. | this spec section 3; `features/ambre-spec-murmure-threads.md` | `ambre_murmure::view::tests::enter_in_a_thread_sends_a_reply_to_its_root` -- the thread shows a composer, and Enter sends a reply to the root at the page's clock and posts nothing in the channel | none | verified |
| `MURMURE-524` | `FRAME` | Sending a reply MUST NOT clear the draft of the channel. | this spec section 3; `MURMURE-202` | `ambre_murmure::view::tests::a_thread_keeps_its_own_draft_apart_from_the_channels` -- a reply sent leaves the channel's draft as it was | none | verified |
| `MURMURE-531` | `POST` | With the box under the thread's composer checked, Enter MUST send the reply to the channel as well, then uncheck the box. | this spec section 3; `MURMURE-109` | `ambre_murmure::view::tests::the_box_sends_the_reply_to_the_channel_too_once` -- a press checks it and keeps the keys in the field, one reply asks for the copy, and the next one does not; `::a_tap_on_the_box_checks_it` -- a tap checks it and a second one unchecks it | none | verified |
| `MURMURE-534` | `POST` | Alt+Enter in the composer of a thread MUST check the box or uncheck it, and send nothing. | this spec section 3; `MURMURE-531` | `ambre_murmure::view::tests::alt_enter_turns_the_box` -- the chord checks the box and sends nothing, Enter then sends the copy, and two chords leave it unchecked; `::alt_enter_in_a_channel_sends` -- a channel's composer has no box, so the chord sends | none | verified |
| `MURMURE-535` | `POST` | A turn of the box by a key MUST be announced with its new state. | this spec section 3; `MURMURE-534` | `ambre_murmure::view::tests::a_turn_of_the_box_by_a_key_is_said` -- two chords announce the box checked, then unchecked | none | verified |

### 1.4 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-203` | `POST` | Editing MUST load the message into the composer and send an edit, never a new message. | this spec section 3; `MURMURE-084` | `ambre_murmure::view::tests::an_edit_loads_the_message_and_sends_an_edit_never_a_new_one` -- the message loads, Enter sends an edit of it and no message, and a blank or unchanged edit sends nothing; `ambre_murmure::composer::tests::an_edit_keeps_the_mentions_it_leaves_whole` -- the mentions the edit leaves whole stay; `ambre_murmure::commands::tests::only_the_author_edits_a_message_or_a_reply` -- only the author is offered the action; `ambre_murmure::tests::chrome_relay::an_edit_is_handed_to_the_region_and_sends_nothing` -- the action hands the message to the region once | none | verified |
| `MURMURE-204` | `FRAME` | Cancelling an edit MUST NOT change the message. | this spec section 3 | `ambre_murmure::view::tests::escape_cancels_an_edit_and_leaves_the_message` -- Escape ends the edit and sends nothing | none | verified |
| `MURMURE-259` | `FRAME` | Editing a message MUST NOT drop the draft of the channel: the draft MUST return once the edit is sent or cancelled. | this spec section 3; `MURMURE-195` | `ambre_murmure::view::tests::an_edit_loads_the_message_and_sends_an_edit_never_a_new_one` -- the draft returns once the edit is sent; `ambre_murmure::view::tests::escape_cancels_an_edit_and_leaves_the_message` -- and once it is cancelled | none | verified |
| `MURMURE-526` | `POST` | The edit action on an own reply MUST load it into the composer of its thread. | this spec section 3; `MURMURE-203` | `ambre_murmure::view::tests::the_edit_of_a_reply_loads_the_threads_composer` -- the channel leaves the edit, the thread's composer loads it and takes the keys, and Enter sends an edit of the reply; `ambre_murmure::commands::tests::only_the_author_edits_a_message_or_a_reply` -- a reply offers the action to its author; `ambre_murmure::tests::chrome_relay::an_edit_is_handed_to_the_region_and_sends_nothing` -- a reply goes to the thread's region alone | none | verified |

### 1.5 Lose access to a channel, or sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-215` | `POST` | Losing access to a channel MUST delete its stored draft. | this spec section 3; `SYNC-109` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-216` | `POST` | Signing out MUST delete every stored draft. | this spec section 3; `ambre-spec-session.md` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-217` | `FRAME` | Deleting the draft of one channel MUST NOT touch the draft of another. | this spec section 3 | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Undo while writing

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-433` | `POST` | An undo in the composer MUST revert the last edit of the draft. | this spec section 3; `UNDO-010` | `ambre_murmure::view::tests::undo_and_redo_walk_the_draft_a_word_at_a_time` -- a word, a run of deletions and a paste are each one step, a redo brings them back, and an edit drops the redo; `ambre_murmure::composer::tests::a_seal_closes_the_waiting_edit_and_never_the_next_one` -- a caret move closes the step and never the next | none | verified |
| `MURMURE-434` | `FRAME` | An undo in the composer MUST NOT reach a message already sent. | this spec section 3; `UNDO-001` | `ambre_murmure::view::tests::an_undo_after_a_send_reaches_no_message` -- after a send, an undo and a redo leave the draft empty and send nothing | none | verified |

* **Budget.** a keystroke holds the input rows of `ambre-spec-budgets.md section 1`, as in Prose.
* **Boundary.** none: a draft stays in the client until it is sent, and a sent message is sealed
  like any delta (`ZK-001`).
* **Verification.** 30 `verified`, 9 `unimplemented`. The host tier drives the view with the events
  the page sends: a press, a tap, text, keys, a paste. The inline marks are built, drawn in the
  field and sent. The block marks, the format bar and the stored draft are not built.

---

## 2. WHAT EACH USER SEES

| Alice does | Alice sees | Test |
|---|---|---|
| types `**done**` | "done" in bold, as she closes the marker (`MURMURE-197`) | none |
| types `**x**` inside a code span | the asterisks, as text (`MURMURE-198`) | none |
| drafts in one channel, opens another, and comes back | her draft, as she left it (`MURMURE-195`) | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` |
| presses Enter on an empty composer | nothing sent (`MURMURE-201`) | `ambre_murmure::view::tests::an_empty_or_blank_draft_sends_nothing` |

---

## 3. MODEL

**CRDT containers.** none while drafting: a draft is local text with its marks, never shared. It
is kept in the local store the launcher owns, so a reload finds it (`MURMURE-210`).

**How long a draft lives.** A draft is work the user has not sent, so no delay removes it and no
eviction drops it (`MURMURE-214`). It goes in three cases only.

| Case | The draft |
|---|---|
| the message is sent | deleted (`MURMURE-212`) |
| the user loses access to the channel: a leave, a revoke | deleted, since it names a channel the user may no longer read (`MURMURE-215`) |
| the user signs out | every draft deleted, so the next person on the device finds none (`MURMURE-216`) |

A sent message becomes a `content` text of `ambre-spec-murmure-schema.md section 3`.

**A layer of marks on the inline editor.** The composer stands on the inline editor of the canvas
crate, and holds its marks beside the text as ranges of chars (`MURMURE-193`). The set is the one
of `MURMURE-194`, and a mark outside it has no type to be written in. The composer is canvas
content. It takes its keys, its input method and its clipboard events from the pipe of
`ambre-spec-text-input.md`, like every editor of the suite (`MURMURE-430`).

**The chords of Prose, for the marks they share.** A mark Prose declares keeps the Prose chord:
`Ctrl+B` bold, `Ctrl+I` italic, `Ctrl+E` inline code (`MURMURE-431`). A chord puts the mark on the
selection, or takes it off a selection it covers whole. A selection will show the format bar of
Prose, with the controls of the set alone (`MURMURE-432`). The bar leaves with the selection, so
nothing stays on screen (`MURMURE-196`).

**A style on the body.** A sent style is a mark on the `content` text, beside the mentions
(`MURMURE-562`). An edit, an amendment, a fold and a relocation carry it. A link holds an `http`
or `https` address: the engine refuses another one, and reads one stored as text (`MURMURE-561`).

**Undo.** A draft has the local history of `ambre-spec-undo.md`. `Ctrl+Z` and `Ctrl+Shift+Z` walk
the edits of the draft and of its marks (`MURMURE-433`). Sending ends that history: a sent message
is changed by an edit or a delete, never by an undo (`MURMURE-434`).

```rust
pub enum TextStyle { Bold, Italic, Code, Link(String), CodeBlock, Quote, List }
pub struct StyleMark { pub style: TextStyle, pub range: Range<usize> }
```

**Markdown, as typed.** An inline marker closes on its second half: `**` bold, `_` italic,
a backtick code (`MURMURE-197`). A link is written `[text](address)`. The marker's chars leave
the draft, and one undo gives them back. A marker touching code stays text (`MURMURE-198`).

**Block markers, on their end** (`MURMURE-563`). `> ` and `- ` at a line's start close on the
line break or the send, three backticks on three more. A mark covers its lines and their breaks,
so a list reads as one. The margin holds a quote's rule and a list's dots (`MURMURE-564`).

**A mark through an edit.** Each mark follows its text as the draft changes. Text typed at either
end of a mark stays out of it, as in most chat editors. A chord or a marker typed in the frame of
Enter lands before the draft leaves.

**The slice.** Each channel holds one draft, in memory. The next paint applies the keys, so Enter
only asks for the send. Each flush pins the marks on the field as spans, so each mark is drawn
as it is typed (`MURMURE-564`).

**A reply in the slice.** The thread's region has a composer of its own, and each thread holds a
draft apart from its channel's (`MURMURE-525`). Enter there sends a reply to the root
(`MURMURE-523`). The page has one pipe, and it writes into the region whose composer last took
the keys (`CANVAS-076`). The channel's region keeps the thread's composer while the panel is shut,
so a draft outlives it (`MURMURE-530`).

**The box of a thread.** A box under the thread's field sends the next reply to the channel as
well, and unchecks once it left (`MURMURE-531`). Alt+Enter turns it from the keys, and the turn
is announced, since the box is drawn (`MURMURE-534`, `MURMURE-535`). At the desktop width, the
label names the chord (`MURMURE-544`). The host reads a token the stylesheet sets at that width, so
no width lives outside the sheet.

**The history of the slice.** The composer keeps one history per draft. A caret move, a space or a
line break closes a step, and a paste or a cut is a step of its own.

**An edit in the slice.** The edit action puts the draft aside and loads the message. Either way
the edit ends, the draft comes back (`MURMURE-259`). The edit keeps each mention outside the
changed text.

**Code.** `crates/ambre-murmure/src/composer.rs` holds the drafts and the band. The view in
`crates/ambre-murmure/src/view.rs` routes the pipe's events to it and sends the message.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| Enter | send |
| Shift+Enter | a line break |
| Alt+Enter, in a thread | the box checked or unchecked |
| `@` | the mention list of `features/ambre-spec-murmure-mentions.md` |
| `:` then a name | the emoji list of `features/ambre-spec-murmure-emoji.md` |
| a file dropped or pasted | an attachment (`features/ambre-spec-murmure-attachments.md`) |
| Escape while editing | the edit is cancelled |
| `Ctrl+Z`, `Ctrl+Shift+Z` | undo or redo the last edit of the draft |
| `Ctrl+B`, `Ctrl+I`, `Ctrl+E` | bold, italic, inline code, as in Prose |
| a selection | the format bar of Prose, with the Murmure marks |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Light Markdown as typed | a permanent formatting toolbar | more chrome on the view that must stay minimal |
| Light Markdown as typed | plain text with mentions alone | a code block is what a technical team shares most |
| A draft lives until it is sent or its channel is lost | a draft that expires after a delay | the user loses a message they were still writing, with no warning |
| A layer of marks on the inline editor | a second text editor for chat | two editors to keep aligned on input, selection and composition |
| A layer of marks on the inline editor | a Prose document per draft | a Prose document lives on the Logic thread: a round trip per keystroke, against `PROSE-UI-001`, and Murmure would depend on Prose |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No block mark.** A code block, a list and a quote wait for the `Block marks in a message`
  Story (`ambre-roadmap.md` E5).
* **No format bar.** A selection shows nothing. That waits for the
  `The format bar of the composer` Story (`ambre-roadmap.md` E5).
* **An undo puts the caret at the end.** The history keeps the text of each step, not its caret.
  That waits for the `The caret after an undo` Story (`ambre-roadmap.md` E5).
* **A draft lives in memory.** A reload loses it, until the `Drafts` Story (`ambre-roadmap.md` E5).
* **A screen reader does not hear the draft.** The pipe takes the text, and nothing reads the draft
  back. That waits for the `The composer read aloud` Story (`ambre-roadmap.md` E5).
