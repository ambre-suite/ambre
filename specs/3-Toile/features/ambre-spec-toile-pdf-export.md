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

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TOILE-IO-005` | A PDF export MUST produce one page per slide, skipping a slide whose frame is gone. | this spec section 3 | `ambre_toile::logic::state::presentation::pdf_tests::one_page_per_slide`, `::dangling_frame_is_skipped`, `::empty_deck_falls_back_to_one_page`, `::empty_board_yields_no_pages`, under the `pdf` feature that `just test-native` enables | verified |
| `TOILE-IO-030` | A page MUST carry its own slide alone: content outside the frame MUST NOT reach it. | this spec section 3 | none, and the clip is what would hold it: two tests assert that a page carries a clip, and none asserts that the clip removes a neighbouring element -- section 5 | unverified |
| `TOILE-IO-031` | A picture MUST reach the page as its pixels, never as a placeholder box. | this spec section 3 | none, and **the rule fails today**: the walk paints a box for a hash it cannot resolve, and no code fills the store the export reads -- section 5 | unverified |
| `TOILE-IO-029` | A user MUST be able to export the deck as a PDF. | this spec section 4 | not built -- lands with the "The Toile interop surface" Story (`ambre-roadmap.md` E4); the page builder and the serializer both exist and no gesture calls either | unimplemented |

* **Budget.** the export of 50 slides under 3 s, a row of `ambre-spec-budgets.md section 1`. No
  benchmark measures it, and no gesture starts it (`TOILE-IO-029`).
* **Boundary.** the finished file reaches the browser and nothing else, on the shared export path
  (`ambre-spec-pdf-export.md`, `PDF-002`, `PDF-003`).
* **Verification.** 1 `verified`, 2 `unverified`, 1 `unimplemented`. The pagination is held slide by
  slide, empty and dangling cases included. What is loose is what the page shows -- the clip and the
  pixels -- and the caller is absent. The shared rules and their coverage are in
  `ambre-spec-pdf-export.md`.

---

## 2. WHAT EACH USER SEES

The shared scenario -- an export writes nothing, and two exports are the same bytes -- is
`ambre-spec-pdf-export.md section 2`. Toile's own turn on the deck:

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
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

**Steps, when the pages are built** (the shared flow draws and assembles them,
`ambre-spec-pdf-export.md section 3`):

1. read the deck in order and drop each entry whose frame is gone (`TOILE-IO-005`);
2. fit each frame's world rect into the page with the padding the on-screen glide uses;
3. derive the content at that camera, without touching the viewer's own camera (`PDF-002`);
4. fall back to one unclipped page framing the whole board when the deck is empty, and to no page at
   all when the board is;
5. clip each page to its frame's rect (`TOILE-IO-030`).

**The clip is load-bearing.** The camera fits a frame by the tighter of the two axes, which leaves
slack on the other. Without the clip that slack pulls a neighbouring frame's content onto the page.
The board grid is dropped, as it is in presentation mode.

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

## 5. NOT BUILT, AND NOT YET RULED

The shared gaps are in `ambre-spec-pdf-export.md section 6`. Toile's own:

* **Nothing asks for a file** (`TOILE-IO-029`). The page builder is tested slide by slide, the
  assembler is tested for byte stability (`ambre-spec-pdf-export.md`, `PDF-004`), and the two are
  joined by no caller.
* **The clip is asserted present, never effective** (`TOILE-IO-030`). Two tests check that a page
  carries a clip and that the fallback page does not. None puts an element outside the frame and
  checks that the page does not carry it, which is the whole reason the clip exists.
* **A picture is a grey box on the page** (`TOILE-IO-031`). The walk paints a placeholder for any
  hash whose pixels it cannot find, and the store it consults is built empty and never filled --
  on screen and on the page alike (`features/ambre-spec-toile-media.md section 6`).
* **Speaker notes reach no page.** The corpus offered them as page annotations. Nothing writes an
  annotation, and the notes exist on the deck already.
