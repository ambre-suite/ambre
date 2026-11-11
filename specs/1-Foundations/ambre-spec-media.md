# AMBRE -- FEATURE: THE MEDIA TRANSFER

**Type:** Horizontal (framework)

> A picture in a document travels apart from it. The document holds an address; the pixels go
> straight to object storage, sealed, and come back the same way.

A picture is too large for the delta path and too large to sit inside a snapshot, so it takes a
transfer of its own. This document owns the address, the seal and the two legs; what a picture
*means* is each application's own feature.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SYNC`, shared with
`ambre-spec-sync.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-050` | `INV` | A blob's address MUST NOT be computable from its plaintext without the document key. | `ZK-007`; this spec section 3 | `ambre_sync::media::tests::an_upload_is_addressed_under_the_document_key` -- the worker answers with the keyed address, and the same bytes in two documents get two; `ambre_crypto::tests::a_media_address_needs_the_document_key` -- another key moves it, and it is never the bare hash of the bytes | none | verified |
| `SYNC-051` | `INV` | A sealed blob MUST be bound to the document it belongs to. | this spec section 3; `SYNC-018` | `ambre_sync::media::tests::a_blob_opens_under_its_own_document_only` -- a blob sealed for one document fails to open under another that holds the same key | none | verified |
| `SYNC-074` | `INV` | A sealed blob MUST be bound to the address that names it. | this spec section 3; `SYNC-018` | `ambre_sync::media::tests::a_blob_opens_under_its_own_address_only` -- a blob sealed under one address fails to open under another address of the same document | none | verified |

### 1.2 Add a picture

The application hands over the plaintext and waits for an address. The seal, the two requests and
the put are the Sync worker's.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-075` | `POST` | An upload MUST answer the requesting application with the blob's address. | this spec section 3, the upload steps | `ambre_sync::media::tests::a_failed_upload_step_confirms_nothing` -- an upload driven through a fake transport answers once, after the confirmation, with the request and the keyed address | none | verified |
| `SYNC-049` | `FRAME` | An upload MUST NOT emit a byte of the picture that is not sealed under the document key. | `ZK-001`; this spec section 3 | type-level -- the worker's only upload takes a `SealedBlob`, whose bytes only the seal builds; `ambre_sync::pipeline::tests::a_blob_bound_for_storage_is_sealed` -- such a blob opens back under the document key alone; `ambre_sync::http::tests::an_upload_carries_the_sealed_blob_as_its_body` -- the upload carries the sealed bytes whole as its body, with no credential | none | verified |
| `SYNC-053` | `FRAME` | An upload MUST NOT tell the relay anything about a blob beyond its size. | `ZK-007`; this spec section 3 | `ambre_sync::http::tests::an_upload_names_only_the_address_and_the_size` -- both requests serialize to the keyed address and the sealed size, and to no other field | none | verified |
| `SYNC-076` | `FAIL` | An upload that fails at any step MUST be reported, and MUST confirm nothing to the relay. | this spec section 2, row 3 | `ambre_sync::media::tests::a_failed_upload_step_confirms_nothing` -- a failing link, put or confirmation each reaches the application as an error for the request, with no address answered, and no failure before the confirmation reaches it | none | verified |
| `SYNC-145` | `POST` | An upload for a document the relay does not hold yet MUST wait for its creation. | this spec section 3, the upload steps | `ambre_sync::media::tests::an_upload_waits_for_its_document_on_the_relay` -- an upload on a local-only document answers nothing, then answers with the address once the document is created | none | verified |
| `SYNC-146` | `FAIL` | An upload that waits for a creation that fails MUST be reported. | this spec section 3, the upload steps | `ambre_sync::media::tests::an_upload_waits_for_its_document_on_the_relay` -- an upload on a local-only document answers nothing, then answers with an error once its creation fails | none | verified |

### 1.3 Show a picture

