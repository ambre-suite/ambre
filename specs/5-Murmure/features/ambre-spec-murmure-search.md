# AMBRE -- FEATURE: MURMURE SEARCH IN LOADED SEGMENTS
**Type:** Vertical (feature)
**Implementation:** design-only

> A member finds a message by its words, in the channels they have loaded. The search runs on the
> client, and it says which part of the history it covered.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-147` | `INV` | A search MUST run on the client alone. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-148` | `INV` | A search MUST cover the active segment and every segment already loaded. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-314` | `INV` | A segment index MUST be signed by the client that finalized its segment. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-320` | `INV` | The index of a segment MUST live in the private file system of the origin, read from a worker, never whole in memory. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Search

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-149` | `POST` | A result list MUST state which segments it covered. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-150` | `POST` | A result MUST open its message in its channel. | this spec section 3 | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-151` | `FRAME` | A search MUST NOT load a segment that holds no match. | this spec section 3; `RESID-016` | not built | unbuilt -- lands with the `Search in loaded segments` Story (`ambre-roadmap.md` E5) | unimplemented |

**Frame:** `FIND-001`

### 1.3 Index a final segment

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-313` | `POST` | The client that finalizes a segment MUST build its index, seal it under the channel key and upload it beside the snapshot. | this spec section 3; `MURMURE-068` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-322` | `POST` | A client that confirms a finalization MUST rebuild the index, and withhold its confirmation when it differs. | this spec section 3; `MURMURE-071` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-319` | `POST` | A re-finalization MUST replace the index of its segment. | this spec section 3; `MURMURE-075` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-336` | `FRAME` | Indexing a segment MUST NOT change the segment. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Search on a new device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-315` | `POST` | A search MUST query the index of each final segment the client does not hold, then fetch only the segments with a match. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-316` | `POST` | Every match MUST be checked against its decrypted segment before it is shown. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-318` | `POST` | A match the amendments delete MUST be dropped, and the amendments MUST be searched on the fly. | this spec section 3; `MURMURE-013` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-321` | `POST` | A search MUST filter by author, date, channel, file type, and replies to the user. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-337` | `FRAME` | A search MUST NOT send the query outside the client. | this spec section 3; `MURMURE-147` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a search over the loaded segments reads memory the client holds, inside the rows
  of `ambre-spec-budgets.md section 6`.
* **Boundary.** none: the query never leaves the client, and the relay searches nothing.
* **Verification.** 16 `unimplemented`: no search exists in Murmure.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| searches "budget" | -- | the loaded segments | the matches, and "searched 2 of 14 segments" (`MURMURE-149`) | none |
| opens a result | -- | the segment of the match | the channel, at the message (`MURMURE-150`) | none |

---

## 3. MODEL

**CRDT containers.** none: a search reads the messages the loaded documents hold.

**The words** are cut with diacritics folded.

**The find of the suite, across channels.** The find chord opens this search (`MURMURE-213`). The
search follows `ambre-spec-find.md` in two points. A search changes no document (`FIND-001`), and a
result brings its message into view (`MURMURE-150`). The scope differs: a Murmure search covers
every channel the member reads, not one document. A search offers no replace.

**An index per final segment.** The finalizing client builds, seals, signs and uploads it beside
the snapshot (`MURMURE-313`, `MURMURE-314`). Only a re-finalization replaces it (`MURMURE-319`).

**A new device** queries the indexes, then fetches only the segments with a match (`MURMURE-315`).

**Integrity.** A check against the decrypted segment removes the false positives (`MURMURE-316`).
The confirming client rebuilds the index, which removes the false negatives (`MURMURE-322`).

**Storage.** The index lives in the Origin Private File System, read from a worker (`MURMURE-320`).
The stack is `unicode-segmentation`, `fst` and `roaring`.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the search field | the matches, grouped by channel, with the coverage |
| a result | the channel opens at the message |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client searches | a search on the relay | the relay would learn the query and the matches, which breaks the zero-knowledge pillar |
| An index per final segment | searchable encryption on the relay | the relay would see which queries repeat and which documents match, and leakage-abuse attacks recover content from that |
| An index per final segment | a full download and re-index on each device | a new device waits for the whole history, which is where Element Web still has no search |
| Search without replace | the replace of `ambre-spec-find.md` | a message is changed by its author alone, one at a time. A replace across channels would rewrite what others read |
| Pure Rust | SQLite with FTS5 compiled to WASM | a C library, against the Rust-first pillar |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No search exists in Murmure.
* **tantivy is not measured.** It is richer than `fst` and `roaring`, and its WASM build is
  fragile. A prototype decides before the index format is frozen.
