# AMBRE -- FEATURE: THE LIVE SOCKET AND THE FAN-OUT

**Type:** Horizontal (framework)

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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-045` | `INV` | One process MUST hold one live subscription per topic, whatever the number of connections watching it. | this spec section 3 | `ambre_server::routes::tests::handler_tests::the_live_set_crosses_instances_and_a_close_leaves_nothing` -- three sockets on one document across two instances count two Redis subscribers on its topic, one per process, database tier | none | verified |
| `SRV-098` | `INV` | What the relay says about a departure MUST be what it already routes on: the document and the author key, and nothing else. | `ambre-spec-zero-knowledge.md`, `ZK-001`; this spec section 3 | type-level -- the message declares a document id and a public key and has no other field, so there is nothing else for it to carry | none | verified |
| `SRV-050` | `INV` | A presence entry MUST stay opaque to the relay. | `ZK-005`; this spec section 3 | `ambre_server::ws::handler::message::ephemeral_binding_tests::a_presence_entry_is_the_ciphertext_as_received` -- the stored entry decodes to exactly the bytes the sample sealed, so the relay neither reads nor rewrites them | none | verified |
| `SRV-102` | `INV` | The socket MUST carry no wire message of its own for a channel: a channel MUST travel as its documents. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | type-level -- `WsClientMsg` and `WsServerMsg` carry no channel variant | none | verified |
| `SRV-166` | `INV` | A change to the shape of any wire message MUST change the version byte every wire message opens on. | this spec section 3; `ambre-spec-sync.md`, `SYNC-137` | `ambre_protocol::tests::the_wire_layout_is_frozen_under_its_version` -- a digest of one wire message per variant, frozen under the version byte, where the decoder counts the variants so that one with no sample fails | none | verified |

### 1.2 A connection opens, and closes

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-046` | `FAIL` | An upgrade refused on authentication MUST close with a code of its own, never with a generic drop. | `ambre-spec-session.md`; `ambre-spec-sync.md`, `SYNC-063` | `ambre_server::routes::tests::handler_tests::a_refused_token_closes_the_socket_with_its_own_code` -- a token that does not verify draws the authentication close code on a live socket, after the upgrade completes | none | verified |
| `SRV-164` | `FAIL` | An upgrade refused for a key setup still owed MUST close with a code distinct from the authentication one. | `ambre-spec-session.md`, `SRV-027`; `SRV-046` | `ambre_server::routes::tests::handler_tests::the_socket_refuses_a_session_without_keys` -- a session minted before the key setup draws the setup close code on a live socket, where a token that does not verify draws the authentication one | none | verified |
| `SRV-269` | `FAIL` | Given a first frame of an older wire version, the relay MUST answer one frame of its own version, then close the socket with the update code. | this spec section 3; `ambre-spec-sync.md`, `SYNC-137` | `ambre_server::routes::tests::handler_tests::a_client_of_an_older_wire_version_is_told_to_update` -- database tier: a first frame one version back draws a frame opening on the relay's byte, then the close 4426 `update required`, where it drew the authentication close before | none | verified |
| `SRV-051` | `FRAME` | A disconnect MUST leave no subscription and no queue behind it. | this spec section 3; `SRV-045` | `ambre_server::routes::tests::handler_tests::the_live_set_crosses_instances_and_a_close_leaves_nothing` -- once both sockets of one instance are gone, the topic counts one Redis subscriber, the other instance's, database tier | none | verified |

### 1.3 A wire message arrives on the socket

