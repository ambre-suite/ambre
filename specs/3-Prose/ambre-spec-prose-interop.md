# AMBRE — SPEC: PROSE / INTEROP
**Type:** Vertical (Feature/Widget)
**Status:** Draft

> Multi-format import and export pipeline for Prose. Bidirectional conversion between the
> native CRDT structure (`LoroDoc` Prose) and external formats: docx, Markdown, PDF, HTML,
> and native `.ambre` format. All conversions run client-side in the Logic thread
> (WASM) — no plaintext document transits through the server.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles:
- Exporting the Prose `LoroDoc` to docx, Markdown, PDF, HTML.
- Importing docx, Markdown, HTML into an initialized Prose `LoroDoc`.
- Serialization/deserialization of the native `.ambre` format (Loro snapshot + postcard + zstd + XChaCha20).
- Managing fidelity losses (best-effort) during conversions and reporting them to the user.

This component delegates:
- Actual CRDT mutations → `ProseState::handle_ui_command()` via `ProseCommand`.
- Native format encryption/decryption → `ambre-sync`.
- Live wgpu + Parley/Swash rendering (display list, virtual scroll) → `prose-ui`.
- Binary asset (image) management → `ambre-server` (presigned S3 URLs).

**Dependencies:**

| Crate | Role | Import / Export |
|-------|------|-----------------|
| `docx-rs` | OOXML `.docx` read and write in pure Rust, WASM-compatible | Import + Export |
| `pulldown-cmark` | CommonMark + GFM Markdown parser (tables, footnotes, strikethrough) | Import + Export |
| `pulldown-cmark-to-cmark` | Serialization of a `pulldown-cmark` event stream into Markdown | Export |
| `krilla` | High-fidelity PDF generation with Parley (vector text, hyperlinks, metadata) | Export only |
| `html5ever` | Tolerant HTML5 parser (used for HTML import and clipboard HTML) | Import |
| `markup5ever_rcdom` | In-memory DOM produced by `html5ever`, traversed for import | Import |
| `ambre-core` | `Uuid`, `DocType` | Both |
| `ambre-draw` | `DrawSurface` trait — `KrillaSurface` implemented here for PDF export | Export |
| `postcard` / `serde` | Serialization of conversion reports (`ConversionReport`) | Both |
| `ruzstd` | Compression/decompression of the native `.ambre` format | Both |

No dependency on `ambre-sync` or `ambre-crypto` — the encryption layer of the native format
is applied by `ambre-sync` downstream, not in `ambre-prose/interop`.

**Invariants:**

- Any conversion is **stateless server-side**: the entire pipeline runs in the Logic thread in WASM memory. No chunk is sent to the Blind Relay during conversion.
- An import exclusively produces typed `ProseCommand`s — it never mutates the `LoroDoc` directly. This guarantees all mutations pass through the controller invariants.
- Each export is **idempotent**: exporting the same document twice produces byte-identical files (up to metadata timestamp).
- Any fidelity loss is **explicitly reported** via `ConversionReport::warnings` — never silent.
- Images are exported as references (`blob_id`) in the CRDT and resolved at export via S3 presigned URL. An offline export without network access produces a document with missing images, reported in `ConversionReport`.

---

## 2. TECHNICAL DESIGN & STATE

### A. Pipeline Architecture

The conversion pipeline is symmetric: a **Reader** produces an intermediate representation
(`ProseIr`), a **Writer** consumes it to produce the target format.

```
Import:  Bytes (docx/md/html)
           → Reader (format-specific)
           → ProseIr
           → IrToCommands
           → Vec<ProseCommand>
           → ProseState::handle_ui_command() [by batch]

Export:  LoroDoc (snapshot read from ProseState)
           → DocToIr
           → ProseIr
           → Writer (format-specific)
           → Bytes (docx/md/pdf/html)
```

#### Intermediate Representation `ProseIr`

`ProseIr` is a format-independent linear AST, aligned with the block model of
`prose-engine`. It is not serialized — it is purely in memory during conversion.

