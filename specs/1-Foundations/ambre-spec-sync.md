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
| `SYNC-096` | `INV` | The Sync worker MUST compile no rendering code. | this spec section 2; `ambre-spec-frame.md`, `CANVAS-057` | none | unheld -- the manifest declares neither the canvas nor `wgpu`, and no `deny.toml` entry refuses them here, unlike the CRDT and cryptography bans beside it. The rule is read off a manifest rather than enforced by a job; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-056` | `INV` | The Sync worker MUST run in its own instance, with a linear memory the rest of the client cannot address. | `ambre-spec-architecture.md section 2` | none | unheld -- the crate builds a standalone artifact and is loaded as a dedicated worker, which the build shape makes true, and nothing in continuous integration boots the pair to observe it; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-060` | `INV` | A token MUST live in this worker's memory alone: never in browser storage, never in a cookie. | `ambre-spec-session.md`, `SRV-031` | none | unheld -- the store is a field of this worker and the crate opens no storage handle, so the rule holds by absence, and nothing would fail the day a writer appeared; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-061` | `INV` | A token MUST NOT reach a log. | `ZK-006`; `ambre-spec-session.md`, `SRV-031` | `ambre_sync::websocket::tests::a_logged_socket_url_carries_no_token`; `ambre_sync::websocket::tests::a_logged_token_pair_carries_no_token` -- the socket address and the token pair print with the token redacted, and the raw form is reachable only through `expose` | none | verified |
| `SYNC-013` | `INV` | Application code MUST contain no unchecked memory access. | README pillar "Security" | none | unheld -- the workspace measures zero unchecked access today, and this crate carries no crate-level refusal of it: two crates of seventeen do (`ambre-spec-testing.md section 8`); lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-058` | `INV` | The worker MUST NOT block its own event loop: the socket and the requests MUST stay serviceable. | `ambre-spec-architecture.md section 4` | none | unheld -- none is possible on host: the property is about a browser event loop. The loop awaits two channels whose producers both live in this worker, which is what makes the historical freeze unreachable by construction; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 The worker starts

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-057` | `POST` | No command MUST be lost while the worker starts: everything sent before it is listening MUST wait. | this spec section 2 | none | unheld -- the worker announces itself as the last step of its start, and the relay on the other side holds every command until it sees that announcement. No test boots a worker slowly to see the first command survive; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-065` | `POST` | An edit made before the session is ready MUST be held, never dropped. | this spec section 2 | none | unheld -- the edit is buffered and replayed once the document is open, and no test issues an edit before a session: the host path seeds its keys before anything runs; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-097` | `FRAME` | Commands held during the start MUST NOT be reordered when they are released. | this spec section 2; `ambre-spec-delta-order.md` | none | unheld -- the holding buffer is a queue and the release drains it in order, so the rule is held by the container's own shape, and nothing asserts it. Releasing an open after the edit that depends on it drops the edit, which is `SYNC-065` failing by another route; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 The socket connects, and reconnects

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-063` | `POST` | A socket the relay refuses on authentication MUST be renewed, not merely retried. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-046` | none | unheld -- the refusal has its own close code, and the handler renews on that code alone and backs off on every other. No host test drives a close event; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-133` | `POST` | A socket the relay refuses for a key setup still owed MUST be neither renewed nor retried until the keys are stored. | this spec section 2; `ambre-spec-broadcast.md`, `SRV-164` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_is_neither_renewed_nor_retried` -- on the native transport, the refusal opens no socket through a timer or a renewal, and the stored session of a setup opens one, as the authentication code does | none | verified |
| `SYNC-134` | `POST` | A key setup still owed MUST reach the rest of the client as a setup to complete, never as a generic error. | this spec section 2; `ambre-spec-session.md`, `SRV-027` | `ambre_sync::websocket::tests::a_socket_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the refused socket; `ambre_sync::setup_tests::a_request_refused_for_a_setup_owed_reaches_logic_as_a_setup` -- the sink turns a setup refusal into the setup, under either error variant; `ambre_sync::http::tests::only_a_setup_refusal_reads_as_one` -- only a 403 with the setup code reads as one; `ambre_sync::http::tests::an_account_without_master_material_owes_its_setup` -- the test an unlock branches on; `ambre_logic::dispatch::tests::a_setup_owed_reaches_the_ui_as_a_setup` -- the logic thread relays it | none | verified |
| `SYNC-070` | `POST` | A socket error MUST tear the connection down and schedule a reconnect, exactly as a close does. | audit `M10`; this spec section 2 | none | unheld -- the error handler takes the same teardown and backoff path as the close handler, and no host test raises a socket error: the event belongs to a browser; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-064` | `POST` | The reconnect delay MUST grow while no frame arrives. | this spec section 2 | none | unheld -- the delay is doubled on each teardown, and nothing observes it; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-098` | `POST` | The reconnect delay MUST reset on an inbound frame, and never on a bare open. | this spec section 2 | none | unheld -- the reset is the first statement of the inbound handler, so a connection that opens and is refused never reaches it, which is the whole point: resetting on the open turns a relay refusing every connection into a client retrying without a backoff at all; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-059` | `FRAME` | The worker MUST stop reading the socket while the path back to the rest of the client is full. | `ambre-spec-architecture.md section 4` | none | fails -- the worker cannot know. It posts messages one way with no answer, so the queue on the other side fills and the receiving thread blocks on it instead; lands with the `SYNC P4` hardening Story (`ambre-roadmap.md` E7) | failing |

