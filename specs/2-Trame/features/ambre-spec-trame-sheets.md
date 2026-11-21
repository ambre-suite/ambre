# AMBRE -- FEATURE: SHEETS

**Type:** Vertical (feature)

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

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-022` | `INV` | A workbook MUST always hold at least one sheet. | this spec section 2, row 4 | `ambre_trame::logic::tests::group3::delete_last_sheet_rejected` -- one peer; `ambre_trame::logic::tests::group11::two_peers_deleting_the_last_two_sheets_keep_one` -- two peers, and the refusal of the sheet they are left with | none | verified |
| `TRAME-072` | `INV` | A sheet's cells MUST NOT be reachable through another sheet's coordinates. | this spec section 3 | `ambre_trame::logic::tests::group3::create_sheet_lists_and_isolates`; `ambre_trame::logic::tests::group1::multi_sheet_isolation_and_name_lookup` | none | verified |
| `TRAME-078` | `INV` | Two sheets MUST NOT carry the same name, case ignored. | this spec section 6 | `ambre_trame::logic::tests::group3::created_sheet_never_repeats_a_name`; `::rename_to_a_taken_name_is_refused`; `::concurrent_duplicate_names_read_apart`; `::created_sheet_never_repeats_a_name_whatever_its_case`; `::rename_to_a_name_taken_in_another_case_is_refused`; `::concurrent_names_differing_in_case_read_apart`; `ambre_trame::message::command::tests::a_sheet_name_key_ignores_case` | none | verified |
| `TRAME-079` | `OBS` | The tab strip MUST show every sheet of the document, in the document's order. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs`; `::first_scroll_viewport_hydrates_sheet_list_once` -- the order the Logic thread publishes; `ambre_trame::tests::chrome_ui::sheet_tabs_render_active_and_emit_intents` -- the strip drawing one tab per entry in that order, what joins the two halves being the launcher mirroring the list into the strip's signals | none | verified |
| `TRAME-071` | `CONC` | Two users adding a sheet at once MUST each keep theirs. | this spec section 2, row 1 | `ambre_trame::logic::tests::group11::two_peers_adding_a_sheet_keep_both` | none | verified |
| `TRAME-074` | `CONC` | Two users reordering tabs at once MUST converge on one order. | this spec section 6 | `ambre_trame::logic::convergence_tests::concurrent_tab_moves_converge_on_one_order` | none | verified |
| `TRAME-077` | `CONC` | Two users deleting the last two sheets at once MUST still leave one. | this spec section 6 | `ambre_trame::logic::tests::group11::two_peers_deleting_the_last_two_sheets_keep_one` -- both read the default sheet, and the merge writes nothing | none | verified |
| `TRAME-235` | `OBS` | A stored name `TRAME-234` refuses MUST be shown and resolved as `Sheet` followed by its tab's position. | this spec section 5 | `ambre_trame::logic::tests::group3::a_stored_name_the_rule_refuses_reads_as_its_fallback` -- the tab, a formula naming it, and a stored name claiming first | none | verified |
| `TRAME-241` | `INV` | Reading a stored name `TRAME-234` refuses MUST leave it in the document until a rename replaces it. | this spec section 5 | `ambre_trame::logic::tests::group3::a_stored_name_the_rule_refuses_reads_as_its_fallback` -- the document after the read, and after the rename | none | verified |
| `TRAME-236` | `INV` | A formula's sheet qualifier MUST resolve whatever its case. | this spec section 5 | `ambre_trame::logic::tests::group3::a_qualifier_resolves_whatever_its_case`; `ambre_trame::formula::render::tests::an_apostrophe_in_a_name_round_trips` -- a name with an inner apostrophe | none | verified |

### 1.2 Add a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-159` | `FRAME` | Adding a sheet MUST NOT change which sheet the reader is on. | this spec section 4 | `ambre_trame::logic::tests::group3::create_sheet_lists_and_isolates` -- it asserts the active id after the create, the new tab never being activated because nothing sends the second command the create leaves to the caller (section 6) | none | verified |
| `TRAME-232` | `FRAME` | Adding a sheet MUST NOT change what a formula on another sheet reads. | this spec section 4 | `ambre_trame::logic::tests::group3::a_new_sheet_leaves_a_cross_sheet_formula_on_its_sheet` -- the new id sorts before the formula's own sheet, so its engine index moves | none | verified |

