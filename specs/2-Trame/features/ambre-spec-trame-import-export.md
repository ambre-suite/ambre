# AMBRE -- FEATURE: TRAME IMPORT AND EXPORT

**Type:** Vertical (feature)

> A user opens a table that came from somewhere else, and hands one back to someone who does not
> use Ambre. Every conversion runs on the user's own machine.

The native `.ambre` file is the only path that keeps everything. Every other format drops something,
and what it drops is the subject of this document.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-001` | `INV` | Every conversion MUST run on the user's own machine, in the Logic thread. | this spec section 3; `ZK-001`, `ZK-004` | cargo-deny -- `deny.toml` lets `ambre-convert` alone declare `csv`, the tables converter module the Logic thread loads and calls (`ARCH-078`); ci-gate -- `scripts/crate_boundaries.py` refuses `csv::` in UI-thread code, fixture `test_a_view_converting_a_file_is_refused` | none | verified |
| `TRAME-IO-033` | `INV` | No plaintext file MUST reach the server: what leaves the device is the sealed document, never the bytes the user opened. | this spec section 3; `ZK-001`, `ZK-004` | `ambre_logic::dispatch::tests::an_import_hands_sync_a_delta_and_never_the_file` -- the Sync worker then seals that delta (`ZK-001`) | none | verified |
| `TRAME-IO-002` | `INV` | Import and export MUST compile to `wasm32-unknown-unknown`: the conversion is client-side, with no server round trip. | this spec section 3 | target-build -- the CI `test-wasm` job runs `cargo test --target wasm32-unknown-unknown` on `ambre-trame`, which compiles the crate and its `csv` dependency for `wasm32-unknown-unknown`, so an interop crate that does not build there fails that job | none | verified |
| `TRAME-IO-034` | `INV` | A reader MUST keep the export: it writes nothing, so the role gate MUST NOT withhold it. | this spec section 3; `ambre-spec-architecture.md section 4`; `INTEROP-006` | `ambre_logic::app::tests::read_only_and_local_commands_are_not_mutating` -- it pins the active-sheet CSV export as non-mutating; `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the CSV and the native exports | none | verified |
| `TRAME-IO-030` | `INV` | A bridge to a third-party spreadsheet MUST seal the content on the user's machine before it enters the document; no plaintext MUST reach Ambre's storage. | this spec section 5; section 6; `ZK-001` | not built | unbuilt -- lands with the `The Google Sheets bridge` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-007` | `IO` | An `.ambre` export/import round trip MUST preserve the document state, byte for byte on a bare document. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`; `::round_trip_preserves_the_document_state` | none | verified |
| `TRAME-IO-004` | `IO` | A CSV import followed by an export MUST round-trip the cell values. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_export_roundtrip`; `::csv_import_then_export_drops_trailing_empties`; `ambre_trame::logic::tests::group7::csv_export_typed_6x10_has_no_trailing_columns` | none | verified |
| `TRAME-IO-019` | `IO` | An xlsx import followed by an export followed by a re-import MUST yield the same values. | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_import_export_reimport_yields_the_same_values` -- a proptest over two sheets of numbers, texts, booleans, parsed and verbatim formulas whose oracle compares each value and each formula text | none | verified |
| `TRAME-IO-029` | `IO` | An OpenDocument import MUST land the same document as the same workbook imported from Excel: same cell states, same report. | this spec section 3; section 6 | not built | unbuilt -- lands with the `OpenDocument spreadsheet interop` Story (`ambre-roadmap.md` E6). One reader reads both formats, so a divergence would be ours | unimplemented |
| `TRAME-IO-052` | `INV` | The xlsx codec MUST build for `wasm32-unknown-unknown` with no browser binding, and only the OOXML codecs and their shared package layer MUST link the zip and XML readers. | this spec section 3; `TRAME-IO-001`, `TRAME-IO-002` | target-build -- `just lint-wasm` runs clippy on `wasm32-unknown-unknown` over every library of the workspace, `ambre-xlsx` among them; cargo-deny -- `deny.toml` lets `ambre-opc` alone declare `zip`, and the three codecs and `ambre-opc` alone declare `quick-xml` | none | verified |
| `TRAME-IO-053` | `IO` | A workbook Trame can hold, written as xlsx and read back, MUST come back unchanged: values, formulas, styles, merges, column widths, row heights and hidden sheets. | this spec section 3; `TRAME-IO-019`, `TRAME-IO-016` | `ambre_xlsx::model_props::a_workbook_trame_can_hold_reads_back_unchanged`; `ambre_xlsx::fixtures::every_fixture_survives_a_write_and_a_read_unchanged` -- twelve files Excel and LibreOffice wrote; `ambre_xlsx::large_sheet::a_sheet_of_100k_cells_reads_back_unchanged` | none | verified |
| `TRAME-IO-054` | `IO` | A Trame formula written in Excel's dialect and read back MUST be the same text. | this spec section 3 | `ambre_xlsx::formula::tests::a_trame_formula_survives_the_round_trip_through_excel`; `::a_newer_function_takes_its_prefix_on_export` | none | verified |

