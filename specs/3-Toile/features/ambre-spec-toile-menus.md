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

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-018` | `INV` | No two menu-bar entries MUST carry the same command id. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::menu_bar_ids_are_distinct` sorts the bar-only id list and asserts the deduplicated length is unchanged | verified |
| `TOILE-UI-121` | `INV` | No context-menu entry MUST carry an id the menu bar already carries, except where both name the same command. | this spec section 2 | none: the two id lists are declared in two modules and the bar reuses the context ids by name, so a collision would be a silent aliasing. The distinctness test reads the bar list alone, never the union | unverified |
| `TOILE-UI-049` | `INV` | A menu entry MUST carry no geometry: it MUST be resolved against the selection the view already holds. | this spec section 2 | structural, and the construction is the proof: a row holds a command id, a label and an optional key binding, so there is no field a coordinate could travel in | verified |
| `TOILE-UI-050` | `OBS` | The menu-bar layout MUST be a pure function of the menu, never of the selection. | this spec section 4 | structural, and the signature is the proof: the layout builder takes a menu id and nothing else, so no selection can reach it | verified |
| `TOILE-UI-122` | `OBS` | Every menu in the bar MUST carry at least one entry. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::every_menu_has_items` walks the five menus and asserts each yields a non-empty item list | verified |
| `TOILE-UI-123` | `OBS` | The View menu MUST show all five zoom actions. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::view_menu_exposes_all_zoom_actions` asserts the five zoom ids are present | verified |
| `TOILE-UI-124` | `OBS` | The Edit menu MUST show undo and redo, and MUST reuse the clipboard ids rather than mint its own. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::edit_menu_carries_undo_redo_and_reuses_clipboard_ids` asserts undo and redo, then asserts the copy and select-all ids are the context menu's own | verified |
| `TOILE-UI-125` | `OBS` | The Object menu MUST reuse the group, z-order and lock ids. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::object_menu_reuses_z_order_and_lock_ids` asserts the six ids are the ones the context menu emits | verified |
| `TOILE-UI-355` | `OBS` | The Object menu MUST carry the six alignments and the two distributions, and each MUST resolve against the selection rather than to a board command of its own. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_align_entries_sit_under_object` asserts the eight ids are in the layout and that none of them resolves through `menu_bar_edit`; `ambre_toile::ui::menu::tests::an_align_id_resolves_to_one_batch_over_the_selection` asserts what each one becomes | verified |
| `TOILE-UI-047` | `OBS` | A context-menu entry MUST be left out when it does not apply to the selection under the cursor. | this spec section 3 | none: the entry builder is wasm-only and reached by no test. What is covered is the scope the builder reads -- section 1.2 -- not the omission it performs | unverified |
| `TOILE-UI-126` | `OBS` | An entry that would move or remove a locked element MUST be left out when the selection is locked. | this spec section 3 | none, and **the rule fails today**: the builder gates only group, ungroup, lock, unlock and the frame pair; cut, delete, duplicate and the four z-order rows are pushed unconditionally, so a locked element is offered every one of them | unverified |
| `TOILE-UI-052` | `OBS` | A shortcut shown beside an entry MUST be the one the keymap binds. | this spec section 3 | none. `ambre_toile::ui::menu_bar::tests::shortcuts_render_canonically` covers how a binding renders, not that the rendered binding is the one bound: the canvas decodes most board chords itself, outside the keymap the bar reads | unverified |

### 1.2 Right-click over the board

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-048` | `POST` | A right-click whose target is already selected MUST keep the whole selection. | this spec section 3 | none: the handler resolves the hit to its group target and re-selects only when the target is absent from the selection, and no test drives a right-click | unverified |
| `TOILE-UI-127` | `POST` | A right-click on an element outside the selection MUST collapse the selection to that element. | this spec section 3 | none: the same handler, the same absent test | unverified |
| `TOILE-UI-128` | `POST` | A right-click that hits nothing MUST clear the selection, even when the point lies inside the selection box. | this spec section 3 | none: an empty hit clears unconditionally, so a click in the gap between two selected elements loses the set. No test drives it | unverified |
| `TOILE-UI-129` | `POST` | An empty selection MUST publish the empty-canvas scope. | this spec section 2 | `ambre_toile::ui::menu::tests::empty_selection_is_empty_canvas_scope` asserts the scope equals the default and that it is not over a selection | verified |
| `TOILE-UI-130` | `POST` | The scope MUST carry the number of selected elements, and MUST mark a multi-selection from two. | this spec section 2 | `ambre_toile::ui::menu::tests::single_unlocked_element_scope` asserts a count of one and no multi flag; `ambre_toile::ui::menu::tests::multi_selection_enables_group` asserts a count of two and the flag set | verified |
| `TOILE-UI-131` | `POST` | The scope MUST say whether a group is selected, and MUST report both lock states separately. | this spec section 2 | `ambre_toile::ui::menu::tests::group_and_locked_flags` asserts the group flag and both lock flags on a mixed pair; `ambre_toile::ui::menu::tests::single_unlocked_element_scope` asserts the unlocked-only case | verified |
| `TOILE-UI-132` | `POST` | The scope MUST name the frame that contains the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::containing_frame_detected` asserts the frame enclosing a sticky is named | verified |
| `TOILE-UI-133` | `POST` | The scope MUST name no frame when the selection falls outside every frame. | this spec section 2 | `ambre_toile::ui::menu::tests::no_frame_when_selection_outside` asserts no frame for a sticky placed beyond one | verified |
| `TOILE-UI-134` | `POST` | The scope MUST describe the whole selection, not the part of it the viewport holds. | this spec section 5 | none, and **the rule fails today**: the classifier looks each selected id up in the culled set, so an element scrolled off screen is skipped -- it leaves the count, the flags and the union it feeds. Every scope test builds a viewport holding the whole selection | unverified |
| `TOILE-UI-135` | `FRAME` | A right-click MUST NOT emit a board command. | this spec section 3 | none: the handler writes the local selection and publishes the scope, and no test drives a right-click and reads the outgoing channel | unverified |
| `TOILE-UI-136` | `FRAME` | A right-click MUST NOT move the camera. | this spec section 3 | none: the handler reads the camera to map the point and never writes it, and no test drives a right-click | unverified |

### 1.3 Choose a context-menu entry

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-137` | `POST` | Copy MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` asserts the copy command carries the selection unchanged | verified |
| `TOILE-UI-138` | `POST` | Cut MUST copy before it deletes. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` asserts the pair in that order, and the channel preserves it | verified |
| `TOILE-UI-139` | `POST` | Delete MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` asserts the delete command carries the selection unchanged | verified |
| `TOILE-UI-140` | `POST` | Duplicate MUST offset the copy so it is visibly distinct from its source. | this spec section 2 | `ambre_toile::ui::menu::tests::duplicate_carries_nudge` asserts the delta is the diagonal nudge on both axes | verified |
| `TOILE-UI-141` | `POST` | Group MUST emit one binding over the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` asserts a single group command for a two-element selection | verified |
| `TOILE-UI-142` | `POST` | Ungroup MUST emit only for the selected nodes that are groups. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` asserts a selection of a group and a sticky yields one ungroup, naming the group | verified |
| `TOILE-UI-143` | `POST` | A z-order entry MUST emit one move per selected element. | this spec section 2 | `ambre_toile::ui::menu::tests::reorder_one_per_element` asserts two moves for two elements, each to the front | verified |
| `TOILE-UI-144` | `POST` | Lock and unlock MUST emit the same command with the opposite flag. | this spec section 2 | `ambre_toile::ui::menu::tests::lock_unlock_resolve` asserts both flags off the two ids | verified |
| `TOILE-UI-145` | `POST` | A reaction entry MUST emit one reaction per selected target, carrying the glyph of the entry. | this spec section 2 | `ambre_toile::ui::menu::tests::react_emoji_resolves_to_one_react_per_target` asserts one command per target and the glyph on each | verified |
| `TOILE-UI-146` | `POST` | The clear entry MUST emit an empty reaction per selected target. | this spec section 2 | `ambre_toile::ui::menu::tests::react_clear_resolves_to_none_reaction` asserts the command carries no reaction | verified |
| `TOILE-UI-147` | `POST` | Paste and select-all MUST stay in the view: they MUST emit no board command. | this spec section 2 | `ambre_toile::ui::menu::tests::paste_and_select_all_are_ui_only` asserts both resolve to nothing | verified |
| `TOILE-UI-148` | `POST` | Add-to-frame MUST target the frame that geometrically contains the selection. | this spec section 2 | `ambre_toile::ui::menu::tests::add_to_frame_targets_container` asserts the frame, the added ids and an empty removal list | verified |
| `TOILE-UI-149` | `POST` | Remove-from-frame MUST target that same frame. | this spec section 3 | none: the removal reuses the geometric lookup, so an element whose real parent is another frame is removed from the wrong one and nothing changes. Only the add half is covered | unverified |
| `TOILE-UI-150` | `POST` | Where frames overlap, the frame the entries target MUST be the front-most one that contains the selection. | this spec section 2 | none: the lookup scans the painted order backwards and takes the first container, and the frame test holds one frame, so the choice is never exercised | unverified |
| `TOILE-UI-151` | `FRAME` | Resolving an entry MUST NOT change the selection. | this spec section 2 | structural, and the signature is the proof: the resolver borrows the selection immutably and returns commands, so it has no way to write it back | verified |
| `TOILE-UI-152` | `FAIL` | An id the resolver does not know MUST be refused, and MUST NOT pass as a view-only intent. | this spec section 5 | none, and **the rule fails today**: the resolver falls through to an empty command list, which is exactly what paste and select-all return, so an unknown id is indistinguishable from a handled one and nobody is told | unverified |
| `TOILE-UI-153` | `FAIL` | With no enclosing frame, the two frame entries MUST tell the user why nothing happened. | this spec section 5 | none: both return an empty command list when the lookup finds no container, and no notice is raised. The entries are omitted in that scope, so this is reached only through the bar | unverified |

### 1.4 Choose a menu-bar entry

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-154` | `POST` | Undo and redo MUST reach the board. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::undo_redo_map_to_edits` asserts each id maps to its board command | verified |
| `TOILE-UI-155` | `POST` | The presentation entry MUST ask to enter presentation mode with focus. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present` asserts the command and the focus flag | verified |
| `TOILE-UI-156` | `FRAME` | A bar entry that reaches the board MUST NOT read the selection. | this spec section 4 | structural, and the signature is the proof: the bar resolver takes a command id alone, so undo, redo and present cannot depend on what is selected | verified |

