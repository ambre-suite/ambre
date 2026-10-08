# AMBRE -- FEATURE: THE MEDIA TRANSFER

**Type:** Horizontal (framework)
**Implementation:** partial -- the upload and the download both ship in the Sync worker. A blob
is bound to its document and its address. No test drives either leg in a browser.

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
| `SYNC-049` | `FRAME` | An upload MUST NOT emit a byte of the picture that is not sealed under the document key. | `ZK-001`; this spec section 3 | type-level -- the worker's only upload takes a `SealedBlob`, whose bytes only the seal builds; `ambre_sync::pipeline::tests::a_blob_bound_for_storage_is_sealed` -- such a blob opens back under the document key alone | none | verified |
| `SYNC-053` | `FRAME` | An upload MUST NOT tell the relay anything about a blob beyond its size. | `ZK-007`; this spec section 3 | `ambre_sync::http::tests::an_upload_names_only_the_address_and_the_size` -- both requests serialize to the keyed address and the sealed size, and to no other field | none | verified |
| `SYNC-076` | `FAIL` | An upload that fails at any step MUST be reported, and MUST confirm nothing to the relay. | this spec section 2, row 3 | `ambre_sync::media::tests::a_failed_upload_step_confirms_nothing` -- a failing link, put or confirmation each reaches the application as an error for the request, with no address answered, and no failure before the confirmation reaches it | none | verified |

### 1.3 Show a picture

The application names an address it does not hold. The worker fetches the object and opens it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-077` | `POST` | A download MUST answer with the plaintext the address names. | this spec section 3, the download steps | `ambre_sync::media::tests::a_blob_that_fails_to_open_delivers_nothing` -- a download driven through a fake transport answers once, with the request, the address and the whole plaintext | none | verified |
| `SYNC-054` | `FRAME` | A download MUST NOT let a decrypted blob reach persistent storage. | `ZK-004`; this spec section 3 | ci-gate -- `scripts/crate_boundaries.py`, run by `just crate-boundaries` in the CI `lint` job, refuses a line of `ambre-sync` that names browser storage, the browser database, the cache or a cookie | none | verified |
| `SYNC-052` | `FAIL` | A download MUST refuse a blob that is not the one the address names. | this spec section 3 | `ambre_sync::media::tests::a_download_refuses_a_blob_the_address_does_not_name` -- it refuses a blob served in place of another, and one a member sealed under the address over other bytes, which only the recomputed address catches. It drives the open the leg calls, not the leg, which runs only in a browser | none | verified |
| `SYNC-055` | `FAIL` | A blob that fails to open MUST be reported, never delivered half-read. | this spec section 3 | `ambre_sync::media::tests::a_blob_that_fails_to_open_delivers_nothing` -- a failed link or fetch, and a short, altered or foreign blob, each reach the application as an error for the request, and no byte is answered | none | verified |

* **Budget.** none of its own for the transfer. `ambre-spec-budgets.md section 1` budgets the first
  paint of a picture and the absence of a refetch, and both are the reading application's. The seal
  of a blob is charged to the crypto rows of the same table.
* **Boundary.** this is the one path on which a whole file leaves the client. The bytes are sealed
  (`SYNC-049`). Beside them travel the size and an address keyed by the document key, which the
  relay cannot compute from a file it holds (`SYNC-050`, `SYNC-053`).
* **Verification.** 11 `verified`. The address and the two requests are held on the host. So is each
  leg end to end through a fake transport, up to the answer or the error it hands the application.
  What is loose is the browser transport itself, and no test round-trips against object storage.

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
| adds a picture, then removes a member | opens the document | one address | Bob sees nothing where the picture was, for good (`SYNC-042`) | none, and it fails -- `ambre-spec-key-rotation.md` |

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

**An upload, in steps.**

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

| The envelope gives a frame | A blob has |
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

* `crates/ambre-sync/src/media.rs` -- the address, the seal, the two legs and the open;
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
* **A rotation leaves every blob unreadable.** Nothing re-seals an uploaded blob, so every picture in
  a document goes blank for every member the first time someone is removed
  (`ambre-spec-key-rotation.md`, `SYNC-042`).
* **The client bounds a blob nowhere.** The relay refuses one over its own cap and binds the
  declared size into the link (`ambre-spec-object-store.md`, `SRV-055`, `SRV-053`). This side
  states no maximum, so what a user may attempt is whatever the browser accepts.
* **Nothing expires a blob.** A picture removed from a document leaves its object in storage until
  the document itself is deleted (`ambre-spec-object-store.md`, `SRV-057`).
* **The addresses stored before the key are plain hashes.** The relay cannot rekey them, and no
  step migrates them. A document that held media before the change still exposes those addresses.
* **No test performs a round trip against object storage.** The suite has none and the continuous
  integration has no object-storage service, which `ambre-spec-testing.md section 9` records as
  deliberate debt.
* **The download leg has no caller in two applications.** The worker's half ships; Toile's reader is
  the gap `TOILE-IO-034` states, and Prose's pictures are unbuilt.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a board does with a picture, and how it caches one | `3-Toile/features/ambre-spec-toile-media.md` |
| What a page does with a picture, and how it lays one out | `4-Prose/features/ambre-spec-prose-images.md` |
| Which container holds an address | each application's schema document |
| The presigned link, its lifetime and the access check before it | `ambre-spec-object-store.md`, `SRV-002`, `ZK-008` |
| What a frame's seal gives that a blob's does not | `ambre-spec-sealed-envelope.md` |
| Which key seals a blob, and what a rotation does to it | `ambre-spec-key-rotation.md` |
| What the relay may learn at all | `ambre-spec-zero-knowledge.md`, `ZK-007` |
| Where the decoded pixels are held, and what evicts them | each application's media feature |
