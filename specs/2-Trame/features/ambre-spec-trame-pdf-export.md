# AMBRE -- FEATURE: TRAME PDF EXPORT

**Type:** Vertical (feature)

> Trame's half of the shared PDF export (`ambre-spec-pdf-export.md`): the pagination that cuts a
> grid into pages, and the persisted page setup.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-017`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-021` | `INV` | The drawing primitives MUST hold no screen-only state. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the seven primitives under `crates/ambre-trame/src/render/` are generic over `DrawSurface`, and none names a scroll offset or a dirty rect; `ambre_trame::render::tests::draw_cells_on_fills_each_and_counts_nonempty_text`; `::draw_grid_lines_on_strokes_with_grid_color`; `::col_headers_draw_bg_separator_and_active_tint` -- three of them run through a recording surface that is neither screen nor page, and the `TextCache` they take is a shaping cache, not screen state | none | verified |
| `TRAME-IO-023` | `OBS` | Text too wide for its cell MUST be cut at the cell edge on the page, with no ellipsis, exactly as on screen. | this spec section 3; `features/ambre-spec-trame-cell-rendering.md section 4` | `ambre_trame::logic::print::tests::a_page_draws_the_walk_the_screen_draws` -- a label too wide for its column, beside a filled neighbour, draws its whole run inside a clip that ends at the column edge, and the page holds the screen's walk call for call | none | verified |
| `TRAME-IO-085` | `INV` | A page setup MUST leave at least 25.4 mm to print on along each axis. A change that leaves less MUST be refused and write nothing, and a stored blend that leaves less MUST read with the default margins. | this spec section 3 | `ambre_trame::logic::print::tests::margins_that_leave_no_printable_area_are_refused_and_read_as_default` | none | verified |
| `TRAME-IO-086` | `CONC` | Two peers that change two different fields of one sheet's page setup at once MUST both keep both changes. | this spec section 3; `ambre-spec-trame-schema.md section 3` | `ambre_trame::logic::print::tests::two_peers_changing_two_fields_keep_both` -- A3 against landscape, crossed deltas, both read A3 landscape; `ambre_trame::ui::commands::tests::each_page_entry_patches_one_field_of_the_active_sheet` -- each palette entry writes one field | none | verified |

### 1.2 Export a sheet to a page

