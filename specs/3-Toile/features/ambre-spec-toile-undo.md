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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-014` | `INV` | One command that mutates the board MUST become one undo item, however many writes it makes. | this spec section 2; architecture section 2.D | `ambre_toile::logic::convergence_tests::set_frame_children_is_one_undo_item`; `::duplicate_a_group_is_one_undo_item`; `::undo_reverts_group_as_one_item` -- three gestures driven through the command path, undo pressed once; `ambre_toile::state::elements::inner_tests::many_inner_creates_share_one_undo_item`; `::create_restyle_and_group_are_one_undo_item` -- these call the `_inner` helpers and commit by hand, so they hold that Loro groups a commit, not that a handler ends with one | fails -- one command breaks it today: a membership change whose clamp fires leaves two undo items rather than one (`TOILE-048`); lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |
| `TOILE-094` | `INV` | One gesture the user performs once MUST become one undo item, however many commands the surface sends for it. | this spec section 2 | none | fails -- a multi-selection ungroup breaks it: the chord sends one `UngroupElements` per selected id, each its own commit, so ungrouping three groups takes three undos (`features/ambre-spec-toile-groups.md section 6`). `TOILE-014` binds the command and never the gesture, so nothing was ruling this; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |
| `TOILE-095` | `INV` | `Ctrl+Z` MUST undo. | this spec section 3 | none | unheld -- `ambre_toile::ui::menu_bar::tests::shortcuts_render_canonically` asserts the chip the menu draws reads `Ctrl+Z`, and its own comment claims that matches the view's chord, but it never reads the decoder. `ui::view::ToileView::handle_key` binds the three chords and no test calls it, so a removed binding would fail nothing; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-096` | `INV` | `Ctrl+Shift+Z` MUST redo. | this spec section 3 | none | unheld -- the same unread decoder binds it, and no menu chip carries it back; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-097` | `INV` | `Ctrl+Y` MUST redo, the binding users of other applications reach for. | this spec section 3 | none | unheld -- it is the one chord the menu never renders, so even the chip test does not reach its name; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-098` | `INV` | The Undo and Redo menu entries MUST send the undo and redo intents. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::undo_redo_map_to_edits` | none | verified |

* **Budget.** none of its own: an undo replays one item and reseeds every cache, so it costs a full
  rebuild. What the history holds sits inside the document holder of `ambre-spec-budgets.md section
  1`, and the cap is `UI-THREAD-016`.
* **Boundary.** none of its own: an undo writes reverting operations into the document, and `ZK-001`
  seals every payload before it leaves the client.
* **Verification.** 1 `verified`, 2 `failing`, 3 `untested`; what holds is the gesture: three driven
  through the command path become one item each. What is loose is the four triggers of section 3,
  three of them chords no test reads. The two failures are the rules the code answers with something
  else. A membership change whose clamp fires takes two items (`TOILE-048`), and a multi-selection
  ungroup takes one per group. The shared rules are `ambre-spec-undo.md`; Toile's gaps are section 4.

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
| ungrouping a selection of three groups | three, and `TOILE-094` says one (`features/ambre-spec-toile-groups.md section 6`) |
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
  handlers of thirty and dropped by the view, so a fresh board always offers Undo. Trame's
  counterpart is `verified` on two tests.
* **Local-only is asserted nowhere** (`UNDO-001`). No test drives a remote change and presses undo.
  It is the cheapest gap here to close: the Trame test (`undo_local_only_not_remote`) transposes.
* **The three chords are decoded by a function no test calls** (`TOILE-095` through `TOILE-097`).
  `handle_key` is reached only from the event loop. The one test naming a chord reads the menu
  chip beside the entry, not the decoder that would honour it.
* **An undo costs a full rebuild.** Step 3 reseeds every cache. `Ctrl+Z` on a board of fifty
  thousand elements rebuilds all of them to revert one move (`ambre-roadmap.md`, "Engine and render
  optimization").
