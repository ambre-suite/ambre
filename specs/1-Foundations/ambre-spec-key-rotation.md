# AMBRE -- FEATURE: KEY ROTATION

**Type:** Horizontal (framework)
**Implementation:** partial -- a revoke lands whole, and a departure leaves a rotation owed. Every
older key is reached from the current one through a chain of links; their signer is not yet checked.

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
| `SYNC-044` | `INV` | The cache of previous keys MUST be bounded. | this spec section 3 | `ambre_sync::updates::tests::the_previous_key_cache_holds_ten` -- eleven rotations keep the ten latest previous keys, newest first, and the first key no longer opens without a link | none | verified |
| `SYNC-078` | `INV` | A key leaving the cache MUST be wiped. | this spec section 3; `ZK-011` | type-level -- the document key is an `Arc` over an `ambre_core::Secret` (`ZK-019`), so truncating the cache wipes each key whose last owner it drops and a key type that did not would be a change to the type rather than to this path | none | verified |
| `SYNC-045` | `INV` | An inbound sealed envelope MUST be opened with the key it names, never by trying every key in turn. | this spec section 3; `SYNC-125` | `ambre_sync::updates::tests::a_frame_opens_with_the_key_it_names_and_no_other` -- a frame under the current key or a cached one opens with it, and a frame naming a key never held is refused before any open | none | verified |
| `SYNC-115` | `INV` | Every document key MUST be named by an identifier derived from the key itself. | this spec section 3 | `ambre_crypto::tests::a_key_id_is_derived_from_the_key` -- one key gives one identifier, another key another, and the key's bytes are not in it | none | verified |
| `SYNC-116` | `INV` | Every rotation MUST leave a link that seals the previous key under the new one, except a rotation that records a history boundary. | this spec section 3 | `ambre_sync::sharing::tests::a_revoke_posts_the_removal_and_the_copies_in_one_request` -- the revoke's link opens under the new key to the key it replaced; `ambre_sync::sharing::tests::an_owed_rotation_is_paid_for_every_member` -- so does the link of a paid debt; `ambre_crypto::tests::a_previous_key_opens_under_the_next_one_and_no_other` -- the seal opens under the next key alone | none | verified |
| `SYNC-117` | `INV` | The relay MUST keep every link of a document for as long as the document exists. | this spec section 3; `ZK-007` | `ambre_server::routes::tests::handler_tests::the_relay_keeps_every_link_in_rotation_order` -- three rotations leave three links, served back to a member in the order they landed | none | verified |

### 1.2 Remove a member

