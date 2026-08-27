# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship; the depth is unbounded and version history has no code.

> Trame's half of the shared undo model (`ambre-spec-undo.md`): what one item is on a grid, and
> which caches an undo rebuilds. The spreadsheet restates none of the shared rules.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-undo.md`,
`UNDO-001` through `UNDO-006`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-017` | Every mutating `TrameCommand` handler MUST end with `doc.commit()`, so that one command is one undo item. | this spec section 2; architecture section 2.D | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item`, `::filtered_column_edit_is_one_undo_item`, `group1::selection_style_is_a_single_undo`, `group6::replace_all_rewrites_and_undoes_as_one_group` | verified |
| `TRAME-121` | A redo MUST be reachable by `Ctrl+Shift+Z` and by `Ctrl+Y`. | this spec section 3 | none. The keymap binds both chords beside `Ctrl+Z` (`chrome/commands.rs`), and no test reads the keymap back | unverified |
| `TRAME-123` | A named snapshot MUST be restorable after the session that made it. | this spec section 4 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-124` | Restoring a snapshot MUST be an ordinary change to the document: undoable, and convergent. | this spec section 4 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** none of its own: an undo replays one item through the command path, under the command
  rows of `ambre-spec-budgets.md section 1`. What the stack holds is inside the `LoroDoc` holder of
  `ambre-spec-budgets.md section 6`, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 1 `verified`, 1 `unverified`, 2 `unimplemented`. The granularity rule is held
  four ways; the keymap is never read back; version history is unbuilt. The shared rules and their
  coverage are in `ambre-spec-undo.md`.

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
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` (`TRAME-121`) |
| `Ctrl+Y` | `Redo`, the second binding users of other spreadsheets reach for |
| the toolbar's two buttons | the same two intents, through the shared command path |

The two affordances are disabled from `UndoState` alone (`UNDO-006`): the chrome counts nothing.

---

## 4. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Trame's own:

* **A five-row delete takes five undos** (`TRAME-088`). The rule is written in
  `features/ambre-spec-trame-rows-and-columns.md`, and it is the one place the granularity table
  above says one item and the code produces several.
* **The keymap is never read back** (`TRAME-121`). The three bindings are set in one function that
  no test calls, so a removed chord would fail nothing.
* **Version history does not exist** (`TRAME-123`, `TRAME-124`). No container, no command, no
  surface. A user who closes a document loses every step of it, and the roadmap's answer is a named
  snapshot rather than a longer stack. Prose carries a design for the same feature and Trame carries
  none (`features/ambre-spec-prose-undo.md section 4`), so the shape is open: whether the snapshot
  list is per document or per application, and whether restoring one is a write or a fork.