```rust
// ambre-prose/src/interop/ir.rs

#[derive(Debug, Clone)]
pub struct ProseIr {
    pub meta:   IrMeta,
    pub blocks: Vec<IrBlock>,
    pub styles: Vec<IrStyle>,   // custom named styles
}

#[derive(Debug, Clone)]
pub struct IrMeta {
    pub title:    Option<String>,
    pub lang:     Option<String>,   // BCP 47
    pub page:     Option<IrPage>,
}

#[derive(Debug, Clone)]
pub struct IrPage {
    pub width_mm:        f32,
    pub height_mm:       f32,
    pub margin_top_mm:   f32,
    pub margin_bottom_mm: f32,
    pub margin_left_mm:  f32,
    pub margin_right_mm: f32,
}

#[derive(Debug, Clone)]
pub enum IrBlock {
    Paragraph  { style_id: Option<String>, para_style: IrParaStyle, runs: Vec<IrRun> },
    Heading    { level: u8, numbered: bool, runs: Vec<IrRun> },
    ListItem   { depth: u8, style: IrListStyle, runs: Vec<IrRun>, children: Vec<IrBlock> },
    Table      { col_widths: Vec<f32>, rows: Vec<IrTableRow> },
    CodeBlock  { lang: Option<String>, text: String },
    Image      { blob_id: Option<Uuid>, src_url: Option<String>, alt: String,
                 width_px: Option<u32>, height_px: Option<u32>,
                 align: Option<IrImageAlign>, caption: Option<String> },
    PageBreak,
    HRule,
    Toc        { max_level: u8, numbered: bool },
    Footnote   { id: Uuid, runs: Vec<IrRun> },
}

#[derive(Debug, Clone)]
pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

#[derive(Debug, Clone)]
pub struct IrTableCell {
    pub rowspan:  u32,
    pub colspan:  u32,
    pub blocks:   Vec<IrBlock>,  // Paragraph, Heading, ListItem -- no nested Table
}

#[derive(Debug, Clone)]
pub struct IrRun {
    pub text:          String,
    pub bold:          bool,
    pub italic:        bool,
    pub underline:     bool,
    pub strikethrough: bool,
    pub superscript:   bool,
    pub subscript:     bool,
    pub code:          bool,
    pub color:         Option<u32>,
    pub background:    Option<u32>,
    pub font_size:     Option<f32>,
    pub font_family:   Option<String>,
    pub link:          Option<String>,
    pub comment_id:    Option<Uuid>,
    pub suggest_ins:   Option<Uuid>,
    pub suggest_del:   Option<Uuid>,
    pub footnote_ref:  Option<Uuid>,
    pub cross_ref:     Option<(Uuid, IrCrossRefKind)>,
}

#[derive(Debug, Clone)]
pub struct IrParaStyle {
    pub align:        Option<IrTextAlign>,
    pub indent_first: Option<f32>,
    pub indent_left:  Option<f32>,
    pub space_before: Option<f32>,
    pub space_after:  Option<f32>,
    pub line_height:  Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub enum IrTextAlign { Left, Center, Right, Justify }

#[derive(Debug, Clone, Copy)]
pub enum IrImageAlign { Block, Left, Right, Center }

#[derive(Debug, Clone)]
pub enum IrListStyle { Bullet, Ordered(IrOrderedStyle), Todo(bool) }

#[derive(Debug, Clone, Copy)]
pub enum IrOrderedStyle { Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }

#[derive(Debug, Clone)]
pub enum IrCrossRefKind { PageNumber, SectionNumber, CustomText(String) }

#[derive(Debug, Clone)]
pub struct IrStyle {
    pub id:         String,
    pub name:       String,
    pub para_style: IrParaStyle,
    pub run_style:  IrRun,  // default values
}
```

#### Conversion Report

```rust
// ambre-prose/src/interop/report.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionReport {
    pub format:   ConversionFormat,
    pub warnings: Vec<ConversionWarning>,
    pub errors:   Vec<ConversionError>,     // non-fatal errors (missing images, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversionFormat { Docx, Markdown, Pdf, Html, Ambre }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversionWarning {
    StyleNotMapped   { style_name: String, fallback: String },
    FormattingLost   { description: String },
    ImageNotResolved { blob_id: Uuid },
    FeatureNotSupported { feature: String, format: ConversionFormat },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversionError {
    ParseFailed     { detail: String },
    RenderFailed    { detail: String },
}
```

---

### B. Native `.ambre` Format

The native format is the reference persistence and sharing format. It involves no data
conversion — it is a packaged Loro binary snapshot.

```
.ambre (binary)
├── [4 bytes] magic: 0x414D4252  ("AMBR")
├── [1 byte]  version: 0x01
├── [1 byte]  doc_type: DocType as u8  (0x00 = Prose)
├── [16 bytes] document_id: Uuid v7
└── [remaining] payload: LoroDoc::export_snapshot()
                          compressed zstd (ruzstd)
                          encrypted XChaCha20-Poly1305 by ambre-sync
```

