# AMBRE -- FEATURE: THE SYNC WORKER

**Type:** Horizontal (framework)
**Implementation:** partial -- the session, the socket and the document register all ship. The
socket has no backpressure, and the access token reaches the console.

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
| `SYNC-095` | `INV` | A key MUST be wiped from memory when it goes out of scope. | `ZK-002`; this spec section 2 | type-level -- both key types derive `ZeroizeOnDrop`, so the wipe is the destructor and cannot be forgotten at a call site. It is what makes a sign-out mean anything on a machine the attacker may later read | none | verified |
| `SYNC-011` | `INV` | The Sync worker MUST compile no CRDT code. | this spec section 2 | crate-graph -- `deny.toml` grants `loro` to the four application crates alone, so this worker fails the `deny` job on the line that would add it | none | verified |
| `SYNC-096` | `INV` | The Sync worker MUST compile no rendering code. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-057` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a path dependency that brings `ambre-canvas`, `ambre-draw` or `wgpu` into the closure of `ambre-sync`; cargo-deny -- `deny.toml` grants `wgpu` to `ambre-canvas` and `ambre-app`, and the CI `deny` job fails any other crate that takes it | none | verified |
| `SYNC-056` | `INV` | The Sync worker MUST run in its own instance, with a linear memory the rest of the client cannot address. | `ambre-spec-architecture.md section 2` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses an `ambre-sync` that builds no `cdylib`, a crate that depends on it, and a page that does not load it as a worker | none | verified |
| `SYNC-060` | `INV` | A token MUST live in this worker's memory alone: never in browser storage, never in a cookie. | `ambre-spec-session.md`, `SRV-031` | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a line of `ambre-sync` that names browser storage, the browser database, the cache or a cookie | none | verified |
| `SYNC-061` | `INV` | A token MUST NOT reach a log. | `ZK-006`; `ambre-spec-session.md`, `SRV-031` | `ambre_sync::websocket::tests::the_socket_address_carries_no_token`; `ambre_sync::websocket::tests::a_logged_token_pair_carries_no_token` -- the socket address holds no token, and the token pair prints with the token redacted, and the raw form is reachable only through `expose` | none | verified |
| `SYNC-013` | `INV` | Application code MUST contain no unchecked memory access. | README pillar "Security" | lint-wall -- `unsafe_code = "forbid"` in the root `[workspace.lints.rust]`, which every crate inherits (`ARCH-032`) | none | verified |
| `SYNC-058` | `INV` | The worker MUST NOT block its own event loop: the socket and the requests MUST stay serviceable. | `ambre-spec-architecture.md section 4` | `ambre_sync::run_tests::the_loop_yields_to_the_tasks_beside_it` -- on one thread, a task beside the waiting loop runs, and closing the commands from it wakes the loop | none | verified |

### 1.2 The worker starts

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-057` | `POST` | No command MUST be lost while the worker starts: everything sent before it is listening MUST wait. | this spec section 2 | `ambre_bus::start::tests::no_command_is_posted_before_the_worker_announces_itself` -- commands queued before the announcement are posted after it, in order, and an encoded event never reads as one | none | verified |
| `SYNC-065` | `POST` | An edit made before the session is ready MUST be held, never dropped. | this spec section 2 | `ambre_sync::updates::tests::an_edit_before_the_session_is_held_then_sent` -- two edits made with no signing key are sent, in order, once the key is set and the replay lands | none | verified |
| `SYNC-097` | `FRAME` | Commands held during the start MUST NOT be reordered when they are released. | this spec section 2; `ambre-spec-delta-order.md` | `ambre_sync::updates::tests::held_commands_are_released_in_order` -- the held opens subscribe in order, and the held edits leave in order at rising clocks | none | verified |