### 1.2 Open a CSV file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-035` | `PRE` | The caller MUST name the separator for anything but a comma, a semicolon or a tab. | this spec section 3 | `ambre_trame::logic::tests::group12::a_separator_outside_the_three_is_named_by_the_caller` | none | verified |
| `TRAME-IO-049` | `PRE` | A separator a command names MUST be an ASCII byte other than the double quote, CR and LF. | this spec section 3 | `ambre_trame::message::command::tests::a_separator_is_an_ascii_byte_that_neither_quotes_nor_ends_a_record` -- `CsvDelimiter::new` is the only way to build one | none | verified |
| `TRAME-IO-006` | `POST` | The separator MUST be auto-detected when the command names none. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_delimiter_autodetect`; `::csv_import_creates_sheet_and_cells` | none | verified |
| `TRAME-IO-036` | `POST` | A header row MUST be dropped when the command declares one. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_skips_header` | none | verified |
| `TRAME-IO-037` | `POST` | An import MUST make the sheet it wrote the active one, so the user lands on what arrived. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_creates_sheet_and_cells` -- it asserts the active sheet is the imported one before reading its cells | none | verified |
| `TRAME-IO-025` | `POST` | An import MUST land as one undo item, whatever the number of cells it writes. | this spec section 3 | `ambre_trame::logic::tests::group12::an_import_is_one_undo_item` | none | verified |
| `TRAME-IO-028` | `FRAME` | A file holding no record MUST leave the document untouched: no sheet, no cell, no undo item. | this spec section 3 | `ambre_trame::logic::tests::group12::a_file_with_no_record_leaves_the_document_alone` | none | verified |
| `TRAME-IO-038` | `FRAME` | An empty field MUST write nothing: an import MUST NOT clear the cell that field covers. | this spec section 3; section 6 | `ambre_trame::logic::tests::group12::an_empty_field_keeps_the_cell_under_it` | none | verified |
| `TRAME-IO-003` | `FAIL` | A malformed or hostile CSV MUST NOT panic. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_malformed_never_panics` -- it feeds nine adversarial shapes through six parameter combinations each; `::csv_import_random_bytes_never_panics` -- four hundred pseudo-random buffers; cargo-fuzz -- `fuzz/fuzz_targets/trame_file.rs`, run by the CI `fuzz` job | none | verified |
| `TRAME-IO-039` | `FAIL` | An import MUST report the records it could not read; it MUST NOT drop them in silence. | this spec section 3; section 6; `INTEROP-003` | `ambre_trame::logic::tests::group7::import_reports_unreadable_records` | none | verified |
| `TRAME-IO-050` | `FAIL` | An import MUST write no field past the last row or the last column of the grid. It MUST report the records and fields it left out. An import that keeps no field MUST leave the document untouched. | this spec section 3; `INTEROP-003` | `ambre_trame::logic::tests::group7::an_import_past_the_grid_edge_is_cut_and_said` -- one record past the last row, one field past the last column; `::an_import_starting_past_the_grid_edge_writes_nothing` -- a start past the last row, then past the last column | none | verified |
| `TRAME-IO-040` | `FAIL` | An import the command queue cannot carry MUST tell the user the file did not land. | this spec section 4; section 6; `BUS-036` | `ambre_app::outbox::tests::a_refusal_past_the_bound_is_kept_for_the_relay_to_tell` -- wasm tier: a command the launcher's outbox refuses past its bound waits for the switchboard, which tells the user once; type-level -- the picker sends through `ShellOutbox`, which holds the one raw sender of the queue (`BUS-035`); ci-gate -- `scripts/crate_boundaries.py` refuses a launcher file other than `outbox.rs` that names that sender | none | verified |
| `TRAME-IO-051` | `FAIL` | An import picked while the command queue is full MUST wait for it and land once it drains; it MUST NOT be lost. | this spec section 4; `BUS-011` | `ambre_belt::trame::tests::an_import_on_a_full_queue_lands_once_it_drains` -- browser tier: a CSV picked while the paused Logic worker leaves the queue full imports once the worker resumes | none | verified |

