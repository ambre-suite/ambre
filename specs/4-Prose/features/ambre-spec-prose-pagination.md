# AMBRE -- FEATURE: PAGINATION

**Type:** Vertical (feature)
**Implementation:** partial

> A document becomes a sequence of measured pages. Text is broken where a reader expects it. Never
> a heading alone at the foot of a page, and never one line stranded at the top of the next. Never
> a footnote away from the line that calls it.

Pagination is derived, never stored: it reads the document and produces a value. Two peers editing
the same document run the same computation, so there is no two-peer table below. Whether they
reach the same page breaks is not ruled anywhere, and section 5 says so.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-010` | `INV` | Pagination MUST run in the Logic thread. | this spec section 2 | structural -- `ambre_prose::logic::ProseState::paginate` is the one way to a layout, and it is a method of the state the Logic thread owns | none | verified |
| `PROSE-064` | `INV` | The layout the Logic thread produces MUST be the single source of truth for both the screen and PDF export: a second layout computation MUST NOT exist beside it. | this spec section 2 | structural -- `ambre_prose::layout::place::place` is the one function in the workspace that assigns a page to a block, and `ambre_prose::logic::ProseState::paginate` is the one way a consumer reaches it | none | verified |
| `PROSE-033` | `INV` | The page layout MUST be derived on every read: a page number, a break position or a measured height MUST NOT be read back from a stored copy. | this spec section 2 | `ambre_prose::logic::tests::the_layout_is_derived_and_the_document_is_left_alone` -- it lays the document out twice and requires the document to be what it was | none | verified |
| `PROSE-065` | `INV` | No container MUST hold a page number, a break position or a measured height. | this spec section 2; `ambre-spec-prose-schema.md section 3` | `ambre_prose::logic::schema_shape::every_declared_key_appears_in_the_shape` -- the key set a build writes is held to the frozen shape, which declares no page, no break and no height | none | verified |
| `PROSE-011` | `INV` | Every in-range page index MUST resolve to a list, empty if the page holds nothing: a reader MUST NOT hit a missing entry. | this spec section 2 | `ambre_prose::layout::place::properties::no_block_claims_a_page_out_of_range`; `ambre_prose::layout::place::tests::every_page_index_a_layout_reports_is_one_it_has` -- it asks a page that carries nothing and requires an empty answer | none | verified |
| `PROSE-066` | `INV` | What a page carries MUST include the continuation of a block broken onto it, not the blocks that start there alone. | this spec section 2 | `ambre_prose::layout::place::tests::a_block_taller_than_a_page_splits_and_its_continuation_starts_at_the_top` -- the page the continuation lands on lists it | none | verified |
| `PROSE-067` | `INV` | A layout MUST carry the first page on which it differs from the layout it replaces. | this spec section 2 | `ambre_prose::layout::reflow::tests::the_layout_names_the_first_page_it_changed`; `ambre_prose::layout::reflow::tests::a_reflow_of_an_untouched_document_moves_nothing` -- the second requires no page at all | none | verified |
| `PROSE-016` | `INV` | A header or footer MUST NOT consume more than a quarter of the content height. | this spec section 2 | `ambre_prose::logic::bands::tests::a_band_taller_than_a_quarter_of_the_page_is_clipped_to_it`; `ambre_prose::layout::geometry::tests::a_band_taller_than_a_quarter_is_clipped_and_never_grows_the_page` | none | verified |
| `PROSE-017` | `INV` | A fresh page MUST accept at least one line: the layout MUST NOT produce a degenerate empty page. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::every_block_reaches_a_page`; `ambre_prose::layout::place::tests::a_body_of_zero_places_every_block_and_terminates` | none | verified |
| `PROSE-068` | `INV` | A footnote ordinal MUST be assigned from the position of its reference in the document flow. | this spec section 2 | `ambre_prose::layout::footnote_pages::every_note_is_numbered_from_its_call_before_any_block_is_placed` | none | verified |
| `PROSE-099` | `INV` | A footnote ordinal MUST NOT change when placement moves the note to another page. | this spec section 2 | `ambre_prose::layout::footnote_pages::a_note_moved_to_another_page_keeps_its_ordinal` | none | verified |
| `PROSE-034` | `INV` | Every font family label MUST be resolved to a loaded face before shaping; an unmatched label MUST NOT be handed to the shaper. | this spec section 2; `ambre-spec-render-assets.md`, `CANVAS-017` | `ambre_prose::layout::fonts::tests::a_family_no_face_carries_still_resolves_to_a_shipped_one`; `ambre_prose::layout::fonts::tests::a_monospace_label_falls_back_to_a_monospace_face` | none | verified |
| `PROSE-036` | `INV` | A block's shaped layout MUST be kept once past measurement and handed to the view as that one run: a block no edit touched MUST NOT be shaped again or copied. | this spec section 2; `features/ambre-spec-prose-page-rendering.md section 2` | `ambre_prose::view::tests::a_block_not_edited_hands_the_view_the_run_it_was_measured_into` | none | verified |
| `PROSE-012` | `OBS` | A manual page break MUST never be relaxed: it opens a page whatever else the placement is weighing. | this spec section 2 | `ambre_prose::layout::place::tests::a_break_is_honoured_even_where_the_page_had_room_to_spare`; `ambre_prose::layout::pass::tests::a_manual_break_puts_what_follows_it_on_the_next_page` | none | verified |
| `PROSE-069` | `OBS` | Margin collapse MUST never be skipped: it sits above every heuristic in the constraint order. | this spec section 2 | `ambre_prose::layout::place::tests::a_leading_margin_collapses_against_the_trailing_one_before_it`; `ambre_prose::layout::place::tests::a_leading_margin_is_dropped_at_a_page_top` | none | verified |
| `PROSE-013` | `OBS` | A heading MUST NOT be split across pages: it is pushed whole to the next page. | this spec section 2 | `ambre_prose::layout::place::tests::a_heading_is_never_cut_however_tall_it_is`; proptest -- `ambre_prose::layout::place::properties::a_heading_or_an_image_is_never_split` | none | verified |
| `PROSE-014` | `OBS` | An image MUST NOT be split across pages. | this spec section 2 | `ambre_prose::layout::place::tests::an_image_is_never_cut_and_moves_to_a_page_of_its_own`; proptest -- `ambre_prose::layout::place::properties::a_heading_or_an_image_is_never_split` | none | verified |
| `PROSE-015` | `OBS` | A footnote MUST appear on the same page as the line that references it, through any later relocation of that line. | this spec section 2 | `ambre_prose::layout::footnote_pages::a_note_sits_on_the_page_of_the_line_that_calls_it` | none | verified |
| `PROSE-018` | `OBS` | A block continuation MUST start at the top of its page. | this spec section 2 | `ambre_prose::layout::place::tests::a_block_taller_than_a_page_splits_and_its_continuation_starts_at_the_top` | none | verified |
| `PROSE-070` | `OBS` | A block continuation MUST NOT carry the block's leading margin. | this spec section 2 | `ambre_prose::layout::place::tests::a_leading_margin_is_dropped_at_a_page_top` -- a page top is where every continuation starts | none | verified |
| `PROSE-071` | `OBS` | A block MUST be broken only at a boundary its own type defines -- a text block between two lines, a table between two rows -- and never inside one. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::a_split_cuts_at_rising_line_indices`; `ambre_prose::layout::pass::tests::a_table_is_one_block_and_each_of_its_rows_is_a_break` | none | verified |
| `PROSE-072` | `OBS` | A block broken across pages MUST leave at least two of its lines on the page it breaks from, unless the whole block is three lines or fewer. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::a_cut_leaves_two_lines_on_each_side`; `ambre_prose::layout::place::tests::a_three_line_block_that_overflows_gives_up_the_two_line_rule` -- it requires the relaxation, so a rule held as an absolute fails it | none | verified |
| `PROSE-073` | `OBS` | A block broken across pages MUST carry at least two of its lines onto the page it continues on, unless honouring that would leave fewer than two behind. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::a_cut_leaves_two_lines_on_each_side`; `ambre_prose::layout::place::tests::a_split_never_leaves_fewer_than_two_lines_on_either_side` | none | verified |
| `PROSE-074` | `OBS` | A heading, or a block that asks to stay with its successor, MUST NOT be placed where fewer than two lines of what follows it fit under it, unless moving it would waste over a quarter of the page. | this spec section 2 | `ambre_prose::layout::place::tests::a_heading_moves_rather_than_standing_alone_at_a_page_bottom`; `ambre_prose::layout::place::tests::a_heading_with_room_under_it_stays_where_it_is` | none | verified |
| `PROSE-075` | `OBS` | A block MUST NOT be broken across pages unless no page of its own holds it whole. | this spec section 2 | `ambre_prose::layout::place::tests::a_block_a_whole_page_holds_moves_rather_than_splits`; `ambre_prose::layout::place::tests::a_block_kept_together_and_taller_than_a_page_splits_all_the_same` | none | verified |
| `PROSE-107` | `INV` | Pagination MUST measure on the faces the application ships and on those alone: a face the device happens to carry MUST NOT enter a measurement. | this spec section 2; `ambre-spec-render-assets.md`, `CANVAS-017` | structural -- `ambre_prose::layout::belts::only_the_font_module_asks_a_context_what_it_holds` reads every source of the module and fails on a collection lookup outside `ambre_prose::layout::fonts::resolve`, over a context `PROSE-114` seals | none | verified |
| `PROSE-108` | `INV` | A family label MUST resolve to one shipped face by the label alone: the resolution MUST NOT read the faces a context holds. | this spec section 2; `ambre-spec-render-assets.md`, `CANVAS-017` | type-level -- `ambre_prose::layout::fonts::resolve` takes a label and no context, so no device can reach it; `ambre_prose::layout::fonts::tests::resolution_reads_the_label_and_nothing_else` | none | verified |
| `PROSE-110` | `OBS` | A layout MUST name every script the document uses that had no face when it was measured. | this spec section 2 | `ambre_prose::layout::reflow::convergence::a_latin_document_names_no_missing_script`; `ambre_prose::layout::reflow::convergence::a_script_with_no_face_is_named_in_the_layout` | none | verified |
| `PROSE-115` | `INV` | A document MUST be paginated on every face the application has loaded, whether the document was opened before that face arrived or after it. | this spec section 2; `1-Foundations/ambre-spec-render-assets.md`, `CANVAS-067` | `ambre_logic::dispatch::tests::every_prose_document_takes_the_faces_whenever_it_was_opened` -- it opens one document before the delivery and one after, and reads the face count back off both | none | verified |
| `PROSE-112` | `INV` | A page number that leaves the screen -- a comment anchor, a cross reference, a table-of-contents entry, an exported file -- MUST NOT be taken from a layout that names a missing script. | this spec section 2 | not built | unbuilt -- lands with the `PDF export` Story (`ambre-roadmap.md` E5), the first output that carries a page number off the screen | unimplemented |

### 1.2 Lay the whole document out again

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-076` | `POST` | A full recomputation MUST measure both bands before it places its first block. | this spec section 2 | `ambre_prose::logic::bands::tests::a_shown_band_is_measured_before_the_body_is_placed` | none | verified |
| `PROSE-117` | `POST` | A full recomputation MUST assign every footnote ordinal before it places its first block. | this spec section 2 | `ambre_prose::layout::footnote_pages::every_note_is_numbered_from_its_call_before_any_block_is_placed` | none | verified |
| `PROSE-077` | `POST` | A full recomputation MUST place blocks in the order of a depth-first walk of the block tree: that walk is the document flow. | this spec section 2 | `ambre_prose::layout::pass::tests::the_walk_visits_the_tree_depth_first` | none | verified |
| `PROSE-078` | `POST` | Placement MUST resolve a constraint it cannot meet by moving forward: it MUST NOT revisit a page it has already emitted. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::pages_only_ever_move_forward`; `ambre_prose::layout::place::tests::placement_never_goes_back_to_a_page_it_left` | none | verified |
| `PROSE-079` | `FRAME` | Laying the document out MUST NOT write to the document it measures. | this spec section 2 | `ambre_prose::layout::pass::tests::measuring_the_document_writes_nothing_to_it` -- it compares the whole document before and after, so a read that attaches a container fails it | none | verified |

### 1.3 Reflow after an edit

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-080` | `POST` | A partial reflow MUST produce the layout a full recomputation of the same document would have produced. | this spec section 2 | `ambre_prose::layout::reflow::tests::a_reflow_produces_what_a_full_recomputation_would_have`; proptest -- `ambre_prose::layout::place::properties::a_resumed_walk_lands_where_the_full_one_did` | none | verified |
| `PROSE-081` | `POST` | A partial reflow MUST resume placement at the edited block. | this spec section 2 | proptest -- `ambre_prose::layout::place::properties::a_resumed_walk_lands_where_the_full_one_did`; `ambre_prose::layout::reflow::tests::a_block_inserted_in_the_middle_reflows_the_whole_tail` | none | verified |
| `PROSE-100` | `POST` | A partial reflow MUST stop at the first later block whose page and page-relative position are what they already were. | this spec section 2 | `ambre_prose::layout::reflow::tests::a_reflow_produces_what_a_full_recomputation_would_have` -- the tail it reuses past the stop is compared entry for entry with the tail a full run places | none | verified |
| `PROSE-035` | `POST` | An edit that inserts or removes a footnote reference MUST force a full recomputation: the incremental path MUST NOT leave an ordinal or a page reservation stale. | this spec section 2 | `ambre_prose::layout::footnote_pages::a_call_added_or_removed_lays_out_what_a_full_recomputation_would` | none | verified |
| `PROSE-127` | `POST` | A reflow after text typed alone MUST produce the layout a full recomputation would, the text typed in a note or in a block that calls one included. | this spec section 2 | `ambre_prose::layout::footnote_pages::text_typed_anywhere_lays_out_what_a_full_recomputation_would` | none | verified |
| `PROSE-128` | `POST` | A block re-shaped from the line before an edit, the lines around it kept, MUST break, draw and place its caret as the same block shaped whole. | this spec section 2 | `ambre_prose::layout::measure::tests::a_run_shaped_from_the_edit_on_matches_one_shaped_whole`; `ambre_prose::layout::measure::tests::a_new_width_shapes_the_run_whole`; `ambre_prose::layout::measure::tests::an_edit_near_the_start_shapes_a_few_lines_and_keeps_the_rest`; `ambre_prose::layout::measure::tests::a_mark_changed_past_the_edit_is_shaped_again` | none | verified |
| `PROSE-082` | `POST` | A change to the page box, to either band, to a document default, or a whole-document import MUST force a full recomputation. | this spec section 2 | `ambre_prose::layout::reflow::tests::a_page_box_change_forces_a_full_recomputation`; `ambre_prose::layout::reflow::tests::a_document_default_change_forces_a_full_recomputation`; `ambre_prose::logic::tests::a_snapshot_import_lays_the_document_out_again` | none | verified |
| `PROSE-083` | `FRAME` | A partial reflow MUST NOT re-shape a block whose text and whose resolved style are both unchanged. | this spec section 2 | `ambre_prose::layout::reflow::tests::a_reflow_does_not_shape_a_block_it_was_not_told_about` -- it counts the runs shaped, so a reflow that re-shaped an untouched block fails it | none | verified |
| `PROSE-123` | `POST` | A layout taken after an edit MUST re-shape the blocks the edit wrote, whether the edit is local or arrived from another peer, and whoever named them. | this spec section 2 | `ambre_prose::view::tests::a_block_not_edited_hands_the_view_the_run_it_was_measured_into`; `ambre_prose::view::tests::an_edit_from_another_peer_reshapes_the_block_it_wrote` | none | verified |
| `PROSE-125` | `FRAME` | A layout handed to the view MUST NOT carry a block, a run or a table of a page outside the pages the view asked for. | this spec section 2 | `ambre_prose::view::tests::a_layout_carries_the_pages_asked_for_alone` | none | verified |

