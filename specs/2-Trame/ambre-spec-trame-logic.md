# AMBRE — SPEC: TRAME LOGIC (Spreadsheet Engine)
**Type:** Vertical (Feature / Application — Logic)
**Status:** Draft
**Implementation:** partial

> Trame collaborative spreadsheet engine: multi-sheet CRDT state management (`LoroDoc`), incremental formula compilation to bytecode, evaluation via stack VM, two-level dependency graph, synchronous recomputation inside `handle_command`, and production of data-oriented structures for rendering.
>
> **Scope of this spec:** The engine (`TrameState`) that runs in the **Logic thread** (one of the three `wasm_thread` threads of the single WASM binary). Trame is a complete Logic + UI application — rendering (QuadRenderer/DrawSurface), user interactions, and the UI thread are covered in a separate spec (`spec-ambre-trame-ui.md`). Features (sort, filters, conditional formatting, undo/redo, validation) are covered in `spec-ambre-trame-features.md`. xlsx/PDF/CSV interoperability is covered in `spec-ambre-trame-interop.md`.
>
> **State registry.** Trame state is a concrete value, not a trait object. The Logic thread holds a typed registry `HashMap<DocumentId, DocState>` where `enum DocState { Trame(TrameState), … }`, and dispatch is a `match` on `(DocState, AppCommand)` — no trait object, no `Box<dyn Any>`, no `downcast`. `TrameState::handle_command` returns a typed [`TrameOutcome`]; the `ambre-logic` dispatcher reads it and forwards display events to the UI and CRDT/ephemeral payloads to Sync over bare `flume` channels. See `spec-ambre-architecture.md §6`.
>
> `LoroDoc` is instantiated in the Logic thread and never leaves it.

---

## 1. OBJECTIVES & CONSTRAINTS

**Scope of this module:**
- Owns the multi-sheet `LoroDoc`
- Incremental compilation of formulas to bytecode (compile once, execute N times)
- Evaluation via stack VM with dynamic cycle detection
- Two-level dependency graph: cell-to-cell + ranges
- Synchronous recomputation of dirty cells inside `handle_command` / `apply_remote` (aligned with the Prose pattern)
- Broken reference detection (`#REF!`)
- Production of `DisplayViewport` for the UI thread
- Export of CRDT deltas to the Sync thread via `SyncCommand::SendUpdate`

**Dependencies:**

| Crate | Role | Rationale |
|---|---|---|
| `ambre-core` | `Uuid`, `DocType` | Shared foundation |
| `ambre-bus` | `AppCommand`/`AppEvent` (incl. `TrameCommand`/`TrameEvent`), `SyncCommand`, `LogicEvent`, `CommandSink` (test seam) | Typed inter-thread data plane + IPC types |
| `ambre-draw` | `DrawSurface` trait, re-exports kurbo/peniko/parley | Shared rendering screen (WgpuSurface) / PDF (KrillaSurface) — required by architectural convention |
| `loro` | Multi-sheet CRDT | `LoroMovableList` (stable rows/columns), `LoroMap`, `LoroDoc` |
| `petgraph` | Cell-to-cell dependency DAG | Kosaraju SCC, topological sort |
| `nom` | Formula parser | Pure Rust, `no_std`, Excel-compatible syntax |
| `flume` | Logic<->Sync channels (prod) + native test bridges | The Logic dispatcher forwards `SyncCommand`s on a bare `flume::Sender`; tests use the `CommandSink` seam (`MockSyncHandle`) |
| `postcard` | Serialization of `TrameCommand` / `TrameEvent` / `DisplayViewport` | Compact, consistent with inter-thread IPC protocol |
| `rstar` | Floating objects R-tree | O(log n) viewport culling |

> **`ruzstd` absent** — compression delegated to the Sync thread.
> **Rayon absent (MVP)** — incremental recomputation O(dirty) is sufficient. Planned Phase 3+.

**Invariants:** prefix `TRAME` (the engine). Sibling specs reserve `TRAME-UI`, `TRAME-FEAT` and `TRAME-IO`. Grammar, falsifiability test and status values are normative in `ambre-spec-architecture.md §4` ("Invariants (normative)"); the cross-spec roll-up is `ambre-spec-testing.md §7`.

| ID | Statement (RFC 2119) | Sources | Verification | Status |
|---|---|---|---|---|
| `TRAME-001` | The `LoroDoc` MUST NOT leave the Logic thread; it exits only as bytes, through `export_updates` / `export_snapshot`. | §1 Scope; architecture §6 | type-level: `TrameState.doc` is private and **no public signature in `ambre-trame` names `LoroDoc`**; both exits return `Vec<u8>` | verified |
| `TRAME-002` | Dirty recomputation MUST complete synchronously inside `handle_command` / `apply_remote`; the returned outcome carries every resulting display event. | §1; §8 | type-level: `handle_command` returns `TrameOutcome` by value and the crate has no async path; `ambre_trame::logic::tests::group7::apply_remote_batch_emits_single_datareset_with_full_state` | verified |
| `TRAME-003` | A row's or column's identity MUST be a minted `Uid` carried as the `LoroMovableList` element's **value**, never its position. | §2.A | `ambre_trame::model::sheet_index::tests::build_maps_positions_to_stable_ids`, `::apply_move_row_permutes_order_and_reindexes`, `::cell_key_is_fixed_length_and_stable` | verified |
| `TRAME-004` | A sort MUST be a pure permutation of the row order (`rows_{sheet}.mov`); it MUST NOT rewrite a cell. | §2.A; §12 | `ambre_trame::logic::tests::group3::sort_range_ascending_moves_whole_rows`, `group4::sort_range_multi_already_sorted_emits_no_delta`, `group2::move_row_carries_cells_and_formula_follows_data` | verified |
| `TRAME-005` | A single-cell reference MUST follow its row through a sort or a move; a range MUST remain a positional block. | §3.A "Cells follow, ranges are blocks"; §12 | `ambre_trame::logic::tests::group4::sort_range_formulas_follow_data`, `::sort_range_sum_block_is_stable_under_internal_sort` | verified |
| `TRAME-006` | A reference past the used range MUST compile to a `Virtual` axis and read `Empty`, never an error. | §3.A; §5 | `ambre_trame::formula::compile::tests::ref_beyond_used_range_is_virtual`, `ambre_trame::formula::vm::tests::push_cell_virtual_beyond_used_range_reads_empty` | verified |
| `TRAME-007` | A reference whose `Bound` `Uid` is absent from the `SheetIndex` MUST evaluate to `#REF!`, and every operator MUST propagate that error. | §5 | `ambre_trame::formula::vm::tests::push_cell_deleted_bound_yields_ref_error`, `::every_operator_propagates_an_operand_error`, `::unary_not_propagates_an_operand_error`, `::eval_range_over_cell_cap_yields_ref_error` | verified |
| `TRAME-008` | A row or column delete MUST propagate `#REF!` to every formula that referenced the deleted line. | §5 "Propagation after delete_row/delete_col" | none, and the code does the **opposite**: the delete path reseeds and re-binds every formula from its raw text, so the reference lands positionally on the shifted grid. `ambre_trame::logic::tests::group2::formula_reference_is_positional_after_delete` asserts `=A2` reads **empty** after deleting row 1. `TRAME-007`'s VM primitive is unreachable from this path | unverified |
| `TRAME-009` | Deleting a sheet MUST break its cross-sheet references (`#REF!`); a rename or a reorder MUST NOT. | §2.D; §12 | `ambre_trame::logic::tests::group1::cross_sheet_reference_survives_rename_and_breaks_on_delete`, `group3::cross_sheet_reference_converges_across_peers_on_rename`, `::rename_sheet_keeps_id_and_cells`, `ambre_trame::formula::parser::tests::deleted_sheet_id_yields_broken_ref` | verified |
| `TRAME-010` | Computed values MUST NOT enter the CRDT: only the raw `v` is shared and each peer recomputes independently. | §1; §2.B | none. `ambre_trame::logic::tests::group1::sum_over_plain_literals_survives_reseed` covers recompute-after-reseed; **nothing asserts the absence of computed values from the document**, which is the half a remote peer depends on | unverified |
| `TRAME-011` | Only cells whose raw text changed MUST be recompiled. | §4.C | `ambre_trame::formula::cache::tests::stale_cell_recompiled`, `::not_stale_no_recompile`, `::non_formula_clears_cache` | verified |
| `TRAME-012` | A cyclic formula MUST be rejected with its culprit cells named; evaluation MUST NOT hang. | §4.D | `ambre_trame::formula::dag::tests::static_cycle_detected`, `::cycle_detection_kosaraju_returns_culprits`, `::self_reference_rejected`, `::rejected_cycle_leaves_no_stale_range_deps` | verified |
| `TRAME-013` | Parser, compiler and VM MUST be total: no input produces a panic, every failure is a `CellValue::Error`. | §4.B; §4.D | `ambre_trame::formula::prop_tests::parser_never_panics_on_arbitrary_text`, `::parser_never_panics_on_formula_noise`, `::compile_emits_balanced_bytecode`, `::vm_is_total_and_deterministic` (proptest) | verified |
| `TRAME-014` | Aggregate functions MUST be exact and order-independent over their range. | §4.A | `ambre_trame::formula::prop_tests::sum_is_exact_and_order_independent`, `::engine_sum_matches_naive_sum`, `::product_is_order_independent`, `::average_times_count_equals_sum`, `::min_max_bound_every_element` (proptest) | verified |
| `TRAME-015` | Two peers exchanging the same operations in any interleaving MUST converge to the same document. | §12; architecture §4 "CRDT convergence" | `ambre_trame::logic::convergence_tests::two_peers_converge_under_interleaved_ops` (proptest), `ambre_trame::logic::tests::group3::two_peers_converge_via_mock_sync` | verified |
| `TRAME-016` | A cell's whole style MUST be a single LWW register; concurrent restyles MUST NOT blend per attribute. | §2.B; §11 bis.F; §12 | `ambre_trame::logic::convergence_tests::concurrent_whole_cell_style_edits_do_not_blend_across_containers`, `::subset_style_edit_does_not_tear_a_whole_cell_rewrite`, `logic::tests::group1::bold_stays_on_the_cell_not_the_selection` | verified |
| `TRAME-017` | Every mutating `TrameCommand` handler MUST end with `doc.commit()`, so that one command is one undo item. | §8 "Command Dispatch and Synchronous Recalculation" | `ambre_trame::logic::tests::group2::batch_write_is_one_undo_item`, `::filtered_column_edit_is_one_undo_item`, `group1::selection_style_is_a_single_undo`, `group6::replace_all_rewrites_and_undoes_as_one_group` | verified |
| `TRAME-018` | Undo MUST revert local operations only, never a remote peer's. | §8; `trame-features` (undo/redo) | `ambre_trame::logic::tests::group2::undo_local_only_not_remote`, `::undo_insert_row_rebuilds_sheet_index`, `::undo_stack_cleared_on_doc_close` | verified |
| `TRAME-019` | A merge MUST be refused locally while a covered cell is non-empty, and an unmerge MUST restore every covered value. | §6 "Data is Never Destroyed" | `ambre_trame::logic::tests::group2::merge_rejected_when_covered_cell_non_empty`, `::merge_marks_master_span_and_blanks_covered`, `::unmerge_restores_covered_cells`, `group3::concurrent_value_under_merge_is_masked_then_restored` | verified |
| `TRAME-020` | A merge-masked cell MUST NOT participate in any computation, on any sheet, whichever tab is active. | §6 "DAG and Masked Cells"; §11 bis.E | `ambre_trame::logic::tests::group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` -- the cross-sheet test §11 bis.E asks for, already green (**§11 bis.E is stale, see the note there**) | verified |
| `TRAME-021` | A formula result MUST NOT depend on view state (active sheet, filter, hidden rows). | §11 bis.E; §12 | `ambre_trame::logic::tests::group5::shared_filter_does_not_affect_sum`, `group3::masked_cell_excluded_across_sheets_regardless_of_active_tab` | verified |
| `TRAME-022` | A workbook MUST always hold at least one sheet. | §2.D | partial: `ambre_trame::logic::tests::group3::delete_last_sheet_rejected` covers the **local** guard only. Two peers concurrently deleting the two last sheets still converge to a sheet-less workbook; §2.D's deterministic re-insertion rule is decided, not implemented, and has no 2-peer regression test | unverified |
| `TRAME-023` | The incrementally maintained `SheetIndex` MUST equal a from-scratch rebuild after any sequence of structural operations. | §2.C | none. Per-operation unit tests only (`ambre_trame::model::sheet_index::tests::apply_insert_and_delete_row_shift_positions`, `::apply_move_row_permutes_order_and_reindexes`). The equivalence property exists on the Toile index and not here, though this is the cache invariant the whole engine rests on | unverified |
| `TRAME-024` | `ambre-trame` MUST contain no `unsafe`. | context pillar "Security" | measured zero `unsafe` in the crate, but no `#![forbid(unsafe_code)]` on its root -- a claim, not a mechanism (architecture §4 ladder) | unverified |

> **Invariant coverage.** 19 `verified`, 5 `unverified`, 0 `human-review`, 0 `qa-manual`. The five gaps are `TRAME-008` (the spec asserts a `#REF!` propagation the delete path prevents), `TRAME-010`, `TRAME-022`, `TRAME-023` and `TRAME-024`; each is reported in `ambre-spec-testing.md §7`.
---

## 2. MULTI-SHEET CRDT MODEL

### A. Own stable uids carried as `LoroMovableList` values

Positional addressing (`"R{row}C{col}"`) is **rejected**: inserting a row at position 5 would invalidate all keys `R5C*..RnC*` → O(n) CRDT operations.

**Decision: each row and column carries an own stable `Uid` that *we* mint, stored as the
*value* of an element in a `LoroMovableList` (`rows_{sheet}` / `cols_{sheet}`).** A `Uid` is
`{peer: u64, counter: u32}`, rendered `"{peer:016x}_{counter:08x}"`, where `peer` is the document
`PeerID` and `counter` a per-session monotonic counter. Inserting a row = 1 CRDT operation
(push/insert one uid value). No cell key is invalidated.

> **Why an owned value, not the Loro element id.** `LoroMovableList` does expose a per-element op
> id via `get_cursor(pos, Side::Middle).id` (a `loro::ID { peer, counter }`), but
> `LoroMovableList::mov(from, to)` **reassigns the moved element's op id** -- it becomes the move
> op's id. A cell keyed by that slot id would be orphaned by every reorder. Owning the uid as the
> element's *value* makes it **move-invariant**: a reorder (`mov`) shuffles the values, the uid each
> carries is untouched, and every `"{row_uid}:{col_uid}"` cell key keeps resolving. This is what
> lets a sort be a **pure order permutation** (§12, `trame-features §2`) instead of a content
> rewrite, and what lets id-based references follow the data. See the rejected "Loro slot id as
> cell key" and "sort by content rewrite" entries in §13.

**Convergence of minting.** Two peers that concurrently insert a row each mint a **distinct** uid
(their `PeerID` differs), so both rows survive and `LoroMovableList` orders the two values
canonically -- no shared counter, no LWW collision. This is the §2.D sheet-id argument applied to
rows/columns (see the rejected shared-counter hazard in §13). The counter need only be unique
within a peer's session; once minted, a uid lives in the CRDT and never changes.

