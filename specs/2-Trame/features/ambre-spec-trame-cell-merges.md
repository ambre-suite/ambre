# AMBRE -- FEATURE: CELL MERGES
**Type:** Vertical (feature)
**Implementation:** partial -- the grid behaviour is built; the file round trip is not.

> A user joins a rectangle of cells into one block. The block shows the top-left cell's value and
> spans the whole rectangle. Merging never destroys data, including when a second user writes into
> the rectangle at the same moment.

---

## 1. CONTRACT

The only mandatory read. Stated in user terms: no container, no type, no function. Grammar and
status values are normative in `ambre-spec-template.md`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-019` | A merge MUST be refused locally while a covered cell is non-empty, and an unmerge MUST restore every covered value. | this spec section 5 | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty`, `::merge_marks_master_span_and_blanks_covered`, `::unmerge_restores_covered_cells`, `group3::concurrent_value_under_merge_is_masked_then_restored` | verified |
| `TRAME-020` | A merge-masked cell MUST NOT participate in any computation, on any sheet, whichever tab is active. | this spec section 4 | `ambre_trame::logic::tests::group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` | verified |
| `TRAME-027` | A merge MUST NOT remove or change the text of a cell it covers. | this spec section 2, row 1 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` | verified |
| `TRAME-028` | A cell hidden by a merge MUST show nothing while the merge lasts. | this spec section 2, row 1 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored`, `ambre_trame::ui::renderer::tests::draw_cells_renders_merge_master_over_its_span` | verified |
| `TRAME-029` | Every peer MUST hide the same set of cells, computed from the merges and the cell contents alone. | this spec section 2, row 1 | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` (proptest): its op stream mints merges and unmerges, and the asserted grid carries each cell's displayed text, so two peers hiding different cells fail the case | verified |
| `TRAME-030` | Removing a merge MUST make every value it hid readable and computable again, with no further edit. | this spec section 2, row 2 | `ambre_trame::logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored`, `logic::tests::group2::unmerge_restores_covered_cells` | verified |
| `TRAME-031` | Detecting a merge conflict MUST NOT write to the document. | this spec section 5 | none. The detection path only fills an in-memory set, and the convergence proptest would catch a corrective write that diverged -- but nothing asserts the absence of the write itself, which is what keeps a conflict from starting a second round | unverified |
| `TRAME-UI-009` | A merged block MUST select as a whole, and navigation MUST step over it rather than into it. | this spec section 4 | `ambre_trame::ui::grid::tests::group1::merge_block_selects_as_a_whole_and_nav_steps_over_it`, `::drag_selection_expands_to_whole_merge`, `::fill_handle_sits_on_merge_extent_corner`, `ambre_trame::ui::grid::tests::group2::snap_active_to_master_rehomes_after_merge` | verified |
| `TRAME-IO-016` | Merges MUST survive an import and export round trip. | this spec section 6 | not built -- lands with the "xlsx interop" Story (`ambre-roadmap.md` E6) | unimplemented |

* **Budget.** none: a merge writes one small container and masks at read time, and
  `ambre-spec-budgets.md` budgets no merge operation.
* **Boundary.** none of its own: the merge is a document write, and `ZK-001` seals every payload
  before it leaves the client.
* **Verification.** 7 `verified`, 1 `unverified`, 1 `unimplemented`. `TRAME-031` is the gap. Two
  behaviours below carry no row at all and are named in section 6.

---

## 2. WHAT EACH USER SEES

Two peers, one rectangle. The last column joins the example to the test that holds it.

| Alice does | Bob does | The document holds | Each user sees | Test |
|---|---|---|---|---|
| merges A1:A2, both empty on her side | writes `concurrent` into A2 at the same moment | the merge and Bob's text, both intact | one empty block over A1:A2, and a notice naming A2 as covered | `logic::tests::group3::concurrent_value_under_merge_is_masked_then_restored` |
| unmerges A1:A2 | -- | the merge is gone, Bob's text untouched | A2 reads `concurrent` again, and a formula over it recomputes | same test |
| merges A1:A2 | inserts a row above row 1 | the merge, now spanning A2:A3 | the block one row lower, holding the same value | `logic::tests::group3::merge_follows_structural_insert` |
| merges A1:B2 | merges B2:C3, overlapping at B2 | both entries, until validation drops one | one block; which one is decided by the document order, not by either user | none -- see section 5 |
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
| `TrameCommand::MergeCells { row, col, row_end, col_end }` | UI to Logic | the selection bounds |
| `TrameCommand::UnmergeCells { row, col }` | UI to Logic | one cell inside the block |
| `CommandId::MergeSelection` / `UnmergeActive` | chrome to Logic | no geometry: the Logic thread resolves it against the selection it tracks |
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

1. walk each merge on the sheet;
2. for each covered cell that still carries raw text, add its key to `masked_cells` and emit one
   conflict event;
3. feed the dependency graph and the formula cache from the cells outside that set;
4. on an unmerge, drop the keys, invalidate the bytecode and propagate.

**Code:**

* `crates/ambre-trame/src/logic/merges.rs` -- entries, split, conflict detection;
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
| context menu, "Merge cells" | `CommandId::MergeSelection` | over the selection the Logic thread tracks. A single cell is a silent no-op; a non-empty covered cell replies `MergeRejected`, shown as a toast |
| context menu, "Unmerge cells" | `CommandId::UnmergeActive` | at the active cell. Outside a merge it is a no-op |
| toolbar, the merge button | `CommandId::MergeSelection` | the same path: the `ToolbarAction::Merge` control reuses `merge_selection` |

Both carry no geometry: the DOM context menu dispatches a bare `CommandId`, and the Logic thread
resolves it against the active cell and selection it already tracks.

**Selection, navigation and rendering** (`TRAME-UI-009`):

* a selection rectangle grows to whole blocks (`expand_to_merges`), so a drag touching part of a
  block selects the block as one cell;
* the active cell rehomes to the master (`snap_active_to_master`);
* the fill handle sits on the block's bottom-right corner;
* arrow-key navigation steps over a block rather than into it;
* the master draws over its whole span, and `draw_cell` skips a covered cell before it fills any
  background -- a covered cell paints nothing, neither text nor fill.

**Fill from a merged source (Excel behaviour).** When the source is tiled by equal merged blocks --
one block, or a uniform grid of identically sized ones -- the fill is block-aware:

1. the UI captures the merge-expanded source and its block size (`fill_source_block`, which checks
   the whole rectangle is a uniform tiling);
2. the drag preview snaps the extension to a whole number of blocks along the locked axis
   (`fill_dest`: a 2x1 block fills two rows at a time);
3. on release the Logic thread tiles the blocks (`logic::fill::merged_source_block`,
   `fill_merged`). Each lane perpendicular to the drag axis extrapolates from its own stack of
   source blocks: a numeric `Series` progresses across blocks, otherwise the source blocks are
   copied cyclically and a copied formula reindexes by the tile offset;
4. each tile master takes the value and its source block's style, the tile's covered cells are
   cleared, and a merge entry is recorded per tile -- so the filled region is re-merged into blocks
   the size of the source.

The `Ctrl` single-against-multi test counts **blocks**, not raw cells, along the drag axis: one
block is one value (`Ctrl` gives `Series` +1), several blocks are several values (`Ctrl` gives
`Copy`). A plain source, or a mix of block sizes, keeps the per-cell fill unchanged.

**Events reaching the user.** `MergeRejected` and `MergeConflictDetected` are chrome-only: the
canvas ignores them, the chrome relay turns each into a notice (`trame-engine`).

---

## 5. DECISIONS

| Decision | Rationale | Rejected alternative |
|---|---|---|
| A local merge is refused while a covered cell is non-empty | the user keeps data they can still see; this is what Excel and Google Sheets do | merging anyway and dropping the covered values |
| A concurrent write under a merge is masked, never deleted | refusing a remote operation breaks convergence, and the data is the user's | dropping the remote write; asking the user to choose |
| The hidden set is derived, never stored | a stored set is a second source of truth that two peers can compute apart | a `masked` flag written into the document beside each cell |
| Detecting a conflict writes nothing back | a corrective write is itself an operation, so it can conflict and start a second round | emitting a delete or a move for the covered value |
| An overlap resolves by document order | it needs no coordination and every peer reaches the same answer | asking the user; refusing both merges |
| A fill from a uniform merged source tiles whole blocks | a per-cell fill from a merged source produces blocks the user did not ask for | filling cell by cell and leaving the destination unmerged |

---

## 6. NOT BUILT, AND NOT YET RULED

* **The file round trip** (`TRAME-IO-016`): xlsx import and export carry no merge yet.
* **Overlap resolution has no test.** Section 5 states that validation after a remote import
  drops the loser by document order. No test drives two overlapping merges, so section 2 row 4
  carries no verification and no invariant claims it.
* **A merge over a filtered or hidden row** is unspecified: no rule says whether the block spans the
  hidden row or closes over it.
