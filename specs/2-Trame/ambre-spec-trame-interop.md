# AMBRE — SPEC: AMBRE-TRAME INTEROP (Import / Export)
**Type:** Vertical (Feature / Application — Interop)
**Status:** Draft

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

**Invariants:**
- **Zero-Knowledge**: all conversions happen in the Logic thread. No unencrypted file transits to the server.
- **Client-side only**: import/export crates are compiled to WASM.
- **No `unsafe`** in application code.

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
) {
    // A formula takes priority over the cached value; both resolve to a raw string.
    let raw: String = if let Some(f) = formula {
        // convert_excel_formula needs the sheet index + name map (section D).
        match convert_excel_formula(f, sheet_index, sheet_name_to_id) {
            ImportedCellKind::Formula(r) | ImportedCellKind::UnsupportedFormula(r) => r,
            ImportedCellKind::Literal(r) => r,
        }
    } else {
        match cell_value {
            calamine::DataType::Int(n)      => n.to_string(),
            calamine::DataType::Float(f)    => format_float(*f),
            // Dates stay Excel serial numbers -- the engine's date functions use
            // serials (epoch 1899-12-30), NOT ISO strings.
            calamine::DataType::DateTime(f) => format_float(*f),
            calamine::DataType::String(s)   => s.clone(),
            calamine::DataType::Bool(b)     => if *b { "TRUE".into() } else { "FALSE".into() },
            calamine::DataType::Error(_)    => return,   // skip error cells
            calamine::DataType::Empty       => return,   // no entry for an empty cell
        }
    };

    let _ = state.materialize(row, col);
    state.write_cell(row, col, &raw);          // flat: raw string is the cell value
    if let Some(s) = style { state.set_cell_style_at(row, col, s); }
}