**Key format in `LoroMap("cells_{sheet}")` — fixed-length 51 chars** (16 + `_` + 8 per side, joined by `:`):
```
"{row_peer:016x}_{row_ctr:08x}:{col_peer:016x}_{col_ctr:08x}"
```

#### The pinned-`PeerID` constructor (`TrameState::with_peer`)

`TrameState::new()` builds over a `LoroDoc::new()`, which draws a **random** `PeerID`. Because every
uid above carries that peer into the document's *content* — row and column uids, cell keys, sheet ids
(§2.D) — the same command sequence run twice produces two different documents. That is correct for a
live client and fatal for a reproducible one, so the engine exposes a second constructor:

```rust
impl TrameState {
    /// A workbook whose `PeerID` and commit clock are pinned: `set_peer_id(peer)`
    /// and `set_record_timestamp(false)`, both applied before the implicit-sheet
    /// scaffolding commit so that op is attributed to `peer` too.
    pub fn with_peer(peer: u64) -> Self;
}
```

**Not for a live document.** Two concurrent writers sharing a `PeerID` mint colliding `OpID`s and
corrupt the history — the one hazard §2.A's minting argument rests on avoiding. This constructor is
for single-owner, offline, host-side reproduction: a fixture generator
(`ambre-spec-fixtures.md` I1) or a deterministic test. Client code calls `new()`.

Ungated (no Cargo feature): it is engine API documented here, and the determinism belt that asserts
on it runs in the default host tier. The fixture *machinery* that consumes it is gated
(`fixtures §I4`).

### A bis. Dense over used range — reconciliation with the virtual 100k grid

The grid is virtual at `TOTAL_ROWS = 100_000`. We do **not** materialize 100k list elements. The
model that keeps O(1) insert *and* CRDT convergence is:

> **Own stable `Uid`s carried as element values, `rows_/cols_` `LoroMovableList` dense over the
> *used range* `[0..hwm]` (high-water mark), rows/columns beyond the used range are virtual** (no
> element, no storage; a read there returns the generated label, a write there materializes up to
> that index).

Crucially, **list index == grid position**: resolving `A5` is a direct `row_by_pos[5]` lookup, and a
concurrent insert of two distinct rows is two independent element insertions that Loro orders
canonically — convergent *and* intent-preserving, with no shared mutable counter.

**Materialization cap.** To bound the pathological "single value far down" case (e.g. an edit at row
99 999 in an otherwise empty sheet, which would materialize ~100k placeholders), a write that would
extend the used range by more than `ROW_MATERIALIZE_CAP` (= 10 000) rows past the current high-water
mark is **rejected** (the cell write is dropped and a warning is surfaced). Normal top-down editing
is unaffected. The symmetric cap applies to columns (bounded by `TOTAL_COLS = 1000` anyway).

The **truly-sparse run-length** alternative (store only non-empty rows + a per-row `gap_before`
counter + an order-statistic tree) is **rejected** — see §13: splitting a shared gap on concurrent
inserts is not convergent under LWW and corrupts absolute numbering. The order-statistic tree only
speeds the position query; it does not remove the shared-counter hazard.

### B. CRDT Schema

> **Target schema, not as-built.** The cell sub-map (`{v,t,s}`) and the shared `styles` map below
> are **not** implemented: cell values are stored flat in `cells_{sheet_id}`, and a cell's style is
> stored **as-built** in one per-sheet map `cellstyle_{sheet_id}` -- key `"{row_uid}:{col_uid}"`,
> value = the whole style of the cell (bold/italic plus the full `CellStyle`: alignments, wrap,
> colours, borders, font, number format, decimals, size) postcard-encoded into a single binary LWW
> register. This is one register **per cell**, not one **per attribute**: it is the convergence
> invariant (§11 bis.F), and the cross-app "style as one LWW register" pattern
> (`ambre-spec-architecture.md §2.D`, normative) that Toile's element `"style"` register also follows.
> It replaced an earlier fan-out over up to 11 per-sheet maps, which had two
> real defects (a border-colour tear and a colour/font reseed data-loss, §11 bis.F). The shared
> `styles` dictionary and the `"s"` indirection -- a *deduplication* optimization, orthogonal to the
> consolidation just landed -- stay deferred behind a benchmark (§11 bis.F, "out of scope"). See §14
> Phase 0.

```
LoroDoc
├── "workbook" (LoroMap)              → {name, author, created}
├── "sheet_order" (LoroMovableList)   → [sheet_id, ...] reorderable
├── "sheets" (LoroMap)
│   └── "{sheet_id}" (LoroMap)        → {name: LoroText, hidden: bool, tab_color: u32}
│
├── "rows_{sheet_id}" (LoroMovableList)   → each element VALUE = a row's own uid (move-invariant)
├── "cols_{sheet_id}" (LoroMovableList)   → same for columns
│
├── "cells_{sheet_id}" (LoroMap)      → sparse, key = "{row_uid}:{col_uid}"
│   └── "{uid}:{uid}" (LoroMap)
│       ├── "v" : LoroText            → raw value ("42", "=SUM(A1:A10)", "text")
│       ├── "t" : String              → "n" | "s" | "b" | ""
│       └── "s" : String              → style_id
│
├── "col_widths_{sheet_id}" (LoroMap) → {col_uid: f64}
├── "row_heights_{sheet_id}" (LoroMap)
│
├── "merges_{sheet_id}" (LoroMap)     → {"{row_uid}:{col_uid}": "{row_uid_end}:{col_uid_end}"}
│
├── "styles" (LoroMap)                → shared styles (TARGET dedup, deferred -- §11 bis.F "out of scope")
│   └── "style_007" (LoroMap)         → {bold, italic, underline, font, size, fg, bg, fmt, halign, valign, wrap, border}
│                                        AS-BUILT instead: one register per cell, whole style inline --
├── "cellstyle_{sheet_id}" (LoroMap) → {"{row_uid}:{col_uid}": Binary(postcard(StyleRecord{bold,italic,CellStyle}))}
│
├── "named_ranges" (LoroMap)          → {"TOTAL": {sheet_id, range}}
├── "media" (LoroMap)                 → {hash_blake2b: {dims, anchor, mime}}
│
│   ── Features (defined in spec-ambre-trame-features.md) ──
├── "filters_{sheet_id}" (LoroMap)    → active shared filter {col_uid, kind, values/condition}
├── "filter_hidden_{sheet_id}" (LoroMap) → {row_uid: bool} — rows hidden by filter
├── "cond_formats_{sheet_id}" (LoroList) → conditional formatting rules [{range, priority, rule, style}]
├── "validations_{sheet_id}" (LoroMap) → data validations [{kind, params, on_invalid, message}]
└── "protections_{sheet_id}" (LoroMap) → range protection (Post-MVP) [{owner, allowed, description}]
```

### C. SheetIndex — Bidirectional O(1) Cache

