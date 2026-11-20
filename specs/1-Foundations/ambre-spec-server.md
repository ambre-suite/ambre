# AMBRE -- FEATURE: THE BLIND RELAY

**Type:** Horizontal (framework)

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
| `SRV-170` | `INV` | A log macro, a span or an `instrument` field of the relay MUST NOT name a payload, a blob, a body or its bytes, unless it reads only the length. | `ZK-001`, `ZK-006`; this spec section 3 | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the type of each value a log macro, a span or an `instrument` field of `ambre-server` formats or records. It refuses a byte buffer and a type named for a payload, a blob or a body, whatever the name of the value. A length is a `usize` and passes (`ambre_primitives::driver::wiring::a_payload_under_a_neutral_name_is_refused_in_a_log_line`) | none | verified |
| `SRV-298` | `INV` | An error of the object store that the relay logs MUST NOT name an object key or a document prefix in its text. | `ZK-030`; this spec section 3 | `ambre_server::hot_tail::db_tests::a_refused_batch_put_names_no_object_key` -- database tier: a batch put the store refuses fails with a text that names no document. The other sites of the object store that built such a text name no key or prefix either, which a reviewer reads | none | verified |
| `SRV-001` | `INV` | The relay MUST NOT store or log a plaintext document payload. | `ZK-001`, `ZK-006`; this spec section 3 | type-level -- every payload the relay stores, reads back or forwards is a `StoredPayload`, with no `Debug` and no `Display`, so a log line naming one under any name does not compile; `ambre_protocol::messages::validate_tests::a_message_debug_form_prints_no_byte_of_its_ciphertext` -- a decoded message prints each ciphertext as its length; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a log line in a file of the relay that reads a payload's bytes or takes a ciphertext whole, read with types (`ambre_primitives::driver::wiring::a_payload_under_a_neutral_name_is_refused_in_a_log_line`) | none | verified |
| `SRV-014` | `INV` | Relay code MUST contain no unchecked memory access. | README pillar "Security" | lint-wall -- `unsafe_code = "forbid"` in the root `[workspace.lints.rust]`, which every crate inherits (`ARCH-032`) | none | verified |
| `SRV-246` | `INV` | Relay code MUST NOT silence a lint without the reason it is kept, and a silence whose reason is gone MUST fail the build. | README pillar "Security"; `ARCH-028`; `ARCH-068` | lint-wall -- the workspace lint table denies the clippy lint `allow_attributes` and `unfulfilled_lint_expectations` (`ARCH-068`), and `ambre-server` inherits it (`ARCH-032`), so a kept site carries an `expect` attribute with its reason. `just lint` reads every target with every feature on, in the CI `lint` job | none | verified |
| `SRV-021` | `INV` | Any instance MUST be able to serve any client: no request state MUST live in the process. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_session_crosses_instances_and_a_sign_out_ends_it_on_both` -- two instances over one database and one Redis server: a session opened, renewed and ended on either is served by the other | none | verified |
| `SRV-241` | `INV` | The relay MUST NOT serve a request over a Redis server older than 7.4. | this spec section 3 | `ambre_server::redis_ops::tests::a_store_older_than_the_minimum_is_refused` -- an older version, a missing version line and an unparsed version are each refused, and 7.4 or later passes | none | verified |
| `SRV-252` | `INV` | A key the relay writes for one document, call or user MUST carry the identifier of that document, call or user as its hash tag. | this spec section 3 | `ambre_server::redis_keys::tests::every_key_of_a_document_or_a_call_sits_in_its_slot` -- each key of ten documents and of a call sits in the slot of its identifier, read by a slot function that `::the_slot_function_reads_the_slots_a_cluster_gives` holds against `CLUSTER KEYSLOT`; `ambre_server::middleware::rate_limit::tests::a_bucket_names_its_kind_in_the_key` -- the window of a user is tagged with the user | none | verified |
| `SRV-253` | `INV` | One script or one transaction of the ephemeral store MUST name keys of one hash slot alone. | this spec section 3; `SRV-252` | `ambre_server::redis_ops::redis_integration_tests::the_dirty_set_and_a_document_never_share_a_script` -- the append, the clear and the purge each run, and keep or drop the dirty mark. The CI `test-db-cluster` job runs it and the whole database tier against a cluster of three primaries, whose client refuses a script over two slots | none | verified |
| `SRV-254` | `INV` | The relay MUST publish a broadcast on a sharded channel, in the hash slot of its topic. | this spec section 3; `ambre-spec-broadcast.md` | `ambre_server::redis_ops::redis_integration_tests::each_broadcast_reaches_a_sharded_subscriber_of_its_topic` -- a sharded subscriber hears the broadcast of a document, a user, a channel and a call, which a classic publish never reaches. Database tier, on one server and on the cluster of the CI `test-db-cluster` job | none | verified |
| `SRV-255` | `INV` | A sweep of the whole keyspace MUST read every primary node of the ephemeral store. | this spec section 3; `ambre-spec-sequencer.md` | `ambre_server::s3::tests::the_sweep_finds_an_unmarked_list_on_every_primary` -- twenty unmarked pending lists are each marked by one sweep. The CI `test-db-cluster` job runs it over three primaries; `ambre_server::redis_ops::tests::the_primaries_are_the_masters_that_serve_a_slot` -- a replica, a failed primary and a primary with no slot are left out | none | verified |
| `SRV-257` | `INV` | The relay MUST expose its metrics in the Prometheus text format, on a port that the public listener does not serve. | this spec section 3; `ZK-029` | `ambre_server::metrics::tests::the_scrape_answers_the_text_format_under_closed_labels_alone` -- the metrics router answers `GET /metrics` in the text format, version 0.0.4; `ambre_server::metrics::tests::the_public_listener_serves_no_metric` -- database tier: the public router answers no metric, at the root or under `/api`. `main.rs` binds the two routers to two listeners, the metrics one on `METRICS_ADDR`, the loopback by default | none | verified |
| `SRV-171` | `INV` | The development server MUST serve the page that loads the client cross-origin isolated. | `ambre-spec-architecture.md section 2` | `ambre_server::isolation_tests::the_page_is_served_cross_origin_isolated` -- the development server's configuration sets both headers | none | verified |
| `SRV-069` | `INV` | The page that loads the client MUST be served cross-origin isolated in every deployment. | `ambre-spec-architecture.md section 2` | human-review -- `crossOriginIsolated` read in the console of each deployment's page | unheld -- `deployment` -- the relay serves the requests and not the page, and the development server's headers, which a test reads, say nothing of a deployment's | human-review |
| `SRV-260` | `INV` | The page that loads the client MUST admit as a script only the files of its own build, each inline script it ships by its digest, and the compile of a wasm module. | README pillar "Security"; this spec section 3 | `ambre_belt::csp::tests::the_page_runs_the_scripts_it_ships_and_no_other` -- browser tier, under Chrome and under Firefox: the violation listener runs before the first script of each document, through the DevTools Protocol under Chrome and WebDriver BiDi `script.addPreloadScript` under Firefox. The built application boots under the policy its build writes and opens a board, a spreadsheet and a text document with no violation, so no code of the client evaluates a string. A script injected into the page, an injected event handler and a `blob:` worker each run nowhere; ci-gate -- `scripts/test_trunk_post_build.py`, run by `just test-scripts` in the CI `lint` job: its `ContentPolicy` tests hold that the post-build hook names each inline script and style of the page by its digest, a script quoted in a comment by none, and writes no `unsafe-inline` and no `unsafe-eval` | none | verified |
| `SRV-281` | `INV` | The server of the page MUST answer every path with a content policy that refuses any frame. | README pillar "Security"; this spec section 3; `SRV-260` | ci-gate -- `scripts/test_trunk_post_build.py`, run by `just test-scripts` in the CI `lint` job: its `Headers` tests hold that the post-build hook writes `_headers` beside the page, with `frame-ancestors 'none'` on every path; `ambre_belt::serve::tests::every_answer_refuses_a_frame_and_a_worker_script_carries_its_policy` -- the server of the browser tier applies that file as a deployment's server does, and the page, a deep link and any file each refuse a frame; `ambre_server::isolation_tests::the_development_server_refuses_a_frame` -- `Trunk.toml` sets the same policy on every answer of the development server | none | verified |
| `SRV-282` | `INV` | The server of the page MUST serve the script of each worker under a policy that admits only the files of the build and a wasm compile. | README pillar "Security"; this spec section 3; `SRV-260` | ci-gate -- `scripts/test_trunk_post_build.py`, run by `just test-scripts` in the CI `lint` job: its `Headers` tests hold that `_headers` names the loader of the Sync worker, the loader of the call worker and the thread script each under the worker policy, with no `unsafe-eval`, no `unsafe-inline` and no `blob:`, and that a build missing one of the three is refused; `ambre_belt::serve::tests::every_answer_refuses_a_frame_and_a_worker_script_carries_its_policy` -- the server of the browser tier answers each worker script under that policy, and no other file | none | verified |
| `SRV-312` | `INV` | The page MUST admit in a frame only the sites its deployment names, and no frame when the deployment names none. | README pillar "Security"; this spec section 3; `SRV-260`; `TOILE-088`; `ZK-048` | ci-gate -- `scripts/test_trunk_post_build.py`, run by `just test-scripts` in the CI `lint` job: its `EmbedSites` tests hold that the post-build hook writes each site of `AMBRE_EMBED_SITES` into `frame-src` as an `https` origin, writes `frame-src 'none'` when the variable names no site, and fails the build on a value that is not a host or an `https` origin, a wildcard, a keyword, another scheme or a path included | none | verified |
| `SRV-261` | `INV` | Every answer of the relay MUST carry a content policy that loads nothing and lets no page frame it. | this spec section 3 | `ambre_server::answer_policy_tests::every_answer_carries_the_policy_that_loads_nothing` -- a success, the generic `500` of a panic and an unknown route each carry it through the layers the relay serves | none | verified |
| `SRV-270` | `INV` | Every answer of the relay MUST carry a strict transport policy of two years that covers the subdomains. | this spec section 3 | `ambre_server::answer_policy_tests::every_route_answers_with_the_strict_transport_policy` -- each route of the public listener with each of its methods, read from the router source, an unknown route, a preflight and the scrape of the metrics listener; `ambre_server::answer_policy_tests::every_answer_carries_the_policy_that_loads_nothing` -- the generic `500` of a panic | none | verified |
| `SRV-103` | `INV` | Membership of a channel MUST grant access to each document the channel groups, and to no other document. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::db::channels::tests::channel_membership_reaches_every_document_it_groups`; `ambre_server::db::channels::tests::channel_membership_reaches_no_other_document`; `ambre_server::db::channels::tests::a_member_row_on_a_segment_grants_nothing` -- database tier | none | verified |
| `SRV-104` | `INV` | The register MUST keep the signed membership entries of a document beside its member rows. | this spec section 3; `SYNC-132` | `ambre_server::routes::tests::handler_tests::the_register_keeps_each_signed_entry_beside_the_rows` -- each change appends its entry, the members answer serves the log in order, and an entry signed by another key, stale or mismatched is refused | none | verified |
| `SRV-272` | `INV` | The register MUST serve a document of a channel the membership log of the channel's `meta`, as it serves the meta's rows. | this spec section 3; `SRV-104`; `ambre-spec-membership.md`, `SYNC-229` | `ambre_server::routes::channels::tests::a_segment_serves_the_log_of_its_meta` -- a segment serves the entries of its `meta` and none of its own, and a document outside a channel serves its own, database tier | none | verified |
| `SRV-248` | `INV` | The relay MUST keep the state of a call in the ephemeral store, under the time to live of `ambre-spec-budgets.md section 1`, which the heartbeat of each open socket of the call refreshes. | this spec section 3; `ambre-spec-call-media.md section 3`, signalling | `ambre_server::ws::handler::call::tests::the_state_of_a_call_holds_only_what_the_relay_may_learn` -- database tier: each key of a call, and the field of each seat, lives under the time to live; `ambre_server::ws::handler::call::tests::nothing_of_a_call_outlives_its_time_to_live_without_a_heartbeat` -- a seat no heartbeat holds leaves no key of its call past it; `ambre_server::ws::handler::call::tests::each_join_of_a_call_gets_a_sender_index_no_earlier_join_held` -- one script takes the seat and the index | none | verified |
| `SRV-306` | `INV` | One signing key MUST hold a bounded count of the seats of a call at once, and take a bounded count of its sender indexes. | this spec section 3; `SRV-248` | `ambre_server::ws::handler::call::tests::one_member_holds_a_bounded_share_of_a_call` -- database tier: past its seats a key reads the call as full, past its indexes as exhausted, and another member is still admitted; `ambre_server::ws::handler::call::tests::the_state_of_a_call_holds_only_what_the_relay_may_learn` -- the stored state counts the sender indexes each key took | none | verified |
| `SRV-249` | `INV` | A socket of a call MUST close with a notice at its first heartbeat, on the period of `ambre-spec-budgets.md section 1`, that finds the state of its call gone from the ephemeral store. | this spec section 3; `SRV-248` | `ambre_server::ws::handler::call::tests::a_call_whose_state_is_lost_ends` -- database tier: a heartbeat keeps the state past its time to live, then the socket of a lost state gets the notice and closes, and a rejoin starts a fresh call under a new room | none | verified |
| `SRV-250` | `INV` | The durable register MUST NOT hold a call, its roster or a signed join. | this spec section 3; `ZK-022` | `ambre_server::ws::handler::call::tests::no_table_of_the_relay_schema_holds_a_call` -- database tier: no table or column of the schema names a call, a sender or a room | none | verified |
| `SRV-251` | `INV` | A seat whose socket stops its heartbeat MUST leave the roster and the cap of its call within the time to live of `ambre-spec-budgets.md section 1`. | this spec section 3; `SRV-248`; `CALL-021` | `ambre_server::ws::handler::call::tests::a_seat_that_stops_its_heartbeat_leaves_the_roster_and_frees_its_place` -- database tier: a seat no socket beats for leaves the roster and frees its place while a beating seat holds the call, and its index is never given again | none | verified |
| `SRV-308` | `INV` | A release MUST publish the relay, the SFU and the client as container images, each named by its digest. | this spec section 3 | not built | unbuilt -- lands with the `The relay ships as images and a chart any host runs` Story (`ambre-roadmap.md` E6). No image is built: the repository holds a development compose file alone | unimplemented |
| `SRV-309` | `INV` | The chart of the repository MUST deploy a stack against which the browser tier passes on a local Kubernetes cluster. | this spec section 3; `SRV-308` | not built | unbuilt -- lands with the `The relay ships as images and a chart any host runs` Story (`ambre-roadmap.md` E6). No chart exists | unimplemented |
| `SRV-310` | `INV` | The self-hosting compose file of the repository MUST deploy, on one host, a stack against which the browser tier passes. | this spec section 3; `SRV-308` | not built | unbuilt -- lands with the `One server runs Ambre from one compose file` Story (`ambre-roadmap.md` E6). The compose file of the repository starts the dependencies of development alone: no relay, no SFU, no TURN and no client | unimplemented |

