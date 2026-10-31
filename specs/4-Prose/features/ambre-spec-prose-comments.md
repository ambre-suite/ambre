# AMBRE -- FEATURE: COMMENTS ON A DOCUMENT

**Type:** Vertical (feature)
**Implementation:** partial -- the threads converge, and nothing paints them yet

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
| Anchor | an `Id<Element>`, or a free world point | a live mark on a text range |
| When the target dies | the pin dangles, and is not painted (`TOILE-025`) | the thread orphans, and is kept for review (`PROSE-FEAT-202`) |
| Thread order | the list's, never a timestamp (`TOILE-FEAT-012`) | the list's, never a timestamp (`PROSE-FEAT-186`) |
| Author | the canonical `Id<User>` (`TOILE-FEAT-014`) | the canonical `Id<User>` and a display name, stamped locally (`PROSE-FEAT-077`) |

Two of the four match. The two that do not are the anchor itself, which is why the feature stays vertical.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-001` | `INV` | A comment's identity MUST be stable for the document's lifetime: an edit, a reply or a remote delta MUST NOT change it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_thread_keeps_its_identity_through_edits_and_peers` | none | verified |
| `PROSE-FEAT-075` | `INV` | The live mark on the text, never the stored anchor, MUST decide where a comment sits. | this spec section 3 | `ambre_prose::logic::comments::tests::the_cache_follows_the_mark_and_never_overrules_it` | none | verified |
| `PROSE-FEAT-185` | `INV` | The stored anchor MUST NOT be read for a position while the comment's mark is alive. | this spec section 3 | `ambre_prose::logic::comments::tests::the_cache_follows_the_mark_and_never_overrules_it` | none | verified |
| `PROSE-FEAT-078` | `INV` | Each comment MUST own its own mark span. | this spec section 3 | `ambre_prose::logic::comments::tests::two_threads_over_shared_characters_keep_their_own_spans` | none | verified |
| `PROSE-FEAT-186` | `INV` | The order of a thread's replies MUST be the list's own, never a timestamp's. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_lands_last_whatever_its_stamp` | none | verified |
| `PROSE-FEAT-077` | `INV` | The author of a reply, its `Id<User>` and its name, MUST be stamped from the session identity, never read from the command. | this spec section 3 | structural -- no comment command carries an author, and `write_reply` reads the session's `Id<User>` and name; `ambre_prose::logic::comments::tests::the_author_is_stamped_from_the_session` | none | verified |
| `PROSE-FEAT-187` | `INV` | Resolving, reopening or deleting a thread MUST NOT be refused on the caller's identity: only a reply is its author's alone. | this spec section 3 | `ambre_prose::logic::comments::tests::another_member_may_close_reopen_and_delete_a_thread` | none | verified |
| `PROSE-FEAT-296` | `INV` | An undo MUST NOT retract a comment command: a thread, a reply, a resolve or a deletion. | this spec section 3; `TOILE-FEAT-017` | `ambre_prose::logic::comments::tests::an_undo_never_retracts_a_comment` | none | verified |
| `PROSE-FEAT-009` | `CONC` | A comment thread MUST stay anchored to its text range across concurrent edits. | this spec section 3 | `ambre_prose::logic::comments::tests::a_thread_stays_on_its_text_under_concurrent_edits` | none | verified |
| `PROSE-FEAT-297` | `CONC` | A reply that races the deletion of its thread MUST be lost on every peer alike. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_racing_the_deletion_of_its_thread_is_lost` | none | verified |
| `PROSE-UI-049` | `OBS` | A comment highlight MUST be canvas content, painted over the characters of its range and no other. | this spec section 4 | `ambre_prose::view::paint::tests::a_thread_the_logic_thread_sends_is_painted_over_its_range_and_no_other` -- a thread received from the Logic thread is tinted over its five characters alone; `::selections_go_under_the_glyphs_and_highlights_over_them` | none | verified |
| `PROSE-UI-144` | `OBS` | Two threads whose spans share characters MUST both be painted, one highlight over the other. | this spec section 3 | `ambre_prose::view::paint::tests::two_overlapping_threads_are_both_painted` | none | verified |
| `PROSE-UI-050` | `INV` | The comment panel MUST be DOM chrome, not a canvas overlay. | this spec section 4; `ambre-spec-ui.md section 2` | structural -- `crates/ambre-prose/src/chrome/mod.rs` renders the panel as Leptos elements, and the Prose painter holds no panel to draw; `ambre_prose::view::annotate::tests::each_open_thread_is_lined_up_with_its_anchor` -- the heights it reads; `::the_panel_lists_open_threads_top_to_bottom`; `::two_anchors_on_one_line_are_stacked_and_the_first_stays`; `::each_intent_becomes_its_command_and_a_focus_scrolls` | none | verified |

### 1.2 Open a thread on a range

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-188` | `POST` | Opening a thread MUST write one annotation entry and one mark reaching the requested range and no character outside it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_thread_marks_its_range_and_no_character_outside_it` | none | verified |
| `PROSE-FEAT-189` | `FRAME` | Opening a thread MUST NOT change the text of the block, or the entry or the span of any other thread. | this spec section 3 | `ambre_prose::logic::comments::tests::opening_a_thread_touches_neither_the_text_nor_another_thread` | none | verified |
| `PROSE-FEAT-295` | `FAIL` | Opening a thread MUST be refused in a header or a footer, on an empty range, on a range the block does not hold, and in a session with no user. | this spec section 3 | `ambre_prose::logic::comments::tests::a_thread_is_refused_in_a_band_on_an_empty_range_or_with_no_user` | none | verified |

### 1.3 Reply to a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-190` | `POST` | A reply MUST be appended at the end of the thread's reply list. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_lands_last_whatever_its_stamp` | none | verified |
| `PROSE-FEAT-079` | `POST` | A reply to a resolved thread MUST reopen it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_reopens_a_resolved_thread` | none | verified |
| `PROSE-FEAT-191` | `FRAME` | Replying MUST NOT move the thread's mark, rewrite an earlier reply, or touch another thread. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_touches_nothing_but_its_own_list` | none | verified |
| `PROSE-UI-231` | `FRAME` | A reply being typed MUST NOT be emptied when its thread moves in the panel. | this spec section 2; section 4 | `ambre_prose::view::annotate::tests::a_draft_outlives_a_move_and_its_thread_leaving_is_told`; `ambre_prose::tests::chrome_panel::a_reply_being_typed_survives_its_thread_moving` | none | verified |
| `PROSE-UI-232` | `FAIL` | Given a thread a peer deletes or resolves while its reply field holds text, the user MUST be told the reply was not posted. | this spec section 2; section 4 | `ambre_prose::view::annotate::tests::a_draft_outlives_a_move_and_its_thread_leaving_is_told`; `ambre_prose::tests::chrome_panel::a_thread_deleted_under_a_draft_is_told`; `ambre_prose::tests::chrome_panel::a_reply_refused_for_its_thread_reaches_the_user` | none | verified |