### 1.3 Hand a sheet back as CSV

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-005` | `POST` | A CSV export MUST write evaluated values, never formula sources. | this spec section 3 | `ambre_trame::logic::tests::group7::csv_export_evaluates_formulas` | none | verified |
| `TRAME-IO-027` | `POST` | An export MUST bound its range by the cells the sheet holds, never by the materialized row and column lists. | this spec section 3; section 5 | `ambre_trame::logic::tests::group6::csv_export_trims_empty_trailing_columns`; `::csv_import_then_export_drops_trailing_empties` | none | verified |
| `TRAME-IO-026` | `FRAME` | A CSV export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | `ambre_trame::logic::tests::group12::a_csv_export_writes_nothing` | none | verified |
| `TRAME-IO-041` | `FAIL` | An export with nothing to write MUST say so, and MUST NOT answer with silence. | this spec section 3; section 6 | `ambre_trame::logic::tests::group7::export_with_nothing_to_write_says_so` | none | verified |

### 1.4 Save the document as a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-042` | `POST` | A saved file MUST name Trame and the document it holds, so opening it dispatches without guessing. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::the_header_names_trame_and_the_document` | none | verified |
| `TRAME-IO-043` | `FRAME` | A save MUST NOT change the document it writes. | this spec section 3 | `ambre_trame::logic::tests::group12::a_save_leaves_the_document_alone` | none | verified |
| `TRAME-IO-044` | `FAIL` | A save whose snapshot fails MUST NOT hand the user a file: a valid header over an empty payload MUST NOT be written. | this spec section 3; section 6 | type-level -- `export_ambre_plaintext` answers `Option<Vec<u8>>` and writes the header inside the branch a read snapshot reaches, so no payload-less file can be constructed. The registry turns the empty answer into the export-failure event | none | verified |

### 1.5 Open a document file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-045` | `POST` | A file that loads MUST show its formula cells as results, never as their source text. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::round_trip_preserves_the_document_state` -- it reads the formula cell back through a viewport request and asserts the result | none | verified |
| `TRAME-IO-008` | `FRAME` | A rejected file MUST leave both the input bytes and the open document untouched. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone` -- it compares the bytes and the snapshot across five rejection shapes | none | verified |
| `TRAME-IO-046` | `FAIL` | A file it cannot read MUST fail as a typed error, never as a panic. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic`; cargo-fuzz -- `fuzz/fuzz_targets/trame_file.rs` wraps each input in a valid header, run by the CI `fuzz` job | none | verified |
| `TRAME-IO-009` | `FAIL` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Trame. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::a_toile_file_is_re_dispatched_not_coerced` | none | verified |

### 1.6 Open an Excel workbook

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-010` | `POST` | An imported Excel formula MUST land in exactly one of three states -- parsed formula, unsupported formula preserved verbatim, or literal -- never a silent mixture. | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_imported_formula_lands_in_exactly_one_of_three_states`; `ambre_trame::logic::xlsx_tests::the_sort_answers_one_state_and_keeps_the_text` -- a proptest over formula texts | none | verified |
| `TRAME-IO-011` | `POST` | An unsupported Excel formula MUST be preserved byte-for-byte and re-exported unchanged; it MUST NOT be downgraded to its last computed value. | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_unsupported_formula_reexports_byte_for_byte` -- a lambda, a 3D reference and an unknown function leave as the file held them | none | verified |
| `TRAME-IO-015` | `POST` | A hidden Excel sheet MUST import as a hidden sheet, never be dropped. | this spec section 3 | `ambre_trame::logic::xlsx_tests::a_hidden_sheet_imports_hidden_and_leaves_hidden` -- its cells feed a formula of a shown sheet, the strip shows no tab, the export writes it hidden | none | verified |
| `TRAME-IO-014` | `POST` | Two cells sharing a style MUST import to a single deduplicated style entry, not to two copies. | this spec section 3; section 6 | not built | unbuilt -- lands with the `The shared style dictionary` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-013` | `FRAME` | An import MUST NOT starve the frame budgets: scrolling MUST stay responsive while it runs. | this spec section 6; `ambre-spec-budgets.md`; `INTEROP-013` | `ambre_trame::logic::xlsx_tests::a_large_import_lands_one_slice_at_a_time` -- a scroll between two slices is served and the import still runs; `ambre_trame::logic::xlsx_tests::an_edit_lands_the_import_first_and_undo_removes_it_whole`; `ambre_logic::pass_tests::a_large_excel_import_lands_over_several_passes` | none | verified |
| `TRAME-IO-012` | `FAIL` | Every ignored Excel feature MUST raise a typed warning; silence MUST NOT be the report. | this spec section 3; section 6; `INTEROP-003` | `ambre_trame::logic::xlsx_tests::every_ignored_excel_feature_is_named_in_the_report` -- a picture, a pivot table and a 1904 workbook: each construct the codec drops or approximates reaches the report under its own name | none | verified |

