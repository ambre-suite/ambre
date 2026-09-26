# AMBRE -- FEATURE: THE BLIND RELAY

**Type:** Horizontal (framework)
**Implementation:** partial -- the register, the roles and the three stores all ship. Deleting a
document leaves its state behind, and the brute-force limiter is never selected.

> The one process every client talks to. It routes, it orders and it keeps, and it can read none of
> what it holds. A document is a row of metadata and a pile of sealed bytes.

This document owns the relay itself: what it is, what it stores and who may touch a document. It
also owns what a caller gets back when something goes wrong. What it does with a token, a delta, a
socket, a blob or an organization is the five documents beside it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-session.md`, `ambre-spec-sequencer.md`, `ambre-spec-broadcast.md`,
`ambre-spec-object-store.md` and `ambre-spec-organizations.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-012` | `INV` | The relay MUST perform no cryptographic operation beyond verifying a signature and minting its own tokens. | this spec section 5; `ZK-004` | cargo-deny -- `deny.toml` refuses the cipher, the exchange and the passphrase derivation in this process, and the continuous-integration `deny` job fails the manifest that adds one. `ed25519-dalek` is the one grant, which is exactly the two operations named | none | verified |
| `SRV-011` | `INV` | A document MUST NOT be left without an owner. | this spec section 3 | `ambre_server::routes::tests::handler_tests::owner_cannot_drop_last_owner` -- the refusal; `::owner_can_leave_when_another_owner_remains` -- the case that must still work, so neither end rots into the other | none | verified |
| `SRV-001` | `INV` | The relay MUST NOT store or log a plaintext document payload. | `ZK-001`, `ZK-006`; this spec section 3 | human-review -- the reviewer reads each diff on the flush and storage paths for a log line that carries the payload, against the explicit "never log the blob" rule the flush path already states | unheld -- nothing refuses the log line that would break it, so the rule leans on `ZK-006` and on attention | human-review |
| `SRV-014` | `INV` | Relay code MUST contain no unchecked memory access. | README pillar "Security" | none | unheld -- the workspace measures zero unchecked access today, and this crate carries no crate-level refusal of it: two crates of seventeen do (`ambre-spec-testing.md section 8`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-021` | `INV` | Any instance MUST be able to serve any client: no request state MUST live in the process. | this spec section 3 | none | unheld -- every store is external and the one process-wide object is the fan-out hub, which holds no request, and nothing boots two instances to observe it. The one exception is stated by `ambre-spec-session.md`, `SRV-024`; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-069` | `INV` | The page that loads the client MUST be served cross-origin isolated. | `ambre-spec-architecture.md section 2` | none | unheld -- the relay serves the requests and not the page, so the two headers are the deployment's obligation on whatever serves the page and the worker bootstrap, and nothing in continuous integration reads them back; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-103` | `INV` | Membership of a channel MUST grant access to each document the channel groups, and to no other document. | this spec section 3; `Todo/ambre-murmure.md` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-104` | `INV` | The register MUST keep the signed membership entries of a document beside its member rows. | this spec section 3; `SYNC-101` | not built | unbuilt -- lands with the `Signed membership log` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 A request is classified and limited

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-013` | `FAIL` | A failure of the rate limiter MUST close the sign-in tier, never open it. | audit `M6`; this spec section 3 | `ambre_server::middleware::rate_limit::tests::only_auth_tier_fails_closed` -- the sign-in tier refuses on an outage and the other two stay open, which is the trade the audit asked for | none | verified |
| `SRV-093` | `POST` | A sign-in request MUST be classified into the sign-in tier. | audit `M6`; this spec section 3 | none | fails -- `rate_limit::tests::classify_auth_path` feeds the classifier `/api/auth/login` and it answers correctly, but the limiter runs inside the nested router, which has already stripped `/api`. The classifier never sees a path it can match, so every sign-in is counted against the ordinary tier and `SRV-013` guards a tier nothing is ever in, section 6; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | failing |
| `SRV-019` | `POST` | The rate limiter MUST bucket a request by its authenticated caller. | audit `M6` | none | unheld -- no test drives a bucketed request at all: the three `client_ip_*` cases cover reading an address out of a header, not choosing which bucket a request lands in; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-094` | `POST` | A request with no authenticated caller MUST be bucketed by its socket peer. | audit `M6` | `ambre_server::middleware::rate_limit::tests::client_ip_from_forwarded_header`; `::client_ip_falls_back_to_remote`; `::client_ip_ignores_empty_forwarded` -- the address the fallback reads | partial -- a forwarded address is trusted only where the deployment declares a proxy, and no test drives that switch; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-095` | `FRAME` | The limiter MUST NOT put two callers in one bucket. | audit `M6`; this spec section 3 | none | unheld -- the bucket key is the tier prefix and the caller, so distinctness follows from the caller being distinct, and nothing asserts it. A forwarded header trusted where no proxy strips it puts every caller behind one address, and one client then exhausts the tier for all of them; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 A request is answered

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-100` | `FAIL` | A frame that does not decode MUST be refused as a typed error, never a panic, on either side of the socket. | this spec section 3; `ARCH-006` | `ambre_protocol::tests::a_frame_that_does_not_decode_is_a_typed_error`; cargo-fuzz -- `fuzz/fuzz_targets/wire_decode.rs`, run by the CI `fuzz` job | none | verified |
| `SRV-101` | `FAIL` | A panic while serving one request or one socket MUST end that request or that socket alone, and the relay MUST keep serving the others. | this spec section 3; `ARCH-028` | none | unheld -- no layer catches a panic in a handler, and nothing states what a poisoned lock means; lands with the `The relay survives a panic` Story (`ambre-roadmap.md` E7) | untested |
| `SRV-020` | `POST` | A failure of a backing service MUST reach the caller as a generic error. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- four store failures, one status and one message for all of them; `::oidc_error_is_infrastructure_level` -- the provider | none | verified |
| `SRV-096` | `FRAME` | An error answered to a caller MUST NOT carry the cause behind it. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- a store's own text is replaced rather than wrapped; `::application_errors_return_their_message` -- the other side, that a real application error keeps its message | partial -- both hold on the request path alone: the socket encodes its own error frames and one of them forwards a store's text verbatim (`ambre-spec-sequencer.md`, `SRV-081`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-068` | `FAIL` | A cross-origin request MUST be refused unless the deployment names its origin. | audit `M17`; this spec section 3 | `ambre_server::cors_tests::parses_and_trims_multiple_origins`; `::empty_list_yields_no_origins` -- the list the refusal is read off, the permissive fallback being reachable only in development mode and announced at start-up | none | verified |
| `SRV-005` | `FAIL` | A non-member MUST receive no metadata about a document -- not its owner, not its kind, not its existence. | audit `H2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::get_document_hidden_from_non_member` -- a not-found rather than a forbidden, which is what hides existence; `::create_document_with_foreign_existing_id_is_not_found` -- the same answer on a taken identifier, so a caller cannot probe the namespace | none | verified |
| `SRV-018` | `POST` | A route that reads a document row MUST require an authenticated caller. | this spec section 3 | none | fails -- the single-document route accepts an anonymous caller, queries the register, and only then answers not-found. The answer is right and the query runs first, so the register is read on behalf of nobody, section 6; lands with the `A document row is read for an authenticated caller` Story (`ambre-roadmap.md` E7) | failing |

