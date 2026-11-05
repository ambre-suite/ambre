# AMBRE -- FEATURE: MURMURE THREADS
**Type:** Vertical (feature)
**Implementation:** partial -- the reply, its thread and the count are built.

> A member replies to a message, and the replies gather in a thread beside the channel. The root
> shows how many replies it has, even when nobody opened the thread.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-105` | `INV` | The replies to a message MUST live in one thread document per root message. | this spec section 3; `ambre-spec-murmure-schema.md section 2`; `SRV-213` | `ambre_murmure::thread_tests::every_reply_to_a_root_writes_its_one_thread_document` -- a first reply creates the thread, later ones write it, and another root has its own; `::a_failed_link_drops_the_own_thread_and_a_locked_root_waits` -- a failed link drops the thread, and a locked root waits for its amendments; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the reply leaves for the thread once it went up whole | none | verified |
| `MURMURE-106` | `INV` | The reply count and the last replier of a root MUST be read from its thread document. | this spec section 3 | `ambre_murmure::thread_tests::the_root_reads_its_reply_count_and_last_replier_from_its_thread` -- a deleted reply aside, and a peer without the thread reads none; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the root's head counts the reply; `::a_root_another_member_threaded_opens_its_thread` -- a member who did not link the thread opens it and reads its replies | none | verified |
| `MURMURE-108` | `CONC` | Two peers sending the first reply at once MUST converge on one thread document. | this spec section 3; `ambre-spec-murmure-schema.md section 6`; `SRV-214` | `ambre_murmure::thread_tests::two_first_replies_converge_on_the_thread_the_relay_linked` -- the loser drops its own, and both roots name one document; `ambre_logic::channels::tests::a_lost_or_failed_thread_link_moves_the_reply_to_another_document` -- the reply waits on the winner, and a failed link creates again | none | verified |
| `MURMURE-261` | `INV` | A mention in a reply MUST count on the channel of its thread. | this spec section 3; `MURMURE-233` | `ambre_murmure::thread_tests::a_mention_in_a_held_thread_counts_past_its_read_cursor` -- a reply naming me counts past my cursor, never at it; `ambre_logic::channels::tests::a_mention_in_a_held_thread_counts_on_its_open_channel` -- the mention marks the open channel, unread unmoved; `::a_mention_in_a_watched_thread_counts_on_its_channel` -- a watched thread marks its open and its closed channel | none | verified |
| `MURMURE-532` | `OBS` | A reply also sent to the channel MUST read, in the channel, a line saying it was sent in a thread. | this spec section 3; `MURMURE-109` | `ambre_murmure::list::tests::a_reply_sent_to_the_channel_too_names_its_thread` -- the line under the copy, which a screen reader hears after the text | none | verified |
| `MURMURE-290` | `INV` | A followed thread MUST stay in the Threads view until the user unfollows it, never leaving on a delay. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Reply in a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-107` | `POST` | The first reply to a message whose `thread` field is absent MUST set it to the thread document. | this spec section 3 | `ambre_murmure::thread_tests::the_link_sets_the_root_thread_field_and_nothing_else` -- the link writes the field, and a second link writes nothing | none | verified |
| `MURMURE-109` | `POST` | A reply also sent to the channel MUST write a copy in the active segment that names its thread. | this spec section 3 | `ambre_murmure::thread_tests::a_reply_also_sent_to_the_channel_writes_a_copy_naming_its_thread` -- the copy reads the reply's text and its thread; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the copy leaves under the active segment; `ambre_murmure::thread_tests::a_reply_to_its_thread_alone_needs_no_active_session` -- before the active segment's PeerID lands, a thread-only reply leaves and a copy waits | none | verified |
| `MURMURE-110` | `FRAME` | Replying MUST NOT change the root message beyond its `thread` field. | this spec section 3 | `ambre_murmure::thread_tests::the_link_sets_the_root_thread_field_and_nothing_else` -- the delta of the link holds one operation, on that field | none | verified |
| `MURMURE-484` | `POST` | The first reply to a message of a locked segment MUST record its thread in the amendments of that segment. | this spec section 3; `MURMURE-013` | `ambre_murmure::amending_tests::a_thread_of_a_locked_root_is_recorded_in_its_amendments` -- the reply waits for the amendments, which record the thread; `::another_member_finds_the_thread_of_a_locked_root` -- a second member reads the record and opens the thread; `::a_thread_record_outside_the_rule_is_refused` -- the check refuses a foreign key, a second thread, a bad value and a removal; `::a_fold_carries_the_thread_of_a_locked_root` -- the fold writes the root's field; `ambre_logic::channels::tests::a_reply_to_a_locked_root_waits_for_its_amendments` -- the link leaves under the amendments | none | verified |
| `MURMURE-485` | `POST` | An edit, a delete or a reaction on a reply MUST write in its thread document. | this spec section 3; `MURMURE-084`, `MURMURE-091` | `ambre_murmure::thread_tests::an_edit_a_reaction_and_a_delete_on_a_reply_write_in_its_thread` -- each change writes the thread, and a delete leaves the count; `::another_member_reacts_to_a_reply_and_never_edits_it` -- the check takes a reaction, and an edit is refused; `ambre_logic::channels::tests::a_change_on_a_reply_leaves_for_its_thread` -- each change leaves under the thread | none | verified |
| `MURMURE-538` | `POST` | An edit of a reply also sent to the channel MUST write the same edit to its copy. | this spec section 3; `MURMURE-109`; `MURMURE-084` | `ambre_logic::channels::tests::a_copy_follows_the_edit_and_the_delete_of_its_reply` -- the edit leaves under the thread, then under the copy's segment, where the copy reads it; `ambre_murmure::thread_tests::a_copy_is_found_from_its_reply` -- the copy is found from its reply, and a reply sent alone has none; `ambre_murmure::channel::threading::copy_id_tests::a_copy_differs_from_its_reply_and_from_every_other_copy` -- over generated identifiers, a copy never takes its reply's identifier, and two replies never share a copy | none | verified |
| `MURMURE-539` | `POST` | A delete of a reply also sent to the channel MUST delete its copy too. | this spec section 3; `MURMURE-109`; `MURMURE-087` | `ambre_logic::channels::tests::a_copy_follows_the_edit_and_the_delete_of_its_reply` -- the copy reads deleted and empty, and a reaction stayed on the reply | none | verified |
| `MURMURE-533` | `POST` | The reply action on a reply also sent to the channel MUST open the thread it was sent in, at its root. | this spec section 3; `MURMURE-109` | `ambre_murmure::view::tests::the_reply_on_a_copy_opens_its_thread` -- the copy under the pointer names the root its list holds; `ambre_murmure::commands::tests::the_reply_on_a_copy_opens_the_thread_it_came_from` -- that root's thread opens, never one of the copy's own | none | verified |
| `MURMURE-545` | `POST` | The reply action on a copy whose root lies in no loaded segment MUST load the segments down to the root's, then open its thread at its root. | this spec section 3; `MURMURE-533`; `MURMURE-023` | `ambre_murmure::thread_tests::the_reply_on_a_far_copy_loads_down_to_its_root` -- a probe reads the header, the segments load one at a time, and the root is found; `ambre_logic::channels::tests::the_reply_on_a_far_copy_loads_its_root_then_names_it` -- the probe is closed, the segment opens, and the view is told the root; `ambre_murmure::commands::tests::the_reply_on_a_copy_opens_the_thread_it_came_from` -- such a copy asks Logic to seek its root; `ambre_murmure::tests::chrome_relay::a_root_found_opens_its_thread_in_the_channel_shown` -- the found root opens its thread in the channel still shown | none | verified |
| `MURMURE-546` | `FRAME` | Seeking the root of a copy MUST NOT load a segment older than the one its thread's header names. | this spec section 3; `MURMURE-545` | `ambre_murmure::thread_tests::the_reply_on_a_far_copy_loads_down_to_its_root` -- the seek stops at the named segment, and a found root ends it; `::a_seek_ends_on_what_it_cannot_follow` -- a named segment without the root asks nothing older | none | verified |
| `MURMURE-547` | `FAIL` | Given a thread whose header names no segment `meta` lists, or a segment on the way that fails, the seek MUST end with no thread opened. | this spec section 3; `MURMURE-545`; `MURMURE-221` | `ambre_murmure::thread_tests::a_seek_ends_on_what_it_cannot_follow` -- an unlisted segment, a failed segment, a failed probe and a missing root each end it; `ambre_logic::channels::tests::the_reply_on_a_far_copy_loads_its_root_then_names_it` -- a probe that fails asks nothing more; `ambre_murmure::thread_tests::a_failed_probe_of_another_thread_leaves_the_seek_waiting` -- only the failure of the seek's own probe ends it | none | verified |
| `MURMURE-548` | `FAIL` | Given a seek that ends with no root, the client MUST tell the user once that the thread cannot open. | this spec section 3; `MURMURE-547` | `ambre_murmure::thread_tests::a_seek_ends_on_what_it_cannot_follow` -- each end of a seek steps lost once, then nothing; `ambre_logic::channels::tests::the_reply_on_a_far_copy_loads_its_root_then_names_it` -- a failed probe tells the view once; `ambre_murmure::tests::chrome_relay::a_root_lost_tells_the_user_the_thread_cannot_open` -- one toast says so, and the pane stays | none | verified |