Every gate below runs before the wire message is acted on, and each one refuses rather than repairs.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-010` | `FAIL` | A wire message larger than its cap MUST be refused with no partial effect. | this spec section 3 | `ambre_server::ws::handler::message::delta_size_cap_tests::oversized_delta_writes_no_pending` -- the queue stays empty and a 413 comes back; `ws::handler::message::ephemeral_binding_tests::oversized_ephemeral_writes_no_presence` -- the same emptiness on the presence path, the socket refusing a wire message past the outer envelope before any handler runs | none | verified |
| `SRV-044` | `FAIL` | One connection MUST NOT exceed the message rate the deployment declares. | this spec section 3 | `ambre_server::ws::handler::token_bucket::tests::starts_full_and_allows_a_full_burst`; `::refills_at_the_configured_rate`; `::refill_is_capped_at_capacity`; `::sustained_rate_below_refill_is_never_rejected` -- the one that matters in production, where a legitimate client must never be caught | none | verified |
| `SRV-237` | `FAIL` | A declared message rate or burst that is zero or does not fit 32 bits MUST stop the relay at startup, never be truncated. | this spec section 3; `SRV-044` | `ambre_server::state::tests::a_message_limit_past_u32_is_refused_not_truncated` -- the checked conversion refuses both; type-level -- `Config` and the token bucket take `Bounded<RatePerSec>` and `Bounded<Burst>`, which `limit_from_env` alone builds from a run-time value, and neither takes `Saturates`, so no door clamps a zero to 1 | none | verified |
| `SRV-087` | `FAIL` | A refused presence sample MUST be dropped in silence. | this spec section 3; `SRV-044` | type-level -- `handle_ephemeral` is handed no outbound queue, so no refusal on that path has anything to answer with. Presence arrives about ten times a minute per peer, and an error message per refusal would be a second flood on the path the cap exists to bound | none | verified |
| `SRV-042` | `FAIL` | A wire message MUST be accepted only for a document this connection has subscribed to. | this spec section 3 | `ambre_server::ws::handler::message::access_tests::a_frame_for_an_unsubscribed_document_is_refused` -- a write, a presence sample and a replay, each refused before any store is touched | none | verified |
| `SRV-043` | `FAIL` | A member removed while connected MUST stop being served within thirty seconds. | audit `H8`, `N9`; this spec section 3 | `ambre_server::ws::handler::message::access_tests::a_member_removed_mid_session_is_no_longer_served` -- once the mark passes, the next write is refused and the subscription dropped; `ambre_server::ws::handler::message::access_tests::a_silent_member_removed_stops_receiving_the_broadcast` -- a removed member who sends nothing stops receiving the broadcasts once the sweep the socket runs on that period reads the removal | none | verified |
| `SRV-301` | `FAIL` | Given a frame of a removed member that the relay refuses, the relay MUST stop routing the broadcasts of that document to the socket. | this spec section 3; `SRV-043` | `ambre_server::ws::handler::message::access_tests::a_refused_frame_drops_the_route_with_the_subscription` -- database tier: the refused frame drops the subscription and the route with it, and no later broadcast reaches the socket | none | verified |
| `SRV-183` | `FAIL` | A wire message refused with an answer MUST name its code from the set of `SRV-182` and the document it concerns, and carry no text. | this spec section 3; `ambre-spec-server.md`, `SRV-182` | `ambre_server::ws::handler::message::delta_size_cap_tests::each_write_refusal_names_its_delta_and_its_code` -- an empty delta, a foreign key and a bad signature are each answered once, under their own code and document; `ambre_server::routes::tests::handler_tests::a_frame_that_does_not_decode_is_refused_for_no_document` -- on a live socket, a garbled frame is answered with a bad request naming no document; `ambre_protocol::tests::the_wire_layout_is_frozen_under_its_version` -- the digest covers every code word, so a new code moves the version; `ambre_sync::websocket::tests::a_socket_refusal_reaches_logic_with_its_status` -- the client reads the status from the code alone | none | verified |
| `SRV-185` | `FAIL` | A delta refused before it is sequenced MUST be answered with a refusal that names that delta. | this spec section 3; `ambre-spec-sync.md`, `SYNC-155` | `ambre_server::ws::handler::message::delta_size_cap_tests::each_write_refusal_names_its_delta_and_its_code` -- each refusal before the sequencer names the signature of the delta it refused, database tier; `ambre_protocol::tests::the_wire_layout_is_frozen_under_its_version` -- the refusal's layout is frozen under its version | none | verified |
| `SRV-186` | `FAIL` | A delta the relay failed to sequence through a fault of its own MUST be answered with a refusal that names that delta. | this spec section 3; `SRV-185`; `ambre-spec-sync.md`, `SYNC-159` | `ambre_server::ws::handler::message::refusal_tests::a_delta_the_relay_failed_to_sequence_is_named_and_fences_the_socket` -- a store failure names the delta and fences the socket until the author's answer, where a deleted document or an owed key fences nothing; `ambre_server::ws::handler::message::refusal_tests::a_store_failure_reaches_the_client_without_its_name_or_text` -- the refusal names the delta's signature | none | verified |

### 1.4 An accepted wire message is fanned out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-073` | `POST` | A wire message one instance accepts MUST reach every subscriber, whichever instance holds their connection. | audit `C2`; `SRV-021` | `ambre_server::ws::pubsub_hub::tests::a_frame_from_one_instance_reaches_a_subscriber_of_the_other` -- a wire message published by one hub reaches the other hub's subscriber. `ambre_server::routes::tests::handler_tests::the_live_set_crosses_instances_and_a_close_leaves_nothing` -- a departure on one instance reaches a socket on the other, database tier | none | verified |
| `SRV-258` | `FAIL` | Given a run of the hub that ends while it holds topics of several hash slots, the next run MUST hear each of those topics again. | this spec section 3; `SRV-101`; `SRV-254` | `ambre_server::ws::pubsub_hub::tests::a_panic_in_the_hub_refuses_one_request_and_keeps_fanning_out` -- the hub routes topics of two slots, panics, and its next run on the same connection hears both; `::a_dropped_sharded_stream_frees_its_topics_of_every_slot` -- a stream over two slots, once dropped, frees both for a new stream on the same connection. Database tier. The CI `test-db-cluster` job runs both over three primaries, where each fails under rustis 0.25 | none | verified |
| `SRV-088` | `FRAME` | A removal MUST reach the person removed alone: no fan-out MUST tell the remaining members who left. | `ZK-005`; this spec section 3 | `ambre_server::routes::tests::handler_tests::a_removal_reaches_the_person_removed_alone` -- the removed member's topic carries the notice, and the document's topic and the remaining member's carry only a re-key notice that names nobody | none | verified |

