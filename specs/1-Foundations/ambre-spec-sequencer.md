# AMBRE -- FEATURE: THE SEQUENCE, AND WHAT SURVIVES A CRASH

**Type:** Horizontal (framework)
**Implementation:** partial -- the ordering, the flush and the paged replay all ship; nothing ever
asks for a snapshot, so no history is ever folded and no delta is ever trimmed.

> The relay cannot read a change, so the one thing it can promise is order: every accepted change
> gets a number, the numbers never repeat, and a client that was away gets exactly what it missed.

This document owns what happens to a sealed delta between the moment it arrives and the moment
another client can ask for it again: the number it is given, the two stores it lands in, and the
replay that hands it back.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SRV-006` | Per document and per author the clock MUST increase by exactly one, and the check and the number MUST be one indivisible step. | this spec section 3; `ambre-spec-delta-order.md`, `SYNC-024` | `ambre_server::redis_ops::lua_tests::sequential_clock_succeeds`, `::first_update_succeeds`, `::duplicate_clock_rejected`, `::clock_mismatch_rejected`, `::independent_clients_have_independent_clocks`, `::lua_script_contains_key_validation`. Database tier, and behind a feature the test recipe now carries (`ambre-spec-testing.md section 5`) | verified |
| `SRV-032` | An ordering state the ephemeral store has lost MUST be rebuilt from the durable history before any delta is validated. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::seed_clock_state_unblocks_the_next_update_after_a_redis_reset`. The rebuild raises and never lowers, so a live writer cannot be undone by it | verified |
| `SRV-033` | A delta given a number MUST reach the durable index, or be recorded where an operator can replay it. | audit `H1`; this spec section 3 | none. The failing insert pushes the whole frame to a dead-letter list, and no test drives an index failure: it needs a register that refuses one statement and accepts the next | unverified |
| `SRV-034` | A delta given a number MUST reach object storage, or be recorded where an operator can replay it. | audit `H1`; this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::dirty_set_marks_on_append_and_clears_only_when_drained` covers the queueing half; nothing drives a failing append, which is the half the dead-letter list exists for | unverified |
| `SRV-035` | A flush MUST NOT drop an entry it did not store. | this spec section 3 | none. The drain advances one front counter over the entries it finished with, stops at the first retryable failure, and trims to that counter; the ordering is stated in the source and no test drives a partial batch | unverified |
| `SRV-036` | Two instances MUST NOT drain one document's queued deltas at the same time. | this spec section 3; `SRV-021` | `ambre_server::redis_ops::redis_integration_tests::flush_lock_is_mutually_exclusive`, on the database tier | verified |
| `SRV-037` | A refused clock MUST reach the client as a conflict, and MUST NOT carry a store's own error text. | `ZK-006`; `ambre-spec-server.md`, `SRV-020` | none, and **the rule fails today**: the ordering script's refusal is wrapped as an ephemeral-store failure and forwarded to the client verbatim, so the frame carries the store's name and message. The typed conflict the mapping declares is constructed nowhere -- section 6 | unverified |
| `SRV-038` | The relay MUST ask for a snapshot once a document's unfolded history passes its threshold. | this spec section 3; `ambre-spec-delta-order.md` | none, and **the rule fails today**: the request exists in the protocol and the client acts on it, and the relay sends it from nowhere. No snapshot is ever produced, so every open replays from the first delta -- section 6. Answered by the `A document is compacted as its history grows` Story | unverified |
| `SRV-039` | The delta index MUST be trimmed to what every retained snapshot already contains. | this spec section 3 | `ambre_server::db::tests::purge_superseded_updates_trims_covered_deltas`, `::purge_horizon_is_min_covered_seq`, `::create_snapshot_roundtrips_covered_seq`. The pass is correct and unreachable while `SRV-038` fails: with no snapshot there is no horizon | verified |
| `SRV-040` | A replay MUST answer every request it accepts, including one with nothing to send. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::empty_replay_still_sends_missing_updates`. The refusals before the replay -- not subscribed, too soon, membership gone -- answer nothing, deliberately | verified |
| `SRV-041` | A replay MUST deliver the whole history the caller asked for, in order. | this spec section 3 | none. The replay pages the index and re-sorts each page by number after a concurrent fetch, and a delta it cannot locate is skipped with a log rather than refused, so a page can be short and the caller cannot tell | unverified |

