# AMBRE -- FEATURE: KEY ROTATION

**Type:** Horizontal (framework)
**Implementation:** partial -- the rotation ships and the remaining members re-key. A revoke that
fails halfway leaves the key live, and the rotation re-keys the deltas alone.

> Taking someone off a document means they cannot read what is written after it. The document gets
> a new key, sealed again for everyone who stays, and the person removed keeps only what they
> already had.

Forward secrecy for a shared document is one act with two halves: remove the member, then change
the key. This document owns the change, what it must reach, and what happens when one half lands
without the other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-044` | `INV` | The cache of previous keys MUST be bounded. | this spec section 3 | none | unheld -- the bound is a truncation at ten with no test: nothing rotates eleven times and counts what is held; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-078` | `INV` | A key leaving the cache MUST be wiped. | this spec section 3; `ZK-011` | type-level -- the document key derives `ZeroizeOnDrop`, so truncating the cache wipes what it drops and a key type that did not would be a change to the type rather than to this path | none | verified |
| `SYNC-045` | `INV` | An inbound frame MUST be opened with the key its parent snapshot names, never by trying every key in turn. | this spec section 3 | none | fails -- the open tries the current key, then every cached key, then the current key again, so a frame nobody can open costs eleven attempts and a frame sealed under a revoked key still opens; lands with the `A revoked key opens nothing` Story (`ambre-roadmap.md` E7) | failing |

### 1.2 Remove a member

The initiating client asks the relay to drop the member. It then generates a key, re-wraps it for
everyone who stays and posts every copy at once.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-041` | `POST` | A revoke MUST rotate the key, so the member removed cannot open what is written after it. | `ZK-001`; this spec section 3 | none | fails -- the removal is posted first, and every later step can fail on its own. The user is told the revoke failed while the member is already out of the list, and nothing retries the rotation; lands with the "A revoke either completes or changes nothing" Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-042` | `POST` | A rotation MUST re-key everything the document key seals, not the deltas alone. | this spec section 3 | none | fails -- the document name and every media blob stay sealed under the previous key and no member can open either again, so a revoke silently blanks the name and every picture; lands with the "The rotation re-keys the whole document" Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-046` | `POST` | Whoever rotates MUST become the wrapper of record, so a later unwrap resolves against their key. | this spec section 3; `ambre-spec-membership.md`, `SYNC-034` | `ambre_server::routes::tests::revoke_then_rotate_replaces_owner_key` -- it reads the wrapper recorded after the rotation and fails when the inviter is still named | none | verified |
| `SYNC-047` | `POST` | The rotation notice MUST reach the other clients only after the new keys are stored. | this spec section 3; `ambre-spec-server.md` | none | unheld -- the relay writes every wrapped copy in one transaction and broadcasts after it commits, and no test asserts the order: a broadcast before the commit would let a client fetch the previous copy and see no error; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-048` | `POST` | A rotation MUST seal a fresh snapshot under the new key, so the state after it stands on its own. | this spec section 3 | not built | unbuilt -- lands with the "The rotation re-keys the whole document" Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-079` | `FRAME` | A revoke MUST NOT change the document's content. | this spec section 3, the opening of the model | none | unheld -- a rotation changes which key opens the document and never what it holds, and no test reads the state back across one. The re-key `SYNC-042` is missing would touch the name and the blobs, which is the one place the two rules meet; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-043` | `FAIL` | A member the rotation cannot re-wrap for MUST be reported, never dropped in silence. | this spec section 3 | none | fails -- `ambre_sync::sharing::tests::rotate_entries_cover_valid_members_and_skip_malformed` asserts the skip this rule forbids. A member whose exchange key does not parse keeps a copy of the previous key and loses the document without a message to anyone; lands with the "A revoke either completes or changes nothing" Story (`ambre-roadmap.md` E7) | failing |

### 1.3 Take on a rotation

Every other client is told the key rotated, fetches its new wrapped copy and swaps.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-080` | `POST` | Taking on a rotation MUST install the new key and move the previous one into the cache. | this spec section 3, every other client, step 3 | none | unheld -- the swap and the cache push are one call, and no test drives the notice through to the key a client holds afterwards; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-081` | `FRAME` | Taking on a rotation MUST NOT close the document. | this spec section 3, a rotation is not a removal | none | unheld -- being removed is a separate, targeted notice on the user's own channel, and the rotation notice reports no departure. Nothing asserts the two stay apart, and a client that closed on a rotation would drop every member who stayed; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** one elliptic-curve operation per remaining member, charged to the crypto rows of
  `ambre-spec-budgets.md section 1`, which a full snapshot seal joins once `SYNC-048` lands. The
  cache of previous keys is the `old_keys` holder of `ambre-spec-budgets.md section 6`.