### 1.5 A presence entry is recorded

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-072` | `POST` | A presence entry MUST expire on its own. | this spec section 3 | `ambre_server::redis_ops::redis_integration_tests::presence_field_self_expires` -- the field is gone after its own life runs out, on the database tier, so a person who disappears without a clean exit prunes themselves | none | verified |
| `SRV-097` | `POST` | A person who stops watching a document MUST be dropped from its presence entries, whether they left cleanly or their connection died. | this spec section 3; `1-Foundations/ambre-spec-presence.md`, `PRESENCE-014` | `ambre_server::redis_ops::redis_integration_tests::presence_field_is_dropped_on_departure` -- the entry itself, on the database tier | none | verified |
| `SRV-099` | `POST` | The others watching MUST be told when a person stops watching a document. | this spec section 3; `1-Foundations/ambre-spec-presence.md`, `PRESENCE-014` | `ambre_server::routes::tests::handler_tests::the_live_set_crosses_instances_and_a_close_leaves_nothing` -- a clean close and a dropped connection each reach the remaining peer as a departure naming the author key, database tier | none | verified |
| `SRV-089` | `FRAME` | Recording a presence entry MUST NOT touch the document's history: it MUST take no number and reach no store the replay reads. | this spec section 3; `ambre-spec-sequencer.md`, `SRV-006` | `ambre_server::ws::handler::message::access_tests::a_presence_frame_leaves_the_history_untouched` -- the sample is recorded, and takes no number, no clock, no pending entry and no row the replay reads | none | verified |

### 1.6 A channel is subscribed and relayed

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-047` | `FAIL` | A subscription to a document of a channel MUST be refused to a caller who is not a member of that channel. | this spec section 3 | `ambre_server::ws::handler::message::channel_gate_tests::a_segment_refuses_a_caller_outside_its_channel` -- the refusal, database tier; `::a_member_of_the_channel_subscribes_to_its_segment` -- a member still subscribes | none | verified |
| `SRV-048` | `POST` | A wire message on a document of a channel MUST pass the gates of any other document before it is relayed. | this spec section 3; `SRV-006`, `SRV-007` | `ambre_server::ws::handler::message::channel_gate_tests::a_viewer_of_the_channel_writes_no_delta_on_its_segment` -- the write gate, database tier; structural -- the handlers hold no channel branch, so a wire message on a segment takes the path of any other | none | verified |
| `SRV-049` | `FRAME` | Signalling for a call MUST NOT travel on this plane: it MUST have an endpoint of its own. | this spec section 3 | `ambre_server::ws::handler::call::tests::a_wrap_reaches_its_recipient_alone_on_the_call_socket` -- database tier: a join and a wrap travel on the call socket, on a topic of the call, and the socket of this plane closes on a call frame; type-level -- the call frames are the types of `crates/ambre-protocol/src/signaling.rs`, and `WsClientMsg` declares none; `ambre_sync::call_link::tests::the_call_socket_is_an_address_of_its_own` -- the Sync worker opens the call at an address of its own | none | verified |
| `SRV-161` | `POST` | A delta on a thread document MUST publish an activity signal on the topic of its channel. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | `ambre_server::ws::handler::message::channel_gate_tests::a_delta_on_a_thread_signals_its_channel_and_names_it_alone` -- a thread delta reaches the channel topic and a segment delta does not, database tier | none | verified |
| `SRV-162` | `FRAME` | The activity signal MUST NOT carry any field beyond the thread document it names. | this spec section 3; `ZK-015` | `ambre_server::ws::handler::message::channel_gate_tests::a_delta_on_a_thread_signals_its_channel_and_names_it_alone` -- the published bytes equal the encoding of the signal naming the thread; type-level -- `WsServerMsg::ThreadActivity` holds the document identifier alone | none | verified |

