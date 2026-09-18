# AMBRE -- FEATURE: DELTA ORDER AND THE QUARANTINE

**Type:** Horizontal (framework)
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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-024` | `INV` | An outbound update MUST carry a clock strictly greater than every clock this client has already used for the document. | this spec section 3; `ambre-spec-sequencer.md`, `SRV-006` | none | unheld -- the counter is incremented before each seal, and no test asserts two successive updates carry two different clocks, nor that a reopened document does not restart at one; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-030` | `INV` | A snapshot MUST NOT be registered as a known parent before its contents reach the Logic thread. | this spec section 3 | none | fails -- a notification that a peer uploaded a snapshot marks that snapshot known and releases everything waiting on it, without downloading it. The Logic thread then receives deltas whose base it does not hold, section 5; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | failing |
| `SYNC-026` | `CONC` | Every client MUST derive the same parent for a document that has never been snapshotted. | this spec section 3 | none | unheld -- the derivation is a pure function of the document identity in `ambre-core` and nothing tests it, so a change to it would break every peer's quarantine at once and no job would fail; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Receive an update

A sealed delta arrives from the relay. Either its base is one this client holds, or it waits.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-006` | `POST` | Released updates MUST be applied in parent-snapshot then clock order. | this spec section 3 | `ambre_sync::updates::tests::quarantine_released_in_clock_order` -- several updates held behind one unknown parent, and the release order read back | none | verified |
| `SYNC-082` | `POST` | An update whose parent snapshot is known MUST be delivered immediately. | this spec section 3 | `ambre_sync::updates::tests::update_with_known_snapshot_delivered_immediately` -- the delivery that makes the quarantine an exception rather than a stage every delta passes through | none | verified |
| `SYNC-007` | `POST` | A quarantine overflow MUST drop the oldest entry. | this spec section 3 | none | unheld -- `::quarantine_overflow_drops_oldest_and_warns` is named for this rule and does not assert it: it counts the warning and reads the quarantine's length, never which entry left. An overflow dropping the newest, the one whose base is most likely to land next, passes; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-084` | `POST` | A quarantine overflow MUST emit exactly one warning. | this spec section 3 | `ambre_sync::updates::tests::quarantine_overflow_drops_oldest_and_warns` -- its warning count, one at the hard cap, so a warning per further arrival fails | none | verified |
| `SYNC-032` | `FRAME` | Receiving an update MUST NOT open one it quarantines. | `ZK-001`; this spec section 3 | none | unheld -- the quarantine holds the wire message and the open runs at release, so the rule holds by construction; nothing refuses a future release path that opens on arrival to inspect the payload; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-083` | `FAIL` | An update whose parent snapshot is unknown MUST be quarantined, never applied. | this spec section 3 | `ambre_sync::updates::tests::update_with_unknown_snapshot_quarantined` -- the update is held and the Logic thread is handed nothing | none | verified |

### 1.3 Open a document

The client asks for a document it may hold nothing of, part of, or all of.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-028` | `POST` | The post-open replay MUST start after the last delta the downloaded snapshot already contains. | this spec section 3 | `ambre_sync::updates::tests::open_document_ready_gates_request_missing_on_covered_seq`; `::open_document_ready_without_snapshot_requests_full_history` | none | verified |
| `SYNC-029` | `POST` | A document MUST be reported ready once per open, after its first replay has landed. | this spec section 3 | `ambre_sync::updates::tests::open_document_ready_deferred_to_initial_replay`; `::open_document_ready_immediate_without_replay`; `::document_ready_carries_the_role_on_both_paths` | none | verified |
| `SYNC-027` | `POST` | A history replay MUST reach the Logic thread as one batch, never as one event per delta. | this spec section 3 | none | unheld -- the replay collects every opened payload and sends one batch event, and no test counts the events a replay of several deltas produces; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-031` | `POST` | The quarantine MUST ask the relay for the missing history once per gap, not once per arrival. | this spec section 3 | none | fails -- the soft threshold is re-tested on every quarantined arrival, so past nine hundred held updates every further one sends another request. The relay rate-limits the socket, which is what the reconnect backoff exists to avoid (`SYNC-064`); lands with the `The quarantine asks once per gap` Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-085` | `FRAME` | Opening a document MUST NOT emit an update. | this spec section 3; derived | none | unheld -- an open reads and requests, and the only thing it sends is a history request. Nothing refuses an open that flushed pending local state, which would give the document a delta nobody typed; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Reconnect

