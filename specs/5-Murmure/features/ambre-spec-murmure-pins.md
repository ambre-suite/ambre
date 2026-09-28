# AMBRE -- FEATURE: PINNED MESSAGES
**Type:** Vertical (feature)
**Implementation:** design-only

> A member pins a message for every member of the channel: a decision, a link, the rules of the
> room. The pinned list opens without loading the history around it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-338` | `INV` | A pin MUST be one entry of the `pinned` map of the channel, keyed by its message. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-339` | `OBS` | The pinned list MUST show at most the pin cap of the budget register, newest first. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Pin a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-340` | `POST` | Pinning a message MUST add its entry, naming its segment, for every member of the channel. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-341` | `FAIL` | Given a channel at its pin cap, the client MUST refuse a new pin and say so. | this spec section 3; `MURMURE-339` | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-342` | `FRAME` | Pinning a message MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Unpin a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-343` | `POST` | Unpinning a message MUST remove its entry. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-344` | `FRAME` | Unpinning a message MUST NOT remove another pin. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Show the pins

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-345` | `POST` | The pinned list MUST show the pins newest first, loading their segments alone. | this spec section 3; `MURMURE-023` | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-346` | `FAIL` | Given a pin whose message was deleted, or whose segment expired, the list MUST leave it out. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-347` | `FRAME` | Showing the pins MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `Pinned messages` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a pin is one small entry of `meta`, and the pin cap of `ambre-spec-budgets.md
  section 6` bounds the list.
* **Boundary.** the pins live in the sealed `meta` document (`ZK-001`). The relay never learns
  which message is pinned.
* **Verification.** 10 `unimplemented`: no pin exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| pins the decision of the week | -- | one pin | the message in the pinned list (`MURMURE-340`) | none |
| pins a message | pins the same message at once | one entry, keyed by the message | one pin, not two (`MURMURE-338`) | none |
| pins the fiftieth message | pins another at the same moment | fifty-one entries | the fifty newest pins (`MURMURE-339`) | none |
| deletes a pinned message | opens the pinned list | the pin, and the deleted message | no line for it (`MURMURE-346`) | none |

---

## 3. MODEL

**CRDT containers.** the `pinned` map of the `meta` document, in
`ambre-spec-murmure-schema.md section 3`: one entry per message, naming its segment and the time it
was pinned.

**A map, not a list.** Two members pinning one message at once write one key, so the channel holds
one pin (`MURMURE-338`). A list would hold it twice, and two unpins would race for one slot.

**Pins open alone.** A pin names its segment, so the client loads that segment and no other
(`MURMURE-345`). A final segment is immutable and cheap to load again.

**A member who can write pins.** A reader cannot, since the relay refuses the write (`SRV-008`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| pin, on a message | the message joins the pinned list of the channel |
| unpin, on a pinned message | the message leaves the list |
| the pin icon of the channel | the pinned list, newest first |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A map keyed by the message | a list of pins | two members pinning one message at once would pin it twice |
| A pin is shared | a pin per member | that is a bookmark, which `features/ambre-spec-murmure-sidebar.md` holds |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No pin exists.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How a bookmark, which is personal, works | `features/ambre-spec-murmure-sidebar.md` |
| How a segment is loaded | `features/ambre-spec-murmure-engine.md` |
