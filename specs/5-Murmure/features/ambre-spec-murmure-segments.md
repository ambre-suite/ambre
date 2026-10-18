# AMBRE -- FEATURE: CHANNEL SEGMENTS AND FINALIZATION
**Type:** Vertical (feature)
**Implementation:** partial -- a full segment closes and finalizes; no finalization is confirmed yet.

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
| `MURMURE-062` | `INV` | A message sent live, and younger than the grace period, MUST NOT be in a final segment. | this spec section 3; `ambre-spec-budgets.md section 8` | `ambre_murmure::finalizing_tests::a_closing_segment_asks_for_its_lease_as_the_relay_says` -- the client asks no earlier than the relay's deferral, which holds the grace period (`SRV-199`); `ambre_logic::channels::tests::a_closing_segment_finalizes_once_its_horizon_is_in` -- the ticks within the deferral ask nothing | none | verified |
| `MURMURE-063` | `INV` | A final snapshot MUST be shallow: it MUST hold no text deleted before the finalization. | this spec section 3 | `ambre_murmure::finalizing_tests::the_lease_holder_seals_a_shallow_snapshot_at_the_horizon` -- a body deleted before the seal is in the full export, never in the snapshot | none | verified |
| `MURMURE-222` | `INV` | An active segment MUST NOT hold more deltas past its latest snapshot than the budget register allows. | this spec section 3; `ambre-spec-budgets.md section 6`; `SRV-038` | `ambre_server::compaction::tests::a_segment_is_asked_before_its_250th_delta` -- a segment past its snapshot is asked for a compaction before its 250th delta | none | verified |
| `MURMURE-239` | `OBS` | A dropped snapshot MUST be shown to the members of the channel, with its author and its reporter. | this spec section 3; `AUTHOR-023` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-435` | `INV` | Every Murmure document MUST take the delta order and the quarantine of the document plane. | this spec section 3; `ambre-spec-delta-order.md` | not built | unbuilt -- lands with the `A finalization is confirmed` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Close the active segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-064` | `POST` | A client MUST ask the relay to close the active segment once it passes the segment size of the budget register. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::closing_tests::a_full_segment_asks_to_close_once` -- the segment asks once, and again only after a refusal; `ambre_logic::channels::tests::a_full_segment_closes_into_the_successor_the_relay_names` -- the send that fills it asks the Sync worker for one close; `ambre_logic::channels::tests::a_failed_close_is_asked_again` -- a failed close asks again on the next send | none | verified |
| `MURMURE-065` | `POST` | A close MUST make the old segment `closing` and its successor `active`. | this spec section 3; `MURMURE-010` | `ambre_murmure::closing_tests::a_close_makes_the_segment_closing_and_its_successor_active` -- in the channel and in `meta`; `ambre_murmure::closing_tests::a_close_while_a_segment_is_closing_is_refused` -- a second close is refused; `ambre_logic::channels::tests::a_full_segment_closes_into_the_successor_the_relay_names` -- the answer leaves as a `meta` delta, and the successor takes the next message | none | verified |
| `MURMURE-066` | `FRAME` | Closing a segment MUST NOT move a message from one segment to another. | this spec section 3 | `ambre_murmure::closing_tests::a_close_makes_the_segment_closing_and_its_successor_active` -- every message stays where it was; `ambre_murmure::closing_tests::a_remote_close_is_followed_once_its_successor_arrives` -- a member following the close keeps sending to the old segment until the successor is in; `ambre_logic::channels::tests::a_remote_close_fetches_its_successor` -- the Logic thread fetches it once | none | verified |
| `MURMURE-481` | `POST` | A client MUST ask to close the active segment once its bytes pass the size of the budget register, whatever its message count. | this spec section 3; `ambre-spec-budgets.md section 6`; `MURMURE-064` | `ambre_murmure::closing_tests::a_segment_past_its_bytes_asks_to_close_whatever_its_count` -- long messages ask at the send that passes the bound, far below the count | none | verified |
| `MURMURE-482` | `FAIL` | Given a successor whose open fails, the client MUST fetch it again, never write to it before it opens. | this spec section 3; `MURMURE-066` | `ambre_murmure::closing_tests::a_successor_that_fails_to_open_is_fetched_again` -- past a delay that doubles, the old segment taking every message meanwhile; `::a_successor_is_fetched_again_four_times_at_most` -- the fifth failure gives up; `ambre_logic::channels::tests::a_tick_fetches_again_a_successor_that_failed_to_open` -- the tick asks the Sync worker | none | verified |

### 1.3 Finalize a closing segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-067` | `POST` | Once the grace period of a closing segment ends, a member client MUST ask for its finalize lease. | this spec section 3; `SRV-113` | `ambre_murmure::finalizing_tests::a_closing_segment_asks_for_its_lease_as_the_relay_says` -- the first tick past the close asks, then each deferral is waited out; `::a_withdrawn_copy_never_asks_for_the_lease` -- a withdrawn copy stops; `::a_lease_that_runs_out_is_asked_again` -- an expired lease is asked again; `ambre_sync::finalize::tests::a_lease_request_reads_the_grant_or_the_deferral` -- the relay's answer reads as a lease or a deferral; `ambre_logic::channels::tests::a_withdrawn_copy_stops_and_a_failed_one_asks_again` -- a failure asks again past its delay | none | verified |
| `MURMURE-068` | `POST` | The lease holder MUST upload a shallow snapshot of the deltas up to the number the relay returns. | this spec section 3; `SRV-114` | `ambre_murmure::finalizing_tests::the_lease_holder_seals_a_shallow_snapshot_at_the_horizon` -- the snapshot waits for every delta up to the horizon; `ambre_sync::finalize::tests::a_commit_uploads_the_snapshot_at_the_horizon_then_names_it` -- it uploads covering the horizon, and the commit names it; `ambre_sync::updates::tests::a_copy_the_relay_lacks_an_edit_of_never_finalizes` -- a copy the relay lacks an edit of seals nothing; `ambre_logic::channels::tests::a_closing_segment_finalizes_once_its_horizon_is_in` -- the tick hands the snapshot over once the horizon is in | none | verified |
| `MURMURE-069` | `POST` | The lease holder MUST mark the segment `final` in `meta` once the relay accepts the commit. | this spec section 3; `SRV-117` | `ambre_murmure::finalizing_tests::an_accepted_commit_marks_the_segment_final_for_every_member` -- `meta` names it final, and a second member settles it from the delta; `ambre_logic::channels::tests::a_closing_segment_finalizes_once_its_horizon_is_in` -- the mark leaves as a `meta` delta | none | verified |
| `MURMURE-070` | `FRAME` | Finalizing a segment MUST NOT change the content of a message. | this spec section 3 | `ambre_murmure::finalizing_tests::an_accepted_commit_marks_the_segment_final_for_every_member` -- the final segment reads the same messages on the holder, on a member who settles it, and on one who loads the snapshot | none | verified |

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
| `MURMURE-263` | `FAIL` | Given a new message the relay refuses for a locked segment, the client MUST send it again to the active segment. | this spec section 3; `SRV-115` | `ambre_murmure::closing_tests::a_message_refused_by_the_lock_moves_to_the_active_segment` -- the segment goes back to the relay's state, and the message moves with its identifier and reaction; `::a_refused_reaction_alone_moves_nothing` -- nothing moves, and the active segment is never rewound; `ambre_logic::channels::tests::a_message_refused_by_the_lock_leaves_for_the_active_segment` -- the moved message leaves for the active segment, and the view moves it | none | verified |

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
* **Verification.** 13 `verified`, 12 `unimplemented`. The engine tests hold the close on one peer
  and on two, the byte bound, and the fetch again of a successor. They hold the lease, the shallow
  snapshot, the final mark and the message the lock refused. The Logic tests hold the close, each
  fetch of a remote successor, and the finalization driven by the tick. The Sync tests hold the
  lease, the upload and the commit.

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

