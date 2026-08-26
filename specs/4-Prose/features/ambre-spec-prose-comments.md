# AMBRE -- FEATURE: COMMENTS

**Type:** Vertical (feature)
**Implementation:** design-only

> A discussion attached to a passage of the text. The thread follows the passage through every
> edit, and when the passage is deleted the thread is kept and listed rather than destroyed with
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
| When the target dies | the pin dangles, and is not painted (`TOILE-025`) | the thread orphans, and is listed for review (`PROSE-FEAT-009`) |
| Thread order | the list's, never a timestamp (`TOILE-FEAT-012`) | the list's, never a timestamp |
| Author | the canonical `UserId` (`TOILE-FEAT-014`) | a display name and a document peer, stamped locally |

Two of the four match, and the two that do not are section 6.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-001` | A comment's identity MUST be stable for the document's lifetime: an edit, a reply or a remote delta MUST NOT change it. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-009` | A comment thread MUST stay anchored to its text range across concurrent edits, and MUST go orphaned rather than silently re-anchor when that range is deleted. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-075` | The live mark on the text, never the stored anchor, MUST decide where a comment sits. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-076` | The stored anchor MUST be rewritten from the mark on every edit that touches the annotated block. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-077` | The author and the peer of a comment MUST be stamped from the session identity, never read from the command. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-078` | Each comment MUST own its own mark span, and resolving or deleting one comment MUST NOT rewrite another's. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-079` | A reply to a resolved thread MUST reopen it. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-080` | Editing or deleting a reply MUST be refused unless the caller wrote it. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-081` | Deleting a thread's opening reply MUST delete the thread. | this spec section 3 | not built -- lands with the "Comments and track changes" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-049` | A comment highlight MUST be canvas content, painted under the annotated range. | this spec section 4 | not built -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-050` | The comment panel MUST be DOM chrome, not a canvas overlay. | this spec section 4; `ambre-spec-ui.md section 2` | not built -- lands with the "Collaborative cursors and annotations" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a comment is an annotation entry and a mark, both written on the
  ordinary command path, and the highlight is painted in the page pass
  `features/ambre-spec-prose-page-rendering.md` charges.
* **Boundary.** none of its own: a thread is content in the document, and `ZK-001` seals every
  payload before it leaves the client. The author field is stamped locally, which is a rule of this
  document and not a boundary the relay sees.
