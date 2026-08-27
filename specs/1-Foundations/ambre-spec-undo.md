# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Horizontal (framework)
**Implementation:** partial -- Trame and Toile ship an undo stack over the shared model; Prose is
design-only, and the depth is unbounded in all three.

> A user presses `Ctrl+Z` and their last action comes back. A collaborator's action never does, and
> never disappears from under them.

Undo is local to the peer that acted: not a rewind of the document, but one more change to it, sent
to the others like any other. This document owns the model; each application owns what merges into
one step and which caches it rebuilds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this feature realizes
is normative in `ambre-spec-message-bus.md`. The prefix is `UNDO`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `UNDO-001` | Undo MUST revert local operations only, never a remote peer's. | this spec section 2, row 1; architecture section 2.D | Trame `ambre_trame::logic::tests::group2::undo_local_only_not_remote`; Toile covers the local half alone (`ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`) and drives no remote change; Prose design-only | unverified |
| `UNDO-002` | An undo MUST broadcast its inverse operations as an ordinary CRDT delta, like any other change. | this spec section 2, row 2 | Trame `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`; Toile emits through the shared emitter by construction and no test reads it back; Prose design-only | unverified |
| `UNDO-003` | An undo with nothing to revert MUST leave the document unchanged and MUST emit no delta. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop`; Toile returns early on an unchanged history and no test drives it; Prose design-only | unverified |
| `UNDO-004` | The history MUST NOT survive the document being closed. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_stack_cleared_on_doc_close`; Toile `ambre_logic::dispatch::tests::close_document_drops_state_and_forwards` | verified |
| `UNDO-005` | An undo MUST rebuild every derived cache whole, so nothing derived is stale afterwards. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index`; Toile `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`, `::undo_reverts_group_as_one_item`; Prose has no derived cache, so its analog is the caret (`4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-UI-045`) | verified |
| `UNDO-006` | The undo and redo affordances MUST report what the engine holds, and a command that changes nothing MUST NOT refresh them. | this spec section 4 | Trame `ambre_trame::logic::tests::group2::mutating_command_reports_undo_state`, chrome `ambre_trame::tests::chrome_ui::undo_state_mirrors_into_command_store`; Toile **fails today**: two handlers of thirty emit the event and the view discards it -- section 6 | unverified |

* **Budget.** none of its own: an undo replays one item through the command path, so it lands under
  the command rows of `ambre-spec-budgets.md section 1`. What the stack holds sits inside the
  `LoroDoc` holder of `ambre-spec-budgets.md section 6`, and the bound that would cap it is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 2 `verified`, 4 `unverified`, 0 `unimplemented`. The rule that holds across both
  building applications is that the history dies with the document; the reseed holds too. What is
  loose is whose operations revert, what an undo emits, and what the affordances are told -- held in
  Trame and open in Toile. Prose is design-only and adds no code to any row.

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
2. stop here when nothing moved, and answer with the availability alone (`UNDO-003`);
3. reseed every derived cache whole from the document, not the set the item touched (`UNDO-005`);
4. flush the inverse operations as a delta the other users receive (`UNDO-002`);
5. answer with the new availability (`UNDO-006`).

Step 3 is a total reseed. That is why nothing derived -- an index, a span, a paint order, a compiled
formula -- is stale after an undo, whichever application ran it.

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Undo reverts the local peer alone | a user pressing `Ctrl+Z` means "take back what I did". Reverting a collaborator's work would let one user erase another's without touching their document, and no collaborative editor does it | a global history, where undo walks the document's operations whoever wrote them |
| An undo is an ordinary change, broadcast like any other | the inverse operations are operations, so they converge by the one path everything else uses, and a peer that was offline receives them in order | rolling the document back to a stored version, which two peers cannot agree on |
| The caches are reseeded whole after an undo | one item can move an element, restore a span and change a derived value at once, so what needs refreshing is the whole derived state. A partial refresh is a list that goes stale | rebuilding only what the reverted item touched |
| The history dies with the document | it lives in memory beside a `LoroDoc` that is itself rebuilt from a snapshot on open, and the operations it would revert are a previous session's. What a user wants across sessions is a named version, not a stack | persisting the stack, which would offer to revert operations the user cannot see |
| The chrome is told what is available, and never counts | one source for the two buttons, and the Logic thread is the only place that knows. A chrome-side counter would drift the moment a command merged two items | the chrome tracking its own depth |
| Both redo chords are bound | the two are split across the applications a user comes from, and binding one teaches nothing to the half that reaches for the other | `Ctrl+Y` alone, which one earlier design carried |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The depth is unbounded** (`UI-THREAD-016`). A long session grows the history until the tab runs
  out of memory, and nothing measures it. The bound is one number shared by every application, which
  is why the row is `ambre-spec-ui.md`'s; what merges into one item stays each application's.
* **Undo is in no convergence property.** The two-peer proptest drives every mutating command and
  never an undo, in Trame or in Toile, so two peers undoing into each other has no coverage anywhere.
* **Toile's affordances are never told anything** (`UNDO-006`). The availability event is emitted by
  two handlers and discarded by the view, so a fresh board always offers Undo. Trame's counterpart is
  held on two tests; the transposition is the cheapest gap in Toile to close
  (`3-Toile/features/ambre-spec-toile-undo.md section 4`).
* **Version history is designed and unbuilt.** The stack dies with the session, and the answer across
  sessions is a named version, not a longer stack. Prose carries a design and Trame carries none, so
  the shape is open -- per document or per application, a write or a fork. It is the "The later
  writing set" Story (`ambre-roadmap.md`), and it stays each application's until then.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| What one undo item is, per application | `2-Trame/features/ambre-spec-trame-undo.md`, `TRAME-017`; `3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-014` |
| Which derived caches an undo reseeds | each application's undo document, over the caches its engine holds |
| Where the caret lands after an undo | `4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-UI-045` |
| The one bounded undo depth | `ambre-spec-ui.md`, `UI-THREAD-016` |
| Which chords mean undo and redo | `ambre-spec-commands.md` |
| A named version restored after the session | each application's undo document; the "The later writing set" Story |
