# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship; the depth is unbounded and version history has no code.

> A user presses `Ctrl+Z` and their last action comes back. A collaborator's action never does, and
> never disappears from under them.

Undo is local to the peer that typed. It is not a rewind of the document: it is one more change to
it, sent to the other users like any other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this
feature realizes is normative in `ambre-spec-message-bus.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-017` | Every mutating `TrameCommand` handler MUST end with `doc.commit()`, so that one command is one undo item. | this spec section 3; architecture section 2.D | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item`, `::filtered_column_edit_is_one_undo_item`, `group1::selection_style_is_a_single_undo`, `group6::replace_all_rewrites_and_undoes_as_one_group` | verified |
| `TRAME-018` | Undo MUST revert local operations only, never a remote peer's. | this spec section 2, row 1; architecture section 2.D | `ambre_trame::logic::tests::group2::undo_local_only_not_remote`, `::undo_insert_row_rebuilds_sheet_index`, `::undo_stack_cleared_on_doc_close` | verified |
| `TRAME-116` | An undo MUST broadcast its inverse operations, like any other change to the document. | this spec section 2, row 2 | `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit` | verified |
| `TRAME-117` | An undo MUST rebuild every derived cache, so nothing derived is stale afterwards. | this spec section 3 | `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index`, `::filtered_column_edit_is_one_undo_item` | verified |
| `TRAME-118` | An undo with nothing to revert MUST leave the document unchanged and MUST emit no delta. | this spec section 3 | `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop`, `::undo_stack_cleared_on_doc_close` | verified |
| `TRAME-119` | The history MUST NOT survive the document being closed. | this spec section 5 | `ambre_trame::logic::tests::group2::undo_stack_cleared_on_doc_close` | verified |
| `TRAME-120` | The undo and redo affordances MUST report what is available, and a command that changes nothing MUST NOT refresh them. | this spec section 4 | `ambre_trame::logic::tests::group2::mutating_command_reports_undo_state`; the chrome half is `ambre_trame::tests::chrome_ui::undo_state_mirrors_into_command_store` | verified |
| `TRAME-121` | A redo MUST be reachable by `Ctrl+Shift+Z` and by `Ctrl+Y`. | this spec section 4 | none. The keymap binds both chords beside `Ctrl+Z` (`chrome/commands.rs`), and no test reads the keymap back | unverified |
| `TRAME-123` | A named snapshot MUST be restorable after the session that made it. | this spec section 6 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-124` | Restoring a snapshot MUST be an ordinary change to the document: undoable, and convergent. | this spec section 6 | not built -- lands with the "Version history" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** none of its own: an undo replays one item through the command path, so it lands under
  the command rows of `ambre-spec-budgets.md section 1`. What the stack holds is inside the
  `LoroDoc` holder of `ambre-spec-budgets.md section 6`, and the bound that would cap it is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 7 `verified`, 1 `unverified`, 2 `unimplemented`. The engine half is the
  best-held surface of the application: what one undo item is, whose operations revert, and what an
  undo leaves behind all have named tests. What is loose is the keymap; the bound the stack still
  lacks is one shared number, and it is `UI-THREAD-016`.

---

## 2. WHAT EACH USER SEES

Two peers, one sheet. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| writes into `A1` | writes into `B1`, then undoes | Alice's value, and `B1` as it was | Bob's own value gone, Alice's untouched | `logic::tests::group2::undo_local_only_not_remote` |
| writes into `A1`, then undoes | reads `A1` | `A1` as it was | the value gone on both screens, from a delta like any other (`TRAME-116`) | `logic::tests::group1::undo_then_redo_round_trips_a_cell_edit` |
| undoes | undoes at the same moment | each peer's own last action reverted | two independent reverts, and one converged document | none -- the convergence property carries no undo, section 6 |
| deletes five rows, then undoes | -- | four of the five rows still gone | one undo per deleted row (`TRAME-088`) | `features/ambre-spec-trame-rows-and-columns.md section 6` |

---

## 3. MODEL

**Containers.** None of its own. Undo reverts operations over the containers the other features own,
which is why one undo item is one commit rather than one container write.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `CommandId::UNDO` / `REDO` | chrome to Logic | the same two intents, from the toolbar or the keymap |
| `TrameEvent::UndoState { can_undo, can_redo }` | Logic to UI | what the two affordances may offer (`TRAME-120`) |