### 1.2 A request is classified and limited

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-013` | `FAIL` | A failure of the rate limiter MUST close the sign-in tier, never open it. | audit `M6`; this spec section 3 | `ambre_server::middleware::rate_limit::tests::only_auth_tier_fails_closed` -- the sign-in tier refuses on an outage and the other two stay open, which is the trade the audit asked for | none | verified |
| `SRV-093` | `POST` | A sign-in request MUST be classified into the sign-in tier. | audit `M6`; this spec section 3 | `ambre_server::routes::tests::handler_tests::a_sign_in_request_counts_against_the_sign_in_tier` -- the classifier reads the original path, so a sign-in through the shipped router lands in the sign-in tier | none | verified |
| `SRV-019` | `POST` | Outside the sign-in tier, the rate limiter MUST bucket a request by its authenticated caller. | audit `M6` | `ambre_server::middleware::rate_limit::tests::an_authenticated_request_is_bucketed_by_its_caller` -- one caller from two peers lands in one bucket, and a token that does not verify is no caller | none | verified |
| `SRV-094` | `POST` | A request with no authenticated caller MUST be bucketed by its socket peer. An IPv4-mapped IPv6 peer MUST share the bucket of its IPv4 address. | audit `M6` | `ambre_server::middleware::rate_limit::tests::client_ip_from_forwarded_header`; `::client_ip_falls_back_to_remote`; `::client_ip_ignores_empty_forwarded` -- the address the fallback reads; `::a_forwarded_entry_that_is_no_address_is_unknown`; `::the_store_key_of_a_real_address_is_unchanged` -- the bucket holds a parsed address, an address in its canonical spelling keeps its store key, and any other spelling moves onto the canonical key on purpose; `::a_forwarded_entry_with_a_port_counts_as_its_address` -- an IPv4 or a bracketed IPv6 entry with a port counts as its address; `::an_ipv4_mapped_address_shares_the_ipv4_window` -- an IPv4-mapped IPv6 address, forwarded or from the socket, shares the window of its IPv4 address, so one client cannot hold two; `::an_anonymous_request_is_bucketed_by_its_peer` -- a forwarded address is read only where the deployment declares a proxy | none | verified |
| `SRV-095` | `FRAME` | The limiter MUST NOT put two callers in one bucket. | audit `M6`; this spec section 3 | `ambre_server::middleware::rate_limit::tests::two_callers_never_share_a_bucket` -- two callers behind one peer, and two anonymous peers forging one forwarded address, each get their own bucket | none | verified |
| `SRV-304` | `POST` | A request of the sign-in tier MUST be bucketed by its peer, whatever token it carries. | this spec section 3; `SRV-093` | `ambre_server::middleware::rate_limit::tests::the_sign_in_tier_counts_by_peer_whatever_the_token` -- a request with a valid token counts by its peer in the sign-in tier, and by its caller in the two others | none | verified |
| `SRV-285` | `POST` | Behind a declared chain of proxies, the limiter MUST bucket an anonymous request by the forwarded entry that the first proxy of the chain wrote. | this spec section 3; `SRV-094` | `ambre_server::middleware::rate_limit::tests::the_client_is_read_from_the_right_of_the_forwarded_chain` -- behind one or two proxies, the entries a caller writes on the left change no bucket, and a header shorter than the chain is the unknown bucket; `::the_proxy_setting_names_the_length_of_the_chain` -- the setting names the number of proxies, and any other value declares none | none | verified |
| `SRV-286` | `FRAME` | A request the limiter refuses MUST add nothing to its bucket. | this spec section 3; `SRV-013` | `ambre_server::middleware::rate_limit::tests::a_refused_request_adds_nothing_to_its_bucket` -- database tier: past a limit of three, five more requests are refused and the window still holds three | none | verified |
| `SRV-287` | `POST` | A request for the provider configuration MUST be counted in the ordinary tier, never in the sign-in tier. | this spec section 3; `SRV-093` | `ambre_server::middleware::rate_limit::tests::the_provider_configuration_counts_in_the_api_tier` -- the classifier; `ambre_server::routes::tests::handler_tests::a_sign_in_request_counts_against_the_sign_in_tier` -- database tier: a peer that spent its sign-in tier still reads the configuration eleven times | none | verified |

### 1.3 A request is answered

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-100` | `FAIL` | A wire message that does not decode MUST be refused as a typed error, never a panic, on either side of the socket. | this spec section 3; `ARCH-006` | `ambre_protocol::tests::a_wire_message_that_does_not_decode_is_a_typed_error`; cargo-fuzz -- `fuzz/fuzz_targets/wire_decode.rs`, run by the CI `fuzz` job | none | verified |
| `SRV-178` | `FAIL` | Given a request body that does not decode, an unknown word of a closed set included, the relay MUST answer `409`. | this spec section 3 | `ambre_server::extract::tests::a_body_that_does_not_decode_answers_409` -- an unknown word, a body cut short and a field missing each answer `409`; `ambre_server::routes::tests::handler_tests::invalid_doc_type_is_rejected`; `::update_member_role_changes_role`; `ambre_server::routes::orgs::tests::integration::update_member_role_rejects_invalid_role` -- an unknown document type and an unknown role answer `409` on their routes | none | verified |
| `SRV-179` | `FAIL` | Given an identifier on the JSON wire of the relay or of Sync that does not parse, the body MUST be refused when it is read, each identifier travelling as its text under its own type. | this spec section 3; `SRV-178`; `BUS-029` | `ambre_core::serde_id::tests::an_identifier_reaches_json_as_its_text` -- the hyphenated form, the hex key and the round trip; `::a_malformed_identifier_is_refused` -- a bare, an optional, a listed and a key identifier each fail the read; `::an_absent_identifier_reads_null`; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses an identifier field held as a `String` or a `Uuid` in a row or route of the relay or in the HTTP layer of Sync, and a serde field holding an identifier of the core family without its text adapter, each type resolved (`ambre_primitives::driver::rules::a_wire_field_under_an_alias_and_a_renamed_json_are_refused`) | none | verified |
| `SRV-180` | `FAIL` | Given a word of a closed set on the JSON wire of the relay or of Sync that names no member of it, the body MUST be refused when it is read, each set parsed by one strict parser into its enum. | this spec section 3; `SRV-178`; `SRV-179` | `ambre_core::doc_type::tests::each_type_reads_back_from_its_word_only` -- the document type reads its lowercase word and refuses its display name; `ambre_core::role::tests::test_from_str_is_case_sensitive`; `ambre_core::channel_kind::tests::each_kind_reads_back_from_its_stored_word`; `ambre_core::status::tests::each_snapshot_status_reads_back_from_its_word_only`; `::the_rotation_status_reads_its_one_word`; `ambre_sync::sharing::tests::map_members_parses_roles_and_refuses_malformed_ids` -- an unknown role and a capitalised one refuse the members answer; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a `doc_type`, `role`, `kind` or `status` field held as a `String` or a `str` in a row or route of the relay or in the HTTP layer of Sync, each alias resolved (`ambre_primitives::driver::rules::a_wire_field_under_an_alias_and_a_renamed_json_are_refused`) | none | verified |
| `SRV-226` | `FAIL` | Given a sealed value on the JSON wire of the relay or of Sync that is not base64, or whose layout fixes a length it does not have, the body MUST be refused when it is read. | this spec section 3; `SRV-178`; `SRV-179` | `ambre_core::encoded::tests::a_fixed_role_refuses_any_other_length` -- a wrapped document key and a wrapped master key each read their own lengths alone, the empty value refused; `::a_value_that_is_not_base64_refuses_the_read`; `ambre_crypto::tests::a_wrap_has_a_length_the_wire_reads` -- each length the wire reads is the length a wrap writes | none | verified |
| `SRV-242` | `FAIL` | Given a fixed-width value on the JSON wire of the relay or of Sync -- a public key, a signature, a key identifier or a salt -- that is not the hexadecimal of its own width, the body MUST be refused when it is read. | this spec section 3; `SRV-178`; `SRV-226` | `ambre_core::fixed_bytes::tests::a_readable_format_spells_hexadecimal_and_refuses_another_width` -- a text one byte short, one byte past, not hexadecimal, empty, or a list of integers is refused; `ambre_core::encoded::tests::a_master_key_salt_of_another_width_is_refused_when_built` -- a short salt fails the read; `ambre_server::routes::tests::handler_tests::a_public_key_of_the_wrong_length_is_refused_and_written_nowhere` -- a short key answers `409` on each route that takes one, database tier; type-level -- every fixed-width value is a `FixedBytes<R>`, whose one hand-written serde reads that form | none | verified |
| `SRV-227` | `FAIL` | Given a membership entry whose bytes are not its own encoding, a byte past its end included, the relay MUST refuse it and store nothing. | this spec section 3; `SRV-104` | `ambre_protocol::membership::tests::an_entry_reads_from_its_own_encoding_alone` -- a trailing byte and a position in a longer form each refuse the read; `ambre_server::routes::tests::handler_tests::the_register_keeps_each_signed_entry_beside_the_rows` -- a signed entry with one byte appended is refused and the log keeps its length, database tier | none | verified |
| `SRV-311` | `FAIL` | Given a membership entry whose exchange key is not the one its member registered, the relay MUST refuse it and store nothing. | this spec section 3; `SRV-104`; `ambre-spec-membership.md`, `SYNC-197`; `ambre-spec-key-rotation.md`, `SYNC-198` | `ambre_server::routes::tests::handler_tests::an_entry_naming_another_exchange_key_than_the_registered_one_is_refused` -- a creation, an invite, a role change and a revoke each naming another exchange key are refused and the log keeps nothing of them, while the creation, the invite and the role change under the registered key land, database tier | none | verified |
| `SRV-228` | `FAIL` | Given a snapshot confirmation that carries a parent proof, the relay MUST refuse it and store nothing, as no client builds one. | this spec section 3; `ZK-001` | `ambre_server::routes::tests::handler_tests::a_snapshot_carrying_a_parent_proof_is_refused` -- a proof answers `400` and writes no row, and a `null` one confirms, database tier | none | verified |
| `SRV-243` | `POST` | A listing of a user's documents MUST send the update time of each document as a whole number of Unix milliseconds, read on the relay's clock. | this spec section 3; `ambre-spec-sync.md`, `SYNC-181` | `ambre_server::routes::tests::handler_tests::a_listed_document_sends_its_update_time_as_relay_milliseconds` -- a pinned row time reaches the JSON as the same number, database tier; `ambre_server::db::documents::relayed_at_tests::a_relay_time_converts_to_its_unix_milliseconds` -- the one conversion out of the store; type-level -- the listing holds an `Instant<Relayed>`, whose serde form is the bare number | none | verified |
| `SRV-244` | `FAIL` | Given a document whose update time falls before the Unix epoch, the listing MUST answer an internal error rather than a time. | this spec section 3; `SRV-243`, `SRV-096` | `ambre_server::db::documents::relayed_at_tests::a_relay_time_before_the_epoch_is_refused` -- the conversion refuses the time as an internal error, which the relay answers as `500` and logs | none | verified |
| `SRV-245` | `FRAME` | The response for a single document MUST carry no time of the document. | this spec section 3; `SRV-243` | `ambre_server::routes::tests::handler_tests::create_and_get_document` -- the answer holds neither the creation time nor the update time, database tier | none | verified |
| `SRV-247` | `POST` | A stored snapshot MUST take the id the relay minted for its upload link, and the confirmation of an upload MUST name none. | this spec section 3; `ambre-spec-sync.md`, `SYNC-182`; `ambre-spec-sealed-envelope.md`, `SYNC-258` | `ambre_server::routes::tests::handler_tests::a_snapshot_is_served_once_confirmed` -- an upload that names no id is announced, confirmed and served under the id the relay answers, database tier; `ambre_server::routes::tests::handler_tests::a_snapshot_takes_the_id_its_upload_link_minted` -- the row takes the id the link named in its object key, a second confirmation of that object is refused and another spelling of the id stores nothing, database tier; type-level -- the confirmation body has no id field, and the one insert of a snapshot takes the id it reads back from the key the link built | none | verified |
| `SRV-230` | `FAIL` | Given a sealed name past the longest name its budget allows, the body MUST be refused when it is read. | this spec section 3; `ambre-spec-budgets.md section 6`; `SRV-226` | `ambre_core::encoded::tests::a_sealed_name_past_the_longest_name_is_refused` -- the longest name in four-byte characters reads, and one byte past it is refused | none | verified |
| `SRV-238` | `FAIL` | Given a stored member count that does not fit the 32 bits of the wire, the relay MUST refuse the row when it is read, never truncate it. | this spec section 3; `SRV-180` | `ambre_server::db::tests::a_member_count_past_u32_is_refused_not_truncated` -- the widest count reads, and one past it or below zero is a decode error; type-level -- the row and the answer hold a `Count<Members>`, which only the checked conversion builds from the store's count | none | verified |
| `SRV-239` | `FAIL` | Given bytes of any width but its own, a master-key salt MUST be refused when it is built, so no short salt reaches the passphrase derivation at unlock. | this spec section 3; `SRV-226` | `ambre_core::encoded::tests::a_master_key_salt_of_another_width_is_refused_when_built` -- a salt one width short, one past and empty are refused, its own width is built and reads back from its JSON text | none | verified |
| `SRV-181` | `FRAME` | Answering a request MUST NOT send a body built ad hoc: each body is a declared type, a refusal and a write's outcome included. | this spec section 3; `SRV-179`; `SRV-180` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a body the `json!` macro of `serde_json` builds above the tests of a route, of the refusal or of a middleware of the relay, whatever name calls it (`ambre_primitives::driver::rules::a_wire_field_under_an_alias_and_a_renamed_json_are_refused`); `ambre_core::status::tests::each_write_status_keeps_its_word` -- each outcome keeps the word the relay answered before; `ambre_sync::http::tests::a_write_answer_passes_the_outcome_asked_alone` -- Sync reads the typed body and its outcome | none | verified |
| `SRV-182` | `FRAME` | Refusing a request MUST name its code from one closed set that the relay and Sync share, and Sync MUST read a code outside it as the bare status. | this spec section 3; `SRV-180`; `SRV-181` | type-level -- one list declares each code and its word, and `as_str`, the parse and serde are generated from it; `ambre_core::status::tests::every_refusal_code_reads_and_writes_as_its_word_alone` -- the wire and the parse spell each code as `as_str`, and an unknown word is serde's unknown variant; `ambre_core::status::tests::each_refusal_code_keeps_its_word` -- each code keeps the word the relay answered before; `ambre_sync::http::tests::a_refusal_code_outside_the_set_reads_as_its_status` -- an unknown code and a lowercase one read as the bare 409; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses an `error` field held as text in the relay's refusal or in the HTTP layer of Sync, each alias resolved (`ambre_primitives::driver::rules::a_wire_field_under_an_alias_and_a_renamed_json_are_refused`) | none | verified |
| `SRV-101` | `FAIL` | A panic while serving one request or one socket MUST end that request or that socket alone, and the relay MUST keep serving the others. | this spec section 3; `ARCH-028` | `ambre_server::ws::pubsub_hub::tests::a_panic_in_the_hub_refuses_one_request_and_keeps_fanning_out` -- the hub panics on one socket's request, refuses that request alone, and fans out to the others after its restart; `ambre_server::routes::tests::handler_tests::a_panicking_socket_ends_alone_and_its_peer_hears_it_leave` -- the socket: it closes, its peer hears it leave, and the peer is still answered; `ambre_server::supervise::tests::a_panicking_handler_answers_a_generic_error_and_the_router_keeps_serving` -- the request; `::a_panicking_unit_of_work_leaves_the_next_one_running` -- the wrapper the flusher, the compaction and the socket loop each run under | none | verified |
| `SRV-297` | `FRAME` | A panic of the relay MUST log where it happened, and MUST NOT log its message. | `ZK-006`; this spec section 3; `SRV-101` | type-level -- the hook `main.rs` installs hands the logging function the location of the panic alone, never its payload; `ambre_server::supervise::tests::a_panic_logs_its_place_alone` -- the line names the file and the line | none | verified |
| `SRV-020` | `POST` | A failure of a backing service MUST reach the caller as a generic error. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- four store failures, one status and one message for all of them; `::oidc_error_is_infrastructure_level` -- the provider | none | verified |
| `SRV-096` | `FRAME` | An error answered to a caller MUST NOT carry the cause behind it. | `ZK-006`; this spec section 3 | `ambre_server::error::tests::infrastructure_errors_return_internal_server_error_message` -- a store's own text is replaced rather than wrapped; `::application_errors_return_their_message` -- the other side, that a real application error keeps its message; `ambre_server::ws::handler::message::refusal_tests::a_refused_subscription_carries_no_store_text` -- the socket path, where a store failure answers a generic 500 | none | verified |
| `SRV-068` | `FAIL` | A cross-origin request MUST be refused unless the deployment names its origin. | audit `M17`; this spec section 3 | `ambre_server::cors_tests::parses_and_trims_multiple_origins`; `::empty_list_yields_no_origins` -- the list the refusal is read off, the permissive fallback being reachable only in development mode and announced at start-up | none | verified |
| `SRV-256` | `FAIL` | Outside development mode, the relay MUST refuse to start when the deployment names no database address or no object store key, and MUST name the missing variable. | this spec section 3; `SRV-068` | `ambre_server::startup::the_relay_refuses_to_start_without_its_database_address`; `::the_relay_refuses_to_start_without_its_object_store_keys` -- the binary, started with each variable unset, exits before any connection and names the variable; `::development_mode_falls_back_to_the_keys_of_the_local_stack` -- the fallback, logged at start-up | none | verified |
| `SRV-005` | `FAIL` | A non-member MUST receive no metadata about a document -- not its owner, not its kind, not its existence. | audit `H2`; this spec section 3; audit `N7` | `ambre_server::routes::tests::handler_tests::get_document_hidden_from_non_member` -- a not-found rather than a forbidden, which is what hides existence; `::create_document_with_foreign_existing_id_is_not_found` -- the same answer on a taken identifier, so a caller cannot probe the namespace; `::members_of_an_absent_or_foreign_document_are_not_found` -- the member list draws the same 404 for an invented document and a foreign one; `ambre_server::routes::tests::handler_tests::a_removed_creator_does_not_reclaim_the_document` -- a creator removed from the document reads not-found like any non-member | none | verified |
| `SRV-018` | `POST` | A route that reads a document row MUST require an authenticated caller. | this spec section 3 | `ambre_server::routes::tests::handler_tests::get_document_refuses_an_anonymous_caller` -- the route takes the session extractor, so a caller without one is refused before any query | none | verified |

