# AMBRE -- FEATURE: MURMURE TYPING AND PRESENCE
**Type:** Vertical (feature)
**Implementation:** design-only

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
| `MURMURE-121` | `INV` | The typing and presence indicators MUST ride the ephemerals of the `meta` document. | this spec section 3; `ambre-spec-presence.md` | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-122` | `INV` | Both indicators MUST be on by default. | this spec section 3 | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-123` | `INV` | Both indicators MUST reach the members of the channel alone. | this spec section 3; `SRV-047` | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Type in a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-124` | `POST` | Typing MUST refresh the indicator while the member types, and let it expire once they stop. | this spec section 3 | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-125` | `FRAME` | A typing indicator MUST NOT reach the history of any document. | this spec section 3; `SRV-089` | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Watch a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-126` | `POST` | A member MUST show as present while a connection of theirs watches the channel. | this spec section 3; `SRV-097` | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-127` | `FRAME` | Presence MUST NOT show which other channel a member reads. | this spec section 3 | not built | unbuilt -- lands with the `Typing and presence in a channel` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Turn the indicators off

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-296` | `POST` | Turning the indicators off MUST stop the client from sending typing and presence samples. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Presence settings` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-297` | `FRAME` | Turning the indicators off MUST NOT hide the indicators of other members. | this spec section 3 | not built | unbuilt -- lands with the `Presence settings` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the presence write of `ambre-spec-budgets.md section 3`, per heartbeat.
* **Boundary.** both indicators are sealed ephemerals (`ZK-005`). The relay learns when a
  member is active, which it knows from the connection anyway.
* **Verification.** 9 `unimplemented`: no channel presence exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| types | watches the channel | nothing durable | "Alice is typing" on Bob's screen (`MURMURE-124`) | none |
| stops typing | watches | nothing | the indicator expires | none |
| reads channel A | reads channel B | nothing | Bob sees nothing of Alice in B (`MURMURE-127`) | none |

---

## 3. MODEL

**CRDT containers.** none. Both indicators are ephemerals of the `meta` document, with the
expiry of `ambre-spec-presence.md`.

**The setting that turns them off** lives in the `presence` map of the user settings. With it off,
the client sends no typing and no presence sample, on any device (`MURMURE-296`). The relay then
learns nothing of when the member is active. The member still sees the others (`MURMURE-297`).

**Code.** not built.

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

* **Nothing is built.** No channel presence exists.
