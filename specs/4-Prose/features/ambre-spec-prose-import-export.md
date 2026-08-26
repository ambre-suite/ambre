# AMBRE -- FEATURE: IMPORT AND EXPORT

**Type:** Vertical (feature)
**Implementation:** design-only

> A user opens a document that came from Word, from a web page or from a Markdown file, and hands
> the document back as a file someone without Ambre can read. Every conversion runs on the user's
> own machine.

The native `.ambre` file is the only path that keeps everything. Every other format drops something,
and what it drops is the subject of this document. The paginated PDF is
`features/ambre-spec-prose-pdf-export.md`, and the pixels a picture leaves behind are
`features/ambre-spec-prose-images.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-IO-001` | Every conversion MUST run in the Logic thread; no part of a document MUST reach the relay during a conversion. | this spec section 3; `ZK-001` | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-002` | An import MUST write through typed commands; it MUST NOT mutate the document directly. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-009` | Every format MUST pass through the intermediate form: a reader MUST NOT emit commands, and a writer MUST NOT read the document. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-003` | An export MUST be idempotent: the same document exported twice MUST yield the same bytes, up to the stamp the format writes. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-017` | An export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-011` | A failed import MUST leave the document untouched: a partial application MUST NOT be observable. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-010` | A malformed or hostile file MUST fail as a typed error; it MUST NOT panic. | this spec section 3; `ambre-spec-testing.md` | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5); the readers are the first `cargo-fuzz` targets | unimplemented |
| `PROSE-IO-004` | Every fidelity loss MUST raise a warning naming the construct that was dropped. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-005` | A reader MUST warn before it discards an element it does not recognise. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-006` | A writer that emits an approximation MUST record it in the report. | this spec section 3 | not built -- lands with the "DOCX interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-026` | A construct the block tree refuses MUST be lifted to the nearest place the tree allows, and the lift MUST be reported; it MUST NOT be dropped. | this spec section 3; `ambre-spec-prose-schema.md section 4` | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-021` | A conversion of a large document MUST report its progress; it MUST NOT hold the Logic thread for the whole file. | this spec section 3; `ambre-spec-budgets.md section 1` | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-020` | An export MUST carry the media type of its bytes, so one download path serves every format. | this spec section 3 | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-022` | An export MUST omit the comments and the suggestions the user did not ask for. | this spec section 4; `ZK-001` | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-012` | The native file MUST be the only lossless round trip, and it MUST reuse the shared container rather than restate it. | this spec section 3; `ambre-spec-docfile.md section 3` | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5), on top of `DOCFILE P0` (Epic `EF`) | unimplemented |
| `PROSE-IO-013` | A native file belonging to another application MUST be re-dispatched, never coerced into Prose. | this spec section 3; `ambre-spec-docfile.md section 3` | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-014` | A Word round trip MUST keep every structure the intermediate form represents, or warn for each one it drops. | this spec section 3 | not built -- lands with the "DOCX interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-024` | A Word import MUST NOT run a macro the file carries. | this spec section 3; `ZK-001` | not built -- lands with the "DOCX interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-018` | A Markdown export MUST report every construct the flavour has no syntax for, the page model included. | this spec section 3 | not built -- lands with the "Markdown interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-016` | An imported HTML document MUST be sanitised: no script MUST run, and no remote fetch MUST start. | this spec section 3; `ZK-001` | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-019` | An HTML export MUST report the header and the footer, which the format has no element for. | this spec section 3 | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-023` | An imported picture naming a remote address MUST NOT be fetched, and the picture MUST be reported as unresolved. | this spec section 3; `ZK-001` | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-025` | The reader an HTML import uses MUST be the reader a rich paste uses: they MUST NOT be two readers. | this spec section 3; `features/ambre-spec-prose-clipboard.md` | not built -- lands with the "HTML interop" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-027` | A user MUST be able to bring a file into the document. | this spec section 4 | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-028` | A user MUST be able to take the document out as a file, and the bytes MUST reach the browser download path. | this spec section 4 | not built -- lands with the "The native format" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** six rows of `ambre-spec-budgets.md section 1`: the Word import and export of a
  hundred pages, the Markdown export, the HTML clipboard import, the plaintext native export, and
  the document-to-intermediate-form walk the four writers share. The four reader crates are billed
  in `section 5`, each one lazily loaded. Every row of both reads **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. `PROSE-IO-001` keeps every
  conversion in the Logic thread, `PROSE-IO-016` and `PROSE-IO-024` stop an imported file executing
  anything, `PROSE-IO-023` stops an import becoming a fetch, and `PROSE-IO-022` stops an export
  carrying commentary the user did not ask to publish. A native file written to disk is plaintext
  at rest by decision, and `ambre-spec-docfile.md section 5` owns that decision.
