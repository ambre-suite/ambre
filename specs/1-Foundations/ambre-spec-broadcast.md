# AMBRE -- FEATURE: THE LIVE SOCKET AND THE FAN-OUT

**Type:** Horizontal (framework)
**Implementation:** partial -- the document plane ships whole. The channel plane relays whatever it
is given, and meetings have no endpoint at all.

> One connection per person, carrying every document they have open. A change reaches every other
> screen in the same instant, and the relay forwards bytes it cannot read.

This document owns the socket the relay serves: what a connection may say and what it is told. It
also owns how one instance's message reaches a person connected to another.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-045` | `INV` | One process MUST hold one live subscription per channel, whatever the number of connections watching it. | this spec section 3 | none | unheld -- the hub is one task holding one stream, which the type makes true, and nothing in continuous integration opens two sockets on one document to observe it; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-098` | `INV` | What the relay says about a departure MUST be what it already routes on: the document and the author key, and nothing else. | `ambre-spec-zero-knowledge.md`, `ZK-001`; this spec section 3 | type-level -- the message declares a document id and a public key and has no other field, so there is nothing else for it to carry | none | verified |
| `SRV-050` | `INV` | A presence entry MUST stay opaque to the relay. | `ZK-005`; this spec section 3 | none | unheld -- the relay stores the frame's ciphertext as it received it and decodes nothing, which is an absence rather than a check. The two `ephemeral_binding_tests` hold the entry a forged sender never gets, which is `SRV-007` and not this rule; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 A connection opens, and closes

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-046` | `FAIL` | An upgrade refused on authentication MUST close with a code of its own, never with a generic drop. | `ambre-spec-session.md`; `ambre-spec-sync.md`, `SYNC-063` | none | unheld -- the upgrade always completes and the handler then closes with the authentication code, so the client can renew instead of backing off; no host test drives a close frame; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-051` | `FRAME` | A disconnect MUST leave no subscription and no queue behind it. | this spec section 3; `SRV-045` | none | unheld -- every watched channel is deregistered and the outbound task is stopped at the end of the connection, and no test closes a socket to observe the hub afterwards; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 A frame arrives on the socket

