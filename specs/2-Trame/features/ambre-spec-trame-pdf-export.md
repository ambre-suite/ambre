# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the surface-generic render walk ships; nothing drives it onto a page.

> Trame's half of the shared PDF export (`ambre-spec-pdf-export.md`): the persisted page setup, and
> the pagination that cuts a grid into pages.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-006`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-021` | `INV` | The drawing primitives MUST hold no screen-only state. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the seven primitives under `crates/ambre-trame/src/render/` are generic over `DrawSurface`, and none names a scroll offset or a dirty rect; `ambre_trame::render::tests::draw_cells_on_fills_each_and_counts_nonempty_text`; `::draw_grid_lines_on_strokes_with_grid_color`; `::col_headers_draw_bg_separator_and_active_tint` -- three of them run through a recording surface that is neither screen nor page, and the `TextCache` they take is a shaping cache, not screen state | none | verified |
| `TRAME-IO-023` | `OBS` | Text too wide for its cell MUST be cut at the cell edge on the page, with no ellipsis, exactly as on screen. | this spec section 3; `features/ambre-spec-trame-cell-rendering.md section 4` | not built | unbuilt -- lands with the `Sheet PDF export` Story (`ambre-roadmap.md` post-V1). Nothing serves a page, so no test compares one with the screen | unimplemented |

### 1.2 Export a sheet to a page

That a page is drawn by the screen's own walk, is pure vector, is byte-stable and reaches the
browser off the UI thread is `ambre-spec-pdf-export.md section 1.1`. What a sheet adds is a page
format, and an override that must not become an edit.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-IO-022` | `POST` | An export MUST take its page format from the sheet's persisted setup, unless the gesture carries a one-off override. | this spec section 2; section 3 | not built | unbuilt -- lands with the `Persisted page setup` Story (`ambre-roadmap.md` E6). The container is declared and no command writes it | unimplemented |
| `TRAME-IO-031` | `FRAME` | An export MUST NOT change the document, the stored page setup of a one-off override included. | this spec section 2; section 5 | not built | unbuilt -- lands with the `Sheet PDF export` Story (`ambre-roadmap.md` post-V1). It is the row that keeps a trial print from becoming a shared edit | unimplemented |

* **Budget.** none: `ambre-spec-budgets.md` budgets no grid export, and the page walk over a large
  sheet is unmeasured until the fixture corpus exists (`ambre-spec-fixtures.md`). The shared
  writer's weight is `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 1 `verified`, 0 `unverified`, 3 `unimplemented`. One row holds, and it is the
  one the shared document does not reach: `PDF-001` binds the walk that draws a page, and
  `TRAME-IO-021` binds that walk's own primitives, which carry no screen state. The page setup has
  no writer, the pagination has no driver, and the shared rules are `ambre-spec-pdf-export.md`.
  Three of Trame's seven primitives run through a surface that is not the screen. The rest reach a
  page only when the driver lands, section 6.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Trame's own turn on the persisted page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the paper size to A3 | exports the sheet | one page format, converged | Bob's file is A3: he did not have to be told (`TRAME-IO-022`) | none -- unbuilt |
| overrides the orientation for one export | reads the page setup | the stored format, unchanged | Alice's file is landscape; Bob's setup is what it was (`TRAME-IO-031`) | none -- unbuilt |

---

## 3. MODEL

**Containers.** `page_setup_{sheet}` -- the paper size, the orientation, the margins, the scale and
the print flags of one sheet (`ambre-spec-trame-schema.md section 3`, declared and unwritten). The
export owns no other state.

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

**Steps, on an export** (the shared flow is `ambre-spec-pdf-export.md section 3`; Trame adds the
pagination, steps 1 to 4):

1. resolve the format: the one-off override when the command carries one, the sheet's stored setup
   otherwise (`TRAME-IO-022`);
2. compute the printable area from the paper size, the orientation and the margins;
3. walk the used extent of the sheet, cutting it into pages that fit that area;
4. build one grid layout per page, with no scroll offset and no band above the header strip;
5. the shared flow draws each page and assembles the file (`ambre-spec-pdf-export.md section 3`).

Step 4 needs no PDF-specific code: the screen's layout already takes the offset and the reserved band
as fields, and a page zeroes both.

**Code:**

* `crates/ambre-trame/src/render/` -- the surface-generic primitives and the grid layout, built;
* the Logic-thread driver -- the pagination, the command handler and the page assembly -- is not
  written. Toile's equivalent is `crates/ambre-toile/src/logic/interop/pdf_export.rs`.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| an export entry | asks the Logic thread for the active sheet, as a CSV export does. No entry exists today |
| the answer | the UI thread wraps the bytes in a browser download (`PDF-003`) |
| a page setup dialog | edits the stored format for the sheet. It is not built, and nothing else writes the container |

No progress reaches the user: a long export is one wait with no report
(`features/ambre-spec-trame-import-export.md section 6`).

---

## 5. REJECTED ALTERNATIVES

The shared decisions -- the page drawn by the screen's own walk, the export in the Logic thread, the
pure-vector file, the glyph runs that carry their source range -- are
`ambre-spec-pdf-export.md section 5`. Trame's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The PDF surface belongs to no application | a PDF module inside `ambre-trame` | it is where the design first put it, and it would make the other two applications depend on Trame. The surface translates drawing primitives into a page and knows nothing about a grid |
| The page format is persisted | passing the format with the export command | it is a per-user setting that nobody can see or agree on, and two peers exporting one sheet must produce one layout |
| An override does not write the container | writing the override | it makes every trial print a shared edit. Printing one copy in landscape is not a decision about the document, and it must not reach the other peers |
| Wide text is cut, never elided | adding an ellipsis on the page | it reads better and matches nothing: the page must match the screen, and the screen cuts at the cell edge. An ellipsis on the page alone would break the promise the export exists for |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Trame's own:

* **Nothing exports a PDF in Trame.** There is no driver, no pagination, no command on the bus and
  no menu entry. What exists is the shared seam they will use.
* **The seam is proven three ways out of seven.** The render walk is generic over the surface for
  all of its primitives, and only the cell bodies, the grid lines and the column headers are ever
  driven through a surface that is not the screen. The merge overlay and the cell borders reach a
  page for the first time when the driver lands, with no recording behind them.
* **Nothing says the PDF surface stays a lazy chunk.** Toile declares `ambre-pdf` behind an
  optional feature, so `krilla` is not in the Main fetch unless an export compiles it in
  (`ambre-spec-budgets.md section 5`). Trame declares nothing yet, and no rule here says which of
  the two shapes it takes.
* **Toile exports and Trame does not, through the same surface.** The difference between the two is
  the pagination: a board yields one page per frame, and a grid must be cut into pages nobody has
  specified.
* **Nothing rules what a page break may cut.** A row taller than the printable area, a merged block
  that straddles the edge, and a column wider than the paper have no rule. Repeating the header strip
  on each page has none either.
* **The stored page format has no writer and no reader.** The container is declared in the schema and
  no code touches it (`TRAME-IO-022`), so the rule that two peers get one layout cannot fail yet.
