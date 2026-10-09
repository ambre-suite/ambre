# AMBRE -- FEATURE: THE BLOB STORE, AND THE PRESIGNED LINK

**Type:** Horizontal (framework)
**Implementation:** partial -- the layout, the links, the caps and the removal of a deleted
document's objects all ship. Nothing collects a picture a document stopped using.

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
| `SRV-052` | `INV` | A presigned link MUST expire within fifteen minutes. | `ambre-spec-zero-knowledge.md`, `ZK-014` | `ambre_server::s3::tests::a_presigned_url_never_outlives_fifteen_minutes` -- the signer caps any configured lifetime at 900 s, and the test reads the expiry back out of both links | none | verified |
| `SRV-059` | `INV` | The relay MUST NOT hold an address it could compute from a blob's plaintext. | `ZK-007`; `ambre-spec-media.md`, `SYNC-050` | `ambre_sync::media::tests::an_upload_is_addressed_under_the_document_key` -- the relay stores the address the Sync worker names, and the worker names one keyed by the document key, which the relay never holds; `ambre_crypto::tests::a_media_address_needs_the_document_key` -- without that key, the bytes alone do not give it | none | verified |
| `SRV-121` | `INV` | A Murmure channel MUST add no branch to the layout: each of its documents MUST take the layout of a document. | this spec section 3; `5-Murmure/ambre-spec-murmure-schema.md` | not built | unbuilt -- lands with the `SYNC P4` / `SRV P1` -- channels Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-138` | `INV` | A media row MUST record the member who uploaded its blob. | this spec section 3 | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Issue a presigned link

One gesture in two directions: the relay checks who is asking and what they named, then signs. The
refusals are the same set whichever way the bytes will travel, so the two share a table.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-053` | `POST` | A presigned upload link MUST bind the size the caller declared. | this spec section 3 | `ambre_server::s3::tests::presigned_upload_url_signs_content_length` -- the length is among the signed headers, which is what makes object storage refuse a body of any other size | none | verified |
| `SRV-078` | `FRAME` | Issuing a link MUST NOT read or write the object it names. | this spec section 3; `ZK-001` | none | unheld -- the relay signs a name and the transfer goes straight to storage, which is what keeps the bytes out of this process. Nothing refuses the convenience fetch that would put them back in it, and the rule is the reason no row here budgets a blob; lands with the `The relay's sessions and limits hold on every instance` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-002` | `FAIL` | A caller who is not a member MUST be refused a link. | `ZK-008`; this spec section 3 | `ambre_server::routes::tests::handler_tests::a_presigned_url_is_signed_only_after_the_access_check` -- an outsider is refused all four links, upload and download, media and snapshot | none | verified |
| `SRV-056` | `FAIL` | A read-only member MUST NOT obtain an upload link. | `SRV-008`; this spec section 3 | `ambre_server::routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` -- the confirmation, which is the write the link leads to; `::a_presigned_url_is_signed_only_after_the_access_check` -- a viewer is refused both upload links, which an owner obtains | none | verified |
| `SRV-054` | `FAIL` | An object name a caller supplies MUST resolve inside the document the caller named. | audit `C4`, `C5`; this spec section 3 | `ambre_server::s3::tests::snapshot_key_validation_scopes_to_document` -- a foreign prefix, a traversal, an empty name and a nested one; `::content_hash_validation` -- everything that is not sixty-four lowercase hexadecimal characters | none | verified |
| `SRV-055` | `FAIL` | A blob MUST NOT exceed the cap its kind declares. | this spec section 3 | none | unheld -- the inline delta half is `SRV-010`. The media and snapshot halves are compared against a figure of the deployment before the link is issued and again on the confirmation, and no test drives either comparison; lands with the `The relay's sessions and limits hold on every instance` Story (`ambre-roadmap.md` E9) | untested |
| `SRV-058` | `FAIL` | A download link MUST be refused for an object the document does not hold. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_download_link_names_an_object_the_document_holds` -- the link is signed only after the media index shows the document holds that address | none | verified |

### 1.3 Delete a document

The relay drops the document's rows. What it named in storage is a second act, and it does not
happen.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-057` | `POST` | An object MUST NOT outlive the document it belongs to by more than the late-write window, the lifetime of an upload link plus that of a flush. | this spec section 3 | `ambre_server::routes::tests::handler_tests::a_deleted_document_leaves_nothing` -- a delete removes every object under the document prefix before the rows go, and the second sweep removes one put after | none | verified |
| `SRV-079` | `FRAME` | Deleting a document MUST NOT remove an object another document names. | this spec section 3; derived | `ambre_server::routes::tests::handler_tests::a_deleted_document_leaves_nothing` -- the sweep walks the deleted document's own prefix, and an object of a second document survives it. A picture two documents share is stored once under each | none | verified |

### 1.4 Erase one blob