### 1.4 A document is written, shared or deleted

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-007` | `FAIL` | Every update, snapshot and presence sample MUST carry a valid signature by the key the caller's token names; an unsigned or forged one MUST have no effect. | this spec section 3; `ZK-005` | `ambre_server::ws::handler::message::ed25519_tests::invalid_signature_is_rejected`; `::tampered_ciphertext_rejects_signature`; `::unsigned_ephemeral_is_rejected` -- the refusals; `ephemeral_binding_tests::forged_author_key_writes_no_presence`; `routes::tests::handler_tests::create_snapshot_rejects_forged_author` -- the absence of effect on both paths | none | verified |
| `SRV-008` | `FAIL` | A read-only member MUST produce no write. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::viewer_role_delta_writes_no_pending` -- the live path leaves the queue empty; `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` -- the request path refuses, and a viewer holds the document key and its own signing key, so this check is the only thing between it and an accepted delta | none | verified |
| `SRV-015` | `FAIL` | Outside the exceptions of `SRV-144` to `SRV-146`, only an owner MUST be able to remove a member or re-key a document. | this spec section 3 | `ambre_server::routes::tests::handler_tests::rotate_key_denied_to_editor`; `::delete_document_only_by_owner` -- the refusals; `::revoke_then_rotate_replaces_owner_key` -- the owner's own path still completes | none | verified |
| `SRV-268` | `FAIL` | The creator of a document MUST NOT gain a member row by creating it again once any member row names it. | audit `N7`; this spec section 3; `SRV-005`, `SRV-175` | `ambre_server::routes::tests::handler_tests::a_removed_creator_does_not_reclaim_the_document` -- a creator a co-owner removed posts the creation again, holds no row after it and reads not-found on the document | none | verified |
| `SRV-283` | `FAIL` | The creator of a document MUST NOT gain a member row by creating it again once its log holds an entry or a channel holds it. | this spec section 3; `SRV-268`, `SRV-173`, `SRV-005`; audit `N38` | `ambre_server::routes::channels::tests::a_creator_never_reclaims_the_meta_of_a_channel` -- the creator of a public channel leaves it last and is removed from the space, then posts the creation of its `meta` again, reads not-found and holds no row | none | verified |
| `SRV-144` | `POST` | While a rotation is owed, the relay MUST accept it from any member with write access. | this spec section 3; `SYNC-128` | `ambre_server::routes::tests::handler_tests::an_owed_rotation_is_open_to_an_editor_and_clears_the_debt` -- an editor's rotation is refused while nothing is owed and accepted once a member left | none | verified |
| `SRV-145` | `POST` | A member MUST be able to remove themselves. | this spec section 3; `SYNC-127` | `ambre_server::routes::tests::handler_tests::a_member_leaves_alone_and_the_document_owes_a_rotation` -- an editor removes their own share, and the relay answers with success | none | verified |
| `SRV-188` | `POST` | A channel delete MUST reach every connection of each member of the channel. | this spec section 3; `SRV-142`, `SRV-187`; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::channels::tests::a_delete_reaches_every_member_of_its_channel` -- the owner's topic and a member's hear the notice on `meta`, and a member of the space outside the channel hears nothing, database tier | none | verified |
| `SRV-187` | `POST` | A removal MUST reach every connection of the member removed, even when it deletes their channel. | this spec section 3; `SRV-145`, `SRV-142`; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::tests::handler_tests::a_removal_reaches_the_person_removed_alone` -- the notice reaches the topic of the member removed alone, database tier; `ambre_server::routes::channels::tests::a_leave_that_deletes_its_channel_still_reaches_its_member` -- the last member's leave deletes the channel and still reaches their own topic, database tier | none | verified |
| `SRV-146` | `POST` | An admin of a space MUST be able to re-key a public channel of that space. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-spaces.md` | `ambre_server::routes::channels::tests::an_admin_of_the_space_rekeys_a_public_channel_of_it` -- an editor of the space outside the channel reads its members and links and re-keys it, while a viewer of the space and a private channel are refused, database tier | none | verified |
| `SRV-157` | `FAIL` | Given a removal from a member whose standing to moderate the relay revoked, the relay MUST refuse it. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-moderation.md` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-159` | `POST` | A conversion of a direct message MUST record the space of the channel and mark it private. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `Direct message conversion` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-160` | `FAIL` | Given a wire message on a document of an archived channel, the relay MUST refuse it. | this spec section 3 | not built | unbuilt -- lands with the `Archived channels` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-016` | `FAIL` | A caller MUST NOT grant a role above its own. | audit `N2`; this spec section 3 | `ambre_server::routes::share::tests::editor_cannot_grant_owner` -- the refusal; `::owner_can_grant_any_valid_role`; `::doc_role_rank_ordering` -- the ordering the clamp is read off | none | verified |
| `SRV-190` | `FAIL` | Given an invite on a document of a channel other than its `meta`, the relay MUST refuse it and add no row. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::channels::tests::an_invite_on_meta_opens_every_document_of_the_channel` -- an invite on the segment answers `409` and adds no row, database tier | none | verified |
| `SRV-224` | `POST` | An invite MUST name its invitee by id or by email, exactly one, so a member known by id alone can be invited. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::tests::handler_tests::an_invite_names_its_invitee_by_id_or_by_email_once` -- both or neither answer `409` and add no row, an unknown id `404`, an id alone invites, database tier | none | verified |
| `SRV-280` | `POST` | A search by address MUST answer only the account whose whole address is the query, after the normalization of section 3, and never an account for a part of its address or of its name. | this spec section 3; `SRV-163` | `ambre_server::routes::tests::handler_tests::a_search_answers_a_whole_address_and_nothing_less` -- a fragment of an address, a fragment of a name and a name answer no user, and a padded address in another case answers its one account, database tier | none | verified |
| `SRV-225` | `FAIL` | Given an invite into a channel of a space whose invitee is no member of that space, the relay MUST refuse it and add no row. | this spec section 3; `SRV-105`; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::channels::tests::an_invite_into_a_channel_of_a_space_needs_a_member_of_it` -- a private and a public channel each answer `409` with no row, and the same invite lands once the invitee joins the space, database tier | none | verified |
| `SRV-070` | `FAIL` | A caller MUST NOT change a member ranked above it. | audit `N2`; this spec section 3 | `ambre_server::routes::tests::handler_tests::editor_cannot_demote_owner_via_invite` -- the invitation is also the role-change path, so this is the half a clamp on the granted role alone leaves open | none | verified |
| `SRV-266` | `FAIL` | Only an owner MUST be able to re-invite a member who already holds a row. | audit `N8`; this spec section 3 | `ambre_server::routes::tests::handler_tests::an_editor_does_not_overwrite_the_row_of_a_peer` -- an editor who re-invites another editor is refused, and that editor's row is unchanged, while an owner's re-invite lands | none | verified |
| `SRV-017` | `FRAME` | Deleting a document MUST leave no relay state behind it. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_deleted_document_leaves_nothing` -- a delete purges the sequence, clock, pending, presence and dead-letter keys before the rows go, and the tombstone refuses every late write after | none | verified |
| `SRV-273` | `POST` | Deleting a document MUST remove every delta row of it, a row a late write inserts after the delete included, once the late-write window has passed. | this spec section 3; `SRV-017`, `SRV-057`; `ambre-spec-sequencer.md`, `SRV-274` | `ambre_server::routes::tests::handler_tests::a_deleted_document_leaves_no_delta_row` -- a delete removes three delta rows, inline and with an object of their own, a row inserted after it goes with the second sweep, and another document keeps its row | none | verified |
| `SRV-275` | `POST` | A delete of a document row MUST remove every delta row of it, whatever path deletes it. | this spec section 3; `SRV-273`; `ambre-spec-sequencer.md`, `SRV-274` | `ambre_server::hot_tail::db_tests::a_document_deleted_by_sql_leaves_no_delta_row` -- a plain SQL delete of a document row, with no relay code on its path, removes its inline delta and its delta with an object of its own, and another document keeps its row, on the database tier, behind the feature the test recipe carries (`ambre-spec-testing.md section 5`) | none | verified |
| `SRV-165` | `POST` | A delete that has begun MUST complete, even when a store fails or restarts before the end. | this spec section 3; `SRV-017`, `SRV-057` | `ambre_server::routes::tests::handler_tests::a_delete_interrupted_is_finished_by_the_pass` -- a delete the object store stops keeps its mark, and the pass empties the document, sets the lost tombstone again, then sweeps and drops the mark | none | verified |
| `SRV-137` | `POST` | A removal MUST delete the read cursor of the member removed. | this spec section 3; `ambre-spec-zero-knowledge.md` | `ambre_server::routes::read_cursors::tests::a_removal_deletes_the_read_cursors_of_the_member_removed` -- a removal from the `meta` deletes the cursors on every document of the channel, and another member's cursor stays | none | verified |
| `SRV-142` | `POST` | Deleting a channel MUST delete every document it groups. | this spec section 3; `SRV-017` | `ambre_server::routes::channels::tests::deleting_a_channel_deletes_its_documents_and_their_objects` -- database tier, with the objects swept from a fake store | none | verified |
| `SRV-173` | `POST` | The last member of a public channel MUST be able to leave it, owner or not, and the relay MUST keep the channel and its documents. | this spec section 3; `SRV-145` | `ambre_server::routes::channels::tests::the_last_member_leaves_a_public_channel_for_the_next_to_join` -- database tier, an editor then the owner leave, the channel and its documents stay with no delete begun, and a member of the space joins it | none | verified |
| `SRV-174` | `POST` | The last owner of a public channel MUST be able to leave it while other members stay, and an owner of its space MUST then be able to delete it. | this spec section 3; `SRV-173`, `MURMURE-244` | `ambre_server::routes::channels::tests::an_owner_of_the_space_deletes_a_public_channel_its_last_owner_left` -- the last owner leaves while a member stays, an owner of the space then deletes it, and an editor of the space or a private channel is refused, database tier | none | verified |
| `SRV-143` | `POST` | Deleting a space MUST delete every channel of that space. | this spec section 3; `SRV-142` | `ambre_server::routes::channels::tests::deleting_a_space_deletes_every_channel_then_the_space` -- every channel and its documents go before the space document, database tier | none | verified |
| `SRV-175` | `POST` | A direct message MUST be created whole in one transaction, in no space, with an editor row for each member and no owner, and only from a log that says so. | this spec section 3; `SRV-104` | `ambre_server::routes::channels::tests::a_direct_message_has_no_space_and_no_owner` -- database tier, an editor row per member; `ambre_server::routes::channels::tests::each_member_holds_the_key_wrapped_for_them` -- each row holds its own wrap; `ambre_server::routes::channels::tests::a_direct_message_with_an_owner_is_refused` -- a log that keeps an owner writes nothing | none | verified |
| `SRV-176` | `FRAME` | A direct message request MUST NOT create a second direct message for a set of members that already holds one. | this spec section 3; `SRV-175` | `ambre_server::routes::channels::tests::the_same_members_reach_the_same_direct_message` -- database tier, the existing one answers; `ambre_server::routes::channels::tests::racing_requests_create_one_direct_message` -- two racing requests create one | none | verified |
| `SRV-240` | `FAIL` | Given a direct message request of more members than its budget, the relay MUST refuse it before any lookup of a member. | `ambre-spec-budgets.md section 6`; `SYNC-180` | `ambre_server::routes::channels::tests::a_direct_message_past_its_budget_is_refused_before_any_lookup` -- database tier, unknown members are refused as a bad request, not as unknown, and one member fewer is created | none | verified |
| `SRV-177` | `POST` | A change of the members of a document MUST reach every connection subscribed to it, naming the document alone. | this spec section 3; `MURMURE-440` | `ambre_server::routes::tests::handler_tests::a_change_of_members_reaches_the_subscribers_of_the_document` -- database tier, an invitation, a change of role and a removal each publish it on the topic of the document; `ambre_server::routes::channels::tests::a_self_join_tells_the_subscribers_of_every_channel_document` -- a self-join that adds a row publishes it on `meta` and on its segment, whose access `meta` decides, and one that adds none is silent | none | verified |

### 1.5 A member joins a public channel alone

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-105` | `FAIL` | Given a caller who is not a member of the space of the channel, a self-join MUST be refused. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::routes::channels::tests::a_self_join_from_outside_the_space_is_refused` -- database tier | none | verified |
| `SRV-106` | `FAIL` | Given a channel the relay does not hold as public, a self-join MUST be refused. | this spec section 3 | `ambre_server::routes::channels::tests::a_self_join_of_a_channel_not_held_public_is_refused` -- database tier | none | verified |
| `SRV-107` | `POST` | An accepted self-join MUST store the entry the joining member signed, beside the new member row. | this spec section 3; `SYNC-106` | `ambre_server::routes::channels::tests::an_accepted_self_join_stores_the_entry_the_member_signed` -- the entry lands at the end of meta's log beside the row; `::a_self_join_of_a_logged_meta_without_an_entry_adds_no_member` -- a logged meta refuses a join with no entry | none | verified |
| `SRV-108` | `FRAME` | A self-join MUST NOT grant a role above editor. | this spec section 3 | `ambre_server::routes::channels::tests::a_self_join_grants_no_role_above_editor` -- database tier | none | verified |
| `SRV-191` | `POST` | A listing of a user's channels MUST answer each channel the user is a member of, with its space or none and whether it is public, and no other. | this spec section 3; `ZK-015`; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::channels::tests::a_user_lists_every_channel_they_are_a_member_of` -- a public channel, a private channel and a direct message each come with their place, and another user's private channel stays out, database tier | none | verified |
| `SRV-216` | `POST` | A listing of a user's channels MUST also answer every space whose document holds a row of the user. | this spec section 3; `ZK-015`; `5-Murmure/features/ambre-spec-murmure-layout.md`, `MURMURE-486` | `ambre_server::routes::channels::tests::a_user_lists_every_space_they_are_a_member_of` -- a space comes to its viewer with no channel, a Murmure document not made a space stays out, and a stranger lists none, database tier | none | verified |
| `SRV-217` | `FAIL` | A document of another kind than Murmure MUST be refused as a space. | this spec section 3; `SRV-216` | `ambre_server::routes::channels::tests::only_a_murmure_document_is_a_space` -- a Prose document marked a space answers a bad request, holds no member and lists no space, database tier | none | verified |
| `SRV-218` | `POST` | A space delete MUST reach every connection of each member of the space, one in no channel of it included. | this spec section 3; `SRV-143`; `5-Murmure/features/ambre-spec-murmure-spaces.md`, `MURMURE-487` | `ambre_server::routes::channels::tests::a_space_delete_reaches_every_member_of_the_space` -- a member of no channel hears the space revoked, and a member of a deleted document that is no space hears nothing, database tier | none | verified |
| `SRV-219` | `POST` | Each channel of a user's listing MUST carry its `meta` id, the name sealed on meta or none, and the user's wrapped key of meta. | this spec section 3; `ZK-015`; `SRV-191`; `5-Murmure/features/ambre-spec-murmure-layout.md` | `ambre_server::routes::channels::tests::a_listed_channel_carries_its_sealed_name_and_the_callers_key` -- a direct message lists its meta with no name, then the name its member sealed, each member with their own wrapped key and its wrapper, database tier | none | verified |
| `SRV-220` | `POST` | Each direct message of a user's listing MUST carry the display names of its other members, and a channel of a space none. | this spec section 3; `ZK-018`; `SRV-191`; `5-Murmure/features/ambre-spec-murmure-layout.md` | `ambre_server::routes::channels::tests::a_listed_direct_message_names_its_other_members` -- each of three members lists the two others by name, and a public channel of the same members lists none, database tier | none | verified |
| `SRV-223` | `POST` | Each channel of a user's listing MUST carry the last number of its `meta`, zero for a `meta` that holds nothing. | this spec section 3; `SRV-219`; `RESID-022` | `ambre_server::routes::channels::tests::a_listed_channel_carries_the_last_number_of_its_meta` -- an ephemeral counter and a snapshot's horizon each answer their number, and an empty `meta` answers zero, database tier | none | verified |
| `SRV-192` | `POST` | Adding a user to a channel, by an invite or a new direct message, MUST reach every connection of that user, naming the channel's `meta` document. | this spec section 3; `SRV-187`; `5-Murmure/features/ambre-spec-murmure-channels.md` | `ambre_server::routes::channels::tests::a_member_a_channel_adds_hears_the_share_of_its_meta` -- each member of a new direct message, and the invitee of a private channel, hears the share of `meta` on their topic, and a reused direct message is silent, database tier | none | verified |
| `SRV-189` | `POST` | A listing of a space MUST answer the channels the relay holds as public in it, and no other, to a member of the space alone. | this spec section 3; `ZK-015`; `5-Murmure/features/ambre-spec-murmure-spaces.md` | `ambre_server::routes::channels::tests::a_member_of_a_space_lists_the_public_channels_it_holds` -- a private, a deleted and another space's channel stay out, and a stranger gets a 403, database tier | none | verified |

