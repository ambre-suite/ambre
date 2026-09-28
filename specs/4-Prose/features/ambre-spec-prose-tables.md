# AMBRE -- FEATURE: TABLES

**Type:** Vertical (feature)
**Implementation:** partial -- every command is built; nothing paints a table yet

> A grid of cells inside the document. Cells hold ordinary text and ordinary paragraphs, and a
> merged region is described by its two corners. A row inserted at the same moment widens the
> region instead of tearing it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-009` | `INV` | A merged region MUST be held as its two corner cells alone: a row count, a column count and a grid position MUST NOT be stored, and the span MUST be worked out from the grid the table has now. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::logic::tables::tests::a_region_is_stored_as_two_corners_and_a_stamp_alone`; `::a_region_a_delete_crosses_keeps_its_corners_and_narrows` | none | verified |
| `PROSE-FEAT-227` | `INV` | Every table command MUST leave each row of its table holding one cell per column. | this spec section 3 | `ambre_prose::logic::tables::tests::a_new_table_holds_one_cell_per_column_in_every_row`; `::every_table_command_leaves_every_row_one_cell_per_column` | none | verified |
| `PROSE-FEAT-044` | `INV` | The stored column widths MUST hold exactly one entry per column. | this spec section 3 | `ambre_prose::logic::tables::tests::every_table_command_leaves_every_row_one_cell_per_column`; `::a_column_insert_splices_one_width_beside_the_cells`; `::a_column_delete_removes_one_width_beside_the_cells` | none | verified |
| `PROSE-FEAT-228` | `INV` | A table of no columns MUST NOT exist. | this spec section 3 | `ambre_prose::logic::tables::tests::a_table_of_no_row_or_no_column_is_refused`; `::deleting_the_last_column_deletes_the_table`; `::deleting_the_last_row_deletes_the_table` | none | verified |
| `PROSE-FEAT-229` | `INV` | Which regions stand MUST be decided by the merge map and the liveness of the cells alone. | this spec section 3 | `ambre_prose::logic::tables::tests::the_regions_that_stand_are_decided_by_the_records_and_the_grid` | none | verified |
| `PROSE-FEAT-017` | `CONC` | A row or column insert MUST converge with a concurrent edit inside the table. | this spec section 3 | `ambre_prose::logic::convergence_tests::a_row_insert_and_a_concurrent_edit_in_the_table_converge`; `::a_column_insert_and_a_concurrent_edit_in_the_table_converge` | none | verified |
| `PROSE-FEAT-230` | `CONC` | A row or column inserted between a region's two corners MUST leave one region on those same two corners, widened by what fell between them. | this spec section 3 | `ambre_prose::logic::convergence_tests::a_row_inserted_between_the_corners_widens_the_region_on_both_peers`; `::a_column_inserted_between_the_corners_widens_the_region_on_both_peers` | none | verified |
| `PROSE-FEAT-050` | `CONC` | Two merges covering one cell at the same moment MUST resolve to a single region, identically on every peer. | this spec section 3 | `ambre_prose::logic::convergence_tests::two_merges_over_one_cell_at_once_resolve_to_one_region_on_both_peers` | none | verified |
| `PROSE-UI-147` | `OBS` | A covered cell MUST NOT be painted: a region MUST be drawn as one cell. | this spec section 3 | `ambre_prose::view::paint::tests::a_merged_region_is_drawn_as_one_cell_and_a_covered_cell_not_at_all` | none | verified |
| `PROSE-UI-179` | `OBS` | A cell MUST be painted with its stored fill, its four border widths and its border colour. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::view::paint::tests::a_cell_draws_its_fill_and_its_border_and_a_plain_one_the_hairline` | none | verified |
| `PROSE-UI-180` | `OBS` | A cell's text MUST sit inside its stored padding and at its stored vertical alignment, and the row height MUST count the padding. | this spec section 3; `ambre-spec-prose-schema.md section 3` | `ambre_prose::view::paint::tests::the_padding_insets_the_text_and_the_alignment_places_it` | none | verified |
| `PROSE-UI-181` | `OBS` | A cell that stores no border MUST be drawn with the theme's hairline, so the grid never vanishes. | this spec section 3 | `ambre_prose::view::paint::tests::a_cell_draws_its_fill_and_its_border_and_a_plain_one_the_hairline` | none | verified |

### 1.2 Insert a row or a column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-231` | `POST` | A column insert MUST splice the matching width entry in the same operation that writes the cells. | this spec section 3 | `ambre_prose::logic::tables::tests::a_column_insert_splices_one_width_beside_the_cells` | none | verified |
| `PROSE-FEAT-232` | `POST` | A column insert carrying no width MUST take the document's default width. | this spec section 3 | `ambre_prose::logic::tables::tests::a_column_insert_carrying_no_width_takes_the_default` | none | verified |
| `PROSE-FEAT-233` | `FAIL` | An insert naming a column index past the grid MUST be refused, and the refusal MUST be reported. | this spec section 3 | `ambre_prose::logic::tables::tests::a_column_insert_past_the_grid_is_refused_and_reported` | none | verified |
| `PROSE-FEAT-234` | `FRAME` | An insert MUST NOT change the width of an existing column, the text of an existing cell, or either corner of a region. | this spec section 3 | `ambre_prose::logic::tables::tests::an_insert_leaves_widths_texts_and_corners_alone` | none | verified |

