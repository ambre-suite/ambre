# AMBRE -- FEATURE: PROSE IMPORT AND EXPORT

**Type:** Vertical (feature)
**Implementation:** partial

> A user opens a document that came from Word, from a web page or from a Markdown file. They hand
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

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-001` | `INV` | Every conversion MUST run in the Logic thread. | this spec section 3; `ZK-001` | structural -- the reader, the writer, the walk and the builder run inside `ProseState::handle_command`, which only the Logic worker holds; `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` | none | verified |
| `PROSE-IO-043` | `INV` | No part of a document MUST reach the relay while a conversion runs. | this spec section 3; `ZK-001` | structural -- an import leaves as the one delta its commit produces, which Sync seals like any edit; `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` -- an export writes no delta | none | verified |
| `PROSE-IO-044` | `INV` | A reader and a writer MUST meet in the intermediate form alone: neither MUST know the other. | this spec section 3 | structural -- `crates/ambre-prose/src/interop/markdown_read.rs` and `markdown_write.rs` share `ProseIr`, and neither names the other outside its tests | none | verified |
| `PROSE-IO-009` | `INV` | A reader MUST NOT emit a command. | this spec section 3 | structural -- `markdown_read::read` returns a `ProseIr` and names no command, document or engine type | none | verified |
| `PROSE-IO-045` | `INV` | A writer MUST NOT read the document. | this spec section 3 | structural -- `markdown_write::write` takes a `&ProseIr` and nothing else | none | verified |
| `PROSE-IO-046` | `INV` | The walk from the document into the intermediate form MUST be one, shared by every writer. | this spec section 3; `ambre-spec-budgets.md section 1` | structural -- `ProseState::walk` in `crates/ambre-prose/src/logic/interop.rs` is the one walk, and the Markdown export calls it | none | verified |
| `PROSE-IO-004` | `INV` | Every fidelity loss MUST raise a warning naming the construct that was dropped. | this spec section 3 | `ambre_prose::interop::markdown_write::tests::what_markdown_lacks_is_reported_once_each`; `ambre_prose::interop::markdown_write::tests::commonmark_writes_no_table_todo_or_footnote`; `ambre_prose::logic::interop::tests::a_styled_cell_is_reported_on_export` | none | verified |
| `PROSE-IO-005` | `INV` | A reader MUST warn before it drops an element it does not recognise. | this spec section 3 | `ambre_prose::interop::markdown_read::tests::an_unknown_element_is_reported_and_its_text_kept` | none | verified |
| `PROSE-IO-006` | `INV` | A writer that emits an approximation MUST record it in the report. | this spec section 3 | not built | unbuilt -- lands with the `DOCX interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-047` | `INV` | A conversion that finished MUST answer with a report, and one that produced nothing MUST answer with an error: the two MUST NOT be one type. | this spec section 3 | structural -- `ConversionReport` and `InteropError` are two types; `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing` | none | verified |
| `PROSE-IO-048` | `INV` | A conversion MUST NOT hold the Logic thread for the length of the file. | this spec section 3; `ambre-spec-budgets.md section 1` | not built | unbuilt -- lands with the `Large-file import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-IO-025` | `INV` | The reader an HTML import uses MUST be the reader a rich paste uses: they MUST NOT be two readers. | this spec section 3; `features/ambre-spec-prose-clipboard.md` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-049` | `INV` | The native file MUST reuse the shared container rather than restate it. | this spec section 3; `ambre-spec-docfile.md section 3` | structural -- `crates/ambre-prose/src/logic/docfile.rs` declares no header of its own and calls the shared writer and reader; `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot` | none | verified |
| `PROSE-IO-012` | `IO` | The native file MUST be the only lossless round trip. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_round_trip_loses_nothing` -- a bare engine fed the payload re-exports the same file, and a live state reading it holds the same text and marks | none | verified |
| `PROSE-IO-014` | `IO` | A Word round trip MUST keep every structure the intermediate form represents, or warn for each one it drops. | this spec section 3 | not built | unbuilt -- lands with the `DOCX interop` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Bring a file into the document

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-027` | `POST` | A user MUST be able to bring a file into the document. | this spec section 4 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks`; `ambre_prose::commands::tests::the_save_entry_is_declared_once` -- the palette lists the import | none | verified |
| `PROSE-IO-002` | `POST` | An import MUST write through a typed operation list the engine applies as one change. | this spec section 3 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` -- the builder emits typed `ImportOp`s | none | verified |
| `PROSE-IO-050` | `POST` | The commands MUST be built in tree order, parent before child. | this spec section 3 | `ambre_prose::logic::interop::tests::a_markdown_file_lands_as_blocks` -- a nested item and the cells land under their parent; structural -- `build` pushes a parent before it recurses | none | verified |
| `PROSE-IO-051` | `POST` | The whole file MUST apply as one batch: one undo MUST take the import back. | this spec section 3 | `ambre_prose::logic::interop::tests::one_undo_takes_the_import_back` | none | verified |
| `PROSE-IO-026` | `POST` | A construct the block tree refuses MUST be lifted to the nearest place the tree allows, never dropped. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::interop::tests::a_block_the_item_refuses_is_lifted_and_reported`; structural -- `place` in `crates/ambre-prose/src/logic/interop.rs`, the one builder every import goes through, walks up the parent chain | none | verified |
| `PROSE-IO-052` | `POST` | Every lift MUST be reported. | this spec section 3; `ambre-spec-prose-schema.md section 4` | `ambre_prose::logic::interop::tests::a_block_the_item_refuses_is_lifted_and_reported`; structural -- `place` records each lift in the import's warnings | none | verified |
| `PROSE-IO-053` | `POST` | The report MUST reach the user: a construct changed in silence MUST NOT be the outcome. | this spec section 4 | `ambre_prose::interop::tests::a_report_reads_as_one_sentence`; structural -- `ImportCompleted` carries the report, and `ambre_prose::chrome::notify_event` toasts it | none | verified |
| `PROSE-IO-021` | `POST` | An import of a large file MUST report its progress. | this spec section 3; `ambre-spec-budgets.md section 1` | not built | unbuilt -- lands with the `Large-file import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-IO-054` | `POST` | An imported picture naming a remote address MUST be reported as unresolved. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-023` | `FRAME` | An import MUST NOT fetch a remote address a picture names. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-055` | `FRAME` | An import MUST NOT mutate the document before its reader has finished: the reader and the builder read no document, and the typed operation list is the only writer. | this spec section 3 | structural -- `build` reads no document, and `apply_import` alone writes | none | verified |
| `PROSE-IO-011` | `FAIL` | A failed import MUST leave the document untouched: a partial application MUST NOT be observable. | this spec section 3 | `ambre_prose::logic::interop::tests::a_file_that_does_not_read_changes_nothing`; `ambre_prose::logic::interop::tests::a_refusal_midway_leaves_nothing_for_the_next_commit` | none | verified |
| `PROSE-IO-010` | `FAIL` | A malformed or hostile file MUST fail as a typed error; it MUST NOT panic. | this spec section 3; `ambre-spec-testing.md` | `ambre_prose::interop::markdown_read::tests::bytes_that_are_not_text_fail_typed`; `ambre_prose::interop::markdown_read::tests::a_hostile_nesting_is_capped`; cargo-fuzz -- `fuzz/fuzz_targets/markdown.rs`, run by the CI `fuzz` job | none | verified |

### 1.3 Take the document out as a file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-028` | `POST` | A user MUST be able to take the document out as a file. | this spec section 4 | `ambre_logic::dispatch::tests::a_prose_document_saves_to_a_native_file_and_reopens` -- the save command goes through the real dispatcher; `ambre_prose::commands::tests::the_save_entry_is_declared_once` -- the palette lists the entry | none | verified |
| `PROSE-IO-056` | `POST` | The exported bytes MUST reach the browser download path. | this spec section 4 | `ambre_prose::ui::tests::a_written_file_is_handed_to_the_host_once` -- the view holds the bytes for the host once; `ambre_belt::prose::tests::a_markdown_export_reaches_the_download_directory` -- browser tier: the typed paragraph lands in the download directory as Markdown | none | verified |
| `PROSE-IO-003` | `POST` | An export MUST be idempotent: the same document exported twice MUST yield the same bytes, up to the stamp the format writes. | this spec section 3 | `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` | none | verified |
| `PROSE-IO-020` | `POST` | An export MUST carry the media type of its bytes, so one download path serves every format. | this spec section 3 | `ambre_logic::dispatch::tests::a_prose_document_saves_to_a_native_file_and_reopens`; structural -- `ProseEvent::ExportReady` carries `media_type`, set from `ExportFormat::media_type` | none | verified |
| `PROSE-IO-022` | `POST` | An export MUST omit the comments and the suggestions the user did not ask for. | this spec section 4; `ZK-001` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-057` | `POST` | An export of a large document MUST report its progress. | this spec section 3; `ambre-spec-budgets.md section 1` | not built | unbuilt -- lands with the `Large-file import` Story (`ambre-roadmap.md` E6) | unimplemented |
| `PROSE-IO-017` | `FRAME` | An export to any format but PDF MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | `ambre_prose::logic::docfile::tests::an_export_changes_nothing` -- the version vector is the same after the export; `ambre_prose::logic::interop::tests::an_export_is_idempotent_and_writes_nothing` | none | verified |

### 1.4 Save a native file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-058` | `POST` | A native file MUST carry Prose's own document-type byte. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot`; `ambre_logic::dispatch::tests::a_saved_file_carries_the_document_id_the_registry_keyed_it_under` | none | verified |
| `PROSE-IO-059` | `FRAME` | A native export MUST NOT route the document through the intermediate form: its payload MUST be the snapshot itself. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_header_names_prose_and_the_payload_is_the_snapshot` -- the payload equals the engine's snapshot byte for byte | none | verified |

### 1.5 Open a native file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-013` | `FAIL` | A native file belonging to another application MUST be re-dispatched, never coerced into Prose. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::a_file_of_another_application_is_handed_back` -- a Trame file comes back as `WrongDocType` and the document is unchanged | none | verified |
| `PROSE-IO-065` | `POST` | A native Prose file MUST open from the home screen as a Prose document. | this spec section 4; `ambre-spec-docfile.md section 3` | `ambre_belt::prose::tests::a_native_prose_file_opens_from_the_home_screen` -- browser tier: a file saved in one browser opens from the home screen of another as a Prose view holding its text | none | verified |
| `PROSE-IO-066` | `FRAME` | Opening a native file MUST NOT write to the document: the opened document MUST export the bytes it was read from. | this spec section 3; `ambre-spec-docfile.md section 3` | `ambre_prose::logic::docfile::tests::the_round_trip_loses_nothing` -- a bare engine fed the payload re-exports the file byte for byte | none | verified |

### 1.6 Import a Word file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-060` | `POST` | A tracked change and a comment the file carries MUST arrive as a suggestion and a comment thread, never as plain text. | this spec section 3 | not built | unbuilt -- lands with the `DOCX interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-024` | `FRAME` | A Word import MUST NOT run a macro the file carries. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `DOCX interop` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.7 Export to Markdown

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-018` | `POST` | A Markdown export MUST report every construct the flavour has no syntax for, the page model included. | this spec section 3 | `ambre_prose::interop::markdown_write::tests::what_markdown_lacks_is_reported_once_each`; `ambre_prose::logic::interop::tests::the_page_model_is_reported` | none | verified |
| `PROSE-IO-061` | `FRAME` | A Markdown export MUST NOT emit a table, a to-do item or a footnote the chosen flavour has no syntax for. | this spec section 3 | `ambre_prose::interop::markdown_write::tests::commonmark_writes_no_table_todo_or_footnote` | none | verified |
| `PROSE-IO-067` | `FRAME` | A Markdown export MUST NOT write a link whose target `PROSE-UI-186` holds back from the host: the run exports as its text alone. | this spec section 3; `PROSE-UI-186` | `ambre_prose::interop::markdown_write::tests::a_link_that_could_run_code_exports_as_its_text` -- a `javascript:` and a `data:` link export as text, a `mailto:` link as a link | none | verified |
| `PROSE-IO-068` | `FRAME` | A Markdown export MUST NOT write a link target that ends its destination, its paragraph or its block: a space, a control character, an angle bracket and a backslash are percent-encoded. | this spec section 3 | `ambre_prose::interop::markdown_write::tests::a_link_target_cannot_break_out_of_its_destination` -- targets carrying a blank line and a link, a blank line and a heading, a paren, angle brackets, a trailing backslash, a tab and a delete | none | verified |
| `PROSE-IO-069` | `FRAME` | A Markdown export MUST NOT write a link target longer than the HTML half of a copy writes: the run exports as its text alone. | this spec section 3; `PROSE-UI-235` | `ambre_prose::interop::markdown_write::tests::a_link_longer_than_a_copy_writes_exports_as_its_text` -- a target one byte past the cap exports as text, a target at the cap as a link | none | verified |

### 1.8 Import an HTML file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-016` | `FRAME` | An imported HTML document MUST NOT run a script it carries. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-062` | `FRAME` | An HTML import MUST NOT resolve a remote reference the document carries: no stylesheet, no font, no frame. | this spec section 3; `ZK-001` | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.9 Export to HTML

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-019` | `POST` | An HTML export MUST report the header and the footer, which the format has no element for. | this spec section 3 | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-063` | `POST` | An HTML export MUST stand alone: it MUST carry the stylesheet it needs, in the head or inline. | this spec section 3 | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-064` | `FRAME` | An HTML export MUST NOT emit a script or an event handler: the file it writes MUST be inert. | this spec section 3 | not built | unbuilt -- lands with the `HTML interop` Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** six rows of `ambre-spec-budgets.md section 1`: the Word import and export of a
  hundred pages, and the Markdown export. The other three are the HTML clipboard import, the
  plaintext native export, and the shared walk. Every one reads **none exists**.
