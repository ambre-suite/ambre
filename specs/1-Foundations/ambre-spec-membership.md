# AMBRE -- FEATURE: MEMBERSHIP AND THE WRAPPED KEY

**Type:** Horizontal (framework)
**Implementation:** partial -- the wrap, the unwrap, the directory and the role all ship; a
transient failure of the members request opens the document as if it were new.

> Sharing a document hands one person the key to it, sealed for that person alone. The relay stores
> the sealed copy and can open none of them. What the recipient may then do, and whose cursor is
> whose, come from the same answer.

One request answers three questions at once: which key opens this document, who the other members
are, and what this client is allowed to do. This document owns all three, because they arrive
together and drift apart the moment they are fetched twice.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md` and the four other documents of this layer.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SYNC-010` | A shared document key MUST be wrapped for the invitee's exchange key alone. | this spec section 3 | `ambre_sync::sharing::tests::invite_request_wraps_for_the_invitee_only`; `ambre_crypto::tests::kek_binds_peer_identity`, `::wrap_unwrap_roundtrip` | verified |
| `SYNC-012` | A failed unwrap of a document key MUST NOT be replaced by a freshly generated one. | derived (audit `H3`) | none. The unwrap failure now stops the open and surfaces an authentication error, and no test drives an unwrappable key row to see it: the open path runs only in the browser | unverified |
| `SYNC-014` | An ephemeral frame MUST reach the Logic thread under the identity the member directory maps its author key to. | this spec section 3 | `ambre_sync::ephemeral_tests::author_resolves_to_the_member_behind_the_signing_key`; `ambre_sync::sharing::tests::presence_directory_maps_valid_entries`, `::list_members_refreshes_the_presence_directory` | verified |
| `SYNC-015` | An ephemeral frame whose author key matches no member MUST be dropped, never delivered under a guessed identity. | this spec section 3 | `ambre_sync::ephemeral_tests::unknown_author_key_is_not_resolved`; `ambre_sync::sharing::tests::presence_directory_skips_unparsable_rows_without_inventing_ids` | verified |
| `SYNC-016` | The role carried on the ready signal MUST be this client's own row in the members answer. | this spec section 3; `ambre-spec-message-bus.md` | `ambre_sync::sharing::tests::own_role_reads_our_own_row`, `::own_role_reads_writable_roles`; `ambre_sync::updates::tests::document_ready_carries_the_role_on_both_paths` | verified |
| `SYNC-017` | A role that cannot be resolved MUST default to the read-only one, never to a writable one. | this spec section 3 | `ambre_sync::sharing::tests::own_role_falls_back_to_viewer_when_unresolvable` | verified |
| `SYNC-034` | A recipient MUST unwrap against the exchange key of whoever wrapped, never against its own. | this spec section 3 | `ambre_crypto::tests::ecdh_symmetric`, `::kek_binds_peer_identity` hold the primitive. The choice of which key to use is a branch in the client with a fallback for an older relay, and no test takes either side of it | unverified |
| `SYNC-035` | A wrap MUST be refused for a peer key with no contribution, rather than sealed under a predictable secret. | this spec section 3 | `ambre_crypto::tests::weak_peer_public_key_rejected`; `ambre_sync::sharing::tests::invite_request_propagates_bad_pubkey`, `::decode_exchange_pubkey_rejects_bad_input` | verified |
| `SYNC-036` | The re-fetched invitee key MUST equal the one the user confirmed, before anything is wrapped. | `ambre-spec-key-verification.md` | `ambre_sync::sharing::tests::invite_pin_mismatch_refuses_and_never_wraps`, `::fetch_invitee_key_emits_key_and_matching_fingerprint` | verified |
| `SYNC-037` | The member directory MUST be rebuilt whenever membership changes, so a revoked member stops resolving. | this spec section 3 | `ambre_sync::sharing::tests::list_members_refreshes_the_presence_directory`; `ambre_sync::ephemeral_tests::refreshing_the_directory_drops_revoked_members` | verified |
| `SYNC-038` | The member list shown to the user MUST NOT carry an invented identity. | this spec section 3 | none, and **the rule fails today**: a member row whose identifier does not parse is given a freshly generated one, so the list shows a person nothing can act on and a revoke aimed at that row addresses nobody -- section 6 | unverified |
| `SYNC-039` | No key material MUST cross to the Logic thread on a member row. | `ZK-002` | type-level: the member row the bus declares carries an identity, a name, an address and a role, and no key field, so there is no shape for one to travel in | verified |
| `SYNC-040` | A transient failure of the members request MUST NOT be read as a document that does not exist. | this spec section 3 | none, and **the rule fails today**: a network failure and a genuine absence take the same branch, so the open generates a throwaway key, marks the document local and never hydrates -- section 6 | unverified |

* **Budget.** the key exchange costs one elliptic-curve operation per member at open, about 150 us
  each on mobile, charged to the crypto rows of `ambre-spec-budgets.md section 1`. The
  directory and the role are read off an answer the open already fetches, so both are free.