### 1.3 Delete a row or a column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-235` | `POST` | A column delete MUST remove the matching width entry in the same operation that removes the cells. | this spec section 3 | `ambre_prose::logic::tables::tests::a_column_delete_removes_one_width_beside_the_cells` | none | verified |
| `PROSE-FEAT-049` | `POST` | A merged region MUST be dropped when either of its corners is deleted. | this spec section 3 | `ambre_prose::logic::tables::tests::deleting_a_corner_drops_the_region_and_frees_its_cells`; `::a_column_delete_drops_a_region_cornered_in_it` | none | verified |
| `PROSE-FEAT-236` | `POST` | The cells a dropped region covered MUST become ordinary cells again. | this spec section 3 | `ambre_prose::logic::tables::tests::deleting_a_corner_drops_the_region_and_frees_its_cells` | none | verified |
| `PROSE-FEAT-237` | `POST` | A region the deleted row or column merely crosses MUST keep both corners and narrow by one. | this spec section 3 | `ambre_prose::logic::tables::tests::a_region_a_delete_crosses_keeps_its_corners_and_narrows` | none | verified |
| `PROSE-FEAT-045` | `POST` | Deleting the last column MUST delete the whole table. | this spec section 3 | `ambre_prose::logic::tables::tests::deleting_the_last_column_deletes_the_table` | none | verified |
| `PROSE-FEAT-293` | `POST` | Deleting the last row MUST delete the whole table. | this spec section 3 | `ambre_prose::logic::tables::tests::deleting_the_last_row_deletes_the_table` | none | verified |
| `PROSE-FEAT-238` | `FRAME` | A delete MUST NOT change the width of a surviving column, or the text of a surviving cell. | this spec section 3 | `ambre_prose::logic::tables::tests::a_delete_leaves_surviving_widths_and_texts_alone` | none | verified |

### 1.4 Resize a column

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-239` | `POST` | A resize MUST rewrite the width entry of the column it names. | this spec section 3 | `ambre_prose::logic::tables::tests::a_resize_rewrites_one_width_and_nothing_else` | none | verified |
| `PROSE-FEAT-240` | `FRAME` | A resize MUST NOT change the number of width entries, the width of another column, or the text of any cell. | this spec section 3 | `ambre_prose::logic::tables::tests::a_resize_rewrites_one_width_and_nothing_else` | none | verified |

### 1.5 Merge cells

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-047` | `FAIL` | A merge whose covered cells are not a solid rectangle MUST be refused. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_over_a_ragged_grid_is_refused` | none | verified |
| `PROSE-FEAT-046` | `FAIL` | A merge MUST be refused while any covered cell holds text, and nothing MUST be written. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_over_a_cell_holding_text_is_refused_and_writes_nothing` | none | verified |
| `PROSE-FEAT-241` | `POST` | The grid positions the command carries MUST be resolved to two corner block ids before anything is written. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_resolves_its_positions_to_two_corner_ids` | none | verified |
| `PROSE-FEAT-242` | `POST` | Every existing region the covered set intersects MUST be removed: a merge supersedes the merges it overlaps. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_supersedes_the_regions_it_overlaps` | none | verified |
| `PROSE-FEAT-243` | `POST` | A merge MUST write one entry, keyed on the anchor's block id. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_writes_one_entry_keyed_on_the_anchor` | none | verified |
| `PROSE-FEAT-048` | `POST` | A covered cell's text MUST survive the merge. | this spec section 3 | `ambre_prose::logic::convergence_tests::a_merge_over_text_typed_at_once_masks_it_and_reports_it`; `::unmerging_brings_a_masked_text_back_verbatim` | none | verified |
| `PROSE-FEAT-244` | `FRAME` | A merge over a selection of one cell MUST NOT write to the document. | this spec section 3 | `ambre_prose::logic::tables::tests::a_merge_of_one_cell_writes_nothing` | none | verified |

