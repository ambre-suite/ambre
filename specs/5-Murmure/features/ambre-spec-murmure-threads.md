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
| `MURMURE-105` | `INV` | The replies to a message MUST live in one thread document per root message. | this spec section 3; `ambre-spec-murmure-schema.md section 2`; `SRV-213` | `ambre_murmure::thread_tests::every_reply_to_a_root_writes_its_one_thread_document` -- a first reply creates the thread, later ones write it, and another root has its own; `::a_root_of_a_locked_segment_starts_no_thread` -- a root no delta may write starts none; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the reply leaves for the thread once it went up whole | none | verified |
| `MURMURE-106` | `INV` | The reply count and the last replier of a root MUST be read from its thread document. | this spec section 3 | `ambre_murmure::thread_tests::the_root_reads_its_reply_count_and_last_replier_from_its_thread` -- a deleted reply aside, and a peer without the thread reads none; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the root's head counts the reply; `::a_root_another_member_threaded_opens_its_thread` -- a member who did not link the thread opens it and reads its replies | none | verified |
| `MURMURE-108` | `CONC` | Two peers sending the first reply at once MUST converge on one thread document. | this spec section 3; `ambre-spec-murmure-schema.md section 6`; `SRV-214` | `ambre_murmure::thread_tests::two_first_replies_converge_on_the_thread_the_relay_linked` -- the loser drops its own, and both roots name one document; `ambre_logic::channels::tests::a_lost_or_failed_thread_link_moves_the_reply_to_another_document` -- the reply waits on the winner, and a failed link creates again | none | verified |
| `MURMURE-261` | `INV` | A mention in a reply MUST count on the channel of its thread. | this spec section 3; `MURMURE-233` | not built | unbuilt -- lands with the `Threads not open` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-290` | `INV` | A followed thread MUST stay in the Threads view until the user unfollows it, never leaving on a delay. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Reply in a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-107` | `POST` | The first reply to a message whose `thread` field is absent MUST set it to the thread document. | this spec section 3 | `ambre_murmure::thread_tests::the_link_sets_the_root_thread_field_and_nothing_else` -- the link writes the field, and a second link writes nothing | none | verified |
| `MURMURE-109` | `POST` | A reply also sent to the channel MUST write a copy in the active segment that names its thread. | this spec section 3 | `ambre_murmure::thread_tests::a_reply_also_sent_to_the_channel_writes_a_copy_naming_its_thread` -- the copy reads the reply's text and its thread; `ambre_logic::channels::tests::a_first_reply_waits_for_the_thread_it_creates` -- the copy leaves under the active segment | none | verified |
| `MURMURE-110` | `FRAME` | Replying MUST NOT change the root message beyond its `thread` field. | this spec section 3 | `ambre_murmure::thread_tests::the_link_sets_the_root_thread_field_and_nothing_else` -- the delta of the link holds one operation, on that field | none | verified |
| `MURMURE-484` | `POST` | The first reply to a message of a locked segment MUST record its thread in the amendments of that segment. | this spec section 3; `MURMURE-013` | not built | unbuilt -- lands with the `A thread on a locked segment` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-485` | `POST` | An edit, a delete or a reaction on a reply MUST write in its thread document. | this spec section 3; `MURMURE-084`, `MURMURE-091` | `ambre_murmure::thread_tests::an_edit_a_reaction_and_a_delete_on_a_reply_write_in_its_thread` -- each change writes the thread, and a delete leaves the count; `::another_member_reacts_to_a_reply_and_never_edits_it` -- the check takes a reaction, and an edit is refused; `ambre_logic::channels::tests::a_change_on_a_reply_leaves_for_its_thread` -- each change leaves under the thread | none | verified |

### 1.3 Follow a thread that is not open

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-111` | `POST` | A client MUST reload the head of a thread when the relay signals activity on it and its root is on screen. | this spec section 3; `SRV-161` | not built | unbuilt -- lands with the `Threads not open` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-260` | `POST` | An activity signal on a thread MUST make the client fetch the summaries of its new deltas, with no document loaded. | this spec section 3; `RESID-014`, `SRV-161` | not built | unbuilt -- lands with the `Threads not open` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-112` | `FRAME` | An activity signal MUST NOT load a thread whose root is off screen. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Threads not open` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Follow a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-291` | `POST` | Writing the root, replying, or being mentioned in a thread MUST follow it. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-292` | `POST` | The Threads view MUST list every followed thread with unread replies, newest first. | this spec section 3; `MURMURE-260` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-293` | `POST` | Marking a thread read MUST move its read cursor to its last reply. | this spec section 3; `MURMURE-114` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-294` | `POST` | Unfollowing a thread MUST remove it from the Threads view and from the user settings. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-415` | `FAIL` | Given a followed thread whose document expired, the client MUST drop it from the Threads view and from the user settings. | this spec section 3; `SRV-156` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-295` | `FRAME` | Following a thread MUST NOT subscribe the client to it. | this spec section 3; `MURMURE-112` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a thread head is a small read per visible root, inside the memory rows of
  `ambre-spec-budgets.md section 6`.
* **Boundary.** a thread is a document of its channel, sealed like the others (`ZK-001`). The relay
  learns that a document is a thread of the channel, the identifier of its root, and when it
  changes (`ZK-015`, `SRV-162`).
* **Verification.** 7 `verified`, 12 `unimplemented`. The engine tests hold the thread document,
  the link, the count and the changes on a reply. The Logic tests hold the wait and the race.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| replies to Bob's message | watches the channel | a thread document, and `thread` on the root | "1 reply" under the root, on both screens (`MURMURE-106`) | `ambre_logic::channels::tests::a_root_another_member_threaded_opens_its_thread` |
| sends the first reply | sends the first reply at the same instant | one thread with both replies | one thread on both screens (`MURMURE-108`) | `ambre_murmure::thread_tests::two_first_replies_converge_on_the_thread_the_relay_linked` |
| replies, and sends to the channel too | watches the channel | the reply in the thread, and a copy in the channel | the reply in the channel, with a link to its thread (`MURMURE-109`) | `ambre_murmure::thread_tests::a_reply_also_sent_to_the_channel_writes_a_copy_naming_its_thread` |

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

**A locked root.** No delta may set its `thread` field, so a first reply is refused for now
(`MURMURE-484`).

**A thread not open.** The client is not subscribed to it. A delta on it makes the relay publish a
signal on the topic of the channel. The client then fetches the new deltas and reads their sealed
summaries alone (`MURMURE-260`). A mention in a reply thus counts on the channel, and can notify
(`MURMURE-261`). It reloads the head of the thread only if the root is on screen.

**Followed threads.** The `followed` map of the user settings names each thread the user follows
(`MURMURE-291`). A followed thread is not subscribed: its activity signals and their summaries
count its unread replies, as for any thread not open (`MURMURE-260`). The Threads view gathers the
followed threads with unread replies (`MURMURE-292`). A thread has a read cursor like any document,
so marking it read moves that cursor (`MURMURE-293`). A thread never archives on a delay
(`MURMURE-290`).

**Code.** `ChannelState::reply_target`, `thread_linked`, `thread_failed`, `threads_to_open`,
`thread_view` and `reply` are in `crates/ambre-murmure/src/channel/threading.rs`. The Logic side
is `crates/ambre-logic/src/threads.rs`, and the Sync side is `thread_task` in
`crates/ambre-sync/src/sharing.rs`.

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

---

## 6. NOT BUILT, AND NOT YET RULED

* **A reply counts in no unread count.** The count reads the segments alone. Replies count with
  the `Threads not open` Story (`ambre-roadmap.md` E5).
* **An open channel holds every thread its roots name.** A thread loads only for a root on
  screen with the `Threads not open` Story (`ambre-roadmap.md` E5).