* **Budget.** the live receive and broadcast of `ambre-spec-budgets.md section 3` and the presence
  write of the same table, all three reading **none exists**. The relay's memory per open document
  belongs to `ambre-spec-sequencer.md`, not to anything held here.
* **Boundary.** every payload is sealed and the relay forwards it unopened. It reads the routing
  metadata of `ZK-007` and a presence entry filed under an opaque author key (`SRV-050`). Who left a
  document is the other fan-out leak, and `SRV-088` holds it to the one person it concerns.
* **Verification.** 31 `verified`. A handler test drives the caps, the rate bucket, the presence
  expiry and removal, the channel gates and the thread signal. What is loose is everything about the
  connection rather than the message. Each needs a live socket, and two tests alone open one, each
  to see it refused (`ambre-spec-testing.md section 9`).
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
| pastes a picture inline instead of on the media path | watches | nothing | Alice is told the delta is too large, and the document is unchanged (`SRV-010`) | `ws::handler::message::delta_size_cap_tests::oversized_delta_writes_no_pending` |
| writes in a channel | is subscribed to it | one numbered delta on the active segment of the channel | Bob's screen changes within the round trip (`SRV-048`) | none -- the gates hold, and no view draws a channel yet |
| replies in a thread Bob has not opened | watches the channel | one numbered delta on the thread document, and one activity signal | Bob's channel counts a reply that mentions him (`SRV-161`) | `ws::handler::message::channel_gate_tests::a_delta_on_a_thread_signals_its_channel_and_names_it_alone` |

---

## 3. MODEL

**CRDT containers.** none. Every payload here is a sealed envelope the relay moves whole.

**One socket per session.** It is opened at sign-in and closed at sign-out, and every document a
person opens is subscribed on it. The token travels in the address, because a browser socket carries
no header of ours (`ambre-spec-session.md`, `SRV-031`).

**What a connection holds**, and none of it is shared with another connection.

