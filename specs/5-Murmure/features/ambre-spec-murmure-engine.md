# AMBRE -- FEATURE: THE MURMURE ENGINE
**Type:** Vertical (feature)
**Implementation:** partial -- the channel state, the open over the relay, the send and the remote
delta are built

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
| `MURMURE-021` | `INV` | A channel MUST be held on the Logic thread as the set of its documents, never merged into one document. | this spec section 3; `ambre-spec-murmure-schema.md section 2` | `ambre_murmure::channel_tests::a_channel_is_held_as_its_documents_apart` -- the `meta` document and the segment carry distinct identifiers and disjoint roots; structural -- `ChannelState` holds one `LoroDoc` per document, and `LogicState` keeps channels in a registry apart from `docs` | none | verified |
| `MURMURE-022` | `INV` | An open channel MUST hold its `meta` document, its active segment and its closing segment. | this spec section 3 | `ambre_murmure::channel_tests::an_open_channel_holds_meta_the_active_and_the_closing_segment`; `ambre_murmure::channel_tests::a_live_segment_with_no_bytes_refuses_the_open` -- a live segment the caller cannot supply refuses the open rather than opening short | none | verified |
| `MURMURE-023` | `INV` | An older segment MUST be loaded only when the view asks for a message it holds. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::channel_tests::an_older_segment_loads_only_when_the_view_asks`; `::a_live_segment_is_not_an_older_one`; `::an_older_segment_asked_for_arrives_then_loads`; `::an_older_segment_nobody_asked_for_is_not_ready`; `ambre_logic::channels::tests::asking_for_the_segment_before_the_oldest_asks_nothing` | none | verified |
| `MURMURE-024` | `INV` | A remote delta MUST reach the document its `document_id` names, and no other. | this spec section 3 | `ambre_murmure::channel_tests::a_remote_delta_reaches_the_document_it_names_and_no_other` -- the other held documents compare equal before and after; `ambre_murmure::channel_tests::a_delta_for_a_document_the_channel_does_not_hold_is_refused`; `ambre_logic::channels::tests::a_remote_delta_reaches_the_channel_that_holds_its_document` -- through the Logic dispatcher | none | verified |

### 1.2 Open a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-025` | `POST` | Opening a channel MUST subscribe to its `meta` document and to its live segments. | this spec section 3; `SRV-047` | `ambre_murmure::channel_tests::opening_takes_meta_then_its_live_segments_and_never_a_final_one`; `ambre_logic::channels::tests::a_channel_opens_its_meta_then_its_live_segments_then_shows_them` | none | verified |
| `MURMURE-026` | `POST` | Opening a channel MUST pin the documents the view shows. | this spec section 3; `RESID-003` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-027` | `FAIL` | Given a document whose header names another channel, the engine MUST refuse it and warn. | this spec section 3; `MURMURE-001` | `ambre_murmure::channel_tests::a_document_naming_another_channel_is_refused`; `ambre_murmure::channel_tests::a_meta_document_of_another_channel_is_refused`; `ambre_murmure::channel_tests::a_segment_in_the_place_of_meta_is_refused`; structural -- the one branch of `schema::admit` that returns `ForeignChannel` emits the `tracing::warn!` first | none | verified |
| `MURMURE-028` | `FRAME` | Opening a channel MUST NOT load a final segment. | this spec section 3 | `ambre_murmure::channel_tests::opening_a_channel_never_loads_a_final_segment` -- the final segment's bytes are handed to the open, and it still holds none of it | none | verified |

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
* **Verification.** 7 `verified`, 3 `unimplemented`. The open is held on both sides of the Logic
  thread: the engine's staged open, and the dispatcher that drives it over the relay.

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

**The state of an open channel.** Private fields, read through methods.

```rust
pub struct ChannelState {
    channel_id: ChannelId,
    meta: Held,                          // a DocumentId and its LoroDoc
    active: Held,
    closing: Option<Held>,
    loaded: HashMap<DocumentId, LoroDoc>, // final segments, on demand
    pending: HashMap<DocumentId, Vec<Vec<u8>>>, // final segments asked for, still arriving
}
```

**An open, in steps.** The relay serves each document of a channel as an ordinary document, so the
channel opens in stages. Nothing of it is shown before the last one.

1. The Sync worker lists the documents of the channel. A non-member is refused (`SRV-047`).
2. The Logic thread opens the `meta` document, and holds its bytes until it is ready.
3. `meta` names the live segments, and the Logic thread opens each one (`MURMURE-025`).
4. The last live segment builds the `ChannelState`, and the view gets their messages.

A refusal at any step closes every document the open asked for, and the view gets
`MurmureEvent::OpenFailed`.

Threads join the state with the threads Story.

**The command surface.** One content-plane variant, `LogicCommand::Channel`, addressed to a channel
(`BUS-027`). Each command arrives with the Story that builds it.

```rust
pub enum MurmureCommand {
    SendMessage { draft: MessageDraft, now: Timestamp },
    LoadOlder { segment: SegmentId },
}
```

The view stamps `now`, because the Logic thread reads no clock. A send hands the Sync worker the
delta and its activity summary, and the view a `MurmureEvent::Messages`.

**A remote delta, in steps.**

1. The Sync worker opens the frame and forwards the delta, the nonce, the summary and the signer.
2. The engine finds the document by its `document_id` (`MURMURE-024`).
3. The authorship check of the messages feature runs on a fork (`MURMURE-029`).
4. The delta is imported, and the view is told which messages changed.

**Code.** `crates/ambre-murmure/src/channel.rs`, `crates/ambre-murmure/src/schema.rs`,
`crates/ambre-murmure/src/opening.rs`, `crates/ambre-bus/src/app/murmure.rs`,
`crates/ambre-logic/src/channels.rs`.

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

* **No view opens a channel yet.** The command exists, and the message list that sends it is a
  later Story.
* **A channel is not persisted locally.** Its documents take residency, a later Story.
* **Two channels side by side** are in `features/ambre-spec-murmure-layout.md`: each pane keeps its
  channel in view, and residency bounds the rest by bytes.
