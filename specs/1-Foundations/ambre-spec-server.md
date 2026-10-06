# AMBRE -- FEATURE: THE BLIND RELAY

**Type:** Horizontal (framework)
**Implementation:** partial -- the register, the roles, the three stores and the document delete all
ship. The brute-force limiter is never selected.

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
| `SRV-011` | `INV` | A document that has an owner MUST NOT be left without one. | this spec section 3 | `ambre_server::routes::tests::handler_tests::owner_cannot_drop_last_owner` -- the refusal; `::owner_can_leave_when_another_owner_remains` -- the case that must still work, so neither end rots into the other | none | verified |
| `SRV-170` | `INV` | A log macro, a span or an `instrument` field of the relay MUST NOT name a payload, a blob, a body or its bytes, unless it reads only the length. | `ZK-001`, `ZK-006`; this spec section 3 | ci-gate -- `scripts/crate_boundaries.py` refuses each of those names in a log macro, a span or an `instrument` field of `ambre-server`, a length aside | none | verified |
| `SRV-001` | `INV` | The relay MUST NOT store or log a plaintext document payload. | `ZK-001`, `ZK-006`; this spec section 3 | type-level -- every payload the relay stores, reads back or forwards is a `Payload`, with no `Debug` and no `Display`, so a log line naming one under any name does not compile; `ambre_protocol::messages::validate_tests::a_message_debug_form_prints_no_byte_of_its_ciphertext` -- a decoded message prints each ciphertext as its length; ci-gate -- `scripts/crate_boundaries.py` refuses a log line in a file that reads a payload's or a ciphertext's bytes | none | verified |
| `SRV-014` | `INV` | Relay code MUST contain no unchecked memory access. | README pillar "Security" | lint-wall -- `unsafe_code = "forbid"` in the root `[workspace.lints.rust]`, which every crate inherits (`ARCH-032`) | none | verified |
| `SRV-021` | `INV` | Any instance MUST be able to serve any client: no request state MUST live in the process. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_session_crosses_instances_and_a_sign_out_ends_it_on_both` -- two instances over one database and one Redis server: a session opened, renewed and ended on either is served by the other | none | verified |
| `SRV-171` | `INV` | The development server MUST serve the page that loads the client cross-origin isolated. | `ambre-spec-architecture.md section 2` | `ambre_server::isolation_tests::the_page_is_served_cross_origin_isolated` -- the development server's configuration sets both headers | none | verified |
| `SRV-069` | `INV` | The page that loads the client MUST be served cross-origin isolated in every deployment. | `ambre-spec-architecture.md section 2` | qa-manual -- `crossOriginIsolated` read in the console of each deployment's page | unheld -- `deployment` -- the relay serves the requests and not the page, and the development server's headers, which a test reads, say nothing of a deployment's | qa-manual |
| `SRV-103` | `INV` | Membership of a channel MUST grant access to each document the channel groups, and to no other document. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::db::channels::tests::channel_membership_reaches_every_document_it_groups`; `ambre_server::db::channels::tests::channel_membership_reaches_no_other_document`; `ambre_server::db::channels::tests::a_member_row_on_a_segment_grants_nothing` -- database tier | none | verified |
| `SRV-104` | `INV` | The register MUST keep the signed membership entries of a document beside its member rows. | this spec section 3; `SYNC-132` | `ambre_server::routes::tests::handler_tests::the_register_keeps_each_signed_entry_beside_the_rows` -- each change appends its entry, the members answer serves the log in order, and an entry signed by another key, stale or mismatched is refused | none | verified |

