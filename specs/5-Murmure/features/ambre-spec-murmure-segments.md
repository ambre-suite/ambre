# AMBRE -- FEATURE: CHANNEL SEGMENTS AND FINALIZATION
**Type:** Vertical (feature)
**Implementation:** partial -- a full segment closes into its successor; no segment is finalized yet.

> A channel grows for years and still opens fast. Its history is cut into segments. A full segment
> closes, stays open to edits for a day, then becomes one immutable snapshot another member checks.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-062` | `INV` | A message sent live, and younger than the grace period, MUST NOT be in a final segment. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-063` | `INV` | A final snapshot MUST be shallow: it MUST hold no text deleted before the finalization. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-222` | `INV` | An active segment MUST NOT hold more deltas past its latest snapshot than the budget register allows. | this spec section 3; `ambre-spec-budgets.md section 6`; `SRV-038` | `ambre_server::compaction::tests::a_segment_is_asked_before_its_250th_delta` -- a segment past its snapshot is asked for a compaction before its 250th delta | none | verified |
| `MURMURE-239` | `OBS` | A dropped snapshot MUST be shown to the members of the channel, with its author and its reporter. | this spec section 3; `AUTHOR-023` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-435` | `INV` | Every Murmure document MUST take the delta order and the quarantine of the document plane. | this spec section 3; `ambre-spec-delta-order.md` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Close the active segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-064` | `POST` | A client MUST ask the relay to close the active segment once it passes the segment size of the budget register. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::closing_tests::a_full_segment_asks_to_close_once` -- the segment asks once, and again only after a refusal; `ambre_logic::channels::tests::a_full_segment_closes_into_the_successor_the_relay_names` -- the send that fills it asks the Sync worker for one close; `ambre_logic::channels::tests::a_failed_close_is_asked_again` -- a failed close asks again on the next send | none | verified |
| `MURMURE-065` | `POST` | A close MUST make the old segment `closing` and its successor `active`. | this spec section 3; `MURMURE-010` | `ambre_murmure::closing_tests::a_close_makes_the_segment_closing_and_its_successor_active` -- in the channel and in `meta`; `ambre_murmure::closing_tests::a_close_while_a_segment_is_closing_is_refused` -- a second close is refused; `ambre_logic::channels::tests::a_full_segment_closes_into_the_successor_the_relay_names` -- the answer leaves as a `meta` delta, and the successor takes the next message | none | verified |
| `MURMURE-066` | `FRAME` | Closing a segment MUST NOT move a message from one segment to another. | this spec section 3 | `ambre_murmure::closing_tests::a_close_makes_the_segment_closing_and_its_successor_active` -- every message stays where it was; `ambre_murmure::closing_tests::a_remote_close_is_followed_once_its_successor_arrives` -- a member following the close keeps sending to the old segment until the successor is in; `ambre_logic::channels::tests::a_remote_close_fetches_its_successor` -- the Logic thread fetches it once | none | verified |
| `MURMURE-481` | `POST` | A client MUST ask to close the active segment once its bytes pass the size of the budget register, whatever its message count. | this spec section 3; `ambre-spec-budgets.md section 6`; `MURMURE-064` | not built | unbuilt -- lands with the `A segment close recovers from its failures` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-482` | `FAIL` | Given a successor whose open fails, the client MUST fetch it again, never write to it before it opens. | this spec section 3; `MURMURE-066` | not built | unbuilt -- lands with the `A segment close recovers from its failures` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Finalize a closing segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-067` | `POST` | Once the grace period of a closing segment ends, a member client MUST ask for its finalize lease. | this spec section 3; `SRV-113` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-068` | `POST` | The lease holder MUST upload a shallow snapshot of the deltas up to the number the relay returns. | this spec section 3; `SRV-114` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-069` | `POST` | The lease holder MUST mark the segment `final` in `meta` once the relay accepts the commit. | this spec section 3; `SRV-117` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-070` | `FRAME` | Finalizing a segment MUST NOT change the content of a message. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Confirm a finalization

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-071` | `POST` | A confirming client MUST rebuild the state from the deltas and compare it with the snapshot before it confirms. | this spec section 3; `SRV-118` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-072` | `FAIL` | Given a snapshot that differs from the deltas, the client MUST withhold its confirmation and report the mismatch. | this spec section 3 | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-238` | `POST` | A dropped final snapshot MUST let another member client finalize the segment again. | this spec section 3; `SRV-133`, `SRV-134` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-153` | `FRAME` | Confirming a finalization MUST NOT change the snapshot or the deltas. | this spec section 3 | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Edit a message of a locked segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-073` | `POST` | An edit the relay refuses for a locked segment MUST be rewritten as an amendment. | this spec section 3; `SRV-115` | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-074` | `FRAME` | The rewrite MUST NOT drop the edit. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-263` | `FAIL` | Given a new message the relay refuses for a locked segment, the client MUST send it again to the active segment. | this spec section 3; `SRV-115` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Re-finalize an amended segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-075` | `POST` | A re-finalization MUST fold the amendments into a new shallow snapshot. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-076` | `POST` | A re-finalization MUST remove from the amendments document the entries it folded in. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-077` | `FRAME` | A re-finalization MUST NOT drop an amendment written after the fold began. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Fill a gap in a final segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-436` | `FAIL` | Given a gap in the deltas of a final segment, the client MUST load its final snapshot, never wait for the purged deltas. | this spec section 3; `SYNC-031`; `SRV-118` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the grace period, the lease and the erasure delay are rows
  of `ambre-spec-budgets.md section 8`. The segment size row of section 6 bounds every snapshot here,
  and the row beside it bounds the deltas a cold open replays.
