# AMBRE -- FEATURE: THE MURMURE MESSAGE LIST
**Type:** Vertical (feature)
**Implementation:** partial -- the channel and the thread each paint on a canvas of their own.
Each list scrolls by the wheel and a finger, and takes no focus yet.

> The conversation of a channel, newest at the bottom. It scrolls through years of history without
> a pause, and shows where the unread messages start.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-182` | `OBS` | Messages MUST be shown oldest at the top and newest at the bottom. | this spec section 3 | `ambre_murmure::list::tests::segments_read_oldest_at_the_top_whatever_their_order_of_arrival` -- the closing segment reads above the active one, whichever arrives first; `ambre_murmure::list::tests::a_changed_message_takes_its_place_and_a_moved_one_leaves_its_old_segment` -- a delta replaces in place and never shows a message twice; `ambre_murmure::paint::tests::the_rows_paint_top_to_bottom_under_one_header` -- the rows paint top to bottom in list order | none | verified |
| `MURMURE-183` | `OBS` | Consecutive messages of one author inside the grouping span MUST share one header. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_murmure::list::tests::one_author_inside_the_span_shares_one_header` -- inside the span one header, past it, another author or a notice a new one; `ambre_murmure::list::tests::a_clock_running_back_keeps_the_group` -- a clock running back keeps the group; `ambre_murmure::paint::tests::the_rows_paint_top_to_bottom_under_one_header` -- one author name paints over three bodies | none | verified |
| `MURMURE-184` | `OBS` | The first unread message MUST follow a separator that marks it. | this spec section 3; `features/ambre-spec-murmure-unread.md`; `MURMURE-499` | `ambre_murmure::list::tests::the_unread_line_holds_while_its_channel_stays_on_screen` -- the line keeps the first message known while its channel stays on screen, and starts over on another; `ambre_murmure::paint::tests::the_unread_line_paints_above_the_first_unread_message` -- the label paints between the read rows and the first unread one; `ambre_murmure::view::tests::the_unread_line_stays_after_the_channel_is_marked_read` -- the view marks the channel read and the line still paints | none | verified |
| `MURMURE-185` | `OBS` | A deleted message MUST show as deleted, with no text. | this spec section 3; `MURMURE-087` | `ambre_murmure::list::tests::a_deleted_message_shows_a_muted_line` -- a deleted message shows a muted stand-in line; `ambre_murmure::paint::tests::a_deleted_body_paints_muted_and_a_reply_count_in_the_accent` -- the stand-in paints in the muted colour, and no body text paints | none | verified |
| `MURMURE-186` | `OBS` | An edited message MUST show that it was edited. | this spec section 3; `MURMURE-084` | `ambre_murmure::list::tests::an_edited_message_carries_the_marker` -- an edited body ends with the marker, a plain one does not | none | verified |
| `MURMURE-187` | `INV` | The list MUST paint only the messages near the viewport. | this spec section 3; `ambre-spec-budgets.md section 2` | `ambre_murmure::list::tests::the_walk_from_the_bottom_measures_only_the_rows_it_reaches` -- five rows of ten thousand are measured; `ambre_murmure::list::tests::a_window_measures_only_the_rows_it_reaches_once_each` -- a window far up measures only its rows, each once; `ambre_murmure::paint::tests::ten_thousand_messages_lay_out_only_one_viewport` -- ten thousand messages lay out one viewport of rows | none | verified |
| `MURMURE-188` | `INV` | The list MUST be canvas content, its text shaped by the shared text pipeline. | this spec section 3; `ambre-spec-ui.md section 2` | `ambre_murmure::paint::tests::ten_thousand_messages_lay_out_only_one_viewport` -- the list paints on a drawing surface, its text shaped by the shared text cache; `ambre_murmure::paint::tests::the_rows_paint_top_to_bottom_under_one_header` -- each text paints as shaped glyph runs | none | verified |
| `MURMURE-207` | `OBS` | The focused message MUST be announced to a screen reader with its author, its time and its text. | this spec section 3; `ambre-spec-accessibility.md section 2`; `CHROME-023` | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-208` | `OBS` | A new message MUST be announced to a screen reader when the user was at the bottom of the list. | this spec section 3; `ambre-spec-accessibility.md section 2` | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-417` | `INV` | The list MUST scroll through the shared kinetic model, never through a speed or a decay of its own. | this spec section 3; `KINETIC-001` | type-level -- the list's `scroll_by` takes a `ScrollStep`, which only the model builds; `ambre_murmure::view::tests::a_paused_glide_coasts_and_a_notch_never_does` -- a paused glide coasts on the model's ticks, a notch never does; `ambre_murmure::view::tests::a_finger_drags_the_list_and_a_still_one_taps` -- a finger drags past the slop, a still one taps; ci-gate -- `scripts/crate_boundaries.py` refuses an anchor moved by a distance outside `scroll_by`, and a friction or a decay outside the model | none | verified |
| `MURMURE-418` | `OBS` | The time of a message MUST show in the reader's own language and time zone. | this spec section 3 | `ambre_app::host::murmure::tests::the_time_reads_in_the_readers_language_and_zone` -- the time reads as the browser writes it in its own language and zone, in the browser tier | none | verified |
| `MURMURE-419` | `INV` | Every action on a message MUST be a command of the command table. | this spec section 3; `UI-THREAD-019` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-420` | `INV` | A thread MUST show its messages through the same list as a channel. | this spec section 3; `features/ambre-spec-murmure-threads.md` | `ambre_murmure::list::tests::a_thread_fills_its_own_list_and_leaving_drops_the_channel` -- a thread fills a list of the same type as a channel; `ambre_murmure::view::tests::the_thread_region_shows_the_thread_through_the_same_list` -- the thread's region paints the thread through the same painter, on a recording surface | none | verified |
| `MURMURE-421` | `INV` | Each region that shows messages MUST be a canvas of its own, one cell of the chrome layout. | this spec section 3; `CHROME-009`; `CANVAS-070` | `ambre_app::shell::murmure_regions::tests::the_channel_and_the_thread_are_each_a_canvas_of_their_cell` -- the channel and the thread each hold a canvas of their cell's width; `ambre_app::host::cell::tests::the_thread_is_a_canvas_of_its_own_sized_by_its_cell` -- the thread's canvas takes its own size, input and resize, in the browser tier | none | verified |
| `MURMURE-509` | `OBS` | The control back to the latest message MUST be reachable from the keyboard and announced to a screen reader. | this spec section 3; `ambre-spec-accessibility.md section 2` | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 A message arrives

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-189` | `POST` | A new message MUST scroll into view when the user was at the bottom of the list. | this spec section 3 | `ambre_murmure::list::tests::a_window_past_the_bottom_rests_on_the_latest_row` -- a row pushed past the bottom rests the list on its latest row; `ambre_murmure::view::tests::a_new_message_scrolls_into_view_at_the_bottom` -- at the bottom the new message paints, and is read | none | verified |
| `MURMURE-190` | `FRAME` | A new message MUST NOT move the list when the user was reading higher up. | this spec section 3 | `ambre_murmure::list::tests::a_window_on_a_row_keeps_it_in_place_as_rows_arrive_below` -- rows added below keep the resting row in place; `ambre_murmure::paint::tests::a_list_resting_on_a_row_paints_from_it` -- the paint starts at the resting row; `ambre_murmure::view::tests::a_new_message_leaves_a_reader_higher_up_in_place` -- higher up the same rows paint, and nothing is read | none | verified |
| `MURMURE-209` | `FRAME` | Announcing a message MUST NOT move the focus. | this spec section 3; `CHROME-065` | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Scroll up

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-191` | `POST` | Reaching the oldest loaded message MUST ask the engine for the previous segment. | this spec section 3; `MURMURE-023` | `ambre_murmure::list::tests::each_row_has_a_key_and_the_oldest_segment_leads` -- the oldest segment is the first the list holds; `ambre_murmure::paint::tests::a_list_resting_on_a_row_paints_from_it` -- the paint says when the oldest row shows; `ambre_murmure::view::tests::the_oldest_row_asks_for_the_segment_before_and_its_rows_move_nothing` -- the view asks once per segment | none | verified |
| `MURMURE-192` | `FRAME` | Loading older messages MUST NOT move the messages on screen. | this spec section 3 | `ambre_murmure::view::tests::the_oldest_row_asks_for_the_segment_before_and_its_rows_move_nothing` -- the rows of the older segment land above, and the rows on screen paint where they were | none | verified |


### 1.4 Move through the messages

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-205` | `POST` | Up and Down in the list MUST move the focus from one message to the next. | this spec section 3 | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-206` | `FRAME` | Moving the focus MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 A segment cannot be opened

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-218` | `FAIL` | Given a segment whose key this client does not hold, the list MUST show one band in its place, naming its time span, the cause and the remedy. | this spec section 3; `ambre-spec-murmure-schema.md section 3`; `SYNC-173` | `ambre_logic::channels::tests::a_key_not_received_shows_one_band_for_the_segment` -- a key not received hands the view one band, and the segment can be asked again; `ambre_logic::channels::tests::a_transient_failure_shows_no_band` -- a relay out of reach hands none; `ambre_murmure::channel_tests::a_band_spans_its_segment_up_to_the_next_one` -- the band spans its segment up to the next listed one; `ambre_murmure::list::tests::an_unreadable_segment_shows_as_one_band_in_its_place` -- one muted row in its place names the span, the cause and the remedy; `ambre_murmure::paint::tests::a_band_paints_as_one_muted_row` -- it paints muted, with no header | none | verified |
| `MURMURE-219` | `FAIL` | Given a segment older than the member's history boundary, the list MUST show the boundary as the start of the channel, never as a failure. | this spec section 3; `features/ambre-spec-murmure-channels.md` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-220` | `FAIL` | Given a segment that fails to open under the key it names, the list MUST show it as damaged, never as a missing key. | this spec section 3; `SYNC-045` | `ambre_logic::channels::tests::a_segment_that_fails_its_check_shows_as_damaged` -- bytes that fail the check hand the view a damaged band, and the document closes; `ambre_murmure::list::tests::an_unreadable_segment_shows_as_one_band_in_its_place` -- a damaged band names no missing key | none | verified |
| `MURMURE-221` | `FRAME` | A segment the client cannot open MUST NOT stop the list from loading the segments before it. | this spec section 3 | `ambre_murmure::channel_tests::a_segment_given_up_still_leads_to_the_one_before` -- a segment given up still leads to the one listed before it; `ambre_murmure::list::tests::messages_replace_the_band_of_their_segment` -- the band holds one row, and the messages of its segment replace it | none | verified |