### 1.2 A request is classified and limited

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-013` | `FAIL` | A failure of the rate limiter MUST close the sign-in tier, never open it. | audit `M6`; this spec section 3 | `ambre_server::middleware::rate_limit::tests::only_auth_tier_fails_closed` -- the sign-in tier refuses on an outage and the other two stay open, which is the trade the audit asked for | none | verified |
| `SRV-093` | `POST` | A sign-in request MUST be classified into the sign-in tier. | audit `M6`; this spec section 3 | `ambre_server::routes::tests::handler_tests::a_sign_in_request_counts_against_the_sign_in_tier` -- the classifier reads the original path, so a sign-in through the shipped router lands in the sign-in tier | none | verified |
| `SRV-019` | `POST` | The rate limiter MUST bucket a request by its authenticated caller. | audit `M6` | `ambre_server::middleware::rate_limit::tests::an_authenticated_request_is_bucketed_by_its_caller` -- one caller from two peers lands in one bucket, and a token that does not verify is no caller | none | verified |
| `SRV-094` | `POST` | A request with no authenticated caller MUST be bucketed by its socket peer. | audit `M6` | `ambre_server::middleware::rate_limit::tests::client_ip_from_forwarded_header`; `::client_ip_falls_back_to_remote`; `::client_ip_ignores_empty_forwarded` -- the address the fallback reads; `::an_anonymous_request_is_bucketed_by_its_peer` -- a forwarded address is read only where the deployment declares a proxy | none | verified |
| `SRV-095` | `FRAME` | The limiter MUST NOT put two callers in one bucket. | audit `M6`; this spec section 3 | `ambre_server::middleware::rate_limit::tests::two_callers_never_share_a_bucket` -- two callers behind one peer, and two anonymous peers forging one forwarded address, each get their own bucket | none | verified |

### 1.3 A request is answered

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-100` | `FAIL` | A wire message that does not decode MUST be refused as a typed error, never a panic, on either side of the socket. | this spec section 3; `ARCH-006` | `ambre_protocol::tests::a_wire_message_that_does_not_decode_is_a_typed_error`; cargo-fuzz -- `fuzz/fuzz_targets/wire_decode.rs`, run by the CI `fuzz` job | none | verified |
| `SRV-101` | `FAIL` | A panic while serving one request or one socket MUST end that request or that socket alone, and the relay MUST keep serving the others. | this spec section 3; `ARCH-028` | `ambre_server::ws::pubsub_hub::tests::a_panic_in_the_hub_refuses_one_request_and_keeps_fanning_out` -- the hub panics on one socket's request, refuses that request alone, and fans out to the others after its restart; `ambre_server::routes::tests::handler_tests::a_panicking_socket_ends_alone_and_its_peer_hears_it_leave` -- the socket: it closes, its peer hears it leave, and the peer is still answered; `ambre_server::supervise::tests::a_panicking_handler_answers_a_generic_error_and_the_router_keeps_serving` -- the request; `::a_panicking_unit_of_work_leaves_the_next_one_running` -- the wrapper the flusher, the compaction and the socket loop each run under | none | verified |
| `SRV-020` | `POST` | A failure of a backing service MUST reach the caller as a generic error. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- four store failures, one status and one message for all of them; `::oidc_error_is_infrastructure_level` -- the provider | none | verified |
| `SRV-096` | `FRAME` | An error answered to a caller MUST NOT carry the cause behind it. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- a store's own text is replaced rather than wrapped; `::application_errors_return_their_message` -- the other side, that a real application error keeps its message; `ambre_server::ws::handler::message::refusal_tests::a_refused_subscription_carries_no_store_text` -- the socket path, where a store failure answers a generic 500 | none | verified |
| `SRV-068` | `FAIL` | A cross-origin request MUST be refused unless the deployment names its origin. | audit `M17`; this spec section 3 | `ambre_server::cors_tests::parses_and_trims_multiple_origins`; `::empty_list_yields_no_origins` -- the list the refusal is read off, the permissive fallback being reachable only in development mode and announced at start-up | none | verified |
| `SRV-005` | `FAIL` | A non-member MUST receive no metadata about a document -- not its owner, not its kind, not its existence. | audit `H2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::get_document_hidden_from_non_member` -- a not-found rather than a forbidden, which is what hides existence; `::create_document_with_foreign_existing_id_is_not_found` -- the same answer on a taken identifier, so a caller cannot probe the namespace; `::members_of_an_absent_or_foreign_document_are_not_found` -- the member list draws the same 404 for an invented document and a foreign one | none | verified |
| `SRV-018` | `POST` | A route that reads a document row MUST require an authenticated caller. | this spec section 3 | `ambre_server::routes::tests::handler_tests::get_document_refuses_an_anonymous_caller` -- the route takes the session extractor, so a caller without one is refused before any query | none | verified |