### 1.4 Resolve or reopen a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-192` | `POST` | Resolving a thread MUST mark it closed. | this spec section 3 | `ambre_prose::logic::comments::tests::resolving_and_reopening_flip_the_flag_alone` | none | verified |
| `PROSE-FEAT-193` | `POST` | Reopening a thread MUST mark it open, with no reply added. | this spec section 3 | `ambre_prose::logic::comments::tests::resolving_and_reopening_flip_the_flag_alone` | none | verified |
| `PROSE-FEAT-194` | `FRAME` | Resolving or reopening a thread MUST NOT rewrite another thread's span, and MUST NOT add, remove or edit a reply of its own. | this spec section 3 | `ambre_prose::logic::comments::tests::resolving_and_reopening_flip_the_flag_alone` | none | verified |

### 1.5 Edit a reply

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-195` | `POST` | Editing a reply MUST replace that reply's text and leave its place in the list. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_edited_in_place_by_its_author_alone` | none | verified |
| `PROSE-FEAT-080` | `FAIL` | Editing a reply MUST be refused unless the caller wrote it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_edited_in_place_by_its_author_alone` | none | verified |
| `PROSE-FEAT-196` | `FRAME` | Editing a reply MUST NOT change the thread's `resolved` flag, its mark, or the author stamped on the reply. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_edited_in_place_by_its_author_alone` | none | verified |