* **Boundary.** a local file, in both directions, and nothing else. `PROSE-IO-001` keeps every
  conversion in the Logic thread, `PROSE-IO-043` keeps the document off the relay, and `PROSE-
  IO-016` stops an imported file executing or fetching.
* **Verification.** 36 `verified`, 16 `unimplemented`. The native file and the Markdown pair are
  built. The Word and HTML pairs are not, and no conversion runs in slices yet. The browser tier
  holds the download and the open from the home screen.

---

## 2. WHAT EACH USER SEES

An import is an ordinary write, so it reaches the other peer like any edit; an export writes
nothing. No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| imports a Word file | types elsewhere | the imported blocks and the typing | both, on both screens | none -- `PROSE-IO-002` |
| imports a file that fails to parse | watches | the document as before | an error on Alice's screen and no change on Bob's | none -- `PROSE-IO-011` |
| imports a file, then undoes | watches | the document as before the import | the import leaves in one step, on both screens | none -- `PROSE-IO-051` |
| exports to Markdown | types | the typing alone: an export writes nothing | Bob's typing; Alice's file holds the document she exported | none -- `PROSE-IO-017` |
| exports the same document twice | -- | nothing | the same bytes, twice | none -- `PROSE-IO-003` |
| saves a native file | opens it | the same document state | the same blocks, styles and comments | none -- `PROSE-IO-012` |
| imports a Word table holding a picture | watches | the picture beside the table, never inside it | the picture out of the cell, and a line in the report | none -- `PROSE-IO-026`, `PROSE-IO-052` |

