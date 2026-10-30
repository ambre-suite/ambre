# AMBRE -- FEATURE: COMMENTS ON THE BOARD

**Type:** Vertical (feature)
**Implementation:** implemented

> A user pins a remark on an element or on empty board, and a teammate answers it hours later. The
> thread is resolved when the point is settled, and it stays readable afterwards.

Comments are the asynchronous half of collaborating on a board. Cursors and reactions are the
synchronous half, and they vanish; a thread must not.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. One rule of this feature lives with the
machine it belongs to and is not restated. The comment tool drops a thread where it clicks
(`TOILE-UI-042`, `features/ambre-spec-toile-tools.md section 1`).

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-024` | `INV` | A pin MUST stay on the thing it is about: an element, or a place on the board, never a place in the current view. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_thread_anchors_to_an_element_or_to_a_world_point` -- it reads back a thread anchored to an element and one anchored to a world point, and no anchor carries a viewport coordinate at all; `ambre_toile::ui::tool::tests::the_comment_tool_drops_a_thread_where_it_clicks` -- the tool anchors on the element under the click | none | verified |
| `TOILE-025` | `OBS` | A pin whose element is gone MUST NOT be painted. | this spec section 3; `features/ambre-spec-toile-connectors.md section 3` | `ambre_toile::logic::convergence_tests::a_pin_leaves_the_board_with_its_anchor_element` -- it deletes the anchor element and reads no pin out of the derived viewport | none | verified |
| `TOILE-FEAT-016` | `INV` | That thread MUST stay in the document and in the list: nothing deletes a discussion because the drawing changed. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::a_thread_survives_the_deletion_of_its_anchor_element` -- it deletes the element and reads the thread and its messages back out of the document | none | verified |
| `TOILE-FEAT-014` | `INV` | A message MUST carry the person who wrote it, not the session they wrote it from, so it survives a reload and a second device. | this spec section 3; `features/ambre-spec-toile-reactions.md section 5` | `ambre_toile::logic::convergence_tests::a_message_author_is_the_user_id_of_the_session` -- it reads the author back as the resolved user rather than the peer that wrote it | none | verified |
| `TOILE-FEAT-011` | `CONC` | Two users commenting on the same spot at the same moment MUST each end up with a thread of their own. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::two_peers_commenting_on_one_spot_mint_distinct_threads` -- it merges two peers commenting on one anchor and reads two surviving threads, so a merge that folded them into one fails it | none | verified |
| `TOILE-FEAT-068` | `CONC` | A thread's message list and its first message body MUST exist from its birth, so two users answering it first MUST converge. | this spec section 3; `TOILE-029` | `ambre_toile::logic::convergence_tests::two_peers_answering_one_brand_new_thread_both_keep_their_reply` -- it merges two first answers to one freshly seeded thread and reads all three messages back on both peers | none | verified |
| `TOILE-FEAT-012` | `CONC` | Messages MUST read in the same order on every screen. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::concurrent_replies_converge_in_list_order_not_clock_order` -- it merges two peers replying concurrently and compares the two resulting sequences | none | verified |
| `TOILE-FEAT-107` | `CONC` | That order MUST NOT come from the clock of the machine that wrote a message. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::concurrent_replies_converge_in_list_order_not_clock_order` -- its replies carry clocks that disagree with the list order, so a sort by `ts` fails it | none | verified |
| `TOILE-FEAT-013` | `CONC` | Two users correcting the same message MUST both keep their correction, character by character. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::concurrent_edits_of_one_message_converge_character_wise` -- it merges two corrections of one body and reads both back | none | verified |
| `TOILE-FEAT-069` | `CONC` | Two users resolving the same thread at the same moment MUST end on one resolved thread, not on two rival answers. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_resolves_converge_and_keep_the_messages` -- the assertion that reads one resolved thread back after both peers resolved it | none | verified |
| `TOILE-026` | `OBS` | The board MUST carry, for each pin it shows, where it sits, how many messages it holds and whether it is resolved. | this spec section 3; `features/ambre-spec-toile-engine.md section 3` | `ambre_toile::logic::convergence_tests::a_pin_carries_its_place_its_count_and_its_state` -- it reads the three fields of an element pin and of a point pin out of the derived viewport | none | verified |
| `TOILE-205` | `INV` | The board MUST NOT carry message bodies: a thread is read when it is opened. | this spec section 3; `features/ambre-spec-toile-engine.md section 3` | `ambre_toile::logic::convergence_tests::the_board_carries_no_body_and_a_read_carries_them_all` -- the read half; type-level -- the pin type declares no body field, so the compiler holds the other half | none | verified |
| `TOILE-UI-022` | `OBS` | A pin MUST keep one screen size through the whole zoom range. | this spec section 4; `features/ambre-spec-toile-engine.md section 3` | `ambre_toile::ui::comments::tests::a_pin_keeps_one_screen_size_through_the_zoom_range` -- it measures the marker at the two ends of the zoom range and at two points inside it | none | verified |
| `TOILE-UI-306` | `OBS` | No board content MUST hide a pin, whatever the z-order puts on top. | this spec section 4 | `ambre_toile::ui::view::view_tests::the_overlays_stack_peers_then_badges_then_pins` -- the board in its own pass, and the pins after every other overlay op | none | verified |
| `TOILE-UI-024` | `INV` | A pin's badge MUST be readable from the board alone, without opening the thread or the panel. | this spec section 4 | `ambre_toile::ui::comments::tests::the_badge_reads_the_count_and_the_resolved_state` -- it reads the badge of an open thread, of a resolved one, and of one past the room a marker has; `ambre_toile::ui::comments::tests::every_pin_paints_a_marker_and_a_badge` -- each pin of the frame paints a disc and its badge glyphs on the board | none | verified |
| `TOILE-UI-023` | `INV` | The panel MUST hold no answer of its own: everything it shows is read back from the board. | this spec section 4 | `ambre_toile::logic::convergence_tests::the_board_carries_no_body_and_a_read_carries_them_all` -- the read the panel lives on; `chrome_ui::the_comment_panel_shows_what_the_board_answered` -- the mounted panel replaces its rows whole on each answer, and a click leaves as an intent | none | verified |