### 1.6 Delete a reply

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-197` | `POST` | Deleting a reply MUST remove it from the list and leave the order of the replies around it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_deleted_by_its_author_alone` | none | verified |
| `PROSE-FEAT-081` | `POST` | Deleting a thread's opening reply MUST delete the thread. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_the_head_deletes_the_thread` | none | verified |
| `PROSE-FEAT-198` | `FAIL` | Deleting a reply MUST be refused unless the caller wrote it. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_deleted_by_its_author_alone` | none | verified |
| `PROSE-FEAT-199` | `FRAME` | Deleting a reply that is not the head MUST NOT change the thread's mark, its `resolved` flag, or another thread. | this spec section 3 | `ambre_prose::logic::comments::tests::a_reply_is_deleted_by_its_author_alone` | none | verified |

### 1.7 Delete a thread

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-200` | `POST` | Deleting a thread MUST remove its entry and its mark. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_a_thread_leaves_the_text_and_the_thread_beside_it` | none | verified |
| `PROSE-FEAT-279` | `POST` | Deleting a thread MUST leave the text its mark covered. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_a_thread_leaves_the_text_and_the_thread_beside_it` | none | verified |
| `PROSE-FEAT-201` | `FRAME` | Deleting a thread MUST NOT rewrite the span of any other thread over the same characters. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_a_thread_leaves_the_text_and_the_thread_beside_it` | none | verified |

### 1.8 Edit the text a thread is anchored to

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-076` | `POST` | The stored anchor MUST be rewritten from the mark on every edit that touches the annotated block. | this spec section 3 | `ambre_prose::logic::comments::tests::the_cache_follows_the_mark_and_never_overrules_it` | none | verified |
| `PROSE-FEAT-202` | `POST` | An edit that deletes the whole marked range MUST keep the entry, with the thread orphaned rather than re-anchored. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_the_range_orphans_the_thread_where_it_last_was` | none | verified |
| `PROSE-FEAT-203` | `POST` | An edit that takes the mark away MUST leave the stored anchor on the last position the mark held. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_the_range_orphans_the_thread_where_it_last_was` | none | verified |
| `PROSE-FEAT-204` | `FRAME` | An edit over an annotated range MUST NOT change the thread's `resolved` flag, its replies, or the entry of another thread. | this spec section 3 | `ambre_prose::logic::comments::tests::deleting_the_range_orphans_the_thread_where_it_last_was` | none | verified |

* **Budget.** none of its own. A comment is an annotation entry and a mark, both written on the
  ordinary command path. An edit refreshes the anchor cache from the annotated blocks alone, never
  the whole text. The highlight is painted in the page pass
  `features/ambre-spec-prose-page-rendering.md` charges.
* **Boundary.** none of its own: a thread is content in the document, and `ZK-001` seals every
  payload before it leaves the client. The author field is stamped locally, a rule of this document
  rather than a boundary the relay sees.
* **Verification.** 38 `verified`. The painted rows read a recording surface, and the panel's
  heights are read off the view on the host. `PROSE-FEAT-009`, `PROSE-FEAT-078` and
  `PROSE-FEAT-297` need two peers. Each frame condition reads the fields the operation may not touch,
  before and after. The reply field is mounted in headless Chrome (`PROSE-UI-231`).

---

## 2. WHAT EACH USER SEES