Compression and encryption are applied by `ambre-sync` — `ambre-prose/interop`
only exposes `export_ambre_plaintext()` and `import_ambre_plaintext()` which operate on
the raw Loro snapshot (before compression/encryption).

```rust
// ambre-prose/src/interop/ambre.rs

pub const AMBRE_MAGIC: [u8; 4] = *b"AMBR";
pub const AMBRE_VERSION: u8 = 0x01;

/// Produces the header + uncompressed, unencrypted Loro snapshot.
/// ambre-sync applies zstd + XChaCha20 downstream.
pub fn export_ambre_plaintext(
    doc: &LoroDoc,
    doc_id: Uuid,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(22 + 1024);
    buf.extend_from_slice(&AMBRE_MAGIC);
    buf.push(AMBRE_VERSION);
    buf.push(DocType::Prose as u8);
    buf.extend_from_slice(doc_id.as_bytes());
    buf.extend(doc.export_snapshot());
    buf
}

/// Reads the header, checks magic + version + doc_type, returns the raw Loro snapshot.
pub fn import_ambre_plaintext(
    data: &[u8],
) -> Result<(Uuid, Vec<u8>), InteropError> {
    if data.len() < 22 { return Err(InteropError::TruncatedHeader); }
    if &data[0..4] != &AMBRE_MAGIC { return Err(InteropError::InvalidMagic); }
    if data[4] != AMBRE_VERSION    { return Err(InteropError::UnsupportedVersion { found: data[4] }); }
    if data[5] != DocType::Prose as u8 { return Err(InteropError::WrongDocType); }
    let doc_id = Uuid::from_bytes(data[6..22].try_into().unwrap());
    Ok((doc_id, data[22..].to_vec()))
}
```

---

### C. DOCX Export / Import

#### Prose Block → OOXML Mapping

| `IrBlock` | OOXML Element | Notes |
|-----------|--------------|-------|
| `Paragraph` | `<w:p>` with `<w:pPr><w:pStyle>` | style_id mapped to Word style name |
| `Heading { level }` | `<w:p><w:pStyle w:val="Heading{level}">` | Built-in Word styles |
| `ListItem` | `<w:p><w:pStyle w:val="ListParagraph">` + `<w:numPr>` | `numId` managed by `docx-rs` |
| `Table` | `<w:tbl>` + `<w:tr>` + `<w:tc>` | `col_widths` → `<w:tblGrid>` |
| `CodeBlock` | `<w:p><w:pStyle w:val="Code">` | Custom style injected if absent |
| `Image` | `<w:drawing>` + relationship `image/png` | Blob resolved from S3 at export time |
| `PageBreak` | `<w:p><w:r><w:br w:type="page">` | |
| `HRule` | `<w:p><w:pBdr><w:bottom>` | |
| `Toc` | `<w:sdt>` standard Word TOC field | `TOC \o "1-3"` field instruction |
| `Footnote` | `<w:footnote>` in `/word/footnotes.xml` | |

#### Prose Inline Mark → `<w:rPr>` Mapping

| Prose Mark | `<w:rPr>` Element |
|------------|-------------------|
| `bold` | `<w:b/>` |
| `italic` | `<w:i/>` |
| `underline` | `<w:u w:val="single"/>` |
| `strikethrough` | `<w:strike/>` |
| `superscript` | `<w:vertAlign w:val="superscript"/>` |
| `subscript` | `<w:vertAlign w:val="subscript"/>` |
| `code` (inline) | `<w:rStyle w:val="Code Char"/>` (custom style injected) |
| `color` | `<w:color w:val="rrggbb"/>` |
| `background` | `<w:highlight>` (8 Word colors) or `<w:shd>` for exact color |
| `font_size` | `<w:sz w:val="{pt*2}"/>` |
| `font_family` | `<w:rFonts w:ascii="{family}"/>` |
| `link` | `<w:hyperlink r:id="rId{n}">` with external relationship |
| `suggest_ins` | `<w:ins w:id="{n}" w:author="{name}" w:date="{ts}">` |
| `suggest_del` | `<w:del w:id="{n}" w:author="{name}" w:date="{ts}">` |
| `comment_id` | `<w:commentRangeStart>` / `<w:commentRangeEnd>` + `/word/comments.xml` |

#### Docx Fidelity Losses (reported in `ConversionReport`)