* **Verification.** 25 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-IO-009` is the one a compiler can hold before a test exists, because a reader that emits a
  command and a writer that reads the document are both type errors once the two halves take the
  intermediate form and nothing else. `PROSE-IO-003` and `PROSE-IO-017` are the pair a host test
  reaches with no network and no file picker. `PROSE-IO-007` and `PROSE-IO-008` left two steps
  earlier for `features/ambre-spec-prose-images.md`: a picture is part of the document structure,
  not of the pipeline beside it.

---

## 2. WHAT EACH USER SEES

An import is an ordinary write, so it reaches the other peer like any edit; an export writes
nothing. No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a Word file | types elsewhere | the imported blocks and the typing | both, on both screens | none -- `PROSE-IO-002` |
| imports a file that fails to parse | watches | the document as before | an error on Alice's screen and no change on Bob's | none -- `PROSE-IO-011` |
| imports a file, then undoes | watches | the document as before the import | the import leaves in one step, on both screens | none -- `PROSE-IO-002` |
| exports to Markdown | types | the typing alone: an export writes nothing | Bob's typing; Alice's file holds the document she exported | none -- `PROSE-IO-017` |
| exports the same document twice | -- | nothing | the same bytes, twice | none -- `PROSE-IO-003` |
| saves a native file | opens it | the same document state | the same blocks, styles and comments | none -- `PROSE-IO-012` |
| imports a Word table holding a picture | watches | the picture beside the table, never inside it | the picture out of the cell, and a line in the report | none -- `PROSE-IO-026` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the block tree, the style register and the
annotation register the schema declares (`ambre-spec-prose-schema.md section 3`); the native export
serializes the whole document and adds nothing to it.

**The pipeline is symmetric.** A reader turns bytes into the intermediate form, and a writer turns
the intermediate form into bytes. Neither half knows the other, and neither touches the document
(`PROSE-IO-009`).

```text
Import:  bytes -> reader -> ProseIr -> command builder -> Vec<ProseCommand> -> the document
Export:  the document -> document walk -> ProseIr -> writer -> bytes
```

The native path is the one exception and it is deliberate: it carries a Loro snapshot under the
shared header, so it passes through no intermediate form and loses nothing (`PROSE-IO-012`).

**The intermediate form.** A linear tree, held in memory for the length of one conversion and never
serialized:

```rust
pub struct ProseIr {
    pub meta:     IrMeta,
    pub blocks:   Vec<IrBlock>,
    pub styles:   Vec<IrStyle>,
    /// Comment threads, out of band from the blocks: a run carries the thread id.
    pub comments: Vec<IrComment>,
}

pub struct IrMeta {
    pub title:        Option<String>,
    /// BCP 47.
    pub lang:         Option<String>,
    pub page:         Option<IrPage>,
    /// The `first` variants are set only when the document uses a distinct first page.
    pub header:       Option<IrHeaderFooter>,
    pub header_first: Option<IrHeaderFooter>,
    pub footer:       Option<IrHeaderFooter>,
    pub footer_first: Option<IrHeaderFooter>,
}

pub struct IrPage {
    pub width_mm:         f32,
    pub height_mm:        f32,
    pub margin_top_mm:    f32,
    pub margin_bottom_mm: f32,
    pub margin_left_mm:   f32,
    pub margin_right_mm:  f32,
    pub orientation:      IrOrientation,
}

pub enum IrOrientation { Portrait, Landscape }

/// One paragraph of runs plus its alignment. A dynamic field is a run, not a marker.
pub struct IrHeaderFooter {
    pub align: IrTextAlign,
    pub runs:  Vec<IrRun>,
}

/// The four dynamic fields, mirroring the marks `f:pn`, `f:pc`, `f:dt` and `f:ti`.
pub enum IrField { PageNumber, PageCount, Date, Title }

