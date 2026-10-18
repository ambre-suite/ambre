# AMBRE -- FEATURE: MURMURE NOTIFICATIONS
**Type:** Vertical (feature)
**Implementation:** partial -- the notifications of an open tab are built. The levels and the push
are not.

> A member learns that someone mentioned them or wrote to them directly. In an open tab the client
> decides with the content in hand. With every tab closed, the relay wakes the device, knowing only
> who was named.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-142` | `INV` | Without a push subscription, a notification MUST be raised only while a tab of the client is open. | this spec section 3 | `ambre_canvas::frame::tests::a_hidden_page_relays_on_its_timer_and_a_shown_one_does_not` -- a hidden tab still drains its events, so it raises; type-level -- the workspace enables no service-worker type of `web-sys`, so nothing outside a tab can raise one | none | verified |
| `MURMURE-143` | `INV` | A notification MUST be raised for a mention of the user or a direct message. | this spec section 3 | `ambre_murmure::tests::chrome_relay::a_mention_or_a_direct_message_notifies_and_a_plain_message_does_not` -- a mention and a direct message each notify under their title, and a plain public message does not | none | verified |
| `MURMURE-493` | `INV` | When its channel is at the all level, a notification MUST be raised for any other message. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-494` | `INV` | A channel of the user never opened in the session MUST notify its first message, as an opened one does. | this spec section 3; `MURMURE-144`; `RESID-022` | `ambre_logic::unopened::tests::the_first_message_of_a_channel_never_opened_notifies` -- a listed channel no session held is watched once its segments are known, and its first message is news; `ambre_logic::unopened::tests::a_channel_never_opened_is_watched_once_listed_and_answered` -- the watch waits on both the listing and the activity | none | verified |
| `MURMURE-265` | `INV` | Every push MUST raise a visible notification. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-266` | `INV` | A push MUST carry no content: it names a mention or a direct message, and nothing more. | this spec section 3; `ZK-016` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-267` | `INV` | A channel past the member threshold of the budget register MUST default to the mentions level, and any other channel to all. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Raise a notification

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-144` | `POST` | A notification MUST come from the sealed summary, with no document loaded. | this spec section 3; `RESID-016` | `ambre_logic::channels::tests::a_message_of_another_member_is_news_and_names_a_mention` -- the summary of an open channel's delta says news and mention, the user's own message is none; `::a_closed_channel_counts_its_mentions_from_the_summaries` -- a closed channel's counters say it, no document loaded | none | verified |
| `MURMURE-145` | `FAIL` | Given a permission the browser refuses, the client MUST keep the badges and raise nothing. | this spec section 3 | `ambre_app::notify::tests::a_refused_permission_raises_nothing_and_an_open_one_is_asked_once` -- browser tier, a refused permission raises nothing, and an unanswered one is asked once | none | verified |
| `MURMURE-146` | `FRAME` | A notification MUST NOT be raised for a message of the channel the user is reading. | this spec section 3 | `ambre_murmure::tests::chrome_relay::the_channel_being_read_notifies_nothing_until_the_page_is_hidden` -- the channel on screen notifies nothing, and the same channel behind a hidden page does | none | verified |

### 1.3 Set the level of a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-268` | `POST` | Changing the level of a channel MUST write the setting, then its copy on the relay. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-269` | `FRAME` | Changing the level of a channel MUST NOT change the level of another. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Push with every tab closed

The relay decides which device to wake, from a copy of the levels and from the members a message names.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-270` | `POST` | A message that mentions members MUST name them to the relay in its routing header, and a message that mentions the channel MUST say so. | this spec section 3; `ZK-016` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-271` | `POST` | The relay MUST push a message to a member whose level is all, or mentions when the message names them or the channel. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-272` | `FRAME` | The relay MUST NOT push to a device that holds an open connection. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-273` | `FAIL` | Given a subscription the push service refuses, the relay MUST drop that subscription. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-274` | `POST` | Signing out MUST remove the push subscription of the device. | this spec section 3; `SETTINGS-014` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a notification reads a summary the client already opened.
* **Boundary.** in an open tab the client decides from the sealed summary (`SYNC-113`), so the
  relay learns nothing. A push is the one path that tells the relay the levels and the members a
  message names, and `ZK-016` bounds it to those.