### 1.6 Act on a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-422` | `POST` | A right-click on a message MUST open its actions in the shared floating menu. | this spec section 3; `CHROME-004` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-423` | `POST` | A long press on a message MUST open its actions on a touch screen. | this spec section 3; `KINETIC-003` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-424` | `FRAME` | Opening the actions of a message MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Select and copy text

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-425` | `POST` | A mouse drag across the messages MUST select their text, from one character to several messages. | this spec section 3; `CANVAS-037` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-426` | `POST` | Copying a selection MUST put its text on the clipboard, each message under its author and its time. | this spec section 3; `UI-THREAD-006` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-427` | `FRAME` | Selecting text MUST NOT move a read cursor. | this spec section 3; `MURMURE-206` | not built | unbuilt -- lands with the `The actions and the selection of a message` Story (`ambre-roadmap.md` E5) | unimplemented |

**Frame:** `UI-THREAD-044`

### 1.8 Jump through the list

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-504` | `POST` | Page Up and Page Down in the list MUST move it by one viewport, through the shared kinetic model. | this spec section 3; `MURMURE-417` | `ambre_murmure::view::tests::page_up_and_page_down_move_the_list_by_one_viewport` -- Page Up rests the list where a wheel step of one page does, Page Down brings it back, a chord stays the browser's | none | verified |
| `MURMURE-505` | `POST` | End, or the control a list read higher up shows, MUST bring the list back to its latest message. | this spec section 3; `MURMURE-189` | `ambre_murmure::view::tests::end_or_the_control_brings_back_the_latest_message` -- End, a click and a tap on the control each bring back the latest message, and the control shows only higher up; `ambre_murmure::paint::tests::the_jump_control_paints_its_words_inside_what_it_answers` -- the control paints its words inside the rectangle a press hits | none | verified |
| `MURMURE-506` | `FRAME` | A jump through the list MUST NOT mark read a message it did not show. | this spec section 3; `MURMURE-206` | `ambre_murmure::view::tests::a_jump_marks_read_only_what_it_showed` -- pages short of a new message mark nothing, and End marks it once it paints | none | verified |

