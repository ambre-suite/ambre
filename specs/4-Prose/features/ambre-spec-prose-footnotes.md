# AMBRE -- FEATURE: FOOTNOTES

**Type:** Vertical (feature)
**Implementation:** design-only

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
| `PROSE-FEAT-013` | `OBS` | The number drawn in front of a note MUST be counted from the order of the calls in the document, at the moment the document is read. | this spec section 3 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-095` | `INV` | The document MUST NOT hold a note's number. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-096` | `INV` | A note's identity MUST be minted once. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-280` | `INV` | A note's identity MUST survive a move of its call, an edit of its body, and a remote change. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-097` | `CONC` | Two peers whose text has converged MUST draw the same number in front of every note. | this spec section 2; this spec section 3 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-098` | `OBS` | A note no call reaches MUST be shown to the author as orphaned. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Add a note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-099` | `POST` | After the command, the document MUST hold the note's body and one call to it at the position the caret was in. | this spec section 3; this spec section 4 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-100` | `FRAME` | Adding a note MUST NOT put the note's text in the block the call sits in, and MUST NOT touch another note. | this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-071` | `FAIL` | A note MUST be refused in a header or a footer, and the refusal MUST be reported. | this spec section 3 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Delete a note

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-070` | `POST` | Deleting a note MUST remove its body and every call still carrying its identity. | this spec section 3 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-101` | `FRAME` | Deleting a note MUST NOT change the text a call sat in, beyond the call itself, and MUST NOT touch another note. | this spec section 3 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Delete the text a note is called from

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-069` | `FRAME` | Deleting the text a call sits on MUST NOT remove the note it reached. | this spec section 3; this spec section 5 | not built | unbuilt -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own. The notes are collected, shaped and placed with the page, the fourth
  step of `features/ambre-spec-prose-pagination.md section 3`, charged to the pagination rows of
  `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a note is text in the document, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 12 `unimplemented`, nothing else: `crates/ambre-prose` is a seven-line
  `lib.rs`. Two rows are reachable by a property test without a page. Numbering gives the first
  call in document order the first number and leaves no gap (`PROSE-FEAT-013`). Agreement means
  two documents converged on the same text count the same numbers (`PROSE-FEAT-097`). The rest
  wait on a page:
  * the identity that outlives a gesture (`PROSE-FEAT-096`);
  * the orphan policy and the set nothing carries (`PROSE-FEAT-098`);
  * what each command may touch in the block its call sits in (`PROSE-FEAT-100`,
    `PROSE-FEAT-101`).

---

## 2. WHAT EACH USER SEES

Two peers over one set of notes. No test resolves yet: the last column names the row each row rests
on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a note before an existing one | -- | one more note, no number anywhere | the new note as one, the old one as two, on both screens | none -- `PROSE-FEAT-013` |
| adds a note at the end | adds a note at the end at the same moment | both notes, in the order the text converged on | the same two notes, numbered one after the other | none -- `PROSE-FEAT-097` |
| deletes the sentence that calls a note | -- | the note, with no call left | the note gone from the page, and listed as orphaned for review | none -- `PROSE-FEAT-069`, `PROSE-FEAT-098` |
| deletes a note | edits that note's text at the same moment | no note | the note gone on both screens, and no call left behind | none -- `PROSE-FEAT-070` |
| moves a paragraph past another note | -- | the two calls, in the new order | the two numbers swapped, on both screens | none -- `PROSE-FEAT-013` |
| tries to add a note in the footer | -- | nothing | a refusal, and the footer unchanged | none -- `PROSE-FEAT-071` |

---

## 3. MODEL

**Containers.** The note's body is one annotation entry of the shared annotation map. The call
is an inline mark on the text carrying that entry's identity. Both are declared in
`ambre-spec-prose-schema.md section 3`. The identity is a value the peer that creates the note
mints (`section 2` of that document) once, for the life of the note (`PROSE-FEAT-096`).

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

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

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

* **Nothing surfaces an orphan.** `PROSE-FEAT-069` requires the note to be kept for review, and no
  event carries the orphan set and no command lists it. A comment orphaned by the same rule has the
  same gap.
* **A refusal travels, and nothing shows it.** `PROSE-FEAT-071` requires the refusal to be
  reported. The engine names the command and the reason
  (`features/ambre-spec-prose-engine.md section 3`), and no footnote command exists to be refused
  yet. Nor does a view that would show one. It lands with the
  `Undo, footnotes and cross references` Story (`ambre-roadmap.md` E5).
* **What the call is a mark on is not settled.** Section 3 makes the call an inline mark and says
  that deleting the character it sits on deletes it. Section 4 says it draws as its number and is
  selected and deleted whole, as one character. Either the mark rides a character the author typed,
  and removing the call takes that character with it. The other shape has the insert write a
  carrier character no rule declares. `PROSE-FEAT-100` and `PROSE-FEAT-101` bound what each command
  may touch around the call; which of the two shapes the schema holds is undecided
  (`ambre-spec-prose-schema.md section 3`).
* **The note's entry carries no author and no stamp**, while a comment's carries both
  (`ambre-spec-prose-schema.md section 3`). Whether a note shows who wrote it is undecided.
* **Nothing rules on an endnote.** Every rule here places the note on its call's page, and no
  design in the corpus places one at the end of the document.
* **Nothing rules on the numbering style.** The number is stated as a count and no rule says
  whether it may be a symbol, restart per page or restart per section.
* **A note taller than the page it belongs to has no rule.** It is the case where placing the
  note on its call's page and fitting it on the page contradict each other.