* **Boundary.** every wrapped copy crosses the relay and the relay can open none of them
  (`SYNC-010`). What it learns is that a document was re-keyed and by whom, which is the membership
  metadata `ambre-spec-zero-knowledge.md` accepts.
* **Verification.** 2 `verified`, 4 `failing`, 5 `untested`, 1 `unimplemented`. What holds is the
  relay half, the rotator becoming the wrapper of record, and the wipe the key type carries. The
  client side is loose throughout. The revoke is not atomic, the rotation reaches the deltas alone,
  a member can be dropped in silence, and the open tries every key.

  `SYNC-044` was a conjunction whose halves have different statuses, which is what hid the second
  one. The wipe is held by the type and the bound is a truncation nothing counts. The two frame
  conditions are the pass's other find. `SYNC-079` says a revoke moves no content, and `SYNC-081`
  says taking on a rotation is not being removed. The document argues both in prose and ruled
  neither.

---

## 2. WHAT EACH USER SEES

Three peers, because a revoke needs someone to remove. The last column joins the example to its
verification.

| Alice does | Bob and Carol do | The document holds | Each user sees | Test |
|---|---|---|---|---|
| removes Carol | Bob edits | Bob's edit under the new key | Carol cannot read Bob's edit; Bob works on (`SYNC-041`) | `sharing::tests::revoke_success_deletes_then_rotates_remaining_members` |
| removes Carol, and the removal fails | -- | nothing new | Alice is told, and Carol stays a member (`SYNC-041`) | `sharing::tests::revoke_delete_failure_emits_500_and_stops` |
| removes Carol, and the rotation fails | Bob edits | Bob's edit under the previous key | Alice is told the revoke failed, Carol is out of the list, and Carol can still read Bob's edit (`SYNC-041`) | none, and it fails -- section 5 |
| removes Carol | Bob reopens the document | Alice's content, and a name nobody can open | the document is called nothing, and every picture in it is missing (`SYNC-042`) | none, and it fails -- section 5 |
| removes Carol, whose row Bob's relay serves malformed | Bob reopens | Alice's content | Bob loses the document with no message (`SYNC-043`) | `sharing::tests::rotate_entries_cover_valid_members_and_skip_malformed` |
| removes Carol while Bob's edit is in flight | Bob's edit arrives sealed under the previous key | Bob's edit | Alice reads it: the previous key is cached (`SYNC-044`) | none -- section 5 |

---

## 3. MODEL

**CRDT containers.** none. A rotation changes no document content: it changes which key opens it,
which is the relay's table and this worker's memory.

**Two triggers, one act.**

| Trigger | Who runs it | What they do |
|---|---|---|
| this user removes a member | the initiating client | generate the key, re-wrap for everyone who stays, post it |
| the relay says the key rotated | every other client | fetch the new wrapped copy, unwrap it, swap |

**The initiating client, in steps.** The Logic thread does no cryptography and no network here: the
key never leaves the Sync worker.

1. ask the relay to remove the member;
2. generate a new document key from the system generator;
3. fetch the remaining members and their exchange keys;
4. wrap the new key once per remaining member (`SYNC-035`, `SYNC-043`);
5. post every wrapped copy in one request. The relay replaces them in one transaction, records the
   rotator as the wrapper (`SYNC-046`) and broadcasts the notice after the commit (`SYNC-047`);
6. move the previous key into the cache, install the new one, and report the revoke done;
7. seal a fresh snapshot under the new key (`SYNC-048`).

**Every other client, in steps.**

1. fetch the members answer, which now carries the new wrapped copy and the rotator as wrapper;
2. run the exchange against the rotator's key, never against its own (`SYNC-034`);
3. move the previous key into the cache and install the new one (`SYNC-080`).

**Why the previous key is kept.** A rotation is not instantaneous: a peer's edit can already be in
flight, sealed under the key it held a moment ago. The cache lets that edit open rather than reading
as corrupt. It is bounded at ten keys. A document past its eleventh rotation can no longer open
its older snapshots locally, and re-reads them from object storage (`SYNC-044`). What leaves the
cache is wiped by the key type itself (`SYNC-078`).

**A rotation is not a removal.** Being taken off a document is a separate, targeted notice on the
user's own topic: it closes the document locally and tells the Logic thread. The rotation notice
is for the members who stay, and it reports no departure (`SYNC-081`).

**What the document key seals.** The rotation reaches the first row and none of the others, which is
`SYNC-042`.