* **Budget.** the frame budget of `ambre-spec-budgets.md section 2` holds the scroll, and the
  memory rows of section 6 hold the loaded segments.
* **Boundary.** none: the list paints what the engine decrypted, and holds no key. A band shows
  only what `meta` already tells every member: the span of a segment.
* **Verification.** 21 `verified`, 14 `unimplemented`. The host tier holds the order, the groups,
  the words and the paint of the rows and the bands. It holds the scroll, the jumps and the thread's
  list too. The browser tier holds each canvas, the finger on it and the local time.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | Alice sees | Test |
|---|---|---|---|
| reads the bottom of the channel | posts | Bob's message scrolls into view (`MURMURE-189`) | `ambre_murmure::view::tests::a_new_message_scrolls_into_view_at_the_bottom` |
| reads last week, higher up | posts | the list stays where she reads (`MURMURE-190`) | `ambre_murmure::view::tests::a_new_message_leaves_a_reader_higher_up_in_place` |
| scrolls past the oldest loaded message | -- | older messages appear above, and nothing on screen jumps (`MURMURE-192`) | `ambre_murmure::view::tests::the_oldest_row_asks_for_the_segment_before_and_its_rows_move_nothing` |
| reads higher up, then presses End or the control | posts | Bob's message at the bottom, read once shown (`MURMURE-505`, `MURMURE-506`) | `ambre_murmure::view::tests::a_jump_marks_read_only_what_it_showed` |
| posts three messages within a minute | -- | one header over the three (`MURMURE-183`) | none |
| opens a channel on a new phone, before its keys arrive | -- | one band per unreadable segment, with its dates, its cause and how to try again (`MURMURE-218`) | none |
| joins a channel invited without history | -- | the boundary at the top, and no error (`MURMURE-219`) | none |