The application names an address it does not hold. The worker fetches the object and opens it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-077` | `POST` | A download MUST answer with the plaintext the address names. | this spec section 3, the download steps | `ambre_sync::media::tests::a_blob_that_fails_to_open_delivers_nothing` -- a download driven through a fake transport answers once, with the request, the address and the whole plaintext | none | verified |
| `SYNC-139` | `POST` | A download MUST fetch, byte for byte, the blob an upload put through the relay's link. | this spec section 3; `ambre-spec-object-store.md` | `ambre_server::routes::tests::handler_tests::a_download_fetches_the_bytes_an_upload_put` -- the relay signs both links, the test puts every byte value through the upload link to the object store of the database tier, garage in the continuous integration, and reads the same bytes back through the download link | none | verified |
| `SYNC-140` | `FRAME` | The browser transport MUST NOT alter a byte it puts to storage or fetches from it. | this spec section 3 | `ambre_belt::toile::tests::an_image_crosses_the_object_store_whole` -- browser tier: one user drops a picture on a new board, and a second user's screen shows its colour where it lies. The seal authenticates every byte, so a byte either leg altered would leave the picture unopened | none | verified |
| `SYNC-054` | `FRAME` | A download MUST NOT let a decrypted blob reach persistent storage. | `ZK-004`; this spec section 3 | ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, reads each path once it resolves, so an alias, a re-export and a macro name what they reach. It refuses a path of `ambre-sync` that reaches browser storage, the browser database, the cache or a cookie (`ambre_primitives::driver::bounds::browser_storage_under_a_foreign_name_is_refused_in_the_worker`) | none | verified |
| `SYNC-052` | `FAIL` | A download MUST refuse a blob that is not the one the address names. | this spec section 3 | `ambre_sync::media::tests::a_download_refuses_a_blob_the_address_does_not_name` -- it refuses a blob served in place of another, and one a member sealed under the address over other bytes, which only the recomputed address catches. It drives the open the leg calls, not the leg, which runs only in a browser | none | verified |
| `SYNC-055` | `FAIL` | A blob that fails to open MUST be reported, never delivered half-read. | this spec section 3 | `ambre_sync::media::tests::a_blob_that_fails_to_open_delivers_nothing` -- a failed link or fetch, and a short, altered or foreign blob, each reach the application as an error for the request, and no byte is answered | none | verified |

### 1.4 Take back a file

The application names an address its user uploaded and no longer shows. The worker asks the relay
to erase it (`ambre-spec-object-store.md`, `SRV-139`).

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-174` | `POST` | An erase MUST answer erased once the relay holds nothing of this user's under the address. | this spec section 3; `SRV-139`, `SRV-140` | `ambre_sync::media::tests::an_erase_answers_erased_unless_it_must_be_retried` -- a yes, a refusal and an absent document each answer erased, with the request | none | verified |
| `SYNC-175` | `FRAME` | An erase MUST NOT tell the relay anything beyond the document and the address. | `ZK-007`; this spec section 3 | type-level -- the transport's erase takes a link alone and sends no body, and the fake transport checks the link ends in the address | none | verified |
| `SYNC-176` | `FAIL` | An erase that met no answer, or a server error, MUST be reported. | this spec section 3; `SYNC-174` | `ambre_sync::media::tests::an_erase_answers_erased_unless_it_must_be_retried` -- a server error and a lost relay each reach the application as an error for the request, and answer nothing else | none | verified |

* **Budget.** none of its own for the transfer. `ambre-spec-budgets.md section 1` budgets the first
  paint of a picture and the absence of a refetch, and both are the reading application's. The seal
  of a blob is charged to the crypto rows of the same table.
* **Boundary.** this is the one path on which a whole file leaves the client. The bytes are sealed
  (`SYNC-049`). Beside them travel the size and an address keyed by the document key, which the
  relay cannot compute from a file it holds (`SYNC-050`, `SYNC-053`).
* **Verification.** 18 `verified`. The address, the two requests and each leg through a fake
  transport are held on the host. The database tier round-trips a blob through a real store
  (`SYNC-139`). The browser tier carries a picture from one user's board to another's screen
  (`SYNC-140`).

