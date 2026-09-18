# AMBRE -- FEATURE: SHEETS

**Type:** Vertical (feature)
**Implementation:** partial -- the lifecycle and the tab strip are built. The concurrent rules are
not, and a new sheet is not the one the user lands on.

> A workbook is several sheets, shown as tabs along the bottom edge. A user adds one, renames it,
> drags it to another position, or deletes it -- and every formula pointing into it keeps working.

The tab strip and the sheet lifecycle are one feature. The strip is the only surface either command
has. Its identity, its containers and the guarantee that a workbook is never empty are all rules the
strip lets a user break.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-022` | `INV` | A workbook MUST always hold at least one sheet. | this spec section 2, row 4 | none | partial -- the local guard is a refusal, held by `TRAME-163`, and two peers deleting the last two sheets at once still converge on a workbook with none, which is `TRAME-077`; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-072` | `INV` | A sheet's cells MUST NOT be reachable through another sheet's coordinates. | this spec section 3 | `ambre_trame::logic::tests::group3::create_sheet_lists_and_isolates`; `ambre_trame::logic::tests::group1::multi_sheet_isolation_and_name_lookup` | none | verified |
| `TRAME-078` | `INV` | Two sheets MUST NOT carry the same name. | this spec section 6 | none | fails -- nothing checks a name on a create or on a rename, and the strip names a new sheet after the count of tabs it shows, so four gestures (add, add, delete the first, add) put two identical labels in the strip with no second user in sight; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |
| `TRAME-079` | `OBS` | The tab strip MUST show every sheet of the document, in the document's order. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs`; `::first_scroll_viewport_hydrates_sheet_list_once` -- the order the Logic thread publishes; `ambre_trame::tests::chrome_ui::sheet_tabs_render_active_and_emit_intents` -- the strip drawing one tab per entry in that order, what joins the two halves being the launcher mirroring the list into the strip's signals | none | verified |
| `TRAME-071` | `CONC` | Two users adding a sheet at once MUST each keep theirs. | this spec section 2, row 1 | none | unheld -- the mint gives each peer its own id, the document's peer id and a per-session counter, and the order list appends rather than elects, but nothing goes red if that mint changes: no test drives two peers through a create, and the convergence property's op set carries no sheet command; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-074` | `CONC` | Two users reordering tabs at once MUST converge on one order. | this spec section 6 | none | fails -- when the stored order differs from the canonical one, the handler empties the list and pushes every id back, dropping the element identities a concurrent move needs to land against; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | failing |
| `TRAME-077` | `CONC` | Two users deleting the last two sheets at once MUST still leave one. | this spec section 6 | not built | unbuilt -- lands with the `Engine completion` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Add a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-159` | `FRAME` | Adding a sheet MUST NOT change which sheet the reader is on. | this spec section 4 | `ambre_trame::logic::tests::group3::create_sheet_lists_and_isolates` -- it asserts the active id after the create, the new tab never being activated because nothing sends the second command the create leaves to the caller (section 6) | none | verified |

### 1.3 Rename a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-160` | `PRE` | The name a rename is given MUST NOT be empty. | this spec section 4 | none | partial -- the strip drops a blank on both its paths, the Enter and the blur, and the engine writes whatever it is handed, so a rename reaching it from anywhere else leaves a tab with no label and no way to click it; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-070` | `POST` | A sheet MUST keep its identity through a rename. | this spec section 3 | `ambre_trame::logic::tests::group3::rename_sheet_keeps_id_and_cells`; `::cross_sheet_reference_converges_across_peers_on_rename` | none | verified |
| `TRAME-161` | `FRAME` | A rename MUST NOT move the sheet in the strip. | this spec section 4 | none | unheld -- the rename writes the name map alone and never the order list, and the canonical order falls back to the id, not to the label, so no sort can follow a name. No test reads the order after a rename; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.4 Drag a tab to another slot

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-073` | `POST` | Reordering the tabs MUST put the sheet at the slot it was dropped on. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs` | none | verified |
| `TRAME-162` | `FRAME` | Reordering the tabs MUST NOT change a cell, a style or a formula. | this spec section 4 | none | unheld -- the move writes the order list alone and asks for no recompute, and no test reads the document after one. The list is not left alone either: a diverging order is cleared and pushed back before the move, which is what `TRAME-074` fails on; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.5 Delete a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-163` | `FAIL` | Deleting the last remaining sheet MUST be refused, with a message and no write. | this spec section 4 | `ambre_trame::logic::tests::group3::delete_last_sheet_rejected` -- it asserts both the error event and the absence of an update | none | verified |
| `TRAME-075` | `POST` | Deleting the sheet a user is reading MUST leave that user on another one. | this spec section 2, row 3 | `ambre_trame::logic::tests::group3::delete_active_sheet_falls_back` | none | verified |
| `TRAME-076` | `POST` | Deleting a sheet MUST leave none of its data in the document. | this spec section 6 | none | partial -- the cells, the styles, the merges and the two axes are dropped, and the column widths, the row heights, the filter and the hidden rows are not; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |
| `TRAME-164` | `FRAME` | Deleting a sheet MUST NOT touch another sheet's data. | this spec section 5 | none | unheld -- every per-sheet container is named by the sheet id, so the drop cannot reach a neighbour's key, it does not reach four of its own either (`TRAME-076`), and no test reads a second sheet after a delete; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

### 1.6 Switch to another tab

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-165` | `FRAME` | Switching tabs MUST NOT change the document, and MUST reach no collaborator. | this spec section 3 | none | unheld -- the switch answers with display events alone, so it carries no delta to send, and no test asserts it. What it does cost is a full engine reseed, which is the debt of section 6; lands with the `The Trame belts` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** none of its own: what opening a sheet costs is the `SheetIndex` rebuild of
  `ambre-spec-budgets.md section 1`, and that budget belongs to
  `features/ambre-spec-trame-engine.md`. A tab switch is a repaint, inside the frame phases of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a sheet create, rename, reorder or delete is a document write, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 7 `verified`, 2 `failing`, 8 `untested`, 1 `unimplemented`. What holds is each
  gesture on one peer, plus what an add leaves alone: the tab the reader is on. What is loose is
  every frame around them, and the one invariant no single peer can reach. A distinct id between two
  peers is an argument, not a mechanism (`TRAME-071`, `TRAME-077`). The local guard offered as
  covering it is a refusal of its own (`TRAME-163`).

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
| the `+` button | adds a sheet at the end of the strip, and leaves the reader on the sheet they were on (`TRAME-159`) |
| a double-click on a tab | renames it in place. A blank name is refused by the strip, not by the engine (`TRAME-160`) |
| a drag of a tab | reorders the strip, dropping at the slot under the pointer (`TRAME-073`) |
| a delete on a tab | removes the sheet, unless it is the last one, which raises an error message and writes nothing (`TRAME-163`) |
| the order shown | the document's order, refreshed after every lifecycle command (`TRAME-079`) |