### 1.4 A document is written, shared or deleted

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-007` | `FAIL` | Every update, snapshot and presence frame MUST carry a valid signature by the key the caller's token names; an unsigned or forged frame MUST have no effect. | this spec section 3; `ZK-005` | `ambre_server::ws::handler::message::ed25519_tests::invalid_signature_is_rejected`; `::tampered_ciphertext_rejects_signature`; `::unsigned_ephemeral_is_rejected` -- the refusals; `ephemeral_binding_tests::forged_author_key_writes_no_presence`; `routes::tests::handler_tests::create_snapshot_rejects_forged_author` -- the absence of effect on both paths | none | verified |
| `SRV-008` | `FAIL` | A read-only member MUST produce no write. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::viewer_role_delta_writes_no_pending` -- the live path leaves the queue empty; `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` -- the request path refuses, and a viewer holds the document key and its own signing key, so this check is the only thing between it and an accepted delta | none | verified |
| `SRV-015` | `FAIL` | Only an owner MUST be able to remove a member or re-key a document. | this spec section 3 | `ambre_server::routes::tests::handler_tests::rotate_key_denied_to_editor`; `::delete_document_only_by_owner` -- the refusals; `::revoke_then_rotate_replaces_owner_key` -- the owner's own path still completes | none | verified |
| `SRV-016` | `FAIL` | A caller MUST NOT grant a role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::share::tests::editor_cannot_grant_owner` -- the refusal; `::owner_can_grant_any_valid_role`; `::doc_role_rank_ordering` -- the ordering the clamp is read off | none | verified |
| `SRV-070` | `FAIL` | A caller MUST NOT change a member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::editor_cannot_demote_owner_via_invite` -- the invitation is also the role-change path, so this is the half a clamp on the granted role alone leaves open | none | verified |
| `SRV-017` | `FRAME` | Deleting a document MUST leave no relay state behind it. | this spec section 3 | none | fails -- the delete removes the rows and nothing else, so the document's sequence counter and its per-author clock hash stay in the ephemeral store with no expiry; lands with the `A deleted document leaves nothing` Story (`ambre-roadmap.md` E7) | failing |

