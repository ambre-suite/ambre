# AMBRE -- FEATURE: MURMURE NOTIFICATIONS
**Type:** Vertical (feature)
**Implementation:** design-only

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
| `MURMURE-142` | `INV` | Without a push subscription, a notification MUST be raised only while a tab of the client is open. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-143` | `INV` | A notification MUST be raised for a mention of the user or a direct message, and for nothing else. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-265` | `INV` | Every push MUST raise a visible notification. | this spec section 3 | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-266` | `INV` | A push MUST carry no content: it names a mention or a direct message, and nothing more. | this spec section 3; `ZK-016` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-267` | `INV` | A channel past the member threshold of the budget register MUST default to the mentions level, and any other channel to all. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Push notifications` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Raise a notification

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-144` | `POST` | A notification MUST come from the sealed summary, with no document loaded. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-145` | `FAIL` | Given a permission the browser refuses, the client MUST keep the badges and raise nothing. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-146` | `FRAME` | A notification MUST NOT be raised for a message of the channel the user is reading. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |

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
* **Verification.** 15 `unimplemented`: no notification exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| mentions Bob | works in another tab | one message | a system notification for Bob (`MURMURE-143`) | none |
| writes in a channel without mentioning Bob | works in another tab | one message | a badge, and no notification | none |
| mentions Bob | reads that channel | one message | no notification (`MURMURE-146`) | none |

---

## 3. MODEL

**CRDT containers.** none.

**The decision, in steps.**

1. The Sync worker opens an update and reads its summary.
2. A mention of the user, or any message in a direct message, raises a notification.
3. The text shown is decrypted on the client, never by a server.

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

**Code.** not built.

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

* **Nothing is built.** No notification exists.