### 1.5 Zoom from the View menu

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-UI-051` | `FRAME` | A zoom action MUST NOT reach the document. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::zoom_ids_are_view_local` asserts the five zoom ids resolve to no board command | verified |
| `TOILE-UI-157` | `POST` | Zoom in, zoom out and zoom to 100 % MUST move the camera and MUST ask for a fresh cull. | this spec section 3 | none: each writes the camera and queues a viewport request, and the wiring is wasm-only, so no test reads the camera back after an entry | unverified |
| `TOILE-UI-158` | `POST` | A fit MUST ask the Logic thread for the extent before it moves the camera. | this spec section 3 | none: the view holds the culled set alone, so both fits arm a pending fit and request the stats; no test drives the round trip | unverified |
| `TOILE-UI-159` | `POST` | A pointer-mode entry MUST stay in the view: it MUST change no camera and reach no thread. | this spec section 3 | none: each writes a view field and returns, and the wiring is wasm-only | unverified |
| `TOILE-UI-160` | `FAIL` | A fit to the selection with nothing selected MUST tell the user why nothing happened. | this spec section 5 | none: the branch returns on an empty selection, raising no notice. The View menu shows the entry whatever is selected | unverified |

* **Budget.** none: a menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation. What an entry commits is budgeted
  by the feature that owns the command.
