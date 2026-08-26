# AMBRE -- FEATURE: THE LIVE SOCKET AND THE FAN-OUT

**Type:** Horizontal (framework)
**Implementation:** partial -- the document plane ships whole; the channel plane relays whatever it
is given, and meetings have no endpoint at all.

> One connection per person, carrying every document they have open. A change reaches every other
> screen in the same instant, and the relay forwards bytes it cannot read.

This document owns the socket the relay serves: what a connection may say, what it is told, and how
one instance's message reaches a person connected to another.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `SRV-010` | A frame larger than its cap MUST be refused with no partial effect. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::oversized_delta_writes_no_pending`, `ws::handler::message::ephemeral_binding_tests::oversized_ephemeral_writes_no_presence`. The socket also refuses a frame past the outer envelope before any handler runs | verified |
| `SRV-042` | A frame MUST be accepted only for a document this connection has subscribed to. | this spec section 3 | none. Every handler checks the connection's own subscription set before anything else, and no test sends a frame for an unsubscribed document: the check has no store behind it and the suite drives the handlers with a subscription already in place | unverified |
| `SRV-043` | A member removed while connected MUST stop being served within thirty seconds. | audit `H8`; this spec section 3 | none. The membership is re-read past a thirty-second mark on every write, presence and replay, and the subscription is dropped when it is gone; no test removes a member on a live connection | unverified |
| `SRV-044` | One connection MUST NOT exceed the message rate the deployment declares. | this spec section 3 | `ambre_server::ws::handler::token_bucket::tests::starts_full_and_allows_a_full_burst`, `::refills_at_the_configured_rate`, `::refill_is_capped_at_capacity`, `::sustained_rate_below_refill_is_never_rejected` | verified |
| `SRV-045` | One process MUST hold one live subscription per channel, whatever the number of connections watching it. | this spec section 3 | none, and none is possible without a second connection: the hub is one task holding one stream, which the type makes true, and nothing in continuous integration opens two sockets on one document to observe it | unverified |
| `SRV-073` | A frame one instance accepts MUST reach every subscriber, whichever instance holds their connection. | audit `C2`; `SRV-021` | none. Each process runs one subscriber task on the shared channel and fans the frames out to its own connections, and nothing in continuous integration runs a second instance to watch a frame cross | unverified |
| `SRV-046` | An upgrade refused on authentication MUST close with a code of its own, never with a generic drop. | `ambre-spec-session.md`; `ambre-spec-sync.md`, `SYNC-063` | none. The upgrade always completes and the handler then closes with the authentication code, so the client can renew instead of backing off; no host test drives a close frame | unverified |
| `SRV-047` | A channel subscription MUST be refused to a caller who is not a member of that channel. | this spec section 3 | not built -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-048` | A channel frame MUST carry a verified signature and a number before it is relayed. | this spec section 3; `SRV-006`, `SRV-007` | not built -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-049` | Signalling for a meeting MUST travel on an endpoint of its own. | this spec section 3 | not built -- lands with the `SYNC P4` / `SRV P1` -- channels and meetings Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-050` | A presence entry MUST stay opaque to the relay. | `ZK-005`; this spec section 3 | none of its own: the relay stores the frame's ciphertext as it received it and decodes nothing, which is an absence rather than a check. `ambre_server::ws::handler::message::ephemeral_binding_tests::forged_author_key_writes_no_presence`, `::bad_signature_writes_no_presence` hold the entry a forged sender never gets, which is `SRV-007` | unverified |
| `SRV-072` | A presence entry MUST expire on its own. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::presence_field_self_expires`, on the database tier | verified |
| `SRV-051` | A disconnect MUST leave no subscription and no queue behind it. | this spec section 3; `SRV-045` | none. Every watched channel is deregistered and the outbound task is stopped at the end of the connection, and no test closes a socket to observe the hub afterwards | unverified |

* **Budget.** the live receive and broadcast of `ambre-spec-budgets.md section 3`, under 5 ms
  at the ninety-ninth percentile, and the presence write of the same table. Both read **none
  exists**. The relay's memory per open document, roughly 50 KB in
  `ambre-spec-budgets.md section 6`, is the queue of
  `ambre-spec-sequencer.md` rather than anything held here: a connection holds its subscription
  sets and one outbound queue of 256 frames.
* **Boundary.** every payload on this socket is sealed, and the relay forwards it without opening
  it. What it reads is the routing metadata of `ZK-007`, plus the presence entry it stores under an
  author key -- opaque, and `ZK-005` is the rule (`SRV-050`). The token that opens the socket
  travels in its address, which is the leak `SRV-031` states.
* **Verification.** 3 `verified`, 7 `unverified`, 3 `unimplemented`. What holds is what a handler
  test can drive: the two caps, the rate bucket and the presence entry's own expiry. What is loose
  is everything about the connection rather than the message -- the subscription gate, the
  mid-session removal, the single subscription per channel, the fan-out between instances, the
  close code -- because each needs a live socket, and the suite drives handlers rather than sockets
  (`ambre-spec-testing.md section 9`).

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
for an answer, so an error frame per refusal is a second flood on the path the cap exists to bound.

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

