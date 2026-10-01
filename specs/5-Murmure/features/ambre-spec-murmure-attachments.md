# AMBRE -- FEATURE: MURMURE ATTACHMENTS AND ANIMATED IMAGES
**Type:** Vertical (feature)
**Implementation:** design-only

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
| `MURMURE-128` | `INV` | An attachment MUST be sealed under the key of its channel. | this spec section 3; `ambre-spec-media.md` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-129` | `INV` | An attachment MUST NOT be addressed by a hash of its plaintext. | this spec section 3; `SYNC-050` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-130` | `OBS` | An animated GIF MUST stay still when the user asks for reduced motion. | this spec section 3 | not built | unbuilt -- lands with the `Animated images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-131` | `OBS` | An animated GIF off screen MUST NOT animate. | this spec section 3 | not built | unbuilt -- lands with the `Animated images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Attach a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-132` | `POST` | An attachment MUST be uploaded through the shared media flow before its message is sent. | this spec section 3; `ambre-spec-media.md` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-133` | `FAIL` | Given an upload that fails, the message MUST NOT be sent with a reference to it, and the user MUST be told. | this spec section 3 | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-134` | `FRAME` | Attaching a file MUST NOT put a byte of it into a document. | this spec section 3; `MURMURE-017` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Show an animated image

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-135` | `POST` | An animated GIF MUST be decoded off the render thread. | this spec section 3; `ambre-spec-architecture.md` | not built | unbuilt -- lands with the `Animated images` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-136` | `FRAME` | Animating a GIF MUST NOT stall the frame. | this spec section 3; `ambre-spec-budgets.md section 2` | not built | unbuilt -- lands with the `Animated images` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Delete a message with attachments

A deleted message takes its files with it, so the erasure a delete promises covers them too.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-240` | `POST` | Deleting a message MUST ask the relay to erase every file it carries: its attachments and the image of its preview. | this spec section 3; `SRV-139` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-241` | `FAIL` | Given an erase the relay did not complete, the client MUST keep it queued and retry until it completes. | this spec section 3 | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-242` | `FRAME` | Deleting a message MUST NOT erase a file another member uploaded. | this spec section 3; `SRV-140` | not built | unbuilt -- lands with the `Attachments` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the frame budget of `ambre-spec-budgets.md section 2` bounds the animation, and the
  memory rows of `ambre-spec-budgets.md section 6` bound the decoded frames.
* **Boundary.** every attachment is sealed (`ZK-001`). Its address is keyed by the document key,
  as every media address is (`SYNC-050`).
* **Verification.** 12 `unimplemented`: no attachment exists in Murmure.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The channel holds | Each user sees | Test |
|---|---|---|---|---|
| drops a photo into the composer, and sends | watches | a message holding one reference | the photo in the message, on both screens | none |
| sends a message while the upload fails | -- | nothing | Alice is told, and nothing half-sent (`MURMURE-133`) | none |
| sends an animated GIF | asks for reduced motion | one reference | the GIF plays for Alice and stays still for Bob (`MURMURE-130`) | none |

---

## 3. MODEL

**CRDT containers.** the `attachments` list of a message, in `ambre-spec-murmure-schema.md
section 3`: a media reference, a name, a media type and a size.

**The media flow** is `ambre-spec-media.md`, the one every application uses. Murmure adds no path.

**An animated GIF** is an attachment like any other. The client decodes it off the render thread,
and the canvas plays its frames while it is on screen. The decoded images sit in the shared image
cache of `ambre-spec-render-assets.md`, bounded like every image the renderer draws.

**A delete takes the files with it** (`MURMURE-240`). The text leaves the ciphertext at the next
re-finalization; the files leave object storage at once. An erase that fails, offline for
instance, stays queued until it completes (`MURMURE-241`). A copy sent to the channel from a thread
shares the file, so it loses the file too: the author asked for it gone.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| drop or paste a file into the composer | an attachment, uploaded before the message leaves |
| a click on an image | the image, full size |
| a click on a file | the file, saved |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One media flow for every application | a Murmure upload path | a second path to seal, to address and to review |
| No GIF search | a search through a third-party service | each search would send what the user types to that service |
| A delete erases the files at once | keeping them until the re-finalization | nothing refers to a file once its message is empty, so waiting only delays the erasure the delete promised |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No attachment exists in Murmure.