### 1.4 Settle the table of contents

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-019` | `POST` | Table-of-contents stabilization MUST run as a bounded multi-pass step. | this spec section 2 | `ambre_prose::layout::reflow::tests::settling_stops_once_the_pages_hold_and_never_runs_past_its_bound` | none | verified |
| `PROSE-084` | `FRAME` | Settling the table of contents MUST NOT run on the path that answers a keystroke. | this spec section 2 | `ambre_prose::layout::reflow::tests::a_reflow_does_not_settle_the_table_of_contents` | none | verified |

### 1.5 Measure the same way on every device

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `PROSE-109` | `POST` | Two peers whose font contexts cover the same scripts MUST paginate one document alike: the same page count, and every block on the same page at the same offset. | this spec section 2 | proptest -- `ambre_prose::layout::reflow::convergence::two_peers_covering_the_same_scripts_paginate_alike` paginates a generated document on two contexts holding different faces | none | verified |
| `PROSE-111` | `POST` | A face registered after a layout was measured MUST force a full recomputation. | this spec section 2 | `ambre_prose::layout::reflow::convergence::a_face_that_arrives_forces_a_full_recomputation` | none | verified |
| `PROSE-113` | `FRAME` | Measuring MUST NOT read anything that tells one peer from another: no peer identity, no clock and no device setting MUST reach a measurement. | this spec section 2 | structural -- `ambre_prose::layout::belts::nothing_identifying_a_peer_reaches_a_measurement` reads every source of the module and fails on a peer id, a clock or a random draw | none | verified |
| `PROSE-114` | `FRAME` | The context a measurement runs on MUST hold only the faces the application registered into it: a face of the device MUST NOT be discoverable from it. | this spec section 2; `ambre-spec-render-assets.md` | type-level -- `ambre_prose::layout::measure::Measurer::new` is the only way to build one and it takes what `ambre_draw::sealed_font_context` returns, which `ambre_draw::text::tests::a_sealed_context_discovers_no_face_of_the_machine` asks for families a desktop carries | none | verified |

* **Budget.** five rows of `ambre-spec-budgets.md section 1`. Three are the full recomputation of
  100 pages, the partial reflow and the layout handoff. Two are cold shapes: ten dirty blocks, and
  the document at open. Shaping and placement are budgeted apart, and section 4 says why.
* **Boundary.** none: pagination runs on the Logic thread, reads the document and returns a value.
  `PROSE-025` (`features/ambre-spec-prose-engine.md section 1`) bans a network call from this crate.
* **Verification.** 50 `verified`, 1 `unimplemented`: `PROSE-112`. The five
  footnote rows run through the engine, since a note is written by its command. Termination rests
  on `PROSE-078` and `PROSE-017`, which a property over generated documents holds together. Two
  bans are structural, held by a test that reads the module's own
  source. The schema holds `PROSE-065`.

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
    pub page_width:        DocPx,
    pub page_height:       DocPx,
    pub content_width:     DocPx,
    pub content_height:    DocPx,
    pub margin_left:       DocPx,
    pub margin_top:        DocPx,
    pub page_count:        PageCount,
    pub blocks:            Vec<BlockLayout>,
    pub header_height:     DocPx,
    pub header_first_height: DocPx,
    pub footer_height:     DocPx,
    pub footer_first_height: DocPx,
    /// The first page this layout differs from the one it replaces on (`PROSE-067`).
    /// `None` when nothing moved.
    pub first_dirty_page: Option<PageIndex>,
    /// The scripts this document uses that no loaded face covered when it was measured
    /// (`PROSE-110`). Empty is the settled case.
    pub missing_scripts: ScriptSet,
    /// The bands as the document held them when this layout was taken.
    pub bands:            doc::Bands,
}

/// Where one block, or one fragment of it, sits.
pub struct BlockLayout {
    pub block_id:    TreeID,
    pub block_type:  BlockType,
    /// The height of the portion on `page_start` alone -- never the whole block when
    /// `is_split`. The later fragments are in `continuation_heights`.
    pub height:      DocPx,
    pub page_start:  PageIndex,
    pub y_in_page:   DocPx,
    pub is_split:    bool,
    /// The first line carried by each page after `page_start`.
    pub split_points: Vec<BlockLine>,
    /// The height of each fragment after `page_start`, one per split point. A
    /// continuation starts at y = 0 (`PROSE-018`) and carries no leading margin
    /// (`PROSE-070`).
    pub continuation_heights: Vec<DocPx>,
    /// The notes this block calls, each on the page its calling line took.
    pub notes:       Vec<FootnoteLayout>,
    /// The height the notes hold at the foot of the block's last page once it is placed.
    pub foot_height: DocPx,
}

/// A footnote placed at the foot of a page.
pub struct FootnoteLayout {
    pub footnote_id:  FootnoteId,
    /// The block whose line calls the note.
    pub block_id:     TreeID,
    /// Sequential across the whole document, assigned in pass 0.
    pub ordinal:      FootnoteOrdinal,
    pub page:         PageIndex,
    pub height:       DocPx,
}

impl PageLayout {
    /// The header and the footer drawn on a page, each with its block and its
    /// alignment. `None` for a hidden side.
    pub fn bands_on_page(&self, page: PageIndex) -> (Option<doc::Band>, Option<doc::Band>);
    /// Every fragment visible on a page, continuations of split blocks included.
    pub fn blocks_on_page(&self, page: PageIndex) -> impl Iterator<Item = &BlockLayout>;
    /// The notes at the foot of a page, in ordinal order. Empty where it carries none.
    pub fn footnotes_on_page(&self, page: PageIndex) -> Vec<&FootnoteLayout>;
}
```

