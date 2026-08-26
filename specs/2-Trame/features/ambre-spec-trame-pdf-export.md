# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the surface-generic render walk ships; nothing drives it onto a page.

> A user prints a sheet to a file anyone can open, and the page looks like the screen. The grid is
> paginated, and every glyph and rule stays a vector.

The export is not a conversion of the document: it is the screen renderer aimed at another surface.
That is what makes the page match the screen, and what makes the pagination the only new work.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-IO-021` | One render walk MUST serve both the screen and the page: the drawing primitives MUST hold no screen-only state. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level: every primitive under `crates/ambre-trame/src/render/` is generic over the surface, and the module uses no GPU type and no scroll state; `ambre_trame::render::tests::draw_cells_on_fills_each_and_counts_nonempty_text`, `::draw_grid_lines_on_strokes_with_grid_color`, `::col_headers_draw_bg_separator_and_active_tint` drive them through a recording surface that is neither screen nor page | verified |
| `TRAME-IO-020` | The export MUST run in the Logic thread, through the shared PDF surface, never through a surface an application owns. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1); `ambre-trame` does not declare the shared PDF crate | unimplemented |
| `TRAME-UI-022` | The page MUST be drawn by the same code as the screen; the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 3; section 4 | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-UI-023` | The export MUST be pure vector: no rasterisation of text, rules or shapes. | this spec section 5 | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-022` | The page format MUST be read from the sheet's persisted setup; a one-off override MUST NOT write to it. | this spec section 2; section 3 | not built -- lands with the "Persisted page setup" Story (`ambre-roadmap.md` E6); the container is declared and no code writes it | unimplemented |
| `TRAME-IO-023` | Text too wide for its cell MUST be cut at the cell edge, with no ellipsis, exactly as on screen. | this spec section 3 | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1) | unimplemented |
| `TRAME-IO-024` | Two exports of one unchanged sheet MUST produce identical bytes. | this spec section 3 | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1); `TOILE-IO-005` holds the same rule over the shipped Toile path | unimplemented |

* **Budget.** none: `ambre-spec-budgets.md` budgets no export to PDF, for either
  application that draws through the shared surface. The page walk over a large sheet is unmeasured
  and stays so until the fixture corpus exists (`ambre-spec-fixtures.md`).
* **Boundary.** the finished file reaches the browser and nothing else. `TRAME-IO-020` keeps the
  export in the Logic thread through the shared surface, and `TRAME-UI-022` limits the UI thread to
  handing the file over.
* **Verification.** 1 `verified`, 0 `unverified`, 6 `unimplemented`. The one verified row is the
  one that matters most for the others: the render walk is already surface-generic and tested
  through a surface that is not the screen, so what is missing is a driver, a pagination and a page
  format -- not a second renderer. Toile exports through that same shared surface today, which is
  why the six rules below are a design and not a hypothesis.

---

## 2. WHAT EACH USER SEES

The page format is a persisted document value, so two peers export the same sheet the same way.
The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the paper size to A3 | exports the sheet | one page format, converged | Bob's file is A3: he did not have to be told (`TRAME-IO-022`) | none -- unbuilt |
| overrides the orientation for one export | reads the page setup | the stored format, unchanged | Alice's file is landscape; Bob's setup is what it was (`TRAME-IO-022`) | none -- unbuilt |
| exports a sheet | exports the same sheet, untouched | nothing: an export writes nothing | the same bytes, twice (`TRAME-IO-024`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** `page_setup_{sheet}` -- the paper size, the orientation, the margins, the scale and
the print flags of one sheet (`ambre-spec-trame-schema.md section 3`, declared and unwritten). The
export owns no other state: it reads the document and produces bytes.

**Commands and events** (`ambre-bus`, neither on the bus today):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::ExportPdf { sheet_id, page_setup }` | UI to Logic | the sheet, and an optional one-off format that overrides the stored one |
| `TrameEvent::ExportComplete { filename, data }` | Logic to UI | the bytes, on the same path a CSV export uses |

**The page format:**

