# AMBRE -- FEATURE: THE MENUS

**Type:** Vertical (feature)

> A user right-clicks and gets what applies to what is under the cursor. Every other command of the
> board lives in its home region, and the command palette finds each one by name.

A menu owns no behaviour: every entry names a command another feature specifies. What this document
holds is the scope of the context menu, the home of each other command and the dispatch. The zoom
shows in the status bar alone (`CHROME-206`). The board lays out no menu bar (`CHROME-160`).

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-018` | `INV` | No two commands the board declares beyond the context menu MUST carry the same id. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::menu_bar_ids_are_distinct` -- it sorts the list of those ids and reads the deduplicated length back unchanged | none | verified |
| `TOILE-UI-121` | `INV` | No command the board declares beyond the context menu MUST carry an id the context menu already carries, except where both name the same command. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::no_bar_id_is_also_a_context_id` -- it reads every such id against the context list by value, so a shared entry has to reuse the context constant | none | verified |
| `TOILE-UI-049` | `INV` | A menu entry MUST carry no geometry: it MUST be resolved against the selection the view already holds. | this spec section 2 | structural -- a row holds a command id, a label and an optional key binding, so there is no field a coordinate could travel in | none | verified |
| `TOILE-UI-050` | `OBS` | The rows of the Ambre menu MUST be a function of the board's command table and of the open document, never of the selection. | this spec section 4; `CHROME-155` | type-level -- the shell builds the rows from the table, the open document and the undo state alone, so no selection can reach them | none | verified |
| `TOILE-UI-122` | `OBS` | The board MUST home at least one command in the Ambre menu and one in the avatar menu. | this spec section 3; `CHROME-154`; `CHROME-156` | `ambre_toile::ui::menu_bar::tests::the_board_homes_commands_in_the_ambre_and_avatar_menus` -- it reads a command of each region in the board's table | none | verified |
| `TOILE-UI-123` | `OBS` | The five zoom actions MUST have the status bar as their home region. | this spec section 3; `CHROME-206` | `ambre_toile::ui::menu_bar::tests::the_zoom_actions_home_in_the_status_bar` -- it reads the home of the five zoom ids | none | verified |
| `TOILE-UI-124` | `OBS` | Undo and redo MUST have the Ambre menu as their home region. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::undo_and_redo_home_in_the_ambre_menu_and_the_clipboard_in_the_context_menu` -- it reads the home of undo and redo | none | verified |
| `TOILE-UI-364` | `OBS` | Cut, copy and paste MUST have the context menu as their home region, under the ids the context menu carries. | this spec section 3; `CHROME-178` | `ambre_toile::ui::menu_bar::tests::undo_and_redo_home_in_the_ambre_menu_and_the_clipboard_in_the_context_menu` -- it reads the home of the context menu's own clipboard ids | none | verified |
| `TOILE-UI-125` | `OBS` | The group, z-order and lock commands MUST have the context panel as their home region, under the ids the context menu carries. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_group_order_and_lock_commands_home_in_the_context_panel` -- it reads the home of the eight ids the context menu emits | none | verified |
| `TOILE-UI-355` | `OBS` | The six alignments and the two distributions MUST have the context panel as their home region. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_align_entries_home_in_the_context_panel` -- it reads the home of the eight ids | none | verified |
| `TOILE-UI-375` | `OBS` | Each alignment and distribution command MUST resolve against the selection rather than to a board command of its own. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_align_entries_home_in_the_context_panel` -- none of the eight resolves through `menu_bar_edit`; `ambre_toile::ui::menu::tests::an_align_id_resolves_to_one_batch_over_the_selection` -- what each one becomes | none | verified |
| `TOILE-UI-047` | `OBS` | A context-menu entry MUST be left out when it does not apply to the selection under the cursor. | this spec section 3 | `ambre_toile::ui::menu::tests::an_entry_that_does_not_apply_is_left_out` -- it reads the id list the DOM menu maps for one element and for the empty board; `ambre_toile::ui::menu::tests::an_unframed_element_inside_a_box_is_offered_add_only` -- an element no frame lists is offered add and never removal; `ambre_toile::tests::chrome_ui::context_menu_empty_canvas_shows_only_paste_and_select_all`; `ambre_toile::tests::chrome_ui::context_menu_copy_row_fires_copy_command`; `ambre_toile::tests::chrome_ui::context_menu_locked_selection_offers_only_unlock` -- wasm tier: the mounted menu leaves out each row that does not apply | none | verified |
| `TOILE-UI-126` | `OBS` | An entry that would move or remove a locked element MUST be left out when the selection is locked. | this spec section 3 | `ambre_toile::ui::menu::tests::a_locked_selection_omits_move_and_remove_entries` -- cut, delete, duplicate and the four z-order ids are absent from a locked scope; `::an_unlocked_selection_keeps_the_full_set` -- the same ids stay when nothing is locked | none | verified |
| `TOILE-UI-052` | `OBS` | A shortcut shown beside an entry MUST be the one the keymap binds. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_shown_shortcut_does_what_its_entry_does` -- it presses the chord the registry announces for each board entry and picks the entry on a twin board, and reads the same commands and selection back | none | verified |
| `TOILE-UI-417` | `OBS` | Set link and remove link MUST have the context panel as their home region. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::the_link_entries_home_in_the_context_panel` -- the home of both ids, neither resolving to a board command of its own | none | verified |
| `TOILE-UI-418` | `OBS` | The save to an `.ambre` file MUST have the Ambre menu as its home region, and resolve to the board's export of it. | this spec section 3; `ambre-spec-docfile.md section 4`; `CHROME-154` | `ambre_toile::ui::menu_bar::tests::the_ambre_menu_offers_the_native_file_and_resolves_it` -- the home of the two saves, and the plain one resolving to the native export | none | verified |

### 1.2 Right-click over the board

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-048` | `POST` | A right-click whose target is already selected MUST keep the whole selection. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_right_click_inside_the_selection_keeps_it` | none | verified |
| `TOILE-UI-127` | `POST` | A right-click on an element outside the selection MUST collapse the selection to that element. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_right_click_outside_the_selection_collapses_it` | none | verified |
| `TOILE-UI-128` | `POST` | A right-click that hits nothing MUST clear the selection, even when the point lies inside the selection box. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_right_click_on_nothing_clears_the_selection` -- it right-clicks the gap between two selected notes | none | verified |
| `TOILE-UI-129` | `POST` | An empty selection MUST publish the empty-canvas scope. | this spec section 2 | `ambre_toile::ui::menu::tests::empty_selection_is_empty_canvas_scope` -- it reads the scope as the default and not over a selection | none | verified |
| `TOILE-UI-130` | `POST` | The scope MUST carry the number of selected elements. | this spec section 2 | `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- a count of one; `ambre_toile::ui::menu::tests::multi_selection_enables_group` -- a count of two | none | verified |
| `TOILE-UI-365` | `POST` | The scope MUST mark a multi-selection from two elements up. | this spec section 2 | `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- no multi flag on one; `ambre_toile::ui::menu::tests::multi_selection_enables_group` -- the flag set on two | none | verified |
| `TOILE-UI-131` | `POST` | The scope MUST say whether a group is selected. | this spec section 2 | `ambre_toile::ui::menu::tests::group_and_locked_flags` -- the group flag on a mixed pair | none | verified |
| `TOILE-UI-366` | `POST` | The scope MUST report both lock states separately. | this spec section 2 | `ambre_toile::ui::menu::tests::group_and_locked_flags` -- the `Both` mix on a mixed pair; `ambre_toile::ui::menu::tests::a_lock_mix_keeps_both_states_once_read` -- each lock state, once read, stays in the mix; `ambre_toile::ui::menu::tests::single_unlocked_element_scope` -- the unlocked-only case | none | verified |
| `TOILE-UI-132` | `POST` | The scope MUST name the frame that contains the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::containing_frame_detected` -- it reads the frame enclosing a sticky as the one named | none | verified |
| `TOILE-UI-133` | `POST` | The scope MUST name no frame when the selection falls outside every frame. | this spec section 2 | `ambre_toile::ui::menu::tests::no_frame_when_selection_outside` -- no frame for a sticky placed beyond one | none | verified |
| `TOILE-UI-134` | `POST` | The scope MUST describe the whole selection, not the part of it the viewport holds. | this spec section 5 | `ambre_toile::ui::menu::tests::off_screen_selection_counts_in_scope` -- a selected element the culled set omits still counts and flags; `ambre_toile::logic::convergence_tests::selection_data_includes_offscreen_elements` -- Logic answers the whole selection | none | verified |
| `TOILE-UI-135` | `FRAME` | A right-click MUST NOT emit a board command. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_right_click_inside_the_selection_keeps_it`; `ambre_toile::ui::view::view_tests::a_right_click_outside_the_selection_collapses_it` -- each reads the outgoing queue empty after the press and the release | none | verified |
| `TOILE-UI-136` | `FRAME` | A right-click MUST NOT move the camera. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_right_click_inside_the_selection_keeps_it`; `ambre_toile::ui::view::view_tests::a_right_click_outside_the_selection_collapses_it` -- each reads the camera back unchanged | none | verified |
| `TOILE-UI-422` | `POST` | The context-menu key and `Shift+F10` MUST open the context menu over the selection, so a user without a pointer reaches its entries. | this spec section 3; `CHROME-027` | `ambre_toile::ui::view::view_tests::the_menu_keys_open_the_menu_under_the_selection` -- each key opens the menu and keeps the selection, the queue and the camera, while a plain `F10` and a `Ctrl` chord stay the browser's; `ambre_belt::toile::tests::the_menu_key_opens_the_board_menu_and_runs_an_entry` -- browser tier: `Shift+F10` opens the menu over a drawn rectangle, the arrows reach Delete, Enter removes the rectangle, and the focus returns to the board | none | verified |
| `TOILE-UI-423` | `POST` | A context menu opened from the keyboard MUST appear under the selection and inside the board, or at the centre of the board when nothing is selected. | this spec section 3 | `ambre_toile::ui::view::view_tests::the_menu_keys_open_the_menu_under_the_selection` -- the lower-left corner of a two-element box; `ambre_toile::ui::view::view_tests::with_nothing_selected_the_menu_key_opens_at_the_centre`; `ambre_toile::ui::view::view_tests::a_selection_past_the_canvas_holds_the_menu_inside_it` -- a box past the canvas holds the menu at its edge | none | verified |

