# AMBRE -- FEATURE: THE CONTEXT MENU

**Type:** Vertical (feature)
**Implementation:** implemented -- the entries are covered, the pointer that scopes them is not.

> A user right-clicks. What opens depends on what was under the cursor: a cell offers the clipboard,
> structure, sort, filter and merge; a column header drops the cell-only entries; the corner offers
> the whole sheet.

The menu owns no behaviour: every entry names a command another feature specifies. What this
document holds is the scope, the order and the dispatch.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-UI-085` | The menu MUST be scoped to what the pointer targeted: a cell, a column, a row or the corner. | this spec section 3 | partial: `ambre_trame::tests::chrome_ui::cell_menu_has_clipboard_and_merge`, `::column_menu_keeps_column_ops_drops_cell_and_row_ops`, `::row_menu_keeps_row_ops_drops_column_inserts` and `::corner_menu_has_sheet_sort_only` cover the four entry sets; nothing covers the pointer handler that picks the scope | unverified |
| `TRAME-UI-086` | A menu entry MUST carry no geometry of its own. | this spec section 2 | type-level: an entry holds a parameterless `CommandId`, so there is no field a coordinate could travel in (`chrome/commands.rs`) | verified |
| `TRAME-UI-087` | A menu entry MUST dispatch through the same command path as the toolbar and the keyboard. | this spec section 2 | type-level: the menu emits `CommandId` and the only route from there is the shared `on_command` dispatch, which the toolbar and the keymap also take | verified |
| `TRAME-UI-088` | A right-click inside the selection MUST keep it. | this spec section 3 | none. The handler re-selects only when the point falls outside the selection, and no test drives a right-click | unverified |
| `TRAME-UI-089` | A command the document refuses MUST answer with a message, never with silence. | this spec section 3 | partial: `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` covers the refusal reply; nothing covers the toast the chrome raises from it | unverified |
| `TRAME-UI-090` | A filter entry MUST NOT offer to filter a row. | this spec section 3 | none. `ambre_trame::tests::chrome_ui::row_menu_keeps_row_ops_drops_column_inserts` reads the row scope and asserts nothing about the filter entry; `::filter_entry_toggles_with_filter_state` asserts the toggle on the cell scope, which is the scope the rule does not constrain | unverified |

* **Budget.** none: the menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation.
* **Boundary.** none: the menu is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What an entry commits leaves as a `TrameCommand`.
* **Verification.** 2 `verified`, 4 `unverified`. The two that hold are true by construction, and
  the four entry sets are covered on the wasm tier. What no test reaches is the pointer: which
  scope a right-click resolves to, and what it does to the selection.

---

## 2. MODEL

**Containers.** None. The menu writes nothing; the commands it dispatches do.

**Shapes** (`ambre-chrome`):

```rust
pub enum CtxScope { Cell, Column, Row, Corner }

pub struct MenuEntry {
    id: CommandId,          // parameterless: there is no field a coordinate fits in
    icon: Option<IconId>,
    // plus a separator variant, which carries nothing
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
| inside the current selection | the selection stays, so the menu acts on the whole range |
| on a cell outside it | collapses to that cell, like a left click |
| on a column or row header body | selects that whole axis |
| on the corner | selects the sheet |
| on a header boundary | keeps the selection, and scopes the menu to that axis |

**Where each entry is specified.**

| Entries | Owner |
|---|---|
| cut, copy, paste | `features/ambre-spec-trame-clipboard.md section 4` |
| merge, unmerge | `features/ambre-spec-trame-cell-merges.md section 4` |
| filter by column, clear filter | `features/ambre-spec-trame-shared-filter.md section 4` |
| insert and delete rows and columns | `features/ambre-spec-trame-rows-and-columns.md section 4` |
| the sorts, and the sort dialog | `features/ambre-spec-trame-sort.md section 4` |

---

## 4. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The menu is the shared DOM component | it already flips at the viewport edge, navigates by keyboard and dismisses, and it already shares one command path with the toolbar and the keymap. A second menu would duplicate all of that to gain nothing | a canvas overlay drawn by the renderer, which is what the earlier design carried |
| Entries are parameterless | the shared component dispatches an identifier, so a command that needed coordinates could not travel through it. The Logic thread already tracks the selection for formatting, so it can resolve the geometry itself | passing the click position with the command, which would give two sources for one selection |
| The scope is resolved on the press, not on the menu | the browser opens the menu from its own event, after ours; deciding later would mean re-deriving the target from a stale position | reading the pointer position when the menu builds |
| Merge is always listed | a greyed entry that explains itself on click teaches the rule; a missing entry teaches nothing. The refusal carries the reason | hiding the entry when the selection cannot merge |
| A right-click keeps a selection it lands in | the menu exists to act on ranges, and collapsing the range first would make every range command unreachable by mouse | collapsing on every right-click, like a plain click |

---

## 5. NOT BUILT, AND NOT YET RULED

* **The pointer half is untested.** The entry builder is covered on the wasm tier, one test per
  scope. The handler that decides which scope a right-click means, and what it does to the
  selection, is reached by nothing. It is a pure function of a point and a selection rectangle, so
  it belongs on the host tier and is the cheapest gap here to close.
* **The sort dialog has no rule.** The multi-column dialog carries ordered levels, a per-level
  direction and a header toggle, and no row states what an empty level list or a duplicated column
  must do.
* **Keyboard access.** The shared component navigates by keyboard once open, but no key opens the
  menu: a user without a pointer cannot reach any of these commands.
