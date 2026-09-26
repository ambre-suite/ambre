# AMBRE -- FEATURE: PRIVATE CHANNELS AND DIRECT MESSAGES
**Type:** Vertical (feature)
**Implementation:** design-only

> A few people talk where nobody else reads: a private channel of a space, or a direct message
> between two or more members. Opening a direct message with the same people finds the one that
> exists.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-046` | `INV` | A direct message MUST belong to no space. | this spec section 3; `ambre-spec-murmure-schema.md section 3` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-047` | `INV` | Two direct messages MUST NOT exist for one set of members. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Create a private channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-048` | `POST` | Creating a private channel MUST make its creator its owner and its only member. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-049` | `POST` | A private channel MUST name its space to the relay. | this spec section 3; `ZK-015` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-050` | `FRAME` | Creating a private channel MUST NOT write the catalogue of its space. | this spec section 3; `MURMURE-014` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Open a direct message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-051` | `POST` | Opening a direct message with a set of members MUST open the existing one when it exists. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-052` | `POST` | A new direct message MUST wrap its key for each of its members alone. | this spec section 3; `SYNC-010` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-053` | `FRAME` | Opening a direct message MUST NOT add a member to an existing one. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Name a direct message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-054` | `POST` | A name a member sets MUST be stored in the `meta` document, and read by every member. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-055` | `FRAME` | Clearing a name MUST remove its key, and MUST NOT store an empty or a composed name. | this spec section 3; `MURMURE-016` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Invite a member

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-056` | `POST` | An invite MUST open the history since the latest history boundary to the invitee, through the key chain. | this spec section 3; `SYNC-119` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-057` | `POST` | A third member joining a direct message of two MUST be announced to both members before the invite applies. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-058` | `FRAME` | An invite MUST NOT hand the invitee a key older than the latest history boundary. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Leave a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-059` | `POST` | Leaving a private channel or a direct message MUST rotate its key. | this spec section 3; `ambre-spec-key-rotation.md` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-060` | `POST` | A rotation MUST close the active segment, so the successor opens under the new key. | this spec section 3; `features/ambre-spec-murmure-segments.md` | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-061` | `FRAME` | Leaving a public channel MUST NOT rotate its key. | this spec section 3 | not built | unbuilt -- lands with the `Private channels and direct messages` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** an invite wraps one key. The invitee then walks the key chain back to the boundary,
  one small open per rotation, charged to the crypto rows of `ambre-spec-budgets.md section 1`. A
  rotation costs one close and one wrap per remaining member.
* **Boundary.** the relay learns the members of each channel, which `ambre-spec-zero-knowledge.md`
  accepts as the membership graph. The name of a direct message is sealed in `meta`.
* **Verification.** 16 `unimplemented`: no channel exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| opens a direct message with Bob | opens a direct message with Alice | one direct message | the same conversation on both screens (`MURMURE-051`) | none |
| names the direct message "Budget" | -- | the name in `meta` | "Budget" on both screens | none |
| invites Carol into the direct message | -- | a notice, then Carol as a member | both told before Carol joins (`MURMURE-057`) | none |
| leaves a private channel | writes a message | a new key, and a new active segment | Bob's message unreadable to Alice (`MURMURE-059`) | none |

---

## 3. MODEL

**CRDT containers.** the `meta` document of `ambre-spec-murmure-schema.md section 3`: its
`channel` map holds the type, the optional name and the topic.

**A direct message is reused.** The relay holds the members of every channel, so it finds a direct
message whose members are exactly the requested set. It learns nothing new by doing so
(`MURMURE-051`).

**No forward secrecy.** A direct message keeps every key, like any channel. Each final segment stays
sealed under the key of its time, and the key chain reaches it. What protects it is the rotation on
a leave.

**An invite, in steps.**

1. The inviting client wraps the current key for the invitee, and its signed entry names that key
   (`SYNC-123`).
2. It posts the wrap and the entry in one request (`SYNC-105`).
3. The invitee walks the key chain back to the latest history boundary (`SYNC-119`), so it reads
   every segment since.
4. For a direct message of two, both members see the notice before the invite applies.

**A leave, in steps.** The leaving member adds a signed entry, and the relay removes the row. An
online member client then rotates the key and closes the active segment (`MURMURE-060`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| new private channel | a channel with its creator alone, in the chosen space |
| message on a member's name | the existing direct message, or a new one |
| rename on a direct message | a name for every member, or none |
| invite into a channel | the invitee joins, with the history since the latest boundary |
| leave a channel | the channel leaves the sidebar, and a private one rotates its key |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A direct message is reused | a new one for each open | two conversations between the same people split one history |
| A rotation closes the active segment | rotating inside the segment | a segment would hold deltas under two keys, and its final snapshot under one |
| A public channel keeps its key on a leave | rotating on every leave | every member of the space reads the key in the catalogue anyway |
| A direct message is a channel | a separate model for direct messages | a conversion would copy the history into a new document, with new identifiers and a new key |
| No forward secrecy | forward secrecy for direct messages | a full history on a new device, the final snapshots and the search index all need every key |
| The channel list is the membership | a sealed document listing the channels of a user | a second list drifts after a revoke, and shows a channel whose key is gone |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No private channel and no direct message exist.
* **Converting a direct message into a channel is not ruled here.** It is the
  `Direct message conversion` Story (`ambre-roadmap.md` E5).
* **Nothing states who rotates when nobody is online.** The rotation waits for the next member
  client, and the leaving member reads nothing new meanwhile only because the relay removed the row.