### 1.6 A client asks for activity

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-109` | `POST` | The answer MUST say, per document, whether its last number is past the read cursor of the caller. | this spec section 3; `ZK-015` | `ambre_server::routes::read_cursors::tests::the_activity_answer_compares_each_last_number_with_the_callers_cursor` -- a document past the cursor, one at it, and a `meta` document that never appears | none | verified |
| `SRV-110` | `FRAME` | The answer MUST NOT carry a count of deltas. | this spec section 3; `ZK-015` | `ambre_server::routes::read_cursors::tests::the_activity_answer_compares_each_last_number_with_the_callers_cursor` -- every entry carries exactly a channel, a document, its kind, a yes or no, the caller's own cursor and the last number, and no count | none | verified |
| `SRV-221` | `POST` | Each entry of the activity answer MUST name its document's kind, a segment or a thread. | this spec section 3; `SRV-109` | `ambre_server::routes::read_cursors::tests::each_activity_entry_names_its_kind` -- a segment answers `segment` and a thread of the same channel `thread`, database tier | none | verified |
| `SRV-222` | `POST` | Each entry of the activity answer MUST carry its document's last number. | this spec section 3; `SRV-109`; `RESID-022` | `ambre_server::routes::read_cursors::tests::each_activity_entry_carries_its_last_number` -- two ephemeral counters and a snapshot's horizon each answer their number, one read up to it included, database tier | none | verified |

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
| `SRV-302` | `FAIL` | While the `meta` of a channel owes a rotation, the relay MUST refuse every wire message of each document of the channel sealed under the key the removal left behind. | this spec section 3; `SRV-123` | `ambre_server::routes::channels::tests::a_leave_of_a_private_channel_owes_its_segment_a_rotation` -- database tier: after a leave of a private channel, the segment owes the rotation in the register and in the store, and a delta on it is refused | none | verified |
| `SRV-124` | `POST` | A rotation that lands MUST clear the debt of its document. | this spec section 3; `SYNC-128` | `ambre_server::routes::tests::handler_tests::an_owed_rotation_is_open_to_an_editor_and_clears_the_debt` -- the register and the mirror read no debt after the rotation; `::a_revoke_removes_and_rotates_in_one_request` -- a revoke leaves none | none | verified |
| `SRV-125` | `FRAME` | Refusing a delta for its key MUST NOT give it a sequence number. | this spec section 3; `ambre-spec-sequencer.md` | `ambre_server::redis_ops::lua_tests::a_frame_owed_a_rotation_takes_no_seq_and_no_clock` -- the refused delta leaves no sequence key, and the same clock is accepted at sequence 1 once the debt clears | none | verified |
| `SRV-167` | `FAIL` | A rotation MUST be refused whole when the document holds a member it did not read. | this spec section 3; `SYNC-143` | `ambre_server::routes::tests::handler_tests::a_member_invited_during_a_revoke_gets_the_new_key` -- a revoke that read the members before an invitation landed is refused with `MEMBERS_CHANGED`, removes no one and stores no link, and the same revoke read again lands with a copy for every member left | none | verified |
| `SRV-299` | `FAIL` | A rotation MUST be refused whole when it carries no copy of the new key for a member it keeps. | this spec section 3; `SRV-167`; `SRV-124` | `ambre_server::routes::tests::handler_tests::an_owed_rotation_is_open_to_an_editor_and_clears_the_debt` -- database tier: a rotation that leaves the owner without a copy is refused and the debt stays owed, and the same rotation with a copy for every member lands | none | verified |
| `SRV-168` | `FAIL` | An invitation MUST be refused when a rotation has replaced the key its copy wraps. | this spec section 3; `SYNC-143` | `ambre_server::routes::tests::handler_tests::a_member_invited_during_a_revoke_gets_the_new_key` -- an invitation naming the replaced key is refused and adds no one, and one naming the current key lands | none | verified |
| `SRV-193` | `FAIL` | A rotation MUST be refused whole when its link names a previous key other than the current key of the document. | this spec section 3; `SYNC-116` | `ambre_server::routes::tests::handler_tests::the_relay_keeps_every_link_in_rotation_order` -- database tier, a link from a key a landed rotation replaced is refused with no copy changed | none | verified |
| `SRV-136` | `POST` | A removal from a space MUST remove the member from every channel of that space, and mark each private one as owing a rotation. | this spec section 3; `5-Murmure/features/ambre-spec-murmure-spaces.md` | `ambre_server::routes::channels::tests::leaving_a_space_leaves_every_channel_of_it`; `::a_removal_from_a_space_leaves_every_channel_of_it` -- the shared private channel owes a rotation, the sole-member one is deleted and the public one owes none, database tier | none | verified |

* **Budget.** the per-operation targets of `ambre-spec-budgets.md section 3`, and the roughly
  50 KB per open document of `ambre-spec-budgets.md section 6`. No bench measures the relay: each
  of its rows there reads **none exists**.
* **Boundary.** this document is the relay side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SRV-001` keeps the payload out of the process and the logs, and `SRV-012` refuses the
  relay the ability to open one. `SRV-005` keeps the routing metadata behind the membership check,
  and `SRV-096` keeps a store's words out of an answer.
