# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- the three applications ship an undo stack over the shared model.
Prose shows no undo affordance, and the depth is unbounded in all three.

> A user presses `Ctrl+Z` and their last action comes back. A collaborator's action never does, and
> never disappears from under them.

Undo is local to the peer that acted. It is not a rewind of the document but one more change to it,
sent to the others like any other. This document owns the model; each application owns what merges into
one step and which caches it rebuilds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this feature realizes
is normative in `ambre-spec-message-bus.md`. The prefix is `UNDO`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UNDO-004` | `INV` | The history MUST NOT survive the document being closed. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_stack_cleared_on_doc_close`; Toile `ambre_logic::dispatch::tests::closing_a_document_of_each_kind_drops_its_state`; Prose `ambre_logic::dispatch::tests::closing_a_document_of_each_kind_drops_its_state` | none | verified |
| `UNDO-009` | `INV` | A command that changes nothing MUST NOT refresh the undo affordances. | this spec section 4 | Trame `ambre_trame::logic::tests::group2::a_command_that_changes_nothing_reports_no_undo_state`; Toile `ambre_toile::logic::state::undo_state_tests::a_command_that_changes_nothing_reports_no_undo_state`; Prose `ambre_prose::logic::tests::a_write_reports_the_history_and_a_no_op_reports_none` -- each drives a redo on an empty stack and a command that writes nothing, and reads that no availability went out | none | verified |
| `UNDO-010` | `INV` | `Ctrl+Z` MUST undo. | this spec section 4 | Trame `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- it seeds the keymap from the declared table and reads the chords back, so a removed `Ctrl+Z` binding fails it; Toile `ambre_toile::ui::view::view_tests::the_undo_chords_reach_the_logic_thread` -- the key event through the board view sends undo; Prose `ambre_prose::input_tests::the_history_chords_undo_and_redo_the_typing` -- each declared undo chord through the page's key decoder reverts the typing | none | verified |
| `UNDO-011` | `INV` | `Ctrl+Shift+Z` MUST redo. | this spec section 4 | Trame `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- a removed `Ctrl+Shift+Z` binding fails it; Toile `ambre_toile::ui::view::view_tests::the_undo_chords_reach_the_logic_thread` -- the key event through the board view sends redo; Prose `ambre_prose::input_tests::the_history_chords_undo_and_redo_the_typing` -- each declared redo chord brings the undone typing back | none | verified |

### 1.2 Undo or redo the last action