A band names the block it draws, and `bands_on_page` answers which band a page shows. The
renderer paints that block from its shaped runs, as `features/ambre-spec-prose-page-rendering.md
section 2` declares them.

**Pass 0 -- what is measured once per full recomputation.**

1. convert the page box and the four margins to pixels at 96 DPI, and derive the content box;
2. measure the header and the footer against the content width, and the first-page variants when
   the document asks for them. Cap each band at a quarter of the content height (`PROSE-016`), and
   clip a band that exceeds the cap rather than growing the page;
3. keep the shaped runs of both bands, since measuring them required shaping them;
4. collect every footnote, shape it against the content width, and order the set by the position
   of its reference in the document flow. Assign the ordinals in that order, and prefix each text
   with its own ordinal before shaping.

The cap of step 2 keeps the usable height of a page positive, the precondition `PROSE-017` needs
to be satisfiable at all.

**Pass 1 -- what each block costs.** Walk the tree depth-first. For each block resolve its
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

Every block keeps its shaped layout as one shared run, which the view draws and hit-tests without
shaping again (`PROSE-036`). The run costs about 150 KB a page (`ambre-spec-budgets.md section 6`).

Two block kinds need a word more. A **merged table cell** is resolved to a grid rectangle by
locating its two corner blocks in the current row and column order. A row or column inserted
between them widens the rectangle instead of leaving a stale span (`PROSE-009`,
`features/ambre-spec-prose-tables.md`). The merged cell counts toward the first row of its
rectangle. Each later row takes the taller of its own cells and the visible slice of the merge.