* **Verification.** 11 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-009` is the row a property test reaches without a screen: any interleaving of edits
  over an annotated range leaves the thread on that range or leaves it orphaned, and never on
  another range. `PROSE-FEAT-078` is the row two peers are needed for, because a single writer can
  never make two spans race.

---

## 2. WHAT EACH USER SEES

Two peers over one thread. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| comments on a sentence | types a word before it | the thread, on the same sentence, one word further down | the highlight on Alice's sentence, on both screens | none -- `PROSE-FEAT-009` |
| comments on a sentence | comments on three words inside it | two threads, two spans | both highlights, stacked over the three shared words | none -- `PROSE-FEAT-078` |
| resolves her thread | -- | both threads, one closed | Bob's highlight untouched, and Alice's out of the open list | none -- `PROSE-FEAT-078` |
| deletes the whole commented sentence | replies to that thread | the thread, with no mark left | the reply, and the thread listed as orphaned on both screens | none -- `PROSE-FEAT-009` |
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
and `anchor_end` fields of the entry are a **cache**, and they exist for two cases alone: rendering
the thread before the block's runs are shaped, and holding the last known position of a thread whose
mark is gone. The mark wins every disagreement (`PROSE-FEAT-075`), and every edit touching an
annotated block rewrites the cache from the mark's current span (`PROSE-FEAT-076`). Nothing renders
from the cache while a live mark exists.

**The orphan.** Deleting the whole marked range deletes the mark with the text. The entry is
**not** removed (`PROSE-FEAT-009`): `resolved` is untouched, the cache freezes at the last known
position, and the view lists the thread as orphaned. Only the delete command removes a thread. The
footnotes feature applies the same policy to a note whose call is deleted.

**Overlap, and why it works.** A mark of one key is single-valued, so two threads over the same
characters cannot both be that one mark's value. They are **layered** instead: each thread owns its
own span, the newer value wins a raw read of the shared characters, and both entries stay live and
both highlights are painted. Resolving or deleting one thread rewrites its own span and no other
(`PROSE-FEAT-078`).

**The identity, and where the author comes from.** The commands carry the text alone. The
dispatcher fills the author name and the peer from the **current session identity**, never from the
payload (`PROSE-FEAT-077`). It is **not** the rule presence applies to a caret: presence resolves the
author from the key that signed the frame and puts nothing in the payload
(`1-Foundations/ambre-spec-presence.md`, `PRESENCE-002`, `PRESENCE-003`), while a comment stores a
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
element to the right of the page, scrolled by the browser, each thread aligned to the vertical
position the view publishes for its anchor (`PROSE-UI-050`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The thread lives in the document's own annotation map | a comment has to converge with the text it points at, and two peers editing the passage and the discussion at once must merge once, not twice. One document is one merge and one history | a second `LoroDoc` for the comments, which doubles the synchronization and makes consistency between the text and the threads unreachable |
| The mark is the position, and the stored anchor is a cache | a mark is what the text CRDT moves for free on every insertion and deletion; a stored offset is a number two concurrent edits race for. Making the mark authoritative means the anchor is correct with no reconciliation to get right (`PROSE-FEAT-075`) | storing the offsets as the truth, which needs every peer to rewrite them identically on every edit, and diverges on the first concurrent one |
| The cache is kept anyway | the panel is drawn before the block's runs are shaped, and an orphaned thread has no mark left to read. Both cases need a last known position, and neither is worth a second pass to recompute (`PROSE-FEAT-076`) | reading the mark alone, which leaves the panel with nothing to align to on the first frame and an orphan with no position at all |
| A deleted range orphans the thread | the discussion is work several people did, and the deletion was of a sentence. Destroying the thread with it loses that work with no gesture that says so; keeping it lets the author see what became unreachable (`PROSE-FEAT-009`) | deleting the thread with its text, which loses a discussion on an ordinary edit and cannot be undone from the thread's side |
| Overlap is layered, not merged | two readers commenting on overlapping passages is the normal case, and a single-valued mark cannot hold two threads. Giving each thread its own span keeps both discussions alive and makes a resolve a write to one span (`PROSE-FEAT-078`) | one mark carrying a list of identities, which turns every comment and every resolve into a read-modify-write of a value two peers race for |
| The author is stamped locally | the name on a comment is what every reader judges it by, so it must not be a field the sender chooses. Stamping it from the session identity leaves nothing to forge (`PROSE-FEAT-077`) | trusting an author field on the command, which lets any member post under any other member's name |
| A reply reopens a resolved thread | someone replying to a closed discussion is reopening it, whatever the control they used. Requiring a separate reopen first means a reply that lands in a list nobody reads (`PROSE-FEAT-079`) | keeping the thread closed, which hides the reply from everyone but the reader who went looking in the resolved list |
| A reply is its author's, a thread is everyone's | rewriting someone else's words is the one operation with no honest use; closing a discussion is what a reviewer is there to do. The two restrictions are different because the two acts are (`PROSE-FEAT-080`) | one permission for both, which either lets a member rewrite another's reply or stops a reviewer resolving a thread they did not open |
| The highlight is canvas and the panel is DOM | the highlight sits under the text, at a position only the layout knows, so it belongs to the page pass. The panel is a scrollable list of text fields, which the browser lays out, focuses and reads to a screen reader for free (`PROSE-UI-049`, `PROSE-UI-050`) | drawing the panel on the canvas, which reimplements scrolling, focus and accessibility for a list of text boxes |

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
* **The refusal has no event.** `PROSE-FEAT-080` requires the refusal to be reported, and the
  engine declares one untyped error alone (`features/ambre-spec-prose-engine.md section 3`). Five
  features report through the same gap.
* **The panel's own state is named and never declared.**
  `features/ambre-spec-prose-engine.md section 3` gives the view a `CommentPanelState` field, and no
  document says what it holds -- which thread is open, where the panel is scrolled, whether the
  orphaned list is shown.
* **Nothing rules on a comment inside a header, a footer or a table cell.** A footnote is refused
  in the two bands (`PROSE-FEAT-071`) for a reason that does not apply here, and no rule allows or
  refuses the three cases.
* **Nothing rules on a comment's page.** A thread whose panel entry names a page number rests on
  two peers breaking pages at the same place, which no rule states
  (`features/ambre-spec-prose-pagination.md section 5`).
* **Two applications stamp a comment's author two ways.** `TOILE-FEAT-014` requires the canonical
  `UserId` resolved by `ambre-sync` and refuses a peer identifier; this document stores a display
  name and the document peer, and no `UserId` at all (`ambre-spec-prose-schema.md section 3`). Both
  refuse a forged author, and only one of the two survives a user renaming themselves. Nothing in
  the corpus rules on which is right.
* **A comment command's place in the undo stack is ruled on in one application.**
  `TOILE-FEAT-017` keeps a comment out of the undo stack, so `Ctrl+Z` cannot retract a message a
  teammate has read. Prose states nothing, and `features/ambre-spec-prose-undo.md` reverts every
  local operation.