The initiating client generates a key and wraps it for everyone who stays. It posts the removal
and every copy in one request, so the revoke lands whole or not at all.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-041` | `POST` | A revoke MUST rotate the key, so the member removed cannot open what is written after it. | `ZK-001`; this spec section 3 | `ambre_sync::sharing::tests::a_revoke_posts_the_removal_and_the_copies_in_one_request` -- the new key is wrapped for everyone but the member removed; `ambre_sync::sharing::tests::a_refused_revoke_swaps_no_key` -- a refused revoke installs no key; `ambre_server::routes::tests::handler_tests::a_revoke_removes_and_rotates_in_one_request` -- the relay removes the member and stores every copy together | none | verified |
| `SYNC-042` | `POST` | A rotation MUST leave everything sealed before it openable by every member who stays. | this spec section 3; `SYNC-119`, `SYNC-125` | `ambre_sync::documents::tests::a_name_sealed_before_a_rotation_opens_through_the_chain` -- a name sealed three rotations back opens with the current key and the chain, and not without it; `ambre_sync::media::tests::a_blob_sealed_before_a_rotation_opens_through_the_chain` -- so does a picture | none | verified |
| `SYNC-046` | `POST` | Whoever rotates MUST become the wrapper of record, so a later unwrap resolves against their key. | this spec section 3; `ambre-spec-membership.md`, `SYNC-034` | `ambre_server::routes::tests::revoke_then_rotate_replaces_owner_key` -- it reads the wrapper recorded after the rotation and fails when the inviter is still named | none | verified |
| `SYNC-047` | `POST` | The rotation notice MUST reach the other clients only after the new keys are stored. | this spec section 3; `ambre-spec-server.md` | `ambre_server::routes::tests::handler_tests::the_rotation_notice_follows_the_stored_copies` -- a revoke the store refuses publishes no notice, and a member hearing the notice of one that lands already reads the new copy, database tier | none | verified |
| `SYNC-048` | `POST` | A rotation MUST seal a fresh snapshot under the new key, so the state after it stands on its own. | this spec section 3 | not built | unbuilt -- lands with the `A snapshot follows each rotation` Story (`ambre-roadmap.md` E7) | unimplemented |
| `SYNC-079` | `FRAME` | A revoke MUST NOT change the document's content. | this spec section 3, the opening of the model | `ambre_sync::updates::tests::a_revoke_changes_no_content` -- across a completed revoke no frame leaves, no remote change reaches the engine, and a frame sealed before it still opens | none | verified |
| `SYNC-043` | `FAIL` | A member the rotation cannot re-wrap for MUST be reported, never dropped in silence. | this spec section 3 | `ambre_sync::sharing::tests::rotate_entries_cover_valid_members_and_report_malformed`; `ambre_sync::sharing::tests::revoke_reports_a_member_it_could_not_rewrap_for` -- the rotation names each member it could not re-wrap for, and the revoke reports them; `::a_malformed_exchange_key_reaches_the_report_by_name` -- a members answer whose row holds a malformed exchange key reads whole, and that member is reported by name and wrapped for by nobody | none | verified |
| `SYNC-118` | `POST` | A rotation MUST post its signed link in the same request as the wrapped copies. | this spec section 3; `SYNC-047` | `ambre_sync::sharing::tests::a_revoke_posts_the_removal_and_the_copies_in_one_request` -- the one revoke post carries the link; `ambre_sync::sharing::tests::an_owed_rotation_is_paid_for_every_member` -- so does a payment; `ambre_server::routes::tests::handler_tests::a_revoke_removes_and_rotates_in_one_request` -- the relay stores it with the copies; `ambre_server::routes::tests::handler_tests::the_relay_keeps_every_link_in_rotation_order` -- a rotation without its link, with another document's, or chained from a stale key changes nothing | none | verified |
| `SYNC-143` | `POST` | A member invited while a rotation is built MUST end up holding the new key. A rotation the relay refused for a member it did not read MUST be built again from a fresh read, and an invitation MUST name the key its copy wraps. | this spec section 3; `SRV-167`, `SRV-168` | `ambre_sync::sharing::tests::a_revoke_refused_for_a_newcomer_is_built_again_for_them` -- the second post names and wraps for the newcomer, under the same new key; `ambre_sync::sharing::tests::an_owed_rotation_refused_for_a_newcomer_is_paid_for_them` -- an owed rotation does the same; `ambre_sync::sharing::tests::a_revoke_gives_up_after_its_tries` -- the retries are bounded, and no key is swapped; `ambre_sync::sharing::tests::an_invitation_names_the_key_it_wraps` -- the copy carries its key's identifier; `ambre_sync::sharing::tests::an_invitation_overtaken_by_a_rotation_is_reported` -- a refused copy adds no one and is reported; `ambre_sync::http::tests::a_moved_membership_or_key_reads_as_its_own_refusal` -- only a 409 with its code reads as either refusal | none | verified |
| `SYNC-126` | `POST` | A revoke MUST post the removal and the wrapped copies in one request. | this spec section 3; `SYNC-041` | `ambre_sync::sharing::tests::a_revoke_posts_the_removal_and_the_copies_in_one_request` -- one read of the members, then one post that names the member and carries every copy; `ambre_server::routes::tests::handler_tests::a_revoke_removes_and_rotates_in_one_request` -- the relay applies the one request whole and leaves no debt | none | verified |

### 1.3 Take on a rotation

Every other client is told the key rotated, fetches its new wrapped copy and swaps.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-080` | `POST` | Taking on a rotation MUST install the new key and move the previous one into the cache. | this spec section 3, every other client, step 3 | `ambre_sync::updates::tests::a_rotation_installs_the_new_key_and_keeps_the_document_open` -- after the notice and the fetched key, the new key is current, the previous one is cached, and a late frame under it still opens | none | verified |
| `SYNC-081` | `FRAME` | Taking on a rotation MUST NOT close the document. | this spec section 3, a rotation is not a removal | `ambre_sync::updates::tests::a_rotation_installs_the_new_key_and_keeps_the_document_open` -- the document stays open through the notice and the swap, and no departure reaches the Logic thread | none | verified |