### 1.5 A member joins a public channel alone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-105` | `FAIL` | Given a caller who is not a member of the space of the channel, a self-join MUST be refused. | this spec section 3; `Todo/ambre-murmure.md` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-106` | `FAIL` | Given a channel the relay does not hold as public, a self-join MUST be refused. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-107` | `POST` | An accepted self-join MUST store the entry the joining member signed, beside the new member row. | this spec section 3; `SYNC-106` | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-108` | `FRAME` | A self-join MUST NOT grant a role above editor. | this spec section 3 | not built | unbuilt -- lands with the `Spaces and the channel catalogue` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 A client asks for activity

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-109` | `POST` | The answer MUST say, per document, whether its last number is past the read cursor of the caller. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-110` | `FRAME` | The answer MUST NOT carry a count of frames. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 A space applies its retention

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-111` | `POST` | A final segment MUST be deleted, with its index and its amendments, once its finalization time is older than the retention of its space. | this spec section 3; `Todo/ambre-murmure.md` | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-112` | `FRAME` | Applying retention MUST NOT delete an active or a closing segment. | this spec section 3 | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the per-operation targets of `ambre-spec-budgets.md section 3`, and the roughly
  50 KB per open document of `ambre-spec-budgets.md section 6`. Every relay row of that register
  reads **none exists**: no bench measures the relay today.
