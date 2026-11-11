# AMBRE -- FEATURE: CALLS FROM A CHANNEL
**Type:** Vertical (feature)
**Implementation:** partial -- the call message, its end and who is in it; the call controls are not built

> A member starts an audio and video call from a channel in one click. The members of the channel
> join it, and the call is encrypted end to end.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-407` | `INV` | A call started from a channel MUST admit the members of that channel alone. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-019`) | `ambre_server::ws::handler::call::tests::a_member_joins_under_pseudonyms_the_relay_draws_per_join` -- database tier: a stranger to the channel takes no seat; `ambre_sync::call::tests::a_joiner_outside_the_channel_gets_no_wrap` -- no key reaches a joiner the log does not hold | none | verified |
| `MURMURE-594` | `CONC` | Two members who start a call in one channel at once MUST hold two calls, each named by a call message of its own. | this spec section 3, racing starts | `ambre_murmure::call_tests::two_starts_at_once_hold_two_calls`; `ambre_server::ws::handler::call::tests::two_starts_at_once_make_two_calls_each_with_its_starter_alone` -- database tier: two rooms, each roster its starter alone | none | verified |
| `MURMURE-592` | `OBS` | Given a call message with no end written, whose call the relay holds no state of, the reader MUST show the call ended. | this spec section 3, the end of a call; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-070`) | `ambre_murmure::call_tests::a_call_message_shows_who_is_in_it_then_ended_once_the_relay_holds_no_state`; `ambre_server::ws::handler::call::tests::a_call_whose_last_seat_stops_its_heartbeat_reads_as_gone` -- database tier; `ambre_belt::murmure::tests::a_call_whose_last_participant_crashed_reads_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-601` | `OBS` | Given a microphone that the browser refuses or does not have, the header MUST show that the call did not start, and why. | this spec section 3, the call controls | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-604` | `INV` | Each call control MUST carry an accessible name and take the keyboard focus, and mute MUST carry its pressed state. | this spec section 3, the call controls; `1-Foundations/ambre-spec-accessibility.md section 2` (`CHROME-022`) | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Start a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-408` | `POST` | Starting a call MUST post one call message in the channel once the relay seats its starter. | this spec section 3 | `ambre_murmure::call_tests::a_start_posts_one_call_message_and_leaves_the_members`; `ambre_sync::call_link::tests::a_start_is_heard_once_seated_and_the_last_leave_asks_for_the_end` -- no call message before the seat; `ambre_logic::channels::tests::a_start_and_a_last_leave_heard_write_the_call_message_and_its_end`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-409` | `POST` | The call message MUST show the members the relay seats in its call, read from the relay and never written into the message. | this spec section 3, who is in the call; `CALL-070` | `ambre_server::ws::handler::call::tests::a_member_out_of_the_call_reads_the_signed_join_of_each_seat` -- database tier; `ambre_murmure::call_tests::a_call_message_shows_who_is_in_it_then_ended_once_the_relay_holds_no_state`; `ambre_logic::channels::tests::who_is_in_a_call_reaches_the_view_and_writes_nothing`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: Carol, out of the call, reads both participants | none | verified |
| `MURMURE-410` | `FRAME` | Starting a call MUST NOT change the membership of the channel. | this spec section 3 | `ambre_murmure::call_tests::a_start_posts_one_call_message_and_leaves_the_members` -- the `meta` document reads the same after the post | none | verified |

### 1.3 Join a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-411` | `POST` | Joining from a call message MUST bring the member into that call. | this spec section 3; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-016`) | `ambre_murmure::commands::tests::a_live_call_message_offers_a_join_of_its_call`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: Bob joins from the menu of the message, and the two decode each other | none | verified |
| `MURMURE-412` | `FAIL` | Given a call that has ended, the call message MUST offer no join. | this spec section 3 | `ambre_murmure::commands::tests::a_call_that_ended_offers_no_join`; `ambre_server::ws::handler::call::tests::a_join_of_a_call_the_relay_holds_no_state_of_is_refused_as_ended` -- database tier; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox: the menu of an ended call holds no join | none | verified |
| `MURMURE-413` | `FRAME` | Joining a call MUST NOT move a read cursor. | this spec section 3 | structural -- `MessageAction::JoinCall` carries no command of the Logic thread, and the shell hands it to the call client of the page; `ambre_murmure::commands::tests::a_live_call_message_offers_a_join_of_its_call` | none | verified |

