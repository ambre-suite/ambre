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
| `TRAME-IO-001` | `INV` | Every conversion MUST run on the user's own machine, in the Main instance: the mapping in the Logic thread, and the module call on the converter thread. | this spec section 3; `ZK-001`, `ZK-004`; `ARCH-123` | cargo-deny -- `deny.toml` lets `ambre-convert` alone declare `csv`, the tables converter module the converter thread loads and calls (`ARCH-078`); ci-gate -- the rustc driver of `crates/ambre-primitives`, run by `just primitives` in the CI `lint` job, refuses an item of `csv` in UI-thread code, through a re-export too (`ambre_primitives::driver::rules::a_ui_thread_name_under_a_re_export_an_alias_or_a_macro_is_refused`) | none | verified |
| `TRAME-IO-033` | `INV` | No plaintext file MUST reach the server: what leaves the device is the sealed document, never the bytes the user opened. | this spec section 3; `ZK-001`, `ZK-004` | `ambre_logic::dispatch::tests::an_import_hands_sync_a_delta_and_never_the_file` -- the Sync worker then seals that delta (`ZK-001`) | none | verified |
| `TRAME-IO-002` | `INV` | Import and export MUST compile to `wasm32-unknown-unknown`: the conversion is client-side, with no server round trip. | this spec section 3 | target-build -- the CI `test-wasm` job runs `cargo test --target wasm32-unknown-unknown` on `ambre-trame`, which compiles the crate and its `csv` dependency for `wasm32-unknown-unknown`, so an interop crate that does not build there fails that job | none | verified |
| `TRAME-IO-034` | `INV` | A reader MUST keep the export: it writes nothing, so the role gate MUST NOT withhold it. | this spec section 3; `ambre-spec-architecture.md section 4`; `INTEROP-006` | `ambre_logic::app::tests::read_only_and_local_commands_are_not_mutating` -- it pins the active-sheet CSV export as non-mutating; `ambre_logic::app::tests::an_import_writes_and_an_export_reads_in_every_application` -- the CSV and the native exports | none | verified |
| `TRAME-IO-030` | `INV` | A bridge to a third-party spreadsheet MUST seal the content on the user's machine before it enters the document; no plaintext MUST reach Ambre's storage. | this spec section 5; section 6; `ZK-001` | not built | unbuilt -- lands with the `The Google Sheets bridge` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-007` | `IO` | An `.ambre` export/import round trip MUST preserve the document state, byte for byte on a bare document. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`; `::round_trip_preserves_the_document_state` | none | verified |
| `TRAME-IO-004` | `IO` | A CSV import followed by an export MUST round-trip the cell values. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_export_roundtrip`; `::csv_import_then_export_drops_trailing_empties`; `ambre_trame::logic::tests::group7::csv_export_typed_6x10_has_no_trailing_columns` | none | verified |
| `TRAME-IO-019` | `IO` | An xlsx import followed by an export followed by a re-import MUST yield the same values. | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_import_export_reimport_yields_the_same_values` -- a proptest over two sheets of numbers, texts, booleans, parsed and verbatim formulas whose oracle compares each value and each formula text | none | verified |
| `TRAME-IO-029` | `IO` | An OpenDocument import MUST land the same document as the same workbook imported from Excel: same cell states, same report. | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_ods_import_lands_the_same_document_as_the_same_workbook_from_excel` -- LibreOffice wrote both files from one source; `::an_ods_and_an_xlsx_of_one_workbook_land_as_one_document` -- a proptest; `ambre_xlsx::model_props::an_ods_and_an_xlsx_of_one_workbook_read_the_same_workbook_and_report`; `ambre_xlsx::fixtures::an_ods_file_reads_as_the_xlsx_file_libreoffice_wrote_from_the_same_source` | none | verified |
| `TRAME-IO-052` | `INV` | The spreadsheet codecs MUST build for `wasm32-unknown-unknown` with no browser binding, and only the office codecs and their shared package layer MUST link the zip and XML readers. | this spec section 3; `TRAME-IO-001`, `TRAME-IO-002` | target-build -- `just lint-wasm` runs clippy on `wasm32-unknown-unknown` over every library of the workspace, `ambre-xlsx` among them; cargo-deny -- `deny.toml` lets `ambre-opc` alone declare `zip`, and the three codecs and `ambre-opc` alone declare `quick-xml` | none | verified |
| `TRAME-IO-053` | `IO` | A workbook Trame can hold, written as xlsx and read back, MUST come back unchanged: values, formulas, styles, merges, column widths, row heights, hidden sheets and frozen panes. | this spec section 3; `TRAME-IO-019`, `TRAME-IO-016` | `ambre_xlsx::model_props::a_workbook_trame_can_hold_reads_back_unchanged`; `ambre_xlsx::fixtures::every_fixture_survives_a_write_and_a_read_unchanged` -- twelve files Excel and LibreOffice wrote; `ambre_xlsx::large_sheet::a_sheet_of_100k_cells_reads_back_unchanged` | none | verified |
| `TRAME-IO-054` | `IO` | A Trame formula written in Excel's dialect and read back MUST be the same text. | this spec section 3 | `ambre_xlsx::formula::tests::a_trame_formula_survives_the_round_trip_through_excel`; `::a_newer_function_takes_its_prefix_on_export` | none | verified |
| `TRAME-IO-065` | `IO` | A workbook Trame can hold, written as an OpenDocument spreadsheet and read back, MUST come back unchanged: values, formulas, styles, merges, column widths, row heights, hidden sheets and frozen panes. | this spec section 3; `TRAME-IO-053` | `ambre_xlsx::model_props::a_workbook_trame_can_hold_reads_back_unchanged_through_ods`; `ambre_xlsx::fixtures::every_ods_fixture_survives_a_write_and_a_read_unchanged` -- the round trip of each `.ods` input fixture; `ambre_xlsx::large_sheet::a_sheet_of_100k_cells_reads_back_unchanged_through_ods` | none | verified |
| `TRAME-IO-080` | `IO` | Each xlsx and OpenDocument spreadsheet input fixture, imported, exported in its own format and imported again, MUST land as the same document: values, formula texts, styles, merges, row heights and hidden sheets. | this spec section 3; `TRAME-IO-019`, `TRAME-IO-053` | `ambre_trame::logic::xlsx_tests::every_sheet_fixture_lands_the_same_document_after_an_export_and_an_import` -- the 20 xlsx and 3 `.ods` fixtures of the codec but the workbook under a password, each document read as the export maps it | none | verified |
| `TRAME-IO-081` | `IO` | The xlsx or OpenDocument export of a document imported from an input fixture MUST name no loss but a circular reference. | this spec section 3; `TRAME-IO-060` | `ambre_trame::logic::xlsx_tests::an_export_of_a_sheet_fixture_names_no_loss_but_a_circular_reference` | none | verified |

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
| `TRAME-IO-090` | `FAIL` | A CSV file past the bound of `ambre-spec-budgets.md section 6` MUST fail as too large before any work, and MUST leave the document untouched. | this spec section 3; `ARCH-128` | `ambre_trame::logic::tests::group7::a_csv_file_past_the_bound_is_refused_and_one_at_it_imports` -- a file one byte past the bound fails with no delta and no sheet, and a file at the bound imports | none | verified |
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
| `TRAME-IO-088` | `POST` | An Excel or OpenDocument import MUST keep the frozen pane of each sheet. | this spec section 3; `features/ambre-spec-trame-grid-viewport.md section 2` | `ambre_trame::logic::xlsx_tests::a_frozen_pane_imports_frozen_and_leaves_frozen`; `ambre_xlsx::model_props::an_excel_frozen_pane_reads_as_its_rows_and_columns`; `ambre_xlsx::model_props::an_ods_and_an_xlsx_of_one_workbook_read_the_same_workbook_and_report` -- an OpenDocument pane reads as the Excel one | none | verified |
| `TRAME-IO-014` | `POST` | Two cells sharing a style MUST import to a single deduplicated style entry, not to two copies. | this spec section 3; section 6 | not built | unbuilt -- lands with the `The shared style dictionary` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-013` | `FRAME` | An import MUST NOT starve the frame budgets: scrolling MUST stay responsive while it runs. | this spec section 6; `ambre-spec-budgets.md`; `INTEROP-013` | `ambre_trame::logic::xlsx_tests::a_large_import_lands_one_slice_at_a_time` -- a scroll between two slices is served and the import still runs; `ambre_trame::logic::xlsx_tests::an_edit_lands_the_import_first_and_undo_removes_it_whole`; `ambre_logic::pass_tests::a_large_excel_import_lands_over_several_passes` | none | verified |
| `TRAME-IO-012` | `FAIL` | Every ignored Excel feature MUST raise a typed warning; silence MUST NOT be the report. | this spec section 3; section 6; `INTEROP-003` | `ambre_trame::logic::xlsx_tests::every_ignored_excel_feature_is_named_in_the_report` -- a picture, a pivot table and a 1904 workbook: each construct the codec drops or approximates reaches the report under its own name | none | verified |