* **Verification.** 121 `verified`, 1 `human-review`, 9 `unimplemented`. A request test drives what
  holds: the roles, the rank clamps, the signature gate, the error mapping, the origin list and the
  delete. No log line can name the payload type. A deployment's isolation is read by hand.
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
| the ephemeral store | the sequence counters, the per-author clocks, the unflushed deltas, the refresh tokens, the presence blobs, the sign-in handshakes, the tombstones of deleted documents, the mirror of each owed rotation, and the state of each call | recoverable for the ordering (`ambre-spec-sequencer.md`) and the mirror, final for a session and for a call, which ends (`SRV-249`) |
| object storage | every sealed blob: snapshots, deltas and media | the content itself, which nothing else holds |

**The ephemeral store is Redis 7.4 or later.** The presence and session scripts expire one hash field
at a time with `HEXPIRE`. An older server accepts every other command, so the failure would come late.
The relay reads `INFO server` at boot and refuses an older version (`SRV-241`).

**Every key and every script is written for a cluster.** One Redis server runs its scripts on one
thread, so the ordering of every delta shares one core. A cluster spreads the documents over
several servers, but it refuses a script whose keys sit in two hash slots. A key thus carries the
identifier it serves as its hash tag (`SRV-252`), and a script names the keys of one slot alone
(`SRV-253`).