1. A send that fills the active segment asks the Sync worker to close it. It is full by its count
   (`MURMURE-064`) or by its bytes (`MURMURE-481`).
2. The worker creates a successor and asks the relay to close into it (`SRV-194`). A close that
   failed is asked again with the same successor (`SYNC-164`).
3. The relay names the one successor. A client that lost the race deletes its own (`SRV-195`).
4. The Logic thread writes the close into `meta`, and opens the successor (`MURMURE-065`).
5. Every other member reads the close in `meta`, and fetches the successor once (`MURMURE-066`).
   An open that fails is fetched again on a later tick (`MURMURE-482`).

**A finalization, on the client.**

1. The first tick past the close asks for the lease (`MURMURE-067`). The relay answers how long
   the grace period still runs, and the client waits that long (`MURMURE-062`).
2. The holder seals a shallow snapshot once it holds every delta up to the horizon (`MURMURE-068`).
   A lease that runs out first is asked again.
3. A copy holding an edit the relay refused withdraws, and leaves the segment to another member.
   A copy whose edit is still unanswered asks again later.
4. The Sync worker uploads the snapshot at the horizon, then commits it under the lease.
5. The holder marks the segment `final` in `meta` (`MURMURE-069`). Every other member reads the
   mark and holds the segment as a final one, with the same messages (`MURMURE-070`).

**A message the lock refused.** A member may write to a closing segment after another took its
lease. The relay refuses that delta and every later one, so no other member holds them
(`SRV-115`). The Sync worker hands them back to the Logic thread (`SYNC-165`). The client forks
the segment at the version the relay holds, and drops its own later operations.

Each message they created is written again in the active segment, with its identifier, stamp,
body and its author's reactions (`MURMURE-263`). The view moves it by its identifier, so it never
shows twice.

**Code.** `ChannelState::wants_close`, `close_into`, `successor_to_open` and `successor_to_retry`,
in `crates/ambre-murmure/src/channel/closing.rs`. `ChannelState::lease_to_ask`, `final_snapshot`,
`mark_final` and `settle_final` are in `crates/ambre-murmure/src/channel/finalizing.rs`.
`ChannelState::relocate_locked` is in `crates/ambre-murmure/src/channel/relocating.rs`. The Logic
side is `crates/ambre-logic/src/segments.rs`. The requests are `close_segment_task` in
`crates/ambre-sync/src/sharing.rs`, and `take_lease` and `commit_final` in
`crates/ambre-sync/src/finalize.rs`.

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

* **No finalization is confirmed.** The deltas of a final segment stay, and no amendment or
  re-finalization exists. A channel refuses a second close while the first is still `closing`.
* **A final snapshot carries no search index.** The index lands with the `Search on a new device`
  Story (`ambre-roadmap.md` E5).
* **A reaction or an edit the lock refused alone is lost.** Only a new message moves to the active
  segment. The amendments of the `Message edits and deletes` Story (`ambre-roadmap.md` E5) carry the
  rest.
* **A successor fetched five times in vain is given up.** The member keeps writing to the old
  segment until the next `meta` delta names the successor again (`MURMURE-482`).
* **A closed channel counts what its `meta` names.** After a key rotation, a new segment counts its
  activity alone. A `meta` write from another device of the same user counts nothing.
