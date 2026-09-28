# AMBRE -- FEATURE: THE MENUS

**Type:** Vertical (feature)
**Implementation:** partial -- the entries and the zoom control ship; the pointer that scopes them does not.

> A user right-clicks and gets what applies to what is under the cursor. A user opens a menu at the
> top and finds everything else, each entry showing the shortcut that reaches it faster.

A menu owns no behaviour: every entry names a command another feature specifies. What this document
holds is the scope, the layout and the dispatch, plus the zoom control that rides the same bar.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-018` | `INV` | No two menu-bar entries MUST carry the same command id. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::menu_bar_ids_are_distinct` -- it sorts the bar-only id list and reads the deduplicated length back unchanged | none | verified |
| `TOILE-UI-121` | `INV` | No context-menu entry MUST carry an id the menu bar already carries, except where both name the same command. | this spec section 2 | none | unheld -- the two id lists are declared in two modules and the bar reuses the context ids by name, so a collision would be a silent aliasing. `ambre_toile::ui::menu_bar::tests::menu_bar_ids_are_distinct` reads the bar list alone, never the union; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-049` | `INV` | A menu entry MUST carry no geometry: it MUST be resolved against the selection the view already holds. | this spec section 2 | structural -- a row holds a command id, a label and an optional key binding, so there is no field a coordinate could travel in | none | verified |
| `TOILE-UI-050` | `OBS` | The menu-bar layout MUST be a pure function of the menu, never of the selection. | this spec section 4 | type-level -- the layout builder takes a menu id and nothing else, so no selection can reach it | none | verified |
| `TOILE-UI-122` | `OBS` | Every menu in the bar MUST carry at least one entry. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::every_menu_has_items` -- it walks the five menus and reads a non-empty item list from each | none | verified |
| `TOILE-UI-123` | `OBS` | The View menu MUST show all five zoom actions. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::view_menu_exposes_all_zoom_actions` -- it reads the five zoom ids back | none | verified |
| `TOILE-UI-124` | `OBS` | The Edit menu MUST show undo and redo. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::edit_menu_carries_undo_redo_and_reuses_clipboard_ids` -- it reads undo and redo | none | verified |
| `TOILE-UI-364` | `OBS` | The Edit menu MUST reuse the clipboard ids rather than mint its own. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::edit_menu_carries_undo_redo_and_reuses_clipboard_ids` -- it reads the copy and select-all ids as the context menu's own | none | verified |
| `TOILE-UI-125` | `OBS` | The Object menu MUST reuse the group, z-order and lock ids. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::object_menu_reuses_z_order_and_lock_ids` -- it reads the six ids as the ones the context menu emits | none | verified |
| `TOILE-UI-355` | `OBS` | The Object menu MUST carry the six alignments and the two distributions. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_align_entries_sit_under_object` -- it reads the eight ids in the layout | none | verified |
| `TOILE-UI-375` | `OBS` | Each alignment and distribution entry MUST resolve against the selection rather than to a board command of its own. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_align_entries_sit_under_object` -- none of the eight resolves through `menu_bar_edit`; `ambre_toile::ui::menu::tests::an_align_id_resolves_to_one_batch_over_the_selection` -- what each one becomes | none | verified |
| `TOILE-UI-047` | `OBS` | A context-menu entry MUST be left out when it does not apply to the selection under the cursor. | this spec section 3 | none | unheld -- the entry builder is wasm-only and reached by no test. What is covered is the scope the builder reads (section 1.2), not the omission it performs; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-126` | `OBS` | An entry that would move or remove a locked element MUST be left out when the selection is locked. | this spec section 3 | `ambre_toile::ui::menu::tests::a_locked_selection_omits_move_and_remove_entries` -- cut, delete, duplicate and the four z-order ids are absent from a locked scope; `::an_unlocked_selection_keeps_the_full_set` -- the same ids stay when nothing is locked | none | verified |
| `TOILE-UI-052` | `OBS` | A shortcut shown beside an entry MUST be the one the keymap binds. | this spec section 3 | none | unheld -- `ambre_toile::ui::menu_bar::tests::shortcuts_render_canonically` covers how a binding renders, not that the rendered binding is the one bound, because the canvas decodes most board chords itself, outside the keymap the bar reads; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.2 Right-click over the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-048` | `POST` | A right-click whose target is already selected MUST keep the whole selection. | this spec section 3 | none | unheld -- the handler resolves the hit to its group target and re-selects only when the target is absent from the selection, and no test drives a right-click; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-127` | `POST` | A right-click on an element outside the selection MUST collapse the selection to that element. | this spec section 3 | none | unheld -- the same handler as `TOILE-UI-048`, and the same absent test; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-128` | `POST` | A right-click that hits nothing MUST clear the selection, even when the point lies inside the selection box. | this spec section 3 | none | unheld -- an empty hit clears unconditionally, so a click in the gap between two selected elements loses the set, and no test drives it; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-129` | `POST` | An empty selection MUST publish the empty-canvas scope. | this spec section 2 | `ambre_toile::ui::menu::tests::empty_selection_is_empty_canvas_scope` -- it reads the scope as the default and not over a selection | none | verified |
| `TOILE-UI-130` | `POST` | The scope MUST carry the number of selected elements. | this spec section 2 | `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- a count of one; `ambre_toile::ui::menu::tests::multi_selection_enables_group` -- a count of two | none | verified |
| `TOILE-UI-365` | `POST` | The scope MUST mark a multi-selection from two elements up. | this spec section 2 | `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- no multi flag on one; `ambre_toile::ui::menu::tests::multi_selection_enables_group` -- the flag set on two | none | verified |
| `TOILE-UI-131` | `POST` | The scope MUST say whether a group is selected. | this spec section 2 | `ambre_toile::ui::menu::tests::group_and_locked_flags` -- the group flag on a mixed pair | none | verified |
| `TOILE-UI-366` | `POST` | The scope MUST report both lock states separately. | this spec section 2 | `ambre_toile::ui::menu::tests::group_and_locked_flags` -- both lock flags on a mixed pair; `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- the unlocked-only case | none | verified |
| `TOILE-UI-132` | `POST` | The scope MUST name the frame that contains the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::containing_frame_detected` -- it reads the frame enclosing a sticky as the one named | none | verified |
| `TOILE-UI-133` | `POST` | The scope MUST name no frame when the selection falls outside every frame. | this spec section 2 | `ambre_toile::ui::menu::tests::no_frame_when_selection_outside` -- no frame for a sticky placed beyond one | none | verified |
| `TOILE-UI-134` | `POST` | The scope MUST describe the whole selection, not the part of it the viewport holds. | this spec section 5 | `ambre_toile::ui::menu::tests::off_screen_selection_counts_in_scope` -- a selected element the culled set omits still counts and flags; `ambre_toile::logic::convergence_tests::selection_data_includes_offscreen_elements` -- Logic answers the whole selection | none | verified |
| `TOILE-UI-135` | `FRAME` | A right-click MUST NOT emit a board command. | this spec section 3 | none | unheld -- the handler writes the local selection and publishes the scope, and no test drives a right-click and reads the outgoing queue; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-136` | `FRAME` | A right-click MUST NOT move the camera. | this spec section 3 | none | unheld -- the handler reads the camera to map the point and never writes it, and no test drives a right-click; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.3 Choose a context-menu entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-137` | `POST` | Copy MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the copy command carrying the selection unchanged | none | verified |
| `TOILE-UI-138` | `POST` | Cut MUST copy before it deletes. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the pair in that order, which the queue preserves | none | verified |
| `TOILE-UI-139` | `POST` | Delete MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the delete command carrying the selection unchanged | none | verified |
| `TOILE-UI-140` | `POST` | Duplicate MUST offset the copy so it is visibly distinct from its source. | this spec section 2 | `ambre_toile::ui::menu::tests::duplicate_carries_nudge` -- the delta as the diagonal nudge on both axes | none | verified |
| `TOILE-UI-141` | `POST` | Group MUST emit one binding over the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` -- a single group command for a two-element selection | none | verified |
| `TOILE-UI-142` | `POST` | Ungroup MUST emit only for the selected nodes that are groups. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` -- a selection of a group and a sticky yielding one ungroup, naming the group | none | verified |
| `TOILE-UI-143` | `POST` | A z-order entry MUST emit one move per selected element. | this spec section 2 | `ambre_toile::ui::menu::tests::reorder_one_per_element` -- two moves for two elements, each to the front | none | verified |
| `TOILE-UI-144` | `POST` | Lock and unlock MUST emit the same command with the opposite flag. | this spec section 2 | `ambre_toile::ui::menu::tests::lock_unlock_resolve` -- both flags off the two ids | none | verified |
| `TOILE-UI-145` | `POST` | A reaction entry MUST emit one reaction per selected target, carrying the glyph of the entry. | this spec section 2 | `ambre_toile::ui::menu::tests::react_emoji_resolves_to_one_react_per_target` -- one command per target and the glyph on each | none | verified |
| `TOILE-UI-146` | `POST` | The clear entry MUST emit an empty reaction per selected target. | this spec section 2 | `ambre_toile::ui::menu::tests::react_clear_resolves_to_none_reaction` -- the command carrying no reaction | none | verified |
| `TOILE-UI-147` | `POST` | Paste and select-all MUST stay in the view: they MUST emit no board command. | this spec section 2 | `ambre_toile::ui::menu::tests::paste_and_select_all_are_ui_only` -- both resolving to nothing | none | verified |
| `TOILE-UI-148` | `POST` | Add-to-frame MUST target the frame that geometrically contains the selection. | this spec section 2 | `ambre_toile::ui::menu::tests::add_to_frame_targets_container` -- the frame, the added ids and an empty removal list | none | verified |
| `TOILE-UI-149` | `POST` | Remove-from-frame MUST target that same frame. | this spec section 3 | none | fails -- the removal reuses the geometric lookup, so an element whose real parent is another frame is removed from the wrong one and nothing changes. Only the add half is covered; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |
| `TOILE-UI-150` | `POST` | Where frames overlap, the frame the entries target MUST be the front-most one that contains the selection. | this spec section 2 | none | unheld -- the lookup scans the painted order backwards and takes the first container, and the frame test holds one frame, so the choice is never exercised; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-151` | `FRAME` | Resolving an entry MUST NOT change the selection. | this spec section 2 | type-level -- the resolver borrows the selection immutably and returns commands, so it has no way to write it back | none | verified |
| `TOILE-UI-152` | `FAIL` | An id the resolver does not know MUST be refused, and MUST NOT pass as a view-only intent. | this spec section 5 | `ambre_toile::ui::menu::tests::an_unknown_id_is_refused_not_ui_only` -- an unknown id resolves to `None`, never the empty list a view-only intent returns | none | verified |
| `TOILE-UI-153` | `FAIL` | With no enclosing frame, the two frame entries MUST tell the user why nothing happened. | this spec section 5 | none | fails -- both return an empty command list when the lookup finds no container, and no notice is raised. The entries are omitted in that scope, so this is reached only through the bar; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |

### 1.4 Choose a menu-bar entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-154` | `POST` | Undo and redo MUST reach the board. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::undo_redo_map_to_edits` -- each id mapping to its board command | none | verified |
| `TOILE-UI-155` | `POST` | The presentation entry MUST ask to enter presentation mode with focus. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present` -- the command and the focus flag | none | verified |
| `TOILE-UI-156` | `FRAME` | A bar entry that reaches the board MUST NOT read the selection. | this spec section 4 | type-level -- the bar resolver takes a command id alone, so undo, redo and present cannot depend on what is selected | none | verified |

### 1.5 Zoom from the View menu

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-051` | `FRAME` | A zoom action MUST NOT reach the document. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::zoom_ids_are_view_local` -- the five zoom ids resolving to no board command | none | verified |
| `TOILE-UI-157` | `POST` | Zoom in, zoom out and zoom to 100 % MUST move the camera and queue a fresh cull. | this spec section 3 | none | unheld -- each writes the camera and queues a viewport request, and the wiring is wasm-only, so no test reads the camera back after an entry; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-158` | `POST` | A fit MUST ask the Logic thread for the extent before it moves the camera. | this spec section 3 | none | unheld -- the view holds the culled set alone, so both fits arm a pending fit and request the stats, and no test drives the round trip; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-159` | `POST` | A pointer-mode entry MUST stay in the view: it MUST change no camera and reach no thread. | this spec section 3 | none | unheld -- each writes a view field and returns, and the wiring is wasm-only; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | untested |
| `TOILE-UI-160` | `FAIL` | A fit to the selection with nothing selected MUST tell the user why nothing happened. | this spec section 5 | none | fails -- the branch returns on an empty selection, raising no notice, and the View menu shows the entry whatever is selected; lands with the `The Toile belts` Story (`ambre-roadmap.md` E9) | failing |

* **Budget.** none: a menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation. What an entry commits is budgeted
  by the feature that owns the command.
* **Boundary.** none: a menu is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. What an entry commits leaves as a command.
* **Verification.** 37 `verified`, 3 `failing`, 12 `untested`. What holds is the entry set, the omission
  of a locked element's move entries, and a scope read from the whole selection (`TOILE-UI-134`). An unknown
  id is refused rather than passed as a view-only intent. What is loose is the pointer: which scope a
  right-click means, and what it does to the selection. The failures are the frame-removal lookup, the frame
  notice, and the zoom rule its test only half proves.

---

## 2. MODEL

**Containers.** None. A menu writes nothing; the commands it dispatches do.

**Shapes** (`ambre-toile`):

```rust
pub enum MenuId { File, Edit, View, Object, Present }