A classic broadcast reaches every node of a cluster, so the relay publishes on sharded
channels (`SRV-254`). A sweep reads every primary, since one node holds a part of the keyspace
(`SRV-255`). The rules hold on one server too, so the move to a cluster changes the deployment
and no code. The relay reads the store's address from `REDIS_URL`, and a cluster's address starts
with `redis+cluster://`.

**The set of unflushed documents sits in a slot of its own.** No script may pair it with a key of a
document. The three operations that touch both thus run as separate commands. Their order keeps
every delta marked:

| Operation | Order | Why it keeps a delta marked |
| :--- | :--- | :--- |
| append | the script pushes the entry; the mark follows when the list was empty | a list that holds an entry was marked by the push that filled it |
| clear, after a drain | drop the mark, read the length, mark again when the list holds an entry | a push the read misses lands after the drop, and marks after it |
| purge | the script empties the document; the mark goes next | a mark that outlives its list costs one empty visit of the flusher |

A relay that stops between a push and its mark leaves the list unmarked. The sweep of the keyspace
finds it (`ambre-spec-sequencer.md section 3`), one primary at a time. The relay reads the
primaries from `CLUSTER NODES` and opens one connection to each for the sweep. It reads the
version of each primary the same way at boot (`SRV-241`).

**No live key moves.** The hash tags renamed every key of a document, of a call and of a rate
window. No relay held live keys on the day this landed, 2026-11-13, so no migration ships. A
deployment that runs an older relay drains its pending lists before the upgrade.

