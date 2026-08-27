# AMBRE -- FEATURE: UNDO AND REDO

**Type:** Vertical (feature)
**Implementation:** partial -- undo and redo ship; the buttons are never told what is available.

> Toile's half of the shared undo model (`ambre-spec-undo.md`): what one item is on a board, and
> which caches an undo reseeds.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-undo.md`,
`UNDO-001` through `UNDO-006`; Trame's half is `2-Trame/features/ambre-spec-trame-undo.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-014` | Each mutating command MUST end with one commit, so one atomic user action is one undo item. | this spec section 2; architecture section 2.D | `ambre_toile::logic::state::elements::inner_tests::many_inner_creates_share_one_undo_item`, `::create_restyle_and_group_are_one_undo_item`, `ambre_toile::logic::convergence_tests::set_frame_children_is_one_undo_item`, `::duplicate_a_group_is_one_undo_item`, `::undo_reverts_group_as_one_item` | verified |

* **Budget.** none of its own: an undo replays one item and reseeds every cache, so it costs a full
  rebuild. What the history holds sits inside the document holder of `ambre-spec-budgets.md section
  1`, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo writes reverting operations into the document, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 1 `verified`: what one undo item is, held five ways. The shared rules and their
  gaps are in `ambre-spec-undo.md`; Toile's own are in section 4.

---

## 2. MODEL

**Containers.** None of its own: an undo item is one commit, not one container write.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `ToileCommand::Undo` / `Redo` | UI to Logic | nothing: the history is the Logic thread's |
| `ToileEvent::UndoState { can_undo, can_redo }` | Logic to UI | what the two affordances may offer (`UNDO-006`) |

The engine field is the shared `undo_manager` (`ambre-spec-undo.md section 3`).

**Which caches an undo reseeds** (`UNDO-005`), whole and from the document:

* the bounds map;
* the spatial index;
* the paint order;
* the group boxes.

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
| a membership change that reorders a child | two, and the rule says one (`TOILE-048`) |
| ungrouping a selection of three groups | three, and the rule says one per group (`features/ambre-spec-toile-groups.md section 6`) |
| a copy | none: it writes nothing (`TOILE-067`) |
| a camera change, a tool change, a presence frame, playback | none: they write nothing |

**Code:**

* `crates/ambre-toile/src/logic/state/mod.rs` -- the revert and replay handler, and the event;
* `logic/state/caches.rs` -- the reseed step 3 runs;
* `ui/menu_bar.rs` -- the two entries and their chords.

---

## 3. SURFACE

| Trigger | Sends |
|---|---|
| `Ctrl+Z` | `Undo` |
| `Ctrl+Shift+Z` | `Redo` |
| `Ctrl+Y` | `Redo`, the second binding users of other applications reach for |
| Undo and Redo, in the edit menu | the same two intents |

The two entries are never disabled, because the availability never reaches them (`UNDO-006`,
section 4).

---

## 4. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-undo.md section 6`. Toile's own:

* **The buttons are never told anything** (`UNDO-006`). The availability event is emitted by two
  handlers of thirty and discarded by the view, so a fresh board always offers Undo. Trame's
  counterpart is `verified` on two tests.
* **Local-only is asserted nowhere** (`UNDO-001`). No test drives a remote change and presses undo.
  It is the cheapest gap here to close: the Trame test (`undo_local_only_not_remote`) transposes.
* **An undo costs a full rebuild.** Step 3 reseeds every cache, so `Ctrl+Z` on a board of fifty
  thousand elements rebuilds all of them to revert one move (`ambre-roadmap.md`, "Engine and render
  optimization").
