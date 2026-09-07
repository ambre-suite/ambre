# AMBRE -- FEATURE: PAGINATION

**Type:** Vertical (feature)
**Implementation:** design-only

> A document becomes a sequence of measured pages. Text is broken where a reader expects it: never
> a heading alone at the foot of a page, never one line stranded at the top of the next, never a
> footnote away from the line that calls it.

Pagination is derived, never stored: it reads the document and produces a value. Two peers editing
the same document run the same computation, so there is no two-peer table below -- but whether they
reach the same page breaks is not ruled anywhere, and section 5 says so.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-010` | `INV` | Pagination MUST run in the Logic thread. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-064` | `INV` | The layout the Logic thread produces MUST be the single source of truth for both the screen and PDF export: a second layout computation MUST NOT exist beside it. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-033` | `INV` | The page layout MUST be derived on every read: a page number, a break position or a measured height MUST NOT be read back from a stored copy. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-065` | `INV` | No container MUST hold a page number, a break position or a measured height. | this spec section 2; `ambre-spec-prose-schema.md section 3` | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5); the schema declares no such container, which is the format holding this row before any code does | unimplemented |
| `PROSE-011` | `INV` | Every in-range page index MUST resolve to a list, empty if the page holds nothing: a reader MUST NOT hit a missing entry. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-066` | `INV` | What a page carries MUST include the continuation of a block broken onto it, not the blocks that start there alone. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-067` | `INV` | A layout MUST carry the first page on which it differs from the layout it replaces. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-016` | `INV` | A header or footer MUST NOT consume more than a quarter of the page height. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-017` | `INV` | A fresh page MUST accept at least one line: the layout MUST NOT produce a degenerate empty page. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-068` | `INV` | A footnote ordinal MUST be assigned from the position of its reference in the document flow, and MUST NOT change when placement moves the note to another page. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-034` | `INV` | Every font family label MUST be resolved to a loaded face before shaping; an unmatched label MUST NOT be handed to the shaper. | this spec section 2; `ambre-spec-render-assets.md`, `CANVAS-017` | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-036` | `INV` | A full shaped layout MUST be retained past measurement only for a block that splits across pages; every other block MUST keep its line metrics alone. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-012` | `OBS` | A manual page break MUST never be relaxed: it opens a page whatever else the placement is weighing. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-069` | `OBS` | Margin collapse MUST never be skipped: it sits above every heuristic in the constraint order. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-013` | `OBS` | A heading MUST NOT be split across pages: it is pushed whole to the next page. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-014` | `OBS` | An image MUST NOT be split across pages. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-015` | `OBS` | A footnote MUST appear on the same page as the line that references it, and MUST follow that line through any later relocation. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-018` | `OBS` | A block continuation MUST start at the top of its page. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-070` | `OBS` | A block continuation MUST NOT carry the block's leading margin. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-071` | `OBS` | A block MUST be broken only at a boundary its own type defines -- a text block between two lines, a table between two rows -- and never inside one. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-072` | `OBS` | A block broken across pages MUST leave at least two of its lines on the page it breaks from, unless the whole block is three lines or fewer. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-073` | `OBS` | A block broken across pages MUST carry at least two of its lines onto the page it continues on, unless honouring that would leave fewer than two behind. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-074` | `OBS` | A heading, or a block that asks to stay with its successor, MUST NOT be placed where fewer than two lines of what follows it fit under it, unless moving it would waste over a quarter of the page. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-075` | `OBS` | A block MUST NOT be broken across pages unless no page of its own holds it whole. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.2 Lay the whole document out again

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-076` | `POST` | A full recomputation MUST measure both bands and assign every footnote ordinal before it places its first block. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-077` | `POST` | A full recomputation MUST place blocks in the order of a depth-first walk of the block tree: that walk is the document flow. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-078` | `POST` | Placement MUST resolve a constraint it cannot meet by moving forward: it MUST NOT revisit a page it has already emitted. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-079` | `FRAME` | Laying the document out MUST NOT write to the document it measures. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.3 Reflow after an edit

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-080` | `POST` | A partial reflow MUST produce the layout a full recomputation of the same document would have produced. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-081` | `POST` | A partial reflow MUST resume placement at the edited block and MUST stop at the first later block whose page and page-relative position are what they already were. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-035` | `POST` | An edit that inserts or removes a footnote reference MUST force a full recomputation: the incremental path MUST NOT leave an ordinal or a page reservation stale. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-082` | `POST` | A change to the page box, to either band, to a document default, or a whole-document import MUST force a full recomputation. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-083` | `FRAME` | A partial reflow MUST NOT re-shape a block whose text and whose resolved style are both unchanged. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |

### 1.4 Settle the table of contents

| ID | Kind | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|---|
| `PROSE-019` | `POST` | Table-of-contents stabilization MUST run as a bounded multi-pass step. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |
| `PROSE-084` | `FRAME` | Settling the table of contents MUST NOT run on the path that answers a keystroke. | this spec section 2 | not built -- lands with the "Pagination" Story (`ambre-roadmap.md` E5) | unimplemented |

* **Budget.** the full recomputation of 100 pages, the partial reflow, the cold shape of ten dirty
  blocks, the cold shape at open and the layout handoff: five rows of
  `ambre-spec-budgets.md section 1`. The shaping and placement halves are budgeted apart,
  and section 4 says why.
* **Boundary.** none: pagination runs on the Logic thread, reads the document and returns a value.
  `PROSE-025` (`features/ambre-spec-prose-engine.md section 1`) bans a network call from this crate.
* **Verification.** 35 `unimplemented`, nothing else: `crates/ambre-prose` is a `lib.rs`. One row
  carries a mechanism before the code does -- `PROSE-065` is held by the schema, which declares no
  container for a page number, a break position or a measured height. Termination is not a row of
  its own: `PROSE-078` never reopens a page and `PROSE-017` puts a line on every page opened, and
  together they bound the loop. Twelve of the drawn rows were the summary's promise and nothing
  else -- never a heading stranded, never a line alone -- and a layout fixture holds each of them
  without a browser. The gap this table does not close is stated in section 5: nothing rules on
  whether two peers break pages at the same place.

---

## 2. MODEL

**Containers.** None of its own. Pagination reads `meta/page`, `meta/defaults`,
`meta/header`, `meta/footer`, `blocks`, `styles` and `annots`, all declared in
`ambre-spec-prose-schema.md section 3`. It writes none of them (`PROSE-079`).

**What it reads, and what each input decides:**

| Input | Decides |
|---|---|
| `meta/page` | the page box and the margins, converted at 96 DPI. `PROSE-032` holds its consistency |
| `meta/defaults` | the family, size and line height a block inherits when its style names none |
| `meta/header`, `meta/footer` | the reserved bands, measured once and subtracted from every page |
| `blocks` | the traversal order: a depth-first pre-order walk of the tree is the document flow |
| `styles` | the named paragraph style each block resolves against |
| `annots` | the footnotes to measure, order and place |

**The result** (`ambre-prose`):

```rust
/// Global document layout -- the single source of truth for the UI thread and PDF
/// export (`PROSE-064`). Derived on every recomputation, never persisted.
pub struct PageLayout {
    pub page_width_px:     f32,
    pub page_height_px:    f32,
    pub content_width_px:  f32,
    pub content_height_px: f32,
    pub page_count:        u32,
    pub blocks:            Vec<BlockLayout>,
    pub header_height_px:  f32,
    pub header_first_height_px: f32,
    pub footer_height_px:  f32,
    pub footer_first_height_px: f32,
    /// One entry per page, empty where the page carries no footnote (`PROSE-011`).
    pub footnotes_per_page: Vec<Vec<FootnoteLayout>>,
    pub header_runs:  PageRuns,
    pub footer_runs:  PageRuns,
}