---

## 3. MODEL

**Containers.** This feature owns none. An import writes the block tree, the style register and the
annotation register the schema declares (`ambre-spec-prose-schema.md section 3`). The native export
serializes the whole document and adds nothing to it.

**The pipeline is symmetric.** A reader turns bytes into the intermediate form, and a writer turns
the intermediate form into bytes. Neither half knows the other (`PROSE-IO-044`), and neither
touches the document (`PROSE-IO-009`, `PROSE-IO-045`).

```text
Import:  bytes -> reader -> ProseIr -> command builder -> Vec<ImportOp> -> the document
Export:  the document -> document walk -> ProseIr -> writer -> bytes
```

The native path is the one exception, and it is deliberate. It carries a Loro snapshot under the
shared header, so it passes through no intermediate form and loses nothing (`PROSE-IO-012`,
`PROSE-IO-059`).

**The intermediate form.** A linear tree, held in memory for the length of one conversion and never
serialized:

```rust
// ambre-prose/src/interop/mod.rs
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProseIr {
    pub meta:      IrMeta,
    pub blocks:    Vec<IrBlock>,
    /// The footnote bodies, in the order of their ordinals.
    pub footnotes: Vec<IrFootnote>,
}

/// What the document carries beside its blocks. A flag says a construct is present, for a
/// writer that has no syntax for it to report (`PROSE-IO-018`). `Debug` prints the title's
/// length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IrMeta {
    pub title: Option<String>,
    pub paged: Paging,   // Flow | Paged
    pub bands: Banding,  // Bare | Banded
}

/// One block. Depth is the nesting, never a number: the document derives it too. `Debug`
/// prints the length of a code block's text and of an image's alternative text alone.
#[derive(Clone, PartialEq)]
pub enum IrBlock {
    /// `styled`: a named style, or alignment, indents or spacing of its own.
    Paragraph { styled: OwnStyle, runs: Vec<IrRun> },
    /// The level is the stored one, unclamped (`PROSE-FEAT-091`).
    Heading   { level: StoredLevel, runs: Vec<IrRun> },
    ListItem  { style: IrListStyle, runs: Vec<IrRun>, children: Vec<IrBlock> },
    /// `merged`: two or more cells are merged into one.
    Table     { merged: Merging, rows: Vec<IrTableRow> },
    CodeBlock { lang: Option<CodeLang>, text: String },
    Image     { alt: String },
    PageBreak,
    HRule,
    Toc,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableRow { pub cells: Vec<IrTableCell> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrTableCell {
    /// A fill, a border, a padding or a vertical alignment.
    pub styled: OwnStyle,  // Plain | Styled
    /// A reader may put anything here; the command builder lifts what the tree refuses
    /// (`PROSE-IO-026`).
    pub blocks: Vec<IrBlock>,
}

/// One footnote body. A run points at it by ordinal.
#[derive(Clone, Debug, PartialEq)]
pub struct IrFootnote { pub ordinal: Ordinal<FootnoteCall>, pub runs: Vec<IrRun> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrListStyle { Bullet, Ordered(IrOrderedStyle), Todo(TodoMark) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrOrderedStyle { Decimal, LowerAlpha, UpperAlpha, LowerRoman, UpperRoman }

/// `Debug` prints the text's length alone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct IrRun {
    pub text:          String,
    pub bold:          Weight,         // Regular | Bold
    pub italic:        Slant,          // Upright | Italic
    pub underline:     Underline,      // Plain | Underlined
    pub strikethrough: Strikethrough,  // Plain | Struck
    pub superscript:   Superscript,    // Level | Raised
    pub subscript:     Subscript,      // Level | Lowered
    pub code:          InlineCode,     // Prose | Code
    pub link:          Option<LinkUrl>,
    /// A colour, a background, a size or a family.
    pub styled:        OwnStyle,       // Plain | Styled
    pub comment:       Commenting,     // Uncommented | Commented
    pub suggestion:    Suggesting,     // Settled | Suggested
    pub cross_ref:     Referencing,    // Text | Reference
    /// The ordinal of the footnote this run calls.
    pub footnote_ref:  Option<Ordinal<FootnoteCall>>,
}
```

