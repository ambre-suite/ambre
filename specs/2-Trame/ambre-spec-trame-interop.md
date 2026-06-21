# AMBRE — SPEC: AMBRE-TRAME INTEROP (Import / Export)
**Type:** Vertical (Feature / Application — Interop)
**Status:** Draft

> Import and export of third-party formats for Trame. All conversions run client-side in the **Logic thread** (WASM) — the server never sees the plaintext content. Supported formats: Excel (.xlsx), CSV, PDF.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**
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
| `ambre-trame` (internal) | `TrameController`, `SheetIndex`, `CellValue`, CRDT schema | Loro doc access for mutation |

> **`calamine` vs `umya-spreadsheet`**: `calamine` is read-only — sufficient for import. For .xlsx export, `rust_xlsxwriter` is the only mature pure Rust crate to date. Both are WASM-compatible.

---

## 2. IMPORT EXCEL (.xlsx)

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
  ├─ Import named styles (shared in "styles" LoroMap)
  ├─ Import named ranges
  └─ subscribe_local_update → SyncCommand::SendUpdate (full snapshot)
```

### B. UI Command

```rust
// From the UI Thread
TrameCommand::ImportXlsx { data: Vec<u8> }   // bytes du fichier .xlsx

// Response
TrameEvent::ImportComplete {
    version:      u64,
    sheets_count: u32,
    warnings:     Vec<ImportWarning>,         // unsupported features
}

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
fn import_cell(
    cell_value: &calamine::DataType,
    formula:    Option<&str>,
    style_id:   Option<u32>,
    controller: &mut TrameController,
    cell_ref:   CellRef,
) {
    let (raw, type_hint) = match cell_value {
        calamine::DataType::Int(n)     => (n.to_string(), "n"),
        calamine::DataType::Float(f)   => (format_float(*f), "n"),
        calamine::DataType::String(s)  => (s.clone(), "s"),
        calamine::DataType::Bool(b)    => (b.to_string(), "b"),
        calamine::DataType::DateTime(f) => (excel_date_to_iso(*f), "n"),
        calamine::DataType::Error(_)   => ("".to_string(), ""),
        calamine::DataType::Empty      => return, // empty cell = no entry in LoroMap
    };

    // If a formula is available, it takes priority over the value
    let final_raw = if let Some(f) = formula {
        convert_excel_formula(f, controller) // see section D
    } else {
        raw
    };

    // Mutation Loro
    let map = controller.doc.get_map(&format!("cells_{}", cell_ref.sheet.0));
    let cell_key = format!("{}:{}", cell_ref.row.to_key_part(), cell_ref.col.to_key_part());
    let cell_map = map.get_or_create_map(&cell_key);
    cell_map.insert("v", final_raw);
    cell_map.insert("t", type_hint.to_string());
    if let Some(sid) = style_id {
        cell_map.insert("s", format!("style_{:06}", sid));
    }
}

fn format_float(f: f64) -> String {
    // Avoids scientific notation on small numbers, preserves precision
    if f == f.trunc() && f.abs() < 1e15 {
        format!("{:.0}", f)
    } else {
        format!("{}", f) // Rust Display for f64 is sufficient
    }
}