### 1.3 Follow a thread that is not open

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-111` | `POST` | A client MUST reload the head of a thread when the relay signals activity on it and its root is on screen. | this spec section 3; `SRV-161` | `ambre_logic::channels::tests::a_root_another_member_threaded_opens_its_thread` -- a root on screen opens its thread, which reads each new reply; `ambre_murmure::thread_tests::the_root_reads_its_reply_count_and_last_replier_from_its_thread` -- the head reads the thread held; `ambre_sync::light::tests::a_thread_signal_reaches_logic_only_for_a_thread_not_held` -- a held thread takes its deltas, never the signal | none | verified |
| `MURMURE-260` | `POST` | An activity signal on a thread MUST make the client fetch the summaries of its new deltas, with no document loaded. | this spec section 3; `RESID-014`, `SRV-161` | `ambre_sync::light::tests::a_watched_thread_counts_after_its_read_cursor` -- the light subscription asks the deltas past the cursor and reads their summaries; `ambre_logic::channels::tests::a_signalled_thread_is_watched_from_its_read_cursor` -- a signal watches the thread from its cursor, once its channel is known | none | verified |
| `MURMURE-112` | `FRAME` | An activity signal MUST NOT load a thread whose root is off screen. | this spec section 3; `RESID-016` | `ambre_logic::channels::tests::a_signalled_thread_is_watched_from_its_read_cursor` -- the signal opens no document, only a watch; `::a_signal_on_a_held_thread_or_an_unopened_channel_watches_nothing` -- a held thread and an unopened channel ask nothing | none | verified |

### 1.4 Follow a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-291` | `POST` | Writing the root, replying, or being mentioned in a thread MUST follow it. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-292` | `POST` | The Threads view MUST list every followed thread with unread replies, newest first. | this spec section 3; `MURMURE-260` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-293` | `POST` | Marking a thread read MUST move its read cursor to its last reply. | this spec section 3; `MURMURE-114` | `ambre_logic::channels::tests::marking_a_thread_read_moves_its_cursor_to_its_last_reply` -- the cursor takes the last reply delivered, the mention stops counting, and a thread not held moves nothing; `ambre_sync::updates::tests::a_read_reaches_the_last_update_delivered_and_no_further` -- the number the worker sends stops at what it delivered | none | verified |
| `MURMURE-497` | `POST` | Showing the last reply of a thread MUST mark the thread read. | this spec section 3; `MURMURE-293`; `MURMURE-115` | `ambre_murmure::list::tests::a_drawn_last_reply_marks_the_thread_read_once` -- each new last reply drawn marks the thread read once, apart from its channel; `ambre_murmure::view::tests::the_thread_region_shows_the_thread_through_the_same_list` -- the thread's region sends it once its last reply paints, on a recording surface | none | verified |
| `MURMURE-294` | `POST` | Unfollowing a thread MUST remove it from the Threads view and from the user settings. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-415` | `FAIL` | Given a followed thread whose document expired, the client MUST drop it from the Threads view and from the user settings. | this spec section 3; `SRV-156` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-295` | `FRAME` | Following a thread MUST NOT subscribe the client to it. | this spec section 3; `MURMURE-112` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a thread head is a small read per visible root, inside the memory rows of
  `ambre-spec-budgets.md section 6`.
