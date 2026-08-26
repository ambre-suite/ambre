# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** design-only

> A user hands the document to someone who has no Ambre: the same pages, in the same order, with
> the links still working and the words still selectable.

The export is not a conversion of the document: it is the page renderer aimed at another surface.
That is what makes the file match the screen. The pages it walks are
`features/ambre-spec-prose-pagination.md`, what a page looks like is
`features/ambre-spec-prose-page-rendering.md`, and the other formats are
`features/ambre-spec-prose-import-export.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-IO-015` | A PDF export MUST draw through the same surface the screen draws through; the page and the screen MUST NOT be two renderers. | this spec section 3; `features/ambre-spec-prose-page-rendering.md` (`PROSE-UI-018`) | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-029` | A PDF page MUST carry the pagination the screen shows: the export MUST NOT lay the document out under another page format. | this spec section 3; `ambre-spec-prose-schema.md` (`PROSE-032`) | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-030` | A word on a page MUST stay a word: the file MUST carry the mapping back to characters, not glyph shapes alone. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-031` | Every link the document carries MUST reach the file as a live link: the table of contents, a cross reference and an external address. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-032` | The exported document's language MUST come from the request or from the document; it MUST NOT be a fixed locale. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-033` | A user MUST be able to export the document as a PDF. | this spec section 4 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the PDF export of a hundred pages, without and with pictures, one row of
  `ambre-spec-budgets.md section 1`. The writer's own weight is a row of `ambre-spec-budgets.md section 5`,
  billed once to the shared crate because Prose is its first consumer. Both read **none exists**.
* **Boundary.** the finished file reaches the browser and nothing else. The export runs in the
  Logic thread, opens no socket and reads no key, and a picture it cannot find becomes a
  placeholder rather than a fetch (`features/ambre-spec-prose-images.md`, `PROSE-IO-008`).