Two peers over one thread. The last column names the test, or the row a painted outcome waits on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| comments on a sentence | types a word before it | the thread, on the same sentence, one word further down | the highlight on Alice's sentence, on both screens | `a_thread_stays_on_its_text_under_concurrent_edits` |
| comments on a sentence | comments on three words inside it | two threads, two spans | both highlights, stacked over the three shared words | `two_threads_over_shared_characters_keep_their_own_spans`; `two_overlapping_threads_are_both_painted` |
| resolves her thread | -- | both threads, one closed | Bob's highlight untouched, and Alice's out of the open list | `resolving_and_reopening_flip_the_flag_alone` |
| deletes the whole commented sentence | replies to that thread | the thread, with no mark left | the reply, and the thread listed as orphaned on both screens | `deleting_the_range_orphans_the_thread_where_it_last_was` |
| replies to a thread Bob resolved | -- | the thread, open again | the thread back in the open list, on both screens | `a_reply_reopens_a_resolved_thread` |
| edits Bob's reply | -- | Bob's reply, unchanged | a refusal on Alice's screen, and nothing on Bob's | `a_reply_is_edited_in_place_by_its_author_alone` |
| deletes the opening reply of her own thread | replies to it at the same moment | no thread | the thread gone on both screens, and no highlight left | `a_reply_racing_the_deletion_of_its_thread_is_lost` |
| types half a reply | types a line above the thread's anchor | the line, and no reply | Alice's field moved down, its text and its focus kept | `a_reply_being_typed_survives_its_thread_moving` |
| types half a reply | deletes the thread | no thread | the card gone from Alice's panel, and a notice that her reply was not posted | `a_draft_outlives_a_move_and_its_thread_leaving_is_told`; `a_thread_deleted_under_a_draft_is_told` |

---

## 3. MODEL

**Containers.** The thread is one annotation entry, and the replies are the list inside it. The
anchor is an inline mark of the thread's own key, `cm:{id}`, carrying the entry's identity. Both are
declared in `ambre-spec-prose-schema.md section 3`, and the identity is a value the peer that opens
the thread mints (`section 2` of that document).

**Two positional records, and only one of them is authoritative.** The mark on the text moves with
every insertion and deletion, because that is what a mark does. The `anchor_block`, `anchor_start`
and `anchor_end` fields of the entry are a **cache**, and they exist for two cases alone. Those are
rendering the thread before the block's runs are shaped, and holding the last known position of a
thread whose mark is gone. The mark wins every disagreement (`PROSE-FEAT-075`), and every edit touching an
annotated block rewrites the cache from the mark's current span (`PROSE-FEAT-076`). Nothing renders
from the cache while a live mark exists.

The rewrite is a commit of its own, out of the undo stack like every comment command. An undo takes
the edit back, and the next commit reads the mark again.

**The orphan.** Deleting the whole marked range deletes the mark with the text. The entry is
**not** removed (`PROSE-FEAT-009`): `resolved` is untouched, the cache freezes at the last known
position, and the view lists the thread as orphaned. Only the delete command removes a thread. The
footnotes feature applies the same policy to a note whose call is deleted.

**Overlap, and why it works.** A mark of one key is single-valued. Two threads under one key
would split the shared characters, and the older thread would lose its middle. Each
thread therefore marks under **its own key**, `cm:{id}`. Two keys never compete for a value, both
spans stay whole, and both highlights are painted. Deleting one thread takes off its own key and no
other (`PROSE-FEAT-078`).

**The identity, and where the author comes from.** The commands carry the text and a display
stamp alone. The engine fills the reply's canonical `Id<User>` and display name from the **current
session identity**, never from the payload (`PROSE-FEAT-077`). A session with no user cannot write
a comment. The author of a thread is the author of its opening reply; the entry names none.

Presence differs (`1-Foundations/ambre-spec-presence.md`, `PRESENCE-002`): it reads the author
from the key that signed the frame, and stores none. A comment writes its author down.

