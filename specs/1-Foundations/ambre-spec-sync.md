# AMBRE -- FEATURE: THE SYNC WORKER

**Type:** Horizontal (framework)

> The one part of the client that talks to the network. It holds every key, opens one connection,
> and hands the rest of the application content that is already decrypted or already sealed.

Nothing else in the client holds a key, opens a socket or makes a request. This document owns the
worker itself: how it starts, what it holds, how it stays connected, and what it relays. What it
does to a payload is the five documents beside it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sealed-envelope.md`, `ambre-spec-delta-order.md`, `ambre-spec-membership.md`,
`ambre-spec-key-rotation.md` and `ambre-spec-media.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-002` | `INV` | The document key and the private signing key MUST NOT appear in any outbound message; only the public half travels. | `ZK-002`; this spec section 2 | crate-graph -- `deny.toml` refuses the exchange, the cipher and `ambre-crypto` itself outside this worker, so no other crate holds a type either key could travel in | none | verified |
| `SYNC-095` | `INV` | A key MUST be wiped from memory when it goes out of scope. | `ZK-002`; this spec section 2 | type-level -- both key types hold an `ambre_core::Secret` (`ZK-019`), and so does the X25519 `SharedSecret` a wrapping key derives from (`a_shared_secret_is_wiped_and_never_printed`), so the wipe is the destructor and cannot be forgotten at a call site. It is what makes a sign-out mean anything on a machine the attacker may later read | none | verified |
| `SYNC-011` | `INV` | The Sync worker MUST compile no CRDT code. | this spec section 2 | crate-graph -- `deny.toml` grants `loro` to the four application crates, `ambre-logic` and `ambre-crdt` alone, and `ambre-crdt` to the four application crates alone, so this worker fails the `deny` job on the line that would add either | none | verified |
| `SYNC-096` | `INV` | The Sync worker MUST compile no rendering code. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-057` | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the graph `cargo metadata` resolves and refuses an edge through any crate of the graph that brings `ambre-canvas`, `ambre-draw`, `wgpu` or another render crate into what `ambre-sync` links (`ambre_primitives::boundaries::tests::a_thread_reaching_the_rendering_through_any_crate_is_refused`); cargo-deny -- `deny.toml` grants `wgpu` to `ambre-canvas` and `ambre-app`, and the CI `deny` job fails any other crate that takes it | none | verified |
| `SYNC-056` | `INV` | The Sync worker MUST run in its own instance, with a linear memory the rest of the client cannot address. | `ambre-spec-architecture.md section 2` | ci-gate -- `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads the graph `cargo metadata` resolves and refuses an `ambre-sync` that builds no `cdylib`, and a crate that depends on it (`ambre_primitives::boundaries::tests::a_dependency_on_the_worker_and_a_worker_with_no_artefact_are_refused`); ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a page that does not load it as a worker, text neither tool reads | none | verified |
| `SYNC-060` | `INV` | A token MUST live in this worker's memory alone: never in browser storage, never in a cookie. | `ambre-spec-session.md`, `SRV-031` | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a path of `ambre-sync` that reaches browser storage, the browser database, the cache or a cookie (`ambre_primitives::driver::bounds::browser_storage_under_a_foreign_name_is_refused_in_the_worker`) | none | verified |
| `SYNC-061` | `INV` | A token MUST NOT reach a log. | `ZK-006`; `ambre-spec-session.md`, `SRV-031` | `ambre_sync::websocket::tests::the_socket_address_carries_no_token`; `ambre_sync::websocket::tests::a_logged_token_pair_carries_no_token`; `ambre_core::secret::tests::a_session_token_prints_nothing_and_travels_as_its_text` -- the socket address holds no token, and the token pair prints with the token redacted. The raw form is read in the Secret module alone (`ARCH-045`). It reaches a request header through the sink `Credential<AccessToken>::present_bearer` (`ARCH-049`), and the socket frame through a clone of that `Credential<AccessToken>`, which shares the wrapper | none | verified |
| `SYNC-013` | `INV` | Application code MUST contain no unchecked memory access. | README pillar "Security" | lint-wall -- `unsafe_code = "forbid"` in the root `[workspace.lints.rust]`, which every crate inherits (`ARCH-032`) | none | verified |
| `SYNC-058` | `INV` | The worker MUST NOT block its own event loop: the socket and the requests MUST stay serviceable. | `ambre-spec-architecture.md section 4` | `ambre_sync::run_tests::the_loop_yields_to_the_tasks_beside_it` -- on one thread, a task beside the waiting loop runs, and closing the commands from it wakes the loop; `ambre_sync::run_tests::the_loop_hands_each_command_to_the_worker` -- the loop hands a command it waited on to the worker | none | verified |
| `SYNC-191` | `INV` | The worker's spacing between two replay asks of one document MUST outlast the relay's throttle window. | this spec section 2; `ambre-spec-sequencer.md`, `SRV-086`; `ambre-spec-budgets.md section 8` | type-level -- a const assertion beside `REPLAY_ASK_SPACING` compares it with the protocol's `REPLAY_MIN_INTERVAL`, the window the relay reads, so a raised window fails the build | none | verified |

### 1.2 The worker starts

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-057` | `POST` | No command MUST be lost while the worker starts: everything sent before it is listening MUST wait. | this spec section 2 | `ambre_bus::start::tests::no_command_is_posted_before_the_worker_announces_itself` -- commands queued before the announcement are posted after it, in order, and an encoded event never reads as one | none | verified |
| `SYNC-065` | `POST` | An edit made before the session is ready MUST be held, never dropped. | this spec section 2 | `ambre_sync::updates::tests::an_edit_before_the_session_is_held_then_sent` -- two edits made with no signing key are sent, in order, once the key is set and the replay lands | none | verified |
| `SYNC-097` | `FRAME` | Commands held during the start MUST NOT be reordered when they are released. | this spec section 2; `ambre-spec-delta-order.md` | `ambre_sync::updates::tests::held_commands_are_released_in_order` -- the held opens subscribe in order, and the held edits leave in order at rising clocks | none | verified |
| `SYNC-271` | `FRAME` | The worker MUST reach the relay with the scheme of the page that started it, on this machine too. | this spec section 2; `ambre-spec-murmure-phone-measures.md section 2` | `ambre_sync::http::tests::the_scheme_of_the_page_wins_over_the_host` -- a page over TLS on the loopback reaches the relay over TLS, a page in the clear on another host reaches it in the clear, and a page with no scheme falls back to the host rule. The worker start reads the protocol of its own location into that scheme | none | verified |
| `SYNC-305` | `POST` | Each message the Sync worker logs MUST print one console line of three segments, each opened by a `%c` directive: the level word in its colour, the file and line of its caller, then the message. Each MUST then leave a performance mark, and a measure from that mark labelled with its level, its caller and its message. | this spec section 2, the log; `ambre-spec-budgets.md section 5` | `ambre_sync::log::tests::a_worker_line_holds_the_level_the_caller_and_the_message`; `::each_level_prints_its_word_in_the_colour_of_the_layer`; `::a_worker_message_marks_the_profile_with_a_measure_of_its_text` -- the native tier reads the values the worker hands the browser; `ambre_sync::tests::worker_log::a_worker_message_reaches_console_log_with_its_three_styles`; `ambre_sync::tests::worker_log::a_worker_message_leaves_its_measure_in_the_browser_profile` -- run by `just test-wasm` in the browser, an edit logged before the session reaches `console.log` with its three styles, and its measure reaches the profile | none | verified |