* **Budget.** three rows of `ambre-spec-budgets.md section 3`: the live receive and broadcast
  under 5 ms, the ordering check under 1 ms, and the flush batch under 2 s for a thousand deltas.
  The cold start of a document, under 500 ms in the same table, is the replay of this document plus
  the download of `ambre-spec-object-store.md`. Every one of the four reads **none exists**.
* **Boundary.** the relay orders bytes it cannot read. What it learns here is exactly the routing
  set of `ZK-007`: which key wrote to which document, at which clock, and how large the frame was.
  Nothing here opens a payload, and `SRV-001` keeps the frame out of the logs.
* **Verification.** 5 `verified`, 6 `unverified`, 0 `unimplemented`. What holds is the ordering
  itself: the script's four cases, the rebuild after a reset, the drain lock and the trim. What is
  loose is every recovery path -- an index that refuses a write, a queue that refuses an append, a
  partial batch -- because each needs a store that fails on demand. Two rules are stated as failing,
  and one of them is why the trim that is verified has never had anything to trim.

---

## 2. WHAT EACH USER SEES

Two peers over one document. Every row is about order, which is the only thing the relay decides.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| edits twice | edits once | three deltas, three numbers, one order | the same document, in the same order, on both screens (`SRV-006`) | `redis_ops::lua_tests::independent_clients_have_independent_clocks` |
| edits, and her frame is replayed by an attacker | watches | one delta | the replayed frame is refused: its clock is already spent (`SRV-006`) | `redis_ops::lua_tests::duplicate_clock_rejected` |
| edits while Bob is offline | reconnects | every delta, indexed | Bob gets exactly what he missed, in order (`SRV-041`) | none -- section 6 |
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
increment (`SRV-006`). The exact script is in the source.

**Why the author clocks are never pruned.** The hash grows one entry per authoring key, which is
bounded by the document's members. Expiring it would reset an author to zero, and a returning client
holding its own advanced clock would then be refused for ever; removing one entry would reopen the
replay window the clock exists to close. The bound is the membership, and that is accepted.

**What happens to an accepted delta**, in order. Each step's failure has its own answer, and none of
them un-assigns the number.

| Step | On failure |
|---|---|
| the number is assigned | the frame is refused, and nothing has moved (`SRV-006`) |
| the durable index gains a row | the whole frame goes to a dead-letter list for an operator (`SRV-033`) |
| the frame is queued for object storage, and the document marked unflushed | the same dead-letter list (`SRV-034`) |
| the frame is fanned out live | logged; a reconnecting client replays it from the index (`ambre-spec-broadcast.md`) |

**The flush.** One task per instance, every fifteen seconds, over the documents marked unflushed
rather than over the whole keyspace. Each document is drained under a lock of its own (`SRV-036`),
and the drain advances a single front counter over every entry it has finished with -- stored or
permanently unreadable -- so a trim never passes an entry still owed a retry (`SRV-035`). Every
twentieth cycle the whole keyspace is swept once, so a queue that missed its mark is still found.

**Why the front counter counts two outcomes.** An entry that will never parse and an entry that was
stored are both finished with. Counting only the stored ones desynchronised the trim: a bad entry
ahead of a good one re-stored the good one every cycle, and a bad entry at the end was retried for
ever.

**A snapshot, and the horizon it moves.** A client seals its whole state, puts it in object storage,
and records it against the document with the highest delta number it had folded in. That number is
the horizon: the trim removes every delta at or below the lowest horizon of the snapshots the
document still keeps, and the hourly pass keeps the ten most recent (`SRV-039`). **Nothing triggers
any of it** (`SRV-038`).

**A replay.** A client asks for everything after a number it names.

1. refuse if the document is not subscribed, if the last replay was under a second ago, or if the
   membership no longer holds -- and answer nothing, in all three cases;
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

