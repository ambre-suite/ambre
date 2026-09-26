# AMBRE -- FEATURE: THE BLOB STORE, AND THE PRESIGNED LINK

**Type:** Horizontal (framework)
**Implementation:** partial -- the layout, the links and the caps all ship. Nothing removes an
object when the document that named it is deleted.

> Everything too large for a frame lives in object storage, sealed. The relay never carries those
> bytes: it checks who is asking and hands out a link that works once, for a quarter of an hour.

This document owns the object layout, the two links that reach it, and the caps a blob is held to.
What is inside a blob belongs to the document that sealed it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `SRV`, shared with
`ambre-spec-server.md` and the four other documents of this layer.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-004` | `INV` | A snapshot MUST be reachable only through the document it belongs to. | audit `C4`; this spec section 3 | `ambre_server::s3::tests::snapshot_key_validation_scopes_to_document` -- the lookup filters on the document and the snapshot together, so a member of one document cannot name another's | none | verified |
| `SRV-052` | `INV` | A presigned link MUST expire within fifteen minutes. | `ambre-spec-zero-knowledge.md`, `ZK-008` | none | unheld -- the lifetime is one operational figure with a fifteen-minute default and no ceiling, so a deployment may set a day with no code change, and no test reads the expiry back out of a link; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-059` | `INV` | The relay MUST NOT hold an address derived from a blob's plaintext. | `ZK-007`; `ambre-spec-media.md`, `SYNC-050` | none | fails -- the address is a hash of the plaintext, the confirmation stores it beside the document, and the object name is built from it, so the relay can hash a file it already has and ask whether this document holds it; lands with the `The media address is not the plaintext` Story (`ambre-roadmap.md` E7) | failing |
| `SRV-121` | `INV` | A Murmure channel MUST add no branch to the layout: each of its documents MUST take the layout of a document. | this spec section 3; `Todo/ambre-murmure.md` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Issue a presigned link

One gesture in two directions: the relay checks who is asking and what they named, then signs. The
refusals are the same set whichever way the bytes will travel, so the two share a table.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-053` | `POST` | A presigned upload link MUST bind the size the caller declared. | this spec section 3 | `ambre_server::s3::tests::presigned_upload_url_signs_content_length` -- the length is among the signed headers, which is what makes object storage refuse a body of any other size | none | verified |
| `SRV-078` | `FRAME` | Issuing a link MUST NOT read or write the object it names. | this spec section 3; `ZK-001` | none | unheld -- the relay signs a name and the transfer goes straight to storage, which is what keeps the bytes out of this process. Nothing refuses the convenience fetch that would put them back in it, and the rule is the reason no row here budgets a blob; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-002` | `FAIL` | A caller who is not a member MUST be refused a link. | `ZK-008`; this spec section 3 | none | unheld -- role refusals are covered by `ambre_server::routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` and `::rotate_key_denied_to_editor`, and no test asserts that a non-member is refused a link, so the gate is held for roles and not for membership; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-056` | `FAIL` | A read-only member MUST NOT obtain an upload link. | `SRV-008`; this spec section 3 | `ambre_server::routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` -- the confirmation, which is the write the link leads to | partial -- the link route gates on the same pair of roles and no test asks it; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | partial |
| `SRV-054` | `FAIL` | An object name a caller supplies MUST resolve inside the document the caller named. | audit `C4`, `C5`; this spec section 3 | `ambre_server::s3::tests::snapshot_key_validation_scopes_to_document` -- a foreign prefix, a traversal, an empty name and a nested one; `::content_hash_validation` -- everything that is not sixty-four lowercase hexadecimal characters | none | verified |
| `SRV-055` | `FAIL` | A blob MUST NOT exceed the cap its kind declares. | this spec section 3 | none | unheld -- the inline delta half is `SRV-010`. The media and snapshot halves are compared against a figure of the deployment before the link is issued and again on the confirmation, and no test drives either comparison; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-058` | `FAIL` | A download link MUST be refused for an object the document does not hold. | this spec section 3 | none | fails -- the media link is built from the address the caller supplies without reading the media index, so a member gets a signed link for any well-formed address inside their own document, section 6; lands with the `A download link names an object the document holds` Story (`ambre-roadmap.md` E7) | failing |

### 1.3 Delete a document

The relay drops the document's rows. What it named in storage is a second act, and it does not
happen.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-057` | `POST` | An object MUST NOT outlive the document it belongs to. | this spec section 3 | none | fails -- deleting a document removes its rows and no object, so every snapshot, every delta and every picture it ever held stays in storage with nothing left pointing at it; lands with the `A deleted document leaves nothing` Story (`ambre-roadmap.md` E7) | failing |
| `SRV-079` | `FRAME` | Deleting a document MUST NOT remove an object another document names. | this spec section 3; derived | none | unheld -- the reason it holds today is that nothing is removed at all. The object layout is keyed by document, so the sweep `SRV-057` is missing has one prefix to walk. A sweep written against the blob address instead would reach a picture two documents share; lands with the `The Sync and relay belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the presigned link of `ambre-spec-budgets.md section 3`, under 50 ms, and the
  document cold start of the same table, under 500 ms. The cold start's download half is here and
  its replay half is `ambre-spec-sequencer.md`; both read **none exists**.