* **Boundary.** a thread is a document of its channel, sealed like the others (`ZK-001`). The relay
  learns that a document is a thread of the channel, the identifier of its root, and when it
  changes (`ZK-015`, `SRV-162`).
* **Verification.** 22 `verified`, 6 `unimplemented`. The engine tests hold the thread document,
  the link, the count, the changes on a reply, a locked root and the mentions. The Logic tests
  hold the waits, the race, the watch and the read cursor. The Sync tests hold the signal, the
  light thread and the number a read reaches. The view's host tests hold the read a thread shown sends.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| replies to Bob's message | watches the channel | a thread document, and `thread` on the root | "1 reply" under the root, on both screens (`MURMURE-106`) | `ambre_logic::channels::tests::a_root_another_member_threaded_opens_its_thread` |
| sends the first reply | sends the first reply at the same instant | one thread with both replies | one thread on both screens (`MURMURE-108`) | `ambre_murmure::thread_tests::two_first_replies_converge_on_the_thread_the_relay_linked` |
| replies, and sends to the channel too | watches the channel | the reply in the thread, and a copy in the channel | the reply in the channel, with a link to its thread (`MURMURE-109`) | `ambre_murmure::thread_tests::a_reply_also_sent_to_the_channel_writes_a_copy_naming_its_thread` |
| deletes that reply | watches the channel | the reply and its copy, both deleted | the text gone from the thread and the channel (`MURMURE-539`) | `ambre_logic::channels::tests::a_copy_follows_the_edit_and_the_delete_of_its_reply` |