### 1.3 The socket connects, and reconnects

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-063` | `POST` | A socket the relay refuses on authentication MUST be renewed, not merely retried. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-046` | `ambre_sync::lifecycle::tests::an_authentication_refusal_renews_and_reopens` -- the refusal's code renews, and the renewal reopens with the wait reset | none | verified |
| `SYNC-133` | `POST` | A socket the relay refuses for a key setup still owed MUST be neither renewed nor retried until the keys are stored. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-164` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_is_neither_renewed_nor_retried` -- on the native transport, the refusal opens no socket through a timer or a renewal, and the stored session of a setup opens one, as the authentication code does | none | verified |
| `SYNC-134` | `POST` | A key setup still owed MUST reach the rest of the client as a setup to complete, never as a generic error. | this spec section 2; `ambre-spec-session.md`, `SRV-027` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the refused socket; `ambre_sync::setup_tests::a_request_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the sink turns a setup refusal into the setup, under either error variant; `ambre_sync::http::tests::only_a_setup_refusal_reads_as_one` -- only a 403 with the setup code reads as one; `ambre_sync::http::tests::an_account_without_master_material_owes_its_setup` -- the test an unlock branches on; `ambre_logic::dispatch::tests::a_setup_owed_reaches_the_ui_as_a_setup` -- the logic thread relays it | none | verified |
| `SYNC-137` | `POST` | A wire message of a newer version MUST reach the rest of the client once as an update to install, and the socket MUST NOT reopen in that session. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-166` | `ambre_protocol::tests::only_a_newer_wire_version_calls_for_an_update` -- an older version and an empty frame do not; `ambre_sync::websocket::tests::a_newer_wire_version_reaches_logic_as_an_update_and_closes_for_good` -- once, with no error, and no close, timer or renewal reopens; `ambre_logic::dispatch::tests::a_newer_relay_reaches_the_ui_as_an_update` -- the logic thread relays it | none | verified |
| `SYNC-070` | `POST` | A socket error MUST tear the connection down and schedule a reconnect, exactly as a close does. | audit `M10`; this spec section 2 | `ambre_sync::lifecycle::tests::an_error_backs_off_as_a_close_does` -- an error reads as a close with no code, and arms the same wait | none | verified |
| `SYNC-064` | `POST` | The reconnect delay MUST grow while no wire message arrives. | this spec section 2 | `ambre_sync::lifecycle::tests::the_wait_grows_while_no_frame_arrives` -- each close doubles the wait up to its cap; `ambre_sync::websocket::tests::a_dropped_socket_reopens_on_the_clock_and_a_frame_resets_the_wait` -- the worker reopens only once its clock reaches the wait | none | verified |
| `SYNC-098` | `POST` | The reconnect delay MUST reset on an inbound wire message, and never on a bare open. | this spec section 2 | `ambre_sync::lifecycle::tests::the_wait_resets_on_a_frame_alone` -- a reopen with no frame keeps the wait growing, and a frame resets it; `ambre_sync::websocket::tests::a_dropped_socket_reopens_on_the_clock_and_a_frame_resets_the_wait` -- the same through the worker, after an open with no frame | none | verified |
| `SYNC-059` | `FRAME` | The worker MUST stop reading the socket while the path back to the rest of the client is full. | this spec section 2; `ambre-spec-message-bus.md`, `BUS-026` | `ambre_bus::conduit::tests::a_backlog_holds_its_source_at_the_high_mark_and_releases_it_at_the_low` -- the Main side asks once at each mark, never in between; `ambre_sync::websocket::tests::a_held_inbound_reopens_only_on_release_and_catches_up` -- no close, timer or renewal reopens a held socket, and the release pulls from the last sequence then sends the edit kept | none | verified |
| `SYNC-153` | `POST` | Given a replay the relay stopped, the worker MUST ask again from its own cursor, and no sooner than the relay's throttle serves it. | this spec section 2; `ambre-spec-sequencer.md`, `SRV-184`, `SRV-086`; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_stopped_replay_is_asked_again_from_the_cursor_when_its_timer_fires` -- nothing is sent before the timer, a stale timer asks nothing, and the live one asks from the document's cursor; `ambre_sync::websocket::tests::the_replay_retry_delay_doubles_up_to_its_budget` -- the first delay is the throttle window, and each later one doubles it | none | verified |
| `SYNC-154` | `FAIL` | A replay stopped more times than its budget MUST reach the rest of the client as one error, and no retry MUST follow before the next reconnect. | this spec section 2; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_replay_stopped_past_its_budget_reaches_logic_once` -- eight stops give one error and no retry, and a reopened socket counts from zero | none | verified |
| `SYNC-157` | `FAIL` | A document whose first replay stops past its budget MUST reach the rest of the client as an open that failed, never as one still loading. | this spec section 2; `SYNC-154` | `ambre_sync::websocket::tests::a_first_replay_given_up_fails_the_open` -- six stops of a first replay give one failed open and no generic error, and the document is unsubscribed and let go | none | verified |
| `SYNC-158` | `POST` | A stop that names a later delta than the stop before it MUST start its document's budget again. | this spec section 2; `SYNC-154`; `ambre-spec-budgets.md section 8` | `ambre_sync::websocket::tests::a_replay_that_moves_on_keeps_its_budget` -- twelve stops each past the last count one apiece, a stop at or before the last still counts, and a timer armed before a reset asks nothing | none | verified |

### 1.4 A request is made

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-062` | `POST` | A request refused for an expired token MUST be retried once, after a silent renewal. | `ambre-spec-session.md`, `SRV-025` | `ambre_sync::http::tests::a_refused_token_is_renewed_and_the_request_replayed_once` -- the replay carries the renewed token; `ambre_sync::http::tests::a_request_is_replayed_at_most_once` -- a second refusal or a refused renewal ends it | none | verified |
| `SYNC-071` | `FAIL` | A request that receives no answer MUST be abandoned, never awaited without end. | audit `M9`; this spec section 2 | `ambre_sync::http::tests::a_request_with_no_answer_is_abandoned_at_its_deadline` -- a transport that never answers ends the request with an error at its thirty-second deadline, sent once | none | verified |
| `SYNC-152` | `POST` | An answer naming a word of a closed set this build does not know MUST reach the rest of the client as an update to install, never as a generic error. | this spec section 2; `ambre-spec-server.md`, `SRV-180` | `ambre_core::serde_enums::tests::an_unknown_word_reads_as_an_unknown_variant_in_every_set` -- each adapter refuses the word as a derived enum does; `ambre_sync::http::tests::only_an_unknown_word_reads_as_one` -- a role or a status no member carries reads as one, a malformed identifier or a missing field does not; `ambre_sync::setup_tests::an_answer_with_an_unknown_word_reaches_logic_as_an_update` -- the sink turns it into the update, under either error variant; `ambre_logic::dispatch::tests::a_newer_relay_reaches_the_ui_as_an_update` -- the logic thread relays it | none | verified |
| `SYNC-099` | `FRAME` | A renewal MUST NOT replay a request refused for anything but an expired token. | `ambre-spec-session.md`, `SRV-025` | `ambre_sync::http::tests::each_answer_maps_to_its_error` -- a 403, a 409, a 500, a 404 and no answer each send one request, with no renewal | none | verified |