| Prose Feature | Docx Behavior |
|--------------|---------------|
| Unmappable Prose custom styles | Fallback to "Normal", `StyleNotMapped` warning |
| Non-standard `background` color | Approximated to nearest Word color (`<w:highlight>`), warning |
| Automatic section numbering | Not exported (Word manages its own numbering via fields), warning |
| Dynamic cross references (`CrossRefKind::PageNumber`) | Exported as Word `{ PAGE }` fields — updated on open in Word |
| `BlockType::Toc` | Exported as standard Word TOC field |

#### DOCX Import → `ProseIr`

Import uses `docx-rs` to parse the OOXML ZIP and traverse the XML document.
Traversal is sequential (no full DOM in memory) to limit memory footprint.

```rust
// ambre-prose/src/interop/docx.rs

pub fn import_docx(bytes: &[u8]) -> Result<(ProseIr, ConversionReport), InteropError> {
    let doc = docx_rs::read_docx(bytes).map_err(|e| InteropError::ParseFailed(e.to_string()))?;
    let mut ir = ProseIr::default();
    let mut report = ConversionReport::new(ConversionFormat::Docx);

    for child in &doc.document.body.children {
        match child {
            BodyChild::Paragraph(p)  => ir.blocks.push(parse_paragraph(p, &mut report)?),
            BodyChild::Table(t)      => ir.blocks.push(parse_table(t, &mut report)?),
            BodyChild::SectionProps(_) => {} // margins/orientation imported from first section
        }
    }
    // Footnotes, comments, suggestions imported from dedicated parts
    import_footnotes(&doc, &mut ir, &mut report)?;
    import_comments(&doc, &mut ir, &mut report)?;
    import_track_changes(&doc, &mut ir, &mut report)?;

    Ok((ir, report))
}
```

---

### D. Markdown Export / Import

Markdown (CommonMark + GFM) is a format with significant losses — it does not support
paragraph styles, pagination, comments, or track changes. Export is **best-effort**;
import is faithful for supported features.

#### Prose → Markdown GFM Export Mapping

| `IrBlock` | Markdown |
|-----------|----------|
| `Paragraph` | Paragraph separated by blank line |
| `Heading { level }` | `#{level} text` |
| `ListItem { Bullet }` | `- text` (nested with 2 spaces) |
| `ListItem { Ordered }` | `1. text` |
| `ListItem { Todo(checked) }` | `- [x] text` / `- [ ] text` (GFM) |
| `Table` | GFM table `\| col \| col \|` |
| `CodeBlock { lang }` | ` ```lang ``` ` |
| `Image` | `![alt](url)` — resolved S3 presigned URL |
| `PageBreak` | `---` (horizontal rule, approximation) |
| `HRule` | `---` |
| `Toc` | `<!-- TOC -->` (HTML comment, non-standard) |
| `Footnote` | `[^id]: text` (GFM footnotes) |

Inline marks lost on Markdown export (systematic warning): `superscript`, `subscript`,
`color`, `background`, `font_size`, `font_family`, `suggest_ins`, `suggest_del`,
`comment_id`, `cross_ref`.

```rust
// ambre-prose/src/interop/markdown.rs

pub fn export_markdown(ir: &ProseIr) -> (String, ConversionReport) {
    let mut events: Vec<pulldown_cmark::Event> = Vec::new();
    let mut report = ConversionReport::new(ConversionFormat::Markdown);

    for block in &ir.blocks {
        ir_block_to_md_events(block, &mut events, &mut report);
    }

    let mut md = String::new();
    pulldown_cmark_to_cmark::cmark(events.iter(), &mut md).unwrap();
    (md, report)
}

pub fn import_markdown(input: &str) -> (ProseIr, ConversionReport) {
    let parser = pulldown_cmark::Parser::new_ext(
        input,
        pulldown_cmark::Options::ENABLE_TABLES
            | pulldown_cmark::Options::ENABLE_FOOTNOTES
            | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS,
    );
    let mut ir = ProseIr::default();
    let mut report = ConversionReport::new(ConversionFormat::Markdown);
    md_events_to_ir(parser, &mut ir, &mut report);
    (ir, report)
}
```

---

### E. PDF Export

PDF export uses **Krilla** with Parley for typographic layout. Krilla produces a
PDF/A-compliant document with vector text, active hyperlinks (TOC, cross references),
and XMP metadata.

**`DrawSurface` architecture:** the `DrawSurface` trait is defined in the `ambre-draw`
crate (source of truth: `ambre-spec-architecture.md` §2.D). It depends on `kurbo`,
`peniko`, `parley` (types only — no direct dependency on the final rendering engine).