* **Verification.** 6 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`.
  `PROSE-IO-015` is the one a compiler can hold before a test exists, and it is the export half of
  `PROSE-UI-018`: one draw path parameterized on the surface is a type-level fact, two are two
  functions. `PROSE-IO-029` and `PROSE-IO-030` are the two a host test reaches from a fixture
  document, with no GPU and no picture.

---

## 2. WHAT EACH USER SEES

The page format is part of the document, so two peers export the same pages. An export writes
nothing. No test resolves yet: the last column names the row each scenario rests on.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports to PDF | exports the same document, untouched | nothing: an export writes nothing | the same pages, and the same breaks | none -- `PROSE-IO-029` |
| widens the margins | exports | one page format, converged | Bob's file follows Alice's margins | none -- `PROSE-IO-029` |
| exports while typing | types | the typing alone | a file holding the document as the export began | none -- `PROSE-IO-017` |
| exports a document whose picture has not arrived | watches | the picture's address | a placeholder in the file, and a line in the report | none -- `PROSE-IO-008` |

---

## 3. MODEL

**Containers.** This feature owns none. It reads the page format, the block tree and the layout the
pagination produced, and writes nothing (`PROSE-IO-017`).

**One draw path, two surfaces.** The drawing surface is Foundation's, declared in
`ambre-spec-drawing-surface.md section 2`, and Prose defines no surface of its own. The screen
implementation is the canvas one, on the UI thread; the page implementation lives in the shared PDF
crate and is app-agnostic, because it translates surface primitives and knows nothing about a
document. What is shared is every primitive and every per-block helper. What is not shared is the
entry point: the screen driver takes the view state and a dirty strip, and the page driver takes
the intermediate form and a page layout, because one is live and repaints in parts while the other
is frozen and writes a whole page (`features/ambre-spec-prose-page-rendering.md`).

```rust
pub fn export_pdf(
    ir:      &ProseIr,
    layout:  &PageLayout,
    /// Shaping needs a mutable context; it is threaded to the page driver, never to the surface.
    fonts:   &mut FontContext,
    options: &ExportOptions,
) -> Result<(Vec<u8>, ConversionReport), InteropError> { /* ... */ }
```

**Steps, per page:**

1. take the pages the layout already produced, in order, under the document's own page format
   (`PROSE-IO-029`);
2. open a page of that size in points, converting from the millimetres the format stores;
3. draw the block runs, the header, the footer and the footnote band through the shared surface
   (`PROSE-IO-015`);
4. write each glyph run with the character range it came from, so the page carries text and not
   only shapes (`PROSE-IO-030`);
5. add the links: the table of contents entries, the cross references, and the external addresses
   the runs carry (`PROSE-IO-031`);
6. write the metadata -- the title, the language and the author -- and close the file
   (`PROSE-IO-032`).

**The pagination is not recomputed.** Step 1 reads the layout the engine already holds, which is
what makes the file match the screen page for page. An export that laid the document out again
under a page size of its own would be a second pagination, and the first divergence between the two
would be invisible until a reader compared a printed page with a screen.

**The language is a parameter, then the document, then nothing.** A fixed locale in a writer is a
default that is wrong for most readers and silent for all of them, and the metadata is what a
screen reader and an archive read to decide how to speak the file (`PROSE-IO-032`).

**A picture is read from the cache, never fetched** (`features/ambre-spec-prose-images.md`,
`PROSE-IO-007`). A picture that has not arrived becomes a placeholder and a line in the report, and
the export finishes (`PROSE-IO-008`).

**What the format carries:**

| What | On the page |
|---|---|
| text | vector glyphs, shaped by the same shaper the screen uses |
| a link, a cross reference, a table-of-contents entry | a live link (`PROSE-IO-031`) |
| a picture | its own pixels, in the raster formats the client decodes |
| a header, a footer, a footnote | drawn in the band the layout placed it in |
| the title, the language, the author | file metadata |
| a form field | nothing: out of scope |
| the tagging an accessible file needs | nothing yet: it is a post-V1 surface with no design |

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 4. SURFACE

**There is none** (`PROSE-IO-033`). No entry of Prose's chrome asks for a file, and nothing hands
bytes to the browser (`features/ambre-spec-prose-import-export.md section 4`).

| Trigger, once it exists | What must happen |
|---|---|
| the export entry | a dialog collects the options, then the Logic thread walks and writes |
| the answer | the view hands the bytes to the browser; the document does not change |
| the report | the pictures that were missing, and anything the writer approximated |

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The page is drawn by the screen's own draw path | a second renderer drifts from the first the day either one changes, and the promise of this feature is that the file matches the screen. Parameterizing on the surface is what makes one path serve both | writing a PDF-specific renderer, which duplicates every block type and every mark |
| The export runs in the Logic thread | a hundred pages can take seconds, and the UI thread owes the visible frame its sixteen milliseconds. The shaper the export needs is already there for the pagination | exporting on the UI thread, which freezes the editor for the length of the file |
| The export reuses the layout, and never recomputes it | the layout is the document's own pagination, and it is what the reader is looking at. Recomputing it is a second implementation of the hardest algorithm in the application | laying the document out again at export time, which makes the file and the screen disagree by construction |
| The output is vector | a page printed at any size stays sharp, and the shared surface already emits paths and glyphs, so the vector path costs nothing extra | rasterizing each page, which trades every advantage of the format for a simpler writer |
| Each glyph run carries its source range | a page that looks right and cannot be searched, selected or read aloud is a picture of a document. The range is the only thing that makes the characters recoverable | writing glyphs by index alone, which is what a naive writer does and what makes a file unsearchable |
| The writer is a shared crate, not an application crate | it translates surface primitives into a file and reads nothing of a document, so a second application gets it for free and its weight is billed once | putting the writer in the Prose crate, which makes the next application copy it or depend on Prose |
| The language comes from the caller or the document | a hardcoded locale is wrong for every reader who does not share it, and nothing in the file says it is wrong | defaulting to one language, which is the failure this rule exists to name |
| A missing picture becomes a placeholder | an export is a gesture with an end, and waiting on a fetch inside it turns a five-second action into an unbounded one. The report carries the loss | fetching the missing pictures inside the export, which makes an offline export hang instead of finishing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing asks for a file** (`PROSE-IO-033`). Prose declares no menu that opens or saves
  anything, so this feature, like the four other formats, is specified from the second step onward.
* **The shared unit is named by neither document.** `PROSE-IO-015` and `PROSE-UI-018` both say the
  drawing is written once, and the only entry point either document declares is the screen's
  display-list builder. What the two paths actually share is a per-block helper that no document
  declares, so the rule is stated twice and the thing it constrains is written down nowhere.
* **An accessible file has no design.** A tagged PDF carries the reading order, the heading levels
  and the alternative text a screen reader needs, and the document already holds all three. It is
  named as post-V1 and no rule covers it, so a file exported today is as opaque to a screen reader
  as a scan.
* **Nothing bounds the export.** A thousand-page document with a picture on every page has a
  budget row and no refusal, no progress event of its own beyond the shared one, and no cap on the
  bytes held in memory while the file is assembled.
* **The writer's lazy loading is a claim, not a mechanism.** `ambre-spec-architecture.md
  section 5.E` bills the writer as lazily loaded on the first export. Splitting a module out of the
  client bundle is a build-system slice nobody has scheduled, and Toile records the same gap for
  the same crate.
* **No benchmark measures it.** The two figures of the budget row have no bench behind them, so
  nothing goes red when the export gets slower.
