# AMBRE -- FEATURE: MURMURE NOTIFICATIONS IN AN OPEN TAB
**Type:** Vertical (feature)
**Implementation:** design-only

> A member working in another tab learns that someone mentioned them or wrote to them directly.
> The client decides, with the content in hand, and the relay learns nothing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-142` | `INV` | A notification MUST be raised only while a tab of the client is open. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-143` | `INV` | A notification MUST be raised for a mention of the user or a direct message, and for nothing else. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Raise a notification

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-144` | `POST` | A notification MUST come from the sealed summary, with no document loaded. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-145` | `FAIL` | Given a permission the browser refuses, the client MUST keep the badges and raise nothing. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-146` | `FRAME` | A notification MUST NOT be raised for a message of the channel the user is reading. | this spec section 3 | not built | unbuilt -- lands with the `Notifications in an open tab` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a notification reads a summary the client already opened.
* **Boundary.** the client decides from the sealed summary (`SYNC-113`), so the relay learns
  nothing. Push notifications, which put a list in front of the relay, are a later Story.
* **Verification.** 5 `unimplemented`: no notification exists in the code.

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

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the first mention, when no permission was asked | the browser asks for the permission once |
| a click on a notification | the channel opens at the message |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client decides | the relay filters | the relay would learn the mentions, which only a push notification needs |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No notification exists.
* **Nothing notifies with every tab closed.** That is the `Push notifications` Story (`ambre-roadmap.md` E5).
