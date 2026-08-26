# AMBRE -- FEATURE: IMPORT AND EXPORT

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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-IO-001` | Every conversion MUST run in the Logic thread; no plaintext file MUST reach the server. | this spec section 3; `ZK-001`, `ZK-004` | crate graph: `csv` is declared by `ambre-trame` (Logic thread) alone, and `ambre-server` declares no interop crate; `deny.toml` holds the relay's capability wall | verified |
| `TRAME-IO-002` | Import and export MUST compile to `wasm32-unknown-unknown`: the conversion is client-side, with no server round trip. | this spec section 3 | the CI `test-wasm` job builds `ambre-trame` for `wasm32-unknown-unknown`; a non-WASM-compatible interop crate fails that job | verified |
| `TRAME-IO-007` | An `.ambre` export/import round trip MUST preserve the document state, byte for byte on a bare document. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`, `::round_trip_preserves_the_document_state`, `::the_header_names_trame_and_the_document` | verified |
| `TRAME-IO-008` | A rejected file MUST leave both the input bytes and the current state untouched, and MUST fail typed rather than panic. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone`, `::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic` | verified |
| `TRAME-IO-009` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Trame. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_trame::logic::docfile::tests::a_toile_file_is_re_dispatched_not_coerced` | verified |
| `TRAME-IO-003` | A malformed or hostile CSV MUST NOT panic; it MUST fail as a typed error. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_malformed_never_panics`, `::csv_import_random_bytes_never_panics` | verified |
| `TRAME-IO-004` | A CSV import followed by an export MUST round-trip the cell values. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_import_export_roundtrip`, `::csv_import_then_export_drops_trailing_empties`, `group7::csv_export_typed_6x10_has_no_trailing_columns` | verified |
| `TRAME-IO-005` | A CSV export MUST write evaluated values, never formula sources. | this spec section 3 | `ambre_trame::logic::tests::group7::csv_export_evaluates_formulas` | verified |
| `TRAME-IO-006` | The CSV delimiter MUST be auto-detected, and a header row MUST be skippable. | this spec section 3 | `ambre_trame::logic::tests::group6::csv_delimiter_autodetect`, `::csv_import_skips_header`, `::csv_import_creates_sheet_and_cells` | verified |
| `TRAME-IO-025` | An import MUST land as one undo item, whatever the number of cells it writes. | this spec section 3 | none. The importer opens one undo group around the sheet creation and every write, and no test undoes an import | unverified |
| `TRAME-IO-026` | An export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | none. Both exporters read alone, and a shared reference proves nothing here -- a Loro container mutates through one. No test asserts the empty outcome | unverified |
| `TRAME-IO-027` | An export MUST bound its range by the cells the sheet holds, never by the materialized row and column lists. | this spec section 3; section 5 | `ambre_trame::logic::tests::group6::csv_export_trims_empty_trailing_columns`, `::csv_import_then_export_drops_trailing_empties` | verified |
| `TRAME-IO-028` | A file holding no record MUST leave the document untouched: no sheet, no cell, no undo item. | this spec section 3 | none. The importer returns before it opens the undo group, and the empty-input case asserts no panic rather than no sheet | unverified |
| `TRAME-IO-010` | An imported Excel formula MUST land in exactly one of three states -- parsed formula, unsupported formula preserved verbatim, or literal -- never a silent mixture. | this spec section 3 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-011` | An unsupported Excel formula MUST be preserved byte-for-byte and re-exported unchanged; it MUST NOT be downgraded to its last computed value. | this spec section 3 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-012` | Every ignored Excel feature MUST raise a typed warning; silence MUST NOT be the report. | this spec section 3; section 6 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-013` | An import MUST NOT starve the frame budgets: scrolling MUST stay responsive while it runs. | this spec section 6; `ambre-spec-budgets.md` | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-014` | Two cells sharing a style MUST import to a single deduplicated style entry, not to two copies. | this spec section 3; section 6 | not built -- lands with the "The shared style dictionary" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-015` | A hidden Excel sheet MUST import as a hidden sheet, never be dropped. | this spec section 3 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-017` | On export, a cell reference MUST be rendered at its current position, and a cross-sheet reference MUST carry the sheet's display name. | this spec section 3; `features/ambre-spec-trame-sheets.md` | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-018` | On export, a broken reference MUST be written as the error the target format understands, never as an empty cell. | this spec section 3 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-019` | An xlsx import followed by an export followed by a re-import MUST yield the same values. | this spec section 3 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |
| `TRAME-IO-029` | An OpenDocument import MUST land the same document as the same workbook imported from Excel: same cell states, same report. | this spec section 3; section 6 | not built -- lands with the "Post-V1 interop" Story (`ambre-roadmap.md` post-V1); one reader reads both formats, so a divergence would be ours | unimplemented |
| `TRAME-IO-030` | A bridge to a third-party spreadsheet MUST seal the content on the user's machine before it enters the document; no plaintext MUST reach Ambre's storage. | this spec section 5; section 6; `ZK-001` | not built -- lands with the "Post-V1 interop" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** the conversion rows of `ambre-spec-budgets.md section 1`: CSV import of 10k
  lines in < 200 ms, CSV export in < 100 ms, xlsx import in < 500 ms or < 3 s by size, xlsx export
  in < 1 s. `TRAME-IO-013` binds the import to the frame budgets on top. Every one of those rows
  reads **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. `TRAME-IO-001` keeps every
  conversion in the Logic thread and lets no plaintext file reach the server, `TRAME-IO-002` bars a
  server round trip, and `TRAME-IO-030` carries the same rule to a third-party bridge.
