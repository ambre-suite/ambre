# AMBRE -- FEATURE: PDF EXPORT

**Type:** Horizontal (framework)
**Applications:** Prose, Trame, Toile

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
| `PDF-008` | `POST` | In Toile, a page MUST be drawn by the application's own screen walk, retargeted to the shared surface, and by no second PDF-only renderer. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | structural -- `crates/ambre-toile/src/logic/interop/pdf_export.rs` imports `paint_content` from `crates/ambre-toile/src/ui/render.rs` and records it for the converter module to replay on a krilla page, so the page walk and the screen walk are one function, and a second renderer would be a second function that does not exist; Toile `ambre_toile::logic::interop::pdf_export::tests::export_yields_a_pdf` | none | verified |
| `PDF-001` | `POST` | In Trame, a page MUST be drawn by the application's own screen walk, retargeted to the shared surface, and by no second PDF-only renderer. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | structural -- `crates/ambre-trame/src/logic/print.rs` draws each page with `SheetWalk`, the walk the screen's `draw_cells` runs, over cells the viewport's packer packs and `CellData::from` reads as the screen cache does; Trame `ambre_trame::logic::print::tests::a_page_draws_the_walk_the_screen_draws` -- the recorded page holds the walk over its region, call for call | none | verified |
| `PDF-009` | `POST` | In Prose, a page MUST be drawn by the application's own screen walk, retargeted to the shared surface, and by no second PDF-only renderer. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | structural -- `crates/ambre-prose/src/view/print.rs` resolves a page through `ProseUiState::resolve_page` and draws it with `PageWalk`, the two the screen's `paint` runs, onto a recorded page the converter module replays; Prose `ambre_prose::logic::print::tests::a_pdf_page_carries_the_pages_the_screen_shows` | none | verified |
| `PDF-010` | `POST` | In Toile, two exports of one unchanged document MUST produce identical bytes, whether one peer runs both or two peers holding the same faces do. | this spec section 3; section 5 | Toile `ambre_toile::logic::interop::pdf_export::tests::export_is_byte_stable`; `::two_peers_holding_the_same_faces_export_one_file` -- two sealed contexts fed the shipped faces, against an empty one as the control, both under the `pdf` feature that `just test-native` enables | none | verified |
| `PDF-004` | `POST` | In Trame, two exports of one unchanged document MUST produce identical bytes, whether one peer runs both or two peers holding the same faces do. | this spec section 3; section 5 | Trame `ambre_trame::logic::print::tests::two_peers_holding_the_same_faces_export_one_file` -- two exports of one peer, and the export of a second peer fed the shipped faces, read the same bytes over two merged blocks, and a third peer fed no face writes another file as the control | none | verified |
| `PDF-011` | `POST` | In Prose, two exports of one unchanged document MUST produce identical bytes, whether one peer runs both or two peers holding the same faces do. | this spec section 3; section 5 | Prose `ambre_prose::logic::print::tests::two_peers_holding_the_same_faces_export_one_file` -- two exports of one peer, and the export of a second peer fed the shipped faces, read the same bytes, and a third peer fed no face writes another file as the control | none | verified |
| `PDF-007` | `FRAME` | The context an export shapes on MUST hold only the faces the application delivered to it: a face of the machine MUST NOT be discoverable from it. | this spec section 3; `PROSE-114` | `ambre_logic::dispatch::tests::the_export_context_holds_only_the_faces_delivered_to_it` -- it asks for families a desktop carries after a delivery, and fails on the unsealed context it replaced; structural -- one site builds the context, from `ambre_draw::sealed_font_context`; `ambre_draw::text::tests::a_sealed_context_discovers_no_face_of_the_machine` -- that context, asked for the same families | none | verified |
| `PDF-012` | `POST` | In Toile, an export MUST be pure vector: it MUST NOT rasterize text, rules or shapes. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the shared surface offers only vector primitives, so text and rules have no raster path; Toile `ambre_toile::logic::interop::pdf_export::tests::a_page_without_a_picture_paints_no_image` -- a page holding a shape and a word records glyphs and a vector fill and no image paint, so a walk that rasterized either fails | none | verified |
| `PDF-006` | `POST` | In Trame, an export MUST be pure vector: it MUST NOT rasterize text, rules or shapes. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the shared surface offers only vector primitives, so text and rules have no raster path; Trame `ambre_trame::logic::print::tests::a_printed_page_paints_no_image` -- pages holding text, fills and a merged block record glyphs and vector fills, and no image paint | none | verified |
| `PDF-013` | `POST` | In Prose, an export MUST be pure vector: it MUST NOT rasterize text, rules or shapes. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- the shared surface offers only vector primitives, so text and rules have no raster path; Prose `ambre_prose::logic::print::tests::a_printed_page_paints_no_image` -- pages holding paragraphs and a table record glyphs and vector rules, and no image paint | none | verified |
| `PDF-005` | `POST` | A word on the page MUST stay selectable and searchable: each glyph run MUST carry its mapping back to characters, not glyph shapes alone. | this spec section 3; `ambre-spec-drawing-surface.md section 2` | type-level -- `DrawSurface::draw_glyphs` takes the text the run's layout was built from, so no walk draws a run without naming its characters; `selectable::a_latin_line_reads_back`; `::a_ligature_reads_back_as_its_letters`; `::a_combining_mark_reads_back_with_its_base`; `::a_cluster_of_several_glyphs_reads_back_once`; `::emoji_read_back_as_their_sequences`; `::a_right_to_left_word_reads_back_in_logical_order`; `::the_live_surface_reads_back_as_the_recorded_one` -- each shapes on the shipped faces, writes the file and reads it back with `pdf-extract`, which shares no code with krilla; Toile `ambre_toile::logic::interop::pdf_export::tests::an_exported_word_reads_back_as_its_characters` -- under the `pdf` feature that `just test-native` enables; Trame `ambre_trame::render::tests::a_cell_written_to_pdf_reads_back_as_its_characters`; Prose `ambre_prose::view::paint::tests::a_page_written_to_pdf_reads_back_as_its_text`; `ambre_prose::layout::measure::tests::a_paragraph_shaped_in_part_reads_back_as_its_text` -- a run re-shaped in part after inserts and erases | none | verified |
| `PDF-014` | `POST` | In Toile, after an export, the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 4 | Toile `ambre_toile::ui::view::view_tests::a_finished_export_is_handed_to_the_browser_and_nothing_else` -- the finished file reaches the download stub once, by name and bytes, and the view sends no command, keeps its selection and paints the same operations | none | verified |
| `PDF-003` | `POST` | In Trame, after an export, the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 4 | Trame `ambre_trame::ui::view::view_tests::a_finished_pdf_is_handed_to_the_browser_and_nothing_else` -- the finished file reaches the download stub once, by name and bytes, and the view sends no command, keeps its selection and paints the same operations | none | verified |
| `PDF-015` | `POST` | In Prose, after an export, the UI thread MUST do nothing but hand the finished file to the browser. | this spec section 4 | Prose `ambre_prose::view::paint::tests::a_finished_pdf_is_handed_to_the_browser_and_nothing_else` -- the file waits for the host once, by name and bytes, and the view asks for no page, keeps its selection and paints the same operations; `ambre_belt::prose::tests::a_pdf_export_reaches_the_download_directory` -- browser tier: the File menu's entry lands a PDF in the download directory | none | verified |
| `PDF-016` | `FRAME` | In Toile, a PDF export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | Toile `ambre_toile::logic::state::export::tests::a_pdf_export_leaves_the_oplog_unchanged` -- the board's version vectors read the same after an export, which a shared receiver does not hold since the document mutates through one; Toile `ambre_logic::dispatch::tests::a_deck_export_sends_no_delta` -- the routed export sends nothing to Sync, both under the `pdf` feature | none | verified |
| `PDF-002` | `FRAME` | In Trame, a PDF export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | Trame `ambre_trame::logic::print::tests::a_pdf_export_leaves_the_oplog_unchanged` -- no slice answers a delta, and the oplog's version vector reads the same after the export; `ambre_logic::dispatch::tests::a_sheet_pdf_export_answers_with_the_file_and_sends_no_delta` -- the routed export sends Sync the module fetch and nothing else | none | verified |
| `PDF-017` | `FRAME` | In Prose, a PDF export MUST NOT change the document: it MUST emit no CRDT delta. | this spec section 3 | Prose `ambre_prose::logic::print::tests::a_pdf_export_leaves_the_oplog_unchanged` -- no slice answers a delta, and the oplog's version vector reads the same after the export; `ambre_logic::dispatch::tests::a_prose_pdf_export_answers_with_the_file_and_sends_no_delta` -- the routed export sends Sync the module fetch and nothing else | none | verified |
| `PDF-018` | `POST` | A glyph run drawn with a text that is not its layout's own MUST still be written, with its shapes and with no character. | this spec section 3 | `ambre_pdf::document::tests::a_cluster_past_its_text_writes_the_shapes_alone` -- a recorded cluster past its text and one inside a character, which krilla would slice and panic on; `selectable::a_text_that_is_not_the_layouts_own_draws_shapes_alone` | none | verified |