### 1.5 Content leaves for the relay

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-067` | `POST` | A document name MUST reach the relay sealed under the document key. | `ZK-001`; this spec section 2 | `ambre_sync::documents::tests::a_document_name_reaches_the_relay_sealed_under_the_document_key` -- the one seal create and rename send carries no plaintext, names the document key, and opens under it alone. `CHROME-036` holds what a card shows | none | verified |
| `SYNC-068` | `FRAME` | A sealed document name MUST NOT open under any document but its own. | `ambre-spec-sealed-envelope.md`, `SYNC-018` | `ambre_sync::pipeline::tests::a_name_opens_under_its_own_document_only` -- a name sealed for one document refuses to open under another holding the same key, because its additional data holds the document id | none | verified |
| `SYNC-136` | `FAIL` | A create whose answer names another document than the one the client minted MUST fail. | this spec section 2; `SYNC-068` | `ambre_sync::documents::tests::a_create_fails_on_a_foreign_answer` -- a foreign id and a malformed one each answer an error, and only the minted id is reported | none | verified |
| `SYNC-069` | `FAIL` | A channel message sent with no open session of its document MUST be refused aloud, never dropped in silence. | this spec section 2 | `ambre_sync::updates::tests::an_update_for_a_document_with_no_session_is_refused_aloud` -- the update answers the Logic thread with an `UpdateRefused` warning, and reaches no socket | none | verified |
| `SYNC-149` | `FAIL` | A channel message sent while the socket is down MUST reach the relay once it is back, or be refused aloud. | this spec section 2; `SYNC-069` | not built | unbuilt -- lands with the `ARCH P4` -- offline durability Story (`ambre-roadmap.md` E6) | unimplemented |
| `SYNC-155` | `POST` | Given a delta the relay refused by name, the worker MUST drop it from the unconfirmed log and report its document as holding an edit the relay lacks. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-185` | `ambre_sync::updates::tests::a_delta_refused_by_name_leaves_the_log_and_is_reported_once` -- the refused delta and the one behind it leave the log, one warning names the document, and it no longer reads as settled | none | verified |
| `SYNC-156` | `FAIL` | After a delta refused by name, the worker MUST send no further delta of its document before that document reopens. | this spec section 2; `SYNC-155`; `ambre-spec-delta-order.md` | `ambre_sync::updates::tests::a_document_with_a_refused_delta_sends_no_further_delta` -- neither a new edit nor a delta a refused clock resends leaves, and the document opened afresh sends again | none | verified |
| `SYNC-159` | `POST` | Given a delta the relay failed to sequence through a fault of its own, the worker MUST send it again, with every delta that left after it. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-186`; `SYNC-155` | not built | unbuilt -- lands with the `A delta the relay failed to sequence is sent again` Story (`ambre-roadmap.md` E7) | unimplemented |

### 1.6 The session ends

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-066` | `POST` | A sign-out MUST close the socket and clear every key. | `ambre-spec-zero-knowledge.md`, `ZK-002` | `ambre_sync::websocket::tests::a_sign_out_closes_the_socket_and_clears_every_key` -- the transport is disconnected, the seed, both keys, the tokens and every document are gone, and a later drop reopens nothing | none | verified |
| `SYNC-100` | `FRAME` | A sign-out MUST leave no token behind it. | `ZK-006`; `ambre-spec-session.md`, `SRV-026` | `ambre_sync::websocket::tests::session_tokens_wipe_on_drop` -- both tokens are held in a type that wipes itself when dropped, so ending the session clears them | none | verified |

