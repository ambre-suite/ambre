# AMBRE -- FEATURE: TOILE PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the file reaches the browser; a picture lands at thumbnail resolution

> Toile's half of the shared PDF export (`ambre-spec-pdf-export.md`): one page per slide, and the
> clip that keeps a page to its own frame.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-017`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-038` | `IO` | The bytes an export hands back MUST be a PDF document. | this spec section 3 | `ambre_toile::logic::interop::pdf_export::tests::export_yields_a_pdf` -- it asserts the `%PDF` header. Its second assertion, a length over 400 bytes, holds no row: a page with no content clears it too, section 6; `ambre_toile::logic::interop::pdf_export::tests::multiple_pages_serialize` -- a three-page export also starts with the `%PDF` header | none | verified |
| `TOILE-IO-030` | `IO` | A page MUST carry its own slide alone: content outside the frame MUST NOT reach it. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::the_page_clip_leaves_the_neighbour_out` -- a shape in view of the page camera lies wholly outside the clip, which is the frame rect the walk takes as its region | none | verified |
| `TOILE-IO-031` | `IO` | A picture MUST reach the page as its pixels, never as a placeholder box. | this spec section 3 | `ambre_toile::logic::state::import::tests::a_picture_reaches_the_page_as_pixels` -- an imported picture is drawn as an image of its own size from the store the export fills, and as none from an empty store | none | verified |
| `TOILE-IO-100` | `IO` | A picture MUST reach the page at the resolution it was stored at. | this spec section 3 | not built | unbuilt -- lands with the `A deck exports at full resolution` Story (`ambre-roadmap.md` E6). The store the export fills holds thumbnails alone, capped at a 256 px edge, and the full blob is ciphertext on the relay that no path on this thread fetches and opens | unimplemented |

### 1.2 Export the deck as a PDF

That a page is drawn by the screen's own walk, is byte-stable, changes nothing and keeps its
characters is `ambre-spec-pdf-export.md section 1.1`. What a deck adds is the pagination.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TOILE-IO-039` | `POST` | An export MUST produce one page per slide the deck still resolves. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide` -- it asserts two pages for a deck of two slides | none | verified |
| `TOILE-IO-040` | `POST` | The pages MUST follow the deck's playback order. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::the_pages_follow_the_playback_order` -- the page clips are read back for one deck in both orders, so a sort by id or by creation fails one | none | verified |
| `TOILE-IO-041` | `POST` | A slide whose frame is gone MUST produce no page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::dangling_frame_is_skipped` -- it asserts one page for a deck of one live entry and one dangling one | none | verified |
| `TOILE-IO-042` | `POST` | Every slide page MUST be clipped. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide` -- its second assertion: every page of the deck carries a clip. Which rect it carries is read by no test, and what the clip removes is `TOILE-IO-030` | none | verified |
| `TOILE-IO-043` | `POST` | A board whose deck is empty MUST export as a single page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::empty_deck_falls_back_to_one_page` -- it asserts one page for a board that carries a frame and no deck | none | verified |
| `TOILE-IO-044` | `POST` | That fallback page MUST NOT be clipped. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::empty_deck_falls_back_to_one_page` -- its second assertion: the fallback page carries no clip. That it frames the whole board is asserted by nothing | none | verified |
| `TOILE-IO-045` | `POST` | A board with no content at all MUST produce no page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::empty_board_yields_no_pages` -- it asserts the empty page list on a fresh board | none | verified |
| `TOILE-IO-029` | `POST` | A user MUST be able to export the deck as a PDF. | this spec section 4 | `ambre_logic::dispatch::tests::a_deck_export_answers_with_the_file` -- it sends the menu's own command to a live board and reads the named file out of the answer; `ambre_toile::ui::menu_bar::tests::file_menu_exports_resolve_to_board_commands` -- the entry that sends it; `ambre_toile::logic::state::export::tests::the_deck_exports_as_a_pdf_document` -- the Logic handler | none | verified |
| `TOILE-IO-046` | `FRAME` | An export MUST NOT move the viewer's own camera. | this spec section 3 | structural -- the page builder takes the board by shared reference and mints a fresh transform per page, and the viewport it would move is a plain field, not a container that mutates through such a reference | none | verified |
| `TOILE-IO-047` | `FRAME` | A page MUST NOT carry the board grid. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::a_page_carries_no_grid` -- the page draws no grid line, where the same viewport walked for editing draws some | none | verified |
| `TOILE-IO-099` | `FRAME` | An export MUST NOT stall the edits on the board it reads. | this spec section 4 | not built | unbuilt -- lands with the `An export does not freeze the board` Story (`ambre-roadmap.md` E6). The handler runs to completion inside the Logic thread's command loop, so the stall lasts exactly as long as the export does | unimplemented |
| `TOILE-IO-048` | `FAIL` | An export with no page MUST refuse and say so, rather than hand back an empty file. | this spec section 3 | `ambre_toile::logic::interop::pdf_export::tests::empty_pages_is_an_error` -- it asserts the `Empty` refusal on an empty page list; `ambre_toile::tests::chrome_ui::a_refused_export_says_why` -- the toast the refusal raises; `ambre_toile::logic::state::export::tests::an_empty_board_has_no_pdf_to_export` -- the board-level export answers the same `Empty` refusal on an empty board | none | verified |

