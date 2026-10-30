# AMBRE -- FEATURE: DOCUMENT RESIDENCY

**Type:** Horizontal (framework)
**Implementation:** partial -- a closed document leaves memory for the local store, keeps a light
subscription and reopens from its copy. The local store holds a byte budget and meets the quota.
No read cursor exists yet.

> Which documents a client holds, in memory and on disk, and what a document it does not hold still
> costs. A user can belong to hundreds of documents that change at once. The client keeps the few
> it shows, and still knows where there is activity.

This document owns the two caches, the order of eviction and the light subscription. What an
application counts as activity, and which of its documents it pins, is that application's rule.

---

## 1. CONTRACT

The only mandatory read. Grammar and status values are normative in `ambre-spec-template.md`. The
prefix is `RESID`, and it belongs to this document alone.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-001` | `INV` | A document no view shows MUST leave memory once its view closes. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_logic::residency::tests::a_closed_document_reaches_the_store_then_leaves_memory` -- it closes the view of a document and finds no document left in the Logic state | none | verified |
| `RESID-002` | `INV` | The local store MUST stay within its byte budget. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_logic::store_ledger::tests::the_store_stays_within_its_budget` -- a copy past the budget is named for deletion and no longer counted | none | verified |
| `RESID-003` | `INV` | A document a view shows MUST NOT leave memory. | this spec section 3 | `ambre_logic::residency::tests::a_shown_document_never_leaves_memory` -- it closes one document and finds the one still shown in memory | none | verified |
| `RESID-004` | `INV` | A document not held in memory MUST cost only a light subscription: its key, its counters and its cursor. | this spec section 3; `ambre-spec-budgets.md section 6` | `ambre_sync::light::tests::an_evicted_document_costs_only_a_light_subscription` -- it releases a document, finds its full session gone, and bounds the entry left by the budget row | none | verified |
| `RESID-005` | `INV` | The activity counters of a document not held in memory MUST come from the activity summaries alone. | this spec section 3; `SYNC-113` | `ambre_sync::light::tests::a_light_update_moves_its_counters_from_the_summary` -- the counters it reads back are the ones the sealed summaries name | none | verified |
| `RESID-020` | `INV` | The local store MUST be requested from the browser as persistent storage. | this spec section 3; `ambre-spec-launcher.md` | `ambre_belt::store::tests::the_local_store_is_asked_to_persist` -- browser tier: the store reads best-effort before the permission, and persistent once a document opens after it | none | verified |
| `RESID-025` | `INV` | Every row of the local store, a document copy and a draft alike, MUST be sealed under a key the device never stores. | this spec section 3; `ambre-spec-zero-knowledge.md`, `ZK-002`; `MURMURE-211` | `ambre_sync::local_store::tests::a_row_is_sealed_under_the_seed_and_opens_on_a_later_session` -- the stored bytes carry no plaintext and the same seed opens them on a later session, `ambre_sync::local_store::tests::without_a_session_the_store_is_locked` -- without a seed nothing is sealed or opened, `ambre_app::local_store::tests::a_copy_lands_sealed_and_one_that_does_not_open_leaves` -- a copy reaches the store only as the worker's sealed bytes | none | verified |
| `RESID-026` | `INV` | The key that seals the local store MUST NOT reach the UI thread. | this spec section 3; `ambre-spec-launcher.md`, `CHROME-044` | cargo-deny -- `deny.toml` lets only `ambre-sync` and `ambre-logic` declare `ambre-crypto`, so the page's crate cannot derive the key, and `ambre_app::vault::tests::a_seal_asks_the_worker_and_takes_its_answer` -- the page asks the worker for each seal and receives only sealed bytes | none | verified |

### 1.2 Evict a document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-006` | `POST` | An evicted document MUST reach the local store before its memory is freed. | this spec section 3 | `ambre_logic::residency::tests::a_closed_document_reaches_the_store_then_leaves_memory` -- the snapshot write is queued before the release reaches Sync | none | verified |
| `RESID-007` | `POST` | Eviction MUST take a document its application marks immutable before any other. | this spec section 3 | `ambre_core::residency::tests::an_immutable_document_leaves_before_any_other` -- an immutable copy leaves before an older mutable one | none | verified |
| `RESID-008` | `POST` | Among the documents of one rank, eviction MUST take the least recently viewed first. | this spec section 3 | `ambre_core::residency::tests::the_least_recently_viewed_leaves_first` -- the order survives a restore; `ambre_logic::store_ledger::tests::a_reload_keeps_the_order_of_the_views` -- the ledger row round-trips; `ambre_logic::persistence::wasm_tests::ledger_rows_come_back_until_the_copy_leaves` -- wasm tier, a real browser database | none | verified |
| `RESID-009` | `FRAME` | Evicting a document MUST NOT drop a local change the relay has not acknowledged. | this spec section 3; `ambre-spec-sync.md` | `ambre_sync::light::tests::a_release_waits_for_the_echo_of_every_frame_sent` -- the full session stays while a sent frame waits on its echo, and leaves with it | none | verified |