* **Budget.** the relay hop of `ambre-spec-budgets.md section 1`, one encode plus one transfer
  under 1 ms, and the two per-session holders of `section 6`. The socket carries no budget of its
  own: the network dominates the inter-user target.
* **Boundary.** this worker **is** the client side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SYNC-002` and `SYNC-095` keep the keys in. `SYNC-060` and `SYNC-061` keep the tokens in,
  and `SYNC-067` seals the one document field that is content rather than routing.
* **Verification.** 36 `verified`, 2 `unimplemented`. The crate graph holds the keys and the CRDT
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
| the relay stops a replay | ask again from the document's cursor after a growing delay, and report it once past the budget (`SYNC-153`, `SYNC-154`) |
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
replayed; a second refusal reaches the screen as an authentication error (`SYNC-062`). A refusal
for a key setup still owed reaches it as the setup to complete. So does an unlock that finds no
master material (`SYNC-134`).

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

**Code.**

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
| the key setup, when the account still owes it | the setup refusal and the empty unlock (`SYNC-134`) |

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
| A hold closes the socket | pausing the read of a stream socket | only one browser engine ships a socket with a paused read, and the worker must run on every engine |

---

## 5. NOT BUILT, AND NOT YET RULED

* **An edit sent while the network is down is lost on the wire.** Only a hold keeps it; any other
  close drops what is sent before the reopen, and nothing sends it again.
* **A name sealed before the envelope opens as untitled.** The old form took the bare cipher, and
  nothing reads it any more.
* **A rotation leaves the name unreadable.** Nothing re-seals it, so the first revoke on a document
  makes its name unopenable for every member who stayed
  (`ambre-spec-key-rotation.md`, `SYNC-042`).
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