### 1.7 Hand a workbook back as Excel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-017` | `POST` | On export, a cell reference MUST be rendered at its current position. | this spec section 3; `features/ambre-spec-trame-sheets.md` | `ambre_trame::logic::xlsx_tests::a_reference_leaves_at_its_current_position` -- after a row inserted above the cell read | none | verified |
| `TRAME-IO-048` | `POST` | On export, a cross-sheet reference MUST carry the sheet's display name. | this spec section 3; `features/ambre-spec-trame-sheets.md` | `ambre_trame::logic::xlsx_tests::a_cross_sheet_reference_carries_the_display_name` -- after a rename | none | verified |
| `TRAME-IO-018` | `POST` | On export, a broken reference MUST be written as the error the target format understands, never as an empty cell. | this spec section 3 | `ambre_trame::logic::xlsx_tests::a_broken_reference_leaves_as_the_error_excel_reads` -- a deleted row and a deleted sheet | none | verified |
| `TRAME-IO-082` | `POST` | On export, the default sheet of a new workbook MUST stay out of the file when it holds nothing a user put there and another sheet exists. | this spec section 3; `TRAME-IO-080` | `ambre_trame::logic::xlsx_tests::an_untouched_default_sheet_stays_out_of_the_export` -- beside an imported sheet it stays out, and it leaves when it holds a cell or stands alone | none | verified |
| `TRAME-IO-047` | `FRAME` | An Excel export MUST NOT change the document it reads, exactly as the CSV export MUST NOT (`TRAME-IO-026`). | this spec section 3 | `ambre_trame::logic::xlsx_tests::an_export_leaves_the_document_as_it_was` -- no operation and no delta | none | verified |
| `TRAME-IO-089` | `POST` | An Excel or OpenDocument export MUST write the frozen pane of each sheet that freezes one. | this spec section 3 | `ambre_trame::logic::xlsx_tests::a_frozen_pane_imports_frozen_and_leaves_frozen`; `ambre_xlsx::model_props::a_workbook_trame_can_hold_reads_back_unchanged`; `::a_workbook_trame_can_hold_reads_back_unchanged_through_ods` | none | verified |

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
  Main instance, no plaintext file reaches the server, no round trip and no third-party bridge
  (`TRAME-IO-001`, `TRAME-IO-033`, `TRAME-IO-002`, `TRAME-IO-030`).
