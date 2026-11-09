# AMBRE -- FEATURE: CELL MERGES
**Type:** Vertical (feature)
**Implementation:** partial -- the grid behaviour is built; the file round trip is not.

> A user joins a rectangle of cells into one block. The block shows the top-left cell's value and
> spans the whole rectangle. Merging never destroys data, including when a second user writes into
> the rectangle at the same moment.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. The grammar, the
eight kinds and the status values are normative in `ambre-spec-template.md`.

### 1.1 The state

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-125` | `INV` | Every merge MUST designate the top-left cell of its rectangle as the master. | `ambre-spec-trame-schema.md` section 3 | `ambre_trame::logic::tests::group2::merge_marks_master_span_and_blanks_covered` | none | verified |
| `TRAME-128` | `INV` | Every merge MUST span at least two cells. | this spec section 4 | `ambre_trame::logic::tests::group3::merge_selection_single_cell_is_noop` -- the local path; `ambre_trame::logic::tests::group11::a_one_cell_merge_from_a_peer_is_no_merge` -- an entry of one cell from a peer draws and masks nothing | none | verified |
| `TRAME-020` | `INV` | A merge-masked cell MUST NOT participate in any computation, on any sheet, whichever tab is active. | this spec section 4 | `ambre_trame::logic::tests::group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` | none | verified |
| `TRAME-029` | `CONC` | Every peer MUST hide the same set of cells, computed from the merges and the cell contents alone. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest whose op stream draws merges and unmerges, over an asserted grid carrying each cell's displayed text, so two peers hiding different cells fail the case | none | verified |
| `TRAME-126` | `CONC` | Two overlapping merges MUST resolve to the same single merge on every peer. | this spec section 5 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` -- a proptest whose op stream draws 2x2 merges at arbitrary anchors, so overlapping pairs occur, over a compared window carrying each cell's merge span | none | verified |
| `TRAME-144` | `CONC` | The surviving merge MUST be the one whose master key sorts first. | this spec section 5 | `ambre_trame::logic::tests::group3::overlapping_merges_resolve_to_the_lowest_master_key` -- it derives the expected winner from the two master keys, so it asserts the order and not the positions of one run | none | verified |
| `TRAME-127` | `INV` | A merge MUST cover the same cells after a row insert above it. | this spec section 2, row 3 | `ambre_trame::logic::tests::group3::merge_follows_structural_insert` | none | verified |
| `TRAME-028` | `OBS` | A cell hidden by a merge MUST show nothing while the merge lasts. | this spec section 2, row 1 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored`; `ambre_trame::ui::renderer::tests::draw_cells_renders_merge_master_over_its_span` | none | verified |
| `TRAME-UI-112` | `OBS` | The fill handle MUST sit on the bottom-right corner of the merged block. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::fill_handle_sits_on_merge_extent_corner` | none | verified |
| `TRAME-IO-016` | `IO` | Merges MUST survive an import and export round trip. | this spec section 6 | not built | unbuilt -- lands with the `xlsx interop through the OOXML module` Story (`ambre-roadmap.md` E6) | unimplemented |

### 1.2 Merge cells

The user joins the selected rectangle into one block.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-130` | `POST` | The rectangle MUST carry exactly one merge entry, keyed by its top-left cell. | this spec section 3 | `ambre_trame::logic::tests::group2::merge_marks_master_span_and_blanks_covered` | none | verified |
| `TRAME-027` | `FRAME` | A merge MUST NOT remove or change the text of a cell it covers. | this spec section 2, row 1 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` | none | verified |
| `TRAME-131` | `FRAME` | A merge MUST NOT change a cell outside its rectangle. | derived | `ambre_trame::logic::tests::group11::a_merge_leaves_its_neighbours` -- it reads every cell around the rectangle | none | verified |
| `TRAME-019` | `FAIL` | A merge over a non-empty covered cell MUST NOT reach the document. | this spec section 5 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` | none | verified |
| `TRAME-132` | `FAIL` | A refused merge MUST tell the local user it was refused. | this spec section 4 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty` | none | verified |
| `TRAME-148` | `FAIL` | A merge whose corner cell cannot resolve MUST write no entry. | this spec section 6 | `ambre_trame::logic::tests::group11::a_merge_past_the_cap_writes_no_entry` | none | verified |
| `TRAME-133` | `FAIL` | A single-cell selection MUST leave the document unchanged. | this spec section 4 | `ambre_trame::logic::tests::group3::merge_selection_single_cell_is_noop` | none | verified |