A member takes back a file they uploaded, such as the attachment of a message they deleted.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `SRV-139` | `POST` | An erase request MUST remove the blob from object storage, with its media row. | this spec section 3; `SRV-057` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-140` | `FAIL` | Given an erase request from a member who did not upload the blob, the relay MUST refuse it. | this spec section 3; `SRV-138` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-158` | `POST` | The owner of a channel, or an admin of its space, MUST be able to erase a file of that channel. | this spec section 3; `SRV-140` | not built | unbuilt -- lands with the `Moderation` Story (`ambre-roadmap.md` E5) | unimplemented |
| `SRV-141` | `FRAME` | Erasing a blob MUST NOT remove another object of the document. | this spec section 3; `SRV-079` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the presigned link of `ambre-spec-budgets.md section 3`, under 50 ms, and the
  document cold start of the same table, under 500 ms. The cold start's download half is here and
  its replay half is `ambre-spec-sequencer.md`; both read **none exists**.
* **Boundary.** every object here is sealed by the client and opaque to the relay (`ZK-001`), which
  reads only a name and a size. A picture's name is keyed by the document key (`SRV-059`); the
  bytes never enter this process.
* **Verification.** 10 `verified`, 2 `untested`, 6 `unimplemented`. What holds is the name: the
  scoping, the shapes refused, the size bound into an upload link, and who may hold a link. What
  is loose is a link that touches no object, and the cap per blob kind.
---

## 2. WHAT EACH USER SEES

Two peers over one document, sharing the address of a blob and never the bytes.

| Alice does | Bob does | The relay holds | Each user sees | Test |
|---|---|---|---|---|
| adds a picture | opens the document | one media row: an address, a type, a size | Alice's picture, then Bob's once his own download lands | none -- section 6 |
| adds the same picture twice | watches | one media row | two pictures, one object | `ambre_toile::logic::state::import::tests::seed_media_dedups_repeated_hashes` |
| is a reader and tries to add a picture | watches | nothing | Alice is refused before any link is signed (`SRV-056`) | `routes::tests::handler_tests::media_confirm_upload_denied_to_viewer` |
| records a snapshot naming another document's object | opens the document | nothing | Alice's record is refused (`SRV-004`, `SRV-054`) | `s3::tests::snapshot_key_validation_scopes_to_document` |
| asks for a link to an address her document never held | watches | nothing | Alice is refused, and no link is signed (`SRV-058`) | `handler_tests::a_download_link_names_an_object_the_document_holds` |
| deletes the document | had a picture in it | no row, and no object (`SRV-057`) | both lists lose it, and the bytes go with it | `handler_tests::a_deleted_document_leaves_nothing` |

---

## 3. MODEL

**CRDT containers.** none. An object is bytes under a name, and a document holds the name.

**The layout.** One prefix per deployment, then one branch per document.

* the deployment prefix
  * `documents`
    * one directory per document
      * `snapshots`, then one object per snapshot identifier
      * `deltas`, then one object per oversized delta, named by its sequence number, zero-padded to
        ten digits so the names sort
      * `batches`, then one object per batch of deltas, named by its first and last numbers
      * `media`, then one object per address
      * `index`, one object, for a final Murmure segment only

The padding is what makes a listing of a document's deltas come back in order. A small delta has
no object: it lives in its index row (`ambre-spec-sequencer.md`, `SRV-148`). A Murmure channel
adds no branch: each of its documents takes this layout under its own identifier (`SRV-121`). A
final segment adds one object, its sealed search index.

**Three kinds of blob**, and the caps and the paths differ for each.

| Kind | Reaches storage by | Cap | Named by |
|---|---|---|---|
| a delta | the relay's own flush (`ambre-spec-sequencer.md`) | 4 MiB, on the frame (`SRV-010`) | its sequence number |
| a snapshot | the client, on a presigned link | 50 MiB | an identifier the client mints |
| a picture or a file | the client, on a presigned link | 100 MiB | a hash of its plaintext, keyed by the document key (`SRV-059`) |

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
| a document deleted | a mark in the register comes first, so a pass finishes a delete a failure stopped (`ambre-spec-server.md`, `SRV-165`); a tombstone fences its keys, every object under its prefix goes, then the relay state, then the rows; once the late-write window has passed, a second sweep removes what a late writer put (`SRV-057`) |
| a picture removed from a document | nothing: no rule says who collects it (`ambre-spec-media.md`) |

**What the relay knows about a picture.** Its address and its size, against the document. The
address is keyed by the document key. The relay cannot hash a file it already holds and ask whether
this document contains it (`SRV-059`). It still sees that two uploads of one document share
an address. The media type never reaches it.

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
| Only the uploader erases a blob | any member of the document | a member could erase the files of everyone else, and the relay cannot read which message a blob belongs to |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A delta's object outlives its index row.** The trim removes the row and no pass removes the
  object, so a compacted document keeps every delta object it ever wrote. Today the trim never runs
  at all (`ambre-spec-sequencer.md`, `SRV-038`).
* **No test performs a round trip against object storage.** The suite signs links offline, and runs
  the delete against an in-process stub of the listing and the removal. The continuous integration
  has no storage service, which
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