| Sealed under the document key | Re-keyed by a rotation |
|---|---|
| every delta and every snapshot, from now on | yes, by construction: they are sealed after it |
| the document name | no. It stays under the previous key, and no member can open it again |
| every media blob already uploaded | no. Same, and there is no path that would re-seal one |
| a snapshot uploaded before the rotation | no, and that is correct: it is history, and the cache opens it while it lasts |

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the revoke, the new key and the re-wrap for each member;
* `crates/ambre-sync/src/websocket.rs` -- the notice, the fetch, the unwrap and the swap;
* `crates/ambre-sync/src/lib.rs` -- the cache of previous keys, and the two fallback opens;
* `crates/ambre-server/src/db/members.rs` -- the transaction, and the wrapper of record.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A revoke rotates the key | removing the member's wrapped copy alone | it is a table edit dressed as a security act: the person removed holds the key in the memory of a browser we do not control, and every later delta would still open under it. Rotation is what makes the removal mean something |
| The relay replaces every wrapped copy in one transaction | one request per member, retried on failure | a partial replacement splits the document: some members open the new deltas and some do not, and nothing tells them apart from a network failure. One transaction makes the rotation atomic on the side that can be atomic |
| The rotator becomes the wrapper of record | keeping the inviter as wrapper | the recipient runs the exchange against whoever wrapped, so every unwrap after a rotation would resolve against a key that never sealed the copy |
| The rotator becomes the wrapper of record | having the recipient try both keys | it turns one exchange into two on every unwrap, and it makes a genuine mismatch indistinguishable from a stale record: the second attempt succeeds either way |
| The notice follows the commit | broadcasting on receipt of the rotation request, which is one round trip earlier | a client that fetches on a notice sent before the commit reads the previous copy, unwraps it successfully, and installs a key that is already stale -- with no error anywhere |
| The previous keys are cached, and bounded at ten | no cache at all | an in-flight edit sealed a moment before the rotation must still open, or a normal race reports as tampering |
| The previous keys are cached, and bounded at ten | an unbounded cache | a rapid rotation loop, whether a bug or an attack, grows the worker's heap with keys nothing will ever need. Ten covers every real case and holds the bound |
| History is not re-keyed | re-sealing every stored snapshot on every rotation | a snapshot from before the rotation is what the removed member already read, so re-sealing it protects nothing and costs a full upload per rotation |
| The Logic thread is not involved | the Logic thread orchestrating the steps and asking the worker for each one | it does no cryptography and no network by design, and a rotation is both. Keeping it out means the key never crosses the boundary, which is the whole reason the boundary is there |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A revoke is four requests and no transaction.** The removal is posted first; the members fetch,
  the wrap and the rotation post can each fail after it. Every failure reports one error to the user
  and leaves the member out of the list with a key that still opens new deltas. Nothing retries, and
  nothing records that this document is owed a rotation (`SYNC-041`).
* **The rotation re-keys the deltas alone.** The name and every media blob are sealed under the
  document key outside the delta path, and nothing re-seals either. So every revoke leaves the
  document unnamed and its pictures unreadable, for everyone who stayed (`SYNC-042`,
  `ambre-spec-media.md`, `ambre-spec-sync.md`, `SYNC-067`).
* **A member with an unparsable exchange key is skipped.** The rotation proceeds without them, the
  relay keeps their previous copy, and they lose the document quietly. The skip is deliberate -- it
  keeps the rest converging -- and nothing reports it (`SYNC-043`).
* **The open tries every cached key.** An inbound frame is attempted against the current key, then
  each cached key, then the current key again for its error. So an unopenable frame costs eleven
  attempts. A frame sealed under a key that was rotated away still opens for as long as it is
  cached (`SYNC-045`).
* **No snapshot follows a rotation.** The state after the removal is never sealed on its own, so the
  document's history stays a chain of deltas across the key change (`SYNC-048`).
* **Nothing bounds how many rotations a document may take.** Past the eleventh, older snapshots stop
  opening locally, and the re-read from object storage that covers it is not driven by any rule.
* **A rotation is invisible while a document is closed.** Only a client that has the document open acts on
  the notice. Another one picks the new key up at its next open, which is correct and stated nowhere
  else.
* **No test drives a rotation end to end.** The pure transforms are covered on host and the relay
  half in the database tier. The client's two rotation paths run only in a browser.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a key is wrapped for one recipient, and against whose key it unwraps | `ambre-spec-membership.md`, `SYNC-010`, `SYNC-034` |
| What the members answer carries, and who may rotate | `ambre-spec-server.md`, `SRV-015` |
| How a delta is sealed, and what refuses one under a wrong key | `ambre-spec-sealed-envelope.md` |
| When a snapshot is asked for, and what it covers | `ambre-spec-delta-order.md` |
| What a media blob is sealed with, and how it is addressed | `ambre-spec-media.md` |
| What the document name is sealed with | `ambre-spec-sync.md`, `SYNC-067` |
| What the sharing screens show, and how a revoke is confirmed | `ambre-spec-chrome.md` |
| Whether a peer's key is the key they claim | `ambre-spec-key-verification.md` |