pub enum IrBlock {
    Paragraph { style_id: Option<String>, para_style: IrParaStyle, runs: Vec<IrRun> },
    Heading   { level: u8, numbered: bool, runs: Vec<IrRun> },
    /// Depth is derived from the nesting, never stored: the document derives it too.
    ListItem  { style: IrListStyle, runs: Vec<IrRun>, children: Vec<IrBlock> },
    Table     { col_widths_pt: Vec<f32>, style: IrTableStyle, rows: Vec<IrTableRow> },
    CodeBlock { lang: Option<String>, text: String },
    Image     { content_hash: Option<String>, src_url: Option<String>, alt: String,
                width_px: Option<u32>, height_px: Option<u32>,
                align: Option<IrImageAlign>, caption: Option<String> },
    PageBreak,
    HRule,
    Toc       { max_level: u8, numbered: bool },
    Footnote  { id: Uuid, runs: Vec<IrRun> },
}

pub struct IrTableStyle {
    /// Promotes the first row to a header row.
    pub header_row:  bool,
    pub banded_rows: bool,
}

pub struct IrTableRow {
    pub cells: Vec<IrTableCell>,
}

pub enum IrCellVAlign { Top, Center, Bottom }

pub struct IrTableCell {
    pub rowspan:      u32,
    pub colspan:      u32,
    /// Colours are `0x00RRGGBB`, and the writer renders them `#rrggbb`.
    pub bg:           Option<u32>,
    pub borders:      bool,
    pub border_color: Option<u32>,
    pub padding_pt:   Option<f32>,
    pub valign:       IrCellVAlign,
    /// A reader may put anything here; the command builder lifts what the tree refuses
    /// (`PROSE-IO-026`).
    pub blocks:       Vec<IrBlock>,
}

pub struct IrRun {
    pub text:          String,
    pub bold:          bool,
    pub italic:        bool,
    pub underline:     bool,
    pub strikethrough: bool,
    pub superscript:   bool,
    pub subscript:     bool,
    pub code:          bool,
    /// Foreground, `0x00RRGGBB`: a run is always opaque.
    pub color:         Option<u32>,
    pub background:    Option<u32>,
    pub font_size:     Option<f32>,
    pub font_family:   Option<String>,
    pub link:          Option<String>,
    pub comment_id:    Option<Uuid>,
    /// The suggested insertion and the suggested deletion, mirroring the marks `si` and `sd`.
    pub suggest_ins:    Option<Uuid>,
    pub suggest_del:    Option<Uuid>,
    pub suggest_author: Option<String>,
    /// Unix ms.
    pub suggest_date:   Option<u64>,
    pub footnote_ref:   Option<Uuid>,
    /// Set inside a header or a footer alone.
    pub field:          Option<IrField>,
    pub cross_ref:      Option<(Uuid, IrCrossRefKind)>,
}

pub struct IrParaStyle {
    pub align:          Option<IrTextAlign>,
    pub indent_first:   Option<f32>,
    pub indent_left:    Option<f32>,
    pub indent_right:   Option<f32>,
    pub space_before:   Option<f32>,
    pub space_after:    Option<f32>,
    /// The three modes the document stores (`features/ambre-spec-prose-inline-styles.md`).
    pub line_height:    Option<LineHeight>,
    pub keep_together:  bool,
    pub keep_with_next: bool,
}

/// The character defaults a named style carries: formatting alone, and none of the
/// content-bearing fields of a run.
pub struct IrRunStyle {
    pub bold:          Option<bool>,
    pub italic:        Option<bool>,
    pub underline:     Option<bool>,
    pub strikethrough: Option<bool>,
    pub superscript:   Option<bool>,
    pub subscript:     Option<bool>,
    pub font_family:   Option<String>,
    pub font_size:     Option<f32>,
    pub color:         Option<u32>,
    pub background:    Option<u32>,
}

pub struct IrStyle {
    pub id:         String,
    pub name:       String,
    pub para_style: IrParaStyle,
    pub run_style:  IrRunStyle,
}

/// A thread. The commented range is the `cm` mark on the runs, never a field here.
pub struct IrComment {
    pub id:         Uuid,
    pub resolved:   bool,
    /// Unix ms.
    pub created_at: u64,
    /// The opening body first, then the replies.
    pub replies:    Vec<IrReply>,
}

pub struct IrReply {
    pub id:         Uuid,
    pub author:     String,
    pub created_at: u64,
    pub text:       String,
}