### 1.3 The socket connects, and reconnects

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-063` | `POST` | A socket the relay refuses on authentication MUST be renewed, not merely retried. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-046` | `ambre_sync::lifecycle::tests::an_authentication_refusal_renews_and_reopens` -- the refusal's code renews, and the renewal reopens with the wait reset; `ambre_sync::websocket::tests::a_failed_renewal_backs_off_before_the_reopen` -- a renewal that fails backs off, then reopens once the wait is over | none | verified |
| `SYNC-133` | `POST` | A socket the relay refuses for a key setup still owed MUST be neither renewed nor retried until the keys are stored. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-164` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_is_neither_renewed_nor_retried` -- on the native transport, the refusal opens no socket through a timer or a renewal, and the stored session of a setup opens one, as the authentication code does | none | verified |
| `SYNC-134` | `POST` | A key setup still owed MUST reach the rest of the client as a setup to complete, never as a generic error. | this spec section 2; `ambre-spec-session.md`, `SRV-027` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the refused socket; `ambre_sync::setup_tests::a_request_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the sink turns a setup refusal into the setup, under either error variant; `ambre_sync::http::tests::only_a_setup_refusal_reads_as_one` -- only a 403 with the setup code reads as one; `ambre_sync::http::tests::an_account_without_master_material_owes_its_setup` -- the test an unlock branches on; `ambre_sync::auth::tests::an_unlock_opens_stored_keys_and_creates_none` -- an unlock of an account that owes its keys ends as a setup owed; `ambre_sync::sharing::tests::a_refused_leave_keeps_the_relay_refusal` -- a refused leave reaches Logic with its code, a setup owed as the setup; `ambre_logic::dispatch::tests::a_setup_owed_reaches_the_ui_as_a_setup` -- the logic thread relays it | none | verified |
| `SYNC-137` | `POST` | A wire message of a newer version MUST reach the rest of the client once as an update to install, and the socket MUST NOT reopen in that session. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-166` | `ambre_protocol::tests::only_a_newer_wire_version_calls_for_an_update` -- an older version and an empty frame do not; `ambre_sync::websocket::tests::a_newer_wire_version_reaches_logic_as_an_update_and_closes_for_good` -- once, with no error, and no close, timer or renewal reopens; `ambre_logic::dispatch::tests::a_newer_relay_reaches_the_ui_as_an_update` -- the logic thread relays it | none | verified |
| `SYNC-070` | `POST` | A socket error MUST tear the connection down and schedule a reconnect, exactly as a close does. | audit `M10`; this spec section 2 | `ambre_sync::lifecycle::tests::an_error_backs_off_as_a_close_does` -- an error reads as a close with no code, and arms the same wait; `ambre_sync::websocket::tests::a_dropped_socket_reports_every_document_disconnected` -- a close reports every open document disconnected | none | verified |
| `SYNC-064` | `POST` | The reconnect delay MUST grow while no wire message arrives. | this spec section 2 | `ambre_sync::lifecycle::tests::the_wait_grows_while_no_frame_arrives` -- each close doubles the wait up to its cap; `ambre_sync::websocket::tests::a_dropped_socket_reopens_on_the_clock_and_a_frame_resets_the_wait` -- the worker reopens only once its clock reaches the wait | none | verified |
| `SYNC-098` | `POST` | The reconnect delay MUST reset on an inbound wire message, and never on a bare open. | this spec section 2 | `ambre_sync::lifecycle::tests::the_wait_resets_on_a_frame_alone` -- a reopen with no frame keeps the wait growing, and a frame resets it; `ambre_sync::websocket::tests::a_dropped_socket_reopens_on_the_clock_and_a_frame_resets_the_wait` -- the same through the worker, after an open with no frame | none | verified |
| `SYNC-059` | `FRAME` | The worker MUST stop reading the socket while the path back to the rest of the client is full. | this spec section 2; `ambre-spec-message-bus.md`, `BUS-026` | `ambre_bus::conduit::tests::a_backlog_holds_its_source_at_the_high_mark_and_releases_it_at_the_low` -- the Main side asks once at each mark, never in between; `ambre_sync::websocket::tests::a_held_inbound_reopens_only_on_release_and_catches_up` -- no close, timer or renewal reopens a held socket, and the release pulls from the last sequence then sends the edit kept; `ambre_sync::websocket::tests::while_held_only_an_edit_is_kept_for_the_reopen` -- while held, a frame other than an edit is not kept; `ambre_sync::websocket::tests::the_inbound_hold_reports_the_documents_and_the_release_reopens_when_wanted` -- the hold reports every document disconnected, and the release reopens only when a document waits and no setup is owed | none | verified |
| `SYNC-153` | `POST` | Given a replay the relay stopped, the worker MUST ask again from its own cursor, and no sooner than the relay's throttle serves it. | this spec section 2; `ambre-spec-sequencer.md`, `SRV-184`, `SRV-086`; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_stopped_replay_is_asked_again_from_the_cursor_when_its_timer_fires` -- nothing is sent before the timer, a stale timer asks nothing, and the live one asks from the document's cursor; `ambre_sync::websocket::tests::the_replay_retry_delay_doubles_up_to_its_budget` -- the first delay is the throttle window, and each later one doubles it; `ambre_sync::websocket::tests::a_replay_retry_asks_nothing_for_a_local_document` -- a retry asks the relay for a document it holds alone | none | verified |
| `SYNC-188` | `FRAME` | The worker MUST NOT ask a second replay of one document on one socket inside the relay's throttle window. | this spec section 2; `ambre-spec-sequencer.md`, `SRV-086`; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_replay_ask_inside_the_relay_window_waits_for_it_to_pass` -- a second ask is held, an early timer sends nothing, the window's end sends the lowest cursor, a document let go asks nothing, and a reopened socket asks at once | none | verified |
| `SYNC-154` | `FAIL` | A replay stopped more times than its budget MUST reach the rest of the client as one error, and no retry MUST follow before the next reconnect. | this spec section 2; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_replay_stopped_past_its_budget_reaches_logic_once` -- eight stops give one error and no retry, and a reopened socket counts from zero | none | verified |
| `SYNC-157` | `FAIL` | A document whose first replay stops past its budget MUST reach the rest of the client as an open that failed, never as one still loading. | this spec section 2; `SYNC-154` | `ambre_sync::websocket::tests::a_first_replay_given_up_fails_the_open` -- six stops of a first replay give one failed open and no generic error, and the document is unsubscribed and let go | none | verified |
| `SYNC-160` | `POST` | Given a resubscription the relay refuses for want of access, the worker MUST close the document and report its user removed from it. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-047` | `ambre_sync::websocket::tests::a_refused_resubscription_closes_its_document` -- the forbidden code on a document held closes it and reports the removal, where another code or a document not held stays a refusal | none | verified |
| `SYNC-167` | `POST` | Given a held document the relay refuses as not found, the worker MUST drop it and report it gone. | this spec section 2; `5-Murmure/features/ambre-spec-murmure-segments.md`, `MURMURE-483` | `ambre_sync::websocket::tests::a_not_found_refusal_for_a_held_document_reports_it_gone` -- the document held is dropped and reported gone, where a document not held stays a refusal | none | verified |
| `SYNC-158` | `POST` | A stop that names a later delta than the stop before it MUST start its document's budget again. | this spec section 2; `SYNC-154`; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_replay_that_moves_on_keeps_its_budget` -- twelve stops each past the last count one apiece, a stop at or before the last still counts, and a timer armed before a reset asks nothing | none | verified |