### 1.7 Hand a workbook back as Excel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-017` | `POST` | On export, a cell reference MUST be rendered at its current position. | this spec section 3; `features/ambre-spec-trame-sheets.md` | `ambre_trame::logic::xlsx_tests::a_reference_leaves_at_its_current_position` -- after a row inserted above the cell read | none | verified |
| `TRAME-IO-048` | `POST` | On export, a cross-sheet reference MUST carry the sheet's display name. | this spec section 3; `features/ambre-spec-trame-sheets.md` | `ambre_trame::logic::xlsx_tests::a_cross_sheet_reference_carries_the_display_name` -- after a rename | none | verified |
| `TRAME-IO-018` | `POST` | On export, a broken reference MUST be written as the error the target format understands, never as an empty cell. | this spec section 3 | `ambre_trame::logic::xlsx_tests::a_broken_reference_leaves_as_the_error_excel_reads` -- a deleted row and a deleted sheet | none | verified |
| `TRAME-IO-047` | `FRAME` | An Excel export MUST NOT change the document it reads, exactly as the CSV export MUST NOT (`TRAME-IO-026`). | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_export_leaves_the_document_as_it_was` -- no operation and no delta | none | verified |

### 1.8 Read a workbook with the xlsx codec

The codec reads a workbook into a plain model and writes the model back. It knows no document and no
gesture: the rows above hold Trame's half, and the rows below hold the codec's.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-055` | `POST` | A shared Excel formula MUST read as one formula per cell it covers, each with its references moved and its anchors kept. | this spec section 3 | `ambre_xlsx::fixtures::a_shared_formula_expands_into_each_cell_it_covers` -- a master read after the cells naming it, an anchored row; `ambre_xlsx::formula::tests::a_shared_formula_shifts_its_relative_references_alone` | none | verified |
| `TRAME-IO-056` | `POST` | A formula Trame's dialect cannot spell MUST keep its Excel text, and the write MUST put that text back byte for byte. | this spec section 3; `TRAME-IO-011` | `ambre_xlsx::fixtures::a_foreign_formula_is_written_back_byte_for_byte`; `ambre_xlsx::formula::tests::a_formula_trame_cannot_spell_stays_excels` | none | verified |
| `TRAME-IO-057` | `POST` | A hidden sheet MUST read as a hidden sheet, and a very hidden one as hidden, reported. | this spec section 3; `TRAME-IO-015` | `ambre_xlsx::fixtures::a_hidden_sheet_stays_hidden_and_a_chart_sheet_is_reported` | none | verified |
| `TRAME-IO-058` | `POST` | A workbook on the 1904 date system MUST read its dates as their serials on the 1900 calendar, and report the move. | this spec section 3 | `ambre_xlsx::fixtures::the_1904_date_system_reads_on_the_1900_calendar` | none | verified |
| `TRAME-IO-064` | `FRAME` | A read MUST leave the bytes it reads as they were, and MUST keep nothing from one read to the next. | this spec section 3 | type-level -- `ambre_xlsx::read` borrows the bytes through a shared reference and is a free function over no state, so it can neither write the bytes nor carry a value across calls | none | verified |
| `TRAME-IO-060` | `FAIL` | Every construct the codec approximates or drops MUST be an entry of its report, counted. | this spec section 3; `TRAME-IO-012`, `INTEROP-003` | `ambre_xlsx::fixtures::every_construct_trame_lacks_is_reported_never_dropped_in_silence`; `ambre_xlsx::fixtures::every_copied_file_reads_as_its_expected_dump` -- the report of each copied file, held against its dump; `ambre_xlsx::report::tests::the_same_construct_twice_is_one_entry_with_its_count` | none | verified |
| `TRAME-IO-061` | `FAIL` | A malformed or hostile file MUST fail as a typed error, never as a panic, and a part past its size cap MUST be refused before it inflates. | this spec section 3 | `ambre_xlsx::model_props::random_bytes_never_panic_the_reader`; `::a_corrupted_workbook_never_panics_the_reader`; `::a_malformed_package_fails_as_a_typed_error`; `ambre_opc::tests::a_part_that_inflates_past_its_bound_is_refused_while_it_inflates`; `::parts_that_inflate_past_the_package_bound_are_refused` | none | verified |
| `TRAME-IO-062` | `FAIL` | A legacy or password-protected Excel file MUST be refused with that reason. | this spec section 3 | `ambre_xlsx::fixtures::a_password_protected_workbook_is_refused_with_its_reason` | none | verified |

