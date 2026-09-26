# AMBRE -- FEATURE: DOCUMENT RESIDENCY

**Type:** Horizontal (framework)
**Implementation:** design-only

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
| `RESID-001` | `INV` | The documents a client holds in memory MUST stay within their byte budget. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-002` | `INV` | The local store MUST stay within its byte budget. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-003` | `INV` | A pinned document MUST NOT be evicted. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-004` | `INV` | A document not held in memory MUST cost only a light subscription: its key, its counters and its cursor. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-005` | `INV` | The activity counters of a document not held in memory MUST come from the activity summaries alone. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Evict a document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-006` | `POST` | An evicted document MUST reach the local store before its memory is freed. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-007` | `POST` | Eviction MUST take a document its application marks immutable before any other. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-008` | `POST` | Among the documents of one rank, eviction MUST take the least recently viewed first. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-009` | `FRAME` | Evicting a document MUST NOT drop a local change the relay has not acknowledged. | this spec section 3; `ambre-spec-sync.md` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Reopen an evicted document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-010` | `POST` | A reopened document MUST start from its local snapshot and replay only the deltas after it. | this spec section 3; `ambre-spec-sequencer.md` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-011` | `POST` | A reopened document MUST replace the counters its summaries produced with the counts it holds. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-012` | `FAIL` | Given a local snapshot that fails to open, the client MUST fetch the document from the relay. | this spec section 3; `SYNC-012` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-013` | `FRAME` | Reopening a document MUST NOT move its read cursor. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Count activity in the background

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `RESID-014` | `POST` | An update that carries a summary MUST advance the counters of its document. | this spec section 3; `SYNC-113` | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-015` | `FAIL` | Given an update with no summary, the client MUST mark its document as active, with no count. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |
| `RESID-016` | `FRAME` | Counting activity MUST NOT load the document. | this spec section 3 | not built | unbuilt -- lands with the `Document residency` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** three rows of `ambre-spec-budgets.md section 6`: the documents in memory, the local
  store, and one light subscription. All three wait on a measure.
* **Boundary.** the summary rides inside the ciphertext (`SYNC-113`), so the relay learns nothing
  of it. The local store stays on the device, owned by the launcher (`ambre-spec-launcher.md`).
* **Verification.** 16 `unimplemented`: no cache and no light subscription exist today. Every row
  lands with one Story.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | Bob's client holds | What Bob sees | Test |
|---|---|---|---|---|
| mentions Bob in a document Bob has not loaded | reads another document | a light subscription, and one more mention on its counter | a mention badge on that document (`RESID-014`) | none |
| edits a document Bob viewed an hour ago | opens thirty other documents | the old document evicted to the local store | nothing, until Bob reopens it from the local store (`RESID-010`) | none |
| nothing | reads the document on screen while memory runs short | that document pinned, and others evicted around it | the document stays open (`RESID-003`) | none |
| Mallory writes a summary that claims a mention, with no mention in the delta | opens the document | the counts the document holds | the false badge disappears on open (`RESID-011`) | none |

---

## 3. MODEL

**CRDT containers.** none. Residency decides where a document lives, and reads none of it.

**Three tiers.**

| Tier | Holds | Bound |
|---|---|---|
| in memory | the Loro document and its full Sync state | a byte budget (`RESID-001`) |
| on disk | a local snapshot and the deltas after it | a byte budget, inside the browser quota (`RESID-002`) |
| light subscription | the key, the counters and the cursor | a few hundred bytes per document (`RESID-004`) |

**The activity summary.** The application that writes an update may add a summary, sealed with it
(`SYNC-113`).

```rust
pub struct ActivitySummary {
    pub kind: ActivityKind,       // Content, Reaction, Edit or Other
    pub mentions: Vec<UserId>,    // the users the update names
    pub anchor: Option<Vec<u8>>,  // an application key, for example a thread root
}
```

The Sync worker reads the summary with no Loro document and advances the counters (`RESID-014`).
A member can lie in a summary. The lie costs one false badge, and the document corrects it on open
(`RESID-011`).

**Eviction, in steps.**

1. Skip every pinned document. The application pins what a view shows (`RESID-003`).
2. Take the documents the application marks immutable first, such as a final Murmure segment
   (`RESID-007`).
3. Within a rank, take the least recently viewed (`RESID-008`).
4. Write the document to the local store, then free the memory (`RESID-006`). A local change the
   relay has not acknowledged stays queued (`RESID-009`).
5. Keep a light subscription, so the counters still move.

**A reopen** starts from the local snapshot, replays the deltas after its horizon, and replaces the
counters with the counts it holds (`RESID-010`, `RESID-011`).

**Code.** not built. The local store is the browser database the launcher opens.

---

## 4. SURFACE

Residency has no control of its own.

| Trigger | Behaviour |
|---|---|
| memory past the budget | the least recently viewed unpinned document leaves memory, and nothing on screen changes |
| a document leaves the screen | it loses its pin, and becomes a candidate for eviction |
| an update arrives for a document not held | its badge moves, and nothing loads |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A byte budget | a cap on the number of documents | documents differ in size by orders of magnitude, so a count holds no memory budget |
| Counters from a sealed summary | decoding each background delta | a Loro delta decodes only against its document, so every active document would stay in memory |
| Counters from a sealed summary | a frame kind the relay reads | one more field outside the `ZK-007` set, for a count the client computes itself |
| A light subscription | the full Sync state for every document | the full state runs up to 1 MB, and hundreds of documents break the memory budget |
| Immutable documents leave first | least recently viewed alone | an immutable document reloads with no loss, so it is the cheapest to drop |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** Every open document keeps its full state until it closes, and no document
  has a light subscription.
* **The three budgets are not measured.** The figures in `ambre-spec-budgets.md section 6` are
  estimates, marked as such.
* **No rule states what a full browser quota does.** The local store can meet the quota before its
  own budget, and nothing says which document the client drops then.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How the summary is sealed, and where it sits in the payload | `ambre-spec-sealed-envelope.md` |
| What each application writes in a summary, and what it pins | the application |
| How a reopened document replays the deltas after its snapshot | `ambre-spec-sequencer.md` |
| Where the local store lives, and on which thread | `ambre-spec-launcher.md`, `ambre-spec-architecture.md` |
| What the memory budgets are | `ambre-spec-budgets.md` |