### 1.4 A request is made

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-062` | `POST` | A request refused for an expired token MUST be retried once, after a silent renewal. | `ambre-spec-session.md`, `SRV-025` | `ambre_sync::http::tests::a_refused_token_is_renewed_and_the_request_replayed_once` -- the replay carries the renewed token; `ambre_sync::http::tests::a_request_is_replayed_at_most_once` -- a second refusal or a refused renewal ends it | none | verified |
| `SYNC-071` | `FAIL` | A request that receives no answer MUST be abandoned, never awaited without end. | audit `M9`; this spec section 2 | `ambre_sync::http::tests::a_request_with_no_answer_is_abandoned_at_its_deadline` -- a transport that never answers ends the request with an error at its thirty-second deadline, sent once | none | verified |
| `SYNC-152` | `POST` | An answer naming a word of a closed set this build does not know MUST reach the rest of the client as an update to install, never as a generic error. | this spec section 2; `ambre-spec-server.md`, `SRV-180` | `ambre_core::serde_enums::tests::an_unknown_word_reads_as_an_unknown_variant_in_every_set` -- each adapter refuses the word as a derived enum does; `ambre_sync::http::tests::only_an_unknown_word_reads_as_one` -- a role or a status no member carries reads as one, a malformed identifier or a missing field does not; `ambre_sync::setup_tests::an_answer_with_an_unknown_word_reaches_logic_as_an_update` -- the sink turns it into the update, under either error variant; `ambre_logic::dispatch::tests::a_newer_relay_reaches_the_ui_as_an_update` -- the logic thread relays it | none | verified |
| `SYNC-179` | `POST` | A request MUST carry each value of its query string percent-encoded whole, so the relay reads the text it was. | this spec section 2 | `ambre_sync::http::tests::a_query_value_is_percent_encoded_whole` -- an ampersand, an equals sign, a hash, a plus, a percent sign, a space, an at sign and a non-ASCII letter each reach the address encoded | none | verified |
| `SYNC-292` | `FRAME` | A log line of the Sync worker MUST NOT carry the text of a user search, nor the query string of a relay route. | this spec section 2; `SYNC-179`; `ambre-spec-zero-knowledge.md`, `ZK-001` | `ambre_sync::members::tests::a_user_search_logs_no_searched_address` -- a search with no relay logs its step and no part of the searched address; `ambre_sync::http::tests::an_error_on_a_search_shows_no_searched_address` -- the error of a search request and its shown target name the route, and no part of the address, encoded or not | none | verified |
| `SYNC-183` | `POST` | The worker MUST renew the token pair before the lifetime the relay announced for its access token runs out, by the margin of `ambre-spec-budgets.md section 8`. | this spec section 2; `ambre-spec-session.md`, `SRV-025` | `ambre_sync::renewal::tests::the_renewal_falls_due_the_margin_before_the_announced_expiry` -- the renewal falls due the margin before the lifetime, on the injected clock, and runs once; `ambre_sync::renewal::tests::a_stored_pair_is_renewed_ahead_of_its_announced_expiry` -- the worker reads the lifetime from the relay's pair, renews it at the due instant and not before, and the renewed pair arms the next renewal; `ambre_sync::http::tests::a_renewal_ahead_of_the_expiry_spares_the_refusal` -- the renewal stores the new pair and its lifetime, and the next request carries the new token with no refusal | none | verified |
| `SYNC-184` | `FRAME` | A renewal MUST NOT run while another renewal of the same pair is in flight, nor after that renewal replaced the pair. | this spec section 2; `ambre-spec-session.md`, `SRV-009` | `ambre_sync::http::tests::two_refusals_of_one_token_renew_the_pair_once` -- two requests refused for one token over a transport that interleaves them send one renewal, and both replay under the new token; `ambre_sync::renewal::tests::only_the_timer_of_the_latest_pair_renews` -- the timer of a replaced pair renews nothing | none | verified |
| `SYNC-185` | `FAIL` | A renewal ahead of the expiry that fails MUST leave the pair in place, so the refusal of `SYNC-062` and `SYNC-063` renews it. | this spec section 2; `SYNC-062`; `SYNC-063` | `ambre_sync::http::tests::a_failed_renewal_ahead_falls_back_to_the_refusal` -- a refused renewal keeps the pair, and the refusal that follows renews it and replays the request; `ambre_sync::renewal::tests::a_short_lifetime_or_a_cancel_arms_nothing` -- a lifetime no longer than the margin, or a sign-out, arms no renewal | none | verified |
| `SYNC-099` | `FRAME` | A renewal MUST NOT replay a request refused for anything but an expired token. | `ambre-spec-session.md`, `SRV-025` | `ambre_sync::http::tests::each_answer_maps_to_its_error` -- a 403, a 409, a 500, a 404 and no answer each send one request, with no renewal | none | verified |
| `SYNC-181` | `POST` | A listed document MUST reach the rest of the client with the update time the relay sent, unchanged and on the relay's clock. | this spec section 2; `ambre-spec-server.md`, `SRV-243` | `ambre_sync::http::tests::a_listed_document_reads_its_update_time_unchanged` -- the number the relay test pins reads as the same instant, and a time sent as text is refused; type-level -- the list entry and the summary hold an `Instant<Relayed>` | none | verified |
| `SYNC-269` | `POST` | The Sync worker MUST move each read cursor with one request per document and per cursor period at most, at the latest number asked. | `5-Murmure/features/ambre-spec-murmure-unread.md` section 3; `ambre-spec-budgets.md section 8` | `ambre_sync::cursors::tests::the_moves_of_one_period_leave_once_per_document_at_the_latest_number`; `ambre_sync::cursors::tests::a_move_back_replaces_a_move_forward_of_the_same_period` -- sixty moves of one period leave as one request per document, and the last move wins, back as well as forward; `ambre_sync::channels::tests::a_move_of_the_read_cursors_waits_for_its_period` -- through the Sync worker, with a relay | none | verified |
| `SYNC-186` | `FAIL` | Given a request and no relay to take it, the worker MUST answer the Logic thread at once, so no view waits on it. | this spec section 2 | `ambre_sync::channels::tests::a_channel_document_request_with_no_relay_answers_its_failure_at_once` -- a segment close, an amendments link and a thread link each answer their own failure, naming the missing relay; `::with_no_relay_the_open_fails_at_once` -- an open answers its failure; `::with_no_relay_a_direct_message_fails_at_once` -- a direct message answers its failure; `ambre_sync::fold::tests::a_fold_with_no_relay_answers_its_failure_at_once` -- a fold lease request and a fold each answer the fold's failure; `ambre_sync::finalize::tests::a_finalize_lease_with_no_relay_answers_its_failure_at_once` -- a finalize lease request answers its failure; `ambre_sync::members::tests::a_member_request_with_no_relay_answers_at_once` -- each member request answers once; `ambre_sync::media::tests::a_media_request_with_no_relay_answers_at_once` -- a download and an erase each answer once | none | verified |
| `SYNC-187` | `FAIL` | Given a public channel whose space is not open, or not on the relay with no creation of it on the way, the worker MUST refuse its creation before any request leaves. | this spec section 2; `SYNC-170` | `ambre_sync::channels::tests::a_public_channel_needs_its_space_on_the_relay_and_a_private_one_does_not` -- a public channel fails for a space not open and for a space held locally alone, and a private one never reads its space | none | verified |
| `SYNC-189` | `FAIL` | Given a public channel created while its new space is on its way to the relay, the worker MUST hold the creation until the relay holds the space, and fail it when the space never lands. | this spec section 2; `SYNC-187` | `ambre_sync::channels::tests::a_public_creation_waits_for_its_space_to_reach_the_relay` -- the creation waits, runs once the space lands, and fails when its creation fails | none | verified |

### 1.5 Content leaves for the relay

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-067` | `POST` | A document name MUST reach the relay sealed under the document key. | `ZK-001`; this spec section 2 | `ambre_sync::documents::tests::a_document_name_reaches_the_relay_sealed_under_the_document_key` -- the one seal create and rename send carries no plaintext, names the document key, and opens under it alone. `CHROME-036` holds what a card shows | none | verified |
| `SYNC-068` | `FRAME` | A sealed document name MUST NOT open under any document but its own. | `ambre-spec-sealed-envelope.md`, `SYNC-018` | `ambre_sync::pipeline::tests::a_name_opens_under_its_own_document_only` -- a name sealed for one document refuses to open under another holding the same key, because its additional data holds the document id | none | verified |
| `SYNC-178` | `FAIL` | Given a document or channel name longer than its budget, the client MUST refuse it before it is sealed, and a rename MUST tell the user why. | `ambre-spec-budgets.md section 6`; `SRV-230` | `ambre_sync::documents::tests::a_name_past_its_budget_is_not_sealed` -- a name one character past the budget is refused, the longest one seals within what the relay reads, and an empty one is the overhead alone; `ambre_app::landing::tests::a_rename_past_the_cap_is_refused_with_a_toast` -- the rename sends nothing, shows the toast and keeps the field open, then goes once shortened | none | verified |
| `SYNC-180` | `FAIL` | Given a direct message of more members than its budget, the client MUST refuse it before any request leaves. | `ambre-spec-budgets.md section 6`; `SRV-240` | `ambre_sync::channels::tests::a_direct_message_past_its_budget_is_refused_before_any_request` -- the budget refuses the request, and one member fewer reaches the relay check | none | verified |
| `SYNC-182` | `FRAME` | The metadata of an uploaded snapshot MUST NOT carry a snapshot id of the client's own, nor a field the relay does not read. | this spec section 2; `ambre-spec-server.md`, `SRV-247` | `ambre_sync::updates::tests::an_uploaded_snapshot_is_confirmed_by_its_author_once_stored` -- the body sent holds the author key, the object key and the horizon alone, and the confirmation names the id the relay answered | none | verified |
| `SYNC-136` | `FAIL` | A create whose answer names another document than the one the client minted MUST fail. | this spec section 2; `SYNC-068` | `ambre_sync::documents::tests::a_create_fails_on_a_foreign_answer` -- a foreign id and a malformed one each answer an error, and only the minted id is reported | none | verified |
| `SYNC-069` | `FAIL` | A channel message sent with no open session of its document MUST be refused aloud, never dropped in silence. | this spec section 2 | `ambre_sync::updates::tests::an_update_for_a_document_with_no_session_is_refused_aloud` -- the update answers the Logic thread with an `UpdateRefused` warning, and reaches no socket | none | verified |
| `SYNC-149` | `FAIL` | A channel message sent while the socket is down MUST reach the relay once it is back, or be refused aloud. | this spec section 2; `SYNC-069` | not built | unbuilt -- lands with the `ARCH P4` -- offline durability Story (`ambre-roadmap.md` E6) | unimplemented |
| `SYNC-155` | `POST` | Given a delta the relay refused by name, the worker MUST drop it from the unconfirmed log and report its document as holding an edit the relay lacks. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-185` | `ambre_sync::updates::tests::a_delta_refused_by_name_leaves_the_log_and_is_reported_once` -- the refused delta and the one behind it leave the log, one warning names the document, and it no longer reads as settled | none | verified |
| `SYNC-156` | `FAIL` | After a delta refused by name, for any cause but a locked segment, the worker MUST send no further delta of its document before that document reopens. | this spec section 2; `SYNC-155`; `ambre-spec-delta-order.md` | `ambre_sync::updates::tests::a_document_with_a_refused_delta_sends_no_further_delta` -- neither a new edit nor a delta a refused clock resends leaves, and the document opened afresh sends again | none | verified |
| `SYNC-159` | `POST` | Given a delta the relay failed to sequence through a fault of its own, the worker MUST send it again, with every delta that left after it, within the resend budget. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-186`; `SYNC-155`; `ambre-spec-budgets.md` section 8 | `ambre_sync::updates::tests::a_delta_the_relay_failed_to_sequence_leaves_again_with_every_later_one` -- once its timer fires, the answer leaves first, then the delta at its own clock with the one behind it, and a stale timer sends nothing; `ambre_sync::updates::tests::a_delta_failing_past_the_budget_holds_its_document` -- the delay doubles per failure, and past the budget the document holds the delta and reports it once | none | verified |
| `SYNC-162` | `POST` | A segment a closed channel watches MUST be counted from its first number, under the key of its `meta`. | `5-Murmure/features/ambre-spec-murmure-unread.md`, `MURMURE-477`; this spec section 2 | `ambre_sync::light::tests::a_watched_segment_counts_from_its_first_number` -- the watch subscribes the segment and asks for everything after number zero, a delta sealed under the `meta` key is counted, and a second watch sends nothing | none | verified |
| `SYNC-163` | `FAIL` | A close that lost its race MUST delete the successor it created, and a close that failed MUST delete nothing. | `ambre-spec-sequencer.md`, `SRV-195`, `SRV-196` | `ambre_sync::sharing::tests::a_lost_close_deletes_the_successor_it_created` -- a won close creates then closes, a `created: false` answer deletes the minted successor, and a failed close sends no delete | none | verified |
| `SYNC-164` | `FAIL` | Given a close that failed, the client MUST ask it again with the successor it already created, so no successor stays unregistered. | `SYNC-163`; `ambre-spec-sequencer.md`, `SRV-198` | `ambre_logic::channels::tests::a_failed_close_is_asked_again` -- the next send asks the same close into the same successor, and the answer makes it active; `ambre_sync::sharing::tests::a_close_asked_again_keeps_its_successor` -- the worker creates and closes into that successor again, and deletes nothing | none | verified |
| `SYNC-165` | `POST` | Given a delta the relay refused for a locked segment, the worker MUST hand it and every later delta of its document to the Logic thread, and hold none. | this spec section 2; `ambre-spec-sequencer.md`, `SRV-115`; `SYNC-156` | `ambre_sync::updates::tests::a_delta_refused_for_the_lock_returns_to_logic_with_every_later_one` -- both deltas return once, in order, and the next delta leaves | none | verified |
| `SYNC-166` | `FAIL` | A registration of amendments that lost its race MUST delete the document it created, and a failed one MUST delete nothing. | `ambre-spec-sequencer.md`, `SRV-201`, `SRV-202`; `SYNC-163` | `ambre_sync::sharing::tests::a_lost_registration_deletes_the_amendments_it_created` -- a won registration creates then registers, a `created: false` answer deletes the minted document and names the winner, and a failed one sends no delete | none | verified |
| `SYNC-169` | `FAIL` | A thread registration that lost its race MUST delete the document it created, and a failed one MUST delete nothing. | `ambre-spec-sequencer.md`, `SRV-213`, `SRV-214`; `SYNC-166` | `ambre_sync::sharing::tests::a_lost_registration_deletes_the_thread_it_created` -- a won registration creates then registers, a `created: false` answer deletes the minted document and names the winner, and a failed one sends no delete | none | verified |
| `SYNC-170` | `POST` | The lazy creation of a document MUST mark it a space when, and only when, the client made it as a new space. | `ambre-spec-server.md`, `SRV-216` | `ambre_sync::channels::tests::a_new_space_is_created_as_a_space` -- the space's creation is marked, another document's is not, a space closed before its upload is forgotten, and a materialized one leaves no trace | none | verified |
| `SYNC-171` | `POST` | The worker MUST open each listed channel name under the user's key of its `meta`, and drop one that opens empty or fails. | `ambre-spec-server.md`, `SRV-219`; `SYNC-067` | `ambre_sync::channels::tests::a_listed_name_opens_under_the_users_key_of_its_meta` -- a name opens, one sealed for another meta, an empty one and a meta never named give none; `::a_channel_name_is_sealed_under_the_channel_key_for_its_meta` -- the seal holds no plaintext, opens for its meta alone, and a locked session seals nothing; `::a_listing_of_the_users_channels_forwards_their_places` -- the names reach the Logic thread with the latest listing | none | verified |
| `SYNC-192` | `FRAME` | A document MUST send no delta inside the send window the send before it opened. | this spec section 2, the send window; `ambre-spec-budgets.md section 8` | `ambre_logic::send_window::tests::a_burst_leaves_once_per_window`; `::keys_farther_apart_than_the_window_leave_one_by_one`; `ambre_logic::window_tests::a_burst_of_passes_sends_once_per_window` | none | verified |
| `SYNC-193` | `POST` | A delta the send window holds MUST leave when that window ends, and no later. | this spec section 2, the send window; README pillar "Performance"; `ambre-spec-budgets.md section 8` | `ambre_logic::send_window::tests::a_lone_key_leaves_at_once`; `::no_key_waits_past_the_window`; `ambre_logic::window_tests::a_held_delta_leaves_with_no_message` | none | verified |
| `SYNC-194` | `POST` | The deltas a send window releases together MUST leave as one delta when they carry one run of one peer's operations. | this spec section 2, the send window | `ambre_logic::send_window::tests::the_deltas_of_a_window_merge_into_one`; `::deltas_that_are_no_single_run_leave_apart`; `ambre_logic::window_tests::the_keys_of_one_pass_leave_as_one_delta` | none | verified |
| `SYNC-195` | `POST` | A document that leaves memory, or a session that ends, MUST send every delta its window holds before the close or the sign-out. | this spec section 2, the send window; `SYNC-066` | `ambre_logic::send_window::tests::a_flush_takes_every_held_delta`; `ambre_logic::window_tests::a_close_sends_the_held_delta_first`; `::a_sign_out_sends_the_held_delta_first` | none | verified |
| `SYNC-196` | `POST` | A delta the send window holds MUST reach local persistence in the pass that wrote it. | this spec section 2, the send window; `ambre-spec-op-authorship.md`, `AUTHOR-028` | `ambre_logic::window_tests::a_held_delta_is_saved_in_its_own_pass` | none | verified |

