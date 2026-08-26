# AMBRE -- FEATURE: THE BLIND RELAY

**Type:** Horizontal (framework)
**Implementation:** partial -- the register, the roles and the three stores all ship; deleting a
document leaves its state behind, and the brute-force limiter is never selected.

> The one process every client talks to. It routes, it orders and it keeps, and it can read none of
> what it holds: a document is a row of metadata and a pile of sealed bytes.

This document owns the relay itself: what it is, what it stores, who may touch a document, and what
a caller gets back when something goes wrong. What it does with a token, a delta, a socket, a blob
or an organization is the five documents beside it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-session.md`, `ambre-spec-sequencer.md`, `ambre-spec-broadcast.md`,
`ambre-spec-object-store.md` and `ambre-spec-organizations.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SRV-001` | The relay MUST NOT store or log a plaintext document payload. | `ZK-001`, `ZK-006`; this spec section 3 | none beyond `ZK-006`: reviewer attention, plus the explicit "never log the blob" rule the flush path carries. Nothing refuses the log line that would break it | human-review |
| `SRV-005` | A non-member MUST receive no metadata about a document -- not its owner, not its kind, not its existence. | audit `H2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::get_document_hidden_from_non_member`, and `::create_document_with_foreign_existing_id_is_not_found` for the same answer on a taken identifier | verified |
| `SRV-007` | Every update, snapshot and presence frame MUST carry a valid signature by the key the caller's token names; an unsigned or forged frame MUST have no effect. | this spec section 3; `ZK-005` | `ambre_server::ws::handler::message::ed25519_tests::invalid_signature_is_rejected`, `::tampered_ciphertext_rejects_signature`, `::unsigned_ephemeral_is_rejected`; `ws::handler::message::ephemeral_binding_tests::forged_author_key_writes_no_presence`; `routes::tests::handler_tests::create_snapshot_rejects_forged_author` | verified |
| `SRV-008` | A member who may only read MUST produce no write. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::viewer_role_delta_writes_no_pending` on the live path, and `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` on the request path | verified |
| `SRV-011` | A document MUST NOT be left without an owner. | this spec section 3 | `ambre_server::routes::tests::handler_tests::owner_cannot_drop_last_owner`, `::owner_can_leave_when_another_owner_remains` | verified |
| `SRV-012` | The relay MUST perform no cryptographic operation beyond verifying a signature and minting its own tokens. | this spec section 5; `ZK-004` | crate graph: `deny.toml` refuses the cipher, exchange and secret-sharing crates in this process, and the continuous-integration `deny` job fails the manifest that adds one | verified |
| `SRV-013` | A failure of the rate limiter MUST close the sign-in tier, never open it. | audit `M6`; this spec section 3 | none, and **the rule fails today**: the limiter runs inside the nested router, which strips the prefix its classifier matches on, so no request is ever read as a sign-in request. `ambre_server::middleware::rate_limit::tests::only_auth_tier_fails_closed` asserts the tier's own answer and never reaches the classification -- section 6 | unverified |
| `SRV-014` | Relay code MUST contain no unchecked memory access. | README pillar "Security" | measured zero across the workspace, and this crate carries no crate-level refusal of it: two crates of seventeen do (`ambre-spec-testing.md section 8`) | unverified |
| `SRV-015` | Only an owner MUST be able to remove a member or re-key a document. | this spec section 3 | `ambre_server::routes::tests::handler_tests::rotate_key_denied_to_editor`, `::revoke_then_rotate_replaces_owner_key`, `::delete_document_only_by_owner` | verified |
| `SRV-016` | A caller MUST NOT grant a role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::share::tests::editor_cannot_grant_owner`, `::owner_can_grant_any_valid_role`, `::doc_role_rank_ordering` | verified |
| `SRV-070` | A caller MUST NOT change a member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::editor_cannot_demote_owner_via_invite`. The invitation is also the role-change path, so this is the half a clamp on the granted role alone leaves open | verified |
| `SRV-017` | Deleting a document MUST leave no relay state behind it. | this spec section 3 | none, and **the rule fails today**: the delete removes the rows and nothing else, so the document's sequence counter and its per-author clock hash stay in the ephemeral store with no expiry. Answered by the `A deleted document leaves nothing` Story | unverified |
| `SRV-018` | A route that reads a document row MUST require an authenticated caller. | this spec section 3 | none, and **the rule fails today**: the single-document route accepts an anonymous caller, queries the register, and only then answers not-found. The answer is right and the query runs first -- section 6 | unverified |
| `SRV-019` | The rate limiter MUST bucket a request by its authenticated caller, or by the socket peer when there is none. | audit `M6` | `ambre_server::middleware::rate_limit::tests::client_ip_from_forwarded_header`, `::client_ip_falls_back_to_remote`, `::client_ip_ignores_empty_forwarded`. A forwarded address is read only where the deployment declares a proxy, and no test drives that switch | unverified |
| `SRV-020` | A failure of a backing service MUST reach the caller as a generic error, never as its cause. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message`, `::oidc_error_is_infrastructure_level`, `::application_errors_return_their_message` | verified |
| `SRV-021` | Any instance MUST be able to serve any client: no request state MUST live in the process. | this spec section 3 | none. Every store is external and the one process-wide object is the fan-out hub, which holds no request; nothing boots two instances to observe it. The one exception is stated by `ambre-spec-session.md`, `SRV-024` | unverified |
| `SRV-068` | A cross-origin request MUST be refused unless the deployment names its origin. | audit `M17`; this spec section 3 | `ambre_server::cors_tests::parses_and_trims_multiple_origins`, `::empty_list_yields_no_origins`. The permissive fallback is reachable only in development mode and is announced at start-up | verified |
| `SRV-069` | The page that loads the client MUST be served cross-origin isolated. | `ambre-spec-architecture.md section 2` | none. The relay serves the requests and not the page, so the two headers are the deployment's obligation on whatever serves the page and the worker bootstrap; nothing in continuous integration reads them back | unverified |

* **Budget.** the per-operation targets of `ambre-spec-budgets.md section 3`, and the roughly
  50 KB per open document of `ambre-spec-budgets.md section 6`. Every relay row of that
  register reads **none exists**: no
  bench measures the relay today, which is the `The performance budgets given a mechanism` Story.
* **Boundary.** this document is the relay side of the boundary
  `ambre-spec-zero-knowledge.md` draws. `SRV-001` keeps the payload out of the process and
  out of the logs, `SRV-012` refuses the relay the ability to open one, and `SRV-005` keeps the
  routing metadata itself behind the membership check.
* **Verification.** 10 `verified`, 7 `unverified`, 1 `human-review`, 0 `unimplemented`. What holds is
  what a request test can drive: the roles, the rank clamps, the signature gate, the error mapping,
  the non-member answer and the origin list. What is loose is every rule about the process or the
  deployment rather than the request -- the limiter's own classification, the state a delete leaves,
  the claim that two instances are interchangeable, and the two headers the page owes. Three rules
  are stated as failing on a reading of the code.

---

## 2. WHAT EACH USER SEES

Two peers over one document. The relay holds the membership, so what each peer may do is a row of
its table and not a client decision.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| creates a document | asks for it by identifier | one row, one member | Alice's document; Bob is told it does not exist (`SRV-005`) | `routes::tests::handler_tests::get_document_hidden_from_non_member` |
| invites Bob as a reader | edits | two members, one of them read-only | Bob's edit refused, Alice's document unchanged (`SRV-008`) | `ws::handler::message::delta_size_cap_tests::viewer_role_delta_writes_no_pending` |
| invites Bob as an editor | invites Carol as an owner | two members, and no third | Bob's invitation refused: he may not grant above himself (`SRV-016`) | `routes::share::tests::editor_cannot_grant_owner` |
| leaves her own document | is the other owner | one member, an owner | Alice leaves, the document keeps an owner (`SRV-011`) | `routes::tests::handler_tests::owner_can_leave_when_another_owner_remains` |
| is the only owner and leaves | watches | two members, unchanged | Alice is refused: the document would have no owner (`SRV-011`) | `routes::tests::handler_tests::owner_cannot_drop_last_owner` |
| deletes the document | had it open | no row, and the ordering state (`SRV-017`) | both lists lose it; the ephemeral store keeps its counters | none, and it fails -- section 6 |

---

## 3. MODEL

**CRDT containers.** none. The relay compiles no CRDT at all (`ZK-004`): a document's content is
opaque bytes it moves and never opens.

**Three stores, and one fact in one of them.** Nothing is duplicated between them, which is what
makes a restart of any one of them survivable.

| Store | Holds | Lost on a reset |
|---|---|---|
| the durable register | users, organizations, documents, members, and the index of every snapshot and delta | everything: it is the authority |
| the ephemeral store | the sequence counters, the per-author clocks, the unflushed deltas, the refresh tokens, the presence blobs and the sign-in handshakes | recoverable for the ordering (`ambre-spec-sequencer.md`), final for a session |
| object storage | every sealed blob: snapshots, deltas and media | the content itself, which nothing else holds |

**The register.** One row per document, and none of it is content.

| Field | Form on the relay |
|---|---|
| the identity, the kind, the owner, the times | plaintext routing metadata |
| the name | a sealed blob, opened by the client (`ambre-spec-sync.md`, `SYNC-067`) |
| one member row per participant | a role, plus the document key sealed for that member |
| the wrapper of a member's key | the identity whose exchange key seals it, which the client needs to open it |

A document with no name stores nothing rather than a placeholder, and the listing returns each
sealed name beside the caller's own sealed key, so a client reads its whole list in one answer with
no document open (`ambre-spec-launcher.md`).

**The three roles**, ranked. A rank decides both what a caller may do and what it may grant.

| Role | May read | May write | May invite | May remove, re-key or delete |
|---|---|---|---|---|
| owner | yes | yes | up to owner | yes (`SRV-015`) |
| editor | yes | yes | up to editor (`SRV-016`) | no |
| viewer | yes | no (`SRV-008`) | no | no |

**Why the clamp is on the target too.** An invitation of an existing member is also the role-change
path, so a clamp on the granted role alone lets an editor re-invite an owner as an editor and
overwrite that owner's sealed key on the way. The target's current rank is clamped as well
(`SRV-070`).

**The request pipeline.** Four gates, in this order, and each one is the reason the next is cheap.

1. the rate limiter picks a tier from the path and a bucket from the caller (`SRV-013`, `SRV-019`);
2. the token is verified locally, with no store touched (`ambre-spec-session.md`, `SRV-003`);
3. the membership and the role are read from the register (`SRV-005`, `SRV-008`);
4. the handler runs, and any failure becomes one status and one code (`SRV-020`).

**What a failure becomes.** One mapping, and the caller learns nothing about the cause of an
infrastructure failure.

| Failure | Answer |
|---|---|
| no token, an expired token | `401`, and the client renews once (`ambre-spec-session.md`) |
| a role too low, a membership missing on a write | `403` |
| a document a caller may not see, or that does not exist | `404`, the same answer for both (`SRV-005`) |
| a name already taken, a payload that will not decode | `409` |
| a signature that does not verify | `422` |
| a blob larger than its cap | `413` (`ambre-spec-object-store.md`) |
| the register, the ephemeral store or object storage | `500`, one generic message, the cause in the log alone (`SRV-020`) |

**What every answer carries**, whatever the route.

| Header | Value, and why |
|---|---|
| the strict transport policy | two years, subdomains included: the relay is reachable over one transport only |
| the content policy | the deployment's own origin alone, which a deployment adjusts for the provider it configures |
| the allowed origins | the list the deployment names, and nothing when it names none (`SRV-068`) |

The two headers that make the client's shared memory legal -- the opener policy and the embedder
policy -- belong to whatever serves the page and the worker bootstrap, not to this process
(`SRV-069`, `ambre-spec-architecture.md section 2`). Their cost is that an embedded cross-origin
resource must carry a policy of its own, which is why a presigned link's target must allow a
cross-origin read.

**Where the process is stateless, and where it is not.** Every store is external, so a client
reaches any instance and the fan-out crosses them (`SRV-021`). Two things are per process and
neither is per request: the signing key derived from the deployment's seed, which every instance
must share, and the fan-out hub (`ambre-spec-broadcast.md`).

**Code.**

* `crates/ambre-server/src/main.rs` -- the boot, the route table, the layers and the periodic tasks;
* `crates/ambre-server/src/state.rs` -- the handles and every operational figure;
* `crates/ambre-server/src/routes/documents.rs` -- the register, the users and the member list;
* `crates/ambre-server/src/routes/share.rs` -- the invitation, the removal, the ranks and the re-key;
* `crates/ambre-server/src/db/` -- every query, one module per subject;
* `crates/ambre-server/migrations/` -- the durable schema, and the source of truth for every column;
* `crates/ambre-server/src/error.rs` -- the one mapping of section 3;
* `crates/ambre-server/src/middleware/rate_limit.rs` -- the tiers, the buckets and the window.

---

## 4. SURFACE

The relay's surface is its routes. Every one below is under `/api`, and every one but the sign-in
group needs a token.

| Group | Routes | Owner |
|---|---|---|
| sign-in | the provider handshake, the account setup, the development sign-in, the renewal, the sign-out, the caller's own profile | `ambre-spec-session.md` |
| keys | storing the caller's sealed master material, and re-sealing it under a new passphrase | `ambre-spec-session.md` |
| users | the search by address, and one user's public keys | this document, and `ambre-spec-key-verification.md` for what a client does with them |
| documents | create, list, read, rename, delete | this document |
| members | invite, remove, change a role, list, re-key | this document, and `ambre-spec-key-rotation.md` |
| content | the snapshot record, the delta page, and the presigned links | `ambre-spec-sequencer.md`, `ambre-spec-object-store.md` |
| media | the two presigned links and the confirmation | `ambre-spec-object-store.md` |
| organizations | the organization, its members and the escrow release | `ambre-spec-organizations.md` |
| the socket | one upgrade | `ambre-spec-broadcast.md` |

**What the surface does not carry.** Channels and meetings have no route at all: the register
declares their tables and nothing serves them (`ambre-spec-broadcast.md`, `SRV-047`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The relay is blind by capability, not by discipline | a rule that says "do not decrypt" is held by review; a process that cannot link a cipher cannot decrypt whatever anyone writes. The manifest is the check, and re-granting a capability is an edit a reviewer meets (`SRV-012`, `ZK-004`) | trusting the code to avoid the operation, which is what the manifest allowed until three unused cipher crates were found declared in it |
| The one crypto library the relay links is the signature one | verifying a signature is the only cryptographic act the relay has a reason to perform. Importing the project's own crypto crate would put the document key and the master seed in reach of this process for one call it does not need | depending on `ambre-crypto`, which is the shape every other crate takes |
| Every fact lives in one store | a fact in two stores is a fact that disagrees with itself after any partial failure. The register is the authority, the ephemeral store is a cache with an owner, and object storage holds only what neither can | mirroring the sequence into the register, which would make a Redis reset invisible and a Redis write a two-phase commit |
| A non-member gets the same answer as a caller of a document that does not exist | telling an arbitrary caller that a document exists, and who owns it, leaks the shape of everyone's workspace to anyone holding an identifier. One answer for both cases leaks nothing and costs the legitimate caller nothing (`SRV-005`) | a distinct `403`, which is what the route did before audit `H2` |
| A role is ranked, and both the grant and the target are clamped against the caller | roles that are a set of strings make every new capability a new comparison, written once per route and forgotten once. A rank is one comparison, and clamping the target closes the re-invite path a grant clamp leaves open (`SRV-016`) | a capability check per route, which is where the editor-demotes-owner path came from |
| The wire format is binary on the socket and textual on the requests | the socket carries a frame per keystroke burst and a compact encoding is what makes that affordable; the request surface is read by operators and tools, and a binary body would cost them a decoder for no measurable gain | one encoding for both |
| The relay terminates no transport of its own | a reverse proxy already terminates the modern transports, and putting a QUIC stack inside this process makes an operational concern a code concern. The relay speaks the simplest protocol to something that speaks every other | terminating the modern transport in this process |
| A document's kind is named after the application, in the project's own vocabulary | the kinds are a closed set the register checks, and naming them after the generic object -- a spreadsheet, a document, a board -- puts a second vocabulary beside the product's own, so every reader has to hold a translation table | naming a kind after the generic object it resembles |
| The queries are written out, against a compile-time check | the queries here are the boundary's own enforcement -- a membership filter, a scoped lookup, a transaction over a re-key -- and an object mapper both hides them and blocks the asynchronous path. A checked query is the abstraction that keeps them readable and still fails the build when a column moves | an object-relational mapper over the register |
| Cross-origin access is refused by default | a permissive origin is one line, ships silently, and is exactly the mistake nobody notices until it is exploited. A deployment that names no origin gets none, and the permissive fallback exists only in development mode and announces itself (`SRV-068`) | a permissive default, adjusted per deployment |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The sign-in tier of the rate limiter is never selected.** The limiter is a layer of the nested
  router, so the path it classifies has already lost the prefix its own rule matches on. Every
  sign-in request is therefore counted as an ordinary request -- a limit thirty times looser -- and
  fails open where the rule says it must fail closed. The tests assert the tier's answer and the
  classifier's answer separately, and neither one runs the path the assembled router produces
  (`SRV-013`, the `The production sign-in path` Story).
* **A delete removes rows and nothing else.** The register cascades, and the document's sequence
  counter, its per-author clock hash and its pending list stay in the ephemeral store with no
  expiry, while every sealed blob stays in object storage (`SRV-017`,
  `ambre-spec-object-store.md`, `SRV-057`).
* **One route reads the register before it knows who is asking.** The single-document route takes an
  optional identity, queries the row, and refuses an anonymous caller only afterwards. The anonymous
  branch can never succeed, so it is one query per probe and no capability (`SRV-018`).
* **The register declares four tables nothing serves.** Channels, their members, their read cursors
  and meetings exist in the durable schema, and no route and no handler writes any of them
  (`ambre-spec-broadcast.md`).
* **The declared schema and the migrated schema differ in three places.** The address is unique and
  case-insensitive, the master material is optional until an account completes its setup, and an
  address carries whether a provider verified it. All three are rules of
  `ambre-spec-session.md`, and none of the three was in the shape this document declared.
* **Nothing measures the relay.** Every row of `ambre-spec-budgets.md section 3` reads "none
  exists", including the two on the write path that the whole design is arranged around.
* **The request tests build a second route table.** The suite assembles its own router rather than
  the one the process serves, so a route registered wrongly in the real table is invisible to every
  test that passes -- which is how the sign-in defect above survived.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a caller proves who it is, and what ends a session | `ambre-spec-session.md` |
| Who assigns a sequence number, and what survives a crash | `ambre-spec-sequencer.md` |
| The live socket, what it fans out, and to whom | `ambre-spec-broadcast.md` |
| Where a sealed blob lives, and who may reach one | `ambre-spec-object-store.md` |
| The organization, its roles and the escrow | `ambre-spec-organizations.md` |
| What the relay may learn at all | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| What the client does with a request, a frame or a token | `ambre-spec-sync.md` |
| Which key opens a document, and how a member gets one | `ambre-spec-membership.md` |
| What a removal re-keys, and in what order | `ambre-spec-key-rotation.md` |
| What a home screen shows of a document list | `ambre-spec-launcher.md` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
