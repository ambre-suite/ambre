# AMBRE -- FEATURE: THE MEDIA TRANSFER

**Type:** Horizontal (framework)
**Implementation:** partial -- the upload and the download both ship in the Sync worker. A blob
is bound to neither its document nor its address.

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
| `SYNC-051` | `INV` | A sealed blob MUST be bound to the document it belongs to. | this spec section 3; `SYNC-018` | none | fails -- the seal's additional data is one constant string, shared by every blob of every document, so a blob sealed for one document opens under any other holding the same key; lands with the `A sealed blob is bound to its document and its address` Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-074` | `INV` | A sealed blob MUST be bound to the address that names it. | this spec section 3; `SYNC-018` | none | fails -- the same constant additional data names no address, so nothing ties the ciphertext to the string the document stores. It is the half `SYNC-052` needs and does not have; lands with the `A sealed blob is bound to its document and its address` Story (`ambre-roadmap.md` E7) | failing |

### 1.2 Add a picture

The application hands over the plaintext and waits for an address. The seal, the two requests and
the put are the Sync worker's.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-075` | `POST` | An upload MUST answer the requesting application with the blob's address. | this spec section 3, the upload steps | none | unheld -- the worker emits the address on the last step and no test drives the leg, which runs only in a browser. `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` reads what an application does with two equal addresses, never that the upload produced one; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-049` | `FRAME` | An upload MUST NOT emit a byte of the picture that is not sealed under the document key. | `ZK-001`; this spec section 3 | none | unheld -- the upload seals before it puts, and the crate graph keeps every application out of the cipher (`TOILE-IO-013`). Nothing here refuses a put that skips the seal, which is the same absence as `SYNC-001`; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-053` | `FRAME` | An upload MUST NOT tell the relay anything about a blob beyond its size. | `ZK-007`; this spec section 3 | `ambre_sync::http::tests::an_upload_names_only_the_address_and_the_size` -- both requests serialize to the keyed address and the sealed size, and to no other field | none | verified |
| `SYNC-076` | `FAIL` | An upload that fails at any step MUST be reported, and MUST confirm nothing to the relay. | this spec section 2, row 3 | none | unheld -- each step returns before the next runs, so a failed put never reaches the confirmation; `ambre_toile::logic::state::import::tests::a_failed_upload_aborts_the_pending_import` holds what one application does with the report, not that the worker sends one; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Show a picture

The application names an address it does not hold. The worker fetches the object and opens it.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SYNC-077` | `POST` | A download MUST answer with the plaintext the address names. | this spec section 3, the download steps | none | unheld -- the leg runs only in a browser and the suite performs no round trip against object storage; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-054` | `FRAME` | A download MUST NOT let a decrypted blob reach persistent storage. | `ZK-004`; this spec section 3 | none | unheld -- the worker opens no storage handle and hands the plaintext straight up, so the rule holds by absence. Nothing would fail the day a writer appeared; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SYNC-052` | `FAIL` | A download MUST refuse a blob that is not the one the address names. | this spec section 3 | none | fails -- the download opens whatever object storage returned and hands the plaintext up without recomputing the address, so a substituted blob under the same document key is accepted; lands with the `A sealed blob is bound to its document and its address` Story (`ambre-roadmap.md` E7) | failing |
| `SYNC-055` | `FAIL` | A blob that fails to open MUST be reported, never delivered half-read. | this spec section 3 | none | unheld -- every failure returns before the answer is built, a short blob, a fetch error and a bad tag each report and stop, so nothing partial is delivered. No test drives a failing download, because the leg runs only in a browser; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** none of its own for the transfer. `ambre-spec-budgets.md section 1` budgets the first
  paint of a picture and the absence of a refetch, and both are the reading application's. The seal
  of a blob is charged to the crypto rows of the same table.
* **Boundary.** this is the one path on which a whole file leaves the client. The bytes are sealed
  (`SYNC-049`). Beside them travel the size and an address keyed by the document key, which the
  relay cannot compute from a file it holds (`SYNC-050`, `SYNC-053`).
* **Verification.** 2 `verified`, 3 `failing`, 6 `untested`, 0 `unimplemented`. The address and
  the two requests are held on the host. The legs run in a browser, the host tier reaches neither,
  and no test round-trips against object storage. Three rows are stated as failing on a reading
  of the code.

---

## 2. WHAT EACH USER SEES

Two peers over one document. The address is shared and the pixels are not: each peer fetches for
itself.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a picture | watches | one address (`SYNC-050`) | Alice's picture, then Bob's once his own fetch lands | none -- section 5 |
| adds the same picture twice | watches | one address, twice | two pictures, one blob | `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| adds a picture, and the seal or the put fails | watches | nothing | an error on Alice's screen and no change on Bob's (`SYNC-055`) | `ambre_toile::logic::state::import::tests::a_failed_upload_aborts_the_pending_import` |
| adds a picture, and the relay serves Bob a different blob | opens the document | one address | Bob sees the wrong picture, with no error (`SYNC-052`) | none, and it fails -- section 5 |
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
3. check the signature and the commitment tag, then open it under the document key;
4. answer with the plaintext (`SYNC-054`).

**The seal is the envelope, with constant additional data.** A blob carries its author key, the
signature and the commitment tag. Its additional data is one constant string, so two of the
envelope's guarantees are still absent, and each absence is a row.

| The envelope gives a frame | A blob has |
|---|---|
| an author signature | the same: the open checks it against the key the blob carries |
| a commitment tag | the same, binding it to the key; the document is absent from the additional data (`SYNC-051`) |
| additional data that binds the metadata to the ciphertext | nothing that names the address (`SYNC-074`), which is why `SYNC-052` has nothing to check against |
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

The blob seal is absent from this table because no alternative was refused. The cipher was the
cheaper path, and the guarantees it drops were never weighed. The three absences are rows, not a
decision: `SYNC-051`, `SYNC-074` and `SYNC-052`. The repair is to give a blob the same additional
data the envelope gives a frame.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A blob travels apart from the document | pictures inside the document state | it is what makes a snapshot unbounded: a picture is megabytes and a delta is kilobytes, so a blob inside the CRDT state puts every picture in every snapshot, and opening a document downloads every picture it ever held |
| The address is content-derived | an identifier per picture, minted at insertion | it is the same string with none of the agreement: a content-derived address makes the same picture used ten times one object and one upload, and every peer reaches it with nothing to agree on |
| The address is keyed by the document key | a plain hash of the bytes | it is a confirmation oracle: the relay hashes a file it already holds and learns whether a document contains it. The key costs one HMAC and keeps the agreement, since every member holds it |
| The Sync worker computes the address | hashing in the Logic thread | it puts a second crate on the crypto path for one call. The worker is the only crypto consumer on the client, so hashing there keeps every primitive behind one wall and the application crates out of the cipher entirely (`TOILE-IO-013`) |
| The transfer goes straight to object storage | uploading through the relay | it pays for the bytes twice, makes the relay a bandwidth bottleneck for something it cannot read, and lets it size every blob against every request. A presigned link is one request and then no relay involvement at all |
| A blob is not compressed | compressing every blob, as the delta path does | the formats users add are already compressed, so a second pass costs time and saves nothing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A blob is bound to neither its document nor its address.** Its additional data is one constant
  string. Two blobs of one document are interchangeable to anyone who can serve the object, and a
  download recomputes no address to notice (`SYNC-051`, `SYNC-052`).
* **A blob sealed before the envelope no longer opens.** The old form took the bare cipher, and the
  download reads only the new one.
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
