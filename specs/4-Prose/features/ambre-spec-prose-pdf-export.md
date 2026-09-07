# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** design-only

> Prose's half of the shared PDF export (`ambre-spec-pdf-export.md`): the pagination the file
> reuses, the links it carries, and the language it declares.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-006`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-IO-029` | `IO` | A PDF page MUST carry the pagination the screen shows. | this spec section 3; `ambre-spec-prose-schema.md` (`PROSE-032`) | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-031` | `IO` | Every link the document carries MUST reach the file as a live link: the table of contents, a cross reference and an external address. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-032` | `IO` | The language the file carries MUST come from the request, or from the document when the request names none. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-035` | `INV` | The writer MUST NOT carry a locale of its own: when neither the request nor the document names a language, the file MUST carry none. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Export the document as a PDF

That a page is drawn by the screen's own walk, is pure vector, keeps its characters and reaches the
browser off the UI thread is `ambre-spec-pdf-export.md section 1.1`. What a paginated document adds
is the layout it must not compute twice.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-IO-033` | `POST` | A user MUST be able to export the document as a PDF. | this spec section 4 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-037` | `POST` | An export MUST write the pages the engine already laid out, and MUST NOT lay the document out again. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-038` | `POST` | An export MUST write the document as it stood when the export began; an edit made while it runs MUST NOT reach the file in part. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-039` | `FRAME` | An export MUST NOT change what the screen shows: not the page the reader is on, not the layout, and not the selection. | derived | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the PDF export of a hundred pages, without and with pictures, one row of
  `ambre-spec-budgets.md section 1`, reading **none exists**. The shared writer's weight is
  `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`); a picture it cannot find becomes a placeholder
  rather than a fetch (`features/ambre-spec-prose-images.md`, `PROSE-IO-008`).
* **Verification.** 8 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`. The shared
  rules and their coverage are `ambre-spec-pdf-export.md`; what is counted here is the pagination,
  the links and the language. `PROSE-IO-029` and `PROSE-IO-031` are the two a host test reaches from
  a fixture document, with no GPU and no picture, and `PROSE-IO-037` is the one no reading of the
  file can tell from a second pagination that happens to agree.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Prose's own turn on the page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports to PDF | exports the same document, untouched | nothing: an export writes nothing | the same pages, and the same breaks | none -- `PROSE-IO-037` |
| widens the margins | exports | one page format, converged | Bob's file follows Alice's margins | none -- `PROSE-IO-029` |
| exports while typing | types | the typing alone | a file holding the document as the export began | none -- `PROSE-IO-038` |
| exports a document whose picture has not arrived | watches | the picture's address | a placeholder in the file, and a line in the report | none -- `PROSE-IO-008` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the page format, the block tree and the layout the
pagination produced, and writes nothing (`PROSE-IO-017`).

**The entry point.** The shared flow draws and assembles the file
(`ambre-spec-pdf-export.md section 3`); Prose's driver takes the intermediate form and the page
layout the pagination already produced, because the page is frozen and written whole, unlike the
screen's live partial repaint (`features/ambre-spec-prose-page-rendering.md`).

```rust
pub fn export_pdf(
    ir:      &ProseIr,
    layout:  &PageLayout,
    /// Shaping needs a mutable context; it is threaded to the page driver, never to the surface.
    fonts:   &mut FontContext,
    options: &ExportOptions,
) -> Result<(Vec<u8>, ConversionReport), InteropError> { /* ... */ }
```

**Steps, per page** (the shared flow owns the draw and the assembly):

1. take the pages the layout already produced, in order, under the document's own page format
   (`PROSE-IO-037`);
2. open a page of that size in points, converting from the millimetres the format stores;
3. add the links: the table of contents entries, the cross references, and the external addresses
   the runs carry (`PROSE-IO-031`);
4. write the metadata -- the title, the language and the author -- and close the file
   (`PROSE-IO-032`).

