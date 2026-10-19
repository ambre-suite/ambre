# AMBRE -- FEATURE: MURMURE UNREAD STATE
**Type:** Vertical (feature)
**Implementation:** partial -- cursors, marks and counts are built, and the sidebar draws them.
The message list marks a channel read, but nothing marks one unread from it yet.

> The sidebar shows where something new is, and how much. The count follows the user across their
> devices, and a reaction or an edit never makes a channel unread.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-113` | `INV` | The unread count of a channel MUST count new messages alone, never reactions and never edits. | this spec section 3 | `ambre_murmure::unread_tests::a_message_takes_the_number_of_the_update_that_created_it`; `ambre_murmure::unread_tests::an_edit_creates_no_message_and_never_counts`; `ambre_murmure::unread_tests::the_authors_own_message_is_numbered_by_its_echo_and_never_counts`; `ambre_logic::channels::tests::a_delivered_message_carries_the_relay_number_its_count_reads` -- a message takes the number of the update that created it, and an edit in a later update moves neither its number nor the count; structural -- the count walks the message list, and a reaction or an edit adds no entry to it | none | verified |
| `MURMURE-114` | `INV` | The read cursor of a user MUST be the same on every device of that user. | this spec section 3; `ambre-spec-server.md` | `ambre_server::routes::read_cursors::tests::a_cursor_moved_on_one_device_is_read_on_every_other` -- two devices of one user read the cursor either moved; structural -- the cursor table is keyed by user and document, and no column names a device | none | verified |

### 1.2 Read a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-115` | `POST` | Showing the latest message of a channel MUST move the read cursor to it. | this spec section 3; `MURMURE-470` | `ambre_murmure::list::tests::a_drawn_latest_message_marks_the_channel_read_once` -- each new latest message drawn marks the channel read once; `ambre_murmure::view::tests::the_selected_channel_shows_its_list_and_is_marked_read_once` -- the region sends mark read once its latest message paints, on a recording surface | none | verified |
| `MURMURE-470` | `POST` | Marking a channel read MUST move each of its read cursors to its latest message. | this spec section 3 | `ambre_logic::channels::tests::marking_a_channel_read_moves_its_live_segments_and_clears_its_mark`; `ambre_sync::updates::tests::a_read_reaches_the_last_update_delivered_and_no_further`; `ambre_murmure::channel_tests::the_final_segments_are_listed_loaded_or_not` -- the command moves the live segments to the last update delivered and the finals to their last number | none | verified |
| `MURMURE-116` | `FRAME` | Moving a read cursor MUST NOT move the cursor of another member. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone`; `ambre_server::routes::read_cursors::tests::a_listed_move_touches_the_listed_documents_alone` -- another member's cursors stay where they were | none | verified |

### 1.3 Mark a message unread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-117` | `POST` | Marking a message unread MUST move the cursor to just before it. | this spec section 3 | `ambre_logic::channels::tests::marking_a_message_unread_moves_the_cursor_to_just_before_it`; `ambre_logic::channels::tests::a_message_with_no_exact_number_is_not_marked_unread` -- the cursor of its segment moves to one below its number, and the count takes it back | none | verified |
| `MURMURE-154` | `FRAME` | Marking a message unread MUST NOT move the cursor of another channel. | this spec section 3 | `ambre_logic::channels::tests::marking_a_message_unread_moves_no_cursor_of_another_channel`; `ambre_murmure::channel_tests::the_segments_after_one_are_those_listed_later` -- the move names the documents of the message's own channel alone | none | verified |

### 1.4 Mark everything read

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-118` | `POST` | One command MUST move the cursor of every channel to its last number. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone` -- every segment of every channel reads inactive after the move | none | verified |
| `MURMURE-155` | `FRAME` | Marking everything read MUST NOT move the cursor of another member. | this spec section 3 | `ambre_server::routes::read_cursors::tests::marking_everything_read_moves_the_callers_cursors_alone` -- the other member's activity answer is unchanged | none | verified |