### 1.6 Unmerge a region

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-245` | `POST` | An unmerge MUST remove the region the named cell anchors, or the one covering it. | this spec section 3 | `ambre_prose::logic::tables::tests::an_unmerge_from_any_cell_of_a_region_removes_it` | none | verified |
| `PROSE-FEAT-246` | `POST` | A covered cell's text MUST reappear verbatim when the region is unmerged. | this spec section 3 | `ambre_prose::logic::convergence_tests::unmerging_brings_a_masked_text_back_verbatim` | none | verified |
| `PROSE-FEAT-247` | `FRAME` | An unmerge of a cell that neither anchors a region nor is covered by one MUST NOT write to the document. | this spec section 3 | `ambre_prose::logic::tables::tests::an_unmerge_of_a_cell_in_no_region_writes_nothing` | none | verified |

### 1.7 Style a cell

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-248` | `POST` | A cell style MUST be written on the cell the command names, and on no other. | this spec section 3 | `ambre_prose::logic::tables::tests::a_cell_style_lands_on_the_named_cell_alone` | none | verified |
| `PROSE-FEAT-294` | `FAIL` | A colour past 24 bits, or a width or a padding that is not a finite number above zero, MUST be refused, and nothing MUST be written. | this spec section 3 | `ambre_prose::logic::tables::tests::a_colour_past_24_bits_or_a_bad_width_or_padding_is_refused` | none | verified |
| `PROSE-FEAT-249` | `FRAME` | Styling a cell MUST NOT change its text, the column widths, or the merge map. | this spec section 3 | `ambre_prose::logic::tables::tests::a_cell_style_lands_on_the_named_cell_alone` | none | verified |

### 1.8 Move the caret to another cell

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-250` | `POST` | Traversal MUST land on a region's anchor and skip the cells it covers. | this spec section 4 | `ambre_prose::logic::tables::tests::traversal_lands_on_an_anchor_and_skips_what_it_covers` | none | verified |
| `PROSE-FEAT-051` | `FRAME` | Moving the caret between cells MUST NOT change the document. | this spec section 3 | `ambre_prose::logic::tables::tests::stepping_between_cells_writes_nothing` | none | verified |

### 1.9 Move the caret past the last cell

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-251` | `POST` | Moving past the last cell MUST append a row and put the caret in its first cell. | this spec section 3 | `ambre_prose::logic::tables::tests::stepping_past_the_last_cell_appends_a_row_and_enters_it` | none | verified |
| `PROSE-FEAT-252` | `FRAME` | Appending that row MUST NOT change the text of an existing cell, the column widths, or the merge map. | this spec section 3 | `ambre_prose::logic::tables::tests::stepping_past_the_last_cell_appends_a_row_and_enters_it` | none | verified |