```rust
pub struct PdfPageSetup {
    pub orientation:     PdfOrientation,
    pub paper_size:      PdfPaperSize,
    pub margins:         PdfMargins,   // top, right, bottom, left, in mm
    pub scale:           f32,          // 1.0 is 100 %
    pub fit_to_page:     bool,         // scale down until the sheet fits one page
    pub print_gridlines: bool,
    pub print_headers:   bool,         // the row numbers and the column letters
}

pub enum PdfOrientation { Portrait, Landscape }
pub enum PdfPaperSize { A4, A3, Letter, Legal }
```

**Steps, on an export:**

1. resolve the format: the one-off override when the command carries one, the sheet's stored setup
   otherwise (`TRAME-IO-022`);
2. compute the printable area from the paper size, the orientation and the margins;
3. walk the used extent of the sheet, cutting it into pages that fit that area;
4. build one grid layout per page, with no scroll offset and no band above the header strip;
5. draw each page through the shared PDF surface, calling the same primitives the screen calls
   (`TRAME-IO-021`, `TRAME-UI-022`);
6. assemble the pages into one file and answer with its bytes;
7. the UI thread hands the bytes to the browser, and changes nothing on the canvas.

Step 4 is why the walk needs no PDF-specific code: the layout type the screen builds already takes
the offset and the reserved band as fields, and a page sets both to zero.

**Code:**

* `crates/ambre-trame/src/render/` -- the surface-generic primitives and the grid layout, built;
* `crates/ambre-trame/src/ui/renderer/` -- the screen caller, built;
* the Logic-thread driver -- the pagination, the command handler and the page assembly -- is not
  written. Toile's equivalent is `crates/ambre-toile/src/logic/interop/pdf_export.rs`.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| an export entry | asks the Logic thread for the active sheet, as a CSV export does. No entry exists today |
| the answer | the UI thread wraps the bytes in a browser download (`TRAME-UI-022`) |
| a page setup dialog | edits the stored format for the sheet. It is not built, and nothing else writes the container |

The user sees no progress: a command runs to completion, so a long export is one wait with no
report (`features/ambre-spec-trame-import-export.md section 6`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The page is drawn in the Logic thread | a hundred sheets can take more than half a second, and that thread may take the time it needs. The UI thread owes the user a frame every 16 ms | drawing on the UI thread, next to the GPU surface, where the fonts and the layout already are |
| The PDF surface belongs to no application | it translates drawing primitives into a page and knows nothing about grids, boards or paragraphs. Three applications need it, so an application owning it would make the other two depend on it | a PDF module inside `ambre-trame`, which is where the design first put it |
| The page format is persisted, not a parameter | two peers exporting one sheet must produce one layout. A format that travels with the command is a per-user setting that nobody can see or agree on | passing the format with the export command |
| An override does not write the container | printing one copy in landscape is not a decision about the document, and it must not reach the other peers | writing the override, which makes every trial print a shared edit |
| The export is pure vector | a PDF of rasterized pixels is a screenshot: it does not zoom, its text is not searchable, and its size grows with the screen | reading back the rendered pixels and embedding them, which is the shortest path to a file |
| Wide text is cut, never elided | the page must match the screen, and the screen cuts at the cell edge. An ellipsis on the page alone would break the one guarantee this feature exists to keep | adding an ellipsis on the page, which reads better and matches nothing |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing exports a PDF in Trame.** Six of the seven rows are unbuilt: there is no driver, no
  pagination, no command on the bus and no menu entry. What exists is the seam they will use.
* **Toile exports and Trame does not, through the same surface.** Its export is verified, byte
  stability included, and the difference between the two is the pagination: a board yields one page
  per frame, and a grid must be cut into pages nobody has specified.
* **Nothing rules what a page break may cut.** A row taller than the printable area, a merged block
  that straddles the edge, and a column wider than the paper have no rule. Repeating the header
  strip on each page has none either.
* **The stored page format has no writer and no reader.** The container is declared in the schema
  and no code touches it, so the rule that two peers get one layout cannot fail yet.
* **Text in the page is not selectable.** The shared surface emits glyph shapes with no mapping back
  to characters, so a reader can see the text and not copy it (`ambre-spec-drawing-surface.md section 2`).
  It is a property of the surface, and every application inherits it.
* **The fonts are seeded on the Logic thread.** Shaping needs a font database, and the thread that
  draws the page has no GPU side to borrow one from. The seeding ships; nothing measures what it
  costs at startup.