**The three rules of a thread's life:**

* **The reply to a closed thread.** `ReplyToComment` on a resolved thread is allowed, and reopens
  it (`PROSE-FEAT-079`). `ReopenComment` is the path for reopening without adding a reply.
* **Whose reply may be touched.** Editing and deleting a reply are the author's alone
  (`PROSE-FEAT-080`), matched on the reply's `Id<User>`. Resolving, reopening and deleting a
  **thread** are not restricted: any collaborator may close a discussion. The check guards the
  honest interface and protects nothing: a peer running other code writes to the CRDT directly.
* **The head of the thread.** The opening comment is the first reply. Deleting it leaves a thread
  with no head, so it deletes the thread (`PROSE-FEAT-081`). A reply written on another peer at the
  same moment lands in a list the deletion removed, and is lost on every peer (`PROSE-FEAT-297`).

**Where a thread may open.** Any text block of the flow, a table cell included. A header and a
footer are refused, since a band is drawn on every page and its thread would sit on all of them. An
empty range, and a range past the text, are refused too (`PROSE-FEAT-295`).

**Out of the undo stack.** Every comment command commits under an origin the undo manager
excludes, as Toile does (`TOILE-FEAT-017`). `Ctrl+Z` takes back the typing, and never a message a
collaborator may have read (`PROSE-FEAT-296`). An undo that removes the marked text orphans the
thread, as any deletion does.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `AddComment { block_id, range, text, now }` | a thread on that range, and the mark that reaches it |
| `ReplyToComment { comment_id, text, now }` | a reply, and a reopen if the thread was closed |
| `ResolveComment { comment_id }` | closes the thread |
| `ReopenComment { comment_id }` | reopens it, with no reply added |
| `DeleteComment { comment_id }` | removes the thread and its mark |
| `DeleteReply { comment_id, reply_id }` | removes one reply, or the thread when it is the head |
| `EditCommentReply { comment_id, reply_id, new_text }` | rewrites the caller's own reply |

**What the view is given.** `CommentsUpdated` carries one entry per thread. It names the
identity, the anchor block and span, the orphan and closed flags, and the replies in order. The
marked threads come in the order of their anchors, and the orphans after them. A comment command
answers it, and so does the pause that recounts the words. The vertical position is the view's,
computed from the layout, which is why the panel can align to the text without holding a document.

**Code:** `crates/ambre-prose/src/comment.rs` reads the threads off the document, and
`crates/ambre-prose/src/logic/comments.rs` holds the seven commands and the anchor cache.

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
`ambre-spec-ui.md section 2` draws. The **highlight** is canvas content: a tinted rectangle over
the annotated range, painted in the page pass (`PROSE-UI-049`). The **panel** is a DOM Leptos
element to the right of the page, scrolled by the browser. Each thread is aligned to the vertical
position the view publishes for its anchor (`PROSE-UI-050`). Two anchors on one line stack, the
first in place. A click on a thread brings its anchor into view.