### 1.10 Validate the table after a remote change

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-FEAT-253` | `POST` | Each region whose anchor or extent no longer resolves to a live cell MUST be dropped. | this spec section 3 | `ambre_prose::logic::tables::tests::a_region_whose_corner_is_gone_is_dropped_by_validation` | none | verified |
| `PROSE-FEAT-254` | `POST` | Two regions covering one cell MUST resolve to the later of the two, and the other MUST be dropped. | this spec section 3 | `ambre_prose::logic::tables::tests::of_two_regions_over_one_cell_the_later_stands` | none | verified |
| `PROSE-FEAT-255` | `POST` | A cell found covered while it still holds text MUST be reported to the view. | this spec section 3 | `ambre_prose::logic::convergence_tests::a_merge_over_text_typed_at_once_masks_it_and_reports_it` | none | verified |
| `PROSE-FEAT-256` | `FRAME` | The pass MUST NOT change the text of a cell, and MUST NOT hold a change back on what it reports. | this spec section 3 | `ambre_prose::logic::convergence_tests::a_merge_over_text_typed_at_once_masks_it_and_reports_it` | none | verified |

### 1.11 Insert or move a block

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-040` | `FAIL` | An insert or a move whose result breaks the nesting rules of the document MUST be refused, and the refusal MUST be reported. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::blocks::tests::a_table_row_is_refused_at_the_root`; `::a_block_moved_under_its_own_descendant_is_refused`; `::an_insert_after_a_table_row_does_not_land_a_paragraph_under_the_table`; `::a_move_after_a_table_row_does_not_land_a_paragraph_under_the_table` | none | verified |

* **Budget.** none of its own: a table is measured with the rest of the page, the pagination row of
  `ambre-spec-budgets.md section 1`. Resolving a merged region to a rectangle is part of that
  measurement, and `features/ambre-spec-prose-pagination.md section 2` owns it.
* **Boundary.** none of its own: a cell is a block like any other, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 47 `verified`: every command and the painter are built and tested in
  `crates/ambre-prose`.
  `PROSE-FEAT-017`, `-050` and `-230` run on two peers that merge their deltas; the rest a single
  writer reaches. `PROSE-009` also holds by the schema: a stored span would be a key in its table,
  and it has none.

---

## 2. WHAT EACH USER SEES

Two peers in one table. The last column names the test that plays the row, under
`ambre_prose::logic`.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| merges four cells | inserts a row between the two corners at the same moment | the same two corners, one row further apart | one region, one row taller, on both screens | `convergence_tests::a_row_inserted_between_the_corners_widens_the_region_on_both_peers` |
| merges four cells | types in a cell of that region at the same moment | the region, and Bob's text under it | the merged cell; Bob's text returns if the region is unmerged | `convergence_tests::unmerging_brings_a_masked_text_back_verbatim` |
| merges cells one to four | merges cells three to six at the same moment | one of the two regions | the same region on both screens, never a half of each | `convergence_tests::two_merges_over_one_cell_at_once_resolve_to_one_region_on_both_peers` |
| deletes the row holding a region's top-left cell | -- | no region, and the surviving cells | the merge gone, the cells back, on both screens | `tables::tests::deleting_a_corner_drops_the_region_and_frees_its_cells` |
| inserts a column | types in a cell at the same moment | one more column, and Bob's text | the same grid on both screens | `convergence_tests::a_column_insert_and_a_concurrent_edit_in_the_table_converge` |
| deletes the only column left | -- | no table | the table gone, the surrounding text kept | `tables::tests::deleting_the_last_column_deletes_the_table` |
| presses Tab in the last cell | -- | one more row | the caret at the start of a new row, on Alice's screen and in Bob's document | `tables::tests::stepping_past_the_last_cell_appends_a_row_and_enters_it` |

---

## 3. MODEL

**Containers.** None of its own. A table, a row and a cell are three block types of the movable
tree, and their attributes are the node's own. The column widths and the merge map are the table
node's attributes. All four are declared in `ambre-spec-prose-schema.md section 3`, and what may
nest under what is `section 4` of that document.

**A merged region is two corners.** The merge map holds one entry per region, keyed on the
top-left cell's block id. The entry holds the bottom-right cell's block id and a stamp: the Lamport
time of the write. It holds no row count, no column count and no grid position.

The number of rows and columns a region spans is worked out at layout time. The two corners are
found in the grid the table has now (`PROSE-009`). A cell inside the rectangle the two corners
describe is **covered**: it is not painted, it is not measured, and it keeps its text
(`PROSE-FEAT-048`).

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertTable { parent_id, after_id, rows, cols }` | a table of that size, one empty paragraph per cell, every column at the default width |
| `InsertTableRow { table_id, after_row }` | a row of one cell per column, at a place in the table |
| `DeleteTableRow { row_id }` | removes the row, after the merge pass below |
| `InsertTableCol { table_id, at_col, width_pt }` | a cell in every row, and one width entry beside them |
| `DeleteTableCol { table_id, col_index }` | removes the cell in every row and the width entry |
| `ResizeTableCol { table_id, col_index, width_pt }` | rewrites one width entry, and never the count |
| `MergeTableCells { table_id, start, end }` | the two selection corners, resolved to two block ids |
| `UnmergeTableCell { cell_id }` | removes the region that cell anchors, or the one covering it |
| `SetCellStyle { cell_id, bg, valign, border, padding_pt }` | the background, the alignment, the four borders and the padding |
| `StepTableCell { cell_id, backwards }` | Tab and Shift-Tab: the next or previous stop, and a new row past the last one |

