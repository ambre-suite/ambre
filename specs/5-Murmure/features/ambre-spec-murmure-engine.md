# AMBRE -- FEATURE: THE MURMURE ENGINE
**Type:** Vertical (feature)
**Implementation:** design-only

> A channel opens, shows its latest messages, and carries what the user writes to every member.
> The engine holds the documents of a channel on the Logic thread and speaks to the interface in
> typed commands.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-021` | `INV` | A channel MUST be held on the Logic thread as the set of its documents, never merged into one document. | this spec section 3; `ambre-spec-murmure-schema.md section 2` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-022` | `INV` | An open channel MUST hold its `meta` document, its active segment and its closing segment. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-023` | `INV` | An older segment MUST be loaded only when the view asks for a message it holds. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-024` | `INV` | A remote delta MUST reach the document its `document_id` names, and no other. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-025` | `POST` | Opening a channel MUST subscribe to its `meta` document and to its live segments. | this spec section 3; `SRV-047` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-026` | `POST` | Opening a channel MUST pin the documents the view shows. | this spec section 3; `RESID-003` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-027` | `FAIL` | Given a document whose header names another channel, the engine MUST refuse it and warn. | this spec section 3; `MURMURE-001` | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-028` | `FRAME` | Opening a channel MUST NOT load a final segment. | this spec section 3 | not built | unbuilt -- lands with the `ARCH P7` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Apply a remote delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-029` | `POST` | A remote delta MUST pass the authorship check of the messages feature before it is imported. | this spec section 3; `features/ambre-spec-murmure-messages.md` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-030` | `FRAME` | A refused delta MUST NOT change any document of the channel. | this spec section 3; `AUTHOR-014` | not built | unbuilt -- lands with the `Message authorship` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the first display of a channel loads its `meta` document and its active segment, about
  2 MB, inside the memory rows of `ambre-spec-budgets.md section 6`. The authorship check costs the
  fork row of `ambre-spec-budgets.md section 1`.
* **Boundary.** every document here travels sealed through the document pipeline (`ZK-001`). What
  the relay learns of the grouping is `ZK-015`.
* **Verification.** 10 `unimplemented`: `crates/ambre-murmure` is a `lib.rs`.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The engine holds | Each user sees | Test |
|---|---|---|---|---|
| opens a channel of two years | -- | `meta` and the active segment | the latest messages at once, and older ones on scroll (`MURMURE-023`) | none |
| sends a message | has the channel open | the message in the active segment, on both clients | the message on both screens | none |
| -- | Mallory sends a delta that rewrites Alice's message | nothing new: the check refuses it | Alice's message as she wrote it (`MURMURE-029`) | none |

---

## 3. MODEL

**CRDT containers.** the five kinds of `ambre-spec-murmure-schema.md section 3`, one Loro document
each.

**The state of an open channel.**

```rust
pub struct ChannelState {
    pub channel_id: ChannelId,
    pub meta: LoroDoc,
    pub active: LoroDoc,
    pub closing: Option<LoroDoc>,
    pub loaded: BTreeMap<DocumentId, LoroDoc>,   // older segments, on demand
    pub threads: BTreeMap<DocumentId, LoroDoc>,  // the threads the view opened
}
```

**The command surface.** One content-plane variant, addressed to a channel (`BUS-027`).

```rust
pub enum MurmureCommand {
    SendMessage { draft: MessageDraft },
    EditMessage { segment: DocumentId, message: MessageId, content: TextDelta },
    DeleteMessage { segment: DocumentId, message: MessageId },
    React { segment: DocumentId, message: MessageId, emoji: Emoji },
    Unreact { segment: DocumentId, message: MessageId, emoji: Emoji },
    OpenThread { root: MessageRef },
    LoadOlder { before: DocumentId },
    MarkRead { document: DocumentId, seq: Seq },
}
```

**A remote delta, in steps.**

1. The Sync worker opens the frame and forwards the delta, the nonce, the summary and the signer.
2. The engine finds the document by its `document_id` (`MURMURE-024`).
3. The authorship check of the messages feature runs on a fork (`MURMURE-029`).
4. The delta is imported, and the view is told which messages changed.

**Code.** not built. `crates/ambre-murmure/src/lib.rs` is a stub.

---

## 4. SURFACE

The engine has no control of its own. It answers the commands the views send.

| Trigger | Behaviour |
|---|---|
| a channel is selected | the channel opens, and its documents are pinned |
| the view scrolls past the oldest loaded message | the previous segment loads |
| the view closes a channel | its documents lose their pin, and residency may evict them |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A channel is a set of documents | one Loro document per channel | a channel of two years weighs tens of megabytes of tombstones, all loaded to show the last message |
| Older segments load on demand | every segment loaded at open | the first display would wait on the whole history |
| Typed commands on the content plane | opaque bytes on a channel plane | a byte payload is a surface the compiler cannot check, which `BUS-002` refuses for every other application |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** The crate is a stub, and the protocol still carries the channel plane.
* **Two channels side by side** are in `features/ambre-spec-murmure-layout.md`: each pane keeps its
  channel in view, and residency bounds the rest by bytes.