The `Vec`s are dense over the **used range** (`len == hwm`, see §2.A bis), never over the 100k
virtual grid, so `resolve` is a direct O(1) index lookup and memory is proportional to real sheet
size. `row_by_pos[i]` is the `Uid` carried by the element at list position `i` (read from the
element's *value*, parsed with [`parse_uid`], not from a Loro cursor -- see §2.A). Because the uid
is move-invariant, a `rows_{sheet}.mov(from, to)` simply **permutes `row_by_pos`** and every cell
key still resolves; this is the single primitive behind sort (§12) and any future drag-to-reorder.

```rust
pub struct SheetIndex {
    row_by_pos: Vec<Uid>,
    col_by_pos: Vec<Uid>,
    row_to_pos: HashMap<Uid, usize>,
    col_to_pos: HashMap<Uid, usize>,
}

impl SheetIndex {
    /// O(N_used) reconstruction at load (N_used = materialized rows + cols, not 100k).
    /// Reads each element's uid VALUE in list order.
    pub fn build(rows: &LoroMovableList, cols: &LoroMovableList) -> Self { /* ... */ }

    /// Incremental O(N) update for shifts
    /// Phase 2: order-statistic tree for O(log N)
    pub fn apply_insert_row(&mut self, pos: usize, id: Uid) { /* ... */ }
    pub fn apply_delete_row(&mut self, pos: usize) -> Uid   { /* ... */ }

    /// Permute the row order to mirror a `mov(from, to)` (sort, reorder).
    pub fn apply_move_row(&mut self, from: usize, to: usize) { /* ... */ }

    /// Position pair -> uid pair. `None` in virtual space (past the used range).
    pub fn resolve(&self, row: usize, col: usize) -> Option<(Uid, Uid)> { /* ... */ }

    /// Used for #REF! detection
    pub fn contains_row(&self, id: &Uid) -> bool { self.row_to_pos.contains_key(id) }
    pub fn contains_col(&self, id: &Uid) -> bool { self.col_to_pos.contains_key(id) }

    /// Bijective base-26 (A..Z, AA..), the spreadsheet convention: 0 -> "A",
    /// 26 -> "AA". Round-trips with `letter_to_index`.
    pub fn col_letter(index: usize) -> String   { /* ... */ }
    pub fn letter_to_index(s: &str) -> usize    { /* ... */ }
}
```

### D. Sheet Lifecycle (create / rename / reorder / delete)

A workbook is a set of sheets ordered by `sheet_order` (a `LoroMovableList` of
stable `sheet_id`s). Each operation below is a small, convergent CRDT mutation;
none rewrites another sheet's content. The four commands are
[`CreateSheet`](#8-interface-contract-api) / `RenameSheet` / `MoveSheet` /
`DeleteSheet` (§9).

> **Sheet identity is a stable `sheet_id`, never the display name.** Formulas
> depend on `sheet_id`; the display name is a mutable label resolved to its id
> at parse time. This is the §2.A principle applied to sheets: rename and
> reorder never invalidate a reference, exactly as a row insert never
> invalidates a cell key. Only **delete** removes an id, and that is the only
> operation that can produce a cross-sheet `#REF!`.

**`sheet_id` generation.** The implicit first sheet keeps the constant
[`DEFAULT_SHEET`](../../crates/ambre-trame/src/logic.rs) `"s0"` so two fresh
states (creator and reloading peer) address the same containers and merge
cleanly. Every **explicit** sheet gets a peer-scoped id
`"{peer:016x}_{counter:08x}"` (the document `PeerID` plus a per-session
counter), so two peers that concurrently "add a sheet" mint two **distinct**
ids -- both survive, neither overwrites the other (LWW on a single shared id is
thereby avoided, mirroring the rejected shared-counter hazard of §13).

#### Create

```
sheets.insert(sheet_id, {name, hidden: false, tab_color})   // metadata
sheet_order.push(sheet_id)                                  // 1 LoroMovableList op
// cells_{id} / rows_{id} / cols_{id} are created lazily on first write.
```

Convergence: concurrent creates append two distinct ids; `LoroMovableList`
orders them canonically. The local engine assigns the new sheet a `u16` index at
the next reseed (its empty cell set needs no recompute). Reply:
`TrameEvent::SheetList` so the tab strip refreshes.

#### Rename

```
sheets.get(sheet_id).set("name", new_name)   // LWW on the name field only
```

The `sheet_id` is untouched, so **every cross-sheet reference keeps resolving and
keeps computing** -- no formula is recompiled for a rename. The formula *text*
shown in the formula bar is reconstructed from `sheet_id -> current name`, so a
renamed sheet immediately displays the new qualifier (`Data!A1` where the user
once typed `Sheet2!A1`). Concurrent renames resolve by LWW on the `name` field;
all peers converge to one name and reconstruct identical formula text.

> **Implementation note.** This requires the compiled cross-sheet dependency to
> carry the `sheet_id` (the engine's local `u16` is a per-session interning of
> it), and the stored raw `v` text to be re-rendered through the current
> `sheet_id -> name` map rather than replayed verbatim. The MVP engine re-parses
> raw text by display name on reseed, so until id-persistence lands a rename
> breaks an existing cross-sheet reference -- tracked in §14.

#### Reorder (move)

```
sheet_order.mov(from_index, to_index)   // 1 LoroMovableList move op
```

Pure presentation: `sheet_order` drives only the tab strip and the local `u16`
index assignment, never resolution (references are by `sheet_id`). No recompute.
`MoveSheet { sheet_id, to_index }` looks up the current index and issues one
`mov`; concurrent moves converge through `LoroMovableList`'s canonical ordering.

#### Delete

```
sheet_order.delete(index, 1)
sheets.delete(sheet_id)
// drop per-sheet containers: cells_{id}, rows_{id}, cols_{id},
// cellstyle_{id}, merges_{id}, and the feature maps of trame-features.
```

- **Cross-sheet `#REF!`.** A formula `=Sheet2!A1` whose `sheet_id` is gone
  resolves to `BrokenRef` at evaluation -- the §5 mechanism, extended to the
  sheet axis: absence in the `sheet_id -> u16` map is the trigger, no bytecode
  patching. Deleting a sheet marks every dependent formula dirty (full reseed,
  since the `u16` remap shifts indices) and recomputes them to `#REF!`.
- **A workbook always keeps >= 1 sheet.** Deleting the last sheet is **rejected
  locally** (`TrameEvent::Error`) -- this is the **as-built** guarantee. For the
  concurrent case (two peers delete different sheets and the deletes converge to an
  empty `sheet_order`), the **decided rule** is **deterministic re-insertion**: the
  sheet whose `sheet_id` sorts first is re-inserted, identically on every peer, so
  the workbook is never sheet-less. This preserves the CRDT convergence + intent
  invariant of §12 (no peer sees a sheet-less state, and all converge to the same
  surviving sheet) rather than momentarily emptying the workbook. **Not yet
  implemented:** the local guard ships today; the concurrent rule must land with a
  2-peer convergence regression test (§14, roadmap) before it is relied on.
- **Active sheet deleted** (locally or by a remote delete): the active sheet
  switches to the previous tab, or the first remaining sheet if it was first.

**Engine impact.** Create/move/rename need at most a `SheetList` refresh; only
**delete** forces a reseed + recompute (sheet set changed -> `u16` remap +
`#REF!` propagation), aligned with the structural-op reseed path of §8.

---

## 3. TWO-LEVEL DEPENDENCY GRAPH

### Problem of Ranges in `petgraph`

`=SUM(A1:A100)` represented by 100 edges in `petgraph`:
- Memory explosion on entire columns
- O(n) updates on each row insert

**Solution: two separate structures.**

### A. Range Types

**Two reference representations.** The `nom` parser produces *positional* refs
(`CellRef { sheet: u16, row: u32, col: u32 }`, `RangeDep { .. : u32 }`) -- a typed
`A1` / `A1:B10`. Compilation resolves those against the current per-sheet
`SheetIndex` into the *uid-anchored* engine forms below (`CellId` / `RangeId`),
which are what the bytecode, dependency graph, bytecode cache and VM are keyed
by. Binding `A1` to the row/column `Uid` is what makes a reference follow the data
(§4.B / §12).

**Bound vs virtual axes (`AxisId`).** Within the used range a single-cell
reference binds to the row/column `Uid`. Beyond the used range -- a *virtual*
row/column with no list element yet (§2.A bis), so no `Uid` -- there is nothing to
bind to, so the reference keeps the grid position it was typed at and reads
`Empty` until that line is materialized (a later re-edit then re-binds it to the
new `Uid`). This is the only correct way to model `=A100` on a 10-row sheet.

**Cells follow, ranges are blocks.** Only *single-cell* references bind to a `Uid`
and follow their row/column through a sort (§12). A **range** is compiled with
`Virtual` (positional) endpoints, so it denotes a rectangular *region* -- a block
of grid positions -- not a set of specific rows. Sorting the rows inside
`SUM(A1:A3)` reorders the same three values, leaving the block (and its total)
intact; this matches Excel/Sheets and is the only well-defined behavior, since
uid-bound endpoints could scatter and invert the rectangle (see §13). A range
therefore never `#REF!`s on a row delete -- it shifts positionally, like Excel.

```rust
pub enum AxisId {
    /// Bound to a materialized row/column `Uid`. #REF! once that id is deleted.
    Bound(Uid),
    /// A virtual position beyond the used range; stays positional, reads Empty.
    Virtual(u32),
}

pub struct CellId {           // engine-side (uid-anchored) cell identity
    pub sheet: u16,           // local engine sheet index
    pub row:   AxisId,
    pub col:   AxisId,
}

pub struct RangeId {          // engine-side (uid-anchored) range identity
    pub sheet:     u16,
    pub row_start: RangeEndpoint,
    pub row_end:   RangeEndpoint,
    pub col_start: RangeEndpoint,
    pub col_end:   RangeEndpoint,
}

pub enum RangeEndpoint {
    /// Bounded endpoint -- stable via the axis `AxisId`. #REF! if a bound id is deleted.
    Specific(AxisId),
    /// Unbounded range: entire column (A:A) or entire row (1:1). Reserved; the
    /// MVP parser never emits it.
    EntireAxis,
}

impl RangeId {
    /// True when the cell falls inside the range on **both** axes. Each axis is
    /// tested independently against its endpoints, resolved through the index.
    pub fn contains(&self, row: Uid, col: Uid, index: &SheetIndex) -> bool { /* ... */ }

    /// Resolves endpoints to positions for iteration. `entire` is the position an
    /// `EntireAxis` bound takes (0 for a start, axis_len-1 for an end). Returns
    /// None only when a `Specific(Bound(_))` endpoint is no longer in the index
    /// (-> #REF!); a `Specific(Virtual(p))` keeps position `p` even past the
    /// used range (the caller clamps iteration to the materialized cells).
    pub fn row_range(&self, index: &SheetIndex) -> Option<(usize, usize)> { /* axis resolve */ }
    pub fn col_range(&self, index: &SheetIndex) -> Option<(usize, usize)> { /* axis resolve */ }

    fn endpoint_pos(ep: &RangeEndpoint, entire: usize, pos: impl Fn(&Uid) -> Option<usize>)
        -> Option<usize>
    {
        match ep {
            RangeEndpoint::EntireAxis                   => Some(entire),
            RangeEndpoint::Specific(AxisId::Bound(u))   => pos(u),         // None -> #REF!
            RangeEndpoint::Specific(AxisId::Virtual(p)) => Some(*p as usize),
        }
    }
}
```

### B. DependencyGraph

```rust
pub struct DependencyGraph {
    /// Level 1: cell-to-cell dependencies (cycles detected statically)
    cell_dag:    petgraph::stable_graph::StableGraph<CellId, ()>,
    cell_to_node: HashMap<CellId, petgraph::graph::NodeIndex>,

    /// Level 2: range dependencies (cycles detected dynamically)
    range_deps:  HashMap<CellId, Vec<RangeId>>,

    /// Reverse index: local sheet index -> [(formula, RangeId)]
    /// MVP: linear scan O(n). Phase 2: 2D interval tree.
    range_index: HashMap<u16, Vec<(CellId, RangeId)>>,
}

impl DependencyGraph {
    pub fn update(
        &mut self,
        formula_cell: CellId,
        new_cell_deps:  Vec<CellId>,
        new_range_deps: Vec<RangeId>,
    ) -> Result<(), CycleError> {
        // Update cell_dag
        let node = self.get_or_create_node(formula_cell);
        let old: Vec<_> = self.cell_dag
            .edges_directed(node, petgraph::Direction::Outgoing)
            .map(|e| e.id()).collect();
        for e in old { self.cell_dag.remove_edge(e); }
        for dep in &new_cell_deps {
            self.cell_dag.add_edge(node, self.get_or_create_node(*dep), ());
        }

        // Static cycle detection on cell_dag
        if petgraph::algo::is_cyclic_directed(&self.cell_dag) {
            // Rollback
            let added: Vec<_> = self.cell_dag
                .edges_directed(node, petgraph::Direction::Outgoing)
                .map(|e| e.id()).collect();
            for e in added { self.cell_dag.remove_edge(e); }
            return Err(CycleError(self.find_cycle_cells()));
        }

        // Update range_deps + range_index
        for (_, entries) in self.range_index.iter_mut() {
            entries.retain(|(c, _)| *c != formula_cell);
        }
        self.range_deps.remove(&formula_cell);
        if !new_range_deps.is_empty() {
            for dep in &new_range_deps {
                self.range_index.entry(dep.sheet).or_default()
                    .push((formula_cell, dep.clone()));
            }
            self.range_deps.insert(formula_cell, new_range_deps);
        }
        Ok(())
    }

    /// Collects all cells to mark dirty after `changed` is modified.
    pub fn collect_dirty(
        &self,
        changed: CellId,
        index: &SheetIndex,   // the changed cell's sheet index
    ) -> HashSet<CellId> {
        let mut dirty = HashSet::new();

        // Level 1: DFS descendants in cell_dag
        if let Some(&node) = self.cell_to_node.get(&changed) {
            let mut dfs = petgraph::visit::Dfs::new(&self.cell_dag, node);
            while let Some(n) = dfs.next(&self.cell_dag) {
                dirty.insert(self.cell_dag[n]);
            }
        }

        // Level 2: formulas whose range contains changed. Only a fully bound
        // cell can sit inside a uid-keyed range; a virtual cell is skipped.
        let (Some(row), Some(col)) = (changed.row_uid(), changed.col_uid()) else { return dirty };
        if let Some(entries) = self.range_index.get(&changed.sheet) {
            for (formula_cell, dep) in entries {
                if dep.contains(row, col, index) {
                    dirty.insert(*formula_cell);
                    // Propagate in cell_dag from formula_cell
                    if let Some(&node) = self.cell_to_node.get(formula_cell) {
                        let mut dfs = petgraph::visit::Dfs::new(&self.cell_dag, node);
                        while let Some(n) = dfs.next(&self.cell_dag) {
                            dirty.insert(self.cell_dag[n]);
                        }
                    }
                }
            }
        }
        dirty
    }

    /// The culprits of a static cycle: the first strongly connected component of
    /// the cell DAG with more than one node (Kosaraju), mapped back to `CellId`s.
    /// Empty when the graph is acyclic.
    fn find_cycle_cells(&self) -> Vec<CellId> { /* ... */ }
}
```

---

## 4. BYTECODE VM & INCREMENTAL COMPILATION

### A. Instruction Set

```rust
#[derive(Clone)]
pub enum Op {
    // Pile
    PushNum(f64),
    PushStr(String),
    PushBool(bool),
    PushEmpty,

    // References -- verify SheetIndex (broken ref) and eval_stack (dynamic cycle)
    PushCell(CellId),
    EvalRange(RangeId),     // iterates range -> pushes Vec<CellValue> for functions

    // Binary operations
    Add, Sub, Mul, Div, Pow,
    Eq, Ne, Lt, Le, Gt, Ge,
    Concat,                 // &

    // Unary operations
    Neg, Not,

    // Appel de fonction : n_args valeurs sur la pile
    CallFn { name: String, n_args: u8 },

    // Push a compile-time error (parse failure, unknown name, unresolved
    // cross-sheet `#REF!`) so evaluation yields it directly. Emitted by
    // `compile()` (see `formula/compile.rs`).
    PushError(FormulaError),
}

pub struct CellBytecode {
    pub ops:        Vec<Op>,
    pub cell_deps:  Vec<CellId>,    // extracted at compilation -> DependencyGraph update
    pub range_deps: Vec<RangeId>,
}
```

### B. AST → Bytecode Compilation

`compile(&Expr) -> Vec<Op>` is a post-order walk of the AST (`ambre-trame/src/formula/compile.rs`):

* a **literal** emits its `Push*`, an unrepresentable one emits `PushEmpty`;
* a **cell reference** is bound against the current `SheetIndex` and emits `PushCell(CellId)`; a **range** binds the same way and emits `EvalRange(RangeId)` -- binding at compile time is what makes references survive a sort (`TRAME-005`);
* a **binary operation** emits its operands then the operator, which is what keeps the stack balanced (`TRAME-013`);
* a **function call** emits every argument then `CallFn { name, n_args }`.

`SheetRef` and `Name` never reach this walk: `parser::resolve_refs` rewrites them into a `CellRef`/`Range` beforehand, or into `Op::PushError` for an undefined name or a deleted target. The arm exists anyway, guarded by a `debug_assert!` and degrading to `PushEmpty` -- a blank value, never a panic (`TRAME-013`).


**A1 → CellId Resolution:** compilation receives the current `SheetIndex` (per sheet). `A1` (col=A→0, row=1→index 0) is resolved against the index to the `Uid`s currently at those positions, producing a stable `CellId { sheet, row: AxisId::Bound(uid), col: AxisId::Bound(uid) }`. The reference then **binds to those rows/columns**, surviving inserts, deletes, and reorders -- after a sort it still points at the same physical cells (now elsewhere), which is the §12 "references follow the data" guarantee. A position *beyond* the used range (e.g. `A100` on a 10-row sheet) has no `Uid`, so it compiles to `AxisId::Virtual(pos)` and reads `Empty` until that line is materialized (§3.A).

### C. Bytecode Cache and Incremental Compilation

```rust
pub struct BytecodeCache {
    entries: HashMap<CellId, CellBytecode>,
    stale:   HashSet<CellId>,   // raw changed -> recompilation needed
}

impl BytecodeCache {
    pub fn invalidate(&mut self, cell: CellId) { self.stale.insert(cell); }

    /// Recompiles if stale. Called just before compute_cell(). `indices` lets
    /// compilation bind every positional `A1` to its row/column `AxisId`.
    pub fn recompile_if_stale(
        &mut self,
        cell:        CellId,
        raw:         &str,
        indices:     &HashMap<u16, SheetIndex>,
        deps:        &mut DependencyGraph,
        ctx:         &dyn FormulaCtx,
    ) {
        if !self.stale.remove(&cell) { return; }

        if !raw.starts_with('=') {
            self.entries.remove(&cell);
            deps.update(cell, vec![], vec![]).ok();
            return;
        }

        match formula_parser::parse(&raw[1..], sheet_index, sheet_name_to_id) {
            Ok(ast) => {
                let bc = compile_with_deps(&ast);
                match deps.update(cell, bc.cell_deps.clone(), bc.range_deps.clone()) {
                    Ok(()) => { self.entries.insert(cell, bc); }
                    Err(CycleError(cycle)) => {
                        self.entries.remove(&cell);
                        // compute_cell traitera le CycleError
                    }
                }
            }
            Err(e) => {
                self.entries.remove(&cell);
                // formula_cache will receive ParseError directly in compute_cell
            }
        }
    }
}
```

### D. Stack VM with Dynamic Cycle Detection

```rust
/// Shared context for an entire recomputation cycle.
pub struct EvalContext<'a> {
    /// Cells currently being evaluated -- dynamic cycle detection
    /// (ranges, named ranges, cross-sheet, any cycle not visible in cell_dag)
    eval_stack: HashSet<CellId>,

    /// Results already computed in this cycle -- avoids redundant recomputation
    resolved:   HashMap<CellId, CellValue>,

    indices: &'a HashMap<u16, SheetIndex>,
    bc:      &'a BytecodeCache,
    cache:   &'a HashMap<CellId, CellValue>,
}

impl<'a> EvalContext<'a> {
    /// Cycle guard, memo, dispatch. See below.
    pub fn eval_cell(&mut self, cell: CellId) -> CellValue { /* ... */ }
    fn execute(&mut self, bc: &CellBytecode) -> CellValue  { /* ... */ }
    fn eval_range(&mut self, dep: &RangeId) -> Vec<CellValue> { /* ... */ }
}
```

**`eval_cell`** -- three steps, in order: a cell already on `eval_stack` returns `CircularReference` (this is the *dynamic* cycle detection, catching what the static `cell_dag` cannot see: ranges, named ranges, cross-sheet); a cell already in `resolved` returns its memo; otherwise the cell is pushed on the stack, evaluated (literal read when it has no bytecode, `execute` when it has), popped, and memoised.

**`execute`** -- a stack machine over `bc.ops`. Each `Push*` pushes its value; `PushCell` recurses through `eval_cell`; an operator pops its operands and pushes the result; `CallFn` pops `n_args` and pushes the return. Every failure is a `CellValue::Error` pushed like any other value, so an error propagates through the operators above it rather than aborting -- which is what makes the VM total (`TRAME-007`, `TRAME-013`). The result is the top of the stack, `Empty` if it is empty.

**`eval_range`** -- resolve both endpoint pairs against the sheet index; a missing `Bound` endpoint returns a single `BrokenRef` for the whole range. Then walk the **logical** span row-major, preserving its shape for `VLOOKUP`/`INDEX`: a position past the materialized range reads `Empty`, a materialized one normalises to its `(row_uid, col_uid)` and recurses through `eval_cell`, so cycle detection covers ranges too.


---

## 5. BROKEN REFERENCES (`#REF!`)

A reference is broken when a `Bound` `Uid` is no longer in the `SheetIndex` — the
row or column has been deleted. (A `Virtual` axis is never broken: it reads
`Empty`, since it never referred to materialized data.)

**Detection at evaluation:**
- `Op::PushCell(r)` → resolving `AxisId::Bound(uid)` finds no position → `BrokenRef`
- `Op::EvalRange(dep)` → `dep.row_range(idx)` returns `None` when a `Specific(Bound(id))` endpoint is missing → `BrokenRef`

**Propagation after delete_row/delete_col:**

A row/column delete shifts every following position, so the MVP takes the
structural-reseed path (`logic/structural.rs`): the removed line's cells and
formatting are purged, the `SheetIndex` is rebuilt, and the positional formula
engine is reseeded (`reseed_engine`) -- every formula is re-bound against the new
order and recomputed. A formula that referenced the deleted `Uid` finds no
position in the rebuilt `SheetIndex` and evaluates to `BrokenRef`.

> **Incremental path (Phase 4, §11 bis.A).** The reseed is the current
> implementation; the target is to drive deletion from the Loro list diff and mark
> only the dependents of the deleted `Uid` dirty (via `collect_dirty`), avoiding the
> O(document) reseed.

No bytecode patching — the `AxisId::Bound(uid)` values in `CellId` and `RangeId` remain, absence in `SheetIndex` triggers `BrokenRef` at evaluation.

> **Divergence (`TRAME-008`), as measured 2026-08.** The paragraph above is the intent; the code does the
> opposite and the suite says so. The reseed re-binds every formula **from its raw text**, so `=A2` after
> deleting row 1 recompiles against the *new* order and reads the shifted (empty) cell rather than
> `#REF!` -- `logic::tests::group2::formula_reference_is_positional_after_delete` asserts exactly that and
> is green. The `BrokenRef` primitive is real and tested at the VM level
> (`formula::vm::tests::push_cell_deleted_bound_yields_ref_error`, `TRAME-007`), it is simply
> **unreachable from the delete path**: nothing survives the reseed still holding the deleted `Uid`.
> Today `#REF!` on a delete is reachable only for a **sheet** delete (`TRAME-009`) and for an off-grid
> shift during paste/fill (`formula::shift`). Deciding between the two behaviours -- positional like
> Excel, or uid-broken like §12's "Delete + Reinsert" -- is a product decision this spec has not made;
> it currently claims one and ships the other.

