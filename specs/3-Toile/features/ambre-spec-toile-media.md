# AMBRE -- FEATURE: MEDIA

**Type:** Vertical (feature)
**Implementation:** partial -- the upload path and the cache ship; nothing downloads and nothing paints.

> A picture on the board is stored once, wherever it appears, and the server that keeps it never
> sees it. The board holds an address; the pixels travel apart.

The picture element itself is `features/ambre-spec-toile-elements.md`; what a file import produces is
`features/ambre-spec-toile-import-export.md`. This document owns the address, the transfer and the
cache.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-IO-006` | Media MUST be content-addressed: two identical pictures MUST resolve to one blob. | this spec section 3 | `ambre_toile::logic::state::import::tests::import_seeds_the_media_map`, `::seed_media_dedups_repeated_hashes`, `::seed_media_writes_load_info`, `::seed_media_skips_unresolved_hash`, `ambre_toile::logic::interop::media::tests::first_request_fetches_then_dedups` | verified |
| `TOILE-IO-007` | The cache MUST evict only resolved entries: a load in flight MUST never be evicted. | this spec section 3 | `ambre_toile::logic::interop::media::tests::loading_entries_are_never_evicted`, `::lru_evicts_least_recently_used_ready_entry`, `::loading_to_ready_carries_pixels`, `::loading_to_failed`, `::capacity_zero_coerced_to_one` | verified |
| `TOILE-IO-008` | A picture import MUST wait for its upload; a failed upload MUST abort the pending import. | this spec section 3 | `ambre_toile::logic::state::import::tests::image_import_defers_until_media_uploads`, `::a_failed_upload_aborts_the_pending_import`, `::a_stale_service_response_is_dropped` | verified |
| `TOILE-IO-009` | An oversized or undecodable picture MUST abort the import atomically, and a generated thumbnail MUST stay bounded. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::oversized_image_aborts_atomically`, `::garbage_bytes_fail_to_decode`, `::thumbnail_is_bounded`, `::imports_one_image_with_thumbnail`, `ambre_toile::logic::interop::tests::embedded_png_maps_to_image_and_media_blob` | verified |
| `TOILE-IO-013` | This board MUST NOT implement cryptography: it hands plaintext to the Sync thread and no pixel reaches the server except as ciphertext. | this spec section 3; `ZK-001`, `ZK-004` | crate graph: `ambre-toile` declares no AEAD, no key derivation and no key exchange crate, and `deny.toml` refuses to let it -- each capability crate lists its allowed wrappers | verified |
| `TOILE-IO-034` | A picture the board names MUST reach the screen: an address MUST resolve to pixels. | this spec section 3; section 4 | none, and **the rule fails today**: no code issues a download, and the store the painter reads is built empty and never filled -- section 6 | unverified |
| `TOILE-IO-035` | A picture MUST show its thumbnail while the full blob loads, so the first paint waits on nothing. | this spec section 3; section 4 | none, and **the rule fails today**: the thumbnail is written into the document on import and read by nothing -- section 6 | unverified |
| `TOILE-IO-036` | Undoing an import MUST take back the media entries that import seeded. | this spec section 3 | none. The seed runs inside the import's single commit, so the rule holds by construction; no test undoes an import and counts the entries | unverified |
| `TOILE-IO-037` | A decoded picture MUST NOT outlive the session: no plaintext pixel MUST reach local storage. | this spec section 3; `ZK-004` | none. The cache is a map in Logic-thread memory and the crate opens no storage handle, so the rule holds by absence; nothing would go red the day a writer appeared | unverified |

* **Budget.** the first paint of a picture under 15 ms and no refetch on a revisit, two rows of
  `ambre-spec-budgets.md section 1`. The first is the budget the thumbnail exists to meet, and
  nothing reads the thumbnail (`TOILE-IO-035`).
* **Boundary.** this document owns Toile's only outbound flow of user content: media blobs, sealed
  by the Sync thread before they leave (`TOILE-IO-013`), plus the routing metadata the relay already
  sees for any document. No board geometry and no pixel leaves in plaintext.