### 1.4 A document is written, shared or deleted

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-007` | `FAIL` | Every update, snapshot and presence sample MUST carry a valid signature by the key the caller's token names; an unsigned or forged one MUST have no effect. | this spec section 3; `ZK-005` | `ambre_server::ws::handler::message::ed25519_tests::invalid_signature_is_rejected`; `::tampered_ciphertext_rejects_signature`; `::unsigned_ephemeral_is_rejected` -- the refusals; `ephemeral_binding_tests::forged_author_key_writes_no_presence`; `routes::tests::handler_tests::create_snapshot_rejects_forged_author` -- the absence of effect on both paths | none | verified |
| `SRV-008` | `FAIL` | A read-only member MUST produce no write. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::viewer_role_delta_writes_no_pending` -- the live path leaves the queue empty; `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` -- the request path refuses, and a viewer holds the document key and its own signing key, so this check is the only thing between it and an accepted delta | none | verified |
| `SRV-015` | `FAIL` | Outside the exceptions of `SRV-144` to `SRV-146`, only an owner MUST be able to remove a member or re-key a document. | this spec section 3 | `ambre_server::routes::tests::handler_tests::rotate_key_denied_to_editor`; `::delete_document_only_by_owner` -- the refusals; `::revoke_then_rotate_replaces_owner_key` -- the owner's own path still completes | none | verified |
| `SRV-144` | `POST` | While a rotation is owed, the relay MUST accept it from any member with write access. | this spec section 3; `SYNC-128` | `ambre_server::routes::tests::handler_tests::an_owed_rotation_is_open_to_an_editor_and_clears_the_debt` -- an editor's rotation is refused while nothing is owed and accepted once a member left | none | verified |
| `SRV-145` | `POST` | A member MUST be able to remove themselves. | this spec section 3; `SYNC-127` | `ambre_server::routes::tests::handler_tests::a_member_leaves_alone_and_the_document_owes_a_rotation` -- an editor removes their own share, and the relay answers with success | none | verified |
| `SRV-146` | `POST` | An admin of a space MUST be able to re-key a public channel of that space. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-spaces.md` | not built | unbuilt -- lands with the `Leaving and deleting a space` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-157` | `FAIL` | Given a removal from a member whose standing to moderate the relay revoked, the relay MUST refuse it. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-moderation.md` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-159` | `POST` | A conversion of a direct message MUST record the space of the channel and mark it private. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-160` | `FAIL` | Given a wire message on a document of an archived channel, the relay MUST refuse it. | this spec section 3 | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-016` | `FAIL` | A caller MUST NOT grant a role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::share::tests::editor_cannot_grant_owner` -- the refusal; `::owner_can_grant_any_valid_role`; `::doc_role_rank_ordering` -- the ordering the clamp is read off | none | verified |
| `SRV-070` | `FAIL` | A caller MUST NOT change a member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::editor_cannot_demote_owner_via_invite` -- the invitation is also the role-change path, so this is the half a clamp on the granted role alone leaves open | none | verified |
| `SRV-017` | `FRAME` | Deleting a document MUST leave no relay state behind it. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_deleted_document_leaves_nothing` -- a delete purges the sequence, clock, pending, presence and dead-letter keys before the rows go, and the tombstone refuses every late write after | none | verified |
| `SRV-165` | `POST` | A delete that has begun MUST complete, even when a store fails or restarts before the end. | this spec section 3; `SRV-017`, `SRV-057` | `ambre_server::routes::tests::handler_tests::a_delete_interrupted_is_finished_by_the_pass` -- a delete the object store stops keeps its mark, and the pass empties the document, sets the lost tombstone again, then sweeps and drops the mark | none | verified |
| `SRV-137` | `POST` | A removal MUST delete the read cursor of the member removed. | this spec section 3; `ambre-spec-zero-knowledge.md` | `ambre_server::routes::read_cursors::tests::a_removal_deletes_the_read_cursors_of_the_member_removed` -- a removal from the `meta` deletes the cursors on every document of the channel, and another member's cursor stays | none | verified |
| `SRV-142` | `POST` | Deleting a channel MUST delete every document it groups. | this spec section 3; `SRV-017` | `ambre_server::routes::channels::tests::deleting_a_channel_deletes_its_documents_and_their_objects` -- database tier, with the objects swept from a fake store | none | verified |
| `SRV-174` | `POST` | The last member of a public channel MUST be able to leave it, owner or not, and the relay MUST keep the channel and its documents. | this spec section 3; `SRV-145` | not built | unbuilt -- lands with the `The last member of a public channel` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-175` | `POST` | The last owner of a public channel MUST be able to leave it while other members stay, and an owner of its space MUST then be able to delete it. | this spec section 3; `SRV-174`, `MURMURE-244` | not built | unbuilt -- lands with the `A public channel without an owner` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-143` | `POST` | Deleting a space MUST delete every channel of that space. | this spec section 3; `SRV-142` | not built | unbuilt -- lands with the `Leaving and deleting a space` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-176` | `POST` | A direct message MUST be created whole in one transaction, in no space, with an editor row for each member and no owner, and only from a log that says so. | this spec section 3; `SRV-104` | `ambre_server::routes::channels::tests::a_direct_message_has_no_space_and_no_owner` -- database tier, an editor row per member; `ambre_server::routes::channels::tests::each_member_holds_the_key_wrapped_for_them` -- each row holds its own wrap; `ambre_server::routes::channels::tests::a_direct_message_with_an_owner_is_refused` -- a log that keeps an owner writes nothing | none | verified |
| `SRV-177` | `FRAME` | A direct message request MUST NOT create a second direct message for a set of members that already holds one. | this spec section 3; `SRV-176` | `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` -- database tier, the existing one answers; `ambre_server::routes::channels::tests::racing_requests_create_one_direct_message` -- two racing requests create one | none | verified |
| `SRV-178` | `POST` | A change of the members of a document MUST reach every connection subscribed to it, naming the document alone. | this spec section 3; `MURMURE-440` | `ambre_server::routes::tests::handler_tests::a_change_of_members_reaches_the_subscribers_of_the_document` -- database tier, an invitation, a change of role and a removal each publish it on the topic of the document; `ambre_server::routes::channels::tests::a_self_join_tells_the_subscribers_of_meta` -- a self-join that adds a row publishes it, and one that adds none is silent | none | verified |