* **Budget.** the per-application export rows of `ambre-spec-budgets.md section 1` (Toile 50 slides,
  Prose 100 pages); the writer's own weight is `ambre-spec-budgets.md section 5`, billed once to the
  shared `ambre-pdf` crate. No whole export is measured. One slice of the Prose walk, one page, is
  held by a count of instructions.
* **Boundary.** the finished file reaches the browser and nothing else; it leaves the machine only
  when the user sends it. The export runs in the Logic thread (`ambre-spec-drawing-surface.md`,
  `CANVAS-028`), opens no socket and reads no key.
* **Verification.** 18 `verified`. The sealed faces hold for every application, since one context
  serves them all. Every other rule holds in each of the three applications, each through its own
  screen walk written by the shared writer. Trame's browser handoff is held by a host test of the
  view alone, while Prose also drives the served build.

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
   (`PDF-005`). A ligature maps one glyph to several characters, and a cluster of several glyphs
   maps once, through an `ActualText` span. A run whose text does not fit its clusters keeps its
   shapes and drops its characters (`PDF-018`);
3. the pages assemble into one file, byte for byte the same on an unchanged document (`PDF-004`).
   The file carries the live links of each page and the title and language the application
   names. A field left empty writes nothing, so the writer adds no value of its own;
4. the UI thread hands the bytes to the browser and changes nothing on screen (`PDF-003`).