fn format_float(f: f64) -> String {
    // Avoids scientific notation on small numbers, preserves precision
    if f == f.trunc() && f.abs() < 1e15 { format!("{:.0}", f) } else { format!("{f}") }
}
```

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
fn convert_excel_formula(
    excel_formula: &str,
    sheet_index:   &SheetIndex,         // explicit parameter -- no implicit current_sheet
    sheet_name_to_id: &HashMap<String, SheetId>,
) -> ImportedCellKind {
    // Normalization: ";" -> "," (argument separator per Excel locale)
    let normalized = excel_formula.replace(';', ",");

    match formula_parser::parse(&normalized, sheet_index, sheet_name_to_id) {
        Ok(_ast) => ImportedCellKind::Formula(format!("={}", normalized)),
        Err(_)   => {
            // Unsupported formula -> opaque prefix for faithful round-trip
            // The "\x00EXCEL:" prefix is a null byte -- never typed by a user
            ImportedCellKind::UnsupportedFormula(
                format!("\x00EXCEL:={}", normalized)
            )
        }
    }
}

fn import_cell_with_kind(
    kind: ImportedCellKind,
    cell_map: &mut loro::LoroMap,
) {
    match kind {
        ImportedCellKind::Formula(raw) => {
            cell_map.insert("v", raw);
            cell_map.insert("t", "f");
        }
        ImportedCellKind::UnsupportedFormula(raw) => {
            // Stored as-is -- the UI displays an "unsupported Excel formula" indicator
            cell_map.insert("v", raw);
            cell_map.insert("t", "x"); // type "x" = unsupported Excel formula
        }
        ImportedCellKind::Literal(raw) => {
            cell_map.insert("v", raw);
            // type inferred at evaluation
        }
    }
}
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
fn xf_styles(workbook: &Xlsx<Cursor<Vec<u8>>>) -> Vec<CellStyle> {
    // One resolved CellStyle per xf index; `import_cell` looks up by the cell's xf.
    workbook.xf_formats().map(|xf| CellStyle {
        bold:     xf.font.bold,
        italic:   xf.font.italic,
        fg_color: xf.font.color.map(to_color),
        bg_color: xf.fill.bg_color.map(to_color),
        fmt:      xf.num_fmt.as_deref().map(number_format_from_excel).unwrap_or_default(),
        ..CellStyle::default()
    }).collect()
}
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
fn export_xlsx(state: &TrameState, sheet_ids: Option<Vec<SheetId>>) -> Result<Vec<u8>, XlsxError> {
    use rust_xlsxwriter::{Workbook, Worksheet, Format};

    let mut workbook = Workbook::new();
    let sheets = sheet_ids.unwrap_or_else(|| state.all_sheet_ids());

    for sheet_id in sheets {
        let sheet_index = state.build_sheet_index(&sheet_id);
        let ws: &mut Worksheet = workbook.add_worksheet();
        // rust_xlsxwriter rejects names > 31 chars / forbidden chars / duplicates;
        // propagate the error rather than panic on a user-chosen sheet name.
        ws.set_name(&state.sheet_display_name(&sheet_id))?;

        // Columns: widths
        for (col_pos, col_id) in sheet_index.col_by_pos.iter().enumerate() {
            if let Some(w) = state.col_width(&sheet_id, *col_id) {
                ws.set_column_width(col_pos as u16, w as f64).ok();
            }
        }

        // Rows and cells
        for (row_pos, row_id) in sheet_index.row_by_pos.iter().enumerate() {
            if let Some(h) = state.row_height(&sheet_id, *row_id) {
                ws.set_row_height(row_pos as u32, h as f64).ok();
            }

            for (col_pos, col_id) in sheet_index.col_by_pos.iter().enumerate() {
                // Positional cell (sheet is the engine u16 index).
                let cell = CellId::bound(idx, *row_id, *col_id);
                let raw = state.get_raw(cell);
                if raw.is_empty() { continue; }

                let fmt = state.cell_style_as_xlsx_format(cell, &workbook);

                if raw.starts_with('=') {
                    // The CRDT raw is id-qualified (`=#s0!A1`); rewrite `#id -> name`
                    // to a name-qualified Excel formula before writing (§12 mapping,
                    // -logic.md §12). The raw `#s0!A1` must never reach the file.
                    let excel = state.formula_to_excel(&raw, &sheet_index);
                    ws.write_formula_with_format(row_pos as u32, col_pos as u16, &excel, &fmt).ok();
                } else if let Some(CellValue::Number(n)) = state.evaluated_value(cell) {
                    // Numeric-vs-text is decided by the cell's **evaluated typed
                    // value**, not by formula_cache membership: the engine already
                    // parses every literal to a `CellValue`, so a plain "42" is a
                    // `Number` and exports numeric, while "N/A" stays text. Reusing
                    // `evaluated_value` keeps a single source of typing truth (no
                    // re-parse of the raw string on the export side).
                    ws.write_number_with_format(row_pos as u32, col_pos as u16, n, &fmt).ok();
                } else {
                    ws.write_with_format(row_pos as u32, col_pos as u16, raw.as_str(), &fmt).ok();
                }
            }
        }

        // Merges
        for (master_key, extent_key) in state.load_merges(&sheet_id) {
            let (r1, c1) = state.parse_cell_key_to_pos(&master_key, &sheet_index);
            let (r2, c2) = state.parse_cell_key_to_pos(&extent_key, &sheet_index);
            ws.merge_range(r1, c1, r2, c2, "", &Format::default()).ok();
        }
    }

    workbook.save_to_buffer()   // propagate, do not unwrap
}
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
    ) -> TrameOutcome {
        let delim = delimiter.unwrap_or_else(|| detect_csv_delimiter(&data));
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delim)
            .has_headers(false)   // we drop the header ourselves, uniform read
            .flexible(true)       // real exports have ragged rows
            .from_reader(data.as_slice());
        let mut records: Vec<Vec<String>> = reader.records()
            .filter_map(Result::ok)
            .map(|rec| rec.iter().map(str::to_owned).collect())
            .collect();
        if has_header && !records.is_empty() { records.remove(0); }
        if records.is_empty() { return TrameOutcome::display_only(vec![]); }

        let _ = self.undo_manager.group_start();
        // Reuse an existing id, else (incl. None) mint a fresh "CSV Import" sheet.
        let sheet_id = match target_sheet_id {
            Some(id) if self.all_sheet_ids().iter().any(|s| s == &id) => id,
            _ => self.create_import_sheet("CSV Import"),
        };
        self.active_sheet = sheet_id.clone();
        self.reseed_active_sheet();

        let rows = records.len() as u32;
        let cols = records.iter().map(Vec::len).max().unwrap_or(0) as u32;
        // Pre-materialize, then write -- so a materialize commit (excluded origin)
        // never splits the single undo item (same reasoning as SetCellsBatch).
        for (r, record) in records.iter().enumerate() {
            for (c, value) in record.iter().enumerate() {
                if !value.is_empty() { let _ = self.materialize(start_row + r as u32, start_col + c as u32); }
            }
        }
        for (r, record) in records.into_iter().enumerate() {
            for (c, value) in record.into_iter().enumerate() {
                if !value.is_empty() { self.write_cell(start_row + r as u32, start_col + c as u32, &value); }
            }
        }

        let mut outcome = self.finish_cell_writes();
        outcome.display.push(self.sheet_list_event());
        outcome.display.push(TrameEvent::ImportComplete { sheet_id, rows, cols });
        outcome
    }
}

