# AMBRE -- FEATURE: THE SEQUENCE, AND WHAT SURVIVES A CRASH

**Type:** Horizontal (framework)
**Implementation:** partial -- the ordering, the flush and the paged replay all ship. Nothing ever
asks for a snapshot, so no history is ever folded and no delta is ever trimmed.

> The relay cannot read a change, so the one thing it can promise is order. Every accepted change
> gets a number, and the numbers never repeat. A client that was away gets exactly what it missed.

This document owns what happens to a sealed delta between its arrival and the next client's
request for it. Three things: the number it is given, the two stores it lands in, and the replay
that hands it back.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-006` | `INV` | Per document and per author the accepted clock MUST increase by exactly one. | this spec section 3; `ambre-spec-delta-order.md`, `SYNC-024` | `ambre_server::redis_ops::lua_tests::sequential_clock_succeeds` -- an accepted value; `::first_update_succeeds` -- the first one; `::duplicate_clock_rejected` -- the replayed refusal; `::clock_mismatch_rejected` -- the out-of-step refusal; `::independent_clients_have_independent_clocks` -- one author's clock does not move another's, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |

### 1.2 A delta is numbered

The one thing a blind relay decides. Two counters -- the author's clock and the document's number --
move together or neither moves.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-032` | `POST` | An ordering state the ephemeral store has lost MUST be rebuilt from the durable history before any delta is validated. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::seed_clock_state_unblocks_the_next_update_after_a_redis_reset` -- the author's next delta is accepted after a reset that would otherwise refuse it for ever, the rebuild raising and never lowering, so a live writer cannot be undone by it | none | verified |
| `SRV-080` | `POST` | The clock check and the number MUST be one indivisible step. | this spec section 3; `SRV-006` | none | unheld -- `ambre_server::redis_ops::lua_tests::lua_script_contains_key_validation` greps the script's text for `HGET`, `INCR` and `HSET`, never runs it, and no test sends two frames from one author at once. The atomicity rests on the script being a script, which the grep does not establish; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-037` | `FAIL` | A refused clock MUST reach the client as a typed conflict, carrying the expected and the received value. | `ambre-spec-server.md`, `SRV-020` | none | fails -- the script's refusal is wrapped as an ephemeral-store failure, and the typed conflict the error mapping declares is constructed nowhere, section 6; lands with the `A refusal carries the conflict, never the store's error` Story (`ambre-roadmap.md` E7) | failing |
| `SRV-081` | `FRAME` | A refusal MUST NOT carry a store's own name or error text out to the client. | `ZK-006`; `ambre-spec-server.md`, `SRV-020` | none | fails -- the wrapped failure is forwarded verbatim, so the frame the client receives names the ephemeral store and quotes its message, section 6; lands with the `A refusal carries the conflict, never the store's error` Story (`ambre-roadmap.md` E7) | failing |

### 1.3 A numbered delta is made durable

