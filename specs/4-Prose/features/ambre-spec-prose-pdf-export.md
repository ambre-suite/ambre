# AMBRE -- FEATURE: PROSE PDF EXPORT

**Type:** Vertical (feature)

> Prose's half of the shared PDF export (`ambre-spec-pdf-export.md`): the pagination the file
> reuses, the links it carries, and the language it declares.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-017`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-029` | `IO` | A PDF page MUST carry the pagination the screen shows. | this spec section 3; `ambre-spec-prose-schema.md` (`PROSE-032`) | `ambre_prose::logic::print::tests::a_pdf_page_carries_the_pages_the_screen_shows` -- the file holds the layout's page count at the document's paper size in points, and each page reads back the paragraphs the layout starts on it and no other | none | verified |
| `PROSE-IO-031` | `IO` | Every link the document carries MUST reach the file as a live link: the table of contents, a cross reference and an external address. | this spec section 3 | `ambre_prose::logic::print::tests::every_link_the_document_carries_reaches_the_file_live` -- read back with `lopdf`, a table-of-contents entry and a cross reference each lead to the page of their heading, and the one address a reader may open is the file's one address | none | verified |
| `PROSE-IO-032` | `IO` | The language the file carries MUST come from the request, or from the document when the request names none. | this spec section 3 | `ambre_prose::logic::print::tests::the_file_takes_its_language_from_the_request_then_the_document` -- the catalog's `/Lang` reads the request's tag, then the document's once the request names none | none | verified |
| `PROSE-IO-035` | `INV` | The writer MUST NOT carry a locale of its own: when neither the request nor the document names a language, the file MUST carry none. | this spec section 3 | `ambre_prose::logic::print::tests::a_file_with_no_language_named_declares_none`; `ambre_pdf::document::tests::the_file_carries_the_language_it_is_given_and_none_of_its_own` -- the writer alone, given an empty meta, writes no `/Lang` and no information dictionary | none | verified |
| `PROSE-IO-103` | `INV` | Each slice of a PDF export MUST hold the `Prose PDF export, one slice` row, for a page near the view and for a page far from it. | this spec section 3; `ambre-spec-budgets.md section 1`; `PROSE-IO-048` | `callgrind` -- the `print_slice` operation of `crates/ambre-prose/benches/instructions.rs`, each slice of a page near the view and of a page past the middle of a file of 2 000 paragraphs counted apart, the dearest under its ceiling in the CI `instructions` job; `ambre_prose::logic::print::tests::a_far_page_is_shaped_over_slices_then_drawn_alone` -- a slice shapes within its room or one block alone, and the slice that draws a page shapes nothing | none | verified |

### 1.2 Export the document as a PDF

