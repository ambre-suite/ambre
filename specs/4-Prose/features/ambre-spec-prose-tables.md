# AMBRE -- FEATURE: TABLES

**Type:** Vertical (feature)
**Implementation:** design-only

> A grid of cells inside the document. Cells hold ordinary text and ordinary paragraphs, and a
> merged region is described by its two corners, so a row inserted at the same moment widens the
> region instead of tearing it.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-009` | A merged region's span MUST be derived from its two corner cells; it MUST NOT be stored, so a concurrent row or column insert cannot corrupt it. | this spec section 3; `ambre-spec-prose-schema.md section 3` | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-040` | An insert or a move whose result breaks the nesting rules of the document MUST be refused, and the refusal MUST be reported. | this spec section 3; `ambre-spec-prose-schema.md section 4` | not built -- lands with `ARCH P2` (Epic `E5`) | unimplemented |
| `PROSE-FEAT-017` | A row or column insert MUST converge with a concurrent edit inside the table, and MUST NOT corrupt the geometry of a merged region. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-044` | The stored column widths MUST hold exactly one entry per column: a column insert and a column delete MUST each move that list in the same operation. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-045` | Deleting the last column MUST delete the whole table: a table of no columns MUST NOT exist. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-046` | A merge MUST be refused while any covered cell holds text, and nothing MUST be written. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-047` | A merge whose covered cells are not a solid rectangle MUST be refused. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-048` | A covered cell's text MUST survive the merge and MUST reappear verbatim when the region is unmerged. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-049` | A merged region MUST be dropped when either corner is deleted, and the cells it covered MUST become ordinary cells again. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-FEAT-050` | Two merges covering one cell at the same moment MUST resolve to a single region, identically on every peer. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5); needs a 2-peer convergence regression test, like Trame's `TRAME-022` | unimplemented |
| `PROSE-FEAT-051` | Moving the caret between cells MUST NOT change the document, except at the last cell, where it MUST append a row. | this spec section 3 | not built -- lands with the "Tables, table of contents and word count" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** none of its own: a table is measured with the rest of the page, and the figure is the
  pagination row of `ambre-spec-budgets.md section 1`. Resolving a merged region to a
  rectangle is part of that measurement, and `features/ambre-spec-prose-pagination.md section 3`
  owns it.
* **Boundary.** none of its own: a cell is a block like any other, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 11 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-009` is the row a reviewer can hold without a test, because a stored span would be a key
  in the schema table and the schema table has none. `PROSE-FEAT-050` is the row that needs two
  peers rather than one, and it is the only one of the eleven a single-peer test cannot reach.

---

## 2. WHAT EACH USER SEES

Two peers in one table. No test resolves yet: the last column names the row each row rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| merges four cells | inserts a row between the two corners at the same moment | the same two corners, one row further apart | one region, one row taller, on both screens | none -- `PROSE-009` |
| merges four cells | types in a cell of that region at the same moment | the region, and Bob's text under it | the merged cell; Bob's text returns if the region is unmerged | none -- `PROSE-FEAT-048` |
| merges cells one to four | merges cells three to six at the same moment | one of the two regions | the same region on both screens, never a half of each | none -- `PROSE-FEAT-050` |
| deletes the row holding a region's top-left cell | -- | no region, and the surviving cells | the merge gone, the cells back, on both screens | none -- `PROSE-FEAT-049` |
| inserts a column | resizes another column at the same moment | one more width entry, and Bob's width | the same grid on both screens | none -- `PROSE-FEAT-044` |
| deletes the only column left | -- | no table | the table gone, the surrounding text kept | none -- `PROSE-FEAT-045` |
| presses Tab in the last cell | -- | one more row | the caret at the start of a new row, on Alice's screen and in Bob's document | none -- `PROSE-FEAT-051` |

---

## 3. MODEL

**Containers.** None of its own. A table, a row and a cell are three block types of the movable
tree, and their attributes are the node's own; the column widths and the merge map are the table
node's attributes. All four are declared in `ambre-spec-prose-schema.md section 3`, and what may
nest under what is `section 4` of that document.

**A merged region is two corners.** The merge map holds one entry per region: the top-left cell's
block id, and the bottom-right cell's. It holds no row count, no column count and no grid position.
The number of rows and columns a region spans is worked out at layout time, by finding the two
corners in the grid the table has now (`PROSE-009`). A cell inside the rectangle the two corners
describe is **covered**: it is not painted, it is not measured, and it keeps its text
(`PROSE-FEAT-048`).

**The commands** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Behaviour |
|---|---|
| `InsertTableRow { table_id, after_row }` | a row of one cell per column, at a place in the table |
| `DeleteTableRow { row_id }` | removes the row, after the merge pass below |
| `InsertTableCol { table_id, after_col, width_pt }` | a cell in every row, and one width entry beside them |
| `DeleteTableCol { table_id, col_index }` | removes the cell in every row and the width entry |
| `ResizeTableCol { table_id, col_index, width_pt }` | rewrites one width entry, and never the count |
| `MergeTableCells { table_id, start, end }` | the two selection corners, resolved to two block ids |
| `UnmergeTableCell { cell_id }` | removes the region that cell anchors, or the one covering it |
| `SetCellStyle { cell_id, bg, valign, border, padding_pt }` | the background, the alignment, the four borders and the padding |
| `SetTableStyle { table_id, style }` | the header row and the two banding flags |

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
   (`PROSE-FEAT-049`); a region merely crossed by the column keeps both corners and narrows by one.
5. on a delete of the last column, delete the table node itself (`PROSE-FEAT-045`).

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
   (`PROSE-FEAT-050`).
3. rebuild the covered set from what survives.