A **nested list item** is measured recursively and placed as a block of its own. A parent item
carries no implicit obligation to keep its children with it.

**Pass 2 -- placement, block by block.** Placement is sequential and never backtracks: a
constraint that cannot be met is resolved by moving forward, never by revisiting a page already
emitted. For each block, in order:

1. **a manual break** opens a new page and emits a zero-height entry (`PROSE-012`);
2. **its footnotes are reserved** on the current page, the separator band once per page: 8 pixels,
   rule and gap together. The reservation moves with the block if a later step relocates it
   (`PROSE-015`). A split block reserves each note on the page its calling line takes. A
   reservation that no longer fits beside the content already placed opens a new page;
3. **the leading margin collapses** against the previous block's trailing one, and is dropped
   entirely at a page top;
4. **the block is tested against what is left.** The trailing margin is excluded from that test,
   symmetrically with the leading one being dropped at a page top;
5. **it fits.** A heading, or a block that asks to stay with its successor, looks ahead. If what
   follows it cannot place at least two lines under it, the block moves to the next page. A block
   that still does not fit on a fresh page is placed anyway;
6. **it does not fit.** The unbreakable constraints are tested in the order of the table below,
   each one relaxed only against a geometric impossibility. What survives decides whether the
   block moves whole or splits;
7. **it splits.** A text block splits at a line, never inside one, keeping at least two lines on
   each side of the break. A table splits at a row, never inside a cell, and an image and a heading
   do not split at all (`PROSE-013`, `PROSE-014`). A fresh page that cannot hold two lines of the
   block relaxes the rule rather than opening another page. That is what makes the loop terminate
   (`PROSE-017`).

