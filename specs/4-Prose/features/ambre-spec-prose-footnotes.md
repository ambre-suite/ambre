# AMBRE -- FEATURE: FOOTNOTES

**Type:** Vertical (feature)
**Implementation:** partial -- the notes are placed on their page and not yet drawn

> A note at the foot of the page, called from a mark in the text. The note keeps one identity for
> the life of the document. The number the reader sees is counted from the order of the calls
> when the document is read. An inserted note renumbers the rest for free.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-013` | `OBS` | The number drawn in front of a note MUST be counted from the order of the calls in the document, at the moment the document is read. | this spec section 3 | `ambre_prose::logic::notes::tests::a_note_is_numbered_from_the_order_of_its_calls_and_stores_no_number` | none | verified |
| `PROSE-FEAT-095` | `INV` | The document MUST NOT hold a note's number. | this spec section 3; this spec section 5 | `ambre_prose::logic::notes::tests::a_note_is_numbered_from_the_order_of_its_calls_and_stores_no_number` | none | verified |
| `PROSE-FEAT-096` | `INV` | A note's identity MUST be minted once. | this spec section 3; this spec section 5 | `ambre_prose::logic::notes::tests::a_note_keeps_its_identity_through_a_move_an_edit_and_a_remote_change`; `::a_note_lands_as_one_call_and_an_entry_holding_its_text` | none | verified |
| `PROSE-FEAT-280` | `INV` | A note's identity MUST survive a move of its call, an edit of its body, and a remote change. | this spec section 3; this spec section 5 | `ambre_prose::logic::notes::tests::a_note_keeps_its_identity_through_a_move_an_edit_and_a_remote_change` | none | verified |
| `PROSE-FEAT-097` | `CONC` | Two peers whose text has converged MUST draw the same number in front of every note. | this spec section 2; this spec section 3 | `ambre_prose::logic::notes::tests::two_converged_peers_count_the_same_numbers` | none | verified |
| `PROSE-FEAT-098` | `OBS` | A note no call reaches MUST be shown to the author as orphaned. | this spec section 3; this spec section 5 | `ambre_prose::logic::notes::tests::deleting_the_text_of_a_call_keeps_the_note_and_lists_it_as_orphaned` | none | verified |
| `PROSE-FEAT-305` | `INV` | A note's text MUST live in a block of its own, which the note's entry names and the body flow never holds, and deleting the note MUST remove that block. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::logic::notes::tests::a_note_holds_its_text_in_a_block_out_of_the_flow` | none | verified |
| `PROSE-FEAT-308` | `INV` | A document written under schema version 8 MUST keep each note's text and its marks once read. | this spec section 3; `ambre-spec-prose-schema.md section 7` | `ambre_prose::logic::notes::tests::a_version_8_note_moves_to_a_block_with_its_marks` | none | verified |

### 1.2 Add a note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-099` | `POST` | After the command, the document MUST hold the note's body and one call to it at the position the caret was in. | this spec section 3; this spec section 4 | `ambre_prose::logic::notes::tests::a_note_lands_as_one_call_and_an_entry_holding_its_text` | none | verified |
| `PROSE-FEAT-100` | `FRAME` | Adding a note MUST NOT put the note's text in the block the call sits in, and MUST NOT touch another note. | this spec section 5 | `ambre_prose::logic::notes::tests::a_note_lands_as_one_call_and_an_entry_holding_its_text`; `::a_note_touches_no_other_note` | none | verified |
| `PROSE-FEAT-071` | `FAIL` | A note MUST be refused in a header or a footer, and the refusal MUST be reported. | this spec section 3 | `ambre_prose::logic::notes::tests::a_note_is_refused_in_a_band_and_the_refusal_is_reported` | none | verified |
| `PROSE-FEAT-306` | `FAIL` | A note MUST be refused in a note's own text, and the refusal MUST be reported. | this spec section 3 | `ambre_prose::logic::notes::tests::a_note_refuses_a_note_a_comment_and_a_split` | none | verified |
| `PROSE-FEAT-307` | `FAIL` | A comment MUST be refused in a note's text, and the refusal MUST be reported. | this spec section 3 | `ambre_prose::logic::notes::tests::a_note_refuses_a_note_a_comment_and_a_split` | none | verified |