### 1.5 A member joins a public channel alone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-105` | `FAIL` | Given a caller who is not a member of the space of the channel, a self-join MUST be refused. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::routes::channels::tests::a_self_join_from_outside_the_space_is_refused` -- database tier | none | verified |
| `SRV-106` | `FAIL` | Given a channel the relay does not hold as public, a self-join MUST be refused. | this spec section 3 | `ambre_server::routes::channels::tests::a_self_join_of_a_channel_not_held_public_is_refused` -- database tier | none | verified |
| `SRV-107` | `POST` | An accepted self-join MUST store the entry the joining member signed, beside the new member row. | this spec section 3; `SYNC-106` | `ambre_server::routes::channels::tests::an_accepted_self_join_stores_the_entry_the_member_signed` -- the entry lands at the end of meta's log beside the row; `::a_self_join_of_a_logged_meta_without_an_entry_adds_no_member` -- a logged meta refuses a join with no entry | none | verified |
| `SRV-108` | `FRAME` | A self-join MUST NOT grant a role above editor. | this spec section 3 | `ambre_server::routes::channels::tests::a_self_join_grants_no_role_above_editor` -- database tier | none | verified |

### 1.6 A client asks for activity

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-109` | `POST` | The answer MUST say, per document, whether its last number is past the read cursor of the caller. | this spec section 3; `ZK-015` | `ambre_server::routes::read_cursors::tests::the_activity_answer_compares_each_last_number_with_the_callers_cursor` -- a document past the cursor, one at it, and a `meta` document that never appears | none | verified |
| `SRV-110` | `FRAME` | The answer MUST NOT carry a count of deltas. | this spec section 3; `ZK-015` | `ambre_server::routes::read_cursors::tests::the_activity_answer_compares_each_last_number_with_the_callers_cursor` -- every entry carries exactly a channel, a document, a yes or no, and the caller's own cursor, which counts nothing | none | verified |

### 1.7 A space applies its retention

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-111` | `POST` | A final segment MUST be deleted, with its index and its amendments, once its finalization time is older than the retention of its space. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-112` | `FRAME` | Applying retention MUST NOT delete an active or a closing segment. | this spec section 3 | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-156` | `POST` | An expired segment MUST take with it the thread documents of its messages. | this spec section 3; `SRV-111` | not built | unbuilt -- lands with the `Space retention` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.8 A document owes a rotation

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-122` | `POST` | A removal posted without a rotation MUST mark its document as owing one, unless that document is the `meta` of a public channel. | this spec section 3; `SYNC-127`, `MURMURE-061` | `ambre_server::routes::tests::handler_tests::a_member_leaves_alone_and_the_document_owes_a_rotation` -- it reads the debt in the register and in its mirror after the departure; `ambre_server::routes::channels::tests::a_leave_owes_a_rotation_on_a_private_channel_only` -- and no debt after a leave of a public channel | none | verified |
| `SRV-123` | `FAIL` | While a rotation is owed, the relay MUST refuse every wire message sealed under the key the removal left behind. | this spec section 3; `SYNC-125` | `ambre_server::redis_ops::lua_tests::a_frame_owed_a_rotation_takes_no_seq_and_no_clock` -- an update and a presence sample are refused while the mirror holds the debt, and accepted once it clears; `ambre_server::ws::handler::message::refusal_tests::a_frame_refused_for_its_key_names_the_key_and_the_clock` -- the author is told which key and which clock | none | verified |
| `SRV-124` | `POST` | A rotation that lands MUST clear the debt of its document. | this spec section 3; `SYNC-128` | `ambre_server::routes::tests::handler_tests::an_owed_rotation_is_open_to_an_editor_and_clears_the_debt` -- the register and the mirror read no debt after the rotation; `::a_revoke_removes_and_rotates_in_one_request` -- a revoke leaves none | none | verified |
| `SRV-125` | `FRAME` | Refusing a delta for its key MUST NOT give it a sequence number. | this spec section 3; `ambre-spec-sequencer.md` | `ambre_server::redis_ops::lua_tests::a_frame_owed_a_rotation_takes_no_seq_and_no_clock` -- the refused delta leaves no sequence key, and the same clock is accepted at sequence 1 once the debt clears | none | verified |
| `SRV-167` | `FAIL` | A rotation MUST be refused whole when the document holds a member it did not read. | this spec section 3; `SYNC-143` | `ambre_server::routes::tests::handler_tests::a_member_invited_during_a_revoke_gets_the_new_key` -- a revoke that read the members before an invitation landed is refused with `MEMBERS_CHANGED`, removes no one and stores no link, and the same revoke read again lands with a copy for every member left | none | verified |
| `SRV-168` | `FAIL` | An invitation MUST be refused when a rotation has replaced the key its copy wraps. | this spec section 3; `SYNC-143` | `ambre_server::routes::tests::handler_tests::a_member_invited_during_a_revoke_gets_the_new_key` -- an invitation naming the replaced key is refused and adds no one, and one naming the current key lands | none | verified |
| `SRV-136` | `POST` | A removal from a space MUST remove the member from every channel of that space, and mark each private one as owing a rotation. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-spaces.md` | not built | unbuilt -- lands with the `Leaving and deleting a space` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the per-operation targets of `ambre-spec-budgets.md section 3`, and the roughly
  50 KB per open document of `ambre-spec-budgets.md section 6`. Every relay row of that register
  reads **none exists**: no bench measures the relay today.
