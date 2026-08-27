# AMBRE -- FEATURE: PDF EXPORT

**Type:** Horizontal (framework)
**Implementation:** partial -- Toile exports through the shared surface; Trame and Prose do not, and
exported text is not selectable in any of them.

> A user hands a document to someone who has no Ambre, and the file looks like the screen. This
> document owns the export flow every editor shares; each application owns how its own content is
> paginated, and `ambre-spec-drawing-surface.md` owns the surface the page is drawn on.

The export is not a conversion of the document: it is the screen renderer aimed at another surface.
That is what makes the file match the screen, and what leaves only the pagination as new work.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `PDF`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `PDF-001` | A page MUST be drawn by the application's own screen walk, retargeted to the shared surface; there MUST be no second, PDF-only renderer. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | Trame's primitives are surface-generic, driven through a recording surface (`ambre_trame::render::tests::draw_cells_on_fills_each_and_counts_nonempty_text`, `::draw_grid_lines_on_strokes_with_grid_color`, `::col_headers_draw_bg_separator_and_active_tint`); Toile drives the same walk against the PDF surface; Prose design-only | verified |
| `PDF-002` | An export MUST emit no CRDT delta: it reads the document and produces bytes, changing nothing. | this spec section 3 | none. Toile derives every page through a read-only camera pass and no test asserts that no container mutated; Trame and Prose are unbuilt | unverified |
| `PDF-003` | After an export, the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 4 | none. Toile hands the bytes to a download and Trame and Prose have no driver; no test reads the handoff back | unverified |
| `PDF-004` | Two exports of one unchanged document MUST produce identical bytes. | this spec section 3 | Toile `ambre_toile::logic::interop::pdf_export::tests::export_is_byte_stable`, under the `pdf` feature that `just test-native` enables; Trame and Prose unbuilt | verified |
| `PDF-005` | A word on the page MUST stay selectable and searchable: each glyph run MUST carry its mapping back to characters, not glyph shapes alone. | this spec section 3; `ambre-spec-drawing-surface.md section 5` | not built -- lands with the "One selectable PDF" Story (`ambre-roadmap.md` E8); the shared surface writes no `ToUnicode` mapping, so every export drops the characters (`ambre-spec-drawing-surface.md section 5`) | unimplemented |
| `PDF-006` | An export MUST be pure vector: it MUST NOT rasterize text, rules or shapes. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level: the shared surface offers only vector primitives, so text and rules have no raster path; a picture reaches the page as its own pixels, and no test forbids an application pre-rasterizing one | unverified |

* **Budget.** the per-application export rows of `ambre-spec-budgets.md section 1` (Toile 50 slides,
  Prose 100 pages); the writer's own weight is `ambre-spec-budgets.md section 5`, billed once to the
  shared `ambre-pdf` crate. None is measured, and no benchmark exists.
* **Boundary.** the finished file reaches the browser and nothing else; it leaves the machine only
  when the user sends it. The export runs in the Logic thread (`ambre-spec-drawing-surface.md`,
  `CANVAS-028`), opens no socket and reads no key.
* **Verification.** 2 `verified`, 3 `unverified`, 1 `unimplemented`. Toile ships the one built export,
  which holds the two verified rows -- the screen walk is surface-generic, and the bytes are stable.
  What is loose is what no test reads back -- the read-only guarantee and the handoff -- and the one
  thing built nowhere is the character mapping, which the surface drops in every application at once.

---

## 2. WHAT EACH USER SEES

The document is shared and an export writes nothing, so two peers, or one machine twice, export the
same bytes.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports a document | exports the same document, untouched | nothing: an export writes nothing (`PDF-002`) | the same bytes, twice (`PDF-004`) | Toile `logic::interop::pdf_export::tests::export_is_byte_stable` |

Each application adds the scenarios that turn on its own page format -- a paper size, a deck order, a
margin -- in its own pdf-export document.

---

## 3. MODEL

**Containers.** none. An export reads the document and produces bytes; it writes nothing (`PDF-002`).

**The flow**, shared by every editor:

1. the Logic thread builds the pages from the document, each drawn by the application's screen walk
   against the shared surface (`PDF-001`, `ambre-spec-drawing-surface.md section 2`);