---

## 6. CELL MERGES

### CRDT Storage

```
"merges_{sheet_id}" (LoroMap) — LWW
  key   = "{row_uid}:{col_uid}"       (master cell = top-left corner)
  value = "{row_uid_end}:{col_uid_end}"  (bottom-right corner)
```

### Client-side rule: merge blocked if covered cell is non-empty

Before applying a merge locally (`TrameCommand::MergeCells`), the Logic thread checks that all covered cells (excluding the master) are empty in `LoroMap`. If a covered cell is non-empty or contains a formula, the merge is **rejected locally** with `TrameEvent::MergeRejected`. This is Excel and Google Sheets behavior. This rule only applies to local actions — it cannot prevent a concurrent conflict.

### Overlap Validation

LWW Loro resolves overlapping merge conflicts deterministically. Validation after each remote import: detection of cells covered by multiple merges → the merge with the most recent `ListItemID` (Loro order) wins, the other is removed.

### Concurrent Conflict Strategy: Data is Never Destroyed

**Scenario:** Alice sends `MergeCells(R3C2:R4C2)` when `R3C2` is empty on her side. Bob simultaneously sends `SetCellText(R3C2, "=SUM(...)")`. Both deltas converge — result: `R3C2` is both a merge master AND non-empty in `LoroDoc`.

This conflict is unavoidable in collaborative CRDT. Rejecting Bob's delta would break convergence. **Strategy: the merge masks covered values without destroying them.**

```rust
/// Field of TrameState -- lid_keys of covered non-empty cells.
masked_cells: HashSet<String>,
```

`detect_and_handle_merge_conflicts(sheet)` walks every merge on the sheet and, for each covered cell that still carries a raw value, inserts its key into `masked_cells` and emits `TrameEvent::MergeConflictDetected { master_cell, covered_cell }`. A masked cell is excluded from the DAG feed, not displayed and not computed for as long as the merge is active -- uniformly on every sheet, not only the active one (`TRAME-020`). The returned events join `TrameOutcome.display` on the local path and the `Vec<TrameEvent>` of `apply_remote` on the remote one.


Called after each `doc.import()` — returned events are added to the `Vec<TrameEvent>` of `apply_remote()`.

**Guaranteed behavior:**
- Bob's value exists in `LoroDoc` — not erased, not corrupted.
- It is invisible as long as Alice's merge is active (`masked_cells`).
- If the merge is removed → removal from `masked_cells`, `bytecode.invalidate()`, dirty propagation → the value immediately reappears.
- All clients converge to the same state: masking is deterministic (based solely on `merges_{sheet}` and the presence of `raw` in `LoroMap`).
- No corrective mutation generated: no risk of a second round of conflicts.

### DAG and Masked Cells

Cells in `masked_cells` are excluded from the `DependencyGraph` and `formula_cache`. They do not participate in any computation while masked. When the mask is lifted (merge split), `bytecode.recompile_if_stale()` and `deps.update()` are called normally.

### DisplayCell

```rust
// Master cell
DisplayCell { is_merged: true, merge_span: Some((col_span, row_span)), .. }
// Covered cells (empty or masked) -- UI thread does not render them
DisplayCell { is_merged: true, merge_span: None, display_text: String::new(), .. }
```

---

## 7. INTERNAL STATE

> **No Sync handle inside the state.** `TrameState` holds **no** `sync_tx` / `Box<dyn CommandSink>`. CRDT and ephemeral payloads leave the state as fields of the [`TrameOutcome`] returned by `handle_command`; the `ambre-logic` dispatcher is the single place that forwards them to the Sync thread (on a bare `flume::Sender<SyncCommand>`). The `document_id` is owned by the registry key, not by the state, so it is not a field here either.

```rust
pub struct TrameState {
    doc:           LoroDoc,
    active_sheet:  SheetId,
    sheet_indices: HashMap<SheetId, SheetIndex>,
    deps:          DependencyGraph,
    bytecode:      BytecodeCache,
    formula_cache: HashMap<CellId, CellValue>,
    dirty:         HashSet<CellId>,
    float_objects: rstar::RTree<FloatObject>,
    /// Cache name -> SheetId for resolving cross-sheet references
    /// in formulas (e.g. Sheet2!A1). Updated on each CreateSheet/RenameSheet/DeleteSheet.
    sheet_name_to_id: HashMap<String, SheetId>,
    viewport:      Viewport,
    tx_version:    u64,
    /// Last CRDT version vector exported to Sync -- produces incremental deltas
    /// via `ExportMode::Updates` in `handle_command`.
    last_exported_vv: VersionVector,
    /// Cells covered by a merge but non-empty (masked, not destroyed)
    masked_cells:  HashSet<String>,
    /// Undo/Redo local -- scope = session document ouverte
    undo_manager:  loro::UndoManager,
    /// Canonical identifier for the current user (received via LogicEvent::SessionReady,
    /// which carries `user_id: UserId`). Stable per-user -- not pub_key, which changes
    /// on key rotation.
    my_user_id:    UserId,
}

// Engine-side, uid-anchored cell identity (the compiled form of a positional
// `CellRef`). `AxisId::Bound(uid)` inside the used range follows the data;
// `AxisId::Virtual(pos)` past it stays positional and reads Empty (§3.A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId { pub sheet: u16, pub row: AxisId, pub col: AxisId }

/// Our own, move-invariant row/column identity (§2.A). Minted as
/// `{peer = document PeerID, counter = per-session monotonic}`, stored as the
/// value of an element in the `rows_/cols_` `LoroMovableList`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uid { pub peer: u64, pub counter: u32 }

impl Uid {
    pub fn to_key_part(&self) -> String { format!("{:016x}_{:08x}", self.peer, self.counter) }
}

#[derive(Debug, Clone)]
pub enum CellValue {
    Number(f64),
    Text(String),
    Boolean(bool),
    Error(FormulaError),
    Empty,
    Range(Vec<CellValue>), // VM internal type only -- never in formula_cache
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FormulaError {
    CircularReference,
    BrokenRef,
    ParseError(String),
    DivisionByZero,
    TypeError,
    InvalidRef(String),
    NameNotFound(String),
    NumError,
    NaError,
}
```

---

## 8. INTERFACE CONTRACT (API)

### `TrameState` — typed inherent methods (dispatched by `ambre-logic`)

`TrameState` exposes plain inherent methods and holds no transport handle of its own. The `ambre-logic` dispatcher owns the registry, routes by `document_id`, calls these methods, and forwards the results over the typed channels. Dirty recomputation (P4) runs synchronously at the end of each `handle_command` / `apply_remote`. No custom loop, no `setTimeout(0)`.

The state's surface (see `ambre-trame/src/logic.rs`):

```rust
// ambre-trame/src/logic.rs
use ambre_bus::{TrameCommand, TrameEvent};

impl TrameState {
    /// Process a UI command. Returns a typed `TrameOutcome`:
    ///   - `display`   : Vec<TrameEvent>  -> UI Thread (UiEvent::Viewport)
    ///   - `update`    : Option<Vec<u8>>  -> Sync (SyncCommand::SendUpdate)
    ///   - `ephemeral` : Option<Vec<u8>>  -> Sync (SyncCommand::SendEphemeral)
    /// A single command can yield several display events (e.g. ViewportData +
    /// DataReset). Loro mutation + dirty propagation + compute_all_dirty()
    /// all run synchronously here before the outcome is returned.
    pub fn handle_command(&mut self, cmd: TrameCommand) -> TrameOutcome { /* §8 dispatch */ }

    /// Apply a remote CRDT delta (decrypted by Sync). Imports into LoroDoc,
    /// rebuilds the derived caches, runs compute_all_dirty(), and returns the
    /// viewport/conflict events for the UI. No sync payload leaves here -- a
    /// remote import is never re-broadcast.
    pub fn apply_remote(&mut self, payload: &[u8]) -> Vec<TrameEvent> { /* import + recompute */ }

    /// Apply a remote ephemeral (presence) payload -> RemoteCursors events.
    /// The author identity is the **Sync-resolved `from_user_id`** (spoof-proof,
    /// `ambre-spec-sync.md §3`): the peer-authored payload carries only the cell position
    /// and a self-asserted, cosmetic `display_name` -- never a `user_id`.
    pub fn apply_ephemeral(&mut self, from_user_id: UserId, payload: &[u8]) -> Vec<TrameEvent> { /* cursors */ }

    /// Export a full CRDT snapshot (compaction / local persistence / SnapshotNeeded).
    pub fn export_snapshot(&self) -> Vec<u8> { self.doc.export(ExportMode::Snapshot) }
}
```

**Production dispatch (`ambre-logic::LogicState`).** The single place that touches the transport — the state never does. `document_id` filtering is the registry lookup, not an `if document_id != self.doc_id` inside the state:

```rust
// ambre-logic/src/lib.rs (condensed)
match (self.docs.get_mut(&document_id)?, command) {
    (DocState::Trame(state), AppCommand::Trame(cmd)) => {
        let outcome = state.handle_command(cmd);
        // Snapshot for local persistence whenever the doc changed.
        let snapshot = outcome.update.is_some().then(|| state.export_snapshot());
        for event in outcome.display {
            self.send_to_ui(UiEvent::Viewport { document_id, event: AppEvent::Trame(event) });
        }
        if let Some(payload) = outcome.update {
            self.send_to_sync(SyncCommand::SendUpdate { document_id, payload });
        }
        if let Some(snapshot) = snapshot { self.send_to_persist(document_id, snapshot); }
        if let Some(payload) = outcome.ephemeral {
            self.send_to_sync(SyncCommand::SendEphemeral { document_id, payload });
        }
    }
}

// Remote inbound (LogicEvent::RemoteUpdate / RemoteSnapshot):
let events = state.apply_remote(&payload);           // import + recompute
let snapshot = (!events.is_empty()).then(|| state.export_snapshot());
for event in events { self.send_to_ui(UiEvent::Viewport { document_id, event: AppEvent::Trame(event) }); }
// SnapshotNeeded -> SyncCommand::SendSnapshot { document_id, payload: state.export_snapshot() }
// SessionReady / ConnectionStatus / SyncWarning are handled by LogicState, not TrameState.
```

### Command Dispatch and Synchronous Recalculation

`handle_command(TrameCommand) -> TrameOutcome` is a `match` over the command, and every mutating arm follows the same four steps (`ambre-trame/src/logic/commands.rs`):

1. **mutate** the `LoroDoc` (and end with `doc.commit()` -- `TRAME-017`);
2. **invalidate** the bytecode of the touched cells;
3. **collect the dirty set** through the dependency graph and recompute it, synchronously (`TRAME-002`);
4. **return** a `TrameOutcome`: the display events, the incremental delta as `update`, presence as `ephemeral`. The state never names `SyncCommand` -- the `ambre-logic` dispatcher turns `update` into `SyncCommand::SendUpdate`.

Two shapes worth naming:

* **`ScrollViewport`** is P1: no mutation, no `update`, recompute the dirty cells inside the new window and answer with one viewport.
* **`SetCellText`** is P2: the full four steps, and it emits a fresh viewport **only if** the recompute touched the visible window. The UI has already updated its cache optimistically, so there is no edit ack (`TRAME-UI-010`).

`Undo` / `Redo` drive the `UndoManager` then rebuild the derived caches. Feature and interop commands fall through to their own handlers.

**`compute_all_dirty`** walks the dependency DAG in topological order and evaluates each dirty cell through a fresh `EvalContext`, writing results into `formula_cache`. It is synchronous and unbatched: recomputing 10 000 cells fits the P95 mobile target of §11 (< 50 ms), and the occasional lag only shows on a large remote batch -- the same trade-off Prose makes with full pagination.


> **Invariant `doc.commit()` per `TrameCommand`:** each `TrameCommand` handler that mutates `LoroDoc` must end with `self.doc.commit()`. Loro groups all mutations since the last commit into a single `UndoManager` item. Without an explicit commit, multiple successive `TrameCommand` values could be grouped into a single undo item — which would break the expected granularity (1 command = 1 undo item). A cell edit (`SetCellText`) commits on cell exit (via `TrameCommand::CommitEdit` or a delay), not on each keystroke, so that an entire cell edit forms 1 undo item.

### Priorities and Budgets

| Priority | Task | Trigger | Behavior |
|:---|:---|:---|:---|
| **P1** | Scroll | `ScrollViewport` | `compute_all_dirty()` synchronous. Padding 20–100 rows depending on velocity. `DisplayViewport` immediate with `is_dirty=true` on pending cells if recomputation is not finished (theoretical case Phase 2). |
| **P2** | Input / Structure | `SetCellText`, `SetCellsBatch`, `InsertRow`, etc. | Loro mutation → dirty propagation → `compute_all_dirty()` synchronous → `ViewportData` (+ `DataReset` when the region changed). |
| **P3** | Remote import | `LogicEvent::RemoteUpdate` | `doc.import()` → diff → dirty → `compute_all_dirty()` synchronous → `ViewportData`. **As-built:** the MVP routes every remote batch through a full engine reseed (O(document)), a known divergence from this diff-driven target that breaks the ≤50 ms budget at 100k cells — tracked as Phase 4 debt (§11 bis.A). |

**Anti-flickering:** `tx_version` incremented on each `DisplayViewport`. UI thread drops `version < last_processed`.

### Optional Phase 2 — Asynchronous P4 Yield

Full recomputation of 10k formula cells takes < 50 ms (target). During these 50 ms, the Logic thread is blocked — P1 (scroll) messages accumulate. This lag is **occasional** and **rare** (large `RemoteUpdate` during active scroll). It is accepted for the MVP, aligned with the Prose pattern.

If Phase 2 benchmarks show a real problem on mobile hardware, a yield mechanism can be added without touching the typed data plane or the `DocState` registry:

- `compute_all_dirty()` is replaced by `compute_dirty_batch(N)` which processes N cells and returns a boolean `more_dirty`.
- If `more_dirty`, `handle_command` adds a `TrameEvent::ComputePending` to `TrameOutcome.display`.
- The `LogicState` dispatcher in `ambre-logic`, when it sees this signal, re-enqueues a `TrameCommand::ContinueDirtyCompute` (e.g. via a self-send on the Logic command channel) so pending P1/P2 commands are drained first.
- The `DisplayViewport` is sent with `is_dirty=true` on cells not yet recomputed.

This mechanism is **additive** — it does not change `TrameState`'s method signatures or the `DocState` dispatch. It adds two variants (`TrameCommand::ContinueDirtyCompute`, `TrameEvent::ComputePending`) specific to Trame.

---

## 9. DATA-ORIENTED STRUCTURES (UI Thread)

> **Typed data plane (no postcard on the UI<->Logic hop):** `TrameCommand` (UI→Logic) and `TrameEvent` (Logic→UI) travel as **Rust values** inside `AppCommand::Trame` / `AppEvent::Trame` over the `flume` channels between the UI and Logic threads of the single WASM binary — they are **not** serialized (shared memory; see `spec-ambre-architecture.md §2/§6`). postcard is reserved for the Logic<->Sync `postMessage` hop (`SyncCommand`/`LogicEvent`) and for real network bytes. The UI thread sends a `LogicCommand::App { document_id, command: AppCommand::Trame(cmd) }`; the Logic thread `match`es it to the right `DocState` and dispatches.