/// Where one block, or one fragment of it, sits.
pub struct BlockLayout {
    pub block_id:    TreeID,
    pub block_type:  BlockType,
    /// The height of the portion on `page_start` alone -- never the whole block when
    /// `is_split`. The later fragments are in `continuation_heights`.
    pub height_px:   f32,
    pub page_start:  u32,
    pub y_in_page:   f32,
    pub is_split:    bool,
    /// Index of the first line carried by each page after `page_start`.
    pub split_points: Vec<u32>,
    /// The height of each fragment after `page_start`, one per split point. A
    /// continuation starts at y = 0 (`PROSE-018`) and carries no leading margin
    /// (`PROSE-070`).
    pub continuation_heights: Vec<f32>,
}

/// A footnote placed at the foot of a page, with its text already shaped.
pub struct FootnoteLayout {
    pub footnote_id:  Uuid,
    /// Sequential across the whole document, assigned in phase 0.
    pub ordinal:      u32,
    pub height_px:    f32,
    pub runs:         Vec<ParleyRunData>,
}

/// Header and footer runs. `Static` when the band carries no page-number field, so one
/// shaped copy serves every page; `PerPage` only when a page number forces one per page.
pub enum PageRuns {
    Static(Vec<ParleyRunData>),
    PerPage(Vec<Vec<ParleyRunData>>),
}