### 1.3 Delete a note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-070` | `POST` | Deleting a note MUST remove its body and every call still carrying its identity. | this spec section 3 | `ambre_prose::logic::notes::tests::deleting_a_note_removes_its_body_and_its_calls_and_nothing_else`; `::a_note_deleted_while_a_peer_edits_it_leaves_no_call` | none | verified |
| `PROSE-FEAT-101` | `FRAME` | Deleting a note MUST NOT change the text a call sat in, beyond the call itself, and MUST NOT touch another note. | this spec section 3 | `ambre_prose::logic::notes::tests::deleting_a_note_removes_its_body_and_its_calls_and_nothing_else` | none | verified |

### 1.4 Delete the text a note is called from

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-069` | `FRAME` | Deleting the text a call sits on MUST NOT remove the note it reached. | this spec section 3; this spec section 5 | `ambre_prose::logic::notes::tests::deleting_the_text_of_a_call_keeps_the_note_and_lists_it_as_orphaned` | none | verified |

* **Budget.** none of its own. The notes are collected, shaped and placed with the page, the fourth
  step of `features/ambre-spec-prose-pagination.md section 3`, charged to the pagination rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a note is text in the document, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 16 `verified`, each by a unit test on the engine in
  `crates/ambre-prose/src/logic/notes.rs`. The two concurrent rows run two peers over one document
  and exchange their updates (`PROSE-FEAT-097`, `PROSE-FEAT-070`). Where a note is drawn is the
  pagination's to verify (`PROSE-015`, `PROSE-035`).

---

## 2. WHAT EACH USER SEES

Two peers over one set of notes. The last column names the test that plays the row.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a note before an existing one | -- | one more note, no number anywhere | the new note as one, the old one as two, on both screens | `a_note_is_numbered_from_the_order_of_its_calls_and_stores_no_number` |
| adds a note at the end | adds a note at the end at the same moment | both notes, in the order the text converged on | the same two notes, numbered one after the other | `two_converged_peers_count_the_same_numbers` |
| deletes the sentence that calls a note | -- | the note, with no call left | the note gone from the page, and listed as orphaned for review | `deleting_the_text_of_a_call_keeps_the_note_and_lists_it_as_orphaned` |
| deletes a note | edits that note's text at the same moment | no note | the note gone on both screens, and no call left behind | `a_note_deleted_while_a_peer_edits_it_leaves_no_call` |
| moves a paragraph past another note | -- | the two calls, in the new order | the two numbers swapped, on both screens | `a_note_is_numbered_from_the_order_of_its_calls_and_stores_no_number` |
| tries to add a note in the footer | -- | nothing | a refusal, and the footer unchanged | `a_note_is_refused_in_a_band_and_the_refusal_is_reported` |

---

## 3. MODEL

**Containers.** The note is one annotation entry of the shared annotation map, and its text is a
block of its own. That block is a root paragraph marked as a note: the entry names it, and the body
flow never holds it (`PROSE-FEAT-305`). The call is an inline mark on the text carrying that entry's
identity. Both are declared in `ambre-spec-prose-schema.md section 3`. The identity is a value the
peer that creates the note mints (`section 2` of that document) once, for the life of the note
(`PROSE-FEAT-096`).

**The number is not stored.** It is counted by walking the block tree in document order and, inside
each block, the positions of the calls (`PROSE-FEAT-013`). It is nowhere written down
(`PROSE-FEAT-095`). An inserted note therefore renumbers everything after it with no write, and two
peers converge on the text and then agree on the numbers (`PROSE-FEAT-097`).

**The three rules the pair obeys:**

* **The orphan.** Deleting the character the call sits on deletes the mark, and the note is then
  reachable from nothing. It is **not** removed (`PROSE-FEAT-069`): the view lists orphaned notes
  for review (`PROSE-FEAT-098`), and only the delete command removes one. The comments feature
  applies the same policy.
* **The delete.** Deleting a note removes its entry and strips every call still carrying its
  identity (`PROSE-FEAT-070`). It leaves the text those calls sat in otherwise as it was
  (`PROSE-FEAT-101`). No renumbering follows, because nothing stored a number.
* **Where a note may be called.** Body text and table-cell text. Not a header and not a footer
  (`PROSE-FEAT-071`): a band repeated on every page cannot own a note belonging to one page's
  foot. The refusal is reported.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertFootnote { block_id, pos, note_text }` | a note, and the call that reaches it (`PROSE-FEAT-099`, `PROSE-FEAT-100`) |
