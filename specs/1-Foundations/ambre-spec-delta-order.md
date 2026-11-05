# AMBRE -- FEATURE: DELTA ORDER AND THE QUARANTINE

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- the ordering, the quarantine and the replay ship. Nothing ever asks
this client for a snapshot, so a parent adopted before its contents arrive is a theoretical case.
The missing-history request has no rate.

> A change made on one screen appears on another with the changes it was built on, or it waits. A
> user never sees half a document. An edit whose base is missing is held back until the base lands,
> and released in the order it was written.

Loro converges whatever the order, and that is not enough. A delta whose base snapshot this client
has never loaded would import into a document that is not the one it was written against. This
document owns what waits, what is released, and in what order.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-024` | `INV` | An outbound update MUST carry a clock strictly greater than every clock this client has already used for the document. | this spec section 3; `ambre-spec-sequencer.md`, `SRV-006` | `ambre_sync::updates::tests::each_update_takes_a_clock_above_every_one_used` -- two successive updates take two rising clocks, and a reopened document resumes above the echo of its own last frame | none | verified |
| `SYNC-030` | `INV` | A snapshot MUST NOT be registered as a known parent before its contents reach the Logic thread. | this spec section 3 | `ambre_sync::updates::tests::an_announced_snapshot_releases_nothing_until_fetched`; `ambre_sync::updates::tests::a_snapshot_that_does_not_open_stays_unknown` -- an announced snapshot releases no held delta until it is downloaded and opened | none | verified |
| `SYNC-026` | `CONC` | Every client MUST derive the same parent for a document that has never been snapshotted. | this spec section 3 | `ambre_core::id::tests::every_client_derives_the_same_genesis_parent` -- the derivation is pinned to the document's own bytes, so a change to it fails here | none | verified |

### 1.2 Receive an update

A sealed delta arrives from the relay. Either its base is one this client holds, or it waits.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-006` | `POST` | Released updates MUST be applied in parent-snapshot then clock order. | this spec section 3 | `ambre_sync::updates::tests::quarantine_released_in_clock_order` -- several updates held behind one unknown parent, and the release order read back | none | verified |
| `SYNC-082` | `POST` | An update whose parent snapshot is known MUST be delivered immediately. | this spec section 3 | `ambre_sync::updates::tests::update_with_known_snapshot_delivered_immediately` -- the delivery that makes the quarantine an exception rather than a stage every delta passes through | none | verified |
| `SYNC-007` | `POST` | A quarantine overflow MUST drop the oldest entry. | this spec section 3 | `ambre_sync::updates::tests::a_quarantine_overflow_drops_the_lowest_clock` -- the lowest clock arrives neither first nor last and waits on one of two parents, so a drop by arrival or by parent fails | none | verified |
| `SYNC-084` | `POST` | A quarantine overflow MUST emit exactly one warning. | this spec section 3 | `ambre_sync::updates::tests::quarantine_overflow_drops_oldest_and_warns` -- its warning count, one at the hard cap, so a warning per further arrival fails | none | verified |
| `SYNC-032` | `FRAME` | Receiving an update MUST NOT open one it quarantines. | `ZK-001`; this spec section 3 | `ambre_sync::updates::tests::a_quarantined_update_is_not_opened_on_arrival` -- an update sealed under a key this client lacks is held, and its arrival raises no failure to open | none | verified |
| `SYNC-101` | `FRAME` | A remote change MUST NOT be re-broadcast. | this spec section 3 | type-level -- Toile's and Prose's remote paths answer display events and no outcome, so neither has a field a delta could leave by; Prose `ambre_prose::logic::convergence_tests::a_remote_change_is_never_re_broadcast`; Toile `ambre_toile::logic::convergence_tests::a_remote_change_is_never_re_broadcast`; Trame `ambre_trame::logic::tests::group3::a_remote_change_is_never_re_broadcast` -- in each, the next local delta carries none of the remote peer's operations | none | verified |
| `SYNC-083` | `FAIL` | An update whose parent snapshot is unknown MUST be quarantined, never applied. | this spec section 3 | `ambre_sync::updates::tests::update_with_unknown_snapshot_quarantined` -- the update is held and the Logic thread is handed nothing | none | verified |

### 1.3 Open a document