That a page is drawn by the screen's own walk, is pure vector, is byte-stable and reaches the
browser off the UI thread is `ambre-spec-pdf-export.md section 1.1`. What a sheet adds is a page
format, and an override that must not become an edit.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-022` | `POST` | An export MUST take its page format from the sheet's persisted setup, unless the gesture carries a one-off override. | this spec section 2; section 3 | `ambre_trame::logic::print::tests::an_export_prints_on_the_setup_a_peer_stored` -- Alice stores A3 landscape, Bob reopens her snapshot and his file reads A3 landscape from its `MediaBox`, its twelve columns on one page, and a second sheet reads the default | none | verified |
| `TRAME-IO-031` | `FRAME` | An export MUST NOT change the document, the stored page setup of a one-off override included. | this spec section 2; section 5 | `ambre_trame::logic::print::tests::an_override_prints_once_and_writes_nothing` -- the override prints landscape, no slice answers a delta, the version vector and the stored setup stay, and the next export prints portrait | none | verified |
| `TRAME-IO-084` | `POST` | A sheet that stores no page setup MUST print on A4 in portrait, inside a margin of 12.7 mm on each edge. A field that is absent, or that holds a value no build writes, MUST read as the default of that field. | this spec section 3; `ambre-spec-trame-schema.md section 7` | `ambre_trame::logic::print::tests::a_sheet_with_no_stored_setup_prints_on_a4_portrait` -- the read and the `MediaBox` of each page; `::margins_that_leave_no_printable_area_are_refused_and_read_as_default` -- an unknown paper word reads as A4 | none | verified |
| `TRAME-IO-087` | `POST` | A change to a page setup MUST be one undo item. | this spec section 3; `TRAME-017` | `ambre_trame::logic::print::tests::a_page_setup_change_is_one_undo_item` -- paper and margins in one change, undone and redone together | none | verified |

### 1.3 Cut a range into pages

What a page holds, and in which order the pages run. The paper and the margins are those of the page
format the export prints on (`TRAME-IO-022`).

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-072` | `POST` | A page MUST hold whole columns and whole rows: a break falls on a column edge and on a row edge. A column or a row larger than the printable area alone MUST take a page of its own, which clips it. | this spec section 3 | `ambre_trame::logic::print::tests::breaks_fall_on_row_and_column_edges`; `::a_wide_column_stands_alone_and_a_hidden_row_opens_no_page`; `::bands_cover_the_range_once_and_each_fits` -- property, over random widths; `::a_wide_row_breaks_at_a_column_edge` -- each label reads back whole from the written file | none | verified |
| `TRAME-IO-073` | `POST` | Fit to width MUST shrink a range wider than the page until every column fits one page width. | this spec section 3 | `ambre_trame::logic::print::tests::fit_to_width_shrinks_a_wide_range_and_never_enlarges`; `::a_wide_row_breaks_at_a_column_edge` -- the fitted file holds every label on one page | none | verified |
| `TRAME-IO-078` | `POST` | Fit to width MUST NOT enlarge a range narrower than the page. | this spec section 3 | `ambre_trame::logic::print::tests::fit_to_width_shrinks_a_wide_range_and_never_enlarges` -- two columns keep the zoom of 1 | none | verified |
| `TRAME-IO-079` | `FRAME` | An export MUST NOT move the selection or change the sheet shown. | this spec section 3 | `ambre_trame::logic::print::tests::a_print_leaves_the_selection_and_the_sheet_shown` | none | verified |
| `TRAME-IO-074` | `POST` | With the first row repeated, each page whose rows start below the first row of the range MUST print that row above its own rows. | this spec section 3 | `ambre_trame::logic::print::tests::the_title_row_heads_every_page_below_the_first`; `::the_repeated_row_heads_each_printed_page` -- read back from the written file | none | verified |
| `TRAME-IO-075` | `POST` | An export MUST print the used range of the active sheet, or the selection cut to that range when the gesture asks for it. A range with no cell to print MUST stop with its reason and write no file. | this spec section 3 | `ambre_trame::logic::print::tests::the_selection_prints_alone_and_an_empty_sheet_stops` | none | verified |
| `TRAME-IO-076` | `POST` | An export MUST draw one page per slice of the Logic thread. It MUST land whole before a command other than a scroll or a selection, before a remote delta and before a close. | this spec section 3; `ambre-spec-architecture.md` | `ambre_trame::logic::print::tests::the_repeated_row_heads_each_printed_page` -- three pages in three slices; `ambre_logic::dispatch::tests::a_sheet_pdf_export_answers_with_the_file_and_sends_no_delta` -- one pass of the loop per page; `::an_edit_lands_a_running_sheet_export_first` -- a scroll leaves it running, and an edit lands the file first; structural -- `LogicState::settle_board_import`, which a remote delta and a close run first (`ARCH-031`), lands the export through `settle_trame_print` | none | verified |
| `TRAME-IO-077` | `POST` | The pages MUST run down the rows of a band of columns first, then across to the next band. | this spec section 3 | `ambre_trame::logic::print::tests::breaks_fall_on_row_and_column_edges`; `::a_wide_row_breaks_at_a_column_edge` | none | verified |
| `TRAME-IO-083` | `POST` | A merged block that crosses a page break MUST print on each page it reaches, laid out over the whole block and clipped at that page's edge. | this spec section 3 | `ambre_trame::logic::print::tests::a_merge_across_a_break_prints_its_part_on_each_page` -- a block across a row break and a column break fills one quarter on each of its four pages, and the quarters tile the block | none | verified |

* **Budget.** none: `ambre-spec-budgets.md` budgets no grid export, and the page walk over a large
  sheet is unmeasured until the fixture corpus exists (`ambre-spec-fixtures.md`). The shared
  writer's weight is `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 17 `verified`. The pagination holds whole: a page and its merged blocks, the
  fit, the repeated row, the range, the slices and the view. The stored page format reads its paper
  size back from the written file, and converges over two crossed deltas. `PDF-001` binds the walk
  that draws a page, and `TRAME-IO-021` binds that walk's primitives, which carry no screen state.
  The shared rules are `ambre-spec-pdf-export.md`.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Trame's own turn on the persisted page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the paper size to A3 | exports the sheet | one page format, converged | Bob's file is A3: he did not have to be told (`TRAME-IO-022`) | `print::tests::an_export_prints_on_the_setup_a_peer_stored` |