**Pass 3 -- what only the final page count can decide.** The footnote band of each page is placed
from the bottom of its body area upward, the separator drawn once. The header and footer of each
page are resolved and shaped. The page number is the only field that varies per page, which is
why a band without one is shaped once and shared.

**Settling the table of contents.** A table of contents is measured on the entries it lists, and
each entry carries the page its heading starts on. So its height moves the pages it reports. The
settling pass lays the document out, reads each heading's start page, and lays it out again with
those pages. It stops once two passes read the same pages, and never runs more than three
(`PROSE-019`). An edit reflows with the pages the last settling read and never settles
(`PROSE-084`); the engine settles after a table-of-contents command.

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
block. It stops at the first later block whose page and page-relative position are what they
already were, and from there the tail is unchanged. Seven changes force the whole pipeline
instead: the page box, either band being enabled or disabled, and the document defaults. So do a
full snapshot import and any edit that inserts or removes a footnote reference (`PROSE-035`).

An edit to a note's text forces it too, since it changes the note's reservation.

An edit that only types or erases body text moves no block, changes no setting and adds or removes
no call. The paginator then keeps the note plan and the bands it measured last. It reads them
again only when the edit wrote into a band, a note or a block that calls one (`PROSE-127`). Every
other edit reads them on each reflow.