* **Budget.** the export of 50 slides under 3 s, a row of
  `ambre-spec-budgets.md section 1`. No benchmark measures it, and the figure is also how long the
  document stops accepting edits (`TOILE-IO-099`) -- section 6.
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 14 `verified`, 2 `unimplemented`. What is counted here is the pagination, the clip and
  what a page shows; the shared rules and their coverage are `ambre-spec-pdf-export.md`. What holds is the
  walk that produces a file, and what a page shows: the clip, the pixels, the grid and the deck order. Each
  of these four is read off the page recorded on a test surface, which is the walk krilla writes. What is
  loose is the export's cost: it freezes the board while it runs and no benchmark times it.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Toile's own turn on the deck:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| reorders the deck | exports it | one order, converged | Bob's file follows Alice's order (`TOILE-IO-040`) | `logic::state::presentation::pdf_tests::the_pages_follow_the_playback_order` |
| deletes a frame that is a slide | exports the deck | the dead entry, still in the deck | a file with one page fewer (`TOILE-IO-041`), and a blank slide in playback | `logic::state::presentation::pdf_tests::dangling_frame_is_skipped` |
| exports a board with no deck | watches | nothing | one unclipped page framing the whole board (`TOILE-IO-043`, `TOILE-IO-044`) | `logic::state::presentation::pdf_tests::empty_deck_falls_back_to_one_page` |
---

## 3. MODEL

**Containers.** This feature owns none. It reads the deck order
(`ambre-spec-toile-schema.md section 3`) and the element geometry, and writes nothing.

**The page** (`ambre-toile`):

```rust
/// One page: the culled, z-ordered content derived at the page camera, the
/// camera that fits the slide into the page, and the frame's world rect to
/// clip to. `clip` is `None` for the single fallback page of an empty deck.
pub struct PdfPage {
    pub viewport: DisplayViewport,
    pub camera:   ViewportTransform,
    pub clip:     Option<Rect>,
}

/// Page dimensions, in points. The camera's pixel size equals these points
/// one for one, so the on-screen fit maps onto the page with no extra scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize { pub width: PrintLength<Pt>, pub height: PrintLength<Pt> }
```

**Steps, when the pages are built** (the shared flow draws and assembles them,
`ambre-spec-pdf-export.md section 3`):

1. read the deck in playback order (`TOILE-IO-040`) and drop each entry whose frame is gone
   (`TOILE-IO-041`), one page for each entry that survives (`TOILE-IO-039`);
2. fit each frame's world rect into the page with the padding the on-screen glide uses;
3. derive the content at that camera, without touching the viewer's own camera (`TOILE-IO-046`);
4. fall back to one unclipped page framing the whole board when the deck is empty (`TOILE-IO-043`,
   `TOILE-IO-044`). Fall back to no page at all when the board is empty (`TOILE-IO-045`);
5. clip each page to its frame's rect (`TOILE-IO-042`), which is what keeps the neighbours off it
   (`TOILE-IO-030`);
6. refuse the export when the page list came back empty, rather than assemble a file with no page
   in it (`TOILE-IO-048`).

**The clip is load-bearing.** The camera fits a frame by the tighter of the two axes, which leaves
slack on the other one. Without the clip that slack pulls a neighbouring frame's content onto the
page.

**One walk serves the screen and the page.** The export drives the same content walk as the screen, against
the PDF surface instead of the GPU one. So a shape, a stroke and a glyph are placed by the same code in both.
The board grid is dropped, as it is in presentation mode (`TOILE-IO-047`). **Code:**

* `crates/ambre-toile/src/logic/state/presentation.rs` -- `pdf_pages`, the page per slide;
* `logic/interop/pdf_export.rs` -- the page loop, the clip and the assembly;
* `crates/ambre-pdf/src/lib.rs` -- the shared PDF surface (`ambre-spec-pdf-export.md`).

---

## 4. SURFACE

One File menu entry, "Export deck as PDF", beside the SVG one
(`features/ambre-spec-toile-import-export.md section 4`). No chord: the gesture is rare and the
menu is where a user looks for a file.

| Trigger | What must happen |
|---|---|
| the File menu export entry | the Logic thread builds the pages and assembles the file (`TOILE-IO-029`) |
| the answer | the UI thread hands the bytes to the browser as `deck.pdf`, a name `DOCFILE-019` and `DOCFILE-020` hold; no board state changes (`PDF-016`, `PDF-014`, `TOILE-IO-046`) |
| an export with no page, or a session whose fonts have not landed | a notice says which, rather than a file that never arrives (`TOILE-IO-048`) |