**The pagination is not recomputed.** Step 1 reads the layout the engine already holds, which is
what makes the file match the screen page for page. An export that laid the document out again
under a page size of its own would be a second pagination (`PROSE-IO-037`), and the first divergence between
the two would be invisible until a reader compared a printed page with a screen.

**The language is a parameter, then the document, then nothing.** A fixed locale in a writer is a
default that is wrong for most readers and silent for all of them, and the metadata is what a
screen reader and an archive read to decide how to speak the file (`PROSE-IO-032`,
`PROSE-IO-035`).
**A picture is read from the cache, never fetched** (`features/ambre-spec-prose-images.md`,
`PROSE-IO-007`). One that has not arrived becomes a placeholder and a line in the report, and the
export finishes (`PROSE-IO-008`).

**What the format carries:**

| What | On the page |
|---|---|
| text | vector glyphs, shaped by the same shaper the screen uses (`ambre-spec-pdf-export.md`, `PDF-001`) |
| a link, a cross reference, a table-of-contents entry | a live link (`PROSE-IO-031`) |
| a picture | its own pixels, in the raster formats the client decodes |
| a header, a footer, a footnote | drawn in the band the layout placed it in |
| the title, the language, the author | file metadata (`PROSE-IO-032`) |
| the tagging an accessible file needs | nothing yet: a post-V1 surface with no design |

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

**There is none** (`PROSE-IO-033`). No entry of Prose's chrome asks for a file, and nothing hands
bytes to the browser (`features/ambre-spec-prose-import-export.md section 4`).

| Trigger, once it exists | What must happen |
|---|---|
| the export entry | a dialog collects the options, then the Logic thread walks and writes |
| the answer | the view hands the bytes to the browser; the document does not change (`PDF-003`) |
| the report | the pictures that were missing, and anything the writer approximated |

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The export reuses the layout, and never recomputes it | laying the document out again at export time | it makes the file and the screen disagree by construction, and it is a second implementation of the hardest algorithm in the application. The layout is the document's own pagination, and it is what the reader is looking at |
| The language comes from the caller or the document | defaulting to one language | it is wrong for every reader who does not share that locale, and nothing in the file says it is wrong |
| A missing picture becomes a placeholder | fetching the missing pictures inside the export | it makes an offline export hang instead of finishing. An export is a gesture with an end, and waiting on a fetch inside it turns a five-second action into an unbounded one, while the report already carries the loss |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps -- no selectable text, lazy loading unbuilt, no benchmark -- are in
`ambre-spec-pdf-export.md section 6`. Prose's own:

* **Nothing asks for a file** (`PROSE-IO-033`). Prose declares no menu that opens or saves anything,
  so this feature, like the four other formats, is specified from the second step onward.
* **An accessible file has no design.** A tagged PDF carries the reading order, the heading levels
  and the alternative text a screen reader needs, and the document already holds all three. It is
  named as post-V1 and no rule covers it, so a file exported today is as opaque to a screen reader
  as a scan.
* **The links are the one thing the shared surface does not carry.** The table of contents, the
  cross references and the external addresses are a Prose payload over the page, and no other editor
  exports them, so the link-writing is this document's and not the shared flow's (`PROSE-IO-031`).
* **Nothing says how the export gets a still document.** `PROSE-IO-038` requires the file to hold
  the document as the export began, and a hundred pages take seconds during which remote deltas
  keep arriving. No document declares a snapshot, a read lock or a version the walk is pinned to,
  so the rule names the outcome and nothing names the mechanism.
* **Nothing bounds the export.** A thousand-page document with a picture on every page has a
  budget row and no refusal, no progress event of its own beyond the shared one, and no cap on the
  bytes held in memory while the file is assembled.
* **The writer's lazy loading is a claim, not a mechanism.** `ambre-spec-architecture.md
  section 5.E` bills the writer as lazily loaded on the first export. Splitting a module out of the
  client bundle is a build-system slice nobody has scheduled, and Toile records the same gap for
  the same crate.
* **No benchmark measures it.** The two figures of the budget row have no bench behind them, so
  nothing goes red when the export gets slower.