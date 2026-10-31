# AMBRE -- FEATURE: MESSAGES
**Type:** Vertical (feature)
**Implementation:** partial -- built in a live segment; the amendments are not

> A member writes a message, edits it, or deletes it. Nobody else can change it: every client
> refuses a change that does not come from its author.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-078` | `INV` | A message MUST carry the author who signed the delta that created it. | this spec section 3; `ambre-spec-op-authorship.md` | `ambre_murmure::authorship_tests::a_message_carries_the_author_who_signed_it_and_no_other` -- the received message reads the signer's user | none | verified |
| `MURMURE-079` | `CONC` | Two peers MUST accept and refuse the same deltas on a message. | this spec section 3; `AUTHOR-007` | `ambre_murmure::authorship_tests::two_peers_accept_and_refuse_the_same_deltas` -- two receivers reach the same verdicts and the same messages | none | verified |
| `MURMURE-223` | `INV` | A delta of a segment, a thread or an amendments document MUST travel as a Loro JSON update. | this spec section 3; `ambre-spec-murmure-schema.md section 7` | `ambre_murmure::authorship_tests::a_segment_delta_travels_as_a_json_update_of_the_pinned_version` -- a send and a first update both parse as JSON | none | verified |
| `MURMURE-227` | `INV` | A JSON update MUST carry the Loro JSON schema version the schema register pins. | this spec section 3; `ambre-spec-murmure-schema.md section 7` | `ambre_murmure::authorship_tests::a_segment_delta_travels_as_a_json_update_of_the_pinned_version` -- the update reads `JSON_SCHEMA_VERSION` | none | verified |
| `MURMURE-565` | `OBS` | A message in the list MUST draw each style of its body. | this spec section 3; `MURMURE-562` | `ambre_murmure::paint::tests::a_sent_body_draws_each_of_its_marks` -- bold widens the layout, code paints its tint under the words, the link alone takes the accent over a rule, past a renamed mention; `ambre_murmure::paint::tests::a_sent_block_draws_its_margin_marks` -- a quote draws one rule and a list one dot per item in the indent, a code block its tint and no margin mark | none | verified |
### 1.2 Send a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-080` | `POST` | A sent message MUST be appended to the active segment with a fresh `Id<Message>` and its author. | this spec section 3; `MURMURE-002` | `ambre_murmure::channel_tests::a_sent_message_lands_in_the_active_segment_with_a_fresh_id_and_its_author`; `ambre_murmure::channel_tests::a_sent_delta_brings_a_peer_to_the_same_messages`; `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- the author is the session's user | none | verified |
| `MURMURE-256` | `POST` | A sent message MUST create its `reactions` map, empty. | this spec section 3; `MURMURE-006` | `ambre_murmure::reaction_tests::a_reaction_adds_one_entry_and_leaves_the_message` -- the map of a sent message is there and empty | none | verified |
| `MURMURE-081` | `POST` | A sent message MUST hand the Sync worker an activity summary of kind content, with its mentions. | this spec section 3; `SYNC-113` | `ambre_murmure::channel_tests::a_sent_message_carries_a_content_summary_with_its_mentions`; `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- the summary leaves with the delta | none | verified |
| `MURMURE-082` | `FAIL` | Given a delta that creates a message under another author, every client MUST refuse the delta. | this spec section 3 | `ambre_murmure::authorship_tests::a_message_carries_the_author_who_signed_it_and_no_other` -- a message forged under another user is refused and changes nothing | none | verified |
| `MURMURE-083` | `FRAME` | Sending a message MUST NOT change another message. | this spec section 3 | `ambre_murmure::channel_tests::sending_a_message_changes_no_other_message` -- the deep value of every earlier message compares equal across the send | none | verified |