### 1.3 Choose a context-menu entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-137` | `POST` | Copy MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the copy command carrying the selection unchanged | none | verified |
| `TOILE-UI-138` | `POST` | Cut MUST copy before it deletes. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the pair in that order, which the queue preserves | none | verified |
| `TOILE-UI-139` | `POST` | Delete MUST carry the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve` -- the delete command carrying the selection unchanged | none | verified |
| `TOILE-UI-140` | `POST` | Duplicate MUST offset the copy so it is visibly distinct from its source. | this spec section 2 | `ambre_toile::ui::menu::tests::duplicate_carries_nudge` -- the delta as the diagonal nudge on both axes | none | verified |
| `TOILE-UI-141` | `POST` | Group MUST emit one binding over the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` -- a single group command for a two-element selection | none | verified |
| `TOILE-UI-142` | `POST` | Ungroup MUST emit only for the selected nodes that are groups. | this spec section 2 | `ambre_toile::ui::menu::tests::group_ungroup_resolve` -- a selection of a group and a sticky yielding one ungroup, naming the group | none | verified |
| `TOILE-UI-143` | `POST` | A z-order entry MUST emit one move over the whole selection, so one undo takes it back (`CHROME-173`). | this spec section 2 | `ambre_toile::ui::menu::tests::reorder_is_one_command_for_the_selection` -- one move for two elements, both to the front | none | verified |
| `TOILE-UI-144` | `POST` | Lock and unlock MUST emit the same command with the opposite flag. | this spec section 2 | `ambre_toile::ui::menu::tests::lock_unlock_resolve` -- both flags off the two ids | none | verified |
| `TOILE-UI-145` | `POST` | A reaction entry MUST emit one reaction over the whole selection, carrying the glyph of the entry. | this spec section 2 | `ambre_toile::ui::menu::tests::react_emoji_resolves_to_one_command_for_the_selection` -- one command for every target, with the glyph | none | verified |
| `TOILE-UI-146` | `POST` | The clear entry MUST emit one empty reaction over the whole selection. | this spec section 2 | `ambre_toile::ui::menu::tests::react_clear_resolves_to_none_reaction` -- the command carrying no reaction | none | verified |
| `TOILE-UI-147` | `POST` | Paste and select-all MUST stay in the view: they MUST emit no board command. | this spec section 2 | `ambre_toile::ui::menu::tests::paste_and_select_all_are_ui_only` -- both resolving to nothing | none | verified |
| `TOILE-UI-148` | `POST` | Add-to-frame MUST target the frame that geometrically contains the selection. | this spec section 2 | `ambre_toile::ui::menu::tests::add_to_frame_targets_container` -- the frame, the added ids and an empty removal list | none | verified |
| `TOILE-UI-149` | `POST` | Remove-from-frame MUST take each selected element out of the frame that holds it, whatever frame contains its box. | this spec section 3 | `ambre_toile::ui::menu::tests::remove_from_frame_names_no_frame` -- the entry names the selection and no frame; `ambre_toile::logic::convergence_tests::remove_from_frames_targets_the_real_parent` -- a box inside one frame leaves the other frame that lists it | none | verified |
| `TOILE-UI-384` | `POST` | Remove-from-frame MUST be offered for a selection a frame holds, even when no frame contains its box. | this spec section 3; `TOILE-UI-149` | `ambre_toile::ui::menu::tests::a_framed_element_outside_every_box_is_offered_removal` -- a scope whose element a frame holds, outside every box, offers removal; `ambre_toile::logic::convergence_tests::selection_data_names_a_member_outside_its_frame` -- Logic names the member off its frame's list, and a write or a peer's delta announces the change that makes the view ask again, while a camera move does not; `ambre_toile::ui::view::view_tests::a_board_change_refetches_the_selection_data_and_a_camera_move_does_not` -- the view asks again once on that announcement; structural -- the view hands the reply's framed ids to the scope classifier | none | verified |
| `TOILE-UI-150` | `POST` | Where frames overlap, the frame the entries target MUST be the front-most one that contains the selection. | this spec section 2 | `ambre_toile::ui::menu::tests::overlapping_frames_resolve_to_the_front_most` -- two frames hold the selection, and the later in the painted order is the target | none | verified |
| `TOILE-UI-151` | `FRAME` | Resolving an entry MUST NOT change the selection. | this spec section 2 | type-level -- the resolver borrows the selection immutably and returns commands, so it has no way to write it back | none | verified |
| `TOILE-UI-152` | `FAIL` | An id the resolver does not know MUST be refused, and MUST NOT pass as a view-only intent. | this spec section 5 | `ambre_toile::ui::menu::tests::an_unknown_id_is_refused_not_ui_only` -- an unknown id resolves to `None`, never the empty list a view-only intent returns | none | verified |
| `TOILE-UI-153` | `FAIL` | With no enclosing frame, the two frame entries MUST tell the user why nothing happened. | this spec section 5 | `ambre_toile::ui::view::view_tests::add_to_frame_with_no_frame_around_says_so_and_sends_nothing` -- one notice and no command, and a frame around the sticky gets the add with no notice; `ambre_toile::ui::view::view_tests::remove_from_frame_outside_any_frame_says_so_and_sends_nothing` -- the same for an unframed selection | none | verified |
| `TOILE-UI-415` | `POST` | Choosing Group in the context menu MUST report the group command id. | this spec section 2 | `ambre_toile::tests::chrome_ui::context_menu_multi_selection_offers_group` -- wasm tier: the mounted menu over two elements shows Group, and a click on it reports `menu::GROUP` | none | verified |