### 1.4 Recover a previous key

A client that lacks the key a delta or a snapshot names walks the chain back from the current key.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-119` | `POST` | A client that lacks the key a delta or a snapshot names MUST recover it by walking the links back from the current key. | this spec section 3; `SYNC-045` | `ambre_sync::chain::tests::the_current_key_walks_back_to_every_older_key` -- past the tenth rotation, the current key reaches every older key and none newer; `ambre_sync::updates::tests::an_open_walks_the_chain_to_a_snapshot_older_than_the_cache` -- an open recovers the key of a snapshot and of a delta twelve rotations back | none | verified |
| `SYNC-120` | `FAIL` | Given a link whose unsealed key does not match the identifier it names, the client MUST refuse that link and every link past it. | this spec section 3; `SYNC-115` | `ambre_sync::chain::tests::a_link_naming_another_key_stops_the_walk_there` -- a link sealing another key than it names ends the walk, and every key past it stays out of reach; `ambre_sync::chain::tests::a_link_tampered_or_misfiled_opens_nothing` -- a re-sealed link and another document's open nothing | none | verified |
| `SYNC-121` | `FAIL` | Given a link whose signer the membership log does not verify, and no rule of the application that owns the document accepts, the client MUST refuse the link. | this spec section 3; `SYNC-103`, `SYNC-106` | `ambre_sync::chain::tests::a_link_no_writer_signed_is_dropped` -- a link signed by a reader, or by a key the log never verified, is left out of the walk | none | verified |
| `SYNC-122` | `FRAME` | A key recovered from the chain MUST NOT become the current key. | this spec section 3 | `ambre_sync::chain::tests::a_recovered_key_never_becomes_the_current_one` -- the walk hands the key back and leaves the current one; `ambre_sync::updates::tests::an_open_walks_the_chain_to_a_snapshot_older_than_the_cache` -- after the open, the current key is the one installed and the cache holds nothing recovered | none | verified |

### 1.5 Leave on one's own

A member who leaves cannot choose the key the others keep. The rotation is owed to those who stay.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-127` | `POST` | A member who leaves MUST post the removal alone, and leave the rotation owed. | this spec section 3; `SRV-122` | `ambre_sync::sharing::tests::leaving_posts_the_removal_alone_and_chooses_no_key` -- one removal of the user's own share, and no other call; `ambre_server::routes::tests::handler_tests::a_member_leaves_alone_and_the_document_owes_a_rotation` -- the relay records the debt | none | verified |
| `SYNC-128` | `POST` | A client with write access that finds a rotation owed MUST rotate before it sends a delta. | this spec section 3; `SRV-123` | `ambre_sync::updates::tests::a_writer_opening_an_owed_document_pays_before_its_first_frame` -- a writer sends nothing before it pays, and a reader records no debt; `ambre_sync::rekey::tests::a_debt_found_holds_the_next_edit_until_the_key_lands` -- an edit waits for the key; `ambre_sync::sharing::tests::an_owed_rotation_is_paid_for_every_member` -- the payment wraps the key for every member; `ambre_sync::sharing::tests::a_paid_debt_posts_nothing_and_a_failed_one_is_retried` -- a debt already paid posts nothing, and a failed payment is tried again; `ambre_sync::updates::tests::a_frame_refused_for_an_owed_rotation_is_sent_again_at_its_clock` -- a refusal is how a writer in flight finds the debt; `ambre_sync::rekey::tests::a_refusal_under_the_current_key_pays_once_then_resends_at_the_same_clocks` -- many refusals start one payment | none | verified |
| `SYNC-129` | `POST` | A delta the relay refuses for its key MUST be sealed again under the current key and sent again. | this spec section 3; `SRV-123` | `ambre_sync::updates::tests::a_frame_refused_for_an_owed_rotation_is_sent_again_at_its_clock` -- the refused delta leaves again at its own clock under the new key, before the edits that waited; `ambre_sync::updates::tests::a_frame_refused_under_an_older_key_is_sent_again_at_once` -- a refusal of a key already replaced is answered at once; `ambre_sync::rekey::tests::a_refusal_under_the_current_key_pays_once_then_resends_at_the_same_clocks` -- the refused deltas come back at their own clocks, in order; `ambre_sync::rekey::tests::a_refusal_of_an_older_key_resends_without_paying` -- an older key owes no payment | none | verified |
| `SYNC-168` | `POST` | A rotation this client makes MUST reach the rest of the client, naming its document. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-channels.md`, `MURMURE-060` | `ambre_sync::updates::tests::each_rotation_this_client_makes_reaches_logic` -- a revoke and an owed rotation paid each reach Logic once | none | verified |
| `SYNC-130` | `FRAME` | Leaving MUST NOT choose the next key. | this spec section 3 | `ambre_sync::sharing::tests::leaving_posts_the_removal_alone_and_chooses_no_key` -- the departure reads no member and posts no key, so there is none to choose | none | verified |
| `SYNC-141` | `POST` | A member who does not own a document MUST be able to leave it from the sharing screen, and the screen MUST send the departure from that document alone. | this spec section 3; `SYNC-127` | `ambre_app::shell::share::tests::a_member_who_does_not_own_the_document_leaves_it_from_their_row` -- a confirmed departure sends one command, the departure from the open document; `ambre_app::shell::share::tests::each_row_offers_the_action_its_owner_may_take` -- the user's own row offers the departure | none | verified |
| `SYNC-142` | `FRAME` | The sharing screen MUST NOT offer an owner a departure. | this spec section 3 | `ambre_app::shell::share::tests::an_owner_is_offered_no_departure` -- the owner's own row carries no departure; `ambre_app::shell::share::tests::each_row_offers_the_action_its_owner_may_take` -- the owner's row offers nothing | none | verified |

* **Budget.** one elliptic-curve operation per remaining member, charged to the crypto rows of
  `ambre-spec-budgets.md section 1`, which a full snapshot seal joins once `SYNC-048` lands. The
  cache of previous keys and the unconfirmed updates are rows of `ambre-spec-budgets.md section 6`.
  A step back along the chain costs one small open and one signature check.
* **Boundary.** every wrapped copy crosses the relay and the relay can open none of them
  (`SYNC-010`). What it learns is that a document was re-keyed and by whom, which is the membership
  metadata `ambre-spec-zero-knowledge.md` accepts. It holds each link too, and opens none.
* **Verification.** 28 `verified`, 1 `unimplemented`. The relay half holds, as do the key wipe, the
  open by name, the one-request revoke, the departure's debt and the chain. The notice follows the
  stored copies, and each rotation this client makes reaches Logic.

---

## 2. WHAT EACH USER SEES

Three peers, because a revoke needs someone to remove. The last column joins the example to its
verification.

| Alice does | Bob and Carol do | The document holds | Each user sees | Test |
|---|---|---|---|---|
| removes Carol | Bob edits | Bob's edit under the new key | Carol cannot read Bob's edit; Bob works on (`SYNC-041`) | `sharing::tests::a_revoke_posts_the_removal_and_the_copies_in_one_request` |
| removes Carol, and the revoke fails | Bob edits | Bob's edit under the same key | Alice is told, Carol stays a member, and nothing changed (`SYNC-041`) | `sharing::tests::a_refused_revoke_swaps_no_key` |
| -- | Carol leaves; Bob edits | Bob's edit under a key Bob chose | Bob's client rotates before its edit leaves, and Carol cannot read it (`SYNC-127`, `SYNC-128`) | `updates::tests::a_writer_opening_an_owed_document_pays_before_its_first_frame` |
| -- | Carol leaves while Bob's edit is in flight | Bob's edit, refused, then sent again under the new key | Bob sees nothing happen; Carol cannot read the edit (`SYNC-129`) | `updates::tests::a_frame_refused_for_an_owed_rotation_is_sent_again_at_its_clock` |
| removes Carol | Bob reopens the document | Alice's content, a name and pictures under the previous key | the name and every picture, opened through the chain (`SYNC-042`) | `documents::tests::a_name_sealed_before_a_rotation_opens_through_the_chain` |
| removes Carol, whose row Bob's relay serves malformed | Bob reopens | Alice's content | Alice is told the rotation could not reach Bob, by name (`SYNC-043`) | `sharing::tests::revoke_reports_a_member_it_could_not_rewrap_for` |
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

1. fetch the members and their exchange keys;
2. generate a new document key from the system generator;
3. wrap the new key once per member but the one removed (`SYNC-035`, `SYNC-043`);
4. post the removal and every wrapped copy in one request (`SYNC-126`). The relay applies them in
   one transaction, records the rotator as the wrapper (`SYNC-046`) and broadcasts the notice after
   the commit (`SYNC-047`). The signed link joins the same request with the chain (`SYNC-118`).
   The request names every member the client read. A member invited since is in none of them, so
   the relay refuses the whole request and the client starts again at step 1 (`SYNC-143`);
5. move the previous key into the cache, install the new one, and report the revoke done;
6. seal a fresh snapshot under the new key (`SYNC-048`).

**An invitation racing a rotation.** Both take the document's lock on the relay, so one lands
first. If the invitation does, the rotation that read the members before it is refused and read
again (`SRV-167`). If the rotation does, the invitation wraps a key it replaced and is refused
(`SRV-168`); the user invites again, under the new key.

**A departure, in steps.** The member who leaves posts the removal alone (`SYNC-127`). They do not
choose the next key: the members who stay would then trust a key the leaver picked (`SYNC-130`).

1. The relay records the removal, and marks the document as owing a rotation (`SRV-122`).
2. It refuses every delta sealed under the key the removal left behind (`SRV-123`).
3. The first member client to send finds the debt, and rotates first (`SYNC-128`).
4. A delta refused for its key is sealed again under the new key and sent again (`SYNC-129`).

A writer finds the debt in the members answer, at the open or at the notice a departure sends. It
pays at its next edit, so a debt nobody writes after costs nothing. An edit made meanwhile waits
unsealed for the new key. Each rotation this client makes, paid or revoked, reaches Logic by its
document (`SYNC-168`). An application whose history is split acts on it.

**Why a refused delta keeps its clock.** The relay refuses before it reads the clock, so the clock
is still free, and so is every one after it. The worker keeps each update it sent until the relay
echoes it back. On a refusal, it seals the refused updates again under the new key, at their own
clocks and in order. A late refusal names the key the delta no longer carries, and is ignored.

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
pub struct KeyLink {
    pub document_id: Id<Document>,
    pub key: KeyId,                 // the key the link starts from
    pub previous: KeyId,            // the key it seals
    pub sealed: SealedPreviousKey,  // `previous` as a sealed object under `key`
}
```

