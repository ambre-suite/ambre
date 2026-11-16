# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile

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
| `UNDO-014` | `INV` | Every hash table of the history MUST hash under a key drawn once per process from the platform entropy source, so a peer cannot choose container ids that collide. | `ambre-spec-architecture.md section 2`, the hash key | lint-wall -- `crates/ambre-crdt/clippy.toml` refuses the std `HashMap` and `HashSet`, so every table names `ambre_core::hash`, whose hasher draws its key through `getrandom`; `ambre_core::hash::tests::the_key_is_the_bytes_the_entropy_source_gives`; `::the_key_of_the_process_is_not_the_fixed_key` | none | verified |
| `UNDO-019` | `INV` | A step MUST NOT keep a copy of the characters a text edit put in. | this spec section 3; `ambre-spec-budgets.md section 6` | structural -- the op of an insertion holds its two cursors, its length and its digest, and no text | none | verified |
| `UNDO-009` | `INV` | A command that changes nothing MUST NOT refresh the undo affordances. | this spec section 4 | Trame `ambre_trame::logic::tests::group2::a_command_that_changes_nothing_reports_no_undo_state`; Toile `ambre_toile::logic::state::undo_state_tests::a_command_that_changes_nothing_reports_no_undo_state`; Prose `ambre_prose::logic::tests::a_write_reports_the_history_and_a_no_op_reports_none` -- each drives a redo on an empty stack and a command that writes nothing, and reads that no availability went out | none | verified |
| `UNDO-010` | `INV` | `Ctrl+Z` MUST undo. | this spec section 4 | Trame `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- it seeds the keymap from the declared table and reads the chords back, so a removed `Ctrl+Z` binding fails it; Toile `ambre_toile::ui::view::view_tests::the_undo_chords_reach_the_logic_thread` -- the key event through the board view sends undo; Prose `ambre_prose::input_tests::the_history_chords_undo_and_redo_the_typing` -- each declared undo chord through the page's key decoder reverts the typing; Prose `ambre_belt::prose::tests::the_history_chords_undo_and_redo_in_the_browser` -- browser tier: `Ctrl+Z` takes the last key back, read off the count and the export | none | verified |
| `UNDO-011` | `INV` | `Ctrl+Shift+Z` MUST redo. | this spec section 4 | Trame `ambre_trame::ui::commands::tests::the_keymap_binds_both_redo_chords` -- a removed `Ctrl+Shift+Z` binding fails it; Toile `ambre_toile::ui::view::view_tests::the_undo_chords_reach_the_logic_thread` -- the key event through the board view sends redo; `ambre_toile::ui::commands::tests::redo_announces_both_chords` -- the redo entry binds the shared chord pair and announces `Ctrl+Shift+Z`; Prose `ambre_prose::input_tests::the_history_chords_undo_and_redo_the_typing` -- each declared redo chord brings the undone typing back; Prose `ambre_belt::prose::tests::the_history_chords_undo_and_redo_in_the_browser` -- browser tier: `Ctrl+Shift+Z` brings the key back, read off the count | none | verified |

### 1.2 Undo or redo the last action

One gesture with two directions and one set of rules. The five steps are the same whichever way the
history moves, so the two share a table rather than repeating one.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `UNDO-001` | `POST` | An undo MUST revert this peer's own last operation. | this spec section 2, row 1; architecture section 2.D | Trame `ambre_trame::logic::tests::group2::undo_local_only_not_remote` -- on the half that reads the peer's own edit back as reverted; Toile `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`; Prose `ambre_prose::logic::blocks::tests::an_undone_deletion_brings_the_block_back_whole` | none | verified |
| `UNDO-007` | `FRAME` | An undo MUST NOT revert a remote peer's operation. | this spec section 2, row 1; architecture section 2.D | Trame `ambre_trame::logic::tests::group2::undo_local_only_not_remote` -- on the half that reads the remote value back unchanged; `ambre_trame::logic::history_tests::an_undone_write_keeps_a_later_write_of_a_peer` -- the peer types over the cell the undone write wrote; `ambre_crdt::history::tests::an_undo_keeps_a_later_write_of_a_peer` -- a key a peer wrote after the edit keeps its value; `ambre_crdt::history::tests::an_undo_of_typing_keeps_a_peer_s_characters` -- a peer types on each side of the typing the undo takes out; Toile `ambre_toile::logic::convergence_tests::an_undo_leaves_a_remote_move_alone`; Prose `ambre_prose::logic::tests::an_undo_leaves_a_remote_edit_alone` | none | verified |
| `UNDO-012` | `FRAME` | An undo MUST NOT bring back a container that a peer deleted. | this spec section 3 | `ambre_crdt::history::tests::an_undo_leaves_a_map_a_peer_deleted_deleted` -- a peer deletes the nested map a move wrote into, and the undo of the move writes nothing; `ambre_toile::logic::convergence_tests::an_undo_leaves_an_element_a_peer_deleted_deleted` | none | verified |
| `UNDO-017` | `POST` | A commit that puts a container into a list the same commit created MUST record one step, whose undo takes the list out with every element. | this spec section 3 | `ambre_crdt::history::tests::a_map_pushed_into_a_list_its_commit_created_is_one_step` -- a map, its list and a map pushed into that list in one commit: one undo takes all three out, and one redo writes them back with their keys; `ambre_crdt::history::tests::a_map_pushed_into_an_older_list_clears_the_history` -- the frame: a list an earlier commit created still ends the history | none | verified |
| `UNDO-013` | `FRAME` | An undo or a redo of a format MUST leave a mark key unchanged on each character where a peer changed that key since. | this spec section 3 | `ambre_crdt::history::format_tests::an_undone_format_keeps_a_peer_s_format` -- a peer sets another weight inside the bolded range and a colour over it, and the undo and the redo keep both; `ambre_crdt::history::format_tests::a_format_undo_writes_only_where_no_peer_formatted_since` -- a property: random formats of two peers, the undo and the redo read against a model taken character by character | none | verified |
| `UNDO-015` | `POST` | An undo or a redo of a format MUST reach each character the step formatted, across the characters a peer typed among them since. | this spec section 3 | `ambre_crdt::history::format_tests::an_undone_format_reaches_across_a_peer_s_typing` -- a peer types inside the bolded word, and the undo and the redo write each of the user's characters; `ambre_crdt::history::format_tests::a_format_undo_across_a_peer_s_typing_writes_by_identity` -- a property: a peer types and formats before or after the step, and the undo and the redo read against a model taken by character identity; `ambre_crdt::history::format_tests::an_undone_format_reaches_a_character_its_own_undo_put_back` | none | verified |
| `UNDO-016` | `FRAME` | An undo or a redo of a format MUST NOT write a character a peer typed since the step. | this spec section 3 | `ambre_crdt::history::format_tests::an_undone_format_reaches_across_a_peer_s_typing` -- the peer's two characters keep the weight they took; `ambre_crdt::history::format_tests::a_format_undo_across_a_peer_s_typing_writes_by_identity` -- the model leaves each character the step did not hold | none | verified |
| `UNDO-020` | `POST` | A redo of typing MUST put the characters back with the marks they carried when the undo took them out. | this spec section 3 | `ambre_crdt::history::tests::a_redo_of_typing_keeps_a_mark_a_peer_set` -- a peer marks the typed characters, and the redo brings them back marked | none | verified |
| `UNDO-018` | `FRAME` | An undo of typing MUST NOT take out its characters when a peer replaced one of them since, even with both ends standing. | this spec section 3 | `ambre_crdt::history::tests::an_undo_of_typing_leaves_what_a_peer_replaced_inside` -- a peer swaps the middle character, and the undo writes nothing | none | verified |
| `UNDO-002` | `POST` | An undo MUST broadcast its inverse operations as an ordinary CRDT delta, like any other change. | this spec section 2, row 2 | Trame `ambre_trame::logic::tests::group1::undo_then_redo_round_trips_a_cell_edit`; Prose `ambre_prose::logic::xrefs::tests::a_target_an_undo_restores_resolves_again_on_the_other_peer` -- the undo's delta applied on a second peer; Toile `ambre_toile::logic::convergence_tests::a_history_step_reaches_the_other_peer_as_a_delta` -- the undo and the redo deltas applied on a second board | none | verified |
| `UNDO-005` | `POST` | After an undo, every derived cache MUST equal what a whole reseed from the document gives. | this spec section 3; `4-Prose/features/ambre-spec-prose-undo.md` (`PROSE-UI-045`), the caret that stands in for a derived cache Prose has not got | Trame `ambre_trame::logic::tests::group2::undo_insert_row_rebuilds_sheet_index`; Toile `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores`; `::undo_reverts_group_as_one_item`; `::an_undo_refresh_matches_a_whole_reseed` -- a proptest, two peers, edits, deltas, undos and redos, each cache against a whole reseed after every step; Prose `ambre_prose::logic::xrefs::tests::a_target_an_undo_restores_resolves_again_on_the_other_peer` -- the cross-reference numbering resolves again | none | verified |
| `UNDO-006` | `POST` | The undo and redo affordances MUST report what the engine holds. | this spec section 4 | Trame `ambre_trame::logic::tests::group2::mutating_command_reports_undo_state`; chrome `ambre_trame::tests::chrome_ui::undo_state_mirrors_into_command_store`; Toile `ambre_toile::logic::state::undo_state_tests::mutating_command_reports_undo_state`; chrome `ambre_toile::tests::chrome_ui::undo_state_mirrors_into_command_store`; Prose `ambre_prose::logic::tests::a_write_reports_the_history_and_a_no_op_reports_none`; chrome `ambre_prose::tests::chrome_panel::the_history_rows_follow_the_reported_state` -- the context menu's two rows follow each reported state, and another event leaves them | none | verified |
| `UNDO-003` | `FAIL` | An undo with nothing to revert MUST leave the document unchanged. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop` -- the document's version vector is unchanged; Toile `ambre_toile::logic::convergence_tests::an_undo_with_nothing_to_revert_writes_nothing`; Prose `ambre_prose::logic::tests::an_undo_with_nothing_to_revert_writes_nothing` | none | verified |
| `UNDO-008` | `FAIL` | An undo with nothing to revert MUST emit no delta. | this spec section 3 | Trame `ambre_trame::logic::tests::group2::undo_with_empty_stack_is_a_noop` -- on the assertion that the handler produced no update at all; Toile `ambre_toile::logic::convergence_tests::an_undo_with_nothing_to_revert_writes_nothing`; Prose `ambre_prose::logic::tests::an_undo_with_nothing_to_revert_writes_nothing` | none | verified |