impl PageLayout {
    /// The header runs for a page: the shared slice or the page's own. Empty out of range.
    pub fn header_runs_for(&self, page: usize) -> &[ParleyRunData];
    pub fn footer_runs_for(&self, page: usize) -> &[ParleyRunData];
    /// Every fragment visible on a page, continuations of split blocks included.
    pub fn blocks_on_page(&self, page: u32) -> impl Iterator<Item = &BlockLayout>;
}
```

`ParleyRunData` is declared in `features/ambre-spec-prose-page-rendering.md section 2`: pagination
shapes, the renderer draws, and the shaped run is what crosses between them.

**Pass 0 -- what is measured once per full recomputation.**

1. convert the page box and the four margins to pixels at 96 DPI, and derive the content box;
2. measure the header and the footer against the content width, and the first-page variants when
   the document asks for them; cap each band at a quarter of the content height (`PROSE-016`), and
   clip a band that exceeds the cap rather than growing the page;
3. keep the shaped runs of both bands, since measuring them required shaping them;
4. collect every footnote, shape it against the content width, order the set by the position of its
   reference in the document flow, assign the ordinals in that order, and prefix each text with its
   own ordinal before shaping.

The cap of step 2 is what keeps the usable height of a page positive, which is the precondition
`PROSE-017` needs to be satisfiable at all.

**Pass 1 -- what each block costs.** Walk the tree depth-first, and for each block resolve its
effective style, deduct its indents from the content width, and measure against what its type is:

| Block type | Measured as |
|---|---|
| paragraph, heading, list item, code block | the shaped lines, plus the leading and trailing margins |
| image, placed as a block | the image box, plus its caption when it has one |
| image, floating left or right | as a block image. Text wrapping is not built (section 5) |
| table | per row, the tallest cell including its padding and its two horizontal borders |
| horizontal rule | one pixel, plus the margins |
| page break | zero: it is a signal, not a sized block |
| table of contents | the qualified headings, each counted for the number of lines its title wraps to |

Only a block that later splits keeps its full shaped layout; every other block keeps the per-line
heights alone (`PROSE-036`), which is what bounds the transient cost of the pass.

Two block kinds need a word more. A **merged table cell** is resolved to a grid rectangle by
locating its two corner blocks in the current row and column order, so a row or column inserted
between them widens the rectangle instead of leaving a stale span (`PROSE-009`,
`features/ambre-spec-prose-tables.md`); the merged cell counts toward the first row of its
rectangle, and each later row takes the taller of its own cells and the visible slice of the merge.
A **nested list item** is measured recursively and placed as a block of its own: a parent item
carries no implicit obligation to keep its children with it.

**Pass 2 -- placement, block by block.** Placement is sequential and never backtracks: a
constraint that cannot be met is resolved by moving forward, never by revisiting a page already
emitted. For each block, in order:

1. **a manual break** opens a new page and emits a zero-height entry (`PROSE-012`);
2. **its footnotes are reserved** on the current page, the separator band once per page, and the
   reservation moves with the block if a later step relocates it (`PROSE-015`). A reservation that
   no longer fits beside the content already placed opens a new page;
3. **the leading margin collapses** against the previous block's trailing one, and is dropped
   entirely at a page top;
4. **the block is tested against what is left.** The trailing margin is excluded from that test,
   symmetrically with the leading one being dropped at a page top;
5. **it fits.** A heading, or a block that asks to stay with its successor, looks ahead: if what
   follows it cannot place at least two lines under it, the block moves to the next page. A block
   that still does not fit on a fresh page is placed anyway;
6. **it does not fit.** The unbreakable constraints are tested in the order of the table below,
   each one relaxed only against a geometric impossibility, and what survives decides whether the
   block moves whole or splits;
7. **it splits.** A text block splits at a line, never inside one, keeping at least two lines on
   each side of the break; a table splits at a row, never inside a cell; an image and a heading do
   not split at all (`PROSE-013`, `PROSE-014`). A fresh page that cannot hold two lines of the
   block relaxes the rule rather than opening another page, which is what makes the loop terminate
   (`PROSE-017`).

**Pass 3 -- what only the final page count can decide.** The footnote band of each page is placed
from the bottom of its body area upward, the separator drawn once; the header and footer of each
page are resolved and shaped, with the page number the only field that varies per page -- which is
why a band without one is shaped once and shared.

**The constraint order.** When two rules cannot both hold, the higher row wins and the lower one is
relaxed. The two absolutes are `PROSE-012`.

| Priority | Constraint | Relaxed |
|---|---|---|
| 1 | a manual page break | never |
| 2 | a footnote on the page of its reference | never today: the footnote pushes the content down |
| 3 | two lines minimum at a page bottom | when the block is three lines or fewer and overflows |
| 4 | two lines minimum at a page top | when honouring it would break priority 3 |
| 5 | a block kept whole | when the block is taller than a whole page |
| 6 | a heading, or a block kept with its successor | when moving the predecessor would waste over a quarter of the page |
| 7 | margin collapse | never |

**Incremental invalidation.** An edit re-measures its own block, and stops there when the height is
unchanged and no footnote reference moved. When the height changes, placement resumes from that
block and stops at the first later block whose page and page-relative position are what they
already were: from there the tail is unchanged. Seven changes force the whole pipeline instead --
the page box, either band being enabled or disabled, the document defaults, a full snapshot
import, and any edit that inserts or removes a footnote reference (`PROSE-035`).

**Code:** none. `crates/ambre-prose/src/lib.rs` is seven lines.

---

## 3. SURFACE

Pagination has no gesture of its own. What reaches the user is the page column
(`features/ambre-spec-prose-viewport.md`) and what is painted on it
(`features/ambre-spec-prose-page-rendering.md`).

| Trigger | Effect |
|---|---|
| any edit that changes a block's height | a partial reflow from that block, and a new layout to the UI thread |
| a page format or margin change | a full recomputation |
| a header or footer enabled, disabled or edited | a full recomputation: the band height changes every page |
| a document default changed -- family, size, line height | a full recomputation |
| a footnote inserted or deleted | a full recomputation, because the ordinals are global (`PROSE-035`) |
| a remote snapshot imported | a full recomputation |
| a heading's page number moving | one deferred table-of-contents pass, off the keystroke path (`PROSE-019`) |

The layout carries the first page it dirtied, so the UI thread repaints from there rather than
everywhere (`features/ambre-spec-prose-page-rendering.md`).

---

## 4. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| Pagination runs in the Logic thread | paginating in the UI thread | the frame budget forbids it there. A full recomputation exceeds a frame, and the render thread must never be blocked by computation |
| Pagination runs in the Logic thread | paginating in the Sync worker | it knows nothing of typography and is reserved for crypto and network. Measuring where the document lives also means measuring once for the screen and for the PDF, which is what makes `PROSE-064` a single source of truth rather than two agreeing implementations |
| Placement never backtracks | a whole-document optimization pass, as a typesetting engine does for a print run | it is quadratic in the block count and its output is unstable: one character typed at the top of a document re-breaks every page below on a different set of trade-offs. Local relaxation gives a break the reader can predict, and an edit whose effect stops where it converges |
| Shaping and placement carry separate budgets | one pagination budget covering the shaping it depends on | it would either be met by a document nobody has and missed by every real one, or so wide it measures nothing. Placement re-assigns pages and vertical positions over already shaped runs, while shaping is a glyph-level pass an order of magnitude more expensive |
| A band is capped rather than allowed to grow the page | growing the page to fit the band | it contradicts the page format the document declares. A header measured taller than the page leaves no usable height, and the placement loop then opens pages forever without placing anything; a cap turns an unbounded loop into a clipped band the user can see and fix |
| The constraint order is written and ranked | stating the constraints as absolutes, and discovering the resolution order by reading the implementation | an unranked list is resolved by whichever branch the code reaches first, and that is not reviewable. Every one of these rules is violable by a document that is geometrically impossible, so what matters is not the list but which rule yields first |
| Footnote ordinals are global and assigned before placement | numbering per page, as a footnote band suggests | it renumbers a note that merely got pushed down. An ordinal is what the reader joins the mark to the note by, so it must be stable across the pages the note may move between |
| A table-of-contents pass is deferred, bounded and never inline | iterating to a fixed point | a pathological document turns it into a hang on the keystroke path. The entries carry page numbers, so stabilizing them can change the page count that produced them; three iterations converge in practice and the cap makes the non-convergent document cheap rather than fatal |
| Floating images are placed as block images | shipping an approximation of the wrap | it puts glyphs under the image. Text wrapping needs line breaking with exclusion zones, which the shaper does not offer; placing them as blocks is visibly wrong and never incorrect |

---

## 5. NOT BUILT, AND NOT YET RULED

* **Nothing rules on whether two peers break pages at the same place.** The layout is derived from
  the document and the page format, both shared -- but it is also derived from the faces actually
  loaded on the device, and family resolution maps an unknown label to whichever base face is
  there (`PROSE-034`). A collaborator with a different font set can measure the same paragraph
  differently and land on a different page count. That matters the moment a page number leaves the
  screen: a comment naming a page, a cross reference to one, or two users reading the same PDF.
  No row states the guarantee and no decision refuses it.
* **The layout has no version rule of its own.** The engine stamps each layout so the UI thread can
  drop a stale one (`PROSE-UI-016`, `features/ambre-spec-prose-page-rendering.md`), and nothing
  here says what the stamp counts or that it never goes backwards.
* **Nothing says what the reader sees of a clipped band.** A header taller than a quarter of the
  page is clipped rather than allowed to grow the page (`PROSE-016`), and no row states whether
  the overflow is hidden, marked, or reported to the user who wrote it.
* **A page whose footnotes are taller than the page is not ruled.** The constraint order says a
  footnote is never separated from its reference and that the notes push the content down; a page
  whose notes alone exceed the body height has nowhere left to push, and no row says what gives.
* **A repeated table header row is deferred.** A table split across pages restarts with its body,
  so the reader loses the column names on every page but the first.
* **A table row taller than a page overflows.** It is placed and clipped, and no rule says what the
  reader sees at the clip.
* **The partial reflow has no measured boundary.** An edit near the top of a document whose reflow
  never converges is a full recomputation charged to the full budget, and nothing distinguishes the
  two cases before the pass runs.
* **Line numbers, section breaks, columns and a pageless mode are declared and unbuilt.** Each
  would change what a page is: `meta/line_numbers` and `meta/pageless` are containers the schema
  carries as deferred, a section break would make the page box vary within one document, and two
  or three columns would make the phases below place blocks across a column set rather than down
  one. All four are the "The later writing set" Story (`ambre-roadmap.md`), and none of the four has
  a block type or an attribute in `ambre-spec-prose-schema.md section 3`.
