# AMBRE -- FEATURE: MESSAGES
**Type:** Vertical (feature)
**Implementation:** design-only

> A member writes a message, edits it, or deletes it. Nobody else can change it: every client
> refuses a change that does not come from its author.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-078` | `INV` | A message MUST carry the author who signed the delta that created it. | this spec section 3; `ambre-spec-op-authorship.md` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-079` | `CONC` | Two peers MUST accept and refuse the same deltas on a message. | this spec section 3; `AUTHOR-007` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-223` | `INV` | A delta of a segment, a thread or an amendments document MUST travel as a Loro JSON update. | this spec section 3; `ambre-spec-murmure-schema.md section 7` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-227` | `INV` | A JSON update MUST carry the Loro JSON schema version the schema register pins. | this spec section 3; `ambre-spec-murmure-schema.md section 7` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Send a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-080` | `POST` | A sent message MUST be appended to the active segment with a fresh `MessageId` and its author. | this spec section 3; `MURMURE-002` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-256` | `POST` | A sent message MUST create its `reactions` map, empty. | this spec section 3; `MURMURE-006` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-081` | `POST` | A sent message MUST carry an activity summary of kind content, with its mentions. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-082` | `FAIL` | Given a delta that creates a message under another author, every client MUST refuse the delta. | this spec section 3 | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-083` | `FRAME` | Sending a message MUST NOT change another message. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-084` | `POST` | An edit MUST change the content of the message and mark it edited. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-085` | `FAIL` | Given a delta that edits a message of another author, every client MUST refuse the delta. | this spec section 3 | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-253` | `FAIL` | Given a delta that amends a message of another author, every client MUST refuse the delta. | this spec section 3; `MURMURE-073` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-086` | `FRAME` | Editing a message MUST NOT change its author, its identity or its reactions. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Delete a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-087` | `POST` | A delete MUST mark the message deleted and empty its content. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-088` | `POST` | A delete of a message in a locked segment MUST write an amendment with `by: author`. | this spec section 3; `features/ambre-spec-murmure-segments.md` | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-089` | `FAIL` | Given a delta that deletes a message of another author, every client MUST refuse the delta. | this spec section 3 | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-090` | `FRAME` | Deleting a message MUST NOT delete its thread. | this spec section 3 | not built | unbuilt -- lands with the `Message edits and deletes` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Receive a delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-224` | `FAIL` | Given a binary delta on a segment, a thread or an amendments document, every client MUST refuse it. | this spec section 3; `MURMURE-223` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-225` | `POST` | An accepted delta MUST be imported exactly as the check read it. | this spec section 3 | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-255` | `POST` | The check MUST accept a delta that sets the absent `thread` field of another author's message, and nothing else of it. | this spec section 3; `MURMURE-107` | not built | unbuilt -- lands with the `Reactions, mentions and threads` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-228` | `FAIL` | Given a JSON update of a schema version the client does not compile, the client MUST refuse it and ask the user to update. | this spec section 3; `MURMURE-019` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-226` | `FRAME` | Checking a delta MUST NOT copy its document. | this spec section 3; `ambre-spec-budgets.md section 1` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the authorship check reads each received delta with no document, a row of
  `ambre-spec-budgets.md section 1`. A delta weighs about 1.4 times its binary form on the wire.
* **Boundary.** a message is sealed like any delta (`ZK-001`), and its kind rides in the sealed
  summary (`SYNC-113`). The relay never learns whether a frame is a message, an edit or a delete.
* **Verification.** 22 `unimplemented`: no message exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The segment holds | Each user sees | Test |
|---|---|---|---|---|
| sends a message | -- | one message, authored by Alice | the message, by Alice | none |
| edits her message | reacts to it at the same instant | the edit and the reaction | the edited message, with Bob's reaction (`MURMURE-086`) | none |
| -- | Mallory sends a delta that edits Alice's message | Alice's message unchanged | the message as Alice wrote it (`MURMURE-085`) | none |
| deletes a message that has replies | -- | a deleted message, and its thread | the replies stay reachable (`MURMURE-090`) | none |

---

## 3. MODEL

**CRDT containers.** the message map of `ambre-spec-murmure-schema.md section 3`: `id`, `author`,
`ts`, `content`, `attachments`, `thread`, `reactions`, `edited` and `deleted`.

**Every rule reads the delta alone.** A Loro container identifier carries the PeerID that created
it. The `author` of a message and the key of a reaction are values inside the operation. So the
check needs the operations of the delta, and never the state of the segment.

**A delta travels as a Loro JSON update** (`MURMURE-223`). A binary update opens only by an import
into a document, and a copy of the segment costs 52 ms at 5000 messages. The JSON form reads with
no document, in microseconds.

**The authorship check, in steps.** It runs on each received delta, before import.

1. Parse the JSON update. A binary delta, or an unknown schema version, is refused (`MURMURE-224`,
   `MURMURE-228`).
2. Read its operations: each carries its container and its value.
3. Refuse the delta when it creates a message under another author (`MURMURE-082`).
4. Refuse it when it changes a field, the `content` or the `attachments` of a message created by
   another author's PeerID (`MURMURE-085`, `MURMURE-089`). One field is open to all: the `thread`
   field, set once by the first reply, while it is absent (`MURMURE-255`).
5. Refuse it when it writes a reaction under another user's key (`MURMURE-094`). The `reactions`
   map is the one container of a message that others write, entry by entry.
6. On an amendments document, read the message in its locked segment. Refuse the delta when the
   segment does not hold it, or when its `author` is another user (`MURMURE-013`, `MURMURE-253`).
7. Import otherwise, from the update the check read (`MURMURE-225`).

An amendment names its message by identifier, in a document its author did not create. So the
check reads the author from the locked segment itself, which is immutable and cheap to load again.

A PeerID resolves to its user through `ambre-spec-op-authorship.md`. Every client runs the same
check on the same delta, so every client reaches the same verdict (`MURMURE-079`).

**Moderation is the one exception.** Only the author changes a message, except a removal by a
moderator whose standing the membership log proves (`features/ambre-spec-murmure-moderation.md`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| Enter in the composer | the message is sent |
| edit on one's own message | the message opens in the composer |
| up arrow in an empty composer | the last own message opens for edit |
| delete on one's own message | the message shows as deleted |
| the same actions on another member's message | not offered |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The check runs before import | a filter at display | the filter detects and prevents nothing: a last-writer-wins value is already overwritten |
| A delta travels as a JSON update | a fork of the segment per delta | a fork encodes and decodes the whole segment: 52 ms at 5000 messages on a desktop |
| A delta travels as a JSON update | a decoder of our own for the binary format | the binary format is internal to Loro, and it would break at each upgrade |
| A delta travels as a JSON update | smaller segments | a fork still costs 3.5 ms at 500 messages on a desktop, and the phone budget is 5 ms |
| A delete empties the content | a tombstone that keeps the text | the text would stay readable to every member, against the wish of its author |
| Only the author changes a message | a moderator role in the same check | a role lives in another document, and a revoke concurrent with a delete has no single answer without the relay |
| A late edit goes to an amendments document | new deltas on the final segment | every old segment would take concurrent writes again, and its search index would turn mutable |
| A late edit goes to an amendments document | a new final snapshot on each edit | a typo fix would take the finalize lease and a full index rebuild |
| Amendments sit beside their segment | amendments in the active segment | opening an old segment would need every later segment, to find its amendments |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No message, and no authorship check, exists.
* **The JSON form costs bytes.** A message delta weighs 346 bytes compressed, against 214 for the
  binary form. The figure comes from zlib, and the envelope compresses with zstd.