fn detect_csv_delimiter(data: &[u8]) -> u8 {
    // Count ',' vs ';' in the first kilobyte; comma wins a tie.
    let sample = &data[..data.len().min(1024)];
    let commas     = sample.iter().filter(|&&b| b == b',').count();
    let semicolons = sample.iter().filter(|&&b| b == b';').count();
    if semicolons > commas { b';' } else { b',' }
}
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
    ) -> TrameOutcome {
        let Some(&idx) = self.sheet_id_to_u16.get(sheet_id) else {
            return TrameOutcome::display_only(vec![]);   // unknown sheet
        };
        let index = if idx == 0 { self.index.clone() } else { self.build_sheet_index(sheet_id) };
        let (row_start, row_end, col_start, col_end) = match range {
            Some(r) => (r.row, r.row_end, r.col, r.col_end),
            None => match self.used_extent(sheet_id, &index) {
                None => return TrameOutcome::display_only(vec![]),   // empty sheet
                Some((max_row, max_col)) => (0, max_row, 0, max_col),
            },
        };

        let delim = delimiter.unwrap_or(b',');
        let mut writer = csv::WriterBuilder::new().delimiter(delim).from_writer(Vec::new());
        for row in row_start..=row_end {
            let record: Vec<String> = (col_start..=col_end)
                .map(|col| self.export_cell_text(sheet_id, idx, &index, row, col))
                .collect();
            let _ = writer.write_record(&record);
        }
        let data = writer.into_inner().unwrap_or_default();   // no panic on flush
        TrameOutcome::display_only(vec![TrameEvent::ExportComplete {
            filename: format!("{}.csv", self.sheet_display_name(sheet_id)),
            data,
        }])
    }
}
```

---

## 5. EXPORT PDF

High-fidelity PDF export runs in the **Logic thread** via `KrillaSurface` (implementation of the `DrawSurface` trait defined in `ambre-draw`, lazy-loaded). The UI Thread only triggers the download upon receiving `TrameEvent::ExportComplete`.

**Flow:**

```
TrameCommand::ExportPdf { sheet_id, page_setup: PdfPageSetup }
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

```rust
pub struct PdfPageSetup {
    pub orientation: PdfOrientation,    // Portrait | Landscape
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

## 8. QUALITY & TESTABILITY

```rust
// Import/Export roundtrip
#[test] fn xlsx_import_export_roundtrip()              { /* import -> export -> re-import -> same values */ }
#[test] fn csv_import_export_roundtrip()               { /* same */ }
#[test] fn xlsx_import_formulas_converted()            { /* =SUM(A1:A10) correctly parsed */ }
#[test] fn xlsx_import_broken_formula_warned()         { /* unsupported function -> warning */ }
#[test] fn xlsx_import_styles_deduplicated()           { /* two cells same style -> same style_id */ }
#[test] fn xlsx_import_merges()                        { /* merges correctly imported */ }
#[test] fn xlsx_import_hidden_sheets()                 { /* hidden: true in LoroDoc */ }
#[test] fn xlsx_export_cell_ref_to_a1()                { /* CellRef -> A1 via SheetIndex */ }
#[test] fn xlsx_export_broken_ref_becomes_ref_error()  { /* broken ref -> #REF! in xlsx */ }
#[test] fn csv_delimiter_autodetect()                  { /* comma vs semicolon */ }
#[test] fn csv_import_extends_rows_cols_as_needed()    { }
#[test] fn import_progress_batching_allows_p1_p2()     { /* scroll during import */ }
```

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
- [ ] PDF export: `TrameCommand::ExportPdf` → `KrillaSurface` in the **Logic thread** (`ambre-trame/src/export_pdf.rs`, lazy-loaded) → `TrameEvent::ExportComplete`
- [ ] .ods (OpenDocument Spreadsheet) import via `calamine` (already supported by the crate)
- [ ] Google Sheets integration via backend proxy (V2)