The client asks for a document it may hold nothing of, part of, or all of.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-028` | `POST` | The post-open replay MUST start after the last delta the downloaded snapshot already contains. | this spec section 3 | `ambre_sync::updates::tests::open_document_ready_gates_request_missing_on_covered_seq`; `::open_document_ready_without_snapshot_requests_full_history` | none | verified |
| `SYNC-029` | `POST` | A document MUST be reported ready once per open, after its first replay has landed. | this spec section 3 | `ambre_sync::updates::tests::open_document_ready_deferred_to_initial_replay`; `::open_document_ready_immediate_without_replay`; `::document_ready_carries_the_role_on_both_paths` | none | verified |
| `SYNC-027` | `POST` | A history replay MUST reach the Logic thread as one batch, never as one event per delta. | this spec section 3 | `ambre_sync::updates::tests::a_history_replay_reaches_logic_as_one_batch` -- a replay of five deltas yields exactly one batch event carrying the five payloads in order, and no single-delta event | none | verified |
| `SYNC-131` | `POST` | The Logic thread MUST import a history replay in slices that each fit the frame budget, and yield between them. | this spec section 3; `ambre-spec-budgets.md section 2` | `ambre_logic::replay::tests::a_long_replay_lands_in_slices_and_yields_between_them` -- each pass imports at most a slice, the loop comes back unasked, and the end of the replay waits for the last slice | none | verified |
| `SYNC-031` | `POST` | The quarantine MUST ask the relay for the missing history once per gap, not once per arrival. | this spec section 3 | `ambre_sync::updates::tests::the_quarantine_asks_once_per_gap` -- arrivals held behind one gap send a single history request for it; `ambre_sync::updates::tests::a_gap_still_past_the_threshold_asks_once` -- a gap that stays at the threshold once one snapshot lands asks no second time | none | verified |
| `SYNC-085` | `FRAME` | Opening a document MUST NOT emit an update. | this spec section 3; derived | `ambre_sync::updates::tests::an_open_sends_no_update` -- with a snapshot to read or without one, the open sends a history request and no update | none | verified |

### 1.4 Reconnect

The socket dropped and came back. Everything the client missed while it was gone is still on the
relay.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-033` | `POST` | A reconnect MUST resubscribe every document that exists on the relay. | this spec section 3 | `ambre_sync::websocket::tests::a_reconnect_resubscribes_and_pulls_every_relay_document` -- the open event, driven over the mock transport, subscribes exactly the documents the relay created and skips the one it never saw | none | verified |
| `SYNC-086` | `POST` | A reconnect MUST pull what each document missed. | this spec section 3 | `ambre_sync::websocket::tests::a_reconnect_resubscribes_and_pulls_every_relay_document` -- each subscribed document is asked for what lies past its own cursor, so a pull from zero or from another document's cursor fails | none | verified |
| `SYNC-025` | `POST` | A reload MUST recover the clock from the relay's echo of this client's own updates. | this spec section 3 | `ambre_sync::updates::tests::a_reload_takes_its_clock_from_the_echo_of_its_own_update` -- the echo of its own clock-41 update moves the counter, a peer's clock 99 does not, and the next frame leaves with clock 42 | none | verified |
| `SYNC-087` | `FRAME` | A reconnect MUST NOT apply an update the client already applied. | this spec section 3; derived | `ambre_sync::updates::tests::a_reconnect_replay_applies_nothing_twice` -- a replay overlapping a broadcast already received delivers the overlap once, and a held copy is not held twice; `ambre_sync::updates::tests::the_applied_window_keeps_the_latest_numbers` -- the record of applied numbers keeps its window whole, and only the oldest leaves | none | verified |

### 1.5 Two sessions of one author write at once

Two tabs or two devices of one user sign with one author key, so they share one clock on the
relay. A delta each sends before it sees the other's echo takes a clock the other already used.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-147` | `POST` | A delta the relay refuses for its clock MUST be sealed again at the next free clock and sent again, with every delta sent after it, in order. | this spec section 3; `ambre-spec-sequencer.md`, `SRV-037`, `SRV-169` | `ambre_sync::updates::tests::a_delta_refused_for_its_clock_leaves_again_with_every_later_one` -- the answer leaves first, then the refused delta and the later one at the expected clock and the next, and a late refusal sends its answer alone; `ambre_sync::rekey::tests::a_clock_refusal_returns_the_delta_and_every_later_one` -- the log gives back the refused delta and every later one, and a refusal of a delta already taken back gives back nothing | none | verified |
| `SYNC-148` | `FRAME` | The echo of another session's delta MUST NOT confirm a delta this session sent at the same clock. | this spec section 3; `SYNC-147` | `ambre_sync::updates::tests::the_echo_of_another_sessions_delta_confirms_nothing_here` -- an echo of the same author and clock, under another signature, leaves the delta in the log; `ambre_sync::rekey::tests::an_echo_at_the_same_clock_from_another_session_confirms_nothing` -- the log confirms by signature alone | none | verified |