`MergeTableCells` carries grid positions, because that is what a selection is. The dispatcher
resolves them to the two corner block ids before it writes, so the *stored* region is never
positional.

**What a column insert and a column delete do, in order:**

1. resolve the target column, and refuse a column index past the grid.
2. insert or remove one cell in **every** row, so the rows keep one length.
3. splice or remove the matching width entry, in the same operation (`PROSE-FEAT-044`). A new
   column takes the document's default width when the command carries none, and the table grows
   wider rather than taking width from its neighbours.
4. on a delete, drop every region whose top-left or bottom-right cell sat in that column
   (`PROSE-FEAT-049`). A region merely crossed by the column keeps both corners and narrows by one.
5. on a delete of the last column or the last row, delete the table node itself (`PROSE-FEAT-045`,
   `PROSE-FEAT-293`).

A row insert and a row delete follow the same five steps, without the width list.

**What a merge does, in order:**

1. resolve the two corners, then the covered set: every cell of the rectangle except the anchor.
2. refuse a covered set that is not a solid rectangle (`PROSE-FEAT-047`).
3. refuse a merge while any covered cell holds text (`PROSE-FEAT-046`). This is the single-peer
   guard, and it is the same one Trame applies (`specs/2-Trame/features/ambre-spec-trame-cell-merges.md`,
   `TRAME-019`).
4. remove every existing region the covered set intersects: a merge supersedes the merges it
   overlaps.
5. write one entry, keyed on the anchor's block id.

A selection of one cell is a no-op. An unmerge of a cell that neither anchors nor is covered is a
no-op too, so unmerging twice costs nothing.

**What validation does after every remote change:**

1. drop each region whose anchor or extent no longer resolves to a live cell (`PROSE-FEAT-049`).
2. resolve two regions covering one cell to the later of the two, and drop the other
   (`PROSE-FEAT-050`). The later is the higher stamp, and the anchor's block id breaks a tie.
3. rebuild the covered set from what survives.

The pass walks every table at the root of the document. It reads the merge map and the liveness of
the cells, and nothing else. That is what makes
every peer land on the same table (`PROSE-FEAT-017`). A cell found covered while it still holds
text is reported to the view as a hint, and it blocks nothing.

**What refuses a shape.** The nesting rules of `ambre-spec-prose-schema.md section 4` are the shape;
this feature is the guard. A command whose result would put a table inside a cell is refused
before the document is touched, and the refusal is reported (`PROSE-040`). So is a block under a
parent that does not accept it. The import path reuses the same predicate rather than a second one, and
lifts an illegal node instead of refusing the whole file
(`features/ambre-spec-prose-images.md section 3`).

**Where a table is measured.** Row height, the split at a row boundary and the rectangle a merged
cell resolves to are `features/ambre-spec-prose-pagination.md section 2`. This document owns the
structure; that one owns the page.

**Code:** `crates/ambre-prose/src/logic/tables.rs` holds the commands and the validation pass;
`crates/ambre-prose/src/doc.rs` holds the readers of the two table attributes.

---

## 4. SURFACE

| Trigger | Behaviour |
|---|---|
| the slash command, or the insert menu | a table of the chosen size, at the caret |
| Tab inside a table | the next cell in reading order, its content selected |
| Shift with Tab | the previous cell, wrapping to the end of the row above |
| Tab in the last cell | a new row, and the caret in its first cell |
| a drag on a column edge | resizes that column, on release |
| the merge entry of the table menu | merges the selected cells, or reports why it refused |
| the unmerge entry | splits the region the caret sits in |
| a row or column entry of the table menu | inserts or deletes at the caret's row or column |