### 1.5 Show activity at start

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-119` | `POST` | The sidebar MUST show the activity answer of the relay before any count is known. | this spec section 3; `MURMURE-471` | `ambre_murmure::sidebar::tests::a_mark_shows_before_the_count_and_the_count_replaces_it` -- the answer marks a channel before any count; `ambre_app::shell::murmure_regions::a_channel_shows_its_mark_then_its_counts` -- the sidebar draws that mark with no count | none | verified |
| `MURMURE-471` | `POST` | The activity answer of the relay MUST reach the view as one mark per channel, before any count is known. | this spec section 3; `SRV-109` | `ambre_logic::channels::tests::an_activity_answer_marks_each_channel_once_and_opens_nothing` -- Logic hands the view one mark per channel | none | verified |
| `MURMURE-120` | `POST` | A count MUST replace the activity mark once the loaded document gives it. | this spec section 3 | `ambre_logic::channels::tests::an_open_channel_counts_the_messages_past_its_cursor`; `ambre_murmure::unread_tests::a_snapshot_hands_on_the_numbers_its_writer_knew`; `ambre_murmure::unread_tests::a_message_known_only_from_a_snapshot_lies_within_its_horizon` -- an open channel whose cursors are known hands the view a count with each delta and each answer, and none before | none | verified |
| `MURMURE-499` | `POST` | Each count of an open channel MUST reach the view with the first message that count counts. | this spec section 3; `MURMURE-184` | `ambre_murmure::unread_tests::the_first_unread_message_is_the_first_past_the_cursor_by_another` -- the first message past the cursor by another member, in list order; `ambre_logic::channels::tests::the_view_is_handed_the_first_unread_message_with_each_count` -- Logic hands it with each count, and none once read | none | verified |
| `MURMURE-460` | `POST` | A count MUST replace the activity mark of a channel not loaded once the summaries give it. | this spec section 3; `RESID-014` | `ambre_logic::channels::tests::a_closed_channel_counts_its_unread_from_the_summaries`; `ambre_logic::channels::tests::a_closed_channel_with_no_count_is_marked_by_a_counted_message` -- a channel counted at close adds the messages its summaries count, and one not counted is marked by the first | none | verified |
| `MURMURE-477` | `POST` | A segment that goes live while its channel is closed MUST count toward the channel's unread. | this spec section 3; `RESID-014` | `ambre_logic::channels::tests::a_closed_channel_watches_a_segment_opened_since` -- a moved `meta` lists the channel again, and each new segment is watched once; `ambre_sync::light::tests::a_watched_segment_counts_from_its_first_number` -- the watched segment counts from its first number | none | verified |
| `MURMURE-156` | `FRAME` | Showing activity MUST NOT load a document. | this spec section 3; `RESID-016` | `ambre_logic::channels::tests::an_activity_answer_marks_each_channel_once_and_opens_nothing` -- the answer reaches the view and no open reaches the Sync worker | none | verified |
| `MURMURE-495` | `FRAME` | The client MUST NOT count unread messages from the last number of the activity answer. | this spec section 3; `SRV-110`; `MURMURE-113` | `ambre_logic::unopened::tests::the_last_number_counts_nothing` -- a last number far past the cursor reaches the view as a mark, with no count | none | verified |

* **Budget.** the counters cost a light subscription per channel not held
  (`ambre-spec-budgets.md section 6`).
* **Boundary.** the relay holds the read cursors and answers yes or no, never a count
  (`SRV-110`, `ZK-015`). The kind of each update stays sealed.
* **Verification.** 17 `verified`, 0 `unimplemented`. The read cursor route tests hold the relay
  rows against a real database. The engine, Logic and Sync unit tests hold the client rows. A
  closed channel counts from its summaries, and a segment it opens meanwhile is not counted yet.
  The view's host tests hold the read cursor the message list moves.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| posts a message | reads another channel | one new message | an unread count of 1 on the channel (`MURMURE-113`) | `ambre_logic::channels::tests::an_open_channel_counts_the_messages_past_its_cursor` |
| reacts to a message | reads another channel | one reaction | no badge moves (`MURMURE-113`) | none |
| edits her message | reads another channel | one edit | no badge moves (`MURMURE-113`) | `ambre_murmure::unread_tests::an_edit_creates_no_message_and_never_counts` |
| -- | marks Alice's message unread | -- | the channel unread from that message, every other channel as it was (`MURMURE-117`, `MURMURE-154`) | `ambre_logic::channels::tests::marking_a_message_unread_moves_the_cursor_to_just_before_it` |
| reads a channel on her phone | -- | the cursor moved on the relay | the channel read on her laptop too (`MURMURE-114`) | `ambre_server::routes::read_cursors::tests::a_cursor_moved_on_one_device_is_read_on_every_other` |

---

## 3. MODEL

**CRDT containers.** none. The read cursor is a relay record, one per member and document
(`ambre-spec-server.md section 3`). It lives as long as the membership: a removal deletes it
(`SRV-137`).

**Three sources, one badge.**

| Source | Gives | When |
|---|---|---|
| the relay activity answer | a mark: something is new | at start, before anything loads |
| the sealed summaries | a count of messages and mentions | as updates arrive |
| the loaded document | the count | when the channel opens, and with each delta and each answer |

**A number per message.** The relay numbers an update after its author wrote it, so no message
carries its own number.

| Delivery | What the message learns |
|---|---|
| an update numbered `n` | `n`, for each message whose creating operation lies in the update's range; an operation sent twice keeps the lower |
| the relay's echo of the author's own update | its number, as any other member learns it |
| a snapshot folding up to `h`, framed | the number its row gives, a row above `h` dropped |
| a snapshot folding up to `h`, no row naming it | at most `h` |

**The snapshot frame.** Sealed with the snapshot, so the relay reads none of it (`ZK-015`). A
truncated frame never imports.

| Bytes | Holds |
|---|---|
| 4 | `amsq` |
| 4 | the row count, little-endian |
| 24 per row | a message identifier, then its number, little-endian |
| the rest | the Loro snapshot |

**The count, and mark unread.**

| Case | Rule |
|---|---|
| a message by someone else, numbered | counts when its number is past the cursor |
| a message bounded by a horizon | counts when the horizon is past the cursor |
| a message not numbered yet | counts: a count runs over, never under |
| a segment held with no cursor known, or an active final segment not held | no count; the mark stands |
| the first unread message | the first message, in list order, that the count counts; sent with each count |
| mark unread | the message's segment moves to one below its number, each later segment of the channel to the start |
| mark unread on a message with no exact number | refused, never guessed |

**Code.**

| Part | Where |
|---|---|
| the cursor table | `crates/ambre-server/migrations/20261010010000_read_cursors.sql` |
| the cursor queries | `crates/ambre-server/src/db/read_cursors.rs` |
| the cursor, activity and mark read routes | `crates/ambre-server/src/routes/read_cursors.rs` |
| the requests and the last number delivered | `crates/ambre-sync/src/channels.rs` |
| the number of each delivery | `crates/ambre-sync/src/updates.rs`, `crates/ambre-sync/src/websocket.rs` |
| the number per message, the snapshot frame | `crates/ambre-murmure/src/sequence.rs` |
| the count, the first unread message and the segments after one | `crates/ambre-murmure/src/channel.rs` |
| the cursors, the count per channel, mark read and mark unread | `crates/ambre-logic/src/channels.rs` |
| the mark read of a latest message drawn | `crates/ambre-murmure/src/list.rs`, `crates/ambre-murmure/src/view.rs` |

**Mark read.** A live segment still grows. Its cursor moves to the last number the Sync worker
delivered, so a message sequenced and not yet shown stays unread. A final segment no longer changes,
so the relay moves its cursor to its own last number, and the client need not load it.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a channel reaches its latest message on screen | its cursor moves, and its badge clears |
| mark unread on a message | the channel shows unread from that message |
| mark all read | every badge clears |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client counts | a wire message kind the relay reads | one more field in front of the relay, for a count the client computes itself |
| Messages alone count | every delta counts | a reaction would make a channel unread, which users of every chat product refuse |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A segment that goes live while its channel is closed** counts what its `meta` names. After a
  key rotation, it counts its activity alone. A `meta` write from another device of the same user
  lists nothing new, so its segment counts nothing until the reopen.
* **A snapshot from a member who knows no number.** Its messages read as bounded by its horizon,
  so a cursor below the horizon counts them all. Not yet ruled whether a compaction should wait
  for a member who holds the numbers.