### 1.3 Reopen an evicted document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-010` | `POST` | A reopened document MUST start from its local snapshot and replay only the deltas after it. | this spec section 3; `ambre-spec-sequencer.md` | `ambre_logic::residency::tests::a_reopen_starts_from_the_local_copy` -- Sync opens only once the local copy landed, and the ledger resumes at the horizon; `ambre_sync::light::tests::a_reopen_resumes_after_the_horizon_it_holds` -- the open skips a relay snapshot that folds in nothing past the horizon | none | verified |
| `RESID-011` | `POST` | A reopened document MUST replace the counters its summaries produced with the counts it holds. | this spec section 3 | `ambre_logic::residency::tests::a_reopen_starts_from_the_local_copy` -- the reopen sends the badge back to empty counters | none | verified |
| `RESID-012` | `FAIL` | Given a local snapshot that fails to open, the client MUST fetch the document from the relay. | this spec section 3; `SYNC-012` | `ambre_logic::residency::tests::a_reopen_with_no_usable_copy_fetches_from_the_relay` -- a missing copy and a refused one each drop the light subscription before the open | none | verified |
| `RESID-013` | `FRAME` | Reopening a document MUST NOT move its read cursor. | this spec section 3 | `ambre_logic::channels::tests::reopening_a_channel_leaves_its_read_cursor_where_it_was` -- a closed channel reopens with the cursors it had, and nothing marks it read | none | verified |