Tab keeps its document-level meaning outside a table. A merged region is one stop for the caret:
traversal lands on the anchor and skips the cells it covers.

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A merged region is stored as two corner block ids | four numbers -- row, column, row span, column span | they are positions, so a concurrent row insert above the region shifts every one of them, and nothing in the document says it happened. A block id survives every insert, delete and move, so the region keeps meaning the same cells whatever happens to the grid around it; the span the reader sees is then a question about today's grid, which cannot be stale because nothing stored it (`PROSE-009`) |
| A row inserted between the corners widens the region | keeping the stored span and pushing the extra row out of the region | it needs a stored count and a rule for what to do with the cell that fell out. The two corners still name the cells the author merged, so the rectangle between them grew; recomputing the span from the corners gives that answer for free, and it is what a spreadsheet does |
| A covered cell is masked, never destroyed | deleting the covered cells | it turns a merge into a data loss no undo of the remote peer can repair. A merge is a display decision, and destroying text to take one would lose a collaborator's sentence written at the same moment; masking makes the merge reversible, and the unmerge is then a removal of one map entry (`PROSE-FEAT-048`) |
| A local merge over non-empty cells is refused | merging anyway and relying on masking | it is the concurrent path's answer to a race, not an answer to a deliberate gesture. The covered content would disappear from the screen with no gesture that says so; refusing lets the chrome tell the author what is in the way, which is the one moment they can act on it (`PROSE-FEAT-046`) |
| A new merge absorbs the merges it overlaps | refusing a merge that overlaps another | it forces an unmerge-then-merge sequence for the commonest correction. Two regions sharing a cell have no meaning, and the author's last gesture is the one they can see; removing the intersecting entries first makes the write a single entry, so validation never meets an overlap it did not create |
| Overlapping concurrent merges resolve to the later entry | asking the author to resolve the conflict | it needs a UI, a pending state in the document, and an answer for the peer who is offline. Both peers apply the same rule to the same map, so both land on the same region without exchanging anything more; the loser's cells reappear rather than vanishing, so nothing is lost by losing |
| Deleting a corner drops the region | keeping the entry and resolving it to the nearest surviving cell | it silently moves a merge the author never moved. The region is its two corners, and one of them is gone, so there is no rectangle left to describe; dropping it and restoring ordinary cells is the outcome the reader can understand from what they did |
| The column widths are one list, kept in step by the same operation | a width per cell | it stores the same number once per row and lets two rows of one column disagree. A width list of a different length than the grid has no correct reading, and the two would drift on the first insert that forgot it; binding them to one operation makes the invariant hold by construction (`PROSE-FEAT-044`) |
| A new column widens the table | redistributing the existing width | it moves every column on every insert. Taking width from the neighbours changes columns the author did not touch, and the arithmetic is different for every neighbour; growing is the behaviour of the popular editors, and it is one number to explain |
| Deleting the last column deletes the table | keeping an empty table node | it leaves an invisible block the caret can enter and nothing can fill. A table of zero columns paints nothing and accepts no text, so it is a state the reader can neither see nor leave; removing it makes the last delete the gesture that removes the table |
| Cell traversal is caret-only, except at the last cell | binding growth to a menu entry alone | it makes the commonest table edit the slowest one. A table is entered from the keyboard, and every stop of the walk that wrote to the document would put a mutation on a key the author presses to look around; the one exception is the growth gesture every word processor binds there (`PROSE-FEAT-051`) |
| Nesting is refused at the command, not documented | stating the nesting rules in the schema alone | it is the shape and not the guard. A rule enforced by prose is enforced by nobody, and the import path is a second writer that never read the prose; one predicate refuses on both paths (`PROSE-040`) |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The table style is not built.** A header row and two banding flags have no key in
  `ambre-spec-prose-schema.md section 3` and no rule on what they paint, so no command writes them.
* **A row insert and a column insert at the same moment leave one row short.** Each peer adds its
  cells to the rows it saw, so the new row lacks the new column's cell. `PROSE-FEAT-227` binds a
  command, not a merge, and nothing repairs the row after the merge.
* **Two column inserts at one place keep no promised order of widths.** The width list and the rows
  are two sequences, and the merge orders each on its own. The counts agree; which width sits on
  which column is not ruled.
* **Nothing rules on a caret inside a cell a merge covers.** Traversal skips a covered cell
  (section 4), and no row says where a caret already sitting in one goes when the merge lands.
* **Nothing rules on the selection inside a table.** Every gesture above needs a rectangle of
  cells, and `features/ambre-spec-prose-selection.md` describes a selection as two positions in the
  text. What a drag across two cells selects is undecided.
* **A repeated header row is deferred**, and a table taller than a page is clipped. Both are gaps
  of `features/ambre-spec-prose-pagination.md section 5`, and both are visible in a table before
  they are visible anywhere else.
* **Nothing bounds a table.** No rule caps the column count, the row count or the smallest column
  width, so a paste can produce a table no page can show.
