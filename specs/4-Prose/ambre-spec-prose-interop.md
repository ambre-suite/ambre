# AMBRE -- SPEC: PROSE / INTEROP
**Type:** Vertical (Feature/Widget)
**Status:** Draft

> Multi-format import and export pipeline for Prose. Bidirectional conversion between the
> native CRDT structure (`LoroDoc` Prose) and external formats: docx, Markdown, PDF, HTML,
> and native `.ambre` format. All conversions run client-side in the Logic thread
> (WASM) -- no plaintext document transits through the server.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope:**

This component handles:
- Exporting the Prose `LoroDoc` to docx, Markdown, PDF, HTML.
- Importing docx, Markdown, HTML into an initialized Prose `LoroDoc`.
- Serialization/deserialization of the native `.ambre` format (Loro snapshot + postcard + zstd + XChaCha20).
- Managing fidelity losses (best-effort) during conversions and reporting them to the user.

This component delegates:
- Actual CRDT mutations -> `ProseState::handle_command()` via `ProseCommand`.
- Native format compression + encryption/decryption -> `ambre-sync`.
- Live wgpu + Parley/Swash rendering (display list, virtual scroll) -> `prose-ui`.
- Binary asset (image) fetch/decrypt -> `ambre-sync` (media flow: `RestCommand::UploadMedia`/`DownloadMedia` -> `RestEvent::MediaUploaded`/`MediaDownloaded`, addressed by `content_hash`). The Logic thread never touches the network or S3.

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
| `ambre-draw` | `DrawSurface` trait -- `KrillaSurface` implemented here for PDF export | Export |
| `postcard` / `serde` | Serialization of conversion reports (`ConversionReport`) and fixtures | Both |

No dependency on `ambre-sync`, `ambre-crypto`, or `ruzstd` -- the native format's **compression
(zstd) and encryption (XChaCha20-Poly1305)** are applied by `ambre-sync` downstream (sole
compression/crypto consumer, per `ambre-spec-architecture.md §1`), not in `ambre-prose/interop`.
The Logic thread carries **zero compression, zero crypto, zero network**.

**Invariants:**

- Any conversion is **stateless server-side**: the entire pipeline runs in the Logic thread in WASM memory. No chunk is sent to the Blind Relay during conversion.
- An import exclusively produces typed `ProseCommand`s -- it never mutates the `LoroDoc` directly. This guarantees all mutations pass through the controller invariants.
- Each export is **idempotent**: exporting the same document twice produces byte-identical files (up to metadata timestamp).
- Any fidelity loss is **explicitly reported** via `ConversionReport::warnings` -- never silent.
  This is a **hard gate**, not best-effort: every reader and writer either (a) represents an
  input feature in `ProseIr` and round-trips it, or (b) pushes a `ConversionWarning` at the
  exact drop site. A feature that is neither represented nor warned about is a spec violation.
  Wildcard/`_` match arms over format elements MUST warn before discarding (see docx
  `import_docx`), and any writer emitting an approximation (nearest color, `---` for both
  `PageBreak` and `HRule`, Markdown dropping image geometry) MUST record it.
- Images are stored in the CRDT as references (`content_hash`) and resolved at export from the Logic thread's `media_cache`, populated by `ambre-sync` via `RestCommand::DownloadMedia` -> `RestEvent::MediaDownloaded`. An export whose images are not yet cached (offline, or download pending) produces a document with placeholder images, reported in `ConversionReport`.

---

## 2. TECHNICAL DESIGN & STATE

### A. Pipeline Architecture

The conversion pipeline is symmetric: a **Reader** produces an intermediate representation
(`ProseIr`), a **Writer** consumes it to produce the target format.

```
Import:  Bytes (docx/md/html)
           -> Reader (format-specific)
           -> ProseIr
           -> IrToCommands
           -> Vec<ProseCommand>
           -> ProseState::handle_command() [by batch]

Export:  LoroDoc (snapshot read from ProseState)
           -> DocToIr
           -> ProseIr
           -> Writer (format-specific)
           -> Bytes (docx/md/pdf/html)
```

#### Intermediate Representation `ProseIr`

`ProseIr` is a format-independent linear AST, aligned with the block model of
`prose-engine`. It is not serialized -- it is purely in memory during conversion.