### 1.3 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-084` | `POST` | An edit MUST change the content of the message and mark it edited. | this spec section 3 | `ambre_murmure::edit_tests::an_edit_changes_the_body_and_marks_the_message_edited` -- here and at a peer, the marks following the draft; `::an_edit_with_a_mention_past_its_body_is_refused` -- nothing written; `ambre_logic::channels::tests::an_edit_and_a_delete_leave_as_edits_and_show_the_message` -- the delta leaves as an edit | none | verified |
| `MURMURE-085` | `FAIL` | Given a delta that edits a message of another author, every client MUST refuse the delta. | this spec section 3 | `ambre_murmure::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing`; `ambre_murmure::authorship_tests::a_later_session_of_the_author_edits_its_own_message` -- the author's later PeerID passes and another user's is refused | none | verified |
| `MURMURE-253` | `FAIL` | Given a delta that amends a message of another author, with no moderator standing behind it, every client MUST refuse the delta. | this spec section 3; `MURMURE-073`; `MURMURE-307` | `ambre_murmure::amending_tests::an_amendment_of_another_authors_message_is_refused` -- a delta that creates the entry, and one that writes into the author's own entry, are each refused and change nothing | none | verified |
| `MURMURE-086` | `FRAME` | Editing a message MUST NOT change its author, its identity or its reactions. | this spec section 3 | `ambre_murmure::edit_tests::an_edit_and_a_concurrent_reaction_both_hold` -- an edit crossing a reaction, both peers reading the same message | none | verified |

### 1.4 Delete a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-087` | `POST` | A delete MUST mark the message deleted and empty its content. | this spec section 3 | `ambre_murmure::edit_tests::a_delete_empties_the_body_and_marks_the_message_deleted` -- here and at a peer, then no edit and no second delete; `::an_edit_or_a_delete_of_a_message_not_ones_own_is_refused`; `ambre_logic::channels::tests::an_edit_and_a_delete_leave_as_edits_and_show_the_message` | none | verified |
| `MURMURE-088` | `POST` | A delete of a message in a locked segment MUST write an amendment with `by: author`. | this spec section 3; `features/ambre-spec-murmure-segments.md` | `ambre_murmure::amending_tests::an_edit_and_a_delete_of_a_locked_segment_write_amendments` -- the entry reads `by: author`, and the segment is unchanged; `::a_member_reads_the_amendments_over_the_segment` -- a second member reads the message deleted | none | verified |
| `MURMURE-089` | `FAIL` | Given a delta that deletes a message of another author, with no moderator standing behind it, every client MUST refuse the delta. | this spec section 3; `MURMURE-307` | `ambre_murmure::authorship_tests::a_delete_of_another_authors_message_is_refused` -- no moderator standing exists yet, so every foreign removal is refused | none | verified |
| `MURMURE-090` | `FRAME` | Deleting a message MUST NOT delete its thread. | this spec section 3 | `ambre_murmure::edit_tests::a_delete_keeps_the_thread_and_the_reactions` -- the `thread` field reads the same here and at a peer | none | verified |

### 1.5 Receive a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-224` | `FAIL` | Given a binary delta on a segment, a thread or an amendments document, every client MUST refuse it. | this spec section 3; `MURMURE-223` | `ambre_murmure::authorship_tests::a_binary_delta_on_a_segment_is_refused` -- the segment is unchanged and the delta counts as delivered | none | verified |
| `MURMURE-225` | `POST` | An accepted delta MUST be imported exactly as the check read it. | this spec section 3 | type-level -- `apply_signed` imports the parsed update it checked, never the bytes again; `ambre_murmure::authorship_tests::a_later_session_of_the_author_edits_its_own_message` | none | verified |
| `MURMURE-255` | `POST` | The check MUST accept a delta that sets the absent `thread` field of another author's message, and nothing else of it. | this spec section 3; `MURMURE-107` | `ambre_murmure::thread_tests::the_check_lets_another_member_set_an_absent_thread_field_alone` -- a second value, another field, or a field set on a reply are refused; `::a_reply_takes_no_thread_field_from_another_member` -- a reply of a thread takes no `thread` field | none | verified |
| `MURMURE-228` | `FAIL` | Given a JSON update of a schema version the client does not compile, the client MUST refuse it and ask the user to update. | this spec section 3; `MURMURE-019` | `ambre_murmure::authorship_tests::a_json_update_of_another_version_is_refused_and_asks_an_update`; `ambre_logic::channels::tests::a_delta_from_a_newer_build_asks_the_user_to_update` -- the view gets `UpdateRequired` | none | verified |
| `MURMURE-226` | `FRAME` | Checking a delta MUST NOT copy its document. | this spec section 3; `ambre-spec-budgets.md section 1` | type-level -- `delta::check` takes the JSON update, a registry lookup and a reactions lookup, never a document | none | verified |