> **Canonical definitions live in `ambre-bus::app::trame`.** `TrameCommand`,
> `TrameEvent` and `DisplayCell` are defined in `crates/ambre-bus/src/app/trame/`
> ({`command`, `event`}`.rs`); the shared value types (`CellStyle`, `StylePatch`,
> `NumberFormat`, `ClipboardPayload`, `FilterKind`, ...) live in the sibling
> `style`/`clipboard`/`features` modules. This section mirrors that surface for
> reference; the code is authoritative. `TrameCommand`/`TrameEvent`/`DisplayCell`
> derive only `Debug, Clone` -- **no `serde`** (they never cross the wire, only
> the shared-memory `flume` plane, `trame-features §7`). The value types they carry
> *do* derive `Serialize`/`Deserialize` because they are postcard-encoded into the
> clipboard payload and the CRDT style register.

The viewport reply is the `TrameEvent::ViewportData` struct-variant (below); there
is no separate `DisplayViewport` type on the bus -- the Logic thread builds the
variant's fields (`cells` / `col_widths` / `row_heights`) directly. `DisplayCell`
is flat and data-oriented:

```rust
#[derive(Debug, Clone, Default)]
pub struct DisplayCell {
    pub row: u32,
    pub col: u32,
    pub text: String,                    // formula result or raw string
    pub raw: String,                     // raw CRDT text (formula bar); == text for plain cells
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: bool,
    pub fg: Option<u32>,                 // 0xRRGGBB, None = theme default
    pub bg: Option<u32>,                 // 0xRRGGBB, None = transparent
    pub font: Option<String>,
    pub size: Option<u16>,
    pub borders: Borders,
    pub is_merged: bool,                 // master or covered cell (§6)
    pub merge_span: Option<(u32, u32)>,  // Some((col_span,row_span)) on the master only
    pub is_filtered: bool,               // row hidden by the active shared filter (features §1.A)
}

/// Resolved cell style sent to the UI thread (projection of the "styles" LoroMap).
/// The UI renders it; it never edits it directly -- every change goes through
/// `TrameCommand::SetCellStyle`. The `fmt` choice drives `display_text` formatting,
/// computed 100% in the Logic thread (the UI computes nothing).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct CellStyle {
    pub bold:      bool,
    pub italic:    bool,
    pub underline: bool,
    pub font:      Option<String>,    // None = default family
    pub size:      Option<f32>,       // None = default 11.0
    pub fg_color:  Option<Color>,
    pub bg_color:  Option<Color>,
    pub fmt:       NumberFormat,
    pub h_align:   HAlign,            // Auto | Left | Center | Right
    pub v_align:   VAlign,            // Top | Middle | Bottom
    pub wrap:      bool,             // automatic line wrapping
    pub border:    Borders,
}

/// Number/value display format. The value -> `display_text` conversion is done in the
/// Logic thread. The UI only *selects* the format (toolbar) via `SetCellStyle`.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub enum NumberFormat {
    General,
    Number     { decimals: u8, thousands: bool },
    Currency   { decimals: u8, code: CurrencyCode },   // EUR, USD, GBP, ...
    Percent    { decimals: u8 },
    Scientific { decimals: u8 },
    Fraction   { denom: FractionDenom },   // Halves | Quarters | Eighths | upto N significant digits
    Date(DatePattern),    // Iso | LocaleShort | LocaleLong
    Time(TimePattern),    // Hm | Hms
    Text,                 // force raw text, no numeric/date auto-cast
    Custom(String),       // Excel-style format code, e.g. "#,##0.00;[Red](#,##0.00)"
}
// "Special" (zip code, phone, SSN) is locale-specific and is NOT a distinct primitive:
// it is expressed via `Custom(..)` patterns rather than a hardcoded, US-centric variant.

/// Partial style update -- only `Some(..)` fields are applied. A toolbar control
/// sets a single field at a time (e.g. toggle bold -> patch { bold: Some(true), .. }).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StylePatch {
    pub bold:      Option<bool>,
    pub italic:    Option<bool>,
    pub underline: Option<bool>,
    pub font:      Option<Option<String>>,
    pub size:      Option<Option<f32>>,
    pub fg_color:  Option<Option<Color>>,
    pub bg_color:  Option<Option<Color>>,
    pub fmt:       Option<NumberFormat>,
    pub h_align:   Option<HAlign>,
    pub v_align:   Option<VAlign>,
    pub wrap:      Option<bool>,
    pub border:    Option<Borders>,
}

// `Color`, `HAlign`, `VAlign`, `Borders`, `CurrencyCode`, `DatePattern`, `TimePattern`, `FractionDenom`
// are defined in `ambre-trame/src/style.rs` and implement Serialize/Deserialize so
// they travel in `DisplayCell` / `TrameCommand` via postcard.

/// UI thread -> Logic thread. Canonical definition in
/// `ambre-bus::app::trame::command`. Positions are **grid positions** (`u32`); the
/// Logic thread resolves them against the tracked selection / active sheet and the
/// per-sheet `SheetIndex`. Parameterless "intent" variants carry no geometry -- the
/// Logic thread resolves it from the selection kept current by `SetActiveCell`.
#[derive(Debug, Clone)]
pub enum TrameCommand {
    // -- Grid & navigation (local view state) ----------------------------
    ScrollViewport { version: u64, row_start: u32, row_end: u32, col_start: u32, col_end: u32 },
    SetActiveCell  { row: u32, col: u32, anchor_row: u32, anchor_col: u32 },
    ActivateSheet  { sheet_id: String },

    // -- Cell input & edit grouping --------------------------------------
    SetCellText  { row: u32, col: u32, text: String },   // empty text clears the cell
    SetCellsBatch { cells: Vec<(u32, u32, String)> },    // one undo group (fill / range clear)
    CommitEdit,                                          // seal the open edit's undo group

    // -- Structure (grid positions) --------------------------------------
    InsertRow { at: u32 }, DeleteRow { at: u32 },
    InsertColumn { at: u32 }, DeleteColumn { at: u32 },
    MoveRow { from: u32, to: u32 }, MoveColumn { from: u32, to: u32 },
    ResizeCol { version: u64, sheet_id: String, col: u32, width_px: f32 },
    ResizeRow { version: u64, sheet_id: String, row: u32, height_px: f32 },

    // -- Merges ----------------------------------------------------------
    MergeCells { row: u32, col: u32, row_end: u32, col_end: u32 },
    UnmergeCells { row: u32, col: u32 },

    // -- Parameterless structural intents (context menu / toolbar) --------
    InsertRowAbove, InsertRowBelow, InsertColLeft, InsertColRight,
    DeleteSelectedRows, DeleteSelectedCols,
    MergeSelection, UnmergeActive,
    SortSelectionAsc, SortSelectionDesc, SortSheetAsc, SortSheetDesc,
    SortSelectionMulti { sort_keys: Vec<SortKey>, has_header: bool },
    RequestActiveColumnValues, RequestSelectionStats, CopyRange,
    ApplyFilterToActiveColumn { kind: FilterKind }, ClearActiveFilter,

    // -- Formatting ------------------------------------------------------
    SetCellStyle { patch: StylePatch },   // applied to the tracked selection

    // -- Clipboard / autofill --------------------------------------------
    PasteRange { target_row: u32, target_col: u32, payload: ClipboardPayload },
    PasteText  { text: String },                          // literal, at active cell
    PasteClipboardAtActive { payload: ClipboardPayload }, // styled, reindexed, at active cell
    FillRange  { sheet_id: String, src: GridRange, dest: GridRange, mode: FillMode },

    // -- Find & replace (whole-document scan, Logic-side) ----------------
    FindNext   { query: String, match_case: bool, whole_cell: bool },
    ReplaceAll { query: String, replacement: String, match_case: bool,
                 whole_cell: bool, in_formulas: bool, all_sheets: bool },
    ReplaceOne { query: String, replacement: String, match_case: bool,
                 whole_cell: bool, in_formulas: bool },

    // -- Undo / Redo -----------------------------------------------------
    Undo, Redo,

    // -- Named ranges ----------------------------------------------------
    DefineName { name: String, reference: String },
    DeleteName { name: String },

    // -- Sheets (lifecycle, §2.D) ----------------------------------------
    CreateSheet { name: String },
    RenameSheet { sheet_id: String, name: String },
    MoveSheet   { sheet_id: String, to_index: u32 },
    DeleteSheet { sheet_id: String },

    // -- Sort (ranged) ---------------------------------------------------
    SortRange { sheet_id: String, row_start: u32, row_end: u32, col_start: u32,
                col_end: u32, sort_col: u32, ascending: bool, has_header: bool },

    // -- Shared filter ---------------------------------------------------
    ApplyFilter { sheet_id: String, col: u32, kind: FilterKind },
    ClearFilter { sheet_id: String },

    // -- CSV interop (interop MVP) ---------------------------------------
    ImportCsv { data: Vec<u8>, delimiter: Option<u8>, has_header: bool,
                target_sheet_id: Option<String>, start_row: u32, start_col: u32 },
    ExportActiveSheetCsv,
    ExportCsv { sheet_id: String, delimiter: Option<u8>, range: Option<GridRange> },

    // -- Deferred feature surface (present in the enum, handlers stubbed) --
    // personal filter views / multi-col sort -> Phase 2; cond. formatting &
    // validation -> Phase 2/3; range protection -> Post-MVP.
    SaveFilterView { name: String, sheet_id: String, filter_col: u32, kind: FilterKind },
    ActivateFilterView { view_id: String },
    SortRangeMulti { sheet_id: String, row_start: u32, row_end: u32, col_start: u32,
                     col_end: u32, sort_keys: Vec<SortKey>, has_header: bool },
    AddCondFormat    { sheet_id: String, range: GridRange, rule: CondFormatKind,
                       style: CondStyle, priority: i64 },
    UpdateCondFormat { sheet_id: String, id: String, range: GridRange,
                       rule: CondFormatKind, style: CondStyle, priority: i64 },
    RemoveCondFormat { sheet_id: String, id: String },
    AddValidation    { sheet_id: String, range: GridRange, kind: ValidationKind,
                       on_invalid: ValidationOnInvalid, message: String },
    RemoveValidation { sheet_id: String, range: GridRange },
    SetProtection    { sheet_id: String, range: GridRange,
                       allowed_user_ids: Vec<UserId>, description: String },
    RemoveProtection { sheet_id: String, range: GridRange },
}

/// **Not yet on the bus (planned surface).** xlsx import/export
/// (`ImportXlsx`/`ExportXlsx`), PDF export (`ExportPdf`), collaborative-cursor
/// presence (`UpdateCursor` + `TrameEvent::RemoteCursors`, ephemeral plane), and
/// the sync `RequestSnapshot` are described by `trame-interop` / §7-§8 as design
/// targets but are **not** in the `ambre-bus` `TrameCommand` today. They join the
/// enum when their handlers land.

/// **Note:** `FilterKind`, `SortKey`, `CondFormatKind`, `CondStyle`,
/// `ValidationKind`, `ValidationOnInvalid`, `GridRange`, `StylePatch`,
/// `NumberFormat`, `ClipboardPayload`, `FillMode`, `HAlign`, `VAlign`, `Borders`
/// are defined in `ambre-bus::app::trame::{style,clipboard,features}` and derive
/// `Serialize`/`Deserialize` (postcard for the clipboard payload and the CRDT
/// style register). `UserId` comes from `ambre-core`. The `TrameCommand` /
/// `TrameEvent` enums themselves derive only `Debug, Clone`.

/// Logic thread -> UI thread. Canonical definition in
/// `ambre-bus::app::trame::event`. Grid positions (`u32`) throughout; a mutating
/// command that changes the visible region rides alongside a `DataReset` so the UI
/// re-fetches. `#REF!`/parse errors are not a distinct event -- they surface as the
/// affected cell's displayed text inside `ViewportData`.
#[derive(Debug, Clone)]
pub enum TrameEvent {
    // -- Engine / viewport -----------------------------------------------
    ViewportData { version: u64, row_start: u32, col_start: u32,
                   cells: Vec<DisplayCell>, col_widths: Vec<f32>, row_heights: Vec<f32> },
    DataReset,                                           // re-fetch the visible region
    SelectRange { r1: u32, c1: u32, r2: u32, c2: u32 }, // e.g. after unmerge
    StyleApplied { style: CellStyle, bold: bool, italic: bool, merged: bool, numeric: bool },
    SheetList { sheets: Vec<(String, String)>, active: String },
    MergeRejected { reason: String },
    MergeConflictDetected { master_row: u32, master_col: u32, covered_row: u32, covered_col: u32 },
    UndoState { can_undo: bool, can_redo: bool },
    Error { message: String },

    // -- Sort / filter / stats -------------------------------------------
    SortComplete,
    FilterApplied { hidden_row_count: u32, col: u32 },
    FilterCleared,
    ColumnValues { col: u32, values: Vec<String> },
    SelectionStats { sum: f64, avg: f64, count: u64, count_num: u64, min: f64, max: f64 },

    // -- Find & replace / clipboard --------------------------------------
    FindResult { matches: Vec<(u32, u32)>, active: Option<usize> },
    ReplaceDone { replaced: u32 },
    PasteRejected { reason: String },
    Clipboard(ClipboardPayload),

    // -- Deferred feature surface (emitted once handlers land) ------------
    FilterViewSaved { view_id: String },
    CondFormatsUpdated { version: u64 },
    ValidationError   { version: u64, row: u32, col: u32, message: String },
    ValidationWarning { version: u64, row: u32, col: u32, message: String },
    ProtectionDenied  { version: u64, row: u32, col: u32, description: String },
    ProtectionViolationWarning { row: u32, col: u32, from_user_id: UserId, description: String },

    // -- Interop (CSV MVP) -----------------------------------------------
    ImportComplete { sheet_id: String, rows: u32, cols: u32 },
    ExportComplete { filename: String, data: Vec<u8> },
}

> **Planned presence surface (not yet on the bus).** The collaborative-cursor
> types below and the `UpdateCursor` command / `RemoteCursors` event that carry
> them are the design target for ephemeral presence; they are **not** in the
> `ambre-bus` `TrameEvent`/`TrameCommand` today (§9 "planned surface" note). They
> land with `apply_ephemeral`.

/// Peer-authored `Ephemeral` wire payload. Carries ONLY the cell position + a
/// self-asserted, cosmetic display name. It deliberately does NOT carry a `user_id`:
/// the canonical identity is the Sync-resolved `from_user_id` delivered out-of-band on
/// `LogicEvent::RemoteEphemeral` (a peer cannot forge another user's id,
/// `ambre-spec-sync.md §3`).
#[derive(Serialize, Deserialize)]
pub struct PresencePayload {
    pub sheet_id:     SheetId,
    pub row:          u32,
    pub col:          u32,
    pub selection:    Option<(u32, u32, u32, u32)>,
    pub display_name: String,   // self-asserted, cosmetic (never trusted for identity)
}