* **Verification.** 67 `verified`, 2 `unimplemented`. The rows hold the round trip of the two
  gestures and their frame conditions. An import is one undo item, an empty file writes nothing, and
  an export or a save changes nothing. Every refusal answers, and the browser tier reads a file
  picked on a full queue land once it drains. A gate and `deny.toml` keep every conversion in the
  Main instance, and `ARCH-123` each module call off the Logic thread.

---

### 1.10 Read and write an OpenDocument spreadsheet

The OpenDocument codec reads into the model the xlsx codec reads into, and writes from it. So the
rows of sections 1.6 to 1.9 hold an `.ods` file through the same map. `TRAME-IO-029` and
`TRAME-IO-065` hold the two paths to one state, and the rows below hold what the format changes.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-066` | `POST` | An OpenDocument export MUST carry the workbook the Excel export of the same document carries. | this spec section 3; `TRAME-IO-029` | `ambre_trame::logic::xlsx_tests::an_ods_export_carries_what_the_excel_export_carries` -- a proptest, whose second import of the file exports the same file again; `ambre_xlsx::fixtures::an_ods_write_libreoffice_saved_again_reads_back_the_same_workbook` | none | verified |
| `TRAME-IO-067` | `POST` | A formula MUST cross between OpenFormula and Excel's dialect unchanged, and a formula OpenFormula cannot spell MUST leave in Excel's grammar. | this spec section 3; `TRAME-IO-056` | `ambre_xlsx::ods::formula::tests::an_excel_formula_survives_the_trip_through_openformula`; `::libreoffice_formulas_read_as_the_text_excel_stores` -- the function names LibreOffice 24.2 writes | none | verified |
| `TRAME-IO-068` | `FAIL` | Every construct an OpenDocument read approximates or drops MUST be an entry of its report, counted. | this spec section 3; `TRAME-IO-060`, `INTEROP-003` | `ambre_xlsx::fixtures::every_construct_an_ods_file_holds_and_trame_lacks_is_reported`; `ambre_xlsx::ods::number::tests::a_second_section_is_dropped_and_named` | none | verified |
| `TRAME-IO-069` | `FAIL` | A malformed or hostile OpenDocument file, or a package of another kind, MUST fail as a typed error, never as a panic. | this spec section 3; `TRAME-IO-061` | `ambre_xlsx::model_props::random_bytes_never_panic_the_ods_reader`; `::a_corrupted_ods_never_panics_the_reader`; `::a_hostile_content_part_never_panics_the_ods_reader`; `ambre_xlsx::fixtures::a_file_of_the_other_format_or_kind_is_refused_with_its_reason`; `ambre_opc::tests::an_opendocument_package_of_another_type_or_none_is_refused` | none | verified |
| `TRAME-IO-070` | `FRAME` | One workbook MUST always write one sequence of OpenDocument bytes. | this spec section 3; `TRAME-IO-059` | `ambre_xlsx::fixtures::one_workbook_writes_one_sequence_of_ods_bytes` | none | verified |
| `TRAME-IO-071` | `POST` | A user MUST be able to import an OpenDocument spreadsheet and to export the workbook as one. | this spec section 4 | `ambre_trame::logic::xlsx_tests::the_file_menu_offers_the_opendocument_import_and_export` -- the menu entry resolves to the export, through the Office module, as a command that writes nothing | none | verified |

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

The two Excel commands are `TrameImport::Xlsx` and `TrameExport::Xlsx`, and the two OpenDocument
commands `TrameImport::Ods` and `TrameExport::Ods`. The Office converter module
runs the codec and maps its model to and from Trame's form, the `Book` of `ambre-xlsx`. The form
crosses as postcard bytes, a style as the bytes Trame stores, so Main decodes no Excel style. Each
construct the codec reports is an entry of the shared report, `TrameConstruct::Excel`
(`TRAME-IO-012`).

**The module call.** A module reads or writes a file whole, so its call holds a thread for the
length of the file. That call runs on the converter thread, never on the Logic thread
(`ARCH-123`, `ambre-spec-architecture.md section 2`):

| Conversion | On the Logic thread | On the converter thread |
|---|---|---|
| import | on the answer, the mapping and the writes, an Excel file one slice per pass | the read into records or into the form |
| export | the walk of the cells, then, on the answer, the file handed over | the write of the records or of the form |

The workbook owes the request under a ticket, and the Logic dispatcher sends it and never waits. A
read in flight changes nothing. Its answer lands as a command would: a running Excel import or PDF
export lands first, and the import is one undo step. An export walks the cells on its command, so
an edit made while the write is in flight never reaches the file. A module that does not run
answers as a module that did not load.

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

1. refuse a file past the 8 MiB bound as too large, before any other step (`TRAME-IO-090`). The
   bound sets the dearest call of the tables module, and so its answer deadline (`ARCH-128`);
2. detect the delimiter when the command names none. It is the most frequent of the comma, the
   semicolon and the tab outside quoted fields, in the first kilobyte (`TRAME-IO-035`);
3. read every record, keeping ragged rows, then drop the first when a header is declared;
4. return with nothing written when no record survives (`TRAME-IO-028`);
5. cut the records past the last row and the fields past the last column, counted from the start
   cell, and report the cut. Return with nothing written when the cut keeps no field
   (`TRAME-IO-050`);
6. open one undo group (`TRAME-IO-025`);
7. reuse the named sheet, or mint one and make it active;
8. materialize every target cell, list a minted sheet only then, inside the group, and write the
   non-empty fields as literals;
9. recompute, and report the rows and the columns written, beside the sheet list.

Step 8 is in two passes on purpose: a materialization commit carries a different origin.
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

1. walk every sheet in tab order, a hidden one marked hidden (`TRAME-IO-015`). The default sheet
   of a new workbook stays out when another sheet exists and it holds nothing a user put there. It
   keeps its name, and holds no cell, style, merge, size or filter. An import into a new workbook
   leaves it so, and writing it would add a sheet to the file at each round trip (`TRAME-IO-082`);
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

**The OpenDocument path.** An `.ods` file takes the Excel path from its first step on. The
Office module reads it into the model an xlsx file reads into, with the same report. It then maps
the model through the same `Book`. So the steps above, the three states of a cell and the slices
hold it unchanged, and its report entries are `TrameConstruct::Excel` too (`TRAME-IO-029`).

The export writes the `Book` the Excel export writes (`TRAME-IO-066`). A date reads as the serial
of its wall time, as an Excel date does, so neither path takes a zone.

**What the OpenDocument codec changes.** Each difference of the format sits in the codec, which
brings the file to the model's terms:

| The file holds | The model holds | The codec |
|---|---|---|
| an OpenFormula text, `of:=SUM([.A1:.B2])` | the text Excel stores, `SUM(A1:B2)` | translates both ways (`TRAME-IO-067`) |
| a function under `COM.MICROSOFT.` or `LEGACY.`, or renamed | the Excel name, with its `_xlfn.` prefix | maps the names LibreOffice 24.2 writes |
| a formula OpenFormula cannot spell | its Excel text | writes it under `msoxl:`, Excel's grammar, which LibreOffice reads |
| a reference to another file | nothing | keeps the cached value, and reports the formula and the link dropped |
| a number style of elements | the format code those elements spell | reads it through the code, so a format lands in the family Excel's code lands in |
| a format code of two or more sections | its first section | writes it, and reports the number format approximated |
| a cell style, its parent and the style of its row or column | one resolved style | resolves the chain from the document's default |
| a run of repeated, styled, empty cells past 1 024 cells | nothing | drops it and reports the style: it styles a whole line, which Trame holds no style for |
| the width LibreOffice gives an unsized column, 0.889 inch | no width | reads it as no width, as LibreOffice's own xlsx export writes none |

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `csv` | reads and writes CSV | yes, by `ambre-convert` |
| `ambre-xlsx` | reads and writes `.xlsx` and `.ods`: the two codecs below, over one model | yes, by `ambre-convert`, whose Office module alone turns on its `codec` feature. Trame links the model and the report alone |
| `ambre-opc` | the package the office codecs share: the archive and the bounds of a hostile package, under the OPC index of the DOCX, xlsx and PPTX codecs or the OpenDocument index of the `.ods` codec | yes, by the three OOXML codecs, which the `.ods` codec shares a crate with |
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
| carried | sheet, cell, formula, shared formula, style, merge, column width, row height, hidden sheet, frozen pane | sheet, cell, formula, style, merge, column width, row height, hidden sheet, frozen pane |
| approximated | anchored reference, formula Trame cannot run, very hidden sheet, 1904 date system, alignment, border stroke, pattern fill, double underline | currency format: Trame draws the reader's currency, and the file names the generic sign |
| dropped | chart, chart sheet, picture, shape, pivot table, macro, comment, data validation, conditional format, hyperlink, defined name, table, protection, filter, link to another workbook, rich text runs, diagonal border, gradient fill, font effect, text rotation, hidden row or column, data table | nothing |

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
| the toolbar Export button | the Logic thread reads the active sheet, the converter thread writes the bytes, and the Logic thread answers with them |
| the menu entry Import Excel workbook | a file picker opens, accepting `.xlsx` and `.xlsm`; each sheet of the file lands as a new sheet |
| the menu entry Export Excel workbook | the whole workbook leaves as one `.xlsx` file, named after its first sheet |
| the menu entry Import OpenDocument spreadsheet | a file picker opens, accepting `.ods`; each sheet of the file lands as a new sheet, as from Excel |
| the menu entry Export OpenDocument spreadsheet | the whole workbook leaves as one `.ods` file, named after its first sheet |
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
| Each input fixture is imported, exported and imported again, and the two documents are compared as data (`TRAME-IO-080`) | comparing the bytes of the file read with the bytes of the file written | the zip order, the timestamps, the namespace prefixes and the style order differ with no defect. A byte comparison fails on each of them and holds nothing a user reads |
| On export, an untouched default sheet beside other sheets stays out (`TRAME-IO-082`) | an import into a new workbook that takes the place of its default sheet | the import would delete a sheet another peer may be typing into while the slices land. Leaving it out of the file is local to one export, and writes nothing |
| Every conversion runs on the user's machine | converting server-side | it is faster to build and hands the relay the whole document. The pillar is that the server holds ciphertext, and a conversion is the one moment a document exists as plaintext outside the editor |
| One codec of ours reads and writes Excel, over `zip` and `quick-xml` | `calamine` 0.38 to read and `rust_xlsxwriter` 0.102 to write | the pair weighs 431 KB of brotli on wasm32, against 159 KB for the codec. `calamine` reads no column width, no row height and no array formula range, and names no construct it skips. Two models drift, and Trame's dialect is ours in both cases. Both run at the codec's speed. Measured on 2026-11-09, section 6 |
| One codec of ours reads and writes Excel, over `zip` and `quick-xml` | `umya-spreadsheet` 4.0 beta, which reads and writes | it weighs 656 KB of brotli on wasm32, and reads 100k cells in 1.6 s, against 0.1 s for the codec. Measured on 2026-11-09, section 6 |
| One codec of ours reads and writes OpenDocument, over the zip and XML readers the Office module links | `spreadsheet-ods` 1.0.4, which reads and writes | it weighs 389 KB of brotli on wasm32 against 30.6 KB for the codec, and brings a second zip stack. Its model keeps OpenFormula text and names no construct it skips, so a second map to Trame would drift from the Excel one. It reads about twice as fast. Measured on 2026-11-11, section 6 |
| One codec of ours reads and writes OpenDocument | `calamine` 0.38 to read | it reads values and formulas alone, no style, width or height, writes nothing, and weighs 158 KB of brotli. Measured on 2026-11-11, section 6 |
| The OpenDocument codec reads into the Excel model | a model of its own, mapped to Trame beside the Excel one | two maps drift, and `TRAME-IO-029` would hold a comparison of two paths instead of one path. The formula translation and the number formats run once, in Excel's terms |
| The OpenDocument codec lives in `ambre-xlsx` | a crate of its own | it shares the model, the report, the formula lexer and the format codes of the xlsx codec. A crate of its own would make each of them public, and change no build: the Office module links both |
| A formula OpenFormula cannot spell leaves under `msoxl:` | its cached value | the formula is the author's work, as `TRAME-IO-011` holds for Excel. LibreOffice reads the Excel grammar back |
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

* **The evaluation of the OpenDocument crates, on 2026-11-11.** The frame is the same: a `cdylib`
  at `opt-level = "z"` with LTO, brotli at level 11, and host release builds on 100k cells. The
  file read was one sheet of numbers, texts and formulas, written by a script. The OpenDocument
  codec of `ambre-xlsx` costs what it adds to the Office module, which already links the zip and
  XML readers it needs.

| Variant | wasm32, raw | wasm32, brotli | Write 100k cells | Read 100k cells |
|---|---|---|---|---|
| the OpenDocument codec of `ambre-xlsx`, added to the Office module | -- | +30.6 KB | 157 ms | 179 ms |
| `spreadsheet-ods` 1.0.4, read and write | 1477 KB | 389 KB | 130 ms | 79 to 112 ms |
| `calamine` 0.38, read alone | 534 KB | 158 KB | -- | 79 ms |

* **Where the test files come from.** The copied files sit in a folder named after their
  source, each with its `.expected` dump, and `fixtures/THIRD_PARTY.md` names each one. Files named
  after a bug report are left out. `generated/generate.sh` writes every other file from this
  repository. LibreOffice wrote them: its exports of two hand-written flat files, and its saves of
  the sample each writer writes.
* **The codec links `zip` with `deflate-flate2`, over `miniz_oxide`.** It reads and writes
  deflate for 159 KB of brotli. The `deflate` feature adds `zopfli` and `zlib-rs` for 31 KB more,
  and buys only a smaller written file. The three OOXML codecs declare the same lines.
* **A column width moves by the default font.** The format counts a width in digits of the
  workbook's font, and the codec reads 7 pixels a digit. A LibreOffice file written in Arial 10
  reads about 6 % narrow.
* **A control character leaves an OpenDocument text.** XML 1.0 carries no control character but the
  tab, the line feed and the carriage return. OpenDocument has no escape for the others, as xlsx has
  `_xHHHH_`. The writer drops each one, and no report entry names it.
* **A currency format reads without its locale tag.** LibreOffice tags a currency with a locale, and
  its xlsx export writes `[$EUR-407]` where the OpenDocument reader writes `[$EUR]`. Both land as a
  pattern, so the two paths keep two patterns a few characters apart.
* **The function names follow one producer.** The map of `COM.MICROSOFT.` and `LEGACY.` names
  follows LibreOffice 24.2. It was read on 2026-11-11 from its export of each `_xlfn.` function.
  Another producer that names a function otherwise lands it as a formula Trame cannot run, kept
  verbatim.
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
