# AMBRE -- FEATURE: COMMENTS ON A DOCUMENT

**Type:** Vertical (feature)
**Implementation:** design-only

> A discussion attached to a passage of the text. The thread follows the passage through every
> edit. When the passage is deleted the thread is kept and listed rather than destroyed with
> the sentence that carried it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The two Stories below are the two halves
of this feature: the thread converges under one, and it is painted under the other.

**This feature took the horizontal test and failed it**, unlike presence, cut at the same step. Two
applications hold a comments feature, and the anchor is the feature.

| | Toile (`features/ambre-spec-toile-comments.md`) | Prose (here) |
|---|---|---|
| Anchor | an `ElementId`, or a free world point | a live mark on a text range |
| When the target dies | the pin dangles, and is not painted (`TOILE-025`) | the thread orphans, and is kept for review (`PROSE-FEAT-202`) |
| Thread order | the list's, never a timestamp (`TOILE-FEAT-012`) | the list's, never a timestamp (`PROSE-FEAT-186`) |
| Author | the canonical `UserId` (`TOILE-FEAT-014`) | a display name and a document peer, stamped locally (`PROSE-FEAT-077`) |

Two of the four match, and the two that do not are section 6.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-001` | `INV` | A comment's identity MUST be stable for the document's lifetime: an edit, a reply or a remote delta MUST NOT change it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-075` | `INV` | The live mark on the text, never the stored anchor, MUST decide where a comment sits. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-185` | `INV` | The stored anchor MUST NOT be read for a position while the comment's mark is alive. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-078` | `INV` | Each comment MUST own its own mark span. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-186` | `INV` | The order of a thread's replies MUST be the list's own, never a timestamp's. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-077` | `INV` | The author and the peer of a comment MUST be stamped from the session identity, never read from the command. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-187` | `INV` | Resolving, reopening or deleting a thread MUST NOT be refused on the caller's identity: only a reply is its author's alone. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-009` | `CONC` | A comment thread MUST stay anchored to its text range across concurrent edits. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-049` | `OBS` | A comment highlight MUST be canvas content, painted under the annotated range. | this spec section 4 | not built | unbuilt -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-144` | `OBS` | Two threads whose spans share characters MUST both be painted, one highlight over the other. | this spec section 3 | not built | unbuilt -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-050` | `INV` | The comment panel MUST be DOM chrome, not a canvas overlay. | this spec section 4; `ambre-spec-ui.md section 2` | not built | unbuilt -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Open a thread on a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-188` | `POST` | Opening a thread MUST write one annotation entry and one mark reaching the requested range and no character outside it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-189` | `FRAME` | Opening a thread MUST NOT change the text of the block, or the entry or the span of any other thread. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Reply to a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-190` | `POST` | A reply MUST be appended at the end of the thread's reply list. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-079` | `POST` | A reply to a resolved thread MUST reopen it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-191` | `FRAME` | Replying MUST NOT move the thread's mark, rewrite an earlier reply, or touch another thread. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Resolve or reopen a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-192` | `POST` | Resolving a thread MUST mark it closed. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-193` | `POST` | Reopening a thread MUST mark it open, with no reply added. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-194` | `FRAME` | Resolving or reopening a thread MUST NOT rewrite another thread's span, and MUST NOT add, remove or edit a reply of its own. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.5 Edit a reply

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-195` | `POST` | Editing a reply MUST replace that reply's text and leave its place in the list. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-080` | `FAIL` | Editing a reply MUST be refused unless the caller wrote it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-196` | `FRAME` | Editing a reply MUST NOT change the thread's `resolved` flag, its mark, or the author stamped on the reply. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.6 Delete a reply

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-197` | `POST` | Deleting a reply MUST remove it from the list and leave the order of the replies around it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-081` | `POST` | Deleting a thread's opening reply MUST delete the thread. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-198` | `FAIL` | Deleting a reply MUST be refused unless the caller wrote it. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-199` | `FRAME` | Deleting a reply that is not the head MUST NOT change the thread's mark, its `resolved` flag, or another thread. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Delete a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-200` | `POST` | Deleting a thread MUST remove its entry and its mark. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-279` | `POST` | Deleting a thread MUST leave the text its mark covered. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-201` | `FRAME` | Deleting a thread MUST NOT rewrite the span of any other thread over the same characters. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.8 Edit the text a thread is anchored to

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-076` | `POST` | The stored anchor MUST be rewritten from the mark on every edit that touches the annotated block. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-202` | `POST` | An edit that deletes the whole marked range MUST keep the entry, with the thread orphaned rather than re-anchored. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-203` | `POST` | An edit that takes the mark away MUST leave the stored anchor on the last position the mark held. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-204` | `FRAME` | An edit over an annotated range MUST NOT change the thread's `resolved` flag, its replies, or the entry of another thread. | this spec section 3 | not built | unbuilt -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own. A comment is an annotation entry and a mark, both written on the
  ordinary command path. The highlight is painted in the page pass
  `features/ambre-spec-prose-page-rendering.md` charges.
* **Boundary.** none of its own: a thread is content in the document, and `ZK-001` seals every
  payload before it leaves the client. The author field is stamped locally, a rule of this document
  rather than a boundary the relay sees.
* **Verification.** 33 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-009` is the row a property test reaches without a screen. Any interleaving of edits
  over an annotated range leaves the thread on that range or orphaned, never on another.
  `PROSE-FEAT-078` needs two peers: a single writer cannot make two spans race. The eight
  frame conditions are the rows one assertion cannot reach: each names a field the operation may
  not touch, read before and after.