/// Logic -> UI event payload. Assembled in `apply_ephemeral` from the trusted
/// `from_user_id` + the wire `PresencePayload`; `user_id` is set to `from_user_id`
/// and is NEVER read from the peer payload. `color` is derived from `user_id`
/// (same hash as `trame-ui::color_for_user` / `prose-ui::user_color`).
pub struct RemoteCursorPayload {
    pub user_id:      UserId,
    pub display_name: String,
    pub color:        u32,
    pub sheet_id:     SheetId,
    pub row:          u32,
    pub col:          u32,
    pub selection:    Option<(u32, u32, u32, u32)>,
}
```

> **Note.** `RemoteCursorPayload` would travel inside `TrameEvent::RemoteCursors` on the content
> plane (by move on `flume`), so it needs no `serde` of its own -- `TrameEvent` derives only
> `Debug, Clone`. The sole presence type that genuinely needs `serde` is the wire
> `PresencePayload` (postcard in the `Ephemeral` bytes).

### Shared Rendering Module — `ambre-trame/src/render.rs`

Rendering functions are parameterized on the `DrawSurface` trait (defined in `ambre-draw`). The **same code** serves both screen rendering (`WgpuSurface` in the UI thread) and PDF export (`KrillaSurface` in the Logic thread, lazy-loaded). These functions live in `ambre-trame`, not in a separate UI crate.

```rust
// ambre-trame/src/render.rs
use ambre_draw::DrawSurface;

/// Rendu des cellules d'un viewport sur n'importe quelle surface.
pub fn draw_cells_on<S: DrawSurface>(
    surface: &mut S,
    vp: &DisplayViewport,
    layout: &GridLayout,
) {
    for cell in &vp.cells {
        if cell.is_merged && cell.merge_span.is_none() { continue; }
        let (x, y, w, h) = layout.cell_rect(cell.row_index, cell.col_index);
        // fill, text, borders -- identical screen and PDF
        surface.fill_path(/* ... */);
        surface.draw_glyphs(/* ... */);
    }
}

pub fn draw_grid_lines_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
pub fn draw_col_headers_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
pub fn draw_row_headers_on<S: DrawSurface>(surface: &mut S, layout: &GridLayout) { /* ... */ }
```

The UI thread instantiates `WgpuSurface` and calls these functions in its `requestAnimationFrame` loop. The Logic thread instantiates `KrillaSurface` for PDF export (triggered by `TrameCommand::ExportPdf`).

### Data Flow

```
UI Thread
  │  flume send: LogicCommand::App { document_id, AppCommand::Trame(cmd) }   (typed value)
  ▼
ambre-logic — LogicState::handle_app_command()
  │  match (DocState::Trame(state), AppCommand::Trame(cmd))
  │  → state.handle_command(cmd) → TrameOutcome
  │       (LoroDoc mutation, compute_all_dirty(), build_viewport — all synchronous)
  │  → for ev in outcome.display: send_to_ui(UiEvent::Viewport { AppEvent::Trame(ev) })
  │  → outcome.update    → send_to_sync(SyncCommand::SendUpdate    { document_id, payload })
  │  → outcome.ephemeral → send_to_sync(SyncCommand::SendEphemeral { document_id, payload })
  ▼
Sync thread → encryption + signature → WsClientMsg::Update → Blind Relay

Sync thread → Logic thread (remote update)
  │  postMessage (postcard): LogicEvent::RemoteUpdate { document_id, payload }
  ▼
ambre-logic — LogicState::apply_remote()
  │  match DocState::Trame(state) → state.apply_remote(&payload) → Vec<TrameEvent>
  │       (doc.import, compute_all_dirty(), build_viewport — all synchronous)
  │  → for ev in events: send_to_ui(UiEvent::Viewport { AppEvent::Trame(ev) })
  ▼
UI Thread — re-render viewport
```

---

## 10. QUALITY & TESTABILITY

> Transverse test architecture: `ambre-spec-architecture.md §4`. **What this engine guarantees, and what verifies each guarantee, is the table in §1.** It is the only list of tests this spec keeps, because it is the only one joined to a claim -- a name here that no longer resolves is a lie the reader cannot detect, and this section used to hold seventeen of them.

* **Tiers.** **host** for the whole engine (`just test-native`): `TrameState`, the formula pipeline, the CRDT model and the render feed are pure Rust with no `cfg(wasm32)` gate. **wasm** covers the DOM chrome only and is specified in `trame-ui`. No **db** tier.
* **Seams.** `CommandSink` / `MockSyncHandle` for the Logic<->Sync boundary; `TrameState::with_peer` for a pinned `PeerID` and clock (`ambre-spec-fixtures.md §2`); `RecordingSurface` (`ambre-draw`, feature `testing`) for the render feed. Isolated, headless, no live network.
* **Mandatory belts.** An increment lands with these or it does not land:
  * **CRDT convergence** -- two peers, `proptest`-generated interleavings, identical documents (`TRAME-015`).
  * **Formula properties** -- totality over arbitrary input, exactness and order independence over generated ASTs (`TRAME-013`, `TRAME-014`).
  * **Cache vs rebuild** -- the incremental `SheetIndex` against a from-scratch rebuild (`TRAME-023`; **not written yet**, the one mandatory belt this engine is missing).
* **Benchmarks.** None today. The seven budgets of §11 are unfalsifiable prose until a `criterion` bench exists per budgeted operation (architecture §4, mechanism ladder); `ambre-crypto` is the only crate in the workspace carrying one.
* **Definition of Done.** An increment is `[x]` only when the mandatory belts above are green in CI **and** no invariant in §1 is left without a `Status`. A new guarantee arrives as a row in §1, never as a paragraph here.

---

## 11. PERFORMANCE & RESOURCES

| Operation | Target | Note |
|---|---|---|
| P1 response (scroll → `DisplayViewport`) | < 15 ms | |
| P2 response (`SetCellText` → viewport) | < 5 ms | Recompile bytecode + update DAG |
| P3 import (`doc.import` 1 KB) | < 2 ms | |
| P4 batch (50 cells) | < 1 ms | |
| 10k-cell DAG recomputation | < 50 ms | |
| SheetIndex rebuild 10k rows | < 10 ms | At load time |
| `range_index` scan (MVP, 1k range formulas) | < 1 ms | Phase 2: interval tree |

**Memory:** `LoroDoc` < 10 MB / 100k cells ; DAG + cache < 50 MB / 100k formula cells.

---

## 11 bis. SCALING & CONSISTENCY DEBT (2026-07 ARCHITECTURE REVIEW)

> Findings of the 2026-07-09 architecture review of `ambre-trame`. The identity layer
> (`Uid` / `SheetIndex` / `AxisId`, §2-§3) is sound. Findings A-E are recomputation strategies
> sitting on top of it, fixable without a document-format migration. **Finding F is the exception:**
> it is a divergence of the persisted schema itself (§2.B), so it must land before the public Beta,
> while no third-party document exists and the fix is a writer/reader change with no migration path
> to carry. Tracked as **Phase 4** Stories in §14 (Epic `E5` in `specs/ambre-roadmap.md`).

### A. Incremental remote apply (kill the O(document) reseed)

**Divergence from §8.** The P3 table (§8, Priorities and Budgets) prescribes
`doc.import() -> diff -> dirty -> compute_all_dirty()`. The implementation instead routes every
remote batch through `reseed_after_merge` -> `reseed_engine` (`logic/sync.rs`, `logic/sheets.rs`),
which does `engine.reset()` and re-parses/recompiles **every formula of every sheet**. The same
full reseed runs on undo/redo, sheet rename, define/delete name, and sheet switch/delete. In
active collaboration each peer keystroke costs O(total cells) on the receiver -- incompatible with
the <= 50 ms client processing budget (`ambre-context.md` §4) at the 100k-cell target.

**Target design (restores §8 P3).** Drive the apply path from Loro's subscription diff events:
- changed `cells_{sheet}` keys -> `engine.on_cell_changed` per touched cell (existing entry point);
- `rows_/cols_{sheet}` list diffs -> the incremental `SheetIndex::apply_insert/delete/move_*`
  methods (already implemented, §2.C);
- derived caches (merges, formatting, metrics, filter) rebuilt only for the containers that
  actually changed.

The full reseed remains the fallback for snapshot import (`import_snapshot`) and for any diff the
incremental path cannot classify. Second step (separate story item): shrink the per-batch
`TrameEvent::DataReset` to an invalidated-range event so the UI stops re-fetching the whole
viewport on every remote batch; `DataReset` stays as the fallback.

### B. Cached `SheetIndices` (divergence from §7)

The §7 state sketch carries a persistent `sheet_indices: HashMap<SheetId, SheetIndex>`. The
implementation instead rebuilds on **every call** to `engine_indices()` (`logic/cells.rs`): the
active sheet's index is cloned and every non-active sheet's index is reconstructed by reading its
full Loro lists. `engine_indices()` runs per `write_cell` (per cell inside `SetCellsBatch`), per
`finish_cell_writes`, and per `ScrollViewport` -- a scroll frame on sheet 1 rebuilds sheet 2's
50k-row index. This threatens the P1 < 15 ms budget on any multi-sheet document.

**Target.** Materialize `SheetIndices` as state with per-sheet invalidation (structural op or
import on that sheet), the active entry sharing the maintained `Self::index` by borrow, not clone.
Engine call sites take the cached map by reference.

### C. Stable engine sheet indices (de-pin the active sheet from 0)

The active sheet is pinned to local engine index 0 (`rebuild_sheet_maps`), so `ActivateSheet`
remaps every `CellId.sheet` and forces a full `reseed_engine`: switching a tab costs O(all cells).
This couples a *view* state (which sheet is displayed) to engine identity.

**Target.** Intern sheet indices per session at first sight, stable for the document lifetime.
A sheet switch then rebuilds only the active-sheet display caches (formatting, merges, metrics,
filter) -- no engine reset. The `CellRef::new` sheet-0 convenience is retired in favor of the
explicit index the call sites already carry.

### D. Dependency-graph and engine micro-fixes

| Finding | Fix |
|---|---|
| `DependencyGraph::remove` rebuilds the whole `cell_to_node` map per removal; its comment claims `StableGraph` may reindex on removal -- it never does (that is the `Stable` guarantee). | Drop the O(N) rebuild; remove the map entry only. |
| Static cycle check runs `is_cyclic_directed` on the **whole** graph per formula edit: O(V+E) per keystroke. | Targeted check: a new cycle exists iff `formula_cell` is reachable from one of its newly added deps (DFS bounded to the affected region). |
| `compute_all_dirty` walks a `Topo` over the **entire** DAG and skips non-dirty nodes: O(all formula cells) per recompute, on every scroll. | Topological order restricted to the dirty closure (Kahn over the dirty set's induced subgraph). |
| Dead tail loop in `compute_all_dirty` re-inserts values already in the cache (no-op). | Delete. |
| `formula_cache: RefCell<HashMap<...>>` -- interior mutability is no longer needed (results are staged in a `Vec` before write-back). | Plain field + disjoint borrows, per the §7 sketch. |

### E. View-independent merge masking (consistency) -- LANDED

`masked_cells` is excluded from the engine feed **only for the active sheet** (`reseed_engine`
skips masked keys only at local index 0). A cross-sheet reference to a masked, conflicted cell
therefore evaluates differently depending on which sheet is currently displayed. This contradicts
§6 ("DAG and Masked Cells": masked cells do not participate in any computation) and the rule that
formula results never depend on view state.

**Target.** Derive masking per sheet from `merges_{sheet}` at feed time and exclude uniformly on
every sheet; add a cross-sheet convergence test asserting a conflicted masked cell reads
identically regardless of the active tab. *Tracked as an Epic `E3` / Phase 1 Story (a formula
result that depends on view state is wrong at any document size), not with the Phase 4 scaling
debt.*

**Done.** `TrameState::masked_cells_for(sheet_id, index)` (`logic/merges.rs`) derives the masked set per
sheet and `reseed_engine` excludes it uniformly, on every sheet. The cross-sheet test this section asked
for exists and is green: `logic::tests::group3::masked_cell_excluded_across_sheets_regardless_of_active_tab`,
which drives two peers into a conflicted masked cell and asserts an identical read from either active tab.
Recorded as `TRAME-020` / `TRAME-021` in §1. *Found by seeding the invariant table -- the section had been
carrying a fixed problem as open debt, which is the cheaper half of the same drift `§11 bis.F` shows in the
other direction.*

### F. Consolidate the per-cell style fan-out (schema divergence from §2.B) -- LANDED

**Original divergence from §2.B.** The schema prescribes one style reference per cell (the `"s"` key
of the cell sub-map) pointing into a shared `styles` map. The implementation instead **used to**
scatter one cell's style across up to **11 per-sheet `LoroMap`s**, each an independent LWW register
keyed by the cell id pair: `format_` (a legacy bold/italic mask), `style_` (the packed rich mask:
underline, wrap, alignments, border presence/style, number-format family, decimals, font size),
`fgcolor_`, `bgcolor_`, `bordcolor{0..3}_` (one per edge), `font_` and `numfmt_`.

**Resolution (landed).** The style fan-out is now **one register per cell**: a per-sheet map
`cellstyle_{sheet}` whose value is the whole style -- bold/italic plus the full `CellStyle` --
postcard-encoded into a single binary LWW register (`StyleRecord { bold, italic, style }`, see
`logic/styles.rs`). The packed-mask machinery (`pack_style_mask` and the `STYLE_*`/`FMT_*` bit
constants) is deleted; `CellStyle` is stored serde-verbatim. `set_cell_style` pre-materializes the
selection, writes every register without committing, then commits **once** inside one `UndoManager`
group -- so a selection restyle is one delta and one undo item.

This closed four problems at once:

1. **Write amplification.** `set_cell_style` touched 9 containers per cell and committed **per cell**
   inside the selection loop (N Loro transactions, each fanned out over 11 containers, compressed and
   encrypted on the sync path). Now: one register, one commit per selection.
2. **Two sources of truth for font style.** `format_{sheet}` carried only bold/italic while
   `style_{sheet}` held a richer mask -- a vestigial container with duplicated `DisplayCell`
   plumbing. Folded into the single record.
3. **Border-colour tear (correctness).** Border *presence/style* lived in the `style_` mask while
   border *colour* lived in `bordcolor{edge}_`: two concurrent edits of one edge could converge on
   one peer's presence with the other's colour -- a torn value inside a single conceptual attribute.
   Gone: the whole edge is in one register.
4. **Colour/font reseed data-loss (correctness).** `reseed_styles` iterated only the `style_` mask
   map; a cell whose *only* style was a colour or font had mask 0, hence no `style_` entry, hence was
   **dropped on reload** -- the colour/font vanished on reopen. Gone: reseed iterates the one map
   that holds every styled cell.

**Convergence semantics (revised -- this replaces the earlier "not a convergence defect" note).**
Style is now atomic **per cell**: two peers concurrently restyling the same cell race exactly once,
so the converged cell always carries a style *one* peer actually wrote -- never a per-attribute blend
across containers. This is a deliberate choice of **whole-cell last-writer-wins** over per-attribute
merge, documented as an assumed Excel difference in §12. The earlier claim that the fan-out was "not
a convergence defect" was **wrong on two counts**: it only checked two *whole-cell* rewrites (which
did converge, because each cell's containers were written at adjacent Lamports) and missed both the
*subset*-vs-whole tear and the border-colour tear above. Locked in by two regression tests in
`logic/convergence_tests.rs`: `concurrent_whole_cell_style_edits_do_not_blend_across_containers`
(the original whole-cell case) and `subset_style_edit_does_not_tear_a_whole_cell_rewrite` (a
colour-only patch racing a whole-cell rewrite, which tore under the old schema).

*Why whole-cell and not per-attribute:* per-attribute merge (Alice bolds while Bob recolours, both
stick) is what a **server-authoritative** suite like Google Sheets does cheaply because a central
server linearizes every op. Ambre is a blind relay with no server-side content ordering, so
per-attribute merge costs exactly the multi-container fan-out we just removed (and it was inconsistent
anyway: the `style_` mask already merged underline/wrap/aligns as one block). Whole-cell LWW is
predictable, matches a single "apply this style" user action, and is closer to Excel co-authoring
(which resolves same-cell conflicts coarsely, even escalating to a manual merge pane). A future
per-attribute model, if ever justified, would be a nested `LoroMap` per cell -- not a return to the
flat fan-out.

**Deliberately still out of scope.** The shared `styles` dictionary and the `"s"` style-id
indirection are a *deduplication* optimization (storage O(distinct styles)), orthogonal to the
consolidation above and with a real cost: Loro does not garbage-collect orphaned map entries, so the
indirection forces a reclamation policy for unreferenced styles plus a dereference on every cell
read. Deferred until benchmarks justify it -- the same treatment §14 Phase 3 gives the
`SheetRangeIndex` interval tree. The `{v,t,s}` cell sub-map falls under the same rule: `"t"` is a
type cache the engine already rederives.

---

## 12. ASSUMED SEMANTIC DIFFERENCES WITH EXCEL

Trame is not a semantic clone of Excel. These differences are **intentional**, documented, and motivated by the collaborative CRDT model. They must be clearly communicated to users coming from Excel.

### References After Structural Operations

**Excel:** inserting a row at position 3 implicitly rewrites `=SUM(A1:A10)` to `=SUM(A1:A11)`. This rewriting is performed by the Excel engine at the time of the operation.

**Trame:** references are stable `Uid` values. Inserting a row within the range `A1:A10` naturally includes the new row in the computation (it lands positionally between the endpoints), without any rewriting. The formula references the same physical rows + the new row inserted within the range.

**Visible difference:** if the user inserts a row **outside** the range (before A1 or after A10), Excel does not modify the formula. Trame does not either — identical behavior. The difference is only visible for insertions **within** the range, where Trame naturally includes the new row while Excel must rewrite.

**Why this is correct for a CRDT:** Excel's implicit rewriting is a non-idempotent operation. If Alice and Bob each simultaneously insert a row in the same range, two concurrent rewrites produce an inconsistent state. The `Uid` model avoids this problem.

### References After Sort

**Excel:** after a sort, absolute references (`$A$1`) do not move, relative references (`A1`) move with the data.

**Trame:** a sort is a **pure permutation of the row order** (`rows_{sheet}.mov`, §2.A) -- it never rewrites a cell. Cells are keyed by their move-invariant row/column `Uid`, so the data follows the reorder for free, and every reference, being a `Uid` too, **follows the same physical rows**. `=SUM(A1:A10)` after sort still references the same 10 physical rows, now in a different order. Behavior equivalent to Excel absolute references (`$A$1`).

**Visible difference:** if the user expected a formula referencing "row 3" to point to "what is now in third position after the sort", Trame does not do this. This is a choice consistent with the CRDT model.

### References After Delete + Reinsert

**Excel:** if a row is deleted then reinserted at the same position, Excel can find the cell "by position" in some cases.

**Trame:** reinsertion mints a new `Uid`. The old `Uid` no longer exists in the `SheetIndex`. Any formula that referenced the old id produces `#REF!` — even if a new row is inserted at the same visual position. The user must retype or copy the formula to point it to the new `Uid`.