* **Boundary.** none: a menu is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. What an entry commits leaves as a command.
* **Verification.** 30 `verified`, 18 `unverified`, against 25 distinct mechanisms -- one more than
  the seven rows cited before the split, and the gain is a single test no row had claimed. The
  ledger did not grow: a cell naming ten tests for one rule now states ten rules, each naming the
  assertion that holds it, and four of the twenty-nine hold by construction. What the split
  withdrew is larger. The omission rule loses its citation outright: the six tests it named classify
  the selection, and the builder that omits an entry is wasm-only and reached by nothing. The zoom
  rule keeps only the half its test proves. The rule that every entry resolves to a command is
  false, so what survives is the distinctness of the bar's own ids. What no test reaches is still
  the pointer -- which scope a right-click means, and what it does to the selection.

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

**Three families of id.** A board command resolves to one or more commands against the current
selection. A view-only intent -- paste, select-all, the zoom actions, the pointer mode -- is
serviced by the view and never leaves it, and resolves to an empty command list, which is also what
an id nobody claims resolves to (`TOILE-UI-152`). Undo, redo and the presentation entry resolve to a board
command with no selection to read, which is why that map is a pure function.

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
| a locked element | unlock -- and, today, every entry that would move or remove it as well (`TOILE-UI-126`) |

**What a right-click does to the selection.** The press resolves the hit through the group scope,
so the target is the enclosing group and not the node under the cursor. A click whose target is
already selected keeps the whole set, so the menu acts on it (`TOILE-UI-048`); a click on an element
outside it collapses to that element (`TOILE-UI-127`). A click that hits nothing clears the
selection, including one that lands in the gap between two selected elements (`TOILE-UI-128`).