| `EditFootnote { footnote_id, new_text }` | rewrites the note's body |
| `DeleteFootnote { footnote_id }` | removes the note and every call to it |

**Where a note is placed.** A note is placed at the foot of the page its call falls on, and follows
that call through any later reflow. The separator is drawn once per page. An edit that adds or
removes a call forces a full recomputation. All three are
`features/ambre-spec-prose-pagination.md`, `PROSE-015` and `PROSE-035`.

**The call is a carrier character.** The insert writes one U+FFFC and marks it with the note's
identity, so the call is selected and deleted whole, as section 4 draws it. The author's own text
never carries a call (`PROSE-FEAT-100`).

**Code:** `crates/ambre-prose/src/note.rs` reads the notes and counts the numbers.
`crates/ambre-prose/src/logic/notes.rs` holds the three commands. The list reaches the view as
`FootnotesUpdated`, after each command and at each pause in typing.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the footnote entry of the insert menu | a note at the caret, and the caret in the note |
| a click on a call | moves the view to the note |
| a click on a note's number | moves the view back to the call |
| typing in the note | edits the note's body, as in any block |
| the delete entry of the note's menu | removes the note and its call |
| a hover over a call | shows the note's text without leaving the line |

A call is drawn as its computed number and behaves as one character to the caret: it is selected
whole and deleted whole.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The number is counted when the document is read | storing the ordinal on the note | it turns an insertion into a rewrite and a merge of numbers: a stored number is wrong for every following note the moment one is inserted -- a write per note, per insertion, per peer. Counting costs one walk and converges by construction (`PROSE-FEAT-013`) |
| The identity is minted once and never changes | keying the note by its number | it breaks the join on the first insertion above it. The identity joins the call to the note and has to survive a move, an edit and a remote change, and a number is exactly what changes |
| The call is a mark, and the body is an entry | storing the note's text inline in the body | it puts a paragraph inside a sentence and paginates it as body text. A call belongs where only a mark can sit, and a note's body has to merge like the paragraph it is, so each half needs the container that merges it correctly |
| A deleted call orphans the note | purging the note with the call | it loses a paragraph on an ordinary text deletion, and the note's side cannot undo it. The note is content the author wrote, so keeping it lets them see what became unreachable and decide (`PROSE-FEAT-069`) |
| Deleting a note strips its calls | removing the entry alone | it leaves calls resolving to nothing scattered through the text. Half of a pair is not a state the reader can act on, so one command removes both halves (`PROSE-FEAT-070`) |
| No note in a header or a footer | allowing it and numbering per page | it contradicts the document-order numbering every other note uses. The band is drawn on every page, so one call produces a note on every page with a different number, and there is no correct number to give it (`PROSE-FEAT-071`) |
| A note in a table cell is an ordinary note | refusing notes in tables | it is a restriction the reader meets with no reason they can see. The cell is body text on a page like any other, and the call's position in document order is well defined |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The note's entry carries no author and no stamp**, while a comment's carries both
  (`ambre-spec-prose-schema.md section 3`). Whether a note shows who wrote it is undecided.
* **Nothing rules on an endnote.** Every rule here places the note on its call's page, and no
  design in the corpus places one at the end of the document.
* **Nothing rules on the numbering style.** The number is stated as a count and no rule says
  whether it may be a symbol, restart per page or restart per section.
* **A note taller than the page it belongs to has no rule.** It is the case where placing the
  note on its call's page and fitting it on the page contradict each other.