**Engine state.** One field of `TrameState`:

```rust
/// Local-only history, created when the document opens and dropped when it
/// closes. It observes the `LoroDoc` itself: nothing is registered by hand.
undo_manager: loro::UndoManager,
```

**Steps, when a user undoes or redoes:**

1. ask the history to revert or replay one item, and read whether anything moved;
2. stop here when nothing moved, and answer with the availability alone (`TRAME-118`);
3. rebuild every derived cache -- the index, the engine, the merges, the styles, the filter -- the
   way a structural command does (`TRAME-117`);
4. flush the inverse operations as a delta the other users receive (`TRAME-116`);
5. answer with the active cell's style and the new availability.

Step 3 is a total reseed, not a rebuild of the cells the item touched. That is why no hidden-row
set, no merge span and no compiled formula is stale after an undo.

**What one undo item is:**

| Gesture | Items |
|---|---|
| typing into a cell, however many keystrokes | one, sealed when the cell is left |
| one value typed into a whole selection | one (`TRAME-UI-053`) |
| a style applied to a whole selection | one (`TRAME-093`) |
| inserting a row, a column, or a sort | one each |
| applying or clearing a shared filter | one (`TRAME-FEAT-007`) |
| replacing every match in the sheet | one (`TRAME-UI-082`) |
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
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` (`TRAME-121`) |
| `Ctrl+Y` | `Redo`, the second binding users of other spreadsheets reach for |
| the toolbar's two buttons | the same two intents, through the shared command path |

**The two affordances** are disabled from `UndoState` alone: the chrome holds no history of its own
and counts nothing (`TRAME-120`). A command that writes nothing sends no refresh, so a selection
move never re-enables a button.

**What the user sees after an undo.** The whole viewport is re-fetched, because the reseed is total:
values, merges, styles and hidden rows arrive together, in one frame.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Undo reverts the local peer alone | a user pressing `Ctrl+Z` means "take back what I did". Reverting a collaborator's work would let one user erase another's without touching their document, and no collaborative editor does it | a global history, where undo walks the document's operations whoever wrote them |
| An undo is an ordinary change, broadcast like any other | the inverse operations are operations, so they converge by the one path everything else uses, and a peer that was offline receives them in order | rolling the document back to a stored version, which two peers cannot agree on |
| The caches are rebuilt whole after an undo | an undo can move rows, restore a merge and change a filtered value at once, so the set of things to refresh is the whole derived state. A partial refresh is a list that goes stale | rebuilding only what the reverted item touched |
| The history dies with the document | it lives in memory beside a `LoroDoc` that is itself rebuilt from a snapshot on open, and the operations it would revert are a previous session's. What a user wants across sessions is a named version, not a stack | persisting the stack, which would offer to revert operations the user cannot see |
| The chrome is told what is available, and never counts | one source for the two buttons, and the Logic thread is the only place that knows. A chrome-side counter would drift the moment a command merged two items | the chrome tracking its own depth |
| Both redo chords are bound | the two are split across the applications a user comes from, and binding one teaches nothing to the half that reaches for the other | `Ctrl+Y` alone, which the earlier design carried |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The depth is unbounded** (`UI-THREAD-016`). A long session grows the history until the tab runs
  out of memory, and nothing measures it. The bound is one number shared with the other
  applications, which is why the row is `ambre-spec-ui.md`'s; what merges into one item stays this
  document's.
* **Version history does not exist** (`TRAME-123`, `TRAME-124`). No container, no command, no
  surface. A user who closes a document loses every step of it, and the roadmap's answer is a named
  snapshot rather than a longer stack. Prose carries a design for the same feature and Trame carries
  none (`features/ambre-spec-prose-undo.md section 6`), so the shape is open: whether the snapshot
  list is per document or per application, and whether restoring one is a write or a fork.
* **Undo is not in the convergence property** (section 2, row 3). The two-peer proptest drives
  edits, structural changes, sorts and filters, and never an undo. The local-only rule has one
  hand-written two-peer case; two peers undoing into each other has none, in this application or in
  Toile.
* **A five-row delete takes five undos** (`TRAME-088`). The rule is written in
  `features/ambre-spec-trame-rows-and-columns.md`, and it is the one place the granularity table
  above says one item and the code produces several.
* **The keymap is never read back** (`TRAME-121`). The three bindings are set in one function that
  no test calls, so a removed chord would fail nothing.