**The menu bar.** Five menus in bar order -- File, Edit, View, Object, Present -- each a fixed layout
with separators, each entry showing its accelerator. Unlike the context menu, the bar hides nothing:
an entry that does not apply is disabled by its command state, so the layout never moves under the
user (`TOILE-UI-050`).

| Menu | Holds |
|---|---|
| Edit | undo, redo, the clipboard entries the context menu also carries, and select all |
| View | zoom in, zoom out, 100 %, fit the board, fit the selection, the pointer mode, the panels |
| Object | group and ungroup, the four z-order moves, lock and unlock, the six alignments and the two distributions, and the two link entries |
| File | bringing a file onto the board, saving the board as a native `.ambre` file, and taking it out as a lossy one -- the save above the separator, the exports below it (`features/ambre-spec-toile-import-export.md section 4`) -- then the presentation entry point |
| Present | the presentation entry point, listed in both, and the deck panel (`features/ambre-spec-toile-presentation.md section 4`) |

**The zoom control.** Zoom out, the current percentage, zoom in, fit, and fit the selection. Each
moves the camera and forwards one cull request (`TOILE-UI-157`); the percentage mirrors what the
status bar reports (`features/ambre-spec-toile-status-bar.md`). Neither fit moves the camera on its
own: both ask the Logic thread for the extent first, because the UI holds the culled set alone
(`TOILE-UI-158`, `features/ambre-spec-toile-viewport.md`).

**Where each entry is specified.** The command an entry names is owned by the feature that
implements it: the clipboard, the groups, the z-order, the frames and the reactions are each
specified in their own document, and this one restates none of them.

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

* **The pointer half is untested.** The entry builder and the five layouts are covered, one test per
  scope. The handler that decides which scope a right-click means, and what it does to the selection,
  is reached by nothing. It is a pure function of a point and a selection, so it belongs on the host
  tier and is the cheapest gap here to close.
* **A displayed shortcut is not joined to the key that fires it** (`TOILE-UI-052`). The launcher
  keymap binds the panel and the tool chords; the board chords -- undo, redo, duplicate, group,
  select all, delete, the nudges -- are decoded on the canvas, so the accelerator a menu prints and
  the key the canvas honours are two independent lists.
* **The clipboard has no keyboard chord on the canvas.** Cut, copy and paste are menu entries; the
  canvas key handler decodes neither, and paste rides the browser's own event. The menu prints an
  accelerator for each.
* **No entry opens the menu from the keyboard.** The shared component navigates by keyboard once
  open, and nothing opens it, so a user without a pointer reaches none of these commands.
* **The context menu hides the align rows by count alone.** The six alignments appear from two
  selected elements and the two distributions from three, and neither gate reads the lock: a pair
  where one element is locked is still offered every alignment, and the operation then moves one
  box and refuses the other (`TOILE-234`, `features/ambre-spec-toile-elements.md`). It is the same
  hole as `TOILE-UI-126`, in a newer row.
* **The comment entry opens the panel and comments on nothing.** The View menu opens the board-wide
  list, and no context-menu entry drops a thread on the element under the cursor: the tool and the
  pin are the only ways in (`features/ambre-spec-toile-comments.md section 4`).
* **A refused command answers with nothing.** The board refuses some commands -- a locked element
  ignores a move -- and no rule states what the user is told when an entry produces no change. Three
  refusals on this surface are already silent: an id the resolver does not know (`TOILE-UI-152`), a
  frame entry with no enclosing frame (`TOILE-UI-153`), and a fit to the selection with nothing
  selected (`TOILE-UI-160`).
* **A locked element is offered every entry that would move it** (`TOILE-UI-126`). The builder gates
  group, ungroup, lock, unlock and the frame pair on the scope; cut, delete, duplicate and the four
  z-order rows are pushed whatever the lock state. The scope carries the flag, so the fix is a
  condition, not a new signal.
* **The scope describes the visible selection, not the selection** (`TOILE-UI-134`). Each selected
  id is looked up in the culled set, so an element scrolled off screen leaves the count, the lock
  flags and the union that finds the enclosing frame. Two entries resolve through the same lookup,
  so an off-screen group is not ungrouped and an off-screen element is not added to its frame.
* **The frame the entries target is geometric, not the real parent.** The lookup takes the
  front-most frame whose box contains the selection, and no test holds two (`TOILE-UI-150`); removal
  reuses it, so an element whose parent is another frame is removed from the wrong one and nothing
  happens (`TOILE-UI-149`).
