# AMBRE -- FEATURE: THE CONTEXT MENU

**Type:** Vertical (feature)

> A user right-clicks. What opens depends on what was under the cursor. A cell offers the clipboard,
> structure, sort, filter and merge; a column header drops the cell-only entries; the corner offers
> the whole sheet.

The menu owns no behaviour: every entry names a command another feature specifies. What this
document holds is the scope, the order and the dispatch.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-085` | `OBS` | A menu MUST list the entries its scope allows, and no others. | this spec section 3 | `ambre_trame::tests::chrome_ui::cell_menu_has_clipboard_and_merge`; `::column_menu_keeps_column_ops_drops_cell_and_row_ops`; `::row_menu_keeps_row_ops_drops_column_inserts`; `::corner_menu_has_sheet_sort_only` -- four scopes, four entry sets, one wasm test each, asserting presence and absence | none | verified |
| `TRAME-UI-086` | `INV` | A menu entry MUST carry no geometry of its own. | this spec section 2 | type-level -- an entry holds a parameterless `StaticKey<Commands>`, so there is no field a coordinate could travel in (`ambre-chrome/src/context_menu.rs`) | none | verified |
| `TRAME-UI-090` | `OBS` | A filter entry MUST NOT offer to filter a row. | this spec section 3 | `ambre_trame::ui::commands::tests::a_row_menu_never_offers_to_filter` | none | verified |

### 1.2 Open the menu

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-117` | `POST` | The menu MUST open scoped to what the pointer targeted: a cell, a column, a row or the corner. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_scopes_the_menu_to_its_target` | none | verified |
| `TRAME-UI-118` | `POST` | A right-click landing outside the current selection MUST move the selection onto what it targets before the menu opens, as a left click would. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_outside_the_selection_reselects` | none | verified |
| `TRAME-UI-088` | `FRAME` | A right-click inside the current selection MUST NOT change it. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_inside_the_selection_keeps_it` | none | verified |
| `TRAME-UI-188` | `POST` | The context-menu key and `Shift+F10` MUST open the menu over the selection as it stands, so a user without a pointer reaches its entries. | this spec section 3; `CHROME-027` | `ambre_trame::ui::view::view_tests::the_menu_keys_open_the_menu_at_the_active_cell` -- each key opens the cell menu and keeps the selection, while nothing reaches the Logic thread, and a plain `F10`, a `Ctrl` chord and a key in an open editor stay the browser's; `ambre_trame::ui::view::view_tests::the_menu_key_scopes_the_menu_to_the_selection_shape` -- a band of columns or rows takes its header menu; `ambre_belt::trame_grid::tests::the_menu_key_opens_the_grid_menu_and_runs_an_entry` -- browser tier: `Shift+F10` opens the menu, the arrows reach Insert row above, Enter moves the value of `B2` down to `B3`, and the focus returns to the grid | none | verified |
| `TRAME-UI-189` | `POST` | A menu opened from the keyboard MUST appear at the lower-left corner of the active cell, held inside the grid area. | this spec section 3 | `ambre_trame::ui::view::view_tests::the_menu_keys_open_the_menu_at_the_active_cell` -- the lower-left corner of the active cell of a range; `ambre_trame::ui::view::view_tests::an_active_cell_past_the_grid_holds_the_menu_inside_it` -- a cell scrolled out holds the menu at the edge it lies past, never over a header | none | verified |

### 1.3 Choose an entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-087` | `POST` | A menu entry MUST dispatch through the same command path as the toolbar and the keyboard. | this spec section 2 | type-level -- the menu emits a `StaticKey<Commands>`, and the only route from there is the shared `on_command` dispatch that the toolbar and the keymap also take | none | verified |
| `TRAME-UI-190` | `FRAME` | A key that moves through an open menu or runs its entry MUST NOT reach the grid. | this spec section 3 | `ambre_app::host::tests::a_key_inside_an_open_menu_stays_there` -- the keyboard listener of the page leaves a key aimed at a menu entry to the menu; `ambre_belt::trame_grid::tests::the_menu_key_opens_the_grid_menu_and_runs_an_entry` -- browser tier: three arrows and Enter in the menu insert the row above `B2`, where arrows that reached the grid would insert it under `B2` | none | verified |
| `TRAME-UI-119` | `FRAME` | Choosing an entry MUST NOT change the selection the menu was opened on. | derived | `ambre_trame::ui::view::view_tests::a_menu_entry_leaves_the_selection_alone` -- each entry runs on a Logic state and its answers reach the view | none | verified |
| `TRAME-UI-089` | `FAIL` | A command the document refuses MUST leave the document untouched and answer with the reason. | this spec section 3 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` -- it asserts the rejection reply, that no document update is emitted, and that the merge span is unchanged | none | verified |
| `TRAME-UI-120` | `FAIL` | A refusal MUST reach the user as a message, never as silence. | this spec section 3 | `ambre_trame::tests::chrome_ui::a_refusal_reaches_the_user_as_a_toast` | none | verified |

* **Budget.** none: the menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation.
* **Boundary.** none: the menu is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What an entry commits leaves as a `TrameCommand`.
* **Verification.** 13 `verified`. The entry sets are read back in wasm, and the merge refusal
  holds in the Logic thread. The pointer half is driven through the view on the host tier: the
  scope a right-click resolves and what it does to the selection. The menu keys are driven there
  too, and the browser tier holds a menu key against the entry it reaches. The toast a refusal
  raises is read back in wasm.

---

## 2. MODEL

**Containers.** None. The menu writes nothing; the commands it dispatches do.

**Shapes** (`ambre-chrome`):

```rust
pub enum CtxScope { Cell, Column, Row, Corner }

