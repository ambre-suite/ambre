# AMBRE -- FEATURE: MURMURE ATTACHMENTS AND ANIMATED IMAGES
**Type:** Vertical (feature)
**Implementation:** partial -- a dropped file travels sealed, and a click saves or opens it. An
image previews, a GIF plays, and a delete erases them. The image of a link preview is not built.

> A member drops a file or an image into a message. The file travels sealed, the message holds a
> reference, and an animated GIF plays in place.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-128` | `INV` | An attachment MUST be sealed under the key of its channel. | this spec section 3; `ambre-spec-media.md` | `ambre_logic::channels::tests::a_file_uploads_and_downloads_under_the_channels_meta` -- the upload goes under the channel's meta document; `ambre_sync::media::tests::a_blob_opens_under_its_own_document_only` -- a blob sealed under one document key opens under no other | none | verified |
| `MURMURE-129` | `INV` | An attachment MUST NOT be addressed by a hash of its plaintext. | this spec section 3; `SYNC-050` | `ambre_sync::media::tests::an_upload_is_addressed_under_the_document_key` -- the same bytes in two documents get two addresses; `ambre_crypto::tests::a_media_address_needs_the_document_key` -- the address is never the bare hash of the bytes | none | verified |
| `MURMURE-130` | `OBS` | An animated GIF MUST stay still when the user asks for reduced motion. | this spec section 3 | `ambre_murmure::view::tests::under_reduced_motion_a_picture_never_asks_a_frame` -- no frame is due, and the frame loop never repaints for it; `ambre_murmure::pictures::tests::reduced_motion_holds_the_first_frame` -- the first frame shows | none | verified |
| `MURMURE-131` | `OBS` | An animated GIF off screen MUST NOT animate. | this spec section 3 | `ambre_murmure::view::tests::a_picture_off_screen_is_not_asked` -- a picture scrolled off is neither painted nor fetched; `ambre_murmure::paint::tests::an_image_previews_in_a_box_that_keeps_its_height_and_plays` -- only a preview painted names a next frame | none | verified |

### 1.2 Attach a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-132` | `POST` | An attachment MUST be uploaded through the shared media flow before its message is sent. | this spec section 3; `ambre-spec-media.md` | `ambre_murmure::view::tests::a_dropped_file_uploads_and_its_message_carries_it` -- a dropped file is uploaded, and the send carries it once the relay answered; `ambre_murmure::uploads::tests::a_file_joins_its_draft_once_its_upload_is_answered` -- no answer, no file in the draft; `ambre_logic::channels::tests::a_file_uploads_and_downloads_under_the_channels_meta` -- the upload is the shared media request | none | verified |
| `MURMURE-133` | `FAIL` | Given an upload that fails, the message MUST NOT be sent with a reference to it. | this spec section 3 | `ambre_murmure::view::tests::a_failed_upload_adds_nothing_to_the_message` -- the send after a refused upload carries no file; `ambre_murmure::uploads::tests::a_failed_upload_adds_nothing_and_names_its_file` -- the draft gains nothing | none | verified |
| `MURMURE-578` | `FAIL` | Given an upload that fails, the user MUST be told which file failed. | this spec section 3 | `ambre_murmure::view::tests::a_failed_upload_adds_nothing_to_the_message` -- the page is warned with the file's name | none | verified |
| `MURMURE-134` | `FRAME` | Attaching a file MUST NOT put a byte of it into a document. | this spec section 3; `MURMURE-017` | `ambre_murmure::attachment_tests::a_file_travels_by_its_address_and_reads_back_whole` -- the message holds the reference, the name, the type and the size, and reads back whole | none | verified |