* **Budget.** none of its own: an undo replays one item through the command path, so it lands under
  the command rows of `ambre-spec-budgets.md section 1`. What the stack holds sits inside the
  `LoroDoc` holder of `ambre-spec-budgets.md section 6`, and the bound that would cap it is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo emits the reverting operations into the document, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 20 `verified`. In every application, an undo reverts the peer's own edit,
  reaches the other peers as a delta, and reports what the affordances show. The history dies with
  the document, and every cache equals a whole reseed. An undo leaves a peer's operation, container,
  mark and typing alone. An undo on an empty stack writes and emits nothing.

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
| `<App>Event::UndoState { undo, redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

**One model: each edit records its own inverse.**

* A subscription keeps a plain copy of every container, nested ones included: the mirror.
* Each local commit reads from the mirror what it overwrites. That is the step:
  * the value of each key; a nested container a commit deletes is kept by value;
  * the place and neighbour of each list element;
  * the characters each text edit took out, placed by Loro cursors;
  * for the characters a text edit put in, their cursors, their count and a digest, never a copy
    (`UNDO-019`). The text holds them. An undo reads them back with their marks before it takes
    them out, and a redo writes them with those marks (`UNDO-020`);
  * the parent and sibling of each tree node a commit moved, and each node it created.
* An undo writes each value back, puts each element back after its old neighbour, and takes out
  the characters this peer put in. A redo writes the edit again.
* A nested container comes back by value, under a new Loro identity. The history notes the old
  identity against the new one, so a later step reaches it.
* An undo of a tree node's creation hides the node under the application's marker. Loro raises no
  deleted node, so the redo shows the same node.
* A key a later write changed keeps that write, and a character a peer typed stays (`UNDO-007`).
  An undo of typing compares the characters between its two cursors with its digest first. A
  peer that replaced one of them leaves the step unable to take them out: the replay writes nothing
  (`UNDO-018`). The digest is keyed like the hash tables of the history (`UNDO-014`).
* A format is undone per mark key and per character (`UNDO-013`). A character keeps a key that no
  longer holds the value the step left. The others take the recorded value back, one run per stretch.
  A redo reads the recorded value the same way.
* A format step finds its characters by identity (`UNDO-015`, `UNDO-016`). It keeps the cursors on
  its two ends and the version vector its commit left. Between the ends, a peer typed each character
  that version does not hold. The replay skips those and writes the rest stretch by stretch. This
  peer's own characters count as the step's, because its own undo of a deletion puts them back
  under new identities.
* A peer that took a character out of the formatted range leaves the format step unable to place
  its recorded values: the replay writes nothing.
* The test compares values, as the map side does. A peer that wrote the very value the step left
  cannot be told apart from no write: the undo takes that value back.
* A container a peer deleted or replaced takes no write: an undo never raises it (`UNDO-012`).
* A commit that deletes a tree node ends the history: no step can replay it. So does a commit
  that puts a container into a list an earlier commit created.
* A container put into a list that the same commit created rides the creation of that list
  (`UNDO-017`). The undo takes the list out whole, and the redo creates it again by value.
* The registry entry and a replay are no step.

**Engine state.** One field of each application's engine, the same type everywhere:

```rust
/// Local-only history, built over the document when it opens and dropped when it
/// closes. It observes the `LoroDoc` itself: nothing is registered by hand.
history: ambre_crdt::history::UndoHistory,
```

The Loro undo manager is rejected (section 5).

* Trame holds it over its root maps and lists alone: a write into a nested container ends its
  history (`2-Trame/features/ambre-spec-trame-undo.md`, `TRAME-288`, `TRAME-290`).
* Toile holds it (`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-290`).
* Prose holds it, with the hiding marker of a deleted block (`4-Prose/features/ambre-spec-prose-undo.md`,
  `PROSE-134`).

**Steps, when a user undoes or redoes.** The same five everywhere; step 3 is the application's own.

1. ask the history to revert or replay one item, and read whether anything moved;
2. stop here when nothing moved, and answer with the availability alone (`UNDO-003`,
   `UNDO-008`);
3. refresh the derived caches, so each equals a whole reseed from the document (`UNDO-005`);
4. flush the inverse operations as a delta the other users receive (`UNDO-002`);
5. answer with the new availability (`UNDO-006`).

Step 3 is the application's choice: a whole reseed, or a refresh of what the step wrote.
Trame reseeds whole, and Prose holds no derived cache beyond the caret. Toile refreshes the
elements the step's own commit names, as for a remote change
(`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-292`). Either way, a test compares each
cache with a whole reseed, so no index, span, paint order or compiled formula is stale.

**What one undo item is, is per application.** The principle is one commit per mutating command, with
a continuous gesture the exception that seals once at the end. Which gestures collapse into one item
is each application's rule (`2-Trame/features/ambre-spec-trame-undo.md`, `TRAME-017`;
`3-Toile/features/ambre-spec-toile-undo.md`, `TOILE-014`).

**A deletion an undo reverses hides what it deletes.** Prose hides a block, and Trame a block of
lines. Only the session that hid it can undo it, so a sweep removes it once that session has ended
(`4-Prose/ambre-spec-prose-schema.md`, `PROSE-121`;
`2-Trame/features/ambre-spec-trame-rows-and-columns.md`, `TRAME-305`). One function of the suite
answers whether a session runs: its own, or one whose user has a session open here. The sweep is
an ordinary delta, never a step of the history, and a viewer never sweeps.

**Code.** `crates/ambre-crdt/src/history.rs` -- the mirror, the step, its replay and the history.
`crates/ambre-crdt/src/session.rs` -- whether the session that hid a part still runs.
Each application's handler seals a step per command, reverts and refreshes in its own engine, cited
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
| Each cache equals a whole reseed after an undo, and an application may refresh only what the step wrote | requiring a whole reseed after every undo | on a board of 2 000 stickies the undo of a move cost about 60 million instructions on 2026-11-08, 98 % of it in the reseed, against a budget of 800 000. The stale list a partial refresh risks is held by a property against the whole reseed (`UNDO-005`) |
| The history dies with the document | persisting the stack | it would offer to revert operations the user cannot see: the stack lives in memory beside a `LoroDoc` that is itself rebuilt from a snapshot on open, and the operations it holds are a previous session's. What a user wants across sessions is a named version, not a stack |
| A format undo compares each character's value of each key | a format undo that writes the recorded marks back over the whole range | it erases every bold, colour or weight a peer set inside the range since, which `UNDO-007` forbids for a key or a character. Loro exposes no author per mark, so the value is the one witness a replay can read |
| A format step finds its characters by the version vector its commit left | a format replay over the range between its two ends, still contiguous | a character a peer typed inside the range breaks the count, so the undo writes nothing and the user sees an undo that does not work |
| A format step finds its characters by the version vector its commit left | a cursor per formatted character, taken when the commit records | the commit of a format pays one cursor per character, on every format a user makes. The version vector costs one entry per peer at record time, and the walk runs on an undo alone |
| Each edit records its own inverse, in one history type for the suite | the Loro undo manager | it answers an undo by a checkout, which rebuilds every map the item wrote: about 250 million instructions for the undo of one cell write on a sheet of 10k formulas. It also restores a value over a peer's later write, which breaks `UNDO-007`, and it mints a new identity for each node it creates again (`4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-134`) |
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
| Which derived caches an undo refreshes, and how | each application's undo document, over the caches its engine holds |
| Where the caret lands after an undo | `4-Prose/features/ambre-spec-prose-undo.md`, `PROSE-UI-045` |
| The one bounded undo depth | `ambre-spec-ui.md`, `UI-THREAD-016` |
| Which chords mean undo and redo | `ambre-spec-commands.md` |
| A named version restored after the session | each application's undo document; the `The later writing set` Story (`ambre-roadmap.md` POST-V1) |
