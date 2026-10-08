# AMBRE -- FEATURE: PDF EXPORT

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile
**Implementation:** partial -- Toile exports through the shared surface, Trame and Prose do not.
Exported text is not selectable in any of them.

> A user hands a document to someone who has no Ambre, and the file looks like the screen. This
> document owns the export flow every editor shares. Each application owns how its own content is
> paginated, and `ambre-spec-drawing-surface.md` owns the surface the page is drawn on.

The export is not a conversion of the document: it is the screen renderer aimed at another surface.
That is what makes the file match the screen, and what leaves only the pagination as new work.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The prefix is `PDF`.

### 1.1 Export the document as a PDF

One gesture, from the command to the finished file. Every rule below binds that run: what each page
is drawn by, what the bytes owe, and what the export must leave alone.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PDF-001` | `POST` | A page MUST be drawn by the application's own screen walk, retargeted to the shared surface, and by no second PDF-only renderer. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | structural -- `crates/ambre-toile/src/logic/interop/pdf_export.rs` imports `paint_content` from `crates/ambre-toile/src/ui/render.rs` and runs it against a krilla page, so the page walk and the screen walk are one function, and a second renderer would be a second function that does not exist; `ambre_toile::logic::interop::pdf_export::tests::export_yields_a_pdf`; `ambre_trame::render::tests::draw_cells_on_fills_each_and_counts_nonempty_text`; `::draw_grid_lines_on_strokes_with_grid_color`; `::col_headers_draw_bg_separator_and_active_tint` | partial -- Toile alone exports. Trame's primitives are surface-generic and driven through a recording surface, and it aims them at no page yet; Prose is design-only; lands with the `The text editor and the PDF export held` Story (`ambre-roadmap.md` E9) | partial |
| `PDF-004` | `POST` | Two exports of one unchanged document MUST produce identical bytes, whether one peer runs both or two peers holding the same faces do. | this spec section 3; section 5 | Toile `ambre_toile::logic::interop::pdf_export::tests::export_is_byte_stable`; `::two_peers_holding_the_same_faces_export_one_file` -- two sealed contexts fed the shipped faces, against an empty one as the control, both under the `pdf` feature that `just test-native` enables | partial -- Toile alone. Trame and Prose export nothing to be stable about; lands with the `The text editor and the PDF export held` Story (`ambre-roadmap.md` E9) | partial |
| `PDF-007` | `FRAME` | The context an export shapes on MUST hold only the faces the application delivered to it: a face of the machine MUST NOT be discoverable from it. | this spec section 3; `PROSE-114` | `ambre_logic::dispatch::tests::the_export_context_holds_only_the_faces_delivered_to_it` -- it asks for families a desktop carries after a delivery, and fails on the unsealed context it replaced; structural -- one site builds the context, from `ambre_draw::sealed_font_context`; `ambre_draw::text::tests::a_sealed_context_discovers_no_face_of_the_machine` -- that context, asked for the same families | none | verified |
| `PDF-006` | `POST` | An export MUST be pure vector: it MUST NOT rasterize text, rules or shapes. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the shared surface offers only vector primitives, so text and rules have no raster path | partial -- a picture reaches the page as its own pixels, and no test forbids an application pre-rasterizing one; lands with the `The text editor and the PDF export held` Story (`ambre-roadmap.md` E9) | partial |
| `PDF-005` | `POST` | A word on the page MUST stay selectable and searchable: each glyph run MUST carry its mapping back to characters, not glyph shapes alone. | this spec section 3; `ambre-spec-drawing-surface.md section 5` | not built | unbuilt -- lands with the `One selectable PDF` Story (`ambre-roadmap.md` E8). The shared surface writes no `ToUnicode` mapping, so every export drops the characters (`ambre-spec-drawing-surface.md section 5`) | unimplemented |
| `PDF-003` | `POST` | After an export, the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 4 | none | unheld -- Toile hands the bytes to a download and Trame and Prose have no driver; no test reads the handoff back; lands with the `The text editor and the PDF export held` Story (`ambre-roadmap.md` E9) | untested |
| `PDF-002` | `FRAME` | A PDF export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | none | unheld -- Toile derives every page through a read-only camera pass and no test asserts that no container mutated; Trame and Prose are unbuilt; lands with the `The text editor and the PDF export held` Story (`ambre-roadmap.md` E9) | untested |

* **Budget.** the per-application export rows of `ambre-spec-budgets.md section 1` (Toile 50 slides,
  Prose 100 pages); the writer's own weight is `ambre-spec-budgets.md section 5`, billed once to the
  shared `ambre-pdf` crate. None is measured, and no benchmark exists.
* **Boundary.** the finished file reaches the browser and nothing else; it leaves the machine only
  when the user sends it. The export runs in the Logic thread (`ambre-spec-drawing-surface.md`,
  `CANVAS-028`), opens no socket and reads no key.
* **Verification.** 1 `verified`, 3 `partial`, 2 `untested`, 1 `unimplemented`. The sealed faces
  hold for every application, since one context serves them all. The screen walk and the stable
  bytes hold for Toile alone, so both rows read `partial`. What is loose is what no test reads back:
  the read-only guarantee and the handoff. The one thing built nowhere is the character mapping,
  which the surface drops in every application at once.

---

## 2. WHAT EACH USER SEES

The document is shared and an export writes nothing, so two peers, or one machine twice, export the
same bytes.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports a document | exports the same document, untouched | nothing: an export writes nothing (`PDF-002`) | the same bytes, twice (`PDF-004`), shaped on the faces Ambre ships alone (`PDF-007`) | Toile `logic::interop::pdf_export::tests::two_peers_holding_the_same_faces_export_one_file` |

Each application adds the scenarios its own page format turns on, in its own pdf-export document.

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

**The surface, the writer and the fonts** are `ambre-spec-drawing-surface.md` and
`ambre-spec-render-assets.md`. They are the `Surface` trait, the krilla back end, the `ambre-pdf`
crate behind a feature, and the font database the shaping needs (`CANVAS-025`, `CANVAS-028`). This
document adds no primitive of its own.

**The export context is sealed** (`PDF-007`). A default context discovers the faces of the machine,
and parley falls back to one of them for a codepoint the asked family misses. Two peers would then
write two files. The Logic thread builds its context from `ambre_draw::sealed_font_context`, the
constructor pagination and the UI thread share. That is why `ambre-logic` depends on `ambre-draw`
directly: the crate is already in its graph through every application, so the edge adds no code.

**What a page is, and how a document is paginated,** is each application's own. One cuts a grid into
pages, one a deck of slides, one a column of pages (section 6).

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

The gesture that starts an export and the report it raises are each application's own. No editor
offers the entry on every path yet (section 6).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| The page is drawn by the screen's own walk | a PDF-specific walk | it duplicates every element kind in every application, and the second renderer drifts from the first the day either changes. The promise of the feature is that the file matches the screen |
| The export runs in the Logic thread | exporting on the UI thread, next to the GPU surface | a large document takes seconds to assemble and the UI thread owes the visible frame its sixteen milliseconds, so it stalls the editor for the length of the file. The shaper the export needs is already in the Logic thread |
| An export writes nothing and is byte-stable | an export that records a page setup or a cursor as it runs | an export is a read, and two peers exporting one document must produce one file. A recorded setup reaches the document, so it reaches the other peer and the undo history |
| Each glyph run carries its source range | writing glyphs by index alone | it is what a naive writer does, and it is what makes a file unsearchable: a page that looks right and cannot be searched, selected or read aloud is a picture of a document |
| The output is vector | rasterizing each page and embedding the pixels | it trades every property the format is for: a raster page does not zoom, its text is not searchable, and its size grows with the page count. The shared surface already emits paths and glyphs, so the vector path is also the shorter one |

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
  not loading. Splitting the writer out of the client bundle is a build-system slice nobody has
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