Each handler sends one command and decides nothing. The fallback after an active-sheet delete, the
refusal of the last sheet and the id of a new sheet are all the Logic thread's.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A sheet is an id, and its name is a label | naming the sheet in the reference, as the file formats do | a formula that depended on the name would have to be rewritten on every rename, in every cell, and a rewrite that fails halfway is a corrupt document |
| The first sheet's id is a constant | minting the first id like every other | two peers each starting from a fresh document must merge into one workbook, not two. A minted id makes the two first sheets different sheets |
| An explicit sheet's id carries the peer that minted it | a document-wide counter | two users adding a sheet at the same second must both get one. A shared counter would give them the same id, and one would silently win |
| An explicit sheet's id carries the peer that minted it | a name-derived id | the name is an editable label and nothing keeps two sheets from sharing one; deriving the id from it would move the id on every rename, which the first decision here exists to prevent |
| The order is a list, and the names are a map | one container holding both | the order is the thing two users move, and a list move is a first-class operation that converges. The name is the thing two users overwrite, and a map entry is the unit that elects |
| Per-sheet containers are named by the id, not nested | one container per sheet holding every sub-container | a sheet's cells can be dropped in one pass, and a name collision between two sheets is impossible |
| Deleting a sheet breaks its references | resolving the reference to empty, as a deleted row does | a formula pointing at deleted data has no right answer, and inventing one hides the deletion from the reader |
| Delete is the one sheet operation a reference can feel | rewriting the name token in every dependent formula on rename, as Excel does | a rename changes a label the reader sees and a reorder changes the strip, so neither touches the dependency graph; only a delete removes a `sheet_id`. Excel's rename-rewrite carries the same hazard as its row-insert rewrite -- two concurrent rewrites of one formula converge on a third -- and binding to the id leaves `#REF!` meaning one thing (`TRAME-009`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **A concurrent reorder may not converge** (`TRAME-074`). When the stored order does not match what
  the strip shows, the handler clears the list and pushes every id back before moving. That is a
  rebuild, not a move: the element identities a remote move would land against are gone. Nothing
  drives two peers through it.
* **A deleted sheet leaves part of itself behind** (`TRAME-076`). The cells, styles, merges and axes
  go. The column widths, row heights, filter definition and hidden rows stay, keyed by a sheet no
  longer in the document. Nothing reads them, and nothing removes them either.
* **The last sheet holds locally and not between peers** (`TRAME-022`, `TRAME-077`). The rule for the
  concurrent case is decided -- deterministic re-insertion of the lowest-sorting id, identical on
  every peer -- and not implemented.
* **The `+` button does not open the sheet it adds** (`TRAME-159`). `create_sheet` leaves the
  active sheet alone by design, and expects the surface to send `ActivateSheet` behind it. The
  launcher does not, so a user who clicks `+` gets a tab they have to click again. Nothing rules
  which of the two is wrong.
* **Two sheets may share a name** (`TRAME-078`). Nothing checks, and one user is enough. The
  launcher names a new sheet after the count of sheets. Adding two, deleting the first of them and
  adding again yields the same name twice. A formula typed with that name resolves to whichever
  the lookup yields first, and the two are indistinguishable in the strip.
* **Switching a tab is a full engine reseed.** The engine pins the active sheet to its own index
  zero, so activating another one remaps every compiled reference. It is engine debt
  (`ambre-spec-trame-engine.md`), and it is the direct cause of `TRAME-062`. A named range defined
  without a sheet qualifier follows the reader from tab to tab.
