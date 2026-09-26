# AMBRE -- FEATURE: THE MURMURE COMPOSER
**Type:** Vertical (feature)
**Implementation:** design-only

> The field where a member writes. It takes light Markdown as it is typed, mentions and emoji, with
> no toolbar in the way. Enter sends, and each channel keeps its own draft.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `MURMURE`, shared with
`ambre-spec-murmure-schema.md` and every feature of this application.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-193` | `INV` | The composer MUST be an instance of the Prose editor, restricted to the Murmure mark set. | this spec section 3; `features/ambre-spec-prose-engine.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-194` | `INV` | The Murmure mark set MUST be bold, italic, inline code, code block, link, list, quote and mention. | this spec section 3; `MURMURE-008` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-195` | `INV` | A draft MUST be kept per channel while the user moves between channels. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-196` | `OBS` | The composer MUST show no permanent formatting toolbar. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-210` | `INV` | A draft MUST survive a reload of the page. | this spec section 3; `ambre-spec-launcher.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-211` | `INV` | A draft MUST NOT leave the device before it is sent. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-214` | `INV` | Eviction from the local store MUST NOT drop a draft. | this spec section 3; `RESID-009` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Type Markdown

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-197` | `POST` | A Markdown marker MUST become its mark as soon as it closes. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-198` | `FRAME` | A marker typed inside code MUST stay text. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Send

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-199` | `POST` | Enter MUST send the draft as a message. | this spec section 3; `features/ambre-spec-murmure-messages.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-200` | `POST` | Shift+Enter MUST insert a line break. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-201` | `FAIL` | Given an empty draft, Enter MUST send nothing. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-212` | `POST` | Sending a message MUST delete its stored draft. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-202` | `FRAME` | Sending MUST NOT clear the draft of another channel. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-203` | `POST` | Editing MUST load the message into the composer and send an edit, never a new message. | this spec section 3; `MURMURE-084` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-204` | `FRAME` | Cancelling an edit MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Lose access to a channel, or sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-215` | `POST` | Losing access to a channel MUST delete its stored draft. | this spec section 3; `SYNC-109` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-216` | `POST` | Signing out MUST delete every stored draft. | this spec section 3; `ambre-spec-session.md` | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-217` | `FRAME` | Deleting the draft of one channel MUST NOT touch the draft of another. | this spec section 3 | not built | unbuilt -- lands with the `The message list and the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a keystroke holds the input rows of `ambre-spec-budgets.md section 1`, as in Prose.
* **Boundary.** none: a draft stays in the client until it is sent, and a sent message is sealed
  like any delta (`ZK-001`).
* **Verification.** 19 `unimplemented`: no composer exists.

---

## 2. WHAT EACH USER SEES

| Alice does | Alice sees | Test |
|---|---|---|
| types `**done**` | "done" in bold, as she closes the marker (`MURMURE-197`) | none |
| types `**x**` inside a code span | the asterisks, as text (`MURMURE-198`) | none |
| drafts in one channel, opens another, and comes back | her draft, as she left it (`MURMURE-195`) | none |
| presses Enter on an empty composer | nothing sent (`MURMURE-201`) | none |

---

## 3. MODEL

**CRDT containers.** none while drafting: a draft is a local Prose document, never shared. It is
kept in the local store the launcher owns, so a reload finds it (`MURMURE-210`).

**How long a draft lives.** A draft is work the user has not sent, so no delay removes it and no
eviction drops it (`MURMURE-214`). It goes in three cases only.

| Case | The draft |
|---|---|
| the message is sent | deleted (`MURMURE-212`) |
| the user loses access to the channel: a leave, a revoke | deleted, since it names a channel the user may no longer read (`MURMURE-215`) |
| the user signs out | every draft deleted, so the next person on the device finds none (`MURMURE-216`) | A sent
message becomes a `content` text of `ambre-spec-murmure-schema.md section 3`.

**The Prose editor, restricted.** The composer reuses the text editing of Prose, with the mark set
of `MURMURE-194`. A mark outside the set is refused, and a paste keeps only the marks of the set.

```rust
pub struct Draft { pub channel: ChannelId, pub text: LoroDoc, pub editing: Option<MessageRef> }
pub enum MurmureMark { Bold, Italic, InlineCode, CodeBlock, Link, List, Quote, Mention }
```

**Markdown, as typed.** The markers close on their second half: `**`, `_`, a backtick, three
backticks, `>` and `-` at the start of a line. A link is written `[text](address)`.

**Code.** not built.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| Enter | send |
| Shift+Enter | a line break |
| `@` | the mention list of `features/ambre-spec-murmure-mentions.md` |
| `:` then a name | the emoji list of `features/ambre-spec-murmure-emoji.md` |
| a file dropped or pasted | an attachment (`features/ambre-spec-murmure-attachments.md`) |
| Escape while editing | the edit is cancelled |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Light Markdown as typed | a permanent formatting toolbar | more chrome on the view that must stay minimal |
| Light Markdown as typed | plain text with mentions alone | a code block is what a technical team shares most |
| A draft lives until it is sent or its channel is lost | a draft that expires after a delay | the user loses a message they were still writing, with no warning |
| The Prose editor, restricted | a second text editor for chat | two editors to keep aligned on input, selection and composition |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing is built.** No composer exists.
