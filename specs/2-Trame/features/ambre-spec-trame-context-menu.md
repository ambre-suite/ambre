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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-085` | `OBS` | A menu MUST list the entries its scope allows, and no others. | this spec section 3 | `ambre_trame::tests::chrome_ui::cell_menu_has_clipboard_and_merge`; `::column_menu_keeps_column_ops_drops_cell_and_row_ops`; `::row_menu_keeps_row_ops_drops_column_inserts`; `::corner_menu_has_sheet_sort_only` -- four scopes, four entry sets, one wasm test each, asserting presence and absence | none | verified |
| `TRAME-UI-086` | `INV` | A menu entry MUST carry no geometry of its own. | this spec section 2 | type-level -- an entry holds a parameterless `CommandId`, so there is no field a coordinate could travel in (`ambre-chrome/src/context_menu.rs`) | none | verified |
| `TRAME-UI-090` | `OBS` | A filter entry MUST NOT offer to filter a row. | this spec section 3 | none | unheld -- `ambre_trame::tests::chrome_ui::row_menu_keeps_row_ops_drops_column_inserts` reads the row scope and asserts the absence of three column entries, never the filter one, and `::filter_entry_toggles_with_filter_state` drives the cell scope, which this rule does not constrain | unverified |

### 1.2 Right-click to open the menu

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-117` | `POST` | The menu MUST open scoped to what the pointer targeted: a cell, a column, a row or the corner. | this spec section 3 | none | unheld -- the scope is resolved in the pointer handler (`ui/view/input.rs`), which no test drives. It is the half `TRAME-UI-085` was carrying: the four entry sets are covered, the point that picks one is not | unverified |
| `TRAME-UI-118` | `POST` | A right-click landing outside the current selection MUST move the selection onto what it targets before the menu opens, as a left click would. | this spec section 3 | none | unheld -- the handler re-selects on the press, and no test drives a right-click. Three of the five cases of the selection table in section 3 reached this pass ruled by nothing at all: the cell outside, the header body and the corner | unverified |
| `TRAME-UI-088` | `FRAME` | A right-click inside the current selection MUST NOT change it. | this spec section 3 | none | unheld -- the handler re-selects only when the point falls outside the selection, and no test drives a right-click. It holds over the cell grid alone: a right-click on a header body selects that axis whether or not the selection already covered it | unverified |

### 1.3 Choose an entry

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-087` | `POST` | A menu entry MUST dispatch through the same command path as the toolbar and the keyboard. | this spec section 2 | type-level -- the menu emits a `CommandId`, and the only route from there is the shared `on_command` dispatch that the toolbar and the keymap also take | none | verified |
| `TRAME-UI-119` | `FRAME` | Choosing an entry MUST NOT change the selection the menu was opened on. | derived | none | unheld -- section 4 rests every range command reached by mouse on this frame condition, and nothing reads the selection back after a menu command | unverified |
| `TRAME-UI-089` | `FAIL` | A command the document refuses MUST leave the document untouched and answer with the reason. | this spec section 3 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` -- it asserts the rejection reply, that no document update is emitted, and that the merge span is unchanged | none | verified |
| `TRAME-UI-120` | `FAIL` | A refusal MUST reach the user as a message, never as silence. | this spec section 3 | none | unheld -- the chrome raises the toast from the rejection event (`chrome/commands.rs`), and no test reaches that arm. It is the half `TRAME-UI-089` was carrying | unverified |

* **Budget.** none: the menu is built from a static entry set on the UI thread, and
  `ambre-spec-budgets.md` budgets no menu operation.
* **Boundary.** none: the menu is view state on the UI thread, where `UI-THREAD-004` bans a
  network, crypto or persistence path. What an entry commits leaves as a `TrameCommand`.
* **Verification.** 4 `verified`, 6 `unverified`. The `verified` count doubled while the cited
  mechanisms did not move: the four wasm entry-set tests and the merge refusal were already here,
  and two conjunctions were split so each half names what covers it. `TRAME-UI-085` claimed the
  entry sets and the pointer that picks them, and only the sets are covered; `TRAME-UI-089` claimed
  the refusal and the message that carries it, and only the refusal is covered. What no test
  reaches is still the pointer, and now the toast: which scope a right-click resolves to, what it
  does to the selection, and whether a refusal is ever seen.

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

* **The pointer half is untested** (`TRAME-UI-117`, `TRAME-UI-118`, `TRAME-UI-088`). The entry
  builder is covered on the wasm tier, one test per scope. The handler that decides which scope a
  right-click means, and what it does to the selection, is reached by nothing, and three of the
  five cases section 3 describes were ruled by no row at all before this pass. It is a pure
  function of a point and a selection rectangle, so it belongs on the host tier and is the cheapest
  gap here to close.
* **A refusal may be silent** (`TRAME-UI-120`). The Logic thread's rejection reply is covered; the
  toast the chrome raises from it is reached by nothing, so a user could meet a command that does
  nothing and says nothing.
* **The sort dialog has no rule.** The multi-column dialog carries ordered levels, a per-level
  direction and a header toggle, and no row states what an empty level list or a duplicated column
  must do.
* **Keyboard access.** The shared component navigates by keyboard once open (`CHROME-026`, itself
  `unverified`), but no key opens the menu: a user without a pointer cannot reach any of these
  commands.
