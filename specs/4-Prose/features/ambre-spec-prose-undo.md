# AMBRE -- FEATURE: PROSE UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** implemented

> Prose's half of the shared undo model (`ambre-spec-undo.md`): where the caret lands, and what one
> item is in running text.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-undo.md`,
`UNDO-001` through `UNDO-006`.

### 1.1 Undo or redo the last action

One gesture with two directions and one caret rule, as `ambre-spec-undo.md section 1.2` states the
shared steps once for both.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-UI-045` | `POST` | After an undo or a redo, the caret MUST be placed at the edit that was taken back or put back. | this spec section 2 | `ambre_prose::logic::history::tests::an_undo_puts_the_caret_where_the_text_was_taken_out`; `::an_undone_deletion_puts_the_caret_after_the_restored_text` | none | verified |
| `PROSE-UI-073` | `POST` | A selection open when the undo or the redo runs MUST collapse to that caret. | this spec section 2 | `ambre_prose::logic::history::tests::an_open_selection_collapses_to_the_placed_caret` | none | verified |
| `PROSE-UI-074` | `FRAME` | Moving the caret after an undo or a redo MUST NOT write to the document: where this author looks is not a change a peer receives. | derived | `ambre_prose::logic::history::tests::placing_the_caret_writes_nothing_beyond_the_inverse` | none | verified |
| `PROSE-133` | `POST` | After an undo puts a deleted block back, every later undo and redo MUST act on that block. | this spec section 2; `1-Foundations/ambre-spec-undo.md section 2` (`UNDO-001`) | `ambre_prose::logic::undo_identity_tests::an_undone_delete_then_an_undone_insert_leaves_no_block`; `::an_undone_delete_after_a_move_restores_the_place_and_the_text`; `::undo_and_redo_follow_a_loaded_page_rebuilt_by_hand` -- a proptest, 256 cases of deletes, moves, typing, undos and redos on a loaded page of four blocks, against a page kept by hand | none | verified |
| `PROSE-134` | `POST` | A redo of an undone block insertion MUST put back the block that the insertion created. | this spec section 2; `1-Foundations/ambre-spec-undo.md section 2` (`UNDO-001`) | `ambre_prose::logic::undo_identity_tests::a_redone_insert_puts_back_the_same_block`; `::undo_and_redo_follow_a_page_rebuilt_by_hand` -- a proptest, 256 cases of inserts, deletes, moves, typing, undos and redos on a page of one block, against a page kept by hand | none | verified |
| `PROSE-135` | `POST` | An undo and a redo of a typed key MUST each hold the Prose local text insert row of `ambre-spec-budgets.md section 1`. | `ambre-spec-budgets.md section 1`; this spec section 2 | `callgrind` -- the `key_undo` and `key_redo` operations of `crates/ambre-prose/benches/instructions.rs`, on a document of 300 paragraphs, under their ceilings in the CI `instructions` job | none | verified |

* **Budget.** the local text insert row of `ambre-spec-budgets.md section 1` (`PROSE-135`): an undo
  applies one inverse operation and exports the delta it produced. The unbounded history is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo is a document write like any other, and `ZK-001` seals
  every payload before it leaves the client. The history lives in memory on the Logic thread and is
  never persisted or sent.
* **Verification.** 6 `verified`. `PROSE-UI-074` is the frame the caret rows lean on. A page kept
  by hand is the oracle for the block rows, and an instruction count holds the budget.

---

## 2. MODEL

**Containers.** None: in memory, on the Logic thread, for the session only.

**The history** of `ambre-spec-undo.md section 3` is a field of the engine state
(`features/ambre-spec-prose-engine.md section 3`). A load, a stamp, a comment and a sweep are no
step.

**Where the caret lands.** The engine reads the edit point from the step's diff and sends it
beside the delta. The view holds no document to read it from. A text edit wins over
a structural one: the caret goes after restored text, or where text was taken out.
The caret moves there (`PROSE-UI-045`), and an open selection collapses to it
(`PROSE-UI-073`). Both are view state, and neither is written back (`PROSE-UI-074`).

**What is one undo item.** A replace-all is one (`ambre-spec-find.md`, `FIND-006`;
`features/ambre-spec-prose-find-replace.md`); everything else is section 5.

**The block an undo puts back.** An undo of a deletion clears the hidden marker on the same
node (`PROSE-133`). An undo of an insertion writes that marker, and its redo clears it on the same
node (`PROSE-134`): Loro raises no deleted node.

**Code:** `crates/ambre-prose/src/logic/history.rs`.

---

## 3. SURFACE

| Trigger | Behaviour |
|---|---|
| the undo chord, or the menu entry | takes back this author's last recorded change |
| the redo chord, or the menu entry | puts it back |
| an undo with nothing left to take back | does nothing (`UNDO-003`) |
| any local edit after an undo | not ruled on: whether the redo history is dropped is undecided |
| a remote change | never enters this author's history (`UNDO-001`) |

---

## 4. REJECTED ALTERNATIVES

The shared decisions are `ambre-spec-undo.md section 5`. They are the local-only revert, the
broadcast inverse, the caches held equal to a whole reseed and the stack that dies with the
document. Prose's own, all of them design:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The caret lands at the edit the undo took back | leaving the caret where the user left it | an undo the author cannot see is an undo they press again. The edit point is in the delta the change carries, so the view reads it there and needs nothing stored |
| An open selection collapses to that caret | restoring the selection the edit was made with | the selection that made the edit is not in the document, and a peer's writes have moved the text under it. A caret is a position the inverse operation still defines; a range is not |
| The caret is view state, never written back | recording it beside the text | where this author looks is not a change a peer receives (`PROSE-UI-074`). A caret in the document turns every undo into an operation the relay carries and the other author's view obeys |
| A version is a named entry the user saves | a longer history, or the Loro snapshot kept in the document | the session's stack answers "take back what I just did" and nothing longer. What a user wants a month later is a name, and the snapshot behind it is sealed with the document key and stored beside the document, not inside it (`ambre-spec-prose-schema.md section 3`) |

---

## 5. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Prose's own:

* **What merges into one undo item is undecided, and something already depends on it.** The
  selection-delete and the insertion that replaces it are sent as two commands and named as one
  step nowhere. The track-changes grouping window declares itself "aligned with the undo merge
  interval" (`features/ambre-spec-prose-track-changes.md`), an interval no document defines. One
  rule is written today, and it is replace-all.
* **Nothing rules on the redo history after a new edit.** Whether typing after an undo drops what
  was undone is the behaviour users meet first, and no rule states it.
* **Nothing rules on undoing an edit whose text a peer has deleted.** The history leaves such a
  run whole today (section 2), and no row states what the author should see.
* **A version history is designed and unbuilt, and it is the answer to the first bullet's span.** The
  history dies with the session. The deferred design keeps a named entry per saved version in
  `meta/version_history`. Its Loro snapshot is sealed with the document key and stored beside the
  document (`ambre-spec-prose-schema.md section 3`, `ambre-spec-object-store.md`). No command
  saves, lists, restores or deletes one, and nothing says whether restoring a version is an
  ordinary undoable change or a fork. Trame records the same gap and carries no design at all
  (`features/ambre-spec-trame-undo.md`); the Story is
  "The later writing set" (`ambre-roadmap.md` POST-V1).
* **Nothing rules on undo inside a table cell, a header or a footnote.** Those hold ordinary
  blocks, so their edits enter the same history. No rule says whether the caret returns to a
  band the reader may not be looking at.