`ambre-spec-pdf-export.md section 1.1` rules the shared half. A page is drawn by the screen's own
walk, is pure vector, keeps its characters, and reaches the browser off the UI thread. What a
paginated document adds is the layout it must not compute twice.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-IO-033` | `POST` | A user MUST be able to export the document as a PDF. | this spec section 4 | `ambre_prose::commands::tests::the_save_entry_is_declared_once` -- the entry is declared once, in the File group; `ambre_belt::prose::tests::a_pdf_export_reaches_the_download_directory` -- browser tier: the File menu's entry lands a PDF in the download directory | none | verified |
| `PROSE-IO-037` | `POST` | An export MUST write the pages the engine already laid out, never laying the document out again. | this spec section 3 | structural -- `ambre_prose::layout::reflow::Paginator::print_reshape` shapes the dropped runs of a page and `Paginator::print_window` cuts the layout in force, and neither calls a placement; `ambre_prose::logic::print::tests::an_export_lays_nothing_out_again_and_leaves_the_view_alone` -- the placement of every block reads the same after the export | none | verified |
| `PROSE-IO-038` | `POST` | An export MUST write the document as it stood when the export began; an edit made while it runs MUST NOT reach the file in part. | this spec section 3 | `ambre_prose::logic::print::tests::an_edit_during_an_export_does_not_reach_the_file` -- a text typed after the first slice lands in the document and is absent from the file, which holds every paragraph of the document as it stood | none | verified |
| `PROSE-IO-039` | `FRAME` | An export MUST NOT change what the screen shows: not the page the reader is on, not the layout, and not the selection. | derived | structural -- the export prints from a `ProseUiState` of its own, so the view's selection and page are out of its reach; `ambre_prose::logic::print::tests::an_export_lays_nothing_out_again_and_leaves_the_view_alone` -- the Logic thread has no new layout to hand the view after the export, and holds the same runs | none | verified |
| `PROSE-IO-104` | `POST` | A PDF export taken one slice per pass MUST write the file that the same export taken whole writes. | this spec section 3; `PDF-011`; `PROSE-IO-100` | `ambre_prose::logic::print::tests::an_export_in_slices_writes_the_file_a_whole_export_writes` -- the same bytes taken in slices, taken whole, and landed whole one slice into a page far from the view | none | verified |

* **Budget.** the PDF export of a hundred pages, without and with pictures, one row of
  `ambre-spec-budgets.md section 1`, measured by no bench. One slice of the walk is the row
  `Prose PDF export, one slice` of the same section, held by a count of instructions
  (`PROSE-IO-103`). The shared writer's weight is `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-017`, `PDF-015`). A link a reader may not open is no link in the
  file (`PROSE-UI-153`).
* **Verification.** 10 `verified`. The shared rules and their coverage are
  `ambre-spec-pdf-export.md`; what is counted here is the pagination, the links, the language and
  the export's own run. `PROSE-IO-037` and `PROSE-IO-039` lean on a structure as much as on a test.
  No reading of the file tells a second pagination that agrees from the first.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Prose's own turn on the page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports to PDF | exports the same document, untouched | nothing: an export writes nothing | the same pages, and the same breaks | `logic::print::tests::two_peers_holding_the_same_faces_export_one_file` |
| widens the margins | exports | one page format, converged | Bob's file follows Alice's margins | none -- `PROSE-IO-029` |
| exports while typing | types | the typing alone | a file holding the document as the export began | `logic::print::tests::an_edit_during_an_export_does_not_reach_the_file` |
| exports a document that holds a picture | watches | the picture's block | the picture in its room, at full resolution, or a placeholder and a line in the report when its bytes did not come back | `ambre_prose::logic::print::tests::a_printed_page_draws_a_fetched_picture` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the page format, the block tree and the layout the
pagination produced, and writes nothing (`PDF-017`).

**The entry point.** The request is one more export format, `ExportFormat::Pdf`. It runs as every
Prose export runs, one slice per pass of the Logic thread (`PROSE-IO-048`). Its step is one page,
not a run of blocks, and a page takes one slice or more (`PROSE-IO-103`).

```rust
pub struct PdfExport {
    /// A request that names none takes the document's own (`PROSE-IO-032`).
    pub language: DocumentLocale,
}
```

**Steps.** The shared flow owns the draw and the assembly (`ambre-spec-pdf-export.md section 3`).

1. refuse when the layout names a script it had no face for: its page numbers are not ones to hand
   out (`PROSE-112`);
2. take the pages the layout already produced, in order, at the document's paper size in points
   (`PROSE-IO-037`, `PROSE-IO-029`);
3. shape again the runs the paginator dropped far from the view, for the next page. A slice shapes
   at most 500 characters, or one longer block alone, and the job holds the runs until the page is
   drawn. The runs go to the print alone, so the paginator keeps holding what the view needs
   (`PROSE-136`, `PROSE-IO-039`). A block the page breaks keeps its run for the page after;
4. once no run of the page is left to shape, cut the layout to that page. The next slice draws it
   through the screen's resolve and walk, onto a recorded page, and shapes nothing (`PDF-009`,
   `PROSE-IO-103`);
5. add the links of the page: each address a reader may open, each cross reference, and each line
   of a table of contents (`PROSE-IO-031`). Each is cut to the part of its block the page shows;
6. hand the recorded pages, the title and the language to the PDF module, which writes the file
   (`PROSE-IO-032`).

**The pagination is not recomputed.** Step 2 reads the layout the engine already holds, which is
what makes the file match the screen page for page (`PROSE-IO-037`).

**The export holds a still document.** A command that writes the document lands the running export
first, and a remote delta settles it first (`PROSE-IO-100`, `ARCH-031`). No slice therefore reads a
document another slice did not. So the runs a slice shaped still match the document when a later
slice draws them, and a landing finishes the page from them (`PROSE-IO-104`). A face that arrives
mid-export moves no page. The export starts only on a layout that misses no script, so the new face
covers nothing the layout shaped.

**The print has its own view.** The export builds a view state of its own on the Logic thread. It
has no caret, no selection and no peer, and it prints black text on white paper. The
view the reader looks at is out of its reach (`PROSE-IO-039`). The pending suggestions are marked as
the screen marks them, an insertion underlined and a deletion struck through. A comment is not
marked: it is a note about the text, and not part of it.

**The language is a parameter, then the document, then nothing** (`PROSE-IO-032`,
`PROSE-IO-035`). A fixed locale in a writer is wrong for most readers and silent for all of them.
The resolution is `ambre_client_core::locale::resolve`, which every consumer of a locale calls
(`LOCALE-011`).

**What the format carries:**

| What | On the page |
|---|---|
| text | vector glyphs, shaped by the same shaper the screen uses (`PDF-009`) |
| an address, a cross reference, a table-of-contents entry | a live link (`PROSE-IO-031`) |
| a pending suggestion | the mark the screen draws over it |
| a picture | its stored file in the room the layout gives it, embedded whole for a JPEG and decoded in the PDF module for a PNG (`PROSE-IO-107`); a placeholder and a line in the report when its bytes did not come back (`PROSE-IO-008`) |
| a header, a footer, a footnote | drawn in the band the layout placed it in |
| the title, the language | file metadata (`PROSE-IO-032`). Prose stores no author, so the file names none |
| the tagging an accessible file needs | nothing yet: a post-V1 surface with no design |

**Code:**

* `crates/ambre-prose/src/logic/print.rs` -- the export's walk over the pages, its links and its
  metadata;
* `crates/ambre-prose/src/view/print.rs` -- one page drawn by the screen's resolve and walk, and the
  area each link covers;
* `crates/ambre-prose/src/layout/reflow.rs` -- `Paginator::print_dropped` and `print_reshape`, the
  runs a page lacks and their shaping, and `Paginator::print_window`, the cut of the layout in force
  a slice prints.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| `Export to PDF (.pdf)`, in the File group | the Logic thread waits on the PDF module, then walks the pages, one slice per pass (`PROSE-IO-033`) |
| the answer | the view hands the bytes to the browser; the document does not change (`PDF-015`) |
| the report | the pictures whose bytes did not come back |

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The export reuses the layout, and never recomputes it | laying the document out again at export time | it makes the file and the screen disagree by construction, and it is a second implementation of the hardest algorithm in the application. The layout is the document's own pagination, and it is what the reader is looking at |
| The language comes from the caller or the document | defaulting to one language | it is wrong for every reader who does not share that locale, and nothing in the file says it is wrong |
| A missing picture becomes a placeholder | waiting for every picture before writing anything, the export held until the network returns | it makes an offline export hang instead of finishing. An export fetches its pictures before its walk and goes on with what came back, while the report carries the loss (`features/ambre-spec-prose-images.md section 3`) |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- lazy loading unbuilt, no benchmark of a whole export -- are in
`ambre-spec-pdf-export.md section 6`. Prose's own:

* **The export asks nothing.** The File entry exports with the document's language. No dialog
  collects a language, a page range or a paper size; the persisted page setup is its own Story.
* **A file without the review has no design.** The page carries the suggestions as the screen marks
  them. Leaving them out takes a layout of its own, which `PROSE-IO-037` refuses, so no rule covers
  it.
* **An accessible file has no design.** A tagged PDF carries the reading order, the heading levels
  and the alternative text a screen reader needs, and the document already holds all three. It is
  named as post-V1 and no rule covers it, so a file exported today is as opaque to a screen reader
  as a scan.
* **Nothing bounds the export.** A thousand-page document has a budget row and no refusal. Nothing
  caps the bytes held in memory while the pages are recorded, and the module writes them whole.
* **The writer's lazy loading is a claim, not a mechanism.** `ambre-spec-architecture.md
  section 5.E` bills the writer as lazily loaded on the first export. Splitting a module out of the
  client bundle is a build-system slice nobody has scheduled, and Toile records the same gap for
  the same crate.
