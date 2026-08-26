# AMBRE -- FEATURE: SHEETS

**Type:** Vertical (feature)
**Implementation:** partial -- the lifecycle and the tab strip are built; the concurrent rules are not.

> A workbook is several sheets, shown as tabs along the bottom edge. A user adds one, renames it,
> drags it to another position, or deletes it -- and every formula pointing into it keeps working.

The tab strip and the sheet lifecycle are one feature: the strip is the only surface either command
has, and every rule about what a sheet is -- its identity, its containers, the guarantee that a
workbook is never empty -- is what the strip lets a user break.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-022` | A workbook MUST always hold at least one sheet. | this spec section 2, row 4 | partial: `ambre_trame::logic::tests::group3::delete_last_sheet_rejected` covers the **local** guard only. Two peers deleting the last two sheets at once still converge on a workbook with none, which is `TRAME-077` | unverified |
| `TRAME-070` | A sheet MUST keep its identity through a rename. | this spec section 3 | `ambre_trame::logic::tests::group3::rename_sheet_keeps_id_and_cells`, `::cross_sheet_reference_converges_across_peers_on_rename` | verified |
| `TRAME-071` | Two users adding a sheet at once MUST each keep theirs. | this spec section 2, row 1 | type-level: an explicit sheet's id is minted from the peer id and a per-peer counter, so two peers cannot mint the same one, and the order list appends rather than elects | verified |
| `TRAME-072` | A sheet's cells MUST NOT be reachable through another sheet's coordinates. | this spec section 3 | `ambre_trame::logic::tests::group3::create_sheet_lists_and_isolates`, `group1::multi_sheet_isolation_and_name_lookup` | verified |
| `TRAME-073` | Reordering the tabs MUST move the strip and nothing else. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs` | verified |
| `TRAME-074` | Two users reordering tabs at once MUST converge on one order. | this spec section 6 | none, and the path is worse than untested: when the stored order differs from the canonical one, the handler empties the list and pushes every id back, discarding the element identities a concurrent move needs to land against | unverified |
| `TRAME-075` | Deleting the sheet a user is reading MUST leave that user on another one. | this spec section 2, row 3 | `ambre_trame::logic::tests::group3::delete_active_sheet_falls_back` | verified |
| `TRAME-076` | Deleting a sheet MUST leave none of its data in the document. | this spec section 6 | none, and the code is partial: the cells, the styles, the merges and the two axes are dropped; the column widths, the row heights, the filter and the hidden rows are not | unverified |
| `TRAME-077` | Two users deleting the last two sheets at once MUST still leave one. | this spec section 6 | not built -- lands with the "Engine completion" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-078` | Two sheets MUST NOT carry the same name. | this spec section 6 | none, and the code does the **opposite**: nothing checks a name when a sheet is created or renamed, so two may share one and a formula naming it reaches whichever the lookup yields first | unverified |
| `TRAME-079` | The tab strip MUST show every sheet of the document, in the document's order. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs`, `::first_scroll_viewport_hydrates_sheet_list_once` | verified |

* **Budget.** none of its own: what opening a sheet costs is the `SheetIndex` rebuild of
  `ambre-spec-budgets.md section 1`, and that budget belongs to
  `features/ambre-spec-trame-engine.md`. A tab switch is a repaint, inside the frame phases of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a sheet create, rename, reorder or delete is a document write, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 6 `verified`, 4 `unverified`, 1 `unimplemented`. The four gaps are all about a
  second user: a concurrent reorder, a concurrent last-sheet delete, the leftovers a delete does
  not remove, and a name nothing keeps unique. Everything one user can do alone is covered.

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| adds a sheet | adds a sheet | both, each under its own id | two new tabs, in a canonical order (`TRAME-071`) | none. The convergence op set carries no sheet command |
| renames `Sheet2` to `Data` | writes `=Sheet2!A1` | the reference, stored by sheet id | both read the value, and both see the qualifier `Data!A1` (`TRAME-070`) | `ambre_trame::logic::tests::group3::cross_sheet_reference_converges_across_peers_on_rename` |
| deletes `Sheet2` | is reading `Sheet2` | the workbook without it | B lands on the neighbouring tab (`TRAME-075`) | `ambre_trame::logic::tests::group3::delete_active_sheet_falls_back`, one peer |
| deletes the last sheet but one | deletes the other | no sheet left | an empty workbook, on both screens (`TRAME-022`, `TRAME-077`) | none, and this is the case the rule exists for |
| drags a tab to the front | drags the same tab to the end | one order, or a rebuilt list | possibly two different strips (`TRAME-074`) | none |

---

## 3. MODEL

**Containers**, declared in `ambre-spec-trame-schema.md section 3`:

| Container | Holds |
|---|---|
| `sheet_order` | the tab order, as a list of sheet ids. List order, no last-write election |
| `sheets` | one entry per sheet: the key is the id, the value the display name. The entry is the unit two writers race for |
| `rows_{sheet}`, `cols_{sheet}`, `cells_{sheet}`, and the rest | one set per sheet, named by the sheet id. They are created on first write, not on create |

