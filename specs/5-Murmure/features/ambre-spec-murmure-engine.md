# AMBRE -- FEATURE: THE MURMURE ENGINE
**Type:** Vertical (feature)

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
| `MURMURE-021` | `INV` | A channel MUST be held on the Logic thread as the set of its documents, never merged into one document. | this spec section 3; `ambre-spec-murmure-schema.md section 2` | `ambre_murmure::logic::channel_tests::a_channel_is_held_as_its_documents_apart` -- the `meta` document and the segment carry distinct identifiers and disjoint roots; structural -- `ChannelState` holds one `LoroDoc` per document, and the Murmure runtime (`ambre_murmure::logic::runtime::Runtime`) keeps channels in a registry apart from the `docs` of the Logic runtime | none | verified |
| `MURMURE-022` | `INV` | An open channel MUST hold its `meta` document, its active segment and its closing segment, save a closing one it opened without (`MURMURE-502`). | this spec section 3 | `ambre_murmure::logic::channel_tests::an_open_channel_holds_meta_the_active_and_the_closing_segment`; `ambre_murmure::logic::channel_tests::a_live_segment_with_no_bytes_refuses_the_open` -- a live segment the caller cannot supply refuses the open rather than opening short | none | verified |
| `MURMURE-023` | `INV` | An older segment MUST be loaded only when the view asks for a message it holds. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_murmure::logic::channel_tests::an_older_segment_loads_only_when_the_view_asks`; `::a_live_segment_is_not_an_older_one`; `::an_older_segment_asked_for_arrives_then_loads`; `::an_older_segment_nobody_asked_for_is_not_ready`; `ambre_logic::channels::tests::asking_for_the_segment_before_the_oldest_asks_nothing` | none | verified |
| `MURMURE-024` | `INV` | A remote delta MUST reach the document its `document_id` names, and no other. | this spec section 3 | `ambre_murmure::logic::channel_tests::a_remote_delta_reaches_the_document_it_names_and_no_other` -- the other held documents compare equal before and after; `ambre_murmure::logic::channel_tests::a_delta_for_a_document_the_channel_does_not_hold_is_refused`; `ambre_logic::channels::tests::a_remote_delta_reaches_the_channel_that_holds_its_document` -- through the Logic dispatcher | none | verified |

### 1.2 Open a channel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-025` | `POST` | Opening a channel MUST subscribe to its `meta` document and to its live segments. | this spec section 3; `SRV-047` | `ambre_murmure::logic::channel_tests::opening_takes_meta_then_its_live_segments_and_never_a_final_one`; `ambre_logic::channels::tests::a_channel_opens_its_meta_then_its_live_segments_then_shows_them` | none | verified |
| `MURMURE-026` | `POST` | Opening a channel MUST pin the documents the view shows. | this spec section 3; `RESID-003` | `ambre_logic::channels::tests::closing_a_channel_releases_its_documents_and_no_others` -- of two open channels, closing one releases its documents alone, and the other keeps every document; `ambre_murmure::logic::channel_tests::the_documents_of_a_channel_are_every_one_it_holds` -- the engine names each document it holds, an older one loaded since included, for the close to release | none | verified |
| `MURMURE-476` | `POST` | A closed channel MUST reopen from the copy of its documents the local store holds. | this spec section 3; `RESID-010` | `ambre_logic::channels::tests::a_closed_channel_reopens_from_its_local_copy` -- each document reaches the store at close and keeps its horizon, the reopen reads `meta` back before it opens, and a segment with no copy is fetched whole | none | verified |
| `MURMURE-027` | `FAIL` | Given a document whose header names another channel, the engine MUST refuse it and warn. | this spec section 3; `MURMURE-001` | `ambre_murmure::logic::channel_tests::a_document_naming_another_channel_is_refused`; `ambre_murmure::logic::channel_tests::a_meta_document_of_another_channel_is_refused`; `ambre_murmure::logic::channel_tests::a_segment_in_the_place_of_meta_is_refused`; structural -- the one branch of `schema::admit` that returns `ForeignChannel` emits the `tracing::warn!` first | none | verified |
| `MURMURE-028` | `FRAME` | Opening a channel MUST NOT load a final segment. | this spec section 3 | `ambre_murmure::logic::channel_tests::opening_a_channel_never_loads_a_final_segment` -- the final segment's bytes are handed to the open, and it still holds none of it | none | verified |
| `MURMURE-502` | `FAIL` | Given a closing segment whose key this client lacks, or whose bytes fail the check, the channel MUST open without it and show it as one band. | this spec section 3; `MURMURE-218` | `ambre_murmure::logic::channel_tests::a_closing_segment_with_no_key_opens_the_channel_without_it` -- the band is handed once; `::a_damaged_closing_segment_opens_the_channel_without_it`; `::a_damaged_active_segment_still_ends_the_opening`; `ambre_logic::channels::tests::a_closing_segment_with_no_key_still_opens_the_channel` -- its session ends; `ambre_logic::channels::tests::a_transient_failure_of_the_closing_segment_ends_the_opening`; `ambre_murmure::logic::channel_tests::a_held_closing_segment_is_no_band` -- a closing segment the channel holds is no band | none | verified |
| `MURMURE-503` | `FRAME` | A closing segment the channel opened without MUST NOT let the active segment ask to close. | this spec section 3; `SRV-196` | `ambre_murmure::logic::channel_tests::a_closing_segment_not_held_blocks_the_next_close` -- past the segment size, nothing is asked; `ambre_murmure::logic::channel_tests::a_channel_with_no_closing_segment_asks_to_close_at_the_size` -- the same size asks when `meta` lists no closing segment | none | verified |
| `MURMURE-514` | `POST` | A band asked for again MUST fetch its key chain and its bytes anew: a final segment loads in place, and a closing segment the channel opened without reopens the channel. | this spec section 3; `MURMURE-502`; `MURMURE-513`; `SYNC-173` | `ambre_murmure::logic::channel_tests::a_final_band_asked_again_loads_in_place` -- asked once, its messages replace the band; `ambre_murmure::logic::channel_tests::only_a_listed_final_segment_is_asked_again`; `ambre_murmure::logic::channel_tests::a_closing_segment_opened_without_is_lacking`; `ambre_logic::channels::tests::a_final_band_asked_again_opens_its_segment` -- one open while it is on its way, none for a live segment; `ambre_logic::channels::tests::a_closing_band_asked_again_opens_the_channel_anew` | none | verified |