### 1.2 Drop a comment on the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-070` | `POST` | Dropping a comment MUST leave a thread that is open, anchored where it was dropped, and carrying its first message. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_fresh_thread_is_open_anchored_and_carries_its_first_message` -- it reads the three clauses (the open flag, the anchor and the first body) off a freshly dropped thread | none | verified |
| `TOILE-UI-307` | `POST` | Dropping a comment MUST open its composer, so the user types where they clicked. | this spec section 4 | `ambre_toile::ui::tool::tests::the_comment_tool_drops_a_thread_where_it_clicks` -- the drop writes nothing and hands the view an anchor; `chrome_ui::a_dropped_comment_opens_its_composer_where_it_clicked` -- the mounted composer opens at the click with its field focused | none | verified |
| `TOILE-FEAT-017` | `FRAME` | A comment MUST NOT enter the history: undo on the board MUST NOT retract a message a teammate has read. | this spec section 3; `features/ambre-spec-toile-undo.md section 1` | `ambre_toile::logic::convergence_tests::undo_reverts_the_drawing_and_never_the_comment` -- it undoes after a message and reads the drawing reverted and the message still there | none | verified |

### 1.3 Write in a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-071` | `POST` | A reply MUST land at the end of the thread, after every message already in it. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_reply_lands_at_the_end_of_the_thread` -- it reads three bodies back in the order they were written, so a reply inserted before an earlier one fails it | none | verified |
| `TOILE-FEAT-072` | `POST` | Correcting a message MUST change that message and leave the rest of the thread as it reads. | this spec section 3 | `ambre_toile::logic::convergence_tests::correcting_a_message_leaves_its_neighbours_as_they_read` -- it corrects the middle of three messages and reads all three back | none | verified |
| `TOILE-FEAT-096` | `FAIL` | Correcting a message written by someone else MUST be refused, and MUST leave the message as it reads. | this spec section 3 | `ambre_toile::logic::convergence_tests::editing_another_authors_message_is_refused` -- it drives the edit from a second session and reads the body back unchanged | none | verified |
| `TOILE-FEAT-097` | `FAIL` | A reply to a thread the document no longer holds MUST be refused, and MUST change nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::replying_to_a_missing_thread_is_a_rejected_no_op` -- it replies to an unknown thread and reads no update on the outcome | none | verified |
| `TOILE-FEAT-073` | `FRAME` | Writing in a thread MUST NOT move its pin nor reopen it. | this spec section 3 | `ambre_toile::logic::convergence_tests::a_reply_moves_neither_the_pin_nor_the_resolved_flag` -- it replies to a resolved thread and reads the anchor and the resolved flag back unchanged | none | verified |

### 1.4 Resolve a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-FEAT-015` | `POST` | Resolving a thread MUST mark it resolved. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_resolves_converge_and_keep_the_messages` -- the resolved flag it reads back | none | verified |
| `TOILE-FEAT-105` | `POST` | Resolving a thread already resolved MUST change nothing. | this spec section 3 | `ambre_toile::logic::convergence_tests::concurrent_resolves_converge_and_keep_the_messages` -- the assertion that resolves an already resolved thread and reads no change | none | verified |
| `TOILE-FEAT-074` | `FRAME` | Resolving a thread MUST NOT touch its messages: a settled point stays readable. | this spec section 3; section 5 | `ambre_toile::logic::convergence_tests::a_resolve_keeps_every_message_body` -- it reads both bodies back after the resolve, so a resolve that blanked the message it kept fails it | none | verified |

### 1.5 Open a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-082` | `POST` | Clicking a pin MUST open that pin's thread. | this spec section 4; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::comments::tests::a_click_on_a_pin_names_its_thread_and_a_miss_names_none` -- it names the thread a press on the marker resolves to, and none for a press beside it | none | verified |
| `TOILE-UI-309` | `POST` | Clicking a thread in the board-wide list MUST bring the camera to that thread's anchor. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_thread_brings_the_camera_to_its_anchor` -- a point anchor at once, an element anchor once its box lands | none | verified |
| `TOILE-UI-310` | `POST` | The panel MUST re-read a thread whenever the board says that thread changed. | this spec section 4 | `ambre_toile::ui::view::view_tests::a_changed_thread_is_read_again_while_the_panel_shows_it` -- the list always, the thread when shown, nothing when closed | none | verified |
| `TOILE-UI-308` | `FRAME` | Clicking a pin MUST NOT select the element under it. | this spec section 4; `features/ambre-spec-toile-selection.md section 3` | `ambre_toile::ui::comments::tests::a_pin_takes_the_press_before_the_board_does` -- it resolves a press on a pin over an element to the thread, and a press beside it, on another button, or under a creation tool to the board; `::a_click_on_a_pin_names_its_thread_and_a_miss_names_none` -- the pick under it | none | verified |
| `TOILE-UI-411` | `POST` | Where pins overlap, a click MUST open the thread of the pin painted last. | this spec section 4 | `ambre_toile::ui::comments::tests::overlapping_pins_pick_the_topmost` -- two pins at one point, and a press on their centre names the later one | none | verified |
| `TOILE-UI-412` | `POST` | The panel filter MUST hide exactly the threads whose state it does not name. | this spec section 4 | `ambre_toile::ui::comments::tests::the_filter_keeps_what_it_names` -- All keeps both states, Open keeps the open thread alone, and Resolved the resolved one alone | none | verified |
* **Budget.** none of its own yet. A pin is one more node the viewport carries, so it lands under the
  visible-set row of `ambre-spec-budgets.md section 1`. The panel reads a thread on demand, so no
  figure covers the board-wide list.