### 1.3 Unmerge the selection

The user splits every block the selection crosses. The gesture carries no geometry: the Logic
thread resolves it against the selection it tracks, so the active cell may sit outside every block.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-145` | `POST` | Every merge the selection crosses MUST be split, including one the selection only clips. | this spec section 4 | `ambre_trame::logic::tests::group3::unmerge_active_splits_every_merge_in_selection`; `::unmerge_selection_grows_to_cover_straddling_merge` | none | verified |
| `TRAME-135` | `POST` | The merge entry MUST be gone from the document. | this spec section 3 | `ambre_trame::logic::tests::group2::unmerge_restores_covered_cells` | none | verified |
| `TRAME-030` | `POST` | Removing a merge MUST make every value it hid readable again, with no further edit. | this spec section 2, row 2 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored`; `ambre_trame::logic::tests::group2::unmerge_restores_covered_cells` | none | verified |
| `TRAME-146` | `POST` | A value the merge hid MUST take part in computation again. | this spec section 2, row 2 | `ambre_trame::logic::tests::group11::an_unmerge_returns_a_masked_value_to_the_formulas` -- a formula over the freed cell reads its value | none | verified |
| `TRAME-UI-113` | `POST` | The selection after the split MUST cover every cell the split freed. | this spec section 4 | `ambre_trame::logic::tests::group3::unmerge_selection_grows_to_cover_straddling_merge`; `::unmerge_two_vertical_merges_selected_by_masters` | none | verified |
| `TRAME-136` | `FRAME` | An unmerge MUST NOT change the text of any cell. | this spec section 5 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` | none | verified |
| `TRAME-137` | `FAIL` | A selection crossing no merge MUST leave the document unchanged. | this spec section 4 | `ambre_trame::logic::tests::group11::an_unmerge_crossing_no_merge_writes_nothing` | none | verified |

### 1.4 Conflict detection

The Logic thread observes a covered cell that carries a value, on the local path and on the remote one.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-138` | `POST` | A merge covering a cell that carries a value MUST emit one conflict notice naming that cell. | this spec section 2, row 1 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` | none | verified |
| `TRAME-031` | `FRAME` | Detecting a merge conflict MUST NOT write to the document. | this spec section 5 | `ambre_trame::logic::tests::group11::an_unmerge_returns_a_masked_value_to_the_formulas` -- the peer that detects the conflict writes no operation | none | verified |

### 1.5 Fill from a merged source

The user drags the fill handle from a source tiled by equal merged blocks.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-140` | `POST` | The filled region MUST be tiled by merged blocks the size of the source block. | this spec section 2, row 6 | `ambre_trame::logic::tests::group6::fill_merged_source_tiles_and_remerges_blocks`; `::fill_multi_merged_source_continues_pattern` | none | verified |
| `TRAME-141` | `POST` | A numeric series MUST progress once per block, never once per cell. | this spec section 4 | `ambre_trame::logic::tests::group6::fill_merged_series_increments_per_block` | none | verified |
| `TRAME-147` | `POST` | A fill MUST clear every covered cell of each destination tile. | this spec section 4 | `ambre_trame::logic::tests::group11::a_merged_fill_clears_its_tiles_and_keeps_its_source` -- a value under a tile is emptied, and none is masked | none | verified |
| `TRAME-149` | `POST` | Each tile master MUST take the style of the source block master. | this spec section 4 | `ambre_trame::logic::tests::group11::a_merged_fill_carries_the_style_and_shifts_the_formula` | none | verified |
| `TRAME-150` | `POST` | A formula copied into a tile MUST reindex by the tile offset. | this spec section 4 | `ambre_trame::logic::tests::group11::a_merged_fill_carries_the_style_and_shifts_the_formula` | none | verified |
| `TRAME-UI-116` | `POST` | The drag preview MUST extend by whole blocks along the drag axis. | this spec section 4 | `ambre_trame::ui::view::view_tests::a_merged_fill_previews_whole_blocks` -- a pointer one row and three rows past a two-row merge | none | verified |
| `TRAME-142` | `FRAME` | A fill MUST NOT change the source blocks. | derived | `ambre_trame::logic::tests::group11::a_merged_fill_clears_its_tiles_and_keeps_its_source` -- the source block reads as before | none | verified |
| `TRAME-143` | `FAIL` | A source mixing block sizes MUST fall back to the cell-by-cell fill. | this spec section 4 | `ambre_trame::logic::tests::group11::a_source_of_unequal_blocks_fills_cell_by_cell` | none | verified |