### 1.9 Write a workbook with the xlsx codec

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-059` | `FRAME` | One workbook MUST always write one sequence of bytes. | this spec section 3 | `ambre_xlsx::fixtures::one_workbook_writes_one_sequence_of_bytes` | none | verified |
| `TRAME-IO-063` | `FAIL` | A model with no sheet, or with two sheets of one name, MUST write no file. | this spec section 3; `TRAME-IO-041` | `ambre_xlsx::fixtures::a_workbook_without_a_sheet_or_with_two_sheets_of_one_name_writes_nothing` | none | verified |

* **Budget.** the four conversion rows of `ambre-spec-budgets.md section 1`: CSV import of 10k
  lines, CSV export, xlsx import and xlsx export. `TRAME-IO-013` binds the import to the frame
  budgets on top. Every one reads **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. Every conversion stays in the
  Logic thread, no plaintext file reaches the server, no round trip and no third-party bridge
  (`TRAME-IO-001`, `TRAME-IO-033`, `TRAME-IO-002`, `TRAME-IO-030`).
* **Verification.** 53 `verified`, 3 `unimplemented`. What holds is the round trip of the two
  gestures and their frame conditions. An import is one undo item, an empty file writes nothing, and
  an export or a save changes nothing. Every refusal answers, and the browser tier reads a file
  picked on a full queue land once it drains. A gate and `deny.toml` keep every conversion in the
  Logic thread of `ambre-trame`.

---

## 2. WHAT EACH USER SEES

Two peers, one document. An import is an ordinary CRDT write, so it reaches the other peer like any
edit. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a CSV | writes into another sheet | the new sheet and the edit | both, on both screens | none. No property test drives an import (section 6) |
| imports a file | imports the same file | two sheets, each minted by its own peer | two imports, not one merged sheet | none |
| exports a sheet | writes into it | the edit alone: the export writes nothing (`TRAME-IO-026`) | Bob's value; Alice's file holds the state she exported | none |
| imports a CSV, then undoes | reads the sheet | the document as before the import (`TRAME-IO-025`) | the sheet leaves in one step, on both screens | none |
| saves an `.ambre` file | opens it | the same document state (`TRAME-IO-007`) | the same cells, formulas and styles | `logic::docfile::tests::round_trip_preserves_the_document_state` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the cell, sheet and style containers the
schema declares (`ambre-spec-trame-schema.md section 3`); the native export serializes the whole
document and adds nothing to it.

**Commands and events** (`ambre-bus`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::Interop(Import(TrameImport::Csv(CsvImport)))` | UI to Logic | the bytes, an optional delimiter, a header flag, a target sheet, an origin |
| `TrameCommand::Interop(Export(TrameExport::Csv { .. }))` | UI to Logic | the sheet, or none for the active one, an optional delimiter, an optional range |
| `TrameEvent::Interop(ImportCompleted { report })` | Logic to UI | the rows and columns written, and what was left out; the sheet list beside it names the target active |
| `TrameEvent::Interop(ExportReady { file, report })` | Logic to UI | the file name, which `DOCFILE-019` and `DOCFILE-020` hold, the media type and the bytes, for the UI thread to hand to the browser |
| `TrameEvent::Interop(Failed(failure))` | Logic to UI | why nothing was written, or why no file left |

The shapes and the report are the shared ones (`1-Foundations/ambre-spec-interop.md section 3`).
Trame counts a row and a column carried. It drops an unreadable record, a record past the last row
and a field past the last column.

The two Excel commands are `TrameImport::Xlsx` and `TrameExport::Xlsx`. The OOXML converter module
runs the codec and maps its model to and from Trame's form, the `Book` of `ambre-xlsx`. The form
crosses as postcard bytes, a style as the bytes Trame stores, so Main decodes no Excel style. Each
construct the codec reports is an entry of the shared report, `TrameConstruct::Excel`
(`TRAME-IO-012`).

**Steps, on an Excel import:**

1. the module reads the file into the model. A file that does not read answers a failure;
2. mint one sheet id per sheet, and refuse the import with nothing written when the session has
   none left (`TRAME-243`);
3. list every sheet, a hidden one in `hidden_sheets`, so a formula reads a sheet listed after it.
   A taken name takes a free variant, and each formula names its sheet by id;
4. sort each formula into one of three states (`TRAME-IO-010`);
5. write the cells one slice of 256 units per pass, a slice materializing its far corner first. A
   slice short of the end reports its progress (`INTEROP-013`);
6. on the last slice, show the first shown sheet and answer the report.

A scroll between two slices is served at once. Any other command lands the import whole first, so
the import stays one undo step (`TRAME-IO-013`).

**Steps, on a CSV import:**

1. detect the delimiter when the command names none. It is the most frequent of the comma, the
   semicolon and the tab outside quoted fields, in the first kilobyte (`TRAME-IO-035`);
2. read every record, keeping ragged rows, then drop the first when a header is declared;
3. return with nothing written when no record survives (`TRAME-IO-028`);
4. cut the records past the last row and the fields past the last column, counted from the start
   cell, and report the cut. Return with nothing written when the cut keeps no field
   (`TRAME-IO-050`);
5. open one undo group (`TRAME-IO-025`);
6. reuse the named sheet, or mint one and make it active;
7. materialize every target cell, list a minted sheet only then, inside the group, and write the
   non-empty fields as literals;
8. recompute, and report the rows and the columns written, beside the sheet list.

Step 7 is in two passes on purpose: a materialization commit carries a different origin.
Interleaving it with the value writes would split the undo item in two.

**Steps, on a CSV export:**

1. resolve the sheet, and answer nothing when it is unknown;
2. bound the range by scanning the cell map, never by the row and column lists. Those stay
   materialized at their high-water mark after a scroll (`TRAME-IO-027`);
3. write each cell by its displayed value, formula results included (`TRAME-IO-005`);
4. return the bytes as one file, and no delta.

**The Excel cell.** An imported cell lands in one of three states, and the
export reads the state back. A Trame text that parses, calls built-ins alone and names sheets of the
workbook alone is a formula. Any other formula is unsupported, stored as the null byte, `=` and its
Excel text, and shown without the byte:

```rust
/// State of a cell imported from a foreign spreadsheet. `UnsupportedFormula`
/// and `Literal` are never mixed: they re-export differently.
pub enum ImportedCellKind {
    /// Parsed, and executable by the engine.
    Formula(String),
    /// Recognized, using a function the engine does not have. Held verbatim
    /// behind a marker byte no user can type, and restored on export.
    UnsupportedFormula(String),
    /// A value: text, number or boolean.
    Literal(String),
}
```

**What an Excel value becomes:**

| Excel type | What is written |
|---|---|
| integer, float | the number, never in scientific notation |
| date and time | the Excel serial number, never an ISO string: the engine's date functions read serials |
| string | verbatim |
| boolean | `TRUE` or `FALSE` |
| error, empty | nothing: no cell is created |

An Excel import writes through the canonical form a typed cell takes, one slice at a time. The
slice reads the line order of every sheet once, and feeds the engine each cell it wrote. A cell past
the grid, or past the lines one write may add, is counted as a cell past the grid.

**Steps, on an Excel export:**

1. walk every sheet in tab order, a hidden one marked hidden (`TRAME-IO-015`);
2. write each formula in its display form: each reference at its position, each sheet by its name
   (`TRAME-IO-017`, `TRAME-IO-048`). A reference to a deleted line or sheet reads `#REF!`
   (`TRAME-IO-018`). The module derives its Excel text;
3. write an unsupported formula back as the file held it (`TRAME-IO-011`). A value leaves as a
   number, a boolean or a text, and each distinct style leaves once, beside the merges and sizes;
4. return the bytes as one file, and no delta (`TRAME-IO-047`).

**What an Excel file loses:**

| What the file holds | What the import does |
|---|---|
| macros, pivot tables, charts, shapes | drops them, and reports each one (`TRAME-IO-012`) |
| a link to another workbook | keeps the formula verbatim, and drops the link |
| an array formula the engine cannot run | keeps it verbatim, to re-export unchanged (`TRAME-IO-011`) |
| a hidden sheet | keeps it, hidden (`TRAME-IO-015`) |
| a comment on a cell | keeps it, once cell comments exist |
| a validation rule | keeps the kinds the write gate has, and reports the others |
| sheet protection | drops it: Trame has no equivalent yet |
| an image in a cell | drops it |

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `csv` | reads and writes CSV | yes, by `ambre-convert` |
| `ambre-xlsx` | reads and writes `.xlsx`: the codec below | yes, by `ambre-convert`, whose OOXML module alone turns on its `codec` feature. Trame links the model and the report alone |
| `ambre-opc` | the OPC package the DOCX, xlsx and PPTX codecs share: the archive, the content types, the relationships and the bounds of a hostile package | yes, by the three codecs |
| `zip` | reads and writes the OPC package, deflate alone, over `flate2` and `miniz_oxide` | yes, by `ambre-opc` alone (`deny.toml`) |
| `quick-xml` | reads the XML parts as a stream of events | yes, by the three codecs and `ambre-opc` alone (`deny.toml`) |

**The xlsx codec.** `ambre-xlsx` holds pure functions over bytes: `read` and `write`. It links no
browser binding and no thread protocol. The package layer is `ambre-opc`, which the DOCX and PPTX
codecs share. It holds the archive, the content types, the relationships, and one set of bounds
under the client memory budget (`ambre-spec-budgets.md section 6`). A part inflates only when the reader asks
for it, and the sheets read under the bound of one part, 32 MiB.

The model speaks Trame's vocabulary: the font, alignment and colour types of `ambre-core`, and a
mirror of the Trame cell style. So the reader does every approximation, and the map from the
model to the document drops nothing. A workbook past 1 048 576 cells is refused, which holds a
read near 80 MB of model.

**The two formula dialects.** Each formula keeps its Excel text, and its Trame text when the
translation reaches one. A formula without a Trame text stays Excel's (`TRAME-IO-056`):

| Excel | Trame | The codec |
|---|---|---|
| no leading `=` | a leading `=` | adds it on a read, drops it on a write |
| `$A$1`, an anchored reference | no anchor | drops the anchor on a read, and reports it |
| `_xlfn.IFS(`, a function newer than Excel 2007 | `IFS(` | drops the prefix on a read, adds it on a write |
| `#VALUE!` | `#TYPE!` | renames it both ways |
| `#NULL!`, `#NAME?`, `#SPILL!`, `#CALC!` in a formula | none | keeps the formula Excel's |
| `{1,2}`, `T[Col]`, `[1]S!A1`, `S1:S3!A1`, `A1 B1`, `(A1,B1)`, `10%`, `A1#` | none | keeps the formula Excel's |
| a legacy array formula over a range | none | keeps the formula Excel's, with its range |
| a shared formula, stored once for a block | one formula per cell | expands it on a read (`TRAME-IO-055`) |

