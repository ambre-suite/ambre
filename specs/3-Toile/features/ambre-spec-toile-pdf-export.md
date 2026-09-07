# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the pages are built and serialized; nothing asks for a file.

> Toile's half of the shared PDF export (`ambre-spec-pdf-export.md`): one page per slide, and the
> clip that keeps a page to its own frame.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`. The shared rules are `ambre-spec-pdf-export.md`,
`PDF-001` through `PDF-006`.

### 1.1 The file

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-IO-038` | `IO` | The bytes an export hands back MUST be a PDF document. | this spec section 3 | `ambre_toile::logic::interop::pdf_export::tests::export_yields_a_pdf` asserts the `%PDF` header. Its second assertion, a length over 400 bytes, holds no row: a page with no content clears it too -- section 6 | verified |
| `TOILE-IO-030` | `IO` | A page MUST carry its own slide alone: content outside the frame MUST NOT reach it. | this spec section 3 | none, and the clip is what would hold it: two rows assert that a page carries a clip or does not (`TOILE-IO-042`, `TOILE-IO-044`), and nothing asserts that the clip removes a neighbouring element -- section 6 | unverified |
| `TOILE-IO-031` | `IO` | A picture MUST reach the page as its pixels, never as a placeholder box. | this spec section 3 | none, and **the rule fails today**: the walk paints a box for a hash it cannot resolve, and no code fills the store the export reads -- section 6 | unverified |

### 1.2 Export the deck as a PDF

That a page is drawn by the screen's own walk, is byte-stable, changes nothing and keeps its
characters is `ambre-spec-pdf-export.md section 1.1`. What a deck adds is the pagination.

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `TOILE-IO-039` | `POST` | An export MUST produce one page per slide the deck still resolves. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide` asserts two pages for a deck of two slides | verified |
| `TOILE-IO-040` | `POST` | The pages MUST follow the deck's playback order. | this spec section 3 | none: the test above builds its deck in creation order and reads no page back, so a walk sorting by id would pass it unchanged -- section 6 | unverified |
| `TOILE-IO-041` | `POST` | A slide whose frame is gone MUST produce no page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::dangling_frame_is_skipped` asserts one page for a deck of one live entry and one dangling one | verified |
| `TOILE-IO-042` | `POST` | Every slide page MUST be clipped. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide`, its second assertion: every page of the deck carries a clip. Which rect it carries is read by no test, and what the clip removes is `TOILE-IO-030` | verified |
| `TOILE-IO-043` | `POST` | A board whose deck is empty MUST export as a single page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::empty_deck_falls_back_to_one_page` asserts one page for a board that carries a frame and no deck | verified |
| `TOILE-IO-044` | `POST` | That fallback page MUST NOT be clipped. | this spec section 3 | the same test, its second assertion: the fallback page carries no clip. That it frames the whole board is asserted by nothing | verified |
| `TOILE-IO-045` | `POST` | A board with no content at all MUST produce no page. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::empty_board_yields_no_pages` asserts the empty page list on a fresh board | verified |
| `TOILE-IO-029` | `POST` | A user MUST be able to export the deck as a PDF. | this spec section 4 | not built -- lands with the "The Toile interop surface" Story (`ambre-roadmap.md` E4); the page builder and the serializer both exist and no gesture calls either | unimplemented |
| `TOILE-IO-046` | `FRAME` | An export MUST NOT move the viewer's own camera. | this spec section 3 | structural, and the construction is the proof: the page builder takes the board by shared reference and mints a fresh transform per page, and the viewport it would move is a plain field, not a container that mutates through such a reference | verified |
| `TOILE-IO-047` | `FRAME` | A page MUST NOT carry the board grid. | this spec section 3 | none: the walk is driven in present mode, which is what drops the grid, and no test reads a page's content back | unverified |
| `TOILE-IO-048` | `FAIL` | An export with no page MUST refuse and say so, rather than hand back an empty file. | this spec section 3 | `ambre_toile::logic::interop::pdf_export::tests::empty_pages_is_an_error` asserts the `Empty` refusal on an empty page list | verified |

* **Budget.** the export of 50 slides under 3 s, a row of
  `ambre-spec-budgets.md section 1`. No benchmark measures it, and no gesture starts it
  (`TOILE-IO-029`).
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 9 `verified`, 4 `unverified`, 1 `unimplemented`. The shared rules and their
  coverage are `ambre-spec-pdf-export.md`; what is counted here is the pagination, the clip and what
  a page shows. The split cost a mechanism rather than earning one. The old single row stacked eight
  tests behind three rules at once, and one of the eight is now cited by nothing:
  `::multiple_pages_serialize` claims a page count in its doc comment and asserts only the `%PDF`
  header, so one page would pass it as readily as three. What stays loose is what a page shows --
  the clip's effect, the pixels, the grid -- and the deck order, which the pagination test builds
  and never reads back. None of the four has a reader that could go red, because the export has no
  caller.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Toile's own turn on the deck:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| reorders the deck | exports it | one order, converged | Bob's file follows Alice's order (`TOILE-IO-040`) | none: no test reads a page order back |
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
pub struct PageSize { pub width: f64, pub height: f64 }
```

**Steps, when the pages are built** (the shared flow draws and assembles them,
`ambre-spec-pdf-export.md section 3`):

1. read the deck in playback order (`TOILE-IO-040`) and drop each entry whose frame is gone
   (`TOILE-IO-041`), one page for each entry that survives (`TOILE-IO-039`);