| Held | Why |
|---|---|
| the set of subscribed documents | the first gate on every wire message (`SRV-042`) |
| the last instant each membership was verified | the thirty-second re-read (`SRV-043`) |
| whether each subscription may write | so a refusal costs no query per wire message (`SRV-008`) |
| the last instant a replay was served, per document | the replay throttle (`ambre-spec-sequencer.md`) |
| the identity's signing key, from the token | so a signature check costs no query (`SRV-007`) |
| a message allowance, refilling | the per-connection rate (`SRV-044`) |
| one outbound queue | everything the fan-out hands this connection |

**What a connection may say.** A wire message is one message on the socket, in either direction.
A client sends the wire messages of the document plane, and each one is gated before it is acted
on. A channel adds no wire message of its own (`SRV-102`).

| Wire message | Gate |
|---|---|
| subscribe to a document | the membership, read from the register |
| unsubscribe | none: it only removes |
| a sealed delta | subscribed, sized, well-formed, still a member, allowed to write, own key, signed, then numbered unless a rotation is owed; that refusal names the key and the clock, to the author alone (`SRV-123`) |
| a sealed presence blob | the same, minus the write role and the numbering; every refusal is silent, an owed rotation included |
| ask for what was missed | subscribed, not too soon, still a member |
| subscribe to a document of a channel | the membership of the channel, read on its `meta` document (`SRV-047`) |

**The version byte.** Every frame opens on it, before the compact encoding. A frame of another
version is refused whole. The encoding names no field, so a shape that moves under the same byte
is read into the wrong fields (`SRV-166`). A client meeting a newer byte is owed an update
(`SYNC-137`).

A first frame of an older byte draws one frame of the relay's own version before the close
(`SRV-269`). The older client knows no new close code, and it reads that byte as an update to
install. Without it, the client read the authentication close, renewed its token and reconnected.

**Why a presence refusal is silent.** Presence is sent about ten times a minute and no client waits
for an answer. An error message per refusal would be a second flood on the path the cap exists to
bound.

**Three topics of the fan-out**, and each one exists because the others cannot reach the reader.

| Topic | Carries | Reaches |
|---|---|---|
| per document | a delta, a presence blob, a snapshot availability, a re-key notice | every connection subscribed to that document, the sender included |
| per person | a document shared, a document revoked | that person's connections, whatever they have open |
| per channel | the activity signal of a thread document (`SRV-161`) | every connection subscribed to a document of that channel |

**Why a person needs a topic of their own.** An invitation has to reach someone who does not yet
know the document exists, so it cannot travel on the document's own topic. A removal is targeted
for the opposite reason: only the person removed learns it, and the re-key notice the others get
names nobody.

**Why the sender is served its own broadcast.** A client importing its own change is a no-op.
Excluding the sender would mean the fan-out knowing which connection wrote -- a filter on the hot
path for nothing.

**A departure is announced.** The relay drops a person's presence entry and publishes one departure
on the document's topic. It does so when they unsubscribe, and once per document they watched when
their socket closes. It names the document and the author key, which are what it routes on already,
so it says nothing it did not already know (`SRV-091`). The per-entry expiry stays underneath as
the fallback for a socket that dies without either path running (`SRV-072`).

**One subscription per topic per process.** The ephemeral store keeps a subscription per
connection to it. Two sockets watching one document would therefore collide on it. A single hub
task owns the one stream and counts the interested connections per topic. It subscribes on the
first and unsubscribes after the last, and copies each arriving message to every interested
outbound queue in the process (`SRV-045`).

**The hub subscribes on a connection of its own.** The store puts a connection that holds a
subscription in its `pubsub` class, RESP3 included. It closes such a connection when its output
buffer passes the limit of that class: 32 MB, or 8 MB for 60 s, by default. The commands of every
socket travel on the shared connection, so the hub keeps its subscriptions off it. Each run of the
hub reuses the connection. A stream that a run drops frees its topics one slot at a time, so the
next run takes them over (`SRV-258`).