---

## 2. WHAT EACH USER SEES

Two peers over one thread. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| comments on a sentence | types a word before it | the thread, on the same sentence, one word further down | the highlight on Alice's sentence, on both screens | none -- `PROSE-FEAT-009` |
| comments on a sentence | comments on three words inside it | two threads, two spans | both highlights, stacked over the three shared words | none -- `PROSE-FEAT-078`, `PROSE-UI-144` |
| resolves her thread | -- | both threads, one closed | Bob's highlight untouched, and Alice's out of the open list | none -- `PROSE-FEAT-194` |
| deletes the whole commented sentence | replies to that thread | the thread, with no mark left | the reply, and the thread listed as orphaned on both screens | none -- `PROSE-FEAT-202` |
| replies to a thread Bob resolved | -- | the thread, open again | the thread back in the open list, on both screens | none -- `PROSE-FEAT-079` |
| edits Bob's reply | -- | Bob's reply, unchanged | a refusal on Alice's screen, and nothing on Bob's | none -- `PROSE-FEAT-080` |
| deletes the opening reply of her own thread | replies to it at the same moment | no thread | the thread gone on both screens, and no highlight left | none -- `PROSE-FEAT-081` |

---

## 3. MODEL

**Containers.** The thread is one annotation entry, and the replies are the list inside it; the
anchor is an inline mark carrying the entry's identity. Both are declared in
`ambre-spec-prose-schema.md section 3`, and the identity is a value the peer that opens the thread
mints (`section 2` of that document).

**Two positional records, and only one of them is authoritative.** The mark on the text moves with
every insertion and deletion, because that is what a mark does. The `anchor_block`, `anchor_start`
and `anchor_end` fields of the entry are a **cache**, and they exist for two cases alone. Those are
rendering the thread before the block's runs are shaped, and holding the last known position of a
thread whose mark is gone. The mark wins every disagreement (`PROSE-FEAT-075`), and every edit touching an
annotated block rewrites the cache from the mark's current span (`PROSE-FEAT-076`). Nothing renders
from the cache while a live mark exists.

**The orphan.** Deleting the whole marked range deletes the mark with the text. The entry is
**not** removed (`PROSE-FEAT-009`): `resolved` is untouched, the cache freezes at the last known
position, and the view lists the thread as orphaned. Only the delete command removes a thread. The
footnotes feature applies the same policy to a note whose call is deleted.

**Overlap, and why it works.** A mark of one key is single-valued, so two threads over the same
characters cannot both be that one mark's value. They are **layered** instead. Each thread owns its
own span, and the newer value wins a raw read of the shared characters. Both entries stay live, and
both highlights are painted. Resolving or deleting one thread rewrites its own span and no other
(`PROSE-FEAT-078`).