2. fit each frame's world rect into the page with the padding the on-screen glide uses;
3. derive the content at that camera, without touching the viewer's own camera (`TOILE-IO-046`);
4. fall back to one unclipped page framing the whole board when the deck is empty (`TOILE-IO-043`,
   `TOILE-IO-044`), and to no page at all when the board is (`TOILE-IO-045`);
5. clip each page to its frame's rect (`TOILE-IO-042`), which is what keeps the neighbours off it
   (`TOILE-IO-030`);
6. refuse the export when the page list came back empty, rather than assemble a file with no page
   in it (`TOILE-IO-048`).

**The clip is load-bearing.** The camera fits a frame by the tighter of the two axes, which leaves
slack on the other one. Without the clip that slack pulls a neighbouring frame's content onto the
page.

**One walk serves the screen and the page.** The export drives the same content walk as the screen
against the PDF surface instead of the GPU one, so a shape, a stroke and a glyph are placed by the
same code in both. The board grid is dropped, as it is in presentation mode (`TOILE-IO-047`).
**Code:**

* `crates/ambre-toile/src/logic/state/presentation.rs` -- `pdf_pages`, the page per slide;
* `logic/interop/pdf_export.rs` -- the page loop, the clip and the assembly;
* `crates/ambre-pdf/src/lib.rs` -- the shared PDF surface (`ambre-spec-pdf-export.md`).

---

## 4. SURFACE

**There is none** (`TOILE-IO-029`). No menu entry and no chord asks for a file; the File menu carries
one row and it starts the presentation (`features/ambre-spec-toile-menus.md section 4`).

| Trigger, once it exists | What must happen |
|---|---|
| the File menu export entry | the Logic thread builds the pages and assembles the file |
| the answer | the UI thread hands the bytes to the browser; no board state changes (`PDF-002`, `PDF-003`) |

The font context the glyphs need is seeded at boot and read by nothing today: the resource is warm
and the caller is absent.

---

## 5. REJECTED ALTERNATIVES

The shared decisions -- the page drawn by the screen's own walk, the export in the Logic thread, the
pure-vector file -- are `ambre-spec-pdf-export.md section 5`. Toile's own:

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A page is one slide, clipped to its frame | one page per board, or a page unclipped | the audience sees a frame, so the page must show a frame. The camera cannot fit both axes exactly, so without the clip the slack on the looser axis pulls a neighbouring frame onto the page |
| An empty deck exports the whole board | refusing to export, or writing an empty file | a user with no deck still wants a file, and the board's own bounds are the only frame available |
| The transition is not exported | rendering the intermediate frames as pages | a page is static, so an animation between two slides has no representation on one. Writing the frames out turns a two-slide deck into a thick document nobody asked for |
| `krilla` is compiled behind a feature | linking it unconditionally | the writer is large and only the Logic thread needs it, so the default build stays smaller (`ambre-spec-budgets.md section 5`) |

---

## 6. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Toile's own:

* **Nothing asks for a file** (`TOILE-IO-029`). The page builder is tested slide by slide, the
  assembler is tested for byte stability (`ambre-spec-pdf-export.md`, `PDF-004`), and the two are
  joined by no caller. Same shape as the deck itself (`TOILE-FEAT-026`), one layer further out.
* **The clip is asserted present, never effective** (`TOILE-IO-030`). Two assertions check that a
  slide page carries a clip (`TOILE-IO-042`) and that the fallback page does not (`TOILE-IO-044`).
  Neither reads which rect the clip is, and none puts an element outside the frame and checks that
  the page does not carry it, which is the whole reason the clip exists.
* **A page count is claimed in a comment and asserted nowhere** (`TOILE-IO-039`). The multi-page
  test says "a page count of N produces N pages" and then asserts the `%PDF` header alone: a writer
  that emitted one page for three would pass it. The count that does hold comes from the page
  builder, one layer earlier, where two slides are asserted to yield two pages. Nothing joins the
  two ends: no test hands the builder's pages to the assembler and counts what comes out.
* **A byte length is not a content walk** (`TOILE-IO-038`). The single-page test asserts a file
  longer than 400 bytes and its comment reads that as proof the rect reached the surface. A PDF
  carrying one empty page clears that length on its structure alone, so the assertion separates
  nothing and holds no row.
* **The deck order is written and never read back** (`TOILE-IO-040`). The pagination test creates
  its two frames in deck order and asserts a count, not a sequence; it reads no page's clip, and the
  two frames are otherwise indistinguishable. A builder that sorted the deck by id would pass it,
  and the deck order is the one thing an audience notices.
* **An empty board has no file, and no rule said so before this pass** (`TOILE-IO-045`,
  `TOILE-IO-048`). The builder returns no page for a board with no content and the assembler refuses
  an empty page list, so the two together mean a user with an empty board gets a refusal rather than
  a blank document. Each half is asserted on its own; nothing calls them in sequence, because
  nothing calls either.
* **A picture is a grey box on the page** (`TOILE-IO-031`). The walk paints a placeholder for any
  hash whose pixels it cannot find, and the store it consults is built empty and never filled --
  on screen and on the page alike (`features/ambre-spec-toile-media.md section 6`).
* **Speaker notes reach no page.** The corpus offered them as page annotations. Nothing writes an
  annotation, and the notes exist on the deck already.
