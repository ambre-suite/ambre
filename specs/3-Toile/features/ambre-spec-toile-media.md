# AMBRE -- FEATURE: MEDIA

**Type:** Vertical (feature)

> A picture on the board is stored once, wherever it appears, and the server that keeps it never
> sees it. The board holds an address; the pixels travel apart.

The picture element itself is `features/ambre-spec-toile-elements.md`; what a file import produces is
`features/ambre-spec-toile-import-export.md`. This document owns the address, the transfer and the
cache.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-006` | `INV` | Media MUST be content-addressed: two identical pictures MUST resolve to one entry. | this spec section 3 | `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` -- it seeds the same address twice and again later; `ambre_toile::logic::state::import::tests::import_seeds_the_media_map` -- the entry an import mints | none | verified |
| `TOILE-IO-049` | `INV` | An entry MUST carry the dimensions, the type and a thumbnail. | this spec section 3; `ambre-spec-toile-schema.md section 3` | structural -- `ambre_toile::logic::state::import::seed_media` writes those four fields | none | verified |
| `TOILE-IO-101` | `INV` | An entry MUST NOT carry the picture. | this spec section 3; `ambre-spec-toile-schema.md section 3` | structural -- the plaintext is not one of the four fields `ambre_toile::logic::state::import::seed_media` writes | none | verified |
| `TOILE-IO-013` | `INV` | This board MUST NOT implement cryptography: it hands plaintext to the Sync thread and no pixel reaches the server except as ciphertext. | this spec section 3; `ZK-001`, `ZK-004` | crate-graph -- `ambre-toile` declares no AEAD, no key derivation and no key exchange crate; cargo-deny -- `deny.toml` refuses to let it, each capability crate listing its allowed wrappers | none | verified |
| `TOILE-IO-037` | `INV` | A decoded picture MUST NOT outlive the session: no decoded pixel MUST reach local storage. | this spec section 3; `ZK-004` | `ambre_toile::logic::state::export::media_request_tests::a_decoded_picture_never_enters_the_persisted_document`; ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses a browser storage name outside the three persistence paths (`UI-THREAD-004`), so `ambre-toile` opens no storage handle | none | verified |
| `TOILE-IO-050` | `OBS` | A picture whose pixels are resident MUST paint them scaled to its box and rotated with it. | this spec section 4 | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- one image draw at the element box | none | verified |
| `TOILE-IO-051` | `OBS` | A picture whose pixels are not resident MUST paint a placeholder box, never a hole. | this spec section 4 | `ambre_toile::ui::render::tests::image_paints_pixels_when_resolved_else_placeholder` -- the fallback fill for the unresolved address in the same walk | none | verified |
| `TOILE-IO-034` | `OBS` | A picture the board names MUST reach the screen: an address MUST resolve to pixels. | this spec section 3; section 4 | `ambre_toile::logic::state::export::media_request_tests::a_media_request_answers_with_decoded_pixels` -- the Logic half; `ambre_toile::ui::render::tests::only_the_unresolved_pictures_are_asked_for` -- the UI half. The pixels are the thumbnail's, which is every pixel this side holds (`TOILE-IO-035`, `TOILE-IO-100`) | none | verified |
| `TOILE-IO-035` | `OBS` | A picture MUST show its thumbnail while the full blob loads, so the first paint waits on nothing. | this spec section 3; section 4 | not built | unbuilt -- lands with the `A deck exports at full resolution` Story (`ambre-roadmap.md` E6): the thumbnail is what reaches the screen now (`TOILE-IO-034`), and no full blob is ever fetched for a paint to wait on, section 6 | unimplemented |

### 1.2 Bring a picture in

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-052` | `PRE` | The caller MUST hand over bytes whose decoded form fits in memory. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::an_oversized_picture_is_refused_off_its_header` -- a whole header over a body that is not there, refused by the bound rather than by the decoder, which proves nothing was expanded; `ambre_toile::logic::interop::raster::tests::oversized_image_aborts_atomically` -- the same refusal on a whole file | none | verified |
| `TOILE-IO-130` | `POST` | A WebP picture MUST land on the board as PNG, decoded in the images converter module and never in the Main instance. | this spec section 2; `ambre-spec-architecture.md section 2`, the converter modules | `ambre_toile::logic::state::import::tests::a_webp_picture_lands_as_the_png_the_images_module_writes`; `::a_webp_picture_without_its_module_fails_and_changes_nothing`; `ambre_logic::dispatch::tests::a_webp_picture_waits_on_the_images_module`; `ambre_convert::engine::webp::tests::a_webp_picture_decodes_to_its_rgba_pixels`; ci-gate -- `scripts/converter_boundary.py` refuses `image-webp` in either instance | none | verified |
| `TOILE-IO-053` | `POST` | An import MUST yield one picture element sized to the decoded resolution and one plaintext blob it names. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::imports_one_image_with_thumbnail` -- the element kind, its size, the blob dimensions, the type and the address they share | none | verified |
| `TOILE-IO-054` | `POST` | An import MUST build a thumbnail bounded on each edge, so the entry stays small enough to ride in the document. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::thumbnail_is_bounded` -- it decodes the produced thumbnail and reads both edges against the bound | none | verified |
| `TOILE-IO-055` | `FRAME` | Converting a file MUST NOT write to the document: it MUST return a batch the state layer applies. | this spec section 3 | type-level -- the decoder takes bytes and returns a batch, and holds no handle on the document | none | verified |
| `TOILE-IO-009` | `FAIL` | A picture past the dimension bound MUST abort the import atomically. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::oversized_image_aborts_atomically` | none | verified |
| `TOILE-IO-056` | `FAIL` | Bytes that do not decode MUST abort the import atomically. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::garbage_bytes_fail_to_decode` -- the refusal; `ambre_toile::logic::state::import::tests::a_broken_file_fails_without_touching_the_board` -- the untouched board | none | verified |
| `TOILE-IO-102` | `POST` | A thumbnail of a picture smaller than the bound MUST keep the picture's own size. | this spec section 3 | `ambre_toile::logic::interop::raster::tests::a_small_picture_is_not_inflated` -- an 8 by 4 picture decodes to an 8 by 4 thumbnail | none | verified |

### 1.3 Wait for the upload, then land the import

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-008` | `POST` | A picture import MUST wait for its upload: nothing MUST be applied until every blob has the address Sync returned. | this spec section 3 | `ambre_toile::logic::state::import::tests::image_import_defers_until_media_uploads` -- one request issued, no delta and no element while it is outstanding | none | verified |
| `TOILE-IO-057` | `POST` | The last address to return MUST land the whole batch in one commit, elements and entries together. | this spec section 3 | `ambre_toile::logic::state::import::tests::image_import_defers_until_media_uploads` -- the element, the entry and the single delta once the confirmation arrives | none | verified |
| `TOILE-IO-058` | `POST` | A seeded entry MUST record the picture's dimensions and its type, so a box can be laid out before the pixels arrive. | this spec section 3 | `ambre_toile::logic::state::import::tests::seed_media_writes_load_info`; `ambre_toile::logic::state::import::tests::import_seeds_the_media_map` -- the same read through a whole import | none | verified |
| `TOILE-IO-036` | `POST` | Undoing an import MUST take back the media entries that import seeded. | this spec section 3 | `ambre_toile::logic::state::import::tests::undoing_an_image_import_takes_its_media_entry_back` | none | verified |
| `TOILE-IO-059` | `FRAME` | A seed MUST NOT put a placeholder token in the document: only an address Sync returned MAY become a key. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_placeholder_token_never_becomes_a_key`; `ambre_toile::logic::state::import::tests::seed_media_skips_unresolved_hash` -- the empty key; `ambre_toile::logic::state::elements::inner_tests::an_image_naming_a_pending_slot_is_refused` -- an image create naming a pending slot | none | verified |
| `TOILE-IO-060` | `FAIL` | A failed upload MUST abort the whole pending import and leave the board untouched. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_failed_upload_aborts_the_pending_import` -- the notice, the empty board and the drained registry | none | verified |
| `TOILE-IO-061` | `FAIL` | A confirmation matching no pending import MUST be dropped, and MUST change nothing. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_stale_service_response_is_dropped` | none | verified |

### 1.4 Ask the cache for a picture

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-062` | `POST` | A first request for an address MUST open a slot and order one download. | this spec section 3 | `ambre_toile::logic::interop::media::tests::first_request_fetches_then_dedups` | none | verified |
| `TOILE-IO-063` | `POST` | A repeat request for the same address MUST order nothing, whatever the slot holds. | this spec section 3 | `ambre_toile::logic::interop::media::tests::first_request_fetches_then_dedups` -- a slot in flight; `ambre_toile::logic::interop::media::tests::loading_to_ready_carries_pixels`; `ambre_toile::logic::interop::media::tests::loading_to_failed` -- the two terminal states | none | verified |
| `TOILE-IO-064` | `POST` | An arriving download MUST resolve its slot and hold the decoded pixels for the painter. | this spec section 3 | `ambre_toile::logic::interop::media::tests::loading_to_ready_carries_pixels` | none | verified |
| `TOILE-IO-065` | `POST` | A failed download MUST mark its slot failed, so a picture the relay cannot serve is not requested again on the next frame. | this spec section 3 | `ambre_toile::logic::interop::media::tests::loading_to_failed` | none | verified |
| `TOILE-IO-066` | `POST` | A cache over capacity MUST drop the least recently used resolved slot. | this spec section 3 | `ambre_toile::logic::interop::media::tests::lru_evicts_least_recently_used_ready_entry` -- it touches one of two ready slots and watches the other leave | none | verified |
| `TOILE-IO-067` | `POST` | A cache MUST have room for one blob however it was sized: a capacity of zero MUST be raised to one. | this spec section 3 | `ambre_toile::logic::interop::media::tests::capacity_zero_coerced_to_one` | none | verified |
| `TOILE-IO-007` | `FRAME` | Eviction MUST NOT take a slot whose download is in flight. | this spec section 3 | `ambre_toile::logic::interop::media::tests::loading_entries_are_never_evicted` -- it fills a cache of one with two in-flight slots and reads both back | none | verified |
| `TOILE-IO-103` | `FAIL` | Given a picture address the board does not carry, a request MUST answer nothing. | this spec section 3 | `ambre_toile::logic::state::export::media_request_tests::an_unknown_address_is_answered_with_nothing` -- the request answers no display event | none | verified |
| `TOILE-IO-105` | `POST` | A picture already asked for and still pending MUST NOT be asked for again on a later frame. | this spec section 4 | `ambre_toile::ui::render::tests::a_pending_request_is_not_repeated` -- a hash in the pending set yields no request, and the same list with nothing pending asks for it | none | verified |

