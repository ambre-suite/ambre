# AMBRE -- FEATURE: THE MURMURE COMPOSER
**Type:** Vertical (feature)
**Implementation:** partial -- a plain-text composer at the foot of the channel's region.

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
| `MURMURE-193` | `INV` | The composer MUST be an instance of the Prose editor, restricted to the Murmure mark set. | this spec section 3; `features/ambre-spec-prose-engine.md` | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-194` | `INV` | The Murmure mark set MUST be bold, italic, inline code, code block, link, list, quote, mention and custom emoji. | this spec section 3; `MURMURE-008` | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-195` | `INV` | A draft MUST be kept per channel while the user moves between channels. | this spec section 3 | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` -- a draft typed in one channel is there when the user comes back from another | none | verified |
| `MURMURE-196` | `OBS` | The composer MUST show no permanent formatting toolbar. | this spec section 3 | `ambre_murmure::view::tests::the_band_holds_a_field_and_no_toolbar` -- the band paints its fill, the field and its outline, and no other shape, under a list cut to the band's top | none | verified |
| `MURMURE-210` | `INV` | A draft MUST survive a reload of the page. | this spec section 3; `ambre-spec-launcher.md` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-211` | `INV` | A draft MUST NOT leave the device before it is sent. | this spec section 3; `ZK-007` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-214` | `INV` | Eviction from the local store MUST NOT drop a draft. | this spec section 3; `RESID-009` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-430` | `INV` | The composer MUST be canvas content, fed by the shared input pipe. | this spec section 3; `UI-THREAD-014`; `ambre-spec-text-input.md` | `ambre_murmure::view::tests::the_composer_takes_the_pipe_until_escape` -- a press in the band focuses the pipe, and its text and keys reach the draft, never the list, until Escape; `ambre_murmure::view::tests::paste_copy_and_cut_go_through_the_draft` -- a paste lands in the draft, and a copy or a cut reads its selection; `ambre_murmure::view::tests::a_tap_in_the_band_focuses_the_composer` -- a tap focuses the pipe, and a press in the list takes the focus back | none | verified |
| `MURMURE-431` | `INV` | A formatting chord of the composer MUST invoke the Prose command of the same mark. | this spec section 3; `UI-THREAD-019` | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-432` | `OBS` | A selection in the composer MUST show the Prose format bar, with the Murmure marks alone. | this spec section 3; `MURMURE-194` | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Type Markdown

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-197` | `POST` | A Markdown marker MUST become its mark as soon as it closes. | this spec section 3 | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-198` | `FRAME` | A marker typed inside code MUST stay text. | this spec section 3 | not built | unbuilt -- lands with the `Marks in a message` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Send

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-199` | `POST` | Enter MUST send the draft as a message. | this spec section 3; `features/ambre-spec-murmure-messages.md` | `ambre_murmure::view::tests::enter_sends_the_draft_and_shift_enter_breaks_the_line` -- Enter sends the draft's text at the page's clock, and the draft is gone | none | verified |
| `MURMURE-200` | `POST` | Shift+Enter MUST insert a line break. | this spec section 3 | `ambre_murmure::view::tests::enter_sends_the_draft_and_shift_enter_breaks_the_line` -- Shift+Enter puts a line break in the draft and sends nothing | none | verified |
| `MURMURE-201` | `FAIL` | Given an empty draft, Enter MUST send nothing. | this spec section 3 | `ambre_murmure::view::tests::an_empty_or_blank_draft_sends_nothing` -- an empty draft, and one of white space alone, send nothing and stay | none | verified |
| `MURMURE-212` | `POST` | Sending a message MUST delete its stored draft. | this spec section 3 | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-202` | `FRAME` | Sending MUST NOT clear the draft of another channel. | this spec section 3 | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` -- a send in one channel leaves the draft of the other as it was, unsent | none | verified |

### 1.4 Edit a message

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-203` | `POST` | Editing MUST load the message into the composer and send an edit, never a new message. | this spec section 3; `MURMURE-084` | not built | unbuilt -- lands with the `Editing a message from the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-204` | `FRAME` | Cancelling an edit MUST NOT change the message. | this spec section 3 | not built | unbuilt -- lands with the `Editing a message from the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-259` | `FRAME` | Editing a message MUST NOT drop the draft of the channel: the draft MUST return once the edit is sent or cancelled. | this spec section 3; `MURMURE-195` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Lose access to a channel, or sign out

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-215` | `POST` | Losing access to a channel MUST delete its stored draft. | this spec section 3; `SYNC-109` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-216` | `POST` | Signing out MUST delete every stored draft. | this spec section 3; `ambre-spec-session.md` | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-217` | `FRAME` | Deleting the draft of one channel MUST NOT touch the draft of another. | this spec section 3 | not built | unbuilt -- lands with the `Drafts` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Undo while writing

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `MURMURE-433` | `POST` | An undo in the composer MUST revert the last edit of the draft. | this spec section 3; `UNDO-010` | not built | unbuilt -- lands with the `Undo in the composer` Story (`ambre-roadmap.md` E5) | unimplemented |
| `MURMURE-434` | `FRAME` | An undo in the composer MUST NOT reach a message already sent. | this spec section 3; `UNDO-001` | not built | unbuilt -- lands with the `Undo in the composer` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** a keystroke holds the input rows of `ambre-spec-budgets.md section 1`, as in Prose.
* **Boundary.** none: a draft stays in the client until it is sent, and a sent message is sealed
  like any delta (`ZK-001`).
