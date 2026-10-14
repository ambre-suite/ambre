# AMBRE -- FEATURE: THE CONTEXT MENU

**Type:** Vertical (feature)
**Implementation:** implemented -- the entries are covered, the pointer that scopes them is not.

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
| `TRAME-UI-086` | `INV` | A menu entry MUST carry no geometry of its own. | this spec section 2 | type-level -- an entry holds a parameterless `CommandId`, so there is no field a coordinate could travel in (`ambre-chrome/src/context_menu.rs`) | none | verified |
| `TRAME-UI-090` | `OBS` | A filter entry MUST NOT offer to filter a row. | this spec section 3 | `ambre_trame::ui::commands::tests::a_row_menu_never_offers_to_filter` | none | verified |

### 1.2 Right-click to open the menu

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-117` | `POST` | The menu MUST open scoped to what the pointer targeted: a cell, a column, a row or the corner. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_scopes_the_menu_to_its_target` | none | verified |
| `TRAME-UI-118` | `POST` | A right-click landing outside the current selection MUST move the selection onto what it targets before the menu opens, as a left click would. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_outside_the_selection_reselects` | none | verified |
| `TRAME-UI-088` | `FRAME` | A right-click inside the current selection MUST NOT change it. | this spec section 3 | `ambre_trame::ui::view::view_tests::a_right_click_inside_the_selection_keeps_it` | none | verified |

### 1.3 Choose an entry

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-087` | `POST` | A menu entry MUST dispatch through the same command path as the toolbar and the keyboard. | this spec section 2 | type-level -- the menu emits a `CommandId`, and the only route from there is the shared `on_command` dispatch that the toolbar and the keymap also take | none | verified |
| `TRAME-UI-119` | `FRAME` | Choosing an entry MUST NOT change the selection the menu was opened on. | derived | `ambre_trame::ui::view::view_tests::a_menu_entry_leaves_the_selection_alone` -- each entry runs on a Logic state and its answers reach the view | none | verified |
| `TRAME-UI-089` | `FAIL` | A command the document refuses MUST leave the document untouched and answer with the reason. | this spec section 3 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` -- it asserts the rejection reply, that no document update is emitted, and that the merge span is unchanged | none | verified |
| `TRAME-UI-120` | `FAIL` | A refusal MUST reach the user as a message, never as silence. | this spec section 3 | `ambre_trame::tests::chrome_ui::a_refusal_reaches_the_user_as_a_toast` | none | verified |

* **Budget.** none: the menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation.
* **Boundary.** none: the menu is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What an entry commits leaves as a `TrameCommand`.
* **Verification.** 10 `verified`. The entry sets are read back in wasm, and the merge refusal
  holds in the Logic thread. The pointer half is driven through the view on the host tier: the
  scope a right-click resolves and what it does to the selection. The toast a refusal raises is
  read back in wasm.

---

## 2. MODEL

**Containers.** None. The menu writes nothing; the commands it dispatches do.

**Shapes** (`ambre-chrome`):

```rust
pub enum CtxScope { Cell, Column, Row, Corner }

pub enum MenuEntry {
    // parameterless: no variant field a coordinate fits in
    Item { command: CommandId, label: String, icon: Option<Icon>, shortcut: Option<KeyBinding> },
    Separator,
}
```

**The dispatch, in steps:**

1. The pointer handler resolves the scope and stores it, synchronously, before the browser opens
   the menu.
2. The menu is built for that scope, with its icons and separators.
3. A chosen entry emits its `CommandId` into the shared command path.
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
| A right-click keeps a selection it lands in | collapsing on every right-click, like a plain click | the menu exists to act on ranges, and collapsing the range first would make every range command unreachable by mouse. |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The sort dialog has no rule.** The multi-column dialog carries ordered levels, a per-level
  direction and a header toggle. No row states what an empty level list or a duplicated column must
  do.
* **Keyboard access.** The shared component navigates by keyboard once open (`CHROME-026`, itself
  `unverified`), but no key opens the menu. A user without a pointer cannot reach any of these
  commands.