### 1.4 Choose a command of the Ambre menu or of the status bar

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-154` | `POST` | Undo and redo MUST reach the board. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::undo_redo_map_to_edits` -- each id mapping to its board command | none | verified |
| `TOILE-UI-155` | `POST` | The presentation entry MUST ask to enter presentation mode with focus. | this spec section 3 | `ambre_toile::ui::menu_bar::tests::present_entry_emits_enter_present` -- the command and the focus flag | none | verified |
| `TOILE-UI-156` | `FRAME` | A command of the Ambre menu or of the status bar that reaches the board directly MUST NOT read the selection. | this spec section 4 | type-level -- the direct resolver takes a command id alone, so undo, redo and present cannot depend on what is selected | none | verified |
| `TOILE-UI-395` | `POST` | A command chosen from the palette MUST apply before the next input the board takes. | this spec section 2 | `ambre_belt::toile::tests::a_palette_command_runs_before_the_click_that_follows_it` -- a click sent as the palette closes lands on the rectangle only once the zoom applies | none | verified |

### 1.5 Zoom from the palette or a chord

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-UI-051` | `FRAME` | A zoom action MUST NOT reach the document. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::zoom_ids_are_view_local` -- the five zoom ids resolving to no board command | none | verified |
| `TOILE-UI-157` | `POST` | Zoom in, zoom out and zoom to 100 % MUST move the camera and queue a fresh cull. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_zoom_entry_moves_the_camera_and_queues_a_cull` -- it reads the zoom back after each entry and a `SetViewport` in the queue | none | verified |
| `TOILE-UI-158` | `POST` | A fit MUST ask the Logic thread for the extent before it moves the camera. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_fit_asks_for_the_extent_before_it_moves` -- both fits send the request with the camera still, and move it on the answer | none | verified |
| `TOILE-UI-159` | `POST` | A pointer-mode entry MUST stay in the view: it MUST change no camera and reach no thread. | this spec section 3 | `ambre_toile::ui::view::view_tests::a_pointer_mode_entry_stays_in_the_view` | none | verified |
| `TOILE-UI-160` | `FAIL` | A fit to the selection with nothing selected MUST tell the user why nothing happened. | this spec section 5 | `ambre_toile::ui::view::view_tests::zoom_to_selection_with_nothing_selected_says_so` -- one notice and no request, then a selected sticky asks for its extent with no notice | none | verified |
| `TOILE-UI-416` | `POST` | The zoom-in entry of the command palette MUST report the zoom-in command. | this spec section 3 | `ambre_toile::tests::chrome_ui::the_palette_reports_zoom_in` -- wasm tier: the palette mounted on the board's table, and its Zoom in row reports `ZOOM_IN` | none | verified |