* **Boundary.** none of its own: a thread is written to the document, and `ZK-001` seals every
  payload before it leaves the client. A message body is document content, so the relay reads none
  of it.
* **Verification.** 33 `verified`. What holds is everything a thread owes: the drop, the reply
  order, the correction, the resolve, the seeding at birth, the dangling anchor. The pins and the
  badge are host-tested, and so is the pass order of the overlay. The view's recentre and re-read
  are host-tested through the board view. The panel and the composer are mounted in a browser.

---

## 2. WHAT EACH USER SEES

The rows describe what the design owes, and each names the rule and the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| drops a comment on a note | drops one on the same note | two threads, distinct | both pins, neither overwritten (`TOILE-FEAT-011`) | `logic::convergence_tests::two_peers_commenting_on_one_spot_mint_distinct_threads` |
| replies to a thread | replies to it at the same moment | both replies, in list order | the same two replies in the same order on both screens (`TOILE-FEAT-012`) | `logic::convergence_tests::concurrent_replies_converge_in_list_order_not_clock_order` |
| edits her own message | reads it | one body, merged character-wise | Alice's correction, without losing Bob's read position (`TOILE-FEAT-013`) | `logic::convergence_tests::concurrent_edits_of_one_message_converge_character_wise` |
| answers a brand-new thread | answers it at the same moment | one message list, two replies | both answers, neither writing over the other's list (`TOILE-FEAT-068`) | `logic::convergence_tests::two_peers_answering_one_brand_new_thread_both_keep_their_reply` |
| resolves a thread | resolves it at the same moment | one resolved flag | one resolved thread, its messages intact (`TOILE-FEAT-069`, `TOILE-FEAT-074`) | `logic::convergence_tests::concurrent_resolves_converge_and_keep_the_messages` |
| deletes the element a thread is pinned to | reads that thread in the panel | the thread, its anchor pointing at nothing | no pin on the board, and the thread still in the list (`TOILE-025`, `TOILE-FEAT-016`) | `logic::convergence_tests::a_pin_leaves_the_board_with_its_anchor_element`, and `::a_thread_survives_the_deletion_of_its_anchor_element` |
| posts a message, then presses `Ctrl+Z` | reads the message | the message | the message on both screens, and Alice's last drawing action reverted (`TOILE-FEAT-017`) | `logic::convergence_tests::undo_reverts_the_drawing_and_never_the_comment` |

