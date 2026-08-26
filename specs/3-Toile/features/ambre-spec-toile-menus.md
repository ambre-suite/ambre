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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-UI-018` | Every context-menu entry and every menu-bar item MUST resolve to a command, with no duplicate id across menus. | this spec section 2 | `ambre_toile::ui::menu::tests::copy_cut_delete_resolve`, `::group_ungroup_resolve`, `::lock_unlock_resolve`, `::reorder_one_per_element`, `::duplicate_carries_nudge`, `::add_to_frame_targets_container`, `ambre_toile::ui::menu_bar::tests::every_menu_has_items`, `::menu_bar_ids_are_distinct`, `::edit_menu_carries_undo_redo_and_reuses_clipboard_ids`, `::object_menu_reuses_z_order_and_lock_ids` | verified |
| `TOILE-UI-047` | A context-menu entry MUST be left out when it does not apply to the selection under the cursor. | this spec section 3 | `ambre_toile::ui::menu::tests::empty_selection_is_empty_canvas_scope`, `::single_unlocked_element_scope`, `::multi_selection_enables_group`, `::group_and_locked_flags`, `::containing_frame_detected`, `::no_frame_when_selection_outside` | verified |
| `TOILE-UI-048` | A right-click MUST keep a selection it lands in, and MUST collapse to the element under the cursor otherwise. | this spec section 3 | none. The handler resolves the hit and re-selects only outside the current selection, and no test drives a right-click | unverified |
| `TOILE-UI-049` | A menu entry MUST carry no geometry: it MUST be resolved against the selection the view already holds. | this spec section 2 | type-level: an entry holds a parameterless command id, so there is no field a coordinate could travel in; `ambre_toile::ui::menu::tests::paste_and_select_all_are_ui_only`, `::react_emoji_resolves_to_one_react_per_target` | verified |
| `TOILE-UI-050` | The menu-bar layout MUST be a pure function of the menu, never of the selection. | this spec section 4 | `ambre_toile::ui::menu_bar::tests::every_menu_has_items`, `::view_menu_exposes_all_zoom_actions`, `::present_entry_emits_enter_present`, `::undo_redo_map_to_edits` | verified |
| `TOILE-UI-051` | A zoom action MUST stay local: it MUST move the camera and MUST NOT reach the document. | this spec section 2 | `ambre_toile::ui::menu_bar::tests::zoom_ids_are_view_local`, which asserts the zoom ids resolve to no board command | verified |
| `TOILE-UI-052` | A shortcut shown beside an entry MUST be the one the keymap binds. | this spec section 3 | none. `ambre_toile::ui::menu_bar::tests::shortcuts_render_canonically` covers how a binding renders, not that the rendered binding is the one bound: the canvas decodes most board chords itself, outside the keymap the bar reads | unverified |

* **Budget.** none: a menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation. What an entry commits is budgeted
  by the feature that owns the command.
* **Boundary.** none: a menu is view state on the UI thread, where `UI-THREAD-004` bans a network,
  crypto or persistence path. What an entry commits leaves as a command.
* **Verification.** 5 `verified`, 2 `unverified`. The entry sets and the layouts are covered ten ways
  and two of the five hold by construction. What no test reaches is the pointer -- which scope a
  right-click means, and what it does to the selection -- and the join between a displayed shortcut
  and the key that fires it.

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
   (`TOILE-UI-049`);
5. a zoom entry never reaches step 4: it moves the camera and queues a cull request
   (`TOILE-UI-051`).

**Three families of id.** A board command resolves to one or more commands against the current
selection. A view-only intent -- paste, select-all, the zoom actions, the pointer mode -- is
serviced by the view and never leaves it. Undo, redo and the presentation entry resolve to a board
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
| several elements | the same, plus group |
| a group | the same, plus ungroup |
| a locked element | unlock, and nothing that would move it |

**What a right-click does to the selection.** A click inside the current selection keeps it, so the
menu acts on the whole set. A click on an element outside it collapses to that element. A click on
empty board clears it (`TOILE-UI-048`).

**The menu bar.** Five menus in bar order -- File, Edit, View, Object, Present -- each a fixed layout
with separators, each entry showing its accelerator. Unlike the context menu, the bar hides nothing:
an entry that does not apply is disabled by its command state, so the layout never moves under the
user (`TOILE-UI-050`).

| Menu | Holds |
|---|---|
| Edit | undo, redo, the clipboard entries the context menu also carries, and select all |
| View | zoom in, zoom out, 100 %, fit the board, fit the selection, the pointer mode, the panels |
| Object | group and ungroup, the four z-order moves, lock and unlock |
| File, Present | the presentation entry point, listed in both (`features/ambre-spec-toile-presentation.md section 4`) |

**The zoom control.** Zoom out, the current percentage, zoom in, fit, and fit the selection. Each
moves the camera and forwards one cull request; the percentage mirrors what the status bar reports
(`features/ambre-spec-toile-status-bar.md`). A fit over the board asks the Logic thread for the
extent, because the UI holds the culled set alone (`features/ambre-spec-toile-viewport.md`).

**Where each entry is specified.** The command an entry names is owned by the feature that
implements it: the clipboard, the groups, the z-order, the frames and the reactions are each
specified in their own document, and this one restates none of them.

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Entries are parameterless | the shared component dispatches an identifier, so a command needing coordinates could not travel through it. The view already holds the selection, so it can resolve the geometry itself | passing the click position with the command, which gives two sources for one selection |
| The scope is resolved on the press, not when the menu builds | the browser opens the menu from its own event, after ours; deciding later would mean re-deriving the target from a stale position | reading the pointer position when the menu builds |
| The context menu omits, the menu bar disables | a context menu is short and appears under the cursor, so a wrong entry is noise; a menu bar is a map of the application, and a layout that changes with the selection cannot be learned | one behaviour for both surfaces |
| A right-click keeps a selection it lands in | the menu exists to act on sets, and collapsing first would make every multi-element entry unreachable by mouse | collapsing on every right-click, like a plain click |
| The zoom ids resolve to no board command | zoom is a way of looking, and a percentage in the document would make two users fight over one camera | a zoom register on the board |
| Undo and redo sit in the bar, not on the canvas | the undo stack is the Logic thread's, and the UI holds none; an entry that emits a command is the whole surface | a UI-side stack mirrored for the menu's enabled state |
| The bar and the context menu share their ids | the same action reached two ways must be one command, or the two surfaces drift and only one of them gets fixed | a separate id per surface |

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
* **Three surfaces the design names carry no entry.** Align and distribute, the link and the
  comment appear in the described menus and in none of the built ones, because none of the three
  commands exists in the workspace. Two of them are Stories --
  "Align and distribute" and "Hyperlinks and go-to-frame" (`ambre-roadmap.md` E4) -- and the third
  lands with "Comments".
* **A refused command answers with nothing.** The board refuses some commands -- a locked element
  ignores a move -- and no rule states what the user is told when an entry produces no change.