### 1.6 The session ends

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-066` | `POST` | A sign-out MUST close the socket and clear every key. | `ambre-spec-zero-knowledge.md`, `ZK-002` | `ambre_sync::websocket::tests::a_sign_out_closes_the_socket_and_clears_every_key` -- the transport is disconnected, the seed, both keys, the tokens and every document are gone, and a later drop reopens nothing | none | verified |
| `SYNC-100` | `FRAME` | A sign-out MUST leave no token behind it. | `ZK-006`; `ambre-spec-session.md`, `SRV-026` | `ambre_sync::websocket::tests::session_tokens_wipe_on_drop` -- both tokens are held in an `ambre_core::Secret`, wiped when the last owner drops it, so ending the session clears them | none | verified |
| `SYNC-177` | `POST` | A sign-out MUST ask the relay to revoke the refresh token the session holds. | `ambre-spec-session.md`, `SRV-026` | `ambre_sync::http::tests::a_sign_out_revokes_the_refresh_token_it_holds` -- one request under the access token carries the refresh token, a refused access token makes it carry the token the renewal handed out, and a refusal is reported | none | verified |
| `SYNC-275` | `POST` | A sign-out MUST drop every key, root, pin, log and buffered delta of the session. | this spec section 2; `SYNC-235`, `SYNC-237` | `ambre_sync::run_tests::a_sign_out_leaves_nothing_of_the_session_in_the_worker` -- the roots, the pins, the listed channel keys, the typed opens and the plaintext deltas buffered before an open are gone | none | verified |
| `SYNC-281` | `FAIL` | Given a request the session spawned before its sign-out, its answer MUST NOT reach the session that follows. | this spec section 2; `SYNC-275` | `ambre_sync::run_tests::a_sign_out_leaves_nothing_of_the_session_in_the_worker` -- a sender a task of the ended session holds reaches the queue of that session alone, and the next session reads another queue | none | verified |
| `SYNC-277` | `FAIL` | Given an answer body or a socket frame past its ceiling in `ambre-spec-budgets.md`, the Sync worker MUST refuse it. | this spec section 2; `ambre-spec-budgets.md` section 1 | `ambre_sync::http::tests::a_body_meter_refuses_the_chunk_past_its_ceiling` -- the meter takes bytes up to the ceiling and refuses the chunk past it; `ambre_sync::http::tests::an_answer_past_its_ceiling_is_no_answer` -- a relay answer past its ceiling is no answer, and an object request holds the larger ceiling; `ambre_sync::websocket::tests::a_socket_frame_past_its_ceiling_is_not_read` -- a frame past its ceiling is not read | none | verified |
| `SYNC-278` | `FAIL` | Given a body whose stated length or whose bytes read so far pass its ceiling, the Sync worker MUST stop reading it. | this spec section 2; `SYNC-277` | `ambre_sync::tests::metered_fetch::a_body_past_its_ceiling_stops_at_the_chunk_that_passes_it` -- wasm tier: a body that never ends stops at the chunk past its ceiling, and the stream is cancelled; `ambre_sync::tests::metered_fetch::a_stated_length_past_its_ceiling_reads_no_byte` -- a stated length past the ceiling pulls no chunk; `ambre_sync::tests::metered_fetch::a_body_up_to_its_ceiling_reads_whole` -- a body up to its ceiling reads whole | none | verified |

### 1.7 An account is recovered

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-248` | `POST` | A recovery MUST store the Master Seed sealed under the new passphrase, so that a later unlock under that passphrase opens the account. | `ambre-spec-zero-knowledge.md section 2`, the re-seal of a recovery; `ambre-spec-zero-knowledge.md`, `ZK-035`, `ZK-043` | `ambre_sync::auth::tests::a_recovery_re_seals_the_seed_that_a_later_unlock_opens` -- the one write goes to the re-seal route, and the open of the unlock reads it under the new passphrase to the signing key of the words, and refuses another passphrase | none | verified |
| `SYNC-249` | `FAIL` | Given recovery words whose seed does not derive the signing key the relay holds for the account, a recovery MUST refuse them and write nothing. | `ambre-spec-zero-knowledge.md section 2`, the re-seal of a recovery; `ambre-spec-session.md`, `SRV-030` | `ambre_sync::auth::tests::recovery_words_of_another_account_write_nothing` -- the words of another seed, and an account that holds no key, each draw `AuthFailure::ForeignRecoveryWords` after the one read, and no write leaves | none | verified |
| `SYNC-250` | `FRAME` | A recovery MUST NOT send the relay the Master Seed, the recovery words or a passphrase. | `ambre-spec-zero-knowledge.md`, `ZK-003`; `ambre-spec-zero-knowledge.md section 2`, the re-seal of a recovery | type-level -- the re-seal body `MasterRotation` holds a `SealedSeed`, the `FixedBytes<MasterKeySalt>` of the replaced seal and a `FixedBytes<Signature>` alone, and `SealedSeed` holds an `Encoded<WrappedMasterKey>` and a `FixedBytes<MasterKeySalt>` alone, so no other value is representable in the write; `ambre_sync::auth::tests::a_recovery_re_seals_the_seed_that_a_later_unlock_opens` -- the body of the write names those fields, and no other | none | verified |
| `SYNC-293` | `POST` | A recovery the relay answered MUST end on the session of the recovered keys, with no second unlock. | `ambre-spec-zero-knowledge.md section 2`, the re-seal of a recovery; `SYNC-248`; `SYNC-261`; `ambre-spec-launcher.md`, `CHROME-133` | `ambre_sync::auth::tests::a_recovery_reads_the_identity_its_session_opens_with` -- after the re-seal, the identity is read under the held pair, and a refused re-seal reads none; `ambre_sync::auth::set_up_keys_tests::a_recovery_ends_on_the_session_of_the_recovered_keys` -- the worker holds the recovered keys, opens one socket, and Logic hears `AccountRecovered` then `SessionReady`; `ambre_belt::account::tests::the_unlock_face_leads_to_a_recovery_that_opens_the_account` -- browser tier, the list shows after the recovery with no second unlock | none | verified |
| `SYNC-294` | `FRAME` | Given a session that holds keys already, a recovery MUST keep its keys and its socket. | `SYNC-287`; `ambre-spec-launcher.md`, `CHROME-135` | `ambre_sync::auth::set_up_keys_tests::a_recovery_ends_on_the_session_of_the_recovered_keys` -- a second recovery over the session leaves the held signing key and the one socket, and Logic hears `AccountRecovered` alone | none | verified |

