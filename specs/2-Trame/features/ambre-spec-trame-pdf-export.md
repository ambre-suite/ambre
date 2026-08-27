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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-IO-022` | The page format MUST be read from the sheet's persisted setup; a one-off override MUST NOT write to it. | this spec section 2; section 3 | not built -- lands with the "Persisted page setup" Story (`ambre-roadmap.md` E6); the container is declared and no code writes it | unimplemented |
| `TRAME-IO-023` | Text too wide for its cell MUST be cut at the cell edge, with no ellipsis, exactly as on screen. | this spec section 3 | not built -- lands with the "Sheet PDF export" Story (`ambre-roadmap.md` post-V1) | unimplemented |

* **Budget.** none: `ambre-spec-budgets.md` budgets no grid export, and the page walk over a large
  sheet is unmeasured until the fixture corpus exists (`ambre-spec-fixtures.md`). The shared writer's
  weight is `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 0 `verified`, 0 `unverified`, 2 `unimplemented`. The page setup has no writer and
  the pagination has no driver. The shared rules and their coverage are in `ambre-spec-pdf-export.md`;
  the render walk they rest on is already surface-generic (`PDF-001`).

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Trame's own turn on the persisted page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| sets the paper size to A3 | exports the sheet | one page format, converged | Bob's file is A3: he did not have to be told (`TRAME-IO-022`) | none -- unbuilt |
| overrides the orientation for one export | reads the page setup | the stored format, unchanged | Alice's file is landscape; Bob's setup is what it was (`TRAME-IO-022`) | none -- unbuilt |

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

## 5. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Trame's own:

* **Nothing exports a PDF in Trame.** There is no driver, no pagination, no command on the bus and no
  menu entry. What exists is the shared seam they will use.
* **Nothing rules what a page break may cut.** A row taller than the printable area, a merged block
  that straddles the edge, and a column wider than the paper have no rule. Repeating the header strip
  on each page has none either.
* **The stored page format has no writer and no reader.** The container is declared in the schema and
  no code touches it (`TRAME-IO-022`), so the rule that two peers get one layout cannot fail yet.