* **Budget.** the first paint of a picture under 15 ms and no refetch on a revisit, two rows of
  `ambre-spec-budgets.md section 1`. The first is the budget the thumbnail exists to meet, and
  nothing reads the thumbnail (`TOILE-IO-035`). Neither row bounds memory, and three quantities here
  are unbounded, section 6.
* **Boundary.** this document owns Toile's only outbound flow of user content: media blobs, sealed
  by the Sync thread before they leave (`TOILE-IO-013`). No board geometry and no pixel leaves in
  plaintext.
* **Verification.** 32 `verified`, 1 `unimplemented`. The upload half holds: the address dedups, and
  a failed upload takes the import with it. An undo takes the entry back, and no placeholder becomes
  a key. A decoded picture stays out of the document. The thumbnail shown during a download is not
  built (`TOILE-IO-035`), because nothing downloads yet.

---

## 2. WHAT EACH USER SEES

The address is shared and the pixels are not: each peer fetches for itself. The last column joins the
example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a picture | watches | one address and one entry (`TOILE-IO-006`) | Alice sees her picture; Bob sees a box until his own fetch lands (`TOILE-IO-034`) | `logic::state::import::tests::import_seeds_the_media_map` |
| imports the same picture twice | watches | one entry, two elements (`TOILE-IO-006`) | two pictures, one blob | `logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| imports a picture whose upload fails | watches | nothing (`TOILE-IO-008`) | an error on Alice's screen and no change on Bob's | `logic::state::import::tests::a_failed_upload_aborts_the_pending_import` |
| imports a picture, then undoes | watches | neither the element nor its entry (`TOILE-IO-036`) | the picture leaves in one step, on both boards | `logic::state::import::tests::undoing_an_image_import_takes_its_media_entry_back` |

---

## 3. MODEL

**Container.** `media`, one entry per address, holding the dimensions, the type and an inline
thumbnail -- never the pixels (`ambre-spec-toile-schema.md section 3`). The picture element carries
the address alone.

**The service plane** (`ambre-bus`): media reuses the shared request plane rather than a queue of
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

1. read the dimensions off the header and refuse a picture past the bound before anything is
   expanded (`TOILE-IO-052`);
2. decode under that same bound, so a header that lies about its size is refused by the decoder.
   Then measure the decoded form and build a bounded thumbnail (`TOILE-IO-009`);
3. hand the plaintext up as an upload and apply nothing yet (`TOILE-IO-008`);
4. rewrite the pending import slot to the address the Sync thread returns, in the blob and in every
   element that named it. A slot is no address, so an image create naming one is refused;
5. seed the entry when the address is new, and leave it alone when it is not (`TOILE-IO-006`);
6. commit once, with the elements, so the entry belongs to the import's undo item
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

**Two of the four rows are the same box today** (`TOILE-IO-034`). The first two have become one: a picture
paints its thumbnail, which is every pixel this side holds. That makes "resident" and "loading" the same
image, and the painter still has two states rather than four (`TOILE-IO-050`, `TOILE-IO-051`). What draws no
distinction is the pair below: nothing orders a download, so no fetch is ever in flight and none can fail.

**The view asks, the board answers.** The store the painter consults lives on the UI thread and the pixels
live on the Logic one, so the crossing is a command. A viewport reply names the addresses it carries, and the
view asks for the ones it cannot paint. Each is answered with decoded pixels (`RequestMedia`, `MediaDecoded`).
Only what survived the cull is asked for, and an address already asked for is not asked again. Otherwise a
picture the document cannot resolve would be re-requested on every frame that fails to paint it.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A picture is addressed by the hash of its content | a minted id per picture | it uploads the same bytes once per use. Under a content hash the same picture pasted ten times is one blob and one upload, and the address is stable across peers with no coordination |
| The Sync thread computes the address | hashing on the Logic side | it puts a second crate on the crypto path for one call. The address is the hash of the plaintext and the Sync thread is the only crypto consumer, so hashing there keeps every primitive behind one wall |
| The document holds an address, never pixels | storing the bytes in the document | it is simpler and unbounded: a board with fifty photographs becomes a CRDT nobody can merge, and every peer carries every byte of history |
| The entry carries a thumbnail inline | fetching the full blob before painting | it leaves a hole on screen for as long as the transfer takes. The first paint must not wait on the network, and a thumbnail is small enough to ride in the document |
| An import waits for every upload | applying the elements first | the picture converges before the pixels do, so an element naming an address that does not exist yet is a broken picture on every other peer, and the failure arrives after the undo item closed |
| The cache never evicts a load in flight | evicting strictly by recency | it loses in-flight work under pressure: the answer resolves into a slot that is gone, and the request is re-issued on the next paint |
| A failure is a cache state, not a retry | leaving the slot empty | it turns one failure into a request loop, and a picture the relay cannot serve is re-requested on every frame of a pan |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing downloads** (`TOILE-IO-100`). An address resolves to the thumbnail's pixels, which the
  document already holds (`TOILE-IO-034`). The cache is complete, tested and exported, with no
  caller outside its own tests. The download request exists on the bus and in the dispatcher, and no
  code in Toile ever builds one. The return leg is missing too: the service handler matches the
  upload confirmation alone and drops a download answer. Wiring the request is not enough.
* **A picture never gets sharper than its thumbnail** (`TOILE-IO-035`, `TOILE-IO-100`). Both paths read the
  same 256 px inline preview. The screen reads it because that is what crosses the bus, the export because
  that is what the document holds. Nothing downloads the full blob. The state the rule names -- the thumbnail
  shown *while* the real pixels arrive -- has no second half to give way to. A board is readable and
  unprintable, and `TOILE-IO-035` stays `unimplemented` for want of a distinction to assert.
* **Every picture crosses the bus as raw pixels.** A `MediaDecoded` carries `width * height * 4` bytes. The
  queue is shared memory and the vector moves rather than copies. The cost is the decode and the peak, not a
  memcpy per frame. But nothing bounds how many answers one viewport reply can produce, and a board opening on
  thirty pictures decodes thirty thumbnails in one command.
* **An entry outlives its last element, and no rule says what should happen.** Deleting a picture
  drops the element and leaves the entry in `media` for good. Eager removal is wrong -- a delete is
  undoable, and another element may share the address -- and no reference count exists. The document
  therefore grows by one entry per picture ever imported, and nothing bounds it.
* **The cache has no owner.** It is a type with a capacity and no instance: no engine field holds one. Its
  capacity is never chosen, and its eviction never runs outside tests.
* **A local blob is never re-used across documents.** Content addressing would let two boards share
  one fetch, and the cache would have to leave the engine to do it.
* **The thumbnail persists in plaintext on the device** (`TOILE-IO-037`). It rides in the document,
  and local storage keeps the document unencrypted at rest, like its text. The rule bans only the
  decoded pixels, which the cache holds in memory. Sealing the local store is ruled, not built
  (`RESID-025`).

* **The plaintext is held whole beside the decoded form.** The blob carries the file's own bytes
  while the decode holds the expanded picture, so a picture at the bound costs both. No rule caps
  the file's size, only the dimensions it declares.

* **The thumbnail is bounded on its edges and not in bytes.** The rule the entry must meet is that
  it rides in the document (`TOILE-IO-054`); what is enforced is the longest edge. A noisy picture
  at the bound still encodes to a couple of hundred kilobytes, inline, in a CRDT that keeps every
  version of it.

* **The dedup is a seed-time dedup, not an upload-time one.** The same picture pasted ten times in one import
  carries ten placeholder addresses and leaves as ten uploads. The addresses collapse only when Sync answers
  and the seed skips the key it already holds. The decision in section 5 holds on its first half -- one entry
  -- and not on its second.

* **A pending import has no deadline, and holds each picture twice.** The bytes are cloned into the upload
  request and kept in the batch until the answer lands. A 20 MB photograph therefore occupies 40 MB for as
  long as the Sync thread takes, and forever if it never answers. Nothing expires the entry.

* **An abandoned import leaves its other uploads in flight.** A failure drops the pending import and cancels
  nothing. The blobs that already reached the relay stay there, sealed, named by an address no document will
  ever carry.

* **The cache can exceed its capacity without a bound.** Eviction walks for a slot that is not in flight and
  stops when it finds none. A cache whose loads never answer grows past its capacity, with nothing to stop it.
  The comment calls this transient; nothing makes it so.

* **A doc comment describes a path that does not exist.** The painter's store is documented as the
  one the PDF export fills from the document's media map. The export takes such a store, has no
  caller outside its tests, and every test hands it an empty one.
* **No test drives the transfer end to end.** The upload round trip is held at the dispatcher with a
  mock, and the download half has nothing to hold.