### 1.4 A request is made

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-062` | `POST` | A request refused for an expired token MUST be retried once, after a silent renewal. | `ambre-spec-session.md`, `SRV-025` | none | unheld -- every request helper renews once and replays, and no test drives a refusal: the helpers are the browser fetch path, and the transport the host tier substitutes is above them; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-071` | `FAIL` | A request that receives no answer MUST be abandoned, never awaited without end. | audit `M9`; this spec section 2 | none | unheld -- every call attaches a thirty-second abort signal, and no test drives a request that never answers: the request path is the browser fetch; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-099` | `FRAME` | A renewal MUST NOT replay a request refused for anything but an expired token. | `ambre-spec-session.md`, `SRV-025` | none | unheld -- the retry is keyed on the one status a stale token produces, and nothing asserts the other refusals fall through. Replaying a write refused for a missing permission sends it twice against a relay that already said no; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Content leaves for the relay

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-067` | `POST` | A document name MUST reach the relay sealed under the document key. | `ZK-001`; this spec section 2 | none | unheld -- the name is sealed on create and on rename and opened for the list, and no test seals one: the three legs run in a browser. `CHROME-036` holds the half about what a card shows; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-068` | `FRAME` | A sealed document name MUST NOT open under any document but its own. | `ambre-spec-sealed-envelope.md`, `SYNC-018` | `ambre_sync::pipeline::tests::a_name_opens_under_its_own_document_only` -- a name sealed for one document refuses to open under another holding the same key, because its additional data holds the document id | none | verified |
| `SYNC-136` | `FAIL` | A create whose answer names another document than the one the client minted MUST fail. | this spec section 2; `SYNC-068` | none | fails -- only a debug assertion compares the two, so a release build reports the minted id whatever the relay stored, and the name sealed under it no longer opens; lands with the `A create fails on a foreign answer` Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-069` | `FAIL` | A channel message MUST reach the relay or be refused: it MUST NOT be dropped in silence. | this spec section 2 | `ambre_sync::channels::tests::a_channel_message_is_refused_not_dropped` -- both send paths answer the caller with a refusal until the relay carries channels | none | verified |

### 1.6 The session ends

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-066` | `POST` | A sign-out MUST close the socket and clear every key. | `ambre-spec-zero-knowledge.md`, `ZK-002` | none | unheld -- the keys are cleared and wipe themselves on drop (`SYNC-095`), and nothing asserts the socket is closed or that a later frame finds no state to apply; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-100` | `FRAME` | A sign-out MUST leave no token behind it. | `ZK-006`; `ambre-spec-session.md`, `SRV-026` | `ambre_sync::websocket::tests::session_tokens_wipe_on_drop` -- both tokens are held in a type that wipes itself when dropped, so ending the session clears them | none | verified |

* **Budget.** the relay hop of `ambre-spec-budgets.md section 1`, one encode plus one transfer
  under 1 ms, and the two per-session holders of `section 6`. The socket carries no budget of its
  own: the network dominates the inter-user target.
* **Boundary.** this worker **is** the client side of the boundary `ambre-spec-zero-knowledge.md`
  draws. `SYNC-002` and `SYNC-095` keep the keys in. `SYNC-060` and `SYNC-061` keep the tokens in,
  and `SYNC-067` seals the one document field that is content rather than routing.
* **Verification.** 9 `verified`, 2 `failing`, 17 `untested`. The crate graph holds the keys and the
  CRDT apart. Unit tests hold the token redaction and wipe, the refused channel send and the name
  bound to its document. Each gap left is a browser property the wasm tier cannot boot,
  `ambre-spec-testing.md section 9`.
---

## 2. MODEL