**A delete fences the document before it empties it.** The relay first marks the delete in the
register. It then sets a tombstone in the ephemeral store. Every script that writes a key of the
document checks it and refuses. An update, a presence sample or a flush already past its checks then
writes nothing (`SRV-017`). The objects go next, then the keys, then the rows.

**A trigger removes the delta rows.** No foreign key ties a delta row to its document, so that an
insert locks no document row (`ambre-spec-sequencer.md`, `SRV-274`). A trigger on the delete of a
document row removes its delta rows in the same statement. It holds on every path: the relay, a
cascade and plain SQL (`SRV-275`). The trigger fires on a delete alone, so an insert pays nothing.

A write past its checks when the tombstone was set may still insert a row after the delete. No
trigger sees that row, so the second sweep removes it (`SRV-273`).

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
| a space document | that a Murmure document is a space, set at its creation | the listing of a user's channels (`SRV-216`) |
| a read cursor | a member, a document and a number | the activity answer (`SRV-109`) |

A space is a document like any other, so its members are its member rows. A direct message is a
channel with no space. It arrives whole in one request: both documents, an editor row for each
member, and the log of `meta` that grants exactly those rows (`SRV-175`). The client signs that log
and the relay checks it act by act. Both follow one sequence, `ownerless_log` in `ambre-protocol`,
so the two sides cannot drift apart.

The segment of a direct message holds no row of its own, since every document of a channel reads
the rows of `meta`. A direct message has no owner, so `SRV-011` never applies to it.

Two direct messages never hold the same members (`SRV-176`). The request takes a transaction lock
keyed by the sorted set of members. It then looks for a channel with no space whose rows on `meta`
are exactly that set. A racing request waits on the lock and finds the first one's channel. The
lock is advisory and lasts the transaction, so no table holds the set.

A change of the member rows of a document is said on the topic of that document (`SRV-177`). An
invitation says it, as do a change of role, a removal and a self-join that adds a row. The message
names the document alone, which the topic already names, so a subscriber learns no row from it. A
client that shows the members reads them again. A change on a channel's `meta` is said on every
document of the channel, since `meta` decides access to each (`SRV-103`). A removal that deletes the
channel says nothing, since no member stays to hear it.

A public channel outlives its last member. The space reads its key in the catalogue, so the next
member joins it alone, and the last one leaves owner or not (`SRV-173`). A private channel goes
with its last member instead (`SRV-142`), each of its documents deleted as a deleted document is
(`SRV-165`). The last owner of a public channel leaves it too, while others stay. An owner of the
space document then stands in for it, as it does for the whole space (`SRV-174`). An editor of the
space does not, so a catalogue writer deletes no channel.

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

**A search by address names one account.** The search exists to find the keys of an invitee the
sharer already knows by address. The relay compares the trimmed
query to each stored address without regard to case, as the unique index on addresses does. Only an
equal address matches, so the answer holds one account at most (`SRV-280`). A caller learns whether
a known address has an account. No part of an address or of a name lists anyone.

**A channel is invited into on `meta`.** The rows of `meta` grant every document of the channel.
A row on a segment would grant nothing, so an invite there is refused (`SRV-190`).

**The request pipeline.** Four gates, in this order, and each one is the reason the next is cheap.

1. the rate limiter picks a tier from the path and a bucket from the caller (`SRV-013`, `SRV-019`).
   Behind a chain of proxies, each proxy appends its peer, so the caller is the entry the first
   proxy wrote, counted from the right (`SRV-285`). A refused request records nothing, so a flood
   grows no bucket (`SRV-286`). The provider configuration proves nothing, and counts in the
   ordinary tier (`SRV-287`);
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
| a name already taken, a payload that will not decode, an identifier that is not one (`SRV-179`), a word no closed set holds (`SRV-180`), a fixed-width value that is not the hexadecimal of its width (`SRV-242`) | `409` |
| a frame while a rotation is owed | on the socket, a refusal naming the document, the key and the clock (`SRV-123`) |
| a signature that does not verify | `422` |
| a blob larger than its cap | `413` (`ambre-spec-object-store.md`) |
| the register, the ephemeral store or object storage | `500`, one generic message, the cause in the log alone (`SRV-020`) |

Each body is a declared type: a refusal holds its code and its message, and a write names its
outcome from one closed set (`SRV-181`). The code of a refusal is a closed set too, shared with
Sync (`SRV-182`).

**What every answer carries**, whatever the route.

| Header | Value, and why |
|---|---|
| the strict transport policy | two years, subdomains included: the relay is reachable over one transport only (`SRV-270`) |
| the content policy | nothing to load and no page to frame it: the relay serves no document (`SRV-261`) |
| the allowed origins | the list the deployment names, and nothing when it names none (`SRV-068`) |

The two headers that make the client's shared memory legal are the opener policy and the embedder
policy. Both belong to whatever serves the page and the worker bootstrap, not to this process
(`SRV-069`, `ambre-spec-architecture.md section 2`). Their cost is that an embedded cross-origin
resource must carry a policy of its own. That is why a presigned link's target must allow a
cross-origin read.

**The page runs the scripts it ships, and no other** (`SRV-260`). A script injected into the page runs
with the plaintext the page holds. So the page carries a content policy in a meta of its head. The
meta stands before every other element. The build writes it into the page itself, so it travels
into every deployment that serves the build. A header would hold only where a server of this
repository sets it, and the deployment's server is outside the repository.

| What the page loads | What the policy admits |
|---|---|
| a script | the files of its own origin, and each inline script of the page by its SHA-256 digest |
| a wasm module | its compile (`wasm-unsafe-eval`), never an evaluated string |
| a worker or a thread | a script of its own origin. The script of each thread is a file of the build, never a `blob:` address |
| a style | the files of its own origin, each inline style by its digest, and the token stylesheet the launcher injects |
| a picture shown from memory | a `blob:` or `data:` address |
| a web embed | a sandboxed frame (`TOILE-088`) of a site the deployment names in `AMBRE_EMBED_SITES`, and no frame when it names none (`SRV-312`) |
| anything else | its own origin. No plugin object, no base address and no form target |