### 1.4 Count activity in the background

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-014` | `POST` | An update that carries a summary MUST advance the counters of its document. | this spec section 3; `SYNC-113` | `ambre_sync::light::tests::a_light_update_moves_its_counters_from_the_summary` -- a sealed update with a mention moves the content and mention counts by one | none | verified |
| `RESID-015` | `FAIL` | Given an update with no summary, the client MUST mark its document as active, with no count. | this spec section 3 | `ambre_sync::light::tests::a_light_update_with_no_summary_marks_activity_alone` -- an update with no summary, and one that does not open, each mark activity and count nothing | none | verified |
| `RESID-016` | `FRAME` | Counting activity MUST NOT load the document. | this spec section 3 | `ambre_sync::light::tests::a_light_update_moves_its_counters_from_the_summary` -- nothing but the counters reaches the Logic thread, and no full session appears | none | verified |
| `RESID-021` | `FRAME` | An update this user signed MUST NOT advance the counters of its document. | this spec section 3; `RESID-014` | `ambre_sync::light::tests::an_update_this_user_signed_counts_nothing` -- an update under the user's own key counts nothing, and another member's counts | none | verified |
| `RESID-022` | `POST` | A document the client never held MUST be counted after the last number the relay gave for it, with no replay of the deltas up to it. | this spec section 3; `RESID-014`; `SRV-222`; `SRV-223` | `ambre_sync::light::tests::a_channel_never_opened_is_watched_after_its_last_number` -- a segment asks for what follows its last number and a reconnection resumes there, a delta at it counts nothing and the next counts; `ambre_sync::light::tests::the_meta_of_a_channel_never_opened_is_watched_after_its_last_number` -- a `meta` with a listed last number asks for what follows it and resumes there; `ambre_logic::unopened::tests::a_channel_never_opened_is_watched_once_listed_and_answered` -- each segment reaches the watch with the last number the answer gave, its `meta` with the listing's | none | verified |
| `RESID-023` | `POST` | A rotation of a document held as a light subscription MUST give it the new key, and a document watched after under its key the same. | this spec section 3; `RESID-014`; `ambre-spec-key-rotation.md`, `SYNC-080` | `ambre_sync::light::tests::a_rotation_rekeys_a_channel_watched_unopened` -- a mention under the new key counts nothing before the rotation and one after it, the listed key follows, and a segment watched after counts under it | none | verified |
| `RESID-024` | `POST` | A frame sealed under a key a light subscription lacks MUST ask for the document's current key once per key, and the key taken MUST replay from that frame. | this spec section 3; `RESID-023`; `ambre-spec-key-rotation.md`, `SYNC-080` | `ambre_sync::light::tests::a_frame_under_a_missing_key_fetches_it_and_replays` -- two frames under a missing key mark activity and ask once, the key taken asks from the first again and both mentions count, a key already held replays nothing | none | verified |

### 1.5 Meet the browser quota

The browser can refuse a write before the local store reaches its own budget.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-017` | `POST` | A write the browser refuses for its quota MUST evict what the relay already holds from the local store, then retry once. | this spec section 3; `RESID-007`, `RESID-008` | `ambre_logic::store_ledger::tests::a_refused_write_frees_what_the_relay_holds_then_retries_once` -- the decision; `ambre_belt::store::tests::a_refused_write_evicts_what_the_relay_holds_then_retries` -- browser tier: a page script makes each store write abort with the quota error, since Chrome applies no driver-set quota to IndexedDB. The write is tried twice, and the copy of a document the relay holds is gone | none | verified |
| `RESID-018` | `FAIL` | Given a change the relay has not acknowledged that the browser still refuses to store, the client MUST stop taking local edits and warn the user until the change is sent or space frees. | this spec section 3 | `ambre_logic::residency::tests::a_full_store_pauses_editing_until_the_change_is_safe` -- one warning, an edit refused, then taken again once space frees or the relay acknowledges | none | verified |
| `RESID-019` | `FRAME` | Meeting the quota MUST NOT evict a change the relay has not acknowledged. | this spec section 3; `RESID-009` | `ambre_core::residency::tests::an_unsent_change_keeps_its_document`; `ambre_logic::store_ledger::tests::a_shown_or_unsent_copy_never_leaves` -- past the budget and under the quota alike; `ambre_sync::light::tests::a_release_waits_for_the_echo_of_every_frame_sent` -- the acknowledgement reaches the Logic thread only after the echo | none | verified |

### 1.6 Read back a sealed row