---

## 3. MODEL

**CRDT containers.** none of its own: the list reads the messages of the loaded segments through
the engine.

**Virtualized in one dimension.** The list keeps the height of each message it measured, and paints
the messages that cross the viewport plus a margin. The scroll anchor is a message, not a pixel
offset, so a load above keeps the anchor in place (`MURMURE-192`).

```rust
pub struct ScrollAnchor { pub message: MessageRef, pub offset: PixelOffset }
pub struct PixelOffset(f32);
pub struct RowHeight(f32);
```

**Canvas and DOM.** The messages are canvas content, shaped by the text pipeline every application
uses (`ambre-spec-ui.md section 2`). An image and an animated GIF are painted in place
(`features/ambre-spec-murmure-attachments.md`). The fonts, the colour emoji and the decoded images
come from `ambre-spec-render-assets.md`. The list paints through the drawing surface of
`ambre-spec-drawing-surface.md`, inside the frame loop of `ambre-spec-frame.md`.

**One list, in every place a message shows.** A channel, a thread and the second pane of a split
view each show one list (`MURMURE-420`). Each list is a canvas of its own, in its own cell of the
chrome layout (`MURMURE-421`). A list owns its scroll, its hit-test and its repaint plan. A closed
list costs nothing. The DOM around a list holds its header and its controls, never a message.

**The scroll.** The list takes its pan, its flick, its wheel momentum and its edge scroll from
`ambre-spec-kinetic-scroll.md` (`MURMURE-417`). The list owns step 4 of that model alone: it moves
its anchor by the distance of each tick. A page key steps it by a viewport (`MURMURE-504`), and
End or the control over the list rests it on the latest message (`MURMURE-505`).

**The time of a message.** The `ts` of a message is Unix milliseconds. The list formats it in the
language and the time zone of the reader (`MURMURE-418`). A channel carries no locale: two members
in two countries read one message at two local times.

**The actions of a message.** Each action is a command of the table of `ambre-spec-commands.md`,
with its label, its icon and its chord where it has one (`MURMURE-419`). The hover bar, the context
menu, the long press and the palette read the same description. A long press is a press held
inside the tap slop of `ambre-spec-kinetic-scroll.md`, past the delay of the platform.