### 1.6 Open a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-567` | `POST` | A click on a link in a message MUST open its address in a fresh tab, with no opener. | this spec section 3; `MURMURE-561` | `ambre_murmure::view::tests::a_click_on_a_link_opens_its_address_and_sends_nothing` -- a click and a tap on it, after a click beside it, a drag over it and a click on a refused address; `ambre_app::host::murmure::tests::a_clicked_link_opens_in_a_fresh_tab_with_no_opener` -- the page's `open` reads `noopener,noreferrer` | none | verified |
| `MURMURE-568` | `FRAME` | Opening a link MUST NOT send the Logic thread anything. | this spec section 3; `MURMURE-427` | `ambre_murmure::view::tests::a_click_on_a_link_opens_its_address_and_sends_nothing` -- no command follows the opens | none | verified |

* **Budget.** the authorship check reads each received delta with no document, a row of
  `ambre-spec-budgets.md section 1`. A delta weighs about 1.4 times its binary form on the wire.
* **Boundary.** a message is sealed like any delta (`ZK-001`), and its kind rides in the sealed
  summary (`SYNC-113`). The relay never learns whether a delta is a message, an edit or a delete.
* **Verification.** 25 `verified`. The send, the authorship check, the edit and the delete are
  built, live and as amendments. So is the `thread` field another member sets once. The list
  draws the styles of a body, and a click on a link opens it.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The segment holds | Each user sees | Test |
|---|---|---|---|---|
| sends a message | -- | one message, authored by Alice | the message, by Alice | none |
| edits her message | reacts to it at the same instant | the edit and the reaction | the edited message, with Bob's reaction (`MURMURE-086`) | `ambre_murmure::edit_tests::an_edit_and_a_concurrent_reaction_both_hold` |
| -- | Mallory sends a delta that edits Alice's message | Alice's message unchanged | the message as Alice wrote it (`MURMURE-085`) | `ambre_murmure::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing` |
| deletes a message that has replies | -- | a deleted message, and its thread | the replies stay reachable (`MURMURE-090`) | `ambre_murmure::edit_tests::a_delete_keeps_the_thread_and_the_reactions` |

---

## 3. MODEL

**CRDT containers.** the message map of `ambre-spec-murmure-schema.md section 3`: `id`, `author`,
`ts`, `content`, `attachments`, `thread`, `reactions`, `edited` and `deleted`.

**Every rule reads the delta alone.** A Loro container identifier carries the PeerID that created
it. The `author` of a message and the key of a reaction are values inside the operation. So the
check needs the operations of the delta, and never the state of the segment.

**A delta travels as a Loro JSON update** (`MURMURE-223`). A binary update opens only by an import
into a document, and a copy of the segment costs 52 ms at 5000 messages. The JSON form reads with
no document, in microseconds.

**The authorship check, in steps.** It runs on each received delta, before import.

1. Parse the JSON update. A binary delta, or an unknown schema version, is refused (`MURMURE-224`,
   `MURMURE-228`).
2. Read its operations: each carries its container and its value.
3. Refuse the delta when it creates a message under another author (`MURMURE-082`).
4. Refuse it when it changes a field, the `content` or the `attachments` of a message created by
   another author's PeerID (`MURMURE-085`, `MURMURE-089`). One field is open to all: the `thread`
   field, set once by the first reply, while it is absent (`MURMURE-255`). One change is open to a
   moderator: a removal whose standing the membership log proves (`MURMURE-307`).
