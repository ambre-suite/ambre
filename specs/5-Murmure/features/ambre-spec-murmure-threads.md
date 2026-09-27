# AMBRE -- FEATURE: MURMURE THREADS
**Type:** Vertical (feature)
**Implementation:** design-only

> A member replies to a message, and the replies gather in a thread beside the channel. The root
> shows how many replies it has, even when nobody opened the thread.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-105` | `INV` | The replies to a message MUST live in one thread document per root message. | this spec section 3; `ambre-spec-murmure-schema.md section 2` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-106` | `INV` | The reply count and the last replier of a root MUST be read from its thread document. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-108` | `CONC` | Two peers sending the first reply at once MUST converge on one thread document. | this spec section 3; `ambre-spec-murmure-schema.md section 6` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-261` | `INV` | A mention in a reply MUST count on the channel of its thread. | this spec section 3; `MURMURE-233` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-290` | `INV` | A followed thread MUST stay in the Threads view until the user unfollows it, never leaving on a delay. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Reply in a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-107` | `POST` | The first reply to a message whose `thread` field is absent MUST set it to the thread document. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-109` | `POST` | A reply also sent to the channel MUST write a copy in the active segment that names its thread. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-110` | `FRAME` | Replying MUST NOT change the root message beyond its `thread` field. | this spec section 3 | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Follow a thread that is not open

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-111` | `POST` | A client MUST reload the head of a thread when the relay signals activity on it and its root is on screen. | this spec section 3; `SRV-100` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-260` | `POST` | An activity signal on a thread MUST make the client fetch the summaries of its new deltas, with no document loaded. | this spec section 3; `RESID-014`, `SRV-100` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-112` | `FRAME` | An activity signal MUST NOT load a thread whose root is off screen. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Follow a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-291` | `POST` | Writing the root, replying, or being mentioned in a thread MUST follow it. | this spec section 3; `SETTINGS-004` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-292` | `POST` | The Threads view MUST list every followed thread with unread replies, newest first. | this spec section 3; `MURMURE-260` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-293` | `POST` | Marking a thread read MUST move its read cursor to its last reply. | this spec section 3; `MURMURE-114` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-294` | `POST` | Unfollowing a thread MUST remove it from the Threads view and from the user settings. | this spec section 3 | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-415` | `FAIL` | Given a followed thread whose document expired, the client MUST drop it from the Threads view and from the user settings. | this spec section 3; `SRV-156` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-295` | `FRAME` | Following a thread MUST NOT subscribe the client to it. | this spec section 3; `MURMURE-112` | not built | unbuilt -- lands with the `Followed threads` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a thread head is a small read per visible root, inside the memory rows of
  `ambre-spec-budgets.md section 6`.
* **Boundary.** a thread is a document of its channel, sealed like the others (`ZK-001`). The relay
  learns that a document is a thread of the channel, and when it changes (`ZK-015`, `SRV-101`).
* **Verification.** 17 `unimplemented`: no thread exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| replies to Bob's message | watches the channel | a thread document, and `thread` on the root | "1 reply" under the root, on both screens (`MURMURE-106`) | none |
| sends the first reply | sends the first reply at the same instant | one thread with both replies | one thread on both screens (`MURMURE-108`) | none |
| replies, and sends to the channel too | watches the channel | the reply in the thread, and a copy in the channel | the reply in the channel, with a link to its thread (`MURMURE-109`) | none |

---

## 3. MODEL

**CRDT containers.** the thread document of `ambre-spec-murmure-schema.md section 3`: its `header`
names the root, and its `messages` list holds the replies.

**The first reply.** The relay creates the thread document, as it creates the successor of a
segment. Two first replies at once receive the same document (`MURMURE-108`).

**A thread not open.** The client is not subscribed to it. A frame on it makes the relay publish a
signal on the topic of the channel. The client then fetches the new deltas and reads their sealed
summaries alone (`MURMURE-260`). A mention in a reply thus counts on the channel, and can notify
(`MURMURE-261`). It reloads the head of the thread only if the root is on screen.

**Followed threads.** The `followed` map of the user settings names each thread the user follows
(`MURMURE-291`). A followed thread is not subscribed: its activity signals and their summaries
count its unread replies, as for any thread not open (`MURMURE-260`). The Threads view gathers the
followed threads with unread replies (`MURMURE-292`). A thread has a read cursor like any document,
so marking it read moves that cursor (`MURMURE-293`). A thread never archives on a delay
(`MURMURE-290`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| reply on a message | the thread opens beside the channel, caret in its composer |
| "also send to channel" in a thread composer | the reply, and its copy in the channel |
| a reply count under a root | the thread opens |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One document per thread | one `threads` document per channel | it grows with every thread of the channel for years |
| The relay creates the thread document | each peer creates its own on the first reply | two first replies at once would open two threads for one root |
| The client fetches the summaries a signal points at | a signal that names the users mentioned | the relay would learn who mentions whom in every thread |
| A followed thread stays until unfollowed | an archive after a fixed delay | Discord users find the delay too short, and lose threads they meant to keep |
| Followed threads live in the user settings | local storage in the browser | a new device would start with no followed thread |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No thread exists.