* **Verification.** 5 `verified`, 4 `unverified`. The upload half is held end to end: the address
  dedups, the cache keeps a load in flight, a failed upload takes the import with it and a hostile
  picture is refused. The four gaps are the download half, and they are one absence -- the cache
  exists, the thumbnail exists, and nothing joins either to a screen.

---

## 2. WHAT EACH USER SEES

The address is shared and the pixels are not: each peer fetches for itself. The last column joins the
example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a picture | watches | one address and one entry (`TOILE-IO-006`) | Alice sees her picture; Bob sees a box until his own fetch lands (`TOILE-IO-034`) | `logic::state::import::tests::import_seeds_the_media_map` |
| imports the same picture twice | watches | one entry, two elements (`TOILE-IO-006`) | two pictures, one blob | `logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| imports a picture whose upload fails | watches | nothing (`TOILE-IO-008`) | an error on Alice's screen and no change on Bob's | `logic::state::import::tests::a_failed_upload_aborts_the_pending_import` |
| imports a picture, then undoes | watches | neither the element nor its entry (`TOILE-IO-036`) | the picture leaves in one step, on both boards | none -- section 6 |

---

## 3. MODEL

**Container.** `media`, one entry per address, holding the dimensions, the type and an inline
thumbnail -- never the pixels (`ambre-spec-toile-schema.md section 3`). The picture element carries
the address alone.

**The service plane** (`ambre-bus`): media reuses the shared request plane rather than a channel of
its own (`ambre-spec-architecture.md section 2`).

| Shape | Direction | Carries |
|---|---|---|
| `ToileRequest::UploadMedia` | Logic to Sync | a request id, the plaintext bytes and the type |
| `ToileRequest::DownloadMedia` | Logic to Sync | a request id and the address |
| `RestEvent::MediaUploaded` | Sync to Logic | the address the Sync thread computed |
| `RestEvent::MediaDownloaded` | Sync to Logic | the address and the plaintext bytes |

Toile allocates the request id, because it owns the correlation back to a pending import; the
dispatcher supplies the document id and lowers the request.

**The cache** (`ambre-toile`):

```rust
/// Lifecycle of one cached blob. `Loading` is in flight, `Ready` holds decoded
/// pixels, `Failed` records an error so the painter shows a broken picture
/// without re-issuing the request on every frame.
pub enum MediaState { Loading, Ready, Failed }