**Why this is correct:** deletion is an explicit intent. Finding "by position" after delete+reinsert is a fragile heuristic, especially in collaborative mode where the position may have changed between the two operations on different clients. `#REF!` is the correct information: the referenced cell no longer exists.

### Cross-Sheet References After Sheet Operations

**Excel:** renaming a sheet rewrites the name token in every dependent formula; deleting a sheet leaves `#REF!`.

**Trame:** cross-sheet references depend on the stable `sheet_id` (§2.D), not the display name. **Rename** therefore changes nothing in the dependency graph -- the formula keeps computing and only its *displayed* qualifier follows the new name. **Reorder** is pure presentation and never touches a reference. **Delete** is the sole sheet operation that removes a `sheet_id`, so it is the only one that yields a cross-sheet `#REF!`, by the same absence-triggers-broken-ref rule as a deleted row (§5).

**Why this is correct for a CRDT:** Excel's rename-rewrite is the same non-idempotent rewrite hazard as its row-insert rewrite (§12, "References After Structural Operations"). Binding to `sheet_id` makes rename and reorder convergent and reference-preserving, leaving `#REF!` to mean exactly one thing: the target no longer exists.

### Concurrent Cell Formatting

**Excel / Google Sheets:** both are server-authoritative (a central server linearizes every op), so they can merge concurrent formatting of the same cell at a fine grain. Sheets in particular tends toward per-attribute merge (Alice bolds while Bob recolours -> both stick); Excel co-authoring is coarser and can escalate a same-cell conflict to a manual merge pane.

**Trame:** a cell's whole style is **one CRDT register** (`cellstyle_{sheet}`, §2.B / §11 bis.F), so concurrent restyles of the same cell resolve by **whole-cell last-writer-wins**: the converged cell carries the style *one* peer wrote, never a per-attribute blend. This is atomic **per cell, not per selection** -- a `SetCellStyle` over a range writes each cell's register independently, so two concurrent restyles of *overlapping, differently-shaped* selections can leave the overlap split cell-by-cell between the two writers (each cell still internally consistent). Applying the style again reconciles it.

**Why this is correct for a blind relay:** per-attribute merge is only cheap with a server that orders content; Ambre's server is a blind relay that never sees plaintext, so the only content ordering is per-register LWW. Splitting one cell's style back over ~11 registers to fake per-attribute merge is exactly the fan-out §11 bis.F removed (and it tore borders and lost colours on reload). Whole-cell LWW is predictable, matches a single "apply this style" intent, and is closer to Excel than to Sheets. A finer model, if ever justified, is a nested per-cell `LoroMap` -- never the flat fan-out.

---

## 13. REJECTED ARCHITECTURAL DECISIONS

| Alternative | Rejected because |
|---|---|
| **Positional addressing `R{row}C{col}`** | Row insert = O(n) CRDT ops. `LoroMovableList` = 1 op. |
| **Sparse run-length rows + order-statistic tree** | Stores only non-empty rows with a shared `gap_before` counter to avoid materializing empty rows. **Not convergent:** two concurrent inserts that split the same gap both `set` the shared counter -> LWW keeps one -> absolute numbering of every following row corrupts identically on all replicas. Dense-over-used-range (each row = its own element, no shared counter) is convergent and intent-preserving; the tree would only speed the query, not remove the hazard. A `ROW_MATERIALIZE_CAP` bounds the memory cost the sparse model aimed to avoid. |
| **Loro slot/cursor id (`ListItemId`) as the row/column identity** | The element's op id (`get_cursor(pos).id`) looks stable, but `LoroMovableList::mov(from, to)` **reassigns the moved element's op id** to the move op. Every cell keyed by that slot id is orphaned on the first reorder, so a sort could only be done by rewriting cell contents (see next row). We instead mint our **own** `Uid` and store it as the element's *value*, which `mov` leaves untouched (§2.A). (This reverses the earlier "Loro already gives stable ids, a UUID is redundant" call: it holds for insert/delete but **not** for `mov`.) |
| **Sort by rewriting cell contents into fixed row slots** | A workaround for the slot-id hazard above: keep the row slots fixed and copy values/formatting into sorted order. It is O(cells) not O(rows), churns the CRDT (every moved cell is a write that replicates and lands on the undo stack), and **breaks id-based references** (a reference to "the row that moved" stays pinned to the now-overwritten slot -> positional, not the §12 follow-the-data ideal). The owned-uid model makes sort a single `mov` permutation instead. |
| **Uid-bound *range* endpoints (ranges follow the data like cells)** | A range bound to two row `Uid`s breaks under a sort: the endpoints reorder independently and can cross, inverting the rectangle (`SUM(A1:A3)` over a sorted block collapses to `0`). A rectangle cannot follow a non-contiguous set of rows. Ranges are compiled with **positional** (`Virtual`) endpoints -- a region/block -- so an internal sort leaves the block and its total intact (Excel/Sheets behavior). Only single-cell references bind to a `Uid` and follow the data (§3.A, §12). |
| **`{client_hex}{counter_hex}` format without fixed width** | Ambiguous without separator. Fixed encoding `{peer:016x}_{ctr:08x}` chosen (shared by `Uid` and `sheet_id`). |
| **100 petgraph edges for a range** | Memory explosion + O(n) updates on insert. Separate `range_deps`. |
| **Interval tree 2D (MVP)** | Premature. Linear scan sufficient for < 1k formulas with ranges. Phase 2. |
| **Direct AST evaluation** | Re-parse on each recomputation. Bytecode = compile once, eval N times. |
| **Computed values in Loro** | Pointlessly replicated. Each client recomputes from `raw`. |
| **Loro for formula cycle detection** | Loro DAG manages CRDT causal order, orthogonal to formula dependencies. |
| **Rayon (MVP)** | 4th Web Worker context unjustified before 100k cells. Phase 3+. |
| **Per-attribute style merge (one CRDT register per style attribute)** | Splitting a cell's style over many registers (the old `format_`/`style_`/`fgcolor_`/... fan-out) to let concurrent edits of different attributes both survive. Only cheap with a server that orders content (Google Sheets); on a blind relay it forces per-register LWW with **no** cross-register atomicity -> whole-cell edits tear (border presence vs colour) and colour/font-only cells were lost on reload (§11 bis.F). Replaced by one whole-cell register with whole-cell LWW (§12, "Concurrent Cell Formatting"). A finer model, if justified, is a nested per-cell `LoroMap`, not a flat fan-out. |

---

## 14. ROADMAP

> Tracked as Stories of Epic **E3** (and the engine core, already delivered under **E1**) in
> `specs/ambre-roadmap.md`.