pub enum IrTextAlign { Left, Center, Right, Justify }

pub enum IrImageAlign { Block, Left, Right, Center }

pub enum IrListStyle { Bullet, Ordered(IrOrderedStyle), Todo(bool) }

pub enum IrOrderedStyle { Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }

pub enum IrCrossRefKind { PageNumber, SectionNumber, CustomText(String) }
```

**The report, and the two error kinds.** A report is what the user reads after a conversion that
finished. An error is what a caller gets when no document could be produced at all:

```rust
pub struct ConversionReport {
    pub format:   ConversionFormat,
    pub warnings: Vec<ConversionWarning>,
    /// Local failures the conversion recovered from.
    pub errors:   Vec<ConversionError>,
}

pub enum ConversionFormat { Docx, Markdown, Pdf, Html, Ambre }

pub enum ConversionWarning {
    StyleNotMapped      { style_name: String, fallback: String },
    FormattingLost      { description: String },
    ImageNotResolved    { content_hash: String },
    ImageNotFetched     { src_url: String },
    BlockLifted         { from: &'static str, to: &'static str },
    FeatureNotSupported { feature: String, format: ConversionFormat },
}

/// A recovered local failure, recorded in the report.
pub enum ConversionError {
    ParseFailed  { detail: String },
    RenderFailed { detail: String },
}

/// The whole call failed and produced nothing.
pub enum InteropError {
    TruncatedHeader,
    InvalidMagic,
    UnsupportedVersion { found: u8 },
    WrongDocType,
    ParseFailed(String),
    RenderFailed(String),
}
```

The two `ParseFailed` variants are homonyms on purpose: one is a paragraph the reader skipped, the
other is a file it could not open.

**Steps, on an import:**

1. read the whole file into the intermediate form, and return a typed error carrying no command on
   any parse failure (`PROSE-IO-010`, `PROSE-IO-011`);
2. lift every block the tree refuses to the nearest place it allows, and record each lift
   (`PROSE-IO-026`);
3. build the commands, in tree order, parent before child (`PROSE-IO-002`);
4. apply them as one batch, so the whole file is one undo item;
5. hand the report to the view.

Step 2 is a separate pass on purpose: the tree's nesting rule is one predicate
(`ambre-spec-prose-schema.md section 4`), and a reader that applied it inline would need a copy of
it per format.

**Steps, on an export:** walk the document once into the intermediate form, then hand that form to
the writer. The walk is shared by all four writers, which is what makes it one budget row rather
than four, and what keeps a writer from reading the document (`PROSE-IO-009`).

**The commands and the events** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `ProseCommand::ImportDocument` | UI to Logic | the format and the bytes |
| `ProseCommand::ExportDocument` | UI to Logic | the format and the options |
| `ProseEvent::ImportCompleted` | Logic to UI | the report of what the file lost |
| `ProseEvent::ExportReady` | Logic to UI | the bytes, their media type and the report |
| `ProseEvent::ImportProgress`, `::ExportProgress` | Logic to UI | a percentage, past one megabyte (`PROSE-IO-021`) |

```rust
pub enum ImportFormat { Ambre, Docx, Markdown, Html }

pub enum ExportFormat { Ambre, Docx, Markdown, Pdf, Html }

pub struct ExportOptions {
    /// Gates the comment threads in the Word and the HTML writers (`PROSE-IO-022`).
    pub include_comments:      bool,
    /// Gates the suggestion marks in the same two writers (`PROSE-IO-022`).
    pub include_suggestions:   bool,
    /// Inline the stylesheet, for a mail client that drops a style block.
    pub html_inline_css:       bool,
    pub pdf_include_toc_links: bool,
    /// BCP 47. Absent falls back to the document's own language, then to none.
    pub pdf_language:          Option<String>,
    pub markdown_flavor:       MarkdownFlavor,
}

