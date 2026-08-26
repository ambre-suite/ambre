# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** design-only

> Undo takes back what this author did, and never what anybody else did. A collaborator's sentence
> cannot disappear because the person beside them pressed a key.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-022` | Undo MUST be scoped to the local author: it MUST NOT revert another author's edits. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-012` | Undo MUST be scoped to the local author and MUST NOT revert another author's edits (`PROSE-022` is the same rule in the engine). | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-032` | An undo MUST reach the other peers as an ordinary change: the inverse edit MUST be applied to the document, never announced as a message of its own. | this spec section 3; `PROSE-023` | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-UI-045` | After an undo or a redo, the caret MUST be placed at the edit that was taken back or put back, and a selection MUST collapse to it. | this spec section 3 | not built -- lands with the "Undo, footnotes and cross references" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none: an undo applies one inverse operation and exports the delta it produced, which
  is the local-write row of `ambre-spec-budgets.md section 1` and no cost of its own. What
  an unbounded history costs in memory is named in section 5, and it is unbudgeted.
* **Boundary.** none of its own: an undo is a document write like any other, and `ZK-001` seals
  every payload before it leaves the client. The history lives in memory on the Logic thread and
  is never persisted or sent.
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-022` and `PROSE-FEAT-012` are one rule stated on both sides of the engine boundary, and
  the two-peer table below is what a reference-model test would replay: two peers, interleaved
  edits, one undo, and an assertion on what survived.

---

## 2. WHAT EACH USER SEES

Two peers, one paragraph. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| types a sentence, then presses undo | types a sentence of his own in between | Bob's sentence alone | Alice's sentence gone, Bob's untouched | none -- `PROSE-022` |
| presses undo | -- | the inverse edit, broadcast like any other | the same document on both screens | none -- `PROSE-FEAT-032` |
| presses undo | presses undo at the same moment | both inverses, each over its own author's work | each author's own last edit taken back | none -- `PROSE-FEAT-012` |
| presses undo, then redo | -- | the edit again | the text back, the caret at it | none -- `PROSE-UI-045` |
| undoes a replace-all of forty matches | -- | one inverse covering the forty | the forty replacements taken back at once | none -- `PROSE-FEAT-015` |
| undoes an edit Bob has since deleted | -- | the inverse, over text that is gone | no error, and nothing restored where the text was | none -- section 5 |

---

## 3. MODEL

**Containers.** None. The history is in memory, on the Logic thread, for the session only.

**The manager.** The document's own undo manager is built from the document at start and observes
local transactions on it. Every mutating handler commits through the document, so the manager
records the operation without any handler naming it. It is declared as a field of the engine state
(`features/ambre-spec-prose-engine.md section 3`):

```rust
/// Undo and redo, scoped to the local author: it observes local transactions on the
/// document and never records a remote one (`PROSE-022`).
undo_manager: loro::UndoManager,
```

**What an undo is.** Two commands, `Undo` and `Redo`, carrying nothing. The manager applies the
inverse of the local operation to the document; the change that produces is exported as one delta
on the ordinary local path and broadcast like a keystroke (`PROSE-FEAT-032`). No peer learns that
what it received was an undo, and none has to treat it apart.

**Why the scope holds.** The manager records local transactions only, so what it can invert is
exactly what this author wrote. A remote operation is applied to the document by the import path
and never enters the history. The rule is therefore a property of what is recorded, not a check
performed at undo time.

**Where the caret lands.** The confirming change carries the delta, and the view reads the edit
point out of it: the caret moves there and a selection collapses to it (`PROSE-UI-045`). Leaving
the caret where it was would put it at an offset the undo has just invalidated.

**What is one undo item.** A replace-all is one (`PROSE-FEAT-015`,
`features/ambre-spec-prose-find-replace.md`). Everything else is not ruled on: see section 5.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the undo chord, or the menu entry | takes back this author's last recorded change |
| the redo chord, or the menu entry | puts it back |
| an undo with nothing left to take back | does nothing |
| any local edit after an undo | not ruled on: whether the redo history is dropped is undecided |
| a remote change | never enters this author's history |

Both redo chords, and the fold of `Ctrl` and `Cmd`, are one decision for every application, and it
is the "One keymap" Story.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Undo takes back the local author's work alone | in a shared document, undoing whatever happened last means undoing a colleague's sentence because they typed while you paused. That is not a history a user can reason about, and it makes the key dangerous rather than reassuring. Scoping to the author is what every collaborative editor of this kind settled on | undoing the document's last operation, whoever wrote it |
| The inverse is an ordinary edit | an undo that travelled as its own message would need every peer to know how to invert an operation, and the two paths would have to agree forever. Applying the inverse locally and shipping the resulting delta means there is one write path and one convergence proof | broadcasting an undo instruction, which is a second protocol over the same document |
| The history is in memory and per session | it is one author's editing gesture, not content: persisting it means merging two histories on reconnect and deciding what a history means after another peer has rewritten the text. Nothing depends on it surviving a reload | persisting the history with the document, which puts a second, mergeable structure in the file |
| The caret follows the change | an undo moves text the caret was sitting in, and leaving it where it was puts it at an offset that no longer means anything. The user's attention is on what just changed, which is where the caret belongs | leaving the caret where it stood, which is the version that types the next character in the wrong place |
| Cross-author undo is refused outright | inverting an operation another author has since built on requires a transformation that is correct only for some interleavings, and the cost of getting it wrong is a document nobody can explain | a collaborative undo, whose complexity buys a gesture users do not ask for |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The history has no bound.** Nothing caps its depth or its memory, and a long editing session
  grows it without limit. A bounded depth in every application is the "One undo bound" Story; what
  merges into one step stays this application's own, and that half is the bullet below.
* **What merges into one undo item is undecided, and something already depends on it.** The
  selection-delete and the insertion that replaces it are sent as two commands and named as one
  step nowhere; the track-changes grouping window declares itself "aligned with the undo merge
  interval" (`features/ambre-spec-prose-track-changes.md`), an interval no document defines. One
  rule is written today, and it is replace-all.
* **Nothing rules on the redo history after a new edit.** Whether typing after an undo drops what
  was undone is the behaviour users meet first, and no rule states it.
* **Nothing rules on undoing an edit whose text a peer has deleted.** The inverse then applies to
  text that is gone; what the author sees, and whether anything is restored, is undecided.
* **A version history is designed and unbuilt, and it is the answer to the bullet above.** The
  history dies with the session; the deferred design keeps a named entry per saved version in
  `meta/version_history`, whose Loro snapshot is sealed with the document key and stored beside the
  document (`ambre-spec-prose-schema.md section 3`, `ambre-spec-object-store.md`). No command
  saves, lists, restores or deletes one, and nothing says whether restoring a version is an
  ordinary undoable change or a fork. Trame records the same gap and carries no design at all
  (`features/ambre-spec-trame-undo.md`); the Story is "The later writing set"
  (`ambre-roadmap.md`).
* **Nothing rules on undo inside a table cell, a header or a footnote.** Those hold ordinary
  blocks, so their edits enter the same history, and no rule says whether the caret returns to a
  band the reader may not be looking at.