**A call is not on this plane.** Its signalling is a short exchange with a lifetime of its own,
on a socket of its own and a topic per call (`SRV-049`). `ambre-spec-call-media.md section 3`
holds its steps.

**A channel is a set of documents.** The relay keeps which documents form each channel, and the kind
of each one (`ZK-015`). A subscription to the channel is a subscription to its `meta` document and
its live segments, each gated by the membership of the channel (`SRV-047`). Every wire message on them is
an ordinary document message (`SRV-048`).

**A thread signals its channel.** A thread document is subscribed while a client has it open. A
delta on it also publishes one activity signal on the topic of the channel (`SRV-161`). A socket
joins that topic with its first document of the channel, and leaves it with its last. The signal
names the thread alone (`SRV-162`). A client that holds the thread drops it.

**Presence.** The relay stores the sealed blob under the author's key and gives that one entry a
fifteen-second life of its own (`SRV-050`). A person who disappears without a clean exit therefore
prunes themselves, and the entry never accumulates (`SRV-072`).

**Code.**

* `crates/ambre-server/src/ws/handler/mod.rs` -- the upgrade, the connection state and the dispatch;
* `crates/ambre-server/src/ws/handler/message.rs` -- one handler per wire message, and the gates;
* `crates/ambre-server/src/ws/handler/token_bucket.rs` -- the per-connection allowance;
* `crates/ambre-server/src/ws/handler/call.rs` -- the call socket, an endpoint of its own for
  the signalling of a call (`SRV-049`, `ambre-spec-call-media.md section 3`);
* `crates/ambre-server/src/ws/pubsub_hub.rs` -- the one stream, the counting and the fan-out;
* `crates/ambre-server/src/redis_ops.rs` -- the publishes, `thread_activity` among them, and the
  presence entry.

---

## 4. SURFACE

The socket has no surface of its own. What a person meets of it is four things.