* **Verification.** 10 `verified`, 3 `unverified`, 11 `unimplemented`. What ships is held where it
  is read: the native round trip, the CSV round trip, the decoder's totality and the export bounds
  all have a host test. The three gaps are what an import does to the rest of the document -- one
  undo item, no delta on the way out, nothing at all on an empty file -- and none of them is a
  conversion rule. The eleven others describe paths with no code, and the last two describe paths
  with no design either: a format and a bridge that are scheduled and unspecified still owe the
  user a rule, and these two are the rules their design will be held to.

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
| `TrameEvent::ExportComplete { filename, data }` | Logic to UI | the bytes, for the UI thread to hand to the browser |

The two Excel commands are designed and absent from the bus. Their warnings have no carrier: the
event above holds no report, which `TRAME-IO-012` needs and the shared vocabulary Story supplies
(`ambre-roadmap.md`, "One interop vocabulary").

**Steps, on a CSV import:**

1. detect the delimiter from the bytes when the command names none;
2. read every record, keeping ragged rows, then drop the first when a header is declared;
3. return with nothing written when no record survives (`TRAME-IO-028`);
4. open one undo group (`TRAME-IO-025`);
5. reuse the named sheet, or mint one and make it active;
6. materialize every target cell, then write the non-empty fields as literals;
7. recompute, and report the sheet, its rows and its columns.

Step 6 is in two passes on purpose: a materialization commit carries a different origin, so
interleaving it with the value writes would split the undo item in two.

**Steps, on a CSV export:**

1. resolve the sheet, and answer nothing when it is unknown;
2. bound the range by scanning the cell map, never by the row and column lists, which stay
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

An Excel import writes through the same positional path as a CSV import -- materialize, then write
the raw string -- so it inherits the undo grouping and the recompute of step 6 rather than reaching
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
| the answer | the UI thread wraps the bytes in a browser download; no canvas state changes |

The picker is transient: it is never attached to the page, and it lives until the user picks a file
or drops it.

**What the surface cannot reach.** The import command carries a delimiter, a header flag, a target
sheet and an origin. The button sets none of them: it always auto-detects, always treats row one as
data, and always creates a sheet. The export button carries no delimiter and no range, so the
parameterized command has no caller (section 6).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Every conversion runs on the user's machine | the pillar is that the server holds ciphertext. A conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device | converting server-side, which is faster to build and hands the relay the whole document |
| Reading and writing Excel are two crates | the mature reader does not write, and the mature writer does not read. One crate that does both exists and is less maintained than either. Both chosen crates are pure Rust and compile to WASM, which the pillar requires | `umya-spreadsheet`, which reads and writes and is maintained by fewer hands; and the Node bindings, which are not Rust and do not run in the Logic thread |
| An unsupported formula is kept verbatim | a formula the engine cannot run is still the author's work. Downgrading it to its last computed value loses it silently, and the loss is invisible until someone reopens the file in Excel | storing the cached value, which is what a naive import does |
| The marker is a byte a user cannot type | the preserved text must be distinguishable from an ordinary cell with no second container to hold the flag, and no legal cell content starts with a null byte | a parallel map of preserved formulas, which every structural edit would have to reindex |
| A date arrives as a serial number | the engine's date functions compute on serials, so an ISO string would have to be converted back at every read | converting to ISO on import, which reads better and computes worse |
| An export writes what the user sees | a spreadsheet leaving Ambre is read by a tool that will not run our engine, so a formula source in a CSV cell is text, not a value | exporting the formula sources |
| The export bounds come from the cells | the row and column lists stay materialized at the high-water mark of the scroll, so their length pads the file with empty trailing rows a user never wrote | using the length of the row list, which is the obvious reading and was the defect |
| Google Sheets is reached through files | a Web Worker cannot call the Google API: the browser blocks the cross-origin call, and the OAuth flow has no place to run | calling the API from the Logic thread |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The Excel paths have no code.** Nine rows of section 1 describe them, and `ambre-trame`
  declares neither reader nor writer. The design in this document is the specification they will be
  built against, and the value table is where it is most likely to be wrong.
* **OpenDocument import is a rule, not a design.** The format is named in one dependency row,
  because the Excel reader also reads it, and nothing states what a `.ods` file becomes.
  `TRAME-IO-029` is what its design will be held to: the Excel path decides the answer, and the
  second format may not drift from the first.
* **The Google Sheets bridge is scheduled and undesigned.** A proxy fetching a sheet as JSON would
  be the first component of Ambre to hold a third party's plaintext, and the audit that decision
  needs is not written. `TRAME-IO-030` states the boundary the design must respect before that
  audit, so the rule does not wait on it.
* **The import surface reaches none of its parameters.** A user cannot choose a delimiter, declare a
  header row, or import into an existing sheet, though the command accepts all three. `TRAME-IO-006`
  is verified at the command and unreachable at the button.
* **No property test drives an import.** An import creates a sheet and writes as many cells as a
  paste, and the convergence property drives neither it nor the two exporters. Three rows say `none`
  for that reason, and every row of section 2 does.
* **The decoder has no fuzz target.** Two host tests feed it crafted bytes and four hundred
  pseudo-random ones, which is more than the corpus average and less than what a decoder fed by user
  files deserves.
* **`TRAME-IO-013` asks for a behaviour the command model forbids.** A command runs to completion,
  so an import cannot yield mid-file, and there is no progress event. The only mechanism available
  is the re-entrant continuation the recompute already uses.
* **The three applications name the same events differently.** Trame reports `ImportComplete` and
  `ExportComplete`, Prose reports `ImportCompleted` and `ExportReady`, and only Prose carries a
  conversion report and a progress model. The "One interop vocabulary" Story unifies them on Prose's
  names, and this document keeps Trame's until it does.
* **The performance targets are not benchmarks.** Import and export carry numbers in
  `ambre-spec-budgets.md` with no `criterion` bench behind them, so nothing fails
  when a conversion gets slower.
