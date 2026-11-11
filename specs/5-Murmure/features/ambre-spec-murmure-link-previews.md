# AMBRE -- FEATURE: LINK PREVIEWS
**Type:** Vertical (feature)

> A link in a message shows a preview: the title and image of a web page, or the name of an Ambre
> document. The sender fetches it once, so no reader tells a website that they read the message.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-399` | `INV` | A preview of a web page MUST travel sealed inside its message. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-400` | `INV` | A preview of an Ambre document MUST be drawn only by a reader who can open that document. | this spec section 3; `SRV-047` | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Send a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-401` | `POST` | Sending a message with a web link MUST fetch its preview on the sender's client, once, and seal it in the message. | this spec section 3 | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-402` | `POST` | The sender MUST be able to remove a preview before sending. | this spec section 3 | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-403` | `FAIL` | Given a page the sender cannot fetch, the message MUST go without a preview. | this spec section 3 | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-404` | `FRAME` | Fetching a preview MUST NOT go through the relay. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.3 Read a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-405` | `POST` | A reader MUST draw a web preview from the message alone. | this spec section 3 | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `MURMURE-406` | `FRAME` | Reading a message MUST NOT make the reader's client request the linked page. | this spec section 3 | not built | unbuilt -- lands with the `Link previews` Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** a preview holds a title, a description and one small image, bounded by the
  attachment rows of `ambre-spec-budgets.md`.
* **Boundary.** the relay never fetches a page (`MURMURE-404`), and a reader never does either
  (`MURMURE-406`). Only the sender's client meets the website, once.
* **Verification.** 8 `unimplemented`: no preview exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The message holds | Each user sees | Test |
|---|---|---|---|---|
| sends a link to an article | reads the message | the preview, sealed | the title and image; Bob's client never calls the site (`MURMURE-406`) | none |
| sends a link to a Trame sheet | reads, as a member of the sheet | the document link | the name of the sheet (`MURMURE-400`) | none |
| sends a link to a Trame sheet | reads, without access to it | the document link | a plain link, and nothing of the sheet | none |

---

## 3. MODEL

**CRDT containers.** the `preview` of a message, in `ambre-spec-murmure-schema.md section 3`: a
title, a description, and an image through the shared media flow.

**The sender fetches, once.** Signal takes the same path. Every reader would otherwise request the
page, and tell the site who read the message and when. The sender meets the site once, and may
drop the preview before sending (`MURMURE-402`).

**An Ambre document previews from what the reader holds.** A link to a sheet, a board or a text
names a document. A reader who is a member draws its name from its own copy; anyone else sees a
plain link (`MURMURE-400`).

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| a link in the composer | its preview, with a control to remove it |
| a message with a preview | the title, the description and the image |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The sender fetches the preview | each reader fetches it | every reader would tell the site that they read the message |
| The sender fetches the preview | the relay fetches it | the relay would learn every link the members share |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No preview exists.
* **A preview can lie.** The sender writes it, so a sender can show a title the page does not have.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| How an image is uploaded | `ambre-spec-media.md` |
| Who can open a document | `ambre-spec-membership.md` |