pub enum MarkdownFlavor { Gfm, CommonMark }
```

The media type each format declares:

| Format | Media type |
|---|---|
| the native file | `application/octet-stream` |
| Word | `application/vnd.openxmlformats-officedocument.wordprocessingml.document` |
| Markdown | `text/markdown; charset=utf-8` |
| PDF | `application/pdf` |
| HTML | `text/html; charset=utf-8` |

A Markdown and an HTML writer produce text, and the bus encodes it as UTF-8 bytes, so the view has
one download path and reads the media type to name the file (`PROSE-IO-020`).

**The native file.** Prose owns one pair of entry points over the shared container, and nothing
else: the header layout, the version registry, the migration policy and the at-rest decision are
`ambre-spec-docfile.md section 3`. Prose writes its own document-type byte, and a file carrying
another one is handed back to the caller for re-dispatch rather than read
(`PROSE-IO-013`). A native file on disk is a different destination from the network snapshot: the
snapshot carries no header and is sealed by the Sync worker, and the file is not.

**What a Word file becomes, and what it loses:**

| Block | Word element |
|---|---|
| a paragraph | `w:p` with its style reference |
| a heading | `w:p` under the built-in heading style of that level |
| a list item | `w:p` under the list style, with the numbering the writer manages |
| a table | `w:tbl`, the widths in twips, the header row and the banding on the table properties |
| a code block | `w:p` under a code style the writer injects when the file has none |
| a picture | a drawing plus an image relationship, its bytes read from the local cache |
| a page break, a rule | a break run, a bottom border |
| a table of contents | the standard field |
| a footnote | an entry in the footnote part |

| Mark | Word element |
|---|---|
| bold, italic, underline, strikethrough | `w:b`, `w:i`, `w:u`, `w:strike` |
| superscript, subscript | `w:vertAlign` |
| inline code | a character style the writer injects |
| colour, size, family | `w:color`, `w:sz` in half-points, `w:rFonts` |
| background | the nearest of the eight highlights, or an exact shading |
| a link | a hyperlink run and an external relationship |
| a suggested insertion or deletion | `w:ins` or `w:del`, carrying the author and the date |
| a comment | a range start and end, plus an entry in the comment part |
| a dynamic field | a field run: page, page count, date or title |

| What is lost | What the reader or the writer does |
|---|---|
| a custom style Word has no name for | falls back to the normal style, and warns (`PROSE-IO-006`) |
| a background colour outside the eight | approximated, and warned |
| the automatic section numbering | not written: Word numbers through its own fields, and the writer warns |
| a picture in a metafile or a tag image format | skipped and warned: neither decodes in the client |
| a Word checkbox control | read as a to-do item when it is recognisable, warned when it is not |
| a picture inside a cell | lifted beside the table, and the lift is reported (`PROSE-IO-026`) |

A page break and a horizontal rule are distinct in Word, and both survive.

**What a Markdown file becomes.** The flavour decides the table, the to-do item and the footnote;
everything else is common:

| Block | Markdown |
|---|---|
| a paragraph, a heading | a paragraph, a hash run |
| a list item | a dash or an ordinal, nested by two spaces |
| a to-do item | a dash and a bracket, in the extended flavour alone |
| a table | a pipe table, in the extended flavour alone |
| a code block | a fence carrying the language |
| a picture | a link, from the local cache as a data address or from the source address |
| a page break | a rule, and a warning: the format has no page (`PROSE-IO-018`) |
| a table of contents | a comment marker, which no reader renders |
| a footnote | a reference and a definition, in the extended flavour alone |

Markdown has no superscript, subscript, colour, size, family, comment, suggestion or cross
reference, and a picture's width, height, alignment and caption have no syntax either. Each one is
a warning at the drop site, never a silent omission (`PROSE-IO-018`, `PROSE-IO-004`).

**What an HTML file becomes.** A standalone document, with the stylesheet in the head or inline:

| Block | Element |
|---|---|
| a paragraph, a heading | `p`, `h1` to `h6` |
| a list item | `li`, under `ul` or `ol`, and under a to-do list with a checkbox |
| a table | `table`, with a head section when the table has a header row |
| a code block | `pre` and `code`, carrying the language as a class |
| a picture, a page break, a rule | `img`, a rule carrying a page-break class, `hr` |
| a table of contents | a labelled `nav` holding an ordered list |

A mark becomes its semantic element -- emphasis, strong emphasis, deletion, insertion, superscript,
subscript, code, anchor -- and a colour, a size or a family becomes an inline style. A comment and
a suggestion become a data attribute carrying the identifier, and both are gated by the export
options (`PROSE-IO-022`). The header and the footer have no element at all, so the writer reports
them (`PROSE-IO-019`).

**HTML import is the paste path.** The same reader serves a file the user chose and the rich half
of a paste, so a paste from a web page and an import of the same page produce the same blocks
(`PROSE-IO-025`, `features/ambre-spec-prose-clipboard.md`). It is the one leg of this feature a
user waits on, which is why it carries its own budget row.

**A remote address is not fetched.** A Markdown or an HTML picture may name a URL rather than a
content hash. The Logic thread never opens a socket, and a fetch on the import path would turn
opening a file into a signal to a third party. So the picture lands unresolved and the report names
it (`PROSE-IO-023`).

**The crates each path uses:**

| Crate | Role | Declared |
|---|---|---|
| `docx-rs` | reads and writes the Word format, in pure Rust | no |
| `pulldown-cmark` | reads Markdown, tables, footnotes and strikethrough included | no |
| `pulldown-cmark-to-cmark` | writes an event stream back as Markdown | no |
| `html5ever` | reads HTML that may be malformed | no |
| `markup5ever_rcdom` | the document object the HTML reader walks | no |
| `ambre-core` | the identifier and the document-type byte | yes |
| `postcard`, `serde` | the fixtures, and nothing on the bus | yes |

None of the four readers is in the manifest today: `crates/ambre-prose` declares none of them, and
`ambre-spec-budgets.md section 5` already budgets three for the day they arrive.

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

**There is none** (`PROSE-IO-027`, `PROSE-IO-028`). The chrome Prose declares is the contextual
toolbar (`features/ambre-spec-prose-toolbar.md`), the slash menu
(`features/ambre-spec-prose-slash-command.md`), the ruler (`features/ambre-spec-prose-ruler.md`) and
the comment panel (`features/ambre-spec-prose-comments.md`); no entry among them opens a file and
none saves one.

| Trigger, once it exists | What must happen |
|---|---|
| the open entry, or a file dropped on the page | the file is read and applied as one batch (`PROSE-IO-011`) |
| the save entry | the Logic thread serializes and answers with the bytes and their media type |
| the export entry, per format | a dialog collects the options, then the same path runs |
| the answer | the view hands the bytes to the browser; the document does not change (`PROSE-IO-017`) |
| the report | what the conversion lost, named construct by construct (`PROSE-IO-004`) |

**The two options a reader must see before an export.** The comments and the suggestions travel in
a Word and in an HTML file, and a document handed to a third party carries the review with it
unless the export leaves it out. The dialog states both, and the writer honours both
(`PROSE-IO-022`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| Every conversion runs on the user's machine | the pillar is that the server holds ciphertext. A conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device | converting on the server, which is faster to build and hands the relay the whole document |
| One intermediate form between every reader and every writer | four formats read and written directly would be twelve converters and twelve places to forget a warning. One form makes a format a reader plus a writer, and the loss report a property of the form rather than of a pair | writing commands straight out of each reader, which gives no place to validate, no place to report, and nothing to roll back |
| The reader emits no command, and the writer reads no document | it is what makes a conversion testable with no engine: a reader is bytes to a value, a writer is a value to bytes, and both are pure functions on the host. It is also the only way the walk stays one budget row for four formats | letting each writer read the document, which multiplies the walk by the format count and couples every writer to the CRDT |
| The native file is the only lossless path | every other format has a model of its own, so a round trip through it is a conversion twice. The native payload is the document itself | promising fidelity on Word, which has no place for a Prose custom style and numbers its own headings |
| The native path skips the intermediate form | its payload is a snapshot, and a snapshot through an intermediate form would be a conversion of the document into a subset of itself. The form exists to lose things safely, and this path loses nothing | routing the native file through the form too, which trades the one lossless path for symmetry |
| The container is Foundation's, not Prose's | the header carries a document-type byte and re-dispatches across applications, so it is a contract between three applications and belongs to none of them | a Prose header, which is what the corpus had, and which made each new application restate the layout |
| A picture is an address, and an import never fetches one | the Logic thread holds no key and opens no socket, and a fetch on the import path would tell a third party which document a user just opened, before the user has seen it | fetching a remote picture during the import, which puts a network call in the thread whose whole contract is that it makes none |
| A block the tree refuses is lifted, never dropped | a file the reader opens is not a command they typed, so losing part of it in silence loses content they still have and cannot see is gone. Lifting keeps it, and the report says where it went | refusing the file, which fails on an ordinary Word document; or dropping the block, which is the silent loss this pipeline exists to make impossible |
| A wildcard arm warns before it discards | a reader meets constructs its author never saw, and the honest answer is to name them. An arm that discards in silence turns every future format extension into an invisible data loss | matching the rest and moving on, which is what makes a lossy import look like a faithful one |
| An export is idempotent | a user who exports twice and gets two different files cannot tell a real change from a serialization detail, and neither can a test. Fixing the order of every attribute is what makes the round trip provable at all | writing in map order, which is stable within one run and different across builds |
| The comments and the suggestions are opt-in on the way out | a review is internal by default: a document sent to a client should not carry who doubted which sentence. The safe direction is the one that needs an explicit act to publish | exporting them always, which publishes the review with the document and is discovered by the reader on the other side |
| A report accompanies a conversion that finished, and an error replaces one that did not | the two are different facts and a caller acts on them differently: one is shown beside a document, the other instead of it. Merging them forces every caller to ask which case it is holding | one error type, which makes "the file opened with three warnings" and "the file did not open" the same value |
| A progress event past one megabyte | a conversion that finishes in ten milliseconds needs no progress bar, and one that takes ten seconds with no sign of life reads as a freeze. The threshold is where the first becomes the second | reporting progress always, which spends an event per small import; or never, which is the freeze |
| Word and the clipboard are the two Google Docs paths | there is no way to read a Google document from the client without an authenticated network call to Google, which leaks which document is opened and when. A download and a paste both arrive as formats already read | calling the Google API from the client, which trades a metadata leak for a convenience the download already gives |
| The HTML reader is the paste reader | a paste from a web page and an import of that page are the same bytes in the same format, so two readers would be two results for one input, and the second one to drift would be the one nobody tested | a paste-specific reader, which is a second HTML implementation with a second set of losses |

---

## 6. NOT BUILT, AND NOT YET RULED

* **No gesture reaches an import or an export** (`PROSE-IO-027`, `PROSE-IO-028`). Prose's chrome is
  a toolbar, a slash menu, a ruler and a comment panel, and none of the four opens or saves a file.
  Two commands are declared on the plane and nothing emits them. Same shape as the collaboration
  features, which are specified from the second step onward.
* **Where an import lands is not ruled.** A failed import leaves the document untouched
  (`PROSE-IO-011`), and nothing says what a successful one does to a document that already has
  content: replace it, append to it, or insert at the caret. Toile answers the same question with
  `TOILE-IO-023` and Prose states nothing, so the batch of step 3 has no defined destination.
* **A named style's character defaults have no home in the document.** The intermediate form
  carries them, because a Word style carries them, and the style register of
  `ambre-spec-prose-schema.md section 3` holds paragraph fields alone. A Word file whose "Quote"
  style is italic therefore imports as a style that is not italic, and no warning covers it: the
  loss is in the command builder, not in the reader.
* **A comment's author survives an export and not a rename.** The document stamps a display name
  and a document peer, and the Word format carries an author string, so the round trip is faithful
  to what Prose stores. It stores no stable user identity, which
  `features/ambre-spec-prose-comments.md section 6` records, and an export freezes whatever name
  the author had that day.
* **A cross reference crosses formats as a bookmark and comes back by name.** The document holds a
  block identifier, Word holds a generated bookmark, and nothing states how the two are joined on a
  round trip through a file that a third party edited in between.
* **Nothing bounds an import.** Toile clamps foreign geometry, bounds the element count and bounds
  the nesting depth (`TOILE-IO-002`); Prose bounds nothing. A file with a million blocks, or a list
  nested ten thousand deep, has no stated refusal, and the atomicity rule means the whole batch is
  held in memory before any of it is applied.
* **No reader has a fuzz target.** They are fed by files the user did not write, and
  `ambre-spec-testing.md` names them as the first targets. None exists, and neither does the crate.
* **Three applications now import and export, and the vocabulary is theirs alone.** Trame reports
  an import through one type and one event name, Toile through another, and Prose through a third:
  the report, the progress event and the completion event differ in shape and in spelling for one
  behaviour. The formats share nothing and belong to each application; the report a user reads does
  not. No rule holds the three together, and no document owns the question.
* **The four reader crates are a plan.** `crates/ambre-prose` declares none of them, so the
  dependency table above marks each one absent rather than reading as a description of the
  manifest.