**The identity, and where the author comes from.** The commands carry the text alone. The
dispatcher fills the author name and the peer from the **current session identity**, never from the
payload (`PROSE-FEAT-077`).

It is **not** the rule presence applies to a caret
(`1-Foundations/ambre-spec-presence.md`, `PRESENCE-002`, `PRESENCE-003`). Presence resolves the
author from the key that signed the frame and puts nothing in the payload. A comment stores a
name and a peer in the document, stamped locally. Both refuse a forged author; only one of them
writes an author down. Section 6 records the divergence.

**The three rules of a thread's life:**

* **The reply to a closed thread.** `ReplyToComment` on a resolved thread is allowed, and reopens
  it (`PROSE-FEAT-079`). `ReopenComment` is the path for reopening without adding a reply.
* **Whose reply may be touched.** Editing and deleting a reply are the author's alone
  (`PROSE-FEAT-080`), matched on the reply's peer. Resolving, reopening and deleting a **thread**
  are not restricted: any collaborator may close a discussion.
* **The head of the thread.** The opening comment is the first reply. Deleting it leaves a thread
  with no head, so it deletes the thread (`PROSE-FEAT-081`).

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `AddComment { block_id, range, text }` | a thread on that range, and the mark that reaches it |
| `ReplyToComment { comment_id, text }` | a reply, and a reopen if the thread was closed |
| `ResolveComment { comment_id }` | closes the thread |
| `ReopenComment { comment_id }` | reopens it, with no reply added |
| `DeleteComment { comment_id }` | removes the thread and its mark |
| `DeleteReply { comment_id, reply_id }` | removes one reply, or the thread when it is the head |
| `EditCommentReply { comment_id, reply_id, new_text }` | rewrites the caller's own reply |

**What the view is given.** One entry per visible thread: the identity, the absolute vertical
position of the anchor, whether it is closed, and the replies in order. The vertical position is the
view's, computed from the layout, which is why the panel can align to the text without holding a
document.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the comment gesture over a selection | a thread on that range, and the caret in the new reply |
| a click on a highlight | opens the thread in the panel |
| a click on a thread in the panel | moves the view to the anchor |
| the reply field of an open thread | a reply, and a reopen when the thread was closed |
| the resolve control of a thread | closes it, and takes it out of the open list |
| the delete control of a reply | removes that reply, or the thread when it is the head |
| the orphaned list | the threads whose text is gone, for review or deletion |