The seal is a standalone object of the sealed envelope. Its header names `key` and the rotator's
signing key, and carries the signature. Its additional data binds the document and `previous`, so
a link opens for its own document and key only.

A link is checked twice. The key it unseals must hash to the identifier it names (`SYNC-120`), and
its signer must be a verified member (`SYNC-121`). A recovered key opens history, and never seals
anything new (`SYNC-122`).

**Why the chain stands before the log.** The seal and the identifier carry the chain alone. To forge
a link, the relay would have to seal under a key it never holds. The hash binds what opens to the
key it names.

The signature adds attribution, and attribution means something only against the signed
membership log. A link counts only when its signer's key is one the log verifies for a member who
may write (`SYNC-121`). A document whose log is empty verifies no one, so its links stand on the
seal and the hash alone.

**The relay chains one rotation at a time.** It stores a link in the transaction that stores the
wrapped copies, under a lock on the document. A link that does not start from the last key chained
is refused with a conflict, and nothing of its rotation lands. Two rotations racing from one key
thus leave one link, never a fork.

**Where the links come from.** The open fetches every link of the document with the members answer.
A rotation this client posts adds its own link, and a rotation it is told of fetches them again. A
name in the document list that names an older key fetches its document's links alone.

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
| every media blob already uploaded | no. Same: it names its key, and the chain opens it |
| a snapshot uploaded before the rotation | no, and that is correct: it is history, and the cache or the chain opens it |

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the revoke, the departure, the payment of a debt, and the
  re-wrap for each member;