**The fields the Word, HTML and PDF filters add.** Not built: they join the intermediate form with
the reader or the writer that reads them, under their Stories (`ambre-roadmap.md` E5). The
Markdown writer has no syntax for any of them, which is why the form above keeps a flag where
these keep the construct. Each struct below lists only what it gains, and a flag gives way to the
field that carries its construct:

```rust
// ambre-prose/src/interop/mod.rs -- not built
pub struct ProseIr {
    // ...
    pub styles:   Vec<IrStyle>,
    /// Comment threads, out of band from the blocks: a run carries the thread id.
    pub comments: Vec<IrComment>,
}

pub struct IrMeta {
    // ...
    /// BCP 47, as the document's `meta.locale` stores it.
    pub lang:         Option<LangTag>,
    /// Replaces `paged`.
    pub page:         Option<IrPage>,
    /// Replace `bands`. The `first` pair is set only when the document uses a distinct first
    /// page.
    pub header:       Option<IrHeaderFooter>,
    pub header_first: Option<IrHeaderFooter>,
    pub footer:       Option<IrHeaderFooter>,
    pub footer_first: Option<IrHeaderFooter>,
}

pub struct IrPage {
    pub width:         Mm,
    pub height:        Mm,
    pub margin_top:    Mm,
    pub margin_bottom: Mm,
    pub margin_left:   Mm,
    pub margin_right:  Mm,
    pub orientation:   Orientation,
}

/// One paragraph of runs plus its alignment. A dynamic field is a run, not a marker.
pub struct IrHeaderFooter { pub align: Align, pub runs: Vec<IrRun> }

pub enum IrBlock {
    /// `style_id` and `para_style` replace `styled`.
    Paragraph { style_id: Option<StyleId>, para_style: IrParaStyle, runs: Vec<IrRun> },
    Heading   { level: StoredLevel, numbered: bool, runs: Vec<IrRun> },
    /// `col_widths` and `style` join `rows`; a cell's spans replace `merged`.
    Table     { col_widths: Vec<Pt>, style: IrTableStyle, rows: Vec<IrTableRow> },
    Image     { content_hash: Option<ContentHash>, src_url: Option<LinkUrl>, alt: String,
                width: Option<Px<Image>>, height: Option<Px<Image>>,
                align: Option<IrImageAlign>, caption: Option<String> },
    Toc       { max_level: HeadingLevel, numbered: bool },
    // ...
}

pub struct IrTableStyle {
    /// Promotes the first row to a header row.
    pub header_row:  bool,
    pub banded_rows: bool,
}

/// The fields below replace `styled`.
pub struct IrTableCell {
    pub rowspan:      Count<Row>,
    pub colspan:      Count<Col>,
    pub bg:           Option<Rgb>,
    pub borders:      bool,
    pub border_color: Option<Rgb>,
    pub padding:      Option<Pt>,
    pub valign:       VAlign,
    // ...
}

pub struct IrRun {
    // ...
    /// Foreground: a run is always opaque. These four replace `styled`.
    pub color:          Option<Rgb>,
    pub background:     Option<Rgb>,
    pub font_size:      Option<FontSize>,
    pub font_family:    Option<FontFamily>,
    /// Replaces `comment`.
    pub comment_id:     Option<Uuid>,
    /// The suggested insertion and the suggested deletion, mirroring the marks `si` and `sd`.
    /// These four replace `suggestion`.
    pub suggest_ins:    Option<Uuid>,
    pub suggest_del:    Option<Uuid>,
    pub suggest_author: Option<DisplayName>,
    pub suggest_date:   Option<Instant<Authored>>,
    /// Set inside a header or a footer alone.
    pub field:          Option<HeaderField>,
    /// Replaces `cross_ref`.
    pub cross_ref:      Option<(Uuid, CrossRefKind)>,
}

pub struct IrParaStyle {
    pub align:          Option<Align>,
    pub indent_first:   Option<Pt>,
    pub indent_left:    Option<Pt>,
    pub indent_right:   Option<Pt>,
    pub space_before:   Option<Pt>,
    pub space_after:    Option<Pt>,
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
    pub font_family:   Option<FontFamily>,
    pub font_size:     Option<FontSize>,
    pub color:         Option<Rgb>,
    pub background:    Option<Rgb>,
}

pub struct IrStyle {
    pub id:         StyleId,
    pub name:       String,
    pub para_style: IrParaStyle,
    pub run_style:  IrRunStyle,
}

/// A thread. The commented range is the `cm` mark on the runs, never a field here.
pub struct IrComment {
    pub id:         Uuid,
    pub resolved:   bool,
    pub created_at: Instant<Authored>,
    /// The opening body first, then the replies.
    pub replies:    Vec<IrReply>,
}

pub struct IrReply {
    pub id:         Uuid,
    pub author:     DisplayName,
    pub created_at: Instant<Authored>,
    pub text:       String,
}

pub enum IrImageAlign { Block, Left, Right, Center }
```