| overrides the orientation for one export | reads the page setup | the stored format, unchanged | Alice's file is landscape; Bob's setup is what it was (`TRAME-IO-031`) | `print::tests::an_override_prints_once_and_writes_nothing` |
| sets A3 | turns the page landscape at the same moment | both fields, one key each | both see A3 landscape (`TRAME-IO-086`) | `print::tests::two_peers_changing_two_fields_keep_both` |

---

## 3. MODEL

**Containers.** `page_setup_{sheet}` -- the paper size, the orientation and the four margins of one
sheet, one key per field (`ambre-spec-trame-schema.md section 3`). The export owns no other state.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::Interop(Export(TrameExport::Pdf(PdfExport { range, fit, title_row, page })))` | UI to Logic | the range, the fit, the repeated row, and a one-off page format or none |
| `TrameCommand::SetPageSetup { sheet_id, patch }` | UI to Logic | the sheet, the active one when absent, and the fields that change |
| `TrameEvent::Error { refusal: NoPrintableArea }` | Logic to UI | a change refused by `TRAME-IO-085` |
| `TrameEvent::Interop(Progress(..))` | Logic to UI | the pages drawn and the pages in all, after each slice short of the end |
| `TrameEvent::Interop(ExportReady { file, report })` | Logic to UI | the file name, the sheet's name as `DOCFILE-019` and `DOCFILE-020` clean it, and the bytes, on the same path a CSV export uses (`1-Foundations/ambre-spec-interop.md section 3`) |

**The options of one export.** `PrintRange` is the used range or the selection. `PrintFit` is the
size on screen at zoom 1, or the width of the page. `TitleRow` is off, or the first row repeated.
The default of each is the first; the menu entries ask for the width and the repeated row.

**The stored page format.** Each field is one key of `page_setup_{sheet}`, so two peers that
change two fields keep both (`TRAME-IO-086`). A change writes the fields it moves alone.

```rust
pub struct PageSetup {
    pub paper:       PaperSize,       // A4, A3, Letter, Legal; stored as a word
    pub orientation: PageOrientation, // Portrait, Landscape; stored as a word
    pub margins:     PageMargins,     // top, right, bottom, left, in mm, each an f64 key
}

