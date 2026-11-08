# AMBRE -- FEATURE: TRAME IMPORT AND EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- CSV and the native file ship; the two Excel paths are a design.

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
| `TRAME-IO-001` | `INV` | Every conversion MUST run on the user's own machine, in the Logic thread. | this spec section 3; `ZK-001`, `ZK-004` | cargo-deny -- `deny.toml` lets `ambre-convert` alone declare `csv`, the converter module the Logic thread loads and calls (`ARCH-078`); ci-gate -- `scripts/crate_boundaries.py` refuses `csv::` in UI-thread code, fixture `test_a_view_converting_a_file_is_refused` | none | verified |
| `TRAME-IO-033` | `INV` | No plaintext file MUST reach the server: what leaves the device is the sealed document, never the bytes the user opened. | this spec section 3; `ZK-001`, `ZK-004` | `ambre_logic::dispatch::tests::an_import_hands_sync_a_delta_and_never_the_file` -- the Sync worker then seals that delta (`ZK-001`) | none | verified |
| `TRAME-IO-002` | `INV` | Import and export MUST compile to `wasm32-unknown-unknown`: the conversion is client-side, with no server round trip. | this spec section 3 | target-build -- the CI `test-wasm` job runs `cargo test --target wasm32-unknown-unknown` on `ambre-trame`, which compiles the crate and its `csv` dependency for `wasm32-unknown-unknown`, so an interop crate that does not build there fails that job | none | verified |
| `TRAME-IO-034` | `INV` | A reader MUST keep the export: it writes nothing, so the role gate MUST NOT withhold it. | this spec section 3; `ambre-spec-architecture.md section 4` | `ambre_logic::app::tests::read_only_and_local_commands_are_not_mutating` -- it pins `ExportActiveSheetCsv` as non-mutating, and does not name `ExportCsv { .. }`, which carries the same claim in `is_mutating` and no test | none | verified |
| `TRAME-IO-030` | `INV` | A bridge to a third-party spreadsheet MUST seal the content on the user's machine before it enters the document; no plaintext MUST reach Ambre's storage. | this spec section 5; section 6; `ZK-001` | not built | unbuilt -- lands with the `The Google Sheets bridge` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-007` | `IO` | An `.ambre` export/import round trip MUST preserve the document state, byte for byte on a bare document. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`; `::round_trip_preserves_the_document_state` | none | verified |
| `TRAME-IO-004` | `IO` | A CSV import followed by an export MUST round-trip the cell values. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_export_roundtrip`; `::csv_import_then_export_drops_trailing_empties`; `ambre_trame::logic::tests::group7::csv_export_typed_6x10_has_no_trailing_columns` | none | verified |
| `TRAME-IO-019` | `IO` | An xlsx import followed by an export followed by a re-import MUST yield the same values. | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-029` | `IO` | An OpenDocument import MUST land the same document as the same workbook imported from Excel: same cell states, same report. | this spec section 3; section 6 | not built | unbuilt -- lands with the `OpenDocument interop` Story (`ambre-roadmap.md` E6). One reader reads both formats, so a divergence would be ours | unimplemented |

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
| `TRAME-IO-039` | `FAIL` | An import MUST report the records it could not read; it MUST NOT drop them in silence. | this spec section 3; section 6 | `ambre_trame::logic::tests::group7::import_reports_unreadable_records` | none | verified |
| `TRAME-IO-050` | `FAIL` | An import MUST write no field past the last row or the last column of the grid. It MUST report the records and fields it left out. An import that keeps no field MUST leave the document untouched. | this spec section 3 | `ambre_trame::logic::tests::group7::an_import_past_the_grid_edge_is_cut_and_said` -- one record past the last row, one field past the last column; `::an_import_starting_past_the_grid_edge_writes_nothing` -- a start past the last row, then past the last column | none | verified |
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
| `TRAME-IO-010` | `POST` | An imported Excel formula MUST land in exactly one of three states -- parsed formula, unsupported formula preserved verbatim, or literal -- never a silent mixture. | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-011` | `POST` | An unsupported Excel formula MUST be preserved byte-for-byte and re-exported unchanged; it MUST NOT be downgraded to its last computed value. | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-015` | `POST` | A hidden Excel sheet MUST import as a hidden sheet, never be dropped. | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-014` | `POST` | Two cells sharing a style MUST import to a single deduplicated style entry, not to two copies. | this spec section 3; section 6 | not built | unbuilt -- lands with the `The shared style dictionary` Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-013` | `FRAME` | An import MUST NOT starve the frame budgets: scrolling MUST stay responsive while it runs. | this spec section 6; `ambre-spec-budgets.md` | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-012` | `FAIL` | Every ignored Excel feature MUST raise a typed warning; silence MUST NOT be the report. | this spec section 3; section 6 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.7 Hand a workbook back as Excel

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-017` | `POST` | On export, a cell reference MUST be rendered at its current position. | this spec section 3; `features/ambre-spec-trame-sheets.md` | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-048` | `POST` | On export, a cross-sheet reference MUST carry the sheet's display name. | this spec section 3; `features/ambre-spec-trame-sheets.md` | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-018` | `POST` | On export, a broken reference MUST be written as the error the target format understands, never as an empty cell. | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-047` | `FRAME` | An Excel export MUST NOT change the document it reads, exactly as the CSV export MUST NOT (`TRAME-IO-026`). | this spec section 3 | not built | unbuilt -- lands with the `xlsx interop` Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** the four conversion rows of `ambre-spec-budgets.md section 1`: CSV import of 10k
  lines, CSV export, xlsx import and xlsx export. `TRAME-IO-013` binds the import to the frame
  budgets on top. Every one reads **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. Every conversion stays in the
  Logic thread, no plaintext file reaches the server, no round trip and no third-party bridge
  (`TRAME-IO-001`, `TRAME-IO-033`, `TRAME-IO-002`, `TRAME-IO-030`).
* **Verification.** 30 `verified`, 13 `unimplemented`. What holds is the round trip of the two
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
| `TrameCommand::ImportCsv` | UI to Logic | the bytes, an optional delimiter, a header flag, a target sheet, an origin |
| `TrameCommand::ExportCsv` | UI to Logic | the sheet, an optional delimiter, an optional range |
| `TrameCommand::ExportActiveSheetCsv` | UI to Logic | nothing: the Logic thread resolves the active sheet itself |
| `TrameEvent::ImportComplete { sheet_id, rows, cols }` | Logic to UI | what landed, and where |
| `TrameEvent::ExportComplete { filename, data }` | Logic to UI | the file name, which `DOCFILE-019` and `DOCFILE-020` hold, and the bytes, for the UI thread to hand to the browser |

The two Excel commands are designed and absent from the bus. Their warnings have no carrier: the
event above holds no report, which `TRAME-IO-012` needs and the shared vocabulary Story supplies
(`ambre-roadmap.md`, "One interop vocabulary").

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
8. recompute, and report the sheet, its rows and its columns.

Step 7 is in two passes on purpose: a materialization commit carries a different origin.
Interleaving it with the value writes would split the undo item in two.

**Steps, on a CSV export:**

1. resolve the sheet, and answer nothing when it is unknown;
2. bound the range by scanning the cell map, never by the row and column lists. Those stay
   materialized at their high-water mark after a scroll (`TRAME-IO-027`);
3. write each cell by its displayed value, formula results included (`TRAME-IO-005`);
4. return the bytes as one file, and no delta.

**The Excel cell, when the path is built.** An imported cell lands in one of three states, and the
export reads the state back:

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

An Excel import writes through the same positional path as a CSV import: materialize, then write
the raw string. It inherits the undo grouping and the recompute of step 7 rather than reaching
into the containers itself.

**What an Excel file loses:**

| What the file holds | What the import does |
|---|---|
| macros, pivot tables, charts, shapes | drops them, and reports each one (`TRAME-IO-012`) |
| a link to another workbook | keeps the last computed value, and drops the link |
| an array formula the engine cannot run | keeps it verbatim, to re-export unchanged (`TRAME-IO-011`) |
| a hidden sheet | keeps it, hidden (`TRAME-IO-015`) |
| a comment on a cell | keeps it, once cell comments exist |
| a validation rule | keeps the kinds the write gate has, and reports the others |
| sheet protection | drops it: Trame has no equivalent yet |
| an image in a cell | drops it |

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `csv` | reads and writes CSV | yes, by `ambre-trame` |
| `calamine` | reads `.xlsx`, `.xls` and `.ods`, and writes none of them | no |
| `rust_xlsxwriter` | writes `.xlsx` | no |

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
| Reading and writing Excel are two crates | `umya-spreadsheet`, which reads and writes | it is maintained by fewer hands than either of the two chosen crates, each mature at the one direction it does |
| Reading and writing Excel are two crates | the Node bindings | they are not Rust and do not run in the Logic thread; both chosen crates are pure Rust and compile to WASM, which the pillar requires |
| An unsupported formula is kept verbatim | storing the cached value, which is what a naive import does | a formula the engine cannot run is still the author's work. Downgrading it to its last computed value loses it silently, and the loss is invisible until someone reopens the file in Excel |
| The marker is a byte a user cannot type | a parallel map of preserved formulas | every structural edit would have to reindex it. The preserved text must be distinguishable from an ordinary cell with no second container to hold the flag, and no legal cell content starts with a null byte |
| A date arrives as a serial number | converting to ISO on import | it reads better and computes worse: the engine's date functions compute on serials, so an ISO string would have to be converted back at every read |
| An export writes what the user sees | exporting the formula sources | a spreadsheet leaving Ambre is read by a tool that will not run our engine, so a formula source in a CSV cell is text, not a value |
| The export bounds come from the cells | using the length of the row list | it is the obvious reading and was the defect: the row and column lists stay materialized at the high-water mark of the scroll, so their length pads the file with empty trailing rows a user never wrote |
| Google Sheets is reached through files | calling the API from the Logic thread | a Web Worker cannot call the Google API: the browser blocks the cross-origin call, and the OAuth flow has no place to run |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The Excel paths have no code.** Nine rows of section 1 describe them, and `ambre-trame`
  declares neither reader nor writer. The design in this document is the specification they will be
  built against. The value table is where it is most likely to be wrong.
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
* **`TRAME-IO-013` asks for a behaviour the command model forbids.** A command runs to completion,
  so an import cannot yield mid-file, and there is no progress event. The only mechanism available
  is the re-entrant continuation the recompute already uses.
* **The three applications name the same events differently.** Trame reports `ImportComplete` and
  `ExportComplete`, Prose reports `ImportCompleted` and `ExportReady`, and only Prose carries a
  conversion report and a progress model. The `One interop vocabulary` Story (`ambre-roadmap.md` E8)
  unifies them on Prose's names, and this document keeps Trame's until it does.
* **The performance targets are not benchmarks.** Import and export carry numbers in
  `ambre-spec-budgets.md` with no `criterion` bench behind them, so nothing fails
  when a conversion gets slower.