Every gate below runs before the frame is acted on, and each one refuses rather than repairs.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-010` | `FAIL` | A frame larger than its cap MUST be refused with no partial effect. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::oversized_delta_writes_no_pending` -- the queue stays empty and a 413 comes back; `ws::handler::message::ephemeral_binding_tests::oversized_ephemeral_writes_no_presence` -- the same emptiness on the presence path, the socket refusing a frame past the outer envelope before any handler runs | none | verified |
| `SRV-044` | `FAIL` | One connection MUST NOT exceed the message rate the deployment declares. | this spec section 3 | `ambre_server::ws::handler::token_bucket::tests::starts_full_and_allows_a_full_burst`; `::refills_at_the_configured_rate`; `::refill_is_capped_at_capacity`; `::sustained_rate_below_refill_is_never_rejected` -- the one that matters in production, where a legitimate client must never be caught | none | verified |
| `SRV-087` | `FAIL` | A refused presence frame MUST be dropped in silence. | this spec section 3; `SRV-044` | type-level -- `handle_ephemeral` is handed no outbound channel, so no refusal on that path has anything to answer with. Presence arrives about ten times a minute per peer, and an error frame per refusal would be a second flood on the path the cap exists to bound | none | verified |
| `SRV-042` | `FAIL` | A frame MUST be accepted only for a document this connection has subscribed to. | this spec section 3 | none | unheld -- every handler checks the connection's own subscription set before anything else, and no test sends a frame for an unsubscribed document: the check has no store behind it and the suite drives the handlers with a subscription already in place; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-043` | `FAIL` | A member removed while connected MUST stop being served within thirty seconds. | audit `H8`; this spec section 3 | none | unheld -- the membership is re-read past a thirty-second mark on every write, presence and replay, and the subscription is dropped when it is gone; no test removes a member on a live connection; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 An accepted frame is fanned out

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-073` | `POST` | A frame one instance accepts MUST reach every subscriber, whichever instance holds their connection. | audit `C2`; `SRV-021` | none | unheld -- each process runs one subscriber task on the shared channel and fans the frames out to its own connections, and nothing in continuous integration runs a second instance to watch a frame cross; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-088` | `FRAME` | A removal MUST reach the person removed alone: no fan-out MUST tell the remaining members who left. | `ZK-005`; this spec section 3 | none | unheld -- the revoke route sends the notice on the target's personal channel and nothing on the document's, so the targeting is held by the call being written that way; no test asserts that a second member's connection is told nothing. The re-key notice the others receive names nobody, and that is the whole of what they may learn; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 A presence entry is recorded

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-072` | `POST` | A presence entry MUST expire on its own. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::presence_field_self_expires` -- the field is gone after its own life runs out, on the database tier, so a person who disappears without a clean exit prunes themselves | none | verified |
| `SRV-097` | `POST` | A person who stops watching a document MUST be dropped from its presence entries, and the others MUST be told, whether they left cleanly or their connection died. | this spec section 3; `1-Foundations/ambre-spec-presence.md`, `PRESENCE-014` | `ambre_server::redis_ops::redis_integration_tests::presence_field_is_dropped_on_departure` -- the entry itself, on the database tier | partial -- the announcement is unread: it is one publish inside the socket handler, and no test drives a socket to its close; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-089` | `FRAME` | Recording a presence entry MUST NOT touch the document's history: it MUST take no number and reach no store the replay reads. | this spec section 3; `ambre-spec-sequencer.md`, `SRV-006` | none | unheld -- `handle_ephemeral` calls neither the ordering script nor the delta queue, so the separation is held by the two paths having been written apart; no test asserts the queue is still empty after a presence frame; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 A channel frame is relayed

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-047` | `FAIL` | A channel subscription MUST be refused to a caller who is not a member of that channel. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5). The channel plane holds nothing back at all today: a subscribe is accepted from anyone the socket already authenticated | unimplemented |
| `SRV-048` | `POST` | A channel frame MUST carry a verified signature and a number before it is relayed. | this spec section 3; `SRV-006`, `SRV-007` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-049` | `FRAME` | Signalling for a meeting MUST NOT travel on this plane: it MUST have an endpoint of its own. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5). No meeting endpoint exists, so nothing refuses signalling smuggled through a channel frame either | unimplemented |

* **Budget.** the live receive and broadcast of `ambre-spec-budgets.md section 3` and the presence
  write of the same table, all three reading **none exists**. The relay's memory per open document
  belongs to `ambre-spec-sequencer.md`, not to anything held here.
* **Boundary.** every payload is sealed and the relay forwards it unopened. It reads the routing
  metadata of `ZK-007` and a presence entry filed under an opaque author key (`SRV-050`). Who left a
  document is the other fan-out leak, and `SRV-088` holds it to the one person it concerns.
* **Verification.** 5 `verified`, 10 `untested`, 3 `unimplemented`. What holds is what a handler
  test drives: the two caps, the rate bucket, the presence expiry and its removal. What is loose is
  everything about the connection rather than the message, each needing a live socket the suite has
  no way to open (`ambre-spec-testing.md section 9`).
---

## 2. WHAT EACH USER SEES

Two peers with one document open, over two connections that may be served by two instances.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| edits | watches | one numbered delta | Bob's screen changes within the round trip (`SRV-045`) | none -- section 6 |
| moves her cursor | watches | one sealed presence entry, expiring | Bob sees Alice's cursor move, and lose it fifteen seconds after she leaves (`SRV-072`) | `redis_ops::redis_integration_tests::presence_field_self_expires` |
| shares the document with Bob | is signed in, without it open | one member row | Bob's list gains the document with no reload (`SRV-042`) | none -- section 6 |
| removes Bob | has the document open | one member row | Bob's document closes; nobody else is told he was the one removed | none -- section 6 |
| removes Bob, then re-keys | keeps typing | one member row | Bob's writes stop within thirty seconds, not at his next reload (`SRV-043`) | none -- section 6 |
| pastes a picture inline instead of on the media path | watches | nothing | Alice is told the frame is too large, and the document is unchanged (`SRV-010`) | `ws::handler::message::delta_size_cap_tests::oversized_delta_writes_no_pending` |
| writes in a channel | is subscribed to it | nothing | the frame is forwarded and never stored (`SRV-048`) | none, and it is unbuilt -- section 6 |

---

## 3. MODEL

**CRDT containers.** none. Every payload here is a sealed frame the relay moves whole.

**One socket per session.** It is opened at sign-in and closed at sign-out, and every document a
person opens is subscribed on it. The token travels in the address, because a browser socket carries
no header of ours (`ambre-spec-session.md`, `SRV-031`).

**What a connection holds**, and none of it is shared with another connection.

| Held | Why |
|---|---|
| the set of subscribed documents | the first gate on every frame (`SRV-042`) |
| the last instant each membership was verified | the thirty-second re-read (`SRV-043`) |
| whether each subscription may write | so a refusal costs no query per frame (`SRV-008`) |
| the last instant a replay was served, per document | the replay throttle (`ambre-spec-sequencer.md`) |
| the identity's signing key, from the token | so a signature check costs no query (`SRV-007`) |
| a message allowance, refilling | the per-connection rate (`SRV-044`) |
| one outbound queue | everything the fan-out hands this connection |

**What a connection may say.** Nine frames on the document and channel planes, and each one is
gated before it is acted on.

| Frame | Gate |
|---|---|
| subscribe to a document | the membership, read from the register |
| unsubscribe | none: it only removes |
| a sealed delta | subscribed, sized, well-formed, still a member, allowed to write, own key, signed, then numbered |
| a sealed presence blob | the same, minus the write role and the numbering; every refusal is silent |
| ask for what was missed | subscribed, not too soon, still a member |
| subscribe to a channel, or leave one | **none today** (`SRV-047`) |
| a channel delta, or channel presence | subscribed to the channel, and nothing else (`SRV-048`) |

**Why a presence refusal is silent.** Presence is sent about ten times a minute and no client waits
for an answer. An error frame per refusal would be a second flood on the path the cap exists to
bound.

**Three channels of the fan-out**, and each one exists because the others cannot reach the reader.

| Channel | Carries | Reaches |
|---|---|---|
| per document | a delta, a presence blob, a snapshot availability, a re-key notice | every connection subscribed to that document, the sender included |
| per person | a document shared, a document revoked | that person's connections, whatever they have open |
| per channel | a channel delta, channel presence | every connection subscribed to that channel |

**Why a person needs a channel of their own.** An invitation has to reach someone who does not yet
know the document exists, so it cannot travel on the document's own channel. A removal is targeted
for the opposite reason: only the person removed learns it, and the re-key notice the others get
names nobody.

**Why the sender is served its own broadcast.** A client importing its own change is a no-op.
Excluding the sender would mean the fan-out knowing which connection wrote -- a filter on the hot
path for nothing.

**A departure is announced.** The relay drops a person's presence entry and publishes one departure
on the document's channel. It does so when they unsubscribe, and once per document they watched when
their socket closes. It names the document and the author key, which are what it routes on already,
so it says nothing it did not already know (`SRV-091`). The per-entry expiry stays underneath as
the fallback for a socket that dies without either path running (`SRV-072`).

**One subscription per channel per process.** The ephemeral store keeps a subscription per
connection to it, and the relay holds one connection to that store. Two sockets watching one
document would therefore collide on it. A single hub task owns the one stream and counts the
interested connections per channel. It subscribes on the first and unsubscribes after the last, and
copies each arriving message to every interested outbound queue in the process (`SRV-045`).

**Presence.** The relay stores the sealed blob under the author's key and gives that one entry a
fifteen-second life of its own (`SRV-050`). A person who disappears without a clean exit therefore
prunes themselves, and the entry never accumulates (`SRV-072`).

**Code.**

* `crates/ambre-server/src/ws/handler/mod.rs` -- the upgrade, the connection state and the dispatch;
* `crates/ambre-server/src/ws/handler/message.rs` -- one handler per frame, and the gates;
* `crates/ambre-server/src/ws/handler/token_bucket.rs` -- the per-connection allowance;
* `crates/ambre-server/src/ws/pubsub_hub.rs` -- the one stream, the counting and the fan-out;
* `crates/ambre-server/src/redis_ops.rs` -- the three publishes and the presence entry.

---

## 4. SURFACE

The socket has no surface of its own. What a person meets of it is four things.

| What the person meets | Comes from |
|---|---|
| a collaborator's change appearing as they make it | the per-document fan-out |
| a collaborator's cursor and selection | the presence entry and its expiry (`SRV-050`, `SRV-072`) |
| a document appearing in their list without a reload | the per-person channel |
| a document closing under them after they are removed | the targeted removal notice, and `SRV-043` |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One socket per session, multiplexed | a socket per open document | it costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and each one keeps alive on a mobile battery. Every frame already names its document. |
| Meetings keep an endpoint of their own | one endpoint for everything | signalling has no subscription pattern and no document: it is a short exchange with a lifetime of its own, and folding it into the multiplexed socket would put a fourth plane on a connection that outlives every meeting (`SRV-049`). |
| One process-wide hub | a subscriber per socket | it works for exactly one reader per document. Subscriptions are a property of the connection to the ephemeral store, so two sockets on one document collide on it, and juggling several store connections desynchronises their command streams. One stream and an in-process copy is the only shape that supports collaboration at all. |
| The membership is read once at subscription and re-read on a thirty-second mark | reading the membership per frame | it puts a query on the hottest path the relay has. A short mark bounds the window without paying per frame (`SRV-043`). |
| The membership is read once at subscription and re-read on a thirty-second mark | trusting the one read at subscription | it lets someone removed keep writing until they close their laptop. |
| A per-connection allowance, held in the process | limiting only the upgrade | it is the state before the allowance existed: the upgrade is limited once, and a socket then lives for hours. Each delta is a durable write plus a fan-out, so an authorised writer flooding is a storage cost with no ceiling, and an allowance in the process costs no round trip. |
| A presence entry expires by itself, per entry | a periodic sweep over the presence entries | it is what this replaced: a sweep is a timer, a scan and a decision about what is stale, where an expiry per entry is a property of the entry. The entry is also what a person leaves behind when their connection dies, which is the case a sweep exists for. |
| A presence entry is stored sealed | a readable presence entry | it is what the store held first: the cursor, the colour and the display name as readable values, which is a person's position in a document sitting in the relay's own store. Sealing costs the relay nothing, because it forwards the entry rather than reading it (`SRV-050`, `ZK-005`). |
| A refused upgrade gets a code of its own | one close for every refusal, and a backoff over all of them | when a token expires or the relay rotates its signing key every token is invalid at once, so a plain backoff never recovers -- each new connection is refused too. A distinct code lets the client renew and reconnect with no action (`SRV-046`). |
| The channel plane relays without checking, and that is written down | leaving the channel handlers looking like the document ones | no channel is wired to real data, so the gap is reachable by nobody, and stating it is what stops a later reader from reading the asymmetry as a subtlety. The three gates it owes are `SRV-047` and `SRV-048`. |
| The socket is bidirectional, and its frames are binary | a server-sent event stream | the client sends as much as it receives, so a server-to-client stream carries half the traffic and needs a second channel for the other half. |
| The socket is bidirectional, and its frames are binary | a textual encoding on the socket | a frame per keystroke burst is what the format has to be affordable for, and a textual body inflates every one of them. The frames are a version byte and a compact encoding instead. |
| The frame shapes live in one crate | declaring the shapes again inside the relay, beside the handlers that read them | two definitions of one wire shape diverge silently, and the first divergence found was a key declared as a bounded array on one side and an unbounded one on the other. One crate is the definition and both ends depend on it. |
| A presence frame carries a signature | an unsigned presence frame | it is what the shape carried first: any connection could claim any author key and write that member's presence entry, which is both an attribution and a way to fill the entry hash. The signature is what makes the author binding checkable rather than declared (`SRV-007`). |
| The fan-out rides the ephemeral store | a dedicated message broker between the instances | it adds an operational component for one publish-subscribe channel per document, with a durability nobody here needs -- the durable copy is the index. The store is already a dependency, already multiplexed, and already holds every other piece of live state. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A channel is joinable by anyone and writable by anyone.** The subscription is granted without
  reading any membership. A channel delta or presence blob is forwarded with no signature check, no
  numbering and no durable write. Nothing reaches it today, because the client drops both its send
  paths (`ambre-spec-sync.md`, `SYNC-069`). The reachable consequence is nil, and the code says so
  in place. Both gates land with the channels Story (`SRV-047`, `SRV-048`).
* **Meetings have no endpoint, and no frame.** The route the design names is registered nowhere.
  The protocol carries no signalling frame at all, so the whole plane is a set of tables in the
  register (`SRV-049`).
* **The relay never asks anyone for a snapshot.** The frame exists and every client acts on it, and
  the relay sends it from nowhere (`ambre-spec-sequencer.md`, `SRV-038`).
* **A dropped fan-out is logged and nothing else.** A publish that fails leaves live subscribers
  without the change until one of them reconnects and replays. That is correct and invisible: no
  count, no measure, and no rule states how often it is allowed to happen.
* **The outbound queue has a depth and no policy.** It holds 256 frames per connection, and what
  happens to a connection that stops draining it is whatever the queue does, which no rule states.
* **No test opens a socket.** The suite drives each handler directly, so the upgrade, the close
  codes, the fan-out across two connections and the teardown are held by nothing
  (`ambre-spec-testing.md section 9`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What number a delta is given, and what makes it durable | `ambre-spec-sequencer.md` |
| What the client does with a frame it receives | `ambre-spec-sync.md`, `ambre-spec-delta-order.md` |
| What a presence payload contains, and who draws it | `ambre-spec-presence.md` |
| How the payload is sealed and opened | `ambre-spec-sealed-envelope.md` |
| What the token in the socket address is, and what refuses it | `ambre-spec-session.md` |
| Who may write, and who may see a document at all | `ambre-spec-server.md` |
| What a re-key notice makes the client do | `ambre-spec-key-rotation.md` |
| What the relay may learn from a frame | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