**What the codec reports.** One entry per construct and outcome, in the words of the shared
report (`INTEROP-003`):

| Outcome | On a read | On a write |
|---|---|---|
| carried | sheet, cell, formula, shared formula, style, merge, column width, row height, hidden sheet | sheet, cell, formula, style, merge, column width, row height, hidden sheet |
| approximated | anchored reference, formula Trame cannot run, very hidden sheet, 1904 date system, alignment, border stroke, pattern fill, double underline | currency format: Trame draws the reader's currency, and the file names the generic sign |
| dropped | chart, chart sheet, picture, shape, pivot table, macro, comment, data validation, conditional format, hyperlink, defined name, table, protection, frozen pane, filter, link to another workbook, rich text runs, diagonal border, gradient fill, font effect, text rotation, hidden row or column, data table | nothing |

**Code:**

* `crates/ambre-trame/src/logic/interop.rs` -- `import_csv`, `export_csv`, `detect_csv_delimiter`;
* `crates/ambre-trame/src/logic/docfile.rs` -- the native pair, over the container
  `ambre-spec-docfile.md` owns;
* `crates/ambre-app/src/platform.rs` -- `open_csv_picker`, the file picker;
* `crates/ambre-trame/src/ui/view/mod.rs` -- `trigger_download`, the browser side.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| the toolbar Import button | a file picker opens, accepting `.csv`; the chosen file is read on the main thread and sent as bytes |
| the toolbar Export button | the Logic thread serializes the active sheet and answers with the bytes |
| the menu entry Import Excel workbook | a file picker opens, accepting `.xlsx` and `.xlsm`; each sheet of the file lands as a new sheet |
| the menu entry Export Excel workbook | the whole workbook leaves as one `.xlsx` file, named after its first sheet |
| the toolbar Save button | the whole workbook leaves as a native `.ambre` file, on the same answer path, and the button's own label says the file is not encrypted (`ambre-spec-docfile.md`, `DOCFILE-001`, `DOCFILE-009`) |
| the answer | the UI thread wraps the bytes in a browser download; no canvas state changes |

The picker is transient: it is never attached to the page, and it lives until the user picks a file
or drops it.

**The native file sits beside the CSV pair, and is not one of them.** The CSV export drops
formulas, styles and every sheet but the active one; the native file drops nothing. Only the labels
carry that difference before the click, which is why the save's names the format and its lack of
encryption. Opening a file is not on this surface: it belongs to the launcher. The file's
header names the application, and at the moment of picking no workbook is open
(`ambre-spec-docfile.md section 4`).

**What the surface cannot reach.** The import command carries a delimiter, a header flag, a target
sheet and an origin. The button sets none of them: it always auto-detects, always treats row one as
data, and always creates a sheet. The export button carries no delimiter and no range, so the
parameterized command has no caller (section 6).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Every conversion runs on the user's machine | converting server-side | it is faster to build and hands the relay the whole document. The pillar is that the server holds ciphertext, and a conversion is the one moment a document exists as plaintext outside the editor |
| One codec of ours reads and writes Excel, over `zip` and `quick-xml` | `calamine` 0.38 to read and `rust_xlsxwriter` 0.102 to write | the pair weighs 431 KB of brotli on wasm32, against 159 KB for the codec. `calamine` reads no column width, no row height and no array formula range, and names no construct it skips. Two models drift, and Trame's dialect is ours in both cases. Both run at the codec's speed. Measured on 2026-11-09, section 6 |
| One codec of ours reads and writes Excel, over `zip` and `quick-xml` | `umya-spreadsheet` 4.0 beta, which reads and writes | it weighs 656 KB of brotli on wasm32, and reads 100k cells in 1.6 s, against 0.1 s for the codec. Measured on 2026-11-09, section 6 |
| One codec of ours reads and writes Excel, over `zip` and `quick-xml` | the Node bindings | they are not Rust and do not run in the Logic thread. The codec is pure Rust and compiles to WASM, which the pillar requires |
| The codec's report mirrors the shared one | the codec names `ambre_bus::interop` | `deny.toml` keeps the bus to the crates that speak the thread protocol, and a codec speaks none. The wiring maps each entry one to one |
| A 1904 workbook moves its dates to 1900 | keeping the 1904 serials | Trame computes on the 1900 serials, so a date would read four years off |
| An unsupported formula is kept verbatim | storing the cached value, which is what a naive import does | a formula the engine cannot run is still the author's work. Downgrading it to its last computed value loses it silently, and the loss is invisible until someone reopens the file in Excel |
| The marker is a byte a user cannot type | a parallel map of preserved formulas | every structural edit would have to reindex it. The preserved text must be distinguishable from an ordinary cell with no second container to hold the flag, and no legal cell content starts with a null byte |
| A date arrives as a serial number | converting to ISO on import | it reads better and computes worse: the engine's date functions compute on serials, so an ISO string would have to be converted back at every read |
| An export writes what the user sees | exporting the formula sources | a spreadsheet leaving Ambre is read by a tool that will not run our engine, so a formula source in a CSV cell is text, not a value |
| The export bounds come from the cells | using the length of the row list | it is the obvious reading and was the defect: the row and column lists stay materialized at the high-water mark of the scroll, so their length pads the file with empty trailing rows a user never wrote |
| Google Sheets is reached through files | calling the API from the Logic thread | a Web Worker cannot call the Google API: the browser blocks the cross-origin call, and the OAuth flow has no place to run |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture shows a hidden sheet again.** An import keeps it hidden, its cells read by the
  formulas, and an export writes it hidden. The gesture lands with the
  `Hide and show a sheet` Story (`ambre-roadmap.md` E6).