| What the person meets | Comes from |
|---|---|
| a collaborator's change appearing as they make it | the per-document fan-out |
| a collaborator's cursor and selection | the presence entry and its expiry (`SRV-050`, `SRV-072`) |
| a document appearing in their list without a reload | the per-person topic |
| a document closing under them after they are removed | the targeted removal notice, and `SRV-043` |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One socket per session, multiplexed | a socket per open document | it costs five connections for one board with four embedded documents, against a browser limit of about six per origin, and each one keeps alive on a mobile battery. Every wire message already names its document. |
| Calls keep an endpoint of their own | one endpoint for everything | signalling has no subscription pattern and no document: it is a short exchange with a lifetime of its own, and folding it into the multiplexed socket would put a fourth plane on a connection that outlives every meeting (`SRV-049`). |
| One process-wide hub | a subscriber per socket | it works for exactly one reader per document. Subscriptions are a property of the connection to the ephemeral store, so two sockets on one document collide on it, and juggling several store connections desynchronises their command streams. One stream and an in-process copy is the only shape that supports collaboration at all. |
| The hub subscribes on a connection of its own | the shared command connection | the store would class the shared connection as `pubsub` and close it past 32 MB of queued output. A burst of broadcasts would then fail the commands of every socket in flight, a sequencing script included. |
| Every run of the hub reuses its connection | a new connection per run | it hid a fault of rustis before 0.27.3: a dropped stream released topics of several slots in one `SUNSUBSCRIBE`, which the cluster client refused, so the topics stayed taken on that connection. The fault is fixed, and `SRV-258` holds the fix on a cluster. |
| The membership is read once at subscription and re-read on a thirty-second mark | reading the membership per wire message | it puts a query on the hottest path the relay has. A short mark bounds the window without paying per message (`SRV-043`). |
| The membership is read once at subscription and re-read on a thirty-second mark | trusting the one read at subscription | it lets someone removed keep writing until they close their laptop. |
| A per-connection allowance, held in the process | limiting only the upgrade | it is the state before the allowance existed: the upgrade is limited once, and a socket then lives for hours. Each delta is a durable write plus a fan-out, so an authorised writer flooding is a storage cost with no ceiling, and an allowance in the process costs no round trip. |
| A presence entry expires by itself, per entry | a periodic sweep over the presence entries | it is what this replaced: a sweep is a timer, a scan and a decision about what is stale, where an expiry per entry is a property of the entry. The entry is also what a person leaves behind when their connection dies, which is the case a sweep exists for. |
| A presence entry is stored sealed | a readable presence entry | it is what the store held first: the cursor, the colour and the display name as readable values, which is a person's position in a document sitting in the relay's own store. Sealing costs the relay nothing, because it forwards the entry rather than reading it (`SRV-050`, `ZK-005`). |
| A refused upgrade gets a code of its own | one close for every refusal, and a backoff over all of them | when a token expires or the relay rotates its signing key every token is invalid at once, so a plain backoff never recovers -- each new connection is refused too. A distinct code lets the client renew and reconnect with no action (`SRV-046`). |
| A setup still owed gets a code of its own | the authentication code for both refusals | a renewal cannot cure it. The renewed session has no keys either, so the client renews and reconnects until the setup, and each attempt is refused (`SRV-164`). |
| A channel travels as its documents | a channel plane with wire messages, numbering and storage of its own | a second pipeline to gate, number and store beside the document one. The gates the plane owed become the document gates (`SRV-047`, `SRV-048`) |
| The socket is bidirectional, and its wire messages are binary | a server-sent event stream | the client sends as much as it receives, so a server-to-client stream carries half the traffic and needs a second connection for the other half. |
| The socket is bidirectional, and its wire messages are binary | a textual encoding on the socket | a wire message per keystroke burst is what the format has to be affordable for, and a textual body inflates every one of them. The messages are a version byte and a compact encoding instead. |
| The wire message shapes live in one crate | declaring the shapes again inside the relay, beside the handlers that read them | two definitions of one wire shape diverge silently, and the first divergence found was a key declared as a bounded array on one side and an unbounded one on the other. One crate is the definition and both ends depend on it. |
| A presence sample carries a signature | an unsigned presence sample | it is what the shape carried first: any connection could claim any author key and write that member's presence entry, which is both an attribution and a way to fill the entry hash. The signature is what makes the author binding checkable rather than declared (`SRV-007`). |
| The fan-out rides the ephemeral store | a dedicated message broker between the instances | it adds an operational component for one publish-subscribe topic per document, with a durability nobody here needs -- the durable copy is the index. The store is already a dependency, already multiplexed, and already holds every other piece of live state. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A dropped fan-out is logged and nothing else.** A publish that fails leaves live subscribers
  without the change until one of them reconnects and replays. That is correct and invisible: no
  count, no measure, and no rule states how often it is allowed to happen.
* **The outbound queue has a depth and no policy.** It holds 256 wire messages per connection. What
  happens to a connection that stops draining it is whatever the queue does, which no rule states.
* **Two tests open a socket, and only to see it refused.** They hold the close of a token that does
  not verify and of a session without keys (`SRV-046`, `SRV-164`, `ambre-spec-session.md`, `SRV-027`). The
  suite drives every other handler directly, so the fan-out across two connections and the teardown
  are held by nothing (`ambre-spec-testing.md section 9`).

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What number a delta is given, and what makes it durable | `ambre-spec-sequencer.md` |
| What the client does with a wire message it receives | `ambre-spec-sync.md`, `ambre-spec-delta-order.md` |
| What a presence payload contains, and who draws it | `ambre-spec-presence.md` |
| How the payload is sealed and opened | `ambre-spec-sealed-envelope.md` |
| What the token in the socket's first frame is, and what refuses it | `ambre-spec-session.md` |
| Who may write, and who may see a document at all | `ambre-spec-server.md` |
| What a re-key notice makes the client do | `ambre-spec-key-rotation.md` |
| What the relay may learn from a wire message | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| Which documents form a channel, and what each one holds | `5-Murmure/ambre-spec-murmure-schema.md` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