---

## 3. MODEL

**CRDT containers.** the thread document of `ambre-spec-murmure-schema.md section 3`: its `header`
names the root and its segment, and its `messages` list holds the replies.

**On screen.** A thread shows its root and its replies through the message list of a channel, in a
canvas of its own (`MURMURE-420`, `MURMURE-421`). Reactions, attachments, edits and selection work
there as in the channel, with no second renderer (`MURMURE-485`).

**The first reply.** The client creates a thread document, and the relay registers it for the
root (`SRV-213`). The reply waits until the document went up whole. The link sets the `thread`
field of the root (`MURMURE-107`). Of two first replies, the loser deletes its own (`MURMURE-108`).
Any other member opens the thread the root names (`MURMURE-106`).

**A reply sent to the channel too.** Its copy reads a line saying so (`MURMURE-532`). Its reply
action opens the thread of the loaded root that names its thread document, and no other
(`MURMURE-533`).

**A copy of a far root.** A probe reads the segment the thread's header names, then closes. The
older segments load one at a time, so the list keeps no gap, down to that one (`MURMURE-545`,
`MURMURE-546`, `MURMURE-547`). The cost is the thread fetched twice, and each segment between.
A seek with no root says so once; replying again seeks anew (`MURMURE-548`).

**A copy follows its reply.** Its identifier is the reply's, the last bit turned. An edit of the
reply reaches the copy (`MURMURE-538`), and so does a delete (`MURMURE-539`), as an amendment once
its segment is locked. A copy lies after its root, so a held root leaves it held.

**A locked root.** No delta may set its `thread` field. The first reply waits for the amendments
of its segment, and the link records the thread there, under the root's identifier
(`MURMURE-484`). Any member writes that entry once. A fold carries it into the root's field.

**A thread not open.** A root is on screen when a segment the open channel holds carries it. Its
thread is then held, so each delta reloads the head (`MURMURE-111`). A delta on any other thread
makes the relay publish a signal on the topic of the channel. The client
watches it from its read cursor, and reads the sealed summaries alone (`MURMURE-260`,
`MURMURE-112`). A channel never opened in the session watches none of its threads.

**What a reply counts.** A mention in a reply counts on the channel of its thread, and can notify
(`MURMURE-261`). A reply counts in no unread count of the channel, as in Slack. Its unread
replies belong to the Threads view. The thread pane marks its thread read once its last reply
shows, as the channel does for its latest message (`MURMURE-497`).

**Followed threads.** The `followed` map of the user settings names each thread the user follows
(`MURMURE-291`). A followed thread is not subscribed: its activity signals and their summaries
count its unread replies, as for any thread not open (`MURMURE-260`). The Threads view gathers the
followed threads with unread replies (`MURMURE-292`). A thread never archives on a delay
(`MURMURE-290`).

**Reading a thread.** Its cursor moves to the last reply this client was delivered, so its
mentions stop counting (`MURMURE-293`).

**Code.** `ChannelState::reply_target`, `thread_linked`, `thread_failed`, `threads_to_open`,
`thread_view` and `reply` are in `crates/ambre-murmure/src/channel/threading.rs`. The Logic side
is `crates/ambre-logic/src/threads.rs`, and the Sync side is `thread_task` in
`crates/ambre-sync/src/sharing.rs`. The thread pane draws through `crates/ambre-murmure/src/view.rs`.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| reply on a message | the thread opens beside the channel, caret in its composer |
| "also send to channel" in a thread composer | the reply, and its copy in the channel |
| a reply count under a root | the thread opens |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One document per thread | one `threads` document per channel | it grows with every thread of the channel for years |
| The relay registers one thread document per root | each peer keeps the thread it created | two first replies at once would open two threads for one root |
| The client creates the thread document | the relay creates it on the first reply | the client already creates every document it writes, under its own key |
| The client fetches the summaries a signal points at | a signal that names the users mentioned | the relay would learn who mentions whom in every thread |
| A followed thread stays until unfollowed | an archive after a fixed delay | Discord users find the delay too short, and lose threads they meant to keep |
| Followed threads live in the user settings | local storage in the browser | a new device would start with no followed thread |
| The copy's identifier derives from its reply's | a key on the copy naming its reply | a new key raises the frozen schema version, and an older build refuses every newer document whole |
| The author writes the change to both | the copy reads its text through the reply | a member who holds no thread would still read a deleted text |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A lost root names no cause.** One notice covers a failed segment and a header naming none.
  Telling them apart is not ruled.
* **An open channel holds every thread its loaded segments name.** A root scrolled off screen
  keeps its thread held. Releasing it is not ruled.