The number is assigned before the delta is stored, so every failure below is a failure after the
sequence has already moved.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-036` | `FRAME` | Two instances MUST NOT drain one document's queued deltas at the same time. | this spec section 3; `SRV-021` | `ambre_server::redis_ops::redis_integration_tests::flush_lock_is_mutually_exclusive` -- the second acquirer is refused while the first holds it, on the database tier | none | verified |
| `SRV-033` | `POST` | A delta given a number MUST reach the durable index. | audit `H1`; this spec section 3 | none | unheld -- no test drives an index failure: it needs a register that refuses one statement and accepts the next (`ambre-spec-testing.md section 9`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-034` | `POST` | A delta given a number MUST reach object storage. | audit `H1`; this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::dirty_set_marks_on_append_and_clears_only_when_drained` -- the queueing half, the document marked unflushed on append and cleared only once drained | partial -- nothing drives a failing append; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-082` | `POST` | A durable write that failed MUST leave the whole frame where an operator can replay it. | audit `H1`; this spec section 3 | none | unheld -- the recovery is nominal: the failing path pushes to a dead-letter list, and nothing drains it, counts it or exposes it over any route, section 6. The rule is held by a list nobody reads; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-035` | `FRAME` | A flush MUST NOT drop an entry it did not store. | this spec section 3 | none | unheld -- the drain advances one front counter over the entries it finished with, stored or permanently unreadable, stops at the first retryable failure and trims to that counter; the ordering is stated in the source and no test drives a partial batch; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 A history is folded and trimmed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-039` | `POST` | A delta a retained snapshot already contains MUST be removed from the index. | this spec section 3 | `ambre_server::db::tests::purge_superseded_updates_trims_covered_deltas` -- ten deltas and a snapshot covering six leave exactly the uncovered tail, correct and unreachable while `SRV-038` is unbuilt, since with no snapshot there is no horizon | none | verified |
| `SRV-083` | `FRAME` | A trim MUST NOT remove a delta the oldest retained snapshot does not cover. | this spec section 3 | `ambre_server::db::tests::purge_horizon_is_min_covered_seq` -- two snapshots covering 3 and 8 stop the trim at 3, not 8; `::create_snapshot_roundtrips_covered_seq` -- the horizon a snapshot records | none | verified |
| `SRV-038` | `POST` | The relay MUST ask for a snapshot once a document's unfolded history passes its threshold. | this spec section 3; `ambre-spec-delta-order.md` | not built | unbuilt -- lands with the `A document is compacted as its history grows` Story (`ambre-roadmap.md` E7). The request exists in the protocol and the client acts on it, and the relay sends it from nowhere, so no document is ever compacted, section 6 | unimplemented |

### 1.5 A client asks for what it missed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-040` | `POST` | A replay MUST answer every request it accepts, including one with nothing to send. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::empty_replay_still_sends_missing_updates` -- a document with no rows still emits one `MissingUpdates`, which is what releases the client's loading overlay | partial -- an index read that fails mid-replay still returns without answering; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-041` | `POST` | A replay MUST deliver every delta the caller asked for. | this spec section 3 | none | fails -- a delta the relay cannot locate in object storage or in the pending map is logged and skipped, so a page can be short and the caller cannot tell, section 6; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | failing |
| `SRV-084` | `POST` | A replay MUST deliver its deltas in the order of their numbers. | this spec section 3 | none | unheld -- the index is paged in order and each page is re-sorted by number after its concurrent fetch, so the ordering is held by the sort being written; no test drives a page whose fetches complete out of order; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-075` | `FRAME` | A page of the delta index served over a request MUST NOT exceed the cap the relay declares. | audit `M16`; this spec section 3 | none | unheld -- `routes::sync` clamps the caller's `limit` to 1..=1000, and no test asks for more. The cap binds that route alone: the socket replay pages the whole history and bounds only each page; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-085` | `FAIL` | A replay MUST be refused, and answered with nothing, when the caller's membership no longer holds. | this spec section 3; `ambre-spec-membership.md` | none | unheld -- the handler re-checks membership on a short cache before it reads the index; no test revokes a membership mid-session and asks again, which is the one path that keeps a revoked member from pulling the whole history down; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-086` | `FAIL` | A second replay of one document within the throttle window MUST be refused, and answered with nothing. | this spec section 3 | none | unheld -- the handler records the instant of each served replay and drops a call that arrives inside the interval; no test calls twice; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 A segment is finalized