**Why the sender is served its own broadcast.** A client importing its own change is a no-op, and
excluding the sender would mean the fan-out knowing which connection wrote -- a filter on the hot
path for nothing.

**One subscription per channel per process.** The ephemeral store keeps a subscription per
connection to it, and the relay holds one connection to that store, so two sockets watching one
document would collide on it. A single hub task therefore owns the one stream, counts the interested
connections per channel, subscribes on the first and unsubscribes after the last, and copies each
arriving message to every interested outbound queue in the process (`SRV-045`).

**Presence.** The relay stores the sealed blob under the author's key and gives that one entry a
fifteen-second life of its own, so a person who disappears without a clean exit prunes themselves
and the entry never accumulates (`SRV-050`, `SRV-072`).

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| One socket per session, multiplexed | a socket per document costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and each one keeps alive on a mobile battery. Every frame already names its document | a socket per open document |
| Meetings keep an endpoint of their own | signalling has no subscription pattern and no document: it is a short exchange with a lifetime of its own, and folding it into the multiplexed socket would put a fourth plane on a connection that outlives every meeting (`SRV-049`) | one endpoint for everything |
| One process-wide hub rather than one subscriber per connection | subscriptions are a property of the connection to the ephemeral store, so two sockets on one document collide on it, and juggling several store connections desynchronises their command streams. One stream and an in-process copy is the only shape that supports collaboration at all | a subscriber per socket, which works for exactly one reader per document |
| The membership is read once at subscription and re-read on a thirty-second mark | a read per frame puts a query on the hottest path the relay has; a read only at subscription lets someone removed keep writing until they close their laptop. A short mark bounds the window without paying per frame (`SRV-043`) | reading the membership per frame, or trusting the one read at subscription |
| A per-connection allowance, held in the process | the upgrade is limited once, and a socket then lives for hours: each delta is a durable write plus a fan-out, so an authorised writer flooding is a storage cost with no ceiling. An allowance in the process costs no round trip | limiting only the upgrade, which is the state before the allowance existed |
| A presence entry expires by itself, per entry | a sweep is a timer, a scan and a decision about what is stale; an expiry per entry is a property of the entry. The entry is also what a person leaves behind when their connection dies, which is the case a sweep exists for | a periodic sweep over the presence entries, which is what this replaced |
| A presence entry is stored sealed | the entry held the cursor, the colour and the display name as readable values, which is a person's position in a document sitting in the relay's own store. Sealing it costs the relay nothing, because it forwards the entry rather than reading it (`SRV-050`, `ZK-005`) | a readable presence entry, which is what the store held first |
| A refused upgrade gets a code of its own | when a token expires or the relay rotates its signing key every token is invalid at once, so a plain backoff never recovers -- each new connection is refused too. A distinct code lets the client renew and reconnect with no action (`SRV-046`) | one close for every refusal, and a backoff over all of them |
| The channel plane relays without checking, and that is written down rather than hidden | no channel is wired to real data, so the gap is reachable by nobody, and stating it is what stops a later reader from reading the asymmetry as a subtlety. The three gates it owes are `SRV-047` and `SRV-048` | leaving the channel handlers looking like the document ones |
| The socket is bidirectional, and its frames are binary | the client sends as much as it receives, so a server-to-client stream carries half the traffic and needs a second channel for the other half. The frames are a version byte and a compact encoding, because a frame per keystroke burst is what the format has to be affordable for | a server-sent event stream, and a textual encoding on the socket |
| The frame shapes live in one crate, and the relay redefines none of them | two definitions of one wire shape diverge silently, and the first divergence found was a key declared as a bounded array on one side and an unbounded one on the other. One crate is the definition and both ends depend on it | declaring the shapes again inside the relay, beside the handlers that read them |
| A presence frame carries a signature | without one, any connection could claim any author key and write that member's presence entry, which is both an attribution and a way to fill the entry hash. The signature is what makes the author binding checkable rather than declared (`SRV-007`) | an unsigned presence frame, which is what the shape carried first |
| The fan-out rides the ephemeral store rather than a broker | the store is already a dependency, already multiplexed, and already holds every other piece of live state. A broker adds an operational component for one publish-subscribe channel per document, with a durability nobody here needs -- the durable copy is the index | a dedicated message broker between the instances |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A channel is joinable by anyone and writable by anyone.** The subscription is granted without
  reading any membership, and a channel delta or presence blob is forwarded with no signature check,
  no numbering and no durable write. Nothing reaches it today -- the client discards both its send
  paths (`ambre-spec-sync.md`, `SYNC-069`) -- so the reachable consequence is nil and the code says
  so in place. Both gates land with the channels Story (`SRV-047`, `SRV-048`).
* **Meetings have no endpoint, and no frame.** The route the design names is registered nowhere, and
  the protocol carries no signalling frame at all, so the whole plane is a set of tables in the
  register (`SRV-049`).
* **The relay never asks anyone for a snapshot.** The frame exists and every client acts on it, and
  the relay sends it from nowhere (`ambre-spec-sequencer.md`, `SRV-038`).
* **A dropped fan-out is logged and nothing else.** A publish that fails leaves live subscribers
  without the change until one of them reconnects and replays, which is correct and invisible: no
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