### 1.6 Move the selection over a block

The user drags a selection across a block, or steps into one with an arrow key. A block answers as
one cell to both.

| ID | Kind | Statement (RFC 2119) | Sources | Mechanism | Gap | Status |
|---|---|---|---|---|---|---|
| `TRAME-UI-009` | `POST` | A selection touching a merged block MUST grow to the whole block. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::drag_selection_expands_to_whole_merge`; `::merge_block_selects_as_a_whole_and_nav_steps_over_it` | none | verified |
| `TRAME-UI-111` | `POST` | An arrow step MUST land past a merged block, never inside it. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::merge_block_selects_as_a_whole_and_nav_steps_over_it` -- its navigation half, which asserts the landing cell beyond the block | none | verified |
| `TRAME-UI-114` | `POST` | The active cell MUST rehome to the master of the block it lands on. | this spec section 4 | `ambre_trame::ui::grid::tests::group2::snap_active_to_master_rehomes_after_merge` | none | verified |
| `TRAME-UI-115` | `FRAME` | Moving the selection MUST NOT change the document. | derived | `ambre_trame::logic::tests::group11::a_selection_move_writes_nothing` | none | verified |

---

* **Budget.** none: a merge writes one small container and masks at read time, and
  `ambre-spec-budgets.md` budgets no merge operation.
* **Boundary.** none of its own: the merge is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 37 `verified`, 1 `unimplemented`. The split into operations exposed eleven of
  the thirteen loose rows. Three are frame conditions no test isolates (`TRAME-131`, `TRAME-142`,
  `TRAME-UI-115`), and three are guards no test refuses (`TRAME-137`, `TRAME-143`, `TRAME-148`).
  Four are rules the fill states and asserts nothing about (`TRAME-146`, `TRAME-147`, `TRAME-149`,
  `TRAME-150`), and one is the drag preview (`TRAME-UI-116`). The twelfth, `TRAME-031`, was the one
  gap the document already named; the thirteenth, `TRAME-128`, is what the `Gap` column exposed.

---

## 2. WHAT EACH USER SEES

Two peers, one rectangle. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| merges A1:A2, both empty on her side | writes `concurrent` into A2 at the same moment | the merge and Bob's text, both intact | one empty block over A1:A2, and a notice naming A2 as covered | `logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` |
| unmerges A1:A2 | -- | the merge is gone, Bob's text untouched | A2 reads `concurrent` again | same test |
| merges A1:A2 | inserts a row above row 1 | the merge, now spanning A2:A3 | the block one row lower, holding the same value | `logic::tests::group3::merge_follows_structural_insert` |
| merges A1:B2 | merges B2:C3, overlapping at B2 | both entries, until validation drops one | one block; which one is decided by the ascending master key, not by either user | `logic::tests::group3::overlapping_merges_resolve_to_the_lowest_master_key` |
| merges A1:A2 while A2 holds `12` on her side | -- | nothing: no merge is written | Alice is told the merge was refused; Bob sees no change | `logic::tests::group2::merge_rejected_when_covered_cell_non_empty` |
| drags the fill handle from a 2x1 merged block | -- | one merge entry per filled tile | the filled region tiled by blocks the size of the source | `logic::tests::group6::fill_merged_source_tiles_and_remerges_blocks`, `::fill_merged_series_increments_per_block` |

