# AMBRE -- FEATURE: A LINK TO A MESSAGE
**Type:** Vertical (feature)
**Implementation:** design-only

> A member copies a link to one message and pastes it anywhere. A member of the channel who opens it
> lands on that message; anyone else learns nothing.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-348` | `INV` | A message link MUST carry the channel, the segment and the message identifiers, and nothing else. | this spec section 3 | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Copy a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-349` | `POST` | Copying the link of a message MUST put the link on the clipboard. | this spec section 3; `ambre-spec-clipboard.md` | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-350` | `FRAME` | Copying a link MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Open a link

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-351` | `POST` | Opening a link MUST open its channel at the message, loading its segment. | this spec section 3; `MURMURE-023` | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-352` | `FAIL` | Given a link to a channel the user does not belong to, the client MUST say so, and show nothing of the channel. | this spec section 3; `SRV-047` | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-353` | `FAIL` | Given a link to a deleted message, the client MUST open the channel where it was, and show it deleted. | this spec section 3; `MURMURE-185` | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-354` | `FRAME` | Opening a link MUST NOT move a read cursor. | this spec section 3 | not built | unbuilt -- lands with the `A link to a message` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: opening a link loads one segment, as scrolling does.
* **Boundary.** a link carries identifiers alone (`MURMURE-348`): no key and no text. A person
  who is not a member holds no key, so the relay refuses the subscription and shows nothing.
* **Verification.** 7 `unimplemented`: no link exists in the code.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | Bob is | Bob sees | Test |
|---|---|---|---|---|
| copies the link of a message | opens it | a member of the channel | the channel at that message (`MURMURE-351`) | none |
| copies the link of a message | opens it | not a member | a notice that he has no access, and nothing more (`MURMURE-352`) | none |
| copies a link, then deletes the message | opens it | a member | the place where it was, shown deleted (`MURMURE-353`) | none |

---

## 3. MODEL

**CRDT containers.** none: a link names a message and holds none of it.

```rust
pub struct MessageLink { pub channel: ChannelId, pub message: MessageRef }  // no key, no text
```

**A link is an address, not an invitation.** It names three identifiers (`MURMURE-348`). A reader
needs the key of the channel, which only membership gives. So a link pasted into a public place
leaks the existence of a channel and a message, and nothing of their content.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| copy link, on a message | the link on the clipboard |
| a pasted link, opened | the channel at the message, or a notice of no access |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A link names identifiers | a link that carries the key | anyone holding the link would read the channel, which is an invitation without a signed entry |
| A link opens for members alone | a preview for anyone | the preview would need the key, which a non-member does not hold |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No link exists.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| Who belongs to a channel | `features/ambre-spec-murmure-channels.md` |
| How the clipboard carries a link | `ambre-spec-clipboard.md` |