fn excel_date_to_iso(serial: f64) -> String {
    // Excel date serial -> ISO 8601 string
    // Excel epoch = January 1, 1900 (with the Lotus 1-2-3 bug for 1900)
    let days = serial as i64 - 25569; // offset to Unix epoch
    let secs = days * 86400;
    // Convert to YYYY-MM-DD date via the `time` crate or manual calculation
    format_unix_date(secs)
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

```rust
fn import_styles(workbook: &Xlsx<Cursor<Vec<u8>>>, controller: &mut TrameController) {
    // calamine exposes cell formats via XlsxCellFormat
    // Mapped to Trame's "styles" LoroMap schema
    // Deduplication: two cells with the same style share a style_id

    let mut style_cache: HashMap<StyleKey, String> = HashMap::new();
    let styles_map = controller.doc.get_map("styles");

    for (xf_index, xf) in workbook.xf_formats().enumerate() {
        let style_key = StyleKey::from_xf(xf); // deterministic hashing
        let style_id = style_cache.entry(style_key.clone()).or_insert_with(|| {
            let id = format!("style_{:06}", style_cache.len());
            let style_map = styles_map.get_or_create_map(&id);
            style_map.insert("bold",   xf.font.bold);
            style_map.insert("italic", xf.font.italic);
            if let Some(color) = xf.font.color { style_map.insert("fg", color.argb()); }
            if let Some(color) = xf.fill.bg_color { style_map.insert("bg", color.argb()); }
            if let Some(fmt) = &xf.num_fmt { style_map.insert("fmt", fmt.clone()); }
            id
        });
    }
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
// Command from the UI Thread
TrameCommand::ExportXlsx { sheet_ids: Option<Vec<SheetId>> } // None = all sheets

// Response: .xlsx file bytes
TrameEvent::ExportComplete {
    version:  u64,
    filename: String,             // "{document_name}.xlsx"
    data:     Vec<u8>,            // bytes sent to the UI Thread via postMessage
                                  // the UI Thread triggers the download
}
```

### B. Implementation

```rust
fn export_xlsx(controller: &TrameController, sheet_ids: Option<Vec<SheetId>>) -> Vec<u8> {
    use rust_xlsxwriter::{Workbook, Worksheet, Format};

    let mut workbook = Workbook::new();
    let sheets = sheet_ids.unwrap_or_else(|| controller.all_sheet_ids());

    for sheet_id in sheets {
        let sheet_index = &controller.sheet_indices[&sheet_id];
        let ws: &mut Worksheet = workbook.add_worksheet();
        ws.set_name(&controller.sheet_name(sheet_id)).unwrap();

        // Columns: widths
        for (col_pos, col_id) in sheet_index.col_by_pos.iter().enumerate() {
            if let Some(w) = controller.col_width(sheet_id, *col_id) {
                ws.set_column_width(col_pos as u16, w as f64).ok();
            }
        }

        // Rows and cells
        for (row_pos, row_id) in sheet_index.row_by_pos.iter().enumerate() {
            if let Some(h) = controller.row_height(sheet_id, *row_id) {
                ws.set_row_height(row_pos as u32, h as f64).ok();
            }

            for (col_pos, col_id) in sheet_index.col_by_pos.iter().enumerate() {
                let cell = CellRef { sheet: sheet_id, row: *row_id, col: *col_id };
                let raw = controller.get_raw(cell);
                if raw.is_empty() { continue; }

                let fmt = controller.cell_style_as_xlsx_format(cell, &workbook);

                if raw.starts_with('=') {
                    ws.write_formula_with_format(row_pos as u32, col_pos as u16, &raw, &fmt).ok();
                } else if let CellValue::Number(n) =
                    controller.formula_cache.get(&cell).unwrap_or(&CellValue::Empty)
                {
                    ws.write_number_with_format(row_pos as u32, col_pos as u16, *n, &fmt).ok();
                } else {
                    ws.write_with_format(row_pos as u32, col_pos as u16, raw.as_str(), &fmt).ok();
                }
            }
        }

        // Merges
        for (master_key, extent_key) in controller.load_merges(sheet_id) {
            let (r1, c1) = controller.parse_cell_key_to_pos(&master_key, sheet_index);
            let (r2, c2) = controller.parse_cell_key_to_pos(&extent_key, sheet_index);
            ws.merge_range(r1, c1, r2, c2, "", &Format::default()).ok();
        }
    }

    workbook.save_to_buffer().unwrap()
}
```

**Formula fidelity on export:** Trame formulas use internal stable `CellRef`s. On export, they are converted to Excel positional references (`A1`) via the `SheetIndex`. If a `CellRef` points to a deleted row/column (broken ref) → exported as `#REF!`.

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

fn import_csv(controller: &mut TrameController, cmd: ImportCsvCmd) {
    use csv::ReaderBuilder;

    let delimiter = cmd.delimiter.unwrap_or_else(|| detect_csv_delimiter(&cmd.data));
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(cmd.has_header)
        .from_reader(cmd.data.as_slice());

    let sheet_id = cmd.target_sheet_id.unwrap_or_else(|| {
        controller.create_sheet("Import CSV".into())
    });

    let sheet_index = controller.sheet_indices.get_mut(&sheet_id).unwrap();

    // Extend rows/columns if needed
    let records: Vec<Vec<String>> = reader.records()
        .filter_map(|r| r.ok())
        .map(|r| r.into_iter().map(String::from).collect())
        .collect();

    // Add required rows to LoroMovableList
    let rows_needed = cmd.start_row as usize + records.len();
    let cols_needed = cmd.start_col as usize + records.first().map(|r| r.len()).unwrap_or(0);
    controller.ensure_rows(sheet_id, rows_needed);
    controller.ensure_cols(sheet_id, cols_needed);

    for (row_offset, record) in records.iter().enumerate() {
        let row_pos = cmd.start_row as usize + row_offset;
        for (col_offset, value) in record.iter().enumerate() {
            let col_pos = cmd.start_col as usize + col_offset;
            if let Some((row_id, col_id)) = sheet_index.resolve(row_pos, col_pos) {
                let cell = CellRef { sheet: sheet_id, row: row_id, col: col_id };
                controller.apply_edit_cell(cell, value);
            }
        }
    }
}

fn detect_csv_delimiter(data: &[u8]) -> u8 {
    // Count occurrences of ',' vs ';' in the first 10 lines
    let sample = &data[..data.len().min(1024)];
    let commas     = sample.iter().filter(|&&b| b == b',').count();
    let semicolons = sample.iter().filter(|&&b| b == b';').count();
    if semicolons > commas { b';' } else { b',' }
}
```

### B. Export CSV

```rust
TrameCommand::ExportCsv {
    sheet_id:  SheetId,
    delimiter: Option<u8>,       // None = comma
    range:     Option<CsvRange>, // None = entire sheet
}

pub struct CsvRange {
    pub row_start: u32, pub row_end: u32,
    pub col_start: u32, pub col_end: u32,
}

fn export_csv(controller: &TrameController, cmd: ExportCsvCmd) -> Vec<u8> {
    let delimiter = cmd.delimiter.unwrap_or(b',');
    let sheet_index = &controller.sheet_indices[&cmd.sheet_id];
    let mut output = Vec::new();
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(&mut output);

    let (row_start, row_end, col_start, col_end) = match cmd.range {
        Some(r) => (r.row_start, r.row_end, r.col_start, r.col_end),
        None    => (0, sheet_index.row_by_pos.len() as u32 - 1,
                    0, sheet_index.col_by_pos.len() as u32 - 1),
    };

    for row_pos in row_start..=row_end {
        let mut record = Vec::new();
        for col_pos in col_start..=col_end {
            let cell_val = sheet_index.resolve(row_pos as usize, col_pos as usize)
                .and_then(|(row_id, col_id)| {
                    let cell = CellRef { sheet: cmd.sheet_id, row: row_id, col: col_id };
                    controller.formula_cache.get(&cell).cloned()
                        .or_else(|| {
                            let raw = controller.get_raw(cell);
                            if raw.is_empty() { None } else { Some(CellValue::Text(raw)) }
                        })
                });

            record.push(match cell_val {
                Some(CellValue::Number(n))  => format_float(n),
                Some(CellValue::Text(s))    => s,
                Some(CellValue::Boolean(b)) => if b { "TRUE".into() } else { "FALSE".into() },
                Some(CellValue::Error(e))   => format!("#{:?}", e),
                _                           => String::new(),
            });
        }
        writer.write_record(&record).ok();
    }
    writer.flush().ok();
    drop(writer);
    output
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

**Progress for long imports:**

```rust
// For imports > 500ms, send progress updates
TrameEvent::ImportProgress { percent: u8, current_sheet: String }
```

Imports are split into batches of 500 cells with `yield` between each batch to allow P1/P2 message processing (scroll, input) during import.

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