* **Budget.** none: a menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation. What an entry commits is budgeted
  by the feature that owns the command.
* **Boundary.** none: a menu is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. What an entry commits leaves as a command.
* **Verification.** 60 `verified`. What holds is the entry set, the omission of what does not
  apply and the home of each other command. A scope reads the whole selection (`TOILE-UI-134`). The
  host view drives the right-click and each zoom entry, and presses a shown chord on a twin board.
  An unknown id is refused, and an entry that changes nothing says why, while the browser tier
  holds a palette command against the next click.

---

## 2. MODEL

**Containers.** None. A menu writes nothing; the commands it dispatches do.

**Shapes** (`ambre-toile`):

```rust
pub struct CtxScope {
    // what the pointer resolved to: the selection size, whether it is locked,
    // whether it is a group, the frame that contains it, if any, and whether
    // a frame holds part of it; for a menu key, the point the view chose
}
```

**Where the scope reads from.** The selection's data comes from the Logic thread, off-screen included
(`TOILE-UI-134`). The same reply names the selected ids a frame's children list holds, which no box
tells. The Logic thread announces each board change, local or a peer's, and the view then asks for that
data again. A camera move is no change, so a pan never re-reads the selection. The re-read costs the
selection, not the board: groups and frames come from a derived parent map (`TOILE-056`).