**The report, and the two error kinds.** A report is what the user reads after a conversion that
finished. An error is what a caller gets when no document could be produced at all:

```rust
// ambre_bus::app::prose
/// A conversion that produced nothing answers an `InteropError` instead: the two are never
/// one type (`PROSE-IO-047`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversionReport {
    pub format:   ConversionFormat,
    pub warnings: Vec<ConversionWarning>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionFormat { Ambre, Markdown }

/// The same construct lost twice is one warning with its count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionWarning {
    /// A construct the format has no syntax for, dropped or approximated (`PROSE-IO-004`).
    FeatureNotSupported { feature: UnsupportedFeature, format: ConversionFormat, count: u32 },
    /// An element the reader does not recognise, its text kept where it has any
    /// (`PROSE-IO-005`).
    ElementDropped      { element: DroppedElement, count: u32 },
    /// A block the tree refuses where the file put it, moved (`PROSE-IO-026`, `PROSE-IO-052`).
    BlockLifted         { block: LiftedBlock, count: u32 },
}

/// The whole call failed and produced nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InteropError {
    /// The bytes do not read as the format named.
    ParseFailed(String),
    /// The file is larger than a conversion accepts.
    TooLarge(ByteSize),
}
```

A warning names its construct from a closed list and carries a count, so a long file does not bury
the user in repeats.

