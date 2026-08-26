# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship; the buttons are never told what is available.

> A user presses `Ctrl+Z` and their last action comes back. A collaborator's action never does, and
> never disappears from under them.

Undo is local to the peer that acted. It is not a rewind of the board: it is one more change to it,
sent to the other users like any other.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The cross-app pattern this feature realizes
is normative in `ambre-spec-message-bus.md`, and Trame's half of it is
`2-Trame/features/ambre-spec-trame-undo.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-014` | Each mutating command MUST end with one commit, so one atomic user action is one undo item. | this spec section 3; architecture section 2.D | `ambre_toile::logic::state::elements::inner_tests::many_inner_creates_share_one_undo_item`, `::create_restyle_and_group_are_one_undo_item`, `ambre_toile::logic::convergence_tests::set_frame_children_is_one_undo_item`, `::duplicate_a_group_is_one_undo_item`, `::undo_reverts_group_as_one_item` | verified |
| `TOILE-015` | Undo MUST revert local operations only, never a remote peer's. | this spec section 2, row 1; architecture section 2.D | none. `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores` covers the local half; **no test drives a remote change and asserts undo leaves it alone** -- the Trame counterpart (`undo_local_only_not_remote`) exists and has none here | unverified |
| `TOILE-072` | An undo MUST rebuild every derived cache, so nothing derived is stale afterwards. | this spec section 3 | `ambre_toile::logic::convergence_tests::undo_reverts_create_and_redo_restores` asserts the bounds cache follows the revert and the replay; `::undo_reverts_group_as_one_item` asserts the paint order does | verified |
| `TOILE-073` | An undo MUST broadcast its reverting operations, like any other change to the board. | this spec section 3 | none. The handler returns its events through the same emitter every mutating command uses, so a delta is produced by construction, and no test reads it back | unverified |
| `TOILE-074` | An undo with nothing to revert MUST leave the board unchanged and MUST emit no delta. | this spec section 3 | none. The handler returns early on an unchanged history, before the cache rebuild and before the emitter, and no test drives an empty history | unverified |
| `TOILE-075` | The undo and redo affordances MUST report what is available. | this spec section 4 | none, and **the rule fails today**: two handlers out of thirty emit the availability event, and the view discards it, so the two menu entries are always offered -- section 6 | unverified |
| `TOILE-076` | The history MUST NOT survive the board being closed. | this spec section 3 | `ambre_logic::dispatch::tests::close_document_drops_state_and_forwards`: closing drops the engine, and the history is a field of it | verified |

* **Budget.** none of its own: an undo replays one item and then reseeds every derived cache from the
  document, so it costs a full rebuild rather than a command. What the history holds sits inside the
  document holder of `ambre-spec-budgets.md section 1`, and the bound that would cap it is
  `UI-THREAD-016`, one number for every application.
* **Boundary.** none of its own: an undo writes reverting operations into the document, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 3 `verified`, 4 `unverified`, 0 `unimplemented`. What one undo item is, is held
  five ways -- the granularity rule is the best-covered claim in the application. Everything around it
  is loose: whose operations revert, what an undo emits, and what the buttons are told.

---

## 2. WHAT EACH USER SEES

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| moves a note | creates a shape, then undoes | Alice's move, and no shape | Bob's own shape gone, Alice's move untouched (`TOILE-015`) | none -- section 6 |
| creates a shape, then undoes | watches | the board as it was | the shape gone on both screens, from a delta like any other (`TOILE-073`) | `logic::convergence_tests::undo_reverts_create_and_redo_restores` |
| groups two shapes, then undoes | watches | the two shapes, unbound, in their order | the binding gone and the shapes intact (`TOILE-014`) | `logic::convergence_tests::undo_reverts_group_as_one_item` |
| undoes | undoes at the same moment | each peer's own last action reverted | two independent reverts, and one converged board | none -- the convergence property carries no undo, section 6 |
| adds a shape to a frame that painted above it, then undoes | watches | the membership gone and the reordering still applied | half the action reverted (`TOILE-048`) | `features/ambre-spec-toile-frames.md section 6` |

---

## 3. MODEL

**Containers.** None of its own. Undo reverts operations over the containers the other features own,
which is why one undo item is one commit rather than one container write.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `ToileEvent::UndoState { can_undo, can_redo }` | Logic to UI | what the two affordances may offer (`TOILE-075`) |

**Engine state.** One field of the board engine:

```rust
/// Local-only history, built over the document when the board opens and dropped
/// with it. It observes the document itself: nothing is registered by hand.
undo: loro::UndoManager,
```

**Steps, when a user undoes or redoes:**

