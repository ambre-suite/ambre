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
| `TRAME-151` | `INV` | `Ctrl+Y` MUST redo, the binding users of other spreadsheets reach for. | this spec section 4 | `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- it seeds the keymap from the declared table and reads the three chords back, so a removed `Ctrl+Y` binding fails it | none | verified |
| `TRAME-123` | `IO` | A named snapshot MUST be restorable after the session that made it. | this spec section 6 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-153` | `CONC` | Two peers MUST converge when one restores a snapshot while the other edits. | this spec section 2 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Restore a named snapshot

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-124` | `POST` | Restoring a snapshot MUST be undoable, like any other change to the document. | this spec section 6 | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-154` | `FRAME` | Restoring a snapshot MUST NOT remove a snapshot from the list, the restored one included. | derived | not built | unbuilt -- lands with the `Version history` Story (`ambre-roadmap.md` post-V1) | unimplemented |
* **Budget.** none of its own: an undo replays one item through the command path, under the command
  rows of `ambre-spec-budgets.md section 1`. What the stack holds is inside the `LoroDoc` holder of
  `ambre-spec-budgets.md section 6`, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 2 `verified`, 4 `unimplemented`. What holds is the granularity rule, four ways,
  and the multi-line delete keeps it (`TRAME-088`). The three chords are read back off the seeded
  keymap, and version history is unbuilt on both sides of the grammar. What a restore owes the
  snapshot list is open, and so is what two peers reach when one restores while the other edits.
  The shared rules and their coverage are `ambre-spec-undo.md`.

---

## 2. WHAT EACH USER SEES

The shared scenarios are `ambre-spec-undo.md section 2`: an undo reverts the local peer alone, a
revert travels as a delta, two peers undo into each other. Trame's own:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| replaces every match in the sheet, then undoes | types into a cell the replace rewrote, after it lands | the replace reverted, Bob's value standing | one undo takes the whole replace back, and never the cell Bob typed (`TRAME-017`, `UNDO-001`) | none -- no two-peer test presses undo (`ambre-spec-undo.md section 6`) |
| restores a named snapshot | edits a cell while the restore lands | not ruled | undecided: whether a restore is a write Bob receives or a fork of his document is open (`TRAME-153`) | not built |

---

## 3. MODEL

**Containers.** None of its own: an undo item is one commit, not one container write.

**Commands and events** (`ambre-bus`), the shared shape of `ambre-spec-undo.md section 3`:

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `CommandId::UNDO` / `REDO` | chrome to Logic | the same two intents, from the toolbar or the keymap |
| `TrameEvent::UndoState { can_undo, can_redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

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
| deleting a selection of five rows | five, and the rule says one (`TRAME-088`) |

**Code:**

* `crates/ambre-trame/src/logic/commands.rs` -- `apply_history`, `undo_state_event`, and the group
  seal at the head of the dispatcher;
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
| Version history is a named snapshot the user asks for | a longer stack, or one persisted across sessions | a stack deeper than the session offers to revert operations the user cannot see, and no depth is the right one. A name is what a user recognizes a month later, and the roadmap schedules it as such (`ambre-roadmap.md` post-V1) |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Trame's own:

* **Version history does not exist** (`TRAME-123`, `TRAME-124`, `TRAME-153`, `TRAME-154`). No
  container, no command, no surface. A user who closes a document loses every step of it, and the
  roadmap's answer is a named snapshot rather than a longer stack. Prose carries a design for the
  same feature and Trame carries none (`features/ambre-spec-prose-undo.md section 5`). The shape is
  open: whether the snapshot list is per document or per application, and whether restoring one is
  a write or a fork.