* **Boundary.** a final snapshot is sealed like any snapshot (`ZK-001`). The relay learns the state
  of each segment and its finalization time (`ZK-015`).
* **Verification.** 4 `verified`, 21 `unimplemented`. The engine tests hold the close on one peer
  and on two. The Logic tests hold the request, the answer and the fetch of a remote successor. The
  relay holds the count that asks for a compaction. No lease, amendment or final snapshot exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| posts the message that fills the segment | posts at the same instant | one successor segment | both messages, in order (`MURMURE-011`) | `ambre_murmure::closing_tests::two_writers_of_one_close_converge_on_one_successor` |
| fixes a typo an hour later | -- | the edit in the closing segment | the fixed message, with no amendment (`MURMURE-062`) | none |
| edits a message of last month | -- | an entry in the amendments document | the edited message | none |
| -- | Mallory finalizes a snapshot that drops Alice's message | the deltas, kept | Bob's client refuses to confirm, and the deltas stay (`MURMURE-072`) | none |
| edits offline, and reconnects after the lock | -- | an amendment, not a lost edit | the edit on both screens (`MURMURE-073`) | none |

---

## 3. MODEL

**CRDT containers.** the segment and amendments documents, and the `segments` list of `meta`, of
`ambre-spec-murmure-schema.md section 3`.

**The life of a segment.**

| State | Accepts | Leaves it when |
|---|---|---|
| `active` | new messages, edits, deletes | it passes the segment size, and the relay closes it |
| `closing` | edits and deletes | the grace period ends and a lease is granted, or a rotation locks it (`MURMURE-262`) |
| `final` | nothing: the amendments document takes late edits | never |

**An active segment is compacted by count, never on a timer.** The relay asks a member client for
a snapshot once the deltas past the latest one pass their budget (`SRV-038`, `MURMURE-222`). A
quiet channel is never compacted for nothing, and a busy one is compacted as often as it writes. The
snapshot is full, not shallow: a client may still send a change made before it. It stays pending
until another member confirms it, as any snapshot does (`SRV-126`, `AUTHOR-016`).

**A finalization, in steps.**