1. ask the history to revert or replay one item, and read whether anything moved;
2. stop there when nothing moved, answering with the availability alone (`TOILE-074`);
3. reseed every derived cache -- the bounds map, the spatial index, the paint order and the group
   boxes -- from the document (`TOILE-072`);
4. answer with a fresh visible set, the availability, and the delta the other users receive
   (`TOILE-073`).

Step 3 is a total reseed, not a rebuild of what the item touched. That is why no group box, no index
entry and no paint order is stale after an undo.

**What one undo item is:**

| Gesture | Items |
|---|---|
| placing an element | one |
| a move, resize or rotate drag | one, sealed on release |
| a restyle over a whole selection | one |
| a delete over a whole selection | one |
| a freehand stroke, however many samples | one |
| a group, an ungroup, a paste, a duplicate | one each (`TOILE-070`) |
| a whole file import, however many elements | one (`features/ambre-spec-toile-import-export.md`) |
| a body edit, however many keystrokes | one, sealed when the editor closes |
| a membership change that reorders a child | two, and the rule says one (`TOILE-048`) |
| ungrouping a selection of three groups | three, and the rule says one per group (`features/ambre-spec-toile-groups.md section 6`) |
| a copy | none: it writes nothing (`TOILE-067`) |
| a camera change, a tool change, a presence frame, playback | none: they write nothing |

**Why one commit per command.** The history groups everything since the last commit into one item, so
two commands without a commit between them collapse into one undo step. Continuous gestures are the
deliberate exception: a drag and a body edit stream without committing and commit once at the end, so
the whole gesture is one item.

**Code:**

* `crates/ambre-toile/src/logic/state/mod.rs` -- the revert and replay handler, and the availability
  event;
* `logic/state/caches.rs` -- the reseed step 3 runs;
* `ui/menu_bar.rs` -- the two entries and their chords.

---

## 4. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` |
| `Ctrl+Y` | `Redo`, the second binding users of other applications reach for |
| Undo and Redo, in the edit menu | the same two intents |

**What the two entries show.** Always the same thing: they are never disabled, because the
availability never reaches them (`TOILE-075`). A user on a fresh board can press either and nothing
happens.

**What the user sees after an undo.** The whole visible set is re-derived, because the reseed is total:
geometry, style, grouping and stacking arrive together, in one frame.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Undo reverts the local peer alone | a user pressing `Ctrl+Z` means "take back what I did". Reverting a collaborator's work would let one user erase another's, and no collaborative editor does it | a shared history, where undo walks the board's operations whoever wrote them |
| An undo is an ordinary change, broadcast like any other | the reverting operations are operations, so they converge by the one path everything else uses, and a peer that was offline receives them in order | rolling the board back to a stored version, which two peers cannot agree on |
| The caches are reseeded whole after an undo | one item can move an element, rebind a group and drop a paint-order entry at once, so what needs refreshing is the whole derived state. A partial refresh is a list that goes stale | rebuilding only what the reverted item touched |
| The history dies with the board | it lives in memory beside a document that is itself rebuilt from a snapshot on open, and the operations it would revert are a previous session's | persisting the history, which would offer to revert operations the user cannot see |
| One commit per mutating command, with the gesture as the exception | the user's unit is the action, not the write. A drag is one action and thousands of writes, so the commit belongs at the release | committing on every write, which makes a drag one item per input event |
| Both redo chords are bound | the two are split across the applications a user comes from, and binding one teaches nothing to the half that reaches for the other | one chord, which the menu alone prints |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The buttons are never told anything** (`TOILE-075`). The availability event is emitted by the
  history handler and by the paste, and by none of the other mutating commands; the view matches it
  and does nothing. The two menu entries carry no enabled state at all, so a fresh board offers Undo.
  Trame's counterpart is `verified` on two tests.
* **Local-only is asserted nowhere** (`TOILE-015`). The history is built over the document, which is
  what makes it local, and no test drives a remote change and presses undo. This is the cheapest gap
  in the application to close: the Trame test exists and transposes directly.
* **Undo is not in the convergence property.** The two-peer property drives every mutating command and
  never an undo, so two peers undoing into each other has no coverage in this application, and none in
  Trame either.
* **The depth is unbounded** (`UI-THREAD-016`). A long session grows the history until the tab runs
  out of memory, and nothing measures it. The bound is one number shared with the other
  applications, which is why the row is `ambre-spec-ui.md`'s; what merges into one item stays this
  document's.
* **An undo costs a full rebuild.** Step 3 reseeds every cache from the document, so `Ctrl+Z` on a
  board of fifty thousand elements rebuilds all of them to revert one move
  (`ambre-roadmap.md`, "Engine and render optimization").