### 1.3 Show an animated image

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-135` | `POST` | An animated GIF MUST be decoded off the render thread. | this spec section 3; `ambre-spec-architecture.md` | `ambre_logic::channels::tests::a_preview_is_fetched_once_and_reaches_the_view_decoded` -- the Logic thread fetches the file and hands the view its frames; `ambre_logic::pictures::tests::an_animated_gif_decodes_into_its_frames_at_the_preview_size` -- each frame, at the preview size | none | verified |
| `MURMURE-136` | `FRAME` | Animating a GIF MUST NOT stall the frame. | this spec section 3; `ambre-spec-budgets.md section 2` | `ambre_murmure::pictures::tests::the_clock_picks_the_frame_and_names_the_next` -- a paint picks a frame it holds, and decodes nothing; `ambre_murmure::view::tests::a_picture_on_screen_is_asked_once_and_plays_its_frames` -- the loop repaints when a frame is due, not before | none | verified |

### 1.4 Delete a message with attachments

A deleted message takes its files with it, so the erasure a delete promises covers them too.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-240` | `POST` | Deleting a message MUST ask the relay to erase every attachment no other live message of its author carries. | this spec section 3; `SRV-139` | `ambre_murmure::attachment_tests::a_delete_names_the_files_no_other_own_message_carries` -- each file once, none another own message keeps; `ambre_logic::channels::tests::a_delete_erases_its_file_and_retries_on_the_next_connection` -- the delete sends the erase | none | verified |
| `MURMURE-577` | `POST` | Deleting a message MUST ask the relay to erase the image of its preview. | this spec section 3; `SRV-139` | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-241` | `FAIL` | Given an erase the relay did not complete, the client MUST keep it queued and retry until it completes. | this spec section 3 | `ambre_logic::channels::tests::a_delete_erases_its_file_and_retries_on_the_next_connection` -- a failed erase is asked again on the next connection | none | verified |
| `MURMURE-242` | `FRAME` | Deleting a message MUST NOT erase a file another member uploaded. | this spec section 3; `SRV-140` | `ambre_server::routes::tests::handler_tests::an_erase_removes_a_blob_once_its_last_uploader_asks` -- the relay drops only the asker's claim, database tier; `ambre_murmure::attachment_tests::a_delete_names_the_files_no_other_own_message_carries` -- another author's message names nothing | none | verified |

* **Budget.** the frame budget of `ambre-spec-budgets.md section 2` bounds the animation, and the
  memory rows of `ambre-spec-budgets.md section 6` bound the decoded frames.
* **Boundary.** every attachment is sealed (`ZK-001`). Its address is keyed by the document key,
  as every media address is (`SYNC-050`).
* **Verification.** 13 `verified`, 1 `unimplemented`: the image of a link preview.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| drops a photo into the composer, and sends | watches | a message holding one reference | a chip naming the photo, on both screens | `ambre_murmure::view::tests::a_dropped_file_uploads_and_its_message_carries_it` |
| sends a message while the upload fails | -- | nothing of the file | Alice is told the file's name, and nothing half-sent (`MURMURE-133`) | `ambre_murmure::view::tests::a_failed_upload_adds_nothing_to_the_message` |
| -- | clicks the chip of a photo | one reference | the photo opens in a tab for Bob | `ambre_murmure::view::tests::a_file_clicked_is_fetched_and_handed_to_the_page` |
| sends an animated GIF | asks for reduced motion | one reference | the GIF plays for Alice and stays still for Bob (`MURMURE-130`) | `ambre_murmure::view::tests::under_reduced_motion_a_picture_never_asks_a_frame` |

---

## 3. MODEL

**CRDT containers.** the `attachments` list of a message, in `ambre-spec-murmure-schema.md
section 3`: a media reference, a name, a media type and a size.

**The media flow** is `ambre-spec-media.md`, the one every application uses. Murmure adds no path.
A file is stored under the channel's meta document, so it is sealed under the channel's key.

**A file joins the draft once its upload is answered.** A failed upload adds nothing.

**A click on a file** saves it. Only a raster image opens in a tab, since a tab runs in the
client's origin (section 5).

**An image previews** above the chips: a PNG, a JPEG or a GIF. The view asks the Logic thread
for each preview it paints. The Logic thread fetches the file and decodes it at the preview size.
Its box holds its height before the picture lands, so the list does not jump.

**An animated GIF** plays from the frames the view holds. A paint picks the frame its clock
reaches, and names when the next is due. The frame loop repaints then, and only for a preview on
screen. Under reduced motion the first frame stays.

**The decoded pictures** sit on one shelf both regions share, bounded by `ambre-spec-budgets.md
section 6`.

**A delete takes the files with it** (`MURMURE-240`). The text leaves the ciphertext at the next
re-finalization; the files leave object storage at once. An erase that fails, offline for
instance, stays queued until it completes (`MURMURE-241`). A file the author's other copy still
carries stays until that copy goes too.

**Code.** `ambre-murmure/src/attachment.rs`, `ambre-murmure/src/uploads.rs`,
`ambre-murmure/src/pictures.rs`, `ambre-logic/src/erasing.rs`, `ambre-logic/src/pictures.rs`, and
the page in `ambre-app/src/host/murmure.rs`.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| drop a file on the channel's canvas | an attachment, uploaded before the message leaves |
| a click on a file chip of the composer | the file, taken off the draft |
| a click on an image, its chip or its preview | the image, full size, in a new tab |
| a click on a file | the file, saved |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One media flow for every application | a Murmure upload path | a second path to seal, to address and to review |
| No GIF search | a search through a third-party service | each search would send what the user types to that service |
| A delete erases the files at once | keeping them until the re-finalization | nothing refers to a file once its message is empty, so waiting only delays the erasure the delete promised |
| Only a raster image opens in a tab | open every file the browser can show | a blob page runs in the client's origin, so an SVG or an HTML file would reach the session |
| The Logic thread decodes a preview | decode it in the view | a large GIF takes several frames' worth of time to decode, and the render thread would drop them |
| Each frame held decoded | decode the next frame when it is due | each step of the animation would decode on the render thread, the stall `MURMURE-136` bans |
| One shelf for both regions | a shelf per region | a picture on both would be decoded and held twice |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The erase queue lives in memory.** A reload before the retry leaves the blob.
* **No paste, and no drop on a thread.**
* **A send during an upload does not leave**, and the draft stays. The user presses Enter again once the file
  lands.
* **A failed upload gives two toasts**, the generic one and the one naming the file.
* **Only PNG, JPEG and GIF preview.** Any other image shows as its chip alone.
* **The shelf is Murmure's own**, apart from the one cache `CANVAS-007` asks for. The `One media
  cache` Story (`ambre-roadmap.md` E8) merges it.
* **A dropped picture is fetched and decoded again** when it comes back on screen. No decoded frame
  is kept on the device.
* **An animation keeps the frames that fit** in its byte bound. A long GIF loops over its start.
* **Each frame is its own texture.** An animation past the texture budget uploads its frames again
  on each loop.