**The surface, the writer and the fonts** are `ambre-spec-drawing-surface.md` and
`ambre-spec-render-assets.md`. They are the `Surface` trait, the krilla back end and the `ambre-pdf`
crate, in the PDF converter module. They also hold the font database the shaping needs
(`CANVAS-025`, `CANVAS-028`).
The Logic thread records each page as a draw list and the PDF converter module writes the file
(`ambre-spec-architecture.md section 2`, the converter modules). This document adds no primitive of
its own.

**The export context is sealed** (`PDF-007`). A default context discovers the faces of the machine,
and parley falls back to one of them for a codepoint the asked family misses. Two peers would then
write two files. The Logic thread builds its context from `ambre_draw::sealed_font_context`, the
constructor pagination and the UI thread share. That is why `ambre-logic` depends on `ambre-draw`
directly: the crate is already in its graph through every application, so the edge adds no code.

**What a page is, and how a document is paginated,** is each application's own. One cuts a grid into
pages, one a deck of slides, one a column of pages (section 6).

**Code:**

* `crates/ambre-pdf/` -- the shared PDF surface, over krilla, and the replay of a recorded document;
* `crates/ambre-convert/` -- the converter modules, whose PDF module links it;
* each application's Logic-thread driver -- Toile's `crates/ambre-toile/src/logic/interop/pdf_export.rs`
  Prose's `crates/ambre-prose/src/logic/print.rs` and Trame's `crates/ambre-trame/src/logic/print.rs`
  are built.

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

* **A right-to-left word is written in visual order.** A reader restores the logical order with its
  own bidi pass, as it does for any PDF. krilla marks a run `ReversedChars` only when no two glyphs
  share a cluster. A word with a two-glyph cluster relies on the reader alone.
* **The writer's lazy loading is a claim, not a mechanism.** The feature flag controls compilation,
  not loading. Splitting the writer out of the client bundle is a build-system slice nobody has
  scheduled, and Toile and Prose record the same gap.
* **No benchmark measures a whole export.** The figures of `ambre-spec-budgets.md section 1` have
  no `criterion` bench behind them, so nothing goes red when an assembly gets slower. One slice of
  the Prose walk is counted, and the module's write of the whole file is not.

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