* **Budget.** the release of 100 quarantined updates under 1 ms, a row of
  `ambre-spec-budgets.md section 1`, and the two memory holders of
  `ambre-spec-budgets.md section 6`. No bench measures the release, which that first table records
  against the row itself.
* **Boundary.** nothing here opens a socket or holds a key of its own. Every message it orders is
  already sealed (`ambre-spec-sealed-envelope.md`). The parent snapshot, the clock and the sequence
  number it reads are the routing metadata `ZK-007` allows.
* **Verification.** 22 `verified`. What holds is the quarantine itself. An update ahead of its
  snapshot waits, and one whose base is known is delivered. The release runs in clock order, and
  the overflow drops the lowest clock and warns once. The clock recovery, the batched replay and the
  pull after a reconnect are driven over the mock transport, whose open event the host tier fires.

---

## 2. WHAT EACH USER SEES

Two peers over one document, and a relay that may deliver in any order. The last column joins the
example to its verification.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| edits twice | watches | both edits, in Alice's order | both edits in the order Alice made them | `updates::tests::update_with_known_snapshot_delivered_immediately` |
| edits after compacting | opens the document | the snapshot and the edit | Bob waits on the snapshot, then sees the edit | `updates::tests::quarantine_released_in_clock_order` |
| edits a thousand times while Bob's base is missing | watches | Alice's edits | Bob loses the oldest, and is told (`SYNC-007`) | `updates::tests::a_quarantine_overflow_drops_the_lowest_clock` |
| edits a document neither peer has snapshotted | opens it | Alice's edits | Bob sees them: both derived the same parent (`SYNC-026`) | none -- section 5 |
| edits while Bob is offline | reconnects | Alice's edits | Bob receives them as one repaint, not one per edit (`SYNC-027`) | `updates::tests::a_history_replay_reaches_logic_as_one_batch` |
| compacts the document | is editing it | the new snapshot | Bob keeps his base until the snapshot is downloaded and opened (`SYNC-030`) | `updates::tests::an_announced_snapshot_releases_nothing_until_fetched` |
| reloads the page and edits | watches | Alice's edit | Alice's edit reaches Bob rather than being refused (`SYNC-025`) | `updates::tests::a_reload_takes_its_clock_from_the_echo_of_its_own_update` |
| edits in two tabs at once | watches | both of Alice's edits | Bob sees both, in the order Alice's tabs sent them; the refused one arrives a round trip later (`SYNC-147`) | `updates::tests::a_delta_refused_for_its_clock_leaves_again_with_every_later_one` |

---

## 3. MODEL

**CRDT containers.** none. This document orders deltas and never reads one: what a delta means is
each application's engine.

**What a delta is parented on.** Every outbound update names the snapshot it was written against
and a clock. The pair is what makes ordering decidable without opening the payload.

| Field | Set by | Read by |
|---|---|---|
| the parent snapshot | this client, from the base it currently holds | the receiver, to decide waiting against applying |
| the clock | this client, one per outbound update, strictly increasing | the relay, to refuse a repeat; the receiver, to order a release |
| the sequence number | the relay, one global order per document | this client, as the cursor of what it has already received |

**The genesis parent.** A document with no snapshot has no parent to name, and a random one per
client would quarantine every first delta for ever. Every client derives the same parent from the
document identity alone, so the earliest deltas are releasable across a reload and across devices
(`SYNC-026`).

**An inbound update, in steps.**

1. raise this client's clock past the update when the author key is its own. A reload then does not
   re-emit a clock the relay has already consumed (`SYNC-025`);
2. record the sequence number as the new cursor;
3. deliver the update when its parent is known (`SYNC-006`);
4. otherwise hold it under its parent, sealed (`SYNC-032`), and enforce the bounds.

**The bounds, and what each one does.**

| Reached | Action |
|---|---|
| nine hundred held updates | ask the relay for everything after the cursor, so the missing parent can arrive (`SYNC-031`) |
| one thousand held updates | drop the update with the lowest clock, and warn (`SYNC-007`) |

**A release.** When a parent becomes known, everything held under it is sorted by clock and opened
in that order, then delivered. Nothing cascades: a release makes one parent known, never two.

**A replay is imported in slices** (`SYNC-131`). One import of a long tail blocks the Logic thread
for as long as it runs. Importing delta by delta costs no more than one batch, so the thread yields
between slices, and the view shows the messages as they land. A slice is 64 KB of payload, and at
least one delta. A live delta, a close or an export lands the rest whole first, and what reads the
whole replay waits for its last slice.

