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
| `SYNC-115` | `INV` | Every document key MUST be named by an identifier derived from the key itself. | this spec section 3 | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-116` | `INV` | Every rotation MUST leave a link that seals the previous key under the new one, except a rotation that records a history boundary. | this spec section 3 | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-117` | `INV` | The relay MUST keep every link of a document for as long as the document exists. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Remove a member

The initiating client asks the relay to drop the member. It then generates a key, re-wraps it for
everyone who stays and posts every copy at once.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-041` | `POST` | A revoke MUST rotate the key, so the member removed cannot open what is written after it. | `ZK-001`; this spec section 3 | none | fails -- the removal is posted first, and every later step can fail on its own. The user is told the revoke failed while the member is already out of the list, and nothing retries the rotation; lands with the "A revoke either completes or changes nothing" Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-042` | `POST` | A rotation MUST leave everything sealed before it openable by every member who stays. | this spec section 3; `SYNC-119`, `SYNC-125` | none | fails -- the document name and every media blob stay sealed under the previous key, nothing names that key, and no member can open either again, so a revoke silently blanks the name and every picture; lands with the `Every sealed object names its key` Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-046` | `POST` | Whoever rotates MUST become the wrapper of record, so a later unwrap resolves against their key. | this spec section 3; `ambre-spec-membership.md`, `SYNC-034` | `ambre_server::routes::tests::revoke_then_rotate_replaces_owner_key` -- it reads the wrapper recorded after the rotation and fails when the inviter is still named | none | verified |
| `SYNC-047` | `POST` | The rotation notice MUST reach the other clients only after the new keys are stored. | this spec section 3; `ambre-spec-server.md` | none | unheld -- the relay writes every wrapped copy in one transaction and broadcasts after it commits, and no test asserts the order: a broadcast before the commit would let a client fetch the previous copy and see no error; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-048` | `POST` | A rotation MUST seal a fresh snapshot under the new key, so the state after it stands on its own. | this spec section 3 | not built | unbuilt -- lands with the `A snapshot follows each rotation` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-079` | `FRAME` | A revoke MUST NOT change the document's content. | this spec section 3, the opening of the model | none | unheld -- a rotation changes which key opens the document and never what it holds, and no test reads the state back across one. The re-key `SYNC-042` is missing would touch the name and the blobs, which is the one place the two rules meet; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-043` | `FAIL` | A member the rotation cannot re-wrap for MUST be reported, never dropped in silence. | this spec section 3 | none | fails -- `ambre_sync::sharing::tests::rotate_entries_cover_valid_members_and_skip_malformed` asserts the skip this rule forbids. A member whose exchange key does not parse keeps a copy of the previous key and loses the document without a message to anyone; lands with the "A revoke either completes or changes nothing" Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-118` | `POST` | A rotation MUST post its signed link in the same request as the wrapped copies. | this spec section 3; `SYNC-047` | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-126` | `POST` | A revoke MUST post the removal, the wrapped copies and the link in one request. | this spec section 3; `SYNC-041` | not built | unbuilt -- lands with the `A revoke either completes or changes nothing` Story (`ambre-roadmap.md` E7) | unimplemented |

### 1.3 Take on a rotation

Every other client is told the key rotated, fetches its new wrapped copy and swaps.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-080` | `POST` | Taking on a rotation MUST install the new key and move the previous one into the cache. | this spec section 3, every other client, step 3 | none | unheld -- the swap and the cache push are one call, and no test drives the notice through to the key a client holds afterwards; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-081` | `FRAME` | Taking on a rotation MUST NOT close the document. | this spec section 3, a rotation is not a removal | none | unheld -- being removed is a separate, targeted notice on the user's own channel, and the rotation notice reports no departure. Nothing asserts the two stay apart, and a client that closed on a rotation would drop every member who stayed; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Recover a previous key

