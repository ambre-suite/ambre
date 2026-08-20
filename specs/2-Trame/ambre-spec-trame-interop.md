# AMBRE — SPEC: AMBRE-TRAME INTEROP (Import / Export)
**Type:** Vertical (Feature / Application — Interop)
**Status:** Draft
**Implementation:** partial

> Import and export of third-party formats for Trame. All conversions run client-side in the **Logic thread** (WASM) — the server never sees the plaintext content. Supported formats: Excel (.xlsx), CSV, PDF.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**
- Native `.ambre` export/import for Trame: the `export_ambre_plaintext()` / `import_ambre_plaintext()` pair over the Trame `LoroDoc`, writing `DocType::Trame` in the header. The container itself -- layout, versioning, `migrate_ambre`, cross-app dispatch, at-rest policy -- is specified in `../1-Foundations/ambre-spec-docfile.md` and must not be re-implemented here. This is the only **lossless** path Trame offers: every conversion below drops something.
- Import .xlsx → `LoroDoc` Trame (via `calamine`)
- Export `LoroDoc` Trame → .xlsx (via `rust_xlsxwriter`)
- Import/Export CSV
- High-fidelity PDF export (via `krilla` — delegated to `spec-ambre-trame-ui.md` for rendering)

**What this module does NOT do:**
- Google Sheets conversion: import via intermediate CSV/JSON (no direct Google API in WASM)
- Full PDF export: the DrawSurface rendering (KrillaSurface) is described in the UI spec
- VBA Macros / Office Scripts: definitively out of scope

**Invariants:** prefix `TRAME-IO`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4`; the cross-spec roll-up is `ambre-spec-testing.md §7`. Rows `001..009` cover the built paths -- CSV (§4) and the native `.ambre` container (§1). Rows `010..020` cover the .xlsx paths (§2, §3) and PDF export (§5), which have no implementing code -- `ambre-trame` declares `csv`, and neither `calamine`, `rust_xlsxwriter` nor the shared `ambre-pdf` crate (which owns `krilla`) -- so they carry `Status: unimplemented` and cite the Story that will build them (architecture §4). `TRAME-024` ("no `unsafe`") covers the whole crate and is not restated here.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-IO-001` | Every conversion MUST run in the Logic thread; no plaintext file MUST reach the server. | §1; `ZK-001`, `ZK-004` | crate graph: `csv` is declared by `ambre-trame` (Logic thread) alone, and `ambre-server` declares no interop crate; `deny.toml` holds the relay's capability wall | verified |
| `TRAME-IO-002` | Import and export MUST compile to `wasm32-unknown-unknown`: the conversion is client-side, with no server round trip. | §1 | the CI `test-wasm` job builds `ambre-trame` for `wasm32-unknown-unknown`; a non-WASM-compatible interop crate fails that job | verified |
| `TRAME-IO-003` | A malformed or hostile CSV MUST NOT panic; it MUST fail as a typed error. | §4.A | `ambre_trame::logic::tests::group6::csv_import_malformed_never_panics`, `::csv_import_random_bytes_never_panics` | verified |
| `TRAME-IO-004` | A CSV import followed by an export MUST round-trip the cell values. | §4 | `ambre_trame::logic::tests::group6::csv_import_export_roundtrip`, `::csv_import_then_export_drops_trailing_empties`, `group7::csv_export_typed_6x10_has_no_trailing_columns` | verified |
| `TRAME-IO-005` | A CSV export MUST write evaluated values, never formula sources. | §4.B | `ambre_trame::logic::tests::group7::csv_export_evaluates_formulas` | verified |
| `TRAME-IO-006` | The CSV delimiter MUST be auto-detected, and a header row MUST be skippable. | §4.A | `ambre_trame::logic::tests::group6::csv_delimiter_autodetect`, `::csv_import_skips_header`, `::csv_import_creates_sheet_and_cells` | verified |
| `TRAME-IO-007` | An `.ambre` export/import round trip MUST preserve the document state, byte for byte on a bare document. | §1; `ambre-spec-docfile.md §2` | `ambre_trame::logic::docfile::tests::round_trip_is_byte_identical_on_a_bare_document`, `::round_trip_preserves_the_document_state`, `::the_header_names_trame_and_the_document` | verified |
| `TRAME-IO-008` | A rejected file MUST leave both the input bytes and the current state untouched, and MUST fail typed rather than panic. | §1; `ambre-spec-docfile.md §2` | `ambre_trame::logic::docfile::tests::a_rejected_file_leaves_both_the_input_and_the_state_alone`, `::a_corrupt_payload_under_a_valid_header_is_typed_not_a_panic` | verified |
| `TRAME-IO-009` | A `.ambre` file belonging to another app MUST be re-dispatched to that app, never coerced into Trame. | §1; `ambre-spec-docfile.md §2.A` | `ambre_trame::logic::docfile::tests::a_toile_file_is_re_dispatched_not_coerced` | verified |
| `TRAME-IO-010` | An imported Excel formula MUST land in exactly one of three states -- parsed formula, unsupported formula preserved verbatim under the `\x00EXCEL:` marker, or literal -- never a silent mixture. | §2.D | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-011` | An unsupported Excel formula MUST be preserved byte-for-byte and re-exported unchanged; it MUST NOT be downgraded to its last computed value. | §2.D | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-012` | Every ignored Excel feature MUST raise a typed `ImportWarning`; silence MUST NOT be the report. | §2.F | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-013` | An import MUST NOT starve the P1/P2 budgets: it lands in batches, and scrolling stays responsive while it runs. | §2.A; §7 | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-014` | Two cells sharing a style MUST import to a single deduplicated style entry, not to two copies. | §2.E | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-015` | A hidden Excel sheet MUST import as a sheet carrying `hidden: true`, never be dropped. | §2.F | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-016` | Merges MUST survive an import/export round trip. | §2.C; §3.B | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-017` | On export, a `CellId` MUST be rendered as its current A1 position through the `SheetIndex`, and a cross-sheet reference MUST be rendered with the sheet's display name (the `#id` sigil is internal). | §3.B; `trame-logic §2.D` | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-018` | On export, a broken reference MUST be written as the literal `#REF!` the target format understands, never as an empty cell. | §3.B | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-019` | An xlsx import followed by an export followed by a re-import MUST yield the same values. | §2; §3 | not built -- lands with §10 Phase 2 (Epic `E6`) | unimplemented |
| `TRAME-IO-020` | PDF export MUST run in the Logic thread through the shared `DrawSurface`, using `ambre_pdf::KrillaSurface` from the shared `ambre-pdf` crate (never an app-local surface); the UI thread MUST only trigger the download on `ExportComplete`. | §5; §9 | not built -- lands with §10 Phase 2 (Epic `E6`); `ambre-trame` does not yet depend on the shared `ambre-pdf` crate | unimplemented |