### 1.8 The passphrase changes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-251` | `POST` | A passphrase change MUST store the Master Seed sealed under the new passphrase, so that a later unlock opens the account under the new passphrase and not under the current one. | `ambre-spec-zero-knowledge.md section 2`, the re-seal of a passphrase change; `ambre-spec-zero-knowledge.md`, `ZK-042`; `SYNC-248` | `ambre_sync::auth::tests::a_passphrase_change_re_seals_the_seed_that_a_later_unlock_opens` -- the one write goes to the re-seal route under a fresh salt, and the open of the unlock reads it under the new passphrase to the same signing key, and refuses the current one | none | verified |
| `SYNC-252` | `FAIL` | Given a current passphrase that does not open the stored Master Seed, a passphrase change MUST refuse it and write nothing. | `ambre-spec-zero-knowledge.md section 2`, the re-seal of a passphrase change; `SYNC-251` | `ambre_sync::auth::tests::a_wrong_current_passphrase_writes_nothing` -- a wrong current passphrase draws `AuthFailure::BadPassphrase`, and an account that owes its setup draws `AuthFailure::CorruptAccount`, each after the one read, and no write leaves | none | verified |
| `SYNC-253` | `FRAME` | A passphrase change MUST NOT send the relay the Master Seed or a passphrase. | `ambre-spec-zero-knowledge.md`, `ZK-003`; `SYNC-250` | type-level -- the change writes the same `MasterRotation` body as a recovery, which holds a sealed seed, two salts and a signature alone; `ambre_sync::auth::tests::a_passphrase_change_re_seals_the_seed_that_a_later_unlock_opens` -- the body of the write names those fields, and no other | none | verified |
| `SYNC-254` | `POST` | A recovery and a passphrase change MUST each sign their re-seal with the signing key of the seed, over the new sealed seed and the salt of the seal they read. | `ambre-spec-session.md`, `SRV-278`, `SRV-279`; `ambre-spec-zero-knowledge.md section 2`, the re-seal of a recovery | `ambre_sync::auth::tests::a_recovery_re_seals_the_seed_that_a_later_unlock_opens`; `ambre_sync::auth::tests::a_passphrase_change_re_seals_the_seed_that_a_later_unlock_opens` -- the write names the salt of the material the read answered, and its signature verifies under the signing key of the seed over the transcript the relay rebuilds, and under no other key | none | verified |
| `SYNC-295` | `FAIL` | Given a re-seal the relay refuses as stale, a recovery or a passphrase change MUST answer a refusal of its own, never a generic failure of the request. | `ambre-spec-session.md`, `SRV-279`; `SYNC-254`; `ambre-spec-message-bus.md`, `BUS-046` | `ambre_sync::auth::tests::a_stale_re_seal_is_refused_as_stale` -- a `CONFLICT` on the write of a recovery and of a change each draw `AuthFailure::StaleReseal`, whose words ask for a new try and blame no passphrase, and another refusal of the write stays an HTTP failure; `ambre_app::account::tests::the_passphrase_change_sends_both_passphrases_and_shows_each_refusal` -- wasm tier, the form reads the words | none | verified |