pub enum MenuRow {
    Item { id: CommandId, accel: Option<KeyBinding> },
    Separator,
}

pub struct CtxScope {
    // what the pointer resolved to: the selection size, whether it is locked,
    // whether it is a group, and the frame that contains it, if any
}
```

**The dispatch, in steps:**

1. the press resolves the hit to its selection target and publishes the scope, before the browser
   opens the menu;
2. the entry set is built from that scope, inapplicable rows left out (`TOILE-UI-047`);
3. a chosen entry writes its command id back to the view;
4. the view resolves the id against the selection it holds, and emits the board commands it produces
   (`TOILE-UI-049`, `TOILE-UI-151`);
5. a zoom entry never reaches step 4: it moves the camera and queues a cull request
   (`TOILE-UI-051`).

**Three families of id.** A board command resolves to one or more commands against the current selection. A
view-only intent -- paste, select-all, the zoom actions, the pointer mode -- is serviced by the view and never
leaves it. It resolves to an empty command list, which is also what an id nobody claims resolves to
(`TOILE-UI-152`). Undo, redo and the presentation entry resolve to a board command with no selection to read,
which is why that map is a pure function.

**Code:**

* `crates/ambre-toile/src/ui/menu.rs` -- the context ids, the scope classifier and the resolver;
* `ui/menu_bar.rs` -- the five menus, their rows, the accelerators and the direct map;
* `ui/view.rs` -- the right-click handler, the scope publication and the id application;
* `chrome/menu.rs`, `chrome/menu_bar.rs` -- the two components, built on `ambre-spec-chrome.md`.

---

## 3. SURFACE

**The context menu.** Right-click, or long-press, over the board.

| Scope | Entries |
|---|---|
| empty board | paste, select all |
| one unlocked element | cut, copy, duplicate, delete; the four z-order moves; lock; add to or remove from the frame containing it; a reaction |
| several elements | the same, plus group and the six alignments |
| three or more elements | the same, plus the two distributions |
| a group | the same, plus ungroup |
| a locked element | unlock, and copy and paste -- every entry that would move or remove it is left out (`TOILE-UI-126`) |

**What a right-click does to the selection.** The press resolves the hit through the group scope, so the
target is the enclosing group and not the node under the cursor. A click whose target is already selected
keeps the whole set, so the menu acts on it (`TOILE-UI-048`). A click on an element outside it collapses to
that element (`TOILE-UI-127`). A click that hits nothing clears the selection, including one that lands in the
gap between two selected elements (`TOILE-UI-128`).

**The menu bar.** Five menus in bar order -- File, Edit, View, Object, Present -- each a fixed layout with
separators, each entry showing its accelerator. Unlike the context menu, the bar hides nothing. An entry that
does not apply is disabled by its command state, so the layout never moves under the user (`TOILE-UI-050`).

| Menu | Holds |
|---|---|
| Edit | undo, redo, the clipboard entries the context menu also carries, and select all |
| View | zoom in, zoom out, 100 %, fit the board, fit the selection, the pointer mode, the panels |
| Object | group and ungroup, the four z-order moves, lock and unlock, the six alignments and the two distributions, and the two link entries |
| File | bringing a file onto the board, saving the board as a native `.ambre` file, and taking it out as a lossy one -- the save above the separator, the exports below it (`features/ambre-spec-toile-import-export.md section 4`) -- then the presentation entry point |
| Present | the presentation entry point, listed in both, and the deck panel (`features/ambre-spec-toile-presentation.md section 4`) |

**The zoom control.** Zoom out, the current percentage, zoom in, fit, and fit the selection. Each moves the
camera and forwards one cull request (`TOILE-UI-157`); the percentage mirrors what the status bar reports
(`features/ambre-spec-toile-status-bar.md`). Neither fit moves the camera on its own. Both ask the Logic
thread for the extent first, because the UI holds the culled set alone (`TOILE-UI-158`,
`features/ambre-spec-toile-viewport.md`).

**Where each entry is specified.** The command an entry names is owned by the feature that implements it. The
clipboard, the groups, the z-order, the frames and the reactions are each specified in their own document.
This one restates none of them.

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Entries are parameterless | passing the click position with the command | it gives two sources for one selection. The shared component dispatches an identifier, so a command needing coordinates could not travel through it, and the view already holds the selection well enough to resolve the geometry itself |
| The scope is resolved on the press, not when the menu builds | reading the pointer position when the menu builds | the browser opens the menu from its own event, after ours, so deciding later means re-deriving the target from a stale position |
| The context menu omits, the menu bar disables | one behaviour for both surfaces | a context menu is short and appears under the cursor, so a wrong entry there is noise. A menu bar is a map of the application, and a layout that changes with the selection cannot be learned |
| A right-click keeps a selection it lands in | collapsing on every right-click, like a plain click | the menu exists to act on sets, and collapsing first makes every multi-element entry unreachable by mouse |
| The zoom ids resolve to no board command | a zoom register on the board | zoom is a way of looking, and a percentage in the document makes two users fight over one camera |
| Undo and redo sit in the bar, not on the canvas | a UI-side stack mirrored for the menu's enabled state | the undo stack is the Logic thread's and the UI holds none, so the mirror is a second stack to keep true. An entry that emits a command is the whole surface |
| The bar and the context menu share their ids | a separate id per surface | the same action reached two ways must be one command, or the two surfaces drift and only one of them gets fixed |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The pointer half is untested.** The entry builder and the five layouts are covered, one test per scope.
  The handler that decides which scope a right-click means, and what it does to the selection, is reached by
  nothing. It is a pure function of a point and a selection. It belongs on the host tier, and it is the
  cheapest gap here to close.
* **A displayed shortcut is not joined to the key that fires it** (`TOILE-UI-052`). The launcher keymap binds
  the panel and the tool chords. The board chords -- undo, redo, duplicate, group, select all, delete, the
  nudges -- are decoded on the canvas. So the accelerator a menu prints and the key the canvas honours are two
  independent lists.
* **The clipboard has no keyboard chord on the canvas.** Cut, copy and paste are menu entries; the
  canvas key handler decodes neither, and paste rides the browser's own event. The menu prints an
  accelerator for each.
* **No entry opens the menu from the keyboard.** The shared component navigates by keyboard once
  open, and nothing opens it, so a user without a pointer reaches none of these commands.
* **The context menu hides the align rows by count alone.** The six alignments appear from two selected
  elements and the two distributions from three, and neither gate reads the lock. A pair where one element is
  locked is still offered every alignment. The operation then moves one box and refuses the other
  (`TOILE-234`, `features/ambre-spec-toile-elements.md`). It is the same hole `TOILE-UI-126` closed for cut,
  delete, duplicate and the z-order rows, left open in the align rows.
* **The comment entry opens the panel and comments on nothing.** The View menu opens the board-wide list, and
  no context-menu entry drops a thread on the element under the cursor. The tool and the pin are the only ways
  in (`features/ambre-spec-toile-comments.md section 4`).
* **A refused command answers with nothing.** The board refuses some commands: a locked element ignores a
  move. No rule states what the user is told when an entry produces no change. Two refusals here are still
  silent: a frame entry with no enclosing frame (`TOILE-UI-153`) and a fit to the selection with nothing
  selected (`TOILE-UI-160`).
* **The frame the entries target is geometric, not the real parent.** The lookup takes the front-most frame
  whose box contains the selection, and no test holds two (`TOILE-UI-150`). Removal reuses it, so an element
  whose parent is another frame is removed from the wrong one and nothing happens (`TOILE-UI-149`).
