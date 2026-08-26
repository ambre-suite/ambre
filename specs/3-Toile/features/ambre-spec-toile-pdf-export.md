# AMBRE -- FEATURE: PDF EXPORT

**Type:** Vertical (feature)
**Implementation:** partial -- the pages are built and serialized; nothing asks for a file.

> A user hands the deck to an audience that has no Ambre: one page per slide, showing exactly what
> the audience would have seen on screen.

The export is not a conversion of the board: it is the screen renderer aimed at another surface.
That is what makes the page match the screen. The deck it walks is
`features/ambre-spec-toile-presentation.md`, and the other formats are
`features/ambre-spec-toile-import-export.md`.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-IO-005` | A PDF export MUST be byte-stable and MUST produce one page per slide, skipping a slide whose frame is gone. | this spec section 3 | `ambre_toile::logic::interop::pdf_export::tests::export_is_byte_stable`, `::export_yields_a_pdf`, `::multiple_pages_serialize`, `::empty_pages_is_an_error`, `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide`, `::dangling_frame_is_skipped`, `::empty_deck_falls_back_to_one_page`, `::empty_board_yields_no_pages`. The eight compile under the `pdf` feature, which `just test-native` enables | verified |
| `TOILE-IO-030` | A page MUST carry its own slide alone: content outside the frame MUST NOT reach it. | this spec section 3 | none, and the clip is what would hold it: two tests assert that a page carries a clip, and none asserts that the clip removes a neighbouring element -- section 6 | unverified |
| `TOILE-IO-031` | A picture MUST reach the page as its pixels, never as a placeholder box. | this spec section 3 | none, and **the rule fails today**: the walk paints a box for a hash it cannot resolve, and no code fills the store the export reads -- section 6 | unverified |
| `TOILE-IO-032` | An export MUST NOT change the board: it MUST emit no CRDT delta. | this spec section 3 | none. Every page is derived through the read-only camera pass, and a shared reference proves nothing here -- a Loro container mutates through one | unverified |
| `TOILE-IO-033` | A word on a page MUST stay a word: it MUST be selectable and searchable, not only drawn. | this spec section 3; section 5 | none, and **the rule fails today**: each glyph run is written with an empty source range, so the file carries no character mapping -- section 6 | unverified |
| `TOILE-IO-029` | A user MUST be able to export the deck as a PDF. | this spec section 4 | not built -- lands with the "The Toile interop surface" Story (`ambre-roadmap.md` E4); the page builder and the serializer both exist and no gesture calls either | unimplemented |

* **Budget.** the export of 50 slides under 3 s, a row of
  `ambre-spec-budgets.md section 1`. No benchmark measures it, and no gesture starts it
  (`TOILE-IO-029`).
* **Boundary.** the finished file reaches the browser and nothing else. The export runs in the Logic
  thread, opens no socket and reads no key.
* **Verification.** 1 `verified`, 4 `unverified`, 1 `unimplemented`. The one verified row is the one
  the others rest on: the pagination, the deck order and the byte stability are held slide by slide,
  including the empty and the dangling cases. What is loose is what the page shows -- the clip, the
  pixels, the characters -- and none of the three has a reader that could go red, because the export
  has no caller.

---

## 2. WHAT EACH USER SEES

The deck is shared, so two peers export the same slides in the same order. The last column joins the
example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| exports the deck | exports the same deck, untouched | nothing: an export writes nothing (`TOILE-IO-032`) | the same bytes, twice (`TOILE-IO-005`) | `logic::interop::pdf_export::tests::export_is_byte_stable` |
| reorders the deck | exports it | one order, converged | Bob's file follows Alice's order (`TOILE-IO-005`) | `logic::state::presentation::pdf_tests::one_page_per_slide` |
| deletes a frame that is a slide | exports the deck | the dead entry, still in the deck | a file with one page fewer, and a blank slide in playback | `logic::state::presentation::pdf_tests::dangling_frame_is_skipped` |
| exports a board with no deck | watches | nothing | one page framing the whole board (`TOILE-IO-005`) | `logic::state::presentation::pdf_tests::empty_deck_falls_back_to_one_page` |

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

**Steps, when the pages are built:**

1. read the deck in order and drop each entry whose frame is gone (`TOILE-IO-005`);
2. fit each frame's world rect into the page with the padding the on-screen glide uses;
3. derive the content at that camera, without touching the viewer's own camera
   (`TOILE-IO-032`);
4. fall back to one unclipped page framing the whole board when the deck is empty, and to no page at
   all when the board is;
5. clip each page to its frame's rect (`TOILE-IO-030`).

**The clip is load-bearing.** The camera fits a frame by the tighter of the two axes, which leaves
slack on the other one. Without the clip that slack pulls a neighbouring frame's content onto the
page.

**One walk serves the screen and the page.** The export drives the same content walk as the screen
against the PDF surface instead of the GPU one, so a shape, a stroke and a glyph are placed by the
same code in both. The board grid is dropped, as it is in presentation mode.

**Code:**

* `crates/ambre-toile/src/logic/state/presentation.rs` -- `pdf_pages`, the page per slide;
* `logic/interop/pdf_export.rs` -- the page loop, the clip and the assembly;
* `crates/ambre-pdf/src/lib.rs` -- the PDF surface, shared with Trame and Prose.

---

## 4. SURFACE

**There is none** (`TOILE-IO-029`). No menu entry and no chord asks for a file; the File menu carries
one row and it starts the presentation (`features/ambre-spec-toile-menus.md section 4`). Nothing
hands bytes to the browser either.

| Trigger, once it exists | What must happen |
|---|---|
| the File menu export entry | the Logic thread builds the pages and assembles the file |
| the answer | the UI thread hands the bytes to the browser; no board state changes (`TOILE-IO-032`) |

The font context the glyphs need is seeded at boot and read by nothing today, which is what makes
the missing entry visible from the other end: the resource is warm and the caller is absent.

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| The page is drawn by the screen's own walk | a second renderer would drift from the first the day either changed, and the promise of the feature is that the page matches the screen | writing a PDF-specific walk, which duplicates every element kind |
| The export runs in the Logic thread | the walk is pure geometry with no GPU state, and the UI thread owes the visible frame its 16 ms | exporting on the UI thread, which stalls the board for the length of the assembly |
| A page is one slide, clipped to its frame | the audience sees a frame, so the page must show a frame. The camera cannot fit both axes exactly, so without the clip the page shows more than the slide | fitting without a clip, which bleeds the neighbouring board onto the page |
| An empty deck exports the whole board | a user with no deck still wants a file, and the board's own bounds are the only frame available | refusing to export, which turns a missing deck into a missing feature |
| The transition is not exported | a page is static, so an animation between two slides has no representation on it | rendering the intermediate frames as pages, which pads the file with blur |
| The output is vector | a slide printed at any size stays sharp, and the shared surface already emits paths and glyphs | rasterizing each page, which trades every advantage of the format for a simpler writer |
| `krilla` is compiled behind a feature | the writer is large and only the Logic thread needs it, so the default build stays smaller | linking it unconditionally, which pays for it in every build that never exports |

---

## 6. NOT BUILT, AND NOT YET RULED

* **Nothing asks for a file** (`TOILE-IO-029`). The page builder is tested slide by slide, the
  assembler is tested for byte stability, and the two are joined by no caller. Same shape as the
  deck itself (`TOILE-FEAT-026`), one layer further out.
* **The clip is asserted present, never effective** (`TOILE-IO-030`). Two tests check that a page
  carries a clip and that the fallback page does not. None puts an element outside the frame and
  checks that the page does not carry it, which is the whole reason the clip exists.
* **A picture is a grey box on the page** (`TOILE-IO-031`). The walk paints a placeholder for any
  hash whose pixels it cannot find, and the store it consults is built empty and never filled --
  on screen and on the page alike (`features/ambre-spec-toile-media.md section 6`). Every image on
  every slide would export as a box.
* **The characters are not in the file** (`TOILE-IO-033`). Each glyph is written by its font index
  with an empty source range, so the file carries no mapping back to characters: the page looks
  right, and no one can select a word, copy a line or search the document. The corpus described the
  export as keeping "text as text", which is true of the shapes and false of the meaning.
* **Speaker notes reach no page.** The corpus offered them as page annotations. Nothing writes an
  annotation, and the notes exist on the deck already.
* **Lazy loading is not delivered.** The feature flag controls compilation, not loading: the writer
  links into the main module. A second entry point and a dynamic import are a build-system slice
  nobody has scheduled, so this path claims no share of the bundle-splitting budget.
* **No benchmark measures the export.** The 50-slide figure is a budget with no `criterion` bench
  behind it, so nothing fails when the assembly gets slower.