### 1.9 The keys of an account are set up

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-255` | `POST` | A key setup of a signed-in account MUST store a new Master Seed sealed under its passphrase on that account, so that a later unlock under that passphrase opens it. | `ambre-spec-zero-knowledge.md section 2`; `ambre-spec-session.md`, `SRV-030`; `ambre-spec-launcher.md`, `CHROME-098` | `ambre_sync::auth::tests::a_key_setup_stores_a_seed_that_a_later_unlock_opens` -- one write under the held pair, then the identity read under the pair the relay bound to the new key, and the unlock path opens the stored seed; `ambre_sync::auth::tests::a_refused_key_store_reads_no_identity`; `ambre_sync::auth::set_up_keys_tests::stored_keys_answer_the_words_then_the_session` -- the worker holds the keys, opens the socket the setup owed, and answers the recovery words before the session | none | verified |
| `SYNC-256` | `FRAME` | A key setup MUST NOT send the relay the Master Seed, the recovery words or a passphrase. | `ambre-spec-zero-knowledge.md`, `ZK-003`; `SYNC-250` | type-level -- the store body `StoreMasterKeysRequest` holds an `Encoded<WrappedMasterKey>`, a `FixedBytes<MasterKeySalt>`, the two public keys and the signature of the signing key alone; `ambre_sync::auth::tests::a_key_setup_stores_a_seed_that_a_later_unlock_opens` -- the body of the write names those five fields, and no other | none | verified |
| `SYNC-261` | `POST` | A key setup the relay answered MUST hold the new keys, then announce the session of the user the relay names, after the recovery words. | `ambre-spec-zero-knowledge.md section 2`; `ambre-spec-session.md`, `SRV-319`; `ambre-spec-launcher.md`, `CHROME-132` | `ambre_sync::auth::set_up_keys_tests::stored_keys_answer_the_words_then_the_session` -- the worker holds the keys and the user the identity read named, and tells Logic the words, then the session of that user; `ambre_belt::account::tests::a_setup_ends_in_the_session_once_the_words_are_confirmed` -- browser tier, the header names the new user under the words | none | verified |
| `SYNC-296` | `FAIL` | Given a first key store the relay refuses because the account holds keys already, the worker MUST answer with the keys the relay holds when the typed passphrase opens them, and MUST NOT announce a second seed. | `ambre-spec-session.md`, `SRV-030`, `SRV-320`; `SYNC-255` | `ambre_sync::auth::tests::a_second_key_store_answers_the_keys_the_relay_holds` -- after the conflict, one read of the master material, one renewal and one identity read, and the seed answered is the stored one; `ambre_sync::auth::tests::a_second_key_store_under_another_passphrase_opens_nothing` -- a passphrase that does not open the stored seed fails as a wrong passphrase with no renewal, and an account that still owes its keys fails with the refusal | none | verified |
| `SYNC-270` | `FRAME` | An unlock MUST only open the keys the relay stores. It MUST NOT create a key or write one to the relay: a key setup alone creates them. | `ambre-spec-zero-knowledge.md section 2`; `SYNC-134`; `SYNC-255`; `ambre-spec-launcher.md`, `CHROME-098` | `ambre_sync::auth::tests::an_unlock_opens_stored_keys_and_creates_none` -- stored keys open after two reads, and an account that owes its keys ends as a setup owed after one read, with no write; `ambre_sync::auth::provider_tests::an_unlock_with_no_relay_creates_no_key` -- with no relay, the unlock fails and holds no key | none | verified |

* **Budget.** the relay hop of `ambre-spec-budgets.md section 1`, one encode plus one transfer
  under 1 ms, the two per-session holders of `section 6`, and the send window of
  `ambre-spec-budgets.md section 8`. The socket carries no budget of its own: the network
  dominates the inter-user target.
* **Boundary.** this worker **is** the client side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SYNC-002` and `SYNC-095` keep the keys in. `SYNC-060` and `SYNC-061` keep the tokens in,
  and `SYNC-067` seals the one document field that is content rather than routing.
* **Verification.** 88 `verified`, 1 `unimplemented`. The crate graph holds the keys and the CRDT
  apart. Unit tests hold the token redaction and wipe, the refused channel send and the name bound
  to its document. A gate refuses browser storage in the worker. Each gap left is a browser property
  the wasm tier cannot boot, `ambre-spec-testing.md section 9`.
---

## 2. MODEL

**CRDT containers.** none. This worker compiles no CRDT at all (`SYNC-011`). What it carries across
the boundary is opaque bytes in one direction and sealed envelopes in the other.

**Where it runs.** Two instances, two heaps. The rest of the client -- the screen and the document
logic -- shares one. This worker has its own, and the two meet only over posted messages
(`ambre-spec-architecture.md section 2`).

| Instance | Holds | Reaches the other by |
|---|---|---|
| Main | the screen, the document logic, the CRDT | a posted message, encoded |
| Sync | every key, one socket, every request | the same, in reverse |

**Starting up, and the window that swallows the first command.** The worker installs its
inbound handler at the end of an asynchronous start, so anything posted before that instant reaches
no handler at all. The contract is an announcement. As the last step of its start, the worker
posts an empty payload. The relay on the Main side holds every command until it sees one. An
encoded event is never empty, so the announcement is unambiguous (`SYNC-057`).