* **Verification.** 6 `verified`, 11 `unimplemented`. Logic reads the news off the summaries, a
  host test. The view decides which news notifies, and the page raises it: two browser-tier tests.
  The levels and the push are not built.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| mentions Bob | works in another tab | one message | a system notification for Bob (`MURMURE-143`) | `ambre_murmure::tests::chrome_relay::a_mention_or_a_direct_message_notifies_and_a_plain_message_does_not` |
| writes in a channel without mentioning Bob | works in another tab | one message | a badge, and no notification | `ambre_murmure::tests::chrome_relay::a_mention_or_a_direct_message_notifies_and_a_plain_message_does_not` |
| mentions Bob | reads that channel | one message | no notification (`MURMURE-146`) | `ambre_murmure::tests::chrome_relay::the_channel_being_read_notifies_nothing_until_the_page_is_hidden` |

---

## 3. MODEL

**CRDT containers.** none.

**The decision, in steps.**

1. The Sync worker opens an update and hands its summary to Logic.
2. Logic sends news of a message from another member, and says whether it names the user.
3. The view raises a notification for a mention or a direct message, unless that channel is on a
   visible screen.
4. The text shown is decrypted on the client, never by a server.

A hidden tab gets no animation frame, so a timer runs the relay instead (`CANVAS-075`).

**A push, in steps.**

1. The sender client names, in the routing header, the members its message mentions, or the channel
   for an `@channel` (`MURMURE-270`).
2. The relay reads the copy of each member's level, and picks who to wake (`MURMURE-271`).
3. It skips every device with an open connection: that tab notifies on its own (`MURMURE-272`).
4. It sends one push through the Web Push service, encrypted to the subscription. The push says
   "a mention" or "a direct message", never the text (`MURMURE-266`).
5. The service worker shows it at once, since a push that shows nothing is refused
   (`MURMURE-265`). A click opens the client, which decrypts the message.

**The level has two copies.** The user settings hold the level the interface reads (`MURMURE-268`).
The relay holds a copy, since it alone decides which push to send. A large channel starts at
mentions, so a busy room wakes nobody for every line (`MURMURE-267`).

**Code.** `ambre-logic` sends `UiEvent::NewMessage` from `channels.rs` and `threads.rs`. The view
decides in `MurmureRelay::notice` of `ambre-murmure`. The page raises it in `notify.rs` of
`ambre-app`, and `ambre-canvas` runs the hidden tab's timer.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the first mention, when no permission was asked | the browser asks for the permission once |
| a click on a notification | the channel opens at the message |
| the level of a channel, from its menu | all, mentions, or none |
| a push, with every tab closed | a notification that names a mention or a direct message |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client decides | the relay filters | the relay would learn the mentions, which only a push notification needs |
| The relay picks who to push | the client filters each push after decryption | Web Push requires a visible notification for each push, and Safari cancels a subscription after three silent ones |
| A push carries no content | the text, decrypted by the service worker | the worker would hold the keys with no tab open, a store of keys the rest of the design never leaves at rest |
| The sender names the mentioned members | a push to every member at the mentions level | every member would be woken by every message, which is what the level exists to prevent |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The levels and the push are not built.** No channel has a level, and no tab closed notifies.
  They land with the `Push notifications` Story (`ambre-roadmap.md` E5).
* **A rotation missed offline leaves a channel never opened under its old key.** A notice lost
  while the socket is down leaves its new summaries counted as activity alone. It lands with
  the `A rotation missed by a light subscription` Story (`ambre-roadmap.md` E5).
* **Each channel never opened costs a subscription per live segment.** No ceiling bounds them yet.
  It lands with the `A ceiling on the channels watched unopened` Story (`ambre-roadmap.md` E5).
* **A click opens the channel, not the message.** The list scrolls to it once the message list
  lands, the `The message list and the composer` Story (`ambre-roadmap.md` E5).
* **A long-hidden tab may lag.** A browser may slow the timers of a tab hidden for minutes, which
  delays a notification. The `Wake a hidden tab from the worker` Story (`ambre-roadmap.md` E5)
  owns it.