---

## 3. MODEL

**Containers.** The `comments` map, one entry per thread under a minted thread id. Each entry holds
its anchor, its resolved flag and its ordered messages, every message body a text container
(`ambre-spec-toile-schema.md section 3`). The engine seeds the whole thread in one write, so no
reply mints a rival container (`TOILE-029`).

**The anchor** (`ambre-bus`):

```rust
pub enum CommentAnchor {
    Element(ElementId),        // the pin tracks the element as it moves
    Point(kurbo::Point),       // a free world point, for a remark on empty board
}
```

**Commands** (`ambre-bus`): `CreateComment { anchor, body, ts }`, `ReplyComment
{ thread, body, ts }`, `EditComment { thread, message, delta }` and `ResolveThread
{ thread, resolved }`. None enters the history (`TOILE-FEAT-017`). `ReadComments { thread }` is the
panel's read: `Some(thread)` for one thread whole, `None` for the board-wide list. It writes
nothing.

**Events** (`ambre-bus`): `CommentsUpdated(thread)` and `CommentThreadOpened(thread)`, and the two
answers to a read -- `CommentList(rows)` and `CommentThreadData(thread)`. A list row carries the
anchor, the resolved flag, the message count and the opening line. A reader recognises a thread
before opening it; every other body arrives on `CommentThreadData` alone (`TOILE-205`).

**Steps, when a thread is created:**

1. mint a thread id, so two peers on the same spot cannot collide (`TOILE-FEAT-011`);
2. write the anchor, the resolved flag and the first message under that id, seeding the message
   list and the body container at once. No container is created lazily, and two peers answering
   first cannot each mint their own (`TOILE-029`, `TOILE-FEAT-068`);
3. commit outside the history scope, and emit the two events.

**Steps, when the viewport is derived:**

1. take each thread whose anchor resolves: an element in the visible set, or a point inside the
   camera;
2. carry its pin position: the anchored element's top-right corner, or the point itself. Carry its
   message count and its resolved flag, and no body (`TOILE-026`, `TOILE-205`);
3. skip a thread whose anchor element is gone, without touching it (`TOILE-025`).

**Why the author is a `UserId`.** A message must carry the person, not the session: the same argument
as a reaction, and it is written once in
`features/ambre-spec-toile-reactions.md section 5` (`TOILE-FEAT-014`).

**Why the timestamp orders nothing.** A clock belongs to the machine that wrote it, and a board is
open on machines nobody controls. The list order is the order the CRDT converges on, and the
timestamp is shown to the reader and used for nothing else (`TOILE-FEAT-012`).

**Code:** `ambre_toile::logic::state::comments` and the five comment commands of
`ambre_bus::app::toile::ToileCommand`. The pins on `DisplayViewport`, the pin painter and the pick
in `ambre_toile::ui::comments`, the comment tool in `ambre_toile::ui::tool`. The panel and the
composer are in `ambre_toile::chrome::comments`.