The pass reads the merge map and the liveness of the cells, and nothing else, which is what makes
every peer land on the same table (`PROSE-FEAT-017`). A cell found covered while it still holds
text is reported to the view as a hint, and it blocks nothing.

**What refuses a shape.** The nesting rules of `ambre-spec-prose-schema.md section 4` are the shape;
this feature is the guard. A command whose result would put a table inside a cell, or a block under
a parent that does not accept it, is refused before the document is touched, and the refusal is
reported (`PROSE-040`). The import path reuses the same predicate rather than a second one, and
lifts an illegal node instead of refusing the whole file
(`features/ambre-spec-prose-images.md section 3`).

**Where a table is measured.** Row height, the split at a row boundary and the rectangle a merged
cell resolves to are `features/ambre-spec-prose-pagination.md section 3`. This document owns the
structure; that one owns the page.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A merged region is stored as two corner block ids | a block id survives every insert, delete and move, so the region keeps meaning the same cells whatever happens to the grid around it. The span the reader sees is then a question about today's grid, which cannot be stale because nothing stored it (`PROSE-009`) | four numbers -- row, column, row span, column span -- which are positions: a concurrent row insert above the region shifts every one of them, and nothing in the document says it happened |
| A row inserted between the corners widens the region | the two corners still name the cells the author merged, so the rectangle between them grew. Recomputing the span from the corners gives that answer for free, and it is what a spreadsheet does | keeping the stored span and pushing the extra row out of the region, which needs a stored count and a rule for what to do with the cell that fell out |
| A covered cell is masked, never destroyed | a merge is a display decision, and destroying text to take one would lose a collaborator's sentence written at the same moment. Masking makes the merge reversible, and the unmerge is then a removal of one map entry (`PROSE-FEAT-048`) | deleting the covered cells, which turns a merge into a data loss no undo of the remote peer can repair |
| A local merge over non-empty cells is refused | the covered content would disappear from the screen with no gesture that says so. Refusing lets the chrome tell the author what is in the way, which is the one moment they can act on it (`PROSE-FEAT-046`) | merging anyway and relying on masking, which is the concurrent path's answer to a race, not an answer to a deliberate gesture |
| A new merge absorbs the merges it overlaps | two regions sharing a cell have no meaning, and the author's last gesture is the one they can see. Removing the intersecting entries first makes the write a single entry, so validation never meets an overlap it did not create | refusing a merge that overlaps another, which forces an unmerge-then-merge sequence for the commonest correction |
| Overlapping concurrent merges resolve to the later entry | both peers apply the same rule to the same map, so both land on the same region without exchanging anything more. The loser's cells reappear rather than vanishing, so nothing is lost by losing | asking the author to resolve the conflict, which needs a UI, a pending state in the document, and an answer for the peer who is offline |
| Deleting a corner drops the region | the region is its two corners, and one of them is gone: there is no rectangle left to describe. Dropping it and restoring ordinary cells is the outcome the reader can understand from what they did | keeping the entry and resolving it to the nearest surviving cell, which silently moves a merge the author never moved |
| The column widths are one list, kept in step by the same operation | a width list of a different length than the grid has no correct reading, and the two would drift on the first insert that forgot it. Binding them to one operation makes the invariant hold by construction (`PROSE-FEAT-044`) | a width per cell, which stores the same number once per row and lets two rows of one column disagree |
| A new column widens the table | taking width from the neighbours changes columns the author did not touch, and the arithmetic is different for every neighbour. Growing is the behaviour of the popular editors, and it is one number to explain | redistributing the existing width, which moves every column on every insert |
| Deleting the last column deletes the table | a table of zero columns paints nothing and accepts no text, so it is a state the reader can neither see nor leave. Removing it makes the last delete the gesture that removes the table | keeping an empty table node, which leaves an invisible block the caret can enter and nothing can fill |
| Cell traversal is caret-only, except at the last cell | a table is entered from the keyboard, and every stop of the walk that wrote to the document would put a mutation on a key the author presses to look around. The one exception is the growth gesture every word processor binds there (`PROSE-FEAT-051`) | binding growth to a menu entry alone, which makes the commonest table edit the slowest one |
| Nesting is refused at the command, not documented | a rule enforced by prose is enforced by nobody, and the import path is a second writer that never read the prose. One predicate refuses on both paths (`PROSE-040`) | stating the nesting rules in the schema alone, which is the shape and not the guard |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The table style has no key.** `SetTableStyle` writes a header-row flag and two banding flags,
  and the table node of `ambre-spec-prose-schema.md section 3` declares two attributes: the column
  widths and the merge map. Nothing says where the three flags live, and nothing says what they
  paint.
* **A cell background crosses the command plane as a number and is stored as text.**
  `SetCellStyle` carries the background and the border colour as packed integers, and the schema
  stores both as `"#rrggbb"` strings. Nothing states the conversion, and it is the same two-unit
  shape `features/ambre-spec-prose-text-editing.md section 6` records for positions.
* **The refusal has no event.** `PROSE-040`, `PROSE-FEAT-046` and `PROSE-FEAT-047` each require the
  refusal to be reported, and the engine declares one untyped error carrying a code and a message
  (`features/ambre-spec-prose-engine.md section 3`). The lists and the styles features report through the
  same gap.
* **Nothing rules on the selection inside a table.** Every gesture above needs a rectangle of
  cells, and `features/ambre-spec-prose-selection.md` describes a selection as two positions in the
  text. What a drag across two cells selects is undecided.
* **A repeated header row is deferred**, and a table taller than a page is clipped: both are gaps
  of `features/ambre-spec-prose-pagination.md section 5`, and both are visible in a table before
  they are visible anywhere else.
* **Nothing bounds a table.** No rule caps the column count, the row count or the smallest column
  width, so a paste can produce a table no page can show.