Inside an edited block, the shaping stops short of the whole paragraph. Lines break greedily, so
an edit cannot move a break two lines above it. The block is shaped again from the line before the
edited one, and keeps the glyphs of the lines above (`PROSE-128`). Once a new line starts where an
old one started past the edit, every break after it is the old one. The window stops there, and
the old lines follow, moved by what the edit added. A new width or style, a mark changed above the
cut, justified or right-to-left text, and an `AtLeast` line height shape it whole.

**The same measurement on every device.** A layout is a function of the text, the page box and
the faces it was measured on. The first two are shared through the CRDT and the third is not, so
the faces are pinned instead. Ambre ships its own set and none is taken from the device.
`ambre-draw` maps a family label to one of them by the label alone (`PROSE-107`, `PROSE-108`).
The renderers reach that same mapping, so the page the UI paints is the page the paginator
measured (`CANVAS-017`).

The measuring context is sealed on top of that: it discovers nothing, and holds only what was
registered into it (`PROSE-114`). Pinning the mapping alone would leave the shaper free. It falls
back to a device face for any codepoint the asked family misses, below every rule the mapping
states.

A sealed context also holds nothing until a face is handed to it. The Logic thread cannot fetch,
so the faces reach it over the bus, and every open document takes them (`PROSE-115`,
`CANVAS-067`). A document opened later takes the same set on its way in.

