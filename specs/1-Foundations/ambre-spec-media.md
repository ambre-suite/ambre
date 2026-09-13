# AMBRE -- FEATURE: THE MEDIA TRANSFER

**Type:** Horizontal (framework)
**Implementation:** partial -- the upload and the download both ship in the Sync worker; the address
is a hash of the plaintext, and the relay reads it.

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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `SYNC-050` | `INV` | A blob's address MUST NOT be derived from its plaintext. | `ZK-007`; this spec section 3 | none, and **the rule fails today**: the address is a hash of the plaintext bytes and it travels to the relay in clear, so the relay can confirm that a document holds a file it already knows. Answered by the "The media address is not the plaintext" Story (`ambre-roadmap.md` E7) | unverified |
| `SYNC-051` | `INV` | A sealed blob MUST be bound to the document it belongs to. | this spec section 3; `SYNC-018` | none, and **the rule fails today**: the seal's additional data is one constant string, shared by every blob of every document, so a blob sealed for one document opens under any other holding the same key | unverified |
| `SYNC-074` | `INV` | A sealed blob MUST be bound to the address that names it. | this spec section 3; `SYNC-018` | none, and **the rule fails today**: the same constant additional data names no address, so nothing ties the ciphertext to the string the document stores. It is the half `SYNC-052` needs and does not have | unverified |

### 1.2 Add a picture

The application hands over the plaintext and waits for an address. The seal, the two requests and
the put are the Sync worker's.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `SYNC-075` | `POST` | An upload MUST answer the requesting application with the blob's address. | this spec section 3, the upload steps | none. The worker emits the address on the last step and no test drives the leg, which runs only in a browser. `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` reads what an application does with two equal addresses, never that the upload produced one | unverified |
| `SYNC-049` | `FRAME` | An upload MUST NOT emit a byte of the picture that is not sealed under the document key. | `ZK-001`; this spec section 3 | none. The upload seals before it puts, and the crate graph keeps every application out of the cipher (`TOILE-IO-013`). Nothing here refuses a put that skips the seal, which is the same absence as `SYNC-001` | unverified |
| `SYNC-053` | `FRAME` | An upload MUST NOT tell the relay anything about a blob beyond its size. | `ZK-007`; this spec section 3 | none, and **the rule fails today**: the confirmation carries the plaintext address and the media type, and neither is in the set `ZK-007` allows. Answered by the "The media address is not the plaintext" Story (`ambre-roadmap.md` E7) | unverified |
| `SYNC-076` | `FAIL` | An upload that fails at any step MUST be reported, and MUST confirm nothing to the relay. | this spec section 2, row 3 | none for the worker's half. Each step returns before the next runs, so a failed put never reaches the confirmation; `ambre_toile::logic::state::import::tests::a_failed_upload_aborts_the_pending_import` holds what one application does with the report, not that the worker sends one | unverified |

### 1.3 Show a picture

The application names an address it does not hold. The worker fetches the object and opens it.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `SYNC-077` | `POST` | A download MUST answer with the plaintext the address names. | this spec section 3, the download steps | none. The leg runs only in a browser and the suite performs no round trip against object storage | unverified |
| `SYNC-054` | `FRAME` | A download MUST NOT let a decrypted blob reach persistent storage. | `ZK-004`; this spec section 3 | none. The worker opens no storage handle and hands the plaintext straight up, so the rule holds by absence. Nothing would fail the day a writer appeared | unverified |
| `SYNC-052` | `FAIL` | A download MUST refuse a blob that is not the one the address names. | this spec section 3 | none, and **the rule fails today**: the download opens whatever object storage returned and hands the plaintext up without recomputing the address, so a substituted blob under the same document key is accepted | unverified |
| `SYNC-055` | `FAIL` | A blob that fails to open MUST be reported, never delivered half-read. | this spec section 3 | none. Every failure returns before the answer is built -- a short blob, a fetch error and a bad tag each report and stop -- so nothing partial is delivered; no test drives a failing download, because the leg runs only in a browser | unverified |

* **Budget.** none of its own for the transfer: `ambre-spec-budgets.md section 1` budgets the
  first paint of a picture under 15 ms and no refetch on a revisit, and both are the reading
  application's. The seal of a blob is charged to the crypto rows of the same table, per kilobyte.
* **Boundary.** this is the one path on which a whole file leaves the client. The bytes are sealed
  (`SYNC-049`), and the metadata beside them is not: the address and the media type reach the relay
  in clear, which is the leak `SYNC-050` and `SYNC-053` state. Nothing here holds a key of its own or
  opens a socket -- the two legs are ordinary requests, and the transfer itself goes straight to
  object storage on a presigned link.
* **Verification.** 0 `verified`, 11 `unverified`, 0 `unimplemented`. Nothing is held: the two legs
  run in a browser, the host tier reaches neither, and the suite performs no round trip against
  object storage at all (`ambre-spec-testing.md section 9`). Five of the eleven are stated as failing
  on a reading of the code, and two of those five are a zero-knowledge leak. The pass split
  `SYNC-051`, which bound a blob to its document and to its address in one row: they are two
  absences of the same constant, and the second is the half `SYNC-052` needs before it can refuse
  anything. It also gave each leg the two rules its steps state and no row held -- the address an
  upload owes its caller, the plaintext a download owes it -- and separated the upload's own failure
  from the download's, which the one row before it collapsed.

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
one upload and one object, with no coordination between peers. It is derived from the **plaintext**,
which is the defect (`SYNC-050`).