* **An Excel text that starts with `=` reads as a formula.** Trame stores a typed text and a value
  in one raw form. So a string such as `=x` lands as the formula Trame reads in it. A string that
  spells a number lands as the number.
* **A width or a height past the last cell of a sheet is dropped, and counted.** The line it sizes
  holds no cell, and a write would add every line up to it.
* **A cell more than 10 000 rows past the last one written is refused, and counted.** The
  materialization cap of one write holds an import too (`TRAME-IO-050`).
* **The last slice of an import costs the reseed of its sheet.** The figures date from 2026-11-10,
  on 10 250 cells in a cloud container (Xeon at 2.1 GHz). The command was `cargo test --release -p
  ambre-trame --lib a_large_import_lands_one_slice_at_a_time`, timed. A slice of 256 cells took 3
  to 6 ms, and the last one 36 ms. A scroll waits behind one slice. No CI job holds the figure.
* **The evaluation of the crates, on 2026-11-09.** Each variant was a `cdylib` that reads and
  writes a workbook. It was built for `wasm32-unknown-unknown` with `opt-level = "z"`, LTO and
  one codegen unit, then compressed by brotli at level 11. The times are host release builds on
  100k cells, in a cloud container.

| Variant | wasm32, raw | wasm32, brotli | Write 100k cells | Read 100k cells |
|---|---|---|---|---|
| `ambre-xlsx`, `zip` with `deflate` | 525 KB | 190 KB | 120 ms | 109 ms |
| `ambre-xlsx`, `zip` with `deflate-flate2` over `miniz_oxide` | 420 KB | 159 KB | -- | -- |
| `calamine`, read alone | 522 KB | 237 KB | -- | 105 ms |
| `rust_xlsxwriter` with its `wasm` feature, write alone | 1012 KB | 261 KB | 119 ms | -- |
| `calamine` and `rust_xlsxwriter` | 1398 KB | 431 KB | -- | -- |
| `umya-spreadsheet` | 2786 KB | 656 KB | -- | 1577 ms |

* **Where the test files come from.** The copied files sit in a folder named after their
  source, each with its `.expected` dump, and `fixtures/THIRD_PARTY.md` names each one. Files named
  after a bug report are left out. `generated/generate.sh` writes every other file from this
  repository.
* **The codec links `zip` with `deflate-flate2`, over `miniz_oxide`.** It reads and writes
  deflate for 159 KB of brotli. The `deflate` feature adds `zopfli` and `zlib-rs` for 31 KB more,
  and buys only a smaller written file. The three OOXML codecs declare the same lines.
* **A column width moves by the default font.** The format counts a width in digits of the
  workbook's font, and the codec reads 7 pixels a digit. A LibreOffice file written in Arial 10
  reads about 6 % narrow.
* **OpenDocument import is a rule, not a design.** The format is named in one dependency row,
  because the Excel reader also reads it, and nothing states what a `.ods` file becomes.
  `TRAME-IO-029` is what its design will be held to: the Excel path decides the answer, and the
  second format may not drift from the first.
* **The Google Sheets bridge is scheduled and undesigned.** A proxy fetching a sheet as JSON would
  be the first component of Ambre to hold a third party's plaintext. The audit that decision
  needs is not written. `TRAME-IO-030` states the boundary the design must respect before that
  audit, so the rule does not wait on it.
* **The import surface reaches none of its parameters.** A user cannot choose a delimiter, declare a
  header row, or import into an existing sheet, though the command accepts all three. `TRAME-IO-006`
  and `TRAME-IO-036` are verified at the command and unreachable at the button.
* **An import into a populated sheet merges, and nothing decides that it should.** An empty field
  writes nothing, so importing a file with a hole over existing values leaves the old value showing
  through. `TRAME-IO-038` states what the code does. Whether a user importing a range expects it
  replaced or merged is undecided, and the button cannot reach the case at all.
* **No property test drives an import.** An import creates a sheet and writes as many cells as a
  paste, and the convergence property drives neither it nor the two exporters. Several rows say
  `none` for that reason, and every row of section 2 does.
* **The performance targets are not benchmarks.** Import and export carry numbers in
  `ambre-spec-budgets.md` with no `criterion` bench behind them, so nothing fails
  when a conversion gets slower.
