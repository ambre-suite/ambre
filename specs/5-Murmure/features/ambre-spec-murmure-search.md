# AMBRE -- FEATURE: MURMURE SEARCH IN LOADED SEGMENTS
**Type:** Vertical (feature)
**Implementation:** partial -- the search over the loaded segments and its box are built; the
index is not

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
| `MURMURE-147` | `INV` | A search MUST run on the client alone. | this spec section 3; `ZK-007` | `ambre_logic::channels::tests::a_search_covers_every_open_channel_and_leaves_nothing` -- the Sync worker receives nothing | none | verified |
| `MURMURE-148` | `INV` | A search MUST cover the active segment and every segment already loaded. | this spec section 3 | `ambre_murmure::channel_tests::a_search_reads_the_held_segments_and_lists_the_rest` -- the live segments, then a loaded final one | none | verified |
| `MURMURE-314` | `INV` | A segment index MUST be signed by the client that finalized its segment. | this spec section 3 | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-320` | `INV` | The index of a segment MUST live in the private file system of the origin, read from a worker, never whole in memory. | this spec section 3; `ambre-spec-budgets.md section 6` | not built | unbuilt -- lands with the `Search on a new device` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Search

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-149` | `POST` | A result list MUST state which segments it covered. | this spec section 3 | `ambre_murmure::channel_tests::a_search_reads_the_held_segments_and_lists_the_rest`; `ambre_logic::channels::tests::a_search_covers_every_open_channel_and_leaves_nothing` -- each channel names the segments read and the count listed; `ambre_app::shell::murmure_regions::the_search_box_asks_lists_and_opens_a_match` -- browser tier, the box shows the coverage above the matches | none | verified |
| `MURMURE-496` | `POST` | A result list MUST show the answer to the words the search box holds, and drop an answer to older words. | this spec section 3 | `ambre_murmure::tests::chrome_relay::the_find_chord_opens_the_search_and_its_answer_lists_the_matches` -- an answer to an older query shows nothing; `ambre_murmure::tests::chrome_relay::a_match_opens_its_message_in_its_channel_and_closes_the_search` -- nor one after the box closed | none | verified |
| `MURMURE-150` | `POST` | A result MUST open its message in its channel. | this spec section 3 | `ambre_murmure::tests::chrome_relay::a_match_opens_its_message_in_its_channel_and_closes_the_search` -- the pane names the channel and the message; `ambre_murmure::view::tests::a_pane_at_a_message_rests_the_list_on_it` -- the list rests on the message, once it is in the list | none | verified |
| `MURMURE-478` | `POST` | A search MUST fold the precomposed Latin, Greek and Cyrillic accents, and drop the combining marks of every script. | this spec section 3 | `ambre_murmure::search::tests::every_script_folds_its_diacritics`; `ambre_murmure::search::tests::a_query_matches_its_words_with_diacritics_folded` -- Vietnamese, pinyin, polytonic Greek, Hebrew points and Arabic vowel signs | none | verified |
| `MURMURE-151` | `FRAME` | A search MUST NOT load a segment that holds no match. | this spec section 3; `RESID-016` | type-level -- `ChannelState::search` takes `&self` and cannot load; `ambre_murmure::channel_tests::a_search_reads_the_held_segments_and_lists_the_rest` | none | verified |

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
* **Verification.** 7 `verified`, 11 `unimplemented`: the search over the loaded segments and its
  box are built, and no index exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| searches "budget" | -- | the loaded segments | the matches, and "searched 2 of 14 segments" (`MURMURE-149`) | `ambre_app::shell::murmure_regions::the_search_box_asks_lists_and_opens_a_match` |
| opens a result | -- | the segment of the match | the channel, at the message (`MURMURE-150`) | `ambre_murmure::view::tests::a_pane_at_a_message_rests_the_list_on_it` |

---

## 3. MODEL

**CRDT containers.** none: a search reads the messages the loaded documents hold.

**The words** are cut by UAX #29, lowercased, with diacritics folded. A query word matches when it
begins a word of the message, and a message matches when it holds every word of the query. A table
folds the precomposed accents, and any combining mark is dropped (`MURMURE-478`); `й` stays.

**The scope.** A search reads every open channel, in the segments each one holds. A final segment
not loaded is counted in the coverage and never fetched (`MURMURE-151`).

**The find of the suite, across channels.** The find chord opens this search (`MURMURE-213`). The
search follows `ambre-spec-find.md` in two points. A search changes no document (`FIND-001`), and a
result brings its message into view (`MURMURE-150`). The scope differs: a Murmure search covers
every channel the member reads, not one document.

**The box.** An answer carries its query, so the box drops one to older words (`MURMURE-496`).

**An index per final segment.** The finalizing client builds, seals, signs and uploads it beside
the snapshot (`MURMURE-313`, `MURMURE-314`). Only a re-finalization replaces it (`MURMURE-319`).

**A new device** queries the indexes, then fetches only the segments with a match (`MURMURE-315`).

**Integrity.** A check against the decrypted segment removes the false positives (`MURMURE-316`).
The confirming client rebuilds the index, which removes the false negatives (`MURMURE-322`).

**Storage.** The index lives in the Origin Private File System, read from a worker (`MURMURE-320`).
The stack is `unicode-segmentation`, `fst` and `roaring`.

**Code.** `Query` and the fold, in `crates/ambre-murmure/src/search.rs`. `ChannelState::search`, in
`crates/ambre-murmure/src/channel.rs`. `LogicState::search_messages`, in
`crates/ambre-logic/src/channels.rs`. The box is `MessageSearch`, and `MurmureRelay` keeps
its state, in `crates/ambre-murmure/src/chrome.rs`.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the find chord | the box opens in the command bar, and takes the caret |
| the search field | the matches, grouped by channel, with the coverage |
| a field with no word | asks nothing, and shows no result |
| a result | the channel opens at the message, and the box closes |
| Escape in the field | the box closes |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The client searches | a search on the relay | the relay would learn the query and the matches, which breaks the zero-knowledge pillar |
| An index per final segment | searchable encryption on the relay | the relay would see which queries repeat and which documents match, and leakage-abuse attacks recover content from that |
| An index per final segment | a full download and re-index on each device | a new device waits for the whole history, which is where Element Web still has no search |
| Search without replace | the replace of `ambre-spec-find.md` | a message is changed by its author alone, one at a time. A replace across channels would rewrite what others read |
| Pure Rust | SQLite with FTS5 compiled to WASM | a C library, against the Rust-first pillar |
| A fold table for the Latin letters | `unicode-normalization` | it would add its decomposition tables to the Main instance, for a fold one table covers |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No index is built.** A search reads only the segments the client holds.
* **tantivy is not measured.** It is richer than `fst` and `roaring`, and its WASM build is
  fragile. A prototype decides before the index format is frozen.
