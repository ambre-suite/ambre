# AMBRE -- FEATURE: MURMURE UNREAD STATE
**Type:** Vertical (feature)
**Implementation:** design-only

> The sidebar shows where something new is, and how much. The count follows the user across their
> devices, and a reaction or an edit never makes a channel unread.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-113` | `INV` | The unread count of a channel MUST count new messages alone, never reactions and never edits. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-114` | `INV` | The read cursor of a user MUST be the same on every device of that user. | this spec section 3; `ambre-spec-server.md` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Read a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-115` | `POST` | Showing the latest message of a channel MUST move the read cursor to it. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-116` | `FRAME` | Moving a read cursor MUST NOT move the cursor of another member. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Mark a message unread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-117` | `POST` | Marking a message unread MUST move the cursor to just before it. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-154` | `FRAME` | Marking a message unread MUST NOT move the cursor of another channel. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Mark everything read

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-118` | `POST` | One command MUST move the cursor of every channel to its last number. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-155` | `FRAME` | Marking everything read MUST NOT move the cursor of another member. | this spec section 3 | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Show activity at start

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-119` | `POST` | The sidebar MUST show the activity answer of the relay before any count is known. | this spec section 3; `SRV-109` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-120` | `POST` | A count MUST replace the activity mark once the summaries or the document give it. | this spec section 3; `RESID-014` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-156` | `FRAME` | Showing activity MUST NOT load a document. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the counters cost a light subscription per channel not held
  (`ambre-spec-budgets.md section 6`).
* **Boundary.** the relay holds the read cursors and answers yes or no, never a count
  (`SRV-110`, `ZK-015`). The kind of each update stays sealed.
* **Verification.** 11 `unimplemented`: no cursor is written by any client.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| posts a message | reads another channel | one new message | an unread count of 1 on the channel (`MURMURE-113`) | none |
| reacts to a message | reads another channel | one reaction | no badge moves (`MURMURE-113`) | none |
| reads a channel on her phone | -- | the cursor moved on the relay | the channel read on her laptop too (`MURMURE-114`) | none |

---

## 3. MODEL

**CRDT containers.** none. The read cursor is a relay record, one per member and document
(`ambre-spec-server.md section 3`). It lives as long as the membership: a removal deletes it
(`SRV-137`).

**Three sources, one badge.**

| Source | Gives | When |
|---|---|---|
| the relay activity answer | a mark: something is new | at start, before anything loads |
| the sealed summaries | a count of messages and mentions | as updates arrive |
| the loaded document | the exact count | when the channel opens |

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a channel reaches its latest message on screen | its cursor moves, and its badge clears |
| mark unread on a message | the channel shows unread from that message |
| mark all read | every badge clears |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client counts | a frame kind the relay reads | one more field in front of the relay, for a count the client computes itself |
| Messages alone count | every frame counts | a reaction would make a channel unread, which users of every chat product refuse |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** The cursor table exists and nothing writes it.