| Command | Offered on |
|---|---|
| react, reply, copy link, pin, bookmark, mark unread | every message the member can read |
| edit, delete | a message of the member |
| remove | a message of another member, for a moderator |
| copy text | a message with text |

**Selection.** A mouse drag across the list selects text through the selection model of
`ambre-spec-text-editor.md` (`MURMURE-425`). On a touch screen a drag scrolls, so the copy text
action takes the whole message. The text is read-only, so the model runs with no caret.
A copy writes the `text/plain` mirror and the private type of `ambre-spec-clipboard.md`
(`MURMURE-426`). The mirror puts each message under its author and its time.

**A screen reader** reaches the list through the mirror of `ambre-spec-accessibility.md section 2`:
the live region carries the focused message, with its author and its time (`MURMURE-207`). A new
message at the bottom is announced there too, politely, and the focus stays put (`MURMURE-209`).

**A segment that cannot be opened** keeps its place. Its entry in the `segments` list of `meta`
gives its start, and the next entry its end. So the band says which messages it stands for. The
Sync worker tells a missing key from a failure that may clear (`SYNC-173`). The engine names the
cause; the list names the remedy.

```rust
pub enum Unreadable {
    KeyNotReceived,         // the key chain did not reach the key of this segment
    BeforeHistoryBoundary,  // the member was invited without this history
    Damaged,                // the key it names is held, and the segment does not open
}
```

| Cause | The band says | Remedy |
|---|---|---|
| `KeyNotReceived` | the keys for these messages have not reached this device | retry, which fetches the key chain again from the relay |
| `BeforeHistoryBoundary` | the history before this date was not shared with you | none: the band is the start of the channel |
| `Damaged` | these messages could not be verified | retry, which fetches the segment again from the relay |

A bare "unable to decrypt" names no cause and no remedy, so no band shows it (`MURMURE-218`).
A band is one row of the list, whatever the segment holds, so scrolling goes on past it
(`MURMURE-221`).

**Code.** `ambre_murmure::list` folds the events into one list per channel and per thread, and
`ambre_murmure::paint` lays out and paints the rows a viewport reaches. `ambre_murmure::view` binds
both to each region, and `ambre_app::host::murmure` hands it the pane, the names and the time.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the wheel, a finger or a key | the list scrolls, and coasts after a flick |
| the pointer over a message | its main actions, in a bar over the message |
| a right-click on a message | every action of the message, in the floating menu |
| a long press on a message, on a touch screen | every action of the message, in the floating menu |
| a mouse drag across the text | a selection, and the list scrolls when the drag reaches an edge |
| the copy chord over a selection | the text on the clipboard, each message under its author and its time |
| the new-messages separator | shown once, at the first unread message |
| a band of unreadable messages | its cause and its remedy, read out to a screen reader as one row |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Messages on the canvas | a DOM list | a DOM node per message breaks the frame budget on a channel of years, and the other applications already paint their content on the canvas |
| Messages of a thread in the canvas list | a DOM thread panel | two renderers of one message drift, and each feature of a message would be written twice |
| One canvas per list | one canvas for every list, cut into views | a canvas per cell keeps the chrome layout whole, and gives each list its own scroll and hit-test |
| Text selection drawn by the list | a transparent DOM text layer over the list | two layouts of one text drift apart, and the layer costs the DOM node per message the canvas avoids |
| A message as the scroll anchor | a pixel offset | a load above shifts every offset, and the reader loses the line |
| A band per unreadable segment | one line per message that did not decrypt | the whole segment shares one key, so one cause covers it, and a thousand identical lines bury the readable ones |
| A band that names cause and remedy | a bare "unable to decrypt" | Element users rank that state among their main complaints: it says nothing they can act on |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The control back to the latest message is painted, not focusable.** No key reaches it and no
  screen reader announces it (`MURMURE-509`). Page Up, Page Down and End stand in for it. No focus
  and no selection exist yet. It lands with the
  `Focus and the screen reader in the list` Story (`ambre-roadmap.md` E5).
* **A band names its remedy in words.** It says to reopen the channel; no control on it asks again.
* **Only the focused message reaches a screen reader.** The list is not mirrored whole, by the rule
  of `ambre-spec-accessibility.md section 2`, so a reader moves through it message by message.