**An open, and why ready is not immediate.** A document is hydrated when its snapshot and every
delta the relay already holds have arrived, not when the socket answers. So the ready signal waits
on the first replay. It fires immediately only when there is no replay to wait for (`SYNC-029`).
That is a document that exists on this client alone, or an open with no connection.

| Leg of an open | Sets |
|---|---|
| the members request | the document key, the directory and this client's role (`ambre-spec-membership.md`) |
| the snapshot request | the real parent, and the cursor the replay starts after (`SYNC-028`) |
| the subscription | the live feed |
| the replay request | the deltas the snapshot does not contain |
| the replay answer | the ready signal, once (`SYNC-029`) |

**Code.**

* `crates/ambre-sync/src/updates.rs` -- the ingest, the quarantine, the bounds and the release;
* `crates/ambre-sync/src/websocket.rs` -- the demultiplexing, the replay batch and the ready signal;
* `crates/ambre-logic/src/replay.rs` -- the replay imported in slices;
* `crates/ambre-core/src/id.rs` -- the genesis parent.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| An update names its parent snapshot, and a missing parent means waiting | trusting convergence and importing on arrival | it is correct for the CRDT and wrong for the document: Loro converges whatever the order, so nothing in it would report the error, and a delta imported into a document that never held its base produces a state no peer will ever have -- silently. Naming the parent turns that into a wait |
| The quarantine is bounded, and drops the oldest | an unbounded quarantine | a parent that never arrives -- a purged snapshot, a revoked member, a relay bug -- grows the heap without limit inside a worker that also holds every key |
| The quarantine is bounded, and drops the oldest | a per-document timeout | it trades a bound for a clock: the heap is still unbounded until the timer fires, and the worker gains a deadline to get right. A count drops the lowest clock, which is the change least likely to still matter |
| The genesis parent is derived, never uploaded | a first snapshot uploaded at creation | it pays a round trip and an object for every document that is never shared, and a parent minted per client is the same field with none of the agreement. Deriving costs nothing and every client reaches the same value with no coordination |
| A replay is one batch | one event per delta | it is what the live path does and what a replay is not: a repaint per delta makes a reconnect flicker through every intermediate state, and a cell that only appears in the last delta arrives last on screen. One batch is one repaint |
| Ready waits on the first replay | reporting ready when the socket subscribes | it is true of the connection and false of the document: a document whose history is deltas alone has no content at all until the replay lands, so the overlay would dismiss over an empty document and call it open |
| The clock is recovered from the relay's echo, not persisted | writing the clock to browser storage on every edit | it is a second store to keep in step with the relay, and a store that is wrong is worse than no store. The relay already echoes this client's own updates on reconnect, so the value is on the wire |

---

## 5. NOT BUILT, AND NOT YET RULED

* **No snapshot is ever made, so every open replays the whole history.** The compaction path --
  export, seal, upload, record -- has one trigger. That trigger is a request from the relay, and the
  relay sends it from nowhere. The parent of every delta therefore stays the derived genesis one for
  ever, and the cursor of `SYNC-028` never moves off zero. The two rules above about adopting and
  chaining a snapshot describe a path nothing takes today
  (`ambre-spec-sequencer.md`, `SRV-038`).
* **Reopening a document drops what it was holding.** The open replaces the whole per-document
  state. The quarantine, the cursor and the clock go back to their initial values, while the
  relay's view of the clock does not. The recovery in `SYNC-025` repairs it on the next echo, and
  nothing states that as the rule.
* **The parent proof is empty.** Snapshot reordering is listed as prevented by a hash chain over
  parents. Every snapshot this client uploads carries an empty proof, so nothing is chained and
  nothing is checked (`ambre-spec-sealed-envelope.md`).
* **A release does not cascade.** Two snapshots waiting on each other stay held until each arrives
  on its own, which is correct and untested.
* **Nothing bounds how long a document may hold a gap.** A parent that never arrives leaves the
  document permanently behind, with a bounded quarantine and no report beyond the overflow warning.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a delta is sealed, and what refuses a tampered one | `ambre-spec-sealed-envelope.md` |
| Who assigns the sequence number, and what refuses a repeated clock | `ambre-spec-sequencer.md`, `SRV-006` |
| When a snapshot is asked for, and what the relay trims after one | `ambre-spec-sequencer.md`, `SRV-038`, `SRV-039` |
| What the socket does on a disconnect, and what it resubscribes | `ambre-spec-sync.md` |
| What an application does with a delta once it arrives | each application's engine document |
| What a rotation does to a delta sealed under an older key | `ambre-spec-key-rotation.md` |
| What a document key is, and who unwraps it at open | `ambre-spec-membership.md` |