2. each glyph run is written with its character range, so the page carries text and not shapes alone
   (`PDF-005`);
3. the pages assemble into one file, byte for byte the same on an unchanged document (`PDF-004`);
4. the UI thread hands the bytes to the browser and changes nothing on screen (`PDF-003`).

**The surface and the writer** are `ambre-spec-drawing-surface.md`: the `Surface` trait, the krilla
back end, and the `ambre-pdf` crate compiled behind a feature so it never enters the startup bundle
(`CANVAS-025`, `CANVAS-028`). This document adds no drawing primitive of its own.

**The fonts** the page shaping needs are seeded on the Logic thread (`ambre-spec-render-assets.md`),
because shaping needs a font database and that thread has no GPU side to borrow one from.

**What a page is, and how a document is paginated,** is each application's own: a grid cut into
pages, a deck of slides, a column of pages (section 6).

**Code:**

* `crates/ambre-pdf/` -- the shared PDF surface, over krilla;
* each application's Logic-thread driver -- Toile's `crates/ambre-toile/src/logic/interop/pdf_export.rs`
  is built; Trame's and Prose's are not.

---

## 4. SURFACE

| Trigger | What happens |
|---|---|
| an export entry, in a menu | asks the Logic thread to build the pages and assemble the file |
| the answer | the UI thread wraps the bytes in a browser download (`PDF-003`) |

The user sees no progress beyond the shared interop report; the gesture that starts an export, and
the report it raises, are each application's own. No editor offers the entry on every path yet
(section 6).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The page is drawn by the screen's own walk | a second renderer drifts from the first the day either changes, and the promise of the feature is that the file matches the screen | a PDF-specific walk, which duplicates every element kind in every application |
| The export runs in the Logic thread | a large document takes seconds to assemble, and the UI thread owes the visible frame its sixteen milliseconds; the shaper the export needs is already there | exporting on the UI thread, next to the GPU surface, which stalls the editor for the length of the file |
| An export writes nothing and is byte-stable | an export is a read, and two peers exporting one document must produce one file, so nothing about it may reach the document or another peer | an export that records a page setup or a cursor as it runs |
| Each glyph run carries its source range | a page that looks right and cannot be searched, selected or read aloud is a picture of a document | writing glyphs by index alone, which is what a naive writer does and what makes a file unsearchable |
| The output is vector | a page printed at any size stays sharp, and the shared surface already emits paths and glyphs | rasterizing each page, which trades every advantage of the format for a simpler writer |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Only Toile exports.** Trame and Prose have no driver, no command on the bus and no menu entry;
  what exists is the shared seam they will use. Each is its own per-application story --
  `2-Trame/features/ambre-spec-trame-pdf-export.md` and
  `4-Prose/features/ambre-spec-prose-pdf-export.md`.
* **Exported text is not selectable, in every application** (`PDF-005`). The shared surface writes no
  `ToUnicode` mapping, so the characters are dropped whoever exports
  (`ambre-spec-drawing-surface.md section 5`). It is one repair in the surface, not three in the
  editors.
* **The writer's lazy loading is a claim, not a mechanism.** The feature flag controls compilation,
  not loading; splitting the writer out of the client bundle is a build-system slice nobody has
  scheduled, and Toile and Prose record the same gap.
* **No benchmark measures any export.** The figures of `ambre-spec-budgets.md section 1` have no
  `criterion` bench behind them, so nothing goes red when an assembly gets slower.

---

## 7. WHAT THIS DOCUMENT DOES NOT OWN

| Question | Owner |
|---|---|
| The drawing surface, the krilla back end, the shared `ambre-pdf` crate | `ambre-spec-drawing-surface.md`, `CANVAS-025`, `CANVAS-028` |
| Why exported text carries no characters yet | `ambre-spec-drawing-surface.md section 5` |
| The fonts the page shaping needs, seeded on the Logic thread | `ambre-spec-render-assets.md` |
| How a grid is cut into pages, and the persisted page setup | `2-Trame/features/ambre-spec-trame-pdf-export.md` |
| How a deck becomes one page per slide, and the clip each carries | `3-Toile/features/ambre-spec-toile-pdf-export.md` |
| How a page column is paginated, and the links it carries | `4-Prose/features/ambre-spec-prose-pdf-export.md` |