One gesture with two directions and one set of rules. The five steps are the same whichever way the
history moves, so the two share a table rather than repeating one.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UNDO-001` | `POST` | An undo MUST revert this peer's own last operation. | this spec section 2, row 1; architecture section 2.D | Trame `ambre_trame::logic::tests::group2::undo_local_only_not_remote` -- on the half that reads the peer's own edit back as reverted; Toile `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`; Prose `ambre_prose::logic::blocks::tests::an_undone_deletion_brings_the_block_back_whole` | none | verified |
| `UNDO-007` | `FRAME` | An undo MUST NOT revert a remote peer's operation. | this spec section 2, row 1; architecture section 2.D | Trame `ambre_trame::logic::tests::group2::undo_local_only_not_remote` -- on the half that reads the remote value back unchanged; Toile `ambre_toile::logic::convergence_tests::an_undo_leaves_a_remote_move_alone`; Prose `ambre_prose::logic::tests::an_undo_leaves_a_remote_edit_alone` | none | verified |
| `UNDO-002` | `POST` | An undo MUST broadcast its inverse operations as an ordinary CRDT delta, like any other change. | this spec section 2, row 2 | Trame `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`; Prose `ambre_prose::logic::xrefs::tests::a_target_an_undo_restores_resolves_again_on_the_other_peer` -- the undo's delta applied on a second peer; Toile `ambre_toile::logic::convergence_tests::a_history_step_reaches_the_other_peer_as_a_delta` -- the undo and the redo deltas applied on a second board | none | verified |
| `UNDO-005` | `POST` | An undo MUST rebuild every derived cache whole, so nothing derived is stale afterwards. | this spec section 3; `4-Prose/features/ambre-spec-prose-undo.md` (`PROSE-UI-045`), the caret that stands in for a derived cache Prose has not got | Trame `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index`; Toile `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`; `::undo_reverts_group_as_one_item`; Prose `ambre_prose::logic::xrefs::tests::a_target_an_undo_restores_resolves_again_on_the_other_peer` -- the cross-reference numbering resolves again | none | verified |
| `UNDO-006` | `POST` | The undo and redo affordances MUST report what the engine holds. | this spec section 4 | Trame `ambre_trame::logic::tests::group2::mutating_command_reports_undo_state`; chrome `ambre_trame::tests::chrome_ui::undo_state_mirrors_into_command_store`; Toile `ambre_toile::logic::state::undo_state_tests::mutating_command_reports_undo_state`; chrome `ambre_toile::tests::chrome_ui::undo_state_mirrors_into_command_store`; Prose `ambre_prose::logic::tests::a_write_reports_the_history_and_a_no_op_reports_none`; chrome `ambre_prose::tests::chrome_panel::the_history_rows_follow_the_reported_state` -- the context menu's two rows follow each reported state, and another event leaves them | none | verified |
| `UNDO-003` | `FAIL` | An undo with nothing to revert MUST leave the document unchanged. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop` -- the document's version vector is unchanged; Toile `ambre_toile::logic::convergence_tests::an_undo_with_nothing_to_revert_writes_nothing`; Prose `ambre_prose::logic::tests::an_undo_with_nothing_to_revert_writes_nothing` | none | verified |
| `UNDO-008` | `FAIL` | An undo with nothing to revert MUST emit no delta. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop` -- on the assertion that the handler produced no update at all; Toile `ambre_toile::logic::convergence_tests::an_undo_with_nothing_to_revert_writes_nothing`; Prose `ambre_prose::logic::tests::an_undo_with_nothing_to_revert_writes_nothing` | none | verified |

* **Budget.** none of its own: an undo replays one item through the command path, so it lands under
  the command rows of `ambre-spec-budgets.md section 1`. What the stack holds sits inside the
  `LoroDoc` holder of `ambre-spec-budgets.md section 6`, and the bound that would cap it is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 11 `verified`. In all three applications, an undo reverts the peer's own edit,
  reaches the other peers as a delta, and reports the availability the affordances show. The
  history dies with the document, and the reseed is clean. An undo leaves a remote peer's
  operation alone, and an undo on an empty stack writes and emits nothing.

---

## 2. WHAT EACH USER SEES

Two peers, one document. The behaviour is the same whichever editor draws it; the last column names
the test that holds it today.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| makes a change | makes his own, then undoes | Alice's change, and Bob's reverted | Bob's own change gone, Alice's untouched (`UNDO-001`) | Trame `logic::tests::group2::undo_local_only_not_remote` |
| makes a change, then undoes | watches | the change reverted | it gone on both screens, from a delta like any other (`UNDO-002`) | Trame `logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`; Toile `logic::convergence_tests::undo_reverts_create_and_redo_restores` |
| undoes | undoes at the same moment | each peer's own last action reverted | two independent reverts, and one converged document | none -- the convergence property carries no undo, section 6 |

Each application adds the scenarios that are its own -- a five-row delete, a frame membership, a
replace-all -- in its own undo document.

---

## 3. MODEL

**Containers.** None of its own. Undo reverts operations over the containers the other features own,
which is why one undo item is one commit rather than one container write.

**Commands and events.** Each application names its own pair (`ambre-bus`); the shape is one.

| Shape | Direction | Carries |
|---|---|---|
| `<App>Command::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `<App>Event::UndoState { can_undo, can_redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

**Engine state.** One field of each application's engine, the same shape everywhere:

```rust
/// Local-only history, built over the document when it opens and dropped when it
/// closes. It observes the `LoroDoc` itself: nothing is registered by hand.
undo_manager: loro::UndoManager,
```

**Steps, when a user undoes or redoes.** The same five everywhere; step 3 is the application's own.

1. ask the history to revert or replay one item, and read whether anything moved;
2. stop here when nothing moved, and answer with the availability alone (`UNDO-003`,
   `UNDO-008`);
3. reseed every derived cache whole from the document, not the set the item touched (`UNDO-005`);
4. flush the inverse operations as a delta the other users receive (`UNDO-002`);
5. answer with the new availability (`UNDO-006`).

Step 3 is a total reseed. That is why nothing derived is stale after an undo: no index, no span, no
paint order, no compiled formula, whichever application ran it.

**What one undo item is, is per application.** The principle is one commit per mutating command, with
a continuous gesture the exception that seals once at the end. Which gestures collapse into one item
is each application's rule (`2-Trame/features/ambre-spec-trame-undo.md`, `TRAME-017`;
`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-014`).

**Code.** none of its own -- each application's handler reverts and reseeds in its own engine, cited
from its undo document.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` |
| `Ctrl+Y` | `Redo`, the second binding users of other applications reach for |
| the two edit-menu or toolbar entries | the same two intents |