A Murmure segment is a document the relay closes to deltas for good. The client writes the final
snapshot, and the relay arbitrates who writes it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-113` | `POST` | The relay MUST grant the finalize lease of a segment to one client at a time. | this spec section 3; `Todo/ambre-murmure.md` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-114` | `POST` | Granting a finalize lease MUST lock the segment against deltas for good. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-115` | `FAIL` | Given a delta for a locked segment, the relay MUST refuse it with an answer that names the lock. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-116` | `FAIL` | Given a finalize commit from a client that holds no live lease, the relay MUST refuse it. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-117` | `POST` | An accepted finalization MUST record the finalization time of the segment. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-118` | `POST` | The deltas of a final segment MUST be purged only once a second member client confirms the finalization, or once the verification delay has passed. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-119` | `POST` | A lease that expires MUST let another client take the lease of the same segment. | this spec section 3; `ambre-spec-budgets.md section 8` | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-120` | `FRAME` | Finalizing a segment MUST NOT touch a delta of any other document. | this spec section 3 | not built | unbuilt -- lands with the `Channel segments and finalization` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** three rows of `ambre-spec-budgets.md section 3`. The live receive and broadcast
  under 5 ms, the ordering check under 1 ms, the flush batch under 2 s for a thousand deltas.
  All three read **none exists**, as does the cold start of the same table.
* **Boundary.** the relay orders bytes it cannot read. What it learns is the routing set of
  `ZK-007`: which key wrote to which document, at which clock, and how large the frame was.
  `SRV-001` keeps the frame out of the logs.
* **Verification.** 5 `verified`, 2 `partial`, 3 `failing`, 8 `untested`, 1 `unimplemented`. What holds is the
  ordering itself. The accepted and refused cases of the script, the rebuild after a reset, the
  drain lock and both halves of the trim. What is loose is every recovery path and every refusal the
  replay performs.
---

## 2. WHAT EACH USER SEES

Two peers over one document. Every row is about order, which is the only thing the relay decides.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| edits twice | edits once | three deltas, three numbers, one order | the same document, in the same order, on both screens (`SRV-006`) | `redis_ops::lua_tests::independent_clients_have_independent_clocks` |
| edits, and her frame is replayed by an attacker | watches | one delta | the replayed frame is refused: its clock is already spent (`SRV-006`) | `redis_ops::lua_tests::duplicate_clock_rejected` |
| edits while Bob is offline | reconnects | every delta, indexed | Bob gets exactly what he missed, in order (`SRV-041`, `SRV-084`) | none -- section 6 |
| edits nothing while Bob is away | reconnects with a current snapshot | nothing new | Bob's document opens rather than spinning (`SRV-040`) | `ws::handler::message::delta_size_cap_tests::empty_replay_still_sends_missing_updates` |
| edits for a year | opens the document | every delta since the first (`SRV-038`) | both wait through the whole history, every time | none, and it fails -- section 6 |
| edits during a reset of the ephemeral store | watches | the durable history alone, then the rebuilt state | Alice's next edit is accepted rather than refused for ever (`SRV-032`) | `redis_ops::redis_integration_tests::seed_clock_state_unblocks_the_next_update_after_a_redis_reset` |

---

## 3. MODEL

**CRDT containers.** none. What is ordered here is a sealed frame; what converges inside it is the
client's business (`ambre-spec-delta-order.md`).

**Two counters, one indivisible step.** The relay keeps a clock per author and a number per
document, and moves both together or neither.

```text
given a document, an author key and an incoming clock:
1. current := the author's last accepted clock for this document, or zero
2. if incoming is not current + 1: refuse, and report the current value
3. record incoming as the author's last accepted clock
4. return the document's next number, incremented
```

The step runs inside the ephemeral store, so no two frames can interleave between the check and the
increment (`SRV-080`). The exact script is in the source.

**Why the author clocks are never pruned.** The hash grows one entry per authoring key, which is
bounded by the document's members. Expiring it would reset an author to zero, and a returning client
holding its own advanced clock would then be refused for ever. Removing one entry would reopen the
replay window the clock exists to close. The bound is the membership, and that is accepted.

**What happens to an accepted delta**, in order. Each step's failure has its own answer, and none of
them un-assigns the number.

| Step | On failure |
|---|---|
| the number is assigned | the frame is refused, and nothing has moved (`SRV-006`) |
| the durable index gains a row | the whole frame goes to a dead-letter list for an operator (`SRV-033`, `SRV-082`) |
| the frame is queued for object storage, and the document marked unflushed | the same dead-letter list (`SRV-034`, `SRV-082`) |
| the frame is fanned out live | logged; a reconnecting client replays it from the index (`ambre-spec-broadcast.md`) |