---

## 3. MODEL

**Containers.** `merges_{sheet}` alone, declared in `ambre-spec-trame-schema.md section 3`: the
top-left cell keys the entry, the bottom-right cell is its value. Nothing else about a merge is
persisted -- no covered-cell list, no hidden flag.

**Commands and events** (`ambre-bus`, `features/ambre-spec-trame-context-menu.md section 2`):

| Shape | Direction | Carries |
|---|---|---|
| `TrameCommand::MergeCells { rect }` | UI to Logic | the selection bounds |
| `TrameCommand::UnmergeCells { row, col }` | UI to Logic | one cell inside the block |
| `StaticKey<Commands>::MergeSelection` / `UnmergeActive` | chrome to Logic | no geometry: the Logic thread resolves it against the selection it tracks |
| `TrameEvent::MergeRejected` | Logic to UI | the local refusal, surfaced as a toast |
| `TrameEvent::MergeConflictDetected { master_cell, covered_cell }` | Logic to UI | one conflict, on the local path and on the remote one |

**Engine state.** One field of `TrameState`:

```rust
/// Keys of the covered cells that still carry a value. Derived on every
/// merge change, never persisted (`TRAME-029`).
masked_cells: HashSet<String>,
```

**Display shape.** The master carries its span; a covered cell carries none and draws nothing.

```rust
DisplayCell { is_merged: true, merge_span: Some((col_span, row_span)), /* master */ }
DisplayCell { is_merged: true, merge_span: None, display_text: String::new() }
```

**Steps, on every merge change:**

1. resolve the sheet's entries into the merges that survive: ascending master key, skipping any
   rectangle that intersects one already placed;
2. for each covered cell that still carries raw text, add its key to `masked_cells` and emit one
   conflict event;
3. feed the dependency graph and the formula cache from the cells outside that set;
4. on an unmerge, drop the keys, invalidate the bytecode and propagate.

**Code:**

* `crates/ambre-trame/src/logic/merges.rs` -- entries, split, overlap resolution, conflict
  detection;
* `logic/fill.rs` -- `merged_source_block`, `fill_merged`;
* `ui/grid/` -- `expand_to_merges`, the fill handle;
* `ambre-app/src/trame/renderer.rs` -- `draw_cell` skips a covered cell.

---

## 4. SURFACE

Everything the user touches. The grid machinery modified here is specified elsewhere:
`features/ambre-spec-trame-selection.md`, `features/ambre-spec-trame-autofill.md`,
`features/ambre-spec-trame-context-menu.md` and `features/ambre-spec-trame-cell-rendering.md`.

**Triggers.**

| Trigger | Sends | Behaviour |
|---|---|---|
| context menu, "Merge cells" | `StaticKey<Commands>::MergeSelection` | over the selection the Logic thread tracks. A single cell is a silent no-op; a non-empty covered cell replies `MergeRejected`, shown as a toast |
| context menu, "Unmerge cells" | `StaticKey<Commands>::UnmergeActive` | over the selection the Logic thread tracks: every merge it crosses is split, and the selection grows over every freed cell. A selection crossing no merge is a no-op |
| toolbar, the merge button | `StaticKey<Commands>::MergeSelection` | the same path: the `ToolbarAction::Merge` control reuses `merge_selection` |

Both carry no geometry. The DOM context menu dispatches a bare `StaticKey<Commands>`, and the Logic thread
resolves it against the active cell and selection it already tracks.

**Selection, navigation and rendering** (`TRAME-UI-009`):

* a selection rectangle grows to whole blocks (`expand_to_merges`), so a drag touching part of a
  block selects the block as one cell;