### 1.4 Show who is in the call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-595` | `POST` | A reader MUST ask the relay who is in a live call only while its view shows the channel, at most once per interval of `ambre-spec-budgets.md section 1`. | this spec section 3, who is in the call; `ZK-024` | `ambre_logic::channels::tests::a_tick_asks_who_is_in_each_live_call_once_per_interval` | none | verified |
| `MURMURE-596` | `FAIL` | Given a signed join whose member the membership log of the channel does not hold under its key, the reader MUST leave it out of who is in the call. | this spec section 3, who is in the call; `CALL-019` | `ambre_sync::call_roster::tests::a_join_the_log_does_not_hold_names_no_one_in_the_call`; `ambre_sync::call_link::tests::who_is_in_a_call_reaches_the_logic_thread_once_the_log_checks_it` | none | verified |
| `MURMURE-597` | `FRAME` | Asking who is in a call MUST NOT write into the channel. | this spec section 3, who is in the call | `ambre_logic::channels::tests::who_is_in_a_call_reaches_the_view_and_writes_nothing` -- no document moves, and nothing leaves for the relay | none | verified |

### 1.5 End a call

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-591` | `POST` | The leave the relay counts as the last of a call MUST write the end into its call message. | this spec section 3, the end of a call; `CALL-069` | `ambre_murmure::call_tests::the_last_leave_writes_an_end_every_member_reads`; `ambre_sync::call_link::tests::a_start_is_heard_once_seated_and_the_last_leave_asks_for_the_end`; `ambre_sync::call_link::tests::a_leave_with_no_answer_ends_the_call_and_writes_no_end`; `ambre_logic::channels::tests::a_start_and_a_last_leave_heard_write_the_call_message_and_its_end`; `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` -- browser tier, under Chrome and under Firefox | none | verified |
| `MURMURE-593` | `FAIL` | Given a delta that writes an end under another member's key, every client MUST refuse the delta. | this spec section 3, the end of a call; `MURMURE-094` | `ambre_murmure::call_tests::an_end_under_another_members_key_is_refused` | none | verified |
| `MURMURE-598` | `FRAME` | Writing the end MUST NOT change the call message beyond the entry of its writer. | this spec section 3, the end of a call; `MURMURE-095` | `ambre_murmure::call_tests::the_last_leave_writes_an_end_every_member_reads` -- the message reads the same but its call | none | verified |

### 1.6 The call controls

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-599` | `POST` | The call button in the header of a channel or a direct message MUST start a call in that channel. | this spec section 3, the call controls; `MURMURE-408` | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-600` | `FAIL` | Given a microphone that the browser refuses or does not have, a call gesture MUST post no call message and take no seat. | this spec section 3, the call controls; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-076`) | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-602` | `POST` | While the page is in a call, the header MUST offer mute and hang up in place of the call button. | this spec section 3, the call controls | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-603` | `POST` | Hang up MUST leave the call, so that each other member reads the member who hung up out of it. | this spec section 3, the call controls; `MURMURE-409`; `1-Foundations/ambre-spec-call-media.md section 3` (`CALL-069`) | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-605` | `FRAME` | A mute MUST NOT write into the channel. | this spec section 3, the call controls; `ZK-025` | not built | unbuilt -- lands with the `Call controls` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the interval between two asks of who is in a call is the call roster row of
  `ambre-spec-budgets.md section 1`. The participant cap and the time to live of a call are the
  call rows of that section.
* **Boundary.** the call message and its end are sealed like any message (`ZK-001`). What the relay
  learns from an ask of who is in a call is `ZK-024`. `ZK-021` and `ZK-022` bound the rest.
* **Verification.** 15 `verified`, 7 `unimplemented`. The relay half holds in the database tier. The
  engine and the two workers hold on the host, and the whole path in the browser tier, under Chrome
  and Firefox.

---

## 2. WHAT EACH USER SEES

| Ada does | Bob does | The channel holds | Each member sees | Test |
|---|---|---|---|---|
| calls Bob from the header of their direct message | joins from the message | a call message | each hears the other (`MURMURE-599`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| mutes | listens | the same message | Bob hears silence from Ada (`CALL-074`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| hangs up | stays | the same message | Bob alone in the call (`MURMURE-603`) | `ambre_belt::murmure::tests::a_call_from_the_header_of_a_direct_message_mutes_and_hangs_up` |
| calls, and her browser refuses the microphone | nothing | nothing new | Ada reads that the microphone is blocked (`MURMURE-600`, `MURMURE-601`) | `ambre_belt::murmure::tests::a_refused_microphone_starts_no_call_and_says_why` |
| starts a call | nothing | a call message | Ada in the call, and a join (`MURMURE-408`, `MURMURE-409`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| is in the call | joins from the message | the same message | Ada and Bob in the call (`MURMURE-411`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| leaves | leaves last | the message and its end | the call ended, and no join (`MURMURE-591`, `MURMURE-412`) | `ambre_belt::murmure::tests::a_call_message_shows_who_is_in_the_call_until_it_ended` |
| is alone in the call, and her browser dies | nothing | the message, no end | the call ended, one time to live later at most (`MURMURE-592`) | `ambre_belt::murmure::tests::a_call_whose_last_participant_crashed_reads_ended` |
| starts a call | starts a call at the same instant | two call messages | two calls, each with its own members (`MURMURE-594`) | `ambre_murmure::call_tests::two_starts_at_once_hold_two_calls` |

---

## 3. MODEL

**CRDT containers.** the `call` and the `call_end` of a message, in
`ambre-spec-murmure-schema.md section 3`. `call` names the call. `call_end` holds one entry per
member who wrote the end, keyed by that member, as `reactions` is.

**A start, in steps.**

1. The client mints the call identifier, its random bits from the generator of the OS.
2. It joins the call as a start. The relay creates the state of the call and seats the starter.
3. Once seated, the client posts the call message in the channel (`MURMURE-408`).

So a call message never names a call the relay did not hold. A join from the message is no start:
the relay refuses it once the state of the call is gone (`CALL-068`). The membership of the channel
admits the call, and the start writes nothing to it (`MURMURE-407`, `MURMURE-410`).

**Racing starts.** Each start draws its own identifier, so two starts at one instant make two calls
and two messages (`MURMURE-594`). Each message shows the members of its own call alone. One call per
channel would need an index of the live call of each channel on the relay.

**Who is in the call.** The relay holds the signed join of each seat (`ZK-022`). A reader asks the
relay for them, and verifies each against the membership log of the channel (`MURMURE-596`). The
message shows the members that pass. Nothing of the list is written into the message
(`MURMURE-409`).

The view ticks while it shows the channel. Each tick asks for each live call of the
held segments, once per interval at most (`MURMURE-595`).

**What the relay learns by the ask** is `ZK-024`: the session, the call and the instant. The
watching sample of the same tick already tells the relay that this session shows the channel
(`MURMURE-121`). The relay knows the calls of the channel from their
state. The ask reads that state and writes nothing.

**The end of a call.** Two sources, in order:

1. **The end written.** The relay answers a leave with whether its seat was the last (`CALL-069`).
   That client writes its end into the call message, sealed (`MURMURE-591`). Each member writes
   under its own key alone (`MURMURE-593`). A message of a final segment takes no end.
2. **The state gone.** A last participant who crashes, or loses its network, writes no end. The
   relay closes a socket that answered no ping for one time to live. It drops the state of
   the call with its last seat (`CALL-073`, `SRV-251`). The next ask answers that the relay
   holds none, and the reader shows the call ended (`MURMURE-592`).

The relay keeps no record that a call ended: the absence of its state is all it holds. So the
relay never learns which calls of a channel ended, and holds no history and no duration
(`ZK-022`). An end written by a member stays when the relay forgets the call.

**The call controls.** The header of a shown channel holds them, a direct message included. Each
control is a button with an icon of its own and an accessible name (`MURMURE-604`).

| The page | The header offers |
|---|---|
| in no call, a channel shown | the call button (`MURMURE-599`) |
| asking for the microphone | no control |
| in a call | mute and hang up, whatever channel it shows (`MURMURE-602`) |

A call gesture is the call button, or Join call in the menu of a call message. The page asks the
browser for the microphone first. Only once it holds the microphone does the page start or join
the call (`CALL-076`). So a refusal posts no call message and takes no seat (`MURMURE-600`). The
header then shows why the call did not start, until the next call gesture (`MURMURE-601`):

| The browser answers | The header shows |
|---|---|
| a refusal of the user or of a policy | the microphone is blocked |
| no capture device | no microphone is found |
| any other failure | the microphone did not open |

Mute stops the sound of the member and keeps the call. What mute does on the wire is
`CALL-074` and `CALL-075`. Hang up is a leave, so the relay takes the seat out of the roster. Each
reader of the call message then reads the member out of the call (`MURMURE-603`). A last hang-up
writes the end, as any last leave does (`MURMURE-591`).

**Code.**

| File | Holds |
|---|---|
| `crates/ambre-murmure/src/call.rs` | the call of a message, its end and who is in it |
| `crates/ambre-murmure/src/delta.rs` | the check of an end under the key of its writer |
| `crates/ambre-logic/src/calls.rs` | the start, the end and the asks of a channel |
| `crates/ambre-sync/src/call_roster.rs` | the ask of the relay and the check of each signed join |
| `crates/ambre-server/src/routes/calls.rs` | the answer of the relay to an ask |
| `crates/ambre-app/src/call.rs` | the microphone, the mute and the hang-up of the page |

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a start, from the page or the bus | a call starts, and a call message appears once the starter is seated |
| Join call, in the menu of a live call message | the member joins the call |
| the tick of a channel the view shows | each live call of the held segments is asked for, once per interval |
| the last leave | the call message shows the call ended |
| the call button in the header | the microphone opens, then a call starts in the shown channel |
| mute, during a call | the others hear silence from the member, until a second press |
| hang up, during a call | the member leaves the call |
| a refused microphone | the header shows why the call did not start |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A call message in the channel | a call outside any channel | the members would share a link by hand, and anyone holding it could join |
| A call message in the channel | a voice room that stays open in the channel | a room shows presence at every instant, and it holds a connection to the SFU for each member who waits in it |
| Calls are a feature of Murmure | a separate application for calls | a call has no document of its own, and it starts only from a channel. Its admission is the membership of that channel |
| Two racing starts make two calls | one live call per channel | the relay would keep an index of the live call of each channel, and a start that lost the race would have to join the other call |
| Who is in the call comes from the relay | each participant writes its join and leave into the message | every join would be a write into a message of another author, and the history of the channel would hold who was in each call |
| The end is written by the last leave | the relay keeps that a call ended | the relay would hold the call history of the channel past the time to live (`ZK-022`) |
| The microphone opens before the start or the join | a call that starts, then asks for the microphone | a refusal would leave a call message and a seat that no sound reaches, a silent call |
| Mute silences the track | mute stops the sender of the track | the SFU would read a mute apart from a silence (`ZK-025`) |
| A reader asks while the view shows the channel | the relay pushes the roster to every member of the channel | the relay would send who is in a call to sessions that show nothing of it |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A removed member stays in the list until it leaves.** The reader keeps the membership log it
  read, and reads it again only when a join names a member that log does not hold.
* **The label on the screen is not chosen.** The term is `call` in the corpus; the word the
  interface shows belongs to `ambre-spec-locale.md`.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the audio and video of a call are encrypted, signalled and carried | `1-Foundations/ambre-spec-call-media.md` |
| What the relay and the SFU learn of a call | `1-Foundations/ambre-spec-zero-knowledge.md` |