**The flush.** One task per instance, every fifteen seconds, over the documents marked unflushed
rather than over the whole keyspace. Each document is drained under a lock of its own (`SRV-036`).
The drain advances a single front counter over every entry it has finished with, stored or
permanently unreadable. A trim therefore never passes an entry still owed a retry (`SRV-035`). Every
twentieth cycle the whole keyspace is swept once, so a queue that missed its mark is still found.

**Why the front counter counts two outcomes.** An entry that will never parse and an entry that was
stored are both finished with. Counting only the stored ones desynchronised the trim. A bad entry
ahead of a good one re-stored the good one every cycle, and a bad entry at the end was retried
for ever.

**A snapshot, and the horizon it moves.** A client seals its whole state and puts it in object
storage. It records it against the document with the highest delta number it had folded in, and
that number is the horizon. The trim removes every delta at or below the lowest horizon of the
snapshots the document still keeps (`SRV-039`, `SRV-083`). The hourly pass keeps the ten most
recent. **Nothing triggers any of it** (`SRV-038`).

**A finalization.** A Murmure segment ends as one immutable snapshot (`Todo/ambre-murmure.md`).

1. A member client asks for the finalize lease of the segment. The relay grants it to one client
   (`SRV-113`) and locks the segment for good (`SRV-114`). A later delta is refused with an answer
   that names the lock, so the client rewrites the edit as an amendment (`SRV-115`).
2. The client uploads the final snapshot and commits it with its horizon. The relay refuses a
   commit without the live lease (`SRV-116`), and records the finalization time (`SRV-117`).
3. The deltas stay until a second member client confirms the snapshot against them, or until the
   verification delay passes (`SRV-118`). A false snapshot therefore loses nothing at once.

The durations are rows of `ambre-spec-budgets.md section 8`.

**A replay.** A client asks for everything after a number it names.

1. refuse, and answer nothing, on three counts: no subscription, a replay under a second old
   (`SRV-086`), or a membership that no longer holds (`SRV-085`);
2. load the still-queued deltas once, as a map from number to frame;
3. read the index in pages, in order;
4. fetch each page's frames from object storage together, falling back to the map for what has not
   flushed yet, and re-sort the page by number;
5. send one answer per page, and one empty answer when the first page is empty (`SRV-040`).

**Why the first empty page still answers.** A client treats the arrival of a replay as the end of
its hydration, so silence leaves a document loading for ever. A later empty page simply ends the
paging: the earlier pages already released the client.

**Code.**

* `crates/ambre-server/src/redis_ops.rs` -- the ordering script, the rebuild, the queue and the
  dead-letter list;
* `crates/ambre-server/src/ws/handler/message.rs` -- the write path and the paged replay;
* `crates/ambre-server/src/s3.rs` -- the flush, its lock and its front counter;
* `crates/ambre-server/src/db/content.rs` -- the index, the snapshot record and the trim;
* `crates/ambre-server/src/routes/sync.rs` -- the snapshot record and the delta page over requests;
* `crates/ambre-server/src/main.rs` -- the hourly pass.

---

## 4. SURFACE

The sequencer has no surface of its own: it is reached by a frame or by a request. What a person
sees is how long a document takes to open.