**CRDT containers.** none. This worker compiles no CRDT at all (`SYNC-011`). What it carries across
the boundary is opaque bytes in one direction and sealed frames in the other.

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
document subscribed on it. Its token travels as a query parameter, because a browser socket carries
no header of ours (`SYNC-061`).

| Event | Response |
|---|---|
| the relay refuses the upgrade on authentication | renew the token silently, then reopen (`SYNC-063`) |
| the relay refuses the upgrade for a key setup still owed | neither renew nor reopen until a session with keys is stored, and tell the client a setup is owed (`SYNC-133`, `SYNC-134`) |
| any other close, or a socket error | back off and reopen (`SYNC-064`) |
| the socket opens | resubscribe every document, and pull what was missed (`SYNC-033`) |
| a frame arrives | reset the delay: only a frame proves the connection is live (`SYNC-064`) |

**Why a bare open is not proof.** A refused upgrade still completes, then closes immediately. A
delay reset on the open alone would reset on every refusal. The reconnection would then become a
tight loop against a relay that rate-limits it.

**Requests.** Every one carries the access token. A refusal for an expired token is renewed once and
replayed; a second refusal reaches the screen as an authentication error (`SYNC-062`). A refusal
for a key setup still owed reaches it as the setup to complete. So does an unlock that finds no
master material (`SYNC-134`).

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
* `crates/ambre-sync/src/channels.rs` -- the channel subscription, and the two dropped sends;
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
| One socket per session, multiplexed | a socket per open document | it costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and every one of them keeps alive on a mobile battery. Every frame already names its document, so the relay demultiplexes |
| One worker for every document | a Sync worker per open document | it means a copy of the cryptography per document, and the memory that goes with it, which is unacceptable on a mobile device. One worker holds a map |
| The readiness announcement, rather than delaying the first command | delaying the first command | the console being open was enough to hide it. The race is between an asynchronous boot and a command posted from a frame loop, so a delay only narrows it, and the failure it leaves is a document stuck behind a loading overlay for ever. Holding on the sending side makes the delivery independent of timing |
| A refused upgrade has its own close code | one close code for every refusal, and a backoff over all of them | when a token expires or the relay rotates its signing key, every token is invalid at once, so a plain backoff never recovers: each new connection is refused too. A distinct code lets the client renew and reconnect with no action from the user |
| The delay resets on a frame | resetting when the socket opens | a refused upgrade opens before it closes -- see section 2 -- so a reset on the open turns a refusal into a loop |
| An early edit is held, not dropped | dropping an edit made before the session, and letting the user retype it | the first edit after a reload lands in the milliseconds while sign-in is still in flight. Dropping it lost that edit permanently and produced a clock the relay then refused on the next one |
| A setup refusal is told at the one sink every event leaves by | a check in each request handler | about forty handlers turn a failed request into an error, each in its own words. One that missed the check would show a setup still owed as a generic failure. The sink sees every event, so one mapping covers each (`SYNC-134`) |
| The tokens live in memory alone | browser storage | a token there survives the tab and is readable by any script that reaches the page. Memory dies with the worker, which is the lifetime a session should have |
| The tokens live in memory alone | a cookie | the browser sends it on requests we did not make, which is the ambient credential the deliberate token exists to avoid |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The socket has no backpressure, and cannot have it as the pair is built.** The design says the
  worker stops reading the socket while the path to Main is full, so the relay feels it as network
  backpressure. A posted message returns nothing, so this worker has no way to learn that the queue
  on the other side is full. What happens instead is the inverse: the queue fills and the receiving
  handler blocks on it, on the thread that draws the screen (`SYNC-059`).
* **A name sealed before the envelope opens as untitled.** The old form took the bare cipher, and
  nothing reads it any more.
* **A rotation leaves the name unreadable.** Nothing re-seals it, so the first revoke on a document
  makes its name unopenable for every member who stayed
  (`ambre-spec-key-rotation.md`, `SYNC-042`).
* **A channel message is refused, never carried.** The subscribe and unsubscribe reach the relay.
  Both send paths answer the caller with a refusal until the relay carries channel messages, and
  both inbound channel frames are dropped (`SYNC-069`).
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
| What the relay does with a request or a frame | `ambre-spec-server.md`, `ambre-spec-broadcast.md` |
| What the relay does with a token, and what mints one | `ambre-spec-session.md` |
| Why this worker is a separate instance rather than a thread | `ambre-spec-architecture.md section 4` |
| What the home screen shows of a document list | `ambre-spec-launcher.md` |
| Which thread may hold document data at all | `ambre-spec-ui.md`, `UI-THREAD-020` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