What is left is script coverage. A face outside the base set is fetched when text needing it
appears (`CANVAS-006`). A peer that has not fetched it yet shapes that text into tofu, and
measures it short. The layout says so rather than hiding it: `missing_scripts` names every script
the document used that had no face (`PROSE-110`). A face arriving afterwards invalidates the whole
layout, since every block of that script re-shapes wherever it sits (`PROSE-111`). Two peers
therefore converge as they fetch, and a page number taken from a layout that names a script is not
one to store (`PROSE-112`).

**Code:** `crates/ambre-prose/src/layout/`. `geometry` reads the page box and `fonts` resolves a
family label to a shipped face. `measure` shapes one block and reports its line heights, `pass`
walks the tree, and `place` is pass 2. `reflow` holds the layout between edits.
`ambre_prose::logic::ProseState::paginate` is the one way in, so the document is measured where it
lives. Pass 0 and pass 3 are not built, and section 5 says what that costs.

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
| Every block keeps its shaped run | keeping the run of a block that splits, and the line heights alone for the rest | the view must then shape again what the measurement just shaped, on the thread that paints or on a second pass here. Keeping the run costs about 150 KB a page, measured over paragraphs of eighty words; bounding it is the `Shaped runs bounded to the view` Story (`ambre-roadmap.md` E7) |
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