| Trigger | Behaviour |
|---|---|
| a client sends a sealed delta | it is checked, numbered, indexed, queued and fanned out |
| a client reconnects and asks for what it missed | one paged replay, throttled to one per second per document |
| a client asks for the current snapshot record | the most recent one, or nothing when the document has never been snapshotted |
| a client asks for a page of the index over a request | up to a thousand rows, after a number it names (`SRV-075`) |
| a client records a snapshot it has uploaded | the row is written and every subscriber is told one is available |
| the hour turns | the oldest snapshots beyond ten are removed, and the index is trimmed to the horizon |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The ordering check and the numbering are one step in the ephemeral store | checking in the process and numbering in the store | it is the same code with a race in it: two round trips leave a window in which a second frame from the same author passes the same check, and the whole anti-replay property rests on that window not existing. One script closes it without a transaction on the durable store |
| The author clock hash is never expired | a lifetime on the hash | it trades an unbounded key for a permanent lockout: a reset to zero locks a legitimate author out of its own document for ever, because its own clock only moves forward. The hash is bounded by the membership, so the growth is bounded already |
| The number is assigned before the delta is durable | assigning after the durable write | it makes every store failure a client failure with no retry, because a client cannot retry at the same clock. Assigning first and dead-lettering a failure keeps the sequence intact and turns a durability failure into an operator's task |
| One flush task per instance, over a marked set | a task per active document | it is a thousand idle tasks on a thousand open documents |
| One flush task per instance, over a marked set | a keyspace scan every cycle | it costs a scan the store does not owe us, every cycle, whatever was written. A set of unflushed documents is one read |
| The drain takes a lock per document | letting both instances drain | it is safe until the second instance exists: two instances draining one queue both trim by their own count, which removes entries a client queued in between, so a delta already numbered and already acknowledged would be gone. The write path never takes the lock, so collaboration is untouched |
| The replay is paged, and each page is fetched together | one batch with a hard cap | it silently truncated any client further behind than the cap: it applied the batch, declared itself hydrated and never asked again |
| The replay is paged, and each page is fetched together | one object fetch at a time | the serial round trip dominated a cold open. Fetching a page's objects together removes it |
| A replay is throttled per document | no throttle | it makes the cheapest frame the most expensive request: a request for everything since the first delta is one small frame that costs the relay the whole history from object storage. One replay per reconnect is the real need, and a second within the same second is not one |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing asks for a snapshot, so no document is ever compacted.** The relay never sends the
  request, and the client's whole compaction path -- export, seal, upload, record -- has that
  request as its only trigger. Three consequences follow, and none is visible from any single
  document. Every cold open replays from the first delta ever written. The index grows without
  bound, because the trim's horizon is computed from snapshots that do not exist. And the cold-start
  budget of `ambre-spec-budgets.md section 3` is unreachable on any document with history
  (`SRV-038`, `SRV-039`).
* **A refused clock is reported as an ephemeral-store failure.** The script's refusal is wrapped and
  forwarded verbatim, so the client is handed the store's name and its message inside a conflict
  frame. The typed conflict the error mapping declares -- with the expected and the received value
  -- is constructed nowhere (`SRV-037`, `SRV-081`).
* **A replay can be short and say nothing.** A delta the relay cannot locate in object storage or in
  the queue is logged and skipped, and the page is sent without it. The client sees a page, applies
  it, and has no way to know a number is missing from it (`SRV-041`).
* **The dead-letter list is written and never read.** Nothing drains it, nothing counts it, and no
  route exposes it, so the recovery `SRV-082` promises is an operator reading the ephemeral store by
  hand.
* **The snapshot chain proof is stored and never checked.** The record carries a parent hash and a
  chain proof, and the relay validates neither -- which it could not, being blind. Whether any
  client checks them is `ambre-spec-delta-order.md`.
* **Finalization is unbuilt.** No segment exists, and no lease, lock or purge after confirmation
  (`SRV-113` to `SRV-120`).
* **No test drives a failing store.** Every recovery path here is reached only when the index, the
  queue or object storage refuses a single operation. The suite has no way to make one refuse
  (`ambre-spec-testing.md section 9`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Why a client waits for a parent, and what it holds meanwhile | `ambre-spec-delta-order.md` |
| What a client puts inside the frame this document numbers | `ambre-spec-sealed-envelope.md` |
| Where a sealed frame is stored, and under which name | `ambre-spec-object-store.md` |
| Who receives the frame live, and over what | `ambre-spec-broadcast.md` |
| Who may write at all | `ambre-spec-server.md`, `SRV-008` |
| The signature the frame carries | `ambre-spec-server.md`, `SRV-007` |
| What the relay may learn from an ordered frame | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