```rust
// ambre-prose/src/interop/ir.rs

#[derive(Debug, Clone)]
pub struct ProseIr {
    pub meta:     IrMeta,
    pub blocks:   Vec<IrBlock>,
    pub styles:   Vec<IrStyle>,     // custom named styles
    pub comments: Vec<IrComment>,   // comment threads (bodies + replies), out-of-band from blocks
}

#[derive(Debug, Clone)]
pub struct IrMeta {
    pub title:  Option<String>,
    pub lang:   Option<String>,   // BCP 47
    pub page:   Option<IrPage>,
    // Headers/footers are MVP features (features §G). They are carried in the IR so no
    // export drops them silently. `first` variants are set only when different_first_page.
    pub header:       Option<IrHeaderFooter>,
    pub header_first: Option<IrHeaderFooter>,
    pub footer:       Option<IrHeaderFooter>,
    pub footer_first: Option<IrHeaderFooter>,
}

/// A header or footer: a single paragraph of runs plus alignment. Dynamic fields
/// (page number, page count, date, title) are carried inline as `IrRun::field`.
#[derive(Debug, Clone)]
pub struct IrHeaderFooter {
    pub align: IrTextAlign,
    pub runs:  Vec<IrRun>,
}

/// Dynamic header/footer field. Maps to features `HeaderField` and marks f:pn/f:pc/f:dt/f:ti,
/// and to docx `<w:fldSimple>` / field-char runs (PAGE, NUMPAGES, DATE, TITLE).
#[derive(Debug, Clone, Copy)]
pub enum IrField { PageNumber, PageCount, Date, Title }

#[derive(Debug, Clone, Copy)]
pub enum IrOrientation { Portrait, Landscape }

#[derive(Debug, Clone)]
pub struct IrPage {
    pub width_mm:         f32,
    pub height_mm:        f32,
    pub margin_top_mm:    f32,
    pub margin_bottom_mm: f32,
    pub margin_left_mm:   f32,
    pub margin_right_mm:  f32,
    pub orientation:      IrOrientation,   // round-trips docx <w:pgSz w:orient>
}

/// A comment thread. `anchor_*` locate the commented range; the live "cm" mark on the runs
/// carries `comment_id` for round-trip. Bodies/replies/author/date/resolved (features §H)
/// have an IR home so docx `comments.xml` and HTML `data-comment-id` export real content.
#[derive(Debug, Clone)]
pub struct IrComment {
    pub id:         Uuid,
    pub resolved:   bool,
    pub created_at: u64,           // Unix ms, mirrors features "created_at"
    pub replies:    Vec<IrReply>,  // first entry is the top-level comment body
}

#[derive(Debug, Clone)]
pub struct IrReply {
    pub id:         Uuid,
    pub author:     String,
    pub created_at: u64,           // Unix ms
    pub text:       String,
}

#[derive(Debug, Clone)]
pub enum IrBlock {
    Paragraph  { style_id: Option<String>, para_style: IrParaStyle, runs: Vec<IrRun> },
    Heading    { level: u8, numbered: bool, runs: Vec<IrRun> },
    // Depth is NOT stored: it is derived from `children` nesting, matching logic §C where
    // `ListItem` depth is derived from the MovableTree ancestor chain and never persisted
    // (avoids the double-encoding that would drift from the tree).
    ListItem   { style: IrListStyle, runs: Vec<IrRun>, children: Vec<IrBlock> },
    Table      { col_widths_pt: Vec<f32>, style: IrTableStyle, rows: Vec<IrTableRow> },
    CodeBlock  { lang: Option<String>, text: String },
    Image      { content_hash: Option<String>, src_url: Option<String>, alt: String,
                 width_px: Option<u32>, height_px: Option<u32>,
                 align: Option<IrImageAlign>, caption: Option<String> },
    PageBreak,
    HRule,
    Toc        { max_level: u8, numbered: bool },
    Footnote   { id: Uuid, runs: Vec<IrRun> },
}

/// Table-level styling (features §E). `header_row` promotes the first row to `<w:tblHeader>`
/// / `<thead>` / `<th>`; `banded_rows` applies alternating row shading.
#[derive(Debug, Clone)]
pub struct IrTableStyle {
    pub header_row:  bool,
    pub banded_rows: bool,
}

#[derive(Debug, Clone)]
pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

#[derive(Debug, Clone, Copy)]
pub enum IrCellVAlign { Top, Center, Bottom }

#[derive(Debug, Clone)]
pub struct IrTableCell {
    pub rowspan:      u32,
    pub colspan:      u32,
    // Cell styling (features §E). Colors are `0x00RRGGBB` (see `IrRun::color`).
    pub bg:           Option<u32>,   // cell shading -> <w:shd> / background-color
    pub borders:      bool,          // draw cell borders -> <w:tcBorders> / border
    pub border_color: Option<u32>,   // border color, defaults to text color when None
    pub padding_pt:   Option<f32>,   // cell margins -> <w:tcMar> / padding
    pub valign:       IrCellVAlign,  // -> <w:vAlign> / vertical-align
    // Paragraph, Heading, ListItem, and Image are allowed. A nested Table or CodeBlock is
    // not representable (logic §C TableCell rule) and is reported `FeatureNotSupported`
    // during import -- never dropped silently. An `Image` cell round-trips (P-112).
    pub blocks:       Vec<IrBlock>,
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
    /// Foreground color as `0x00RRGGBB` (top byte 0, no alpha: text runs are always
    /// fully opaque). Emitted as `#rrggbb` in docx/html/pdf.
    pub color:         Option<u32>,
    /// Highlight/background color as `0x00RRGGBB` (top byte 0, no alpha). Same byte order
    /// as `color`. Alpha is not modeled -- a background is either present or absent.
    pub background:    Option<u32>,
    pub font_size:     Option<f32>,
    pub font_family:   Option<String>,
    pub link:          Option<String>,
    pub comment_id:    Option<Uuid>,
    // Suggestion (track-change) marks. `suggest_ins`/`suggest_del` are the suggestion ids
    // (logic `MarkType si/sd`). `suggest_author`/`suggest_date` carry the docx
    // `<w:ins/w:del w:author w:date>` metadata so it round-trips both ways (features §I).
    pub suggest_ins:    Option<Uuid>,
    pub suggest_del:    Option<Uuid>,
    pub suggest_author: Option<String>,
    pub suggest_date:   Option<u64>,       // Unix ms
    pub footnote_ref:   Option<Uuid>,
    /// Dynamic header/footer field carried inline (features §G). Set only inside
    /// `IrHeaderFooter::runs`; `None` in body runs.
    pub field:          Option<IrField>,
    /// Cross reference: `(target_block_id, kind)`. `target_block_id` is the IR-level id of
    /// the referenced block, mapping to logic `InsertCrossRef { target_block_id, kind }`
    /// (logic §C). Rendered as a docx/pdf field and an HTML anchor.
    pub cross_ref:      Option<(Uuid, IrCrossRefKind)>,
}