The socket dropped and came back. Everything the client missed while it was gone is still on the
relay.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-033` | `POST` | A reconnect MUST resubscribe every document that exists on the relay. | this spec section 3 | none | unheld -- the resubscription runs from the socket's open callback, once per document, and no host test drives an open callback: the mock transport has none; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-086` | `POST` | A reconnect MUST pull what each document missed. | this spec section 3 | none | unheld -- it is the half that decides whether the reconnection is worth anything: a resubscription with no pull leaves the client subscribed to a future it cannot place, because every delta arriving next has a base it never received; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-025` | `POST` | A reload MUST recover the clock from the relay's echo of this client's own updates. | this spec section 3 | none | unheld -- the recovery reads the author key of every inbound update and raises the counter to its clock, and no test replays a client's own update to see the counter move; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-087` | `FRAME` | A reconnect MUST NOT apply an update the client already applied. | this spec section 3; derived | none | unheld -- the pull is bounded by the sequence the client last saw, so a duplicate is not requested today. Nothing holds it, and Loro's convergence hides the defect rather than refusing it: a replayed delta is idempotent on the state and not on the clock `SYNC-024` guards; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the release of 100 quarantined updates under 1 ms, a row of
  `ambre-spec-budgets.md section 1`, and the two memory holders of
  `ambre-spec-budgets.md section 6`. No bench measures the release, which that first table records
  against the row itself.
* **Boundary.** nothing here opens a socket or holds a key of its own. Every message it orders is
  already sealed (`ambre-spec-sealed-envelope.md`). The parent snapshot, the clock and the sequence
  number it reads are the routing metadata `ZK-007` allows.
* **Verification.** 6 `verified`, 2 `failing`, 10 `untested`, 0 `unimplemented`. What holds is the
  quarantine itself. An update ahead of its snapshot waits, one whose base is known is delivered,
  the release runs in clock order, and the overflow warns once. What is loose is everything about the
  clock and the socket, both needing a driven transport the host tier does not have.

---

## 2. WHAT EACH USER SEES

Two peers over one document, and a relay that may deliver in any order. The last column joins the
example to its verification.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| edits twice | watches | both edits, in Alice's order | both edits in the order Alice made them | `updates::tests::update_with_known_snapshot_delivered_immediately` |
| edits after compacting | opens the document | the snapshot and the edit | Bob waits on the snapshot, then sees the edit | `updates::tests::quarantine_released_in_clock_order` |
| edits a thousand times while Bob's base is missing | watches | Alice's edits | Bob loses the oldest, and is told (`SYNC-007`) | `updates::tests::quarantine_overflow_drops_oldest_and_warns` |
| edits a document neither peer has snapshotted | opens it | Alice's edits | Bob sees them: both derived the same parent (`SYNC-026`) | none -- section 5 |
| edits while Bob is offline | reconnects | Alice's edits | Bob receives them as one repaint, not one per edit (`SYNC-027`) | none -- section 5 |
| compacts the document | is editing it | the new snapshot | Bob adopts a base he has not loaded (`SYNC-030`) | none, and it fails -- section 5 |
| reloads the page and edits | watches | Alice's edit | Alice's edit reaches Bob rather than being refused (`SYNC-025`) | none -- section 5 |

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
* **A snapshot is adopted as a parent before it is downloaded.** The notification that a peer
  uploaded one marks it known. It makes that snapshot the base of this client's next deltas, and
  releases everything waiting on it. So the Logic thread receives deltas parented on a state it does
  not hold. This client's own next edits name a snapshot it has never read (`SYNC-030`).
* **The missing-history request has no rate.** Past the soft threshold every further quarantined
  arrival sends another request, and the relay rate-limits the socket (`SYNC-031`).
* **Reopening a document drops what it was holding.** The open replaces the whole per-document
  state. The quarantine, the cursor and the clock go back to their initial values, while the
  relay's view of the clock does not. The recovery in `SYNC-025` repairs it on the next echo, and
  nothing states that as the rule.
* **The parent proof is empty.** Snapshot reordering is listed as prevented by a hash chain over
  parents. Every snapshot this client uploads carries an empty proof, so nothing is chained and
  nothing is checked (`ambre-spec-sealed-envelope.md`).
* **A release does not cascade.** Two snapshots waiting on each other stay held until each arrives
  on its own, which is correct and untested.
* **No host test drives a socket callback.** The resubscription, the reconnect and the replay all
  run from a browser callback. The mock transport has none, so `SYNC-033` is held by reading it.
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
