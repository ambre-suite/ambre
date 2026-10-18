# AMBRE -- FEATURE: MURMURE TYPING AND PRESENCE
**Type:** Vertical (feature)
**Implementation:** partial -- both indicators are built; the setting that turns them off is not

> A member sees who is in the channel and who is typing. Both indicators are on by default, and
> both reach the members of the channel alone.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-121` | `INV` | The typing and presence indicators MUST ride the ephemerals of the `meta` document. | this spec section 3; `ambre-spec-presence.md` | `ambre_logic::channels::tests::a_channel_tick_sends_its_watching_sample_on_its_meta_alone`; `ambre_logic::channels::tests::typing_leaves_as_an_ephemeral_and_writes_no_document` -- both samples leave as ephemerals of the `meta` | none | verified |
| `MURMURE-122` | `INV` | Both indicators MUST be on by default. | this spec section 3 | `ambre_logic::channels::tests::a_channel_tick_sends_its_watching_sample_on_its_meta_alone`; `ambre_logic::channels::tests::typing_leaves_as_an_ephemeral_and_writes_no_document` -- the first tick and the first keystroke send with no setting | none | verified |
| `MURMURE-123` | `INV` | Both indicators MUST reach the members of the channel alone. | this spec section 3; `SRV-047` | `ambre_logic::channels::tests::a_channel_tick_sends_its_watching_sample_on_its_meta_alone`; `ambre_sync::pipeline::tests::roundtrip_ephemeral` -- the sample rides the `meta` alone, sealed with its key like a delta | none | verified |

### 1.2 Type in a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-124` | `POST` | Typing MUST refresh the indicator while the member types, and let it expire once they stop. | this spec section 3 | `ambre_murmure::watchers::tests::typing_refreshes_while_it_lasts_and_expires_once_it_stops`; `ambre_logic::channels::tests::a_sample_heard_shows_its_member_until_the_expiry_or_a_leave` -- one sample per typing period, and the view's tick ends it past the expiry | none | verified |
| `MURMURE-125` | `FRAME` | A typing indicator MUST NOT reach the history of any document. | this spec section 3; `SRV-089` | `ambre_logic::channels::tests::typing_leaves_as_an_ephemeral_and_writes_no_document`; `ambre_server::ws::handler::message::access_tests::a_presence_frame_leaves_the_history_untouched` -- no document moves, and the relay numbers no ephemeral | none | verified |

### 1.3 Watch a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-126` | `POST` | A member MUST show as present while a connection of theirs watches the channel. | this spec section 3; `SRV-097` | `ambre_murmure::watchers::tests::a_member_watches_while_their_samples_come_and_stops_when_they_leave`; `ambre_logic::channels::tests::a_sample_heard_shows_its_member_until_the_expiry_or_a_leave` -- present while samples come, gone at the expiry or on a leave | none | verified |
| `MURMURE-127` | `FRAME` | Presence MUST NOT show which other channel a member reads. | this spec section 3 | `ambre_murmure::watchers::tests::a_sample_names_no_channel_and_nothing_else_reads_as_one`; `ambre_logic::channels::tests::a_sample_shows_in_its_own_channel_alone` -- the same two bytes for every channel, each on its own `meta` | none | verified |

### 1.4 Turn the indicators off

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-296` | `POST` | Turning the indicators off MUST stop the client from sending typing and presence samples. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Presence settings` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-297` | `FRAME` | Turning the indicators off MUST NOT hide the indicators of other members. | this spec section 3 | not built | unbuilt -- lands with the `Presence settings` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the presence write of `ambre-spec-budgets.md section 3`, per heartbeat.
* **Boundary.** both indicators are sealed ephemerals (`ZK-005`). The relay learns when a
  member is active, which it knows from the connection anyway.
* **Verification.** 7 `verified`, 2 `unimplemented`. The watcher tests of `crates/ambre-murmure`
  and the channel tests of `crates/ambre-logic` hold both indicators. The setting is not built.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| types | watches the channel | nothing durable | "Alice is typing" on Bob's screen (`MURMURE-124`) | `ambre_logic::channels::tests::a_sample_heard_shows_its_member_until_the_expiry_or_a_leave` |
| stops typing | watches | nothing | the indicator expires | `ambre_murmure::watchers::tests::typing_refreshes_while_it_lasts_and_expires_once_it_stops` |
| reads channel A | reads channel B | nothing | Bob sees nothing of Alice in B (`MURMURE-127`) | `ambre_logic::channels::tests::a_sample_shows_in_its_own_channel_alone` |

---

## 3. MODEL

**CRDT containers.** none. Both indicators are ephemerals of the `meta` document. A sample is a
tag and a state byte, and names no channel (`MURMURE-127`).

**The clock.** The view ticks an open channel, and each tick drops the samples past their
expiry. Every period is a row of `ambre-spec-budgets.md section 8`.

**The setting that turns them off** lives in the `presence` map of the user settings. With it off,
the client sends no typing and no presence sample, on any device (`MURMURE-296`). The relay then
learns nothing of when the member is active. The member still sees the others (`MURMURE-297`).

**Code.** `crates/ambre-murmure/src/watchers.rs`, and the channel commands of the Logic thread.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a keystroke in the composer | the typing indicator refreshes |
| a channel open on screen | the member shows as present in it |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| On by default | off by default | a chat where nobody sees anybody reads as empty, and the relay learns the activity anyway |
| Off stops sending, and still shows the others | off hides both ways | a member who wants privacy for themselves would lose a view nobody asked them to give up |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No surface is built.** The indicators land in the message list and its composer.
* **The setting is not built.** Turning the indicators off waits on the user settings.