* **Boundary.** this document is the relay side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SRV-001` keeps the payload out of the process and the logs, and `SRV-012` refuses the
  relay the ability to open one. `SRV-005` keeps the routing metadata behind the membership check,
  and `SRV-096` keeps a store's words out of an answer.
* **Verification.** 12 `verified`, 2 `partial`, 3 `failing`, 6 `untested`, 1 `human-review`. What
  holds is what a request test can drive: the roles, the rank clamps, the signature gate, the error
  mapping and the origin list. What is loose is every rule about the process or the deployment, and
  the three failures sit in the limiter and its bucketing.
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
| the membership log | one signed entry per invite, revoke and role change, served beside the member rows (`SRV-104`) |

**What a Murmure channel adds to the register.** Every document of a channel is an ordinary
document row. The register adds only what the relay needs to route, seal and expire them, which is
the set `ZK-015` names.

| Record | Holds | Read by |
|---|---|---|
| a channel | its space and its public flag | the self-join (`SRV-105`, `SRV-106`) |
| a channel document | the channel, the document and its kind | the access check (`SRV-103`) |
| a segment | its state and its finalization time | the finalization (`ambre-spec-sequencer.md`) and the retention (`SRV-111`) |
| a space | its retention duration | the retention (`SRV-111`) |
| a read cursor | a member, a document and a number | the activity answer (`SRV-109`) |

A space is a document like any other, so its members are its member rows. A direct message is a
channel with no space.

A document with no name stores nothing rather than a placeholder. The listing returns each sealed
name beside the caller's own sealed key. A client therefore reads its whole list in one answer,
with no document open (`ambre-spec-launcher.md`).

**The three roles**, ranked. A rank decides both what a caller may do and what it may grant.

| Role | May read | May write | May invite | May remove, re-key or delete |
|---|---|---|---|---|
| owner | yes | yes | up to owner | yes (`SRV-015`) |
| editor | yes | yes | up to editor (`SRV-016`) | no |
| viewer | yes | no (`SRV-008`) | no | no |

**Why the clamp is on the target too.** An invitation of an existing member is also the role-change
path. A clamp on the granted role alone lets an editor re-invite an owner as an editor, and
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

The two headers that make the client's shared memory legal are the opener policy and the embedder
policy. Both belong to whatever serves the page and the worker bootstrap, not to this process
(`SRV-069`, `ambre-spec-architecture.md section 2`). Their cost is that an embedded cross-origin
resource must carry a policy of its own. That is why a presigned link's target must allow a
cross-origin read.

**Where the process is stateless, and where it is not.** Every store is external, so a client
reaches any instance and the fan-out crosses them (`SRV-021`). Two things are per process, and
neither is per request. The first is the signing key derived from the deployment's seed, which
every instance must share; the second is the fan-out hub (`ambre-spec-broadcast.md`).

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

**What the surface does not carry.** Meetings have no route at all. The channel routes are
designed and unbuilt (`SRV-103` to `SRV-112`). They create a channel, join a public channel alone,
read the activity, and set the retention of a space.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The relay is blind by capability, not by discipline | trusting the code to avoid the operation | it is what the manifest allowed until three unused cipher crates were found declared in it. A rule that says "do not decrypt" is held by review; a process that cannot link a cipher cannot decrypt whatever anyone writes. The manifest is the check, and re-granting a capability is an edit a reviewer meets (`SRV-012`, `ZK-004`). |
| The one crypto library the relay links is the signature one | depending on `ambre-crypto` | it is the shape every other crate takes, and here it would put the document key and the master seed in reach of this process for one call it does not need. Verifying a signature is the only cryptographic act the relay has a reason to perform. |
| Every fact lives in one store | mirroring the sequence into the register | it would make a Redis reset invisible and a Redis write a two-phase commit. A fact in two stores is a fact that disagrees with itself after any partial failure: the register is the authority, the ephemeral store is a cache with an owner, and object storage holds only what neither can. |
| A non-member gets the answer of a document that does not exist | a distinct `403` | it is what the route did before audit `H2`: telling an arbitrary caller that a document exists, and who owns it, leaks the shape of everyone's workspace to anyone holding an identifier. One answer for both cases leaks nothing and costs the legitimate caller nothing (`SRV-005`). |
| A role is ranked, and the grant and the target are both clamped | a capability check per route | it is where the editor-demotes-owner path came from. Roles that are a set of strings make every new capability a new comparison, written once per route and forgotten once; a rank is one comparison, and clamping the target closes the re-invite path a grant clamp leaves open (`SRV-016`). |
| The wire format is binary on the socket and textual on the requests | one encoding for both | the socket carries a frame per keystroke burst and a compact encoding is what makes that affordable; the request surface is read by operators and tools, and a binary body would cost them a decoder for no measurable gain. |
| The relay terminates no transport of its own | terminating the modern transport in this process | a reverse proxy already terminates the modern transports, and putting a QUIC stack inside this process makes an operational concern a code concern. The relay speaks the simplest protocol to something that speaks every other. |
| A document's kind is named after the application | naming a kind after the generic object it resembles | the kinds are a closed set the register checks, and naming them after the generic object -- a spreadsheet, a document, a board -- puts a second vocabulary beside the product's own, so every reader has to hold a translation table. |
| The queries are written out, against a compile-time check | an object-relational mapper over the register | it hides the queries and blocks the asynchronous path. The queries here are the boundary's own enforcement -- a membership filter, a scoped lookup, a transaction over a re-key -- and a checked query is the abstraction that keeps them readable and still fails the build when a column moves. |
| Cross-origin access is refused by default | a permissive default, adjusted per deployment | a permissive origin is one line, ships silently, and is exactly the mistake nobody notices until it is exploited. A deployment that names no origin gets none, and the permissive fallback exists only in development mode and announces itself (`SRV-068`). |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The sign-in tier of the rate limiter is never selected.** The limiter is a layer of the nested
  router, so the path it classifies has already lost the prefix its own rule matches on. Every
  sign-in request is therefore counted as an ordinary request, a limit thirty times looser. It
  fails open where the rule says it must fail closed. The tests assert the tier's answer and the
  classifier's answer separately, and neither one runs the path the assembled router produces
  (`SRV-013`). The repair is the `The production sign-in path` Story (`ambre-roadmap.md` E7).
* **A delete removes rows and nothing else.** The register cascades, and the document's sequence
  counter, its per-author clock hash and its pending list stay in the ephemeral store with no
  expiry (`SRV-017`). Every sealed blob stays in object storage (`ambre-spec-object-store.md`,
  `SRV-057`).
* **One route reads the register before it knows who is asking.** The single-document route takes an
  optional identity, queries the row, and refuses an anonymous caller only afterwards. The anonymous
  branch can never succeed, so it is one query per probe and no capability (`SRV-018`).
* **The register declares four tables nothing serves.** Channels, their members, their read cursors
  and meetings exist in the durable schema, and no route and no handler writes any of them
  (`ambre-spec-broadcast.md`). The design replaces the channel member table: a channel grants
  access through its documents (`SRV-103`).
* **The declared schema and the migrated schema differ in three places.** The address is unique and
  case-insensitive, and the master material is optional until an account completes its setup. An
  address also carries whether a provider verified it. All three are rules of
  `ambre-spec-session.md`, and none of the three was in the shape this document declared.
* **Nothing measures the relay.** Every row of `ambre-spec-budgets.md section 3` reads "none
  exists", including the two on the write path that the whole design is arranged around.
* **The request tests build a second route table.** The suite assembles its own router rather than
  the one the process serves. A route registered wrongly in the real table is invisible to every
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
