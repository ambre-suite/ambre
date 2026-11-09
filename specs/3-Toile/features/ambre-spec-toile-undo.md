# AMBRE -- FEATURE: TOILE UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship, and the buttons read the availability.

> Toile's half of the shared undo model (`ambre-spec-undo.md`): what one item is on a board, and
> which caches an undo reseeds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-undo.md`,
`UNDO-001` through `UNDO-006`; Trame's half is `2-Trame/features/ambre-spec-trame-undo.md`.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-014` | `INV` | One command that mutates the board MUST become one undo item, however many writes it makes. | this spec section 2; architecture section 2.D | `ambre_toile::logic::convergence_tests::set_frame_children_is_one_undo_item`; `::a_clamped_membership_change_is_one_undo_item`; `::duplicate_a_group_is_one_undo_item`; `::undo_reverts_group_as_one_item`; `::ungrouping_three_groups_is_one_undo_item` -- five commands driven through the command path, undo pressed once; `ambre_toile::state::elements::inner_tests::many_inner_creates_share_one_undo_item`; `::create_restyle_and_group_are_one_undo_item` -- these call the `_inner` helpers and commit by hand, so they hold that Loro groups a commit, not that a handler ends with one | none | verified |
| `TOILE-094` | `INV` | One gesture the user performs once MUST become one undo item, however many commands the surface sends for it. | this spec section 2 | `ambre_toile::ui::menu::tests::ungrouping_a_selection_sends_one_command` -- the menu entry and the chord each send one command for three selected ids; `ambre_toile::logic::convergence_tests::ungrouping_three_groups_is_one_undo_item` -- that command over three groups, undone once; `ambre_toile::logic::convergence_tests::ungrouping_a_group_with_its_parent_unbinds_both` -- a nested pair in either order; `ambre_toile::ui::view::view_tests::the_ungroup_chord_sends_one_command_over_the_selection` -- the chord through the board view sends one ungroup over two selected ids | none | verified |
| `TOILE-097` | `INV` | `Ctrl+Y` MUST redo, the binding users of other applications reach for. | this spec section 3 | `ambre_toile::ui::view::view_tests::the_undo_chords_reach_the_logic_thread` -- `Ctrl+Z`, `Ctrl+Shift+Z` and `Ctrl+Y` through the view send undo, redo and redo | none | verified |
| `TOILE-098` | `INV` | The Undo and Redo menu entries MUST send the undo and redo intents. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::undo_redo_map_to_edits` | none | verified |
| `TOILE-290` | `INV` | Each step of the history MUST act on the element it recorded, also after an undo has put that element back. | this spec section 2; `1-Foundations/ambre-spec-undo.md section 2` (`UNDO-001`) | `ambre_toile::logic::undo_identity_tests::an_undone_delete_then_an_undone_create_leaves_no_element`; `::an_undone_delete_after_a_reorder_and_a_move_restores_both`; `::undo_and_redo_follow_a_board_rebuilt_by_hand` -- a proptest, 256 cases of creates, deletes, reorders, moves, undos and redos against a board kept by hand, with a second peer that takes every delta | none | verified |
| `TOILE-291` | `INV` | An undo and a redo of a move MUST each hold the Toile transform command row of `ambre-spec-budgets.md section 1`. | `ambre-spec-budgets.md section 1`; this spec section 2 | none | fails -- on a board of 2 000 stickies the `move_undo` and `move_redo` operations of `crates/ambre-toile/benches/instructions.rs` cost 59 930 493 and 59 594 465 instructions under `callgrind` on 2026-11-08, about 75 ms on the target phone. The whole reseed `UNDO-005` asks for costs 98 % of it. The CI `instructions` job holds both at a bound of 62 000 000, and it lands with the `An undo on a large board within its budget` Story (`ambre-roadmap.md` E8) | failing |

* **Budget.** the Toile transform command row of `ambre-spec-budgets.md section 1` (`TOILE-291`).
  An undo reseeds every cache, so it costs a full rebuild. What the history holds sits inside the
  document holder of that section, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo writes reverting operations into the document, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 5 `verified`, 1 `failing`. Five commands driven through the command path become
  one item each, a clamped membership change and a multi-selection ungroup included. The chords
  are driven through the view, and the menu entries are read. A board kept by hand is the oracle
  for `TOILE-290`. The undo of a move misses its budget on a large board.

---

## 2. MODEL

**Containers.** None of its own: an undo item is one commit, not one container write.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `ToileEvent::UndoState { undo, redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

The engine field is the one history of `ambre-spec-undo.md section 3`. A comment write is no step
(`features/ambre-spec-toile-comments.md`, `TOILE-FEAT-017`).

**Which caches an undo reseeds** (`UNDO-005`), whole and from the document:

* the bounds map;
* the spatial index;
* the paint order;
* the group boxes.

**The element an undo puts back** (`TOILE-290`). An undo of a delete writes the element record
again by value, with its body, and a new entry of the paint order. The element keeps its id, the
key of its record. The history notes the record's new Loro identity against the old one. A later
step of the stack so still reaches the element.

A record a peer deleted takes no write (`ambre-spec-undo.md`, `UNDO-012`).

**What one undo item is** (`TOILE-014`):

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
| a membership change that reorders a child | one, the clamp included (`TOILE-048`) |
| ungrouping a selection of three groups | one: the surface sends one command for it (`TOILE-094`) |
| a copy | none: it writes nothing (`TOILE-067`) |
| a camera change, a tool change, a presence sample, playback | none: they write nothing |

**Code:**

* `crates/ambre-toile/src/logic/state/mod.rs` -- the revert and replay handler, and the event;
* `logic/state/caches.rs` -- the reseed step 3 runs;
* `ui/menu_bar.rs` -- the two entries and their chords;
* `chrome/commands.rs` -- the mirror from `UndoState` into the command store the menu bar reads.

---

## 3. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` |
| `Ctrl+Y` | `Redo`, the second binding users of other applications reach for |
| Undo and Redo, in the edit menu | the same two intents |

The two entries are disabled from `UndoState` alone, which the chrome reads from its command store
(`UNDO-006`).

---

## 4. REJECTED ALTERNATIVES

The shared decisions are `ambre-spec-undo.md section 5`: the local-only revert, the broadcast
inverse, the whole reseed, the stack that dies with the document. Toile's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A drag is one item, sealed on release | one item per frame the pointer moved | a drag writes the position dozens of times a second, and each write is a commit. One item per commit would make `Ctrl+Z` walk the path the hand took, which is not what the user did: they moved the element once |
| A freehand stroke is one item, however many samples it holds | one item per sample | the same reason, and worse: a stroke holds hundreds. What the user drew is the stroke |
| A whole file import is one item | one item per element the file carried | a user who imports a board of two thousand elements means one action, and the only useful undo is the one that takes the file back out |
| A comment never enters the history | recording it like any other write | undo would retract a message a teammate has already read (`TOILE-FEAT-017`). A message is addressed to someone, and taking it back is not the same act as taking back a drawing |
| The camera, the tool and the presence sample enter no history | recording what the user is looking at | they write no document, so there is nothing to revert and no peer receives them. An undo that scrolled the board back would spend the user's one chord on their own viewport |

---

## 5. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Toile's own:

* **Local-only is asserted nowhere** (`UNDO-001`). No test drives a remote change and presses undo.
  It is the cheapest gap here to close: the Trame test (`undo_local_only_not_remote`) transposes.
* **An undo costs a full rebuild.** Step 3 reseeds every cache. `Ctrl+Z` on a board of fifty
  thousand elements rebuilds all of them to revert one move (`ambre-roadmap.md`, "Engine and render
  optimization").