1. A member client asks for the lease, and the relay locks the segment (`SRV-113`, `SRV-114`).
2. The client exports a shallow snapshot of the deltas up to the number the relay returned.
3. It uploads the snapshot and its search index, and commits (`SRV-116`).
4. Another member client confirms, and the deltas go (`SRV-118`). A segment nobody opens again
   keeps its deltas: about 1.8 MB beside a 0.6 MB snapshot, for 5000 messages.

**A mismatch** drops the snapshot, and the deltas stay (`SRV-133`). Another member finalizes
again (`MURMURE-238`). Every member sees who sealed the snapshot and who refused it
(`MURMURE-239`). A member who refuses honest snapshots is named just as plainly.

**Delta order.** A segment, a thread, `meta` and the amendments document are documents of the
plane. Each takes the order and the quarantine of `ambre-spec-delta-order.md` (`MURMURE-435`). A
final segment differs in one point: once a member confirms it, its deltas are gone. A client that
misses some of them loads the final snapshot, which holds them all (`MURMURE-436`). The quarantine
waits for no delta the relay has purged.

**A re-finalization** runs rarely and in batches. It is what turns a delete into an erasure: the
deleted text leaves the ciphertext only then.

**A close, in steps.**

1. A send that fills the active segment asks the Sync worker to close it (`MURMURE-064`).
2. The worker creates a successor and asks the relay to close into it (`SRV-194`).
3. The relay names the one successor. A client that lost the race deletes its own (`SRV-195`).
4. The Logic thread writes the close into `meta`, and opens the successor (`MURMURE-065`).
5. Every other member reads the close in `meta`, and fetches the successor once (`MURMURE-066`).

**Code.** `ChannelState::wants_close`, `close_into` and `successor_to_open`, in
`crates/ambre-murmure/src/channel.rs`. The Logic side is `crates/ambre-logic/src/segments.rs`, and
the request is `close_segment_task` in `crates/ambre-sync/src/sharing.rs`.

---

## 4. SURFACE

The segments have no control of their own. A user meets them only as a channel that stays fast.

| Trigger | Behaviour |
|---|---|
| a segment fills | the relay closes it, and nothing changes on screen |
| the grace period ends | a member client finalizes it in the background |
| an edit of an old message | an amendment, shown as an edit |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Closing and finalizing are two acts | finalizing at the size threshold | the last message before the cut would land in a final segment, and its first typo fix would create an amendments document |
| The deltas stay until a second client confirms | purging at the finalization | a false snapshot from a malicious member would lose messages for good |
| A shallow final snapshot | a full snapshot with its history | the history keeps the text of every deleted message in the ciphertext |
| The relay arbitrates the finalization | a lock the clients take | a client does not reach the relay store, so the relay holds the lease |
| An active segment is compacted by count | a compaction every one or two minutes | a quiet channel would upload a snapshot for nothing, and a busy one could still pile up deltas between two ticks |
| A full snapshot while the segment is active | a shallow snapshot at each compaction | a client that sends a change made before the snapshot could no longer merge it |
| A compaction bounds the deltas past the snapshot | smaller segments instead | every close adds a lease, a final snapshot and a search index, and a smaller segment still replays its whole tail |
| The deltas wait for a confirmation | purging them after a verification delay | a false snapshot nobody checked in time would become the only record of the segment |
| Both names are shown | naming the author alone | a false report would then accuse an honest member, and nobody would see who made it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No segment is finalized.** No lease, amendment or re-finalization exists. A channel closes one
  segment, and refuses a second close while the first is still `closing`.
* **The size is counted in messages.** The 2 MB bound of the segment size row is not measured
  (`MURMURE-481`).
* **A successor may be fetched before its first update.** Its open then fails, and the member keeps
  writing to the old segment until the next `meta` delta (`MURMURE-482`).
* **A failed close leaves its successor on the relay.** Only a lost race deletes it. An orphan holds
  no message and no member reads it (`SYNC-164`).
* **A closed channel counts what its `meta` names.** After a key rotation, a new segment counts its
  activity alone. A `meta` write from another device of the same user counts nothing.
