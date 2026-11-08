# AMBRE -- FEATURE: TRAME UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship; the depth is unbounded and version history has no code.

> Trame's half of the shared undo model (`ambre-spec-undo.md`): what one item is on a grid, and
> which caches an undo rebuilds. The spreadsheet restates none of the shared rules.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-undo.md`,
`UNDO-001` through `UNDO-009`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-017` | `INV` | One gesture that writes cells MUST become one undo item, however many times the document is committed to carry it out. | this spec section 3; architecture section 2.D | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item`; `::filtered_column_edit_is_one_undo_item` -- the two-commit case; `ambre_trame::logic::tests::group1::selection_style_is_a_single_undo`; `ambre_trame::logic::tests::group6::replace_all_rewrites_and_undoes_as_one_group` -- four gestures that write cells, the gesture that deletes lines being outside this row and held by `TRAME-088` | none | verified |
| `TRAME-151` | `INV` | `Ctrl+Y` MUST redo, the binding users of other spreadsheets reach for. | this spec section 4 | `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- it seeds the keymap from the declared table and reads the three chords back, so a removed `Ctrl+Y` binding fails it; `ambre_belt::trame_grid::tests::undo_takes_back_one_edit_and_redo_brings_it_back` -- in the browser | none | verified |
| `TRAME-288` | `INV` | Undo and redo MUST walk the line deletes and every other edit in the order the user made them. | this spec section 3 | `ambre_trame::logic::line_history_tests::a_recorded_line_delete_undoes_as_the_undo_manager` -- a proptest, 256 cases: after each step the two histories offer the same undo and the same redo, line deletes mixed with writes, inserts, moves, sorts, merges and remote merges | none | verified |
| `TRAME-123` | `IO` | A named snapshot MUST be restorable after the session that made it. | this spec section 6 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-153` | `CONC` | Two peers MUST converge when one restores a snapshot while the other edits. | this spec section 2 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Restore a named snapshot

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-124` | `POST` | Restoring a snapshot MUST be undoable, like any other change to the document. | this spec section 6 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-154` | `FRAME` | Restoring a snapshot MUST NOT remove a snapshot from the list, the restored one included. | derived | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
### 1.3 Undo a line insert or delete

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-286` | `POST` | An undo of a line delete MUST leave the document the undo manager would leave, and so MUST its redo. | this spec section 3; `ambre-spec-undo.md` (`UNDO-001`, `UNDO-002`) | none | fails -- an undo of a recorded line delete puts back a new list element, and a later undo of an older move or sort, through the undo manager, revives the element the delete took: the row list then holds the line twice. `ambre_trame::logic::line_history_tests::an_undone_delete_keeps_one_element_through_a_managed_sort` reproduces it and stays, ignored. The proptest that held this row, `ambre_trame::logic::line_history_tests::a_recorded_line_delete_undoes_as_the_undo_manager`, draws it in some runs; lands with the `One undo history for Trame, or two by decision` Story (`ambre-roadmap.md` E6) | failing |
| `TRAME-289` | `POST` | An undo and a redo of a line insert MUST each hold the Trame whole line delete row of `ambre-spec-budgets.md section 1`. | `ambre-spec-budgets.md section 1`; this spec section 6 | not built | unbuilt -- lands with the `An undo of a line insert within its budget on a large sheet` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-287` | `FRAME` | An undo or a redo of a line delete MUST NOT overwrite a value a peer wrote after the delete. | this spec section 2, row 2; `ambre-spec-undo.md` (`UNDO-007`) | `ambre_trame::logic::line_history_tests::an_undone_line_delete_keeps_a_later_write_of_a_peer` -- the peer types over a formula the delete rewrote, and the value stands through the undo and the redo, on both peers | none | verified |

* **Budget.** an undo and a redo of a line delete each hold the Trame whole line delete row of
  `ambre-spec-budgets.md section 1`. Any other undo runs under the command rows there. The stack
  sits inside the `LoroDoc` holder of section 6, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 4 `verified`, 1 `failing`, 5 `unimplemented`. What holds is the granularity
  rule, four ways, and the multi-line delete keeps it (`TRAME-088`). The three chords are read back
  off the seeded keymap, and version history is unbuilt on both sides of the grammar. What a restore
  owes the snapshot list is open, and so is what two peers reach when one restores while the other
  edits. The shared rules and their coverage are `ambre-spec-undo.md`.

---

## 2. WHAT EACH USER SEES

The shared scenarios are `ambre-spec-undo.md section 2`: an undo reverts the local peer alone, a
revert travels as a delta, two peers undo into each other. Trame's own:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| replaces every match in the sheet, then undoes | types into a cell the replace rewrote, after it lands | the replace reverted, Bob's value standing | one undo takes the whole replace back, and never the cell Bob typed (`TRAME-017`, `UNDO-001`) | none -- no two-peer test presses undo (`ambre-spec-undo.md section 6`) |
| deletes a row a formula reads, then undoes | types over that formula once the delete lands | the row back, Bob's value standing | the row comes back with its cells, and the cell Bob typed keeps Bob's value (`TRAME-287`) | `ambre_trame::logic::line_history_tests::an_undone_line_delete_keeps_a_later_write_of_a_peer` |
| restores a named snapshot | edits a cell while the restore lands | not ruled | undecided: whether a restore is a write Bob receives or a fork of his document is open (`TRAME-153`) | not built |

---

## 3. MODEL

**Containers.** None of its own: an undo item is one commit, not one container write.

**A line delete records its own step** (`TRAME-286`). The undo manager answers an undo by a
checkout, and a checkout rebuilds the state of every map the item wrote. On a sheet of 10k formulas
that is the whole cell map and the whole generation record, far over the budget.

So a line delete notes each key it writes, with the value on each side. It also notes each list
element it removes, with the element before it. Its commit carries an origin the undo manager
excludes. An undo writes each value back, and puts each element back after its neighbour. A redo
writes the delete again. A key a later write changed keeps that write (`TRAME-287`).

**One history orders the two kinds of step** (`TRAME-288`). It holds an entry for each item of the
undo manager and each recorded line delete, in the order they were made. An undo takes the top
entry: an item goes to the manager, a line delete replays its notes. A new edit clears the redo
side, as the manager clears its own. A delete whose notes cannot hold a write commits for the
manager, as every delete did before.

**Commands and events** (`ambre-bus`), the shared shape of `ambre-spec-undo.md section 3`:

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `StaticKey<Commands>::UNDO` / `REDO` | chrome to Logic | the same two intents, from the toolbar or the keymap |
| `TrameEvent::UndoState { undo, redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

The engine field is the shared `undo_manager` (`ambre-spec-undo.md section 3`).

**Which caches step 3 of the shared sequence reseeds** (`UNDO-005`), whole and from the document:

* the sheet index;
* the formula engine;
* the merge spans;
* the styles;
* the filter.

**What one undo item is** (`TRAME-017`):

| Gesture | Items |
|---|---|
| typing into a cell, however many keystrokes | one, sealed when the cell is left |
| one value typed into a whole selection | one (`TRAME-UI-053`) |
| a style applied to a whole selection | one (`TRAME-093`) |
| inserting a row, a column, or a sort | one each |
| applying or clearing a shared filter | one (`TRAME-FEAT-007`) |
| replacing every match in the sheet | one (`ambre-spec-find.md`, `FIND-006`) |
| activating a personal filter view | none: it changes no document (`TRAME-FEAT-016`) |
| adding or removing a validation or a protection | none: a rule over the cells is configuration, not data (`features/ambre-spec-trame-cell-editing.md section 3`) |
| deleting a selection of five rows | one (`TRAME-088`) |

**Code:**

* `crates/ambre-trame/src/logic/commands.rs` -- `apply_history`, `undo_state_event`, and the group
  seal at the head of the dispatcher;
* `crates/ambre-trame/src/logic/line_history.rs` -- the notes of a line delete, their replay, and
  the history that orders them with the items of the undo manager;
* `chrome/commands.rs` -- the two command identifiers, the three chords, the enabled state.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` (`UNDO-010`) |
| `Ctrl+Shift+Z` | `Redo` (`UNDO-011`) |
| `Ctrl+Y` | `Redo` (`TRAME-151`), the second binding users of other spreadsheets reach for |
| the toolbar's two buttons | the same two intents, through the shared command path |

The two affordances are disabled from `UndoState` alone (`UNDO-006`): the chrome counts nothing.

---

## 5. REJECTED ALTERNATIVES

The shared decisions are `ambre-spec-undo.md section 5`: the local-only revert, the broadcast
inverse, the whole reseed, the stack that dies with the document. Trame's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| One value or one style over a whole selection is one item | one item per cell written | a user who styles a thousand cells means one gesture, and pressing `Ctrl+Z` a thousand times to take it back is not an undo. The selection is what the user acted on, so the selection is what the item holds |
| A validation and a protection stay outside the history | recording them like any other write | a rule over the cells is configuration, not data. An undo that removes the protection a user just set takes back a decision rather than a change, and the two live at different speeds |
| A personal filter view enters no history | recording the activation | it writes no document, so there is nothing to revert and nothing a peer would receive. What it changes is what this user looks at, and `TRAME-FEAT-016` keeps that local |
| A line delete records its own step | the undo manager's item, as for every other edit | on a sheet of 10k formulas, the manager spends about 336 million instructions undoing one line delete and 288 million redoing it, measured on 2026-11-07 under `callgrind`. A first undo also builds a cache of the whole history. That is about 420 ms on a phone, against a budget of 10 ms. The cost grows with the maps the item wrote, not with what it changed |
| The inverse is recorded by Trame | a fix of the checkout inside Loro, on a fork | a fork of the CRDT engine is a second engine to keep, and every upstream release must be merged into it. The checkout rebuilds a map at a version by design, and the notes of a line delete are a few keys |
| A deleted line loses its cells at once | cells kept under the deleted line, hidden, and removed by a later pass | the document keeps every cell of every deleted line until the pass runs. The pass needs its own rule on when a peer may still undo, as Prose needed one for a hidden block (`PROSE-121`) |
| Version history is a named snapshot the user asks for | a longer stack, or one persisted across sessions | a stack deeper than the session offers to revert operations the user cannot see, and no depth is the right one. A name is what a user recognizes a month later, and the roadmap schedules it as such (`ambre-roadmap.md` post-V1) |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Trame's own:

* **The two histories disagree on a list element** (`TRAME-286`). A recorded step names a list
  element by its spelling, and an item of the undo manager names it by its identity in the list.
  An undo of a recorded delete puts back a new element under the old spelling. A later undo of an
  older move or sort revives the old element, so the line stands twice. A line insert that
  recorded its own step failed the same way on its redo, measured and withdrawn on 2026-11-08
  (`ambre_trame::logic::line_history_tests::a_redone_insert_keeps_one_element_through_a_managed_move`).

  A write past the used range grows the lists inside an item of the manager too. Whether every list
  write is recorded, or the manager stops at a recorded step, is the decision of the roadmap.

* **Version history does not exist** (`TRAME-123`, `TRAME-124`, `TRAME-153`, `TRAME-154`). No
  container, no command, no surface. A user who closes a document loses every step of it, and the
  roadmap's answer is a named snapshot rather than a longer stack. Prose carries a design for the
  same feature and Trame carries none (`features/ambre-spec-prose-undo.md section 5`). The shape is
  open: whether the snapshot list is per document or per application, and whether restoring one is
  a write or a fork.