* **Boundary.** the wrapped key crosses the relay, and the relay can open none of them: `SYNC-010`
  seals each copy for one exchange key. What the relay learns is who is a member of what, which is
  the membership metadata `ambre-spec-zero-knowledge.md` accepts. No plaintext key and no
  private key ever leaves the Sync worker (`SYNC-002`, `SYNC-039`).
* **Verification.** 9 `verified`, 4 `unverified`, 0 `unimplemented`. What holds is every pure
  transform: the wrap for one invitee, the refusal of a weak key, the pin comparison, the directory
  build and the role resolution, all extracted out of the browser and all covered. What is loose is
  the four branches that only run against a live relay -- and two of them are stated as failing.

---

## 2. WHAT EACH USER SEES

Three peers, because sharing needs a third. The last column joins the example to its verification.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| invites Bob after confirming his key | opens the document | Alice's content | Bob reads it: the key was wrapped for him alone (`SYNC-010`) | `sharing::tests::invite_success_posts_wrapped_key_and_emits_invitesent` |
| invites Bob, and the relay serves a different key at the wrap | -- | nothing new | Alice is told the invite is refused, and nothing is wrapped (`SYNC-036`) | `sharing::tests::invite_pin_mismatch_refuses_and_never_wraps` |
| invites Bob as a reader | tries to edit | Alice's content | Bob sees no editing affordance (`SYNC-016`) | `sharing::tests::own_role_reads_our_own_row` |
| invites Bob, whose row the relay serves malformed | opens the document | Alice's content | Bob's cursor never appears on Alice's screen, rather than appearing as someone else (`SYNC-015`) | `sharing::tests::presence_directory_skips_unparsable_rows_without_inventing_ids` |
| moves her cursor | watches on two devices | nothing | one Alice, in one colour (`SYNC-014`, `PRESENCE-005`) | `ephemeral_tests::author_resolves_to_the_member_behind_the_signing_key` |
| opens the document while the network drops the members answer | edits | Bob's edit | Alice gets an empty document she can edit and nobody receives (`SYNC-040`) | none, and it fails -- section 6 |

---

## 3. MODEL

**CRDT containers.** none. Membership is the relay's table, not the document's content: putting it
in the document would make every member's identity part of the encrypted state and unable to change
without a delta.

**One answer, three uses.** The members request is issued once at open and again whenever
membership changes. Every row of it carries the same four fields, and each is read by a different
rule.

| Field of a member row | Answers | Rule |
|---|---|---|
| the wrapped document key | which key opens this document, for this client | `SYNC-012`, `SYNC-034` |
| the wrapper's exchange key | whose key to run the exchange against | `SYNC-034` |
| the signing key | which member authored an ephemeral frame | `SYNC-014`, `SYNC-015` |
| the role | what this client may do, and what to show for each member | `SYNC-016`, `SYNC-017` |

**The wrap.** The document key is sealed under a key derived from an X25519 exchange between the
wrapper's private key and the recipient's public one. The shared secret is symmetric, so the
recipient must run the exchange against the **wrapper's** public key, never against its own
(`SYNC-034`). Who wrapped is recorded by the relay, per member row: the inviter for a shared
member, the member themselves for the owner's copy at creation.

**Why the own key looks like it works.** A document with one member is a self-wrap, so the exchange
degenerates and a client using its own key derives the correct secret. The defect only appears on
the first shared document, which is why the rule is stated rather than left to the code.

**An invite, in two phases.** The trust step between them is
`ambre-spec-key-verification.md`; this document owns the wrap.

1. resolve the invitee, fetch their exchange key and compute its fingerprint. **Nothing is
   wrapped**;
2. the user confirms the fingerprint out of band;
3. re-resolve the invitee, and refuse the invite when the key served now differs from the confirmed
   one (`SYNC-036`);
4. wrap the document key for that key, and refuse a key with no contribution (`SYNC-035`);
5. post the wrapped copy. The relay pushes a notice to the invitee, who is not yet subscribed to
   the document.

**The directory is membership, not trust.** It answers "which member wrote this frame", trusting the
relay's mapping of keys to people. Detecting a relay that substitutes a member's key is the
trusted-key store's job (`ambre-spec-key-verification.md`), and presence does not wait on it.

**A row that does not parse is handled two ways, deliberately.**

| Where | A bad row becomes | Why |
|---|---|---|
| the directory | skipped | a cursor under the wrong name is worse than no cursor (`SYNC-015`) |
| the role | the read-only role | hiding an affordance the user has beats offering one the relay refuses (`SYNC-017`) |
| the list shown to the user | a fresh identifier, which is the defect | nothing chose it: the list degrades rather than dropping the row (`SYNC-038`) |

**A document the relay does not know is this client's own.** A missing document on open means it
exists on this client alone and has not been created on the relay yet, so the local user is the only
party and holds the owning role. That inference is right for an absence and wrong for a failure,
which is `SYNC-040`.