### 1.3 Rename a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-160` | `PRE` | The name a rename is given MUST NOT be empty. | this spec section 4 | `ambre_trame::message::command::tests::a_sheet_name_follows_the_excel_rule` -- `RenameSheet` carries a `SheetName`, and `SheetName::new` is the only constructor that gives one | none | verified |
| `TRAME-070` | `POST` | A sheet MUST keep its identity through a rename. | this spec section 3 | `ambre_trame::logic::tests::group3::rename_sheet_keeps_id_and_cells`; `::cross_sheet_reference_converges_across_peers_on_rename` | none | verified |
| `TRAME-161` | `FRAME` | A rename MUST NOT move the sheet in the strip. | this spec section 4 | `ambre_trame::logic::tests::group11::a_rename_keeps_the_tab_order` | none | verified |

### 1.4 Name a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-234` | `PRE` | The name a create or a rename is given MUST have 1 to 31 characters, none of `: \ / ? * [ ]`, and no apostrophe at either end. | this spec section 4 | `ambre_trame::message::command::tests::a_sheet_name_follows_the_excel_rule`; `::a_numbered_variant_is_a_valid_name` -- the free variant of a create; `ambre_trame::tests::chrome_ui::sheet_tabs_refuse_a_name_the_rule_refuses` -- the strip | none | verified |

### 1.5 Drag a tab to another slot

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-073` | `POST` | Reordering the tabs MUST put the sheet at the slot it was dropped on. | this spec section 4 | `ambre_trame::logic::tests::group3::move_sheet_reorders_tabs` | none | verified |
| `TRAME-162` | `FRAME` | Reordering the tabs MUST NOT change a cell, a style or a formula. | this spec section 4 | `ambre_trame::logic::tests::group11::a_tab_reorder_changes_no_content` | none | verified |

### 1.6 Delete a sheet

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-163` | `FAIL` | Deleting the last remaining sheet MUST be refused, with a message and no write. | this spec section 4 | `ambre_trame::logic::tests::group3::delete_last_sheet_rejected` -- it asserts both the error event and the absence of an update | none | verified |
| `TRAME-075` | `POST` | Deleting the sheet a user is reading MUST leave that user on another one. | this spec section 2, row 3 | `ambre_trame::logic::tests::group3::delete_active_sheet_falls_back`; `ambre_trame::logic::tests::group9::a_peer_delete_of_the_sheet_being_read_leaves_the_reader_on_another` | none | verified |
| `TRAME-076` | `POST` | Deleting a sheet MUST leave none of its data in the document. | this spec section 6 | `ambre_trame::logic::tests::group3::deleted_sheet_leaves_no_data` | none | verified |
| `TRAME-164` | `FRAME` | Deleting a sheet MUST NOT touch another sheet's data. | this spec section 5 | `ambre_trame::logic::tests::group11::a_sheet_delete_leaves_the_other_sheets` | none | verified |