The feature is painted in two places, and the split is the canvas and DOM boundary
`ambre-spec-ui.md section 2` draws. The **highlight** is canvas content: a tinted rectangle under
the annotated range, painted in the page pass (`PROSE-UI-049`). The **panel** is a DOM Leptos
element to the right of the page, scrolled by the browser. Each thread is aligned to the vertical
position the view publishes for its anchor (`PROSE-UI-050`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The thread lives in the document's own annotation map | a second `LoroDoc` for the comments | it doubles the synchronization and makes consistency between the text and the threads unreachable. A comment has to converge with the text it points at, and two peers editing the passage and the discussion at once must merge once, not twice: one document is one merge and one history |
| The mark is the position, and the stored anchor is a cache | storing the offsets as the truth | it needs every peer to rewrite them identically on every edit, and diverges on the first concurrent one. A mark is what the text CRDT moves for free on every insertion and deletion, and a stored offset is a number two concurrent edits race for; making the mark authoritative means the anchor is correct with no reconciliation to get right (`PROSE-FEAT-075`) |
| The cache is kept anyway | reading the mark alone | it leaves the panel with nothing to align to on the first frame, and an orphan with no position at all. The panel is drawn before the block's runs are shaped, and an orphaned thread has no mark left to read, so both cases need a last known position and neither is worth a second pass to recompute (`PROSE-FEAT-076`) |
| A deleted range orphans the thread | deleting the thread with its text | it loses a discussion on an ordinary edit and cannot be undone from the thread's side. The discussion is work several people did, and the deletion was of a sentence; keeping the thread lets the author see what became unreachable (`PROSE-FEAT-009`) |
| Overlap is layered, not merged | one mark carrying a list of identities | it turns every comment and every resolve into a read-modify-write of a value two peers race for. Two readers commenting on overlapping passages is the normal case, and a single-valued mark cannot hold two threads; giving each thread its own span keeps both discussions alive and makes a resolve a write to one span (`PROSE-FEAT-078`) |
| The author is stamped locally | trusting an author field on the command | it lets any member post under any other member's name. The name on a comment is what every reader judges it by, so it must not be a field the sender chooses; stamping it from the session identity leaves nothing to forge (`PROSE-FEAT-077`) |
| A reply reopens a resolved thread | keeping the thread closed | it hides the reply from everyone but the reader who went looking in the resolved list. Someone replying to a closed discussion is reopening it, whatever the control they used; requiring a separate reopen first means a reply that lands in a list nobody reads (`PROSE-FEAT-079`) |
| A reply is its author's, a thread is everyone's | one permission for both | it either lets a member rewrite another's reply or stops a reviewer resolving a thread they did not open. Rewriting someone else's words is the one operation with no honest use; closing a discussion is what a reviewer is there to do (`PROSE-FEAT-080`) |
| The highlight is canvas and the panel is DOM | drawing the panel on the canvas | it reimplements scrolling, focus and accessibility for a list of text boxes. The highlight sits under the text, at a position only the layout knows, so it belongs to the page pass; the panel is a scrollable list of text fields, which the browser lays out, focuses and reads to a screen reader for free (`PROSE-UI-049`, `PROSE-UI-050`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture opens a thread.** `AddComment` is declared, and no surface sends it: the two
  surfaces a selection reaches name no comment
  (`features/ambre-spec-prose-toolbar.md section 4`,
  `features/ambre-spec-prose-slash-command.md section 4`). Section 4 states the gesture the feature
  needs; nothing in the corpus states where it is.
* **Nothing surfaces an orphan.** `PROSE-FEAT-009` requires the thread to be kept for review, and
  no event carries the orphan set and no command lists it. `features/ambre-spec-prose-footnotes.md`
  records the same gap for a note whose call is deleted; this is the comments half of it.
* **A refusal travels, and nothing shows it.** `PROSE-FEAT-080` requires the refusal to be
  reported. The engine names the command and the reason
  (`features/ambre-spec-prose-engine.md section 3`), and no comment command exists to be refused
  yet. Nor does a view that would show one. It lands with
  the "Comments and track changes" Story (`ambre-roadmap.md` E5).
* **The panel's own state is named and never declared.**
  `features/ambre-spec-prose-engine.md section 3` gives the view a `CommentPanelState` field, and no
  document says what it holds. The open thread, the panel's scroll position and the orphaned
  list's visibility are all undeclared.
* **Nothing rules on a comment inside a header, a footer or a table cell.** A footnote is refused
  in the two bands (`PROSE-FEAT-071`) for a reason that does not apply here. No rule allows or
  refuses the three cases.
* **Nothing rules on a comment's page.** A thread whose panel entry names a page number rests on
  two peers breaking pages at the same place, which no rule states
  (`features/ambre-spec-prose-pagination.md section 5`). The `Two peers read the same page number`
  Story settles it (`ambre-roadmap.md` E5).
* **Two applications stamp a comment's author two ways.** `TOILE-FEAT-014` requires the canonical
  `UserId` resolved by `ambre-sync` and refuses a peer identifier. This document stores a display
  name and the document peer, and no `UserId` at all (`ambre-spec-prose-schema.md section 3`). Both
  refuse a forged author, and only one of the two survives a user renaming themselves. Nothing in
  the corpus rules on which is right.
* **Nothing rules on a reply that lands on a thread another peer deleted.** Section 2 pairs the
  two and expects no thread on either screen. `PROSE-FEAT-081` states the local gesture alone, and
  no row says what becomes of the reply that raced it.
* **No row rules on the range a thread opens over.** `AddComment` takes a `block_id` and a
  `range`, and section 1 carries one refusal, on a reply's author. An empty range, and a range the
  block does not hold, are refused nowhere.
* **A comment command's place in the undo stack is ruled on in one application.**
  `TOILE-FEAT-017` keeps a comment out of the undo stack, so `Ctrl+Z` cannot retract a message a
  teammate has read. Prose states nothing, and `features/ambre-spec-prose-undo.md` reverts every
  local operation.