### 1.3 Apply a remote delta

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-029` | `POST` | A remote delta MUST pass the authorship check of the messages feature before it is imported. | this spec section 3; `features/ambre-spec-murmure-messages.md` | `ambre_murmure::logic::authorship_tests::a_message_carries_the_author_who_signed_it_and_no_other`; `ambre_murmure::logic::authorship_tests::a_delta_refused_while_the_channel_opens_is_left_out`; `ambre_logic::channels::tests::a_delta_the_authorship_check_refuses_reaches_no_view` -- an open channel and an opening one both check before import | none | verified |
| `MURMURE-030` | `FRAME` | A refused delta MUST NOT change any document of the channel. | this spec section 3; `AUTHOR-014` | `ambre_murmure::logic::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing` -- every document of the channel compares equal across the refusal | none | verified |

### 1.4 Log out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-638` | `POST` | A logout MUST forget every channel delete, channel leave and file erase the relay has not answered. | this spec section 3; `MURMURE-241` | `ambre_logic::spaces::tests::a_logout_forgets_the_deletes_and_leaves_the_relay_has_not_answered`; `ambre_logic::channels::tests::a_logout_forgets_the_erases_the_relay_has_not_answered` -- an erase on its way and an erase owed, past a new sign-in and two connections | none | verified |
| `MURMURE-639` | `POST` | A logout MUST forget every preview fetch and every ask of who is in a call, so the next session asks its own. | this spec section 3 | `ambre_logic::channels::tests::a_logout_forgets_the_preview_fetches_the_relay_has_not_answered` -- the same image fetched anew; `ambre_logic::channels::tests::a_logout_forgets_when_each_channel_last_asked_who_is_in_its_calls` -- asked within the interval of the last ask | none | verified |
| `MURMURE-640` | `FRAME` | A late answer to a request of an earlier session MUST NOT reach a view or write a document. | this spec section 3; `MURMURE-638`; `MURMURE-639` | `ambre_logic::spaces::tests::a_logout_forgets_the_deletes_and_leaves_the_relay_has_not_answered` -- a late delete and a late leave show and send nothing; `ambre_logic::channels::tests::a_logout_forgets_the_preview_fetches_the_relay_has_not_answered` -- a late image reaches no view | none | verified |

* **Budget.** the first display of a channel loads its `meta` document and its active segment, about
  2 MB, inside the memory rows of `ambre-spec-budgets.md section 6`. The authorship check costs the
  JSON check row of `ambre-spec-budgets.md section 1`.
* **Boundary.** every document here travels sealed through the document pipeline (`ZK-001`). What
  the relay learns of the grouping is `ZK-015`.