**The handler sits on the Logic thread, not in the board.** Every other board command is answered by
`ToileState`; this one is not. The glyphs need the base font faces, which the Main thread fetches at boot.
They are seeded into the Logic thread's own `export_cx`, a resource beside the documents rather than inside
one. So the dispatcher intercepts `ExportPdf` before the document lookup. It borrows the board and the faces
apart, and answers on the same queue as any other command; the board stays read-only through it
(`TOILE-IO-046`).

**A build without the writer still answers.** `krilla` is compiled behind a feature, and the launcher is the
build that enables it. A build that does not gets the same event carrying the reason instead of the bytes. The
surface never depends on which build it is talking to.

---

## 5. REJECTED ALTERNATIVES

The shared decisions -- the page drawn by the screen's own walk, the export in the Logic thread, the
pure-vector file -- are `ambre-spec-pdf-export.md section 5`. Toile's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A page is one slide, clipped to its frame | one page per board, or a page unclipped | the audience sees a frame, so the page must show a frame. The camera cannot fit both axes exactly, so without the clip the slack on the looser axis pulls a neighbouring frame onto the page |
| An empty deck exports the whole board | refusing to export, or writing an empty file | a user with no deck still wants a file, and the board's own bounds are the only frame available |
| The transition is not exported | rendering the intermediate frames as pages | a page is static, so an animation between two slides has no representation on one. Writing the frames out turns a two-slide deck into a thick document nobody asked for |
| `krilla` is compiled behind a feature | linking it unconditionally | the writer is large and only the Logic thread needs it. The saving is real for every build that is not the launcher -- the workspace test run, the server, a host tool -- and not for the client: the launcher enables the feature, so the shipped bundle carries the writer either way (`ambre-spec-budgets.md section 5`). Making the client pay only on the first export needs a separate wasm chunk, which no build step produces -- section 6 |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Toile's own:

* **A page is no sharper than the board is on screen** (`TOILE-IO-100`). The store the export
  reads is filled from each media entry's inline thumbnail, capped at a 256 px edge. That is every pixel this
  thread holds: the full-resolution blob is ciphertext on the relay, and no path on this side fetches and
  opens it. A picture therefore reaches the page, and reaches it at thumbnail resolution -- enough to read on
  screen, not enough to print. What closes it is a media-decrypt path on the Logic thread, which is a slice of
  its own. The export would then await a fetch it cannot await today, so the two gaps below are its
  prerequisites rather than its neighbours.
* **The export freezes the document while it runs** (`TOILE-IO-099`). The handler is called from
  `handle_app_command`, itself called from the thread's parking loop, and it returns only once the
  file is assembled. Until then no UI command is dequeued and no remote delta is applied. The deltas queue
  rather than drop, so nothing is lost; the board is simply unresponsive, for the 3 s the budget allows 50
  slides. Closing it means taking the walk off the command loop: its own worker, or a resumable pass the loop
  re-enters. `krilla` assembles a document in one call, so the worker is the shape that fits.
* **The budget is not measured** (`ambre-spec-budgets.md section 1`). 50 slides under 3 s is a
  figure no benchmark reads, and the walk now has a caller that could be timed. It is the same
  figure as the stall above, so one benchmark holds both.
* **The feature gate no longer shrinks the client** (`ambre-spec-budgets.md section 5`). The launcher enables
  it, so every user downloads the writer whether or not they ever export. The lazy-loading the budget
  prescribes for Prose's interop wants the same treatment here. It is not a Cargo feature that delivers it:
  the build produces one wasm artifact per instance. Deferring the writer needs a second artifact fetched on
  the first export, which no build step produces.
* **A page count is claimed in a comment and asserted nowhere** (`TOILE-IO-039`). The multi-page test says "a
  page count of N produces N pages", then asserts the `%PDF` header alone. A writer that emitted one page for
  three would pass it. The count that does hold comes from the page builder, one layer earlier, where two
  slides are asserted to yield two pages. Nothing joins the two ends: no test hands the builder's pages to the
  assembler and counts what comes out.
* **A byte length is not a content walk** (`TOILE-IO-038`). The single-page test asserts a file
  longer than 400 bytes and its comment reads that as proof the rect reached the surface. A PDF
  carrying one empty page clears that length on its structure alone, so the assertion separates
  nothing and holds no row.
* **An empty board has no file** (`TOILE-IO-045`, `TOILE-IO-048`). The builder returns no page for a board
  with no content, and the assembler refuses an empty page list. So a user with an empty board gets a refusal
  rather than a blank document. Each half is asserted on its own; nothing calls them in sequence, because
  nothing calls either.
* **Speaker notes reach no page.** The corpus offered them as page annotations. Nothing writes an
  annotation, and the notes exist on the deck already.