**The dispatch, in steps:**

1. the press resolves the hit to its selection target and publishes the scope, before the browser
   opens the menu. A menu key publishes the scope of the selection as it stands, and names the point
   of the menu itself (`TOILE-UI-422`);
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
* `ui/menu_bar.rs` -- the ids no context-menu entry carries, and the direct map;
* `ui/commands.rs` -- the home region of every command;
* `ui/view.rs` -- the right-click handler, the scope publication and the id application;
* `chrome/menu.rs` -- the context menu, built on `ambre-spec-chrome.md`;
* `crates/ambre-app/src/shell/header.rs` -- the Ambre menu and the avatar menu, which read the
  board's table.

---

## 3. SURFACE

**The context menu.** Right-click, or long-press, over the board.

| Scope | Entries |
|---|---|
| empty board | paste, select all |
| one unlocked element | cut, copy, duplicate, delete; the four z-order moves; lock; add to the frame whose box contains it, and remove from the frame that holds it, wherever it sits (`TOILE-UI-384`); a reaction |
| several elements | the same, plus group and the six alignments |
| three or more elements | the same, plus the two distributions |
| a group | the same, plus ungroup |
| a locked element | unlock, and copy and paste -- every entry that would move or remove it is left out (`TOILE-UI-126`) |

**From the keyboard.** The context-menu key and `Shift+F10` open the same menu over the selection as it
stands (`TOILE-UI-422`). A key names no point, so the selection keeps every element. The menu appears at
the lower-left corner of the selection's box, held inside the board. With nothing selected, it appears at
the centre of the board (`TOILE-UI-423`). A plain `F10` stays the browser's, as in the Murmure message
list (`MURMURE-511`).

The arrows move through the entries, Enter runs one, and Escape closes the menu. Closing it gives the
focus back to the board (`CHROME-027`).