* **Verification.** 17 `verified`. The open is held on both sides of the Logic thread: the engine's
  staged open, and the dispatcher that drives it over the relay. A close hands each document's copy
  to the local store and releases it to a light subscription. A reopen starts each document from its
  copy.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The engine holds | Each user sees | Test |
|---|---|---|---|---|
| opens a channel of two years | -- | `meta` and the active segment | the latest messages at once, and older ones on scroll (`MURMURE-023`) | none |
| sends a message | has the channel open | the message in the active segment, on both clients | the message on both screens | none |
| -- | Mallory sends a delta that rewrites Alice's message | nothing new: the check refuses it | Alice's message as she wrote it (`MURMURE-029`) | `ambre_murmure::logic::authorship_tests::an_edit_of_another_authors_message_is_refused_and_changes_nothing` |

---

## 3. MODEL

**CRDT containers.** the five kinds of `ambre-spec-murmure-schema.md section 3`, one Loro document
each.

**The state of an open channel.** Private fields, read through methods.

```rust
#[derive(Debug)]
pub struct ChannelState {
    channel_id: Id<Channel>,
    meta: Held,                          // an Id<Document> and its LoroDoc
    active: Held,
    closing: Option<Held>,
    loaded: HashMap<Id<Segment>, LoroDoc>, // final segments, on demand
    pending: HashMap<Id<Segment>, Vec<Arrival>>, // final segments asked for, still arriving
    threads: HashMap<Id<Message>, Thread>, // the open threads, by their root
    /* and the bookkeeping: sequences, the segment lifecycle, amendments, the seek */
}
```

**An open, in steps.** The relay serves each document of a channel as an ordinary document, so the
channel opens in stages. Nothing of it is shown before the last one.

1. The Sync worker lists the documents of the channel. A non-member is refused (`SRV-047`).
2. The Logic thread opens the `meta` document, and holds its bytes until it is ready.
3. `meta` names the live segments, and the Logic thread opens each one (`MURMURE-025`).
4. The last live segment builds the `ChannelState`, and the view gets their messages.

A refusal at any step closes every document the open asked for, and the view gets
`MurmureEvent::OpenFailed`. The closing segment is the one exception. A key this client lacks, or
bytes that fail the check, leave it out. The channel opens, and the view gets a band for it
(`MURMURE-502`). A transient failure still ends the open. The active segment asks no close while
the closing one stays listed (`MURMURE-503`).

A band asked for again opens its segment anew, and Sync walks the key chain again. A final
segment takes the path of an older one, and its messages replace the band. A closing segment
the channel opened without has no place to load into, so the channel closes and opens again
(`MURMURE-514`).

The open threads join the same state, keyed by their root (`features/ambre-spec-murmure-threads.md`).

**The command surface.** One content-plane variant, `LogicCommand::Channel`, addressed to a channel
(`BUS-027`). Each command arrives with the Story that builds it.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MurmureCommand {
    SendMessage {
        draft: MessageDraft,
        attachments: Vec<Attachment>, // files the Sync worker answered uploaded (`MURMURE-132`)
        now: Instant<Authored>,
        confirmed: Confirmation,      // the `@channel` the author confirmed (`MURMURE-231`)
    },
    LoadOlder { segment: Id<Segment> },
    /* one variant per operation, each in the feature document that builds it */
}
```

The view stamps `now`, because the Logic thread reads no clock. A send hands the Sync worker the
delta and its activity summary, and the view a `MurmureEvent::Messages`.

**A remote delta, in steps.**

1. The Sync worker opens the envelope and forwards the delta, the nonce, the summary and the signer.
2. The engine finds the document by its `document_id` (`MURMURE-024`).
3. The authorship check of the messages feature reads the JSON update, with no document
   (`MURMURE-029`). A refusal changes nothing (`MURMURE-030`), and the delta still counts as
   delivered.
4. The delta is imported, and the view is told which messages changed.

A segment still opening holds each delta as it arrived, and checks it when the segment is built.

**A logout ends what the user asked.** A request the relay has not answered belongs to the user who
asked it. The logout drops each one, with the erases still owed (`MURMURE-638`). A later session
asks its own previews and call rosters anew (`MURMURE-639`). A late answer finds no request and
does nothing (`MURMURE-640`).

**Code.** `crates/ambre-murmure/src/logic/channel.rs`, `crates/ambre-murmure/src/logic/delta.rs`,
`crates/ambre-murmure/src/logic/schema.rs`,
`crates/ambre-murmure/src/logic/opening.rs`, `crates/ambre-bus/src/app/murmure.rs`,
`crates/ambre-murmure/src/logic/runtime/channels.rs`.

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