A card is keyed by its thread, and its draft lives in the panel's signals (`PROSE-UI-231`). A
draft whose thread leaves the panel is dropped, and the shell says the reply was not posted
(`PROSE-UI-232`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The thread lives in the document's own annotation map | a second `LoroDoc` for the comments | it doubles the synchronization and makes consistency between the text and the threads unreachable. A comment has to converge with the text it points at, and two peers editing the passage and the discussion at once must merge once, not twice: one document is one merge and one history |
| The mark is the position, and the stored anchor is a cache | storing the offsets as the truth | it needs every peer to rewrite them identically on every edit, and diverges on the first concurrent one. A mark is what the text CRDT moves for free on every insertion and deletion, and a stored offset is a number two concurrent edits race for; making the mark authoritative means the anchor is correct with no reconciliation to get right (`PROSE-FEAT-075`) |
| The cache is kept anyway | reading the mark alone | it leaves the panel with nothing to align to on the first frame, and an orphan with no position at all. The panel is drawn before the block's runs are shaped, and an orphaned thread has no mark left to read, so both cases need a last known position and neither is worth a second pass to recompute (`PROSE-FEAT-076`) |
| A deleted range orphans the thread | deleting the thread with its text | it loses a discussion on an ordinary edit and cannot be undone from the thread's side. The discussion is work several people did, and the deletion was of a sentence; keeping the thread lets the author see what became unreachable (`PROSE-FEAT-009`) |
| Overlap is layered, not merged | one mark carrying a list of identities | it turns every comment and every resolve into a read-modify-write of a value two peers race for. Two readers commenting on overlapping passages is the normal case, and a single-valued mark cannot hold two threads; giving each thread its own span keeps both discussions alive and makes a resolve a write to one span (`PROSE-FEAT-078`) |
| One mark key per thread | one `cm` key for every thread | the newer thread's value overwrites the older one on the characters they share, and the older span loses its middle. A key per thread costs one registry root, `cm`, and makes two threads two marks that never compete (`PROSE-FEAT-078`) |
| A comment is out of the undo stack | reverting it like any local edit | it lets `Ctrl+Z` retract a message a collaborator has already read, and answer it with nothing. Toile rules the same way (`TOILE-FEAT-017`); a comment is deleted with its own command (`PROSE-FEAT-296`) |
| The author is stamped locally | trusting an author field on the command | it lets any member post under any other member's name. The name on a comment is what every reader judges it by, so it must not be a field the sender chooses; stamping it from the session identity leaves nothing to forge (`PROSE-FEAT-077`) |
| A reply reopens a resolved thread | keeping the thread closed | it hides the reply from everyone but the reader who went looking in the resolved list. Someone replying to a closed discussion is reopening it, whatever the control they used; requiring a separate reopen first means a reply that lands in a list nobody reads (`PROSE-FEAT-079`) |
| A reply is its author's, a thread is everyone's | one permission for both | it either lets a member rewrite another's reply or stops a reviewer resolving a thread they did not open. Rewriting someone else's words is the one operation with no honest use; closing a discussion is what a reviewer is there to do (`PROSE-FEAT-080`) |
| A draft lives in the panel's signals, keyed by thread | a signal owned by each card | the panel maps the list to fresh cards on each change, so every scroll emptied every field. A draft keyed by thread outlives its card, and the view can tell when its thread leaves (`PROSE-UI-231`, `PROSE-UI-232`) |
| The highlight is canvas and the panel is DOM | drawing the panel on the canvas | it reimplements scrolling, focus and accessibility for a list of text boxes. The highlight sits on the text, at a position only the layout knows, so it belongs to the page pass; the panel is a scrollable list of text fields, which the browser lays out, focuses and reads to a screen reader for free (`PROSE-UI-049`, `PROSE-UI-050`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture opens a thread.** `AddComment` is declared, and no surface sends it: the two
  surfaces a selection reaches name no comment
  (`features/ambre-spec-prose-toolbar.md section 4`,
  `features/ambre-spec-prose-slash-command.md section 4`). Section 4 states the gesture the feature
  needs; nothing in the corpus states where it is.
* **A refusal travels, and nothing shows it.** `PROSE-FEAT-080` requires the refusal to be
  reported. The engine names the command and the reason
  (`features/ambre-spec-prose-engine.md section 3`), and no view shows one yet.
* **The panel's own state is named and never declared.**
  `features/ambre-spec-prose-engine.md section 3` gives the view a `CommentPanelState` field, and no
  document says what it holds. The open thread, the panel's scroll position and the orphaned
  list's visibility are all undeclared.
* **Nothing rules on a comment's page.** A thread whose panel entry names a page number rests on
  two peers breaking pages at the same place, which no rule states
  (`features/ambre-spec-prose-pagination.md section 5`). The `Two peers read the same page number`
  Story settles it (`ambre-roadmap.md` E5).