#[derive(Clone, PartialEq)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

#[derive(Clone, PartialEq)]
pub struct MenuItem {
    // parameterless: no field a coordinate fits in
    command: StaticKey<Commands>,
    label: String,
    icon: Option<icondata::Icon>,
    shortcut: Option<KeyBinding>,
    disabled: bool,
}
```

**The dispatch, in steps:**

1. The pointer handler resolves the scope and stores it, synchronously, before the browser opens
   the menu. A menu key stores the scope of the selection as it stands, and names the point of the
   menu itself (`TRAME-UI-188`).
2. The menu is built for that scope, with its icons and separators.
3. A chosen entry emits its `StaticKey<Commands>` into the shared command path.
4. The path turns it into an application command, which reaches the Logic thread.
5. The Logic thread resolves the geometry against the active cell and the selection rectangle it
   already tracks.

**Code.** `chrome/commands.rs` -- `context_entries_for` and the entry groups; `ui/view/input.rs` --
the right-click scope resolution; `ambre-chrome` -- the menu component itself.

---

## 3. SURFACE

**Scopes, and what each lists.**

| Scope | Entries |
|---|---|
| a cell | cut, copy, paste; insert and delete rows and columns; sort, filter; merge and unmerge |
| a column header | insert and delete columns; sort; filter |
| a row header | insert and delete rows; sort; the filter entry only to clear one (`TRAME-UI-090`) |
| the corner | sort the sheet by the active column, ascending or descending; clear the filter |

**What the right-click does to the selection.**

| Where it lands | Effect |
|---|---|
| inside the current selection | the selection stays, so the menu acts on the whole range (`TRAME-UI-088`) |
| on a cell outside it | collapses to that cell, like a left click (`TRAME-UI-118`) |
| on a column or row header body | selects that whole axis (`TRAME-UI-118`) |
| on the corner | selects the sheet (`TRAME-UI-118`) |
| on a header boundary | keeps the selection, and scopes the menu to that axis (`TRAME-UI-117`) |

**From the keyboard.** The context-menu key, alone or with Shift, and `Shift+F10` open the same
menu over the selection as it stands (`TRAME-UI-188`). A key names no target, so the selection
stays, and its shape picks the scope:

| Selection | Scope |
|---|---|
| cells | a cell |
| whole columns, the whole sheet included | a column header |
| whole rows | a row header |

The menu appears at the lower-left corner of the active cell, held inside the grid area
(`TRAME-UI-189`). A plain `F10` stays the browser's, as on a Toile board (`TOILE-UI-422`). While a
cell editor is open, the keys stay the editor's: the browser opens its own text menu there. The
arrows move through the entries, Enter runs one, and Escape closes the menu. None of these keys
reaches the grid while the menu is open (`TRAME-UI-190`). Closing the menu gives the focus back to
the grid (`CHROME-027`).

**Where each entry is specified.**

| Entries | Owner |
|---|---|
| cut, copy, paste | `features/ambre-spec-trame-clipboard.md section 4` |
| merge, unmerge | `features/ambre-spec-trame-cell-merges.md section 4` |
| filter by column, clear filter | `features/ambre-spec-trame-shared-filter.md section 4` |
| insert and delete rows and columns | `features/ambre-spec-trame-rows-and-columns.md section 4` |
| the sorts, and the sort dialog | `features/ambre-spec-trame-sort.md section 4` |

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The menu is the shared DOM component | a canvas overlay drawn by the renderer | it is what the earlier design carried, and it would duplicate to gain nothing what the shared component already does: flipping at the viewport edge, keyboard navigation, dismissal, and one command path shared with the toolbar and the keymap. |
| Entries are parameterless | passing the click position with the command | it would give two sources for one selection. The shared component dispatches an identifier, so a command that needed coordinates could not travel through it, and the Logic thread already tracks the selection for formatting. |
| The scope is resolved on the press | reading the pointer position when the menu builds | the browser opens the menu from its own event, after ours; deciding later would mean re-deriving the target from a stale position. |
| Merge is always listed | hiding the entry when the selection cannot merge | a missing entry teaches nothing, where a greyed entry that explains itself on click teaches the rule. The refusal carries the reason. |
| A menu key opens the menu at the active cell | opening it at the last pointer position | a keyboard user leaves the pointer anywhere, often off the grid. The menu then opens far from the cell it acts on. |
| A menu key keeps the selection, and its shape picks the scope | always opening the cell menu | a band selected from a header would lose its header menu, which a right-click on that header offers. |
| A right-click keeps a selection it lands in | collapsing on every right-click, like a plain click | the menu exists to act on ranges, and collapsing the range first would make every range command unreachable by mouse. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The sort dialog has no rule.** The multi-column dialog carries ordered levels, a per-level
  direction and a header toggle. No row states what an empty level list or a duplicated column must
  do.