**Code.**

* `crates/ambre-sync/src/sharing.rs` -- the wrap, the directory, the role, the invite and the list;
* `crates/ambre-sync/src/members.rs` -- the handlers that gather the state and spawn the requests;
* `crates/ambre-sync/src/updates.rs` -- the unwrap at open, and the wrapper-key choice;
* `crates/ambre-crypto/src/ecdh.rs` -- the exchange, the derivation and the seal of the key itself.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a share entry in a document's menu | phase 1: the invitee is resolved and their fingerprint shown |
| confirming the fingerprint | phase 2: the key is wrapped and posted (`SYNC-036`) |
| opening a member list | the list is refetched, and the directory is rebuilt from it (`SYNC-037`) |
| changing a member's role | the relay is asked, and the answer refreshes the list |
| receiving a share notice | the document list refreshes, and the user is told, with no reload |
| opening a document | the key, the directory and the role all arrive on one answer |

What a user sees of the role is an affordance that is present or absent. The relay stays the
authority: the role here hides what the relay would refuse anyway.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One answer carries the key, the directory and the role | the three change together -- an invite adds a key, a member and a role in one act -- and they are read at the same instant, at open. Fetching them apart means three round trips and three moments at which they disagree | a request per question, which is what a reader expects and what makes a stale directory possible |
| The recipient unwraps against the wrapper's key | the exchange is symmetric, so only one of the two public keys yields the secret, and the relay is the only party that knows which member wrapped. Recording the wrapper per row makes it answerable with no extra state on the client | unwrapping against the member's own key, which happens to work for a single-member document and fails silently on the first shared one |
| The directory maps signing keys, and lives in the Sync worker | the signing key is per session and per device, so presence keyed on it shows one collaborator twice. The resolution has to happen where signatures are verified, which is this worker and nowhere else | resolving in the Logic thread, which would need the keys to cross the boundary `SYNC-039` closes |
| An unresolvable author is dropped | presence is best-effort and self-correcting: the next frame arrives a moment later. An identity guessed from the payload is a peer's own claim, which is exactly what keying on a resolved identity exists to refuse | trusting an identifier in the payload, or attributing the frame to the first member |
| An unresolvable role is the read-only one | the role is a user-experience gate, so the failure directions are asymmetric: hiding an affordance the user has costs a click through a menu, offering one the relay refuses costs a confusing error after the edit | defaulting to the owning role, or refusing the open |
| A failed unwrap stops the open | a random key opens the document and then poisons it: this client seals its edits under a key nobody else holds, and every peer sees a commitment failure on content that looks like ordinary history | opening with a fresh key and repairing later, which was the behaviour audit `H3` found |
| Membership is the relay's table | a document that carried its own membership would need a delta to add a member, so the relay could not answer "who may read this" without opening the document, and no invite could reach someone who cannot yet decrypt | membership as a CRDT container inside the document |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A transient failure of the members request opens the document as new.** The absence and the
  failure take one branch, so a network blip yields a fresh key, a local-only document and no
  hydration. The user edits a document that reaches nobody (`SYNC-040`).
* **The member list invents an identifier for a row it cannot parse.** The list is display-only,
  and every action the user takes from it -- a revoke, a role change -- carries that identifier to
  the relay, where it matches nobody (`SYNC-038`).
* **The wrapper-key choice has an untested fallback.** A member row with no wrapper recorded falls
  back to the member's own key, for an older relay. Nothing states how long that fallback lives, and
  no test takes either branch.
* **Nothing states what a role change does to an open document.** The relay answers the request and
  the list refreshes; the role carried on the ready signal was read at open and is not revisited, so
  a demotion takes effect on the next open.
* **The directory is only rebuilt when someone asks for the list.** An invite or a revoke that this
  client learns about another way leaves the directory as it was, so a new member's cursor does not
  appear until something refetches.
* **No test drives an open.** The whole open path -- the members request, the unwrap, the directory
  and the role -- runs in a browser, and the host tier reaches its pure transforms alone.
* **Nothing rules on a member who cannot be reached.** An invitee whose address resolves to no
  account is refused; an account that exists and never opens the document keeps a wrapped key for
  ever, and no rule bounds that.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The trust step before a key is wrapped, and what a fingerprint is | `ambre-spec-key-verification.md` |
| What a revoke rotates, and what a rotation leaves behind | `ambre-spec-key-rotation.md` |
| How a frame is signed, and what refuses a forged author | `ambre-spec-sealed-envelope.md` |
| What a presence frame carries, and what it paints | `ambre-spec-presence.md` |
| What the relay stores per member, and which role may do what | `ambre-spec-server.md` |
| Where the role gate is applied on a command | `ambre-spec-message-bus.md` |
| Which key opens the document once it is unwrapped, and who holds it | `ambre-spec-sync.md`, `SYNC-002` |
| What the sharing screens look like | `ambre-spec-chrome.md`, `ambre-spec-launcher.md` |