/// What a request resolved to. `Fetch` means the address was unknown and the
/// caller must issue a download; `CacheHit` means an entry exists and no fetch
/// is due -- the dedup that keeps identical pictures to one blob.
pub enum RequestOutcome { Fetch, CacheHit(MediaState) }
```

**Steps, when a picture is imported:**

1. decode the bytes, refuse a picture past the dimension bound, and build a bounded thumbnail
   (`TOILE-IO-009`);
2. hand the plaintext up as an upload and apply nothing yet (`TOILE-IO-008`);
3. rewrite the placeholder address to the one the Sync thread returns, in the blob and in every
   element that named it;
4. seed the entry when the address is new, and leave it alone when it is not (`TOILE-IO-006`);
5. commit once, with the elements, so the entry belongs to the import's undo item
   (`TOILE-IO-036`).

**Steps, when a picture is displayed:**

1. ask the cache for the address; a first ask creates the slot and orders a download, a repeat asks
   for nothing (`TOILE-IO-006`);
2. paint the thumbnail meanwhile, and the placeholder box when there is not even one
   (`TOILE-IO-035`, `TOILE-UI-013`);
3. resolve the slot with the decoded pixels when the download lands, and repaint;
4. mark the slot failed on an error, so the request is not re-issued in a loop;
5. evict the least recently used resolved slot when the cache is over capacity, and never a slot in
   flight (`TOILE-IO-007`).

**The Sync thread owns the address.** It compresses, encrypts and hashes; the hash of the plaintext
becomes the key. Toile never hashes and never encrypts (`TOILE-IO-013`), which is why an import has
to wait for an answer before it can name anything.

**Code:**

* `crates/ambre-toile/src/logic/interop/media.rs` -- the lifecycle and the bounded cache;
* `logic/interop/raster.rs` -- the decode, the bound and the thumbnail;
* `logic/state/import.rs` -- the pending import, the address rewrite and the seed;
* `crates/ambre-sync/src/media.rs` -- the upload and the download, with the crypto;
* `crates/ambre-toile/src/ui/render.rs` -- the paint, and the store it reads.

---

## 4. SURFACE

A picture has no controls of its own: it is selected, moved, resized and styled like any element
(`features/ambre-spec-toile-elements.md section 4`). What this feature owns on screen is what a
picture looks like before its pixels arrive.

| State | What the board shows |
|---|---|
| the pixels are resident | the picture, scaled to its box and rotated with it |
| the download is in flight | its thumbnail, at the same box (`TOILE-IO-035`) |
| there is no thumbnail either | a placeholder box in the element's own style (`TOILE-UI-013`) |
| the download failed | the same box, and no further request |

**Two of the four rows are the same box today** (`TOILE-IO-034`). Nothing orders a download and
nothing fills the store the painter consults, so every picture on every board paints the placeholder,
on screen and in an exported page alike.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A picture is addressed by the hash of its content | the same picture pasted ten times is one blob and one upload, and the address is stable across peers with no coordination | a minted id per picture, which uploads the same bytes once per use |
| The Sync thread computes the address | the address is the hash of the plaintext and the Sync thread is the only crypto consumer, so hashing there keeps every primitive behind one wall | hashing on the Logic side, which puts a second crate on the crypto path for one call |
| The document holds an address, never pixels | a board with fifty photographs would otherwise be a CRDT nobody can merge, and every peer would carry every byte of history | storing the bytes in the document, which is simpler and unbounded |
| The entry carries a thumbnail inline | the first paint must not wait on the network, and a thumbnail is small enough to ride in the document | fetching the full blob before painting, which leaves a hole on screen for as long as the transfer takes |
| An import waits for every upload | an element naming an address that does not exist yet would be a broken picture on every other peer, and the failure would arrive after the undo item closed | applying the elements first, which makes the picture converge before the pixels do |
| The cache never evicts a load in flight | the answer would resolve into a slot that is gone, and the request would be re-issued on the next paint | evicting strictly by recency, which loses in-flight work under pressure |
| A failure is a cache state, not a retry | a picture the relay cannot serve would otherwise be re-requested on every frame of a pan | leaving the slot empty, which turns one failure into a request loop |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing downloads** (`TOILE-IO-034`). The cache is complete, tested and exported, and its only
  callers are its own tests. The download request exists on the bus and in the dispatcher, and no
  code in Toile ever builds one. The store the painter reads is created empty and written by nothing
  outside the render tests.
* **The thumbnail is written and never read** (`TOILE-IO-035`). Every import encodes one and puts it
  in the document; no reader consults it. The field that exists to make the first paint instant costs
  bytes in the document and buys nothing -- the same shape as the stroke pressure and the reaction
  timestamp, and the third field of its kind in this application.
* **So every picture is a grey box.** The two absences compose: no download, no thumbnail read. A
  user who imports a photograph sees the placeholder, and so does the PDF export
  (`features/ambre-spec-toile-pdf-export.md section 5`). The import path that produces the picture is
  itself unreachable, which is why nobody has met the defect.
* **An entry outlives its last element, and no rule says what should happen.** Deleting a picture
  drops the element and leaves the entry in `media` for good. Eager removal is wrong -- a delete is
  undoable, and another element may share the address -- and no reference count exists. The document
  therefore grows by one entry per picture ever imported, and nothing bounds it.
* **The cache has no owner.** It is a type with a capacity and no instance: no engine field holds one,
  so its capacity is never chosen and its eviction never runs outside tests.
* **A local blob is never re-used across documents.** Content addressing would let two boards share
  one fetch, and the cache would have to leave the engine to do it.
* **Undo is not tested against the media map** (`TOILE-IO-036`). The seed runs inside the import's
  single commit, so the rule follows from the commit; no test undoes an import and counts what is
  left.
* **No test drives the transfer end to end.** The upload round trip is held at the dispatcher with a
  mock, and the download half has nothing to hold.