A client that lacks the key a frame or a snapshot names walks the chain back from the current key.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-119` | `POST` | A client that lacks the key a frame or a snapshot names MUST recover it by walking the links back from the current key. | this spec section 3; `SYNC-045` | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-120` | `FAIL` | Given a link whose unsealed key does not match the identifier it names, the client MUST refuse that link and every link past it. | this spec section 3; `SYNC-115` | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-121` | `FAIL` | Given a link whose signer the membership log does not verify, and no rule of the application that owns the document accepts, the client MUST refuse the link. | this spec section 3; `SYNC-103`, `SYNC-106` | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SYNC-122` | `FRAME` | A key recovered from the chain MUST NOT become the current key. | this spec section 3 | not built | unbuilt -- lands with the `The key chain` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Leave on one's own

A member who leaves cannot choose the key the others keep. The rotation is owed to those who stay.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-127` | `POST` | A member who leaves MUST post the removal alone, and leave the rotation owed. | this spec section 3; `SRV-122` | not built | unbuilt -- lands with the `A revoke either completes or changes nothing` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-128` | `POST` | A client that finds a rotation owed MUST rotate before it sends a frame. | this spec section 3; `SRV-123` | not built | unbuilt -- lands with the `A revoke either completes or changes nothing` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-129` | `POST` | A frame the relay refuses for its key MUST be sealed again under the current key and sent again. | this spec section 3; `SRV-123` | not built | unbuilt -- lands with the `A revoke either completes or changes nothing` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-130` | `FRAME` | Leaving MUST NOT choose the next key. | this spec section 3 | not built | unbuilt -- lands with the `A revoke either completes or changes nothing` Story (`ambre-roadmap.md` E7) | unimplemented |

* **Budget.** one elliptic-curve operation per remaining member, charged to the crypto rows of
  `ambre-spec-budgets.md section 1`, which a full snapshot seal joins once `SYNC-048` lands. The
  cache of previous keys is the `old_keys` holder of `ambre-spec-budgets.md section 6`. A step
  back along the chain costs one small open and one signature check.
* **Boundary.** every wrapped copy crosses the relay and the relay can open none of them
  (`SYNC-010`). What it learns is that a document was re-keyed and by whom, which is the membership
  metadata `ambre-spec-zero-knowledge.md` accepts. It also holds each link and each key
  identifier, and opens neither.
* **Verification.** 2 `verified`, 4 `failing`, 5 `untested`, 14 `unimplemented`. What holds is the
  relay half and the wipe the key type carries. The client side is loose: the revoke is not atomic,
  the rotation reaches the deltas alone, and the open tries every key.

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

1. generate a new document key from the system generator;
2. fetch the remaining members and their exchange keys;
3. wrap the new key once per remaining member (`SYNC-035`, `SYNC-043`);
4. post the removal, every wrapped copy and the signed link in one request (`SYNC-126`). The
   relay applies them in one transaction, records the
   rotator as the wrapper (`SYNC-046`) and broadcasts the notice after the commit (`SYNC-047`);
5. move the previous key into the cache, install the new one, and report the revoke done;
6. seal a fresh snapshot under the new key (`SYNC-048`).

**A departure, in steps.** The member who leaves posts the removal alone (`SYNC-127`). They do not
choose the next key: the members who stay would then trust a key the leaver picked (`SYNC-130`).

1. The relay records the removal, and marks the document as owing a rotation (`SRV-122`).
2. It refuses every frame sealed under the key the removal left behind (`SRV-123`).
3. The first member client to send finds the debt, and rotates first (`SYNC-128`).
4. A frame refused for its key is sealed again under the new key and sent again (`SYNC-129`).

**Every other client, in steps.**

1. fetch the members answer, which now carries the new wrapped copy and the rotator as wrapper;
2. run the exchange against the rotator's key, never against its own (`SYNC-034`);
3. move the previous key into the cache and install the new one (`SYNC-080`).

**Why the previous key is kept.** A rotation is not instantaneous: a peer's edit can already be in
flight, sealed under the key it held a moment ago. The cache lets that edit open rather than reading
as corrupt. It is bounded at ten keys (`SYNC-044`). What leaves the cache is wiped by the key type
itself (`SYNC-078`).

**The key chain.** The relay keeps one wrapped copy per member, the current one. Every older key
comes back through a chain of links (`SYNC-116`). Each rotation leaves one link: the previous key,
sealed under the new one and signed by the rotator.

A client holding the current key walks back to any older key (`SYNC-119`). So a reload, a new
member or a document past its tenth rotation still opens its history. Keybase teams chain their
keys the same way.