### 1.7 Switch to another tab

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-165` | `FRAME` | Switching tabs MUST NOT change the document, and MUST reach no collaborator. | this spec section 3 | `ambre_trame::logic::tests::group11::a_tab_switch_writes_nothing` | none | verified |

* **Budget.** none of its own: what opening a sheet costs is the `SheetIndex` rebuild of
  `ambre-spec-budgets.md section 1`, and that budget belongs to
  `features/ambre-spec-trame-engine.md`. A tab switch is a repaint, inside the frame phases of
  `ambre-spec-budgets.md section 2`.
* **Boundary.** none of its own: a sheet create, rename, reorder or delete is a document write, and
  `ZK-001` seals every payload before it leaves the client.
* **Verification.** 23 `verified`. What holds is each gesture on one peer, and what it leaves
  alone. Three concurrent cases hold too: two adds (`TRAME-071`), a delete under a reader
  (`TRAME-075`), and two deletes of the last two sheets (`TRAME-077`).

---

## 2. WHAT EACH USER SEES

| A does | B does | Converged document | What each user sees | Test |
|---|---|---|---|---|
| adds a sheet | adds a sheet | both, each under its own id | two new tabs, in a canonical order (`TRAME-071`) | none. The convergence op set carries no sheet command |
| renames `Sheet2` to `Data` | writes `=Sheet2!A1` | the reference, stored by sheet id | both read the value, and both see the qualifier `Data!A1` (`TRAME-070`) | `ambre_trame::logic::tests::group3::cross_sheet_reference_converges_across_peers_on_rename` |
| deletes `Sheet2` | is reading `Sheet2` | the workbook without it | B lands on the neighbouring tab (`TRAME-075`) | `ambre_trame::logic::tests::group9::a_peer_delete_of_the_sheet_being_read_leaves_the_reader_on_another` |
| deletes the last sheet but one | deletes the other | no sheet left | one sheet, the default one, on both screens, and the merge writes nothing (`TRAME-022`, `TRAME-077`) | `ambre_trame::logic::tests::group11::two_peers_deleting_the_last_two_sheets_keep_one` |
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
| move | a refreshed tab strip, and nothing else |
| create | a full reseed: the engine numbers the sheets by sorted id, so a new id shifts the sheets after it, and a compiled reference holds the old number (`TRAME-232`) |
| rename | a full reseed, which re-resolves every qualifier that names the sheet |
| delete | a full reseed: a removed id breaks every formula that pointed into it (`TRAME-009`) |
| activate | a full reseed as well, which it should not be -- the engine pins the active sheet to its own index zero (section 6) |

**Code.**

* `logic/sheets.rs` -- `create_sheet`, `rename_sheet`, `move_sheet`, `delete_sheet`,
  `fallback_sheet`, `leave_a_deleted_sheet`, `drop_sheet_containers`, `rebuild_sheet_maps`;
* `logic/sync.rs` -- `reseed_after_merge`, which leaves a sheet a merge deleted;
* `ui/view/interact.rs` and `trame_chrome` -- the strip.

---

## 4. SURFACE

The tab strip is DOM chrome in the status bar, before the zoom (`ambre-spec-shell.md`, `CHROME-162`),
not painted on the canvas.

| Part | Behaviour |
|---|---|
| a tab | the sheet's name. The active one is marked, and a click activates its sheet |
| the `+` button | adds a sheet at the end of the strip, and leaves the reader on the sheet they were on (`TRAME-159`) |
| a double-click on a tab | renames it in place. A name the rule refuses keeps the editor open, its reason on the field (`TRAME-234`) |
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
| A workbook whose map holds no sheet reads the default one, and its first edit stores it | each peer re-inserting a sheet when a merge leaves none | a merge would write. A reader cannot write, so it would read an empty workbook. Two peers would each push the sheet on the order list and converge on a duplicate. The read costs nothing and is the same on every peer (`TRAME-077`) |
| A stored name the rule refuses reads as `Sheet` and its tab's position | showing it as stored | a tab would carry a name no rename could give, and a formula could not always spell it back (`TRAME-235`) |
| The fallback is a read, and the document keeps the stored name | repairing it on read, by cutting it and replacing its refused characters | a reader cannot write, and two peers that repair at once would each write a fix and elect one at random. A rename is the one write that fixes the name (`TRAME-241`) |
| A qualifier resolves whatever its case | resolving the exact spelling only | uniqueness ignores case, so a case-folded qualifier names one sheet at most, and Excel resolves it the same way (`TRAME-236`) |
| Deleting a sheet breaks its references | resolving the reference to empty, as a deleted row does | a formula pointing at deleted data has no right answer, and inventing one hides the deletion from the reader |
| Delete is the one sheet operation a reference can feel | rewriting the name token in every dependent formula on rename, as Excel does | a rename changes a label the reader sees and a reorder changes the strip, so neither touches the dependency graph; only a delete removes a `sheet_id`. Excel's rename-rewrite carries the same hazard as its row-insert rewrite -- two concurrent rewrites of one formula converge on a third -- and binding to the id leaves `#REF!` meaning one thing (`TRAME-009`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The `+` button does not open the sheet it adds** (`TRAME-159`). `create_sheet` leaves the
  active sheet alone by design, and expects the surface to send `ActivateSheet` behind it. The
  launcher does not, so a user who clicks `+` gets a tab they have to click again. Nothing rules
  which of the two is wrong.
* **Switching a tab is a full engine reseed.** The engine pins the active sheet to its own index
  zero, so activating another one remaps every compiled reference. It is engine debt
  (`ambre-spec-trame-engine.md`). A named range escapes it by storing its sheet id (`TRAME-062`).