**An upload, in steps.**

1. compute the address from the bytes;
2. seal the bytes under the document key;
3. ask the relay for a presigned link to put the object;
4. put the sealed bytes straight to object storage, with no session and no header of ours;
5. confirm to the relay: the address, the media type and the sealed size (`SYNC-053`);
6. answer the requesting application with the address.

**A download, in steps.**

1. ask the relay for a presigned link, by address;
2. get the object from object storage;
3. split the nonce from the ciphertext and open it under the document key;
4. answer with the plaintext (`SYNC-054`).

**The seal is not the envelope.** A blob takes the cipher on its own, with one constant string as
additional data. Three of the envelope's guarantees are therefore absent from a blob, and each
absence is a row.

| The envelope gives a frame | A blob has |
|---|---|
| an author signature | nothing: a blob names no author |
| a commitment tag binding it to its document | nothing: the additional data is a constant (`SYNC-051`) |
| additional data that binds the metadata to the ciphertext | nothing that names the address (`SYNC-074`), which is why `SYNC-052` has nothing to check against |
| a compression step before the seal | nothing: a blob is sealed as it comes |

**Why compression is absent, and why that is not a row.** The formats a user adds are already
compressed, so a zstd pass over a PNG or a JPEG costs time and saves nothing. What the envelope's
version byte buys elsewhere is a choice per payload; here there is no choice to record.

**What the relay stores, and what that lets it do.** The relay keeps the address, the media type and
the size against the document. The address is a hash of the plaintext, so the relay can take a file
it already has -- a photograph, a leaked page, a known image -- hash it, and answer whether this
document contains it. That is a confirmation oracle over content, which `ZK-007` does not allow
(`SYNC-050`, `SYNC-053`).

**Code.**

* `crates/ambre-sync/src/media.rs` -- the address, the seal, the two legs and the open;
* `crates/ambre-server/src/routes/media.rs` -- the presigned links and the confirmation;
* `crates/ambre-crypto/src/aead.rs` -- the cipher the blob takes directly.

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

The blob seal is absent from this table because no alternative was refused: the cipher was
the cheaper path and the guarantees it drops were never weighed. The three absences are
rows, not a decision -- `SYNC-051`, `SYNC-074` and `SYNC-052` -- and the repair is to give a
blob the same additional data the envelope gives a frame.

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A blob travels apart from the document | pictures inside the document state | it is what makes a snapshot unbounded: a picture is megabytes and a delta is kilobytes, so a blob inside the CRDT state puts every picture in every snapshot, and opening a document downloads every picture it ever held |
| The address is content-derived | an identifier per picture, minted at insertion | it is the same string with none of the agreement: a content-derived address makes the same picture used ten times one object and one upload, and every peer reaches it with nothing to agree on |
| The Sync worker computes the address | hashing in the Logic thread | it puts a second crate on the crypto path for one call. The worker is the only crypto consumer on the client, so hashing there keeps every primitive behind one wall and the application crates out of the cipher entirely (`TOILE-IO-013`) |
| The transfer goes straight to object storage | uploading through the relay | it pays for the bytes twice, makes the relay a bandwidth bottleneck for something it cannot read, and lets it size every blob against every request. A presigned link is one request and then no relay involvement at all |
| A blob is not compressed | compressing every blob, as the delta path does | the formats users add are already compressed, so a second pass costs time and saves nothing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The address is a hash of the plaintext, and the relay reads it.** So the relay holds a
  fingerprint of every file in every document, and can confirm by hash whether a document contains a
  file it already knows. Nothing about the seal helps: the seal protects the bytes and the address
  travels beside it (`SYNC-050`, `SYNC-053`).
* **A blob is bound to nothing.** No signature, no commitment tag, and one constant string as
  additional data. Two blobs of one document are interchangeable to anyone who can serve the object,
  and a download recomputes no address to notice (`SYNC-051`, `SYNC-052`).
* **A rotation leaves every blob unreadable.** Nothing re-seals an uploaded blob, so every picture in
  a document goes blank for every member the first time someone is removed
  (`ambre-spec-key-rotation.md`, `SYNC-042`).
* **The media type reaches the relay in clear.** It says what kind of file a document holds, which is
  outside the set `ZK-007` allows and is not what `SYNC-050` is about.
* **The client bounds a blob nowhere.** The relay refuses one over its own cap and binds the
  declared size into the link (`ambre-spec-object-store.md`, `SRV-055`, `SRV-053`); this side
  states no maximum, so what a user may attempt is whatever the browser accepts.
* **Nothing expires a blob.** A picture removed from a document leaves its object in storage, and
  deleting the document leaves every one of them there (`ambre-spec-object-store.md`, `SRV-057`).
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