Page rendering functions (`render_block_on`, `render_page_on`, etc.) are parameterized
on this trait — rendering code is not duplicated between live rendering (wgpu +
Parley/Swash, UI thread) and export (Krilla, Logic thread):

```rust
// ambre-draw/src/lib.rs  -- SOURCE OF TRUTH (see ambre-spec-architecture.md sec.2.D)
use ambre_draw::DrawSurface;

// wgpu implementation -- UI thread (in ambre-app)
pub struct GpuSurface<'a> {
    pub device: &'a wgpu::Device,
    pub queue:  &'a wgpu::Queue,
}
impl DrawSurface for GpuSurface<'_> {
    // Delegates to the wgpu + Parley/Swash rendering pipeline.
    // Zero-cost wrapper -- no type conversion needed (pipeline uses kurbo/peniko).
}

// Krilla implementation -- Logic thread (in ambre-prose/interop, lazy-loaded)
pub struct KrillaSurface<'a> {
    pub surface: &'a mut krilla::Surface,
    pub fonts:   &'a parley::FontCollection,
}
impl DrawSurface for KrillaSurface<'_> {
    // Converts kurbo/peniko types to krilla primitives (PDF paths, glyphs, images).
    // krilla is designed to integrate with Parley -- conversion is direct.
}
```

**Execution in the Logic thread:** Parley is already available there for pagination.
The PDF is produced in memory (`Vec<u8>`) and transmitted to the UI thread via
`ProseEvent::ExportReady { format: Pdf, data: Vec<u8> }` for download.
Generating a 100-page document can exceed 500ms — blocking the UI thread during that
time would freeze the interface.

```rust
// ambre-prose/src/interop/pdf.rs

pub fn export_pdf(
    ir: &ProseIr,
    layout: &PageLayout,
    fonts: &FontCollection,
) -> Result<(Vec<u8>, ConversionReport), InteropError> {
    let mut report = ConversionReport::new(ConversionFormat::Pdf);
    let mut doc = krilla::Document::new();

    doc.set_title(ir.meta.title.as_deref().unwrap_or(""));
    doc.set_language(ir.meta.lang.as_deref().unwrap_or("fr-FR"));

    let page_size = krilla::PageSize {
        width:  ir.meta.page.as_ref().map_or(210.0, |p| p.width_mm) * MM_TO_PT,
        height: ir.meta.page.as_ref().map_or(297.0, |p| p.height_mm) * MM_TO_PT,
    };

    for page_idx in 0..layout.page_count {
        let mut page = doc.add_page(page_size);
        let mut krilla_surface = page.surface();
        let mut surface = KrillaSurface { surface: &mut krilla_surface, fonts };

        // Exact reuse of rendering code -- no duplication with prose-ui
        render_page_on(&mut surface, page_idx, ir, layout, &mut report);

        krilla_surface.finish();
        page.finish();
    }

    // TOC hyperlinks and cross references
    add_toc_links(&mut doc, ir, layout);

    let pdf_bytes = doc.finish().map_err(|e| InteropError::RenderFailed(e.to_string()))?;
    Ok((pdf_bytes, report))
}

const MM_TO_PT: f32 = 2.834_645_7;
```

**PDF features supported:**

| Feature | Support |
|---------|---------|
| Vector text (no text image) | ✓ via Parley + Krilla |
| TOC hyperlinks (clickable) | ✓ |
| Internal cross references (page, section) | ✓ |
| External links (`link` mark) | ✓ |
| Raster images (png, jpeg) | ✓ |
| Page numbers in footer | ✓ |
| XMP metadata (title, lang, author) | ✓ |
| PDF forms | ✗ (out of scope) |
| PDF/UA (accessibility) | Post-MVP |

---

### F. HTML Export / Import

#### HTML Export

HTML export produces a standalone semantic HTML5 document with optional inline CSS.
Use case: copy-paste to CMS, email, or web archiving.

```rust
// ambre-prose/src/interop/html.rs

pub fn export_html(ir: &ProseIr, inline_css: bool) -> (String, ConversionReport) {
    // Produces full <html><head><body>
    // Inline styles if inline_css = true (for emails), otherwise <style> in <head>
    todo!()
}
```

`IrBlock` → Semantic HTML Mapping:

| `IrBlock` | HTML |
|-----------|------|
| `Paragraph` | `<p>` |
| `Heading { level }` | `<h1>`..`<h6>` |
| `ListItem { Bullet }` | `<ul><li>` |
| `ListItem { Ordered }` | `<ol><li>` |
| `ListItem { Todo }` | `<ul class="todo"><li><input type="checkbox">` |
| `Table` | `<table><thead><tbody><tr><td>/<th>` |
| `CodeBlock { lang }` | `<pre><code class="language-{lang}">` |
| `Image` | `<img src="..." alt="...">` |
| `PageBreak` | `<hr class="page-break">` |
| `HRule` | `<hr>` |
| `Toc` | `<nav aria-label="Table of contents"><ol>` |

Inline Marks → HTML Attributes:

| Mark | HTML |
|------|------|
| `bold` | `<strong>` |
| `italic` | `<em>` |
| `underline` | `<u>` |
| `strikethrough` | `<s>` |
| `superscript` | `<sup>` |
| `subscript` | `<sub>` |
| `code` | `<code>` |
| `link` | `<a href="...">` |
| `color` / `background` | `style="color: #rrggbb"` / `style="background: #rrggbb"` |
| `font_size` | `style="font-size: {n}pt"` |
| `font_family` | `style="font-family: {family}"` |
| `suggest_ins` | `<ins data-suggestion-id="...">` |
| `suggest_del` | `<del data-suggestion-id="...">` |
| `comment_id` | `<span data-comment-id="...">` |
| `footnote_ref` | `<sup><a href="#fn-{id}">` |

#### HTML Import

HTML import uses `html5ever` + `markup5ever_rcdom` to parse potentially malformed HTML
(clipboard from browser, Google Docs HTML export, etc.).

```rust
// ambre-prose/src/interop/html.rs

pub fn import_html(input: &str) -> (ProseIr, ConversionReport) {
    let dom = parse_html5(input); // html5ever -> RcDom
    let mut ir = ProseIr::default();
    let mut report = ConversionReport::new(ConversionFormat::Html);
    traverse_dom(&dom.document, &mut ir, &mut report);
    (ir, report)
}
```

HTML import is also used for **clipboard paste** (`text/html` format) — the same pipeline
is reused, ensuring that pasting from Google Docs or Word Online produces a consistent
result.

#### Google Docs

There is no Google Docs API allowing direct content access from WASM without OAuth
authentication. Interoperability with Google Docs goes through two paths:

1. **Google Export → DOCX:** the user exports their Google Docs document as `.docx`
   (File → Download → Microsoft Word), then imports into Prose via the docx pipeline.
   This is the most faithful path.

2. **Copy-paste:** Google Docs places HTML in the clipboard (`text/html`) on copy.
   The `import_html` pipeline is used directly, with best-effort on styles.

Both paths are documented in the UI (tooltip on "Import from Google Docs").

---

### G. Interop IPC Commands

Import/export commands travel via `ProseCommand` / `ProseEvent` like all other
operations, so as not to short-circuit the controller.

```rust
// In ProseCommand:
ImportDocument  { format: ImportFormat, data: Vec<u8> },
ExportDocument  { format: ExportFormat, options: ExportOptions },

// In ProseEvent:
ImportCompleted { report: ConversionReport },
ExportReady     { format: ExportFormat, data: Vec<u8>, report: ConversionReport },
ImportProgress  { percent: u8 },   // for large documents (> 1 MB)
ExportProgress  { percent: u8 },

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportFormat { Ambre, Docx, Markdown, Html }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExportFormat { Ambre, Docx, Markdown, Pdf, Html }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub include_comments:      bool,   // comments in docx/html
    pub include_suggestions:   bool,   // track changes in docx/html
    pub html_inline_css:       bool,   // inline CSS for emails
    pub pdf_include_toc_links: bool,
    pub markdown_flavor:       MarkdownFlavor,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum MarkdownFlavor { #[default] Gfm, CommonMark }
```

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

**Main guarantee:** all conversions run in the Logic thread, in WASM memory, without any
transfer to the server during conversion.

| Flow | Data | Risk | Mitigation |
|------|---------|--------|------------|
| Import docx/md/html | `Vec<u8>` received from UI thread via `UiEnvelope` | The imported file may contain macros or scripts | `docx-rs` does not execute macros. `html5ever` sanitizes HTML (no JS execution). |
| PDF export with images | Image resolution via S3 presigned URL | The presigned URL temporarily exposes the S3 blob | Presigned URLs have a 5-minute TTL. PDF export is done in a single pass — no re-fetch. |
| Native `.ambre` format | Plaintext Loro snapshot in the Logic thread | Isolated WASM memory of the Logic thread | The Sync thread does not read the `LoroDoc` directly — it receives opaque `Vec<u8>` via `postMessage` (postcard serialized). Plaintext never leaves the Logic thread unencrypted. |
| Import HTML clipboard | HTML from `navigator.clipboard.read()` | Malicious HTML (injection) | `html5ever` sanitizes, `markup5ever_rcdom` does not preserve `on*` attributes or `<script>`. |