**Identity.** A sheet is its id, never its name. The first sheet keeps the constant `s0`, so two
fresh documents merge cleanly. Every sheet created afterwards takes an id minted from the writing
peer and a counter, which is why two concurrent creates cannot collide (`TRAME-071`).

**Commands** (`ambre-bus`):

| Shape | Effect |
|---|---|
| `TrameCommand::CreateSheet { name }` | one entry in `sheets`, one push on `sheet_order`. Logic mints the id |
| `TrameCommand::RenameSheet { sheet_id, name }` | one write of the name field. The id is untouched |
| `TrameCommand::MoveSheet { sheet_id, to_index }` | one move on `sheet_order` |
| `TrameCommand::DeleteSheet { sheet_id }` | drops the entry, the order slot and the sheet's containers |
| `TrameCommand::ActivateSheet { sheet_id }` | changes which sheet the reader is on. View state, not document state |

**What each one costs the engine:**

| Command | Cost |
|---|---|
| create, rename, move | a refreshed tab strip, and nothing else |
| delete | a full reseed: a removed id breaks every formula that pointed into it (`TRAME-009`) |
| activate | a full reseed as well, which it should not be -- the engine pins the active sheet to its own index zero (section 6) |

**Code.** `logic/sheets.rs` -- `create_sheet`, `rename_sheet`, `move_sheet`, `delete_sheet`,
`drop_sheet_containers`, `rebuild_sheet_maps`; `ui/view/interact.rs` and `trame_chrome` -- the strip.

---

## 4. SURFACE

The tab strip is DOM chrome along the bottom edge, not painted on the canvas.

| Part | Behaviour |
|---|---|
| a tab | the sheet's name. The active one is marked, and a click activates its sheet |
| the `+` button | adds a sheet, and the new one becomes active |
| a double-click on a tab | renames it in place. A blank name is refused by the strip, not by the engine |
| a drag of a tab | reorders the strip, dropping at the slot under the pointer (`TRAME-073`) |
| a delete on a tab | removes the sheet, unless it is the last one, which raises an error message |
| the order shown | the document's order, refreshed after every lifecycle command (`TRAME-079`) |

Each handler sends one command and decides nothing: the fallback after deleting the active sheet,
the refusal of the last sheet and the id of a new sheet are all the Logic thread's.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A sheet is an id, and its name is a label | a formula that depended on the name would have to be rewritten on every rename, in every cell, and a rewrite that fails halfway is a corrupt document | naming the sheet in the reference, as the file formats do |
| The first sheet's id is a constant | two peers each starting from a fresh document must merge into one workbook, not two. A minted id makes the two first sheets different sheets | minting the first id like every other |
| An explicit sheet's id carries the peer that minted it | two users adding a sheet at the same second must both get one. A shared counter would give them the same id, and one would silently win | a document-wide counter, or a name-derived id |
| The order is a list, and the names are a map | the order is the thing two users move, and a list move is a first-class operation that converges. The name is the thing two users overwrite, and a map entry is the unit that elects | one container holding both |
| Per-sheet containers are named by the id, not nested | a sheet's cells can be dropped in one pass, and a name collision between two sheets is impossible | one container per sheet holding every sub-container |
| Deleting a sheet breaks its references rather than rewriting them | a formula pointing at deleted data has no right answer, and inventing one hides the deletion from the reader | resolving the reference to empty, as a deleted row does |
| Delete is the one sheet operation a reference can feel | a rename changes a label the reader sees and a reorder changes the strip, so neither touches the dependency graph; only a delete removes a `sheet_id`. Excel's rename-rewrite carries the same hazard as its row-insert rewrite -- two concurrent rewrites of one formula converge on a third -- and binding to the id leaves `#REF!` meaning one thing (`TRAME-009`) | rewriting the name token in every dependent formula on rename, as Excel does |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A concurrent reorder may not converge** (`TRAME-074`). When the stored order does not match what
  the strip shows, the handler clears the list and pushes every id back before moving. That is a
  rebuild, not a move: the element identities a remote move would land against are gone. Nothing
  drives two peers through it.
* **A deleted sheet leaves part of itself behind** (`TRAME-076`). The cells, styles, merges and axes
  go; the column widths, row heights, filter definition and hidden rows stay, keyed by a sheet no
  longer in the document. Nothing reads them, and nothing removes them either.
* **The last sheet holds locally and not between peers** (`TRAME-022`, `TRAME-077`). The rule for the
  concurrent case is decided -- deterministic re-insertion of the lowest-sorting id, identical on
  every peer -- and not implemented.
* **Two sheets may share a name** (`TRAME-078`). Nothing checks. A formula typed with that name
  resolves to whichever the lookup yields first, and the two are indistinguishable in the strip.
* **Switching a tab is a full engine reseed.** The engine pins the active sheet to its own index
  zero, so activating another one remaps every compiled reference. It is engine debt
  (`ambre-spec-trame-engine.md`), and it is the direct cause of `TRAME-062`: a
  named range defined without a sheet qualifier follows the reader from tab to tab.