pub struct PageSetupPatch {           // each `Some` replaces its field
    pub paper:       Option<PaperSize>,
    pub orientation: Option<PageOrientation>,
    pub margins:     Option<PageMargins>,
}
```

Each field is valid alone, but two fields from two writers can blend into margins wider than the
paper. A read then keeps the paper and the orientation and takes the default margins, so every peer
reads one format (`TRAME-IO-085`). The scale and the print flags are not stored yet (section 6).

**Steps, on an export** (the shared flow is `ambre-spec-pdf-export.md section 3`; Trame adds the
page format and the pagination, steps 0 to 3):

0. take the override of the gesture, or else read the sheet's stored format (`TRAME-IO-022`). Neither
   is written (`TRAME-IO-031`);
1. resolve the range (`TRAME-IO-075`). It is the used range of the active sheet, or the
   selection cut to it. The used range runs from A1 to the last cell with a value, a style or a
   merged block;
2. compute the zoom (`TRAME-IO-073`, `TRAME-IO-078`). It is 1, unless the gesture fits to width
   and the range is wider than the printable area;
3. cut the range into bands of whole columns, then each band into bands of whole rows. A band takes
   positions while they fit, and always takes one. A hidden row takes no height. After the first
   row band, the repeated row takes its height from each page (`TRAME-IO-072`, `TRAME-IO-074`);
4. each slice packs the cells of one page through the viewport's packer (`TRAME-IO-076`). The pack
   adds the master of each merged block that reaches into the page;
5. the page draws the screen's sheet walk over the region of the repeated row, then over its own
   region (`PDF-001`). A camera places each region on the paper, inside a clip to the printable
   area;
6. the slice that draws the last page hands the recorded file to the PDF module, on the converter
   thread (`ARCH-124`). The answer hands the file over (`PDF-003`).

The export reads the light palette, whatever theme the user shows: a page is printed on white. A
merged block that straddles a break prints its part on each page, clipped there (`TRAME-IO-083`).
Its text is laid out over the whole block, so a label on a break prints cut on both pages.

**Code:**

* `crates/ambre-trame/src/render/` -- the surface-generic primitives and the sheet walk;
* `crates/ambre-trame/src/logic/print.rs` -- the pagination, the slices and the page assembly;
* `crates/ambre-trame/src/logic/page_setup.rs` -- the stored format, its read and its write;
* `crates/ambre-logic/src/data.rs` -- the slices of the loop, the export context and the landing
  before a command.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| `Export PDF`, in the command palette | asks the Logic thread for the used range, fitted to the width, its first row repeated |
| `Export selection as PDF`, in the command palette | the same, over the selection |
| a progress answer | reaches the chrome relay, and changes nothing on the grid |
| the answer | the UI thread wraps the bytes in a browser download (`PDF-003`) |
| `Page setup: portrait` and `Page setup: landscape`, in the command palette | store the orientation of the active sheet |
| `Page setup: A4 paper`, `A3`, `Letter`, `Legal`, in the command palette | store the paper of the active sheet |
| `Page setup: normal margins`, `narrow` (6.35 mm), `wide` (25.4 mm), in the command palette | store the four margins of the active sheet |
| a page setup dialog, with free margins and a one-off override | not built: no gesture carries an override, and no view shows the stored format |

---

## 5. REJECTED ALTERNATIVES

The shared decisions are `ambre-spec-pdf-export.md section 5`: the page drawn by the screen's own
walk, the export in the Logic thread. The pure-vector file and the glyph runs that carry their
source range are shared too. Trame's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The PDF surface belongs to no application | a PDF module inside `ambre-trame` | it is where the design first put it, and it would make the other two applications depend on Trame. The surface translates drawing primitives into a page and knows nothing about a grid |
| The page format is persisted | passing the format with the export command | it is a per-user setting that nobody can see or agree on, and two peers exporting one sheet must produce one layout |
| One key per field of the page format | one register holding the whole format, as `cellstyle_{sheet}` does | a paper size and an orientation set by two peers are two decisions, and one register would drop one. Every field is valid alone, and the one blend that is not, margins too wide, reads back the same on every peer (`TRAME-IO-085`) |
| An override does not write the container | writing the override | it makes every trial print a shared edit. Printing one copy in landscape is not a decision about the document, and it must not reach the other peers |
| Wide text is cut, never elided | adding an ellipsis on the page | it reads better and matches nothing: the page must match the screen, and the screen cuts at the cell edge. An ellipsis on the page alone would break the promise the export exists for |
| Each slice packs its own page, and a command lands the export first | packing the whole range when the export starts, so the walk never waits on the document | a sheet of 100 000 rows would hold every display cell at once, past the memory budget, and the first slice would pay the whole pack. Landing the export before a command keeps the document still at the cost of one wait |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Trame's own:

* **The format has no view.** The palette sets four papers, two orientations and three sets of
  margins. No dialog shows the stored format or takes a free margin. No view sends a one-off override,
  so `TRAME-IO-031` holds for a gesture the logic accepts and no view sends yet.
* **An import keeps no page format.** An Excel or OpenDocument workbook carries a paper, an
  orientation and margins per sheet, and the import drops them. The export writes none back.
* **No scale is stored.** Fit to width is a choice of the gesture, and a fixed print scale has no key.
* **A page prints the grid lines, and never the headers.** The flags of the stored format would
  choose both. No row rules either until that format exists.
* **One sheet a file.** The export prints the active sheet. A workbook printed whole, or a sheet
  printed without showing it, has no command.
* **A text that spills across a column break is cut at the page edge.** The next page starts at
  its own first column, as the screen starts at its first visible column.
* **A merged label sits in the text layer of each page its block reaches.** The clip hides the
  glyphs outside a page, and the file still holds them. A search then finds the label on a page
  that shows none of it.
* **A large range is unmeasured.** No budget row covers a page or a whole export, and no count of
  instructions holds one slice.