#[derive(Debug, Clone)]
pub struct IrParaStyle {
    pub align:          Option<IrTextAlign>,
    pub indent_first:   Option<f32>,
    pub indent_left:    Option<f32>,
    pub indent_right:   Option<f32>,   // logic ParagraphStyle.indent_right (§C)
    pub space_before:   Option<f32>,
    pub space_after:    Option<f32>,
    /// Line spacing as a **multiple** (e.g. `1.15`), matching logic §C which stores only a
    /// multiple. Docx `<w:spacing w:lineRule="exact"|"atLeast">` (fixed-point spacing) is
    /// NOT representable and is reported `FormattingLost` on import -- never dropped silently.
    pub line_height:    Option<f32>,
    pub keep_together:  bool,          // logic ParagraphStyle.keep_together (§C)
    pub keep_with_next: bool,          // logic ParagraphStyle.keep_with_next (§C)
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
    ImageNotResolved { content_hash: String },
    FeatureNotSupported { feature: String, format: ConversionFormat },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversionError {
    ParseFailed     { detail: String },
    RenderFailed    { detail: String },
}
```

**`InteropError` vs `ConversionError` -- fatal vs non-fatal (disambiguation).**
`ConversionError` is a **non-fatal** entry inside a `ConversionReport`: the conversion still
produced a usable document (e.g. a missing image, a paragraph that failed to render but was
skipped). `InteropError` is the **fatal** `Result::Err` returned by an import/export entry
point when no document can be produced at all (corrupt ZIP, unreadable header). The two carry
homonymous `ParseFailed`/`RenderFailed` variants intentionally: a `ParseFailed` *inside*
`ConversionError` is a recovered local parse skip, while `InteropError::ParseFailed` aborts
the whole call.

```rust
// ambre-prose/src/interop/error.rs

#[derive(Debug, Clone)]
pub enum InteropError {
    // Native .ambre header failures (see §B).
    TruncatedHeader,
    InvalidMagic,
    UnsupportedVersion { found: u8 },   // recovery: see §B migration path
    WrongDocType,
    // Whole-document import/export failures.
    ParseFailed(String),    // format bytes could not be parsed at all
    RenderFailed(String),   // target writer could not emit bytes at all
}
```

---

### B. Native `.ambre` Format

The native format is the reference persistence and sharing format. It involves no data
conversion -- it is a packaged Loro binary snapshot.

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

Compression and encryption are applied by `ambre-sync` -- `ambre-prose/interop`
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

**Version / doc-type recovery path (`UnsupportedVersion`, `WrongDocType`).**
Both are surfaced to the UI as typed, non-destructive failures -- the original bytes are
never mutated:

- `UnsupportedVersion { found }` where `found < AMBRE_VERSION`: a forward migration step is
  applied (`migrate_ambre(found, bytes) -> Result<Vec<u8>, InteropError>`, added per version
  bump) before retrying import. `found > AMBRE_VERSION` (file written by a newer client) is
  unrecoverable: the UI prompts the user to update, the file is left untouched.
- `WrongDocType`: the file is a valid `.ambre` but for another app (e.g. a Toile board). The
  caller re-dispatches to the matching app's importer using `data[5]` (`DocType`); Prose
  never coerces it.

---

### C. DOCX Export / Import

#### Prose Block -> OOXML Mapping

| `IrBlock` | OOXML Element | Notes |
|-----------|--------------|-------|
| `Paragraph` | `<w:p>` with `<w:pPr><w:pStyle>` | style_id mapped to Word style name |
| `Heading { level }` | `<w:p><w:pStyle w:val="Heading{level}">` | Built-in Word styles |
| `ListItem` | `<w:p><w:pStyle w:val="ListParagraph">` + `<w:numPr>` | `numId` managed by `docx-rs` |
| `Table` | `<w:tbl>` + `<w:tr>` + `<w:tc>` | `col_widths_pt` (points) -> `<w:tblGrid>` (twips); `IrTableStyle` -> `<w:tblHeader>` + banded shading; cell `bg`/`borders`/`border_color`/`padding_pt`/`valign` -> `<w:shd>`/`<w:tcBorders>`/`<w:tcMar>`/`<w:vAlign>` |
| `CodeBlock` | `<w:p><w:pStyle w:val="Code">` | Custom style injected if absent |
| `Image` | `<w:drawing>` + relationship `image/png` | Bytes read from the Logic `media_cache` (`content_hash`), populated by `ambre-sync` |
| `PageBreak` | `<w:p><w:r><w:br w:type="page">` | |
| `HRule` | `<w:p><w:pBdr><w:bottom>` | |
| `Toc` | `<w:sdt>` standard Word TOC field | `TOC \o "1-3"` field instruction |
| `Footnote` | `<w:footnote>` in `/word/footnotes.xml` | |

#### Prose Inline Mark -> `<w:rPr>` Mapping

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
| `suggest_ins` | `<w:ins w:id="{n}" w:author="{suggest_author}" w:date="{suggest_date}">` |
| `suggest_del` | `<w:del w:id="{n}" w:author="{suggest_author}" w:date="{suggest_date}">` |
| `comment_id` | `<w:commentRangeStart>` / `<w:commentRangeEnd>` + `/word/comments.xml` (bodies/replies/author/resolved/created_at from `IrComment`) |
| `field` (in header/footer) | `<w:fldSimple>` / field-char run: `IrField::PageNumber`->`PAGE`, `PageCount`->`NUMPAGES`, `Date`->`DATE`, `Title`->`TITLE` |

#### Docx Fidelity Losses (reported in `ConversionReport`)

| Prose Feature | Docx Behavior |
|--------------|---------------|
| Unmappable Prose custom styles | Fallback to "Normal", `StyleNotMapped` warning |
| Non-standard `background` color | Approximated to nearest Word color (`<w:highlight>`), warning |
| Automatic section numbering | Not exported (Word manages its own numbering via fields), warning |
| Dynamic cross references (`CrossRefKind::PageNumber`) | Exported as Word `{ PAGE }` fields -- updated on open in Word |
| `BlockType::Toc` | Exported as standard Word TOC field |
| Docx `<w:spacing w:lineRule="exact"/"atLeast">` (import) | Not representable (IR stores a spacing multiple only, logic §C) -- degraded to nearest multiple, `FormattingLost` warning |
| Headers / footers | Round-trip via `/word/header{n}.xml` + `/word/footer{n}.xml` referenced from `<w:sectPr>` (`IrMeta.header/footer`, `first` variants under `w:titlePg`) |
| Page orientation | Round-trips via `<w:pgSz w:orient>` <-> `IrPage.orientation` |
| Docx image content-type EMF/WMF/TIFF | Not decodable in WASM -- skipped with a `FeatureNotSupported` warning (png/jpeg import natively) |
| Docx checkbox content control / `PfMark` numbering | Mapped to `IrListStyle::Todo` when recognizable, else `FeatureNotSupported` warning |

#### DOCX Import -> `ProseIr`

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
            BodyChild::SectionProps(sp) => import_section_props(sp, &mut ir.meta, &mut report),
            // A structured-document tag at body level is almost always a TOC field. Emit a
            // Toc block if it is a TOC sdt, otherwise flatten its content and warn (P-108).
            BodyChild::Sdt(sdt)      => parse_body_sdt(sdt, &mut ir, &mut report)?,
            // Any other body child (custom XML, structured markup we do not model) is dropped
            // -- but never silently: loss is always reported (P-109).
            other => report.warnings.push(ConversionWarning::FeatureNotSupported {
                feature: format!("docx body element {:?}", core::mem::discriminant(other)),
                format:  ConversionFormat::Docx,
            }),
        }
    }
    // margins/orientation come from the first `<w:sectPr>` (import_section_props above);
    // headers/footers from the parts it references.
    import_headers_footers(&doc, &mut ir.meta, &mut report)?;   // header{n}.xml / footer{n}.xml
    // Footnotes, comments, suggestions imported from dedicated parts
    import_footnotes(&doc, &mut ir, &mut report)?;
    import_comments(&doc, &mut ir, &mut report)?;               // bodies/replies/author/resolved
    import_track_changes(&doc, &mut ir, &mut report)?;          // fills suggest_author/suggest_date

    Ok((ir, report))
}

/// Detects the raster content-type from magic bytes; unknown/vector types are not decodable
/// in WASM and are reported rather than embedded (P-111).
fn image_content_type(bytes: &[u8], report: &mut ConversionReport) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..]        => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..]              => Some("image/jpeg"),
        _ => {   // EMF/WMF/TIFF and friends -- keep the alt text, warn, embed nothing.
            report.warnings.push(ConversionWarning::FeatureNotSupported {
                feature: "docx image (EMF/WMF/TIFF or unknown content-type)".into(),
                format:  ConversionFormat::Docx,
            });
            None
        }
    }
}
```

---

### D. Markdown Export / Import

Markdown (CommonMark + GFM) is a format with significant losses -- it does not support
paragraph styles, pagination, comments, or track changes. Export is **best-effort**;
import is faithful for supported features.

#### Prose -> Markdown GFM Export Mapping

| `IrBlock` | Markdown |
|-----------|----------|
| `Paragraph` | Paragraph separated by blank line |
| `Heading { level }` | `#{level} text` |
| `ListItem { Bullet }` | `- text` (nested with 2 spaces) |
| `ListItem { Ordered }` | `1. text` |
| `ListItem { Todo(checked) }` | `- [x] text` / `- [ ] text` (GFM) |
| `Table` | GFM table `\| col \| col \|` |
| `CodeBlock { lang }` | ` ```lang ``` ` |
| `Image` | `![alt](url)` -- data URI from the Logic `media_cache`, or `src_url` if external. `width_px`/`height_px`/`align`/`caption` have no CommonMark/GFM syntax -> `FormattingLost` warning when any is set (P-115) |
| `PageBreak` | `---` (horizontal rule, approximation) -- indistinguishable from `HRule`; emits a `FeatureNotSupported` warning so the semantic loss is not silent (P-116) |
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
`peniko`, `parley` (types only -- no direct dependency on the final rendering engine).

Prose does **not** define its own surface types. Both `DrawSurface` implementations are
owned by Foundation and reused as-is:

- **Screen:** `ambre_canvas::render::WgpuSurface` (UI thread) -- `ambre-spec-canvas.md §D`.
- **PDF:** `ambre_pdf::KrillaSurface` (Logic thread, lazy-loaded), living in the shared
  **`ambre-pdf`** crate (deps `ambre-draw` + `krilla`) -- *not* in an app crate, since PDF
  export only translates `DrawSurface` primitives to krilla and is app-agnostic
  (`ambre-spec-canvas.md §D`). Its signature is Foundation's:
  `KrillaSurface<'a, 'b> { surface: &'a mut krilla::surface::Surface<'b> }` -- the surface
  carries **no** `FontContext`; fonts are threaded to the render layer separately, exactly
  as `paint_content(.., fc: &mut FontContext, ..)` does on screen.

```rust
// ambre-prose/src/interop/pdf.rs
use ambre_pdf::KrillaSurface;   // Foundation-owned, not redefined here
```

**What is actually shared, and what is not.** The reuse is at the **primitive** level: the
`DrawSurface` trait methods (`draw_glyphs`, `fill_rect`, `draw_image`, path/clip/transform)
and the per-run/per-block draw helpers are identical whether the target is the screen or a
PDF. The **top-level entry point is not** a single shared function: the screen renderer
draws from live `ProseUiState` one visible block at a time (`render_block_on`, `prose-ui §D`)
for partial repaint + glyph caching, while the export renderer draws from the immutable
`ProseIr` + `PageLayout` a whole page at a time (`render_page_on`, below). Their input models
differ by design (live editable state vs frozen export IR), so they are distinct callers
over the same `DrawSurface` primitives -- there is no claim of one function serving both.

**Execution in the Logic thread:** Parley is already available there for pagination.
The PDF is produced in memory (`Vec<u8>`) and transmitted to the UI thread via
`ProseEvent::ExportReady { format: Pdf, data: Vec<u8>, mime: "application/pdf", .. }` for download.
Generating a 100-page document can exceed 500ms -- blocking the UI thread during that
time would freeze the interface.

```rust
// ambre-prose/src/interop/pdf.rs

pub fn export_pdf(
    ir: &ProseIr,
    layout: &PageLayout,
    fonts: &mut FontContext,   // shaping needs &mut; threaded to render_page_on, not to the surface
    options: &ExportOptions,   // plumbed through: no hardcoded language, TOC links gated (P-24/P-45)
) -> Result<(Vec<u8>, ConversionReport), InteropError> {
    let mut report = ConversionReport::new(ConversionFormat::Pdf);
    let mut doc = krilla::Document::new();

    doc.set_title(ir.meta.title.as_deref().unwrap_or(""));
    // Language precedence: explicit option -> document language -> unset. Never a fixed locale.
    if let Some(lang) = options.pdf_language.as_deref().or(ir.meta.lang.as_deref()) {
        doc.set_language(lang);
    }

    // A4 (210x297 mm) is the fallback ONLY when the document carries no page setup; a document
    // with an `IrPage` (e.g. Letter, Landscape) is honored exactly.
    let page_size = krilla::PageSize {
        width:  ir.meta.page.as_ref().map_or(210.0, |p| p.width_mm) * MM_TO_PT,
        height: ir.meta.page.as_ref().map_or(297.0, |p| p.height_mm) * MM_TO_PT,
    };

    for page_idx in 0..layout.page_count {
        let mut page = doc.add_page(page_size);
        let mut krilla_surface = page.surface();
        let mut surface = KrillaSurface { surface: &mut krilla_surface };

        // Shared DrawSurface primitives -- fonts threaded separately (surface holds none).
        render_page_on(&mut surface, fonts, page_idx, ir, layout, &mut report);

        krilla_surface.finish();
        page.finish();
    }

    // TOC hyperlinks and cross references -- gated on the caller's option (P-24).
    if options.pdf_include_toc_links {
        add_toc_links(&mut doc, ir, layout);
    }

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

HTML has no page model, so `IrMeta.header`/`footer` and their inline `IrField`s have no target
element. They are not dropped silently: export emits a `FeatureNotSupported { feature:
"header/footer", format: Html }` warning when either is present. `IrTableStyle::banded_rows`
maps to a `<tr class="banded">` hook; `header_row` promotes the first row to `<thead>`/`<th>`.

```rust
// ambre-prose/src/interop/html.rs

// Produces a full <html><head><body>. Inline styles when inline_css = true (for emails),
// otherwise a <style> block in <head>. See the IrBlock -> HTML mapping table below.
pub fn export_html(ir: &ProseIr, inline_css: bool) -> (String, ConversionReport);
```

`IrBlock` -> Semantic HTML Mapping:

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

Inline Marks -> HTML Attributes:

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
| `color` / `background` | `style="color: #rrggbb"` / `style="background: #rrggbb"` (both `0x00RRGGBB`) |
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

HTML import is also used for **clipboard paste** (`text/html` format) -- the same pipeline
is reused, ensuring that pasting from Google Docs or Word Online produces a consistent
result.

#### Google Docs

There is no Google Docs API allowing direct content access from WASM without OAuth
authentication. Interoperability with Google Docs goes through two paths:

1. **Google Export -> DOCX:** the user exports their Google Docs document as `.docx`
   (File -> Download -> Microsoft Word), then imports into Prose via the docx pipeline.
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
// All exporters converge on `ExportReady`. The text writers (`export_markdown`,
// `export_html`) return `String`; the IPC layer encodes them UTF-8 into `data: Vec<u8>`, so
// the UI has a single download path for every format. `mime` disambiguates the byte payload
// for the browser download (P-46).
ExportReady     { format: ExportFormat, data: Vec<u8>, mime: &'static str, report: ConversionReport },
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
    pub pdf_include_toc_links: bool,   // gates add_toc_links in export_pdf
    /// PDF/XMP document language (BCP 47). `None` -> fall back to `IrMeta.lang`, then to the
    /// document with no language set. Never hardcoded to a locale (P-24/P-45).
    pub pdf_language:          Option<String>,
    pub markdown_flavor:       MarkdownFlavor,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum MarkdownFlavor { #[default] Gfm, CommonMark }
```

**`ExportReady.mime` per format:** `Ambre` -> `application/octet-stream`,
`Docx` -> `application/vnd.openxmlformats-officedocument.wordprocessingml.document`,
`Markdown` -> `text/markdown; charset=utf-8`, `Pdf` -> `application/pdf`,
`Html` -> `text/html; charset=utf-8`.

---

## 3. SECURITY & CONFIDENTIALITY (ZERO-KNOWLEDGE)

**Main guarantee:** all conversions run in the Logic thread, in WASM memory, without any
transfer to the server during conversion.

| Flow | Data | Risk | Mitigation |
|------|---------|--------|------------|
| Import docx/md/html | `Vec<u8>` received from UI thread via `UiEnvelope` | The imported file may contain macros or scripts | `docx-rs` does not execute macros. `html5ever` sanitizes HTML (no JS execution). |
| PDF export with images | Image bytes read from the Logic `media_cache` | Presigned-URL fetch + decrypt happen entirely in `ambre-sync` | The Logic thread never sees the presigned URL nor the network: it only receives already-decrypted bytes via `RestEvent::MediaDownloaded`. Presigned URLs (5-minute TTL) are handled inside Sync. |
| Native `.ambre` format | Plaintext Loro snapshot in the Logic thread | Isolated WASM memory of the Main instance (Logic thread) | Logic hands the plaintext snapshot to Sync as a typed `SyncCommand` payload over `flume`; the Main thread relays it to the separate Sync worker (`postMessage` + `postcard`), which alone compresses (zstd) and encrypts (XChaCha20). Plaintext never reaches the network. |
| Import HTML clipboard | HTML from `navigator.clipboard.read()` | Malicious HTML (injection) | `html5ever` sanitizes, `markup5ever_rcdom` does not preserve `on*` attributes or `<script>`. |

**Image resolution on export:**

Images are resolved ahead of export through the Foundation media flow -- the Logic thread
issues **no network call** and holds **no crypto** (`ambre-spec-architecture.md §1`,
`§2.D RestCommand`). All fetch + decrypt work happens in the Sync worker:

```
UI / Logic thread                          Sync worker (ambre-sync)
  RestCommand::DownloadMedia { document_id, content_hash }
      ──(LogicCommand::Rest -> SyncCommand::Rest, relayed by Main)──>
                                             presigned S3 GET -> decrypt
      <──(RestEvent::MediaDownloaded { document_id, content_hash, data })──
  ProseState::on_media_downloaded(content_hash, data)
      -> media_cache.insert(content_hash, data)

  ProseCommand::ExportDocument { format: Pdf }
      -> for each IrBlock::Image { content_hash }:
          -> bytes read from media_cache (already decrypted, no I/O)
          -> image bytes included in the PDF
```

In V1 MVP, an export whose images are not yet in `media_cache` produces a placeholder and
an `ImageNotResolved` warning; eager pre-fetch of all document media before export is
post-MVP.

---

## 4. QUALITY & TESTABILITY

**Unit tests:**

| Case | Method |
|-----|---------|
| Round-trip `ProseIr` -> Markdown -> `ProseIr` | Assertions on preserved fields (text, heading level, list style) |
| Round-trip `ProseIr` -> HTML -> `ProseIr` | Same + inline mark verification |
| Export docx: valid OOXML structure | `docx-rs` parses the produced file without error |
| Import docx -> `ProseIr`: headings, lists, tables, comments | Reference docx files in fixtures |
| Import docx track changes: `<w:ins>` / `<w:del>` -> `suggest_ins` / `suggest_del` | docx fixture with revisions |
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
| `DocToIr` (LoroDoc -> ProseIr) 100 pages | < 50 ms | Logic | MovableTree traversal |

**WASM size (interop crates):**

| Crate | Raw estimated | Gzip estimated |
|-------|-----------|-------------|
| `docx-rs` | ~300 KB | ~90 KB |
| `pulldown-cmark` + `pulldown-cmark-to-cmark` | ~80 KB | ~25 KB |
| `krilla` (in shared `ambre-pdf`, not `interop`) | ~400 KB | ~120 KB |
| `html5ever` + `markup5ever_rcdom` | ~200 KB | ~60 KB |
| `ambre-prose/interop` (logic) | ~40 KB | ~13 KB |
| **Total interop** | **~1.02 MB** | **~308 KB** |

> `krilla` is billed to the shared **`ambre-pdf`** crate (§E), not to `ambre-prose/interop`.
> It is counted once here because Prose is the first PDF-exporting consumer; a second consumer
> (e.g. Toile) reuses the same `ambre-pdf` code and adds no krilla weight (P-44).

> ⚠️ Interop crates represent a significant binary cost. If the total WASM budget is
> exceeded, use **lazy loading** (WASM bundle splitting): `docx-rs`, `krilla`,
> `html5ever`, and `pulldown-cmark` are excluded from the startup binary and dynamically
> loaded in the Logic thread on the first import/export operation. No additional worker --
> the two-instance model (Main = UI + Logic, separate Sync worker) is unchanged. During
> import/export, the Logic thread is intentionally blocked (waiting screen on the UI side);
> the Logic thread has **no JS event loop**, so Sync -> Logic events queue in the bounded
> `flume` relay drained by the Main thread and are delivered once the Logic thread unblocks
> -- none are lost (backpressure via `CHANNEL_CAPACITY`, see `ambre-spec-architecture.md §2.B`).

---

## 6. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|-------------|-------------|
| **Server-side conversion** | Violates Zero-Knowledge principle: the plaintext document would leave for the server. All conversion happens in the Logic thread. |
| **LibreOffice WASM for docx** | Binary > 50 MB in WASM. Incompatible with < 2 MB gzip budget. |
| **`mammoth.js` for docx import** | JavaScript, outside the Rust-first vision. |
| **`pandoc` WASM** | Binary > 20 MB. Same. |
| **IR = `ProseCommand[]` directly** | An intermediate `ProseIr` IR allows validating, transforming, and reporting warnings before application. A direct command stream does not allow rollback on partial error. |
| **Direct Google Docs API** | Requires OAuth + network call to Google servers from WASM -- metadata leak (when and which document is opened). Docx + clipboard path retained. |
| **`wasm-bindgen` + `fetch` for PDF** | PDF export is 100% local (Krilla + Parley). No external service. |
| **`printpdf` for PDF export** | Does not support Parley for typographic layout -- text would be rendered as graphic primitives without proper ligatures or justification. Krilla is designed to integrate with Parley. |
| **Duplicating PDF/live rendering code** | Rendering functions parameterized on `DrawSurface` are shared between `WgpuSurface` (UI thread, live rendering) and `KrillaSurface` (Logic thread, PDF export). Duplicating rendering code for PDF would be a permanent source of divergence. |
| **PDF in the UI thread** | Generating a 100-page document can exceed 500ms -- would block rendering. `KrillaSurface` is instantiated in the Logic thread; the UI thread receives `ProseEvent::ExportReady` and triggers the download. |
| **`comrak` instead of `pulldown-cmark`** | `pulldown-cmark` is lighter and actively maintained. `comrak` is complete but ~2x heavier in binary for zero functional gain on our use case. |

---

## 7. INTEGRATION PLAN (ROADMAP)

**Phase 0 -- IR & Native Format (Week 1)**
- [ ] Complete definition of `ProseIr` and `ConversionReport`.
- [ ] `DocToIr`: `LoroDoc` traversal -> `ProseIr` (without images).
- [ ] `IrToCommands`: `ProseIr` -> `Vec<ProseCommand>` (batch import into the controller).
- [ ] `export_ambre_plaintext` / `import_ambre_plaintext` (magic header, version, doc_type).
- [ ] Round-trip unit tests `DocToIr` + `IrToCommands`.

**Phase 1 -- Markdown (Week 1, parallel to Phase 0)**
- [ ] `export_markdown`: `ProseIr` -> Markdown GFM via `pulldown-cmark-to-cmark`.
- [ ] `import_markdown`: Markdown -> `ProseIr` via `pulldown-cmark` (all GFM options).
- [ ] Round-trip unit tests + lost features warnings.
- [ ] Criterion benchmarks export/import 100 pages.

**Phase 2 -- HTML (Week 2)**
- [ ] `export_html`: `ProseIr` -> semantic HTML5, optional inline CSS.
- [ ] `import_html`: HTML5 -> `ProseIr` via `html5ever` + `markup5ever_rcdom`.
- [ ] Google Docs HTML fixtures, clipboard import tests.
- [ ] Integration into the UI thread paste pipeline (via `ProseCommand::ImportDocument { format: Html }`).

**Phase 3 -- DOCX (Week 2-3)**
- [ ] `export_docx`: `ProseIr` -> OOXML via `docx-rs` (paragraphs, headings, lists, tables).
- [ ] `import_docx`: OOXML -> `ProseIr` (same + comments + track changes).
- [ ] `reference.docx` fixture, structural tests.
- [ ] Benchmarks import/export 100 pages.

**Phase 4 -- PDF (Week 3-4)**
- [ ] `export_pdf`: `ProseIr` + `PageLayout` -> PDF via Krilla + Parley.
- [ ] TOC hyperlinks + cross references.
- [ ] XMP metadata.
- [ ] Tests: parseable PDF, correct pages, valid links.
- [ ] Benchmarks export 100 pages.

**Phase 5 -- Images & Lazy Loading (post-MVP)**
- [ ] Image resolution via the Foundation media flow (`RestCommand::DownloadMedia` -> `RestEvent::MediaDownloaded` -> `media_cache`); eager pre-fetch of document media before export.
- [ ] Lazy loading (WASM bundle splitting) of interop crates in the Logic thread if WASM budget exceeded.
- [ ] PDF/UA (accessibility).