**Image resolution on export:**

```
Logic thread
  → ProseCommand::ExportDocument { format: Pdf }
  → for each IrBlock::Image { blob_id }:
      → SyncCommand::RequestPresignedUrl { blob_id }  [new variant, post-MVP]
      → LogicEvent::PresignedUrl { blob_id, url, ttl }
      → fetch(url) in the Logic thread (not the Sync thread — no crypto required)
      → image bytes included in the PDF
```

In V1 MVP, PDF exports with unresolved images produce a placeholder and an
`ImageNotResolved` warning — S3 resolution is post-MVP.

---

## 4. QUALITY & TESTABILITY

**Unit tests:**

| Case | Method |
|-----|---------|
| Round-trip `ProseIr` → Markdown → `ProseIr` | Assertions on preserved fields (text, heading level, list style) |
| Round-trip `ProseIr` → HTML → `ProseIr` | Same + inline mark verification |
| Export docx: valid OOXML structure | `docx-rs` parses the produced file without error |
| Import docx → `ProseIr`: headings, lists, tables, comments | Reference docx files in fixtures |
| Import docx track changes: `<w:ins>` / `<w:del>` → `suggest_ins` / `suggest_del` | docx fixture with revisions |
| `import_ambre_plaintext`: invalid magic, unknown version, incorrect doc_type | Appropriate `InteropError` |
| Import HTML clipboard Google Docs | HTML fixture exported from Google Docs (static snapshot) |
| `ConversionReport`: warnings generated for lost features | Assertions on `report.warnings` |

**Test fixtures:** a directory `ambre-prose/tests/fixtures/` contains:
- `reference.docx`: document with all features (headings, lists, tables, placeholder images, comments, revisions).
- `reference.md`: Markdown equivalent (supported features only).
- `reference.html`: Google Docs HTML export.
- `expected_ir.json`: expected `ProseIr` for each fixture (serialized for comparison).

Fixtures are static versioned files, never generated on the fly in tests.

**Benchmarks (criterion):**

| Benchmark | Target |
|-----------|-------|
| Import docx 100 pages (1 MB) | < 500 ms |
| Export docx 100 pages | < 300 ms |
| Export Markdown 100 pages | < 50 ms |
| Export PDF 100 pages (without images) | < 2 s |
| Import HTML 500 KB (clipboard) | < 100 ms |
| `export_ambre_plaintext` 100 pages | < 10 ms |

---

## 5. PERFORMANCE & RESOURCES

| Operation | Target | Thread | Notes |
|-----------|-------|--------|-------|
| Import docx 100 pages | < 500 ms | Logic | Sequential ZIP + XML parsing |
| Export docx 100 pages | < 300 ms | Logic | |
| Export Markdown 100 pages | < 50 ms | Logic | No layout required |
| Export PDF 100 pages (without images) | < 2 s | Logic | Parley layout per page |
| Export PDF 100 pages (with images) | < 5 s | Logic | + S3 fetch (post-MVP) |
| Import HTML clipboard 500 KB | < 100 ms | Logic | |
| `DocToIr` (LoroDoc → ProseIr) 100 pages | < 50 ms | Logic | MovableTree traversal |

**WASM size (interop crates):**

| Crate | Raw estimated | Gzip estimated |
|-------|-----------|-------------|
| `docx-rs` | ~300 KB | ~90 KB |
| `pulldown-cmark` + `pulldown-cmark-to-cmark` | ~80 KB | ~25 KB |
| `krilla` | ~400 KB | ~120 KB |
| `html5ever` + `markup5ever_rcdom` | ~200 KB | ~60 KB |
| `ambre-prose/interop` (logic) | ~40 KB | ~13 KB |
| **Total interop** | **~1.02 MB** | **~308 KB** |