**Steps, on an import:**

1. read the whole file into the intermediate form, and return a typed error carrying no command on
   any parse failure (`PROSE-IO-010`, `PROSE-IO-011`);
2. lift every block the tree refuses to the nearest place it allows, and record each lift
   (`PROSE-IO-026`);
3. build the typed operation list, in tree order, parent before child (`PROSE-IO-002`, `PROSE-IO-050`);
4. write them first into a scratch document, and drop it: a refusal there answers the import and
   writes nothing (`PROSE-IO-011`);
5. apply them as one batch, so the whole file is one undo item (`PROSE-IO-051`);
6. hand the report to the view (`PROSE-IO-053`).

Step 2 is a separate pass on purpose. The tree's nesting rule is one predicate
(`ambre-spec-prose-schema.md section 4`), and a reader that applied it inline would need a copy of
it per format.

Step 4 exists because the engine takes no write back once made. A write refused midway would
leave the earlier ones for the next commit. The scratch document holds only the new blocks, so the
rehearsal costs the size of the file, not of the document. The one step that reads the real
document is where the first block lands, and it refuses before anything is written.

**Steps, on an export:** walk the document once into the intermediate form, then hand that form to
the writer. The walk is shared by all four writers (`PROSE-IO-046`). That is what makes it one
budget row rather than four, and what keeps a writer from reading the document (`PROSE-IO-045`).

**The commands and the events** (`ambre-bus`, declared in `features/ambre-spec-prose-engine.md section 3`):

| Shape | Direction | Carries |
|---|---|---|
| `ProseCommand::ImportDocument` | UI to Logic | the format and the bytes |
| `ProseCommand::ExportDocument` | UI to Logic | the format; the options join it with the first writer that reads one |
| `ProseEvent::ImportCompleted` | Logic to UI | the report of what the file lost |
| `ProseEvent::ExportReady` | Logic to UI | the file name, which `DOCFILE-019` and `DOCFILE-020` hold, the bytes and their media type; the report joins it with the first lossy writer |
| `ProseEvent::ImportProgress`, `::ExportProgress` | Logic to UI | a percentage, past one megabyte (`PROSE-IO-021`) |

```rust
// ambre_bus::app::prose. A reader or a writer adds its variant on the day it lands, so the
// view never offers a format no code reads or writes: Word, HTML and the PDF join with their
// Stories (`ambre-roadmap.md` E5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportFormat { Markdown }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The native file: the shared header over the document's own snapshot.
    Ambre,
    Markdown(MarkdownFlavor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownFlavor { Gfm, CommonMark }

/// Not built: the options join `ExportDocument` with the first writer that reads one.
pub struct ExportOptions {
    /// Gates the comment threads in the Word and the HTML writers (`PROSE-IO-022`).
    pub include_comments:      bool,
    /// Gates the suggestion marks in the same two writers (`PROSE-IO-022`).
    pub include_suggestions:   bool,
    /// Inline the stylesheet, for a mail client that drops a style block.
    pub html_inline_css:       bool,
    pub pdf_include_toc_links: bool,
    /// Absent falls back to the document's own language, then to none.
    pub pdf_language:          Option<LangTag>,
}
```

The media type each format declares:

| Format | Media type |
|---|---|
| the native file | `application/octet-stream` |
| Word | `application/vnd.openxmlformats-officedocument.wordprocessingml.document` |
| Markdown | `text/markdown; charset=utf-8` |
| PDF | `application/pdf` |
| HTML | `text/html; charset=utf-8` |

A Markdown and an HTML writer produce text, and the bus encodes it as UTF-8 bytes. The view has
one download path and types the download with the media type (`PROSE-IO-020`). The file name
obeys `DOCFILE-019` and `DOCFILE-020`.

**The native file.** Prose owns one pair of entry points over the shared container, and nothing
else. The header layout, the version registry, the migration policy and the at-rest decision are
`ambre-spec-docfile.md section 3`. Prose writes its own document-type byte (`PROSE-IO-058`), and a file
carrying another one is handed back to the caller for re-dispatch rather than read
(`PROSE-IO-013`). A native file on disk is a different destination from the network snapshot. The
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
| the automatic heading numbering | not written: Word numbers through its own fields, and the writer warns |
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