**The two affordances** are disabled from `UndoState` alone: the chrome holds no history of its own
and counts nothing (`UNDO-006`). A command that writes nothing sends no refresh, so a selection move
never re-enables a button. Which keys carry these intents is `ambre-spec-commands.md`.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Undo reverts the local peer alone | a global history, where undo walks the document's operations whoever wrote them | it lets one user erase another's work without touching their document, and no collaborative editor does it. A user pressing `Ctrl+Z` means "take back what I did" |
| An undo is an ordinary change, broadcast like any other | rolling the document back to a stored version | two peers cannot agree on it. Inverse operations are operations, so they converge by the one path everything else uses, and a peer that was offline receives them in order |
| The caches are reseeded whole after an undo | rebuilding only what the reverted item touched | one item can move an element, restore a span and change a derived value at once, so what needs refreshing is the whole derived state. A partial refresh is a list that goes stale |
| The history dies with the document | persisting the stack | it would offer to revert operations the user cannot see: the stack lives in memory beside a `LoroDoc` that is itself rebuilt from a snapshot on open, and the operations it holds are a previous session's. What a user wants across sessions is a named version, not a stack |
| The chrome is told what is available, and never counts | the chrome tracking its own depth | a chrome-side counter drifts the moment a command merges two items. One source for the two buttons, and the Logic thread is the only place that knows |
| Both redo chords are bound | `Ctrl+Y` alone, which one earlier design carried | the two chords are split across the applications a user comes from, so binding one teaches nothing to the half that reaches for the other |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The depth is unbounded** (`UI-THREAD-016`). A long session grows the history until the tab runs
  out of memory, and nothing measures it. The bound is one number shared by every application, which
  is why the row is `ambre-spec-ui.md`'s; what merges into one item stays each application's.
* **Undo is in no convergence property.** The two-peer proptest drives every mutating command and
  never an undo, in Trame or in Toile. Two peers undoing into each other has no coverage anywhere.
* **Version history is designed and unbuilt.** The stack dies with the session, and the answer across
  sessions is a named version, not a longer stack. Prose carries a design and Trame carries none, so
  the shape is open -- per document or per application, a write or a fork. It is the
  `The later writing set` Story (`ambre-roadmap.md` POST-V1), and it stays each application's
  until then.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What one undo item is, per application | `2-Trame/features/ambre-spec-trame-undo.md`, `TRAME-017`; `3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-014` |
| Which derived caches an undo reseeds | each application's undo document, over the caches its engine holds |
| Where the caret lands after an undo | `4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-UI-045` |
| The one bounded undo depth | `ambre-spec-ui.md`, `UI-THREAD-016` |
| Which chords mean undo and redo | `ambre-spec-commands.md` |
| A named version restored after the session | each application's undo document; the `The later writing set` Story (`ambre-roadmap.md` POST-V1) |