---

## 2. WHAT EACH USER SEES

Two peers over one document. The address is shared and the pixels are not: each peer fetches for
itself.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a picture | watches | one address (`SYNC-050`) | Alice's picture, then Bob's once his own fetch lands | none -- section 5 |
| adds the same picture twice | watches | one address, twice | two pictures, one blob | `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| adds a picture, and the seal or the put fails | watches | nothing | an error on Alice's screen and no change on Bob's (`SYNC-055`) | `ambre_toile::logic::state::import::tests::a_failed_upload_aborts_the_pending_import` |
| adds a picture, and the relay serves Bob a different blob | opens the document | one address | Bob sees an error where the picture was, never the wrong picture (`SYNC-052`) | `ambre_sync::media::tests::a_download_refuses_a_blob_the_address_does_not_name` |
| adds a picture, then removes a member | opens the document | one address | Bob, who stays, sees the picture: the blob opens through the key chain (`SYNC-042`) | `ambre_sync::media::tests::a_blob_sealed_before_a_rotation_opens_through_the_chain` |

---

## 3. MODEL

**CRDT containers.** none of its own. A document holds a picture's address inside its own container,
and that container belongs to the application: `3-Toile/ambre-spec-toile-schema.md` and
`4-Prose/ambre-spec-prose-schema.md` declare theirs.

**The address.** One string per blob, computed by the Sync worker because it is the only crypto
consumer on the client. It is content-derived, which is what makes the same picture used ten times
one upload and one object, with no coordination between peers. It is a keyed hash of the bytes: an
HMAC under a subkey of the document key. Every member reaches the same address, and the relay,
which holds no key, cannot compute it from a file it already has (`SYNC-050`).

**An upload, in steps.** A document the relay does not hold yet gets no link. The upload waits for
its creation, which it starts (`SYNC-145`).

1. compute the address from the bytes, under the document key;
2. seal the bytes under the document key, signed and committed (`ambre-spec-sealed-envelope.md`,
   `SYNC-018`);
3. ask the relay for a presigned link to put the object;
4. put the sealed bytes straight to object storage, with no session and no header of ours;
5. confirm to the relay: the address and the sealed size, and nothing else (`SYNC-053`);
6. answer the requesting application with the address.

**A download, in steps.**

1. ask the relay for a presigned link, by address;
2. get the object from object storage;
3. check the signature and the commitment tag, then open it under the document key. The document
   and the requested address are the additional data;
4. recompute the address from the plaintext, and refuse a blob whose address differs (`SYNC-052`);
5. answer with the plaintext (`SYNC-054`).

**The seal is the envelope, bound to the document and the address.** A blob carries the
identifier of its key (`SYNC-125`), its author key, the signature and the commitment tag. Its
additional data is a domain string, the document id and the address.

| The envelope gives a delta | A blob has |
|---|---|
| an author signature | the same: the open checks it against the key the blob carries |
| a commitment tag | the same, binding it to the key |
| additional data that binds the metadata to the ciphertext | the document id (`SYNC-051`) and the address (`SYNC-074`) |
| a compression step before the seal | nothing: a blob is sealed as it comes |

**Why compression is absent, and why that is not a row.** The formats a user adds are already
compressed, so a zstd pass over a PNG or a JPEG costs time and saves nothing. What the envelope's
version byte buys elsewhere is a choice per payload; here there is no choice to record.

**What the relay stores, and what that lets it do.** The relay keeps the address and the size
against the document. A plain hash would let it hash a known file, a photograph or a leaked page,
and learn whether this document holds it. The key closes that oracle. What remains is equality
inside one document: two uploads of one picture share an address (`SYNC-050`, `SYNC-053`). The
media type stays with the application.

**Code.**

* `crates/ambre-sync/src/media.rs` -- the address, the seal, the two legs, the open and the erase;
* `crates/ambre-server/src/routes/media.rs` -- the presigned links and the confirmation;
* `crates/ambre-crypto/src/aead.rs` -- the cipher the blob takes directly;
* `crates/ambre-crypto/src/commitment.rs` -- the keyed address.

---

## 4. SURFACE

The transfer has no surface of its own: every trigger belongs to the application that holds the
picture.

| Trigger | Behaviour |
|---|---|
| a picture added, pasted, dropped or imported | the application hands over the plaintext, and waits for the address |
| a document opened that names an address the client does not hold | the application asks for the blob, once per address |
| the same address named twice | one upload and one fetch, by construction |
| a failed upload | the application takes back whatever it was about to insert |
| a file its user takes back | the application names the address, and retries until the answer is erased |

What a user sees is a picture that appears, a placeholder while it loads, or nothing at all. Which
of the three, and what is painted in the meantime, is the application's own feature.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The additional data names the document and the address | one constant string for every blob | a blob of one document then opens under any other holding the same key, and any blob of a document passes for any other. The relay serves the wrong picture and no check notices |
| The download recomputes the address after the open | trusting the additional data alone | a member holds the key and the signing key, so a member can seal other bytes under an existing address. Only the recomputed address refuses them, for one HMAC over bytes already in memory |
| A blob travels apart from the document | pictures inside the document state | it is what makes a snapshot unbounded: a picture is megabytes and a delta is kilobytes, so a blob inside the CRDT state puts every picture in every snapshot, and opening a document downloads every picture it ever held |
| The address is content-derived | an identifier per picture, minted at insertion | it is the same string with none of the agreement: a content-derived address makes the same picture used ten times one object and one upload, and every peer reaches it with nothing to agree on |
| The address is keyed by the document key | a plain hash of the bytes | it is a confirmation oracle: the relay hashes a file it already holds and learns whether a document contains it. The key costs one HMAC and keeps the agreement, since every member holds it |
| The Sync worker computes the address | hashing in the Logic thread | it puts a second crate on the crypto path for one call. The worker is the only crypto consumer on the client, so hashing there keeps every primitive behind one wall and the application crates out of the cipher entirely (`TOILE-IO-013`) |
| The transfer goes straight to object storage | uploading through the relay | it pays for the bytes twice, makes the relay a bandwidth bottleneck for something it cannot read, and lets it size every blob against every request. A presigned link is one request and then no relay involvement at all |
| A blob is not compressed | compressing every blob, as the delta path does | the formats users add are already compressed, so a second pass costs time and saves nothing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A blob sealed before the binding no longer opens.** The older forms took the bare cipher or one
  constant string, and the download reads only the bound form.
* **The client bounds a blob nowhere.** The relay refuses one over its own cap and binds the
  declared size into the link (`ambre-spec-object-store.md`, `SRV-055`, `SRV-053`). This side
  states no maximum, so what a user may attempt is whatever the browser accepts.
* **Nothing expires a blob.** A picture removed from a document leaves its object in storage until
  the document itself is deleted (`ambre-spec-object-store.md`, `SRV-057`).
* **The addresses stored before the key are plain hashes.** The relay cannot rekey them, and no
  step migrates them. A document that held media before the change still exposes those addresses.
* **The download leg has no caller in two applications.** The worker's half ships. Toile paints the
  thumbnail the document holds and never fetches the blob (`TOILE-IO-100`), and Prose's pictures
  are unbuilt.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a board does with a picture, and how it caches one | `3-Toile/features/ambre-spec-toile-media.md` |
| What a page does with a picture, and how it lays one out | `4-Prose/features/ambre-spec-prose-images.md` |
| Which container holds an address | each application's schema document |
| The presigned link, its lifetime and the access check before it | `ambre-spec-object-store.md`, `SRV-002`, `ZK-008` |
| What a delta's seal gives that a blob's does not | `ambre-spec-sealed-envelope.md` |
| Which key seals a blob, and what a rotation does to it | `ambre-spec-key-rotation.md` |
| What the relay may learn at all | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| Where the decoded pixels are held, and what evicts them | each application's media feature |