* the active cell rehomes to the master (`snap_active_to_master`);
* the fill handle sits on the block's bottom-right corner;
* arrow-key navigation steps over a block rather than into it;
* the master draws over its whole span, and `draw_cell` skips a covered cell before it fills any
  background. A covered cell paints nothing, neither text nor fill.

**Fill from a merged source (Excel behaviour).** When the source is tiled by equal merged blocks --
one block, or a uniform grid of identically sized ones -- the fill is block-aware:

1. the UI captures the merge-expanded source and its block size (`fill_source_block`, which checks
   the whole rectangle is a uniform tiling);
2. the drag preview snaps the extension to a whole number of blocks along the locked axis
   (`fill_dest`: a 2x1 block fills two rows);
3. on release the Logic thread tiles the blocks (`logic::fill::merged_source_block`,
   `fill_merged`). Each lane perpendicular to the drag axis extrapolates from its own stack of
   source blocks. A numeric `Series` progresses across blocks, otherwise the source blocks are
   copied cyclically and a copied formula reindexes by the tile offset;
4. each tile master takes the value and its source block's style. The tile's covered cells are
   cleared, and a merge entry is recorded per tile. The filled region is re-merged into blocks the
   size of the source.

The `Ctrl` single-against-multi test counts **blocks**, not raw cells, along the drag axis. One
block is one value (`Ctrl` gives `Series` +1), several blocks are several values (`Ctrl` gives
`Copy`). A plain source, or a mix of block sizes, keeps the per-cell fill unchanged.

**Events reaching the user.** `MergeRejected` and `MergeConflictDetected` are chrome-only: the
canvas ignores them, the chrome relay turns each into a notice (`trame-engine`).

---

## 5. REJECTED ALTERNATIVES

| Decision | Rejected alternative | Why not |
| :--- | :--- | :--- |
| A local merge is refused while a covered cell is non-empty | merging anyway and dropping the covered values | the user loses data they can still see. Excel and Google Sheets both refuse it. |
| A concurrent write under a merge is masked, never deleted | dropping the remote write | refusing a remote operation breaks convergence, and the data is the user's. |
| A concurrent write under a merge is masked, never deleted | asking the user to choose | the write arrives while nobody is looking at it, and a prompt makes convergence wait on a person who may be away. |
| The hidden set is derived, never stored | a `masked` flag written into the document beside each cell | a stored set is a second source of truth that two peers can compute apart. |
| Detecting a conflict writes nothing back | emitting a delete or a move for the covered value | a corrective write is itself an operation, so it can conflict and start a second round. |
| An overlap resolves by ascending master key | asking the user | an overlap is discovered on merge, on every peer independently, so a prompt fires once per peer for one conflict and each answer is a new write. |
| An overlap resolves by ascending master key | refusing both merges | it drops two intents to resolve one collision, and leaves the document in a state neither user asked for. The master key is stable and every peer derives the order from the document alone, so no coordination is needed: one function, `resolved_merges`, is read by the display caches and by the formula feed alike, where a second copy would let a masked cell differ from the block the grid draws. |
| A fill from a uniform merged source tiles whole blocks | filling cell by cell and leaving the destination unmerged | a per-cell fill from a merged source produces blocks the user did not ask for. |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The file round trip** (`TRAME-IO-016`): xlsx import and export carry no merge yet.
* **A merge that resolves no corner says nothing** (`TRAME-148`). The write returns before the map
  insert and emits no event, so the user sees a gesture that did nothing. `TRAME-132` requires a
  notice for the refused merge and no rule covers this path.
* **No test asserts isolation** (`TRAME-131`, `TRAME-142`, `TRAME-UI-115`): each one reads the
  rectangle it wrote. A write landing on a neighbouring cell, back on a fill source, or reaching
  the document from a selection move, passes today.
* **Two guards have no refusal test** (`TRAME-137`, `TRAME-143`). The unmerge over a selection
  crossing no block and the fill from a source of unequal blocks both return early. Nothing
  exercises either path.
* **A merge over a filtered or hidden row** is unspecified: no rule says whether the block spans the
  hidden row or closes over it.