* **Verification.** 7 `verified`, 18 `unimplemented`. The host tier drives the view with the
  events the page sends: a press, a tap, text, keys, a paste. The marks, the edit, the undo and the
  stored draft are not built.

---

## 2. WHAT EACH USER SEES

| Alice does | Alice sees | Test |
|---|---|---|
| types `**done**` | "done" in bold, as she closes the marker (`MURMURE-197`) | none |
| types `**x**` inside a code span | the asterisks, as text (`MURMURE-198`) | none |
| drafts in one channel, opens another, and comes back | her draft, as she left it (`MURMURE-195`) | `ambre_murmure::view::tests::each_channel_keeps_its_own_draft` |
| presses Enter on an empty composer | nothing sent (`MURMURE-201`) | `ambre_murmure::view::tests::an_empty_or_blank_draft_sends_nothing` |

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
| the user signs out | every draft deleted, so the next person on the device finds none (`MURMURE-216`) |

A sent message becomes a `content` text of `ambre-spec-murmure-schema.md section 3`.

**The Prose editor, restricted.** The composer reuses the text editing of Prose, with the mark set
of `MURMURE-194`. A mark outside the set is refused, and a paste keeps only the marks of the set.
The composer is canvas content. It takes its keys, its input method and its clipboard events from
the pipe of `ambre-spec-text-input.md`, like every editor of the suite (`MURMURE-430`).

**The commands of Prose, for the marks they share.** A mark of the set that Prose declares keeps
its Prose command, and so its chord: `Ctrl+B` bold, `Ctrl+I` italic, `Ctrl+E` inline code
(`MURMURE-431`). A selection shows the format bar of Prose, with the controls of the set alone
(`MURMURE-432`). The bar leaves with the selection, so nothing stays on screen (`MURMURE-196`).

**Undo.** A draft is a Prose document, so it has the local history of `ambre-spec-undo.md`.
`Ctrl+Z` and `Ctrl+Shift+Z` walk the edits of the draft (`MURMURE-433`). Sending ends that history:
a sent message is changed by an edit or a delete, never by an undo (`MURMURE-434`).

```rust
pub struct Draft { pub channel: ChannelId, pub text: LoroDoc, pub editing: Option<MessageRef> }
pub enum MurmureMark { Bold, Italic, InlineCode, CodeBlock, Link, List, Quote, Mention }
```

**Markdown, as typed.** The markers close on their second half: `**`, `_`, a backtick, three
backticks, `>` and `-` at the start of a line. A link is written `[text](address)`.

**The plain-text slice.** The composer built today holds no mark. It stands on the inline editor
the canvas crate shares, the one a board's text and a sheet's cell already use. Each channel holds
one draft, in memory. Keys reach the draft between frames, and the next paint applies them, so
Enter only asks for the send. That paint reads the draft, sends it unless it is blank, and drops it.

**Code.** `crates/ambre-murmure/src/composer.rs` holds the drafts and the band. The view in
`crates/ambre-murmure/src/view.rs` routes the pipe's events to it and sends the message.

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
| `Ctrl+Z`, `Ctrl+Shift+Z` | undo or redo the last edit of the draft |
| `Ctrl+B`, `Ctrl+I`, `Ctrl+E` | bold, italic, inline code, as in Prose |
| a selection | the format bar of Prose, with the Murmure marks |

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

* **A draft holds no mark.** Whether the marks come from the Prose editor or from a rich text
  layer the inline editor would share is not ruled. The marks wait for the
  `Marks in a message` Story (`ambre-roadmap.md` E5).
* **No message is edited from the composer.** The edit waits for the
  `Editing a message from the composer` Story (`ambre-roadmap.md` E5).
* **A draft has no undo.** The history waits for the `Undo in the composer` Story (`ambre-roadmap.md` E5).
* **A draft lives in memory.** A reload loses it, until the `Drafts` Story (`ambre-roadmap.md` E5).
* **`@` opens no panel.** The panel waits for the `The mention panel` Story (`ambre-roadmap.md` E5).
* **A thread has no composer.** A reply waits for the `Replying in a thread` Story (`ambre-roadmap.md` E5).
* **A screen reader does not hear the draft.** The pipe takes the text, and nothing reads the draft
  back. That waits for the `The composer read aloud` Story (`ambre-roadmap.md` E5).