**The address of the relay.** The relay sits at the origin of the page that started the worker. The
worker reads its host and its protocol from its own location. A page served over TLS reaches the
relay over TLS, on the loopback too, since the phone stack serves `https://localhost:8443`. With no
protocol, a relay on this machine speaks plain HTTP, and any other relay TLS (`SYNC-271`).

**The log.** Every message of the worker goes through one function, which hands it to the browser
itself: one console line, then a performance mark and a measure (`SYNC-305`). No `tracing` layer
stands between, so the worker links neither `tracing` nor a subscriber registry. The line keeps the
shape and the colours of the `tracing-wasm` layer it replaces. On the host, the same function reaches
`tracing`, which the native tests capture.

**The loop.** One non-blocking wait over two queues: the commands relayed in, and the results of
the socket and the requests. Both producers live in this worker, so the wait wakes correctly and the
browser event loop stays free for the socket and the fetches (`SYNC-058`). Network work runs as
spawned tasks on that free loop.

**What it holds.**

| Held | Lifetime | Wiped |
|---|---|---|
| the master seed, and the signing and exchange keys derived from it | from sign-in to sign-out | on drop, by the type (`SYNC-066`) |
| the access and refresh tokens | the same | dropped, and not wiped -- section 5 |
| one socket, and the callbacks attached to it | the same | on sign-out |
| one state per open document: its key, its counters, its cursor and its quarantine | from open to close | on drop |
| the roots, the pins, the listed channel keys, the logs, the footings and the deltas buffered before an open | from sign-in to sign-out | on sign-out (`SYNC-275`) |

**A sign-out ends the session's event queue.** Each task the session spawned holds a sender of
that queue. The worker starts a fresh queue at the sign-out and drops the old one unread. So a task
in flight, an open or a fetched rotation, puts no key back into the next session (`SYNC-281`).

**Every byte the worker reads has a ceiling.** A relay answer, an object of the store and a socket
frame each have one, a row of `ambre-spec-budgets.md` section 1. The worker refuses a body past
it (`SYNC-277`). It stops reading at a stated length past it, or at the chunk that passes it
(`SYNC-278`). It checks a frame before the copy into its memory, which never shrinks.

**Three planes cross the boundary.** They are declared in
`ambre-spec-message-bus.md` and never restated here. A request expects one answer and carries an
identifier to match it. A control and content plane serves documents and channels, and one
lifecycle command stands apart. A request identifier is minted by the screen, echoed by the logic thread, and
echoed once more here.

**The socket.** One per session, multiplexed: opened at sign-in, closed at sign-out, and every
document subscribed on it. A browser socket carries no header of ours, so its token travels as the
first frame and never in the address (`SRV-031`, `SYNC-061`).

| Event | Response |
|---|---|
| the relay refuses the upgrade on authentication | renew the token silently, then reopen (`SYNC-063`) |
| the relay refuses the upgrade for a key setup still owed | neither renew nor reopen until a session with keys is stored, and tell the client a setup is owed (`SYNC-133`, `SYNC-134`) |
| a frame of a newer wire version | close for good, and tell the client an update is owed (`SYNC-137`) |
| the Main side asks for a hold | close, and keep each edit sealed meanwhile; reopen on the release alone (`SYNC-059`) |
| any other close, or a socket error | back off and reopen (`SYNC-064`) |
| the socket opens | resubscribe every document, and pull what was missed (`SYNC-033`) |
| the relay refuses a resubscription for want of access | close the document, and report the user removed from it: the notice went while the socket was down (`SYNC-160`) |
| the relay refuses a held document as not found | drop it, and report it gone: a fold erased it (`SYNC-167`) |
| the relay stops a replay | ask again from the document's cursor after a growing delay, and report it once past the budget (`SYNC-153`, `SYNC-154`) |
| a replay ask inside the relay's window for its document | hold it, and send it once the window passed: the relay answers nothing inside it (`SYNC-188`) |
| the relay refuses a delta for a locked segment | hand it and every later one to the Logic thread, and hold nothing (`SYNC-165`) |
| a wire message arrives | reset the delay: only a wire message proves the connection is live (`SYNC-064`) |

**Why a bare open is not proof.** A refused upgrade still completes, then closes immediately. A
delay reset on the open alone would reset on every refusal. The reconnection would then become a
tight loop against a relay that rate-limits it.

**The hold.** A browser socket cannot pause its reads. So past a high mark of events waiting for
the logic thread, the Main side asks for a hold. This worker then closes the socket. The relay keeps
every delta, so nothing is lost. At the low mark the Main side releases it, and the reopen pulls
from each document's last sequence.

Presence sent meanwhile is lost, which costs a cursor for a moment. An edit sealed during the hold
is kept and sent after the reopen (`SYNC-059`). The two marks are a row of
`ambre-spec-budgets.md section 6`.

**Requests.** Every one carries the access token. A refusal for an expired token is renewed once and
replayed; a second refusal reaches the screen as an authentication error (`SYNC-062`).

**The renewal ahead of the expiry.** The relay announces how long each access token lives. The
worker keeps that lifetime with the pair, and arms one timer on its own clock. The timer falls due
a margin before the expiry, a row of `ambre-spec-budgets.md section 8` (`SYNC-183`). The worker
reads the announced lifetime and never decodes the token: the token stays opaque to the client.

| Event | Response |
|---|---|
| a pair is stored, or a renewal replaces it | arm the renewal of that pair; the timer of the pair before goes stale |
| an unlock hands back the pair the store holds | keep the renewal armed: the lifetime did not start again |
| the timer falls due | renew the pair once, then arm the renewal of the new pair |
| a second renewal is asked while one is in flight | wait for it, then send nothing if it replaced the pair (`SYNC-184`) |
| the renewal fails | keep the pair: the next refusal renews it (`SYNC-185`) |
| the session ends | disarm, and drop an answer that lands after it (`SYNC-100`) |

**Why one renewal per pair.** The relay honours each refresh token once (`SRV-009`). A second
renewal of one pair would carry a spent token, and its refusal would fail a request that a single
renewal serves. A refusal
for a key setup still owed reaches it as the setup to complete. So does an unlock that finds no
master material (`SYNC-134`). An unlock never creates the keys it did not find (`SYNC-270`).

**A query string.** Each value of a query string is percent-encoded whole. A search
text holding `&` or `#` then never ends the value or opens a fragment (`SYNC-179`).

**A word this build does not know.** Each closed set on the JSON wire is read strictly, so a new
role or status refuses the whole answer (`SRV-180`). Only a newer relay writes such a word, and no
retry cures it. The answer therefore reaches the screen as an update to install (`SYNC-152`). A
relay that adds a word must ship after the client that reads it.

**The document register.** The list, the create, the rename and the delete are ordinary requests,
and one field of them is content rather than routing: the name.

| Field | Form on the relay |
|---|---|
| the identity, the kind, the times, the member count | plaintext routing metadata |
| the name | sealed under the document key, signed, and bound to the document (`SYNC-067`, `SYNC-068`) |
| the document key | sealed once per member (`ambre-spec-membership.md`) |

A document with no name stores nothing rather than a placeholder (`CHROME-037`). The list opens
each name with the key it unwraps from the same row. The client mints the document id on create, so
the name is bound to it before the relay sees either.

**The send window.** A key typed alone leaves as one delta, and each delta costs the relay an
index row, an ordering script and a broadcast. The window groups the deltas of one document. A
delta offered with no window open leaves at once, and opens a window. A delta offered while one is
open waits for its end. At that end every delta held leaves as one, and that send opens the next
window (`SYNC-192`, `SYNC-193`).

| Event | Response |
|---|---|
| a pass writes deltas, no window open | merge them, send them, open a window |
| a pass writes deltas inside a window | hold them |
| the window ends holding deltas | merge them, send them, open the next window; the thread wakes for it with no message |
| the document leaves memory, or the user signs out | send what the window holds first (`SYNC-195`) |
| a delta is written | hand it to local persistence in its own pass, whatever the window (`SYNC-196`) |

