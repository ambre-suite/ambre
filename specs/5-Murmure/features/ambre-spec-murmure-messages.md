# AMBRE -- FEATURE: MESSAGES
**Type:** Vertical (feature)
**Implementation:** partial -- the send and the authorship check are built; the edit and the delete are not

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

### 1.2 Send a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-080` | `POST` | A sent message MUST be appended to the active segment with a fresh `MessageId` and its author. | this spec section 3; `MURMURE-002` | `ambre_murmure::channel_tests::a_sent_message_lands_in_the_active_segment_with_a_fresh_id_and_its_author`; `ambre_murmure::channel_tests::a_sent_delta_brings_a_peer_to_the_same_messages`; `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- the author is the session's user | none | verified |
| `MURMURE-256` | `POST` | A sent message MUST create its `reactions` map, empty. | this spec section 3; `MURMURE-006` | `ambre_murmure::reaction_tests::a_reaction_adds_one_entry_and_leaves_the_message` -- the map of a sent message is there and empty | none | verified |
| `MURMURE-081` | `POST` | A sent message MUST hand the Sync worker an activity summary of kind content, with its mentions. | this spec section 3; `SYNC-113` | `ambre_murmure::channel_tests::a_sent_message_carries_a_content_summary_with_its_mentions`; `ambre_logic::channels::tests::a_channel_command_reaches_its_engine_and_leaves_as_a_delta_and_an_event` -- the summary leaves with the delta | none | verified |
| `MURMURE-082` | `FAIL` | Given a delta that creates a message under another author, every client MUST refuse the delta. | this spec section 3 | `ambre_murmure::authorship_tests::a_message_carries_the_author_who_signed_it_and_no_other` -- a message forged under another user is refused and changes nothing | none | verified |
| `MURMURE-083` | `FRAME` | Sending a message MUST NOT change another message. | this spec section 3 | `ambre_murmure::channel_tests::sending_a_message_changes_no_other_message` -- the deep value of every earlier message compares equal across the send | none | verified |

### 1.3 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-084` | `POST` | An edit MUST change the content of the message and mark it edited. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-085` | `FAIL` | Given a delta that edits a message of another author, every client MUST refuse the delta. | this spec section 3 | `ambre_murmure::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing`; `ambre_murmure::authorship_tests::a_later_session_of_the_author_edits_its_own_message` -- the author's later PeerID passes and another user's is refused | none | verified |
| `MURMURE-253` | `FAIL` | Given a delta that amends a message of another author, with no moderator standing behind it, every client MUST refuse the delta. | this spec section 3; `MURMURE-073`; `MURMURE-307` | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-086` | `FRAME` | Editing a message MUST NOT change its author, its identity or its reactions. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Delete a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-087` | `POST` | A delete MUST mark the message deleted and empty its content. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-088` | `POST` | A delete of a message in a locked segment MUST write an amendment with `by: author`. | this spec section 3; `features/ambre-spec-murmure-segments.md` | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-089` | `FAIL` | Given a delta that deletes a message of another author, with no moderator standing behind it, every client MUST refuse the delta. | this spec section 3; `MURMURE-307` | `ambre_murmure::authorship_tests::a_delete_of_another_authors_message_is_refused` -- no moderator standing exists yet, so every foreign removal is refused | none | verified |
| `MURMURE-090` | `FRAME` | Deleting a message MUST NOT delete its thread. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Receive a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-224` | `FAIL` | Given a binary delta on a segment, a thread or an amendments document, every client MUST refuse it. | this spec section 3; `MURMURE-223` | `ambre_murmure::authorship_tests::a_binary_delta_on_a_segment_is_refused` -- the segment is unchanged and the delta counts as delivered | none | verified |
| `MURMURE-225` | `POST` | An accepted delta MUST be imported exactly as the check read it. | this spec section 3 | type-level -- `apply_signed` imports the parsed update it checked, never the bytes again; `ambre_murmure::authorship_tests::a_later_session_of_the_author_edits_its_own_message` | none | verified |
| `MURMURE-255` | `POST` | The check MUST accept a delta that sets the absent `thread` field of another author's message, and nothing else of it. | this spec section 3; `MURMURE-107` | not built | unbuilt -- lands with the `Threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-228` | `FAIL` | Given a JSON update of a schema version the client does not compile, the client MUST refuse it and ask the user to update. | this spec section 3; `MURMURE-019` | `ambre_murmure::authorship_tests::a_json_update_of_another_version_is_refused_and_asks_an_update`; `ambre_logic::channels::tests::a_delta_from_a_newer_build_asks_the_user_to_update` -- the view gets `UpdateRequired` | none | verified |
| `MURMURE-226` | `FRAME` | Checking a delta MUST NOT copy its document. | this spec section 3; `ambre-spec-budgets.md section 1` | type-level -- `delta::check` takes the JSON update, a registry lookup and a reactions lookup, never a document | none | verified |

* **Budget.** the authorship check reads each received delta with no document, a row of
  `ambre-spec-budgets.md section 1`. A delta weighs about 1.4 times its binary form on the wire.
* **Boundary.** a message is sealed like any delta (`ZK-001`), and its kind rides in the sealed
  summary (`SYNC-113`). The relay never learns whether a delta is a message, an edit or a delete.
* **Verification.** 15 `verified`, 7 `unimplemented`: the send and the authorship check are built,
  and no edit or delete exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The segment holds | Each user sees | Test |
|---|---|---|---|---|
| sends a message | -- | one message, authored by Alice | the message, by Alice | none |
| edits her message | reacts to it at the same instant | the edit and the reaction | the edited message, with Bob's reaction (`MURMURE-086`) | none |
| -- | Mallory sends a delta that edits Alice's message | Alice's message unchanged | the message as Alice wrote it (`MURMURE-085`) | `ambre_murmure::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing` |
| deletes a message that has replies | -- | a deleted message, and its thread | the replies stay reachable (`MURMURE-090`) | none |

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

**Code.** the send is `ChannelState::send`, in `crates/ambre-murmure/src/channel.rs`. The check is
`delta::check`, in `crates/ambre-murmure/src/delta.rs`, and `ChannelState::apply_signed` runs it on
each received segment delta.

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

* **No edit and no delete are built.** The check refuses a foreign one, but no client writes one yet.
* **A segment is the only document the check reads.** The thread and amendments documents do not
  exist yet. Each lands with its own story and takes the same JSON path.
* **The JSON form costs bytes.** A message delta weighs 346 bytes compressed, against 214 for the
  binary form. The figure comes from zlib, and the envelope compresses with zstd.