A peer writes a link's target straight into the document, unchecked. The writer keeps a link only
when the host may open its target, and exports any other as its text alone (`PROSE-IO-067`). A
target longer than a copy writes is exported as its text alone too (`PROSE-IO-069`). A
target it keeps is percent-encoded where Markdown would read it as syntax, so it cannot end its
destination, its paragraph or its block (`PROSE-IO-068`).

**What an HTML file becomes.** A standalone document, with the stylesheet in the head or inline
(`PROSE-IO-063`), and no script in it at all (`PROSE-IO-064`):

| Block | Element |
|---|---|
| a paragraph, a heading | `p`, `h1` to `h6` |
| a list item | `li`, under `ul` or `ol`, and under a to-do list with a checkbox |
| a table | `table`, with a head section when the table has a header row |
| a code block | `pre` and `code`, carrying the language as a class |
| a picture, a page break, a rule | `img`, a rule carrying a page-break class, `hr` |
| a table of contents | a labelled `nav` holding an ordered list |

A mark becomes its semantic element: emphasis, strong emphasis, deletion, insertion, superscript,
subscript, code, anchor. A colour, a size or a family becomes an inline style. A comment and
a suggestion become a data attribute carrying the identifier, and both are gated by the export
options (`PROSE-IO-022`). The header and the footer have no element at all, so the writer reports
them (`PROSE-IO-019`).

**HTML import is the paste path.** The same reader serves a file the user chose and the rich half
of a paste. A paste from a web page and an import of the same page produce the same blocks
(`PROSE-IO-025`, `features/ambre-spec-prose-clipboard.md`). It is the one leg of this feature a
user waits on, which is why it carries its own budget row.

**A remote address is not fetched.** A Markdown or an HTML picture may name a URL rather than a
content hash. The Logic thread never opens a socket. A fetch on the import path would turn
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

**Code:** the native pair is `crates/ambre-prose/src/logic/docfile.rs`. The dispatcher in
`crates/ambre-logic/src/data.rs` serves the save, because the header needs the document id the
registry holds. `ExportFormat` has one variant today, `Ambre`. No reader, no writer and no
intermediate form exist yet.

---

## 4. SURFACE

**The save entry is in the command palette** (`PROSE-IO-028`), declared in
`crates/ambre-prose/src/commands.rs`. A native file opens from the home screen, like a Trame or a
Toile file (`PROSE-IO-065`). No entry imports a foreign format yet (`PROSE-IO-027`).

| Trigger, once it exists | What must happen |
|---|---|
| the open entry, or a file dropped on the page | the file is read and applied as one batch (`PROSE-IO-051`) |
| the save entry | the Logic thread serializes and answers with the bytes and their media type |
| the export entry, per format | a dialog collects the options, then the same path runs |
| the answer | the view hands the bytes to the browser; the document does not change (`PROSE-IO-017`) |
| the report | what the conversion lost, named construct by construct (`PROSE-IO-004`) |