```rust
pub struct KeyId([u8; 16]);         // BLAKE2b("ambre-key-id-v1" || key), truncated
pub struct SealedKey(Vec<u8>);      // one document key, sealed under another
pub struct KeyLink {
    pub document: DocumentId,
    pub key: KeyId,                 // the key the link starts from
    pub previous: KeyId,            // the key it seals
    pub sealed: SealedKey,          // `previous`, sealed under `key`
    pub author: UserId,             // the rotator
    pub signature: Signature,
}
```

A link is checked twice. The key it unseals must hash to the identifier it names (`SYNC-120`), and
its signer must be a verified member (`SYNC-121`). A recovered key opens history, and never seals
anything new (`SYNC-122`).

**A history boundary is a rotation with no link.** The chain stops there. A member who joined after
it holds no key before it, and no walk reaches one.

**A rotation is not a removal.** Being taken off a document is a separate, targeted notice on the
user's own topic: it closes the document locally and tells the Logic thread. The rotation notice
is for the members who stay, and it reports no departure (`SYNC-081`).

**What the document key seals.** The rotation reaches the first row and none of the others, which is
`SYNC-042`.

| Sealed under the document key | Re-keyed by a rotation |
|---|---|
| every delta and every snapshot, from now on | yes, by construction: they are sealed after it |
| the document name | no. It stays under the previous key, which it names, and the chain opens it (`SYNC-042`) |
| every media blob already uploaded | no. Same: it names its key, and nothing re-seals it |
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
| Older keys come back through a chain of links | a signed bundle of every key, sent with each invite (Matrix MSC4268) | it serves the invitee alone: a reload, a member's new device, a self-join and an eleventh rotation still lose the history |
| A sealed object keeps the key of its time | re-sealing the name and every blob at each rotation | a rotation would download and upload every picture, for a member who already saw them all |
| A departure leaves the rotation owed | the leaving member rotates | the members who stay would trust a key the leaver chose |
| The relay refuses the key a removal left behind | trusting the next writer to rotate | until someone rotates, a frame under the old key would reach the member who left |
| Older keys come back through a chain of links | keys forwarded one by one by any member (Matrix MSC3061) | a key that arrives on its own proves nothing about its sender, and that flaw was exploited in Matrix |
| A key identifier derived from the key | a random identifier | a random one binds nothing: a false link would name a real identifier and seal a false key |
| History is not re-keyed | re-sealing every stored snapshot on every rotation | a snapshot from before the rotation is what the removed member already read, so re-sealing it protects nothing and costs a full upload per rotation |
| The Logic thread is not involved | the Logic thread orchestrating the steps and asking the worker for each one | it does no cryptography and no network by design, and a rotation is both. Keeping it out means the key never crosses the boundary, which is the whole reason the boundary is there |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A revoke is four requests and no transaction.** The removal is posted first; the members fetch,
  the wrap and the rotation post can each fail after it. Every failure reports one error to the user
  and leaves the member out of the list with a key that still opens new deltas. Nothing retries, and
  nothing records that this document is owed a rotation (`SYNC-041`).
* **Nothing names the key of a name or a blob.** Both stay sealed under the key of their time,
  which is correct once each names its key and the chain opens it. Neither names it today, so
  every revoke leaves the document unnamed and its pictures unreadable (`SYNC-042`, `SYNC-125`).
* **A member with an unparsable exchange key is skipped.** The rotation proceeds without them, the
  relay keeps their previous copy, and they lose the document quietly. The skip is deliberate -- it
  keeps the rest converging -- and nothing reports it (`SYNC-043`).
* **The open tries every cached key.** An inbound frame is attempted against the current key, then
  each cached key, then the current key again for its error. So an unopenable frame costs eleven
  attempts. A frame sealed under a key that was rotated away still opens for as long as it is
  cached (`SYNC-045`).
* **No snapshot follows a rotation.** The state after the removal is never sealed on its own, so the
  document's history stays a chain of deltas across the key change (`SYNC-048`).
* **The key chain is not built.** Past the tenth rotation an older key leaves the cache, and nothing
  brings it back until the chain lands (`SYNC-119`).
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