> **Invariant coverage.** 9 `verified`, 0 `unverified`, 11 `unimplemented`. Every `verified` row covers a shipped path. The eleven others are the rules the two large unbuilt sections of this spec have always contained -- they were prose, they are now falsifiable and countable.

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `calamine` | Read .xlsx / .xls / .ods | Pure Rust, WASM-compatible, read-only |
| `rust_xlsxwriter` | Write .xlsx | Pure Rust, WASM-compatible |
| `csv` | CSV read/write | Pure Rust, `no_std`-compatible |
| `ambre-trame` (internal) | `TrameState`, `SheetIndex`, `CellValue`, CRDT schema | Loro doc access for mutation |

> **`calamine` vs `umya-spreadsheet`**: `calamine` is read-only — sufficient for import. For .xlsx export, `rust_xlsxwriter` is the only mature pure Rust crate to date. Both are WASM-compatible.

---

## 2. IMPORT EXCEL (.xlsx)

> **Unimplemented — Phase 2 design sketch (reconcile to the `TrameState` model when built).**
> §2, §3 and §5 (xlsx import/export, PDF export) are **not** implemented (roadmap
> §10 Phase 2/3); only CSV (§4) exists. The pseudo-code below predates the current
> engine model and must be reconciled to it on implementation:
> - **No `TrameController`.** The engine type is `TrameState` (`logic/*.rs`); its
>   methods are inherent and return a **one-way** [`TrameOutcome`] whose `update`
>   field the `ambre-logic` dispatcher forwards to Sync. Import must **not** call
>   `SyncCommand::SendUpdate` directly nor `subscribe_local_update`.
> - **Positional `CellRef`.** A `CellRef` is `{ sheet: u16, row: u32, col: u32 }`
>   (positional); writes go through the positional `write_cell(row, col, &raw)` /
>   `materialize(row, col)` path, exactly like `import_csv`. There is no
>   `CellRef { sheet: SheetId, row: Uid, col: Uid }`, and `to_key_part()` lives on
>   `Uid`, not on a `u32` position.
> - **Flat cells, no shared `styles` map.** Values are stored flat in
>   `cells_{sheet}` (a cell's raw string is the map value); style is one
>   postcard-encoded `cellstyle_{sheet}` register per cell (`-logic.md` §2.B). The
>   `{v,t,s}` cell sub-map and the shared `styles` dictionary are deferred, so
>   import must write the flat model, not `cell_map.insert("v"/"t"/"s", ..)`.
> - **`#id`↔name sheet-qualifier mapping (`-logic.md` §12).** The CRDT stores
>   cross-sheet references id-qualified (`#s0!A1`); xlsx/CSV **export** must map
>   `#id -> name` and **import** must map `name -> #id`. The raw `#s0!A1` must never
>   reach the exported file verbatim.
> - **Synchronous, no `ImportProgress`.** `handle_command` runs to completion
>   synchronously; there is no mid-import yield and `ImportProgress` is not a bus
>   event (§7). Batching, if reintroduced, needs the additive `ContinueDirtyCompute`
>   mechanism of `-logic.md` §8, not a re-entrant import.
> - **`ImportXlsx`/`ExportXlsx`/`ExportPdf` are not yet on the bus** (`-logic.md` §9
>   "planned surface"); `TrameEvent::ImportComplete` is `{ sheet_id, rows, cols }`
>   (no `version`/`sheets_count`/`warnings`) and `ExportComplete` is
>   `{ filename, data }`.

> **Target state -- shared interop vocabulary (X18/X19/X20).** These names and types are
> superseded by one import/export vocabulary in Foundations, Toile included: event names unify on
> Prose's (`ImportCompleted` / `ExportReady`); the import-only off-bus `ImportWarning` is replaced by
> the shared `ConversionReport` (+ `ConversionWarning`); and a shared progress model
> (`ImportProgress` / `ExportProgress { percent }`, emitted past a size threshold) arrives. See the
> cross-app factoring Story (`ambre-roadmap.md`).

### A. General Flow

```
.xlsx File (Vec<u8> received from the UI Thread via postMessage)
  │
  ├─ calamine::open_workbook_from_rs() → Xlsx<Cursor<Vec<u8>>>
  │
  ├─ For each sheet:
  │   ├─ Create a new sheet in LoroDoc (CreateSheet)
  │   ├─ Import dimensions (rows/columns with data)
  │   ├─ Import cells (value, type, formula, style)
  │   ├─ Import column widths / row heights
  │   └─ Import cell merges
  │
  ├─ Import cell styles (flat cellstyle_{sheet} register per cell -- no shared "styles" map)
  ├─ Import named ranges
  └─ return TrameOutcome { update: Some(delta), .. } → ambre-logic → SyncCommand::SendUpdate
     (one-way; import never calls SyncCommand directly, -logic.md §8)
```

### B. UI Command

```rust
// From the UI Thread (planned; ImportXlsx is not yet on the bus, -logic.md §9)
TrameCommand::ImportXlsx { data: Vec<u8> }   // .xlsx file bytes

// Response -- the real bus event carries no version/warnings; warnings would need
// a dedicated channel when xlsx import lands.
TrameEvent::ImportComplete { sheet_id: String, rows: u32, cols: u32 }

// Planned unsupported-feature warning type (not yet on the bus):
pub enum ImportWarning {
    FormulaNotConverted { sheet: String, cell: String, formula: String },
    StyleNotSupported   { sheet: String, feature: String },
    MacrosIgnored,
    PivotTablesIgnored,
    ChartsIgnored,                            // Excel charts are not imported
}
```

### C. Cell Conversion

```rust
// Positional write into the flat `cells_{sheet}` model, via the same
// `write_cell(row, col, &raw)` path as `import_csv`. `style` (if any) is written
// into the per-cell `cellstyle_{sheet}` register (§2.B), never a shared styles map.
fn import_cell(
    state: &mut TrameState,
    sheet_index: &SheetIndex,
    sheet_name_to_id: &HashMap<String, SheetId>,
    row: u32, col: u32,
    cell_value: &calamine::DataType,
    formula: Option<&str>,
    style: Option<CellStyle>,
) { /* ... */ }

/// Never scientific notation: an integral value under 1e15 prints without a
/// fractional part, everything else uses the default float formatting.
fn format_float(f: f64) -> String { /* ... */ }
```

A formula takes priority over the cached value; both end up as one raw string in the flat cell model. Value conversion, by `calamine::DataType`:

| Excel type | Raw string written |
|---|---|
| `Int`, `Float` | `format_float` |
| `DateTime` | `format_float` -- **dates stay Excel serial numbers** (epoch 1899-12-30), never ISO strings, because the engine's date functions work on serials |
| `String` | verbatim |
| `Bool` | `TRUE` / `FALSE` |
| `Error`, `Empty` | nothing written -- no entry is created |

### D. Excel Formula → Trame AST Conversion

This is the most complex part of the import. The conversion distinguishes **three explicit states** — mixing them would be a design error that would make the export non-faithful:

```rust
/// Explicit state of a cell imported from Excel.
/// Never mix UnsupportedFormula and Literal -- they have different semantics.
pub enum ImportedCellKind {
    /// Successfully parsed formula -> executable by the Trame engine.
    /// Stored with "=" prefix in LoroMap("cells")["v"].
    Formula(String),

    /// Excel formula syntactically recognized but using functions
    /// unsupported by the Trame engine. Stored with a special prefix
    /// to guarantee a faithful round-trip on xlsx export.
    /// Displayed as text in the cell, not executed.
    /// On xlsx export: the prefix is removed and the original formula is restored.
    UnsupportedFormula(String),  // raw = "\x00EXCEL:=ARRAYFORMULA(...)"

    /// Raw value -- text, number, boolean. No "=".
    Literal(String),
}
```

```rust
/// Normalises the argument separator (`;` -> `,`, an Excel locale artefact) and
/// parses. A parse success yields `Formula("=...")`; a failure yields
/// `UnsupportedFormula("\x00EXCEL:=...")` -- the leading null byte is a marker a
/// user can never type, so the original text round-trips out unchanged instead of
/// being downgraded to its last computed value (`TRAME-IO-010`, `TRAME-IO-011`).
fn convert_excel_formula(
    excel_formula: &str,
    sheet_index:   &SheetIndex,         // explicit parameter -- no implicit current_sheet
    sheet_name_to_id: &HashMap<String, SheetId>,
) -> ImportedCellKind { /* ... */ }

fn import_cell_with_kind(
    kind: ImportedCellKind,
    cell_map: &mut loro::LoroMap,
) { /* ... */ }
```

**Round-trip guarantee:** on xlsx export, cells with `"t": "x"` have their `raw` stripped of the `\x00EXCEL:` prefix and the original formula is restored. Excel can thus re-execute it without loss.

**Excel functions supported from MVP onwards:** all functions implemented in the Trame engine Phase 2 (~50 functions). Formulas using unsupported functions are imported as `UnsupportedFormula` with a warning.

### E. Style Import

There is **no** shared `styles` LoroMap in the as-built schema (the dedup
dictionary is deferred, `-logic.md` §2.B / §11 bis.F). A cell's style is one flat
`cellstyle_{sheet}` register holding the whole `CellStyle`. So import resolves each
xf format to a `CellStyle` value (indexed by xf id for reuse during the cell pass)
and hands it to `import_cell`, which writes the per-cell register:

```rust
fn xf_styles(workbook: &Xlsx<Cursor<Vec<u8>>>) -> Vec<CellStyle> { /* ... */ }
```

### F. Documented Import Limitations

| Excel Feature | Import behavior |
|---|---|
| VBA Macros / Office Scripts | Ignored + `ImportWarning::MacrosIgnored` |
| Pivot tables | Ignored (source data preserved) + warning |
| Charts | Ignored + warning |
| Shapes / SmartArt | Ignored + warning |
| External links (`=[otherfile.xlsx]Sheet!A1`) | Converted to static value |
| Array formulas (`{=...}`) | Converted to static value if unsupported |
| Data validation | Imported if kind supported by Trame, otherwise ignored + warning |
| Comments | Imported into `cells[...]["c"]` Loro |
| Images in cells | Ignored (out of scope) |
| Hidden sheets | Imported with `hidden: true` |
| Sheet protection | Ignored (soft-enforcement not implemented in V1) |

---

## 3. EXPORT EXCEL (.xlsx)

### A. General Flow

```rust
// Command from the UI Thread (planned; ExportXlsx is not yet on the bus)
TrameCommand::ExportXlsx { sheet_ids: Option<Vec<SheetId>> } // None = all sheets

// Response: .xlsx file bytes. The real bus event is { filename, data } (no version).
TrameEvent::ExportComplete {
    filename: String,   // "{document_name}.xlsx"
    data:     Vec<u8>,  // bytes; the UI Thread triggers the download
}
```

### B. Implementation

```rust
fn export_xlsx(state: &TrameState, sheet_ids: Option<Vec<SheetId>>) -> Result<Vec<u8>, XlsxError> { /* ... */ }
```

**Formula fidelity on export:** Trame formulas are stored id-qualified in the CRDT (`#s0!A1`). On export they are rendered to Excel positional references (`A1`) via the `SheetIndex` **and** their `#id` sheet qualifiers mapped back to display names (§12, `-logic.md` §12). A reference to a deleted row/column (broken ref) → exported as `#REF!`.

---

## 4. IMPORT / EXPORT CSV

### A. Import CSV

```rust
TrameCommand::ImportCsv {
    data:      Vec<u8>,          // UTF-8 bytes
    delimiter: Option<u8>,       // None = auto-detect (comma or semicolon)
    has_header: bool,
    target_sheet_id: Option<SheetId>, // None = new sheet
    start_row: u32,
    start_col: u32,
}

// As-built: an inherent method of `TrameState` (`logic/interop.rs`). Writes are
// **positional** (`write_cell(row, col, &value)`, `materialize(row, col)`) -- the
// same grid-position write path as `SetCellText`/`SetCellsBatch`; there is no
// borrow of `sheet_indices` nor an `apply_edit_cell(CellRef, ..)` call. The whole
// import is one undo group; empty fields are skipped.
impl TrameState {
    pub(super) fn import_csv(
        &mut self, data: Vec<u8>, delimiter: Option<u8>, has_header: bool,
        target_sheet_id: Option<String>, start_row: u32, start_col: u32,
    ) -> TrameOutcome { /* ... */ }
}

fn detect_csv_delimiter(data: &[u8]) -> u8 { /* ... */ }
```

### B. Export CSV

```rust
TrameCommand::ExportCsv {
    sheet_id:  String,
    delimiter: Option<u8>,        // None = comma
    range:     Option<GridRange>, // None = used extent (ambre_bus::GridRange)
}

// As-built: `TrameState::export_csv` (`logic/interop.rs`). Read-only. The bounds
// come from `used_extent` -- a scan of the `cells_{sheet}` map (source of truth),
// NOT `row_by_pos.len() - 1`: the row/column lists stay materialized at their
// high-water mark after a scroll, so their length would pad the export with empty
// trailing rows; and an empty sheet yields `None`, so there is no `len - 1`
// underflow. Each cell is written by its **displayed** value via `export_cell_text`
// (formula result through the engine, keyed by `CellId::bound`, not a `CellRef`
// into `formula_cache`).
impl TrameState {
    pub(super) fn export_csv(
        &self, sheet_id: &str, delimiter: Option<u8>, range: Option<GridRange>,
    ) -> TrameOutcome { /* ... */ }
}
```

---

## 5. EXPORT PDF

High-fidelity PDF export runs in the **Logic thread** via `ambre_pdf::KrillaSurface` (the `DrawSurface` implementation owned by the shared `ambre-pdf` crate, not by an app crate; lazy-loaded). The UI Thread only triggers the download upon receiving `TrameEvent::ExportComplete`.

**Flow:**

```
TrameCommand::ExportPdf { sheet_id, page_setup: Option<PdfPageSetup> }
  │   (None -> read the sheet's persisted "page_setup_{sheet_id}"; Some -> one-off override)
  │
  [Logic Thread]
  ├─ Computes the layout: which cells fit on which page
  ├─ Produces a Vec<DisplayPage> (list of paginated DisplayViewports)
  ├─ Generates the PDF via KrillaSurface (impl DrawSurface, lazy-loaded)
  │   └─ Reuses rendering functions parameterized on DrawSurface
  │      (same code as screen rendering — see spec-ambre-trame-ui.md §12)
  └─ TrameEvent::ExportComplete { filename, data: Vec<u8> }
       │
  [UI Thread]
  └─ trigger_download(&filename, &data)
```

`PdfPageSetup` is the resolved projection of the sheet's **persisted** `"page_setup_{sheet_id}"`
container (`trame-logic §2.B`, X67/X68): the export reads the shared page format rather than taking
it as an ephemeral argument, so two peers exporting the same sheet get the same layout. A one-off
export MAY override fields for a single run without mutating the container.

```rust
pub struct PdfPageSetup {
    pub orientation: PdfOrientation,    // Portrait | Landscape (first-class, not derived)
    pub paper_size:  PdfPaperSize,      // A4 | Letter | A3 | ...
    pub margins:     PdfMargins,        // top, right, bottom, left in mm
    pub scale:       f32,               // 1.0 = 100%
    pub fit_to_page: bool,              // auto-scale to fit on one page
    pub print_gridlines: bool,
    pub print_headers:   bool,          // row numbers + column letters
}

pub enum PdfOrientation { Portrait, Landscape }
pub enum PdfPaperSize { A4, A3, Letter, Legal }
```

The `krilla` implementation details are in `spec-ambre-trame-ui.md`.

---

## 6. GOOGLE SHEETS (MVP)

No direct Google Sheets API in WASM — authentication cookies and CORS block direct calls from a WASM Worker.

**MVP Import:** the user exports from Google Sheets as .xlsx or .csv, then imports into Trame. Standard flow via `ImportXlsx` or `ImportCsv`.

**MVP Export:** the user exports as .xlsx from Trame, then imports into Google Sheets.

**V2:** integration via a lightweight backend service (CORS proxy + Google OAuth) that fetches the Google Sheet as JSON and converts it to Trame CRDT operations. Content would transit encrypted to Trame upon client-side reception.

---

## 7. PERFORMANCE & RESOURCES

| Operation | Target | Note |
|---|---|---|
| Import .xlsx 1000 cells | < 500 ms | Logic thread, includes Loro mutations |
| Import .xlsx 10k cells | < 3 s | Displayed with progress indicator |
| Import CSV 10k lines | < 200 ms | Very fast parsing |
| Export .xlsx 10k cells | < 1 s | `rust_xlsxwriter` in memory |
| Export CSV 10k lines | < 100 ms | |

> **These targets are not yet `criterion` benches (X66).** Like the engine budgets of `trame-logic`,
> a number with no bench behind it is unfalsifiable prose (architecture §4, mechanism ladder). Prose's
> import/export subsystem carries a per-format `criterion` table (`prose-interop`); Trame's shared
> import/export path does not yet. Each row above becomes a falsifiable benchmark when the format's
> path is built (test debt, Epic `ET`, §8).

**Progress for long imports (deferred — must fit the synchronous model).**
`handle_command` runs to completion synchronously (`-logic.md` §8), so an import
cannot yield mid-operation and there is **no** `ImportProgress` bus event today. A
long import produces its single [`ImportComplete`] outcome when it finishes. If
progress reporting is ever needed, it must reuse the additive
`ContinueDirtyCompute`/`ComputePending` re-entrancy mechanism of `-logic.md` §8
(re-enqueue a continuation command on the Logic channel between batches) rather
than a re-entrant import call -- the CSV import (§4.A) already runs as one
synchronous pass.

---

## SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

### Boundary Audit

Every conversion runs client-side in the Logic thread. The outbound flows and
where their content is sealed:

| Flow | Data | Encryption |
|------|------|------------|
| Import (.xlsx / .csv) | File bytes read locally in the Logic thread; the conversion produces a Loro delta -> `SyncCommand::SendUpdate` | The delta is sealed in `ambre-sync`. No file byte reaches the server (`TRAME-IO-001`) |
| Export (.xlsx / .csv / PDF) | `TrameEvent::Export*` carries `Vec<u8>` to the UI thread over shared WASM memory; the UI thread triggers a local browser download | No process boundary; the bytes leave the device only by the user's own download action, never to `ambre-server` |
| Google Sheets (MVP) | None: no direct API call from WASM. The user round-trips through .xlsx / .csv | No outbound flow specific to this feature in the MVP |
| Google Sheets (V2, out of MVP scope) | A backend proxy would fetch the sheet as JSON | Content would be encrypted on client reception; the design is deferred and MUST be re-audited when built |

`ambre-server` declares no interop crate; `csv`, the Excel reader, and the PDF
surface are declared by `ambre-trame` (Logic thread) alone. The `deny.toml`
capability wall holds the relay's boundary. No plaintext file crosses to the
server.

### Cryptography

This audit identifies the flows only. The proof that the CRDT seal is correct
belongs to the Sync and crypto specs in `1-Foundations/`; it MUST NOT be
re-derived here.

---

## 8. QUALITY & TESTABILITY

> What the built paths guarantee, and what verifies each guarantee, is the table in §1. This section used to list 12 `#[test]` names, of which **2** existed -- the ten others named tests for the .xlsx and PDF code that has never been written, in the same format as the two real ones.

* **Tier.** host, entirely. Conversion is pure Rust in the Logic thread; a decoder needs no browser.
* **Mandatory belts per format.** A new import path lands with: a round trip (import -> export -> re-import, identical values), a malformed-input case and a random-bytes case -- **the decoder is fed by user files, so totality is not optional** (architecture §4 names decoders as the first `cargo-fuzz` targets, and none exists yet).
* **Unwritten .xlsx and PDF paths.** Their rules are `TRAME-IO-010..020` in §1, not prose here. Each lands with a `Verification` cell naming its test on the day the path exists.
* **Definition of Done.** An increment is `[x]` only when its belts are green in CI **and** no invariant in §1 is left without a `Status`.

---

## 9. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **`umya-spreadsheet`** | .xlsx read + write in Rust, but not actively maintained and less mature than separate `calamine` + `rust_xlsxwriter`. |
| **`xlsx` crate (Node bindings)** | Outside the Rust-first vision. |
| **Server-side .xlsx conversion** | Would violate Zero-Knowledge — the server would see the plaintext content. |
| **Direct Google Sheets API from WASM** | CORS + OAuth authentication impossible from a Web Worker without a backend proxy. Import/export via .xlsx for MVP. |
| **PDF export in the UI Thread** | Would block rendering (>500ms for a large document). `KrillaSurface` is instantiated in the Logic thread (lazy-loaded) — the UI Thread receives `TrameEvent::ExportComplete` and triggers the download. See `spec-ambre-architecture.md` §5 and `ambre-spec-ui.md` §2.B. |

---

## 10. ROADMAP

**Phase 1 — MVP (Epic E3)**
- [x] CSV import: `TrameCommand::ImportCsv` (auto-detect delimiter)
- [x] CSV export: `TrameCommand::ExportCsv`
- [x] `TrameEvent::ImportComplete` / `ExportComplete` returned in `dispatch()`'s `Vec<TrameEvent>` (CSV path)
- [x] **Tested DoD (test debt, Epic `ET`).** CSV import/export roundtrip green on the **host** tier in CI (`csv_import_export_roundtrip`, §8); plus fuzz/robustness cases proving malformed CSV input never panics (`csv_import_malformed_never_panics` -- ragged rows, embedded quotes/newlines, mixed delimiters, invalid UTF-8 -> graceful result; `csv_import_random_bytes_never_panics` -- deterministic byte-soup fuzz, no `unwrap` blow-up). The xlsx `#[test]`s in §8 follow their Phase 2 implementation. **DoD:** the CSV belt green in CI for the MVP.

**Phase 2 — V1 (Epic E5, `Trame Phase 2+`)**
- [ ] Basic .xlsx import: `TrameCommand::ImportXlsx` (values, simple formulas, basic styles, merges)
- [ ] Excel formula conversion via `formula_parser::parse(formula, sheet_index, sheet_name_to_id)` — cross-sheet resolution
- [ ] Full .xlsx export: `TrameCommand::ExportXlsx` (styles, merges, column widths, row heights)
- [ ] .xlsx import: data validations, named ranges, comments
- [ ] .xlsx import: cross-sheet references (`Sheet2!A1` via `sheet_name_to_id`)
- [ ] Progress for long imports: `TrameEvent::ImportProgress` (500-cell batching)

**Phase 3 — Post-V1**
- [ ] PDF export: `TrameCommand::ExportPdf` → `ambre_pdf::KrillaSurface` (shared crate) driven from the Trame PDF pipeline in the **Logic thread** (`ambre-trame/src/export_pdf.rs`, lazy-loaded) → `TrameEvent::ExportComplete`
- [ ] .ods (OpenDocument Spreadsheet) import via `calamine` (already supported by the crate)
- [ ] Google Sheets integration via backend proxy (V2)
