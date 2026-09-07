# AMBRE -- FEATURE: UNDO AND REDO

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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-017` | `INV` | One gesture that writes cells MUST become one undo item, however many times the document is committed to carry it out. | this spec section 2; architecture section 2.D | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item`, `::filtered_column_edit_is_one_undo_item` on the two-commit case, `group1::selection_style_is_a_single_undo`, `group6::replace_all_rewrites_and_undoes_as_one_group`. Four gestures that write cells; the gesture that deletes lines is outside this row and `TRAME-088` holds it, unverified and broken | verified |
| `TRAME-121` | `INV` | `Ctrl+Shift+Z` MUST redo. | this spec section 3 | `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords`, which seeds the keymap from the declared table and reads the three chords back: a removed `Ctrl+Shift+Z` binding fails it | verified |
| `TRAME-151` | `INV` | `Ctrl+Y` MUST redo, the binding users of other spreadsheets reach for. | this spec section 3 | `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords`, which seeds the keymap from the declared table and reads the three chords back: a removed `Ctrl+Y` binding fails it | verified |
| `TRAME-152` | `INV` | `Ctrl+Z` MUST undo. | this spec section 3 | `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords`, which seeds the keymap from the declared table and reads the three chords back: a removed `Ctrl+Z` binding fails it | verified |
| `TRAME-123` | `IO` | A named snapshot MUST be restorable after the session that made it. | this spec section 4 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-153` | `CONC` | Two peers MUST converge when one restores a snapshot while the other edits. | this spec section 4 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |

### 1.2 Restore a named snapshot

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TRAME-124` | `POST` | Restoring a snapshot MUST be undoable, like any other change to the document. | this spec section 4 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-154` | `FRAME` | Restoring a snapshot MUST NOT remove a snapshot from the list, the restored one included. | derived | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |
* **Budget.** none of its own: an undo replays one item through the command path, under the command
  rows of `ambre-spec-budgets.md section 1`. What the stack holds is inside the `LoroDoc` holder of
  `ambre-spec-budgets.md section 6`, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 4 `verified`, 0 `unverified`, 4 `unimplemented`. The granularity rule is held
  four ways, and only for gestures that write cells: `TRAME-017` claimed every mutating handler, and
  `TRAME-088` says in so many words that the line delete contradicts it, so the row was counted
  `verified` for a span it does not hold. The three chords are read back off the seeded keymap, and
  `Ctrl+Z` was in no row at all before this pass. Version history is unbuilt, and the conjunction it
  was written as hid two rules on opposite sides of the grammar: what a restore owes the history is
  a `POST`, what two peers reach around it is a `CONC`. The shared rules and their coverage are in
  `ambre-spec-undo.md`.

---

## 2. MODEL

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

## 3. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` (`TRAME-152`) |
| `Ctrl+Shift+Z` | `Redo` (`TRAME-121`) |
| `Ctrl+Y` | `Redo` (`TRAME-151`), the second binding users of other spreadsheets reach for |
| the toolbar's two buttons | the same two intents, through the shared command path |

The two affordances are disabled from `UndoState` alone (`UNDO-006`): the chrome counts nothing.

---

## 4. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Trame's own:

* **A five-row delete takes five undos** (`TRAME-088`). The rule is written in
  `features/ambre-spec-trame-rows-and-columns.md`, and it is the one place the granularity table
  above says one item and the code produces several.
* **Version history does not exist** (`TRAME-123`, `TRAME-124`, `TRAME-153`, `TRAME-154`). No
  container, no command, no surface. A user who closes a document loses every step of it, and the
  roadmap's answer is a named snapshot rather than a longer stack. Prose carries a design for the
  same feature and Trame carries none (`features/ambre-spec-prose-undo.md section 4`), so the shape
  is open: whether the snapshot list is per document or per application, and whether restoring one
  is a write or a fork.