The merge reads back from the document the one run of operations the held deltas carry, and
exports it as one delta (`SYNC-194`). Any other shape leaves the deltas apart and in order.

**Why the window lives in the Logic thread.** The merge needs the document, and this worker
compiles no CRDT (`SYNC-011`). A window here could only frame several sealed deltas together. Each
would keep its own overhead, and the relay would need a new frame to store them.

**What the window buys, and what it does not.** The window is 25 ms, the share of the client
budget the rest of the path leaves (`ambre-spec-budgets.md section 8`). At five keys a second the
keys fall 200 ms apart, so the window groups none of them and costs them nothing. It caps a
document at 40 deltas a second. A key held down, the fast runs of a typist and a burst of passes
reach that cap.

Grouping the keys of an average typist takes a window longer than the time
between two keys. That window would take more than the 300 ms budget leaves once the network is
paid.

**Code.**

* `crates/ambre-logic/src/send_window.rs` -- the send window, and the merge of what it releases;
* `crates/ambre-sync/src/worker.rs` -- the start, the announcement and the two posted directions;
* `crates/ambre-sync/src/lib.rs` -- the state, the loop and the command dispatch;
* `crates/ambre-sync/src/websocket.rs` -- the socket, the callbacks, the backoff and the demultiplex;
* `crates/ambre-sync/src/auth.rs` -- sign-in, sign-out and the derived keys;
* `crates/ambre-sync/src/http.rs` -- the request helpers, and the renew-and-replay;
* `crates/ambre-sync/src/documents.rs` -- the register, and the sealed name;
* `crates/ambre-sync/src/channels.rs` -- the channel listing, which names the documents to open;
* `crates/ambre-app/src/platform.rs` -- the Main side of the pair, and the readiness gate.

---

## 3. SURFACE

The worker has no surface of its own. What a user sees of it is five things.

| What the user meets | Comes from |
|---|---|
| a sign-in screen, and a session that survives a page reload | the tokens and the renewal (`SYNC-062`) |
| a connection state per document | the socket's own events (`SYNC-064`) |
| a document that reconnects with no action after a network drop | the backoff and the resubscription (`SYNC-063`, `SYNC-033`) |
| an authentication error asking them to sign in again | the second refusal of a renewal (`SYNC-062`) |
| the key setup, when the account still owes it | the setup refusal and the empty unlock (`SYNC-134`), then the key setup alone (`SYNC-270`) |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One separate instance, reached by posted messages | a thread inside the Main instance sharing its memory | it is what the screen and the document logic still use and what this worker must not: that instance runs to completion, so an asynchronous loop returns after its first step and the runtime collects it -- the freeze this project spent weeks on. An instance with its own event loop cannot have that failure, and it buys key isolation as a side effect: the heap that holds the keys is not addressable from the screen |
| One socket per session, multiplexed | a socket per open document | it costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and every one of them keeps alive on a mobile battery. Every wire message already names its document, so the relay demultiplexes |
| One worker for every document | a Sync worker per open document | it means a copy of the cryptography per document, and the memory that goes with it, which is unacceptable on a mobile device. One worker holds a map |
| The readiness announcement, rather than delaying the first command | delaying the first command | the console being open was enough to hide it. The race is between an asynchronous boot and a command posted from a frame loop, so a delay only narrows it, and the failure it leaves is a document stuck behind a loading overlay for ever. Holding on the sending side makes the delivery independent of timing |
| A refused upgrade has its own close code | one close code for every refusal, and a backoff over all of them | when a token expires or the relay rotates its signing key, every token is invalid at once, so a plain backoff never recovers: each new connection is refused too. A distinct code lets the client renew and reconnect with no action from the user |
| The delay resets on a wire message | resetting when the socket opens | a refused upgrade opens before it closes -- see section 2 -- so a reset on the open turns a refusal into a loop |
| An early edit is held, not dropped | dropping an edit made before the session, and letting the user retype it | the first edit after a reload lands in the milliseconds while sign-in is still in flight. Dropping it lost that edit permanently and produced a clock the relay then refused on the next one |
| A setup refusal is told at the one sink every event leaves by | a check in each request handler | about forty handlers turn a failed request into an error, each in its own words. One that missed the check would show a setup still owed as a generic failure. The sink sees every event, so one mapping covers each (`SYNC-134`) |
| The tokens live in memory alone | browser storage | a token there survives the tab and is readable by any script that reaches the page. Memory dies with the worker, which is the lifetime a session should have |
| The tokens live in memory alone | a cookie | the browser sends it on requests we did not make, which is the ambient credential the deliberate token exists to avoid |
| A hold closes the socket | keeping it open and dropping the deltas that arrive | a rotation notice or a removal arrives on the same socket, and a filter that drops the deltas alone must know every frame that may not be dropped. A closed socket drops nothing: the relay keeps it all, and the reopen asks |
| The first delta leaves at once, and the window holds only what follows it | a window opened by every delta, which holds that delta too | a key typed alone is the common case at five keys a second, and it would pay the whole window for no saving |
| The window ends at a fixed time after the send that opened it | a window that ends when typing pauses | a paragraph typed without a pause would reach no peer before the pause, so the wait has no bound |
| A window of 25 ms | a window longer than the time between two keys, about 200 ms, so an average typist's keys group | a key would wait up to 200 ms, which the 300 ms budget cannot pay once the network takes its share |
| The window merges in the Logic thread | several sealed deltas framed together by this worker | this worker compiles no CRDT (`SYNC-011`), so each delta keeps its overhead, and the relay needs a new frame |
| A hold closes the socket | pausing the read of a stream socket | only one browser engine ships a socket with a paused read, and the worker must run on every engine |

---

## 5. NOT BUILT, AND NOT YET RULED

* **An edit sent while the network is down is lost on the wire.** Only a hold keeps it; any other
  close drops what is sent before the reopen, and nothing sends it again.
* **A name sealed before the envelope opens as untitled.** The old form took the bare cipher, and
  nothing reads it any more.
* **An update sent while the socket is down is dropped.** It is logged, and nothing tells the view.
  A channel message shares the fate of any document update (`SYNC-149`).
* **Meetings are a log line each.** Four commands reach this worker and none does anything.
* **A pending reconnect timer is not cancelled.** A second disconnect replaces the callback the
  timer will call, and the timer itself is not cleared. The first one then fires against a callback
  that does not exist.
* **Reopening a document replaces its whole state.** The quarantine, the cursor, the clock and the
  cached keys go back to their initial values. The relay's view of the clock does not
  (`ambre-spec-delta-order.md`).
* **A sign-out is the only teardown.** Nothing closes the socket on a page hidden or a tab
  suspended, so a background tab keeps a connection and its keepalives.
* **Neither instance is booted in continuous integration.** A real boot needs a shared-memory page, a
  worker loader and a graphics adapter, and the unit-test runner provisions none of the three.
  `ambre-spec-testing.md section 9` records that as deliberate debt.

---

## 6. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a payload is sealed and opened | `ambre-spec-sealed-envelope.md` |
| In what order sealed updates are applied, and what waits | `ambre-spec-delta-order.md` |
| Which key opens a document, who the members are, and what they may do | `ambre-spec-membership.md` |
| What a revoke rotates | `ambre-spec-key-rotation.md` |
| How a file is addressed and transferred | `ambre-spec-media.md` |
| The three planes that cross the boundary, and their shapes | `ambre-spec-message-bus.md` |
| What the relay does with a request or a wire message | `ambre-spec-server.md`, `ambre-spec-broadcast.md` |
| What the relay does with a token, and what mints one | `ambre-spec-session.md` |
| Why this worker is a separate instance rather than a thread | `ambre-spec-architecture.md section 4` |
| What the home screen shows of a document list | `ambre-spec-launcher.md` |
| Which thread may hold document data at all | `ambre-spec-ui.md`, `UI-THREAD-020` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