* **Boundary.** this document is the relay side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SRV-001` keeps the payload out of the process and the logs, and `SRV-012` refuses the
  relay the ability to open one. `SRV-005` keeps the routing metadata behind the membership check,
  and `SRV-096` keeps a store's words out of an answer.
* **Verification.** 46 `verified`, 1 `qa-manual`, 12 `unimplemented`. What holds is
  what a request test can drive: the roles, the rank clamps, the signature gate, the error mapping
  and the origin list. The delete holds too, and so does the payload type no log line can name.
  The process rules are loose; a deployment's isolation is read by hand.
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
| deletes the document | had it open | no row, and no ordering state (`SRV-017`) | both lists lose it, and the ephemeral store keeps nothing of it | `handler_tests::a_deleted_document_leaves_nothing` |

---

## 3. MODEL

**CRDT containers.** none. The relay compiles no CRDT at all (`ZK-004`): a document's content is
opaque bytes it moves and never opens.

**Three stores, and one fact in one of them.** Nothing is duplicated between them, which is what
makes a restart of any one of them survivable.

| Store | Holds | Lost on a reset |
|---|---|---|
| the durable register | users, organizations, documents, members, and the index of every snapshot and delta | everything: it is the authority |
| the ephemeral store | the sequence counters, the per-author clocks, the unflushed deltas, the refresh tokens, the presence blobs, the sign-in handshakes, the tombstones of deleted documents, and the mirror of each owed rotation | recoverable for the ordering (`ambre-spec-sequencer.md`) and the mirror, final for a session |
| object storage | every sealed blob: snapshots, deltas and media | the content itself, which nothing else holds |

**A delete fences the document before it empties it.** The relay first marks the delete in the
register. It then sets a tombstone in the ephemeral store. Every script that writes a key of the
document checks it and refuses. An update, a presence sample or a flush already past its checks then
writes nothing (`SRV-017`). The objects go next, then the keys, then the rows.

A presigned upload link cannot be revoked, so the mark schedules a second sweep of the objects for
when the last link has expired (`SRV-057`). The tombstone lives as long as that window.

**A delete that has begun completes.** The flusher's cycle ends with a pass over every mark. A
delete a failure stopped runs again from its first step, and each step may run twice. A tombstone a
reset lost is set again until the sweep is due. The sweep then runs, and the mark goes (`SRV-165`).

**The register.** One row per document, and none of it is content.

| Field | Form on the relay |
|---|---|
| the identity, the kind, the owner, the times | plaintext routing metadata |
| the name | a sealed blob, opened by the client (`ambre-spec-sync.md`, `SYNC-067`) |
| one member row per participant | a role, plus the document key sealed for that member |
| the wrapper of a member's key | the identity whose exchange key seals it, which the client needs to open it |
| the membership log | one signed entry per invite, revoke and role change, served beside the member rows (`SRV-104`) |
| a rotation owed | one flag, set by a departure and cleared by the rotation that follows (`SRV-122`, `SRV-124`) |
| a delete begun | one row, which outlives the document, holding the time of its second sweep (`SRV-165`) |

**A departure leaves a debt, and the relay holds it.** A member who removes themselves posts no key,
so the removal sets the flag in the same transaction (`SRV-145`, `SRV-122`). The ephemeral store
mirrors it, and the scripts that take a clock or a presence sample read the mirror. While it holds,
every key the document has is one the leaver held. So every frame is refused before its clock is
read, and takes no sequence number (`SRV-123`, `SRV-125`).

Any writer may then rotate (`SRV-144`). The rotation clears the flag in its own transaction, and
the mirror after it (`SRV-124`). A subscription rewrites the mirror from the register, so a reset
of the ephemeral store loses the debt until the next subscription only.

A revoke by an owner owes nothing. It posts the removal and every wrapped copy to one route, and the
relay applies both in one transaction (`ambre-spec-key-rotation.md`, `SYNC-126`).

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
channel with no space. It arrives whole in one request: both documents, an editor row for each
member, and the log of `meta` that grants exactly those rows (`SRV-176`). The client signs that log
and the relay checks it act by act against one sequence, `ownerless_log` in `ambre-protocol`, so
the two sides cannot drift apart. Its segment holds no row of its own, since every document of a
channel reads the rows of `meta`. A direct message has no owner, so `SRV-011` never applies to it.

Two direct messages never hold the same members (`SRV-177`). The request takes a transaction lock
keyed by the sorted set of members, then looks for a channel with no space whose rows on `meta`
are exactly that set. A racing request waits on the lock and finds the first one's channel. The
lock is advisory and lasts the transaction, so no table holds the set.

A change of the member rows of a document is said on the topic of that document (`SRV-178`). An
invitation says it, as do a change of role, a removal and a self-join that adds a row. The message
names the document alone, which the topic already names, so a subscriber learns no row from it. A
client that shows the members reads them again. A removal that deletes the channel says nothing,
since no member stays to hear it.

A public channel outlives its last member. The space reads its key in the catalogue, so the next
member joins it alone, and the last one leaves owner or not (`SRV-174`). A private channel goes
with its last member instead (`SRV-142`), each of its documents deleted as a deleted document is
(`SRV-165`). The last owner of a public channel leaves it too, while others stay: an owner of the
space document then stands in for it, as it does for the whole space (`SRV-175`). An editor of the space does not, so a catalogue writer deletes no channel.

A document with no name stores nothing rather than a placeholder. The listing returns each sealed
name beside the caller's own sealed key. A client therefore reads its whole list in one answer,
with no document open (`ambre-spec-launcher.md`).

**The three roles**, ranked. A rank decides both what a caller may do and what it may grant.

| Role | May read | May write | May invite | May remove, re-key or delete |
|---|---|---|---|---|
| owner | yes | yes | up to owner | yes (`SRV-015`) |
| editor | yes | yes | up to editor (`SRV-016`) | itself only; an owed rotation; a public channel of a space it administers (`SRV-144` to `SRV-146`) |
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
| a frame while a rotation is owed | on the socket, a refusal naming the document, the key and the clock (`SRV-123`) |
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

**A panic ends the work it hit, and nothing more** (`SRV-101`). A request already runs on its
connection's task, and a socket on its own. A layer answers a panicking request with the generic
failure of section 3, never with the panic's message (`SRV-096`). That layer is the `catch-panic`
feature of `tower-http`, a crate the relay already links, so nothing new is trusted. A socket that
panics still leaves the hub and announces its departure.

The hub, the flusher and the compaction serve every client. Each runs its work one unit at a time
and drops a unit that panics. The hub keeps its routing table across a restart and subscribes its
new stream to every topic in it. A socket whose topic the hub cannot route closes, and its client
reopens it.

**Code.**

* `crates/ambre-server/src/main.rs` -- the boot, the route table, the layers and the periodic tasks;
* `crates/ambre-server/src/supervise.rs` -- what keeps a panic inside the request, socket or task unit it hit;
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
| members | invite, remove, leave, revoke in one request, change a role, list, re-key | this document, and `ambre-spec-key-rotation.md` |
| content | the snapshot record, the delta page, and the presigned links | `ambre-spec-sequencer.md`, `ambre-spec-object-store.md` |
| media | the two presigned links and the confirmation | `ambre-spec-object-store.md` |
| organizations | the organization, its members and the escrow release | `ambre-spec-organizations.md` |
| channels | the documents a channel groups, listed to a member alone; a public or private channel registered in its space; a direct message, created or found; a self-join; the owner's delete | this document, `5-Murmure/features/ambre-spec-murmure-spaces.md` and `5-Murmure/features/ambre-spec-murmure-channels.md` |
| read cursors | a cursor, the activity answer, marking read | this document, and `ambre-spec-murmure-unread.md` |
| the socket | one upgrade | `ambre-spec-broadcast.md` |

**What the surface does not carry.** Meetings have no route at all. Of the channel routes, the
listing, the activity answer, the registration of a channel, the direct message, the signed
self-join and the delete are built. The retention of a space is designed and unbuilt (`SRV-111`
and `SRV-112`).

**The last number of a document.** The activity answer compares the caller's cursor with the
higher of two numbers. One is the sequence counter in the ephemeral store. The other is the
register's horizon: the highest flushed delta and the highest snapshot coverage. The counter alone
is lost with the ephemeral store; the register alone lags the unflushed deltas. Only segments and
threads appear: `meta` and amendments carry no message a member reads.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The relay is blind by capability, not by discipline | trusting the code to avoid the operation | it is what the manifest allowed until three unused cipher crates were found declared in it. A rule that says "do not decrypt" is held by review; a process that cannot link a cipher cannot decrypt whatever anyone writes. The manifest is the check, and re-granting a capability is an edit a reviewer meets (`SRV-012`, `ZK-004`). |
| The one crypto library the relay links is the signature one | depending on `ambre-crypto` | it is the shape every other crate takes, and here it would put the document key and the master seed in reach of this process for one call it does not need. Verifying a signature is the only cryptographic act the relay has a reason to perform. |
| Every fact lives in one store | mirroring the sequence into the register | it would make a Redis reset invisible and a Redis write a two-phase commit. A fact in two stores is a fact that disagrees with itself after any partial failure: the register is the authority, the ephemeral store is a cache with an owner, and object storage holds only what neither can. |
| A non-member gets the answer of a document that does not exist | a distinct `403` | it is what the route did before audit `H2`: telling an arbitrary caller that a document exists, and who owns it, leaks the shape of everyone's workspace to anyone holding an identifier. One answer for both cases leaks nothing and costs the legitimate caller nothing (`SRV-005`). |
| A role is ranked, and the grant and the target are both clamped | a capability check per route | it is where the editor-demotes-owner path came from. Roles that are a set of strings make every new capability a new comparison, written once per route and forgotten once; a rank is one comparison, and clamping the target closes the re-invite path a grant clamp leaves open (`SRV-016`). |
| The wire format is binary on the socket and textual on the requests | one encoding for both | the socket carries a wire message per keystroke burst and a compact encoding is what makes that affordable; the request surface is read by operators and tools, and a binary body would cost them a decoder for no measurable gain. |
| The relay terminates no transport of its own | terminating the modern transport in this process | a reverse proxy already terminates the modern transports, and putting a QUIC stack inside this process makes an operational concern a code concern. The relay speaks the simplest protocol to something that speaks every other. |
| A document's kind is named after the application | naming a kind after the generic object it resembles | the kinds are a closed set the register checks, and naming them after the generic object -- a spreadsheet, a document, a board -- puts a second vocabulary beside the product's own, so every reader has to hold a translation table. |
| The queries are written out, against a compile-time check | an object-relational mapper over the register | it hides the queries and blocks the asynchronous path. The queries here are the boundary's own enforcement -- a membership filter, a scoped lookup, a transaction over a re-key -- and a checked query is the abstraction that keeps them readable and still fails the build when a column moves. |
| Cross-origin access is refused by default | a permissive default, adjusted per deployment | a permissive origin is one line, ships silently, and is exactly the mistake nobody notices until it is exploited. A deployment that names no origin gets none, and the permissive fallback exists only in development mode and announces itself (`SRV-068`). |
| The relay refuses the key a removal left behind | refusing every write until someone rotates | a delta sealed under the new key would wait for nothing, and the document would freeze whenever no member is online |
| The relay marks a debt and a member rotates | the relay rotating the key itself | the relay holds no key, and a key it chose would break the boundary `ZK-001` draws |
| Any writer settles an owed rotation | the owner alone | the relay refuses every wire message under the old key until the rotation lands, so a channel whose owner is away would stop for everyone |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The register declares meeting tables nothing serves.** Meetings exist in the durable schema,
  and no route and no handler writes them (`ambre-spec-broadcast.md`).
* **The declared schema and the migrated schema differ in three places.** The address is unique and
  case-insensitive, and the master material is optional until an account completes its setup. An
  address also carries whether a provider verified it. All three are rules of
  `ambre-spec-session.md`, and none of the three was in the shape this document declared.
* **Nothing measures the relay.** Every row of `ambre-spec-budgets.md section 3` reads "none
  exists", including the two on the write path that the whole design is arranged around.
* **Most request tests build a second route table.** A few tests mount the table the process
  serves. The rest assemble their own router, so a route registered wrongly in the real table stays
  invisible to them.

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
| What the client does with a request, a wire message or a token | `ambre-spec-sync.md` |
| Which key opens a document, and how a member gets one | `ambre-spec-membership.md` |
| What a removal re-keys, and in what order | `ambre-spec-key-rotation.md` |
| What a home screen shows of a document list | `ambre-spec-launcher.md` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