**The two options a reader must see before an export.** The comments and the suggestions travel in
a Word and in an HTML file. A document handed to a third party carries the review with it
unless the export leaves it out. The dialog states both, and the writer honours both
(`PROSE-IO-022`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Every conversion runs on the user's machine | converting on the server | it is faster to build and hands the relay the whole document. The pillar is that the server holds ciphertext; a conversion is the one moment a document exists as plaintext outside the editor, so it is the one that must not leave the device |
| One intermediate form between every reader and every writer | writing commands straight out of each reader | it gives no place to validate, no place to report, and nothing to roll back. Four formats read and written directly would be twelve converters and twelve places to forget a warning; one form makes a format a reader plus a writer, and the loss report a property of the form rather than of a pair |
| The reader emits no command, and the writer reads no document | letting each writer read the document | it multiplies the walk by the format count and couples every writer to the CRDT. A reader is bytes to a value, a writer is a value to bytes, and both are pure functions on the host, which is what makes a conversion testable with no engine and keeps the walk one budget row for four formats |
| The native file is the only lossless path | promising fidelity on Word | Word has no place for a Prose custom style and numbers its own headings. Every other format has a model of its own, so a round trip through it is a conversion twice; the native payload is the document itself |
| The native path skips the intermediate form | routing the native file through the form too | it trades the one lossless path for symmetry. The payload is a snapshot, and a snapshot through an intermediate form would be a conversion of the document into a subset of itself; the form exists to lose things safely, and this path loses nothing |
| The container is Foundation's, not Prose's | a Prose header | it is what the corpus had, and it made each new application restate the layout. The header carries a document-type byte and re-dispatches across applications, so it is a contract between three applications and belongs to none of them |
| A picture is an address, and an import never fetches one | fetching a remote picture during the import | it puts a network call in the thread whose whole contract is that it makes none. The Logic thread holds no key and opens no socket, and a fetch on the import path would tell a third party which document a user just opened, before the user has seen it |
| A block the tree refuses is lifted, never dropped | refusing the file | it fails on an ordinary Word document. A file the reader opens is not a command they typed, so losing part of it in silence loses content they still have and cannot see is gone; lifting keeps it, and the report says where it went |
| A block the tree refuses is lifted, never dropped | dropping the block | it is the silent loss this pipeline exists to make impossible. A file the reader opens is not a command they typed, so losing part of it in silence loses content they still have and cannot see is gone; lifting keeps it, and the report says where it went |
| A wildcard arm warns before it drops | matching the rest and moving on | it is what makes a lossy import look like a faithful one. A reader meets constructs its author never saw, and the honest answer is to name them; an arm that drops in silence turns every future format extension into an invisible data loss |
| An export is idempotent | writing in map order | it is stable within one run and different across builds. A user who exports twice and gets two different files cannot tell a real change from a serialization detail, and neither can a test; fixing the order of every attribute is what makes the round trip provable at all |
| The comments and the suggestions are opt-in on the way out | exporting them always | it publishes the review with the document and is discovered by the reader on the other side. A review is internal by default: a document sent to a client should not carry who doubted which sentence, so the safe direction is the one that needs an explicit act to publish |
| A report accompanies a conversion that finished, and an error replaces one that did not | one error type | it makes "the file opened with three warnings" and "the file did not open" the same value. The two are different facts and a caller acts on them differently: one is shown beside a document, the other instead of it; merging them forces every caller to ask which case it is holding |
| A progress event past one megabyte | reporting progress always | it spends an event per small import. A conversion that finishes in ten milliseconds needs no progress bar, and one that takes ten seconds with no sign of life reads as a freeze; the threshold is where the first becomes the second |
| A progress event past one megabyte | reporting progress never | it is the freeze. A conversion that finishes in ten milliseconds needs no progress bar, and one that takes ten seconds with no sign of life reads as a freeze; the threshold is where the first becomes the second |
| Word and the clipboard are the two Google Docs paths | calling the Google API from the client | it trades a metadata leak for a convenience the download already gives. There is no way to read a Google document from the client without an authenticated network call to Google, which leaks which document is opened and when; a download and a paste both arrive as formats already read |
| The HTML reader is the paste reader | a paste-specific reader | it is a second HTML implementation with a second set of losses. A paste from a web page and an import of that page are the same bytes in the same format, so two readers would be two results for one input, and the second one to drift would be the one nobody tested |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Only the native file has a gesture.** The palette saves it and the home screen opens it. No
  gesture imports or exports a foreign format (`PROSE-IO-027`), and `ImportDocument` is not on the
  plane yet.
* **Where an import lands is not ruled.** A failed import leaves the document untouched
  (`PROSE-IO-011`). Nothing says what a successful one does to a document that already has
  content: replace it, append to it, or insert at the caret. Toile answers the same question with
  `TOILE-IO-023` and Prose states nothing, so the batch of step 5 has no defined destination.
* **A failed conversion has no way back to the user.** `PROSE-IO-047` makes the report and the
  error two answers, and the bus carries one of them. `ImportCompleted` holds a report, and no
  event holds an `InteropError`. A file that does not open therefore has a typed refusal and no
  declared path to the screen that must show it.
* **An export has no size to measure its progress against.** `PROSE-IO-057` owes a progress event
  and the threshold is one megabyte of file. An import knows that before it starts, and an export
  learns it only when it has finished. Nothing states what an export counts instead.
* **A named style's character defaults have no home in the document.** The intermediate form
  carries them, because a Word style carries them, and the style register of
  `ambre-spec-prose-schema.md section 3` holds paragraph fields alone. A Word file whose "Quote"
  style is italic therefore imports as a style that is not italic, and no warning covers it. The
  loss is in the command builder, not in the reader.
* **A comment's author survives an export and not a rename.** The document stamps a display name
  and a document peer, and the Word format carries an author string. The round trip is faithful
  to what Prose stores. It stores no stable user identity, which
  `features/ambre-spec-prose-comments.md section 6` records, and an export freezes whatever name
  the author had that day.
* **A cross reference crosses formats as a bookmark and comes back by name.** The document holds a
  block identifier, and Word holds a generated bookmark. Nothing states how the two are joined on a
  round trip through a file that a third party edited in between.
* **Nothing bounds an import.** Toile clamps foreign geometry, bounds the element count and bounds
  the nesting depth (`TOILE-IO-002`); Prose bounds nothing. A file with a million blocks, or a list
  nested ten thousand deep, has no stated refusal. The atomicity rule means the whole batch is
  held in memory before any of it is applied.
* **Three applications now import and export, and the vocabulary is theirs alone.** Trame reports
  an import through one type and one event name, Toile through another, and Prose through a third.
  The report, the progress event and the completion event differ in shape and in spelling for one
  behaviour. The formats share nothing and belong to each application; the report a user reads does
  not. No rule holds the three together, and no document owns the question.
* **The four reader crates are a plan.** `crates/ambre-prose` declares none of them, so the
  dependency table above marks each one absent rather than reading as a description of the
  manifest.