* `crates/ambre-sync/src/rekey.rs` -- the updates kept until their echo, and the debt;
* `crates/ambre-sync/src/websocket.rs` -- the notice, the fetch, the unwrap, the swap and the refusal;
* `crates/ambre-sync/src/lib.rs` -- the cache of previous keys, and the opens by name;
* `crates/ambre-sync/src/chain.rs` -- the walk back from the current key;
* `crates/ambre-sync/src/pipeline.rs` -- the seal and the open of one link;
* `crates/ambre-protocol/src/key_link.rs` -- the link as the relay stores it;
* `crates/ambre-server/src/db/members.rs` -- the transaction, the wrapper of record and the link.

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
| The relay refuses the key a removal left behind | trusting the next writer to rotate | until someone rotates, a delta under the old key would reach the member who left |
| Older keys come back through a chain of links | keys forwarded one by one by any member (Matrix MSC3061) | a key that arrives on its own proves nothing about its sender, and that flaw was exploited in Matrix |
| A key identifier derived from the key | a random identifier | a random one binds nothing: a false link would name a real identifier and seal a false key |
| History is not re-keyed | re-sealing every stored snapshot on every rotation | a snapshot from before the rotation is what the removed member already read, so re-sealing it protects nothing and costs a full upload per rotation |
| The Logic thread is not involved | the Logic thread orchestrating the steps and asking the worker for each one | it does no cryptography and no network by design, and a rotation is both. Keeping it out means the key never crosses the boundary, which is the whole reason the boundary is there |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A departure is confirmed on screen only.** The sharing screen asks once, then sends it
  (`SYNC-141`). Nothing asks the relay whether the user is the last owner; an owner is simply never
  offered the departure (`SYNC-142`).
* **A rotation outrun three times gives up.** Members who keep joining between the read and the
  post end the revoke with an error after three tries (`SYNC-143`). The user revokes again.
* **A member with an unparsable exchange key is reported, and still loses the document.** The
  rotation proceeds without them so the rest converge, and the user is told who was left out.
  Nothing re-wraps for that member later; a fresh invitation is the only way back (`SYNC-043`).
* **A key rotated away still opens on the client.** An inbound sealed envelope opens with the key it
  names, and a cached previous key is one of them (`SYNC-045`). That is the in-flight tolerance the
  cache exists for. Refusing a delta under the key a removal left behind is the relay's job
  (`SRV-123`).
* **No snapshot follows a rotation.** The state after the removal is never sealed on its own, so the
  document's history stays a chain of deltas across the key change (`SYNC-048`).
* **The relay can withhold a link.** It cannot forge one, and a link it drops ends the walk there: the
  history before it reads as closed, not as wrong. Nothing tells the user it was withheld.
* **A revoke runs from an open document only.** It seals the current key under the new one, and the
  worker holds the current key only while the document is open.
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