A row sealed under one session may meet another.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-027` | `FAIL` | Given a row that fails to open under the session's key, the client MUST drop it and treat its document as having no local copy. | this spec section 3; `RESID-012` | `ambre_sync::local_store::tests::a_row_that_does_not_open_answers_none_in_its_slot` -- a foreign, a moved and a clear row each answer `None` beside a row that opens, `ambre_app::local_store::tests::a_copy_lands_sealed_and_one_that_does_not_open_leaves` -- a copy with a row that fails is deleted and restores as empty | none | verified |

* **Budget.** three rows of `ambre-spec-budgets.md section 6`: the documents in memory, the local
  store, and one light subscription. All three wait on a measure.
* **Boundary.** the summary rides inside the ciphertext (`SYNC-113`), so the relay learns nothing
  of it. The local store stays on the device, owned by the launcher (`ambre-spec-launcher.md`).
* **Verification.** 27 `verified`, by host tests on the Logic, Sync and ledger states. The browser
  tier holds the quota refusal and the persistent storage request.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | Bob's client holds | What Bob sees | Test |
|---|---|---|---|---|
| mentions Bob in a document Bob has not loaded | reads another document | a light subscription, and one more mention on its counter | a mention badge on that document (`RESID-014`) | `ambre_sync::light::tests::a_light_update_moves_its_counters_from_the_summary` |
| edits a document Bob closed an hour ago | opens thirty other documents | the old document in the local store, and a light subscription | nothing, until Bob reopens it from the local store (`RESID-010`) | `ambre_logic::residency::tests::a_reopen_starts_from_the_local_copy` |
| nothing | reads the document on screen while closing another | the document on screen, and no other | the document stays open (`RESID-003`) | `ambre_logic::residency::tests::a_shown_document_never_leaves_memory` |
| Mallory writes a summary that claims a mention, with no mention in the delta | opens the document | the counts the document holds | the false badge disappears on open (`RESID-011`) | `ambre_logic::residency::tests::a_reopen_starts_from_the_local_copy` |

---

## 3. MODEL

**CRDT containers.** none. Residency decides where a document lives, and reads none of it.

**Three tiers.**

| Tier | Holds | Bound |
|---|---|---|
| in memory | the Loro document and its full Sync state | the documents a view shows (`RESID-001`) |
| on disk | a local snapshot and the deltas after it | a byte budget, inside the browser quota (`RESID-002`) |
| light subscription | the key, the counters and the cursor | a few hundred bytes per document (`RESID-004`) |

**The activity summary.** The application that writes an update may add a summary, sealed with it
(`SYNC-113`).

```rust
pub struct ActivitySummary {
    pub kind: ActivityKind,              // Content, Reaction, Edit or Other
    pub mentions: Vec<UserId>,           // the users the update names
    pub mentions_all: MentionScope,      // Named, or All: every member, an `@channel`
    pub anchor: Option<ActivityAnchor>,  // an application key, for example a thread root
}
pub enum ActivityKind { Content, Reaction, Edit, Other }
pub enum MentionScope { Named, All }   // `define_flag!`: the seal keeps its flag bit
pub struct ActivityAnchor(Vec<u8>);     // opaque to the foundation, typed by the application
```

`ActivityAnchor::new` bounds an anchor to `MAX_ANCHOR_LEN` bytes. Serde reads an anchor with no
bound, so the seal checks it again and drops one past the bound.

The Sync worker reads the summary with no Loro document and advances the counters (`RESID-014`).
A member can lie in a summary. The lie costs one false badge, and the document corrects it on open
(`RESID-011`).

**Leaving memory, in steps.** Memory holds what a view shows, and nothing else (`RESID-001`,
`RESID-003`). When a view closes:

1. Write the document to the local store, then free the memory (`RESID-006`).
2. Sync keeps the full session while a frame it sent waits on its echo (`RESID-009`).
3. Sync then keeps a light subscription at the horizon the document held, so the counters still
   move (`RESID-004`).

**Eviction from the local store.** The order holds the local store within its budget:

1. Skip every document a view shows (`RESID-003`).
2. Take the documents the application marks immutable first, such as a final Murmure segment
   (`RESID-007`).
3. Within a rank, take the least recently viewed (`RESID-008`).

**A full browser quota.** Two kinds of data sit in the local store. What the relay already holds
is a copy: losing it costs a download. A change the relay has not acknowledged exists nowhere
else, so it is never evicted (`RESID-019`).

1. The client asks the browser for persistent storage, so the browser does not clear the store on
   its own (`RESID-020`).
2. A refused write evicts what the relay already holds, in the usual order, and retries once
   (`RESID-017`).
3. When even an unsent change cannot be stored, the client stops taking edits and warns the user.
   Editing resumes once the change is sent or space frees (`RESID-018`). No edit is accepted
   that could be lost.

**A reopen** starts from the local snapshot, replays the deltas after its horizon, and replaces the
counters with the counts it holds (`RESID-010`, `RESID-011`). Sync skips the relay's snapshot when
it folds in nothing past the horizon. A copy that is missing or refused drops the light
subscription, and the open fetches the whole document (`RESID-012`).

**The ledger of the local store.** The Main thread counts what each write costs, by document. Logic
tells it which document a view shows and when the relay holds every change. Any write marks its
document unsent, and only that acknowledgement clears it. One row per document keeps its bytes,
its last view and its flags, so a reload restores the order (`RESID-008`).

**At rest.** Every content row is sealed: snapshots, deltas and drafts. The key derives from the
session's master seed and is never written down (`RESID-025`). The Sync worker seals and opens on
request over the service plane, so the Main thread moves sealed bytes it cannot read
(`RESID-026`). A row that fails to open deletes its copy, which then counts as missing
(`RESID-027`).

The bookkeeping rows stay in the clear: sessions, ledger entries and trust pins. They carry no
content, and their keys are document and user ids the relay already sees. Before sign-in no key
exists, so a cold boot offline opens no local copy. That is the accepted cost of a key never
stored.

**Code.** `ambre_core::activity` holds the summary and the counters. `ambre_sync::light` holds the
light subscription, and `ambre_logic::residency` the close and the reopen. `ambre_core::residency`
holds the eviction order, and `ambre_logic::store_ledger` the budget and the quota decision. The
launcher's `local_store` performs them on the browser database it opens.

---

## 4. SURFACE

Residency has no control of its own.

| Trigger | Behaviour |
|---|---|
| a document leaves the screen | it reaches the local store and leaves memory, and its badge keeps moving |
| a document comes back on screen | it reopens from the local store, and its badge clears |
| the local store past its budget | the least recently viewed copy that no view shows leaves the store |
| an update arrives for a document not held | its badge moves, and nothing loads |
| the browser refuses a write for its quota | older copies leave the local store; past that, editing pauses with a warning until the unsent changes reach the relay |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Memory holds what a view shows | a memory cache within a byte budget | a closed document reopens from the local store in one read, and a cache buys little for a budget no one has measured |
| A byte budget for the local store | a cap on the number of documents | documents differ in size by orders of magnitude, so a count holds no byte budget |
| Counters from a sealed summary | decoding each background delta | a Loro delta decodes only against its document, so every active document would stay in memory |
| Counters from a sealed summary | a wire message kind the relay reads | one more field outside the `ZK-007` set, for a count the client computes itself |
| A light subscription | the full Sync state for every document | the full state runs up to 1 MB, and hundreds of documents break the memory budget |
| Immutable documents leave first | least recently viewed alone | an immutable document reloads with no loss, so it is the cheapest to drop |
| A refused write evicts, then retries | stopping the document | the relay holds every acknowledged change, so losing the local copy costs a download, never data |
| Editing pauses when an unsent change cannot be stored | keeping the change in memory alone | a reload or a closed tab offline would lose it, and the user would never know |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No application marks a document immutable yet.** The order takes such a document first, and
  Murmure will mark its final segments.
* **No application writes a summary yet.** Every update counts as activity alone, until an
  application writes summaries with its own messages.
* **The three budgets are not measured.** The figures in `ambre-spec-budgets.md section 6` are
  estimates, marked as such.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the summary is sealed, and where it sits in the payload | `ambre-spec-sealed-envelope.md` |
| What each application writes in a summary, and what it pins | the application |
| How a reopened document replays the deltas after its snapshot | `ambre-spec-sequencer.md` |
| Where the local store lives, and on which thread | `ambre-spec-launcher.md`, `ambre-spec-architecture.md` |
| What the memory budgets are | `ambre-spec-budgets.md` |