* **A table is measured on its text alone.** Cell padding, the two horizontal borders and a
  merged cell's rectangle are in the measurement table of section 2 and in no code. A table
  breaks at a row, which is the rule that matters for a page, but it is measured shorter than it
  draws.
* **A concurrent tree edit re-measures every block once.** A subscription on the document names
  the blocks each commit and each import wrote (`PROSE-123`). While a peer holds a tree operation
  the other has not seen, Loro rewinds the tree on each import and reports every node. The window
  closes when both peers hold each other's operations; until then, a remote edit costs a full pass.
* **Nothing says what the reader is told while a face is still coming.** Two peers agree once
  both hold the faces the document needs, and `missing_scripts` names what a measurement was
  short of (`PROSE-110`, `PROSE-111`). Nothing says whether the one still waiting sees that its
  page numbers are provisional, or reads them as settled.
* **Nothing says that two architectures measure one paragraph to the same pixel.** The peers run
  the same shaper over the same bytes, so the code is not the risk. The arithmetic under it is:
  hinting, curve evaluation and the accumulations of a maths library are free to differ between
  x86-64, aarch64 and wasm32. `PROSE-109` compares two contexts on one host, which says nothing
  about two machines. Ruling it needs a layout produced on one target and replayed on another.
  The repository has no headless wasm run to replay it in (`ambre-roadmap.md` E5).
* **The layout has no version rule of its own.** The engine stamps each layout so the UI thread can
  drop a stale one (`PROSE-UI-016`, `features/ambre-spec-prose-page-rendering.md`). Nothing
  here says what the stamp counts, or that it never goes backwards.
* **Nothing says what the reader sees of a clipped band.** A header taller than a quarter of the
  page is clipped rather than allowed to grow the page (`PROSE-016`). No row states whether
  the overflow is hidden, marked, or reported to the user who wrote it.
* **A note costs every edit something.** Each reflow reads the text of every note to check the
  notes still hold. An edit to one note's text lays the whole document out again. No budget row
  charges either.
  It lands with the `A note edit reflows only its page` Story (`ambre-roadmap.md` E5).
* **A page whose footnotes are taller than the page is not ruled.** The constraint order says a
  footnote is never separated from its reference and that the notes push the content down. A page
  whose notes alone exceed the body height has nowhere left to push, and no row says what gives.
* **A repeated table header row is deferred.** A table split across pages restarts with its body,
  so the reader loses the column names on every page but the first.
* **A table row taller than a page overflows.** It is placed and clipped, and no rule says what the
  reader sees at the clip.
* **The partial reflow has no measured boundary.** An edit near the top of a document whose reflow
  never converges is a full recomputation charged to the full budget. Nothing distinguishes the
  two cases before the pass runs.
* **Line numbers, section breaks, columns and a pageless mode are declared and unbuilt.** Each
  would change what a page is. `meta/line_numbers` and `meta/pageless` are containers the schema
  carries as deferred, and a section break would make the page box vary within one document. Two
  or three columns would make the phases below place blocks across a column set rather than down
  one. All four are the `The later writing set` Story (`ambre-roadmap.md` POST-V1), and none has
  a block type or an attribute in `ambre-spec-prose-schema.md section 3`.