The post-build hook computes each digest from the page as the build leaves it
(`scripts/trunk_post_build.py`). A view sets a style through the style declaration of its element,
which the policy does not govern. It never sets a `style` attribute, which the policy refuses
(`UI-THREAD-063`). The thread library read its global scope through an evaluated string. The
vendored copy reads it without one (`vendor/wasm_thread/PATCH.md`).

A meta policy governs the page alone, and it cannot refuse a page that frames this one. A thread or
a worker takes its policy from the answer that serves its script. So the post-build hook also
writes `_headers` beside the page, the format Netlify and Cloudflare Pages read. The file travels
with the build, as the meta does.

| Path | What its answer carries |
|---|---|
| every path | the two isolation headers, and `frame-ancestors 'none'` (`SRV-281`) |
| the loader of the Sync worker, the loader of the call worker, the thread script | the worker policy (`SRV-282`) |

The worker policy admits as a script the files of the build and the compile of a wasm module. It
admits a worker of its own origin alone. It leaves a connection open: the Sync worker fetches the
links the relay presigns, and the build does not know the origin of object storage. The server of
the browser tier applies the file. The development server sets one set of headers on every answer,
so it carries the frame refusal alone. A deployment's server applies the file, or writes the same
headers in its own configuration.

**Where the process is stateless, and where it is not.** Every store is external, so a client
reaches any instance and the fan-out crosses them (`SRV-021`). Two things are per process, and
neither is per request. The first is the signing key derived from the deployment's seed, which
every instance must share; the second is the fan-out hub (`ambre-spec-broadcast.md`).

**A deployment runs on any host.** A release publishes its images by digest (`SRV-308`), so an
auditor can match a running image to its source. The chart of the repository deploys them on any
Kubernetes cluster (`SRV-309`), and a compose file runs them on one host (`SRV-310`). The SFU media
takes a public address with no load balancer in front (`CALL-141`). The relay needs Redis 7.4 or
later (`SRV-241`), an S3-compatible object store and its metrics port kept private (`SRV-257`). The
configuration of one hosting provider lives outside this repository.

**One address serves the page and TURN.** A host with one address has one port 443, and TURN over
TCP needs that port on networks that open nothing else (`CALL-042`). The compose file puts one TLS
front before both, and the front routes each connection by its server name. A second address for
TURN is rare on a self-hosted server.

**A deployment names its own credentials** (`SRV-256`). The local stack of `docker-compose.yml`
has a database password and an object store key that guard nothing. The relay falls back to them
in development mode alone, and announces each fallback at start-up. Outside it, an unset variable
stops the start before the first connection, and the error names the variable.

**A panic ends the work it hit, and nothing more** (`SRV-101`). A request already runs on its
connection's task, and a socket on its own. A layer answers a panicking request with the generic
failure of section 3, never with the panic's message (`SRV-096`). That layer is the `catch-panic`
feature of `tower-http`, a crate the relay already links, so nothing new is trusted. A socket that
panics still leaves the hub and announces its departure.

The hub, the flusher and the compaction serve every client. Each runs its work one unit at a time
and drops a unit that panics. The hub keeps its routing table across a restart and subscribes its
new stream to every topic in it. A socket whose topic the hub cannot route closes, and its client
reopens it.

**The metrics take a listener of their own** (`SRV-257`). The public listener serves the API alone.
A scrape reads `GET /metrics` on the address of `METRICS_ADDR`, the loopback by default. So a
deployment that names no address exposes no metric off the host. The text is the Prometheus text
format, version 0.0.4.

Each label value is a variant of a closed `enum`, and each metric holds one
series per variant from the start (`ZK-029`). The crate `ambre-metrics` writes them: a few atomics
and a text writer, with no dependency beyond `ambre-core`. `deny.toml` grants it to the relay and
the SFU alone.

| Metric | Kind | Label |
|---|---|---|
| `ambre_relay_sockets` | gauge | `socket`: the document socket or the call socket |
| `ambre_relay_deltas_total` | counter | `outcome`: sequenced or refused |
| `ambre_relay_sequencing_seconds` | histogram | none: the time from the clock check of a delta to its broadcast |
| `ambre_relay_failures_total` | counter | `failure`: the index write, the pending append, the dead letter or the broadcast |

A log line at the info level names no identifier, and no log line names an address (`ZK-030`,
`ZK-031`). The debug level keeps the document of each step of the sequencer.

**Code.**

* `crates/ambre-server/src/main.rs` -- the boot, the route table, the layers and the periodic tasks;
* `crates/ambre-server/src/supervise.rs` -- what keeps a panic inside the request, socket or task unit it hit;
* `crates/ambre-server/src/metrics.rs` -- the metrics, their closed labels and their listener;
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
| channels | the documents a channel groups, listed to a member alone; the public channels of a space, listed to its members; a public or private channel registered in its space; a direct message, created or found; a self-join; the owner's delete | this document, `5-Murmure/features/ambre-spec-murmure-spaces.md` and `5-Murmure/features/ambre-spec-murmure-channels.md` |
| read cursors | a cursor, the activity answer, marking read | this document, and `ambre-spec-murmure-unread.md` |
| the socket | one upgrade | `ambre-spec-broadcast.md` |

**What the surface does not carry.** Calls have no route at all (`ambre-spec-call-media.md`). Of
the channel routes, these are built: two listings, the activity answer, a registration, the direct
message, the signed self-join and the delete. The retention of a space is designed and unbuilt
(`SRV-111` and `SRV-112`).

**The last number of a document.** The activity answer compares the caller's cursor with the
higher of two numbers. One is the sequence counter in the ephemeral store. The other is the
register's horizon: the highest flushed delta and the highest snapshot coverage. The counter alone
is lost with the ephemeral store; the register alone lags the unflushed deltas. Only segments and
threads appear: `meta` and amendments carry no message a member reads.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A trigger removes the delta rows of a deleted document | the relay removes them in each delete path | a new delete path, or a delete by hand, that forgets the rows leaves orphans no query reaches |
| A trigger removes the delta rows of a deleted document | one trigger per statement, over the rows it deleted | the set of rows deleted carries no statistics, and the plan may scan all of `updates` for one document |
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
| A deployment names its database address and its object store key | the credentials of the local stack as the default | a deployment that forgets one variable starts, and signs with a key that every clone of the repository holds. The fallback exists only in development mode and announces itself (`SRV-256`). |
| The relay refuses the key a removal left behind | refusing every write until someone rotates | a delta sealed under the new key would wait for nothing, and the document would freeze whenever no member is online |
| The relay marks a debt and a member rotates | the relay rotating the key itself | the relay holds no key, and a key it chose would break the boundary `ZK-001` draws |
| Any writer settles an owed rotation | the owner alone | the relay refuses every wire message under the old key until the rotation lands, so a channel whose owner is away would stop for everyone |
| One set of unflushed documents, marked once per drained list | one set per bucket of documents, each with a hash tag of its own | the mark is one `SADD` per list that was empty, not one per delta, so the one set is not the hot key of a cluster. Buckets add keys and a read per bucket to every flush cycle, for a load no measure shows |
| The metrics are a crate of the workspace | the official Prometheus client, or the `metrics` facade | each one takes a label value as a runtime text, so one call with an identifier copies the membership graph into the store. A closed `enum` makes that call unwritable. The format is a few lines, and each crate adds a registry and a lock for nothing more |
| The scrape answers no token, on the loopback by default | a bearer token on a public port | a token is one more secret to rotate, for figures that name no one. The loopback, or a private network the operator names, keeps the scrape off the internet |
| The page carries its content policy in a meta its build writes | a header of the server that serves the page | the deployment's server is outside the repository, so no test reads its header, and the digest of each inline script changes with each build. The build writes the headers a meta cannot carry into `_headers`: the policy of a worker, and a refusal to be framed (`SRV-281`, `SRV-282`) |
| The headers file names each worker script by its exact path | a pattern such as `/ambre-*.js` | a pattern reaches a file that is no worker script, and the hook knows each hashed name when it writes the file |
| An inline script is admitted by its digest | a nonce, or `unsafe-inline` | a nonce needs a server that writes a fresh one into each answer, and the page is a static file. `unsafe-inline` admits the injected script the policy exists to refuse |
| The thread library is vendored without its evaluated strings | `unsafe-eval` in the policy | it would admit any string an injected value reaches `eval` with. The copy changes four calls (`vendor/wasm_thread/PATCH.md`) |
| The script of each thread is a file of the build | a `blob:` worker, as the thread library builds it | a `blob:` source in `worker-src` admits a worker built from any text a script of the page holds |
| The sweep connects to each primary in turn | a scan through the cluster client | the client refuses `SCAN` on a cluster, since each node answers for its own slots alone. `KEYS` on every shard blocks each node for the length of its keyspace |
| A search by address matches the whole address | a match on a part of the address or of the name, for a suggestion list | any caller with a session lists the directory two characters at a time, each address and key with it, and the setup route issues a session to anyone (audit `P4`). The sharing flow already holds the whole address it searches |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The register declares meeting tables nothing serves.** Meetings exist in the durable schema,
  and no route and no handler writes them (`ambre-spec-broadcast.md`). Their plaintext name
  breaks `ZK-022`, and they name no channel: the calls story replaces them.
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