**Phase 0 — Core standalone**
- [~] Multi-sheet `LoroDoc`, per-sheet CRDT schema (§2.B: `sheets`, `cells_/format_/rows_/cols_{sheet}`, dense stable-id model) *(Landed: the structural foundation -- implicit default sheet, lazy materialization, `MATERIALIZE_CAP`, explicit add-sheet + `sheet_order` (with the tabs UI), and `filter_hidden_{sheet}` (with the shared filter). `cond_formats`/`validations`/`protections` legitimately await their Phase 2 / Post-MVP features.*
  **Still open, and the reason this box is `[~]`:** the `{v,t,s}` cell sub-map and the shared
  `styles` map were never implemented -- values are flat in `cells_{sheet}`. Split in two by the
  2026-07-09 review (§11 bis.F): **(a)** consolidating the style fan-out into one container per cell
  is **DONE** -- styles now live in one `cellstyle_{sheet}` register per cell (whole-cell LWW), which
  also fixed a border tear and a colour/font reseed data-loss; **(b)** the shared `styles` dictionary
  + `"s"` indirection (a *dedup* optimization) and the `{v,t,s}` sub-map are **deferred behind a
  benchmark** and are not a prerequisite for anything.*
- [x] Bidirectional `SheetIndex` (`model::SheetIndex`, dense over used range) + `sheet_name_to_id`
- [x] `nom` parser: arithmetic, `CellRef`, bounded `Range` (`A1:B10`), basic functions, cross-sheet references (`Sheet2!A1` via `sheet_name_to_id`). `RangeEndpoint::EntireAxis` (A:A / 1:1) is reserved in the type but not parsed yet -- Phase 3.
- [x] Bytecode VM: full `Op` set, `compile()`, `execute()`, `EvalContext`
- [x] Two-level `DependencyGraph` + `collect_dirty`
- [x] Cycles: static Kosaraju + dynamic `eval_stack` (cell-to-cell, ranges, cross-sheet)
- [x] `#REF!`: detection via `SheetIndex.contains_*`
- [x] `TrameCommand` and `TrameEvent`: consolidated enums (§9, single source of truth) *(MVP subset; extended per feature)*
- [x] Unit tests + property tests + postcard round-trip `TrameCommand`/`TrameEvent`

**Phase 1 — Logic Thread Integration**
- [x] Logic-thread command dispatch (`handle_command` returning `TrameOutcome`, §8) *(typed `DocState` + `match`; see `spec-ambre-architecture.md §6`)*
- [x] Synchronous recomputation `compute_all_dirty()` in each handler
- [x] `DisplayViewport` + anti-flickering (`tx_version`)
- [x] Insert/delete rows/columns (`TrameCommand::Insert/DeleteRow/Column { at }`): one `LoroMovableList` op, `rebuild_index` + positional engine reseed + formatting reseed; delete purges the removed line's cells/formatting; no-op past the used range *(Epic `E3`)*
- [x] Merges: `merges_{sheet}` LWW map (master/extent stable-id pairs), client-side non-empty rejection (`MergeRejected`), `DisplayCell.is_merged`/`merge_span`, masked covered cells (`masked_cells`, excluded from the engine, restored on unmerge), deterministic overlap precedence, conflict detection (`MergeConflictDetected`) on import. Merges follow their cells across structural ops *(Epic `E3`)*
- [x] **View-independent merge masking (correction, §11 bis.E).** Masking was excluded from the engine feed only for the active sheet (`reseed_engine` skipped masked keys only at local index 0), so a cross-sheet reference to a masked conflicted cell evaluated differently depending on the displayed tab -- a formula result that depended on view state, contradicting §6 ("masked cells do not participate in any computation"). Fixed: masking is now derived per sheet from `merges_{sheet}` at feed time via `masked_cells_for(sheet_id, index)` (the side-effect-free masking half of `rebuild_merges`) and excluded uniformly on every sheet; for the active sheet it returns exactly `self.masked_cells`. Covered by the 2-peer cross-sheet regression `masked_cell_excluded_across_sheets_regardless_of_active_tab` (a conflicted masked cell reads identically whether or not its sheet is the active tab). *(Epic `E3`)*
- [x] Shared rendering module `ambre-trame/src/render.rs`: `GridLayout` + `draw_cells_on<S>()`, `draw_grid_lines_on<S>()`, `draw_col_headers_on<S>()`, `draw_row_headers_on<S>()`, all `DrawSurface`-generic. `ui::renderer` composes them (cells layer + header overlay); uniform-metrics `GridLayout` (variable widths/heights land with the resize story below); `DisplayViewport`-fed PDF path lands with `export_pdf.rs` (Phase 3). *(Epic `E3`)*
- [x] `MockSyncHandle` for controller tests (via `CommandSink`) *(`ambre-bus::CommandSink` seam + `ambre-trame::test_support::MockSyncHandle`: captures emitted `SyncCommand`s and relays CRDT deltas peer-to-peer. Production forwards `TrameOutcome` payloads from `ambre-logic`; the test seam mirrors that exact relay via `MockSyncHandle::absorb`, so tests drive the same `SyncCommand`s the dispatcher emits) (Epic `E3`)*
- [x] Undo/Redo via Loro `UndoManager` (`TrameCommand::Undo/Redo` -> `undo_manager.undo()/redo()` + full derived-cache reseed via `after_structural` + `flush_delta` broadcast of the inverse ops; `TrameEvent::UndoState { can_undo, can_redo }`; chrome `UNDO`/`REDO` commands bound to Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y). Implicit row/column materialization is committed under an excluded origin (`add_exclude_origin_prefix`) so undo reverts only the value and the cell key still resolves to its stable list-element ids on redo *(Epic `E3`)*
- [x] **Own-uid cell model (§2.A) + id-based engine (§3, §12).** Each `rows_/cols_` element carries a minted `Uid` as its *value* (replacing the Loro cursor id); `SheetIndex` reads values and gained `apply_move_row`. The compiled engine is uid-anchored: positional AST `CellRef`/`RangeDep` are resolved at compile time into `CellId`/`RangeId` over `AxisId { Bound(Uid), Virtual(pos) }` (a reference past the used range stays virtual and reads `Empty`). `SortRange` is a single `mov` permutation that **preserves** the uid-anchored bytecode (no reseed/re-parse) and recomputes, so single-cell references follow the moved data -- the §12 guarantee, proven by `sort_range_formulas_follow_data`. Ranges are positional blocks (`Virtual` endpoints), so `SUM(A1:A3)` over a sorted block stays intact (`sort_range_sum_block_is_stable_under_internal_sort`; uid-bound endpoints would invert -- see §13). Other structural ops still reset+reseed (re-bind against the converged order). Landed in stages, 164 tests green. *(Epic `E3`)*
- [x] Manual cell formatting (`TrameCommand::SetCellStyle` + `StylePatch`): write the extended `CellStyle` (bold/italic/underline, font/size, fg/bg, `NumberFormat`, h/v align, wrap, borders) into the `styles` LoroMap; value -> `display_text` formatting per `NumberFormat` done in the Logic thread. Emits `StyleApplied`. *(Epic `E3`)*
- [x] Clipboard paste (`TrameCommand::PasteRange` + `ClipboardPayload`): relative-formula reindexing + style application from the anchor; `PasteRejected` on protected range. (Copy/cut stay UI-local.) *(Epic `E3`)*
- [x] Autofill (`TrameCommand::FillRange` + `FillMode`): Copy replication or Series increment (numeric/date pattern detection). *(Epic `E3`)*
- [x] Find & replace (`ReplaceAll` / `FindNext`): full-document scan (UI cache is viewport-only); `FindResult` / `ReplaceDone`. *(Epic `E3`)*
- [x] Selection aggregates (`RequestSelectionStats` -> `SelectionStats`: sum/avg/count/count_num/min/max) for the status bar; computed in the Logic thread since the selection can exceed the viewport. *(Epic `E3`)*
- [x] **Column/row resize (`ResizeCol` / `ResizeRow`).** `col_widths_{sheet}` / `row_heights_{sheet}` LWW maps keyed by col/row `Uid` (§2.B), clamped to a minimum (`logic/metrics.rs`); the `ViewportData` reply carries per-column `col_widths` / per-row `row_heights` slices. **Variable-metrics `GridLayout`** (§9): cumulative offsets from the width/height maps (default for unset axes), via `col_w`/`row_h`/`col_left`/`row_top`/`col_at`/`row_at`, used by the screen path (the PDF path passes empty maps = uniform). Undoable (plain CRDT write); LWW convergence on concurrent resize covered by a 2-peer test. *(was Phase 2; promoted to MVP.)*
- [x] **Multi-column sort (`SortRangeMulti`).** Lexicographic `sort_by` over `sort_keys` (decreasing priority), same `compare_cell_values` precedence and `has_header` pin as single-column sort, realised as the same pure `rows_{sheet}` `mov` permutation (uid-anchored bytecode preserved, references follow). Emits `SortComplete` + `DataReset`. *(was `-features` Phase 2; promoted to MVP -- see `-features` §2. Landed with the `SortDialog` UI; the ranged `SortRangeMulti` and the parameterless `SortSelectionMulti` share `realize_sort` -- see `-features` §2.)*
- [x] **CRDT convergence property-test (test debt, Epic `ET`).** `proptest`-generated interleaved sequences of `SetCellText` / insert-delete row-column / `SortRange` / merge / `FillRange` applied across 2 peers, asserting identical final states after full history exchange (per `ambre-spec-architecture.md §4`). Primary target: references-after-sort and structural-op reconciliation (the own-uid model, §2.A/§12). Includes recompute determinism (a fresh peer rebuilt from the converged snapshot renders the same grid). *(`ambre-trame/src/logic/convergence_tests.rs::two_peers_converge_under_interleaved_ops`: routes a generated `(peer, op)` program over a bounded 5x4 grid, exchanges full snapshots both ways, and `prop_assert_eq!`s the materialized window -- text, formula results, bold, merge geometry. Uses snapshots rather than incremental `SendUpdate` deltas, since each peer's implicit-s0 `new()` op is a private dependency an incremental delta would leave dangling.)* **DoD:** required for the MVP to be considered tested.

**Phase 2 — Core features (~50 functions)**
- [x] SUM, AVERAGE, MIN, MAX, COUNT, IF, AND, OR *(plus ABS, COUNTA, CONCATENATE, LEFT, RIGHT, MID, LEN, LOWER, UPPER, NOT, POWER, ROUND, SQRT, TRIM -- 22 implemented)*
- [x] VLOOKUP, INDEX, MATCH, IFERROR, SUMIF, COUNTIF *(2D `RangeValue` added to `CellValue::Range` so lookups can address cells by row/col; criteria parser for SUMIF/COUNTIF) (Epic `E3`)*
- [x] TEXT, CONCAT *(`CONCAT`/`CONCATENATE` already present; `TEXT` minimal numeric formatting: decimals + thousands grouping) (Epic `E3`)*
- [x] TODAY, NOW, DATE, YEAR, MONTH, DAY *(Excel serial numbers, epoch 1899-12-30; evaluation clock threaded through `EvalContext::now_serial` -> `call_builtin`, sourced from `functions::now_serial()` (js-sys on wasm, std::time native)) (Epic `E3`)*
- [x] INT, MOD, ROUNDUP, ROUNDDOWN, CEILING, FLOOR, PRODUCT, SUMPRODUCT, MEDIAN, COUNTBLANK, XOR, ISBLANK, ISNUMBER, PROPER, SUBSTITUTE, REPT, FIND, VALUE *(first MVP extension batch; all single-result, no dynamic arrays) (Epic `E3`)*
- [x] **Second MVP batch -- core Excel coverage (~113 built-ins total):** *(Epic `E3`)*
  - Financial: PMT, FV, PV, RATE, NPER, NPV, IRR *(annuity formulas with Excel sign convention; RATE/IRR via Newton-Raphson, `#NUM!` on non-convergence)*
  - Conditional aggregation: SUMIFS, COUNTIFS, AVERAGEIF, AVERAGEIFS *(multi-criteria reuse the SUMIF/COUNTIF `Criterion` parser; ranges paired by index)*
  - Statistics: STDEV, VAR (sample, `n-1`), LARGE, SMALL, RANK, MODE
  - Math: TRUNC, SIGN, SUMSQ, EXP, LN, LOG, LOG10, PI, SIN, COS, TAN, RADIANS, DEGREES
  - Logic/info: IFS, SWITCH, ISERROR, ISNA, IFNA, ISTEXT, ISEVEN, ISODD, TRUE, FALSE
  - Text: SEARCH, REPLACE, EXACT, TEXTJOIN, CHAR, CODE
  - Date/time: TIME, HOUR, MINUTE, SECOND, WEEKDAY, EDATE, EOMONTH
  - Lookup/reference: HLOOKUP, CHOOSE, ROWS, COLUMNS, ROW, COLUMN. **Context note:** `ROW()`/`COLUMN()` need the evaluating cell's position, threaded into `call_builtin` via `FnCtx` (`EvalContext::current_pos`, set in `eval_cell` from the cell's `SheetIndex` position -- same pattern as `now_serial`). Only the **no-argument** form is supported; the reference form (`ROW(A1)`) and `OFFSET`/`INDIRECT` need reference-preserving compilation (args are pre-evaluated to values, losing the reference identity) and are **deferred to Phase 3**.
- [x] Named ranges, cross-sheet references *(Epic `E3`: `CellRef`/`RangeDep` carry a local `sheet: u16`; the parser emits `Expr::SheetRef`/`Expr::Name`, resolved by a context-aware `resolve_refs` pass against a `FormulaCtx` (sheet display name -> index, defined name -> target). Named ranges fully wired e2e: `named_ranges` LWW CRDT map, `TrameCommand::DefineName`/`DeleteName`, positional resolution on reseed. `reseed_engine` is multi-sheet (active sheet pinned to index 0; cross-sheet `Sheet2!A1` evaluates in the engine, unit-tested). **Gap:** the end-to-end cross-sheet path needs a sheet-creation command (`CreateSheet`/`ActivateSheet`), which lands with the sheet-tabs UI (see Phase 0) -- the engine is ready, only the entry point is missing)*
- [x] Sheet lifecycle commands `CreateSheet` / `RenameSheet` / `MoveSheet` / `DeleteSheet` + `ActivateSheet` (§2.D): peer-scoped `sheet_id` minting (`{peer:016x}_{counter:08x}`), `sheet_order` `LoroMovableList` ops (default sheet implicit-first, never eagerly pushed), last-sheet-deletion guard (`TrameEvent::Error`), active-sheet fallback on delete, delete-driven reseed (drops per-sheet containers) + full engine remap. `TrameEvent::SheetList { sheets, active }` refreshes the tab strip (also on load). Unlocks the end-to-end cross-sheet path (unit-tested `cross_sheet_formula_evaluates_end_to_end`). *(Epic `E3`. The sheet-tabs UI has since landed as the DOM `SheetTabs` component. **Still open:** the last-sheet-deletion guard is enforced **locally** only. The concurrent-deletion rule is now **decided** -- deterministic re-insertion of the first-sorting `sheet_id` (§2.D) -- but not yet implemented; it needs a 2-peer convergence regression test before it ships.)*
- [x] Cross-sheet references bound to `sheet_id` (not display name): the CRDT `v` text is canonicalized to an id-qualified form (`#s0!A1` via the `#` sigil) at edit time, and the formula-bar text is reconstructed through `sheet_id -> name`, so rename/reorder preserve references and delete yields cross-sheet `#REF!` (§2.D rename note, §12). *(Epic `E3`: `formula::SheetQualifier` on `Expr::SheetRef`, parser `#id` token + `FormulaCtx::resolve_sheet_id`, `FormulaError::BrokenRef`/`Op::PushError`, `formula::render` AST printer with `canonicalize`/`to_display_form`, `TrameState::canonicalize_input`/`formula_display` + `sheet_id_to_u16`/`sheet_id_to_name` maps. Tests: parser `#id`/`#REF!`, render round-trips, `cross_sheet_reference_survives_rename_and_breaks_on_delete`, `cross_sheet_reference_converges_across_peers_on_rename`)* **Interop note (deferred):** xlsx/CSV export must map `#id -> name` and import `name -> #id` (tracked in the interop spec). Pre-existing display-name formulas are canonicalized on next edit (no bulk rewrite).

**Phase 3 — Optimization & Completion**
- [ ] `SheetRangeIndex` interval tree 2D if benchmarks justify it
- [ ] Floating objects R-tree
- [ ] `compute_all_dirty` benchmarks on mobile hardware — if > 50ms P95, implement the asynchronous P4 yield (`ContinueDirtyCompute` / `ComputePending`, see §8 optional Phase 2)
- [ ] **Concurrent last-sheet-deletion convergence (§2.D):** implement the decided deterministic re-insertion rule (the first-sorting `sheet_id` is re-inserted identically on every peer when converging deletes empty `sheet_order`), gated by a 2-peer convergence regression test. Today only the **local** last-sheet guard ships.
- [ ] **`RangeEndpoint::EntireAxis` (`A:A` / `1:1`) parsing:** the type variant is reserved but the MVP parser never emits it; wire whole-column/row range parsing + evaluation (§4.B, §9).
- [ ] Advanced functions: XLOOKUP, LAMBDA, dynamic arrays
- [ ] Reference-preserving functions: `ROW(reference)`/`COLUMN(reference)`, OFFSET, INDIRECT *(require the compiler/VM to pass reference identity, not the pre-evaluated value, into the call; the no-arg `ROW()`/`COLUMN()` already ship in Phase 2)*
- [ ] Rayon for independent DAG branches
- [ ] `SheetIndex` O(log N) for shifts
- [ ] PDF export via `KrillaSurface` in `ambre-trame/src/export_pdf.rs` (lazy-loaded)
- [ ] 1M-cell benchmarks

**Phase 4 — Scaling & consistency debt (2026-07 architecture review, §11 bis)**

> Tracked as the "Trame scaling & consistency debt" Story of Epic **E5** in
> `specs/ambre-roadmap.md`. Ordered by impact; A/B unblock the collaboration and P1 budgets.

- [ ] **Incremental remote apply (§11 bis.A):** Loro subscription diffs drive `on_cell_changed` +
  incremental `SheetIndex` maintenance + targeted derived-cache rebuilds; full reseed demoted to
  the snapshot-import / unclassifiable-diff fallback. Restores the §8 P3 contract.
- [ ] **Invalidated-range display events (§11 bis.A, step 2):** replace the per-remote-batch
  `TrameEvent::DataReset` with a ranged invalidation; `DataReset` stays as fallback.
- [ ] **Cached `SheetIndices` with per-sheet invalidation (§11 bis.B):** stop rebuilding non-active
  indices and cloning the active one on every `write_cell` / `finish_cell_writes` /
  `ScrollViewport`. Restores the §7 state shape.
- [ ] **Stable engine sheet indices (§11 bis.C):** de-pin the active sheet from local index 0;
  `ActivateSheet` rebuilds display caches only, no engine reset.
- [ ] **Engine micro-fixes (§11 bis.D):** O(1) `DependencyGraph::remove` (fix the false StableGraph
  reindex comment), targeted cycle check instead of whole-graph `is_cyclic_directed`,
  dirty-subgraph topological recompute, delete the dead write-back loop, drop the `formula_cache`
  `RefCell`.
- [x] **Consolidate the per-cell style fan-out (§11 bis.F):** DONE. One `cellstyle_{sheet}` register
  per cell holding the whole style (bold/italic + `CellStyle`, postcard-encoded), one commit per
  selection inside a single undo group. Folded bold/italic in (the `format_`/`style_`/colour/font/
  numfmt maps are gone), and turned out to be a **correctness** fix after all, not just cost: the old
  fan-out tore border colour vs presence and lost colour/font-only cells on reload. Convergence is now
  whole-cell last-writer-wins (§12, "Concurrent Cell Formatting"). Covered by
  `subset_style_edit_does_not_tear_a_whole_cell_rewrite` and `selection_style_is_a_single_undo`.
  *(The shared `styles` dictionary + `"s"` indirection and the `{v,t,s}` sub-map are explicitly
  **not** part of this item -- deferred behind a benchmark, §11 bis.F "out of scope".)*

> **View-independent merge masking (§11 bis.E)** is a correctness defect, not scaling debt, so it
> is tracked as an **E3 / Phase 1** Story (see the Phase 1 list above), not in this phase.