---

## 4. SURFACE

What the board offers, stated so the rows above have a section to cite:

| Trigger | Behaviour |
|---|---|
| the comment tool, then a click on the board | drops a thread at that point or on the element under it (`TOILE-FEAT-070`), and opens the composer (`TOILE-UI-042`, `TOILE-UI-307`) |
| a click on a pin | opens that thread in the panel (`TOILE-UI-082`) |
| the comments entry of the view menu | opens the panel on the whole board, filterable by open or resolved |
| a click on a thread in the board-wide list | recentres the camera on that thread's anchor (`TOILE-UI-309`) |
| the resolve toggle of an open thread | resolves it, or reopens it (`TOILE-FEAT-015`) |

**The pins.** A pin paints in screen space above all content, so it keeps one size through the zoom
(`TOILE-UI-022`). No element hides it, whatever the z-order puts on top (`TOILE-UI-306`). It
carries its message count and its resolved state, both read from the derived viewport and never from
the panel (`TOILE-UI-024`). The pick tests the pins before the elements. A pin over a note takes the
click, and the note under it stays unselected (`TOILE-UI-082`, `TOILE-UI-308`,
`features/ambre-spec-toile-selection.md section 3`).

**The panel** is a chrome side panel over the command queue, the same plumbing as the find panel
(`features/ambre-spec-toile-find.md section 4`). It holds no authoritative state (`TOILE-UI-023`):
the list and the open thread are what a `ReadComments` answered. It re-reads a thread when the
board says the thread changed (`TOILE-UI-310`). Every action it offers leaves as one intent the
canvas view carries out, because the panel names no camera and holds no command queue.

**The composer** is the field the comment tool opens at the click (`TOILE-UI-307`). A thread is born
with its first message, so nothing is written until the user posts. Abandoning the composer leaves
the board as it was.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A thread is its own entry under a minted id | one thread per element, keyed by the element id | two peers commenting on the same spot then collide on one key, and a merge resolves what should never have been a conflict. A minted id makes the collision unrepresentable rather than resolved |
| The anchor is an element or a point, never a screen position | a pin at a place in the current view | a viewport coordinate is meaningless on the second screen. A pin must follow the note it is about, through every pan, zoom and move |
| A dangling pin is kept and not painted | deleting the thread with its anchor | deleting the discussion because the drawing changed destroys the record of why it changed, and the element may come back through an undo or a late change |
| A message body is a text container | one register per message | a whole-string register loses one of two concurrent editors silently. It is rare, but the text container makes it cost nothing |
| Order is the list order, not the timestamp | sorting by the author's timestamp | a clock is untrusted and unsynchronized, so a wrong clock rewrites a conversation |
| A comment is outside the history | commenting like any other write | `Ctrl+Z` reverts what the user drew; withdrawing a message a teammate has already read is not an undo, it is a deletion nobody asked for |
| Mentions and notifications are out | a mention that mails or notifies | a mention needs a notification, which is Murmure's subject, and routing between the two applications does not exist |
| The author's clock rides the command | reading the clock in the Logic thread | that thread holds none, which is why a reaction carries its own `ts` too (`features/ambre-spec-toile-reactions.md section 5`). The clock is untrusted either way, and `TOILE-FEAT-012` is what keeps it out of the ordering |
| An edit carries a `TextDelta`, never a whole body | the edit command carrying the new body whole | the body is a text container, and a whole body settles two concurrent corrections by replacing one of them. It would undo `TOILE-FEAT-013` at the command layer, where the container cannot see it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A thread carries no reader state.** Nothing marks a thread read, so the panel cannot say what is
  new since the last visit. A board of open threads offers no unread count.
* **Nothing says who may comment.** Whether a viewer may open a thread, and whether a member may
  resolve someone else's, is a permission question no row covers
  (`ambre-spec-membership.md`). The engine takes the one decision that needs no rule: an edit on
  another author's message writes nothing.
* **Nothing says what a resolved thread costs.** A board of a thousand resolved threads carries them
  all in the document and in every export, and no rule prunes or archives them.
* **A comment reaches no export.** Neither the file round trip nor the image and page exports name a
  thread, so a resolved discussion exists on the board alone.
* **Mentions are deferred.** They need a notification path to Murmure, which does not exist
  (`README.md`, "Applications").