5. Refuse it when it writes a reaction under another user's key (`MURMURE-094`). The `reactions`
   map is the one container of a message that others write, entry by entry. It comes with its
   message, and a delta that replaces it later is refused.
6. On an amendments document, read the message in its locked segment. Refuse the delta when the
   segment does not hold it, or when its `author` is another user (`MURMURE-013`, `MURMURE-253`).
7. Import otherwise, from the update the check read (`MURMURE-225`).

An amendment names its message by identifier, in a document its author did not create. So the
check reads the author from the locked segment itself, which is immutable and cheap to load again.

A PeerID resolves to its user through `ambre-spec-op-authorship.md`. Every client runs the same
check on the same delta, so every client reaches the same verdict (`MURMURE-079`).

**Moderation is the one exception.** Only the author changes a message, except a removal by a
moderator whose standing the membership log proves (`features/ambre-spec-murmure-moderation.md`).

**An edit and a delete in a live segment.** An edit replaces the body by its difference alone, writes
the marks again from the draft, and sets `edited`. A delete empties the body and sets `deleted`. It
leaves `thread` and `reactions` as they were. Both take a summary of kind edit, which names no one.

**The list draws the marks** (`MURMURE-565`). The shape of each mark is part of the layout's cache
key, and its ink and tint are painted on top. A renamed mention shifts the bytes, so each char
range is mapped onto the body as shown. A quote draws a rule and a list a dot per item, in the
indent its lines start past (`MURMURE-570`).

**A click opens a link** (`MURMURE-567`). The next paint looks for a link under a click or a tap
that moved nothing. It checks the address again, and the page opens it as a Toile link opens. The
open runs one frame after the gesture, inside the browser's window for a popup.

**Code.** the send is `ChannelState::send`, in `crates/ambre-murmure/src/channel.rs`, beside
`ChannelState::edit` and `ChannelState::delete`. The check is
`delta::check`, in `crates/ambre-murmure/src/delta.rs`, and `ChannelState::apply_signed` runs it on
each received segment delta. An amendment is written by `crates/ambre-murmure/src/channel/amending.rs`,
which runs step 6 on each received amendments delta.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| Enter in the composer | the message is sent |
| edit on one's own message | the message opens in the composer |
| up arrow in an empty composer | the last own message opens for edit |
| delete on one's own message | the message shows as deleted |
| the same actions on another member's message | not offered |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The check runs before import | a filter at display | the filter detects and prevents nothing: a last-writer-wins value is already overwritten |
| A delta travels as a JSON update | a fork of the segment per delta | a fork encodes and decodes the whole segment: 52 ms at 5000 messages on a desktop |
| A delta travels as a JSON update | a decoder of our own for the binary format | the binary format is internal to Loro, and it would break at each upgrade |
| A delta travels as a JSON update | smaller segments | a fork still costs 3.5 ms at 500 messages on a desktop, and the phone budget is 5 ms |
| A delete empties the content | a tombstone that keeps the text | the text would stay readable to every member, against the wish of its author |
| Only the author changes a message | a moderator role in the same check | a role lives in another document, and a revoke concurrent with a delete has no single answer without the relay |
| A late edit goes to an amendments document | new deltas on the final segment | every old segment would take concurrent writes again, and its search index would turn mutable |
| A late edit goes to an amendments document | a new final snapshot on each edit | a typo fix would take the finalize lease and a full index rebuild |
| Amendments sit beside their segment | amendments in the active segment | opening an old segment would need every later segment, to find its amendments |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A mention added by an edit notifies no one.** The edit summary lists no mention. Whether it
  should is not yet ruled.
* **The thread document does not exist yet.** The check reads segments and amendments documents.
  Threads land with their own story and take the same JSON path.
* **A replay waits for the channel's next tick.** The Logic thread has no clock of its own. A channel
  closed within that second drops the replay, and the log says so.
* **The JSON form costs bytes.** A message delta weighs 346 bytes compressed, against 214 for the
  binary form. The figure comes from zlib, and the envelope compresses with zstd.