> ⚠️ Interop crates represent a significant binary cost. If the total WASM budget is
> exceeded, use **lazy loading** (WASM bundle splitting): `docx-rs`, `krilla`,
> `html5ever`, and `pulldown-cmark` are excluded from the startup binary and dynamically
> loaded in the Logic thread on the first import/export operation. No additional Worker —
> the 3-Worker architecture remains unchanged. During import/export, the Logic thread is
> intentionally blocked (waiting screen on the UI side, incoming `postMessage` messages from
> the Sync thread accumulate in the Logic thread's JS event loop without being lost).

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **Server-side conversion** | Violates Zero-Knowledge principle: the plaintext document would leave for the server. All conversion happens in the Logic thread. |
| **LibreOffice WASM for docx** | Binary > 50 MB in WASM. Incompatible with < 2 MB gzip budget. |
| **`mammoth.js` for docx import** | JavaScript, outside the Rust-first vision. |
| **`pandoc` WASM** | Binary > 20 MB. Same. |
| **IR = `ProseCommand[]` directly** | An intermediate `ProseIr` IR allows validating, transforming, and reporting warnings before application. A direct command stream does not allow rollback on partial error. |
| **Direct Google Docs API** | Requires OAuth + network call to Google servers from WASM — metadata leak (when and which document is opened). Docx + clipboard path retained. |
| **`wasm-bindgen` + `fetch` for PDF** | PDF export is 100% local (Krilla + Parley). No external service. |
| **`printpdf` for PDF export** | Does not support Parley for typographic layout — text would be rendered as graphic primitives without proper ligatures or justification. Krilla is designed to integrate with Parley. |
| **Duplicating PDF/live rendering code** | Rendering functions parameterized on `DrawSurface` are shared between `WgpuSurface` (UI thread, live rendering) and `KrillaSurface` (Logic thread, PDF export). Duplicating rendering code for PDF would be a permanent source of divergence. |
| **PDF in the UI thread** | Generating a 100-page document can exceed 500ms — would block rendering. `KrillaSurface` is instantiated in the Logic thread; the UI thread receives `ProseEvent::ExportReady` and triggers the download. |
| **`comrak` instead of `pulldown-cmark`** | `pulldown-cmark` is lighter and actively maintained. `comrak` is complete but ~2x heavier in binary for zero functional gain on our use case. |

---

## 7. INTEGRATION PLAN (ROADMAP)

**Phase 0 — IR & Native Format (Week 1)**
- [ ] Complete definition of `ProseIr` and `ConversionReport`.
- [ ] `DocToIr`: `LoroDoc` traversal → `ProseIr` (without images).
- [ ] `IrToCommands`: `ProseIr` → `Vec<ProseCommand>` (batch import into the controller).
- [ ] `export_ambre_plaintext` / `import_ambre_plaintext` (magic header, version, doc_type).
- [ ] Round-trip unit tests `DocToIr` + `IrToCommands`.

**Phase 1 — Markdown (Week 1, parallel to Phase 0)**
- [ ] `export_markdown`: `ProseIr` → Markdown GFM via `pulldown-cmark-to-cmark`.
- [ ] `import_markdown`: Markdown → `ProseIr` via `pulldown-cmark` (all GFM options).
- [ ] Round-trip unit tests + lost features warnings.
- [ ] Criterion benchmarks export/import 100 pages.

**Phase 2 — HTML (Week 2)**
- [ ] `export_html`: `ProseIr` → semantic HTML5, optional inline CSS.
- [ ] `import_html`: HTML5 → `ProseIr` via `html5ever` + `markup5ever_rcdom`.
- [ ] Google Docs HTML fixtures, clipboard import tests.
- [ ] Integration into the UI thread paste pipeline (via `ProseCommand::ImportDocument { format: Html }`).

**Phase 3 — DOCX (Week 2-3)**
- [ ] `export_docx`: `ProseIr` → OOXML via `docx-rs` (paragraphs, headings, lists, tables).
- [ ] `import_docx`: OOXML → `ProseIr` (same + comments + track changes).
- [ ] `reference.docx` fixture, structural tests.
- [ ] Benchmarks import/export 100 pages.

**Phase 4 — PDF (Week 3-4)**
- [ ] `export_pdf`: `ProseIr` + `PageLayout` → PDF via Krilla + Parley.
- [ ] TOC hyperlinks + cross references.
- [ ] XMP metadata.
- [ ] Tests: parseable PDF, correct pages, valid links.
- [ ] Benchmarks export 100 pages.

**Phase 5 — Images & Lazy Loading (post-MVP)**
- [ ] S3 image resolution via `SyncCommand::RequestPresignedUrl`.
- [ ] Lazy loading (WASM bundle splitting) of interop crates in the Logic thread if WASM budget exceeded.
- [ ] PDF/UA (accessibility).