**What a right-click does to the selection.** The press resolves the hit through the group scope, so the
target is the enclosing group and not the node under the cursor. A click whose target is already selected
keeps the whole set, so the menu acts on it (`TOILE-UI-048`). A click on an element outside it collapses to
that element (`TOILE-UI-127`). A click that hits nothing clears the selection, including one that lands in the
gap between two selected elements (`TOILE-UI-128`).

**The home of each other command.** The board lays out no menu bar (`CHROME-160`). Each command the
context menu does not carry lives in its home region (`ambre-spec-shell.md section 2`), and the
command palette lists every one of them (`CHROME-008`).

| Command | Home region |
|---|---|
| undo, redo, find, the command palette | the Ambre menu (`TOILE-UI-124`) |
| importing a file, the two saves to an `.ambre` file, the five exports | the Ambre menu, the saves before the exports (`TOILE-UI-418`, `features/ambre-spec-toile-import-export.md section 4`) |
| the five zoom actions, the start of the presentation | the status bar (`TOILE-UI-123`, `TOILE-UI-087`) |
| the pointer mode | the avatar menu: a preference of the person, kept on the device |
| group and ungroup, the z-order, the lock, the alignments, the distributions, the links, the comments, the deck | the context panel (`TOILE-UI-125`, `TOILE-UI-355`, `TOILE-UI-417`) |
| embedding a web page or a document | the creation bar |
| cut, copy and paste | the context menu (`TOILE-UI-364`) |

The Ambre menu disables an undo or a redo with nothing to take back, from the state the Logic thread
reports (`UNDO-006`). Its rows never move with the selection (`TOILE-UI-050`).

**The zoom entries.** Zoom out, zoom in, 100 %, fit, and fit the selection sit in the palette, and
the chords of `ambre-spec-zoom.md` reach them. The zoom chip of the status bar shows the
percentage (`features/ambre-spec-toile-status-bar.md`, `CHROME-206`). Each entry moves the camera
and forwards one cull request (`TOILE-UI-157`). Neither fit moves the camera on its own. Both ask
the Logic thread for the extent first, because the UI holds the culled set alone (`TOILE-UI-158`,
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
| The context menu omits, the Ambre menu disables | one behaviour for both surfaces | a context menu is short and appears under the cursor, so a wrong entry there is noise. The Ambre menu is a map of the file, and a layout that changes with the selection cannot be learned |
| Each command in its home region | a menu bar of five menus | it files commands under words a user must guess, and hides each one behind two clicks (`ambre-spec-shell.md section 4`) |
| A menu key opens the menu at the selection | opening it at the last pointer position | a keyboard user leaves the pointer anywhere, often off the board. The menu then opens far from what it acts on |
| A right-click keeps a selection it lands in | collapsing on every right-click, like a plain click | the menu exists to act on sets, and collapsing first makes every multi-element entry unreachable by mouse |
| The zoom ids resolve to no board command | a zoom register on the board | zoom is a way of looking, and a percentage in the document makes two users fight over one camera |
| Undo and redo sit in the Ambre menu, not on the canvas | a UI-side stack mirrored for the menu's enabled state | the undo stack is the Logic thread's and the UI holds none, so the mirror is a second stack to keep true. An entry that emits a command is the whole surface |
| The palette and the context menu share their ids | a separate id per surface | the same action reached two ways must be one command, or the two surfaces drift and only one of them gets fixed |

---

## 5. NOT BUILT, AND NOT YET RULED

* **A displayed shortcut and the key that fires it are two lists** (`TOILE-UI-052`). The launcher keymap
  binds the panel and the tool chords, and the canvas decodes the board chords itself. A host test presses
  each announced board chord against its entry, so a drift goes red. The two lists stay two declarations.
* **The clipboard has no keyboard chord on the canvas.** Cut, copy and paste are menu entries; the
  canvas key handler decodes neither, and paste rides the browser's own event. The menu prints an
  accelerator for each.
* **The context menu hides the align rows by count alone.** The six alignments appear from two selected
  elements and the two distributions from three, and neither gate reads the lock. A pair where one element is
  locked is still offered every alignment. The operation then moves one box and refuses the other
  (`TOILE-234`, `features/ambre-spec-toile-elements.md`). It is the same hole `TOILE-UI-126` closed for cut,
  delete, duplicate and the z-order rows, left open in the align rows.
* **The comment entry opens the panel and comments on nothing.** The comments command of the panel
  opens the board-wide list, and no context-menu entry drops a thread on the element under the
  cursor. The tool and the pin are the only ways
  in (`features/ambre-spec-toile-comments.md section 4`).
