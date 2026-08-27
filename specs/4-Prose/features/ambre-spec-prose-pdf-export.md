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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PROSE-IO-029` | A PDF page MUST carry the pagination the screen shows: the export MUST NOT lay the document out under another page format. | this spec section 3; `ambre-spec-prose-schema.md` (`PROSE-032`) | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-031` | Every link the document carries MUST reach the file as a live link: the table of contents, a cross reference and an external address. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-032` | The exported document's language MUST come from the request or from the document; it MUST NOT be a fixed locale. | this spec section 3 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-IO-033` | A user MUST be able to export the document as a PDF. | this spec section 4 | not built -- lands with the "PDF export" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the PDF export of a hundred pages, without and with pictures, one row of
  `ambre-spec-budgets.md section 1`, reading **none exists**. The shared writer's weight is
  `ambre-spec-pdf-export.md`.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`); a picture it cannot find becomes a placeholder
  rather than a fetch (`features/ambre-spec-prose-images.md`, `PROSE-IO-008`).
* **Verification.** 4 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`. The
  pagination reuse and the links are the two a host test reaches from a fixture document, with no GPU
  and no picture. The shared rules and their coverage are in `ambre-spec-pdf-export.md`.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Prose's own turn on the page format:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| widens the margins | exports | one page format, converged | Bob's file follows Alice's margins | none -- `PROSE-IO-029` |
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
   (`PROSE-IO-029`);
2. open a page of that size in points, converting from the millimetres the format stores;
3. add the links: the table of contents entries, the cross references, and the external addresses
   the runs carry (`PROSE-IO-031`);
4. write the metadata -- the title, the language and the author -- and close the file
   (`PROSE-IO-032`).

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

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The export reuses the layout, and never recomputes it | the layout is the document's own pagination, and it is what the reader is looking at. Recomputing it is a second implementation of the hardest algorithm in the application | laying the document out again at export time, which makes the file and the screen disagree by construction |
| The language comes from the caller or the document | a hardcoded locale is wrong for every reader who does not share it, and nothing in the file says it is wrong | defaulting to one language, which is the failure this rule exists to name |
| A missing picture becomes a placeholder | an export is a gesture with an end, and waiting on a fetch inside it turns a five-second action into an unbounded one. The report carries the loss | fetching the missing pictures inside the export, which makes an offline export hang instead of finishing |

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
