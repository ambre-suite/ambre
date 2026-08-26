# AMBRE -- FEATURE: THE SYNC WORKER

**Type:** Horizontal (framework)
**Implementation:** partial -- the session, the socket and the document register all ship; the
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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SYNC-002` | The document key and the private signing key MUST NOT appear in any outbound message; only the public half travels. | `ZK-002`; this spec section 2 | crate graph: `deny.toml` refuses the exchange, cipher and key crates outside this worker, and both key types wipe themselves on drop | verified |
| `SYNC-011` | The Sync worker MUST compile neither CRDT nor rendering code. | this spec section 2 | crate graph: this crate declares neither the CRDT engine nor the canvas, and `deny.toml` refuses the CRDT engine outside the four application crates | verified |
| `SYNC-013` | Application code MUST contain no unchecked memory access. | README pillar "Security" | measured zero across the workspace, and this crate carries no crate-level refusal of it: two crates of seventeen do (`ambre-spec-testing.md section 8`) | unverified |
| `SYNC-056` | The Sync worker MUST run in its own instance, with a linear memory the rest of the client cannot address. | `ambre-spec-architecture.md section 2` | none. The crate builds a standalone artifact and is loaded as a dedicated worker, which the build shape makes true, and nothing in continuous integration boots the pair to observe it | unverified |
| `SYNC-057` | No command MUST be lost while the worker starts: everything sent before it is listening MUST wait. | this spec section 2 | none. The worker announces itself as the last step of its start, and the relay on the other side holds every command until it sees that announcement. No test boots a worker slowly to see the first command survive | unverified |
| `SYNC-058` | The worker MUST NOT block its own event loop: the socket and the requests MUST stay serviceable. | `ambre-spec-architecture.md section 4` | none, and none is possible on host: the property is about a browser event loop. The loop awaits two channels whose producers both live in this worker, which is what makes the historical freeze unreachable by construction | unverified |
| `SYNC-059` | The worker MUST stop reading the socket while the path back to the rest of the client is full. | `ambre-spec-architecture.md section 4` | none, and **the rule fails today**: the worker cannot know. It posts messages one way with no answer, so the queue on the other side fills and the receiving thread blocks on it instead. The repair is the `SYNC P4` hardening Story | unverified |
| `SYNC-060` | A token MUST live in this worker's memory alone: never in browser storage, never in a cookie. | `ambre-spec-session.md`, `SRV-031` | none. The store is a field of this worker and the crate opens no storage handle, so the rule holds by absence; nothing would fail the day a writer appeared | unverified |
| `SYNC-061` | A token MUST NOT reach a log. | `ZK-006`; `ambre-spec-session.md`, `SRV-031` | none, and **the rule fails today**: the socket carries its token as a query parameter, and the whole address is logged on every connection attempt and on every failure. The repair is the `SYNC P4` hardening Story | unverified |
| `SYNC-062` | A request refused for an expired token MUST be retried once, after a silent renewal. | `ambre-spec-session.md`, `SRV-025` | none. Every request helper renews once and replays, and no test drives a refusal: the helpers are the browser fetch path, and the transport the host tier substitutes is above them | unverified |
| `SYNC-063` | A socket the relay refuses on authentication MUST be renewed, not merely retried. | this spec section 2 | none. The refusal has its own close code, and the handler renews on that code alone and backs off on every other. No host test drives a close event | unverified |
| `SYNC-064` | The reconnect delay MUST grow until a frame arrives, and MUST reset on a frame, never on a bare open. | this spec section 2 | none. The reset is the first statement of the inbound handler, so a refused connection never reaches it. Nothing observes the delay | unverified |
| `SYNC-070` | A socket error MUST tear the connection down and schedule a reconnect, exactly as a close does. | audit `M10`; this spec section 2 | none. The error handler takes the same teardown and backoff path as the close handler, and no host test raises a socket error: the event belongs to a browser | unverified |
| `SYNC-071` | A request that receives no answer MUST be abandoned, never awaited without end. | audit `M9`; this spec section 2 | none. Every call attaches a thirty-second abort signal, and no test drives a request that never answers: the request path is the browser fetch | unverified |
| `SYNC-065` | An edit made before the session is ready MUST be held, never dropped. | this spec section 2 | none. The edit is buffered and replayed once the document is open, and no test issues an edit before a session: the host path seeds its keys before anything runs | unverified |
| `SYNC-066` | A sign-out MUST close the socket and wipe every key and every token. | `ZK-002` | none. The keys are cleared and wipe themselves on drop; the two tokens are plain strings that are dropped and not wiped, and nothing asserts either | unverified |
| `SYNC-067` | A document name MUST reach the relay sealed under the document key. | `ZK-001`; this spec section 2 | none. The name is sealed on create and on rename and opened for the list, and no test seals one: the three legs run in a browser. `CHROME-036` holds the half about what a card shows | unverified |
| `SYNC-068` | A document name MUST be bound to the document it belongs to. | `SYNC-018` | none, and **the rule fails today**: the name is sealed with one constant string as additional data, no commitment tag and no signature, so nothing ties a sealed name to its document -- section 5 | unverified |
| `SYNC-069` | A channel message MUST reach the relay or be refused: it MUST NOT be dropped in silence. | this spec section 2 | none, and **the rule fails today**: the client subscribes to a channel and both send paths log the payload and discard it, so a message a user writes disappears with no error. Answered by the `SYNC P4` / `SRV P1` channels and meetings Story | unverified |

* **Budget.** the relay hop of `ambre-spec-budgets.md section 1` -- one encode plus one
  transfer under 1 ms -- and the two per-session holders of
  `ambre-spec-budgets.md section 6`: under 1 MB for this worker's own state, and under 1 MB
  per open document. The socket and the requests carry no budget of their own: the network dominates
  the inter-user target, which the same table records.
* **Boundary.** this worker **is** the client side of the boundary
  `ambre-spec-zero-knowledge.md` draws. It holds every key, opens the one socket the client
  has, and makes every request. `SYNC-002` keeps the keys in, `SYNC-060` and `SYNC-061` keep the
  tokens in, and `SYNC-067` seals the one field of a document that is content rather than routing.
* **Verification.** 2 `verified`, 17 `unverified`, 0 `unimplemented`. What holds is what the crate
  graph refuses: no key crate outside this worker, no CRDT inside it. Everything else is a property
  of a browser -- a worker boot, an event loop, a socket close, a token renewal -- and the wasm tier
  boots neither this worker nor its pair (`ambre-spec-testing.md section 9`). Four rules are stated
  as failing on a reading of the code.

---

## 2. MODEL

**CRDT containers.** none. This worker compiles no CRDT at all (`SYNC-011`): what it carries across
the boundary is opaque bytes in one direction and sealed frames in the other.

**Where it runs.** Two instances, two heaps. The rest of the client -- the screen and the document
logic -- shares one; this worker has its own, and the two meet only over posted messages
(`ambre-spec-architecture.md section 2`).

| Instance | Holds | Reaches the other by |
|---|---|---|
| Main | the screen, the document logic, the CRDT | a posted message, encoded |
| Sync | every key, one socket, every request | the same, in reverse |

**Starting up, and the window that used to swallow the first command.** The worker installs its
inbound handler at the end of an asynchronous start, so anything posted before that instant reaches
no handler at all. The contract is an announcement: as the last step of its start, the worker posts
an empty payload, and the relay on the Main side holds every command until it sees one. An encoded
event is never empty, so the announcement is unambiguous (`SYNC-057`).

**The loop.** One non-blocking wait over two channels: the commands relayed in, and the results of
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

**Three planes cross the boundary**, and they are declared in
`ambre-spec-message-bus.md`, never restated here: a request that expects one answer and
carries an identifier to match it, a control and content plane for documents and channels, and one
lifecycle command. A request identifier is minted by the screen, echoed by the logic thread, and
echoed once more here.

**The socket.** One per session, multiplexed: opened at sign-in, closed at sign-out, and every
document subscribed on it. Its token travels as a query parameter, because a browser socket carries
no header of ours (`SYNC-061`).

| Event | Response |
|---|---|
| the relay refuses the upgrade on authentication | renew the token silently, then reopen (`SYNC-063`) |
| any other close, or a socket error | back off and reopen (`SYNC-064`) |
| the socket opens | resubscribe every document, and pull what was missed (`SYNC-033`) |
| a frame arrives | reset the delay: only a frame proves the connection is live (`SYNC-064`) |

**Why a bare open is not proof.** A refused upgrade still completes, then closes immediately. A
delay reset on the open alone would therefore reset on every refusal, and the reconnection would
become a tight loop against a relay that rate-limits it.

**Requests.** Every one carries the access token. A refusal for an expired token is renewed once and
replayed; a second refusal reaches the screen as an authentication error (`SYNC-062`).

**The document register.** The list, the create, the rename and the delete are ordinary requests,
and one field of them is content rather than routing: the name.

| Field | Form on the relay |
|---|---|
| the identity, the kind, the times, the member count | plaintext routing metadata |
| the name | sealed under the document key (`SYNC-067`) |
| the document key | sealed once per member (`ambre-spec-membership.md`) |

A document with no name stores nothing rather than a placeholder (`CHROME-037`), and the list opens
each name with the key it unwraps from the same row.

**Code.**

* `crates/ambre-sync/src/worker.rs` -- the start, the announcement and the two posted directions;
* `crates/ambre-sync/src/lib.rs` -- the state, the loop and the command dispatch;
* `crates/ambre-sync/src/websocket.rs` -- the socket, the callbacks, the backoff and the demultiplex;
* `crates/ambre-sync/src/auth.rs` -- sign-in, sign-out and the derived keys;
* `crates/ambre-sync/src/http.rs` -- the request helpers, and the renew-and-replay;
* `crates/ambre-sync/src/documents.rs` -- the register, and the sealed name;
* `crates/ambre-sync/src/channels.rs` -- the channel subscription, and the two discarded sends;
* `crates/ambre-app/src/platform.rs` -- the Main side of the pair, and the readiness gate.

---

## 3. SURFACE

The worker has no surface of its own. What a user sees of it is four things.

| What the user meets | Comes from |
|---|---|
| a sign-in screen, and a session that survives a page reload | the tokens and the renewal (`SYNC-062`) |
| a connection state per document | the socket's own events (`SYNC-064`) |
| a document that reconnects with no action after a network drop | the backoff and the resubscription (`SYNC-063`, `SYNC-033`) |
| an authentication error asking them to sign in again | the second refusal of a renewal (`SYNC-062`) |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One separate instance, reached by posted messages | the alternative that was tried runs to completion, so an asynchronous loop returns after its first step and the runtime collects it -- the freeze this project spent weeks on. An instance with its own event loop cannot have that failure, and it buys key isolation as a side effect: the heap that holds the keys is not addressable from the screen | a thread inside the Main instance sharing its memory, which is what the screen and the document logic still use and what this worker must not |
| One socket per session, multiplexed | a socket per document costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and every one of them keeps alive on a mobile battery. Every frame already names its document, so the relay demultiplexes | a socket per open document |
| One worker for every document | a worker per document means a copy of the cryptography per document, and the memory that goes with it, which is unacceptable on a mobile device. One worker holds a map | a Sync worker per open document |
| The readiness announcement, rather than delaying the first command | the race is between an asynchronous boot and a command posted from a frame loop, so any delay only narrows it and the failure it produces is a document stuck behind a loading overlay for ever. Holding on the sending side makes the delivery independent of timing | delaying the first command, which the console being open was enough to hide |
| A refused upgrade has its own close code | when a token expires or the relay rotates its signing key, every token is invalid at once, so a plain backoff never recovers: each new connection is refused too. A distinct code lets the client renew and reconnect with no action from the user | one close code for every refusal, and a backoff over all of them |
| The delay resets on a frame | see section 2: a refused upgrade opens before it closes, so any reset on the open turns a refusal into a loop | resetting when the socket opens |
| An early edit is held, not dropped | the first edit after a reload lands in the milliseconds while sign-in is still in flight. Dropping it lost that edit permanently and produced a clock the relay then refused on the next one | dropping an edit made before the session, and letting the user retype it |
| The tokens live in memory alone | a token in browser storage survives the tab and is readable by any script that reaches the page; a cookie is sent by the browser on requests we did not make. Memory dies with the worker, which is the lifetime a session should have | browser storage, or a cookie |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The socket has no backpressure, and cannot have it as the pair is built.** The design says the
  worker stops reading the socket while the path to Main is full, so the relay feels it as network
  backpressure. A posted message returns nothing, so this worker has no way to learn that the queue
  on the other side is full. What happens instead is the inverse: the queue fills and the receiving
  handler blocks on it, on the thread that draws the screen (`SYNC-059`).
* **The access token is logged.** It rides the socket address as a query parameter, and the address
  is logged on every connection attempt and on every failure, so a session token sits in the browser
  console (`SYNC-061`).
* **The two tokens are not wiped.** The keys wipe themselves on drop; the tokens are plain strings,
  so a sign-out drops them and leaves their bytes where the allocator left them (`SYNC-066`).
* **The document name is bound to nothing.** It is sealed with a constant additional-data string, no
  commitment tag and no signature, which is the same gap a media blob has and the same repair
  (`SYNC-068`, `SYNC-018`, `ambre-spec-media.md`).
* **A rotation leaves the name unreadable.** Nothing re-seals it, so the first revoke on a document
  makes its name unopenable for every member who stayed
  (`ambre-spec-key-rotation.md`, `SYNC-042`).
* **A channel message is discarded.** The subscribe and unsubscribe reach the relay; both send paths
  log the payload and drop it, and both inbound channel frames are discarded too. So a channel can be
  joined and never written to, with no error anywhere (`SYNC-069`).
* **Meetings are a log line each.** Four commands reach this worker and none does anything.
* **A pending reconnect timer is not cancelled.** A second disconnect replaces the callback the
  timer will call, and the timer itself is not cleared, so the first one fires against a callback
  that no longer exists.
* **Reopening a document replaces its whole state.** The quarantine, the cursor, the clock and the
  cached keys go back to their initial values, and the relay's view of the clock does not
  (`ambre-spec-delta-order.md`).
* **A sign-out is the only teardown.** Nothing closes the socket on a page hidden or a tab
  suspended, so a background tab keeps a connection and its keepalives.
* **Neither instance is booted in continuous integration.** A real boot needs a shared-memory page,
  a worker loader and a graphics adapter, and the unit-test runner provisions none of the three,
  which `ambre-spec-testing.md section 9` records as deliberate debt.

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