* **Boundary.** every object here is sealed by the client and opaque to the relay (`ZK-001`), which
  reads only a name, a size and a media type. A picture's name is a hash of its plaintext, outside
  what `ZK-007` allows and what `SRV-059` states; the bytes never enter this process.
* **Verification.** 3 `verified`, 1 `partial`, 3 `failing`, 5 `untested`. What holds is the name: the scoping,
  the shapes refused, and the size bound into an upload link. What is loose is a link's lifetime and
  who may hold one -- neither the membership half of `SRV-002` nor the expiry has a test.

  Three rules are stated as failing, and one of the three is a zero-knowledge leak the relay shares
---

## 2. WHAT EACH USER SEES

Two peers over one document, sharing the address of a blob and never the bytes.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| adds a picture | opens the document | one media row: an address, a type, a size | Alice's picture, then Bob's once his own download lands | none -- section 6 |
| adds the same picture twice | watches | one media row | two pictures, one object | `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| is a reader and tries to add a picture | watches | nothing | Alice is refused before any link is signed (`SRV-056`) | `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` |
| records a snapshot naming another document's object | opens the document | nothing | Alice's record is refused (`SRV-004`, `SRV-054`) | `s3::tests::snapshot_key_validation_scopes_to_document` |
| asks for a link to an address her document never held | watches | nothing | Alice gets a signed link that resolves to nothing (`SRV-058`) | none, and it fails -- section 6 |
| deletes the document | had a picture in it | no row, and every object (`SRV-057`) | both lists lose it, and the bytes stay | none, and it fails -- section 6 |

---

## 3. MODEL

**CRDT containers.** none. An object is bytes under a name, and a document holds the name.

**The layout.** One prefix per deployment, then one branch per document.

* the deployment prefix
  * `documents`
    * one directory per document
      * `snapshots`, then one object per snapshot identifier
      * `deltas`, then one object per sequence number, zero-padded to ten digits so the names sort
      * `media`, then one object per address
      * `index`, one object, for a final Murmure segment only

The padding is what makes a listing of a document's deltas come back in order. A Murmure channel
adds no branch: each of its documents takes this layout under its own identifier (`SRV-121`). A
final segment adds one object, its sealed search index.

**Three kinds of blob**, and the caps and the paths differ for each.

| Kind | Reaches storage by | Cap | Named by |
|---|---|---|---|
| a delta | the relay's own flush (`ambre-spec-sequencer.md`) | 4 MiB, on the frame (`SRV-010`) | its sequence number |
| a snapshot | the client, on a presigned link | 50 MiB | an identifier the client mints |
| a picture or a file | the client, on a presigned link | 100 MiB | a hash of its plaintext (`SRV-059`) |

**A link, in steps.** The same four for an upload and a download, and the check is always first.

1. read the caller's membership, and the role where the link leads to a write (`SRV-002`,
   `SRV-056`);
2. refuse a name that is not the shape its kind declares (`SRV-054`);
3. refuse a declared size outside the cap of its kind (`SRV-055`);
4. sign a link that names one object, expires, and for an upload binds the declared size
   (`SRV-052`, `SRV-053`).

**Why the size is signed into the link.** A link that names an object and nothing else is a licence
to write any number of bytes under that name. Binding the declared length makes object storage
itself refuse a body of any other size, so the cap holds without the relay seeing the bytes.

**Why a name is validated rather than trusted.** Two names reach the relay from a client: the
snapshot object a record names, and the address of a picture. Both are interpolated into an object
name. A name carrying a traversal or another document's prefix would let a member of one document
obtain a link into another. A snapshot name must match this document's own prefix and end in one
element with no separator. An address must be sixty-four lowercase hexadecimal characters
(`SRV-054`).

**Two ways an object dies, and the gap between them.**

| Removal | What happens |
|---|---|
| a snapshot beyond the ten most recent | the hourly pass removes the object, then the row; a failure on the object still removes the row |
| a delta the trim covers | the row goes and the object stays (`ambre-spec-sequencer.md`, `SRV-039`) |
| a document deleted | the rows cascade and no object is touched (`SRV-057`) |
| a picture removed from a document | nothing: no rule says who collects it (`ambre-spec-media.md`) |

**What the relay knows about a picture.** Its address, its media type and its size, against the
document. The address is a hash of the plaintext. So the relay can take a file it already holds, hash it,
and answer whether this document contains it. That is a confirmation oracle over content, which
`ZK-007` does not allow (`SRV-059`).

**Code.**

* `crates/ambre-server/src/s3.rs` -- the three names, the two validators and the signing;
* `crates/ambre-server/src/routes/media.rs` -- the two media links and the confirmation;
* `crates/ambre-server/src/routes/sync.rs` -- the two snapshot links and the record;
* `crates/ambre-server/src/db/content.rs` -- the media index, and the snapshot removal.

---

## 4. SURFACE

The store has no surface of its own: every trigger belongs to the document that holds the blob.

| Trigger | Behaviour |
|---|---|
| a client seals a snapshot | it asks for an upload link, puts the bytes, then records the snapshot |
| a client opens a document | it asks for a download link for the current snapshot, then gets the bytes |
| a client adds a picture | it asks for an upload link, puts the bytes, then confirms the address |
| a client meets an address it does not hold | it asks for a download link, once per address |
| a link expires before the transfer completes | the client asks for another: a link names one object and nothing else |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The bytes never pass through the relay | uploading and downloading through the relay | it pays for every object twice, makes the relay a bandwidth bottleneck for content it cannot read, and lets it size every blob against every request. A signed link is one request and then no relay involvement at all |
| A link is signed after the membership is read, never before | checking the membership on the transfer | the relay never sees the transfer. The link is the capability, so signing first and checking after is the same as not checking -- the caller keeps the link. The order is the rule, and it is `ZK-008` on the boundary side (`SRV-002`) |
| The declared size is bound into an upload link | checking the size on the confirmation alone, after the bytes are already stored | the cap is then a promise the client keeps. Binding it makes storage enforce it, which is the only enforcement that survives a client that lies |
| A client-supplied name is validated against the document's own prefix | trusting the name a record carries | it is where audit `C4` came from: one interpolation into an object name is all a traversal needs, and the blast radius is another document's sealed content, reachable through a link this document's members legitimately receive (`SRV-054`) |
| A delta's name is its sequence number, zero-padded | a minted identifier per delta | it needs the index to be readable, where padded numbers sort in the order the deltas were accepted and a listing is a replay order with no index at all |
| An upload is confirmed in a second request | recording the blob when the link is issued | it indexes every abandoned upload, leaving rows that point at nothing. The relay never sees the transfer, so the only moment it can record a blob is when the client says it landed |
| Storage lifetimes are a bucket policy, not relay code | a collector inside the relay | it would put a retention policy in a binary, for an operational decision that differs per deployment -- and it would still not close `SRV-057`, the object of a deleted document, which is a defect rather than a policy |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Deleting a document deletes no object.** The rows cascade -- members, snapshots, deltas, media
  -- and every sealed object they named stays in storage, unreferenced, for ever. The content is
  unopenable once the member rows are gone, so the harm is not disclosure. It is unbounded growth
  and a claim of deletion the deployment cannot keep (`SRV-057`).
* **A media download link is signed for an address the document never held.** The link is built from
  the caller's own address without reading the media index. It resolves to nothing when the object is
  absent and to the object when it is present. Combined with the address being a hash of the
  plaintext, the second case is the same oracle from the other side (`SRV-058`, `SRV-059`).
* **A picture's address is a hash of its plaintext.** The client computes it and the relay stores
  it, so a fingerprint of every file in every document sits in the register
  (`ambre-spec-media.md`, `SYNC-050`, `SYNC-053`).
* **The link lifetime has a default and no ceiling.** Fifteen minutes is a figure of the deployment
  and nothing refuses a day, which is what `ZK-008` states (`SRV-052`).
* **A delta's object outlives its index row.** The trim removes the row and no pass removes the
  object, so a compacted document keeps every delta object it ever wrote. Today the trim never runs
  at all (`ambre-spec-sequencer.md`, `SRV-038`).
* **No test performs a round trip against object storage.** The suite signs links offline and
  asserts their shape, and the continuous integration has no storage service, which
  `ambre-spec-testing.md section 9` records as deliberate debt.
* **Nothing measures a link or a transfer.** Both rows of `ambre-spec-budgets.md section 3`
  that name this path read "none exists".

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What a client puts in a picture's object, and how it seals it | `ambre-spec-media.md` |
| What a snapshot contains, and when a client makes one | `ambre-spec-delta-order.md`, `ambre-spec-sequencer.md` |
| Which sequence number an object's name carries | `ambre-spec-sequencer.md` |
| Who may reach a document at all, and with which role | `ambre-spec-server.md` |
| How a payload is sealed and opened | `ambre-spec-sealed-envelope.md` |
| What a rotation does to an object already sealed | `ambre-spec-key-rotation.md` |
| What the relay may learn from a blob | `ambre-spec-zero-knowledge.md`, `ZK-007`, `ZK-008` |
| What is verified where, and by which tier | `ambre-spec-testing.md` |
