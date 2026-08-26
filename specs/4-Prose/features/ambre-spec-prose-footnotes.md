# AMBRE -- FEATURE: FOOTNOTES

**Type:** Vertical (feature)
**Implementation:** design-only

> A note at the foot of the page, called from a mark in the text. The note keeps one identity for
> the life of the document, and the number the reader sees is counted from the order of the calls
> when the document is read, so an inserted note renumbers the rest for free.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-FEAT-013` | A footnote MUST keep one identity, and its number MUST be counted from document order when the document is read, never from a stored number. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-069` | Deleting the text that calls a footnote MUST leave the note orphaned; the note MUST NOT be removed automatically. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-070` | Deleting a footnote MUST remove the note and every call still carrying its identity. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-071` | A footnote MUST be refused in a header or a footer. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: the notes are collected, shaped and placed with the page, which is
  the fourth step of `features/ambre-spec-prose-pagination.md section 3` and is charged to the
  pagination rows of `ambre-spec-budgets.md section 1`.
* **Boundary.** none of its own: a note is text in the document, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-FEAT-013` is the row a property test reaches without a page: inserting a note anywhere and
  reading the numbers must give one to the first call in document order and no gap after it.

---

## 2. WHAT EACH USER SEES

Two peers over one set of notes. No test resolves yet: the last column names the row each row rests
on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| adds a note before an existing one | -- | one more note, no number anywhere | the new note as one, the old one as two, on both screens | none -- `PROSE-FEAT-013` |
| adds a note at the end | adds a note at the end at the same moment | both notes, in the order the text converged on | the same two notes, numbered one after the other | none -- `PROSE-FEAT-013` |
| deletes the sentence that calls a note | -- | the note, with no call left | the note gone from the page, and listed as orphaned for review | none -- `PROSE-FEAT-069` |
| deletes a note | edits that note's text at the same moment | no note | the note gone on both screens, and no call left behind | none -- `PROSE-FEAT-070` |
| moves a paragraph past another note | -- | the two calls, in the new order | the two numbers swapped, on both screens | none -- `PROSE-FEAT-013` |
| tries to add a note in the footer | -- | nothing | a refusal, and the footer unchanged | none -- `PROSE-FEAT-071` |

---

## 3. MODEL

**Containers.** The note's body is one annotation entry of the shared annotation map, and the call
is an inline mark on the text carrying that entry's identity. Both are declared in
`ambre-spec-prose-schema.md section 3`, and the identity is a value the peer that creates the note
mints (`section 2` of that document).

**The number is not stored.** It is counted by walking the block tree in document order and, inside
each block, the positions of the calls (`PROSE-FEAT-013`). An inserted note therefore renumbers
everything after it with no write, and two peers converge on the text and then agree on the
numbers.

**The three rules the pair obeys:**

* **The orphan.** Deleting the character the call sits on deletes the mark, and the note is then
  reachable from nothing. It is **not** removed (`PROSE-FEAT-069`): the view lists orphaned notes
  for review, and only the delete command removes one. The comments feature applies the same
  policy.
* **The delete.** Deleting a note removes its entry and strips every call still carrying its
  identity (`PROSE-FEAT-070`). No renumbering follows, because nothing stored a number.
* **Where a note may be called.** Body text and table-cell text. Not a header and not a footer
  (`PROSE-FEAT-071`): a band repeated on every page cannot own a note belonging to one page's
  foot. The refusal is reported.

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertFootnote { block_id, pos, note_text }` | a note, and the call that reaches it |
| `EditFootnote { footnote_id, new_text }` | rewrites the note's body |
| `DeleteFootnote { footnote_id }` | removes the note and every call to it |

**Where a note is placed.** A note is placed at the foot of the page its call falls on, and follows
that call through any later reflow; the separator is drawn once per page, and an edit that adds or
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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The number is counted when the document is read | a stored number is wrong for every following note the moment one is inserted, so it would have to be rewritten down the whole document -- a write per note, per insertion, per peer. Counting costs one walk and converges by construction (`PROSE-FEAT-013`) | storing the ordinal on the note, which turns an insertion into a rewrite and a merge of numbers |
| The identity is minted once and never changes | the identity is what joins the call to the note, and it has to survive a move, an edit and a remote change. A number cannot: it is exactly what changes | keying the note by its number, which breaks the join on the first insertion above it |
| The call is a mark, and the body is an entry | a call belongs inside a sentence, where only a mark can sit, and a note's body is a paragraph of text that has to merge like one. Splitting them puts each half in the container that merges it correctly | storing the note's text inline in the body, which would put a paragraph inside a sentence and paginate as body text |
| A deleted call orphans the note | the note is content the author wrote, and the deletion was of a sentence: destroying the note with it loses work with no gesture that says so. Keeping it lets the author see what became unreachable and decide (`PROSE-FEAT-069`) | purging the note with the call, which loses a paragraph on an ordinary text deletion and cannot be undone from the note's side |
| Deleting a note strips its calls | half of a pair is not a state the reader can act on: a call to nothing renders a number with no note, and a note with no call is the orphan above. One command removes both halves (`PROSE-FEAT-070`) | removing the entry alone, which leaves calls resolving to nothing scattered through the text |
| No note in a header or a footer | the band is drawn on every page, so one call would produce a note on every page, and the number would differ per page. There is no correct number, so there is no correct behaviour (`PROSE-FEAT-071`) | allowing it and numbering per page, which contradicts the document-order numbering every other note uses |
| A note in a table cell is an ordinary note | the cell is body text on a page like any other, and the call's position in document order is well defined. Refusing it would be a second rule with no cause behind it | refusing notes in tables, which is a restriction the reader meets with no reason they can see |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing surfaces an orphan.** `PROSE-FEAT-069` requires the note to be kept for review, and no
  event carries the orphan set and no command lists it. A comment orphaned by the same rule has the
  same gap.
* **The refusal has no event.** `PROSE-FEAT-071` requires the refusal to be reported, and the
  engine declares one untyped error alone (`features/ambre-spec-prose-engine.md section 3`). Four features
  report through the same gap.
* **The note's entry carries no author and no stamp**, while a comment's carries both
  (`ambre-spec-prose-schema.md section 3`). Whether a note shows who wrote it is undecided.
* **Nothing rules on an endnote.** Every rule here places the note on its call's page, and no
  design in the corpus places one at the end of the document.
* **Nothing rules on the numbering style.** The number is stated as a count and no rule says
  whether it may be a symbol, restart per page or restart per section.
* **A note taller than the page it belongs to has no rule**, and it is the case where placing the
  note on its call's page and fitting it on the page contradict each other.