The sequencer has no surface of its own: it is reached by a frame or by a request, and what a person
sees is how long a document takes to open.

| Trigger | Behaviour |
|---|---|
| a client sends a sealed delta | it is checked, numbered, indexed, queued and fanned out |
| a client reconnects and asks for what it missed | one paged replay, throttled to one per second per document |
| a client asks for the current snapshot record | the most recent one, or nothing when the document has never been snapshotted |
| a client asks for a page of the index over a request | up to a thousand rows, after a number it names |
| a client records a snapshot it has uploaded | the row is written and every subscriber is told one is available |
| the hour turns | the oldest snapshots beyond ten are removed, and the index is trimmed to the horizon |

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The ordering check and the numbering are one step in the ephemeral store | two round trips leave a window in which a second frame from the same author passes the same check, and the whole anti-replay property rests on that window not existing. One script closes it without a transaction on the durable store | checking in the process and numbering in the store, which is the same code with a race in it |
| The author clock hash is never expired | a reset to zero locks a legitimate author out of its own document for ever, because its own clock only moves forward. The hash is bounded by the membership, so the growth is bounded too | a lifetime on the hash, which trades an unbounded key for a permanent lockout |
| The number is assigned before the delta is durable | a client cannot retry at the same clock, so refusing after the assignment would strand it. Assigning first and dead-lettering a failure keeps the sequence intact and turns a durability failure into an operator's task | assigning after the durable write, which makes every store failure a client failure with no retry |
| One flush task per instance, over a marked set | a task per active document is a thousand idle tasks on a thousand open documents, and a sweep of the whole keyspace every cycle costs a scan the store does not owe us. A set of unflushed documents is one read | a task per active document, and a keyspace scan every cycle |
| The drain takes a lock per document | two instances draining one queue both trim by their own count, which removes entries a client queued in between: a delta already numbered and already acknowledged would be gone. The write path never takes the lock, so collaboration is untouched | letting both instances drain, which is safe until the second instance exists |
| The replay is paged, and each page is fetched together | one capped batch silently truncated any client further behind than the cap: it applied the batch, declared itself hydrated and never asked again. Pages remove the truncation, and fetching a page's objects together removes the serial round trip that dominated a cold open | one batch with a hard cap, and one object fetch at a time |
| A replay is throttled per document | a request for everything since the first delta is one small frame that costs the relay the whole history from object storage. One replay per reconnect is the real need, and a second within the same second is not one | no throttle, which makes the cheapest frame the most expensive request |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing asks for a snapshot, so no document is ever compacted.** The relay never sends the
  request, and the client's whole compaction path -- export, seal, upload, record -- has that
  request as its only trigger. Three consequences follow and none of them is visible from any single
  document: every cold open replays from the first delta ever written, the index grows without
  bound because the trim's horizon is computed from snapshots that do not exist, and the cold-start
  budget of `ambre-spec-budgets.md section 3` is unreachable on any document with history
  (`SRV-038`, `SRV-039`).
* **A refused clock is reported as an ephemeral-store failure.** The script's refusal is wrapped and
  forwarded verbatim, so the client is handed the store's name and its message inside a conflict
  frame. The typed conflict the error mapping declares -- with the expected and the received value
  -- is constructed nowhere (`SRV-037`).
* **A replay can be short and say nothing.** A delta the relay cannot locate in object storage or in
  the queue is logged and skipped, and the page is sent without it. The client sees a page, applies
  it, and has no way to know a number is missing from it (`SRV-041`).
* **The dead-letter list is written and never read.** Nothing drains it, nothing counts it, and no
  route exposes it, so the recovery `SRV-033` and `SRV-034` promise is an operator reading the
  ephemeral store by hand.
* **The snapshot chain proof is stored and never checked.** The record carries a parent hash and a
  chain proof, and the relay validates neither -- which it could not, being blind. Whether any
  client checks them is `ambre-spec-delta-order.md`.
* **No test drives a failing store.** Every recovery path here is reached only when the index, the
  queue or object storage refuses a single operation, and the suite has no way to make one refuse
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
